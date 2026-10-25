//! Explicit local scripted peer for cross-crate lifecycle fixtures only.
//! Peer observation memory is external to the admitted production owner bank.
use crate::xpc::message::{decode_message, encode_message, flags};
use crate::{XpcMessage, XpcValue};
use bytes::Bytes;
use smoltcp::{
    iface::{Config, Interface, SocketSet},
    phy::{Device, DeviceCapabilities, Medium, RxToken, TxToken},
    socket::tcp,
    time::Instant as SmolInstant,
    wire::{HardwareAddress, IpAddress, IpCidr, Ipv6Address},
};
use std::{collections::VecDeque, future::pending, io, sync::Arc, time::Duration};
use tokio::{
    io::{AsyncRead, AsyncReadExt, AsyncWriteExt, DuplexStream},
    sync::Notify,
};
const OLD_SERVICE: &str = "wire.initial.only";
const NEW_SERVICE: &str = "wire.checked.only";
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
pub struct Connection {
    request: Vec<u8>,
    parsed: usize,
    pub uuid: Option<[u8; 16]>,
    pub port: Option<u16>,
    pub established: bool,
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
pub struct PeerReport {
    pub connections: [Connection; 2],
    pub cancelled_headers: usize,
    pub literal_eof: bool,
}

pub async fn remote(mut peer: DuplexStream, checked_uid: &'static str) -> PeerReport {
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
        response(checked_uid, OLD_SERVICE, 12345),
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
