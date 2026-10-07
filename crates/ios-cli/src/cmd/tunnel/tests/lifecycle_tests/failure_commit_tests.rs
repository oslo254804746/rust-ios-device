use super::*;
use futures_util::FutureExt;

fn identity(id: u32) -> AttemptIdentity {
    AttemptIdentity {
        route: Route::Usbmux,
        native_id: Some(id),
    }
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
