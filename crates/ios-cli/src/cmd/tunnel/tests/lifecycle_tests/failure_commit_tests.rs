use super::*;
use futures_util::FutureExt;

fn identity(id: u32) -> AttemptIdentity {
    AttemptIdentity {
        route: Route::Usbmux,
        native_id: Some(id),
    }
}

#[tokio::test]
async fn lifecycle_tunnel_clear_preserves_failure_committed_in_replacement_epoch() {
    let source = FakeSource::new();
    let state = state(source.clone(), TUNNEL_CONNECT_TIMEOUT);
    let old = state.inner.lifecycle.ticket(A).await.unwrap();
    let slot = state.reserve_failure_slot(A).await.unwrap();
    state
        .record_failed_attempt(A, identity(71), slot, &old)
        .await;
    let old_version = Arc::downgrade(&state.inner.failed_attempts.read().await[A].version);
    let expected = Arc::new(StdMutex::new(None));
    let captured = Arc::clone(&expected);
    let fresh = state.clone();
    state.inner.lifecycle.on_next_clear_cleanup(move |_| {
        let ticket = fresh
            .inner
            .lifecycle
            .ticket(A)
            .now_or_never()
            .unwrap()
            .unwrap();
        let slot = fresh
            .reserve_failure_slot(A)
            .now_or_never()
            .unwrap()
            .unwrap();
        fresh
            .record_failed_attempt(A, identity(71), slot, &ticket)
            .now_or_never()
            .expect("replacement failure blocked by old cleanup");
        let failed = fresh.inner.failed_attempts.try_read().unwrap();
        let row = &failed[A];
        *captured.lock().unwrap() = Some((
            Arc::clone(&row.version),
            row.slot.as_ref().map(|slot| Arc::as_ptr(slot) as usize),
        ));
        drop(failed);
        // A device-only stop must not make old global cleanup erase this row.
        fresh.inner.lifecycle.stop(A).now_or_never().unwrap();
    });
    state.clear().await;
    {
        let failed = state.inner.failed_attempts.read().await;
        let row = failed
            .get(A)
            .expect("old clear erased replacement-epoch failure");
        let expected = expected.lock().unwrap();
        let (version, slot) = expected.as_ref().unwrap();
        assert!(Arc::ptr_eq(&row.version, version));
        assert_eq!(
            row.slot.as_ref().map(|slot| Arc::as_ptr(slot) as usize),
            *slot
        );
        assert_eq!(row.fail_count, 1);
        assert!(row.identity == identity(71));
    }
    assert!(old_version.upgrade().is_none());
    assert_eq!(
        state.inner.failure_slots.available_permits(),
        MAX_FAILURE_RECORDS - 1
    );
    assert!(source.calls.lock().unwrap().is_empty());
    state.clear().await;
    assert_eq!(
        state.inner.failure_slots.available_permits(),
        MAX_FAILURE_RECORDS
    );
}

struct ClearCause {
    state: TunnelAgentState,
    drops: Arc<AtomicUsize>,
}
impl std::fmt::Debug for ClearCause {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("ClearCauseFixture")
    }
}
impl std::fmt::Display for ClearCause {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("synthetic original clear cause")
    }
}
impl std::error::Error for ClearCause {}
impl Drop for ClearCause {
    fn drop(&mut self) {
        assert!(
            self.state.inner.failed_attempts.try_write().is_ok(),
            "cause dropped under failure table guard"
        );
        assert!(
            self.state.inner.tunnels.try_write().is_ok(),
            "cause dropped under publication guard"
        );
        self.drops.fetch_add(1, Ordering::SeqCst);
    }
}

fn retired_cause(state: &TunnelAgentState, udid: &str, id: u32, drops: Arc<AtomicUsize>) -> usize {
    let ticket = state
        .inner
        .lifecycle
        .ticket(udid)
        .now_or_never()
        .unwrap()
        .unwrap();
    let slot = state
        .reserve_failure_slot(udid)
        .now_or_never()
        .unwrap()
        .unwrap();
    let attempt = ticket
        .commit(|| state.inner.retry.start(udid, identity(id), &ticket, slot))
        .unwrap();
    // A Source already polled before stop still returns its original error into
    // the preallocated pending node. This creates actual retired cause storage.
    state.inner.lifecycle.stop(udid).now_or_never().unwrap();
    let original = anyhow::Error::new(std::io::Error::new(
        std::io::ErrorKind::ConnectionRefused,
        ClearCause {
            state: state.clone(),
            drops,
        },
    ));
    let pointer = original.downcast_ref::<std::io::Error>().unwrap() as *const _ as usize;
    let result: Result<TunnelRecord> = Err(attempt.source_failed(original));
    attempt.finish(&result);
    drop(result);
    pointer
}

#[tokio::test]
async fn lifecycle_tunnel_clear_preserves_retired_cause_created_in_replacement_epoch() {
    let source = FakeSource::new();
    let state = state(source.clone(), TUNNEL_CONNECT_TIMEOUT);
    let old_drops = Arc::new(AtomicUsize::new(0));
    retired_cause(&state, A, 71, Arc::clone(&old_drops));
    assert_eq!(state.inner.retry.receipt_count(), 1);
    let new_drops = Arc::new(AtomicUsize::new(0));
    let observed_drops = Arc::clone(&new_drops);
    let expected = Arc::new(StdMutex::new(None));
    let captured = Arc::clone(&expected);
    let fresh = state.clone();
    state.inner.lifecycle.on_next_clear_cleanup(move |_| {
        *captured.lock().unwrap() = Some(retired_cause(&fresh, B, 72, observed_drops));
    });
    state.clear().await;
    assert_eq!(old_drops.load(Ordering::SeqCst), 1);
    assert_eq!(
        new_drops.load(Ordering::SeqCst),
        0,
        "old clear destroyed a newer retired cause"
    );
    let receipts = state.inner.retry.retired_receipts();
    assert_eq!(receipts.len(), 1);
    assert_eq!(
        receipts[0]
            .primary()
            .downcast_ref::<std::io::Error>()
            .unwrap() as *const _ as usize,
        expected.lock().unwrap().unwrap()
    );
    drop(receipts);
    assert!(source.calls.lock().unwrap().is_empty());
    state.clear().await;
    assert_eq!(new_drops.load(Ordering::SeqCst), 1);
    assert_eq!(
        state.inner.failure_slots.available_permits(),
        MAX_FAILURE_RECORDS
    );
}

#[tokio::test]
async fn lifecycle_tunnel_clear_cancels_old_epoch_before_waiting_for_failure_table() {
    let source = FakeSource::new();
    let state = state(source.clone(), TUNNEL_CONNECT_TIMEOUT);
    let ticket = state.inner.lifecycle.ticket(A).await.unwrap();
    let failed = state.inner.failed_attempts.write().await;
    let mut clear = Box::pin(state.clear());
    assert!(futures_util::poll!(&mut clear).is_pending());
    assert!(
        !ticket.is_current(),
        "clear delayed old-epoch cancellation behind failure writer"
    );
    // Acquiring the failure writer must not hold the publication table while
    // waiting: independent list/stop remains runnable within the local budget.
    tokio::time::timeout(LOCAL_BUDGET, state.list_records())
        .await
        .unwrap()
        .unwrap();
    drop(failed);
    tokio::time::timeout(LOCAL_BUDGET, &mut clear)
        .await
        .unwrap();
    assert!(!ticket.is_current());
    assert!(source.calls.lock().unwrap().is_empty());
}

#[tokio::test]
async fn lifecycle_tunnel_clear_preserves_pending_source_node_until_original_arrives() {
    let source = FakeSource::new();
    let state = state(source.clone(), TUNNEL_CONNECT_TIMEOUT);
    let ticket = state.inner.lifecycle.ticket(A).await.unwrap();
    let slot = state.reserve_failure_slot(A).await.unwrap();
    let attempt = ticket
        .commit(|| state.inner.retry.start(A, identity(71), &ticket, slot))
        .unwrap();
    state.clear().await;
    assert_eq!(state.inner.retry.receipt_count(), 0);
    let drops = Arc::new(AtomicUsize::new(0));
    let result: Result<TunnelRecord> = Err(attempt.source_failed(anyhow::Error::new(
        std::io::Error::new(
            std::io::ErrorKind::ConnectionRefused,
            ClearCause {
                state: state.clone(),
                drops: Arc::clone(&drops),
            },
        ),
    )));
    attempt.finish(&result);
    drop(result);
    assert_eq!(drops.load(Ordering::SeqCst), 0);
    assert_eq!(state.inner.retry.receipt_count(), 1);
    state.clear().await;
    assert_eq!(drops.load(Ordering::SeqCst), 1);
    assert_eq!(
        state.inner.failure_slots.available_permits(),
        MAX_FAILURE_RECORDS
    );
    assert!(source.calls.lock().unwrap().is_empty());
}

#[tokio::test]
async fn lifecycle_tunnel_clear_cancelled_while_publication_locked_keeps_old_epoch_and_failures() {
    let source = FakeSource::new();
    let state = state(source.clone(), TUNNEL_CONNECT_TIMEOUT);
    let ticket = state.inner.lifecycle.ticket(A).await.unwrap();
    let slot = state.reserve_failure_slot(A).await.unwrap();
    state
        .record_failed_attempt(A, identity(71), slot, &ticket)
        .await;
    let version = Arc::clone(&state.inner.failed_attempts.read().await[A].version);
    let tunnels = state.inner.tunnels.write().await;
    let mut clear = Box::pin(state.clear());
    assert!(futures_util::poll!(&mut clear).is_pending());
    assert!(ticket.is_current());
    drop(clear);
    drop(tunnels);
    let failed = state.inner.failed_attempts.try_write().unwrap();
    assert!(Arc::ptr_eq(&failed[A].version, &version));
    assert_eq!(failed[A].fail_count, 1);
    drop(failed);
    assert!(source.calls.lock().unwrap().is_empty());
    state.clear().await;
    assert_eq!(
        state.inner.failure_slots.available_permits(),
        MAX_FAILURE_RECORDS
    );
}

#[tokio::test]
async fn lifecycle_tunnel_later_clear_collects_failure_left_by_cancelled_earlier_clear() {
    let source = FakeSource::new();
    let state = state(source.clone(), TUNNEL_CONNECT_TIMEOUT);
    let ticket = state.inner.lifecycle.ticket(A).await.unwrap();
    let slot = state.reserve_failure_slot(A).await.unwrap();
    state
        .record_failed_attempt(A, identity(71), slot, &ticket)
        .await;
    let failed = state.inner.failed_attempts.write().await;
    let mut clear = Box::pin(state.clear());
    assert!(futures_util::poll!(&mut clear).is_pending());
    assert!(!ticket.is_current());
    drop(clear);
    drop(failed);
    assert!(state.inner.failed_attempts.read().await.contains_key(A));
    state.clear().await;
    assert!(state.inner.failed_attempts.read().await.is_empty());
    assert_eq!(
        state.inner.failure_slots.available_permits(),
        MAX_FAILURE_RECORDS
    );
    assert!(source.calls.lock().unwrap().is_empty());
}

#[tokio::test]
async fn lifecycle_tunnel_shutdown_fences_failure_close_after_check_releases_reservation() {
    let source = FakeSource::new();
    let state = state(source.clone(), TUNNEL_CONNECT_TIMEOUT);
    let ticket = state.inner.lifecycle.ticket(A).await.unwrap();
    let slot = state.reserve_failure_slot(A).await.unwrap();
    assert_eq!(
        state.inner.failure_slots.available_permits(),
        MAX_FAILURE_RECORDS - 1
    );
    let called = Arc::new(AtomicUsize::new(0));
    let observed = called.clone();
    state
        .inner
        .lifecycle
        .on_next_failure_record(move |lifecycle| {
            observed.fetch_add(1, Ordering::SeqCst);
            lifecycle.close();
        });
    tokio::time::timeout(
        LOCAL_BUDGET,
        state.record_failed_attempt(A, identity(71), slot, &ticket),
    )
    .await
    .expect("failure recording did not finish after close");
    assert_eq!(called.load(Ordering::SeqCst), 1);
    assert!(state.inner.failed_attempts.read().await.is_empty());
    assert_eq!(
        state.inner.failure_slots.available_permits(),
        MAX_FAILURE_RECORDS
    );
    let error = state.inner.lifecycle.ticket(A).await.err().unwrap();
    assert!(error.is::<ios_core::OperationCancelled>());
    assert_eq!(
        ios_core::classify_error(error.as_ref()),
        ios_core::ErrorClass::Cancelled
    );
    assert!(source.calls.lock().unwrap().is_empty());
    state.clear().await;
    assert_eq!(
        state.inner.lifecycle.connects.available_permits(),
        lifecycle::MAX_CONNECTS
    );
}

#[tokio::test]
async fn lifecycle_tunnel_shutdown_fences_failure_clear_and_stop_preserve_old_row() {
    for clear in [false, true] {
        let source = FakeSource::new();
        let state = state(source.clone(), TUNNEL_CONNECT_TIMEOUT);
        let ticket = state.inner.lifecycle.ticket(A).await.unwrap();
        let slot = state.reserve_failure_slot(A).await.unwrap();
        state
            .record_failed_attempt(A, identity(71), slot, &ticket)
            .await;
        let before = {
            let failed = state.inner.failed_attempts.read().await;
            let old = failed.get(A).unwrap();
            (old.identity.clone(), old.fail_count, old.last_attempt)
        };
        let slot = state.reserve_failure_slot(A).await.unwrap();
        let called = Arc::new(AtomicUsize::new(0));
        let observed = called.clone();
        state
            .inner
            .lifecycle
            .on_next_failure_record(move |lifecycle| {
                observed.fetch_add(1, Ordering::SeqCst);
                if clear {
                    lifecycle.clear();
                } else {
                    // The real stop path has no contention in this fixture. Poll
                    // it once while the caller owns the required tunnel guard.
                    assert!(lifecycle.stop(A).now_or_never().is_some());
                }
            });
        let tunnels = state.inner.tunnels.write().await;
        tokio::time::timeout(
            LOCAL_BUDGET,
            state.record_failed_attempt(A, identity(72), slot, &ticket),
        )
        .await
        .expect("stale failure update did not return");
        drop(tunnels);
        let after = {
            let failed = state.inner.failed_attempts.read().await;
            let old = failed.get(A).unwrap();
            (old.identity.clone(), old.fail_count, old.last_attempt)
        };
        assert_eq!(called.load(Ordering::SeqCst), 1);
        assert!(
            after == before,
            "stale update changed an existing failure row"
        );
        assert_eq!(
            state.inner.failure_slots.available_permits(),
            MAX_FAILURE_RECORDS - 1
        );
        let fresh = state.inner.lifecycle.ticket(A).await.unwrap();
        let slot = state.reserve_failure_slot(A).await.unwrap();
        state
            .record_failed_attempt(A, identity(72), slot, &fresh)
            .await;
        {
            let failed = state.inner.failed_attempts.read().await;
            let current = failed.get(A).unwrap();
            assert!(current.identity == identity(72));
            assert_eq!(current.fail_count, 1);
        }
        assert_eq!(called.load(Ordering::SeqCst), 1);
        assert!(source.calls.lock().unwrap().is_empty());
        state.clear().await;
        assert!(state.inner.failed_attempts.read().await.is_empty());
        assert_eq!(
            state.inner.failure_slots.available_permits(),
            MAX_FAILURE_RECORDS
        );
        assert_eq!(
            state.inner.lifecycle.connects.available_permits(),
            lifecycle::MAX_CONNECTS
        );
    }
}
