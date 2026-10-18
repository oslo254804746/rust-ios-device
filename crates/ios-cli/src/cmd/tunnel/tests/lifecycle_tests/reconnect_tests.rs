//! The real automatic candidate/scanner paths with deterministic local sources.
use super::*;
use std::num::NonZeroU32;

#[derive(Default)]
struct CapturedEvents {
    rows: Vec<HashMap<String, String>>,
    overflow: bool,
}

struct ReconnectSubscriber(Arc<StdMutex<CapturedEvents>>);

#[derive(Default)]
struct EventFields(HashMap<String, String>);

impl tracing::field::Visit for EventFields {
    fn record_debug(&mut self, field: &tracing::field::Field, value: &dyn std::fmt::Debug) {
        self.0.insert(field.name().into(), format!("{value:?}"));
    }

    fn record_str(&mut self, field: &tracing::field::Field, value: &str) {
        self.0.insert(field.name().into(), value.into());
    }

    fn record_u64(&mut self, field: &tracing::field::Field, value: u64) {
        self.0.insert(field.name().into(), value.to_string());
    }
}

impl tracing::Subscriber for ReconnectSubscriber {
    fn register_callsite(
        &self,
        _: &'static tracing::Metadata<'static>,
    ) -> tracing::subscriber::Interest {
        tracing::subscriber::Interest::always()
    }

    fn max_level_hint(&self) -> Option<tracing::metadata::LevelFilter> {
        Some(tracing::metadata::LevelFilter::TRACE)
    }

    fn enabled(&self, _: &tracing::Metadata<'_>) -> bool {
        true
    }

    fn new_span(&self, _: &tracing::span::Attributes<'_>) -> tracing::span::Id {
        tracing::span::Id::from_u64(1)
    }

    fn record(&self, _: &tracing::span::Id, _: &tracing::span::Record<'_>) {}

    fn record_follows_from(&self, _: &tracing::span::Id, _: &tracing::span::Id) {}

    fn enter(&self, _: &tracing::span::Id) {}

    fn exit(&self, _: &tracing::span::Id) {}

    fn event(&self, event: &tracing::Event<'_>) {
        let mut fields = EventFields::default();
        event.record(&mut fields);
        if !fields
            .0
            .get("message")
            .is_some_and(|message| message.contains("tunnel reconnect state"))
        {
            return;
        }
        let mut captured = self.0.lock().unwrap();
        if captured.rows.len() < 32 {
            captured.rows.push(fields.0);
        } else {
            captured.overflow = true;
        }
    }
}

fn subscriber() -> (tracing::Dispatch, Arc<StdMutex<CapturedEvents>>) {
    tracing::callsite::rebuild_interest_cache();
    let captured = Arc::new(StdMutex::new(CapturedEvents::default()));
    (
        tracing::Dispatch::new(ReconnectSubscriber(captured.clone())),
        captured,
    )
}

fn policy(
    initial: u64,
    cap: u64,
    attempts: Option<u32>,
    total: Option<u64>,
) -> reconnect::RetryPolicy {
    reconnect::RetryPolicy::new(
        Duration::from_millis(initial),
        Duration::from_millis(cap),
        attempts.map(|value| NonZeroU32::new(value).unwrap()),
        total.map(Duration::from_millis),
    )
    .unwrap()
}

fn configured(source: Arc<FakeSource>, retry: reconnect::RetryPolicy) -> TunnelAgentState {
    TunnelAgentState::with_source_and_retry(
        TunMode::Userspace,
        AccessControl::trusted_test_local(),
        Upstreams::default(),
        source,
        TUNNEL_CONNECT_TIMEOUT,
        retry,
    )
    .unwrap()
}

fn attach(source: &FakeSource, id: u32) {
    *source.usb_details.lock().unwrap() = Some(vec![source::UsbDevice {
        udid: A.into(),
        device_id: id,
        usb: true,
    }]);
    source.connected_ids.lock().unwrap().insert(A.into(), id);
}

#[tokio::test(start_paused = true)]
async fn reconnect_default_production_scans_keep_legacy_schedule_and_unlimited_retries() {
    let source = FakeSource::new();
    source.usb.lock().unwrap().push(A.into());
    source.script(
        A,
        (0..6).map(|_| Behavior::Failure(std::io::ErrorKind::BrokenPipe)),
    );
    let state = state(source.clone(), TUNNEL_CONNECT_TIMEOUT);
    state.refresh_once().await.unwrap();
    for (index, seconds) in [30, 60, 120, 240, 300].into_iter().enumerate() {
        tokio::time::advance(Duration::from_secs(seconds - 1)).await;
        state.refresh_once().await.unwrap();
        assert_eq!(source.count(A), index + 1);
        tokio::time::advance(Duration::from_secs(1)).await;
        state.refresh_once().await.unwrap();
        assert_eq!(source.count(A), index + 2);
        assert_eq!(
            state.inner.failed_attempts.read().await[A].fail_count,
            index as u32 + 2
        );
        assert_eq!(
            state.inner.retry.len(),
            0,
            "default failed write remains the only retained owner"
        );
    }
    state.clear().await;
    assert_eq!(
        state.inner.failure_slots.available_permits(),
        MAX_FAILURE_RECORDS
    );
}

#[tokio::test(start_paused = true)]
async fn reconnect_default_subscriber_omits_episode_fields_while_legacy_scans_back_off() {
    let source = FakeSource::new();
    source.usb.lock().unwrap().push(A.into());
    source.script(
        A,
        (0..4).map(|_| Behavior::Failure(std::io::ErrorKind::BrokenPipe)),
    );
    let state = state(source.clone(), TUNNEL_CONNECT_TIMEOUT);
    let (dispatch, captured) = subscriber();
    let capture = tracing::dispatcher::set_default(&dispatch);
    state.refresh_once().await.unwrap();
    let mut scans = Vec::new();
    for seconds in [30, 60, 120] {
        let before = source.count(A);
        tokio::time::advance(Duration::from_secs(seconds - 1)).await;
        state.refresh_once().await.unwrap();
        let early = source.count(A);
        tokio::time::advance(Duration::from_secs(1)).await;
        state.refresh_once().await.unwrap();
        scans.push((before, early, source.count(A)));
    }
    drop(capture);
    let fail_count = state.inner.failed_attempts.read().await[A].fail_count;
    let ledger_rows = state.inner.retry.len();
    state.clear().await;
    assert_eq!(
        state.inner.failure_slots.available_permits(),
        MAX_FAILURE_RECORDS
    );
    assert_eq!(ledger_rows, 0, "default subscriber retained a retry owner");
    assert_eq!(scans, [(1, 1, 2), (2, 2, 3), (3, 3, 4)]);
    assert_eq!(fail_count, 4);
    let captured = captured.lock().unwrap();
    assert!(!captured.overflow);
    assert_eq!(
        captured.rows.len(),
        8,
        "production reconnect events were not captured"
    );
    for (index, fields) in captured.rows.iter().enumerate() {
        assert_eq!(fields["schedule_source"], "legacy_failure_table");
        assert_eq!(
            fields["state"],
            if index % 2 == 0 {
                "Connecting"
            } else {
                "Backoff"
            }
        );
        assert_eq!(
            fields["reason"],
            if index % 2 == 0 {
                "AttemptStarted"
            } else {
                "ConnectionFailed"
            }
        );
        assert_eq!(
            fields["error_class"],
            if index % 2 == 0 {
                "None"
            } else {
                "Some(Disconnected)"
            }
        );
        assert_ne!(fields["device"], A);
        for unestablished in [
            "attempt",
            "failures",
            "sequence",
            "elapsed_ms",
            "remaining_ms",
            "next_delay_ms",
        ] {
            assert!(
                !fields.contains_key(unestablished),
                "legacy event invented {unestablished}"
            );
        }
    }
}

#[tokio::test(start_paused = true)]
async fn reconnect_configured_subscriber_keeps_authoritative_episode_counters_and_delay() {
    let source = FakeSource::new();
    source.usb.lock().unwrap().push(A.into());
    source.script(
        A,
        (0..2).map(|_| Behavior::Failure(std::io::ErrorKind::BrokenPipe)),
    );
    let state = configured(source.clone(), policy(1_000, 4_000, None, Some(10_000)));
    let (dispatch, captured) = subscriber();
    let capture = tracing::dispatcher::set_default(&dispatch);
    state.refresh_once().await.unwrap();
    tokio::time::advance(Duration::from_secs(1)).await;
    state.refresh_once().await.unwrap();
    drop(capture);
    let snapshot = state.inner.retry.snapshot(A);
    state.clear().await;
    assert_eq!(
        state.inner.failure_slots.available_permits(),
        MAX_FAILURE_RECORDS
    );
    assert_eq!(
        snapshot.unwrap().schedule,
        reconnect::ScheduleProvenance::Episode
    );
    let captured = captured.lock().unwrap();
    assert!(!captured.overflow);
    assert_eq!(captured.rows.len(), 4);
    for (index, fields) in captured.rows.iter().enumerate() {
        assert_eq!(fields["schedule_source"], "episode");
        assert_eq!(fields["sequence"], (index + 1).to_string());
        assert_eq!(fields["attempt"], (index / 2 + 1).to_string());
        assert_eq!(fields["failures"], index.div_ceil(2).to_string());
        assert_eq!(fields["elapsed_ms"], if index < 2 { "0" } else { "1000" });
        assert_eq!(
            fields["remaining_ms"],
            if index < 2 { "10000" } else { "9000" }
        );
        if index % 2 == 1 {
            assert_eq!(
                fields["next_delay_ms"],
                if index == 1 { "1000" } else { "2000" }
            );
            assert_eq!(fields["error_class"], "Some(Disconnected)");
        }
    }
}

#[tokio::test(start_paused = true)]
async fn reconnect_configured_backoff_and_cap_drive_actual_scans() {
    let source = FakeSource::new();
    source.usb.lock().unwrap().push(A.into());
    source.script(
        A,
        (0..5).map(|_| Behavior::Failure(std::io::ErrorKind::ConnectionReset)),
    );
    let state = configured(source.clone(), policy(1_000, 4_000, None, None));
    state.refresh_once().await.unwrap();
    for (call, delay) in [(1, 1_000), (2, 2_000), (3, 4_000), (4, 4_000)] {
        let before = state.inner.retry.snapshot(A).unwrap();
        assert_eq!(before.attempts, call);
        assert_eq!(
            before.next_attempt.unwrap().duration_since(Instant::now()),
            Duration::from_millis(delay)
        );
        state.refresh_once().await.unwrap();
        assert_eq!(source.count(A), call as usize);
        tokio::time::advance(Duration::from_millis(delay - 1)).await;
        state.refresh_once().await.unwrap();
        assert_eq!(source.count(A), call as usize);
        tokio::time::advance(Duration::from_millis(1)).await;
        state.refresh_once().await.unwrap();
        assert_eq!(source.count(A), call as usize + 1);
    }
    state.clear().await;
    assert_eq!(state.inner.retry.len(), 0);
    assert_eq!(
        state.inner.failure_slots.available_permits(),
        MAX_FAILURE_RECORDS
    );
}

#[tokio::test(start_paused = true)]
async fn reconnect_attempt_limit_survives_unwritten_failure_and_held_writer() {
    let source = FakeSource::new();
    source.script(A, [Behavior::Failure(std::io::ErrorKind::ConnectionReset)]);
    let state = configured(source.clone(), policy(1_000, 4_000, Some(1), None));
    let ticket = state.inner.lifecycle.ticket(A).await.unwrap();
    let first = state
        .refresh_connect_with_total(A, Route::Usbmux, &ticket, None, None)
        .await;
    assert!(matches!(first, ScanAttemptOutcome::Failure(_)));
    drop(first); // Simulate the scanner dropping its acquired deferred outcome.
    assert!(state.inner.failed_attempts.read().await.is_empty());
    let writer = state.inner.failed_attempts.write().await;
    for _ in 0..3 {
        let outcome = tokio::time::timeout(
            LOCAL_BUDGET,
            state.refresh_connect_with_total(A, Route::Usbmux, &ticket, None, None),
        )
        .await
        .unwrap();
        assert!(matches!(outcome, ScanAttemptOutcome::NoChange));
        tokio::time::advance(Duration::from_secs(300)).await;
    }
    let row = state.inner.retry.snapshot(A).unwrap();
    assert_eq!((row.phase, row.attempts), (reconnect::Phase::Exhausted, 1));
    assert_eq!(source.count(A), 1);
    assert_eq!(
        state.inner.failure_slots.available_permits(),
        MAX_FAILURE_RECORDS - 1
    );
    drop(writer);
    state.clear().await;
    assert_eq!(
        state.inner.failure_slots.available_permits(),
        MAX_FAILURE_RECORDS
    );
}

#[tokio::test(start_paused = true)]
async fn reconnect_pending_source_obeys_total_started_at_admission() {
    let source = FakeSource::new();
    source.script(A, [Behavior::Pending]);
    let state = configured(source.clone(), policy(1_000, 4_000, None, Some(70)));
    let ticket = state.inner.lifecycle.ticket(A).await.unwrap();
    let start = Instant::now();
    state.refresh_connect(A, Route::Usbmux, &ticket, None).await;
    let snapshot = state.inner.retry.snapshot(A).unwrap();
    assert_eq!(snapshot.first_attempt, start);
    assert_eq!(snapshot.deadline, Some(start + Duration::from_millis(70)));
    assert_eq!(snapshot.phase, reconnect::Phase::Exhausted);
    assert_eq!(snapshot.failures, 1);
    assert_eq!(snapshot.last_class, Some(ios_core::ErrorClass::Timeout));
    assert!(Instant::now() - start < Duration::from_secs(1));
    assert_eq!(source.count(A), 1);
    assert_eq!(source.active.load(Ordering::SeqCst), 0);
    assert_eq!(
        state.inner.lifecycle.connects.available_permits(),
        lifecycle::MAX_CONNECTS
    );
    let failed = state.inner.failed_attempts.read().await;
    assert_eq!(failed[A].fail_count, 1);
    drop(failed);
    state.clear().await;
}

#[tokio::test(start_paused = true)]
async fn reconnect_early_caller_drop_remains_interrupted_without_local_timeout() {
    let source = FakeSource::new();
    source.script(A, [Behavior::Pending]);
    let state = configured(source.clone(), policy(1_000, 4_000, None, Some(10_000)));
    let ticket = state.inner.lifecycle.ticket(A).await.unwrap();
    let mut attempt =
        Box::pin(state.refresh_connect_with_total(A, Route::Usbmux, &ticket, None, None));
    let pending = futures_util::poll!(&mut attempt).is_pending();
    let during = state.inner.retry.snapshot(A);
    drop(attempt);
    let after = state.inner.retry.snapshot(A);
    let released = source.active.load(Ordering::SeqCst) == 0
        && state.inner.lifecycle.connects.available_permits() == lifecycle::MAX_CONNECTS;
    let no_failure_write = state.inner.failed_attempts.read().await.is_empty();
    state.clear().await;
    assert!(released);
    assert_eq!(
        state.inner.failure_slots.available_permits(),
        MAX_FAILURE_RECORDS
    );
    assert!(pending && no_failure_write);
    let during = during.unwrap();
    let after = after.unwrap();
    assert_eq!(during.phase, reconnect::Phase::Connecting);
    assert_eq!(
        (after.phase, after.attempts, after.failures),
        (reconnect::Phase::Interrupted, 1, 0)
    );
    assert_eq!(after.last_class, None);
    assert!(Instant::now() < after.deadline.unwrap());
    assert_eq!(source.count(A), 1);
}

#[tokio::test(start_paused = true)]
async fn reconnect_total_counts_idle_and_does_not_repeat_exhaustion_event() {
    let source = FakeSource::new();
    source.usb.lock().unwrap().push(A.into());
    source.script(A, [Behavior::Failure(std::io::ErrorKind::ConnectionReset)]);
    let state = configured(source.clone(), policy(1_000, 4_000, None, Some(100)));
    state.refresh_once().await.unwrap();
    tokio::time::advance(Duration::from_millis(100)).await;
    state.refresh_once().await.unwrap();
    let first = state.inner.retry.snapshot(A).unwrap();
    assert_eq!(first.phase, reconnect::Phase::Exhausted);
    for _ in 0..3 {
        state.refresh_once().await.unwrap();
    }
    let after = state.inner.retry.snapshot(A).unwrap();
    assert_eq!(after.sequence, first.sequence);
    assert_eq!(source.count(A), 1);
    state.clear().await;
}

#[tokio::test(start_paused = true)]
async fn reconnect_exhausted_a_does_not_delay_b_or_remove_published_b() {
    let source = FakeSource::new();
    *source.usb.lock().unwrap() = vec![A.into(), B.into()];
    source.script(A, [Behavior::Failure(std::io::ErrorKind::ConnectionReset)]);
    let state = configured(source.clone(), policy(1_000, 4_000, Some(1), None));
    state.refresh_once().await.unwrap();
    let before = state.get_record(B).await.unwrap().unwrap();
    let publication = state.inner.tunnels.read().await[B].publication.clone();
    tokio::time::advance(Duration::from_secs(600)).await;
    state.refresh_once().await.unwrap();
    let record = tokio::time::timeout(LOCAL_BUDGET, state.get_record(B))
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    assert_eq!(record, before);
    assert!(state.inner.tunnels.read().await[B]
        .publication
        .ptr_eq(&publication));
    assert_eq!((source.count(A), source.count(B)), (1, 1));
    assert!(tokio::time::timeout(LOCAL_BUDGET, state.remove_tunnel(B))
        .await
        .unwrap());
    assert_eq!(
        state.inner.retry.snapshot(A).unwrap().phase,
        reconnect::Phase::Exhausted
    );
    state.clear().await;
}

#[tokio::test(start_paused = true)]
async fn reconnect_identity_change_starts_new_episode_and_recovery_clears_it() {
    let source = FakeSource::new();
    attach(&source, 71);
    source.script(
        A,
        [
            Behavior::Failure(std::io::ErrorKind::ConnectionReset),
            Behavior::Success,
        ],
    );
    let state = configured(source.clone(), policy(1_000, 4_000, Some(1), None));
    state.refresh_once().await.unwrap();
    state.refresh_once().await.unwrap();
    assert_eq!(source.count(A), 1);
    attach(&source, 72);
    state.refresh_once().await.unwrap();
    assert_eq!(source.count(A), 2);
    assert!(state.get_record(A).await.unwrap().is_some());
    assert!(state.inner.retry.snapshot(A).is_none());
    assert!(state.inner.failed_attempts.read().await.is_empty());
    assert_eq!(
        state.inner.failure_slots.available_permits(),
        MAX_FAILURE_RECORDS
    );
    state.clear().await;
}

#[tokio::test(start_paused = true)]
async fn reconnect_paired_failure_preserves_budget_but_confirmed_host_change_resets_it() {
    let source = FakeSource::new();
    *source.paired.lock().unwrap() = vec![PairedMobdev2Device {
        udid: A.into(),
        host: "192.0.2.1".into(),
    }];
    source.script(
        A,
        [
            Behavior::Failure(std::io::ErrorKind::ConnectionReset),
            Behavior::Success,
        ],
    );
    let state = configured(source.clone(), policy(1_000, 4_000, Some(1), None));
    state.refresh_once().await.unwrap();
    source.paired_failure.store(true, Ordering::SeqCst);
    assert!(state.refresh_once().await.is_err());
    assert_eq!(state.inner.retry.snapshot(A).unwrap().attempts, 1);
    source.paired_failure.store(false, Ordering::SeqCst);
    source.paired.lock().unwrap()[0].host = "192.0.2.2".into();
    state.refresh_once().await.unwrap();
    assert_eq!(source.count(A), 2);
    assert!(
        matches!(&source.calls.lock().unwrap()[1].1, Route::Mobdev2(host) if host == "192.0.2.2")
    );
    assert!(state.inner.retry.snapshot(A).is_none());
    state.clear().await;
}

#[tokio::test(start_paused = true)]
async fn reconnect_unadmitted_semaphore_and_native_owner_do_not_start_episode() {
    for reject_owner in [false, true] {
        let source = FakeSource::new();
        let state = configured(source.clone(), policy(1_000, 4_000, Some(1), Some(70)));
        let ticket = state.inner.lifecycle.ticket(A).await.unwrap();
        let permits = if reject_owner {
            state.inner.native.close_and_stop_all();
            None
        } else {
            Some(
                state
                    .inner
                    .lifecycle
                    .connects
                    .acquire_many(lifecycle::MAX_CONNECTS as u32)
                    .await
                    .unwrap(),
            )
        };
        let outcome = state
            .refresh_connect_with_total(A, Route::Usbmux, &ticket, None, None)
            .await;
        drop(outcome);
        assert!(state.inner.retry.snapshot(A).is_none());
        assert_eq!(source.count(A), 0);
        assert_eq!(state.inner.retry.len(), 0);
        drop(permits);
        state.clear().await;
        assert_eq!(
            state.inner.failure_slots.available_permits(),
            MAX_FAILURE_RECORDS
        );
    }
}

#[tokio::test(start_paused = true)]
async fn reconnect_stop_clear_shutdown_fence_pending_and_queued_attempts() {
    for operation in 0..3 {
        let source = FakeSource::new();
        source.script(A, [Behavior::Pending]);
        let state = configured(source.clone(), policy(1_000, 4_000, Some(3), None));
        let ticket = state.inner.lifecycle.ticket(A).await.unwrap();
        let mut active =
            Box::pin(state.refresh_connect_with_total(A, Route::Usbmux, &ticket, None, None));
        assert!(futures_util::poll!(&mut active).is_pending());
        assert_eq!(state.inner.retry.snapshot(A).unwrap().attempts, 1);
        let mut queued =
            Box::pin(state.refresh_connect_with_total(A, Route::Usbmux, &ticket, None, None));
        assert!(futures_util::poll!(&mut queued).is_pending());
        match operation {
            0 => {
                state.remove_tunnel(A).await;
            }
            1 => state.clear().await,
            _ => state.notify_shutdown(),
        }
        assert!(matches!(active.await, ScanAttemptOutcome::NoChange));
        assert!(matches!(queued.await, ScanAttemptOutcome::NoChange));
        assert_eq!(source.count(A), 1);
        assert_eq!(state.inner.retry.len(), 0);
        assert_eq!(source.active.load(Ordering::SeqCst), 0);
        assert_eq!(
            state.inner.lifecycle.connects.available_permits(),
            lifecycle::MAX_CONNECTS
        );
        state.clear().await;
        assert_eq!(
            state.inner.failure_slots.available_permits(),
            MAX_FAILURE_RECORDS
        );
    }
}

#[tokio::test(start_paused = true)]
async fn reconnect_old_epoch_drop_cannot_clear_fresh_exhausted_episode() {
    let source = FakeSource::new();
    source.script(
        A,
        [
            Behavior::Pending,
            Behavior::Failure(std::io::ErrorKind::BrokenPipe),
        ],
    );
    let state = configured(source.clone(), policy(1_000, 4_000, Some(1), None));
    let old_ticket = state.inner.lifecycle.ticket(A).await.unwrap();
    let mut old =
        Box::pin(state.refresh_connect_with_total(A, Route::Usbmux, &old_ticket, None, None));
    assert!(futures_util::poll!(&mut old).is_pending());
    // A reusable clear admits a fresh automatic incarnation. DELETE now
    // deliberately keeps this device stopped; its reactivation is separately
    // exercised through an authorized successful HTTP start.
    state.clear().await;
    // The old Source future still owns the original serial guard. It must drop
    // before a new connection can proceed; its eventual completion stays stale.
    assert!(matches!(old.await, ScanAttemptOutcome::NoChange));
    let fresh = state.inner.lifecycle.ticket(A).await.unwrap();
    let outcome = state
        .refresh_connect_with_total(A, Route::Usbmux, &fresh, None, None)
        .await;
    assert!(matches!(outcome, ScanAttemptOutcome::Failure(_)));
    drop(outcome);
    let snapshot = state.inner.retry.snapshot(A).unwrap();
    assert_eq!(
        (snapshot.phase, snapshot.attempts),
        (reconnect::Phase::Exhausted, 1)
    );
    let obsolete = state
        .refresh_connect_with_total(A, Route::Usbmux, &old_ticket, None, None)
        .await;
    assert!(matches!(obsolete, ScanAttemptOutcome::NoChange));
    let after = state.inner.retry.snapshot(A).unwrap();
    assert_eq!(
        (after.sequence, after.first_attempt),
        (snapshot.sequence, snapshot.first_attempt)
    );
    assert_eq!(source.count(A), 2);
    state.clear().await;
}

#[tokio::test(start_paused = true)]
async fn reconnect_complete_detach_releases_gate_and_permit_but_not_partial_discovery() {
    let source = FakeSource::new();
    source.usb.lock().unwrap().push(A.into());
    source.script(
        A,
        [
            Behavior::Failure(std::io::ErrorKind::ConnectionReset),
            Behavior::Success,
        ],
    );
    let state = configured(source.clone(), policy(1_000, 4_000, Some(1), None));
    state.refresh_once().await.unwrap();
    assert_eq!(state.inner.retry.len(), 1);
    source.usb.lock().unwrap().clear();
    state.refresh_once().await.unwrap();
    assert_eq!(state.inner.retry.len(), 0);
    assert_eq!(
        state.inner.failure_slots.available_permits(),
        MAX_FAILURE_RECORDS
    );
    source.usb.lock().unwrap().push(A.into());
    state.refresh_once().await.unwrap();
    assert_eq!(source.count(A), 2);
    assert!(state.get_record(A).await.unwrap().is_some());
    state.clear().await;
}

#[tokio::test(start_paused = true)]
async fn reconnect_remaining_total_bounds_contested_metadata_without_an_extra_attempt() {
    let source = FakeSource::new();
    source.script(A, [Behavior::Failure(std::io::ErrorKind::ConnectionReset)]);
    let state = configured(source.clone(), policy(1, 4, Some(3), Some(70)));
    let ticket = state.inner.lifecycle.ticket(A).await.unwrap();
    let first = state
        .refresh_connect_with_total(A, Route::Usbmux, &ticket, None, None)
        .await;
    assert!(matches!(first, ScanAttemptOutcome::Failure(_)));
    drop(first);
    tokio::time::advance(Duration::from_millis(1)).await;
    let held = state.inner.failed_attempts.write().await;
    let began = Instant::now();
    let outcome = tokio::time::timeout(
        LOCAL_BUDGET,
        state.refresh_connect_with_total(A, Route::Usbmux, &ticket, None, None),
    )
    .await
    .unwrap();
    assert!(matches!(outcome, ScanAttemptOutcome::NoChange));
    assert!(Instant::now() - began < LOCAL_BUDGET);
    let after = state.inner.retry.snapshot(A).unwrap();
    assert_eq!(
        (after.phase, after.attempts),
        (reconnect::Phase::Exhausted, 1)
    );
    assert_eq!(source.count(A), 1);
    drop(held);
    state.clear().await;
}

#[tokio::test(start_paused = true)]
async fn reconnect_explicit_success_ends_the_prior_automatic_failure_episode() {
    let source = FakeSource::new();
    source.usb.lock().unwrap().push(A.into());
    source.script(
        A,
        [
            Behavior::Failure(std::io::ErrorKind::ConnectionReset),
            Behavior::Success,
            Behavior::Failure(std::io::ErrorKind::BrokenPipe),
        ],
    );
    let state = configured(source.clone(), policy(1_000, 4_000, Some(1), None));
    state.refresh_once().await.unwrap();
    assert_eq!(
        state.inner.retry.snapshot(A).unwrap().phase,
        reconnect::Phase::Exhausted
    );
    start(state.clone(), A).await.unwrap();
    assert!(state.inner.retry.snapshot(A).is_none());
    // The existing failure backoff remains until its normal reconciliation, so
    // advance its schedule before independently failing this new live runtime.
    tokio::time::advance(Duration::from_secs(2)).await;
    source.alive.store(false, Ordering::SeqCst);
    state.refresh_once().await.unwrap();
    assert_eq!(source.count(A), 3);
    assert_eq!(state.inner.retry.snapshot(A).unwrap().attempts, 1);
    state.clear().await;
}

#[tokio::test(start_paused = true)]
async fn reconnect_ledger_reservations_are_bounded_by_original_failure_capacity() {
    let source = FakeSource::new();
    *source.usb.lock().unwrap() = vec![A.into(), B.into(), "synthetic-device-c".into()];
    for udid in [A, B, "synthetic-device-c"] {
        source.script(udid, [Behavior::Failure(std::io::ErrorKind::BrokenPipe)]);
    }
    let mut state = configured(source.clone(), policy(1_000, 4_000, Some(1), None));
    state.inner.get_mut().unwrap().failure_slots = Arc::new(Semaphore::new(2));
    state.refresh_once().await.unwrap();
    assert_eq!(
        (
            source.count(A),
            source.count(B),
            source.count("synthetic-device-c")
        ),
        (1, 1, 0)
    );
    assert_eq!(state.inner.retry.len(), 2);
    assert!(state.inner.retry.snapshot("synthetic-device-c").is_none());
    assert_eq!(state.inner.failure_slots.available_permits(), 0);
    for _ in 0..3 {
        state.refresh_once().await.unwrap();
    }
    assert_eq!(source.count("synthetic-device-c"), 0);
    state.clear().await;
    assert_eq!(state.inner.retry.len(), 0);
    assert_eq!(state.inner.failure_slots.available_permits(), 2);
}

#[tokio::test(start_paused = true)]
async fn reconnect_obsolete_success_version_cannot_clear_a_newer_real_attempt() {
    let source = FakeSource::new();
    source.usb.lock().unwrap().push(A.into());
    source.script(
        A,
        [
            Behavior::Failure(std::io::ErrorKind::ConnectionReset),
            Behavior::Failure(std::io::ErrorKind::BrokenPipe),
        ],
    );
    let state = configured(source.clone(), policy(1_000, 4_000, Some(3), None));
    state.refresh_once().await.unwrap();
    let ticket = state.inner.lifecycle.ticket(A).await.unwrap();
    let obsolete = state.inner.retry.version(A, &ticket).unwrap();
    tokio::time::advance(Duration::from_secs(1)).await;
    state.refresh_once().await.unwrap();
    let before = state.inner.retry.snapshot(A).unwrap();
    state.inner.retry.recovered(A, &ticket, &obsolete);
    let after = state.inner.retry.snapshot(A).unwrap();
    assert_eq!(
        (after.attempts, after.sequence, after.first_attempt),
        (before.attempts, before.sequence, before.first_attempt)
    );
    assert_eq!(after.attempts, 2);
    assert_eq!(source.count(A), 2);
    state.clear().await;
}

#[test]
fn reconnect_invalid_policy_is_rejected_before_a_manager_can_start() {
    for (initial, cap, total) in [
        (Duration::ZERO, Duration::from_secs(1), None),
        (Duration::from_secs(2), Duration::from_secs(1), None),
        (
            Duration::from_secs(1),
            Duration::from_secs(2),
            Some(Duration::ZERO),
        ),
        (Duration::from_secs(1), Duration::MAX, None),
        (
            Duration::from_secs(1),
            Duration::from_secs(2),
            Some(Duration::MAX),
        ),
    ] {
        let error = reconnect::RetryPolicy::new(initial, cap, None, total).unwrap_err();
        assert_eq!(
            ios_core::classify_error(error.as_ref()),
            ios_core::ErrorClass::InvalidInput
        );
    }
}
