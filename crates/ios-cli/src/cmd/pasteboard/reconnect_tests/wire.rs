use super::*;
use bytes::Bytes;
use ios_core::xpc::h2_raw::{H2Framer, H2_PREFACE};
use ios_core::xpc::message::{decode_message, encode_message, flags, XpcMessage};
use ios_core::XpcValue;
use tokio::io::{AsyncReadExt, AsyncWriteExt, DuplexStream};
use tokio::sync::Semaphore;

#[derive(Clone)]
pub(super) enum Pull {
    Snapshot(&'static str, i64),
    Anonymous,
    Protocol,
    Eof,
    Partial,
    Late(&'static str, i64, Duration),
}
#[derive(Clone, Copy)]
pub(super) enum Notice { Change, Unrelated, Death, Eof, Partial, Hold }
#[derive(Clone, Copy)]
pub(super) enum Fault {
    Protocol, Unsupported, Permission, Input, Limit, Cancelled, GenericEof,
    TypedEof, MissingDevice, Pending, LateProtocol(Duration),
}
impl Fault {
    pub(super) fn error(self) -> anyhow::Error {
        match self {
            Self::Protocol | Self::LateProtocol(_) => CoreError::Protocol("owned private Source protocol".into()).into(),
            Self::Unsupported => CoreError::Unsupported("owned private Source feature".into()).into(),
            Self::Permission => io::Error::new(io::ErrorKind::PermissionDenied, "owned private Source permission").into(),
            Self::Input => crate::output::input_decision("owned private Source input"),
            Self::Limit => crate::output::resource_decision("owned private Source limit"),
            Self::Cancelled => ios_core::OperationCancelled::new().into(),
            Self::GenericEof => io::Error::new(io::ErrorKind::UnexpectedEof, "owned host-file EOF").into(),
            Self::TypedEof => CoreError::Xpc(ios_core::XpcError::Io(io::Error::new(io::ErrorKind::UnexpectedEof, "owned XPC EOF"))).into(),
            Self::MissingDevice => CoreError::DeviceNotFound(UDID.into()).into(),
            Self::Pending => unreachable!("pending Source does not fabricate an error"),
        }
    }
}

pub(super) struct Episode {
    pub(super) pulls: VecDeque<Pull>,
    pub(super) notices: Vec<Notice>,
    pub(super) fault: Option<Fault>,
    pub(super) native: native::Behavior,
    pub(super) identity: &'static str,
    pub(super) missing: bool,
    pub(super) rsd_partial: bool,
}
impl Episode {
    pub(super) fn new(pulls: impl IntoIterator<Item = Pull>, notices: impl IntoIterator<Item = Notice>) -> Self {
        Self { pulls: pulls.into_iter().collect(), notices: notices.into_iter().collect(), fault: None,
            native: native::Behavior::Normal, identity: UDID, missing: false, rsd_partial: false }
    }
    pub(super) fn fault(fault: Fault) -> Self {
        let mut episode = Self::new([], []);
        episode.fault = Some(fault);
        episode
    }
}

#[derive(Default)]
pub(super) struct Observations {
    pub(super) selectors: Mutex<Vec<String>>,
    pub(super) services: Mutex<Vec<(usize, String, u16)>>,
    pub(super) policies: Mutex<Vec<(usize, DataInclusionPolicy)>>,
    pub(super) registration_before_baseline: AtomicBool,
    pub(super) device_slots: AtomicUsize,
    pub(super) native_slots: Mutex<Vec<(Arc<TunnelHandle>, DuplexStream, Arc<native::Probe>)>>,
    pub(super) peers: Mutex<Vec<tokio::task::JoinHandle<bool>>>,
    pub(super) discoveries: AtomicUsize,
    pub(super) partials: AtomicUsize,
}
pub(super) struct Source {
    episodes: Mutex<VecDeque<Episode>>,
    pub(super) seen: Arc<Observations>,
}
impl Source {
    pub(super) fn new(episodes: impl IntoIterator<Item = Episode>) -> Self {
        Self { episodes: Mutex::new(episodes.into_iter().collect()), seen: Arc::new(Observations {
            registration_before_baseline: AtomicBool::new(true), ..Observations::default()
        }) }
    }
    pub(super) fn calls(&self) -> usize { self.seen.selectors.lock().unwrap().len() }
}

fn dictionary(values: impl IntoIterator<Item = (&'static str, XpcValue)>) -> XpcValue {
    XpcValue::Dictionary(values.into_iter().map(|(name, value)| (name.to_owned(), value)).collect())
}
fn wire(message: XpcMessage) -> Bytes { encode_message(&message).unwrap() }
fn frame(kind: u8, flag: u8, stream: u32, payload: &[u8]) -> Vec<u8> {
    assert!(payload.len() <= 4096);
    let mut bytes = (payload.len() as u32).to_be_bytes()[1..].to_vec();
    bytes.extend_from_slice(&[kind, flag]);
    bytes.extend_from_slice(&stream.to_be_bytes());
    bytes.extend_from_slice(payload);
    bytes
}
async fn read_frame(peer: &mut DuplexStream) -> (u8, u8, u32, Vec<u8>) {
    let mut header = [0; 9];
    peer.read_exact(&mut header).await.unwrap();
    let length = (usize::from(header[0]) << 16) | (usize::from(header[1]) << 8) | usize::from(header[2]);
    assert!(length <= 4096);
    let mut payload = vec![0; length];
    peer.read_exact(&mut payload).await.unwrap();
    (header[3], header[4], u32::from_be_bytes(header[5..].try_into().unwrap()), payload)
}

// Actual three-message RemoteXPC initialization, shared by each owned RSD or
// PULL peer. Each snapshot opens a new connection and validates its new request.
async fn bootstrap(peer: &mut DuplexStream) {
    let mut preface = [0; 24];
    peer.read_exact(&mut preface).await.unwrap();
    assert_eq!(&preface, H2_PREFACE);
    assert_eq!(read_frame(peer).await.0, 4);
    assert_eq!(read_frame(peer).await.0, 8);
    peer.write_all(&frame(4, 0, 0, &[])).await.unwrap();
    assert_eq!({ let ack = read_frame(peer).await; (ack.0, ack.1, ack.2) }, (4, 1, 0));
    assert_eq!({ let head = read_frame(peer).await; (head.0, head.2) }, (1, 1));
    let first = read_frame(peer).await;
    assert_eq!((first.0, first.2), (0, 1));
    let first = decode_message(Bytes::from(first.3)).unwrap();
    assert_eq!(first.flags, flags::ALWAYS_SET);
    assert!(first.body.unwrap().as_dict().unwrap().is_empty());
    peer.write_all(&frame(0, 0, 1, &wire(XpcMessage { flags: flags::ALWAYS_SET, msg_id: 0, body: None }))).await.unwrap();
    assert_eq!({ let head = read_frame(peer).await; (head.0, head.2) }, (1, 3));
    let second = read_frame(peer).await;
    assert_eq!((second.0, second.2), (0, 1));
    let second = decode_message(Bytes::from(second.3)).unwrap();
    assert_eq!(second.flags, flags::ALWAYS_SET | 0x200);
    assert!(second.body.is_none());
    peer.write_all(&frame(0, 0, 1, &wire(XpcMessage { flags: flags::ALWAYS_SET, msg_id: 0, body: None }))).await.unwrap();
    let third = read_frame(peer).await;
    assert_eq!((third.0, third.2), (0, 3));
    let third = decode_message(Bytes::from(third.3)).unwrap();
    assert_eq!(third.flags, flags::ALWAYS_SET | flags::INIT_HANDSHAKE);
    assert!(third.body.is_none());
    peer.write_all(&frame(0, 0, 3, &wire(XpcMessage { flags: flags::ALWAYS_SET, msg_id: 0, body: None }))).await.unwrap();
}
async fn eof(peer: &mut DuplexStream) -> bool {
    matches!(peer.read_u8().await, Err(error) if error.kind() == io::ErrorKind::UnexpectedEof)
}

impl WatchSource for Source {
    fn connect<'a>(&'a self, udid: &'a str, owner: &'a dyn TunnelOwner) -> BoxFuture<'a, Result<Arc<dyn WatchDevice>>> {
        Box::pin(async move {
            let index = { let mut selectors = self.seen.selectors.lock().unwrap(); selectors.push(udid.to_owned()); selectors.len() };
            let episode = self.episodes.lock().unwrap().pop_front().expect("unbudgeted Source admission");
            let mut reservation = owner.reserve()?;
            let (handle, native_peer, probe) = native::start(episode.native).await;
            reservation.attach(handle.clone())?;
            let reader = probe.clone();
            self.seen.native_slots.lock().unwrap().push((handle, native_peer, probe));
            // Poll the original native packet reader before any setup can fail.
            while reader.reads.load(Ordering::SeqCst) == 0 { tokio::task::yield_now().await; }
            if let Some(fault) = episode.fault {
                if let Fault::Pending = fault { std::future::pending::<()>().await; }
                if let Fault::LateProtocol(delay) = fault { std::thread::sleep(delay); }
                return Err(fault.error());
            }
            let notification_port = 21000 + index as u16 * 2;
            let pasteboard_port = notification_port + 1;
            let body = dictionary([
                ("MessageType", XpcValue::String("Handshake".into())),
                ("Properties", dictionary([("UniqueDeviceID", XpcValue::String(episode.identity.into()))])),
                ("Services", if episode.missing { dictionary([]) } else { dictionary([
                    (ios_core::notificationproxy::RSD_SERVICE_NAME, dictionary([("Port", XpcValue::String(notification_port.to_string()))])),
                    (ios_core::pasteboard::SERVICE_NAME, dictionary([("Port", XpcValue::String(pasteboard_port.to_string()))])),
                ]) }),
            ]);
            let (client, mut peer) = tokio::io::duplex(8192);
            let observed = self.seen.clone();
            let partial = episode.rsd_partial;
            self.seen.peers.lock().unwrap().push(tokio::spawn(async move {
                bootstrap(&mut peer).await;
                let message = wire(XpcMessage { flags: flags::ALWAYS_SET | flags::DATA, msg_id: 1, body: Some(body) });
                peer.write_all(&frame(0, 0, 1, if partial { &message[..12] } else { &message[..] })).await.unwrap();
                if partial { observed.partials.fetch_add(1, Ordering::SeqCst); }
                eof(&mut peer).await
            }));
            let mut framer = H2Framer::connect(client).await?;
            ios_core::xpc::rsd::initialize_xpc_connection_on_framer(&mut framer).await?;
            let directory = ios_core::xpc::rsd::handshake_on_framer(&mut framer).await?;
            drop(framer);
            self.seen.discoveries.fetch_add(1, Ordering::SeqCst);
            anyhow::ensure!(directory.udid == udid, CoreError::Protocol("fresh RSD identity mismatch".into()));
            reservation.complete();
            self.seen.device_slots.fetch_add(1, Ordering::SeqCst);
            Ok(Arc::new(Device { index, directory, pulls: Mutex::new(episode.pulls), notices: Mutex::new(Some(episode.notices)),
                seen: self.seen.clone(), registered: Arc::new(AtomicBool::new(false)), baseline: Arc::new(Semaphore::new(0)) }) as Arc<dyn WatchDevice>)
        })
    }
}

struct Device {
    index: usize,
    directory: ios_core::RsdHandshake,
    pulls: Mutex<VecDeque<Pull>>,
    notices: Mutex<Option<Vec<Notice>>>,
    seen: Arc<Observations>,
    registered: Arc<AtomicBool>,
    baseline: Arc<Semaphore>,
}
impl Drop for Device {
    fn drop(&mut self) { self.seen.device_slots.fetch_sub(1, Ordering::SeqCst); }
}
impl WatchDevice for Device {
    fn notification(&self) -> BoxFuture<'_, Result<ServiceStream>> {
        Box::pin(async move {
            let port = self.directory.get_port(ios_core::notificationproxy::RSD_SERVICE_NAME)
                .ok_or_else(|| CoreError::Xpc(ios_core::XpcError::ServiceNotFound("notification".into())))?;
            self.seen.services.lock().unwrap().push((self.index, "notification".into(), port));
            let notices = self.notices.lock().unwrap().take().unwrap();
            let registered = self.registered.clone();
            let baseline = self.baseline.clone();
            let observed = self.seen.clone();
            let (client, mut peer) = tokio::io::duplex(8192);
            self.seen.peers.lock().unwrap().push(tokio::spawn(async move {
                let length = peer.read_u32().await.unwrap();
                assert!(length <= 4096);
                let mut bytes = vec![0; length as usize];
                peer.read_exact(&mut bytes).await.unwrap();
                let request: plist::Dictionary = plist::from_bytes(&bytes).unwrap();
                assert_eq!(request["Command"].as_string(), Some("ObserveNotification"));
                assert_eq!(request["Name"].as_string(), Some(ios_core::pasteboard::PASTEBOARD_CHANGED_NOTIFICATION));
                registered.store(true, Ordering::SeqCst);
                // The baseline finishes silently before subsequent notifications.
                let mut wait = Box::pin(baseline.acquire());
                tokio::select! {
                    permit = &mut wait => permit.unwrap().forget(),
                    ended = peer.read_u8() => return matches!(ended, Err(error) if error.kind() == io::ErrorKind::UnexpectedEof),
                }
                for notice in notices {
                    match notice {
                        Notice::Hold => break,
                        Notice::Eof => { peer.shutdown().await.unwrap(); break; }
                        Notice::Partial => {
                            peer.write_all(&[0, 0]).await.unwrap();
                            observed.partials.fetch_add(1, Ordering::SeqCst);
                            break;
                        }
                        _ => {
                            let (command, name) = match notice {
                                Notice::Change => ("RelayNotification", ios_core::pasteboard::PASTEBOARD_CHANGED_NOTIFICATION),
                                Notice::Unrelated => ("RelayNotification", "owned.unrelated"),
                                Notice::Death => ("ProxyDeath", ""),
                                _ => unreachable!(),
                            };
                            let value = plist::Value::Dictionary(plist::Dictionary::from_iter([
                                ("Command".to_owned(), plist::Value::String(command.into())),
                                ("Name".to_owned(), plist::Value::String(name.into())),
                            ]));
                            let mut bytes = Vec::new();
                            value.to_writer_xml(&mut bytes).unwrap();
                            peer.write_u32(bytes.len() as u32).await.unwrap();
                            peer.write_all(&bytes).await.unwrap();
                        }
                    }
                }
                eof(&mut peer).await
            }));
            Ok(Box::new(client) as ServiceStream)
        })
    }
    fn snapshot<'a>(&'a self, name: &'a str, policy: DataInclusionPolicy) -> BoxFuture<'a, std::result::Result<PasteboardSnapshot, PasteboardError>> {
        Box::pin(async move {
            self.seen.policies.lock().unwrap().push((self.index, policy));
            // Registration is sent before baseline; its peer may not yet have
            // been scheduled, so yield before checking actual wire receipt.
            tokio::task::yield_now().await;
            if !self.registered.load(Ordering::SeqCst) { self.seen.registration_before_baseline.store(false, Ordering::SeqCst); }
            let port = self.directory.get_port(ios_core::pasteboard::SERVICE_NAME)
                .ok_or_else(|| PasteboardError::Xpc(ios_core::XpcError::ServiceNotFound("pasteboard".into())))?;
            self.seen.services.lock().unwrap().push((self.index, "pasteboard".into(), port));
            let pull = self.pulls.lock().unwrap().pop_front().expect("unbounded snapshot read");
            let reply = match &pull {
                Pull::Snapshot(nonce, count) | Pull::Late(nonce, count, _) => dictionary([
                    ("command", XpcValue::String("PULL_REPLY".into())),
                    ("pasteboard", dictionary([
                        ("metadata", dictionary([("nonce", XpcValue::String((*nonce).into())), ("changeCount", XpcValue::Int64(*count))])),
                        ("items", XpcValue::Array(Vec::new())),
                    ])),
                ]),
                Pull::Anonymous => dictionary([
                    ("command", XpcValue::String("PULL_REPLY".into())),
                    ("pasteboard", dictionary([("items", XpcValue::Array(Vec::new()))])),
                ]),
                _ => dictionary([("command", XpcValue::Int64(-1))]),
            };
            let baseline = self.baseline.clone();
            let observed = self.seen.clone();
            let is_baseline = policy == DataInclusionPolicy::AllPromised;
            let (client, mut peer) = tokio::io::duplex(8192);
            let script = pull.clone();
            let peer_name = name.to_owned();
            self.seen.peers.lock().unwrap().push(tokio::spawn(async move {
                bootstrap(&mut peer).await;
                let request = loop {
                    let request = read_frame(&mut peer).await;
                    if request.0 == 0 && request.2 == 1 { break decode_message(Bytes::from(request.3)).unwrap(); }
                };
                let body = request.body.unwrap();
                let body = body.as_dict().unwrap();
                assert_eq!(body["command"].as_str(), Some("PULL"));
                assert_eq!(body["pasteboardName"].as_str(), Some(peer_name.as_str()));
                let policy_wire = body["dataPolicy"].as_dict().unwrap();
                assert!(policy_wire.contains_key(if is_baseline { "allPromised" } else { "promiseSecondary" }));
                match script {
                    Pull::Eof => peer.shutdown().await.unwrap(),
                    Pull::Partial => {
                        let message = wire(XpcMessage { flags: flags::ALWAYS_SET | flags::REPLY | flags::DATA, msg_id: request.msg_id, body: Some(reply) });
                        peer.write_all(&frame(0, 0, 3, &message[..12])).await.unwrap();
                        observed.partials.fetch_add(1, Ordering::SeqCst);
                    }
                    _ => {
                        peer.write_all(&frame(0, 0, 3, &wire(XpcMessage { flags: flags::ALWAYS_SET | flags::REPLY | flags::DATA, msg_id: request.msg_id, body: Some(reply) }))).await.unwrap();
                    }
                }
                let closed = eof(&mut peer).await;
                if is_baseline { baseline.add_permits(1); }
                closed
            }));
            let result = super::super::super::read_notification_snapshot(
                async move { ios_core::XpcClient::connect_stream(client).await.map_err(CoreError::from) }, name, policy).await;
            if let Pull::Late(_, _, delay) = pull { std::thread::sleep(delay); }
            result
        })
    }
}
