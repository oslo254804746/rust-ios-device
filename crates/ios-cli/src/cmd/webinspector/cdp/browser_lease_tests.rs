//! Actual admitted browser lease and removal paths over owned plist streams.
use super::super::{
    bridge_tests::{envelope, read_frame, send_json, strings, write_frame},
    connection::{ConnectionFuture, InspectorSource},
};
use super::lifecycle_tests::{
    app, no_more_requests, setup_before_target, source_with_delay, state, Source,
};
use super::*;
use crate::access::{
    test_support::{grant, PolicyFixture, DEVICE, REPLACEMENT, TOKEN},
    AccessControl, Permission,
};
use plist::{Dictionary, Value};
use std::{
    collections::VecDeque,
    future::Future,
    sync::{
        atomic::{AtomicU64, AtomicUsize, Ordering},
        Mutex as StdMutex,
    },
};
use tokio::{
    io::{AsyncWriteExt, DuplexStream},
    sync::oneshot,
};
use tokio_tungstenite::tungstenite::{client::IntoClientRequest, Message as ClientMessage};

type Socket =
    tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;
static PAGE: AtomicU64 = AtomicU64::new(24000);
const BOUND: Duration = Duration::from_secs(8);
async fn bounded<T>(operation: impl Future<Output = T>) -> T {
    tokio::time::timeout(BOUND, operation)
        .await
        .expect("owned browser lease fixture stalled")
}

struct QueueSource {
    remaining: StdMutex<VecDeque<Arc<Source>>>,
    calls: AtomicUsize,
}
impl InspectorSource for QueueSource {
    fn connect<'a>(&'a self, udid: &'a str, timeout: Duration) -> ConnectionFuture<'a> {
        Box::pin(async move {
            assert_eq!(udid, DEVICE);
            self.calls.fetch_add(1, Ordering::SeqCst);
            let source = self
                .remaining
                .lock()
                .unwrap()
                .pop_front()
                .expect("only expected owned fixture sources");
            source.connect("owned-cdp-device", timeout).await
        })
    }
}

struct Host {
    base: String,
    policy: PolicyFixture,
    shutdown: Option<oneshot::Sender<()>>,
    task: tokio::task::JoinHandle<()>,
}
impl Drop for Host {
    fn drop(&mut self) {
        self.task.abort();
    }
}
impl Host {
    async fn reload(&self) {
        self.policy.replace(vec![grant(
            REPLACEMENT,
            vec![Permission::Debug, Permission::ManageServer],
        )]);
        let response = bounded(
            reqwest::Client::builder()
                .no_proxy()
                .timeout(Duration::from_secs(3))
                .build()
                .unwrap()
                .post(format!("{}/auth/reload", self.base))
                .bearer_auth(TOKEN)
                .send(),
        )
        .await
        .unwrap();
        assert_eq!(response.status(), reqwest::StatusCode::OK);
    }
    async fn finish(mut self) {
        self.shutdown.take().unwrap().send(()).unwrap();
        bounded(&mut self.task).await.unwrap();
    }
}

type Observation = (oneshot::Receiver<()>, oneshot::Sender<Result<()>>);

async fn connected(
    source: Arc<QueueSource>,
    expiry: bool,
    observation: Option<Observation>,
) -> (Host, Socket) {
    let owner = grant(TOKEN, vec![Permission::Debug, Permission::ManageServer]);
    let policy = PolicyFixture::new(vec![if expiry {
        owner.expires_after(3)
    } else {
        owner
    }]);
    let access = AccessControl::from_file(policy.path.clone()).await.unwrap();
    let mut server = state(source, Duration::from_secs(1));
    server.udid = DEVICE.into();
    server.access = access;
    let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
        .await
        .unwrap();
    let port = listener.local_addr().unwrap().port();
    server.cdp_port = port;
    let (router, path) = if let Some((stop, result)) = observation {
        let stop = Arc::new(StdMutex::new(Some(stop)));
        let result = Arc::new(StdMutex::new(Some(result)));
        // Observe the actual handler's native result with an owned stop.
        // This private route makes no admission or principal revocation claim.
        let route = Router::new()
            .route(
                "/owned-browser",
                get(
                    move |ws: WebSocketUpgrade, State(server): State<ServerState>| {
                        let stop = stop.lock().unwrap().take().unwrap();
                        let result = result.lock().unwrap().take().unwrap();
                        async move {
                            ws.max_frame_size(MAX_CDP_CLIENT_MESSAGE_BYTES)
                                .max_message_size(MAX_CDP_CLIENT_MESSAGE_BYTES)
                                .on_upgrade(move |socket| async move {
                                    let error = handle_cdp_browser_socket(socket, &server, async {
                                        let _ = stop.await;
                                    })
                                    .await;
                                    let _ = result.send(error);
                                })
                        }
                    },
                ),
            )
            .with_state(server);
        (route, "/owned-browser")
    } else {
        (cdp_router(server), "/devtools/browser/ios-cli")
    };
    let (shutdown, stopped) = oneshot::channel();
    let task = tokio::spawn(async move {
        axum::serve(listener, router)
            .with_graceful_shutdown(async {
                let _ = stopped.await;
            })
            .await
            .unwrap();
    });
    let host = Host {
        base: format!("http://127.0.0.1:{port}"),
        policy,
        shutdown: Some(shutdown),
        task,
    };
    let mut request = format!("ws://127.0.0.1:{port}{path}")
        .into_client_request()
        .unwrap();
    request
        .headers_mut()
        .insert("authorization", format!("Bearer {TOKEN}").parse().unwrap());
    let (socket, _) = bounded(tokio_tungstenite::connect_async(request))
        .await
        .unwrap();
    (host, socket)
}

async fn discovery(peer: &mut DuplexStream, page: u64) {
    loop {
        let frame = read_frame(peer).await.unwrap().unwrap();
        match frame["__selector"].as_string().unwrap() {
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
                return;
            }
            other => panic!("unexpected discovery selector: {other}"),
        }
    }
}

struct Browser {
    host: Host,
    socket: Socket,
    source: Arc<QueueSource>,
    child: Arc<Source>,
    peer: DuplexStream,
    extra: Arc<Source>,
    extra_peer: DuplexStream,
    page: u64,
    session: String,
    sender: String,
}

async fn attached(expiry: bool) -> Browser {
    attached_with_observation(expiry, None).await
}

async fn attached_with_observation(expiry: bool, observation: Option<Observation>) -> Browser {
    let page = PAGE.fetch_add(1, Ordering::SeqCst);
    let (initial, mut first_peer) = source_with_delay(Duration::ZERO);
    let (child, mut peer) = source_with_delay(Duration::ZERO);
    // A periodic discovery may start before expiry. It owns an actual stream
    // while pending; no detached service worker or invented response is used.
    let (extra, extra_peer) = source_with_delay(Duration::from_secs(4));
    let source = Arc::new(QueueSource {
        remaining: StdMutex::new(VecDeque::from([
            initial.clone(),
            child.clone(),
            extra.clone(),
        ])),
        calls: AtomicUsize::new(0),
    });
    let (host, mut socket) = connected(source.clone(), expiry, observation).await;
    bounded(discovery(&mut first_peer, page)).await;
    no_more_requests(&mut first_peer).await;
    assert!(initial.dropped.load(Ordering::SeqCst));
    socket.send(ClientMessage::Text(json!({"id":1,"method":"Target.setAutoAttach","params":{"autoAttach":true,"flatten":true}}).to_string())).await.unwrap();
    bounded(setup_before_target(&mut peer, page)).await;
    send_json(&mut peer,json!({"method":"Target.targetCreated","params":{"targetInfo":{"targetId":"owned-cdp-target"}}})).await.unwrap();
    let session = bounded(async {
        loop {
            let message = socket.next().await.unwrap().unwrap().into_text().unwrap();
            let value: JsonValue = serde_json::from_str(&message).unwrap();
            if value["method"] == "Target.attachedToTarget" {
                assert_eq!(
                    value["params"]["targetInfo"]["targetId"],
                    format!("PID:42:page-{page}")
                );
                break value["params"]["sessionId"].as_str().unwrap().to_owned();
            }
            assert_eq!(value, json!({"id":1,"result":{}}));
        }
    })
    .await;
    socket
        .send(ClientMessage::Text(
            json!({"id":7,"sessionId":session,"method":"Runtime.enable"}).to_string(),
        ))
        .await
        .unwrap();
    let forwarded = bounded(read_frame(&mut peer)).await.unwrap().unwrap();
    assert_eq!(
        forwarded["__selector"].as_string(),
        Some("_rpc_forwardSocketData:")
    );
    let sender = forwarded["__argument"].as_dictionary().unwrap()["WIRSenderKey"]
        .as_string()
        .unwrap()
        .to_owned();
    assert_eq!(source.calls.load(Ordering::SeqCst), 2);
    Browser {
        host,
        socket,
        source,
        child,
        peer,
        extra,
        extra_peer,
        page,
        session,
        sender,
    }
}

async fn close(browser: &mut Browser) {
    let frame = bounded(read_frame(&mut browser.peer))
        .await
        .unwrap()
        .expect("attached browser child must close on revocation");
    assert_eq!(
        frame["__selector"].as_string(),
        Some("_rpc_forwardDidClose:")
    );
    let argument = frame["__argument"].as_dictionary().unwrap();
    assert_eq!(
        argument["WIRApplicationIdentifierKey"].as_string(),
        Some("PID:42")
    );
    assert_eq!(
        argument["WIRPageIdentifierKey"].as_unsigned_integer(),
        Some(browser.page)
    );
    assert_eq!(
        argument["WIRSenderKey"].as_string(),
        Some(browser.sender.as_str())
    );
}

async fn released(mut browser: Browser) {
    no_more_requests(&mut browser.peer).await;
    assert!(browser.child.dropped.load(Ordering::SeqCst));
    assert_eq!(browser.child.calls.load(Ordering::SeqCst), 1);
    drop(
        bounded(lock_cdp_page(
            &format!("PID:42:page-{}", browser.page),
            Duration::from_millis(100),
        ))
        .await
        .unwrap(),
    );
    assert!(matches!(
        bounded(browser.socket.next()).await,
        None | Some(Err(_)) | Some(Ok(ClientMessage::Close(_)))
    ));
    if browser.extra.calls.load(Ordering::SeqCst) > 0 {
        assert_eq!(browser.extra.calls.load(Ordering::SeqCst), 1);
        no_more_requests(&mut browser.extra_peer).await;
        assert!(browser.extra.dropped.load(Ordering::SeqCst));
    }
    assert!(browser.source.calls.load(Ordering::SeqCst) <= 3);
    drop(browser.socket);
    browser.host.finish().await;
}

#[tokio::test]
async fn lifecycle_cdp_browser_lease_reload_closes_attached_child() {
    let mut browser = attached(false).await;
    browser.host.reload().await;
    close(&mut browser).await;
    released(browser).await;
}

#[tokio::test]
async fn lifecycle_cdp_browser_lease_expiry_closes_attached_child() {
    let mut browser = attached(true).await;
    close(&mut browser).await;
    released(browser).await;
}

#[tokio::test]
async fn lifecycle_cdp_browser_lease_reload_preserves_delayed_flush() {
    let mut browser = attached(false).await;
    let before = browser.child.flush_completed.load(Ordering::SeqCst);
    browser.child.flush_fault.store(3, Ordering::SeqCst);
    browser.host.reload().await;
    close(&mut browser).await;
    no_more_requests(&mut browser.peer).await;
    assert_eq!(
        browser.child.flush_completed.load(Ordering::SeqCst),
        before + 1
    );
    released(browser).await;
}

#[tokio::test]
async fn lifecycle_cdp_browser_lease_reload_after_observed_partial_frame_skips_close() {
    let mut browser = attached(false).await;
    let before = browser.child.received.load(Ordering::SeqCst);
    browser.peer.write_all(&[0, 0]).await.unwrap();
    bounded(async {
        while browser.child.received.load(Ordering::SeqCst) != before + 2 {
            tokio::task::yield_now().await;
        }
    })
    .await;
    browser.host.reload().await;
    released(browser).await;
}

#[tokio::test]
async fn lifecycle_cdp_browser_lease_reload_awaits_one_real_cleanup_total() {
    let mut browser = attached(false).await;
    browser.child.flush_fault.store(2, Ordering::SeqCst);
    let began = tokio::time::Instant::now();
    browser.host.reload().await;
    close(&mut browser).await;
    no_more_requests(&mut browser.peer).await;
    assert!(began.elapsed() >= CDP_PAGE_CLEANUP_TIMEOUT);
    released(browser).await;
}

#[tokio::test]
async fn lifecycle_cdp_browser_lease_reload_during_detach_keeps_retiring_child() {
    struct ReleaseFlush(Arc<Source>);
    impl Drop for ReleaseFlush {
        fn drop(&mut self) {
            self.0.flush_gate.release();
        }
    }
    let mut browser = attached(false).await;
    let before = browser.child.flush_completed.load(Ordering::SeqCst);
    // Keep the original flush pending until the actual reload has completed.
    // An 80ms timer can finish (and legitimately reply to detach) before HTTP
    // reload on a busy host, which does not exercise cancellation during detach.
    browser.child.flush_fault.store(4, Ordering::SeqCst);
    let _release = ReleaseFlush(browser.child.clone());
    browser.socket.send(ClientMessage::Text(json!({"id":9,"method":"Target.detachFromTarget","params":{"sessionId":browser.session}}).to_string())).await.unwrap();
    close(&mut browser).await;
    assert_eq!(
        browser.child.flush_completed.load(Ordering::SeqCst),
        before,
        "reload must interrupt still-pending detach cleanup"
    );
    browser.host.reload().await;
    assert_eq!(browser.child.flush_completed.load(Ordering::SeqCst), before);
    browser.child.flush_gate.release();
    no_more_requests(&mut browser.peer).await;
    assert_eq!(
        browser.child.flush_completed.load(Ordering::SeqCst),
        before + 1
    );
    released(browser).await;
}

#[tokio::test]
async fn lifecycle_cdp_browser_lease_reload_during_initial_source_releases_before_return() {
    let (pending, mut peer) = source_with_delay(Duration::from_secs(4));
    let source = Arc::new(QueueSource {
        remaining: StdMutex::new(VecDeque::from([pending.clone()])),
        calls: AtomicUsize::new(0),
    });
    let (host, mut socket) = connected(source.clone(), false, None).await;
    bounded(async {
        while pending.calls.load(Ordering::SeqCst) == 0 {
            tokio::task::yield_now().await;
        }
    })
    .await;
    assert!(!pending.returned.load(Ordering::SeqCst));
    host.reload().await;
    no_more_requests(&mut peer).await;
    assert!(pending.dropped.load(Ordering::SeqCst));
    assert_eq!(pending.received.load(Ordering::SeqCst), 0);
    assert_eq!(source.calls.load(Ordering::SeqCst), 1);
    assert!(matches!(
        bounded(socket.next()).await,
        None | Some(Err(_)) | Some(Ok(ClientMessage::Close(_)))
    ));
    drop(socket);
    host.finish().await;
}

#[tokio::test]
async fn lifecycle_cdp_browser_lease_reload_during_child_source_releases_unpublished_owner() {
    let page = PAGE.fetch_add(1, Ordering::SeqCst);
    let (initial, mut first_peer) = source_with_delay(Duration::ZERO);
    let (pending, mut peer) = source_with_delay(Duration::from_secs(4));
    let source = Arc::new(QueueSource {
        remaining: StdMutex::new(VecDeque::from([initial.clone(), pending.clone()])),
        calls: AtomicUsize::new(0),
    });
    let (host, mut socket) = connected(source.clone(), false, None).await;
    bounded(discovery(&mut first_peer, page)).await;
    no_more_requests(&mut first_peer).await;
    assert!(initial.dropped.load(Ordering::SeqCst));
    socket.send(ClientMessage::Text(json!({"id":1,"method":"Target.setAutoAttach","params":{"autoAttach":true,"flatten":true}}).to_string())).await.unwrap();
    let reply = bounded(socket.next())
        .await
        .unwrap()
        .unwrap()
        .into_text()
        .unwrap();
    assert_eq!(
        serde_json::from_str::<JsonValue>(&reply).unwrap(),
        json!({"id":1,"result":{}})
    );
    bounded(async {
        while pending.calls.load(Ordering::SeqCst) == 0 {
            tokio::task::yield_now().await;
        }
    })
    .await;
    assert!(!pending.returned.load(Ordering::SeqCst));
    host.reload().await;
    no_more_requests(&mut peer).await;
    assert!(pending.dropped.load(Ordering::SeqCst));
    assert_eq!(pending.received.load(Ordering::SeqCst), 0);
    assert_eq!(source.calls.load(Ordering::SeqCst), 2);
    drop(
        bounded(lock_cdp_page(
            &format!("PID:42:page-{page}"),
            Duration::from_millis(100),
        ))
        .await
        .unwrap(),
    );
    assert!(matches!(
        bounded(socket.next()).await,
        None | Some(Err(_)) | Some(Ok(ClientMessage::Close(_)))
    ));
    drop(socket);
    host.finish().await;
}

#[tokio::test]
async fn lifecycle_cdp_browser_lease_owned_stop_keeps_cancelled_primary_and_native_cleanup_io() {
    let (cancel, stopped) = oneshot::channel();
    let (result, received) = oneshot::channel();
    let mut browser = attached_with_observation(false, Some((stopped, result))).await;
    browser.child.flush_fault.store(1, Ordering::SeqCst);
    cancel.send(()).unwrap();
    let error = bounded(received).await.unwrap().unwrap_err();
    let both = error
        .downcast_ref::<super::super::tasks::control::CleanupFailure>()
        .expect("owner retains cancellation plus native cleanup");
    assert_eq!(
        ios_core::classify_error(error.as_ref()),
        ios_core::ErrorClass::Cancelled
    );
    assert!(both
        .primary
        .chain()
        .any(|source| source.is::<ios_core::OperationCancelled>()));
    assert_eq!(
        ios_core::classify_error(both.cleanup.as_ref()),
        ios_core::ErrorClass::PermissionDenied
    );
    assert!(both.cleanup.chain().any(|source| source
        .downcast_ref::<std::io::Error>()
        .is_some_and(|native| native.kind() == std::io::ErrorKind::PermissionDenied)));
    close(&mut browser).await;
    released(browser).await;
}
