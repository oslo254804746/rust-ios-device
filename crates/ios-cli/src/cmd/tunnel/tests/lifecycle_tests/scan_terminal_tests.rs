use super::*;

fn assert_no_source(source: &FakeSource) {
    assert_eq!(source.usb_calls.load(Ordering::SeqCst), 0);
    assert_eq!(source.paired_calls.load(Ordering::SeqCst), 0);
    assert_eq!(source.usb_active.load(Ordering::SeqCst), 0);
    assert_eq!(source.paired_active.load(Ordering::SeqCst), 0);
    assert_eq!(source.active.load(Ordering::SeqCst), 0);
    assert!(source.calls.lock().unwrap().is_empty());
}

fn assert_scan_released(state: &TunnelAgentState) {
    assert!(state.inner.scan_lock.try_lock().is_ok());
    assert_eq!(
        state.inner.lifecycle.connects.available_permits(),
        lifecycle::MAX_CONNECTS
    );
    assert_eq!(
        state.inner.failure_slots.available_permits(),
        MAX_FAILURE_RECORDS
    );
}

fn assert_cancelled(error: &anyhow::Error) {
    assert!(error.is::<ios_core::OperationCancelled>());
    assert_eq!(
        ios_core::classify_error(error.as_ref()),
        ios_core::ErrorClass::Cancelled
    );
}

#[tokio::test]
async fn lifecycle_tunnel_shutdown_fences_scan_close_before_capture_never_polls_source() {
    let source = FakeSource::new();
    let state = state(source.clone(), TUNNEL_CONNECT_TIMEOUT);
    let captures = Arc::new(AtomicUsize::new(0));
    let observed = captures.clone();
    state
        .inner
        .lifecycle
        .on_next_scan_capture(move |lifecycle| {
            observed.fetch_add(1, Ordering::SeqCst);
            lifecycle.close();
        });
    // The callback completes close after the real open check and deadline,
    // before the real watch subscription. No scheduler race is required.
    let outcome = tokio::time::timeout(LOCAL_BUDGET, state.refresh_once())
        .await
        .expect("scan capture close failed to return");
    assert_eq!(captures.load(Ordering::SeqCst), 1);
    assert_no_source(&source);
    assert_scan_released(&state);
    assert_cancelled(&outcome.expect_err("closed capture admitted a scan"));
    assert!(!state.is_ready().await);
    assert!(state.inner.tunnels.read().await.is_empty());
    assert!(state.inner.failed_attempts.read().await.is_empty());
    assert_eq!(state.inner.lifecycle.coordinator_count().await, 0);
    let error = tokio::time::timeout(LOCAL_BUDGET, state.refresh_once())
        .await
        .unwrap()
        .unwrap_err();
    assert_cancelled(&error);
    assert_eq!(captures.load(Ordering::SeqCst), 1);
    assert_no_source(&source);
}

#[tokio::test]
async fn lifecycle_tunnel_shutdown_fences_scan_clear_before_capture_remains_reusable() {
    let source = FakeSource::new();
    let state = state(source.clone(), TUNNEL_CONNECT_TIMEOUT);
    let captures = Arc::new(AtomicUsize::new(0));
    let observed = captures.clone();
    state
        .inner
        .lifecycle
        .on_next_scan_capture(move |lifecycle| {
            observed.fetch_add(1, Ordering::SeqCst);
            lifecycle.clear();
        });
    for expected in 1..=2 {
        tokio::time::timeout(LOCAL_BUDGET, state.refresh_once())
            .await
            .expect("fresh scan after clear did not return")
            .unwrap();
        assert_eq!(captures.load(Ordering::SeqCst), 1);
        assert_eq!(source.usb_calls.load(Ordering::SeqCst), expected);
        assert_eq!(source.paired_calls.load(Ordering::SeqCst), expected);
        assert_eq!(source.usb_active.load(Ordering::SeqCst), 0);
        assert_eq!(source.paired_active.load(Ordering::SeqCst), 0);
        assert!(source.calls.lock().unwrap().is_empty());
        assert!(state.is_ready().await);
        assert_scan_released(&state);
    }
}

#[tokio::test]
async fn lifecycle_tunnel_shutdown_fences_scan_closed_and_overflow_keep_admission_priority() {
    for terminal in [false, true] {
        let source = FakeSource::new();
        let state = state(source.clone(), Duration::MAX);
        let captures = Arc::new(AtomicUsize::new(0));
        let observed = captures.clone();
        state.inner.lifecycle.on_next_scan_capture(move |_| {
            observed.fetch_add(1, Ordering::SeqCst);
        });
        if terminal {
            state.notify_shutdown();
        }
        let error = tokio::time::timeout(LOCAL_BUDGET, state.refresh_once())
            .await
            .expect("closed/overflow scan admission did not return")
            .unwrap_err();
        if terminal {
            assert_cancelled(&error);
        } else {
            assert_eq!(
                ios_core::classify_error(error.as_ref()),
                ios_core::ErrorClass::InvalidInput
            );
        }
        assert_eq!(captures.load(Ordering::SeqCst), 0);
        assert_no_source(&source);
        assert_scan_released(&state);
        assert!(!state.is_ready().await);
        assert!(state.inner.tunnels.read().await.is_empty());
        assert!(state.inner.failed_attempts.read().await.is_empty());
        assert_eq!(state.inner.lifecycle.coordinator_count().await, 0);
    }
}

#[tokio::test]
async fn lifecycle_tunnel_shutdown_fences_scan_close_after_capture_never_polls_drive_work() {
    let source = FakeSource::new();
    let state = state(source.clone(), TUNNEL_CONNECT_TIMEOUT);
    let mut control = state
        .inner
        .lifecycle
        .scan_control(TUNNEL_CONNECT_TIMEOUT)
        .unwrap();
    state.inner.lifecycle.close();
    let polls = AtomicUsize::new(0);
    let outcome = tokio::time::timeout(
        LOCAL_BUDGET,
        control.drive(async {
            polls.fetch_add(1, Ordering::SeqCst);
            source.usb_devices().await
        }),
    )
    .await
    .expect("captured scan ignored close")
    .unwrap();
    assert!(outcome.is_none());
    assert_eq!(polls.load(Ordering::SeqCst), 0);
    assert_no_source(&source);
    assert_scan_released(&state);
    assert!(!state.is_ready().await);
}
