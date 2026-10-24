use super::*;

#[tokio::test]
async fn lifecycle_tunnel_liveness_inactive_native_result_is_released_without_query() {
    let source = FakeSource::new();
    source.alive.store(false, Ordering::SeqCst);
    let state = state(source.clone(), TUNNEL_CONNECT_TIMEOUT);
    let mut control_input_0 = state.prepare_cutover::<2>().unwrap();

    let result = start(state.clone(), A).await;
    let released = source.drops.load(Ordering::SeqCst);
    state.clear(&mut control_input_0).await;
    drop(control_input_0);
    assert!(result.unwrap_err().is::<ios_core::OperationCancelled>());
    assert_eq!(source.count(A), 1);
    assert_eq!(source.usb_calls.load(Ordering::SeqCst), 0);
    assert_eq!(released, 1);
    assert!(state.list_records().await.unwrap().as_slice().is_empty());
    assert_eq!(
        state.inner.lifecycle.connects.available_permits(),
        lifecycle::MAX_CONNECTS
    );
}

#[tokio::test]
async fn lifecycle_tunnel_liveness_owned_result_dies_during_current_id_query() {
    let source = FakeSource::new();
    let connect_gate = Arc::new(Semaphore::new(0));
    let query_gate = Arc::new(Semaphore::new(0));
    source.script(A, [Behavior::Held(connect_gate.clone())]);
    let state = state(source.clone(), TUNNEL_CONNECT_TIMEOUT);
    let mut control_input_0 = state.prepare_cutover::<2>().unwrap();

    let mut request =
        Box::pin(state.ensure_tunnel(A, None, Some(StartTunnelConnectionType::Usbmux)));
    assert!(futures_util::poll!(&mut request).is_pending());
    *source.usb_gate.lock().unwrap() = Some(query_gate.clone());
    connect_gate.add_permits(1);
    assert!(futures_util::poll!(&mut request).is_pending());
    assert_eq!(source.active.load(Ordering::SeqCst), 0);
    assert_eq!(source.drops.load(Ordering::SeqCst), 0);
    source.alive.store(false, Ordering::SeqCst);
    query_gate.add_permits(1);
    let result = request.await;
    let released = source.drops.load(Ordering::SeqCst);
    state.clear(&mut control_input_0).await;
    drop(control_input_0);
    assert!(result.unwrap_err().is::<ios_core::OperationCancelled>());
    assert_eq!(source.usb_calls.load(Ordering::SeqCst), 1);
    assert_eq!(released, 1);
    assert_eq!(
        state.inner.lifecycle.connects.available_permits(),
        lifecycle::MAX_CONNECTS
    );
}

#[tokio::test]
async fn lifecycle_tunnel_liveness_owned_result_dies_while_publication_lock_pending() {
    let source = FakeSource::new();
    let gate = Arc::new(Semaphore::new(0));
    source.script(A, [Behavior::Held(gate.clone())]);
    let state = state(source.clone(), TUNNEL_CONNECT_TIMEOUT);
    let mut control_input_0 = state.prepare_cutover::<2>().unwrap();

    let mut request =
        Box::pin(state.ensure_tunnel(A, None, Some(StartTunnelConnectionType::Usbmux)));
    assert!(futures_util::poll!(&mut request).is_pending());
    let table = state.inner.tunnels.write().await;
    gate.add_permits(1);
    assert!(futures_util::poll!(&mut request).is_pending());
    assert_eq!(source.usb_calls.load(Ordering::SeqCst), 1);
    assert_eq!(source.drops.load(Ordering::SeqCst), 0);
    source.alive.store(false, Ordering::SeqCst);
    drop(table);
    let result = request.await;
    let released = source.drops.load(Ordering::SeqCst);
    state.clear(&mut control_input_0).await;
    drop(control_input_0);
    assert!(result.unwrap_err().is::<ios_core::OperationCancelled>());
    assert_eq!(released, 1);
    assert!(state.list_records().await.unwrap().as_slice().is_empty());
    assert_eq!(
        state.inner.lifecycle.connects.available_permits(),
        lifecycle::MAX_CONNECTS
    );
}

#[tokio::test]
async fn lifecycle_tunnel_liveness_wifi_uses_same_local_guard_without_daemon_query() {
    let source = FakeSource::new();
    source.alive.store(false, Ordering::SeqCst);
    let state = state(source.clone(), TUNNEL_CONNECT_TIMEOUT);
    let mut control_input_0 = state.prepare_cutover::<2>().unwrap();

    let result = state
        .ensure_tunnel(A, Some("192.0.2.42"), Some(StartTunnelConnectionType::Wifi))
        .await;
    let released = source.drops.load(Ordering::SeqCst);
    state.clear(&mut control_input_0).await;
    drop(control_input_0);
    assert!(result.unwrap_err().is::<ios_core::OperationCancelled>());
    assert_eq!(source.usb_calls.load(Ordering::SeqCst), 0);
    assert_eq!(
        source.calls.lock().unwrap().as_slice(),
        &[(A.into(), Route::Wifi(Some("192.0.2.42".into())))]
    );
    assert_eq!(released, 1);
}
