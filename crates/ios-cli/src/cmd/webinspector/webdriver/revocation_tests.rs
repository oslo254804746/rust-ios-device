//! Product WebDriver creation/reload with an owned Automation peer and join result.
use super::super::{
    cdp::lifecycle_tests::state,
    connection::{ConnectionFuture, InspectorSource},
    tasks::{
        control::CleanupFailure,
        test_peer::{self, FlushFault, PeerPolicy, Trace},
    },
};
use super::*;
use crate::access::{
    test_support::{grant, PolicyFixture, DEVICE, TOKEN},
    AccessControl, HttpBoundaryArgs, Permission,
};
use std::{future::Future, pin::Pin, sync::atomic::Ordering};
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
    completion: Option<Pin<Box<dyn Future<Output = Result<()>> + Send>>>,
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
async fn body(response: reqwest::Response) -> JsonValue {
    serde_json::from_slice(&response.bytes().await.unwrap()).unwrap()
}
impl Server {
    async fn reload(&self) {
        let response = test_peer::bounded(
            client()
                .post(format!("{}/auth/reload", self.base))
                .bearer_auth(TOKEN)
                .send(),
        )
        .await
        .unwrap();
        assert_eq!(response.status(), reqwest::StatusCode::OK);
        assert_eq!(
            body(response).await,
            json!({"operation":"reload_policy","data":true})
        );
    }
    async fn outcome(&mut self) -> Result<()> {
        test_peer::bounded(self.completion.take().unwrap()).await
    }
    fn safe(&self, error: &anyhow::Error) {
        let text = crate::output::error_message(error);
        for secret in [DEVICE, TOKEN, self.session.as_str()] {
            assert!(!text.contains(secret));
        }
    }
    async fn finish(mut self) -> Trace {
        let trace = test_peer::bounded(&mut self.peer).await.unwrap().unwrap();
        assert!(self
            .state
            .selenium_sessions
            .get(&self.session)
            .await
            .is_none());
        assert!(self.source.dropped.load(Ordering::SeqCst));
        assert_eq!(self.source.calls.load(Ordering::SeqCst), 1);
        self.shutdown.take().unwrap().send(()).unwrap();
        test_peer::bounded(&mut self.task).await.unwrap();
        trace
    }
}
async fn opened(fault: FlushFault, policy: PeerPolicy, expiry: bool) -> Server {
    let (source, stream) = super::cleanup_tests::fixture(fault);
    let peer = tokio::spawn(test_peer::peer(stream, policy));
    let owner = grant(TOKEN, vec![Permission::Debug, Permission::ManageServer]);
    let fixture = PolicyFixture::new(vec![if expiry {
        owner.expires_after(3)
    } else {
        owner
    }]);
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
    let address = listener.local_addr().unwrap();
    state.cdp_port = address.port();
    let router = HttpBoundaryArgs::default()
        .validate()
        .unwrap()
        .for_listener(address)
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
    let base = format!("http://{address}");
    let response = test_peer::bounded(
        client()
            .post(format!("{base}/session"))
            .bearer_auth(TOKEN)
            .send(),
    )
    .await
    .unwrap();
    assert_eq!(response.status(), reqwest::StatusCode::OK);
    let session = body(response).await["value"]["sessionId"]
        .as_str()
        .unwrap()
        .to_owned();
    assert!(state.selenium_sessions.get(&session).await.is_some());
    // This observes the actual product actor's private Result, rather than
    // replacing its cleanup or claiming that a test listener is its supervisor.
    let completion = Some(Box::pin(state.selenium_sessions.observe(&session).await)
        as Pin<Box<dyn Future<Output = Result<()>> + Send>>);
    Server {
        base,
        state,
        source,
        session,
        completion,
        _policy: fixture,
        shutdown: Some(shutdown),
        task,
        peer,
    }
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
fn native_io(error: &anyhow::Error) {
    assert_eq!(
        ios_core::classify_error(error.as_ref()),
        ios_core::ErrorClass::PermissionDenied
    );
    assert!(error.chain().any(|cause| cause
        .downcast_ref::<std::io::Error>()
        .is_some_and(|native| native.kind() == std::io::ErrorKind::PermissionDenied)));
}
fn elapsed(error: &anyhow::Error) {
    assert_eq!(
        ios_core::classify_error(error.as_ref()),
        ios_core::ErrorClass::Timeout
    );
    assert!(error
        .chain()
        .any(|cause| cause.is::<tokio::time::error::Elapsed>()));
}
fn both(error: &anyhow::Error) -> &CleanupFailure {
    assert_eq!(
        ios_core::classify_error(error.as_ref()),
        ios_core::ErrorClass::Protocol
    );
    let both = error
        .downcast_ref::<CleanupFailure>()
        .expect("native business and socket outcomes");
    assert!(both.primary.chain().any(|cause| cause
        .downcast_ref::<ios_core::webinspector::WebInspectorError>()
        .is_some_and(|native| matches!(
            native,
            ios_core::webinspector::WebInspectorError::Protocol(_)
        ))));
    both
}

#[tokio::test]
async fn lifecycle_webdriver_revocation_reload_closes_original_context_and_forwarding() {
    let mut server = opened(FlushFault::None, PeerPolicy::default(), false).await;
    server.reload().await;
    server.outcome().await.unwrap();
    completed(server.finish().await, 1);
}
#[tokio::test]
async fn lifecycle_webdriver_revocation_expiry_closes_original_context_and_forwarding() {
    let mut server = opened(FlushFault::None, PeerPolicy::default(), true).await;
    server.outcome().await.unwrap();
    completed(server.finish().await, 1);
}
#[tokio::test]
async fn lifecycle_webdriver_revocation_preserves_native_forwarding_io() {
    let mut server = opened(FlushFault::TeardownIo, PeerPolicy::default(), false).await;
    server.reload().await;
    let error = server.outcome().await.unwrap_err();
    native_io(&error);
    server.safe(&error);
    completed(server.finish().await, 1);
}
#[tokio::test]
async fn lifecycle_webdriver_revocation_real_default_forwarding_expiry_retains_elapsed() {
    let mut server = opened(FlushFault::TeardownStall, PeerPolicy::default(), false).await;
    let began = tokio::time::Instant::now();
    server.reload().await;
    let error = server.outcome().await.unwrap_err();
    elapsed(&error);
    server.safe(&error);
    assert!(began.elapsed() >= SESSION_CLEANUP_MAX);
    completed(server.finish().await, 1);
}
#[tokio::test]
async fn lifecycle_webdriver_revocation_retains_business_primary_and_native_forwarding_io() {
    let mut server = opened(
        FlushFault::TeardownIo,
        PeerPolicy {
            stop_error: true,
            ..PeerPolicy::default()
        },
        false,
    )
    .await;
    server.reload().await;
    let error = server.outcome().await.unwrap_err();
    native_io(&both(&error).cleanup);
    server.safe(&error);
    completed(server.finish().await, 1);
}
#[tokio::test]
async fn lifecycle_webdriver_revocation_retains_business_primary_and_forwarding_elapsed() {
    let mut server = opened(
        FlushFault::TeardownStall,
        PeerPolicy {
            stop_error: true,
            ..PeerPolicy::default()
        },
        false,
    )
    .await;
    server.reload().await;
    let error = server.outcome().await.unwrap_err();
    elapsed(&both(&error).cleanup);
    server.safe(&error);
    completed(server.finish().await, 1);
}
#[tokio::test]
async fn lifecycle_webdriver_revocation_mutex_wait_expiry_preserves_native_cause_without_cleanup() {
    let mut server = opened(FlushFault::None, PeerPolicy::default(), false).await;
    let runtime = server
        .state
        .selenium_sessions
        .get(&server.session)
        .await
        .unwrap();
    let held = runtime.clone().lock_owned().await;
    let began = tokio::time::Instant::now();
    server.reload().await;
    let error = server.outcome().await.unwrap_err();
    elapsed(&error);
    server.safe(&error);
    assert!(began.elapsed() >= SESSION_CLEANUP_MAX);
    assert!(held.cleanup_armed);
    assert!(!server.source.dropped.load(Ordering::SeqCst));
    drop(held);
    drop(runtime);
    let trace = server.finish().await;
    assert_eq!(trace.methods, ["Automation.createBrowsingContext"]);
    assert_eq!(trace.command_ids, [1]);
    assert_eq!(trace.teardowns, 0);
    // Fixture Arc release is not a claim that revocation cancels all requests.
}
#[tokio::test]
async fn lifecycle_webdriver_revocation_pending_context_close_skips_invalid_socket_recovery() {
    let mut server = opened(
        FlushFault::None,
        PeerPolicy {
            stop_stall: Some(Arc::new(tokio::sync::Notify::new())),
            ..PeerPolicy::default()
        },
        false,
    )
    .await;
    server.reload().await;
    let error = server.outcome().await.unwrap_err();
    elapsed(&error);
    server.safe(&error);
    completed(server.finish().await, 0);
}
