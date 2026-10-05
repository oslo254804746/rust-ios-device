//! Actual runtime constructor and Automation cleanup over owned plist streams.
use super::super::{
    cdp::lifecycle_tests::state,
    connection::{ConnectionFuture, InspectorSource},
    tasks::{
        control::CleanupFailure,
        test_peer::{self, FlushFault, PeerPolicy},
    },
};
use super::*;
use ios_core::{classify_error, ErrorClass};
use std::sync::{
    atomic::{AtomicBool, AtomicUsize, Ordering},
    Mutex as StdMutex,
};

struct Source {
    stream: StdMutex<Option<ServiceStream>>,
    calls: AtomicUsize,
    dropped: Arc<AtomicBool>,
}
impl InspectorSource for Source {
    fn connect<'a>(&'a self, udid: &'a str, _timeout: Duration) -> ConnectionFuture<'a> {
        Box::pin(async move {
            assert_eq!(udid, "owned-cdp-device");
            self.calls.fetch_add(1, Ordering::SeqCst);
            Ok(InspectorConnection {
                device: None,
                stream: self
                    .stream
                    .lock()
                    .unwrap()
                    .take()
                    .expect("one owned stream"),
            })
        })
    }
}

fn fixture(fault: FlushFault) -> (Arc<Source>, tokio::io::DuplexStream) {
    let (source, peer) = test_peer::fixture(fault);
    let stream = source.stream.lock().unwrap().take();
    (
        Arc::new(Source {
            stream: StdMutex::new(stream),
            calls: AtomicUsize::new(0),
            dropped: source.dropped.clone(),
        }),
        peer,
    )
}

async fn stopped(fault: FlushFault, policy: PeerPolicy) -> (Result<()>, test_peer::Trace) {
    let (source, peer) = fixture(fault);
    let server = state(source.clone(), Duration::from_secs(2));
    let (result, trace) = test_peer::bounded(async {
        tokio::join!(
            async {
                let mut runtime = build_selenium_runtime(&server).await.unwrap();
                let result = runtime.stop().await;
                drop(runtime);
                result
            },
            test_peer::peer(peer, policy)
        )
    })
    .await;
    assert!(source.dropped.load(Ordering::SeqCst));
    assert_eq!(source.calls.load(Ordering::SeqCst), 1);
    let trace = trace.unwrap();
    assert_eq!(
        trace.methods,
        [
            "Automation.createBrowsingContext",
            "Automation.closeBrowsingContext"
        ]
    );
    assert_eq!(trace.command_ids, [1, 2]);
    (result, trace)
}

fn native_io(error: &anyhow::Error) {
    assert_eq!(classify_error(error.as_ref()), ErrorClass::PermissionDenied);
    assert!(error.chain().any(|e| e
        .downcast_ref::<std::io::Error>()
        .is_some_and(|native| native.kind() == std::io::ErrorKind::PermissionDenied)));
}

fn elapsed(error: &anyhow::Error) {
    assert_eq!(classify_error(error.as_ref()), ErrorClass::Timeout);
    assert!(error.chain().any(|e| e.is::<tokio::time::error::Elapsed>()));
}

#[tokio::test]
async fn lifecycle_webdriver_cleanup_closes_context_and_owned_forwarding_socket() {
    let (result, trace) = stopped(FlushFault::None, PeerPolicy::default()).await;
    result.unwrap();
    assert_eq!(trace.teardowns, 1);
}

#[tokio::test]
async fn lifecycle_webdriver_cleanup_preserves_native_socket_flush_failure() {
    let (result, trace) = stopped(FlushFault::TeardownIo, PeerPolicy::default()).await;
    native_io(&result.unwrap_err());
    assert_eq!(trace.teardowns, 1);
}

#[tokio::test]
async fn lifecycle_webdriver_cleanup_keeps_business_close_and_native_socket_errors() {
    let (result, trace) = stopped(
        FlushFault::TeardownIo,
        PeerPolicy {
            stop_error: true,
            ..Default::default()
        },
    )
    .await;
    let error = result.unwrap_err();
    assert_eq!(classify_error(error.as_ref()), ErrorClass::Protocol);
    let both = error
        .downcast_ref::<CleanupFailure>()
        .expect("both causes remain owned");
    assert!(both.primary.chain().any(|e| e
        .downcast_ref::<ios_core::webinspector::WebInspectorError>()
        .is_some_and(|native| matches!(
            native,
            ios_core::webinspector::WebInspectorError::Protocol(_)
        ))));
    native_io(&both.cleanup);
    assert_eq!(trace.teardowns, 1);
}

#[tokio::test]
async fn lifecycle_webdriver_cleanup_stalled_forwarding_flush_has_native_expiry() {
    let (result, trace) = stopped(FlushFault::TeardownStall, PeerPolicy::default()).await;
    elapsed(&result.unwrap_err());
    assert_eq!(trace.teardowns, 1);
}

#[tokio::test]
async fn lifecycle_webdriver_cleanup_context_and_socket_share_stored_total() {
    let (result, trace) = stopped(
        FlushFault::TeardownDelay,
        PeerPolicy {
            stop_delay: Duration::from_millis(80),
            ..Default::default()
        },
    )
    .await;
    elapsed(&result.unwrap_err());
    assert_eq!(trace.teardowns, 1);
}

#[tokio::test]
async fn lifecycle_webdriver_cleanup_stalled_context_close_skips_socket_recovery() {
    let (result, trace) = stopped(
        FlushFault::None,
        PeerPolicy {
            stop_stall: Some(Arc::new(tokio::sync::Notify::new())),
            ..Default::default()
        },
    )
    .await;
    elapsed(&result.unwrap_err());
    assert_eq!(trace.teardowns, 0);
}

#[tokio::test]
async fn lifecycle_webdriver_cleanup_does_not_repeat_successful_or_business_failed_stop() {
    for stop_error in [false, true] {
        let (source, peer) = fixture(FlushFault::None);
        let server = state(source.clone(), Duration::from_secs(2));
        let ((), trace) = test_peer::bounded(async {
            tokio::join!(
                async {
                    let mut runtime = build_selenium_runtime(&server).await.unwrap();
                    let first = runtime.stop().await;
                    if stop_error {
                        assert_eq!(
                            classify_error(first.unwrap_err().as_ref()),
                            ErrorClass::Protocol
                        );
                    } else {
                        first.unwrap();
                    }
                    assert!(runtime.client.is_usable());
                    runtime.stop().await.unwrap();
                    drop(runtime);
                },
                test_peer::peer(
                    peer,
                    PeerPolicy {
                        stop_error,
                        ..Default::default()
                    }
                )
            )
        })
        .await;
        let trace = trace.unwrap();
        assert_eq!(
            trace.methods,
            [
                "Automation.createBrowsingContext",
                "Automation.closeBrowsingContext"
            ]
        );
        assert_eq!(trace.command_ids, [1, 2]);
        assert_eq!(trace.teardowns, 1);
        assert!(source.dropped.load(Ordering::SeqCst));
        assert_eq!(source.calls.load(Ordering::SeqCst), 1);
    }
}

#[tokio::test]
async fn lifecycle_webdriver_cleanup_caller_cancel_disarms_committed_context_close() {
    let (source, peer) = fixture(FlushFault::None);
    let server = state(source.clone(), Duration::from_secs(2));
    let started = Arc::new(tokio::sync::Notify::new());
    let ((),trace)=test_peer::bounded(async {
        tokio::join!(async {
            let mut runtime=build_selenium_runtime(&server).await.unwrap();
            {
                let operation=runtime.stop();
                tokio::pin!(operation);
                tokio::select! {
                    _=started.notified()=>{},
                    result=&mut operation=>panic!("context close must stay pending: {result:?}"),
                }
            }
            assert!(!runtime.client.is_usable());
            runtime.stop().await.unwrap();
            drop(runtime);
        },test_peer::peer(peer,PeerPolicy {stop_stall:Some(started.clone()),..Default::default()}))
    }).await;
    let trace = trace.unwrap();
    assert_eq!(
        trace.methods,
        [
            "Automation.createBrowsingContext",
            "Automation.closeBrowsingContext"
        ]
    );
    assert_eq!(trace.command_ids, [1, 2]);
    assert_eq!(trace.teardowns, 0);
    assert!(source.dropped.load(Ordering::SeqCst));
    assert_eq!(source.calls.load(Ordering::SeqCst), 1);
}
