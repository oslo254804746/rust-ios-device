use super::*;

fn assert_elapsed(error: &anyhow::Error) {
    assert!(error.is::<tokio::time::error::Elapsed>());
    assert_eq!(
        ios_core::classify_error(error.as_ref()),
        ios_core::ErrorClass::Timeout
    );
}

fn assert_no_source(source: &FakeSource) {
    assert!(source.calls.lock().unwrap().is_empty());
    assert_eq!(source.usb_calls.load(Ordering::SeqCst), 0);
    assert_eq!(source.paired_calls.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn lifecycle_tunnel_manual_deadline_coordinator_expires_before_lock_release() {
    let source = FakeSource::new();
    let state = state(source.clone(), Duration::from_millis(80));
    let held = state.inner.lifecycle.hold_coordinator().await;
    let observed = tokio::time::timeout(LOCAL_BUDGET, start(state.clone(), A)).await;
    assert_no_source(&source);
    drop(held);
    assert_elapsed(
        &observed
            .expect("coordinator wait ignored connection total")
            .unwrap_err(),
    );
    start(state.clone(), A).await.unwrap();
    state.clear().await;
    assert_eq!(source.drops.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn lifecycle_tunnel_manual_deadline_coordinator_and_connect_share_one_budget() {
    let source = FakeSource::new();
    source.script(A, [Behavior::Pending, Behavior::Success]);
    let state = state(source.clone(), Duration::from_millis(1000));
    let held = state.inner.lifecycle.hold_coordinator().await;
    let mut request = Box::pin(start(state.clone(), A));
    assert!(futures_util::poll!(&mut request).is_pending());
    assert_no_source(&source);
    tokio::time::sleep(Duration::from_millis(600)).await;
    drop(held);
    assert!(futures_util::poll!(&mut request).is_pending());
    assert_eq!(source.count(A), 1);
    let observed = tokio::time::timeout(Duration::from_millis(650), request.as_mut()).await;
    drop(request);
    assert_elapsed(
        &observed
            .expect("connection reset the coordinator budget")
            .unwrap_err(),
    );
    assert_eq!(source.active.load(Ordering::SeqCst), 0);
    assert_eq!(source.exited.load(Ordering::SeqCst), 1);
    assert_eq!(
        state.inner.lifecycle.connects.available_permits(),
        lifecycle::MAX_CONNECTS
    );
    assert!(state.get_record(A).await.is_none());
    start(state.clone(), A).await.unwrap();
    state.clear().await;
    assert_eq!(source.drops.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn lifecycle_tunnel_manual_deadline_zero_does_not_poll_any_transport() {
    for route in [
        None,
        Some(StartTunnelConnectionType::Usbmux),
        Some(StartTunnelConnectionType::Usb),
        Some(StartTunnelConnectionType::Wifi),
    ] {
        let source = FakeSource::new();
        let state = state(source.clone(), Duration::ZERO);
        let error = state.ensure_tunnel(A, None, route).await.unwrap_err();
        assert_elapsed(&error);
        assert_no_source(&source);
        assert!(state.list_records().await.is_empty());
        assert_eq!(
            state.inner.lifecycle.connects.available_permits(),
            lifecycle::MAX_CONNECTS
        );
    }
}

#[tokio::test]
async fn lifecycle_tunnel_manual_deadline_overflow_is_typed_input_before_source() {
    let source = FakeSource::new();
    let state = state(source.clone(), Duration::MAX);
    let error = start(state.clone(), A).await.unwrap_err();
    assert_eq!(
        ios_core::classify_error(error.as_ref()),
        ios_core::ErrorClass::InvalidInput
    );
    assert_no_source(&source);
    assert!(state.list_records().await.is_empty());
}

#[tokio::test]
async fn lifecycle_tunnel_manual_deadline_late_coordinator_release_does_not_connect() {
    let source = FakeSource::new();
    let state = state(source.clone(), Duration::from_millis(60));
    let held = state.inner.lifecycle.hold_coordinator().await;
    let mut request = Box::pin(start(state.clone(), A));
    assert!(futures_util::poll!(&mut request).is_pending());
    tokio::time::sleep(Duration::from_millis(100)).await;
    drop(held);
    let error = request.await.unwrap_err();
    assert_elapsed(&error);
    assert_no_source(&source);
    assert!(state.list_records().await.is_empty());
}

#[tokio::test]
async fn lifecycle_tunnel_manual_deadline_late_native_success_cannot_publish() {
    let source = FakeSource::new();
    let gate = Arc::new(Semaphore::new(0));
    source.script(A, [Behavior::Held(gate.clone()), Behavior::Success]);
    let state = state(source.clone(), Duration::from_millis(60));
    let mut request = Box::pin(start(state.clone(), A));
    assert!(futures_util::poll!(&mut request).is_pending());
    assert_eq!(source.active.load(Ordering::SeqCst), 1);
    tokio::time::sleep(Duration::from_millis(100)).await;
    gate.add_permits(1);
    let error = request.await.unwrap_err();
    assert_elapsed(&error);
    assert_eq!(source.usb_calls.load(Ordering::SeqCst), 0);
    assert_eq!(source.drops.load(Ordering::SeqCst), 0);
    assert_eq!(source.active.load(Ordering::SeqCst), 0);
    assert_eq!(source.exited.load(Ordering::SeqCst), 1);
    assert_eq!(
        state.inner.lifecycle.connects.available_permits(),
        lifecycle::MAX_CONNECTS
    );
    assert!(state.list_records().await.is_empty());
    start(state.clone(), A).await.unwrap();
    state.clear().await;
    assert_eq!(source.drops.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn lifecycle_tunnel_manual_deadline_coordinator_wait_still_observes_cancellation() {
    for terminal in [false, true] {
        let source = FakeSource::new();
        let state = state(source.clone(), TUNNEL_CONNECT_TIMEOUT);
        let held = state.inner.lifecycle.hold_coordinator().await;
        let mut request = Box::pin(start(state.clone(), A));
        assert!(futures_util::poll!(&mut request).is_pending());
        if terminal {
            state.notify_shutdown();
        } else {
            state.inner.lifecycle.clear();
        }
        let observed = tokio::time::timeout(LOCAL_BUDGET, request.as_mut()).await;
        drop(request);
        assert_no_source(&source);
        drop(held);
        let error = observed
            .expect("coordinator cancellation waited for lock release")
            .unwrap_err();
        assert!(error.is::<ios_core::OperationCancelled>());
        assert_eq!(
            ios_core::classify_error(error.as_ref()),
            ios_core::ErrorClass::Cancelled
        );
    }
}

#[tokio::test]
async fn lifecycle_tunnel_manual_deadline_caller_drop_releases_coordinator_wait() {
    let source = FakeSource::new();
    let state = state(source.clone(), TUNNEL_CONNECT_TIMEOUT);
    let held = state.inner.lifecycle.hold_coordinator().await;
    let mut request = Box::pin(start(state.clone(), A));
    assert!(futures_util::poll!(&mut request).is_pending());
    drop(request);
    assert_no_source(&source);
    drop(held);
    start(state.clone(), A).await.unwrap();
    state.clear().await;
    assert_eq!(source.drops.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn lifecycle_tunnel_manual_deadline_permit_wait_expires_without_source_contact() {
    for late_release in [false, true] {
        let source = FakeSource::new();
        let state = state(source.clone(), Duration::from_millis(60));
        let held = state
            .inner
            .lifecycle
            .connects
            .acquire_many(lifecycle::MAX_CONNECTS as u32)
            .await
            .unwrap();
        let mut request = Box::pin(start(state.clone(), A));
        assert!(futures_util::poll!(&mut request).is_pending());
        if late_release {
            tokio::time::sleep(Duration::from_millis(100)).await;
            drop(held);
            assert_elapsed(&request.await.unwrap_err());
        } else {
            let observed = tokio::time::timeout(LOCAL_BUDGET, request.as_mut()).await;
            drop(request);
            assert_no_source(&source);
            drop(held);
            assert_elapsed(
                &observed
                    .expect("permit wait ignored connection total")
                    .unwrap_err(),
            );
        }
        assert_no_source(&source);
        assert!(state.list_records().await.is_empty());
        assert_eq!(
            state.inner.lifecycle.connects.available_permits(),
            lifecycle::MAX_CONNECTS
        );
    }
}

#[tokio::test]
async fn lifecycle_tunnel_manual_deadline_paired_discovery_and_fallback_share_budget() {
    let source = FakeSource::new();
    let paired_gate = Arc::new(Semaphore::new(0));
    source.script(
        A,
        [
            Behavior::Failure(std::io::ErrorKind::BrokenPipe),
            Behavior::Pending,
        ],
    );
    source.paired.lock().unwrap().push(PairedMobdev2Device {
        udid: A.into(),
        host: "192.0.2.9".into(),
    });
    *source.paired_gate.lock().unwrap() = Some(paired_gate.clone());
    let state = state(source.clone(), Duration::from_millis(1000));
    let mut request = Box::pin(state.ensure_tunnel(A, None, None));
    assert!(futures_util::poll!(&mut request).is_pending());
    assert_eq!(source.count(A), 1);
    assert_eq!(source.paired_active.load(Ordering::SeqCst), 1);
    tokio::time::sleep(Duration::from_millis(600)).await;
    paired_gate.add_permits(1);
    assert!(futures_util::poll!(&mut request).is_pending());
    assert_eq!(source.count(A), 2);
    assert_eq!(
        source.calls.lock().unwrap().last().unwrap().1,
        Route::Mobdev2("192.0.2.9".into())
    );
    let observed = tokio::time::timeout(Duration::from_millis(650), request.as_mut()).await;
    drop(request);
    assert_elapsed(
        &observed
            .expect("fallback reset discovery budget")
            .unwrap_err(),
    );
    assert_eq!(source.paired_active.load(Ordering::SeqCst), 0);
    assert_eq!(source.paired_exited.load(Ordering::SeqCst), 1);
    assert_eq!(source.active.load(Ordering::SeqCst), 0);
    assert_eq!(source.exited.load(Ordering::SeqCst), 2);
    assert!(state.list_records().await.is_empty());
    assert_eq!(
        state.inner.lifecycle.connects.available_permits(),
        lifecycle::MAX_CONNECTS
    );
}

#[tokio::test]
async fn lifecycle_tunnel_manual_deadline_late_paired_reply_cannot_start_fallback() {
    let source = FakeSource::new();
    let paired_gate = Arc::new(Semaphore::new(0));
    source.script(
        A,
        [
            Behavior::Failure(std::io::ErrorKind::BrokenPipe),
            Behavior::Success,
        ],
    );
    source.paired.lock().unwrap().push(PairedMobdev2Device {
        udid: A.into(),
        host: "192.0.2.9".into(),
    });
    *source.paired_gate.lock().unwrap() = Some(paired_gate.clone());
    let state = state(source.clone(), Duration::from_millis(60));
    let mut request = Box::pin(state.ensure_tunnel(A, None, None));
    assert!(futures_util::poll!(&mut request).is_pending());
    assert_eq!(source.paired_active.load(Ordering::SeqCst), 1);
    tokio::time::sleep(Duration::from_millis(100)).await;
    paired_gate.add_permits(1);
    assert_elapsed(&request.await.unwrap_err());
    assert_eq!(source.count(A), 1);
    assert_eq!(source.paired_active.load(Ordering::SeqCst), 0);
    assert_eq!(source.paired_exited.load(Ordering::SeqCst), 1);
    assert!(state.list_records().await.is_empty());
    assert_eq!(
        state.inner.lifecycle.connects.available_permits(),
        lifecycle::MAX_CONNECTS
    );
}

#[tokio::test]
async fn lifecycle_tunnel_manual_deadline_late_publication_lock_releases_completed_device() {
    let source = FakeSource::new();
    let gate = Arc::new(Semaphore::new(0));
    source.script(A, [Behavior::Held(gate.clone()), Behavior::Success]);
    let state = state(source.clone(), Duration::from_millis(60));
    let mut request = Box::pin(start(state.clone(), A));
    assert!(futures_util::poll!(&mut request).is_pending());
    let held = state.inner.tunnels.write().await;
    gate.add_permits(1);
    assert!(futures_util::poll!(&mut request).is_pending());
    assert_eq!(source.active.load(Ordering::SeqCst), 0);
    assert_eq!(source.usb_calls.load(Ordering::SeqCst), 1);
    assert_eq!(source.drops.load(Ordering::SeqCst), 0);
    tokio::time::sleep(Duration::from_millis(100)).await;
    drop(held);
    assert_elapsed(&request.await.unwrap_err());
    assert_eq!(source.drops.load(Ordering::SeqCst), 1);
    assert!(state.list_records().await.is_empty());
    assert_eq!(
        state.inner.lifecycle.connects.available_permits(),
        lifecycle::MAX_CONNECTS
    );
    start(state.clone(), A).await.unwrap();
    state.clear().await;
    assert_eq!(source.drops.load(Ordering::SeqCst), 2);
}

#[tokio::test]
async fn lifecycle_tunnel_manual_deadline_http_timeout_preserves_native_class() {
    let source = FakeSource::new();
    source.script(A, [Behavior::Pending]);
    let state = state(source.clone(), Duration::from_millis(60));
    let (port, server) = spawn_test_router(agent_router(state.clone())).await;
    let response = client()
        .post(format!("http://127.0.0.1:{port}/start-tunnel"))
        .json(&json!({"udid": A, "connection_type": "usbmux"}))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::BAD_GATEWAY);
    assert_eq!(response.headers()["x-ios-error-class"], "timeout");
    let body = response.text().await.unwrap();
    assert!(!body.contains(A));
    assert!(!body.contains("synthetic-private"));
    assert_eq!(source.active.load(Ordering::SeqCst), 0);
    assert!(state.list_records().await.is_empty());
    state.clear().await;
    server.abort();
    assert!(server.await.unwrap_err().is_cancelled());
}
