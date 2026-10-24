use super::*;
use std::task::Poll;

fn snapshot(source: &FakeSource, rows: &[(&str, u32)]) {
    *source.usb_details.lock().unwrap() = Some(
        rows.iter()
            .map(|(udid, id)| source::UsbDevice {
                udid: (*udid).into(),
                device_id: *id,
                usb: true,
            })
            .collect(),
    );
}

fn device_id(source: &FakeSource, udid: &str, id: u32) {
    source.connected_ids.lock().unwrap().insert(udid.into(), id);
}

#[tokio::test]
async fn lifecycle_tunnel_publication_manual_native_result_detached_during_connect_is_released() {
    let source = FakeSource::new();
    snapshot(&source, &[(A, 71)]);
    device_id(&source, A, 71);
    let gate = Arc::new(Semaphore::new(0));
    source.script(A, [Behavior::Held(gate.clone())]);
    let state = state(source.clone(), TUNNEL_CONNECT_TIMEOUT);
    let mut control_input_0 = state.prepare_cutover::<2>().unwrap();

    let mut request =
        Box::pin(state.ensure_tunnel(A, None, Some(StartTunnelConnectionType::Usbmux)));
    assert!(futures_util::poll!(&mut request).is_pending());
    snapshot(&source, &[]);
    gate.add_permits(1);
    let result = request.await;
    let published = state.get_record(A).await.unwrap().is_some();
    let released = source.drops.load(Ordering::SeqCst);
    state.clear(&mut control_input_0).await;
    drop(control_input_0);
    assert!(result.unwrap_err().is::<ios_core::OperationCancelled>());
    assert!(!published);
    assert_eq!(released, 1);
    assert_eq!(source.usb_calls.load(Ordering::SeqCst), 1);
    assert_eq!(
        state.inner.lifecycle.connects.available_permits(),
        lifecycle::MAX_CONNECTS
    );
}

#[tokio::test]
async fn lifecycle_tunnel_publication_scan_matching_old_id_is_rejected_by_new_snapshot() {
    let source = FakeSource::new();
    snapshot(&source, &[(A, 71)]);
    device_id(&source, A, 71);
    let gate = Arc::new(Semaphore::new(0));
    source.script(A, [Behavior::Held(gate.clone())]);
    let state = state(source.clone(), TUNNEL_CONNECT_TIMEOUT);
    let mut control_input_0 = state.prepare_cutover::<2>().unwrap();

    let mut scan = Box::pin(state.refresh_once());
    assert!(futures_util::poll!(&mut scan).is_pending());
    snapshot(&source, &[(A, 72)]);
    gate.add_permits(1);
    scan.await.unwrap();
    let published = state.get_record(A).await.unwrap().is_some();
    let released = source.drops.load(Ordering::SeqCst);
    let failed = state.inner.failed_attempts.read().await.contains_key(A);
    state.clear(&mut control_input_0).await;
    drop(control_input_0);
    assert!(
        !published,
        "old result matched initial snapshot but not current attachment"
    );
    assert_eq!(released, 1);
    assert!(!failed);
    assert_eq!(source.usb_calls.load(Ordering::SeqCst), 2);
}

#[tokio::test]
async fn lifecycle_tunnel_publication_query_wait_shares_original_connect_deadline() {
    let source = FakeSource::new();
    let connect_gate = Arc::new(Semaphore::new(0));
    let query_gate = Arc::new(Semaphore::new(0));
    source.script(A, [Behavior::Held(connect_gate.clone())]);
    let state = state(source.clone(), Duration::from_millis(80));
    let mut control_input_0 = state.prepare_cutover::<2>().unwrap();

    let mut request =
        Box::pin(state.ensure_tunnel(A, None, Some(StartTunnelConnectionType::Usbmux)));
    assert!(futures_util::poll!(&mut request).is_pending());
    *source.usb_gate.lock().unwrap() = Some(query_gate);
    connect_gate.add_permits(1);
    let result = tokio::time::timeout(LOCAL_BUDGET, request).await.unwrap();
    let released = source.drops.load(Ordering::SeqCst);
    state.clear(&mut control_input_0).await;
    drop(control_input_0);
    assert!(result.unwrap_err().is::<tokio::time::error::Elapsed>());
    assert_eq!(source.usb_calls.load(Ordering::SeqCst), 1);
    assert_eq!(source.active.load(Ordering::SeqCst), 0);
    assert_eq!(released, 1);
    assert_eq!(
        state.inner.lifecycle.connects.available_permits(),
        lifecycle::MAX_CONNECTS
    );
}

#[tokio::test]
async fn lifecycle_tunnel_publication_stop_cancels_query_with_owned_device_before_store() {
    let source = FakeSource::new();
    let connect_gate = Arc::new(Semaphore::new(0));
    let query_gate = Arc::new(Semaphore::new(0));
    source.script(A, [Behavior::Held(connect_gate.clone())]);
    let state = state(source.clone(), TUNNEL_CONNECT_TIMEOUT);
    let mut stop_input_0 = state.prepare_cutover::<3>().unwrap();

    let mut request =
        Box::pin(state.ensure_tunnel(A, None, Some(StartTunnelConnectionType::Usbmux)));
    assert!(futures_util::poll!(&mut request).is_pending());
    *source.usb_gate.lock().unwrap() = Some(query_gate);
    connect_gate.add_permits(1);
    let stage = futures_util::poll!(&mut request);
    assert_eq!(source.active.load(Ordering::SeqCst), 0);
    assert_eq!(source.drops.load(Ordering::SeqCst), 0);
    let removed = tokio::time::timeout(LOCAL_BUDGET, state.remove_tunnel(A, &mut stop_input_0))
        .await
        .unwrap();
    drop(stop_input_0);
    let result = match stage {
        Poll::Pending => request.await,
        Poll::Ready(result) => result,
    };
    assert!(!removed, "result was stored before attachment query");
    assert!(result.unwrap_err().is::<ios_core::OperationCancelled>());
    assert_eq!(source.drops.load(Ordering::SeqCst), 1);
    assert!(state.list_records().await.unwrap().as_slice().is_empty());
    assert_eq!(
        state.inner.lifecycle.connects.available_permits(),
        lifecycle::MAX_CONNECTS
    );
}

#[tokio::test]
async fn lifecycle_tunnel_publication_query_native_io_is_preserved_after_connect() {
    let source = FakeSource::new();
    source.list_failure.store(true, Ordering::SeqCst);
    let state = state(source.clone(), TUNNEL_CONNECT_TIMEOUT);
    let mut control_input_0 = state.prepare_cutover::<2>().unwrap();

    let result = start(state.clone(), A).await;
    let released = source.drops.load(Ordering::SeqCst);
    state.clear(&mut control_input_0).await;
    drop(control_input_0);
    let error = result.unwrap_err();
    assert_eq!(
        error.downcast_ref::<std::io::Error>().unwrap().kind(),
        std::io::ErrorKind::BrokenPipe
    );
    assert!(!crate::output::error_message(&error).contains("synthetic-private"));
    assert_eq!(source.count(A), 1);
    assert_eq!(source.usb_calls.load(Ordering::SeqCst), 1);
    assert_eq!(released, 1);
}

#[tokio::test]
async fn lifecycle_tunnel_publication_another_live_transport_id_is_accepted() {
    let source = FakeSource::new();
    snapshot(&source, &[(A, 71), (A, 72)]);
    device_id(&source, A, 71);
    let state = state(source.clone(), TUNNEL_CONNECT_TIMEOUT);
    let mut control_input_0 = state.prepare_cutover::<2>().unwrap();

    start(state.clone(), A).await.unwrap();
    assert_eq!(source.usb_calls.load(Ordering::SeqCst), 1);
    assert!(state.get_record(A).await.unwrap().is_some());
    assert_eq!(source.drops.load(Ordering::SeqCst), 0);
    state.clear(&mut control_input_0).await;
    drop(control_input_0);
    assert_eq!(source.drops.load(Ordering::SeqCst), 1);
}
