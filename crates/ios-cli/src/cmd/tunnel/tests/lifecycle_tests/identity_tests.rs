use super::*;

fn attached(source: &FakeSource, rows: &[(&str, u32, bool)]) {
    *source.usb_details.lock().unwrap() = Some(
        rows.iter()
            .map(|(udid, id, usb)| source::UsbDevice {
                udid: (*udid).into(),
                device_id: *id,
                usb: *usb,
            })
            .collect(),
    );
}

fn connected(source: &FakeSource, udid: &str, id: u32) {
    source.connected_ids.lock().unwrap().insert(udid.into(), id);
}

async fn id(state: &TunnelAgentState, udid: &str) -> u32 {
    state
        .inner
        .tunnels
        .read()
        .await
        .get(udid)
        .unwrap()
        .device
        .device_id()
}

#[tokio::test]
async fn lifecycle_tunnel_identity_replug_replaces_alive_old_native_id() {
    let source = FakeSource::new();
    attached(&source, &[(A, 71, true)]);
    connected(&source, A, 71);
    let state = state(source.clone(), TUNNEL_CONNECT_TIMEOUT);
    let mut control_input_0 = state.prepare_cutover::<2>().unwrap();

    start(state.clone(), A).await.unwrap();
    assert_eq!(id(&state, A).await, 71);
    attached(&source, &[(A, 72, true)]);
    connected(&source, A, 72);
    state.refresh_once().await.unwrap();
    let observed = id(&state, A).await;
    let calls = source.count(A);
    let retired = source.drops.load(Ordering::SeqCst);
    state.clear(&mut control_input_0).await;
    drop(control_input_0);
    assert_eq!(
        observed, 72,
        "same UDID retained the disconnected native identity"
    );
    assert_eq!(calls, 2);
    assert_eq!(retired, 1);
    assert_eq!(source.drops.load(Ordering::SeqCst), 2);
}

#[tokio::test]
async fn lifecycle_tunnel_identity_stale_candidate_rejects_new_generation_without_backoff() {
    let source = FakeSource::new();
    attached(&source, &[(A, 71, true)]);
    connected(&source, A, 72);
    let state = state(source.clone(), TUNNEL_CONNECT_TIMEOUT);
    let mut control_input_0 = state.prepare_cutover::<2>().unwrap();
    let mut control_input_1 = state.prepare_cutover::<2>().unwrap();

    state.refresh_once().await.unwrap();
    let published = state.get_record(A).await.unwrap().is_some();
    let failed = state.inner.failed_attempts.read().await.contains_key(A);
    let released = source.drops.load(Ordering::SeqCst);
    state.clear(&mut control_input_0).await;
    drop(control_input_0);
    assert!(
        !published,
        "old snapshot admitted a different native device generation"
    );
    assert!(
        !failed,
        "snapshot cancellation created a connection-failure backoff"
    );
    assert_eq!(released, 1);
    attached(&source, &[(A, 72, true)]);
    state.refresh_once().await.unwrap();
    assert_eq!(id(&state, A).await, 72);
    assert_eq!(source.count(A), 2);
    state.clear(&mut control_input_1).await;
    drop(control_input_1);
    assert_eq!(source.drops.load(Ordering::SeqCst), 2);
}

#[tokio::test]
async fn lifecycle_tunnel_identity_another_attached_transport_keeps_original_live_record() {
    let source = FakeSource::new();
    attached(&source, &[(A, 71, false)]);
    connected(&source, A, 71);
    let state = state(source.clone(), TUNNEL_CONNECT_TIMEOUT);
    let mut control_input_0 = state.prepare_cutover::<2>().unwrap();

    start(state.clone(), A).await.unwrap();
    attached(&source, &[(A, 72, true), (A, 71, false)]);
    connected(&source, A, 72);
    state.refresh_once().await.unwrap();
    assert_eq!(id(&state, A).await, 71);
    assert_eq!(source.count(A), 1);
    assert_eq!(source.drops.load(Ordering::SeqCst), 0);
    state.clear(&mut control_input_0).await;
    drop(control_input_0);
    assert_eq!(source.drops.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn lifecycle_tunnel_identity_candidate_keeps_core_first_usb_preference() {
    let source = FakeSource::new();
    attached(&source, &[(A, 71, false), (A, 72, true), (A, 73, true)]);
    connected(&source, A, 72);
    let state = state(source.clone(), TUNNEL_CONNECT_TIMEOUT);
    let mut control_input_0 = state.prepare_cutover::<2>().unwrap();

    state.refresh_once().await.unwrap();
    assert_eq!(id(&state, A).await, 72);
    assert_eq!(source.count(A), 1);
    assert!(!state.inner.failed_attempts.read().await.contains_key(A));
    state.clear(&mut control_input_0).await;
    drop(control_input_0);
    assert_eq!(source.drops.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn lifecycle_tunnel_identity_candidate_keeps_core_last_non_usb_fallback() {
    let source = FakeSource::new();
    attached(&source, &[(A, 71, false), (A, 72, false)]);
    connected(&source, A, 72);
    let state = state(source.clone(), TUNNEL_CONNECT_TIMEOUT);
    let mut control_input_0 = state.prepare_cutover::<2>().unwrap();

    state.refresh_once().await.unwrap();
    assert_eq!(id(&state, A).await, 72);
    assert_eq!(source.count(A), 1);
    state.clear(&mut control_input_0).await;
    drop(control_input_0);
    assert_eq!(source.drops.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn lifecycle_tunnel_identity_old_snapshot_does_not_retire_concurrent_new_publication() {
    let source = FakeSource::new();
    attached(&source, &[(A, 71, true)]);
    connected(&source, A, 71);
    let state = state(source.clone(), TUNNEL_CONNECT_TIMEOUT);
    let mut control_input_0 = state.prepare_cutover::<2>().unwrap();

    start(state.clone(), A).await.unwrap();
    let gate = Arc::new(Semaphore::new(0));
    *source.usb_gate.lock().unwrap() = Some(gate.clone());
    let mut scan = Box::pin(state.refresh_once());
    assert!(futures_util::poll!(&mut scan).is_pending());
    source.alive.store(false, Ordering::SeqCst);
    assert!(state.get_record(A).await.unwrap().is_none());
    source.alive.store(true, Ordering::SeqCst);
    connected(&source, A, 72);
    attached(&source, &[(A, 72, true)]);
    *source.usb_gate.lock().unwrap() = None;
    start(state.clone(), A).await.unwrap();
    gate.add_permits(1);
    scan.await.unwrap();
    assert_eq!(id(&state, A).await, 72);
    assert_eq!(source.count(A), 2);
    assert_eq!(source.drops.load(Ordering::SeqCst), 1);
    state.clear(&mut control_input_0).await;
    drop(control_input_0);
    assert_eq!(source.drops.load(Ordering::SeqCst), 2);
}

#[tokio::test]
async fn lifecycle_tunnel_identity_route_family_controls_pruning_instead_of_zero_sentinel() {
    let source = FakeSource::new();
    attached(&source, &[(A, 0, true)]);
    connected(&source, A, 0);
    let state = state(source.clone(), TUNNEL_CONNECT_TIMEOUT);
    start(state.clone(), A).await.unwrap();
    assert_eq!(id(&state, A).await, 0);
    attached(&source, &[]);
    state.refresh_once().await.unwrap();
    assert!(state.get_record(A).await.unwrap().is_none());
    assert_eq!(source.count(A), 1);
    assert_eq!(source.drops.load(Ordering::SeqCst), 1);
    // Zero is a synthetic value in this private fixture, not device-wire evidence.
}
