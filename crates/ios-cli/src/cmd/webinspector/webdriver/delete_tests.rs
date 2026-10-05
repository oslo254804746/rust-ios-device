//! Actual admitted WebDriver DELETE over the existing owned Automation peer.
use super::super::{
    cdp::lifecycle_tests::state,
    connection::{ConnectionFuture, InspectorSource},
    tasks::test_peer::{self, FlushFault, PeerPolicy, Trace},
};
use super::*;
use crate::access::{
    test_support::{grant, PolicyFixture, DEVICE, TOKEN},
    AccessControl, HttpBoundaryArgs, Permission,
};
use std::sync::atomic::Ordering;
use tokio::sync::oneshot;

struct AuthorizedSource(Arc<super::cleanup_tests::Source>);
impl InspectorSource for AuthorizedSource {
    fn connect<'a>(&'a self, udid: &'a str, timeout: Duration) -> ConnectionFuture<'a> {
        assert_eq!(udid, DEVICE);
        self.0.connect("owned-cdp-device", timeout)
    }
}

struct Server {
    base: String,
    state: ServerState,
    source: Arc<super::cleanup_tests::Source>,
    session: String,
    _policy: PolicyFixture,
    shutdown: Option<oneshot::Sender<()>>,
    task: tokio::task::JoinHandle<()>,
    peer: tokio::task::JoinHandle<Result<Trace>>,
}
impl Drop for Server {
    fn drop(&mut self) {
        self.task.abort();
        self.peer.abort();
    }
}
fn client() -> reqwest::Client {
    reqwest::Client::builder()
        .no_proxy()
        .timeout(Duration::from_secs(3))
        .build()
        .unwrap()
}

async fn response_body(response: reqwest::Response) -> JsonValue {
    serde_json::from_slice(&response.bytes().await.unwrap()).unwrap()
}
impl Server {
    async fn delete(&self) -> reqwest::Response {
        test_peer::bounded(
            client()
                .delete(format!("{}/session/{}", self.base, self.session))
                .bearer_auth(TOKEN)
                .send(),
        )
        .await
        .unwrap()
    }
    async fn finish(mut self) -> Trace {
        let trace = test_peer::bounded(&mut self.peer).await.unwrap().unwrap();
        assert!(self.source.dropped.load(Ordering::SeqCst));
        assert_eq!(self.source.calls.load(Ordering::SeqCst), 1);
        self.shutdown.take().unwrap().send(()).unwrap();
        test_peer::bounded(&mut self.task).await.unwrap();
        trace
    }
}

async fn opened(fault: FlushFault, policy: PeerPolicy) -> Server {
    let (source, stream) = super::cleanup_tests::fixture(fault);
    let peer = tokio::spawn(test_peer::peer(stream, policy));
    let fixture = PolicyFixture::new(vec![grant(TOKEN, vec![Permission::Debug])]);
    let access = AccessControl::from_file(fixture.path.clone())
        .await
        .unwrap();
    let mut state = state(
        Arc::new(AuthorizedSource(source.clone())),
        Duration::from_secs(2),
    );
    state.udid = DEVICE.into();
    state.access = access;
    let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
        .await
        .unwrap();
    let bound = listener.local_addr().unwrap();
    state.cdp_port = bound.port();
    let router = HttpBoundaryArgs::default()
        .validate()
        .unwrap()
        .for_listener(bound)
        .protect(
            selenium_router(state.clone()),
            crate::http_diagnostics::ErrorShape::WebDriver,
        );
    let (shutdown, stopped) = oneshot::channel();
    let task = tokio::spawn(async move {
        axum::serve(listener, router)
            .with_graceful_shutdown(async {
                let _ = stopped.await;
            })
            .await
            .unwrap();
    });
    let base = format!("http://127.0.0.1:{}", bound.port());
    let response = test_peer::bounded(
        client()
            .post(format!("{base}/session"))
            .bearer_auth(TOKEN)
            .send(),
    )
    .await
    .unwrap();
    assert_eq!(response.status(), reqwest::StatusCode::OK);
    let body = response_body(response).await;
    let session = body["value"]["sessionId"].as_str().unwrap().to_owned();
    assert!(state.selenium_sessions.get(&session).await.is_some());
    Server {
        base,
        state,
        source,
        session,
        _policy: fixture,
        shutdown: Some(shutdown),
        task,
        peer,
    }
}

async fn failed(server: &Server, response: reqwest::Response, class: &str) {
    assert_eq!(
        response.status(),
        reqwest::StatusCode::INTERNAL_SERVER_ERROR
    );
    assert_eq!(response.headers()["x-ios-error-schema-version"], "1");
    assert_eq!(response.headers()["x-ios-error-class"], class);
    assert_eq!(response.headers()["x-ios-error-phase"], "webinspector");
    assert_eq!(response.headers()["x-ios-error-effects"], "unknown");
    let body = response_body(response).await;
    assert_eq!(body["value"]["error"], "unknown error");
    let text = body.to_string();
    for secret in [DEVICE, TOKEN, server.session.as_str()] {
        assert!(!text.contains(secret));
    }
    assert!(server
        .state
        .selenium_sessions
        .get(&server.session)
        .await
        .is_none());
    let repeated = server.delete().await;
    assert_eq!(repeated.status(), reqwest::StatusCode::OK);
    assert!(!repeated.headers().contains_key("x-ios-error-class"));
    assert_eq!(response_body(repeated).await, json!({"value":null}));
}

fn completed(trace: Trace, teardowns: usize) {
    assert_eq!(
        trace.methods,
        [
            "Automation.createBrowsingContext",
            "Automation.closeBrowsingContext"
        ]
    );
    assert_eq!(trace.command_ids, [1, 2]);
    assert_eq!(trace.teardowns, teardowns);
}

#[tokio::test]
async fn lifecycle_webdriver_delete_normal_and_repeat_keep_success_contract() {
    let server = opened(FlushFault::None, PeerPolicy::default()).await;
    for _ in 0..2 {
        let response = server.delete().await;
        assert_eq!(response.status(), reqwest::StatusCode::OK);
        assert!(!response.headers().contains_key("x-ios-error-class"));
        assert_eq!(response_body(response).await, json!({"value":null}));
        assert!(server
            .state
            .selenium_sessions
            .get(&server.session)
            .await
            .is_none());
    }
    completed(server.finish().await, 1);
}

#[tokio::test]
async fn lifecycle_webdriver_delete_native_flush_error_returns_machine_class() {
    let server = opened(FlushFault::TeardownIo, PeerPolicy::default()).await;
    failed(&server, server.delete().await, "permission_denied").await;
    completed(server.finish().await, 1);
}

#[tokio::test]
async fn lifecycle_webdriver_delete_pending_flush_uses_real_default_total() {
    let server = opened(FlushFault::TeardownStall, PeerPolicy::default()).await;
    let began = tokio::time::Instant::now();
    failed(&server, server.delete().await, "timeout").await;
    assert!(began.elapsed() >= SESSION_CLEANUP_MAX);
    completed(server.finish().await, 1);
}

#[tokio::test]
async fn lifecycle_webdriver_delete_keeps_business_primary_after_cleanup_expiry() {
    let server = opened(
        FlushFault::TeardownStall,
        PeerPolicy {
            stop_error: true,
            ..PeerPolicy::default()
        },
    )
    .await;
    failed(&server, server.delete().await, "protocol").await;
    completed(server.finish().await, 1);
}

#[tokio::test]
async fn lifecycle_webdriver_delete_mutex_wait_expires_before_any_cleanup_request() {
    let server = opened(FlushFault::None, PeerPolicy::default()).await;
    let runtime = server
        .state
        .selenium_sessions
        .get(&server.session)
        .await
        .unwrap();
    let guard = runtime.clone().lock_owned().await;
    let began = tokio::time::Instant::now();
    failed(&server, server.delete().await, "timeout").await;
    assert!(began.elapsed() >= SESSION_CLEANUP_MAX);
    assert!(
        guard.cleanup_armed,
        "cleanup never acquired this owned mutex"
    );
    assert!(!server.source.dropped.load(Ordering::SeqCst));
    drop(guard);
    drop(runtime);
    let trace = server.finish().await;
    assert_eq!(trace.methods, ["Automation.createBrowsingContext"]);
    assert_eq!(trace.command_ids, [1]);
    assert_eq!(trace.teardowns, 0);
}

#[tokio::test]
async fn lifecycle_webdriver_delete_mutex_time_reduces_forwarding_cleanup_budget() {
    let server = opened(FlushFault::TeardownDelay, PeerPolicy::default()).await;
    let runtime = server
        .state
        .selenium_sessions
        .get(&server.session)
        .await
        .unwrap();
    let guard = runtime.clone().lock_owned().await;
    let (response, ()) = test_peer::bounded(async {
        tokio::join!(server.delete(), async {
            while server
                .state
                .selenium_sessions
                .get(&server.session)
                .await
                .is_some()
            {
                tokio::task::yield_now().await;
            }
            tokio::time::sleep(Duration::from_millis(85)).await;
            drop(guard);
        })
    })
    .await;
    failed(&server, response, "timeout").await;
    // This Arc outlives admission removal; its explicit release is fixture
    // ownership, not a claim that DELETE cancels all in-flight request Arcs.
    drop(runtime);
    completed(server.finish().await, 1);
}

async fn owned_failure(fault: FlushFault) -> anyhow::Error {
    let (source, peer) = super::cleanup_tests::fixture(fault);
    let state = state(source.clone(), Duration::from_secs(2));
    let (result, trace) = test_peer::bounded(async {
        tokio::join!(
            async {
                let runtime = build_selenium_runtime(&state).await.unwrap();
                stop_selenium_runtime_until(
                    Arc::new(Mutex::new(runtime)),
                    Budget::new(SESSION_CLEANUP_MAX, "WebDriver deletion").unwrap(),
                )
                .await
            },
            test_peer::peer(
                peer,
                PeerPolicy {
                    stop_error: true,
                    ..PeerPolicy::default()
                }
            )
        )
    })
    .await;
    completed(trace.unwrap(), 1);
    assert!(source.dropped.load(Ordering::SeqCst));
    assert_eq!(source.calls.load(Ordering::SeqCst), 1);
    result.unwrap_err()
}

fn business_and_cleanup(error: &anyhow::Error) -> &super::super::tasks::control::CleanupFailure {
    assert_eq!(
        ios_core::classify_error(error.as_ref()),
        ios_core::ErrorClass::Protocol
    );
    let both = error
        .downcast_ref::<super::super::tasks::control::CleanupFailure>()
        .expect("original business and cleanup causes remain owned");
    assert!(both.primary.chain().any(|cause| cause
        .downcast_ref::<ios_core::webinspector::WebInspectorError>()
        .is_some_and(|native| matches!(
            native,
            ios_core::webinspector::WebInspectorError::Protocol(_)
        ))));
    both
}

#[tokio::test]
async fn lifecycle_webdriver_delete_owned_result_preserves_business_and_native_io() {
    // Observe the actual private DELETE cleanup result. This fixture does not
    // make an HTTP/admission claim; the routed cases above cover that boundary.
    let error = owned_failure(FlushFault::TeardownIo).await;
    let both = business_and_cleanup(&error);
    assert_eq!(
        ios_core::classify_error(both.cleanup.as_ref()),
        ios_core::ErrorClass::PermissionDenied
    );
    assert!(both.cleanup.chain().any(|cause| cause
        .downcast_ref::<std::io::Error>()
        .is_some_and(|native| native.kind() == std::io::ErrorKind::PermissionDenied)));
}

#[tokio::test]
async fn lifecycle_webdriver_delete_owned_result_preserves_business_and_native_expiry() {
    let error = owned_failure(FlushFault::TeardownStall).await;
    let both = business_and_cleanup(&error);
    assert_eq!(
        ios_core::classify_error(both.cleanup.as_ref()),
        ios_core::ErrorClass::Timeout
    );
    assert!(both
        .cleanup
        .chain()
        .any(|cause| cause.is::<tokio::time::error::Elapsed>()));
}

#[tokio::test]
async fn lifecycle_webdriver_delete_owned_mutex_wait_preserves_native_expiry() {
    let (source, peer) = super::cleanup_tests::fixture(FlushFault::None);
    let state = state(source.clone(), Duration::from_secs(2));
    let ((), trace) = test_peer::bounded(async {
        tokio::join!(
            async {
                let runtime = Arc::new(Mutex::new(build_selenium_runtime(&state).await.unwrap()));
                let guard = runtime.clone().lock_owned().await;
                let error = stop_selenium_runtime_until(
                    runtime.clone(),
                    Budget::new(SESSION_CLEANUP_MAX, "WebDriver deletion").unwrap(),
                )
                .await
                .unwrap_err();
                assert_eq!(
                    ios_core::classify_error(error.as_ref()),
                    ios_core::ErrorClass::Timeout
                );
                assert!(error
                    .chain()
                    .any(|cause| cause.is::<tokio::time::error::Elapsed>()));
                assert!(guard.cleanup_armed);
                drop(guard);
                drop(runtime);
            },
            test_peer::peer(peer, PeerPolicy::default())
        )
    })
    .await;
    let trace = trace.unwrap();
    assert_eq!(trace.methods, ["Automation.createBrowsingContext"]);
    assert_eq!(trace.command_ids, [1]);
    assert_eq!(trace.teardowns, 0);
    assert!(source.dropped.load(Ordering::SeqCst));
    assert_eq!(source.calls.load(Ordering::SeqCst), 1);
}
