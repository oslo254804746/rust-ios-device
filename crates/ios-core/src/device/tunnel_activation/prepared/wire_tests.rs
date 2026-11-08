//! Real private IPv6/TCP/H2 preparation; only the authenticated peer is scripted.
use super::*;
use crate::xpc::message::{decode_message, encode_message, flags};
use crate::{TunnelOwner, TunnelReservation, XpcMessage, XpcValue};
use bytes::Bytes;
use smoltcp::{
    iface::{Config, Interface, SocketSet},
    phy::{Device, DeviceCapabilities, Medium, RxToken, TxToken},
    socket::tcp,
    time::Instant as SmolInstant,
    wire::{HardwareAddress, IpAddress, IpCidr, Ipv6Address},
};
use std::{
    collections::VecDeque,
    future::pending,
    io,
    sync::{
        atomic::{AtomicUsize, Ordering},
        Arc, Mutex,
    },
    time::Duration,
};
use tokio::{
    io::{AsyncRead, AsyncReadExt, AsyncWriteExt, DuplexStream},
    sync::{Notify, Semaphore},
    time::Instant,
};

const UID: &str = "prepared-wire-real-private-device";
const OLD_SERVICE: &str = "wire.initial.only";
const NEW_SERVICE: &str = "wire.checked.only";
const TOTAL: usize = 32 * 1024 * 1024;
const OBSERVE: Duration = Duration::from_secs(8);

#[cfg(all(feature = "tunnel-kernel", target_os = "windows"))]
mod kernel_tests;

fn dict(items: impl IntoIterator<Item = (&'static str, XpcValue)>) -> XpcValue {
    XpcValue::Dictionary(items.into_iter().map(|(k, v)| (k.into(), v)).collect())
}
fn h2(kind: u8, stream: u32, body: &[u8]) -> Vec<u8> {
    let n = body.len();
    let mut frame = vec![(n >> 16) as u8, (n >> 8) as u8, n as u8, kind, 0];
    frame.extend_from_slice(&stream.to_be_bytes());
    frame.extend_from_slice(body);
    frame
}
fn response(uid: &str, service: &'static str, port: u16) -> Vec<u8> {
    let message = encode_message(&XpcMessage {
        flags: flags::ALWAYS_SET | flags::DATA,
        msg_id: 0,
        body: Some(dict([
            ("MessageType", XpcValue::String("Handshake".into())),
            (
                "Properties",
                dict([("UniqueDeviceID", XpcValue::String(uid.into()))]),
            ),
            (
                "Services",
                dict([(
                    service,
                    dict([
                        ("Port", XpcValue::Uint64(u64::from(port))),
                        (
                            "Properties",
                            dict([(
                                "Features",
                                XpcValue::Array(vec![XpcValue::String("wire-feature".into())]),
                            )]),
                        ),
                    ]),
                )]),
            ),
        ])),
    })
    .unwrap();
    let mut wire = h2(4, 0, &[]);
    wire.extend_from_slice(&h2(0, 1, &message));
    wire
}

#[derive(Default)]
struct PeerDevice {
    incoming: VecDeque<Vec<u8>>,
    outgoing: VecDeque<Vec<u8>>,
}
struct Received(Vec<u8>);
struct Transmit<'a>(&'a mut VecDeque<Vec<u8>>);
impl RxToken for Received {
    fn consume<R, F: FnOnce(&mut [u8]) -> R>(mut self, f: F) -> R {
        f(&mut self.0)
    }
}
impl TxToken for Transmit<'_> {
    fn consume<R, F: FnOnce(&mut [u8]) -> R>(self, len: usize, f: F) -> R {
        let mut packet = vec![0; len];
        let result = f(&mut packet);
        self.0.push_back(packet);
        result
    }
}
impl Device for PeerDevice {
    type RxToken<'a> = Received;
    type TxToken<'a> = Transmit<'a>;
    fn receive(&mut self, _: SmolInstant) -> Option<(Received, Transmit<'_>)> {
        let packet = self.incoming.pop_front()?;
        Some((Received(packet), Transmit(&mut self.outgoing)))
    }
    fn transmit(&mut self, _: SmolInstant) -> Option<Transmit<'_>> {
        Some(Transmit(&mut self.outgoing))
    }
    fn capabilities(&self) -> DeviceCapabilities {
        let mut caps = DeviceCapabilities::default();
        caps.medium = Medium::Ip;
        caps.max_transmission_unit = 1280;
        caps
    }
}

/// Cancellation leaves every consumed header/payload byte in this owner.
/// A test-only pause deliberately drops next() after a genuine short header.
struct Frames {
    header: [u8; 40],
    header_read: usize,
    packet: Vec<u8>,
    payload_read: usize,
    pause: Arc<Notify>,
}
impl Frames {
    fn new(pause: Arc<Notify>) -> Self {
        Self {
            header: [0; 40],
            header_read: 0,
            packet: Vec::new(),
            payload_read: 0,
            pause,
        }
    }
    async fn next<R: AsyncRead + Unpin>(&mut self, reader: &mut R) -> io::Result<Option<Vec<u8>>> {
        while self.header_read < self.header.len() {
            let end = (self.header_read + 7).min(self.header.len());
            let n = reader.read(&mut self.header[self.header_read..end]).await?;
            if n == 0 {
                return Ok(None);
            } // The actual underlying read returned Ok(0).
            self.header_read += n;
            if self.header_read == 7 {
                self.pause.notify_one();
                tokio::task::yield_now().await;
            }
        }
        if self.packet.is_empty() {
            assert_eq!(self.header[0] >> 4, 6);
            let payload = usize::from(u16::from_be_bytes([self.header[4], self.header[5]]));
            self.packet.extend_from_slice(&self.header);
            self.packet.resize(40 + payload, 0);
        }
        while 40 + self.payload_read < self.packet.len() {
            let start = 40 + self.payload_read;
            let end = (start + 11).min(self.packet.len());
            let n = reader.read(&mut self.packet[start..end]).await?;
            if n == 0 {
                return Ok(None);
            }
            self.payload_read += n;
        }
        let packet = std::mem::take(&mut self.packet);
        self.header_read = 0;
        self.payload_read = 0;
        Ok(Some(packet))
    }
}

#[derive(Default, Debug)]
struct Connection {
    request: Vec<u8>,
    parsed: usize,
    uuid: Option<[u8; 16]>,
    port: Option<u16>,
    established: bool,
    sent: usize,
}
impl Connection {
    fn received(&mut self, bytes: &[u8]) {
        self.request.extend_from_slice(bytes);
        if self.request.len() < crate::xpc::h2_raw::H2_PREFACE.len() {
            return;
        }
        if self.parsed == 0 {
            assert_eq!(&self.request[..24], crate::xpc::h2_raw::H2_PREFACE);
            self.parsed = 24;
        }
        loop {
            if self.request.len() < self.parsed + 9 {
                break;
            }
            let header = &self.request[self.parsed..self.parsed + 9];
            let n = (usize::from(header[0]) << 16)
                | (usize::from(header[1]) << 8)
                | usize::from(header[2]);
            if self.request.len() < self.parsed + 9 + n {
                break;
            }
            if header[3] == 0 && u32::from_be_bytes(header[5..9].try_into().unwrap()) == 1 {
                let bytes =
                    Bytes::copy_from_slice(&self.request[self.parsed + 9..self.parsed + 9 + n]);
                let message = decode_message(bytes).unwrap();
                if let Some(body) = message.body.as_ref().and_then(XpcValue::as_dict) {
                    if body.get("MessageType").and_then(XpcValue::as_str) == Some("Handshake") {
                        let XpcValue::Uuid(uuid) = &body["UUID"] else {
                            panic!("real host identity missing")
                        };
                        self.uuid = Some(*uuid);
                    }
                }
            }
            self.parsed += 9 + n;
        }
    }
}

#[derive(Debug)]
struct PeerReport {
    connections: [Connection; 2],
    cancelled_headers: usize,
    literal_eof: bool,
}

async fn remote(mut peer: DuplexStream, checked_uid: &'static str) -> PeerReport {
    // This one awaited CDTunnel exchange is never cancelled by the packet tick.
    let mut header = [0; 10];
    peer.read_exact(&mut header).await.unwrap();
    assert_eq!(&header[..8], b"CDTunnel");
    let mut request = vec![0; usize::from(u16::from_be_bytes([header[8], header[9]]))];
    peer.read_exact(&mut request).await.unwrap();
    let handshake = serde_json::to_vec(&serde_json::json!({
        "serverAddress": "fd00::1", "serverRSDPort": 58783,
        "clientParameters": { "address": "fd00::2", "mtu": 1280 },
    }))
    .unwrap();
    peer.write_all(b"CDTunnel").await.unwrap();
    peer.write_all(&(handshake.len() as u16).to_be_bytes())
        .await
        .unwrap();
    peer.write_all(&handshake).await.unwrap();

    let mut device = PeerDevice::default();
    let origin = std::time::Instant::now();
    let mut interface = Interface::new(
        Config::new(HardwareAddress::Ip),
        &mut device,
        SmolInstant::from_millis(0),
    );
    interface.update_ip_addrs(|addresses| {
        addresses
            .push(IpCidr::new(
                IpAddress::Ipv6(Ipv6Address::new(0xfd00, 0, 0, 0, 0, 0, 0, 1)),
                64,
            ))
            .unwrap();
    });
    let mut sockets = SocketSet::new(Vec::new());
    let handles = std::array::from_fn::<_, 2, _>(|_| {
        let mut socket = tcp::Socket::new(
            tcp::SocketBuffer::new(vec![0; 64 * 1024]),
            tcp::SocketBuffer::new(vec![0; 64 * 1024]),
        );
        socket.set_ack_delay(None);
        socket.listen(58783).unwrap();
        sockets.add(socket)
    });
    let replies = [
        response(UID, OLD_SERVICE, 12345),
        response(checked_uid, NEW_SERVICE, 23456),
    ];
    let mut connections: [Connection; 2] = std::array::from_fn(|_| Connection::default());
    let pause = Arc::new(Notify::new());
    let mut frames = Frames::new(pause.clone());
    let (mut reader, mut writer) = tokio::io::split(peer);
    let mut writing: Option<(Vec<u8>, usize)> = None;
    let mut writer_closed = false;
    let mut tick = tokio::time::interval(Duration::from_millis(1));
    tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    let mut cancelled_headers = 0;

    loop {
        let now = SmolInstant::from_millis(i64::try_from(origin.elapsed().as_millis()).unwrap());
        interface.poll(now, &mut device, &mut sockets);
        for (index, handle) in handles.iter().copied().enumerate() {
            let socket = sockets.get_mut::<tcp::Socket>(handle);
            if socket.state() == tcp::State::Established {
                connections[index].established = true;
                connections[index].port = Some(socket.remote_endpoint().unwrap().port);
            }
            let mut buffer = [0; 2048];
            while socket.can_recv() {
                let n = socket.recv_slice(&mut buffer).unwrap();
                if n == 0 {
                    break;
                }
                connections[index].received(&buffer[..n]);
            }
            // Real SETTINGS unblock the real bootstrap; the directory is sent
            // only after this connection's actual host Handshake was received.
            let allowed = if connections[index].uuid.is_some() {
                replies[index].len()
            } else {
                9
            };
            if socket.can_send() && connections[index].sent < allowed {
                let n = socket
                    .send_slice(&replies[index][connections[index].sent..allowed])
                    .unwrap();
                connections[index].sent += n;
            }
            if socket.state() == tcp::State::CloseWait {
                socket.close();
            }
        }
        interface.poll(now, &mut device, &mut sockets);
        if writing.is_none() {
            writing = device.outgoing.pop_front().map(|packet| (packet, 0));
        }

        enum Event {
            Packet(io::Result<Option<Vec<u8>>>),
            Written(io::Result<usize>),
            Pause,
            Tick,
        }
        let event = tokio::select! {
            biased;
            packet = frames.next(&mut reader) => Event::Packet(packet),
            _ = pause.notified() => Event::Pause,
            result = async {
                match &writing {
                    Some((packet, offset)) if !writer_closed => writer.write(&packet[*offset..]).await,
                    _ => pending().await,
                }
            } => Event::Written(result),
            _ = tick.tick() => Event::Tick,
        };
        match event {
            Event::Packet(Ok(Some(packet))) => device.incoming.push_back(packet),
            Event::Packet(Ok(None)) => {
                return PeerReport {
                    connections,
                    cancelled_headers,
                    literal_eof: true,
                }
            }
            Event::Packet(Err(error)) => panic!("packet read lost original framing: {error}"),
            Event::Written(Ok(0)) | Event::Written(Err(_)) => {
                writer_closed = true;
                writing = None;
            }
            Event::Written(Ok(n)) => {
                let (packet, offset) = writing.as_mut().unwrap();
                *offset += n;
                if *offset == packet.len() {
                    writing = None;
                }
            }
            Event::Pause => {
                if frames.header_read != 0 {
                    cancelled_headers += 1;
                }
            }
            Event::Tick => {}
        }
    }
}

type DormantAttachment = (bool, bool, Option<TunMode>, bool);
struct Owner {
    bank: Arc<Semaphore>,
    mode: TunMode,
    attached: Mutex<Option<Arc<TunnelHandle>>>,
    completions: AtomicUsize,
    attached_dormant: Mutex<Option<DormantAttachment>>,
}
struct Reservation<'a> {
    owner: &'a Owner,
    completed: bool,
}
impl TunnelOwner for Owner {
    fn reserve(&self) -> Result<Box<dyn TunnelReservation + '_>, CoreError> {
        Ok(Box::new(Reservation {
            owner: self,
            completed: false,
        }))
    }
}
impl TunnelReservation for Reservation<'_> {
    fn byte_bank(&self) -> Option<Arc<Semaphore>> {
        Some(self.owner.bank.clone())
    }
    fn attach(&mut self, handle: Arc<TunnelHandle>) -> Result<(), CoreError> {
        *self.owner.attached_dormant.lock().unwrap() = Some((
            handle.is_alive(),
            handle.ready_for_publication(),
            handle.prepared_mode(),
            handle.userspace_port.is_none(),
        ));
        *self.owner.attached.lock().unwrap() = Some(handle);
        Ok(())
    }
    fn complete(&mut self) {
        self.completed = true;
        self.owner.completions.fetch_add(1, Ordering::SeqCst);
    }
}
impl Drop for Reservation<'_> {
    fn drop(&mut self) {
        if !self.completed {
            if let Some(handle) = self.owner.attached.lock().unwrap().as_ref() {
                handle.request_stop();
            }
        }
    }
}
fn connection(bank: &Arc<Semaphore>) -> TunnelConnection {
    TunnelConnection::new(
        DeviceInfo {
            udid: UID.into(),
            device_id: 0,
            connection_type: "Network".into(),
            product_id: 0,
        },
        None,
        LockdownTransport::Tcp { host: "::1".into() },
        "real wire private peer",
    )
    .with_initial_rsd_bank(Some(bank))
}
fn modes() -> Vec<TunMode> {
    vec![
        #[cfg(feature = "tunnel-kernel")]
        TunMode::Kernel,
        #[cfg(feature = "tunnel-userspace")]
        TunMode::Userspace,
    ]
}

#[cfg(feature = "tunnel-userspace")]
#[tokio::test]
async fn real_prepared_user_cannot_extend_original_clock_when_promoting_listener() {
    let baseline = tokio::runtime::Handle::current()
        .metrics()
        .num_alive_tasks();
    let bank = Arc::new(Semaphore::new(TOTAL));
    let owner = Owner {
        bank: bank.clone(),
        mode: TunMode::Userspace,
        attached: Mutex::new(None),
        completions: AtomicUsize::new(0),
        attached_dormant: Mutex::new(None),
    };
    let (stream, peer) = tokio::io::duplex(97);
    let peer_task = tokio::spawn(remote(peer, UID));
    let deadline = Instant::now() + OBSERVE;
    let prepared = prepare_tunnel(
        connection(&bank),
        stream,
        TunMode::Userspace,
        Some(owner.reserve_owned().unwrap()),
        Prepared {
            deadline,
            bank: &bank,
        },
    )
    .await
    .unwrap();
    let handle = prepared.tunnel_handle().clone();
    assert_eq!(prepared.deadline(), deadline);
    assert!(!handle.ready_for_publication());
    tokio::time::pause();
    tokio::time::advance(
        deadline.saturating_duration_since(Instant::now()) + Duration::from_secs(1),
    )
    .await;
    let result = prepared
        .finish_userspace_at(Instant::now() + Duration::from_secs(3600))
        .await;
    let ready = handle.ready_for_publication();
    let cleanup = handle.shutdown(OBSERVE).await;
    let report = tokio::time::timeout(OBSERVE, peer_task)
        .await
        .unwrap()
        .unwrap();
    tokio::time::timeout(OBSERVE, async {
        while tokio::runtime::Handle::current()
            .metrics()
            .num_alive_tasks()
            != baseline
        {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    cleanup.unwrap();
    assert_eq!(
        result.as_ref().err().unwrap().class(),
        crate::ErrorClass::Timeout
    );
    assert!(!ready);
    assert!(report.literal_eof);
    assert!(report
        .connections
        .iter()
        .all(|connection| connection.established && connection.uuid.is_some()));
    assert_eq!(owner.completions.load(Ordering::SeqCst), 1);
    drop((result, handle, owner));
    assert_eq!(bank.available_permits(), TOTAL);
}

#[tokio::test]
async fn real_two_private_tcp_directories_prepare_same_owner_without_native_publication() {
    for mode in modes() {
        let baseline = tokio::runtime::Handle::current()
            .metrics()
            .num_alive_tasks();
        let bank = Arc::new(Semaphore::new(TOTAL));
        let owner = Owner {
            bank: bank.clone(),
            mode,
            attached: Mutex::new(None),
            completions: AtomicUsize::new(0),
            attached_dormant: Mutex::new(None),
        };
        let (stream, peer) = tokio::io::duplex(97);
        let peer_task = tokio::spawn(remote(peer, UID));
        let deadline = Instant::now() + OBSERVE;
        let mut result = Some(
            tokio::time::timeout_at(
                deadline,
                prepare_tunnel(
                    connection(&bank),
                    stream,
                    mode,
                    Some(owner.reserve_owned().unwrap()),
                    Prepared {
                        deadline,
                        bank: &bank,
                    },
                ),
            )
            .await,
        );
        let handle = owner.attached.lock().unwrap().as_ref().cloned();
        // Snapshot all functional evidence before terminal cleanup changes ready.
        let mut checked = None;
        let mut initial = None;
        let mut same = false;
        let mut prepared_not_ready = false;
        let mut owned_banks = false;
        let mut independent = false;
        let mut uid_pointer = std::ptr::null();
        let mut catalogue_pointer = 0;
        if let Some(Ok(Ok(prepared))) = result.as_ref() {
            same = handle
                .as_ref()
                .is_some_and(|attached| Arc::ptr_eq(attached, prepared.tunnel_handle()));
            prepared_not_ready = !prepared.tunnel_handle().ready_for_publication();
            checked = Some(prepared.current_rsd_catalogue().clone());
            initial = prepared.rsd().cloned();
            let current = checked.as_ref().unwrap();
            uid_pointer = current.udid().as_ptr();
            catalogue_pointer = std::ptr::from_ref(&**current) as usize;
            if let Some(first) = initial.as_ref() {
                owned_banks = current.belongs_to(&bank)
                    && first.belongs_to(&bank)
                    && prepared.initial_rsd_bytes_in(&bank).unwrap() == first.known_bytes();
                independent = first.get_port(OLD_SERVICE) == Some(12345)
                    && first.get_port(NEW_SERVICE).is_none()
                    && current.service_port(NEW_SERVICE) == Some(23456)
                    && current.service_port(OLD_SERVICE).is_none()
                    && first.udid().as_ptr() != current.udid().as_ptr();
            }
        }
        #[cfg(feature = "tunnel-userspace")]
        let mut finished_same_ready = false;
        #[cfg(feature = "tunnel-userspace")]
        let mut finished = None;
        let successful = result
            .as_ref()
            .is_some_and(|result| matches!(result, Ok(Ok(_))));
        #[cfg(feature = "tunnel-userspace")]
        if mode == TunMode::Userspace {
            if let Some(Ok(Ok(prepared))) = result.take() {
                let result = prepared.finish_userspace_at(deadline).await;
                finished_same_ready = result.as_ref().ok().is_some_and(|device| {
                    device
                        .tunnel_handle()
                        .zip(handle.as_ref())
                        .is_some_and(|(a, b)| Arc::ptr_eq(a, b))
                        && device.tunnel_handle().unwrap().ready_for_publication()
                        && device
                            .prepared_rsd_owner()
                            .unwrap()
                            .ptr_eq(checked.as_ref().unwrap())
                });
                finished = Some(result);
            }
        }
        let cleanup = match &handle {
            Some(handle) => handle.shutdown(OBSERVE).await,
            None => Ok(()),
        };
        let report = tokio::time::timeout(OBSERVE, peer_task)
            .await
            .unwrap()
            .unwrap();
        tokio::time::timeout(OBSERVE, async {
            while tokio::runtime::Handle::current()
                .metrics()
                .num_alive_tasks()
                != baseline
            {
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        cleanup.unwrap();
        assert!(successful, "real production prepare_tunnel failed");
        assert!(same && prepared_not_ready && owned_banks && independent);
        let current = checked.as_ref().unwrap();
        assert_eq!(current.udid().as_ptr(), uid_pointer);
        assert_eq!(std::ptr::from_ref(&**current) as usize, catalogue_pointer);
        assert_eq!(current.udid(), UID);
        assert_eq!(
            current.selection().unwrap().attempted_modes,
            [crate::device::existing_rsd::RsdHandshakeMode::Direct]
        );
        assert_eq!(owner.completions.load(Ordering::SeqCst), 1);
        assert_eq!(
            *owner.attached_dormant.lock().unwrap(),
            Some((
                false,
                false,
                Some(owner.mode),
                owner.mode == TunMode::Kernel
            ))
        );
        assert!(report.literal_eof && report.cancelled_headers > 0);
        assert!(report
            .connections
            .iter()
            .all(|connection| connection.established && connection.uuid.is_some()));
        assert_eq!(report.connections[0].uuid, report.connections[1].uuid);
        assert_ne!(report.connections[0].port, report.connections[1].port);
        assert_eq!(
            report.connections[0].sent,
            response(UID, OLD_SERVICE, 12345).len()
        );
        assert_eq!(
            report.connections[1].sent,
            response(UID, NEW_SERVICE, 23456).len()
        );
        if mode == TunMode::Kernel {
            assert!(handle.as_ref().unwrap().userspace_port.is_none());
            assert!(!handle.as_ref().unwrap().ready_for_publication());
        }
        #[cfg(feature = "tunnel-userspace")]
        if mode == TunMode::Userspace {
            assert!(finished_same_ready);
        }
        #[cfg(feature = "tunnel-userspace")]
        drop(finished);
        // The selected Kernel candidate stays dormant. Its prepared owner is
        // dropped only after actual joins, while the attached Arc was held.
        drop(result.take());
        drop((handle, owner, initial));
        assert_eq!(
            bank.available_permits(),
            TOTAL - checked.as_ref().unwrap().known_bytes()
        );
        drop(checked);
        assert_eq!(bank.available_permits(), TOTAL);
    }
}

#[tokio::test]
async fn real_fresh_checked_wrong_uid_rejects_after_two_tcp_handshakes_without_transferring_owner()
{
    const OTHER_UID: &str = "prepared-wire-independent-wrong-device";
    for mode in modes() {
        let baseline = tokio::runtime::Handle::current()
            .metrics()
            .num_alive_tasks();
        let bank = Arc::new(Semaphore::new(TOTAL));
        let owner = Owner {
            bank: bank.clone(),
            mode,
            attached: Mutex::new(None),
            completions: AtomicUsize::new(0),
            attached_dormant: Mutex::new(None),
        };
        let (stream, peer) = tokio::io::duplex(97);
        let peer_task = tokio::spawn(remote(peer, OTHER_UID));
        let deadline = Instant::now() + OBSERVE;
        let result = tokio::time::timeout_at(
            deadline,
            prepare_tunnel(
                connection(&bank),
                stream,
                mode,
                Some(owner.reserve_owned().unwrap()),
                Prepared {
                    deadline,
                    bank: &bank,
                },
            ),
        )
        .await;
        let handle = owner.attached.lock().unwrap().as_ref().cloned();
        let cleanup = match &handle {
            Some(handle) => handle.shutdown(OBSERVE).await,
            None => Ok(()),
        };
        let report = tokio::time::timeout(OBSERVE, peer_task)
            .await
            .unwrap()
            .unwrap();
        tokio::time::timeout(OBSERVE, async {
            while tokio::runtime::Handle::current()
                .metrics()
                .num_alive_tasks()
                != baseline
            {
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        cleanup.unwrap();
        assert!(matches!(&result, Ok(Err(CoreError::Protocol(_)))));
        assert_eq!(owner.completions.load(Ordering::SeqCst), 0);
        assert!(!handle.as_ref().unwrap().ready_for_publication());
        assert_eq!(handle.as_ref().unwrap().prepared_mode(), Some(mode));
        assert!(report.literal_eof && report.cancelled_headers > 0);
        assert!(report
            .connections
            .iter()
            .all(|connection| connection.established && connection.uuid.is_some()));
        assert_eq!(report.connections[0].uuid, report.connections[1].uuid);
        assert_ne!(report.connections[0].port, report.connections[1].port);
        assert_eq!(
            report.connections[0].sent,
            response(UID, OLD_SERVICE, 12345).len()
        );
        assert_eq!(
            report.connections[1].sent,
            response(OTHER_UID, NEW_SERVICE, 23456).len()
        );
        drop((result, handle, owner));
        assert_eq!(bank.available_permits(), TOTAL);
    }
}
