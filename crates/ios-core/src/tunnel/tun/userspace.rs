//! Userspace TUN device via smoltcp.
//!
//! Implements a go-ios-compatible userspace TCP/IP stack that:
//! - Accepts local TCP connections on 127.0.0.1:random_port
//! - Forwards them through smoltcp → CDTunnel IPv6 stream
//! - Compatible with go-ios local proxy protocol:
//!   client sends 16-byte IPv6 addr + 4-byte LE port after connecting

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

#[cfg(test)]
use smoltcp::iface::SocketSet;
use smoltcp::iface::{Config, Interface, SocketHandle};
use smoltcp::phy::{Device, DeviceCapabilities, Medium};
use smoltcp::socket::tcp;
use smoltcp::time::Instant as SmolInstant;
use smoltcp::wire::{HardwareAddress, IpAddress, IpCidr, IpEndpoint, Ipv6Address};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use tokio::sync::{watch, OwnedSemaphorePermit, Semaphore};

use crate::tunnel::owner::{self, ChildRole, Ledger, OwnedTask, ParentRole, SharedChildTaskSet};
use crate::tunnel::{TunnelError, ValidatedMtu, IPV6_HEADER_LEN};

pub(crate) mod owned_buffer;
use owned_buffer::OwnedBuffer;
mod runtime_storage;
use runtime_storage::{socket_buffers, OwnedQueue, RuntimeStorage, SocketFees};
mod socket_slots;
use socket_slots::SocketSlots;
mod owned_channel;
use owned_channel as mpsc;

const PREFIX_LEN: u8 = 64;
const IOS_PACKET_CHANNEL_CAPACITY: usize = 8192;
// The device-to-client side can carry packets up to SOCKET_RECV_CHUNK_BYTES;
// keeping this item count small caps queued data at roughly 2 MiB per client
// and lets TCP/mpsc backpressure reach the tunnel instead of retaining hundreds
// of MiB for a stalled local reader.
const LOCAL_PROXY_CHANNEL_CAPACITY: usize = 32;
const SOCKET_BUFFER_BYTES: usize = 1_048_576;
const SOCKET_RECV_CHUNK_BYTES: usize = 65_536;
const EPHEMERAL_PORT_MIN: u16 = 40000;
const EPHEMERAL_PORT_MAX: u16 = u16::MAX;
const EPHEMERAL_PORT_COUNT: usize = (EPHEMERAL_PORT_MAX - EPHEMERAL_PORT_MIN) as usize + 1;
const LOCAL_PROXY_PREAMBLE_LEN: usize = 16 + 4;
const LOCAL_PROXY_PREAMBLE_TIMEOUT: Duration = Duration::from_secs(2);
const MAX_LOCAL_PROXY_CLIENTS: usize = 64;

#[cfg(test)]
fn runtime_channel_bytes() -> usize {
    2 * mpsc::required_bytes::<OwnedBuffer>(IOS_PACKET_CHANNEL_CAPACITY)
        + mpsc::required_bytes::<ConnRequest>(32)
}

#[cfg(test)]
fn client_channel_bytes() -> usize {
    2 * mpsc::required_bytes::<OwnedBuffer>(LOCAL_PROXY_CHANNEL_CAPACITY)
}

// ── smoltcp Device implementation ─────────────────────────────────────────────

#[cfg(test)]
type TxAdmissionHook = std::cell::RefCell<Option<Box<dyn FnOnce(bool)>>>;

#[cfg(test)]
thread_local! {
    static TX_ADMISSION: TxAdmissionHook = const { std::cell::RefCell::new(None) };
}

struct ChannelDevice {
    mtu: ValidatedMtu,
    rx_buf: OwnedQueue<OwnedBuffer>,
    tx_buf: OwnedQueue<OwnedBuffer>,
    storage: RuntimeStorage,
}

impl ChannelDevice {
    fn new(mtu: ValidatedMtu, storage: RuntimeStorage) -> Self {
        Self {
            mtu,
            rx_buf: OwnedQueue::new(storage.bank.as_ref()),
            tx_buf: OwnedQueue::new(storage.bank.as_ref()),
            storage,
        }
    }

    fn prepare_transmit(&mut self) -> Option<Option<OwnedBuffer>> {
        if self.storage.stopped.load(Ordering::Acquire) {
            return None;
        }
        let prepared = self.tx_buf.prepare_push().and_then(|()| {
            self.storage
                .bank
                .as_ref()
                .map(|bank| OwnedBuffer::zeroed(self.mtu.as_usize(), Some(bank)))
                .transpose()
        });
        #[cfg(test)]
        TX_ADMISSION.with(|hook| {
            if let Some(callback) = hook.borrow_mut().take() {
                callback(prepared.is_err());
            }
        });
        match prepared {
            Ok(buffer) => Some(buffer),
            Err(original) => {
                self.storage.fail(ParentRole::Smoltcp, original);
                None
            }
        }
    }
}

struct SmolRxToken(OwnedBuffer);
struct SmolTxToken<'a> {
    queue: &'a mut OwnedQueue<OwnedBuffer>,
    buffer: Option<OwnedBuffer>,
}

impl smoltcp::phy::RxToken for SmolRxToken {
    fn consume<R, F: FnOnce(&mut [u8]) -> R>(mut self, f: F) -> R {
        f(&mut self.0)
    }
}

impl smoltcp::phy::TxToken for SmolTxToken<'_> {
    fn consume<R, F: FnOnce(&mut [u8]) -> R>(mut self, len: usize, f: F) -> R {
        let mut buffer = match self.buffer.take() {
            Some(mut buffer) => {
                // Metered admission completed before this token existed.
                assert!(
                    len <= buffer.len(),
                    "smoltcp transmit exceeds negotiated MTU"
                );
                buffer.truncate(len);
                buffer
            }
            // Preserve the legacy no-bank constructor's exact requested-length
            // allocation. This branch never substitutes for a quota refusal.
            None => OwnedBuffer::unmetered_zeroed(len),
        };
        let result = f(&mut buffer);
        self.queue.push_reserved(buffer);
        result
    }
}

impl Device for ChannelDevice {
    type RxToken<'a> = SmolRxToken;
    type TxToken<'a> = SmolTxToken<'a>;

    fn receive(&mut self, _ts: SmolInstant) -> Option<(Self::RxToken<'_>, Self::TxToken<'_>)> {
        if self.rx_buf.is_empty() {
            return None;
        }
        let buffer = self.prepare_transmit()?;
        let packet = self
            .rx_buf
            .pop_front()
            .expect("observed receive queue slot");
        Some((
            SmolRxToken(packet),
            SmolTxToken {
                queue: &mut self.tx_buf,
                buffer,
            },
        ))
    }

    fn transmit(&mut self, _ts: SmolInstant) -> Option<Self::TxToken<'_>> {
        let buffer = self.prepare_transmit()?;
        Some(SmolTxToken {
            queue: &mut self.tx_buf,
            buffer,
        })
    }

    fn capabilities(&self) -> DeviceCapabilities {
        let mut caps = DeviceCapabilities::default();
        caps.medium = Medium::Ip;
        caps.max_transmission_unit = self.mtu.as_usize();
        caps
    }
}

// ── Connection request ─────────────────────────────────────────────────────────

struct ConnRequest {
    remote_ip: Ipv6Address,
    remote_port: u16,
    from_client: mpsc::Receiver<OwnedBuffer>,
    to_client: mpsc::Sender<OwnedBuffer>,
    connected: tokio::sync::oneshot::Sender<()>,
}

struct ConnState {
    from_client: mpsc::Receiver<OwnedBuffer>,
    to_client: mpsc::Sender<OwnedBuffer>,
    connected: Option<tokio::sync::oneshot::Sender<()>>,
    pending_to_client: OwnedQueue<OwnedBuffer>,
    pending_from_client: OwnedQueue<OwnedBuffer>,
    _socket_fees: SocketFees,
}

// ── Main struct ────────────────────────────────────────────────────────────────

/// Userspace TUN device using smoltcp + local TCP proxy.
pub struct UserspaceTunDevice {
    pub local_port: u16,
    task_handles: Vec<OwnedTask>,
    stopped: Arc<AtomicBool>,
    stop: watch::Sender<()>,
    clients: Arc<SharedChildTaskSet>,
    outcomes: Arc<Ledger>,
}

impl UserspaceTunDevice {
    /// Start the userspace TUN stack.
    ///
    /// Spawns background tasks for:
    /// - Reading/writing IPv6 packets from `ios_stream`
    /// - Running the smoltcp event loop
    /// - Accepting local TCP connections on `127.0.0.1:local_port`
    pub async fn start<S>(
        client_address: &str,
        _server_address: &str,
        mtu: u32,
        ios_stream: S,
    ) -> Result<Self, TunnelError>
    where
        S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin + Send + 'static,
    {
        let mtu = ValidatedMtu::new(mtu)?;
        Self::start_validated(client_address, _server_address, mtu, ios_stream).await
    }

    pub(crate) async fn start_validated<S>(
        client_address: &str,
        _server_address: &str,
        mtu: ValidatedMtu,
        ios_stream: S,
    ) -> Result<Self, TunnelError>
    where
        S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin + Send + 'static,
    {
        Self::start_validated_with_bank(client_address, _server_address, mtu, ios_stream, None)
            .await
    }

    /// Manager-owned runtime storage uses the reservation's original byte bank.
    /// Legacy callers keep the same no-bank constructor and packet semantics.
    pub(crate) async fn start_validated_with_bank<S>(
        client_address: &str,
        _server_address: &str,
        mtu: ValidatedMtu,
        ios_stream: S,
        byte_bank: Option<Arc<Semaphore>>,
    ) -> Result<Self, TunnelError>
    where
        S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin + Send + 'static,
    {
        let payload_capacity = mtu.ipv6_payload_capacity()?;
        let client_ipv6: Ipv6Address = client_address.parse().map_err(|_| {
            TunnelError::Protocol(format!("invalid client IPv6 address: {client_address}"))
        })?;

        // Header admission and allocation finish before listener or workers.
        // The exact Vec and permit then move into the original reader task.
        let header = OwnedBuffer::zeroed(IPV6_HEADER_LEN, byte_bank.as_ref())?;

        let (ios_to_smol_tx, ios_to_smol_rx) =
            mpsc::with_bank::<OwnedBuffer>(IOS_PACKET_CHANNEL_CAPACITY, byte_bank.as_ref())?;
        let (smol_to_ios_tx, smol_to_ios_rx) =
            mpsc::with_bank::<OwnedBuffer>(IOS_PACKET_CHANNEL_CAPACITY, byte_bank.as_ref())?;
        let (conn_req_tx, conn_req_rx) = mpsc::with_bank::<ConnRequest>(32, byte_bank.as_ref())?;

        // All buffer/channel admission precedes listener setup and workers.
        let listener = TcpListener::bind("127.0.0.1:0").await?;
        let local_port = listener.local_addr()?.port();

        let (mut ios_reader, mut ios_writer) = tokio::io::split(ios_stream);
        let outcomes = Ledger::new();
        let clients = SharedChildTaskSet::new(Arc::clone(&outcomes), ChildRole::ProxyClient);
        let (stop, stop_rx) = watch::channel(());

        let stopped = Arc::new(AtomicBool::new(false));
        let storage = RuntimeStorage {
            bank: byte_bank.clone(),
            ledger: Arc::clone(&outcomes),
            stop: stop.clone(),
            stopped: Arc::clone(&stopped),
        };
        let reader_storage = storage.clone();
        let mut reader_stop = stop.subscribe();
        let reader_task = tokio::spawn(async move {
            let result = tokio::select! {
                biased;
                result = read_ios_packets_owned(&mut ios_reader, payload_capacity, ios_to_smol_tx, header, byte_bank) => result,
                _ = reader_stop.changed() => Ok(()),
            };
            if let Err(original) = result {
                reader_storage.fail(ParentRole::ForwardOrReader, original);
            }
            let result = reader_storage.result();
            tracing::debug!("userspace: iOS packet reader exited");
            result
        });

        let writer_storage = storage.clone();
        let mut writer_stop = stop.subscribe();
        let writer_task = tokio::spawn(async move {
            let result = tokio::select! {
                biased;
                result = write_ios_packets_owned(&mut ios_writer, mtu, smol_to_ios_rx) => result,
                _ = writer_stop.changed() => Ok(()),
            };
            if let Err(original) = result {
                writer_storage.fail(ParentRole::Writer, original);
            }
            let result = writer_storage.result();
            tracing::debug!("userspace: iOS packet writer exited");
            result
        });

        let smoltcp_storage = storage.clone();
        let smoltcp_task = tokio::spawn(async move {
            run_smoltcp(
                client_ipv6,
                mtu,
                ios_to_smol_rx,
                smol_to_ios_tx,
                conn_req_rx,
                smoltcp_storage.clone(),
            )
            .await;
            tracing::debug!("userspace: smoltcp loop exited");
            smoltcp_storage.result()
        });

        let listener_clients = Arc::clone(&clients);
        let listener_outcomes = Arc::clone(&outcomes);
        let listener_task = tokio::spawn(async move {
            run_local_listener_owned(
                listener,
                conn_req_tx,
                stop_rx,
                listener_clients,
                listener_outcomes,
                storage,
            )
            .await;
            tracing::debug!("userspace: local listener exited");
            Ok(())
        });

        tracing::info!("userspace TUN started on 127.0.0.1:{local_port}");
        Ok(Self {
            local_port,
            task_handles: vec![
                OwnedTask::new(
                    reader_task,
                    ParentRole::ForwardOrReader,
                    Arc::clone(&outcomes),
                ),
                OwnedTask::new(writer_task, ParentRole::Writer, Arc::clone(&outcomes)),
                OwnedTask::new(smoltcp_task, ParentRole::Smoltcp, Arc::clone(&outcomes)),
                OwnedTask::new(listener_task, ParentRole::Listener, Arc::clone(&outcomes)),
            ],
            stopped,
            stop,
            clients,
            outcomes,
        })
    }

    pub fn is_alive(&self) -> bool {
        !self.stopped.load(Ordering::Acquire)
            && self.task_handles.iter().all(|handle| !handle.is_finished())
    }

    pub(crate) fn request_stop(&self) {
        self.stopped.store(true, Ordering::Release);
        self.stop.send_replace(());
        // Listener remains owned/running long enough to drain its child set.
        for task in self.task_handles.iter().take(3) {
            task.abort();
        }
    }

    /// Stop and join all four workers and proxy children under one cooperative
    /// absolute budget, starting at this call before the future is polled.
    /// Join-mutex waits share it. Zero requests stop and returns native expiry;
    /// overflow returns InvalidInput without stopping. Retain the owner to retry
    /// unfinished joins after timeout or caller-future Drop. Native failures
    /// remain errors after recovery. Final Drop requests cancellation only.
    pub fn shutdown(
        &self,
        budget: Duration,
    ) -> impl std::future::Future<Output = Result<(), TunnelError>> + Send + '_ {
        let deadline = owner::deadline(budget);
        if deadline.is_ok() {
            self.request_stop();
        }
        self.shutdown_until(deadline)
    }

    pub(crate) fn shutdown_until(
        &self,
        deadline: Result<tokio::time::Instant, TunnelError>,
    ) -> impl std::future::Future<Output = Result<(), TunnelError>> + Send + '_ {
        owner::join_until(deadline, &self.outcomes, async {
            for task in &self.task_handles {
                task.join().await;
            }
            self.clients.drain().await;
        })
    }

    /// Observe acquired native failures/join progress without requesting stop
    /// or polling workers. This is an immutable observation, not an attempt.
    pub fn shutdown_status(&self) -> Result<(), TunnelError> {
        self.outcomes.result(None)
    }
}

impl Drop for UserspaceTunDevice {
    fn drop(&mut self) {
        for handle in self.task_handles.drain(..) {
            handle.abort();
        }
    }
}

// ── iOS packet I/O ─────────────────────────────────────────────────────────────

#[cfg(test)]
async fn read_ios_packets<R>(reader: &mut R, payload_capacity: usize, tx: mpsc::Sender<OwnedBuffer>)
where
    R: tokio::io::AsyncRead + Unpin,
{
    let header = OwnedBuffer::zeroed(IPV6_HEADER_LEN, None).unwrap();
    let _ = read_ios_packets_owned(reader, payload_capacity, tx, header, None).await;
}

async fn read_ios_packets_owned<R>(
    reader: &mut R,
    payload_capacity: usize,
    tx: mpsc::Sender<OwnedBuffer>,
    mut hdr: OwnedBuffer,
    byte_bank: Option<Arc<Semaphore>>,
) -> Result<(), TunnelError>
where
    R: tokio::io::AsyncRead + Unpin,
{
    loop {
        reader.read_exact(&mut hdr).await?;
        let payload_len = u16::from_be_bytes([hdr[4], hdr[5]]) as usize;
        if hdr[0] >> 4 != 6 {
            tracing::warn!("userspace: non-IPv6 packet from iOS, skipping");
            discard_bytes_owned(reader, payload_len).await?;
            continue;
        }
        if payload_len > payload_capacity {
            tracing::warn!(
                "userspace: IPv6 payload {payload_len} exceeds negotiated payload capacity {payload_capacity}"
            );
            discard_bytes_owned(reader, payload_len).await?;
            continue;
        }
        let packet_len = match IPV6_HEADER_LEN.checked_add(payload_len) {
            Some(packet_len) => packet_len,
            None => {
                tracing::warn!("userspace: IPv6 packet length overflow");
                break;
            }
        };
        let mut pkt = OwnedBuffer::zeroed(packet_len, byte_bank.as_ref())?;
        pkt[..IPV6_HEADER_LEN].copy_from_slice(&hdr);
        reader.read_exact(&mut pkt[IPV6_HEADER_LEN..]).await?;
        if tx.send(pkt).await.is_err() {
            break;
        }
    }
    Ok(())
}

async fn discard_bytes_owned<R>(reader: &mut R, mut remaining: usize) -> Result<(), TunnelError>
where
    R: tokio::io::AsyncRead + Unpin,
{
    let mut discard = [0u8; 4096];
    while remaining > 0 {
        let chunk_len = remaining.min(discard.len());
        reader.read_exact(&mut discard[..chunk_len]).await?;
        remaining -= chunk_len;
    }
    Ok(())
}

#[cfg(test)]
async fn write_ios_packets<W>(writer: &mut W, mtu: ValidatedMtu, rx: mpsc::Receiver<OwnedBuffer>)
where
    W: tokio::io::AsyncWrite + Unpin,
{
    let _ = write_ios_packets_owned(writer, mtu, rx).await;
}

async fn write_ios_packets_owned<W>(
    writer: &mut W,
    mtu: ValidatedMtu,
    mut rx: mpsc::Receiver<OwnedBuffer>,
) -> Result<(), TunnelError>
where
    W: tokio::io::AsyncWrite + Unpin,
{
    while let Some(pkt) = rx.recv().await {
        if pkt.len() > mtu.as_usize() {
            tracing::warn!(
                "userspace: generated IPv6 packet length {} exceeds negotiated MTU {}",
                pkt.len(),
                mtu.as_u16()
            );
            continue;
        }
        writer.write_all(&pkt).await?;
    }
    Ok(())
}

// ── smoltcp event loop ─────────────────────────────────────────────────────────

/// Floor for a no-progress poll sleep.
///
/// smoltcp reports a zero delay whenever it wants to be polled again right away.
/// This loop only sleeps once an iteration made no progress, so honouring a zero
/// delay literally turns into a busy-spin on a core.
const MIN_POLL_SLEEP: Duration = Duration::from_micros(50);

fn smol_now() -> SmolInstant {
    // smoltcp only needs a monotonic millisecond clock. Wall-clock time can step
    // backwards (NTP correction, manual change), which wedges its timers.
    static START: std::sync::OnceLock<std::time::Instant> = std::sync::OnceLock::new();
    let start = START.get_or_init(std::time::Instant::now);
    SmolInstant::from_millis(start.elapsed().as_millis() as i64)
}

fn smoltcp_poll_sleep_duration(
    delay: Option<smoltcp::time::Duration>,
    has_active_connections: bool,
) -> Duration {
    // Active tunnel sockets need a short poll interval so inbound packets do not sit
    // in the channel queue long enough to starve real-time streams like kperf/fps.
    if has_active_connections {
        return delay
            .map(|delay| {
                Duration::from_micros(delay.total_micros())
                    .clamp(MIN_POLL_SLEEP, Duration::from_millis(1))
            })
            .unwrap_or_else(|| Duration::from_millis(1));
    }

    // `None` means there is no internal timer deadline, but this loop still needs
    // to wake up periodically to observe channel activity.
    delay
        .map(|delay| Duration::from_micros(delay.total_micros()).max(MIN_POLL_SLEEP))
        .unwrap_or_else(|| Duration::from_millis(1))
}

/// Take the next free local port for a tunneled socket.
///
/// smoltcp identifies a socket by its full 4-tuple, so reusing a port that a live
/// connection still holds makes `connect` fail — or, worse, lands two tunneled
/// streams on the same tuple. Walks the ephemeral range from `cursor`, wrapping at
/// its end (a plain `wrapping_add` would drop the cursor into the reserved low
/// ports) and skipping everything `in_use` still claims. `None` means the range is
/// fully taken.
fn take_ephemeral_port(cursor: &mut u16, mut in_use: impl FnMut(u16) -> bool) -> Option<u16> {
    for _ in 0..EPHEMERAL_PORT_COUNT {
        let port = *cursor;
        // EPHEMERAL_PORT_MAX is u16::MAX, so `==` is the whole wrap condition.
        *cursor = if port == EPHEMERAL_PORT_MAX {
            EPHEMERAL_PORT_MIN
        } else {
            port + 1
        };
        if !in_use(port) {
            return Some(port);
        }
    }
    None
}

async fn run_smoltcp(
    client_ipv6: Ipv6Address,
    mtu: ValidatedMtu,
    mut ios_rx: mpsc::Receiver<OwnedBuffer>,
    ios_tx: mpsc::Sender<OwnedBuffer>,
    mut conn_req_rx: mpsc::Receiver<ConnRequest>,
    storage: RuntimeStorage,
) {
    let mut device = ChannelDevice::new(mtu, storage.clone());
    let config = Config::new(HardwareAddress::Ip);
    let mut iface = Interface::new(config, &mut device, smol_now());
    iface.update_ip_addrs(|addrs| {
        let _ = addrs.push(IpCidr::new(IpAddress::Ipv6(client_ipv6), PREFIX_LEN));
    });

    // Declare the fee owners before sockets: SocketSet drops first on normal,
    // cancelled or unwinding exits, and explicit removal follows the same order.
    let mut connections: OwnedQueue<(SocketHandle, ConnState)> =
        OwnedQueue::new(storage.bank.as_ref());
    let mut sockets = SocketSlots::new(storage.bank.as_ref());
    let mut ephemeral_port: u16 = EPHEMERAL_PORT_MIN;
    let mut pending_ios_tx = OwnedQueue::new(storage.bank.as_ref());

    loop {
        if storage.stopped.load(Ordering::Acquire) {
            return;
        }
        let mut made_progress = false;

        while let Ok(pkt) = ios_rx.try_recv() {
            tracing::trace!("userspace: rx packet {} bytes from iOS", pkt.len());
            if let Err((original, packet)) = device.rx_buf.push_back(pkt) {
                storage.fail(ParentRole::Smoltcp, original);
                drop(packet);
                return;
            }
            made_progress = true;
        }

        while let Ok(req) = conn_req_rx.try_recv() {
            tracing::info!(
                remote = %crate::redaction::diagnostic_id(&req.remote_ip.to_string()),
                port = req.remote_port,
                "userspace: new conn request"
            );
            // All fallible connection backing work ends before any socket
            // buffer is allocated or transferred to SocketSet. Later insertion
            // is reserved and cannot drop fees inside a failed queue admission.
            if let Err(original) = connections.prepare_push() {
                storage.fail(ParentRole::Smoltcp, original);
                return;
            }
            if let Err(original) = sockets.prepare_add() {
                storage.fail(ParentRole::Smoltcp, original);
                return;
            }
            let (rx, tx, socket_fees) =
                match socket_buffers(storage.bank.as_ref(), SOCKET_BUFFER_BYTES) {
                    Ok(buffers) => buffers,
                    Err(original) => {
                        storage.fail(ParentRole::Smoltcp, original);
                        return;
                    }
                };
            let rx_buf = tcp::SocketBuffer::new(rx);
            let tx_buf = tcp::SocketBuffer::new(tx);
            let mut socket = tcp::Socket::new(rx_buf, tx_buf);
            socket.set_timeout(Some(smoltcp::time::Duration::from_secs(30)));
            socket.set_keep_alive(Some(smoltcp::time::Duration::from_secs(1)));
            let Some(local_port) = take_ephemeral_port(&mut ephemeral_port, |port| {
                connections.iter().any(|(handle, _)| {
                    let handle = *handle;
                    sockets.with_set(|set| {
                        set.get::<tcp::Socket>(handle)
                            .local_endpoint()
                            .is_some_and(|endpoint| endpoint.port == port)
                    })
                })
            }) else {
                tracing::error!(
                    remote = %crate::redaction::diagnostic_id(&req.remote_ip.to_string()),
                    port = req.remote_port,
                    "userspace: no free ephemeral port"
                );
                continue;
            };
            let remote_ep = IpEndpoint::new(IpAddress::Ipv6(req.remote_ip), req.remote_port);
            match socket.connect(iface.context(), remote_ep, local_port) {
                Ok(()) => {
                    tracing::debug!("userspace: socket connected state={:?}", socket.state());
                    let handle = sockets.with_set(|set| set.add(socket));
                    let state = ConnState {
                        from_client: req.from_client,
                        to_client: req.to_client,
                        connected: Some(req.connected),
                        pending_to_client: OwnedQueue::new(storage.bank.as_ref()),
                        pending_from_client: OwnedQueue::new(storage.bank.as_ref()),
                        _socket_fees: socket_fees,
                    };
                    connections.push_reserved((handle, state));
                    made_progress = true;
                }
                Err(e) => {
                    tracing::error!("userspace: smoltcp connect failed: {e:?}");
                }
            }
        }

        sockets.with_set(|set| iface.poll(smol_now(), &mut device, set));
        if storage.stopped.load(Ordering::Acquire) {
            return;
        }

        while let Some(pkt) = device.tx_buf.pop_front() {
            tracing::trace!("userspace: tx packet {} bytes to iOS", pkt.len());
            match ios_tx.try_send(pkt) {
                Ok(()) => {
                    made_progress = true;
                }
                Err(mpsc::error::TrySendError::Full(pkt)) => {
                    if let Err((original, packet)) = pending_ios_tx.push_back(pkt) {
                        storage.fail(ParentRole::Smoltcp, original);
                        drop(packet);
                        return;
                    }
                    break;
                }
                Err(mpsc::error::TrySendError::Closed(_)) => {
                    tracing::debug!("userspace: iOS packet writer closed");
                    return;
                }
            }
        }
        made_progress |= flush_channel_queue(&ios_tx, &mut pending_ios_tx);

        let mut to_remove = OwnedQueue::new(storage.bank.as_ref());
        let socket_result = sockets.with_set(|set| -> Result<(), ()> {
            for (handle, state) in connections.iter_mut() {
                let handle = *handle;
                let socket = set.get_mut::<tcp::Socket>(handle);
                tracing::trace!(
                    "userspace: socket state={:?} can_send={} can_recv={} is_open={}",
                    socket.state(),
                    socket.can_send(),
                    socket.can_recv(),
                    socket.is_open()
                );

                // Notify proxy_local_client when TCP connection is established
                if socket.state() == tcp::State::Established {
                    if let Some(tx) = state.connected.take() {
                        let _ = tx.send(());
                        made_progress = true;
                    }
                }
                made_progress |=
                    flush_channel_queue(&state.to_client, &mut state.pending_to_client);
                if state.pending_to_client.is_empty() {
                    while socket.can_recv() {
                        let mut buf = match OwnedBuffer::zeroed(
                            SOCKET_RECV_CHUNK_BYTES,
                            storage.bank.as_ref(),
                        ) {
                            Ok(buffer) => buffer,
                            Err(original) => {
                                storage.fail(ParentRole::Smoltcp, original);
                                return Err(());
                            }
                        };
                        match socket.recv_slice(&mut buf) {
                            Ok(0) => break,
                            Ok(n) => {
                                buf.truncate(n);
                                tracing::trace!("userspace: from_device {} bytes", n);
                                match state.to_client.try_send(buf) {
                                    Ok(()) => {
                                        made_progress = true;
                                    }
                                    Err(mpsc::error::TrySendError::Full(buf)) => {
                                        if let Err((original, buffer)) =
                                            state.pending_to_client.push_back(buf)
                                        {
                                            storage.fail(ParentRole::Smoltcp, original);
                                            drop(buffer);
                                            return Err(());
                                        }
                                        break;
                                    }
                                    Err(mpsc::error::TrySendError::Closed(_)) => {
                                        tracing::debug!(
                                            "userspace: local client receive channel closed"
                                        );
                                        socket.close();
                                        break;
                                    }
                                }
                            }
                            Err(_) => break,
                        }
                    }
                }
                made_progress |= flush_socket_send(socket, &mut state.pending_from_client);
                if state.pending_from_client.is_empty() && socket.can_send() {
                    loop {
                        let data = match state.from_client.try_recv() {
                            Ok(data) => data,
                            Err(mpsc::error::TryRecvError::Empty) => break,
                            Err(mpsc::error::TryRecvError::Disconnected) => {
                                socket.close();
                                break;
                            }
                        };
                        tracing::trace!("userspace: to_device {} bytes via socket", data.len());
                        if let Err((original, buffer)) = state.pending_from_client.push_back(data) {
                            storage.fail(ParentRole::Smoltcp, original);
                            drop(buffer);
                            return Err(());
                        }
                        made_progress |= flush_socket_send(socket, &mut state.pending_from_client);
                        if !state.pending_from_client.is_empty() || !socket.can_send() {
                            break;
                        }
                    }
                }
                if !socket.is_open() {
                    tracing::debug!("userspace: socket closed state={:?}", socket.state());
                    if let Err((original, _)) = to_remove.push_back(handle) {
                        storage.fail(ParentRole::Smoltcp, original);
                        return Err(());
                    }
                }
            }
            Ok(())
        });
        if socket_result.is_err() {
            return;
        }
        while let Some(handle) = to_remove.pop_front() {
            sockets.with_set(|set| drop(set.remove(handle)));
            let index = connections
                .iter()
                .position(|(current, _)| *current == handle)
                .expect("original socket connection owner");
            drop(connections.remove(index));
        }

        if made_progress {
            tokio::task::yield_now().await;
            continue;
        }

        let mut stop = storage.stop.subscribe();
        if storage.stopped.load(Ordering::Acquire) {
            return;
        }
        tokio::select! {
            biased;
            _ = stop.changed() => return,
            _ = tokio::time::sleep(smoltcp_poll_sleep_duration(
                sockets.with_set(|set| iface.poll_delay(smol_now(), set)), !connections.is_empty(),
            )) => {},
        }
    }
}

fn flush_channel_queue<T>(tx: &mpsc::Sender<T>, pending: &mut OwnedQueue<T>) -> bool {
    let mut made_progress = false;
    while let Some(buf) = pending.pop_front() {
        match tx.try_send(buf) {
            Ok(()) => {
                made_progress = true;
            }
            Err(mpsc::error::TrySendError::Full(buf)) => {
                pending.push_front_reserved(buf);
                break;
            }
            Err(mpsc::error::TrySendError::Closed(_)) => {
                pending.clear();
                break;
            }
        }
    }
    made_progress
}

fn flush_socket_send(socket: &mut tcp::Socket, pending: &mut OwnedQueue<OwnedBuffer>) -> bool {
    let mut made_progress = false;
    while socket.can_send() {
        let Some(mut buf) = pending.pop_front() else {
            break;
        };
        match socket.send_slice(&buf) {
            Ok(sent) if sent == buf.len() => {
                made_progress = true;
            }
            Ok(sent) => {
                if sent < buf.len() {
                    buf.advance(sent);
                    pending.push_front_reserved(buf);
                }
                made_progress |= sent > 0;
                break;
            }
            Err(_) => {
                pending.push_front_reserved(buf);
                break;
            }
        }
    }
    made_progress
}

// ── Local TCP listener (go-ios compatible protocol) ────────────────────────────

struct ListenerLimits {
    max_clients: usize,
    preamble_timeout: Duration,
}

async fn run_local_listener_owned(
    listener: TcpListener,
    conn_req_tx: mpsc::Sender<ConnRequest>,
    stop: watch::Receiver<()>,
    clients: Arc<SharedChildTaskSet>,
    outcomes: Arc<Ledger>,
    storage: RuntimeStorage,
) {
    run_local_listener_with_limit_storage(
        listener,
        conn_req_tx,
        ListenerLimits {
            max_clients: MAX_LOCAL_PROXY_CLIENTS,
            preamble_timeout: LOCAL_PROXY_PREAMBLE_TIMEOUT,
        },
        stop,
        clients,
        outcomes,
        storage,
    )
    .await;
}

#[cfg(test)]
async fn run_local_listener_with_limit(
    listener: TcpListener,
    conn_req_tx: mpsc::Sender<ConnRequest>,
    max_clients: usize,
    preamble_timeout: Duration,
) {
    let outcomes = Ledger::new();
    let clients = SharedChildTaskSet::new(Arc::clone(&outcomes), ChildRole::ProxyClient);
    let (_stop, receiver) = watch::channel(());
    run_local_listener_with_limit_owned(
        listener,
        conn_req_tx,
        max_clients,
        preamble_timeout,
        receiver,
        clients,
        outcomes,
    )
    .await;
}

#[cfg(test)]
async fn run_local_listener_with_limit_owned(
    listener: TcpListener,
    conn_req_tx: mpsc::Sender<ConnRequest>,
    max_clients: usize,
    preamble_timeout: Duration,
    stop: watch::Receiver<()>,
    clients: Arc<SharedChildTaskSet>,
    outcomes: Arc<Ledger>,
) {
    run_local_listener_with_limit_storage(
        listener,
        conn_req_tx,
        ListenerLimits {
            max_clients,
            preamble_timeout,
        },
        stop,
        clients,
        outcomes,
        RuntimeStorage::unmetered(),
    )
    .await;
}

async fn run_local_listener_with_limit_storage(
    listener: TcpListener,
    conn_req_tx: mpsc::Sender<ConnRequest>,
    limits: ListenerLimits,
    mut stop: watch::Receiver<()>,
    clients: Arc<SharedChildTaskSet>,
    outcomes: Arc<Ledger>,
    storage: RuntimeStorage,
) {
    let ListenerLimits {
        max_clients,
        preamble_timeout,
    } = limits;
    let permits = Arc::new(Semaphore::new(max_clients));
    let mut client_tasks = clients.lock().await;

    loop {
        tokio::select! {
            biased;
            _ = stop.changed() => break,
            _ = conn_req_tx.closed() => break,
            joined = client_tasks.join_next(), if !client_tasks.is_empty() => {
                if let Some(Err(error)) = joined {
                    tracing::debug!(error = %crate::redaction::diagnostic_error(&error), "userspace: local proxy task exited with error");
                }
            }
            accepted = listener.accept() => {
                let (client, peer) = match accepted {
                    Ok(accepted) => accepted,
                    Err(error) => {
                        // Capture the acquired native primary before child joins.
                        outcomes.record_work(3, Arc::new(TunnelError::Io(error)));
                        break;
                    }
                };
                let permit = match permits.clone().try_acquire_owned() {
                    Ok(permit) => permit,
                    Err(_) => {
                        tracing::debug!(peer = %crate::redaction::diagnostic_id(&peer.to_string()), "userspace: rejecting local connection: proxy limit reached");
                        // Dropping the stream sends a connection reset/FIN without
                        // allowing an excess client to consume an unbounded task.
                        drop(client);
                        continue;
                    }
                };
                tracing::info!(peer = %crate::redaction::diagnostic_id(&peer.to_string()), "userspace: local connection");
                let tx = conn_req_tx.clone();
                let storage = storage.clone();
                client_tasks.spawn(async move {
                    handle_local_client_storage(client, tx, permit, preamble_timeout, storage).await;
                });
            }
        }
    }
    drop(listener);
    client_tasks.abort_all();
    while client_tasks.join_next().await.is_some() {}
}

#[cfg(test)]
async fn handle_local_client(
    client: tokio::net::TcpStream,
    conn_req_tx: mpsc::Sender<ConnRequest>,
    permit: OwnedSemaphorePermit,
    preamble_timeout: Duration,
) {
    handle_local_client_storage(
        client,
        conn_req_tx,
        permit,
        preamble_timeout,
        RuntimeStorage::unmetered(),
    )
    .await;
}

async fn handle_local_client_storage(
    mut client: tokio::net::TcpStream,
    conn_req_tx: mpsc::Sender<ConnRequest>,
    _permit: OwnedSemaphorePermit,
    preamble_timeout: Duration,
    storage: RuntimeStorage,
) {
    let (remote_ip, remote_port) = match tokio::time::timeout(
        preamble_timeout,
        read_proxy_preamble(&mut client),
    )
    .await
    {
        Ok(Ok(remote)) => remote,
        Ok(Err(error)) => {
            tracing::debug!(error = %crate::redaction::diagnostic_error(&error), "userspace: local proxy preamble read failed");
            return;
        }
        Err(_) => {
            tracing::debug!(
                "userspace: local proxy preamble timed out after {} ms",
                preamble_timeout.as_millis()
            );
            return;
        }
    };

    tracing::info!(remote = %crate::redaction::diagnostic_id(&remote_ip.to_string()), port = remote_port, "userspace: tunneling");

    let channels = (|| -> Result<_, TunnelError> {
        Ok((
            mpsc::with_bank::<OwnedBuffer>(LOCAL_PROXY_CHANNEL_CAPACITY, storage.bank.as_ref())?,
            mpsc::with_bank::<OwnedBuffer>(LOCAL_PROXY_CHANNEL_CAPACITY, storage.bank.as_ref())?,
        ))
    })();
    let ((from_client_tx, from_client_rx), (to_client_tx, to_client_rx)) = match channels {
        Ok(channels) => channels,
        Err(original) => {
            storage.fail(ParentRole::Smoltcp, original);
            return;
        }
    };
    let (connected_tx, connected_rx) = tokio::sync::oneshot::channel::<()>();

    let req = ConnRequest {
        remote_ip,
        remote_port,
        from_client: from_client_rx,
        to_client: to_client_tx,
        connected: connected_tx,
    };
    if conn_req_tx.send(req).await.is_err() {
        return;
    }

    proxy_local_client(client, from_client_tx, to_client_rx, connected_rx, storage).await;
}

async fn read_proxy_preamble(
    client: &mut tokio::net::TcpStream,
) -> std::io::Result<(Ipv6Address, u16)> {
    let mut preamble = [0u8; LOCAL_PROXY_PREAMBLE_LEN];
    client.read_exact(&mut preamble).await?;
    let remote_port = u32::from_le_bytes(preamble[16..20].try_into().expect("fixed preamble"));
    let remote_port = u16::try_from(remote_port).map_err(|source| {
        crate::error::context::retained_io(
            std::io::ErrorKind::InvalidInput,
            "decode userspace proxy port",
            source,
        )
    })?;
    let mut remote_addr = [0u8; 16];
    remote_addr.copy_from_slice(&preamble[..16]);
    Ok((Ipv6Address::from_bytes(&remote_addr), remote_port))
}

async fn proxy_local_client(
    client: tokio::net::TcpStream,
    to_smoltcp: mpsc::Sender<OwnedBuffer>,
    mut from_smoltcp: mpsc::Receiver<OwnedBuffer>,
    connected: tokio::sync::oneshot::Receiver<()>,
    storage: RuntimeStorage,
) {
    // Wait for TCP connection to be established before forwarding data
    // This mirrors gVisor's behavior of waiting for Connect() to complete
    if connected.await.is_err() {
        tracing::debug!("userspace: connection aborted before established");
        return;
    }

    let (mut r, mut w) = client.into_split();
    let read_half = async {
        loop {
            let mut buf = match OwnedBuffer::zeroed(4096, storage.bank.as_ref()) {
                Ok(buffer) => buffer,
                Err(original) => {
                    storage.fail(ParentRole::Smoltcp, original);
                    return;
                }
            };
            match r.read(&mut buf).await {
                Ok(0) | Err(_) => break,
                Ok(n) => {
                    buf.truncate(n);
                    if to_smoltcp.send(buf).await.is_err() {
                        break;
                    }
                }
            }
        }
    };
    let write_half = async {
        while let Some(data) = from_smoltcp.recv().await {
            if w.write_all(&data).await.is_err() {
                break;
            }
        }
    };
    tokio::select! { _ = read_half => {} _ = write_half => {} }
}

#[cfg(test)]
mod shutdown_tests;

#[cfg(test)]
mod receive_storage_tests;

#[cfg(test)]
mod transmit_storage_tests;

#[cfg(test)]
mod tests {
    fn test_buffer(bytes: &[u8]) -> super::OwnedBuffer {
        let mut buffer = super::OwnedBuffer::zeroed(bytes.len(), None).unwrap();
        buffer.copy_from_slice(bytes);
        buffer
    }
    #[tokio::test]
    async fn connection_native_proxy_port_keeps_real_conversion_and_local_eof() {
        use std::error::Error;
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        tokio::time::timeout(std::time::Duration::from_secs(1), async {
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
            let address = listener.local_addr().unwrap();
            let caller = async move {
                let mut client = tokio::net::TcpStream::connect(address).await.unwrap();
                client
                    .write_all(&proxy_preamble([0; 16], u32::MAX))
                    .await
                    .unwrap();
                client.shutdown().await.unwrap();
                let mut reply = Vec::new();
                client.read_to_end(&mut reply).await.unwrap();
                assert!(reply.is_empty());
            };
            let server = async move {
                let (mut peer, _) = listener.accept().await.unwrap();
                let error = super::read_proxy_preamble(&mut peer).await.unwrap_err();
                assert_eq!(error.kind(), std::io::ErrorKind::InvalidInput);
                assert_eq!(
                    crate::classify_error(&error),
                    crate::ErrorClass::InvalidInput
                );
                assert!(error.source().unwrap().is::<std::num::TryFromIntError>());
                drop(peer);
            };
            tokio::join!(caller, server);
        })
        .await
        .unwrap();
    }

    use std::collections::VecDeque;
    use std::pin::Pin;
    use std::task::{Context, Poll};

    use super::mpsc;
    use tokio::io::AsyncWrite;

    use super::*;

    #[cfg(target_os = "linux")]
    #[tokio::test(flavor = "current_thread")]
    async fn lifecycle_userspace_native_bind_failure_releases_stream_without_workers() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        use tokio::io::{AsyncRead, ReadBuf};

        const CHILD_ENV: &str = "IOS_USERSPACE_BIND_FAILURE_CHILD";
        const TEST_NAME: &str = "tunnel::tun::userspace::tests::lifecycle_userspace_native_bind_failure_releases_stream_without_workers";
        let isolated_child = std::env::var_os(CHILD_ENV).is_some()
            && std::env::args().any(|argument| argument == "--exact");
        if !isolated_child {
            // Descriptor limits are process-wide. Only this exact child changes
            // them; the parent harness keeps its normal limit and runtime.
            let mut child = tokio::process::Command::new(std::env::current_exe().unwrap())
                .args(["--exact", TEST_NAME, "--test-threads=1"])
                .env(CHILD_ENV, "1")
                .stdin(std::process::Stdio::null())
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .kill_on_drop(true)
                .spawn()
                .expect("spawn isolated userspace bind-failure fixture");
            match tokio::time::timeout(Duration::from_secs(10), child.wait()).await {
                Ok(Ok(status)) => {
                    assert!(status.success(), "isolated userspace bind fixture failed");
                }
                result => {
                    // Both a timeout and a wait error keep ownership until an
                    // explicit kill request and a separate, bounded reap.
                    let kill = child.start_kill();
                    let reaped = tokio::time::timeout(Duration::from_secs(2), child.wait()).await;
                    panic!(
                        "isolated userspace bind fixture did not exit: wait={result:?}, kill={kill:?}, reap={reaped:?}"
                    );
                }
            }
            return;
        }

        struct DescriptorLimit {
            original: libc::rlimit,
            restored: bool,
        }

        impl DescriptorLimit {
            fn reduce() -> Self {
                let mut original = libc::rlimit {
                    rlim_cur: 0,
                    rlim_max: 0,
                };
                // SAFETY: a valid writable rlimit pointer is supplied.
                assert_eq!(
                    unsafe { libc::getrlimit(libc::RLIMIT_NOFILE, &mut original) },
                    0
                );
                let limit = libc::rlimit {
                    rlim_cur: original.rlim_cur.min(64),
                    rlim_max: original.rlim_max,
                };
                let guard = Self {
                    original,
                    restored: false,
                };
                // SAFETY: only this isolated child's soft limit is reduced;
                // the hard limit and the valid original value are preserved.
                assert_eq!(unsafe { libc::setrlimit(libc::RLIMIT_NOFILE, &limit) }, 0);
                guard
            }

            fn restore(mut self) {
                // SAFETY: the original limits came from getrlimit and the
                // hard limit was never changed by this fixture.
                assert_eq!(
                    unsafe { libc::setrlimit(libc::RLIMIT_NOFILE, &self.original) },
                    0
                );
                self.restored = true;
            }
        }

        impl Drop for DescriptorLimit {
            fn drop(&mut self) {
                if !self.restored {
                    // SAFETY: restore the saved, valid limit on assertion or
                    // other early exits. The normal path checks restoration.
                    let _ = unsafe { libc::setrlimit(libc::RLIMIT_NOFILE, &self.original) };
                }
            }
        }

        #[derive(Default)]
        struct Probe {
            read_polls: AtomicUsize,
            write_polls: AtomicUsize,
            drops: AtomicUsize,
        }

        struct TrackedStream {
            inner: tokio::io::DuplexStream,
            probe: Arc<Probe>,
        }

        impl AsyncRead for TrackedStream {
            fn poll_read(
                self: Pin<&mut Self>,
                cx: &mut Context<'_>,
                buf: &mut ReadBuf<'_>,
            ) -> Poll<std::io::Result<()>> {
                let this = self.get_mut();
                this.probe.read_polls.fetch_add(1, Ordering::SeqCst);
                Pin::new(&mut this.inner).poll_read(cx, buf)
            }
        }

        impl AsyncWrite for TrackedStream {
            fn poll_write(
                self: Pin<&mut Self>,
                cx: &mut Context<'_>,
                buf: &[u8],
            ) -> Poll<std::io::Result<usize>> {
                let this = self.get_mut();
                this.probe.write_polls.fetch_add(1, Ordering::SeqCst);
                Pin::new(&mut this.inner).poll_write(cx, buf)
            }

            fn poll_flush(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
                let this = self.get_mut();
                this.probe.write_polls.fetch_add(1, Ordering::SeqCst);
                Pin::new(&mut this.inner).poll_flush(cx)
            }

            fn poll_shutdown(
                self: Pin<&mut Self>,
                cx: &mut Context<'_>,
            ) -> Poll<std::io::Result<()>> {
                let this = self.get_mut();
                this.probe.write_polls.fetch_add(1, Ordering::SeqCst);
                Pin::new(&mut this.inner).poll_shutdown(cx)
            }
        }

        impl Drop for TrackedStream {
            fn drop(&mut self) {
                self.probe.drops.fetch_add(1, Ordering::SeqCst);
            }
        }

        // The current-thread runtime and its driver already exist. This
        // stable Tokio metric is consistent here and requires no new FD.
        let metrics = tokio::runtime::Handle::current().metrics();
        let baseline = metrics.num_alive_tasks();
        let limit = DescriptorLimit::reduce();
        let mut held_files = Vec::new();
        loop {
            match std::fs::File::open("/dev/null") {
                Ok(file) => {
                    assert!(held_files.len() < 64, "descriptor exhaustion is bounded");
                    held_files.push(file);
                }
                Err(error) => {
                    assert_eq!(error.raw_os_error(), Some(libc::EMFILE));
                    break;
                }
            }
        }

        for attempt in 0..3 {
            let (inner, mut peer) = tokio::io::duplex(128);
            let probe = Arc::new(Probe::default());
            let stream = TrackedStream {
                inner,
                probe: Arc::clone(&probe),
            };
            let error = UserspaceTunDevice::start("fd00::1", "fd00::2", 1280, stream)
                .await
                .err()
                .expect("native listener bind must fail with no available descriptors");
            assert_eq!(crate::classify_error(&error), crate::ErrorClass::Io);
            let native = std::error::Error::source(&error)
                .and_then(|source| source.downcast_ref::<std::io::Error>())
                .expect("native bind failure lost its IO source");
            assert_eq!(native.raw_os_error(), Some(libc::EMFILE));
            let TunnelError::Io(error) = error else {
                panic!("native bind failure lost its IO error variant");
            };
            assert_eq!(error.raw_os_error(), Some(libc::EMFILE));
            assert_eq!(crate::classify_error(&error), crate::ErrorClass::Io);
            eprintln!(
                "failed construction attempt={attempt}, alive={}, baseline={baseline}, drops={}, read_polls={}, write_polls={}",
                metrics.num_alive_tasks(),
                probe.drops.load(Ordering::SeqCst),
                probe.read_polls.load(Ordering::SeqCst),
                probe.write_polls.load(Ordering::SeqCst),
            );
            assert_eq!(
                metrics.num_alive_tasks(),
                baseline,
                "failed construction spawned workers on attempt {attempt}"
            );
            assert_eq!(probe.drops.load(Ordering::SeqCst), 1);
            assert_eq!(probe.read_polls.load(Ordering::SeqCst), 0);
            assert_eq!(probe.write_polls.load(Ordering::SeqCst), 0);
            // Keep the actual peer open: EOF must come from releasing the
            // failed construction's owned stream, not from dropping the peer.
            let mut byte = [0u8; 1];
            assert_eq!(
                tokio::time::timeout(Duration::from_secs(1), peer.read(&mut byte))
                    .await
                    .expect("failed construction retained the duplex stream")
                    .unwrap(),
                0
            );
            tokio::task::yield_now().await;
            assert_eq!(metrics.num_alive_tasks(), baseline);
        }

        drop(held_files);
        limit.restore();

        // A real successful bind after restoring the limit still starts all
        // four workers. No local proxy client or physical device is involved.
        let (inner, mut peer) = tokio::io::duplex(128);
        let probe = Arc::new(Probe::default());
        let stream = TrackedStream {
            inner,
            probe: Arc::clone(&probe),
        };
        let device = UserspaceTunDevice::start("fd00::1", "fd00::2", 1280, stream)
            .await
            .expect("userspace initialization after descriptor-limit restoration");
        assert_ne!(device.local_port, 0);
        assert_eq!(device.task_handles.len(), 4);
        assert!(device.is_alive());
        assert_eq!(metrics.num_alive_tasks(), baseline + 4);
        assert_eq!(probe.drops.load(Ordering::SeqCst), 0);
        tokio::time::timeout(Duration::from_secs(1), async {
            while probe.read_polls.load(Ordering::SeqCst) == 0 {
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("successful constructor's packet reader was not polled");
        assert!(device.is_alive());
        drop(device);
        // Observe this fixture's abort completion and stream release; this
        // does not add or certify an explicit join API or device cleanup ACK.
        tokio::time::timeout(Duration::from_secs(1), async {
            while metrics.num_alive_tasks() != baseline || probe.drops.load(Ordering::SeqCst) != 1 {
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("dropped successful userspace owner retained workers or stream");
        let mut byte = [0u8; 1];
        assert_eq!(
            tokio::time::timeout(Duration::from_secs(1), peer.read(&mut byte))
                .await
                .expect("dropped successful userspace owner retained duplex peer")
                .unwrap(),
            0
        );
    }

    #[tokio::test]
    async fn proxy_timeout_and_eof_diagnostics_hide_raw_partial_preamble() {
        const CHILD_ENV: &str = "IOS_PROXY_DIAGNOSTIC_CAPTURE_CHILD";
        const TEST_NAME: &str = "tunnel::tun::userspace::tests::proxy_timeout_and_eof_diagnostics_hide_raw_partial_preamble";
        let isolated_child = std::env::var_os(CHILD_ENV).is_some()
            && std::env::args().any(|argument| argument == "--exact");
        if !isolated_child {
            // Tracing callsite interests and maximum levels are process-wide.
            // Capture in a dedicated harness so other parallel tests cannot
            // change the filter while this async fixture is suspended.
            let mut child = tokio::process::Command::new(std::env::current_exe().unwrap())
                .args(["--exact", TEST_NAME, "--test-threads=1"])
                .env(CHILD_ENV, "1")
                .stdin(std::process::Stdio::null())
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .kill_on_drop(true)
                .spawn()
                .expect("spawn isolated proxy diagnostic fixture");
            let result = tokio::time::timeout(Duration::from_secs(10), child.wait()).await;
            if result.is_err() {
                let _ = child.start_kill();
                let _ = tokio::time::timeout(Duration::from_secs(2), child.wait()).await;
                panic!("isolated proxy diagnostic fixture timed out");
            }
            assert!(
                result
                    .unwrap()
                    .expect("wait for diagnostic fixture")
                    .success(),
                "isolated proxy diagnostic fixture failed"
            );
            return;
        }
        let (dispatch, log) = crate::redaction::test_support::capture();
        tracing::dispatcher::set_global_default(dispatch).expect("isolated diagnostic subscriber");
        let secret = b"secret42";
        for eof in [false, true] {
            let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
            let mut client = tokio::net::TcpStream::connect(listener.local_addr().unwrap())
                .await
                .unwrap();
            let (server, _) = listener.accept().await.unwrap();
            client.write_all(secret).await.unwrap();
            let held_client = if eof {
                drop(client);
                None
            } else {
                Some(client)
            };
            let semaphore = Arc::new(Semaphore::new(1));
            let permit = semaphore.clone().acquire_owned().await.unwrap();
            let (tx, mut rx) = mpsc::channel(1);
            *log.lock().unwrap() = Default::default();
            handle_local_client(server, tx, permit, Duration::from_millis(20)).await;
            let log = log.lock().unwrap();
            assert!(!log.overflow);
            assert!(!log.raw.contains(std::str::from_utf8(secret).unwrap()));
            assert!(!log.raw.contains(&hex::encode(secret)));
            assert!(!log.raw.contains(&format!("{secret:?}")));
            assert!(
                log.raw.contains(if eof {
                    "preamble read failed"
                } else {
                    "preamble timed out"
                }),
                "expected lifecycle phase missing; eof={eof}, captured_bytes={}",
                log.raw.len()
            );
            if eof {
                assert!(log.raw.contains("io/UnexpectedEof error:"));
            }
            assert_eq!(semaphore.available_permits(), 1);
            assert!(rx.try_recv().is_err());
            drop(held_client);
        }
    }

    #[derive(Default)]
    struct CountingWriter {
        writes: Vec<Vec<u8>>,
        flushes: usize,
    }

    impl AsyncWrite for CountingWriter {
        fn poll_write(
            mut self: Pin<&mut Self>,
            _cx: &mut Context<'_>,
            buf: &[u8],
        ) -> Poll<std::io::Result<usize>> {
            self.writes.push(buf.to_vec());
            Poll::Ready(Ok(buf.len()))
        }

        fn poll_flush(
            mut self: Pin<&mut Self>,
            _cx: &mut Context<'_>,
        ) -> Poll<std::io::Result<()>> {
            self.flushes += 1;
            Poll::Ready(Ok(()))
        }

        fn poll_shutdown(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
            Poll::Ready(Ok(()))
        }
    }

    #[tokio::test]
    async fn write_ios_packets_does_not_flush_every_packet() {
        let (tx, rx) = mpsc::channel(4);
        tx.send(test_buffer(&[1, 2, 3])).await.unwrap();
        tx.send(test_buffer(&[4, 5, 6])).await.unwrap();
        drop(tx);

        let mut writer = CountingWriter::default();
        write_ios_packets(&mut writer, ValidatedMtu::new(1280).unwrap(), rx).await;

        assert_eq!(writer.writes, vec![vec![1, 2, 3], vec![4, 5, 6]]);
        assert_eq!(
            writer.flushes, 0,
            "packet forwarding should rely on stream buffering instead of per-packet flushes"
        );
    }

    #[tokio::test]
    async fn write_ios_packets_drops_generated_packets_over_negotiated_mtu() {
        let (tx, rx) = mpsc::channel(2);
        tx.send(OwnedBuffer::zeroed(1281, None).unwrap())
            .await
            .unwrap();
        drop(tx);

        let mut writer = CountingWriter::default();
        write_ios_packets(&mut writer, ValidatedMtu::new(1280).unwrap(), rx).await;

        assert!(writer.writes.is_empty());
    }

    fn ipv6_packet(payload: &[u8]) -> Vec<u8> {
        let mut packet = vec![0u8; IPV6_HEADER_LEN + payload.len()];
        packet[0] = 0x60;
        packet[4..6].copy_from_slice(&(payload.len() as u16).to_be_bytes());
        packet[IPV6_HEADER_LEN..].copy_from_slice(payload);
        packet
    }

    #[tokio::test]
    async fn read_ios_packets_discards_packets_over_negotiated_mtu() {
        let mtu = ValidatedMtu::new(1280).unwrap();
        let payload_capacity = mtu.ipv6_payload_capacity().unwrap();
        let oversized = ipv6_packet(&vec![0xaa; payload_capacity + 1]);
        let expected = ipv6_packet(b"fits");
        let mut input = oversized;
        input.extend_from_slice(&expected);

        let mut reader = tokio::io::BufReader::new(input.as_slice());
        let (tx, mut rx) = mpsc::channel(4);
        read_ios_packets(&mut reader, payload_capacity, tx).await;

        assert_eq!(&*rx.recv().await.unwrap(), expected.as_slice());
        assert!(rx.recv().await.is_none());
    }

    #[test]
    fn channel_device_uses_the_negotiated_mtu() {
        let mtu = ValidatedMtu::new(9000).unwrap();
        let device = ChannelDevice::new(mtu, RuntimeStorage::unmetered());

        assert_eq!(device.capabilities().max_transmission_unit, 9000);
    }

    #[tokio::test]
    async fn flush_channel_queue_preserves_packets_until_capacity_returns() {
        let (tx, mut rx) = mpsc::channel(1);
        tx.send(vec![9]).await.unwrap();

        let mut pending = OwnedQueue::new(None);
        pending.push_back(vec![1, 2, 3]).unwrap();
        pending.push_back(vec![4, 5, 6]).unwrap();
        assert!(!flush_channel_queue(&tx, &mut pending));
        assert_eq!(pending.len(), 2);

        assert_eq!(rx.recv().await.unwrap(), vec![9]);
        assert!(flush_channel_queue(&tx, &mut pending));
        assert_eq!(rx.recv().await.unwrap(), vec![1, 2, 3]);
        assert_eq!(pending.values(), &VecDeque::from([vec![4, 5, 6]]));
    }

    #[test]
    fn take_ephemeral_port_skips_ports_held_by_live_connections() {
        let mut cursor = EPHEMERAL_PORT_MIN;
        let busy = [EPHEMERAL_PORT_MIN, EPHEMERAL_PORT_MIN + 1];

        assert_eq!(
            take_ephemeral_port(&mut cursor, |port| busy.contains(&port)),
            Some(EPHEMERAL_PORT_MIN + 2)
        );
        assert_eq!(cursor, EPHEMERAL_PORT_MIN + 3);
    }

    #[test]
    fn take_ephemeral_port_wraps_inside_the_ephemeral_range() {
        let mut cursor = EPHEMERAL_PORT_MAX;

        assert_eq!(
            take_ephemeral_port(&mut cursor, |_| false),
            Some(EPHEMERAL_PORT_MAX)
        );
        assert_eq!(cursor, EPHEMERAL_PORT_MIN);
        assert_eq!(
            take_ephemeral_port(&mut cursor, |_| false),
            Some(EPHEMERAL_PORT_MIN)
        );
    }

    #[test]
    fn take_ephemeral_port_reports_exhaustion_after_one_lap() {
        let mut cursor = EPHEMERAL_PORT_MIN;

        assert_eq!(take_ephemeral_port(&mut cursor, |_| true), None);
        assert_eq!(cursor, EPHEMERAL_PORT_MIN);
    }

    #[test]
    fn smoltcp_poll_sleep_duration_preserves_idle_smoltcp_backoff() {
        assert_eq!(
            smoltcp_poll_sleep_duration(Some(smoltcp::time::Duration::ZERO), false),
            MIN_POLL_SLEEP
        );
        assert_eq!(
            smoltcp_poll_sleep_duration(Some(smoltcp::time::Duration::from_micros(250)), false),
            Duration::from_micros(250)
        );
        assert_eq!(
            smoltcp_poll_sleep_duration(Some(smoltcp::time::Duration::from_millis(25)), false),
            Duration::from_millis(25)
        );
        assert_eq!(
            smoltcp_poll_sleep_duration(None, false),
            Duration::from_millis(1)
        );
    }

    #[test]
    fn smoltcp_poll_sleep_duration_keeps_active_connections_responsive() {
        assert_eq!(
            smoltcp_poll_sleep_duration(Some(smoltcp::time::Duration::ZERO), true),
            MIN_POLL_SLEEP
        );
        assert_eq!(
            smoltcp_poll_sleep_duration(Some(smoltcp::time::Duration::from_micros(250)), true),
            Duration::from_micros(250)
        );
        assert_eq!(
            smoltcp_poll_sleep_duration(Some(smoltcp::time::Duration::from_millis(25)), true),
            Duration::from_millis(1)
        );
        assert_eq!(
            smoltcp_poll_sleep_duration(None, true),
            Duration::from_millis(1)
        );
    }

    fn proxy_preamble(address: [u8; 16], port: u32) -> [u8; LOCAL_PROXY_PREAMBLE_LEN] {
        let mut preamble = [0u8; LOCAL_PROXY_PREAMBLE_LEN];
        preamble[..16].copy_from_slice(&address);
        preamble[16..].copy_from_slice(&port.to_le_bytes());
        preamble
    }

    async fn assert_proxy_client_closed(client: &mut tokio::net::TcpStream) {
        let mut byte = [0u8; 1];
        let result = tokio::time::timeout(Duration::from_millis(300), client.read(&mut byte))
            .await
            .expect("proxy client should be closed promptly");
        assert!(
            matches!(result, Ok(0) | Err(_)),
            "proxy should close or reset the client, got {result:?}"
        );
    }

    #[tokio::test]
    async fn stalled_first_client_does_not_block_second_preamble() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let (request_tx, mut request_rx) = mpsc::channel(4);
        let listener_task = tokio::spawn(run_local_listener_with_limit(
            listener,
            request_tx,
            2,
            Duration::from_secs(1),
        ));

        let mut stalled = tokio::net::TcpStream::connect(address).await.unwrap();
        stalled.write_all(&[0xaa]).await.unwrap();
        // Let the listener accept the first connection so this test exercises
        // a client task blocked in its preamble read, rather than a backlog
        // that has not been serviced yet.
        tokio::time::sleep(Duration::from_millis(10)).await;
        let mut second = tokio::net::TcpStream::connect(address).await.unwrap();
        second
            .write_all(&proxy_preamble(
                [0x20, 1, 0xdb, 0x8, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 1],
                62078,
            ))
            .await
            .unwrap();

        let request = tokio::time::timeout(Duration::from_millis(300), request_rx.recv())
            .await
            .expect("second preamble should be accepted while first is stalled")
            .expect("listener request channel should remain open");
        assert_eq!(
            request.remote_ip,
            Ipv6Address::from_bytes(&[0x20, 1, 0xdb, 0x8, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 1,])
        );
        assert_eq!(request.remote_port, 62078);

        drop(request);
        drop(second);
        drop(stalled);
        listener_task.abort();
        let _ = listener_task.await;
    }

    #[tokio::test]
    async fn partial_preamble_is_closed_after_bounded_timeout() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let (request_tx, mut request_rx) = mpsc::channel(4);
        let listener_task = tokio::spawn(run_local_listener_with_limit(
            listener,
            request_tx,
            2,
            Duration::from_millis(20),
        ));

        let mut client = tokio::net::TcpStream::connect(address).await.unwrap();
        client.write_all(&[0xbb; 8]).await.unwrap();
        assert_proxy_client_closed(&mut client).await;
        assert!(request_rx.try_recv().is_err());

        drop(client);
        listener_task.abort();
        let _ = listener_task.await;
    }

    #[tokio::test]
    async fn proxy_limit_rejects_excess_client_without_a_task() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let (request_tx, mut request_rx) = mpsc::channel(4);
        let listener_task = tokio::spawn(run_local_listener_with_limit(
            listener,
            request_tx,
            1,
            Duration::from_secs(1),
        ));

        let mut first = tokio::net::TcpStream::connect(address).await.unwrap();
        first
            .write_all(&proxy_preamble([0u8; 16], 1))
            .await
            .unwrap();
        let request = tokio::time::timeout(Duration::from_millis(300), request_rx.recv())
            .await
            .expect("first client should be accepted")
            .expect("listener request channel should remain open");

        let mut excess = tokio::net::TcpStream::connect(address).await.unwrap();
        excess
            .write_all(&proxy_preamble([0u8; 16], 2))
            .await
            .unwrap();
        assert_proxy_client_closed(&mut excess).await;

        drop(request);
        drop(excess);
        drop(first);
        listener_task.abort();
        let _ = listener_task.await;
    }

    #[tokio::test]
    async fn listener_shutdown_aborts_stalled_client_tasks() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let (request_tx, _request_rx) = mpsc::channel(4);
        let listener_task = tokio::spawn(run_local_listener_with_limit(
            listener,
            request_tx,
            1,
            Duration::from_secs(10),
        ));

        let mut client = tokio::net::TcpStream::connect(address).await.unwrap();
        client.write_all(&[0xcc]).await.unwrap();
        tokio::time::sleep(Duration::from_millis(10)).await;
        let mut excess = tokio::net::TcpStream::connect(address).await.unwrap();
        excess.write_all(&[0xdd]).await.unwrap();
        assert_proxy_client_closed(&mut excess).await;
        listener_task.abort();
        let _ = listener_task.await;
        assert_proxy_client_closed(&mut client).await;
    }
}
