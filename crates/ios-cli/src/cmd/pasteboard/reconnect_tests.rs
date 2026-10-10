//! Darwin watch driver over owned RSD, RemoteXPC, notification and native peers.
//! USB/mDNS routing and device daemon behavior remain physical NOT_RUN.
use super::*;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::task::Waker;
use tokio::io::AsyncReadExt;

#[path = "reconnect_tests/native.rs"]
mod native;
#[path = "reconnect_tests/wire.rs"]
mod wire;

const UDID: &str = "owned-darwin-watch-device";
const OBSERVE: Duration = Duration::from_secs(4);

#[derive(Clone, Default)]
struct Signal {
    ready: Arc<AtomicBool>,
    delivered: Arc<AtomicUsize>,
    waker: Arc<Mutex<Option<Waker>>>,
}
impl Signal {
    fn trigger(&self) {
        self.ready.store(true, Ordering::SeqCst);
        if let Some(waker) = self.waker.lock().unwrap().take() {
            waker.wake();
        }
    }
}
impl Future for Signal {
    type Output = Result<(), io::Error>;
    fn poll(self: Pin<&mut Self>, cx: &mut std::task::Context<'_>) -> Poll<Self::Output> {
        *self.waker.lock().unwrap() = Some(cx.waker().clone());
        if self.ready.load(Ordering::SeqCst) {
            self.delivered.fetch_add(1, Ordering::SeqCst);
            Poll::Ready(Ok(()))
        } else {
            Poll::Pending
        }
    }
}

fn options() -> WatchOptions {
    WatchOptions {
        reconnect: true,
        reconnect_backoff_ms: 1,
        reconnect_max_backoff_ms: 4,
        connect_timeout: 2,
        cleanup_timeout: 2,
        ..WatchOptions::default()
    }
}
fn baseline() -> usize {
    tokio::runtime::Handle::current()
        .metrics()
        .num_alive_tasks()
}

struct Certificate {
    handles: Vec<Arc<TunnelHandle>>,
    probes: Vec<Arc<native::Probe>>,
    complete_before_recovery: bool,
}

async fn finish(source: &wire::Source, expected: usize, allow_pending: bool) -> Certificate {
    let originals = std::mem::take(&mut *source.seen.native_slots.lock().unwrap());
    let mut joined = true;
    let mut original_eof = true;
    let mut stopped_before_recovery = true;
    let mut complete_before_recovery = true;
    let mut handles = Vec::new();
    let mut probes = Vec::new();
    for (native, mut peer, probe) in originals {
        stopped_before_recovery &= !native.is_alive();
        complete_before_recovery &= native::complete(&native.shutdown_status());
        joined &= native::complete(&native.shutdown(OBSERVE).await);
        original_eof &= matches!(tokio::time::timeout(OBSERVE, peer.read_u8()).await,
            Ok(Err(error)) if error.kind() == io::ErrorKind::UnexpectedEof);
        handles.push(native);
        probes.push(probe);
    }
    let peers = std::mem::take(&mut *source.seen.peers.lock().unwrap());
    let mut protocol_eof = true;
    for peer in peers {
        protocol_eof &= matches!(tokio::time::timeout(OBSERVE, peer).await, Ok(Ok(true)));
    }
    let resources = tokio::time::timeout(OBSERVE, async {
        while baseline() != expected || source.seen.device_slots.load(Ordering::SeqCst) != 0 {
            tokio::task::yield_now().await;
        }
    })
    .await;
    // Recover every original IO peer and join before evaluating business
    // assertions; a retained handle never certifies its own task completion.
    assert!(joined && original_eof && protocol_eof && resources.is_ok());
    assert!(stopped_before_recovery);
    assert!(allow_pending || complete_before_recovery);
    assert_eq!(baseline(), expected);
    assert_eq!(source.seen.device_slots.load(Ordering::SeqCst), 0);
    assert!(probes
        .iter()
        .all(|probe| probe.reads.load(Ordering::SeqCst) > 0
            && probe.drops.load(Ordering::SeqCst) == 1));
    Certificate {
        handles,
        probes,
        complete_before_recovery,
    }
}

fn connected() -> wire::Episode {
    wire::Episode::new([wire::Pull::Snapshot("baseline", 1)], [wire::Notice::Hold])
}
fn has_source<T: Error + 'static>(error: &(dyn Error + 'static)) -> bool {
    let mut next = Some(error);
    for _ in 0..32 {
        let Some(error) = next else { return false };
        if error.is::<T>() {
            return true;
        }
        if let Some(io) = error.downcast_ref::<io::Error>() {
            next = io
                .get_ref()
                .map(|inner| inner as &(dyn Error + 'static))
                .or_else(|| error.source());
        } else {
            next = error.source();
        }
    }
    false
}

#[tokio::test]
async fn pasteboard_reconnect_real_protocol_recovery_silent_baselines_and_last_emitted_id() {
    let expected = baseline();
    let source = wire::Source::new([
        wire::Episode::new(
            [
                wire::Pull::Snapshot("one", 1),
                wire::Pull::Snapshot("one", 1),
                wire::Pull::Snapshot("one", 2),
            ],
            [
                wire::Notice::Unrelated,
                wire::Notice::Change,
                wire::Notice::Change,
                wire::Notice::Death,
            ],
        ),
        wire::Episode::new(
            [
                wire::Pull::Snapshot("two", 100),
                wire::Pull::Snapshot("one", 2),
                wire::Pull::Snapshot("two", 2),
            ],
            [
                wire::Notice::Change,
                wire::Notice::Change,
                wire::Notice::Hold,
            ],
        ),
    ]);
    let signal = Signal::default();
    let mut ids = Vec::new();
    let mut states = Vec::new();
    let result = run(
        &source,
        UDID,
        "general",
        DataInclusionPolicy::PromiseSecondary,
        &options(),
        signal.clone(),
        |event| {
            match event {
                WatchEvent::Snapshot(snapshot) => {
                    let (nonce, count) = snapshot.change_id().unwrap();
                    ids.push((nonce.as_str().unwrap().to_owned(), count));
                    if ids.len() == 2 {
                        signal.trigger();
                    }
                }
                WatchEvent::State(state) => states.push(state),
            }
            Ok(())
        },
    )
    .await;
    let certificate = finish(&source, expected, false).await;
    let report = result.unwrap();
    assert_eq!(ids, [("one".into(), 2), ("two".into(), 2)]);
    assert_eq!(
        (source.calls(), report.retries, report.failures.len()),
        (2, 1, 1)
    );
    assert_eq!(source.seen.discoveries.load(Ordering::SeqCst), 2);
    assert!(source
        .seen
        .registration_before_baseline
        .load(Ordering::SeqCst));
    assert_eq!(
        *source.seen.policies.lock().unwrap(),
        [
            (1, DataInclusionPolicy::AllPromised),
            (1, DataInclusionPolicy::PromiseSecondary),
            (1, DataInclusionPolicy::PromiseSecondary),
            (2, DataInclusionPolicy::AllPromised),
            (2, DataInclusionPolicy::PromiseSecondary),
            (2, DataInclusionPolicy::PromiseSecondary),
        ]
    );
    let services = source.seen.services.lock().unwrap();
    assert!(services
        .iter()
        .any(|(attempt, name, port)| *attempt == 1 && name == "notification" && *port == 21002));
    assert!(services
        .iter()
        .any(|(attempt, name, port)| *attempt == 2 && name == "notification" && *port == 21004));
    assert!(services
        .iter()
        .any(|(attempt, name, port)| *attempt == 2 && name == "pasteboard" && *port == 21005));
    assert!(!Arc::ptr_eq(
        &certificate.handles[0],
        &certificate.handles[1]
    ));
    assert_eq!(signal.delivered.load(Ordering::SeqCst), 1);
    assert!(states.iter().any(|state| state.state == "connected"
        && state.attempt == 1
        && state.gap == Some("changes_during_disconnect_not_replayed")));
    assert!(states
        .iter()
        .filter(|state| state.state == "backoff")
        .all(|state| state.native_reclaimed));
}

#[tokio::test]
async fn pasteboard_reconnect_default_stops_after_one_real_proxy_death() {
    let expected = baseline();
    let source = wire::Source::new([
        wire::Episode::new([wire::Pull::Snapshot("one", 1)], [wire::Notice::Death]),
        connected(),
    ]);
    let mut snapshots = 0;
    let result = run(
        &source,
        UDID,
        "general",
        DataInclusionPolicy::PromiseSecondary,
        &WatchOptions::default(),
        std::future::pending::<Result<(), io::Error>>(),
        |event| {
            if matches!(event, WatchEvent::Snapshot(_)) {
                snapshots += 1;
            }
            Ok(())
        },
    )
    .await;
    finish(&source, expected, false).await;
    let error = result.err().expect("default watch silently retried");
    assert!(has_source::<
        ios_core::notificationproxy::NotificationProxyError,
    >(error.as_ref()));
    assert_eq!((source.calls(), snapshots), (1, 0));
}

#[tokio::test]
async fn pasteboard_reconnect_typed_transport_eof_and_missing_device_use_fresh_native_attempt() {
    for first in [
        wire::Episode::fault(wire::Fault::TypedEof),
        wire::Episode::fault(wire::Fault::MissingDevice),
        wire::Episode::new([wire::Pull::Snapshot("one", 1)], [wire::Notice::Eof]),
        wire::Episode::new([wire::Pull::Eof], [wire::Notice::Hold]),
    ] {
        let expected = baseline();
        let source = wire::Source::new([first, connected()]);
        let signal = Signal::default();
        let result = run(
            &source,
            UDID,
            "general",
            DataInclusionPolicy::PromiseSecondary,
            &options(),
            signal.clone(),
            |event| {
                if matches!(event, WatchEvent::State(ref state) if state.state == "connected" && state.attempt > 0) {
                    signal.trigger();
                }
                Ok(())
            },
        )
        .await;
        let certificate = finish(&source, expected, false).await;
        let report = result.unwrap();
        assert_eq!(
            (source.calls(), report.retries, report.failures.len()),
            (2, 1, 1)
        );
        assert!(!Arc::ptr_eq(
            &certificate.handles[0],
            &certificate.handles[1]
        ));
        assert_eq!(signal.delivered.load(Ordering::SeqCst), 1);
    }
}

#[tokio::test]
async fn pasteboard_reconnect_fatal_typed_and_untyped_host_eof_never_retry() {
    for fault in [
        wire::Fault::Protocol,
        wire::Fault::Unsupported,
        wire::Fault::Permission,
        wire::Fault::Input,
        wire::Fault::Limit,
        wire::Fault::Cancelled,
        wire::Fault::GenericEof,
    ] {
        let expected = baseline();
        let source = wire::Source::new([wire::Episode::fault(fault), connected()]);
        let result = run(
            &source,
            UDID,
            "general",
            DataInclusionPolicy::PromiseSecondary,
            &options(),
            std::future::pending::<Result<(), io::Error>>(),
            |_| Ok(()),
        )
        .await;
        finish(&source, expected, false).await;
        assert!(result.is_err());
        assert_eq!(source.calls(), 1);
    }
}

#[tokio::test]
async fn pasteboard_reconnect_fresh_rsd_identity_and_missing_service_are_final() {
    for identity in [true, false] {
        let expected = baseline();
        let mut episode = connected();
        if identity {
            episode.identity = "owned-other-device";
        } else {
            episode.missing = true;
        }
        let source = wire::Source::new([episode, connected()]);
        let result = run(
            &source,
            UDID,
            "general",
            DataInclusionPolicy::PromiseSecondary,
            &options(),
            std::future::pending::<Result<(), io::Error>>(),
            |_| Ok(()),
        )
        .await;
        finish(&source, expected, false).await;
        let error = result
            .err()
            .expect("invalid fresh RSD metadata was retried");
        assert_eq!(
            crate::output::operation_error_class(error.as_ref()),
            if identity {
                ErrorClass::Protocol
            } else {
                ErrorClass::ServiceMissing
            }
        );
        assert_eq!(
            (
                source.calls(),
                source.seen.discoveries.load(Ordering::SeqCst)
            ),
            (1, 1)
        );
        assert!(source.seen.policies.lock().unwrap().is_empty());
    }
}

#[tokio::test]
async fn pasteboard_reconnect_count_exhaustion_retains_each_original_source_and_backoff_cap() {
    let expected = baseline();
    let source = wire::Source::new((0..4).map(|_| wire::Episode::fault(wire::Fault::TypedEof)));
    let mut options = options();
    options.reconnect_attempts = 2;
    let mut states = Vec::new();
    let result = run(
        &source,
        UDID,
        "general",
        DataInclusionPolicy::PromiseSecondary,
        &options,
        std::future::pending::<Result<(), io::Error>>(),
        |event| {
            if let WatchEvent::State(state) = event {
                states.push(state);
            }
            Ok(())
        },
    )
    .await;
    finish(&source, expected, false).await;
    let error = result.err().expect("retry ceiling did not exhaust");
    assert_eq!(source.calls(), 3);
    assert_eq!(
        error
            .downcast_ref::<FailureContext>()
            .unwrap()
            .earlier
            .len(),
        2
    );
    assert!(has_source::<ios_core::XpcError>(error.as_ref()));
    let backoffs = states
        .iter()
        .filter(|state| state.state == "backoff")
        .map(|state| state.backoff_ms.unwrap())
        .collect::<Vec<_>>();
    assert_eq!(backoffs, [1, 2]);
    assert!(states
        .iter()
        .any(|state| state.state == "exhausted" && state.attempt == 2));
    assert_eq!(options.delay(31), Duration::from_millis(4));
}

#[tokio::test]
async fn pasteboard_reconnect_cancel_at_each_admission_boundary_never_polls_source_or_spends_retry()
{
    for boundary in ["before", "connecting", "backoff", "reconnecting"] {
        let expected = baseline();
        let source = wire::Source::new([wire::Episode::fault(wire::Fault::TypedEof), connected()]);
        let signal = Signal::default();
        if boundary == "before" {
            signal.trigger();
        }
        let result = run(
            &source,
            UDID,
            "general",
            DataInclusionPolicy::PromiseSecondary,
            &options(),
            signal.clone(),
            |event| {
                if matches!(event, WatchEvent::State(ref state) if state.state == boundary) {
                    signal.trigger();
                }
                Ok(())
            },
        )
        .await;
        finish(&source, expected, false).await;
        let report = result.unwrap();
        assert_eq!(report.retries, 0);
        assert_eq!(
            source.calls(),
            usize::from(matches!(boundary, "backoff" | "reconnecting"))
        );
        assert_eq!(signal.delivered.load(Ordering::SeqCst), 1);
    }
}

#[tokio::test]
async fn pasteboard_reconnect_cancel_during_owned_setup_and_partial_notification_discards_original()
{
    for setup in [true, false] {
        let expected = baseline();
        let source = wire::Source::new([if setup {
            wire::Episode::fault(wire::Fault::Pending)
        } else {
            wire::Episode::new([wire::Pull::Snapshot("one", 1)], [wire::Notice::Partial])
        }]);
        let signal = Signal::default();
        let options = options();
        let mut watch = Box::pin(run(
            &source,
            UDID,
            "general",
            DataInclusionPolicy::PromiseSecondary,
            &options,
            signal.clone(),
            |_| Ok(()),
        ));
        let entered = tokio::time::timeout(OBSERVE, async {
            loop {
                if futures_util::poll!(&mut watch).is_ready() {
                    return false;
                }
                let native = source
                    .seen
                    .native_slots
                    .lock()
                    .unwrap()
                    .iter()
                    .any(|(_, _, probe)| probe.reads.load(Ordering::SeqCst) > 0);
                let partial = source.seen.partials.load(Ordering::SeqCst) > 0;
                if native && (setup || partial) {
                    return true;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap_or(false);
        signal.trigger();
        let result = watch.await;
        finish(&source, expected, false).await;
        assert!(entered);
        assert_eq!(result.unwrap().retries, 0);
        assert_eq!(source.calls(), 1);
        assert_eq!(signal.delivered.load(Ordering::SeqCst), 1);
    }
}

#[tokio::test]
async fn pasteboard_reconnect_original_operation_primary_and_native_cleanup_secondary_are_preserved(
) {
    let expected = baseline();
    let mut episode = wire::Episode::fault(wire::Fault::Protocol);
    episode.native = native::Behavior::Failure(io::ErrorKind::BrokenPipe);
    let source = wire::Source::new([episode, connected()]);
    let result = run(
        &source,
        UDID,
        "general",
        DataInclusionPolicy::PromiseSecondary,
        &options(),
        std::future::pending::<Result<(), io::Error>>(),
        |_| Ok(()),
    )
    .await;
    let certificate = finish(&source, expected, false).await;
    let error = result
        .err()
        .expect("Source or native cleanup failure disappeared");
    assert_eq!(
        crate::output::operation_error_class(error.as_ref()),
        ErrorClass::Protocol
    );
    assert!(matches!(
        error.downcast_ref::<CoreError>(),
        Some(CoreError::Protocol(_))
    ));
    let secondary = error
        .downcast_ref::<FailureContext>()
        .unwrap()
        .secondary
        .as_ref()
        .unwrap();
    let native = secondary.downcast_ref::<ios_core::TunnelError>().unwrap();
    let receipt = original_receipt(native).unwrap();
    assert!(receipt.joins_complete() && receipt.has_retained_outcomes());
    let ios_core::TunnelError::Io(io) = receipt
        .primary()
        .downcast_ref::<ios_core::TunnelError>()
        .unwrap()
    else {
        panic!("original native packet reader cause was lost")
    };
    let marker = io
        .get_ref()
        .unwrap()
        .downcast_ref::<native::Marker>()
        .unwrap();
    assert!(Arc::ptr_eq(&marker.0, &certificate.probes[0].marker));
    assert_eq!(source.calls(), 1);
}

#[tokio::test]
async fn pasteboard_reconnect_fatal_original_native_cleanup_blocks_otherwise_retryable_source() {
    let expected = baseline();
    let mut episode = wire::Episode::fault(wire::Fault::TypedEof);
    episode.native = native::Behavior::Failure(io::ErrorKind::InvalidData);
    let source = wire::Source::new([episode, connected()]);
    let result = run(
        &source,
        UDID,
        "general",
        DataInclusionPolicy::PromiseSecondary,
        &options(),
        std::future::pending::<Result<(), io::Error>>(),
        |_| Ok(()),
    )
    .await;
    finish(&source, expected, false).await;
    let error = result
        .err()
        .expect("fatal native cleanup allowed fresh admission");
    assert!(has_source::<ios_core::XpcError>(error.as_ref()));
    let secondary = error
        .downcast_ref::<FailureContext>()
        .unwrap()
        .secondary
        .as_ref()
        .unwrap();
    assert_eq!(
        crate::output::operation_error_class(secondary.as_ref()),
        ErrorClass::Protocol
    );
    assert_eq!(source.calls(), 1);
}

#[tokio::test]
async fn pasteboard_reconnect_output_failure_is_final_and_not_transport_retry_evidence() {
    let expected = baseline();
    let source = wire::Source::new([
        wire::Episode::new(
            [
                wire::Pull::Snapshot("one", 1),
                wire::Pull::Snapshot("one", 2),
            ],
            [wire::Notice::Change, wire::Notice::Hold],
        ),
        connected(),
    ]);
    let result = run(
        &source,
        UDID,
        "general",
        DataInclusionPolicy::PromiseSecondary,
        &options(),
        std::future::pending::<Result<(), io::Error>>(),
        |event| {
            if matches!(event, WatchEvent::Snapshot(_)) {
                return Err(io::Error::new(io::ErrorKind::BrokenPipe, "owned output pipe").into());
            }
            Ok(())
        },
    )
    .await;
    finish(&source, expected, false).await;
    let error = result.err().expect("output failure retried a watch");
    assert!(error.downcast_ref::<OutputFailure>().is_some());
    assert_eq!(source.calls(), 1);
}

#[tokio::test]
async fn pasteboard_reconnect_zero_retries_and_recovery_backoff_deadline_stop_before_fresh_source()
{
    for exhausted in [true, false] {
        let expected = baseline();
        let source = wire::Source::new([wire::Episode::fault(wire::Fault::TypedEof), connected()]);
        let mut options = options();
        if exhausted {
            options.reconnect_attempts = 0;
        } else {
            options.reconnect_elapsed = 1;
            options.reconnect_backoff_ms = 1200;
            options.reconnect_max_backoff_ms = 1200;
        }
        let result = run(
            &source,
            UDID,
            "general",
            DataInclusionPolicy::PromiseSecondary,
            &options,
            std::future::pending::<Result<(), io::Error>>(),
            |_| Ok(()),
        )
        .await;
        finish(&source, expected, false).await;
        let error = result.err().expect("expired recovery started a new Source");
        assert!(has_source::<ios_core::XpcError>(error.as_ref()));
        if !exhausted {
            let completion = error
                .downcast_ref::<FailureContext>()
                .unwrap()
                .secondary
                .as_ref()
                .unwrap();
            assert!(completion.is::<tokio::time::error::Elapsed>());
        }
        assert_eq!(source.calls(), 1);
    }
}

#[tokio::test]
async fn pasteboard_reconnect_cleanup_recovery_clock_preserves_source_and_retains_exact_original_owner(
) {
    let expected = baseline();
    let mut episode = wire::Episode::fault(wire::Fault::TypedEof);
    episode.native = native::Behavior::SlowDrop(Duration::from_millis(1200));
    let source = wire::Source::new([episode, connected()]);
    let mut options = options();
    options.reconnect_elapsed = 1;
    options.cleanup_timeout = 10;
    let began = Instant::now();
    let result = run(
        &source,
        UDID,
        "general",
        DataInclusionPolicy::PromiseSecondary,
        &options,
        std::future::pending::<Result<(), io::Error>>(),
        |_| Ok(()),
    )
    .await;
    let elapsed = began.elapsed();
    let retained = result
        .as_ref()
        .err()
        .and_then(|error| error.downcast_ref::<PendingNative>())
        .map(|owner| owner._handle.clone());
    let certificate = finish(&source, expected, true).await;
    let error = result
        .err()
        .expect("cleanup exceeded recovery time without a failure");
    assert!(elapsed >= Duration::from_secs(1) && elapsed < OBSERVE);
    assert!(has_source::<ios_core::XpcError>(error.as_ref()));
    assert!(Arc::ptr_eq(&retained.unwrap(), &certificate.handles[0]));
    assert_eq!(source.calls(), 1);
    let cleanup = error
        .downcast_ref::<FailureContext>()
        .unwrap()
        .secondary
        .as_ref()
        .unwrap();
    assert!(has_source::<tokio::time::error::Elapsed>(cleanup.as_ref()));
}

#[tokio::test]
async fn pasteboard_reconnect_total_expiry_during_partial_xpc_stops_and_joins_without_reusing_service(
) {
    let expected = baseline();
    let source = wire::Source::new([
        wire::Episode::new(
            [wire::Pull::Snapshot("one", 1), wire::Pull::Partial],
            [wire::Notice::Change, wire::Notice::Hold],
        ),
        connected(),
    ]);
    let mut options = options();
    options.timeout = Some(1);
    let result = run(
        &source,
        UDID,
        "general",
        DataInclusionPolicy::PromiseSecondary,
        &options,
        std::future::pending::<Result<(), io::Error>>(),
        |_| Ok(()),
    )
    .await;
    let retained = result
        .as_ref()
        .err()
        .and_then(|error| error.downcast_ref::<PendingNative>())
        .map(|owner| owner._handle.clone());
    let certificate = finish(&source, expected, true).await;
    let error = result.err().expect("late incomplete XPC frame succeeded");
    assert!(error.is::<tokio::time::error::Elapsed>());
    assert_eq!(source.calls(), 1);
    assert_eq!(
        *source.seen.policies.lock().unwrap(),
        [
            (1, DataInclusionPolicy::AllPromised),
            (1, DataInclusionPolicy::PromiseSecondary)
        ]
    );
    if let Some(retained) = retained {
        assert!(Arc::ptr_eq(&retained, &certificate.handles[0]));
    }
}

#[tokio::test]
async fn pasteboard_reconnect_late_original_source_error_survives_total_expiry_without_retry() {
    let expected = baseline();
    let source = wire::Source::new([
        wire::Episode::fault(wire::Fault::LateProtocol(Duration::from_millis(1200))),
        connected(),
    ]);
    let mut options = options();
    options.timeout = Some(1);
    let result = run(
        &source,
        UDID,
        "general",
        DataInclusionPolicy::PromiseSecondary,
        &options,
        std::future::pending::<Result<(), io::Error>>(),
        |_| Ok(()),
    )
    .await;
    finish(&source, expected, true).await;
    let error = result
        .err()
        .expect("late original Source cause became a success");
    assert!(matches!(
        error.downcast_ref::<CoreError>(),
        Some(CoreError::Protocol(_))
    ));
    assert_eq!(
        crate::output::operation_error_class(error.as_ref()),
        ErrorClass::Protocol
    );
    assert_eq!(source.calls(), 1);
}

#[tokio::test]
async fn pasteboard_reconnect_late_ready_snapshot_never_emits_after_total_clock() {
    let expected = baseline();
    let source = wire::Source::new([wire::Episode::new(
        [
            wire::Pull::Snapshot("one", 1),
            wire::Pull::Late("one", 2, Duration::from_millis(1200)),
        ],
        [wire::Notice::Change, wire::Notice::Hold],
    )]);
    let mut options = options();
    options.timeout = Some(1);
    let mut emitted = 0;
    let result = run(
        &source,
        UDID,
        "general",
        DataInclusionPolicy::PromiseSecondary,
        &options,
        std::future::pending::<Result<(), io::Error>>(),
        |event| {
            if matches!(event, WatchEvent::Snapshot(_)) {
                emitted += 1;
            }
            Ok(())
        },
    )
    .await;
    finish(&source, expected, true).await;
    assert!(result.err().unwrap().is::<tokio::time::error::Elapsed>());
    assert_eq!((source.calls(), emitted), (1, 0));
}

#[tokio::test]
async fn pasteboard_reconnect_native_owner_capacity_and_receipt_are_exact_not_nested() {
    let expected = baseline();
    let source = wire::Source::new([]);
    let owner = NativeOwner::default();
    let mut reservation = owner.reserve().unwrap();
    let (handle, peer, probe) =
        native::start(native::Behavior::Failure(io::ErrorKind::BrokenPipe)).await;
    reservation.attach(handle.clone()).unwrap();
    source
        .seen
        .native_slots
        .lock()
        .unwrap()
        .push((handle.clone(), peer, probe));
    let reserved_rejection = owner.reserve().err().unwrap();
    reservation.complete();
    drop(reservation);
    let retained_rejection = owner.reserve().err().unwrap();
    tokio::task::yield_now().await;
    let cleanup = owner.cleanup(OBSERVE, None).await;
    let empty = owner.retained().is_none() && !owner.0.lock().unwrap().reserved;
    let reuse = owner.reserve().is_ok();
    let native_error = cleanup.native.unwrap();
    let foreign = ios_core::TunnelError::Io(io::Error::other(native_error));
    let nested_rejected = original_receipt(&foreign).is_none();
    let certificate = finish(&source, expected, false).await;
    assert!(
        cleanup.reclaimed
            && empty
            && reuse
            && nested_rejected
            && certificate.complete_before_recovery
    );
    for error in [reserved_rejection, retained_rejection] {
        assert_eq!(
            crate::output::operation_error_class(&error),
            ErrorClass::ResourceLimit
        );
    }
}

#[test]
fn pasteboard_reconnect_options_selector_and_state_text_limits_are_pre_io() {
    assert!(!WatchOptions::default().reconnect);
    for mut options in [
        WatchOptions {
            reconnect_attempts: 33,
            ..options()
        },
        WatchOptions {
            reconnect_elapsed: 0,
            ..options()
        },
        WatchOptions {
            cleanup_timeout: 0,
            ..options()
        },
        WatchOptions {
            connect_timeout: 301,
            ..options()
        },
        WatchOptions {
            timeout: Some(0),
            ..options()
        },
        WatchOptions {
            reconnect_backoff_ms: 10,
            reconnect_max_backoff_ms: 9,
            ..options()
        },
    ] {
        assert!(options.validate(true).is_err());
        options.reconnect = true;
        assert!(options.validate(false).is_err());
    }
    for (udid, name) in [
        ("", "general"),
        ("one\0two", "general"),
        (UDID, ""),
        (UDID, "one\0two"),
    ] {
        assert!(selectors(udid, name, DataInclusionPolicy::PromiseSecondary).is_err());
    }
    assert!(selectors(
        &"x".repeat(4097),
        "general",
        DataInclusionPolicy::PromiseSecondary
    )
    .is_err());
    assert!(selectors(
        UDID,
        &"x".repeat(4097),
        DataInclusionPolicy::PromiseSecondary
    )
    .is_err());
    assert!(selectors(UDID, "general", DataInclusionPolicy::Threshold(-1)).is_err());
    let state = serde_json::to_string(&event(
        "backoff",
        1,
        Some(&wire::Fault::Protocol.error()),
        true,
        Some(Duration::from_millis(4)),
    ))
    .unwrap();
    assert!(!state.contains(UDID) && !state.contains("owned private"));
    assert!(state.contains("changes_during_disconnect_not_replayed"));
}

#[tokio::test]
async fn pasteboard_reconnect_missing_change_id_does_not_erase_last_successfully_emitted_id() {
    let expected = baseline();
    let source = wire::Source::new([
        wire::Episode::new(
            [
                wire::Pull::Snapshot("one", 1),
                wire::Pull::Snapshot("one", 2),
                wire::Pull::Anonymous,
            ],
            [
                wire::Notice::Change,
                wire::Notice::Change,
                wire::Notice::Death,
            ],
        ),
        wire::Episode::new(
            [
                wire::Pull::Snapshot("two", 10),
                wire::Pull::Snapshot("one", 2),
                wire::Pull::Snapshot("two", 2),
            ],
            [
                wire::Notice::Change,
                wire::Notice::Change,
                wire::Notice::Hold,
            ],
        ),
    ]);
    let signal = Signal::default();
    let mut ids = Vec::new();
    let mut anonymous = 0;
    let result = run(
        &source,
        UDID,
        "general",
        DataInclusionPolicy::PromiseSecondary,
        &options(),
        signal.clone(),
        |event| {
            if let WatchEvent::Snapshot(snapshot) = event {
                if let Some((nonce, count)) = snapshot.change_id() {
                    ids.push((nonce.as_str().unwrap().to_owned(), count));
                    if ids.len() == 2 {
                        signal.trigger();
                    }
                } else {
                    anonymous += 1;
                }
            }
            Ok(())
        },
    )
    .await;
    finish(&source, expected, false).await;
    assert_eq!(result.unwrap().retries, 1);
    assert_eq!(ids, [("one".into(), 2), ("two".into(), 2)]);
    assert_eq!(anonymous, 1);
}

#[tokio::test]
async fn pasteboard_reconnect_received_baseline_and_change_protocol_failures_are_final() {
    for baseline_failure in [true, false] {
        let expected = baseline();
        let source = wire::Source::new([
            if baseline_failure {
                wire::Episode::new([wire::Pull::Protocol], [wire::Notice::Hold])
            } else {
                wire::Episode::new(
                    [wire::Pull::Snapshot("one", 1), wire::Pull::Protocol],
                    [wire::Notice::Change, wire::Notice::Hold],
                )
            },
            connected(),
        ]);
        let mut snapshots = 0;
        let result = run(
            &source,
            UDID,
            "general",
            DataInclusionPolicy::PromiseSecondary,
            &options(),
            std::future::pending::<Result<(), io::Error>>(),
            |event| {
                if matches!(event, WatchEvent::Snapshot(_)) {
                    snapshots += 1;
                }
                Ok(())
            },
        )
        .await;
        finish(&source, expected, false).await;
        let error = result
            .err()
            .expect("received protocol error was retried or emitted");
        assert!(matches!(
            error.downcast_ref::<PasteboardError>(),
            Some(PasteboardError::Protocol(_))
        ));
        assert_eq!((source.calls(), snapshots), (1, 0));
    }
}

#[tokio::test]
async fn pasteboard_reconnect_connect_clock_cancels_real_partial_rsd_and_reclaims_original_owner() {
    let expected = baseline();
    let mut episode = connected();
    episode.rsd_partial = true;
    let source = wire::Source::new([episode, connected()]);
    let mut options = options();
    options.connect_timeout = 1;
    options.reconnect_attempts = 0;
    let result = run(
        &source,
        UDID,
        "general",
        DataInclusionPolicy::PromiseSecondary,
        &options,
        std::future::pending::<Result<(), io::Error>>(),
        |_| Ok(()),
    )
    .await;
    finish(&source, expected, false).await;
    let error = result
        .err()
        .expect("partial RSD crossed the connection budget");
    assert!(error.is::<tokio::time::error::Elapsed>());
    assert_eq!(
        (source.calls(), source.seen.partials.load(Ordering::SeqCst)),
        (1, 1)
    );
    assert_eq!(source.seen.discoveries.load(Ordering::SeqCst), 0);
    assert!(source.seen.policies.lock().unwrap().is_empty());
}

#[test]
fn pasteboard_reconnect_cli_defaults_push_conflicts_and_explicit_budget_parsing() {
    use clap::Parser;
    #[derive(clap::Parser)]
    struct Cli {
        #[command(subcommand)]
        command: super::super::PasteboardSub,
    }
    let cli = Cli::parse_from(["pasteboard", "watch"]);
    let super::super::PasteboardSub::Watch {
        recovery,
        autonotify,
        ..
    } = cli.command
    else {
        panic!("watch missing")
    };
    assert!(!autonotify && !recovery.reconnect);
    assert_eq!(
        (
            recovery.reconnect_attempts,
            recovery.reconnect_elapsed,
            recovery.connect_timeout,
            recovery.cleanup_timeout
        ),
        (3, 60, 30, 10)
    );
    assert!(recovery.validate(true).is_ok());
    assert!(Cli::try_parse_from(["pasteboard", "watch", "--autonotify", "--experimental"]).is_ok());
    for flag in [
        "--reconnect",
        "--timeout",
        "--connect-timeout",
        "--cleanup-timeout",
        "--reconnect-attempts",
        "--reconnect-elapsed",
        "--reconnect-backoff-ms",
        "--reconnect-max-backoff-ms",
    ] {
        let mut args = vec![
            "pasteboard",
            "watch",
            "--autonotify",
            "--experimental",
            flag,
        ];
        if flag != "--reconnect" {
            args.push("30");
        }
        assert!(
            Cli::try_parse_from(args).is_err(),
            "push accepted Darwin recovery option {flag}"
        );
    }
    assert!(Cli::try_parse_from(["pasteboard", "watch", "--reconnect-attempts", "0"]).is_err());
    let cli = Cli::parse_from([
        "pasteboard",
        "watch",
        "--reconnect",
        "--reconnect-attempts",
        "0",
        "--reconnect-elapsed",
        "2",
        "--reconnect-backoff-ms",
        "1",
        "--reconnect-max-backoff-ms",
        "2",
        "--timeout",
        "3",
        "--cleanup-timeout",
        "1",
    ]);
    let super::super::PasteboardSub::Watch { recovery, .. } = cli.command else {
        panic!("watch missing")
    };
    assert!(recovery.reconnect && recovery.validate(true).is_ok());
    assert_eq!(
        (recovery.reconnect_attempts, recovery.timeout),
        (0, Some(3))
    );
}

#[tokio::test]
async fn pasteboard_reconnect_caller_drop_requests_stop_on_exact_native_owner_without_new_source() {
    let expected = baseline();
    let source = wire::Source::new([wire::Episode::fault(wire::Fault::Pending), connected()]);
    let options = options();
    let mut watch = Box::pin(run(
        &source,
        UDID,
        "general",
        DataInclusionPolicy::PromiseSecondary,
        &options,
        std::future::pending::<Result<(), io::Error>>(),
        |_| Ok(()),
    ));
    let entered = tokio::time::timeout(OBSERVE, async {
        loop {
            if futures_util::poll!(&mut watch).is_ready() {
                return false;
            }
            if source
                .seen
                .native_slots
                .lock()
                .unwrap()
                .iter()
                .any(|(_, _, probe)| probe.reads.load(Ordering::SeqCst) > 0)
            {
                return true;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap_or(false);
    drop(watch);
    finish(&source, expected, true).await;
    assert!(entered);
    assert_eq!(source.calls(), 1);
}

#[tokio::test]
async fn pasteboard_reconnect_constructed_cli_invalid_selectors_and_push_options_are_pre_io() {
    for oversized in [false, true] {
        let cmd = super::super::PasteboardCmd {
            sub: super::super::PasteboardSub::Watch {
                pasteboard: "general".into(),
                policy: "promisesecondary".into(),
                show_data: false,
                experimental: false,
                autonotify: false,
                recovery: WatchOptions::default(),
            },
        };
        let udid = if oversized {
            "x".repeat(4097)
        } else {
            String::new()
        };
        let error = cmd
            .run(Some(udid), false)
            .await
            .err()
            .expect("invalid selector reached device IO");
        assert_eq!(
            crate::output::operation_error_class(error.as_ref()),
            if oversized {
                ErrorClass::ResourceLimit
            } else {
                ErrorClass::InvalidInput
            }
        );
    }
    let cmd = super::super::PasteboardCmd {
        sub: super::super::PasteboardSub::Watch {
            pasteboard: "general".into(),
            policy: "promisesecondary".into(),
            show_data: false,
            experimental: true,
            autonotify: true,
            recovery: options(),
        },
    };
    let error = cmd
        .run(Some(UDID.into()), false)
        .await
        .err()
        .expect("push recovery reached device IO");
    assert_eq!(
        crate::output::operation_error_class(error.as_ref()),
        ErrorClass::InvalidInput
    );
}
