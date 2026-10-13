use super::*;

#[tokio::test]
async fn scan_capsule_preflight_refusal_has_zero_source_calls_and_no_history_or_ready_change() {
    let source = FakeSource::new();
    source.usb.lock().unwrap().push(A.into());
    let mut state = state(source.clone(), TUNNEL_CONNECT_TIMEOUT);
    let bytes = scan::capsule_reservation_bytes() - 1;
    let native_bytes = state.inner.native.structural_bytes();
    state.set_global_metadata_limit(native_bytes + (bytes));
    let bank = Arc::clone(&state.inner.global_metadata_bytes);
    let error = state.refresh_once().await.unwrap_err();
    assert_eq!(
        crate::output::operation_error_class(error.as_ref()),
        ios_core::ErrorClass::ResourceLimit
    );
    assert_eq!(source.usb_calls.load(Ordering::SeqCst), 0);
    assert_eq!(source.paired_calls.load(Ordering::SeqCst), 0);
    assert!(source.calls.lock().unwrap().is_empty());
    assert_eq!(state.inner.lifecycle.coordinator_count().await, 0);
    assert_eq!(state.inner.scan_outcomes.payload_history().0, 0);
    assert!(state.inner.scan_outcomes.history().is_empty());
    assert!(!state.is_ready().await);
    assert_eq!(bank.available_permits(), bytes);
    drop(state);
    assert_eq!(bank.available_permits(), bytes + native_bytes);
}

#[tokio::test]
async fn desired_listing_bytes_refusal_is_before_discovery_and_preserves_stop() {
    let source = FakeSource::new();
    let mut state = state(source.clone(), TUNNEL_CONNECT_TIMEOUT);
    let reserve = scan::capsule_reservation_bytes();
    let bytes = Lifecycle::initial_metadata_bytes(A) + reserve;
    let native_bytes = state.inner.native.structural_bytes();
    state.set_global_metadata_limit(native_bytes + (bytes));
    let bank = Arc::clone(&state.inner.global_metadata_bytes);
    let ticket = state.inner.lifecycle.ticket(A).await.unwrap();
    state.inner.lifecycle.stop(A).await;
    let before = ticket.desired();
    assert_eq!(bank.available_permits(), reserve);
    let error = state.refresh_once().await.unwrap_err();
    assert_eq!(
        crate::output::operation_error_class(error.as_ref()),
        ios_core::ErrorClass::ResourceLimit
    );
    assert_eq!(source.usb_calls.load(Ordering::SeqCst), 0);
    assert_eq!(source.paired_calls.load(Ordering::SeqCst), 0);
    assert!(source.calls.lock().unwrap().is_empty());
    assert!(Arc::ptr_eq(&before.version, &ticket.desired().version));
    assert!(matches!(ticket.desired().desired, intent::Desired::Stopped));
    assert!(!state.is_ready().await);
    assert_eq!(bank.available_permits(), reserve);
    drop(ticket);
    drop(state);
    assert_eq!(bank.available_permits(), bytes + native_bytes);
}

#[tokio::test]
async fn write_batch_bytes_refusal_happens_before_original_source_poll() {
    let source = FakeSource::new();
    source.usb.lock().unwrap().push(A.into());
    let raw = std::mem::size_of::<source::UsbDevice>() + A.len();
    let selection = std::mem::size_of::<source::UsbDevice>();
    let rows = candidate_index::Candidate::first_usb_bytes(A);
    let bytes = scan::capsule_reservation_bytes()
        + raw
        + selection
        + rows
        + Lifecycle::initial_metadata_bytes(A)
        + bookkeeping::WriteBatch::backing_bytes(1)
        - 1;
    let mut state = state(source.clone(), TUNNEL_CONNECT_TIMEOUT);
    let native_bytes = state.inner.native.structural_bytes();
    state.set_global_metadata_limit(native_bytes + (bytes));
    let bank = Arc::clone(&state.inner.global_metadata_bytes);
    let error = state.refresh_once().await.unwrap_err();
    assert_eq!(
        crate::output::operation_error_class(error.as_ref()),
        ios_core::ErrorClass::ResourceLimit
    );
    assert_eq!(source.paired_calls.load(Ordering::SeqCst), 1);
    assert!(source.calls.lock().unwrap().is_empty());
    assert!(state.inner.failed_attempts.read().await.is_empty());
    assert_eq!(state.inner.retry.receipt_count(), 0);
    assert_eq!(state.inner.native.snapshot().occupied(), 0);
    assert!(!state.is_ready().await);
    state.notify_shutdown();
    state.clear().await;
    drop(state);
    assert_eq!(bank.available_permits(), bytes + native_bytes);
}

#[tokio::test]
async fn candidate_index_bytes_refusal_preserves_original_backoff_before_source_contact() {
    let source = FakeSource::new();
    source.usb.lock().unwrap().push(A.into());
    let state = state(source.clone(), TUNNEL_CONNECT_TIMEOUT);
    let ticket = state.inner.lifecycle.ticket(B).await.unwrap();
    let slot = state.reserve_failure_slot(B).await.unwrap();
    state
        .record_failed_attempt(
            B,
            AttemptIdentity {
                route: Route::Mobdev2("original-host".into()),
                native_id: None,
            },
            slot,
            &ticket,
        )
        .await;
    let original = state.inner.failed_attempts.read().await[B].clone();
    let bank = Arc::clone(&state.inner.global_metadata_bytes);
    let raw = std::mem::size_of::<source::UsbDevice>() + A.len();
    let selection = std::mem::size_of::<source::UsbDevice>();
    let remaining = scan::capsule_reservation_bytes()
        + raw
        + selection
        + Lifecycle::additional_metadata_bytes(A)
        + candidate_index::Candidate::first_usb_bytes(A)
        + std::mem::size_of::<usize>()
        - 1;
    let occupied = bank
        .clone()
        .try_acquire_many_owned(u32::try_from(bank.available_permits() - remaining).unwrap())
        .unwrap();
    let error = state.refresh_once().await.unwrap_err();
    assert_eq!(
        crate::output::operation_error_class(error.as_ref()),
        ios_core::ErrorClass::ResourceLimit
    );
    let failed = state.inner.failed_attempts.read().await;
    let current = &failed[B];
    assert!(Arc::ptr_eq(&current.version, &original.version));
    assert_eq!(current.last_attempt, original.last_attempt);
    assert_eq!(current.fail_count, original.fail_count);
    assert_eq!(current.identity, original.identity);
    assert!(Arc::ptr_eq(
        current.slot.as_ref().unwrap(),
        original.slot.as_ref().unwrap()
    ));
    drop(failed);
    assert!(source.calls.lock().unwrap().is_empty());
    assert_eq!(source.paired_calls.load(Ordering::SeqCst), 1);
    assert!(!state.is_ready().await);
    assert_eq!(
        bank.available_permits(),
        remaining - Lifecycle::additional_metadata_bytes(A)
    );
    state.notify_shutdown();
    state.clear().await;
    drop(ticket);
    drop(original);
    drop(state);
    drop(occupied);
    assert_eq!(bank.available_permits(), MAX_GLOBAL_KNOWN_METADATA_BYTES);
}

#[tokio::test]
async fn paired_plan_bytes_refusal_is_incomplete_and_still_publishes_admitted_usb() {
    let source = FakeSource::new();
    source.usb.lock().unwrap().push(A.into());
    let host = "h".repeat(2048);
    source.paired.lock().unwrap().push(PairedMobdev2Device {
        udid: B.into(),
        host: host.clone(),
    });
    let raw_usb = std::mem::size_of::<source::UsbDevice>() + A.len();
    let selection = std::mem::size_of::<source::UsbDevice>();
    let raw_paired = std::mem::size_of::<PairedMobdev2Device>() + B.len() + host.len();
    let bytes = scan::capsule_reservation_bytes()
        + raw_usb
        + selection
        + Lifecycle::initial_metadata_bytes(A)
        + candidate_index::Candidate::first_usb_bytes(A)
        + raw_paired
        + std::mem::size_of::<usize>()
        - 1;
    let mut state = state(source.clone(), TUNNEL_CONNECT_TIMEOUT);
    let native_bytes = state.inner.native.structural_bytes();
    state.set_global_metadata_limit(native_bytes + (bytes));
    let bank = Arc::clone(&state.inner.global_metadata_bytes);
    let error = state.refresh_once().await.unwrap_err();
    assert_eq!(
        crate::output::operation_error_class(error.as_ref()),
        ios_core::ErrorClass::ResourceLimit
    );
    assert_eq!(source.count(A), 1);
    assert_eq!(
        source.count(B),
        0,
        "rejected paired plan must not create a connection"
    );
    assert_eq!(source.paired_calls.load(Ordering::SeqCst), 1);
    assert_eq!(state.inner.lifecycle.coordinator_count().await, 1);
    assert_eq!(state.list_records().await.len(), 1);
    assert!(!state.is_ready().await);
    state.clear().await;
    assert_eq!(
        bank.available_permits(),
        bytes - Lifecycle::initial_metadata_bytes(A)
    );
    assert_eq!(source.active.load(Ordering::SeqCst), 0);
    assert_eq!(source.drops.load(Ordering::SeqCst), 1);
    state.notify_shutdown();
    drop(state);
    assert_eq!(bank.available_permits(), bytes + native_bytes);
}

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
    let selection = std::mem::size_of::<source::UsbDevice>();
    let candidate = candidate_index::Candidate::first_usb_bytes(A);
    let native_bytes = state.inner.native.structural_bytes();
    state.set_global_metadata_limit(
        native_bytes
            + (scan::capsule_reservation_bytes()
                + charge
                + selection
                + candidate
                + Lifecycle::initial_metadata_bytes(A)),
    );
    let mut scan = Box::pin(state.refresh_once());
    assert!(futures_util::poll!(&mut scan).is_pending());
    assert_eq!(source.paired_calls.load(Ordering::SeqCst), 1);
    assert_eq!(state.inner.global_metadata_bytes.available_permits(), 0);
    assert!(state.list_records().await.is_empty());
    drop(scan);
    assert_eq!(
        state.inner.global_metadata_bytes.available_permits(),
        scan::capsule_reservation_bytes() + charge + selection + candidate
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
    let reserve = scan::capsule_reservation_bytes();
    let native_bytes = state.inner.native.structural_bytes();
    state.set_global_metadata_limit(
        native_bytes + (charge + Lifecycle::initial_metadata_bytes(A) + reserve),
    );
    state
        .ensure_tunnel(A, None, Some(StartTunnelConnectionType::Wifi))
        .await
        .unwrap();
    let publication = state.inner.tunnels.read().await[A].publication.clone();
    assert_eq!(
        state.inner.global_metadata_bytes.available_permits(),
        reserve
    );
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
    assert_eq!(
        state.inner.global_metadata_bytes.available_permits(),
        reserve
    );
    state.clear().await;
    assert_eq!(
        state.inner.global_metadata_bytes.available_permits(),
        charge + reserve
    );
    state.notify_shutdown();
}
