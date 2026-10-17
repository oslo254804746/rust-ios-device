//! The shared known-capacity permit follows actual native join ownership and
//! table metadata independently. These are real smoltcp reader/writer fixtures.
use super::*;

#[tokio::test]
async fn candidate_input_native_table_removal_keeps_input_until_original_joins_and_strict_eof() {
    let expected = baseline();
    let source = LocalSource::new();
    let state = state(source.clone());
    state.refresh_once().await.unwrap();
    let bank = Arc::clone(&state.inner.global_metadata_bytes);
    let input = {
        let table = state.inner.tunnels.read().await;
        Arc::downgrade(table[DEVICE]._input.as_ref().expect("scan input omitted"))
    };
    let route = {
        let table = state.inner.tunnels.read().await;
        Arc::downgrade(&table[DEVICE].route_intent)
    };
    let publication = {
        let table = state.inner.tunnels.read().await;
        Arc::downgrade(&table[DEVICE].publication)
    };
    let before = bank.available_permits();
    state.clear().await;
    assert_eq!(state.inner.native.snapshot().retired, 1);
    assert!(
        input.upgrade().is_some(),
        "input refunded before original native joins"
    );
    assert!(
        route.upgrade().is_none(),
        "removed route still owns its body"
    );
    assert!(
        publication.upgrade().is_none(),
        "removed publication payload is still alive"
    );
    assert_eq!(
        bank.available_permits(),
        before
            + actual_publication_destination_bytes()
            + route_body_bytes()
            + publication::Life::bytes()
    );
    state.drain_native(OBSERVE).await.unwrap();
    assert!(input.upgrade().is_none());
    assert!(source.handle().shutdown_status().is_ok());
    assert!(
        recovered(&source, expected).await,
        "original peer EOF/tasks did not recover"
    );
    state.notify_shutdown();
    drop(state);
    assert_eq!(bank.available_permits(), MAX_GLOBAL_KNOWN_METADATA_BYTES);
}

#[tokio::test]
async fn candidate_input_native_original_joins_keep_input_while_table_still_owns_it() {
    let expected = baseline();
    let source = LocalSource::new();
    let state = state(source.clone());
    state.refresh_once().await.unwrap();
    let bank = Arc::clone(&state.inner.global_metadata_bytes);
    let input = {
        let table = state.inner.tunnels.read().await;
        Arc::downgrade(table[DEVICE]._input.as_ref().unwrap())
    };
    let before = bank.available_permits();
    state.notify_shutdown();
    state.drain_native(OBSERVE).await.unwrap();
    assert!(state.inner.native.snapshot().closed_and_empty());
    assert_eq!(state.inner.tunnels.read().await.len(), 1);
    assert!(input.upgrade().is_some());
    assert_eq!(bank.available_permits(), before);
    assert!(source.handle().shutdown_status().is_ok());
    assert!(recovered(&source, expected).await);
    state.clear().await;
    assert!(input.upgrade().is_none());
    drop(state);
    assert_eq!(bank.available_permits(), MAX_GLOBAL_KNOWN_METADATA_BYTES);
}

fn record_copy_bytes() -> usize {
    // Same raw fixture as NativeDevice::record; ports have no text storage.
    let mut record = fake_record(1);
    record.udid = DEVICE.into();
    [
        &record.udid,
        &record.interface,
        &record.connection_type,
        &record.mode,
        &record.address,
        &record.tunnel_address,
    ]
    .iter()
    .map(|text| text.len())
    .sum::<usize>()
        + record
            .userspace_tun_host
            .as_ref()
            .map_or(0, |text| text.len())
}

fn route_body_bytes() -> usize {
    // These fixtures select Usbmux/Wifi(None), so the independent retained
    // route has a real Arc payload and no host allocation. Unlike the native
    // device permit, it ends when its last table/desired owner is destroyed.
    std::mem::size_of::<intent::RetainedRoute>()
}

fn source_future_bytes(source: &dyn Source) -> usize {
    struct NeverPoll;
    impl TunnelOwner for NeverPoll {
        fn reserve(
            &self,
        ) -> Result<Box<dyn ios_core::TunnelReservation + '_>, ios_core::CoreError> {
            panic!("measuring an empty Source allocation must never poll it")
        }
    }
    let bank = Arc::new(Semaphore::new(MAX_GLOBAL_KNOWN_METADATA_BYTES));
    let future = source
        .connect_owned(
            DEVICE,
            Route::Wifi(None),
            TunMode::Userspace,
            &NeverPoll,
            &bank,
        )
        .unwrap();
    let bytes = MAX_GLOBAL_KNOWN_METADATA_BYTES - bank.available_permits();
    drop(future);
    assert_eq!(bank.available_permits(), MAX_GLOBAL_KNOWN_METADATA_BYTES);
    bytes
}

struct SourcePeak {
    _hook: future_storage::DropGuard,
    held: Arc<StdMutex<Option<tokio::sync::OwnedSemaphorePermit>>>,
}
impl SourcePeak {
    fn new(state: &TunnelAgentState, bytes: usize) -> Self {
        let bank = state.inner.global_metadata_bytes.clone();
        let held = Arc::new(StdMutex::new(None));
        let captured = held.clone();
        let hook = future_storage::after_drop(move || {
            // The actual Source Box and its fee are ALREADY destroyed. Model
            // other quota usage with its measured temporary peak, retaining
            // the original zero-free-byte metadata/cleanup fixture boundary.
            *captured.lock().unwrap() =
                Some(bank.clone().try_acquire_many_owned(bytes as u32).unwrap());
        });
        Self { _hook: hook, held }
    }
    fn checked(&self) {
        assert!(
            self.held.lock().unwrap().is_some(),
            "original Source Box Drop boundary not reached"
        );
    }
}

fn configured(source: Arc<LocalSource>) -> (TunnelAgentState, usize, SourcePeak) {
    configured_with_discovery(source, 0)
}

fn configured_with_discovery(
    source: Arc<LocalSource>,
    discovery: usize,
) -> (TunnelAgentState, usize, SourcePeak) {
    let temporary = source_future_bytes(&*source);
    let mut state = state(source);
    let charged = std::mem::size_of::<NativeDevice>()
        + actual_publication_destination_bytes()
        + publication::Life::bytes();
    Arc::get_mut(&mut state.inner).unwrap().metadata_bytes = Arc::new(Semaphore::new(charged));
    let native_bytes =
        state.inner.native.structural_bytes() + reconnect::Ledger::structural_bytes();
    state.set_global_metadata_limit(
        native_bytes
            + charged
            + Lifecycle::initial_metadata_bytes(DEVICE)
            + record_copy_bytes()
            + route_body_bytes()
            + temporary
            + discovery,
    );
    let peak = SourcePeak::new(&state, temporary);
    (state, charged, peak)
}

#[tokio::test]
async fn native_metadata_charge_survives_table_removal_until_original_joins_and_eof() {
    let expected = baseline();
    let source = LocalSource::new();
    let (state, charged, peak) = configured(source.clone());
    let record = publish(&state, Route::Wifi(None)).await.unwrap();
    peak.checked();
    assert_eq!(state.inner.metadata_bytes.available_permits(), 0);
    assert_eq!(state.inner.global_metadata_bytes.available_permits(), 0);
    state.clear().await;
    assert_eq!(state.inner.native.snapshot().retired, 1);
    assert_eq!(
        state.inner.metadata_bytes.available_permits(),
        actual_publication_destination_bytes() + publication::Life::bytes()
    );
    assert_eq!(
        state.inner.global_metadata_bytes.available_permits(),
        actual_publication_destination_bytes() + route_body_bytes() + publication::Life::bytes()
    );
    state.drain_native(OBSERVE).await.unwrap();
    assert_eq!(state.inner.metadata_bytes.available_permits(), charged);
    assert_eq!(
        state.inner.global_metadata_bytes.available_permits(),
        charged + route_body_bytes()
    );
    assert!(source.handle().shutdown_status().is_ok());
    assert!(
        recovered(&source, expected).await,
        "native peer did not reach EOF or task baseline"
    );
    state.notify_shutdown();
    drop(record);
    assert_eq!(
        state.inner.global_metadata_bytes.available_permits(),
        charged + route_body_bytes() + record_copy_bytes()
    );
}

#[tokio::test]
async fn native_metadata_charge_survives_certified_joins_while_table_still_owns_metadata() {
    let expected = baseline();
    let source = LocalSource::new();
    let (state, charged, peak) = configured(source.clone());
    let record = publish(&state, Route::Wifi(None)).await.unwrap();
    peak.checked();
    state.notify_shutdown();
    state.drain_native(OBSERVE).await.unwrap();
    assert!(state.inner.native.snapshot().closed_and_empty());
    assert_eq!(state.inner.tunnels.read().await.len(), 1);
    assert_eq!(state.inner.metadata_bytes.available_permits(), 0);
    assert_eq!(state.inner.global_metadata_bytes.available_permits(), 0);
    assert!(source.handle().shutdown_status().is_ok());
    assert!(recovered(&source, expected).await);
    state.clear().await;
    assert_eq!(state.inner.metadata_bytes.available_permits(), charged);
    assert_eq!(
        state.inner.global_metadata_bytes.available_permits(),
        charged + route_body_bytes()
    );
    drop(record);
    assert_eq!(
        state.inner.global_metadata_bytes.available_permits(),
        charged + route_body_bytes() + record_copy_bytes()
    );
}

#[tokio::test]
async fn native_metadata_charge_survives_cancelled_publication_query_until_original_joins() {
    let expected = baseline();
    let source = LocalSource::new();
    let probe = Arc::new(Semaphore::new(MAX_GLOBAL_KNOWN_METADATA_BYTES));
    let frame = source.usb_devices_owned(&probe).unwrap();
    let discovery = MAX_GLOBAL_KNOWN_METADATA_BYTES - probe.available_permits();
    drop(frame); // Actual local constructor only, no discovery poll.
    assert_eq!(probe.available_permits(), MAX_GLOBAL_KNOWN_METADATA_BYTES);
    let (state, charged, peak) = configured_with_discovery(source.clone(), discovery);
    *source.usb_gate.lock().unwrap() = Some(Arc::new(Semaphore::new(0)));
    let mut connection = Box::pin(publish(&state, Route::Usbmux));
    assert!(futures_util::poll!(&mut connection).is_pending());
    peak.checked();
    assert_eq!(source.usb_calls.load(Ordering::SeqCst), 1);
    assert_eq!(state.inner.metadata_bytes.available_permits(), 0);
    assert_eq!(state.inner.global_metadata_bytes.available_permits(), 0);
    drop(connection);
    assert_eq!(state.inner.tunnels.read().await.len(), 0);
    assert_eq!(state.inner.native.snapshot().retired, 1);
    let backing = state.inner.tunnels.read().await.backing_bytes();
    let node_bytes = actual_publication_destination_bytes() - backing;
    assert_eq!(
        state.inner.metadata_bytes.available_permits(),
        node_bytes + publication::Life::bytes()
    );
    assert_eq!(
        state.inner.global_metadata_bytes.available_permits(),
        node_bytes
            + record_copy_bytes()
            + route_body_bytes()
            + publication::Life::bytes()
            + discovery
    );
    state.drain_native(OBSERVE).await.unwrap();
    assert_eq!(
        state.inner.metadata_bytes.available_permits(),
        charged - backing
    );
    assert_eq!(
        state.inner.global_metadata_bytes.available_permits(),
        charged - backing + record_copy_bytes() + route_body_bytes() + discovery
    );
    assert!(source.handle().shutdown_status().is_ok());
    assert!(recovered(&source, expected).await);
    state.notify_shutdown();
    state.clear().await;
    assert_eq!(state.inner.metadata_bytes.available_permits(), charged);
}

#[tokio::test]
async fn global_metadata_rejected_native_publication_keeps_snapshot_charge_and_joins_owner() {
    let expected = baseline();
    let source = LocalSource::new();
    let (mut state, charged, old_peak) = configured(source.clone());
    drop(old_peak);
    let temporary = source_future_bytes(&*source);
    let devices = vec![source::UsbDevice {
        udid: DEVICE.into(),
        device_id: 7,
        usb: true,
    }];
    let snapshot_size =
        devices.capacity() * std::mem::size_of::<source::UsbDevice>() + devices[0].udid.capacity();
    let total = charged + snapshot_size + route_body_bytes() - 1;
    let native_bytes =
        state.inner.native.structural_bytes() + reconnect::Ledger::structural_bytes();
    state.set_global_metadata_limit(
        native_bytes + (total + Lifecycle::initial_metadata_bytes(DEVICE)) + temporary,
    );
    let peak = SourcePeak::new(&state, temporary);
    let snapshot =
        admission::retain_usb_snapshot(&state.inner.global_metadata_bytes, &devices).unwrap();
    let result = publish(&state, Route::Wifi(None)).await;
    peak.checked();
    assert_eq!(
        crate::output::operation_error_class(result.unwrap_err().as_ref()),
        ios_core::ErrorClass::ResourceLimit
    );
    assert!(state.inner.tunnels.read().await.is_empty());
    let backing = state.inner.tunnels.read().await.backing_bytes();
    assert_eq!(
        state.inner.global_metadata_bytes.available_permits(),
        charged - backing + route_body_bytes() - 1
    );
    assert_eq!(
        state.inner.metadata_bytes.available_permits(),
        charged - backing
    );
    assert_eq!(state.inner.native.snapshot().retired, 1);
    state.drain_native(OBSERVE).await.unwrap();
    assert!(source.handle().shutdown_status().is_ok());
    assert!(recovered(&source, expected).await);
    assert_eq!(
        state.inner.global_metadata_bytes.available_permits(),
        charged - backing + route_body_bytes() - 1
    );
    drop(snapshot);
    assert_eq!(
        state.inner.global_metadata_bytes.available_permits(),
        total - backing
    );
    state.notify_shutdown();
    state.clear().await;
}
