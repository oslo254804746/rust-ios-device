//! Owned protocol peers exercise the production H2/RemoteXPC probe. No tunnel
//! creation, pairing, service execution, OS TUN or physical device is involved.
use super::*;
use crate::xpc::message::{decode_message, encode_message, flags, XpcMessage, XpcValue};
use bytes::Bytes;
use indexmap::IndexMap;
use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Mutex,
};
use tokio::io::AsyncReadExt;

const UDID: &str = "synthetic-private-existing-rsd-identity";
const SERVICE: &str = "synthetic.private.service";
const OBSERVE: Duration = Duration::from_secs(3);

fn route() -> ExistingRsdRoute {
    ExistingRsdRoute::direct("[::1]:12345".parse().unwrap()).unwrap()
}

struct OwnedSource {
    identity_calls: AtomicUsize,
    dial_calls: AtomicUsize,
    streams: Mutex<std::collections::VecDeque<ServiceStream>>,
}

impl OwnedSource {
    fn new(streams: impl IntoIterator<Item = ServiceStream>) -> Self {
        Self {
            identity_calls: AtomicUsize::new(0),
            dial_calls: AtomicUsize::new(0),
            streams: Mutex::new(streams.into_iter().collect()),
        }
    }
}

impl Source for OwnedSource {
    fn identity(&self) -> BoxFuture<'_, Result<uuid::Uuid, XpcError>> {
        Box::pin(async {
            self.identity_calls.fetch_add(1, Ordering::SeqCst);
            Ok(uuid::Uuid::from_bytes([0x55; 16]))
        })
    }

    fn dial(&self, _: ExistingRsdRoute) -> BoxFuture<'_, Result<ServiceStream, CoreError>> {
        Box::pin(async {
            self.dial_calls.fetch_add(1, Ordering::SeqCst);
            self.streams.lock().unwrap().pop_front().ok_or_else(|| {
                io::Error::new(
                    io::ErrorKind::ConnectionRefused,
                    "unexpected fresh RSD dial",
                )
                .into()
            })
        })
    }
}

fn directory() -> IndexMap<String, XpcValue> {
    IndexMap::from([
        (
            String::from("MessageType"),
            XpcValue::String(String::from("Handshake")),
        ),
        (
            String::from("Properties"),
            XpcValue::Dictionary(IndexMap::from([(
                String::from("UniqueDeviceID"),
                XpcValue::String(UDID.into()),
            )])),
        ),
        (
            String::from("Services"),
            XpcValue::Dictionary(IndexMap::from([(
                SERVICE.into(),
                XpcValue::Dictionary(IndexMap::from([
                    (String::from("Port"), XpcValue::Uint64(23456)),
                    (
                        String::from("Properties"),
                        XpcValue::Dictionary(IndexMap::from([(
                            String::from("Features"),
                            XpcValue::Array(vec![XpcValue::String("feature".into())]),
                        )])),
                    ),
                ])),
            )])),
        ),
    ])
}

fn wire(dictionary: IndexMap<String, XpcValue>) -> Bytes {
    encode_message(&XpcMessage {
        flags: flags::ALWAYS_SET | flags::DATA,
        msg_id: 0,
        body: Some(XpcValue::Dictionary(dictionary)),
    })
    .unwrap()
}

fn frame(kind: u8, flag: u8, stream: u32, body: &[u8]) -> Vec<u8> {
    assert!(body.len() <= 0xff_ffff);
    let n = body.len() as u32;
    let mut bytes = Vec::with_capacity(9 + body.len());
    bytes.extend_from_slice(&n.to_be_bytes()[1..]);
    bytes.extend_from_slice(&[kind, flag]);
    bytes.extend_from_slice(&stream.to_be_bytes());
    bytes.extend_from_slice(body);
    bytes
}

async fn read_frame<S: AsyncRead + Unpin>(peer: &mut S) -> (u8, u8, u32, Vec<u8>) {
    let mut header = [0; 9];
    peer.read_exact(&mut header).await.unwrap();
    let n = (usize::from(header[0]) << 16) | (usize::from(header[1]) << 8) | usize::from(header[2]);
    assert!(n <= 4096, "unexpected outgoing probe payload");
    let mut body = vec![0; n];
    peer.read_exact(&mut body).await.unwrap();
    (
        header[3],
        header[4],
        u32::from_be_bytes(header[5..].try_into().unwrap()),
        body,
    )
}

async fn active_bootstrap<S: AsyncRead + AsyncWrite + Unpin>(peer: &mut S) {
    let mut preface = [0; 24];
    peer.read_exact(&mut preface).await.unwrap();
    assert_eq!(&preface, crate::xpc::h2_raw::H2_PREFACE);
    assert_eq!(read_frame(peer).await.0, 4);
    assert_eq!(read_frame(peer).await.0, 8);
    peer.write_all(&frame(4, 0, 0, &[])).await.unwrap();
    for (kind, stream) in [(1, 1), (0, 1), (1, 3), (0, 1), (0, 3)] {
        let received = read_frame(peer).await;
        assert_eq!((received.0, received.2), (kind, stream));
    }
    let ack = read_frame(peer).await;
    assert_eq!((ack.0, ack.1, ack.2), (4, 1, 0));
    let sent = read_frame(peer).await;
    assert_eq!((sent.0, sent.2), (0, 1));
    let message = decode_message(Bytes::from(sent.3)).unwrap();
    let body = message.body.unwrap();
    let body = body.as_dict().unwrap();
    assert_eq!(
        body.get("MessageType").and_then(XpcValue::as_str),
        Some("Handshake")
    );
    assert_eq!(
        body.get("Services")
            .and_then(XpcValue::as_dict)
            .unwrap()
            .len(),
        0
    );
}

async fn reply<S: AsyncWrite + Unpin>(peer: &mut S, messages: Vec<Bytes>) {
    for message in messages {
        for chunk in message.chunks(1024) {
            peer.write_all(&frame(0, 0, 1, chunk)).await.unwrap();
        }
    }
}

fn assert_control_tail(mut remaining: &[u8]) {
    // H2 flow-control credit may be returned while a large directory is read.
    // No later XPC DATA, service stream HEADERS or mutation is permitted.
    while !remaining.is_empty() {
        assert!(remaining.len() >= 9);
        let n = (usize::from(remaining[0]) << 16)
            | (usize::from(remaining[1]) << 8)
            | usize::from(remaining[2]);
        assert_eq!(remaining[3], 8, "probe executed an extra request");
        assert_eq!(n, 4);
        assert!(remaining.len() >= 9 + n);
        remaining = &remaining[9 + n..];
    }
}

async fn roundtrip(messages: Vec<Bytes>) -> (Result<ExistingRsdObservation, CoreError>, usize) {
    let (stream, mut peer) = tokio::io::duplex(16_384);
    let source = OwnedSource::new([Box::new(stream) as ServiceStream]);
    let (result, ()) = tokio::time::timeout(OBSERVE, async {
        tokio::join!(observe_from_source(&source, route(), OBSERVE), async {
            active_bootstrap(&mut peer).await;
            reply(&mut peer, messages).await;
            let mut remaining = Vec::new();
            peer.read_to_end(&mut remaining).await.unwrap();
            assert_control_tail(&remaining);
        })
    })
    .await
    .unwrap();
    (result, source.dial_calls.load(Ordering::SeqCst))
}

#[tokio::test]
async fn existing_rsd_checked_directory_uses_real_handshake_and_closes_its_socket() {
    let unrelated = IndexMap::from([
        (
            String::from("MessageType"),
            XpcValue::String("Unrelated".into()),
        ),
        (String::from("Services"), XpcValue::Bool(false)),
    ]);
    let (result, calls) = roundtrip(vec![wire(unrelated), wire(directory())]).await;
    let observation = result.unwrap();
    assert_eq!(calls, 1);
    assert_eq!(observation.udid(), Some(UDID));
    assert!(observation.complete_service_directory());
    assert_eq!(
        observation.service_evidence(SERVICE),
        ExistingRsdServiceEvidence::Advertised
    );
    assert_eq!(
        observation.service_evidence("missing"),
        ExistingRsdServiceEvidence::NotAdvertised
    );
    let service = &observation.services().unwrap()[SERVICE];
    assert_eq!(service.port(), Some(23456));
    assert_eq!(service.features().unwrap(), &[String::from("feature")]);
    assert!(!format!("{observation:?}").contains(UDID));
    assert!(!format!("{observation:?}").contains(SERVICE));
}

#[tokio::test]
async fn existing_rsd_zero_overflow_and_unpolled_budget_have_no_source_work() {
    let source = OwnedSource::new([]);
    drop(observe_from_source(&source, route(), OBSERVE));
    let zero = observe_from_source(&source, route(), Duration::ZERO)
        .await
        .unwrap_err();
    let overflow = observe_from_source(&source, route(), Duration::MAX)
        .await
        .unwrap_err();
    assert_eq!(crate::classify_error(&zero), ErrorClass::Timeout);
    assert_eq!(crate::classify_error(&overflow), ErrorClass::InvalidInput);
    assert_eq!(source.identity_calls.load(Ordering::SeqCst), 0);
    assert_eq!(source.dial_calls.load(Ordering::SeqCst), 0);
}

#[test]
fn existing_rsd_route_validation_preserves_direct_scope_and_redacts_address() {
    let scoped = SocketAddrV6::new("fe80::1".parse().unwrap(), 12345, 7, 2);
    let direct = ExistingRsdRoute::direct(scoped).unwrap();
    assert_eq!(direct.endpoint(), scoped);
    assert!(ExistingRsdRoute::via_userspace_proxy(scoped, 34567).is_err());
    assert!(ExistingRsdRoute::direct("[::1]:0".parse().unwrap()).is_err());
    assert!(ExistingRsdRoute::via_userspace_proxy(route().endpoint(), 0).is_err());
    assert!(!format!("{direct:?}").contains("fe80"));
}

#[tokio::test]
async fn existing_rsd_missing_fields_do_not_prove_service_absence_or_false_features() {
    for missing in [
        "identity",
        "directory",
        "port",
        "features",
        "empty_features",
    ] {
        let mut dictionary = directory();
        match missing {
            "identity" => {
                dictionary.shift_remove("Properties");
            }
            "directory" => {
                dictionary.shift_remove("Services");
            }
            _ => {
                let XpcValue::Dictionary(services) = dictionary.get_mut("Services").unwrap() else {
                    unreachable!()
                };
                let XpcValue::Dictionary(service) = services.get_mut(SERVICE).unwrap() else {
                    unreachable!()
                };
                match missing {
                    "port" => {
                        service.shift_remove("Port");
                    }
                    "features" => {
                        service.shift_remove("Properties");
                    }
                    "empty_features" => {
                        let XpcValue::Dictionary(properties) =
                            service.get_mut("Properties").unwrap()
                        else {
                            unreachable!()
                        };
                        properties.insert("Features".into(), XpcValue::Array(Vec::new()));
                    }
                    _ => unreachable!(),
                }
            }
        }
        let (result, calls) = roundtrip(vec![wire(dictionary)]).await;
        let observation = result.unwrap();
        assert_eq!(calls, 1);
        if matches!(missing, "features" | "empty_features") {
            assert!(observation.complete_service_directory());
            assert_eq!(
                observation.service_evidence(SERVICE),
                ExistingRsdServiceEvidence::Advertised
            );
            let features = observation.services().unwrap()[SERVICE].features();
            assert_eq!(features.is_none(), missing == "features");
            if let Some(features) = features {
                assert!(features.is_empty());
            }
        } else {
            assert!(!observation.complete_service_directory());
            assert_eq!(
                observation.service_evidence("missing"),
                ExistingRsdServiceEvidence::MetadataMissing
            );
            assert_eq!(
                observation.service_evidence(SERVICE),
                ExistingRsdServiceEvidence::MetadataMissing
            );
        }
    }
}

#[tokio::test]
async fn existing_rsd_canonical_incomplete_entry_does_not_fall_back_to_a_shim() {
    let mut dictionary = directory();
    let XpcValue::Dictionary(services) = dictionary.get_mut("Services").unwrap() else {
        unreachable!()
    };
    services.insert(format!("{SERVICE}.shim.remote"), services[SERVICE].clone());
    let XpcValue::Dictionary(service) = services.get_mut(SERVICE).unwrap() else {
        unreachable!()
    };
    service.shift_remove("Port");
    let (result, calls) = roundtrip(vec![wire(dictionary)]).await;
    let observation = result.unwrap();
    assert_eq!(calls, 1);
    assert_eq!(
        observation.service_evidence(SERVICE),
        ExistingRsdServiceEvidence::MetadataMissing
    );
    assert_eq!(
        observation.service_evidence("missing"),
        ExistingRsdServiceEvidence::MetadataMissing
    );
}

#[tokio::test]
async fn existing_rsd_malformed_handshake_is_final_and_never_replaced_by_another_dial() {
    for field in [
        "MessageType",
        "Properties",
        "UniqueDeviceID",
        "Services",
        "service",
        "Port",
        "Features",
        "feature",
    ] {
        let mut dictionary = directory();
        match field {
            "MessageType" | "Properties" | "Services" => {
                dictionary.insert(field.into(), XpcValue::Bool(false));
            }
            "UniqueDeviceID" => {
                dictionary.insert(
                    "Properties".into(),
                    XpcValue::Dictionary(IndexMap::from([(
                        String::from("UniqueDeviceID"),
                        XpcValue::Bool(false),
                    )])),
                );
            }
            _ => {
                let XpcValue::Dictionary(services) = dictionary.get_mut("Services").unwrap() else {
                    unreachable!()
                };
                if field == "service" {
                    services.insert("unselected.private.malformed".into(), XpcValue::Bool(false));
                } else {
                    let XpcValue::Dictionary(service) = services.get_mut(SERVICE).unwrap() else {
                        unreachable!()
                    };
                    if field == "Port" {
                        service.insert("Port".into(), XpcValue::Uint64(65536));
                    } else {
                        service.insert(
                            "Properties".into(),
                            XpcValue::Dictionary(IndexMap::from([(
                                String::from("Features"),
                                if field == "feature" {
                                    XpcValue::Array(vec![XpcValue::Bool(false)])
                                } else {
                                    XpcValue::Bool(false)
                                },
                            )])),
                        );
                    }
                }
            }
        }
        let (result, calls) = roundtrip(vec![wire(dictionary)]).await;
        let error = result.unwrap_err();
        assert_eq!(calls, 1, "malformed {field} was retried");
        assert_eq!(
            crate::classify_error(&error),
            ErrorClass::Protocol,
            "{field}"
        );
        assert!(!format!("{error}").contains(UDID));
        assert!(!format!("{error:?}").contains(SERVICE));
    }
}

#[tokio::test]
async fn existing_rsd_logical_metadata_limits_reject_the_whole_decoded_directory() {
    for scope in ["field", "services", "features", "retained_text"] {
        let mut dictionary = directory();
        match scope {
            "field" => {
                dictionary.insert(
                    "Properties".into(),
                    XpcValue::Dictionary(IndexMap::from([(
                        String::from("UniqueDeviceID"),
                        XpcValue::String("x".repeat(MAX_FIELD_BYTES + 1)),
                    )])),
                );
            }
            "services" => {
                dictionary.insert(
                    "Services".into(),
                    XpcValue::Dictionary(
                        (0..=MAX_SERVICES)
                            .map(|n| {
                                (
                                    format!("service.{n}"),
                                    XpcValue::Dictionary(IndexMap::from([(
                                        String::from("Port"),
                                        XpcValue::Uint64(1),
                                    )])),
                                )
                            })
                            .collect(),
                    ),
                );
            }
            _ => {
                let count = if scope == "features" {
                    MAX_FEATURES + 1
                } else {
                    MAX_TEXT_BYTES / MAX_FIELD_BYTES + 1
                };
                let size = if scope == "features" {
                    1
                } else {
                    MAX_FIELD_BYTES
                };
                let XpcValue::Dictionary(services) = dictionary.get_mut("Services").unwrap() else {
                    unreachable!()
                };
                let XpcValue::Dictionary(service) = services.get_mut(SERVICE).unwrap() else {
                    unreachable!()
                };
                service.insert(
                    "Properties".into(),
                    XpcValue::Dictionary(IndexMap::from([(
                        String::from("Features"),
                        XpcValue::Array(
                            (0..count)
                                .map(|_| XpcValue::String("x".repeat(size)))
                                .collect(),
                        ),
                    )])),
                );
            }
        }
        let (result, calls) = roundtrip(vec![wire(dictionary)]).await;
        let error = result.unwrap_err();
        assert_eq!(calls, 1);
        assert_eq!(
            crate::classify_error(&error),
            ErrorClass::ResourceLimit,
            "{scope}: {error}"
        );
        assert!(!format!("{error:?}").contains(UDID));
    }
}

struct ReadObserved {
    stream: ServiceStream,
    bytes: usize,
    consumed: std::sync::Arc<tokio::sync::Notify>,
}

impl AsyncRead for ReadObserved {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        let before = buf.filled().len();
        let result = Pin::new(&mut self.stream).poll_read(cx, buf);
        self.bytes += buf.filled().len() - before;
        // Nine settings bytes plus the 9+12 byte partial DATA frame. Observing
        // the read prevents cancellation with unread TCP receive bytes, which
        // legitimately causes RST rather than EOF on Linux.
        if self.bytes >= 30 {
            self.consumed.notify_one();
        }
        result
    }
}
impl AsyncWrite for ReadObserved {
    fn poll_write(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        bytes: &[u8],
    ) -> Poll<io::Result<usize>> {
        Pin::new(&mut self.stream).poll_write(cx, bytes)
    }
    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.stream).poll_flush(cx)
    }
    fn poll_shutdown(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.stream).poll_shutdown(cx)
    }
}
struct ReadObservedSource(std::sync::Arc<tokio::sync::Notify>);
impl Source for ReadObservedSource {
    fn identity(&self) -> BoxFuture<'_, Result<uuid::Uuid, XpcError>> {
        NativeSource.identity()
    }
    fn dial(&self, route: ExistingRsdRoute) -> BoxFuture<'_, Result<ServiceStream, CoreError>> {
        Box::pin(async move {
            Ok(Box::new(ReadObserved {
                stream: NativeSource.dial(route).await?,
                bytes: 0,
                consumed: self.0.clone(),
            }) as ServiceStream)
        })
    }
}

#[tokio::test]
async fn existing_rsd_owned_loopback_proxy_success_and_cancel_preserve_the_existing_listener() {
    let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
        .await
        .unwrap();
    let port = listener.local_addr().unwrap().port();
    let endpoint: SocketAddrV6 = "[fd00::1]:23456".parse().unwrap();
    let route = ExistingRsdRoute::via_userspace_proxy(endpoint, port).unwrap();
    for cancel in [true, false, true, false] {
        let ready = std::sync::Arc::new(tokio::sync::Notify::new());
        let source = ReadObservedSource(ready.clone());
        let (result, ()) = tokio::time::timeout(OBSERVE, async {
            tokio::join!(async {
                let probe = async {
                    if cancel { observe_from_source(&source, route, OBSERVE).await }
                    else { observe_existing_rsd(route, OBSERVE).await }
                };
                tokio::pin!(probe);
                if cancel {
                    tokio::select! {
                        biased;
                        result = &mut probe => { panic!("probe finished before caller cancellation: {result:?}"); }
                        _ = ready.notified() => None,
                    }
                } else {
                    Some(probe.await)
                }
            }, async {
                let (mut peer, _) = listener.accept().await.unwrap();
                let mut prelude = [0; 20];
                peer.read_exact(&mut prelude).await.unwrap();
                assert_eq!(&prelude[..16], &endpoint.ip().octets());
                assert_eq!(&prelude[16..], &(endpoint.port() as u32).to_le_bytes());
                active_bootstrap(&mut peer).await;
                if cancel {
                    // Partial response makes reuse impossible and cancellation
                    // must release this exact probe socket, never the proxy.
                    peer.write_all(&frame(0, 0, 1, &wire(directory())[..12])).await.unwrap();
                } else {
                    reply(&mut peer, vec![wire(directory())]).await;
                }
                let mut remaining = Vec::new();
                peer.read_to_end(&mut remaining).await.unwrap();
                assert_control_tail(&remaining);
            })
        }).await.unwrap();
        assert_eq!(listener.local_addr().unwrap().port(), port);
        match result {
            None => assert!(cancel),
            Some(result) => {
                assert!(!cancel);
                assert_eq!(result.unwrap().udid(), Some(UDID));
            }
        }
    }
}

#[derive(Debug)]
struct NativeMarker(std::sync::Arc<()>);
impl fmt::Display for NativeMarker {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("synthetic native RSD primary")
    }
}
impl Error for NativeMarker {}

struct CrossingIo {
    calls: std::sync::Arc<AtomicUsize>,
    marker: Option<std::sync::Arc<()>>,
}

impl AsyncRead for CrossingIo {
    fn poll_read(
        self: Pin<&mut Self>,
        _: &mut Context<'_>,
        _: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        Poll::Pending
    }
}
impl AsyncWrite for CrossingIo {
    fn poll_write(
        self: Pin<&mut Self>,
        _: &mut Context<'_>,
        bytes: &[u8],
    ) -> Poll<io::Result<usize>> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        // A cooperative deadline cannot preempt this poll. The guarded stream
        // must suppress late success and keep an already-returned native cause.
        std::thread::sleep(Duration::from_millis(40));
        Poll::Ready(match &self.marker {
            None => Ok(bytes.len()),
            Some(marker) => Err(io::Error::new(
                io::ErrorKind::BrokenPipe,
                NativeMarker(marker.clone()),
            )),
        })
    }
    fn poll_flush(self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<io::Result<()>> {
        Poll::Ready(Ok(()))
    }
    fn poll_shutdown(self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<io::Result<()>> {
        Poll::Ready(Ok(()))
    }
}

fn find_marker<'a>(mut error: &'a (dyn Error + 'static)) -> Option<&'a NativeMarker> {
    for _ in 0..32 {
        if let Some(marker) = error.downcast_ref::<NativeMarker>() {
            return Some(marker);
        }
        let next = if let Some(native) = error.downcast_ref::<io::Error>() {
            native
                .get_ref()
                .map(|source| source as &(dyn Error + 'static))
        } else {
            error.source()
        };
        error = next?;
    }
    None
}

#[tokio::test]
async fn existing_rsd_crossing_write_stops_further_io_and_preserves_native_primary_identity() {
    for failing in [false, true] {
        let calls = std::sync::Arc::new(AtomicUsize::new(0));
        let marker = std::sync::Arc::new(());
        let source = OwnedSource::new([Box::new(CrossingIo {
            calls: calls.clone(),
            marker: failing.then(|| marker.clone()),
        }) as ServiceStream]);
        let error = observe_from_source(&source, route(), Duration::from_millis(10))
            .await
            .unwrap_err();
        assert_eq!(source.dial_calls.load(Ordering::SeqCst), 1);
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        if failing {
            let retained = find_marker(&error).expect("native primary was replaced by expiry");
            assert!(std::sync::Arc::ptr_eq(&retained.0, &marker));
        } else {
            assert_eq!(crate::classify_error(&error), ErrorClass::Timeout);
            assert!(find_marker(&error).is_none());
        }
    }
}

#[tokio::test]
async fn existing_rsd_partial_h2_response_expires_and_drops_the_original_socket() {
    let (stream, mut peer) = tokio::io::duplex(4096);
    let source = OwnedSource::new([Box::new(stream) as ServiceStream]);
    let (result, ()) = tokio::time::timeout(OBSERVE, async {
        tokio::join!(
            observe_from_source(&source, route(), Duration::from_millis(100)),
            async {
                active_bootstrap(&mut peer).await;
                peer.write_all(&[0, 0, 100, 0]).await.unwrap();
                let mut remaining = Vec::new();
                peer.read_to_end(&mut remaining).await.unwrap();
                assert_control_tail(&remaining);
            }
        )
    })
    .await
    .unwrap();
    let error = result.unwrap_err();
    assert_eq!(crate::classify_error(&error), ErrorClass::Timeout);
    assert_eq!(source.dial_calls.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn existing_rsd_compatibility_fallback_dials_a_fresh_socket_under_one_budget() {
    let (first, mut first_peer) = tokio::io::duplex(4096);
    let (second, mut second_peer) = tokio::io::duplex(4096);
    let source = OwnedSource::new([
        Box::new(first) as ServiceStream,
        Box::new(second) as ServiceStream,
    ]);
    let (result, (), ()) = tokio::time::timeout(OBSERVE, async {
        tokio::join!(
            observe_from_source(&source, route(), OBSERVE),
            async {
                let mut preface = [0; 24];
                first_peer.read_exact(&mut preface).await.unwrap();
                assert_eq!(&preface, crate::xpc::h2_raw::H2_PREFACE);
                drop(first_peer);
            },
            async {
                let mut preface = [0; 24];
                second_peer.read_exact(&mut preface).await.unwrap();
                assert_eq!(&preface, crate::xpc::h2_raw::H2_PREFACE);
                assert_eq!(read_frame(&mut second_peer).await.0, 4);
                assert_eq!(read_frame(&mut second_peer).await.0, 8);
                second_peer.write_all(&frame(4, 0, 0, &[])).await.unwrap();
                let ack = read_frame(&mut second_peer).await;
                assert_eq!((ack.0, ack.1, ack.2), (4, 1, 0));
                for (kind, stream) in [(1, 1), (0, 1), (1, 3), (0, 1), (0, 3)] {
                    let received = read_frame(&mut second_peer).await;
                    assert_eq!((received.0, received.2), (kind, stream));
                }
                reply(&mut second_peer, vec![wire(directory())]).await;
                let mut remaining = Vec::new();
                second_peer.read_to_end(&mut remaining).await.unwrap();
                assert_control_tail(&remaining);
            }
        )
    })
    .await
    .unwrap();
    assert_eq!(result.unwrap().udid(), Some(UDID));
    assert_eq!(source.dial_calls.load(Ordering::SeqCst), 2);
    assert_eq!(source.identity_calls.load(Ordering::SeqCst), 1);
}
