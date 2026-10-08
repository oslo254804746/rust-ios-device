//! Actual automatic candidate path; recover native workers before assertions.
use super::*;
use futures_util::FutureExt;

struct Observation {
    unpublished: bool,
    calls: usize,
    records: usize,
    queries: usize,
    failed: Option<u32>,
    native: retirement::Snapshot,
    no_panic: bool,
}

async fn automatic(delay: Delay, budget: Duration, insert_delay: bool) -> Observation {
    let expected = baseline();
    let base = LocalSource::new();
    let source = Arc::new(DelayedSource {
        base: base.clone(),
        records: Arc::new(AtomicUsize::new(0)),
        paired: AtomicUsize::new(0),
        delay,
    });
    let state = TunnelAgentState::with_source(
        TunMode::Userspace,
        AccessControl::trusted_test_local(),
        Upstreams::default(),
        source.clone(),
        budget,
    );
    if insert_delay {
        *state.inner.publication_after_insert.lock().unwrap() =
            Some(Box::new(|| std::thread::sleep(CROSS)));
    }
    let ticket = state.inner.lifecycle.ticket(DEVICE).await.unwrap();
    let no_panic = std::panic::AssertUnwindSafe(state.refresh_connect(
        DEVICE,
        if matches!(delay, Delay::Publication) {
            Route::Wifi(None)
        } else {
            Route::Usbmux
        },
        &ticket,
        None,
    ))
    .catch_unwind()
    .await
    .is_ok();
    let observation = Observation {
        unpublished: state.inner.tunnels.read().await.is_empty(),
        calls: base.connect_calls.load(Ordering::SeqCst),
        records: source.records.load(Ordering::SeqCst),
        queries: base.usb_calls.load(Ordering::SeqCst),
        failed: state
            .inner
            .failed_attempts
            .read()
            .await
            .get(DEVICE)
            .map(|failed| failed.fail_count),
        native: state.inner.native.snapshot(),
        no_panic,
    };
    let handles = base.handles.lock().unwrap().clone();
    let recovery = finish(&state, &base, expected).await;
    assert!(
        recovery
            && handles
                .iter()
                .all(|handle| handle.shutdown_status().is_ok())
    );
    observation
}

fn rejected(observation: &Observation) {
    assert!(observation.no_panic && observation.unpublished);
    assert_eq!(observation.calls, 1);
    assert_eq!(observation.queries, 0);
    assert_eq!(observation.failed, Some(1));
    assert_eq!(observation.native.active, 0);
    assert_eq!(observation.native.retired, 1);
    assert_eq!(observation.native.native_handles, 1);
}

#[tokio::test]
async fn automatic_deadline_crossing_metadata_cannot_query_or_publish() {
    let observation = automatic(Delay::Metadata, BUDGET, false).await;
    rejected(&observation);
    assert_eq!(observation.records, 1);
}

#[tokio::test]
async fn automatic_deadline_crossing_source_success_cannot_read_metadata_or_publish() {
    let observation = automatic(Delay::SourceSuccess, BUDGET, false).await;
    rejected(&observation);
    assert_eq!(observation.records, 0);
}

#[tokio::test]
async fn automatic_deadline_crossing_insert_cannot_leave_active_row() {
    let observation = automatic(Delay::Publication, BUDGET, true).await;
    rejected(&observation);
    assert_eq!(observation.records, 1);
}

#[tokio::test]
async fn automatic_deadline_crossing_zero_does_not_contact_source() {
    let observation = automatic(Delay::Publication, Duration::ZERO, false).await;
    assert!(observation.no_panic && observation.unpublished);
    assert_eq!(
        (observation.calls, observation.records, observation.queries),
        (0, 0, 0)
    );
    assert!(observation.failed.is_none());
    assert_eq!(observation.native.occupied(), 0);
}

#[tokio::test]
async fn automatic_deadline_crossing_overflow_rejects_without_source_or_panic() {
    let observation = automatic(Delay::Publication, Duration::MAX, false).await;
    assert!(observation.no_panic && observation.unpublished);
    assert_eq!(
        (observation.calls, observation.records, observation.queries),
        (0, 0, 0)
    );
    assert!(observation.failed.is_none());
    assert_eq!(observation.native.occupied(), 0);
}

#[tokio::test]
async fn automatic_deadline_crossing_scanned_clock_zero_and_overflow_retain_native_categories() {
    let invalid = lifecycle::ConnectionDeadline::for_scan_candidate(Duration::MAX)
        .err()
        .unwrap();
    assert_eq!(
        ios_core::classify_error(invalid.as_ref()),
        ios_core::ErrorClass::InvalidInput
    );
    let clock = lifecycle::ConnectionDeadline::for_scan_candidate(Duration::ZERO).unwrap();
    let polls = AtomicUsize::new(0);
    let result = clock
        .run(async {
            polls.fetch_add(1, Ordering::SeqCst);
            Ok::<_, anyhow::Error>(())
        })
        .await;
    elapsed(&result.unwrap_err());
    assert_eq!(polls.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn automatic_deadline_crossing_scanned_clock_acquired_io_remains_primary() {
    let clock = lifecycle::ConnectionDeadline::for_scan_candidate(BUDGET).unwrap();
    let error = clock
        .run(async {
            std::thread::sleep(CROSS);
            Err::<(), _>(io::Error::new(io::ErrorKind::BrokenPipe, "private-scanned-io").into())
        })
        .await
        .unwrap_err();
    assert_eq!(
        error.downcast_ref::<io::Error>().unwrap().kind(),
        io::ErrorKind::BrokenPipe
    );
    assert_eq!(
        ios_core::classify_error(error.as_ref()),
        ios_core::ErrorClass::Disconnected
    );
}

#[tokio::test]
async fn whole_scan_deadline_crossing_real_native_source_metadata_and_publication_are_retired() {
    for delay in [Delay::SourceSuccess, Delay::Metadata, Delay::Publication] {
        let expected = baseline();
        let base = LocalSource::new();
        let source = Arc::new(DelayedSource {
            base: base.clone(),
            records: Arc::new(AtomicUsize::new(0)),
            paired: AtomicUsize::new(0),
            delay,
        });
        let mut state = TunnelAgentState::with_source(
            TunMode::Userspace,
            AccessControl::trusted_test_local(),
            Upstreams::default(),
            source.clone(),
            OBSERVE,
        );
        Arc::get_mut(&mut state.inner).unwrap().scan_timeout = BUDGET;
        if matches!(delay, Delay::Publication) {
            *state.inner.publication_after_insert.lock().unwrap() =
                Some(Box::new(|| std::thread::sleep(CROSS)));
        }
        let error = state.refresh_once().await.unwrap_err();
        let unpublished = state.inner.tunnels.read().await.is_empty();
        let no_backoff = state.inner.failed_attempts.read().await.is_empty();
        let not_ready = !state.is_ready().await;
        let recovery = finish(&state, &base, expected).await;
        assert!(recovery && base.handle().shutdown_status().is_ok());
        assert!(unpublished && no_backoff && not_ready);
        elapsed(&error);
        assert_eq!(base.connect_calls.load(Ordering::SeqCst), 1);
        assert_eq!(
            source.records.load(Ordering::SeqCst),
            usize::from(!matches!(delay, Delay::SourceSuccess))
        );
        assert_eq!(
            base.usb_calls.load(Ordering::SeqCst),
            if matches!(delay, Delay::Publication) {
                2
            } else {
                1
            }
        );
    }
}
