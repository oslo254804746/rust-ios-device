//! Actual admitted CDP page revocation over synthetic policy and owned streams.
use super::super::{
    bridge_tests::{read_frame, send_json},
    connection::{ConnectionFuture, InspectorSource},
};
use super::lifecycle_tests::{
    no_more_requests, setup_before_target, source_with_delay, state, Source,
};
use super::*;
use crate::access::{
    test_support::{grant, PolicyFixture, DEVICE, REPLACEMENT, TOKEN},
    AccessControl, Permission,
};
use std::{
    future::Future,
    sync::atomic::{AtomicU64, Ordering},
};
use tokio::{
    io::{AsyncWriteExt, DuplexStream},
    sync::oneshot,
};
use tokio_tungstenite::tungstenite::client::IntoClientRequest;

static PAGE: AtomicU64 = AtomicU64::new(9000);
const BOUND: Duration = Duration::from_secs(8);
type Socket =
    tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;

async fn bounded<T>(operation: impl Future<Output = T>) -> T {
    tokio::time::timeout(BOUND, operation)
        .await
        .expect("owned admitted page fixture stalled")
}

struct AuthorizedSource(Arc<Source>);
impl InspectorSource for AuthorizedSource {
    fn connect<'a>(&'a self, udid: &'a str, timeout: Duration) -> ConnectionFuture<'a> {
        assert_eq!(udid, DEVICE);
        self.0.connect("owned-cdp-device", timeout)
    }
}

struct Server {
    base: String,
    fixture: PolicyFixture,
    shutdown: Option<oneshot::Sender<()>>,
    task: tokio::task::JoinHandle<()>,
}
impl Drop for Server {
    fn drop(&mut self) {
        self.task.abort();
    }
}
impl Server {
    async fn reload(&self) {
        self.fixture.replace(vec![grant(
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

async fn connected(source: Arc<Source>, expiry: bool, timeout: Duration) -> (Server, Socket, u64) {
    let page = PAGE.fetch_add(1, Ordering::SeqCst);
    let owner = grant(TOKEN, vec![Permission::Debug, Permission::ManageServer]);
    let fixture = PolicyFixture::new(vec![if expiry {
        owner.expires_after(3)
    } else {
        owner
    }]);
    let access = AccessControl::from_file(fixture.path.clone())
        .await
        .unwrap();
    let mut server_state = state(Arc::new(AuthorizedSource(source.clone())), timeout);
    server_state.udid = DEVICE.into();
    server_state.access = access;
    let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
        .await
        .unwrap();
    let port = listener.local_addr().unwrap().port();
    server_state.cdp_port = port;
    let router = cdp_router(server_state);
    let (shutdown, stopped) = oneshot::channel();
    let task = tokio::spawn(async move {
        axum::serve(listener, router)
            .with_graceful_shutdown(async {
                let _ = stopped.await;
            })
            .await
            .unwrap();
    });
    let server = Server {
        base: format!("http://127.0.0.1:{port}"),
        fixture,
        shutdown: Some(shutdown),
        task,
    };
    let mut request = format!("ws://127.0.0.1:{port}/devtools/page/{page}")
        .into_client_request()
        .unwrap();
    request
        .headers_mut()
        .insert("authorization", format!("Bearer {TOKEN}").parse().unwrap());
    let (socket, _) = bounded(tokio_tungstenite::connect_async(request))
        .await
        .unwrap();
    (server, socket, page)
}

async fn attached(
    expiry: bool,
    timeout: Duration,
) -> (Arc<Source>, DuplexStream, Server, Socket, u64, String) {
    let (source, mut peer) = source_with_delay(Duration::ZERO);
    let (server, mut socket, page) = connected(source.clone(), expiry, timeout).await;
    bounded(setup_before_target(&mut peer, page)).await;
    send_json(&mut peer,json!({"method":"Target.targetCreated","params":{"targetInfo":{"targetId":"owned-cdp-target"}}})).await.unwrap();
    socket
        .send(tokio_tungstenite::tungstenite::Message::Text(
            json!({"id":1,"method":"Runtime.enable"}).to_string(),
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
    (source, peer, server, socket, page, sender)
}

async fn released(source: &Source, peer: &mut DuplexStream, socket: &mut Socket, page: u64) {
    released_at(source, peer, socket, &page.to_string()).await;
}

async fn released_at(source: &Source, peer: &mut DuplexStream, socket: &mut Socket, target: &str) {
    no_more_requests(peer).await;
    assert!(source.dropped.load(Ordering::SeqCst));
    assert_eq!(source.calls.load(Ordering::SeqCst), 1);
    drop(
        bounded(lock_cdp_page(target, Duration::from_millis(200)))
            .await
            .unwrap(),
    );
    let frame = bounded(socket.next()).await;
    assert!(matches!(
        frame,
        None | Some(Err(_)) | Some(Ok(tokio_tungstenite::tungstenite::Message::Close(_)))
    ));
}

async fn close(peer: &mut DuplexStream, page: u64, sender: &str) {
    let frame = bounded(read_frame(peer))
        .await
        .unwrap()
        .expect("attached page must clean on revocation");
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
        Some(page)
    );
    assert_eq!(argument["WIRSenderKey"].as_string(), Some(sender));
}

#[tokio::test]
async fn lifecycle_cdp_lease_reload_cleans_attached_page_before_release() {
    let (source, mut peer, server, mut socket, page, sender) =
        attached(false, Duration::from_millis(500)).await;
    server.reload().await;
    close(&mut peer, page, &sender).await;
    released(&source, &mut peer, &mut socket, page).await;
    drop(socket);
    server.finish().await;
}

#[tokio::test]
async fn lifecycle_cdp_lease_expiry_cleans_attached_page_before_release() {
    let (source, mut peer, server, mut socket, page, sender) =
        attached(true, Duration::from_millis(500)).await;
    close(&mut peer, page, &sender).await;
    released(&source, &mut peer, &mut socket, page).await;
    drop(socket);
    server.finish().await;
}

#[tokio::test]
async fn lifecycle_cdp_lease_reload_cleanup_has_independent_flush_allowance() {
    let (source, mut peer, server, mut socket, page, sender) =
        attached(false, Duration::from_millis(50)).await;
    let before = source.flush_completed.load(Ordering::SeqCst);
    source.flush_fault.store(3, Ordering::SeqCst);
    server.reload().await;
    close(&mut peer, page, &sender).await;
    released(&source, &mut peer, &mut socket, page).await;
    assert_eq!(source.flush_completed.load(Ordering::SeqCst), before + 1);
    drop(socket);
    server.finish().await;
}

#[tokio::test]
async fn lifecycle_cdp_lease_reload_after_observed_partial_frame_skips_recovery() {
    let (source, mut peer, server, mut socket, page, _) =
        attached(false, Duration::from_millis(500)).await;
    let before = source.received.load(Ordering::SeqCst);
    peer.write_all(&[0, 0]).await.unwrap();
    bounded(async {
        while source.received.load(Ordering::SeqCst) < before + 2 {
            tokio::task::yield_now().await;
        }
    })
    .await;
    server.reload().await;
    released(&source, &mut peer, &mut socket, page).await;
    drop(socket);
    server.finish().await;
}

#[tokio::test]
async fn lifecycle_cdp_lease_reload_drops_pending_owned_source_without_wire() {
    let (source, mut peer) = source_with_delay(Duration::from_secs(1));
    let (server, mut socket, page) = connected(source.clone(), false, Duration::from_secs(2)).await;
    bounded(async {
        while source.calls.load(Ordering::SeqCst) == 0 {
            tokio::task::yield_now().await;
        }
    })
    .await;
    server.reload().await;
    released(&source, &mut peer, &mut socket, page).await;
    assert!(!source.returned.load(Ordering::SeqCst));
    assert_eq!(source.received.load(Ordering::SeqCst), 0);
    drop(socket);
    server.finish().await;
}

#[tokio::test]
async fn lifecycle_cdp_lease_reload_does_not_repoll_completed_stop_during_cleanup_expiry() {
    let (source, mut peer, server, mut socket, page, sender) =
        attached(false, Duration::from_millis(500)).await;
    source.flush_fault.store(2, Ordering::SeqCst);
    let began = tokio::time::Instant::now();
    server.reload().await;
    close(&mut peer, page, &sender).await;
    released(&source, &mut peer, &mut socket, page).await;
    assert!(began.elapsed() >= CDP_PAGE_CLEANUP_TIMEOUT);
    drop(socket);
    server.finish().await;
}

#[tokio::test]
async fn lifecycle_cdp_lease_private_cancel_and_native_cleanup_errors_both_survive() {
    let page = PAGE.fetch_add(1, Ordering::SeqCst);
    let (source, mut peer) = source_with_delay(Duration::ZERO);
    let (stop, stopped) = oneshot::channel();
    let mut server = super::lifecycle_tests::start_with_stop(
        source.clone(),
        page,
        Duration::from_millis(500),
        async {
            let _ = stopped.await;
        },
    )
    .await;
    let (mut socket, _) = bounded(tokio_tungstenite::connect_async(format!(
        "ws://127.0.0.1:{}/owned-page",
        server.port
    )))
    .await
    .unwrap();
    bounded(setup_before_target(&mut peer, page)).await;
    send_json(&mut peer,json!({"method":"Target.targetCreated","params":{"targetInfo":{"targetId":"owned-cdp-target"}}})).await.unwrap();
    socket
        .send(tokio_tungstenite::tungstenite::Message::Text(
            json!({"id":1,"method":"Runtime.enable"}).to_string(),
        ))
        .await
        .unwrap();
    let frame = bounded(read_frame(&mut peer)).await.unwrap().unwrap();
    assert_eq!(
        frame["__selector"].as_string(),
        Some("_rpc_forwardSocketData:")
    );
    let sender = frame["__argument"].as_dictionary().unwrap()["WIRSenderKey"]
        .as_string()
        .unwrap()
        .to_owned();
    source.flush_fault.store(1, Ordering::SeqCst);
    stop.send(()).unwrap();
    let error = bounded(&mut server.result).await.unwrap().unwrap_err();
    assert_eq!(
        ios_core::classify_error(error.as_ref()),
        ios_core::ErrorClass::Cancelled
    );
    let both = error
        .downcast_ref::<super::super::tasks::control::CleanupFailure>()
        .expect("both errors remain owned");
    assert!(both
        .primary
        .chain()
        .any(|error| error.is::<ios_core::OperationCancelled>()));
    assert_eq!(
        ios_core::classify_error(both.cleanup.as_ref()),
        ios_core::ErrorClass::PermissionDenied
    );
    assert!(both.cleanup.chain().any(|error| error
        .downcast_ref::<std::io::Error>()
        .is_some_and(|native| native.kind() == std::io::ErrorKind::PermissionDenied)));
    close(&mut peer, page, &sender).await;
    released_at(
        &source,
        &mut peer,
        &mut socket,
        &format!("PID:42:page-{page}"),
    )
    .await;
    drop(socket);
    server.finish().await;
}
