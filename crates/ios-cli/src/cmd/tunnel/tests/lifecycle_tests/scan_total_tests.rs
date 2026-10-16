//! One scan budget spans queued scans, candidates, reconciliation and readiness.
use super::*;

const TOTAL: Duration = Duration::from_millis(60);

fn bounded(source: Arc<FakeSource>, total: Duration) -> TunnelAgentState {
    let mut manager = state(source, Duration::from_secs(5));
    Arc::get_mut(&mut manager.inner).unwrap().scan_timeout = total;
    manager
}

fn elapsed(error: &anyhow::Error) {
    assert!(error.is::<tokio::time::error::Elapsed>());
    assert_eq!(
        ios_core::classify_error(error.as_ref()),
        ios_core::ErrorClass::Timeout
    );
}

#[tokio::test]
async fn lifecycle_tunnel_whole_scan_zero_and_overflow_do_not_poll_sources() {
    for total in [Duration::ZERO, Duration::MAX] {
        let source = FakeSource::new();
        let state = bounded(source.clone(), total);
        let error = state.refresh_once().await.unwrap_err();
        if total.is_zero() {
            elapsed(&error);
        } else {
            assert_eq!(
                ios_core::classify_error(error.as_ref()),
                ios_core::ErrorClass::InvalidInput
            );
        }
        assert_eq!(source.usb_calls.load(Ordering::SeqCst), 0);
        assert_eq!(source.paired_calls.load(Ordering::SeqCst), 0);
        assert!(state.inner.scan_outcomes.history().is_none());
        assert!(state.inner.scan_lock.try_lock().is_ok());
    }
}

#[tokio::test]
async fn lifecycle_tunnel_whole_scan_queued_lock_consumes_no_error_slot() {
    let source = FakeSource::new();
    let state = bounded(source.clone(), TOTAL);
    let lock = state.inner.scan_lock.lock().await;
    let error = tokio::time::timeout(LOCAL_BUDGET, state.refresh_once())
        .await
        .unwrap()
        .unwrap_err();
    elapsed(&error);
    assert_eq!(source.usb_calls.load(Ordering::SeqCst), 0);
    assert!(state.inner.scan_outcomes.history().is_none());
    drop(lock);
    state.refresh_once().await.unwrap();
    assert!(state.is_ready().await);
}

#[tokio::test]
async fn lifecycle_tunnel_whole_scan_reconciliation_wait_expires_or_cancels_before_release() {
    for cancel in [false, true] {
        let source = FakeSource::new();
        let state = bounded(
            source.clone(),
            if cancel {
                Duration::from_secs(5)
            } else {
                TOTAL
            },
        );
        let table = state.inner.failed_attempts.write().await;
        let mut scan = Box::pin(state.refresh_once());
        assert!(futures_util::poll!(&mut scan).is_pending());
        assert_eq!(source.paired_calls.load(Ordering::SeqCst), 1);
        if cancel {
            state.inner.lifecycle.clear();
        }
        let result = tokio::time::timeout(LOCAL_BUDGET, scan).await.unwrap();
        if cancel {
            result.unwrap();
        } else {
            elapsed(&result.unwrap_err());
        }
        assert!(table.is_empty());
        assert!(!state.is_ready().await);
        assert!(state.inner.scan_lock.try_lock().is_ok());
        drop(table);
    }
}

#[tokio::test]
async fn lifecycle_tunnel_whole_scan_readiness_wait_expires_without_late_ready() {
    let source = FakeSource::new();
    let state = bounded(source, TOTAL);
    let ready = state.inner.ready.write().await;
    let mut scan = Box::pin(state.refresh_once());
    assert!(futures_util::poll!(&mut scan).is_pending());
    let error = tokio::time::timeout(LOCAL_BUDGET, scan)
        .await
        .unwrap()
        .unwrap_err();
    elapsed(&error);
    assert!(!*ready);
    drop(ready);
    assert!(!state.is_ready().await);
    assert!(state.inner.scan_lock.try_lock().is_ok());
}

#[tokio::test]
async fn lifecycle_tunnel_whole_scan_cancels_all_candidates_without_failure_backoff() {
    for cancel in [false, true] {
        let source = FakeSource::new();
        for device in [A, B, "synthetic-c", "synthetic-d", "synthetic-e"] {
            source.usb.lock().unwrap().push(device.into());
            source.script(device, [Behavior::Pending]);
        }
        let state = bounded(
            source.clone(),
            if cancel {
                Duration::from_secs(5)
            } else {
                TOTAL
            },
        );
        let mut scan = Box::pin(state.refresh_once());
        assert!(futures_util::poll!(&mut scan).is_pending());
        assert_eq!(
            source.active.load(Ordering::SeqCst),
            lifecycle::MAX_CONNECTS
        );
        if cancel {
            state.inner.lifecycle.clear();
        }
        let result = tokio::time::timeout(LOCAL_BUDGET, scan).await.unwrap();
        if cancel {
            result.unwrap();
        } else {
            elapsed(&result.unwrap_err());
        }
        assert_eq!(source.active.load(Ordering::SeqCst), 0);
        assert_eq!(source.count("synthetic-e"), 0);
        assert_eq!(
            state.inner.lifecycle.connects.available_permits(),
            lifecycle::MAX_CONNECTS
        );
        assert_eq!(
            state.inner.failure_slots.available_permits(),
            MAX_FAILURE_RECORDS
        );
        assert!(state.inner.failed_attempts.read().await.is_empty());
        assert!(!state.is_ready().await);
    }
}

#[tokio::test]
async fn lifecycle_tunnel_whole_scan_stop_a_does_not_cancel_b_or_readiness() {
    let source = FakeSource::new();
    source.usb.lock().unwrap().extend([A.into(), B.into()]);
    let gate = Arc::new(Semaphore::new(0));
    source.script(A, [Behavior::Pending]);
    source.script(B, [Behavior::Held(gate.clone())]);
    let state = bounded(source.clone(), Duration::from_secs(5));
    let mut scan = Box::pin(state.refresh_once());
    assert!(futures_util::poll!(&mut scan).is_pending());
    assert_eq!(source.active.load(Ordering::SeqCst), 2);
    state.remove_tunnel(A).await;
    gate.add_permits(1);
    tokio::time::timeout(LOCAL_BUDGET, scan)
        .await
        .unwrap()
        .unwrap();
    assert!(state.get_record(A).await.is_none());
    assert!(state.get_record(B).await.is_some());
    assert!(state.is_ready().await);
    assert!(state.inner.failed_attempts.read().await.is_empty());
    state.clear().await;
}

#[tokio::test]
async fn lifecycle_tunnel_whole_scan_publication_crossing_rolls_back_without_backoff() {
    let source = FakeSource::new();
    source.usb.lock().unwrap().push(A.into());
    let state = bounded(source.clone(), TOTAL);
    *state.inner.publication_after_insert.lock().unwrap() = Some(Box::new(|| {
        std::thread::sleep(Duration::from_millis(100));
    }));
    let error = state.refresh_once().await.unwrap_err();
    elapsed(&error);
    assert!(state.inner.tunnels.read().await.is_empty());
    assert!(state.inner.failed_attempts.read().await.is_empty());
    assert!(!state.is_ready().await);
    assert_eq!(source.drops.load(Ordering::SeqCst), 1);
    assert_eq!(
        state.inner.failure_slots.available_permits(),
        MAX_FAILURE_RECORDS
    );
}
