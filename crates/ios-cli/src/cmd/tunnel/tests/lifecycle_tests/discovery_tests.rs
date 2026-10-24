use super::*;

#[tokio::test]
async fn lifecycle_tunnel_discovery_stalled_usb_returns_native_total_elapsed_and_drops_probe() {
    let source = FakeSource::new();
    *source.usb_gate.lock().unwrap() = Some(Arc::new(Semaphore::new(0)));
    let state = state(source.clone(), Duration::from_millis(80));
    let observed = tokio::time::timeout(LOCAL_BUDGET, state.refresh_once()).await;
    assert_eq!(source.usb_active.load(Ordering::SeqCst), 0);
    assert_eq!(source.usb_exited.load(Ordering::SeqCst), 1);
    let error = observed
        .expect("product discovery deadline never completed")
        .unwrap_err();
    assert!(error.is::<tokio::time::error::Elapsed>());
    assert_eq!(source.usb_calls.load(Ordering::SeqCst), 1);
    assert_eq!(source.paired_calls.load(Ordering::SeqCst), 0);
    assert!(!state.is_ready().await);
}

#[tokio::test]
async fn lifecycle_tunnel_discovery_scan_queue_counts_without_polling_device_source() {
    let source = FakeSource::new();
    let state = state(source.clone(), Duration::from_millis(60));
    let serial = state.inner.scan_lock.lock().await;
    let observed = tokio::time::timeout(LOCAL_BUDGET, state.refresh_once()).await;
    drop(serial);
    assert_eq!(source.usb_calls.load(Ordering::SeqCst), 0);
    assert_eq!(source.paired_calls.load(Ordering::SeqCst), 0);
    assert!(observed
        .expect("queue wait exceeded product setup deadline")
        .unwrap_err()
        .is::<tokio::time::error::Elapsed>());
    assert!(!state.is_ready().await);
}

#[tokio::test]
async fn lifecycle_tunnel_discovery_usb_and_paired_wait_share_absolute_budget() {
    let source = FakeSource::new();
    let usb_gate = Arc::new(Semaphore::new(0));
    let paired_gate = Arc::new(Semaphore::new(0));
    *source.usb_gate.lock().unwrap() = Some(usb_gate.clone());
    *source.paired_gate.lock().unwrap() = Some(paired_gate);
    let state = state(source.clone(), Duration::from_millis(80));
    let mut scan = Box::pin(state.refresh_once());
    assert!(futures_util::poll!(&mut scan).is_pending());
    tokio::time::sleep(Duration::from_millis(65)).await;
    usb_gate.add_permits(1);
    let observed = tokio::time::timeout(Duration::from_millis(40), scan.as_mut()).await;
    drop(scan);
    assert_eq!(source.usb_active.load(Ordering::SeqCst), 0);
    assert_eq!(source.paired_active.load(Ordering::SeqCst), 0);
    assert!(observed
        .expect("paired phase reset the original setup budget")
        .unwrap_err()
        .is::<tokio::time::error::Elapsed>());
    assert_eq!(source.usb_calls.load(Ordering::SeqCst), 1);
    assert_eq!(source.paired_calls.load(Ordering::SeqCst), 1);
    assert_eq!(source.paired_exited.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn lifecycle_tunnel_discovery_stop_cancels_old_usb_probe_before_reply() {
    let source = FakeSource::new();
    *source.usb_gate.lock().unwrap() = Some(Arc::new(Semaphore::new(0)));
    let state = state(source.clone(), TUNNEL_CONNECT_TIMEOUT);
    let mut stop_input_0 = state.prepare_cutover::<3>().unwrap();

    let mut scan = Box::pin(state.refresh_once());
    assert!(futures_util::poll!(&mut scan).is_pending());
    assert_eq!(source.usb_active.load(Ordering::SeqCst), 1);
    assert!(!state.remove_tunnel(A, &mut stop_input_0).await);
    drop(stop_input_0);
    let observed = tokio::time::timeout(Duration::from_millis(50), scan.as_mut()).await;
    drop(scan);
    assert_eq!(source.usb_active.load(Ordering::SeqCst), 0);
    observed
        .expect("old discovery probe kept waiting after stop")
        .unwrap();
    assert_eq!(source.usb_exited.load(Ordering::SeqCst), 1);
    *source.usb_gate.lock().unwrap() = None;
    state.refresh_once().await.unwrap();
    assert!(state.is_ready().await);
}

#[tokio::test]
async fn lifecycle_tunnel_discovery_clear_cancels_old_paired_probe_and_retains_new_generation() {
    let source = FakeSource::new();
    *source.paired_gate.lock().unwrap() = Some(Arc::new(Semaphore::new(0)));
    let state = state(source.clone(), TUNNEL_CONNECT_TIMEOUT);
    let mut control_input_0 = state.prepare_cutover::<2>().unwrap();

    let mut scan = Box::pin(state.refresh_once());
    assert!(futures_util::poll!(&mut scan).is_pending());
    assert_eq!(source.paired_active.load(Ordering::SeqCst), 1);
    state.clear(&mut control_input_0).await;
    drop(control_input_0);
    let observed = tokio::time::timeout(Duration::from_millis(50), scan.as_mut()).await;
    drop(scan);
    assert_eq!(source.paired_active.load(Ordering::SeqCst), 0);
    observed
        .expect("old paired probe kept waiting after clear")
        .unwrap();
    assert_eq!(source.paired_exited.load(Ordering::SeqCst), 1);
    *source.paired_gate.lock().unwrap() = None;
    state.refresh_once().await.unwrap();
    assert!(state.is_ready().await);
}

#[tokio::test]
async fn lifecycle_tunnel_discovery_zero_never_polls_source_or_marks_ready() {
    let source = FakeSource::new();
    let state = state(source.clone(), Duration::ZERO);
    let result = state.refresh_once().await;
    assert_eq!(source.usb_calls.load(Ordering::SeqCst), 0);
    assert_eq!(source.paired_calls.load(Ordering::SeqCst), 0);
    assert!(result.unwrap_err().is::<tokio::time::error::Elapsed>());
    assert!(!state.is_ready().await);
}

#[tokio::test]
async fn lifecycle_tunnel_discovery_overflow_rejects_before_source_or_ready() {
    let source = FakeSource::new();
    let state = state(source.clone(), Duration::MAX);
    let result = state.refresh_once().await;
    assert_eq!(source.usb_calls.load(Ordering::SeqCst), 0);
    assert_eq!(source.paired_calls.load(Ordering::SeqCst), 0);
    let error = result.unwrap_err();
    let rendered = crate::output::operation_failure_message(
        crate::output::ErrorFormat::Json,
        "tunnel-scan",
        &error,
    )
    .unwrap();
    let body: serde_json::Value = serde_json::from_str(&rendered).unwrap();
    assert_eq!(body["error"]["class"], "invalid_input");
    assert!(!state.is_ready().await);
}

#[tokio::test]
async fn lifecycle_tunnel_discovery_stop_cancels_queued_scan_without_source_poll() {
    let source = FakeSource::new();
    let state = state(source.clone(), TUNNEL_CONNECT_TIMEOUT);
    let mut stop_input_0 = state.prepare_cutover::<3>().unwrap();

    let serial = state.inner.scan_lock.lock().await;
    let mut scan = Box::pin(state.refresh_once());
    assert!(futures_util::poll!(&mut scan).is_pending());
    assert!(!state.remove_tunnel(A, &mut stop_input_0).await);
    drop(stop_input_0);
    let observed = tokio::time::timeout(Duration::from_millis(50), scan.as_mut()).await;
    drop(scan);
    drop(serial);
    observed
        .expect("queued old scan ignored stop epoch")
        .unwrap();
    assert_eq!(source.usb_calls.load(Ordering::SeqCst), 0);
    assert_eq!(source.paired_calls.load(Ordering::SeqCst), 0);
    assert!(!state.is_ready().await);
}

#[tokio::test]
async fn lifecycle_tunnel_discovery_budget_ends_before_candidate_connection_phase() {
    let source = FakeSource::new();
    source.usb.lock().unwrap().push(A.into());
    source.script(A, [Behavior::Pending]);
    let state = state(source.clone(), Duration::from_millis(60));
    state.refresh_once().await.unwrap();
    assert!(state.is_ready().await);
    assert_eq!(source.count(A), 1);
    assert_eq!(source.active.load(Ordering::SeqCst), 0);
    assert_eq!(state.inner.failed_attempts.read().await[A].fail_count, 1);
    assert!(state.list_records().await.unwrap().as_slice().is_empty());
    assert_eq!(
        state.inner.lifecycle.connects.available_permits(),
        lifecycle::MAX_CONNECTS
    );
}
