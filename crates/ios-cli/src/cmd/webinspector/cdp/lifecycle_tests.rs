//! Actual private CDP page bridge over an owned WebSocket and plist service.
use super::super::{
    bridge_tests::{envelope, read_frame, send_json, strings, write_frame},
    connection::{ConnectionFuture, InspectorSource},
    sessions::Sessions,
};
use super::*;
use crate::access::AccessControl;
use ios_core::{classify_error, device::ServiceStream, ErrorClass};
use plist::{Dictionary, Value};
use std::{
    future::Future,
    pin::Pin,
    sync::{
        atomic::{AtomicBool, AtomicU64, AtomicU8, AtomicUsize, Ordering},
        Mutex as StdMutex,
    },
    task::{Context, Poll},
};
use tokio::{
    io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt, DuplexStream, ReadBuf},
    net::TcpListener,
    sync::oneshot,
};

const REQUEST: Duration = Duration::from_millis(50);
const BOUND: Duration = Duration::from_secs(4);
static PAGE: AtomicU64 = AtomicU64::new(1000);

async fn bounded<T>(future: impl Future<Output = T>) -> T {
    tokio::time::timeout(BOUND, future)
        .await
        .expect("owned CDP fixture stalled")
}

struct OwnedStream {
    inner: DuplexStream,
    dropped: Arc<AtomicBool>,
    stall: Arc<AtomicBool>,
    blocked: Arc<AtomicUsize>,
    flush_fault: Arc<AtomicU8>,
    flush_completed: Arc<AtomicUsize>,
    flush_delay: Option<Pin<Box<tokio::time::Sleep>>>,
}
impl Drop for OwnedStream {
    fn drop(&mut self) {
        self.dropped.store(true, Ordering::SeqCst);
    }
}
impl AsyncRead for OwnedStream {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<std::io::Result<()>> {
        Pin::new(&mut self.inner).poll_read(cx, buf)
    }
}
impl AsyncWrite for OwnedStream {
    fn poll_write(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        bytes: &[u8],
    ) -> Poll<std::io::Result<usize>> {
        if self.stall.load(Ordering::SeqCst) {
            self.blocked.fetch_add(1, Ordering::SeqCst);
            return Poll::Pending;
        }
        Pin::new(&mut self.inner).poll_write(cx, bytes)
    }
    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        match self.flush_fault.load(Ordering::SeqCst) {
            1 => return Poll::Ready(Err(std::io::ErrorKind::PermissionDenied.into())),
            2 => return Poll::Pending,
            3 => {
                let delay = self
                    .flush_delay
                    .get_or_insert_with(|| Box::pin(tokio::time::sleep(Duration::from_millis(80))));
                if delay.as_mut().poll(cx).is_pending() {
                    return Poll::Pending;
                }
            }
            _ => {}
        }
        let result = Pin::new(&mut self.inner).poll_flush(cx);
        if matches!(&result, Poll::Ready(Ok(()))) {
            self.flush_completed.fetch_add(1, Ordering::SeqCst);
        }
        result
    }
    fn poll_shutdown(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        Pin::new(&mut self.inner).poll_shutdown(cx)
    }
}

pub(in crate::cmd::webinspector) struct Source {
    stream: StdMutex<Option<ServiceStream>>,
    pub(in crate::cmd::webinspector) calls: AtomicUsize,
    pub(in crate::cmd::webinspector) dropped: Arc<AtomicBool>,
    pub(in crate::cmd::webinspector) returned: AtomicBool,
    connect_delay: Duration,
    pub(in crate::cmd::webinspector) stall: Arc<AtomicBool>,
    pub(in crate::cmd::webinspector) blocked: Arc<AtomicUsize>,
    pub(in crate::cmd::webinspector) flush_fault: Arc<AtomicU8>,
    pub(in crate::cmd::webinspector) flush_completed: Arc<AtomicUsize>,
}
impl InspectorSource for Source {
    fn connect<'a>(&'a self, udid: &'a str, _timeout: Duration) -> ConnectionFuture<'a> {
        Box::pin(async move {
            assert_eq!(udid, "owned-cdp-device");
            self.calls.fetch_add(1, Ordering::SeqCst);
            let stream = self
                .stream
                .lock()
                .unwrap()
                .take()
                .expect("one owned connection");
            if !self.connect_delay.is_zero() {
                tokio::time::sleep(self.connect_delay).await;
            }
            self.returned.store(true, Ordering::SeqCst);
            Ok(InspectorConnection {
                device: None,
                stream,
            })
        })
    }
}

fn source() -> (Arc<Source>, DuplexStream) {
    source_with_delay(Duration::ZERO)
}

pub(in crate::cmd::webinspector) fn source_with_delay(
    connect_delay: Duration,
) -> (Arc<Source>, DuplexStream) {
    let (stream, peer) = tokio::io::duplex(16 * 1024);
    let dropped = Arc::new(AtomicBool::new(false));
    let stall = Arc::new(AtomicBool::new(false));
    let blocked = Arc::new(AtomicUsize::new(0));
    let flush_fault = Arc::new(AtomicU8::new(0));
    let flush_completed = Arc::new(AtomicUsize::new(0));
    (
        Arc::new(Source {
            stream: StdMutex::new(Some(Box::new(OwnedStream {
                inner: stream,
                dropped: dropped.clone(),
                stall: stall.clone(),
                blocked: blocked.clone(),
                flush_fault: flush_fault.clone(),
                flush_completed: flush_completed.clone(),
                flush_delay: None,
            }))),
            calls: AtomicUsize::new(0),
            returned: AtomicBool::new(false),
            connect_delay,
            dropped,
            stall,
            blocked,
            flush_fault,
            flush_completed,
        }),
        peer,
    )
}

pub(super) struct Server {
    pub(super) port: u16,
    pub(super) result: oneshot::Receiver<Result<()>>,
    shutdown: Option<oneshot::Sender<()>>,
    task: tokio::task::JoinHandle<()>,
}
impl Drop for Server {
    fn drop(&mut self) {
        self.task.abort();
    }
}
impl Server {
    pub(super) async fn finish(mut self) {
        self.shutdown.take().unwrap().send(()).unwrap();
        bounded(&mut self.task).await.unwrap();
    }
}

pub(in crate::cmd::webinspector) fn state(
    source: Arc<dyn InspectorSource>,
    timeout: Duration,
) -> ServerState {
    ServerState {
        udid: "owned-cdp-device".into(),
        timeout,
        cdp_host: "127.0.0.1".into(),
        cdp_port: 0,
        access: AccessControl::trusted_test_local(),
        selenium_sessions: Sessions::default(),
        source,
    }
}

pub(super) async fn start(source: Arc<Source>, page: u64) -> Server {
    start_with_timeout(source, page, REQUEST).await
}

pub(super) async fn start_with_timeout(
    source: Arc<Source>,
    page: u64,
    timeout: Duration,
) -> Server {
    let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let mut state = state(source, timeout);
    state.cdp_port = port;
    let (tx, result) = oneshot::channel();
    let tx = Arc::new(StdMutex::new(Some(tx)));
    // This cfg(test) route observes the actual private page loop's returned
    // native error. It adds no product route or source-selection argument and
    // makes no new authorization/revocation claim.
    let router = Router::new().route(
        "/owned-page",
        get(move |ws: WebSocketUpgrade| {
            let state = state.clone();
            let tx = tx.lock().unwrap().take().expect("one owned upgrade");
            async move {
                ws.on_upgrade(move |socket| async move {
                    let result =
                        handle_cdp_socket(socket, &state, &format!("PID:42:page-{page}")).await;
                    let _ = tx.send(result);
                })
            }
        }),
    );
    let (shutdown, stop) = oneshot::channel();
    let task = tokio::spawn(async move {
        axum::serve(listener, router)
            .with_graceful_shutdown(async {
                let _ = stop.await;
            })
            .await
            .unwrap();
    });
    Server {
        port,
        result,
        shutdown: Some(shutdown),
        task,
    }
}

pub(in crate::cmd::webinspector) fn app() -> Dictionary {
    let mut app = strings(&[
        ("WIRApplicationIdentifierKey", "PID:42"),
        (
            "WIRApplicationBundleIdentifierKey",
            "com.apple.mobilesafari",
        ),
        ("WIRApplicationNameKey", "Safari"),
        (
            "WIRAutomationAvailabilityKey",
            "WIRAutomationAvailabilityAvailable",
        ),
    ]);
    for (key, value) in [
        ("WIRIsApplicationActiveKey", true),
        ("WIRIsApplicationProxyKey", false),
        ("WIRIsApplicationReadyKey", true),
    ] {
        app.insert(key.into(), Value::Boolean(value));
    }
    app
}

async fn setup(peer: &mut DuplexStream, page: u64) {
    setup_before_target(peer, page).await;
    send_json(peer, json!({"method":"Target.targetCreated","params":{"targetInfo":{"targetId":"owned-cdp-target"}}})).await.unwrap();
}

pub(super) async fn setup_before_target(peer: &mut DuplexStream, page: u64) {
    loop {
        let message = read_frame(peer)
            .await
            .unwrap()
            .expect("complete setup request");
        let selector = message["__selector"].as_string().unwrap();
        let argument = message["__argument"].as_dictionary().unwrap();
        match selector {
            "_rpc_reportIdentifier:" => write_frame(
                peer,
                envelope(
                    "_rpc_reportCurrentState:",
                    strings(&[(
                        "WIRAutomationAvailabilityKey",
                        "WIRAutomationAvailabilityAvailable",
                    )]),
                ),
            )
            .await
            .unwrap(),
            "_rpc_getConnectedApplications:" => write_frame(
                peer,
                envelope(
                    "_rpc_reportConnectedApplicationList:",
                    Dictionary::from_iter([(
                        "WIRApplicationDictionaryKey".to_owned(),
                        Value::Dictionary(Dictionary::from_iter([(
                            "PID:42".to_owned(),
                            Value::Dictionary(app()),
                        )])),
                    )]),
                ),
            )
            .await
            .unwrap(),
            "_rpc_forwardGetListing:" => {
                let mut entry = strings(&[
                    ("WIRTypeKey", "WIRTypeWebPage"),
                    ("WIRTitleKey", "Owned page"),
                    ("WIRURLKey", "https://example.invalid/"),
                ]);
                entry.insert("WIRPageIdentifierKey".into(), Value::Integer(page.into()));
                let mut listing = strings(&[("WIRApplicationIdentifierKey", "PID:42")]);
                listing.insert(
                    "WIRListingKey".into(),
                    Value::Dictionary(Dictionary::from_iter([(
                        format!("page-{page}"),
                        Value::Dictionary(entry),
                    )])),
                );
                write_frame(peer, envelope("_rpc_applicationSentListing:", listing))
                    .await
                    .unwrap();
            }
            "_rpc_forwardSocketSetup:" => {
                assert_eq!(
                    argument["WIRPageIdentifierKey"].as_unsigned_integer(),
                    Some(page)
                );
                assert_eq!(argument["WIRAutomaticallyPause"].as_boolean(), Some(false));
                return;
            }
            other => panic!("unexpected actual setup selector: {other}"),
        }
    }
}

pub(in crate::cmd::webinspector) async fn no_more_requests(peer: &mut DuplexStream) {
    let mut bytes = Vec::new();
    bounded(peer.read_to_end(&mut bytes)).await.unwrap();
    assert!(
        bytes.is_empty(),
        "failed raw read must not issue teardown or retry bytes"
    );
}

async fn partial(header: bool) {
    let page = PAGE.fetch_add(1, Ordering::SeqCst);
    let (source, mut peer) = source();
    let mut server = start(source.clone(), page).await;
    let (socket, _) = bounded(tokio_tungstenite::connect_async(format!(
        "ws://127.0.0.1:{}/owned-page",
        server.port
    )))
    .await
    .unwrap();
    bounded(setup(&mut peer, page)).await;
    if header {
        peer.write_all(&1024u32.to_be_bytes()[..2]).await.unwrap();
    } else {
        peer.write_all(&1024u32.to_be_bytes()).await.unwrap();
        peer.write_all(b"<?xml").await.unwrap();
    }
    let error = bounded(&mut server.result).await.unwrap().unwrap_err();
    assert_eq!(classify_error(error.as_ref()), ErrorClass::Timeout);
    assert!(error
        .chain()
        .any(|source| source.is::<tokio::time::error::Elapsed>()));
    no_more_requests(&mut peer).await;
    assert!(source.dropped.load(Ordering::SeqCst));
    assert_eq!(source.calls.load(Ordering::SeqCst), 1);
    drop(socket);
    server.finish().await;
}

#[tokio::test]
async fn lifecycle_cdp_page_partial_header_retains_primary_elapsed() {
    partial(true).await;
}

#[tokio::test]
async fn lifecycle_cdp_page_partial_body_retains_primary_elapsed() {
    partial(false).await;
}

#[tokio::test]
async fn lifecycle_cdp_page_blocked_automatic_listing_retains_primary_elapsed() {
    let page = PAGE.fetch_add(1, Ordering::SeqCst);
    let (source, mut peer) = source();
    let mut server = start(source.clone(), page).await;
    let (socket, _) = bounded(tokio_tungstenite::connect_async(format!(
        "ws://127.0.0.1:{}/owned-page",
        server.port
    )))
    .await
    .unwrap();
    bounded(setup(&mut peer, page)).await;
    source.stall.store(true, Ordering::SeqCst);
    write_frame(
        &mut peer,
        envelope(
            "_rpc_reportConnectedApplicationList:",
            Dictionary::from_iter([(
                "WIRApplicationDictionaryKey".to_owned(),
                Value::Dictionary(Dictionary::from_iter([(
                    "PID:42".to_owned(),
                    Value::Dictionary(app()),
                )])),
            )]),
        ),
    )
    .await
    .unwrap();
    let error = bounded(&mut server.result).await.unwrap().unwrap_err();
    assert_eq!(classify_error(error.as_ref()), ErrorClass::Timeout);
    assert!(error
        .chain()
        .any(|source| source.is::<tokio::time::error::Elapsed>()));
    assert!(
        source.blocked.load(Ordering::SeqCst) > 0,
        "actual routing must attempt an automatic listing write"
    );
    no_more_requests(&mut peer).await;
    assert!(source.dropped.load(Ordering::SeqCst));
    drop(socket);
    server.finish().await;
}

#[tokio::test]
async fn lifecycle_cdp_page_empty_idle_resumes_and_tears_down_on_normal_close() {
    let page = PAGE.fetch_add(1, Ordering::SeqCst);
    let (source, mut peer) = source();
    let mut server = start(source.clone(), page).await;
    let (mut socket, _) = bounded(tokio_tungstenite::connect_async(format!(
        "ws://127.0.0.1:{}/owned-page",
        server.port
    )))
    .await
    .unwrap();
    bounded(setup(&mut peer, page)).await;
    // Multiple complete empty idle caps must not end a healthy listener.
    tokio::time::sleep(REQUEST * 3).await;
    assert!(matches!(
        server.result.try_recv(),
        Err(oneshot::error::TryRecvError::Empty)
    ));
    send_json(
        &mut peer,
        json!({"method":"Target.dispatchMessageFromTarget","params":{
            "targetId":"owned-cdp-target",
            "message":json!({"id":17,"result":{"value":42}}).to_string()
        }}),
    )
    .await
    .unwrap();
    let reply = bounded(socket.next())
        .await
        .unwrap()
        .unwrap()
        .into_text()
        .unwrap();
    assert_eq!(
        serde_json::from_str::<JsonValue>(&reply).unwrap(),
        json!({"id":17,"result":{"value":42}})
    );
    socket.close(None).await.unwrap();
    bounded(&mut server.result).await.unwrap().unwrap();
    let close = bounded(read_frame(&mut peer)).await.unwrap().unwrap();
    assert_eq!(
        close["__selector"].as_string(),
        Some("_rpc_forwardDidClose:")
    );
    assert!(source.dropped.load(Ordering::SeqCst));
    drop(socket);
    server.finish().await;
}

#[tokio::test]
async fn lifecycle_cdp_page_native_eof_codec_and_size_failures_remain_terminal() {
    for kind in 0..3 {
        let page = PAGE.fetch_add(1, Ordering::SeqCst);
        let (source, mut peer) = source();
        let mut server = start(source.clone(), page).await;
        let (socket, _) = bounded(tokio_tungstenite::connect_async(format!(
            "ws://127.0.0.1:{}/owned-page",
            server.port
        )))
        .await
        .unwrap();
        bounded(setup(&mut peer, page)).await;
        match kind {
            0 => peer.shutdown().await.unwrap(),
            1 => peer
                .write_all(&[0, 0, 0, 3, b'b', b'a', b'd'])
                .await
                .unwrap(),
            2 => peer
                .write_all(&(16u32 * 1024 * 1024 + 1).to_be_bytes())
                .await
                .unwrap(),
            _ => unreachable!(),
        }
        let error = bounded(&mut server.result).await.unwrap().unwrap_err();
        match kind {
            0 => {
                assert_eq!(classify_error(error.as_ref()), ErrorClass::Io);
                assert!(error.chain().any(|source| source
                    .downcast_ref::<std::io::Error>()
                    .is_some_and(|native| native.kind() == std::io::ErrorKind::UnexpectedEof)));
            }
            1 => {
                assert_eq!(classify_error(error.as_ref()), ErrorClass::Io);
                assert!(error.chain().any(|source| source.is::<plist::Error>()));
            }
            2 => assert_eq!(classify_error(error.as_ref()), ErrorClass::Protocol),
            _ => unreachable!(),
        }
        no_more_requests(&mut peer).await;
        assert!(source.dropped.load(Ordering::SeqCst));
        assert_eq!(source.calls.load(Ordering::SeqCst), 1);
        drop(socket);
        server.finish().await;
    }
}
