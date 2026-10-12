use super::*;

#[tokio::test]
async fn global_metadata_scan_retains_usb_charge_across_paired_wait_and_drop_releases_it() {
    let source = FakeSource::new();
    *source.usb_details.lock().unwrap() = Some(vec![source::UsbDevice {
        udid: A.into(),
        device_id: 7,
        usb: true,
    }]);
    *source.paired_gate.lock().unwrap() = Some(Arc::new(Semaphore::new(0)));
    let mut state = state(source.clone(), TUNNEL_CONNECT_TIMEOUT);
    let charge = std::mem::size_of::<source::UsbDevice>() + A.len();
    state.set_global_metadata_limit(charge + Lifecycle::initial_metadata_bytes(A));
    let mut scan = Box::pin(state.refresh_once());
    assert!(futures_util::poll!(&mut scan).is_pending());
    assert_eq!(source.paired_calls.load(Ordering::SeqCst), 1);
    assert_eq!(state.inner.global_metadata_bytes.available_permits(), 0);
    assert!(state.list_records().await.is_empty());
    drop(scan);
    assert_eq!(
        state.inner.global_metadata_bytes.available_permits(),
        charge
    );
    assert_eq!(source.paired_active.load(Ordering::SeqCst), 0);
    assert_eq!(source.paired_exited.load(Ordering::SeqCst), 1);
    assert!(source.calls.lock().unwrap().is_empty());
    state.notify_shutdown();
    state.clear().await;
}

#[tokio::test]
async fn global_metadata_rejected_usb_snapshot_keeps_old_publication_and_no_extra_source() {
    let source = FakeSource::new();
    let mut state = state(source.clone(), TUNNEL_CONNECT_TIMEOUT);
    let charge =
        std::mem::size_of::<ManagedDevice>() + std::mem::size_of::<ManagedTunnel>() + A.len();
    state.set_global_metadata_limit(charge + Lifecycle::initial_metadata_bytes(A));
    state
        .ensure_tunnel(A, None, Some(StartTunnelConnectionType::Wifi))
        .await
        .unwrap();
    let publication = state.inner.tunnels.read().await[A].publication.clone();
    assert_eq!(state.inner.global_metadata_bytes.available_permits(), 0);
    *source.usb_details.lock().unwrap() = Some(vec![source::UsbDevice {
        udid: A.into(),
        device_id: 7,
        usb: true,
    }]);
    let result = state.refresh_once().await;
    assert_eq!(
        crate::output::operation_error_class(result.unwrap_err().as_ref()),
        ios_core::ErrorClass::ResourceLimit
    );
    assert!(Arc::ptr_eq(
        &state.inner.tunnels.read().await[A].publication,
        &publication
    ));
    assert_eq!(source.paired_calls.load(Ordering::SeqCst), 0);
    assert_eq!(source.count(A), 1);
    assert_eq!(state.inner.global_metadata_bytes.available_permits(), 0);
    state.clear().await;
    assert_eq!(
        state.inner.global_metadata_bytes.available_permits(),
        charge
    );
    state.notify_shutdown();
}
