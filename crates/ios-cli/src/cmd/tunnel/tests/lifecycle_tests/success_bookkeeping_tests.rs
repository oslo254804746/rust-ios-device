//! Successful publications must release the candidate window before bookkeeping.
use super::*;
use crate::cmd::tunnel::marker_identity::Cutover;
use bookkeeping::{ScanAttemptOutcome, ScanIntent, SuccessIntent};
use futures_util::FutureExt;

const WRITE_BUDGET: Duration = Duration::from_millis(40);
const WINDOW_BUDGET: Duration = Duration::from_millis(200);
const FIFTH: &str = "synthetic-success-bookkeeping-4";

fn bounded(source: Arc<FakeSource>, write: Duration, total: Duration) -> TunnelAgentState {
    let mut manager = state(source, Duration::from_secs(5));
    let inner = manager.inner.get_mut().unwrap();
    inner.failure_write_timeout = write;
    inner.scan_timeout = total;
    manager
}

fn identity() -> AttemptIdentity {
    AttemptIdentity {
        route: Route::Usbmux,
        native_id: Some(1),
    }
}

async fn seed(state: &TunnelAgentState, udid: &str) {
    let ticket = state.inner.lifecycle.ticket(udid).await.unwrap();
    let slot = state.reserve_failure_slot(udid).await.unwrap();
    state
        .record_failed_attempt(udid, identity(), slot, &ticket)
        .await;
    state
        .inner
        .failed_attempts
        .write()
        .await
        .get_mut(udid)
        .unwrap()
        .last_attempt -= Duration::from_secs(31);
}

async fn fixture(
    write: Duration,
    total: Duration,
) -> (
    Arc<FakeSource>,
    TunnelAgentState,
    Arc<Semaphore>,
    Arc<Semaphore>,
) {
    let source = FakeSource::new();
    let first = Arc::new(Semaphore::new(0));
    let fifth = Arc::new(Semaphore::new(0));
    let manager = bounded(source.clone(), write, total);
    for index in 0..5 {
        let udid = format!("synthetic-success-bookkeeping-{index}");
        source.usb.lock().unwrap().push(udid.clone());
        source.script(
            &udid,
            [Behavior::Held(if index < 4 {
                first.clone()
            } else {
                fifth.clone()
            })],
        );
        seed(&manager, &udid).await;
    }
    (source, manager, first, fifth)
}

async fn drive_until(
    scan: &mut std::pin::Pin<Box<impl std::future::Future<Output = Result<()>>>>,
    source: &FakeSource,
    calls: usize,
) -> bool {
    tokio::time::timeout(WINDOW_BUDGET, async {
        loop {
            if futures_util::poll!(scan.as_mut()).is_ready() {
                return false;
            }
            if source.calls.lock().unwrap().len() == calls {
                return true;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap_or(false)
}

async fn recover(state: &TunnelAgentState, source: &FakeSource, clear: &mut Cutover<2>) {
    state.clear(clear).await;
    assert_eq!(source.active.load(Ordering::SeqCst), 0);
    assert_eq!(
        state.inner.lifecycle.connects.available_permits(),
        lifecycle::MAX_CONNECTS
    );
    assert_eq!(
        state.inner.failure_slots.available_permits(),
        MAX_FAILURE_RECORDS
    );
    assert!(state.inner.tunnels.read().await.is_empty());
    assert!(state.inner.failed_attempts.read().await.is_empty());
}

fn elapsed(error: &anyhow::Error) {
    assert!(error.is::<tokio::time::error::Elapsed>());
    assert_eq!(
        ios_core::classify_error(error.as_ref()),
        ios_core::ErrorClass::Timeout
    );
}

#[tokio::test]
async fn lifecycle_tunnel_success_writers_do_not_occupy_later_candidate_window() {
    let (source, state, first, fifth) = fixture(WRITE_BUDGET, Duration::from_secs(5)).await;
    let mut recover_input = state.prepare_cutover::<2>().unwrap();
    let mut scan = Box::pin(state.refresh_once());
    let initial = drive_until(&mut scan, &source, 4).await;
    let failed = state.inner.failed_attempts.read().await;
    first.add_permits(4);
    let later = drive_until(&mut scan, &source, 5).await;
    let first_published = state.list_records().await.unwrap().len();
    fifth.add_permits(1);
    let result = if later {
        Some(tokio::time::timeout(LOCAL_BUDGET, scan.as_mut()).await)
    } else {
        None
    };
    let published = state.list_records().await.unwrap().len();
    let unchanged = failed.len() == 5 && failed.values().all(|row| row.fail_count == 1);
    let slots = state.inner.failure_slots.available_permits();
    let not_ready = !state.is_ready().await;
    drop(scan);
    drop(failed);
    recover(&state, &source, &mut recover_input).await;
    drop(recover_input);

    assert!(
        initial && later,
        "success writers blocked the fifth candidate's read"
    );
    assert_eq!(first_published, 4);
    assert_eq!(published, 5);
    assert!(
        unchanged && not_ready,
        "cleanup timeout created fresh backoff or readiness"
    );
    assert_eq!(source.count(FIFTH), 1);
    assert_eq!(slots, MAX_FAILURE_RECORDS - 5);
    elapsed(
        &result
            .unwrap()
            .expect("success cleanup exceeded its finite total")
            .unwrap_err(),
    );
}

#[tokio::test]
async fn lifecycle_tunnel_success_flush_removes_old_rows_after_reader_releases() {
    let (source, state, first, fifth) =
        fixture(Duration::from_secs(1), Duration::from_secs(5)).await;
    let mut recover_input = state.prepare_cutover::<2>().unwrap();
    let mut scan = Box::pin(state.refresh_once());
    let initial = drive_until(&mut scan, &source, 4).await;
    let failed = state.inner.failed_attempts.read().await;
    first.add_permits(4);
    let later = drive_until(&mut scan, &source, 5).await;
    fifth.add_permits(1);
    drop(failed);
    let result = tokio::time::timeout(LOCAL_BUDGET, scan.as_mut()).await;
    drop(scan);
    let cleared = state.inner.failed_attempts.read().await.is_empty();
    let slots = state.inner.failure_slots.available_permits();
    let ready = state.is_ready().await;
    let published = state.list_records().await.unwrap().len();
    recover(&state, &source, &mut recover_input).await;
    drop(recover_input);

    assert!(
        initial && later,
        "success cleanup delayed the connection window"
    );
    result
        .expect("released success cleanup did not finish")
        .unwrap();
    assert!(cleared && ready);
    assert_eq!(published, 5);
    assert_eq!(slots, MAX_FAILURE_RECORDS);
}

#[tokio::test]
async fn lifecycle_tunnel_success_bookkeeping_whole_clear_and_close_never_mark_ready() {
    for fence in 0..3 {
        let (source, state, first, fifth) = fixture(
            Duration::from_secs(5),
            if fence == 0 {
                Duration::from_millis(400)
            } else {
                Duration::from_secs(5)
            },
        )
        .await;
        let mut clear_input = (fence == 1).then(|| state.prepare_cutover::<2>().unwrap());
        let mut close_input = (fence == 2).then(|| state.prepare_cutover::<1>().unwrap());
        let mut recover_input = state.prepare_cutover::<2>().unwrap();
        let mut scan = Box::pin(state.refresh_once());
        let initial = drive_until(&mut scan, &source, 4).await;
        let failed = state.inner.failed_attempts.read().await;
        first.add_permits(4);
        let later = drive_until(&mut scan, &source, 5).await;
        fifth.add_permits(1);
        // Poll the fifth publication through to the held bookkeeping writer.
        let pending = futures_util::poll!(scan.as_mut()).is_pending();
        let published = state.list_records().await.unwrap().len();
        if fence == 1 {
            state.inner.lifecycle.clear(clear_input.as_mut().unwrap());
        } else if fence == 2 {
            state.notify_shutdown(close_input.as_mut().unwrap());
        }
        let result = tokio::time::timeout(LOCAL_BUDGET, scan.as_mut()).await;
        let unchanged = failed.len() == 5 && failed.values().all(|row| row.fail_count == 1);
        let not_ready = !state.is_ready().await;
        drop(scan);
        drop(failed);
        let no_late_ready = !state.is_ready().await;
        recover(&state, &source, &mut recover_input).await;
        drop(recover_input);

        assert!(initial && later && pending);
        assert_eq!(published, 5);
        assert!(unchanged && not_ready && no_late_ready);
        let result = result.expect("whole deadline or epoch did not release success bookkeeping");
        if fence == 0 {
            elapsed(&result.unwrap_err());
        } else {
            result.unwrap();
        }
    }
}

async fn deferred_success(state: &TunnelAgentState, udid: &str) -> Option<SuccessIntent> {
    let ticket = state.inner.lifecycle.ticket(udid).await.unwrap();
    let reader = state.inner.failed_attempts.read().await;
    let outcome = tokio::time::timeout(
        WINDOW_BUDGET,
        state.refresh_connect_with_total(udid, Route::Usbmux, &ticket, Some(1), None),
    )
    .await;
    drop(reader);
    match outcome {
        Ok(ScanAttemptOutcome::Success(success, ticket)) => Some(SuccessIntent {
            _key_storage: None,
            _input: None,
            udid: udid.into(),
            ticket,
            success,
        }),
        _ => None,
    }
}

#[tokio::test]
async fn lifecycle_tunnel_old_success_intent_preserves_later_same_identity_failure() {
    let source = FakeSource::new();
    let state = bounded(
        source.clone(),
        Duration::from_secs(1),
        Duration::from_secs(5),
    );
    let mut recover_input = state.prepare_cutover::<2>().unwrap();
    seed(&state, A).await;
    let original = {
        let failed = state.inner.failed_attempts.read().await;
        (failed[A].last_attempt, failed[A].fail_count)
    };
    let pending = deferred_success(&state, A).await;
    let ticket = state.inner.lifecycle.ticket(A).await.unwrap();
    let slot = state.reserve_failure_slot(A).await.unwrap();
    state
        .record_failed_attempt(A, identity(), slot, &ticket)
        .await;
    // Make every visible value collide with the old snapshot. The newer
    // actual write's opaque version is the only protection left for this row.
    {
        let mut failed = state.inner.failed_attempts.write().await;
        let row = failed.get_mut(A).unwrap();
        row.last_attempt = original.0;
        row.fail_count = original.1;
    }
    let before = {
        let failed = state.inner.failed_attempts.read().await;
        let row = &failed[A];
        (
            row.last_attempt,
            row.fail_count,
            row.slot
                .as_ref()
                .map(|slot| &**slot as *const failure_slot::Slot),
        )
    };
    let had_intent = pending.is_some();
    let result = state
        .flush_scan_intents_with_total(pending.into_iter().map(ScanIntent::Success).collect(), None)
        .await;
    let preserved = {
        let failed = state.inner.failed_attempts.read().await;
        failed.get(A).is_some_and(|row| {
            (
                row.last_attempt,
                row.fail_count,
                row.slot
                    .as_ref()
                    .map(|slot| &**slot as *const failure_slot::Slot),
            ) == before
        })
    };
    let slots = state.inner.failure_slots.available_permits();
    recover(&state, &source, &mut recover_input).await;
    drop(recover_input);

    assert!(
        had_intent && preserved,
        "old success erased a newer failure sharing its permit"
    );
    result.unwrap();
    assert_eq!(before.1, original.1);
    assert_eq!(slots, MAX_FAILURE_RECORDS - 1);
}

#[tokio::test]
async fn lifecycle_tunnel_old_success_intent_preserves_new_clear_epoch_failure() {
    let source = FakeSource::new();
    let state = bounded(
        source.clone(),
        Duration::from_secs(1),
        Duration::from_secs(5),
    );
    let mut recover_input = state.prepare_cutover::<2>().unwrap();
    let mut prepared_outer_0 = state.prepare_cutover::<2>().unwrap();

    seed(&state, A).await;
    let pending = deferred_success(&state, A).await;
    state.clear(&mut prepared_outer_0).await;
    drop(prepared_outer_0);
    seed(&state, A).await;
    let before = {
        let failed = state.inner.failed_attempts.read().await;
        (failed[A].last_attempt, failed[A].fail_count)
    };
    let had_intent = pending.is_some();
    let result = state
        .flush_scan_intents_with_total(pending.into_iter().map(ScanIntent::Success).collect(), None)
        .await;
    let preserved = {
        let failed = state.inner.failed_attempts.read().await;
        failed
            .get(A)
            .is_some_and(|row| (row.last_attempt, row.fail_count) == before)
    };
    let slots = state.inner.failure_slots.available_permits();
    recover(&state, &source, &mut recover_input).await;
    drop(recover_input);

    assert!(
        had_intent && preserved,
        "old epoch success erased the replacement failure"
    );
    result.unwrap();
    assert_eq!(slots, MAX_FAILURE_RECORDS - 1);
}

#[tokio::test]
async fn lifecycle_tunnel_success_cleanup_deadline_crossing_restores_original_row_and_slot() {
    for after_remove in [false, true] {
        let source = FakeSource::new();
        let state = bounded(source.clone(), WRITE_BUDGET, Duration::from_secs(5));
        let mut recover_input = state.prepare_cutover::<2>().unwrap();
        seed(&state, A).await;
        let pending = deferred_success(&state, A).await;
        let before = {
            let failed = state.inner.failed_attempts.read().await;
            let row = &failed[A];
            (
                row.last_attempt,
                row.fail_count,
                row.slot
                    .as_ref()
                    .map(|slot| &**slot as *const failure_slot::Slot),
                row.version.as_ptr(),
            )
        };
        let hook = if after_remove {
            &state.inner.success_after_remove
        } else {
            &state.inner.success_before_remove
        };
        *hook.lock().unwrap() = Some(Box::new(|| {
            std::thread::sleep(Duration::from_millis(100));
        }));
        let had_intent = pending.is_some();
        let result = state
            .flush_scan_intents_with_total(
                pending.into_iter().map(ScanIntent::Success).collect(),
                None,
            )
            .await;
        let restored = {
            let failed = state.inner.failed_attempts.read().await;
            failed.get(A).is_some_and(|row| {
                (
                    row.last_attempt,
                    row.fail_count,
                    row.slot
                        .as_ref()
                        .map(|slot| &**slot as *const failure_slot::Slot),
                    row.version.as_ptr(),
                ) == before
            })
        };
        let slots = state.inner.failure_slots.available_permits();
        let published = state.get_record(A).await.unwrap().is_some();
        recover(&state, &source, &mut recover_input).await;
        drop(recover_input);

        assert!(had_intent && restored && published);
        assert_eq!(slots, MAX_FAILURE_RECORDS - 1);
        elapsed(&result.unwrap_err());
    }
}

#[tokio::test]
async fn lifecycle_tunnel_success_cleanup_timely_prefix_survives_shared_total_expiry() {
    let source = FakeSource::new();
    let state = bounded(
        source.clone(),
        Duration::from_millis(80),
        Duration::from_secs(5),
    );
    let mut recover_input = state.prepare_cutover::<2>().unwrap();
    seed(&state, A).await;
    seed(&state, B).await;
    let a = deferred_success(&state, A).await;
    let b = deferred_success(&state, B).await;
    let had_intents = a.is_some() && b.is_some();
    let weak = state.inner.downgrade();
    *state.inner.success_after_remove.lock().unwrap() = Some(Box::new(move || {
        std::thread::sleep(Duration::from_millis(50));
        *weak.upgrade().unwrap().success_after_remove.lock().unwrap() = Some(Box::new(|| {
            std::thread::sleep(Duration::from_millis(50));
        }));
    }));
    let result = state
        .flush_scan_intents_with_total(
            a.into_iter().chain(b).map(ScanIntent::Success).collect(),
            None,
        )
        .await;
    let prefix = {
        let failed = state.inner.failed_attempts.read().await;
        failed.len() == 1 && failed.contains_key(B)
    };
    let slots = state.inner.failure_slots.available_permits();
    recover(&state, &source, &mut recover_input).await;
    drop(recover_input);

    assert!(
        had_intents && prefix,
        "success removals reset their shared absolute total"
    );
    assert_eq!(slots, MAX_FAILURE_RECORDS - 1);
    elapsed(&result.unwrap_err());
}

#[tokio::test]
async fn lifecycle_tunnel_success_cleanup_stop_a_preserves_a_and_still_removes_b() {
    let source = FakeSource::new();
    let state = bounded(
        source.clone(),
        Duration::from_secs(1),
        Duration::from_secs(5),
    );
    let retained_stop = Arc::new(StdMutex::new(state.prepare_cutover::<3>().unwrap()));
    let callback_stop = retained_stop.clone();
    let mut recover_input = state.prepare_cutover::<2>().unwrap();
    seed(&state, A).await;
    seed(&state, B).await;
    let a = deferred_success(&state, A).await;
    let b = deferred_success(&state, B).await;
    let had_intents = a.is_some() && b.is_some();
    let weak = state.inner.downgrade();
    *state.inner.success_before_remove.lock().unwrap() = Some(Box::new(move || {
        weak.upgrade()
            .unwrap()
            .lifecycle
            .stop(A, &mut callback_stop.lock().unwrap())
            .now_or_never()
            .expect("uncontended stop must complete");
    }));
    let result = state
        .flush_scan_intents_with_total(
            a.into_iter().chain(b).map(ScanIntent::Success).collect(),
            None,
        )
        .await;
    let only_a = {
        let failed = state.inner.failed_attempts.read().await;
        failed.len() == 1 && failed.contains_key(A)
    };
    let slots = state.inner.failure_slots.available_permits();
    recover(&state, &source, &mut recover_input).await;
    drop(recover_input);

    assert!(
        had_intents && only_a,
        "stopping A suppressed B's successful cleanup"
    );
    result.unwrap();
    assert_eq!(slots, MAX_FAILURE_RECORDS - 1);
}

#[tokio::test]
async fn lifecycle_tunnel_success_fast_cleanup_returns_full_cache_slot_to_next_candidate() {
    let source = FakeSource::new();
    *source.usb.lock().unwrap() = vec![A.into(), B.into()];
    source.paired_failure.store(true, Ordering::SeqCst);
    let state = bounded(
        source.clone(),
        Duration::from_secs(1),
        Duration::from_secs(5),
    );
    let mut recover_input = state.prepare_cutover::<2>().unwrap();
    seed(&state, A).await;
    for index in 0..MAX_FAILURE_RECORDS - 1 {
        let udid = format!("synthetic-success-full-cache-{index}");
        let ticket = state.inner.lifecycle.ticket(&udid).await.unwrap();
        let slot = state.reserve_failure_slot(&udid).await.unwrap();
        state
            .record_failed_attempt(
                &udid,
                AttemptIdentity {
                    route: Route::Mobdev2("192.0.2.9".into()),
                    native_id: None,
                },
                slot,
                &ticket,
            )
            .await;
    }
    let full = state.inner.failure_slots.available_permits() == 0;
    // Complete A before the scan admits B. This checks capacity handoff
    // without depending on buffer_unordered's cooperative polling order.
    let ticket = state.inner.lifecycle.ticket(A).await.unwrap();
    let outcome = state
        .refresh_connect_with_total(A, Route::Usbmux, &ticket, Some(1), None)
        .await;
    let immediate = matches!(outcome, ScanAttemptOutcome::NoChange)
        && !state.inner.failed_attempts.read().await.contains_key(A)
        && state.inner.failure_slots.available_permits() == 1;
    let result = tokio::time::timeout(LOCAL_BUDGET, state.refresh_once()).await;
    let published = state.get_record(A).await.unwrap().is_some()
        && state.get_record(B).await.unwrap().is_some();
    let remaining = state.inner.failed_attempts.read().await.len();
    let slots = state.inner.failure_slots.available_permits();
    recover(&state, &source, &mut recover_input).await;
    drop(recover_input);

    assert!(
        full && immediate && published,
        "successful A withheld its full-cache slot from B"
    );
    assert_eq!((source.count(A), source.count(B)), (1, 1));
    assert_eq!(remaining, MAX_FAILURE_RECORDS - 1);
    assert_eq!(slots, 1);
    let error = result.expect("full-cache scan did not finish").unwrap_err();
    assert_eq!(
        error.downcast_ref::<std::io::Error>().unwrap().kind(),
        std::io::ErrorKind::PermissionDenied
    );
}

#[tokio::test]
async fn lifecycle_tunnel_success_fast_cleanup_crossing_candidate_clock_defers_without_failure() {
    let source = FakeSource::new();
    let mut state = bounded(
        source.clone(),
        Duration::from_secs(1),
        Duration::from_secs(5),
    );
    let mut recover_input = state.prepare_cutover::<2>().unwrap();
    state.inner.get_mut().unwrap().connect_timeout = WRITE_BUDGET;
    seed(&state, A).await;
    let before = {
        let failed = state.inner.failed_attempts.read().await;
        let row = &failed[A];
        (
            row.last_attempt,
            row.fail_count,
            row.slot
                .as_ref()
                .map(|slot| &**slot as *const failure_slot::Slot),
            row.version.as_ptr(),
        )
    };
    *state.inner.success_after_remove.lock().unwrap() = Some(Box::new(|| {
        std::thread::sleep(Duration::from_millis(100));
    }));
    let ticket = state.inner.lifecycle.ticket(A).await.unwrap();
    let outcome = state
        .refresh_connect_with_total(A, Route::Usbmux, &ticket, Some(1), None)
        .await;
    let restored = {
        let failed = state.inner.failed_attempts.read().await;
        failed.get(A).is_some_and(|row| {
            (
                row.last_attempt,
                row.fail_count,
                row.slot
                    .as_ref()
                    .map(|slot| &**slot as *const failure_slot::Slot),
                row.version.as_ptr(),
            ) == before
        })
    };
    let published = state.get_record(A).await.unwrap().is_some();
    let pending = match outcome {
        ScanAttemptOutcome::Success(success, ticket) => Some(SuccessIntent {
            _key_storage: None,
            _input: None,
            udid: A.into(),
            ticket,
            success,
        }),
        _ => None,
    };
    let had_intent = pending.is_some();
    let result = state
        .flush_scan_intents_with_total(pending.into_iter().map(ScanIntent::Success).collect(), None)
        .await;
    let cleaned = state.inner.failed_attempts.read().await.is_empty();
    let slots = state.inner.failure_slots.available_permits();
    recover(&state, &source, &mut recover_input).await;
    drop(recover_input);

    assert!(
        restored && published && had_intent,
        "published success became a connection failure"
    );
    result.unwrap();
    assert!(cleaned);
    assert_eq!(slots, MAX_FAILURE_RECORDS);
    assert_eq!(source.count(A), 1);
}
