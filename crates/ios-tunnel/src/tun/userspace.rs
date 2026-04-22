//! Userspace TUN device via smoltcp.
//!
//! Implements a go-ios-compatible userspace TCP/IP stack that:
//! - Accepts local TCP connections on 127.0.0.1:random_port
//! - Forwards them through smoltcp → CDTunnel IPv6 stream
//! - Compatible with go-ios local proxy protocol:
//!   client sends 16-byte IPv6 addr + 4-byte LE port after connecting

use std::collections::{HashMap, VecDeque};
use std::time::Duration;

use smoltcp::iface::{Config, Interface, SocketHandle, SocketSet};
use smoltcp::phy::{Device, DeviceCapabilities, Medium};
use smoltcp::socket::tcp;
use smoltcp::time::Instant as SmolInstant;
use smoltcp::wire::{HardwareAddress, IpAddress, IpCidr, IpEndpoint, Ipv6Address};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use tokio::sync::mpsc;
use tokio::task::JoinHandle;

use crate::TunnelError;

const PREFIX_LEN: u8 = 64;
const IOS_PACKET_CHANNEL_CAPACITY: usize = 8192;
const LOCAL_PROXY_CHANNEL_CAPACITY: usize = 4096;
const SOCKET_BUFFER_BYTES: usize = 1_048_576;
const SOCKET_RECV_CHUNK_BYTES: usize = 65_536;

// ── smoltcp Device implementation ─────────────────────────────────────────────

struct ChannelDevice {
    rx_buf: VecDeque<Vec<u8>>,
    tx_buf: VecDeque<Vec<u8>>,
}

struct SmolRxToken(Vec<u8>);
struct SmolTxToken<'a>(&'a mut VecDeque<Vec<u8>>);

impl smoltcp::phy::RxToken for SmolRxToken {
    fn consume<R, F: FnOnce(&mut [u8]) -> R>(mut self, f: F) -> R {
        f(&mut self.0)
    }
}

impl smoltcp::phy::TxToken for SmolTxToken<'_> {
    fn consume<R, F: FnOnce(&mut [u8]) -> R>(self, len: usize, f: F) -> R {
        let mut buf = vec![0u8; len];
        let r = f(&mut buf);
        self.0.push_back(buf);
        r
    }
}

impl Device for ChannelDevice {
    type RxToken<'a> = SmolRxToken;
    type TxToken<'a> = SmolTxToken<'a>;

    fn receive(&mut self, _ts: SmolInstant) -> Option<(Self::RxToken<'_>, Self::TxToken<'_>)> {
        self.rx_buf
            .pop_front()
            .map(|pkt| (SmolRxToken(pkt), SmolTxToken(&mut self.tx_buf)))
    }

    fn transmit(&mut self, _ts: SmolInstant) -> Option<Self::TxToken<'_>> {
        Some(SmolTxToken(&mut self.tx_buf))
    }

    fn capabilities(&self) -> DeviceCapabilities {
        let mut caps = DeviceCapabilities::default();
        caps.medium = Medium::Ip;
        caps.max_transmission_unit = 1280;
        caps
    }
}

// ── Connection request ─────────────────────────────────────────────────────────

struct ConnRequest {
    remote_ip: Ipv6Address,
    remote_port: u16,
    from_client: mpsc::Receiver<Vec<u8>>,
    to_client: mpsc::Sender<Vec<u8>>,
    connected: tokio::sync::oneshot::Sender<()>,
}

struct ConnState {
    from_client: mpsc::Receiver<Vec<u8>>,
    to_client: mpsc::Sender<Vec<u8>>,
    connected: Option<tokio::sync::oneshot::Sender<()>>,
    pending_to_client: VecDeque<Vec<u8>>,
    pending_from_client: VecDeque<Vec<u8>>,
}

// ── Main struct ────────────────────────────────────────────────────────────────

/// Userspace TUN device using smoltcp + local TCP proxy.
pub struct UserspaceTunDevice {
    pub local_port: u16,
    task_handles: Vec<JoinHandle<()>>,
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
        let client_ipv6: Ipv6Address = client_address.parse().map_err(|_| {
            TunnelError::Protocol(format!("invalid client IPv6 address: {client_address}"))
        })?;

        let (ios_to_smol_tx, ios_to_smol_rx) =
            mpsc::channel::<Vec<u8>>(IOS_PACKET_CHANNEL_CAPACITY);
        let (smol_to_ios_tx, smol_to_ios_rx) =
            mpsc::channel::<Vec<u8>>(IOS_PACKET_CHANNEL_CAPACITY);
        let (conn_req_tx, conn_req_rx) = mpsc::channel::<ConnRequest>(32);

        let (mut ios_reader, mut ios_writer) = tokio::io::split(ios_stream);

        let reader_task = tokio::spawn(async move {
            read_ios_packets(&mut ios_reader, ios_to_smol_tx).await;
            tracing::debug!("userspace: iOS packet reader exited");
        });

        let writer_task = tokio::spawn(async move {
            write_ios_packets(&mut ios_writer, smol_to_ios_rx).await;
            tracing::debug!("userspace: iOS packet writer exited");
        });

        let smoltcp_task = tokio::spawn(async move {
            run_smoltcp(
                client_ipv6,
                mtu,
                ios_to_smol_rx,
                smol_to_ios_tx,
                conn_req_rx,
            )
            .await;
            tracing::debug!("userspace: smoltcp loop exited");
        });

        let listener = TcpListener::bind("127.0.0.1:0").await?;
        let local_port = listener.local_addr()?.port();

        let listener_task = tokio::spawn(async move {
            run_local_listener(listener, conn_req_tx).await;
            tracing::debug!("userspace: local listener exited");
        });

        tracing::info!("userspace TUN started on 127.0.0.1:{local_port}");
        Ok(Self {
            local_port,
            task_handles: vec![reader_task, writer_task, smoltcp_task, listener_task],
        })
    }

    pub fn is_alive(&self) -> bool {
        self.task_handles.iter().all(|handle| !handle.is_finished())
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

async fn read_ios_packets<R>(reader: &mut R, tx: mpsc::Sender<Vec<u8>>)
where
    R: tokio::io::AsyncRead + Unpin,
{
    let mut hdr = vec![0u8; 40];
    loop {
        if reader.read_exact(&mut hdr).await.is_err() {
            break;
        }
        if hdr[0] >> 4 != 6 {
            tracing::warn!("userspace: non-IPv6 packet from iOS, skipping");
            continue;
        }
        let payload_len = u16::from_be_bytes([hdr[4], hdr[5]]) as usize;
        let mut pkt = hdr.clone();
        pkt.resize(40 + payload_len, 0);
        if reader.read_exact(&mut pkt[40..]).await.is_err() {
            break;
        }
        if tx.send(pkt).await.is_err() {
            break;
        }
    }
}

async fn write_ios_packets<W>(writer: &mut W, mut rx: mpsc::Receiver<Vec<u8>>)
where
    W: tokio::io::AsyncWrite + Unpin,
{
    while let Some(pkt) = rx.recv().await {
        if writer.write_all(&pkt).await.is_err() {
            break;
        }
    }
}

// ── smoltcp event loop ─────────────────────────────────────────────────────────

fn smol_now() -> SmolInstant {
    SmolInstant::from_millis(
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as i64,
    )
}

fn smoltcp_poll_sleep_duration(
    delay: Option<smoltcp::time::Duration>,
    has_active_connections: bool,
) -> Duration {
    // Active tunnel sockets need a short poll interval so inbound packets do not sit
    // in the channel queue long enough to starve real-time streams like kperf/fps.
    if has_active_connections {
        return delay
            .map(|delay| Duration::from_micros(delay.total_micros()).min(Duration::from_millis(1)))
            .unwrap_or_else(|| Duration::from_millis(1));
    }

    // `None` means there is no internal timer deadline, but this loop still needs
    // to wake up periodically to observe channel activity.
    delay
        .map(|delay| Duration::from_micros(delay.total_micros()))
        .unwrap_or_else(|| Duration::from_millis(1))
}

async fn run_smoltcp(
    client_ipv6: Ipv6Address,
    _mtu: u32,
    mut ios_rx: mpsc::Receiver<Vec<u8>>,
    ios_tx: mpsc::Sender<Vec<u8>>,
    mut conn_req_rx: mpsc::Receiver<ConnRequest>,
) {
    let mut device = ChannelDevice {
        rx_buf: VecDeque::new(),
        tx_buf: VecDeque::new(),
    };
    let config = Config::new(HardwareAddress::Ip);
    let mut iface = Interface::new(config, &mut device, smol_now());
    iface.update_ip_addrs(|addrs| {
        let _ = addrs.push(IpCidr::new(IpAddress::Ipv6(client_ipv6), PREFIX_LEN));
    });

    let mut sockets = SocketSet::new(vec![]);
    let mut connections: HashMap<SocketHandle, ConnState> = HashMap::new();
    let mut ephemeral_port: u16 = 40000;
    let mut pending_ios_tx = VecDeque::new();

    loop {
        let mut made_progress = false;

        while let Ok(pkt) = ios_rx.try_recv() {
            tracing::trace!("userspace: rx packet {} bytes from iOS", pkt.len());
            device.rx_buf.push_back(pkt);
            made_progress = true;
        }

        while let Ok(req) = conn_req_rx.try_recv() {
            tracing::info!(
                "userspace: new conn request to [{:?}]:{}",
                req.remote_ip,
                req.remote_port
            );
            let rx_buf = tcp::SocketBuffer::new(vec![0u8; SOCKET_BUFFER_BYTES]);
            let tx_buf = tcp::SocketBuffer::new(vec![0u8; SOCKET_BUFFER_BYTES]);
            let mut socket = tcp::Socket::new(rx_buf, tx_buf);
            socket.set_timeout(Some(smoltcp::time::Duration::from_secs(30)));
            socket.set_keep_alive(Some(smoltcp::time::Duration::from_secs(1)));
            let local_port = ephemeral_port;
            ephemeral_port = ephemeral_port.wrapping_add(1).max(40000);
            let remote_ep = IpEndpoint::new(IpAddress::Ipv6(req.remote_ip), req.remote_port);
            match socket.connect(iface.context(), remote_ep, local_port) {
                Ok(()) => {
                    tracing::debug!("userspace: socket connected state={:?}", socket.state());
                    let handle = sockets.add(socket);
                    connections.insert(
                        handle,
                        ConnState {
                            from_client: req.from_client,
                            to_client: req.to_client,
                            connected: Some(req.connected),
                            pending_to_client: VecDeque::new(),
                            pending_from_client: VecDeque::new(),
                        },
                    );
                    made_progress = true;
                }
                Err(e) => {
                    tracing::error!("userspace: smoltcp connect failed: {e:?}");
                }
            }
        }

        iface.poll(smol_now(), &mut device, &mut sockets);

        while let Some(pkt) = device.tx_buf.pop_front() {
            tracing::trace!(
                "userspace: tx packet {} bytes to iOS, src={:?} dst={:?}",
                pkt.len(),
                pkt.get(8..24).map(|b| format!("{:02x?}", b)),
                pkt.get(24..40).map(|b| format!("{:02x?}", b))
            );
            match ios_tx.try_send(pkt) {
                Ok(()) => {
                    made_progress = true;
                }
                Err(mpsc::error::TrySendError::Full(pkt)) => {
                    pending_ios_tx.push_back(pkt);
                    break;
                }
                Err(mpsc::error::TrySendError::Closed(_)) => {
                    tracing::debug!("userspace: iOS packet writer closed");
                    return;
                }
            }
        }
        made_progress |= flush_channel_queue(&ios_tx, &mut pending_ios_tx);

        let mut to_remove = Vec::new();
        for (&handle, state) in &mut connections {
            let socket = sockets.get_mut::<tcp::Socket>(handle);
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
            made_progress |= flush_channel_queue(&state.to_client, &mut state.pending_to_client);
            if state.pending_to_client.is_empty() {
                while socket.can_recv() {
                    let mut buf = vec![0u8; SOCKET_RECV_CHUNK_BYTES];
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
                                    state.pending_to_client.push_back(buf);
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
                    state.pending_from_client.push_back(data);
                    made_progress |= flush_socket_send(socket, &mut state.pending_from_client);
                    if !state.pending_from_client.is_empty() || !socket.can_send() {
                        break;
                    }
                }
            }
            if !socket.is_open() {
                tracing::debug!("userspace: socket closed state={:?}", socket.state());
                to_remove.push(handle);
            }
        }
        for h in to_remove {
            connections.remove(&h);
            sockets.remove(h);
        }

        if made_progress {
            tokio::task::yield_now().await;
            continue;
        }

        tokio::time::sleep(smoltcp_poll_sleep_duration(
            iface.poll_delay(smol_now(), &sockets),
            !connections.is_empty(),
        ))
        .await;
    }
}

fn flush_channel_queue(tx: &mpsc::Sender<Vec<u8>>, pending: &mut VecDeque<Vec<u8>>) -> bool {
    let mut made_progress = false;
    while let Some(buf) = pending.pop_front() {
        match tx.try_send(buf) {
            Ok(()) => {
                made_progress = true;
            }
            Err(mpsc::error::TrySendError::Full(buf)) => {
                pending.push_front(buf);
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

fn flush_socket_send(socket: &mut tcp::Socket, pending: &mut VecDeque<Vec<u8>>) -> bool {
    let mut made_progress = false;
    while socket.can_send() {
        let Some(buf) = pending.pop_front() else {
            break;
        };
        match socket.send_slice(&buf) {
            Ok(sent) if sent == buf.len() => {
                made_progress = true;
            }
            Ok(sent) => {
                if sent < buf.len() {
                    pending.push_front(buf[sent..].to_vec());
                }
                made_progress |= sent > 0;
                break;
            }
            Err(_) => {
                pending.push_front(buf);
                break;
            }
        }
    }
    made_progress
}

// ── Local TCP listener (go-ios compatible protocol) ────────────────────────────

async fn run_local_listener(listener: TcpListener, conn_req_tx: mpsc::Sender<ConnRequest>) {
    loop {
        let Ok((mut client, peer)) = listener.accept().await else {
            break;
        };
        tracing::info!("userspace: local connection from {peer}");

        let mut addr_buf = [0u8; 16];
        if client.read_exact(&mut addr_buf).await.is_err() {
            continue;
        }
        let mut port_buf = [0u8; 4];
        if client.read_exact(&mut port_buf).await.is_err() {
            continue;
        }
        let remote_port = u32::from_le_bytes(port_buf) as u16;
        let remote_ip = Ipv6Address::from_bytes(&addr_buf);

        tracing::info!("userspace: tunneling to [{remote_ip}]:{remote_port}");

        let (from_client_tx, from_client_rx) =
            mpsc::channel::<Vec<u8>>(LOCAL_PROXY_CHANNEL_CAPACITY);
        let (to_client_tx, to_client_rx) = mpsc::channel::<Vec<u8>>(LOCAL_PROXY_CHANNEL_CAPACITY);
        let (connected_tx, connected_rx) = tokio::sync::oneshot::channel::<()>();

        let req = ConnRequest {
            remote_ip,
            remote_port,
            from_client: from_client_rx,
            to_client: to_client_tx,
            connected: connected_tx,
        };
        if conn_req_tx.send(req).await.is_err() {
            break;
        }

        tokio::spawn(async move {
            proxy_local_client(client, from_client_tx, to_client_rx, connected_rx).await;
        });
    }
}

async fn proxy_local_client(
    client: tokio::net::TcpStream,
    to_smoltcp: mpsc::Sender<Vec<u8>>,
    mut from_smoltcp: mpsc::Receiver<Vec<u8>>,
    connected: tokio::sync::oneshot::Receiver<()>,
) {
    // Wait for TCP connection to be established before forwarding data
    // This mirrors gVisor's behavior of waiting for Connect() to complete
    if connected.await.is_err() {
        tracing::debug!("userspace: connection aborted before established");
        return;
    }

    let (mut r, mut w) = client.into_split();
    let read_half = async {
        let mut buf = vec![0u8; 4096];
        loop {
            match r.read(&mut buf).await {
                Ok(0) | Err(_) => break,
                Ok(n) => {
                    if to_smoltcp.send(buf[..n].to_vec()).await.is_err() {
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
mod tests {
    use std::pin::Pin;
    use std::task::{Context, Poll};

    use tokio::io::AsyncWrite;

    use super::*;

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
        tx.send(vec![1, 2, 3]).await.unwrap();
        tx.send(vec![4, 5, 6]).await.unwrap();
        drop(tx);

        let mut writer = CountingWriter::default();
        write_ios_packets(&mut writer, rx).await;

        assert_eq!(writer.writes, vec![vec![1, 2, 3], vec![4, 5, 6]]);
        assert_eq!(
            writer.flushes, 0,
            "packet forwarding should rely on stream buffering instead of per-packet flushes"
        );
    }

    #[tokio::test]
    async fn flush_channel_queue_preserves_packets_until_capacity_returns() {
        let (tx, mut rx) = mpsc::channel(1);
        tx.send(vec![9]).await.unwrap();

        let mut pending = VecDeque::from([vec![1, 2, 3], vec![4, 5, 6]]);
        assert!(!flush_channel_queue(&tx, &mut pending));
        assert_eq!(pending.len(), 2);

        assert_eq!(rx.recv().await.unwrap(), vec![9]);
        assert!(flush_channel_queue(&tx, &mut pending));
        assert_eq!(rx.recv().await.unwrap(), vec![1, 2, 3]);
        assert_eq!(pending, VecDeque::from([vec![4, 5, 6]]));
    }

    #[test]
    fn smoltcp_poll_sleep_duration_preserves_idle_smoltcp_backoff() {
        assert_eq!(
            smoltcp_poll_sleep_duration(Some(smoltcp::time::Duration::ZERO), false),
            Duration::ZERO
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
            Duration::ZERO
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
}
