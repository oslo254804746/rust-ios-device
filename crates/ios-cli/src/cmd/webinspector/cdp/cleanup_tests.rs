//! Returned primary and cleanup results from the actual private page bridge.
use super::super::{bridge_tests::read_frame, tasks::control::CleanupFailure};
use super::lifecycle_tests::{no_more_requests, source_with_delay, start, Source};
use super::*;
use ios_core::{classify_error, ErrorClass};
use std::{
    future::Future,
    sync::atomic::{AtomicU64, Ordering},
};
use tokio::io::DuplexStream;

static PAGE: AtomicU64 = AtomicU64::new(6000);
const BOUND: Duration = Duration::from_secs(4);
type Socket =
    tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;

async fn bounded<T>(future: impl Future<Output = T>) -> T {
    tokio::time::timeout(BOUND, future)
        .await
        .expect("owned cleanup fixture stalled")
}

async fn attached() -> (
    Arc<Source>,
    DuplexStream,
    super::lifecycle_tests::Server,
    Socket,
) {
    let page = PAGE.fetch_add(1, Ordering::SeqCst);
    let (source, mut peer) = source_with_delay(Duration::ZERO);
    let server = start(source.clone(), page).await;
    let (socket, _) = bounded(tokio_tungstenite::connect_async(format!(
        "ws://127.0.0.1:{}/owned-page",
        server.port
    )))
    .await
    .unwrap();
    bounded(super::lifecycle_tests::setup_before_target(&mut peer, page)).await;
    super::super::bridge_tests::send_json(
        &mut peer,
        json!({
            "method":"Target.targetCreated", "params":{"targetInfo":{"targetId":"owned-cdp-target"}}
        }),
    )
    .await
    .unwrap();
    (source, peer, server, socket)
}

async fn closed(peer: &mut DuplexStream, source: &Source) {
    let close = bounded(read_frame(peer)).await.unwrap().unwrap();
    assert_eq!(
        close["__selector"].as_string(),
        Some("_rpc_forwardDidClose:")
    );
    no_more_requests(peer).await;
    assert!(source.dropped.load(Ordering::SeqCst));
    assert_eq!(source.calls.load(Ordering::SeqCst), 1);
}

fn native_io(error: &anyhow::Error) {
    assert_eq!(classify_error(error.as_ref()), ErrorClass::PermissionDenied);
    assert!(error.chain().any(|source| source
        .downcast_ref::<std::io::Error>()
        .is_some_and(|native| native.kind() == std::io::ErrorKind::PermissionDenied)));
}

fn elapsed(error: &anyhow::Error) {
    assert_eq!(classify_error(error.as_ref()), ErrorClass::Timeout);
    assert!(error
        .chain()
        .any(|source| source.is::<tokio::time::error::Elapsed>()));
}

#[tokio::test]
async fn lifecycle_cdp_cleanup_successful_close_reports_native_flush_failure() {
    let (source, mut peer, mut server, mut socket) = attached().await;
    source.flush_fault.store(1, Ordering::SeqCst);
    socket.close(None).await.unwrap();
    let error = bounded(&mut server.result).await.unwrap().unwrap_err();
    native_io(&error);
    closed(&mut peer, &source).await;
    drop(socket);
    server.finish().await;
}

#[tokio::test]
async fn lifecycle_cdp_cleanup_primary_json_and_native_cleanup_causes_both_survive() {
    let (source, mut peer, mut server, mut socket) = attached().await;
    source.flush_fault.store(1, Ordering::SeqCst);
    socket
        .send(tokio_tungstenite::tungstenite::Message::Text("{".into()))
        .await
        .unwrap();
    let error = bounded(&mut server.result).await.unwrap().unwrap_err();
    let both = error
        .downcast_ref::<CleanupFailure>()
        .expect("both errors must remain owned");
    assert_eq!(classify_error(error.as_ref()), ErrorClass::Protocol);
    assert!(both
        .primary
        .chain()
        .any(|source| source.is::<serde_json::Error>()));
    native_io(&both.cleanup);
    closed(&mut peer, &source).await;
    drop(socket);
    server.finish().await;
}

#[tokio::test]
async fn lifecycle_cdp_cleanup_successful_close_reports_real_default_expiry() {
    let (source, mut peer, mut server, mut socket) = attached().await;
    source.flush_fault.store(2, Ordering::SeqCst);
    let began = tokio::time::Instant::now();
    socket.close(None).await.unwrap();
    let error = bounded(&mut server.result).await.unwrap().unwrap_err();
    elapsed(&error);
    assert!(
        began.elapsed() >= CDP_PAGE_CLEANUP_TIMEOUT,
        "actual default cleanup must not inherit the 50ms request cap"
    );
    closed(&mut peer, &source).await;
    drop(socket);
    server.finish().await;
}

#[tokio::test]
async fn lifecycle_cdp_cleanup_primary_survives_real_default_cleanup_expiry() {
    let (source, mut peer, mut server, mut socket) = attached().await;
    source.flush_fault.store(2, Ordering::SeqCst);
    socket
        .send(tokio_tungstenite::tungstenite::Message::Text("{".into()))
        .await
        .unwrap();
    let error = bounded(&mut server.result).await.unwrap().unwrap_err();
    let both = error
        .downcast_ref::<CleanupFailure>()
        .expect("both errors must remain owned");
    assert_eq!(classify_error(error.as_ref()), ErrorClass::Protocol);
    assert!(both
        .primary
        .chain()
        .any(|source| source.is::<serde_json::Error>()));
    elapsed(&both.cleanup);
    closed(&mut peer, &source).await;
    drop(socket);
    server.finish().await;
}

#[tokio::test]
async fn lifecycle_cdp_cleanup_delayed_flush_completes_under_independent_budget() {
    let (source, mut peer, mut server, mut socket) = attached().await;
    let before = source.flush_completed.load(Ordering::SeqCst);
    source.flush_fault.store(3, Ordering::SeqCst);
    socket.close(None).await.unwrap();
    bounded(&mut server.result).await.unwrap().unwrap();
    assert_eq!(source.flush_completed.load(Ordering::SeqCst), before + 1);
    closed(&mut peer, &source).await;
    drop(socket);
    server.finish().await;
}
