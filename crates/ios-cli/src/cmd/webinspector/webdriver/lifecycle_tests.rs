use super::super::bridge_tests::read_frame;
use super::super::cdp::lifecycle_tests::{no_more_requests, source_with_delay, state};
use super::*;
use ios_core::{classify_error, ErrorClass};
use std::sync::atomic::Ordering;

#[tokio::test]
async fn lifecycle_server_setup_webdriver_owns_pending_source() {
    let (source, mut peer) = source_with_delay(Duration::from_millis(200));
    let server = state(source.clone(), Duration::from_millis(50));
    let error = match tokio::time::timeout(Duration::from_secs(4), build_selenium_runtime(&server))
        .await
        .unwrap()
    {
        Err(error) => error,
        Ok(_) => panic!("stalled source cannot create a runtime"),
    };
    assert_eq!(classify_error(error.as_ref()), ErrorClass::Timeout);
    assert!(error.chain().any(|e| e.is::<tokio::time::error::Elapsed>()));
    assert!(!source.returned.load(Ordering::SeqCst));
    assert!(source.dropped.load(Ordering::SeqCst));
    assert_eq!(source.calls.load(Ordering::SeqCst), 1);
    no_more_requests(&mut peer).await;
}

#[tokio::test]
async fn lifecycle_server_setup_webdriver_zero_and_overflow_reject_before_source() {
    for duration in [Duration::ZERO, Duration::MAX] {
        let (source, mut peer) = source_with_delay(Duration::ZERO);
        let server = state(source.clone(), duration);
        let error = match build_selenium_runtime(&server).await {
            Err(error) => error,
            Ok(_) => panic!("inadmissible budget cannot create a runtime"),
        };
        assert_eq!(
            classify_error(error.as_ref()),
            if duration.is_zero() {
                ErrorClass::Timeout
            } else {
                ErrorClass::InvalidInput
            }
        );
        assert!(!error.chain().any(|e| e.is::<tokio::time::error::Elapsed>()));
        assert_eq!(source.calls.load(Ordering::SeqCst), 0);
        drop(server);
        drop(source);
        no_more_requests(&mut peer).await;
    }
}

#[tokio::test]
async fn lifecycle_server_setup_webdriver_connect_and_identifier_share_total() {
    let (source, mut peer) = source_with_delay(Duration::from_millis(200));
    let server = state(source.clone(), Duration::from_millis(600));
    let (result, ()) = tokio::time::timeout(Duration::from_secs(4), async {
        tokio::join!(build_selenium_runtime(&server), async {
            let request = read_frame(&mut peer).await.unwrap().unwrap();
            assert_eq!(
                request["__selector"].as_string(),
                Some("_rpc_reportIdentifier:")
            );
            tokio::time::sleep(Duration::from_millis(500)).await;
            assert!(
                source.dropped.load(Ordering::SeqCst),
                "the total must expire before a fresh startup cap would"
            );
            no_more_requests(&mut peer).await;
        })
    })
    .await
    .unwrap();
    let error = match result {
        Err(error) => error,
        Ok(_) => panic!("startup exceeded the total"),
    };
    assert_eq!(classify_error(error.as_ref()), ErrorClass::Timeout);
    assert!(error.chain().any(|e| e.is::<tokio::time::error::Elapsed>()));
    assert!(source.returned.load(Ordering::SeqCst));
    assert!(source.dropped.load(Ordering::SeqCst));
}

#[tokio::test]
async fn lifecycle_server_setup_webdriver_success_clears_setup_cap_for_active_runtime() {
    let (source, peer) = source_with_delay(Duration::ZERO);
    let server = state(source.clone(), Duration::from_millis(750));
    let ((), trace) = tokio::time::timeout(Duration::from_secs(4), async {
        tokio::join!(
            async {
                let mut runtime = build_selenium_runtime(&server).await.unwrap();
                assert_eq!(runtime.automation.page_id(), 7);
                assert!(runtime.client.is_usable());
                tokio::time::sleep(Duration::from_millis(750)).await;
                assert_eq!(
                    runtime.current_url().await.unwrap().as_deref(),
                    Some("https://example.invalid/")
                );
                runtime.stop().await.unwrap();
                drop(runtime);
            },
            super::super::tasks::automation_test_peer(peer, None, Duration::from_millis(400))
        )
    })
    .await
    .unwrap();
    let (methods, teardowns) = trace.unwrap();
    assert_eq!(
        methods,
        [
            "Automation.createBrowsingContext",
            "Automation.getBrowsingContext",
            "Automation.closeBrowsingContext"
        ]
    );
    assert_eq!(teardowns, 1); // Stop now also closes its owned forwarding socket.
    assert!(source.dropped.load(Ordering::SeqCst));
    assert_eq!(source.calls.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn lifecycle_server_setup_webdriver_stalled_context_creation_uses_remainder() {
    let (source, peer) = source_with_delay(Duration::ZERO);
    let server = state(source.clone(), Duration::from_millis(650));
    let started = Arc::new(tokio::sync::Notify::new());
    let (result, trace) = tokio::time::timeout(Duration::from_secs(4), async {
        tokio::join!(
            build_selenium_runtime(&server),
            super::super::tasks::automation_test_peer(peer, Some(started.clone()), Duration::ZERO)
        )
    })
    .await
    .unwrap();
    let error = match result {
        Err(error) => error,
        Ok(_) => panic!("stalled context creation cannot succeed"),
    };
    assert_eq!(classify_error(error.as_ref()), ErrorClass::Timeout);
    assert!(error.chain().any(|e| e.is::<tokio::time::error::Elapsed>()));
    tokio::time::timeout(Duration::from_secs(1), started.notified())
        .await
        .unwrap();
    let (methods, teardowns) = trace.unwrap();
    assert_eq!(methods, ["Automation.createBrowsingContext"]);
    assert_eq!(teardowns, 0);
    assert!(source.dropped.load(Ordering::SeqCst));
}

#[tokio::test]
async fn lifecycle_server_setup_webdriver_caller_cancel_drops_committed_context_creation() {
    let (source, peer) = source_with_delay(Duration::ZERO);
    let server = state(source.clone(), Duration::from_secs(2));
    let started = Arc::new(tokio::sync::Notify::new());
    let ((), trace) = tokio::time::timeout(Duration::from_secs(4), async {
        tokio::join!(
            async {
                let operation = build_selenium_runtime(&server);
                tokio::pin!(operation);
                tokio::select! {
                    _ = started.notified() => {},
                    _ = &mut operation => panic!("context creation must stay pending"),
                }
                // Leaving this scope drops the actual pending constructor future.
            },
            super::super::tasks::automation_test_peer(peer, Some(started.clone()), Duration::ZERO)
        )
    })
    .await
    .unwrap();
    let (methods, teardowns) = trace.unwrap();
    assert_eq!(methods, ["Automation.createBrowsingContext"]);
    assert_eq!(teardowns, 0);
    assert!(source.dropped.load(Ordering::SeqCst));
}
