//! Setup owner controls, using the same real stream and WebSocket fixtures.
use super::super::bridge_tests::{envelope, read_frame, strings, write_frame};
use super::lifecycle_tests::{
    app, no_more_requests, source_with_delay, start, start_with_timeout, state,
};
use super::*;
use ios_core::{classify_error, ErrorClass};
use std::{future::Future, sync::atomic::Ordering};

const SHORT: Duration = Duration::from_millis(50);
const DELAY: Duration = Duration::from_millis(200);
const BOUND: Duration = Duration::from_secs(4);

async fn bounded<T>(future: impl Future<Output = T>) -> T {
    tokio::time::timeout(BOUND, future).await.unwrap()
}

fn elapsed(error: &anyhow::Error) {
    assert_eq!(classify_error(error.as_ref()), ErrorClass::Timeout);
    assert!(error.chain().any(|e| e.is::<tokio::time::error::Elapsed>()));
}

#[tokio::test]
async fn lifecycle_server_setup_cdp_discovery_owns_pending_source() {
    let (source, mut peer) = source_with_delay(DELAY);
    let server = state(source.clone(), SHORT);
    let error = bounded(load_open_pages(&server)).await.unwrap_err();
    elapsed(&error);
    assert!(!source.returned.load(Ordering::SeqCst));
    assert!(source.dropped.load(Ordering::SeqCst));
    assert_eq!(source.calls.load(Ordering::SeqCst), 1);
    no_more_requests(&mut peer).await;
}

#[tokio::test]
async fn lifecycle_server_setup_cdp_page_owns_pending_source() {
    let (source, mut peer) = source_with_delay(DELAY);
    let mut server = start(source.clone(), 9001).await;
    let (socket, _) = bounded(tokio_tungstenite::connect_async(format!(
        "ws://127.0.0.1:{}/owned-page",
        server.port
    )))
    .await
    .unwrap();
    let error = bounded(&mut server.result).await.unwrap().unwrap_err();
    elapsed(&error);
    assert!(!source.returned.load(Ordering::SeqCst));
    assert!(source.dropped.load(Ordering::SeqCst));
    assert_eq!(source.calls.load(Ordering::SeqCst), 1);
    no_more_requests(&mut peer).await;
    drop(socket);
    server.finish().await;
}

#[tokio::test]
async fn lifecycle_server_setup_cdp_browser_page_owns_pending_source() {
    let (source, mut peer) = source_with_delay(DELAY);
    let server = state(source.clone(), SHORT);
    let (output, _receiver) = mpsc::channel(1);
    let error = bounded(open_cdp_browser_page(
        &server,
        "PID:42:page-9002",
        "owned-session",
        output,
    ))
    .await
    .unwrap_err();
    elapsed(&error);
    assert!(!source.returned.load(Ordering::SeqCst));
    assert!(source.dropped.load(Ordering::SeqCst));
    assert_eq!(source.calls.load(Ordering::SeqCst), 1);
    no_more_requests(&mut peer).await;
}

fn admission(error: &anyhow::Error, duration: Duration) {
    assert_eq!(
        classify_error(error.as_ref()),
        if duration.is_zero() {
            ErrorClass::Timeout
        } else {
            ErrorClass::InvalidInput
        }
    );
    assert!(!error.chain().any(|e| e.is::<tokio::time::error::Elapsed>()));
}

#[tokio::test]
async fn lifecycle_server_setup_cdp_zero_and_overflow_reject_before_source() {
    for duration in [Duration::ZERO, Duration::MAX] {
        for browser in [false, true] {
            let (source, mut peer) = source_with_delay(Duration::ZERO);
            let server = state(source.clone(), duration);
            let error = if browser {
                let (output, _receiver) = mpsc::channel(1);
                bounded(open_cdp_browser_page(
                    &server,
                    "PID:42:page-9004",
                    "owned-session",
                    output,
                ))
                .await
                .unwrap_err()
            } else {
                bounded(load_open_pages(&server)).await.unwrap_err()
            };
            admission(&error, duration);
            assert_eq!(source.calls.load(Ordering::SeqCst), 0);
            assert!(!source.dropped.load(Ordering::SeqCst));
            drop(server);
            drop(source);
            no_more_requests(&mut peer).await;
        }
        let (source, mut peer) = source_with_delay(Duration::ZERO);
        let mut server = start_with_timeout(source.clone(), 9005, duration).await;
        let (socket, _) = bounded(tokio_tungstenite::connect_async(format!(
            "ws://127.0.0.1:{}/owned-page",
            server.port
        )))
        .await
        .unwrap();
        let error = bounded(&mut server.result).await.unwrap().unwrap_err();
        admission(&error, duration);
        assert_eq!(source.calls.load(Ordering::SeqCst), 0);
        drop(socket);
        server.finish().await;
        drop(source);
        no_more_requests(&mut peer).await;
    }
}

#[tokio::test]
async fn lifecycle_server_setup_cdp_connect_and_identifier_share_total() {
    for browser in [false, true] {
        let (source, mut peer) = source_with_delay(Duration::from_millis(200));
        let server = state(source.clone(), Duration::from_millis(600));
        let (output, _receiver) = mpsc::channel(1);
        let (error, ()) = bounded(async {
            tokio::join!(
                async {
                    if browser {
                        open_cdp_browser_page(&server, "PID:42:page-9006", "owned-session", output)
                            .await
                            .unwrap_err()
                    } else {
                        load_open_pages(&server).await.unwrap_err()
                    }
                },
                async {
                    let request = read_frame(&mut peer).await.unwrap().unwrap();
                    assert_eq!(
                        request["__selector"].as_string(),
                        Some("_rpc_reportIdentifier:")
                    );
                    // Reply delay exceeds the remainder, while each individual
                    // stage is shorter than the configured total.
                    tokio::time::sleep(Duration::from_millis(500)).await;
                    assert!(
                        source.dropped.load(Ordering::SeqCst),
                        "the total must expire before a fresh startup cap would"
                    );
                    no_more_requests(&mut peer).await;
                }
            )
        })
        .await;
        elapsed(&error);
        assert!(source.returned.load(Ordering::SeqCst));
        assert!(source.dropped.load(Ordering::SeqCst));
        assert_eq!(source.calls.load(Ordering::SeqCst), 1);
    }
}

#[tokio::test]
async fn lifecycle_server_setup_cdp_page_consumes_connection_and_discovery_remainder() {
    let (source, mut peer) = source_with_delay(Duration::from_millis(100));
    let mut server = start_with_timeout(source.clone(), 9007, Duration::from_millis(600)).await;
    let (socket, _) = bounded(tokio_tungstenite::connect_async(format!(
        "ws://127.0.0.1:{}/owned-page",
        server.port
    )))
    .await
    .unwrap();
    let request = bounded(read_frame(&mut peer)).await.unwrap().unwrap();
    assert_eq!(
        request["__selector"].as_string(),
        Some("_rpc_reportIdentifier:")
    );
    tokio::time::sleep(Duration::from_millis(100)).await;
    write_frame(
        &mut peer,
        envelope(
            "_rpc_reportCurrentState:",
            strings(&[(
                "WIRAutomationAvailabilityKey",
                "WIRAutomationAvailabilityAvailable",
            )]),
        ),
    )
    .await
    .unwrap();
    let request = bounded(read_frame(&mut peer)).await.unwrap().unwrap();
    assert_eq!(
        request["__selector"].as_string(),
        Some("_rpc_getConnectedApplications:")
    );
    tokio::time::sleep(Duration::from_millis(100)).await;
    write_frame(
        &mut peer,
        envelope(
            "_rpc_reportConnectedApplicationList:",
            plist::Dictionary::from_iter([(
                "WIRApplicationDictionaryKey".to_owned(),
                plist::Value::Dictionary(plist::Dictionary::from_iter([(
                    "PID:42".to_owned(),
                    plist::Value::Dictionary(app()),
                )])),
            )]),
        ),
    )
    .await
    .unwrap();
    let request = bounded(read_frame(&mut peer)).await.unwrap().unwrap();
    assert_eq!(
        request["__selector"].as_string(),
        Some("_rpc_forwardGetListing:")
    );
    let error = tokio::time::timeout(Duration::from_millis(400), &mut server.result)
        .await
        .expect("discovery must use the remainder, not restart600ms")
        .unwrap()
        .unwrap_err();
    elapsed(&error);
    assert!(source.returned.load(Ordering::SeqCst));
    assert!(source.dropped.load(Ordering::SeqCst));
    no_more_requests(&mut peer).await;
    drop(socket);
    server.finish().await;
}

#[tokio::test]
async fn lifecycle_server_setup_cdp_initial_blocked_write_is_bounded() {
    for browser in [false, true] {
        let (source, mut peer) = source_with_delay(Duration::ZERO);
        source.stall.store(true, Ordering::SeqCst);
        let server = state(source.clone(), SHORT);
        let error = if browser {
            let (output, _receiver) = mpsc::channel(1);
            bounded(open_cdp_browser_page(
                &server,
                "PID:42:page-9008",
                "owned-session",
                output,
            ))
            .await
            .unwrap_err()
        } else {
            bounded(load_open_pages(&server)).await.unwrap_err()
        };
        elapsed(&error);
        assert!(source.blocked.load(Ordering::SeqCst) > 0);
        assert!(source.dropped.load(Ordering::SeqCst));
        no_more_requests(&mut peer).await;
    }
}

#[tokio::test]
async fn lifecycle_server_setup_cdp_page_lock_expiry_does_not_contact_source() {
    let guard = lock_cdp_page("PID:42:page-9009", BOUND).await.unwrap();
    let (source, mut peer) = source_with_delay(Duration::ZERO);
    let server = state(source.clone(), SHORT);
    let (output, _receiver) = mpsc::channel(1);
    let error = bounded(open_cdp_browser_page(
        &server,
        "PID:42:page-9009",
        "owned-session",
        output,
    ))
    .await
    .unwrap_err();
    elapsed(&error);
    assert_eq!(source.calls.load(Ordering::SeqCst), 0);
    drop(guard);
    drop(server);
    drop(source);
    no_more_requests(&mut peer).await;
    let _released = bounded(lock_cdp_page("PID:42:page-9009", SHORT))
        .await
        .unwrap();
}

#[tokio::test]
async fn lifecycle_server_setup_cdp_browser_cancel_during_attach_releases_stream_and_lock() {
    let (source, mut peer) = source_with_delay(Duration::ZERO);
    let server = state(source.clone(), Duration::from_secs(1));
    let started = Arc::new(tokio::sync::Notify::new());
    let (output, mut receiver) = mpsc::channel(1);
    bounded(async {
        tokio::join!(
            async {
                let operation =
                    open_cdp_browser_page(&server, "PID:42:page-9010", "owned-session", output);
                tokio::pin!(operation);
                tokio::select! {
                    _ = started.notified() => {},
                    _ = &mut operation => panic!("target creation must remain pending"),
                }
            },
            async {
                super::lifecycle_tests::setup_before_target(&mut peer, 9010).await;
                started.notify_one();
                no_more_requests(&mut peer).await;
            }
        );
    })
    .await;
    assert!(source.dropped.load(Ordering::SeqCst));
    assert!(source.returned.load(Ordering::SeqCst));
    assert_eq!(source.calls.load(Ordering::SeqCst), 1);
    assert!(matches!(
        receiver.try_recv(),
        Err(mpsc::error::TryRecvError::Disconnected)
    ));
    let _released = bounded(lock_cdp_page("PID:42:page-9010", SHORT))
        .await
        .unwrap();
}
