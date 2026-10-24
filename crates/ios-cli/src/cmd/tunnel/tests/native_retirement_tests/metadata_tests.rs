//! The shared known-capacity permit follows actual native join ownership and
//! table metadata independently. These are real smoltcp reader/writer fixtures.
use super::*;

#[tokio::test]
async fn candidate_input_native_table_removal_keeps_input_until_original_joins_and_strict_eof() {
    let expected = baseline();
    let source = LocalSource::new();
    let state = state(source.clone());
    let mut control_input_0 = state.prepare_cutover::<2>().unwrap();
    let mut control_input_1 = state.prepare_cutover::<1>().unwrap();

    state.refresh_once().await.unwrap();
    let bank = Arc::clone(&state.inner.global_metadata_bytes);
    let input = {
        let table = state.inner.tunnels.read().await;
        table[DEVICE]
            ._input
            .as_ref()
            .expect("scan input omitted")
            .downgrade()
    };
    let route = {
        let table = state.inner.tunnels.read().await;
        table[DEVICE].route_intent.downgrade()
    };
    let publication = {
        let table = state.inner.tunnels.read().await;
        table[DEVICE].publication.downgrade()
    };
    let published = state.inner.metadata_bytes.clone();
    let published_before = published.available_permits();
    let before = bank.available_permits();
    state.clear(&mut control_input_0).await;
    drop(control_input_0);
    assert_eq!(state.inner.native.snapshot().retired, 1);
    assert!(
        input.upgrade().is_some(),
        "input refunded before original native joins"
    );
    assert!(
        route.upgrade().is_none(),
        "removed route value must end before its weak allocation"
    );
    assert!(
        publication.upgrade().is_none(),
        "removed publication payload is still alive"
    );
    assert_eq!(
        bank.available_permits(),
        before
            + actual_publication_destination_bytes()
            + publication::TicketOwner::body_bytes()
            + ControlIdentity::body_bytes(),
        "publication Weak still owns the original body in the global pool"
    );
    assert_eq!(
        published.available_permits(),
        published_before
            + actual_publication_destination_bytes()
            + publication::TicketOwner::body_bytes(),
        "publication Weak still owns the original body in the publication pool"
    );
    drop(publication);
    assert_eq!(
        published.available_permits(),
        published_before
            + actual_publication_destination_bytes()
            + publication::Life::bytes()
            + publication::TicketOwner::body_bytes()
    );
    assert_eq!(
        bank.available_permits(),
        before
            + actual_publication_destination_bytes()
            + publication::Life::bytes()
            + publication::TicketOwner::body_bytes()
            + ControlIdentity::body_bytes()
    );
    drop(route);
    assert_eq!(
        bank.available_permits(),
        before
            + actual_publication_destination_bytes()
            + route_body_bytes()
            + publication::Life::bytes()
            + publication::TicketOwner::body_bytes()
            + ControlIdentity::body_bytes()
    );
    state.drain_native(OBSERVE).await.unwrap();
    assert!(input.upgrade().is_none());
    assert!(source.handle().shutdown_status().is_ok());
    assert!(
        recovered(&source, expected).await,
        "original peer EOF/tasks did not recover"
    );
    state.notify_shutdown(&mut control_input_1);
    drop(control_input_1);
    drop(state);
    assert_eq!(
        bank.available_permits(),
        MAX_GLOBAL_KNOWN_METADATA_BYTES - candidate_input::Input::body_bytes()
    );
    drop(input);
    assert_eq!(bank.available_permits(), MAX_GLOBAL_KNOWN_METADATA_BYTES);
}

#[tokio::test]
async fn candidate_input_native_original_joins_keep_input_while_table_still_owns_it() {
    let expected = baseline();
    let source = LocalSource::new();
    let state = state(source.clone());
    let mut control_input_0 = state.prepare_cutover::<1>().unwrap();
    let mut control_input_1 = state.prepare_cutover::<1>().unwrap();

    state.refresh_once().await.unwrap();
    let bank = Arc::clone(&state.inner.global_metadata_bytes);
    let input = {
        let table = state.inner.tunnels.read().await;
        table[DEVICE]._input.as_ref().unwrap().downgrade()
    };
    let before = bank.available_permits();
    state.notify_shutdown(&mut control_input_0);
    drop(control_input_0);
    state.drain_native(OBSERVE).await.unwrap();
    assert!(state.inner.native.snapshot().closed_and_empty());
    assert_eq!(state.inner.tunnels.read().await.len(), 1);
    assert!(input.upgrade().is_some());
    assert_eq!(
        bank.available_permits(),
        before + ControlIdentity::body_bytes()
    );
    assert!(source.handle().shutdown_status().is_ok());
    assert!(recovered(&source, expected).await);
    state.clear(&mut control_input_1).await;
    drop(control_input_1);
    assert!(input.upgrade().is_none());
    drop(state);
    assert_eq!(
        bank.available_permits(),
        MAX_GLOBAL_KNOWN_METADATA_BYTES - candidate_input::Input::body_bytes()
    );
    drop(input);
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
    // route has a real Arc body/paid guard and no host allocation. A retained
    // Weak keeps this body fee after table/desired value destruction.
    intent::RetainedRoute::body_bytes()
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
            RouteRef::Wifi(None),
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
            // The actual Source and Attempt Boxes and their fees are ALREADY
            // destroyed. Model
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

fn configured(
    source: Arc<LocalSource>,
    control_births: usize,
) -> (TunnelAgentState, usize, SourcePeak) {
    configured_with_discovery(source, 0, control_births)
}

fn configured_with_discovery(
    source: Arc<LocalSource>,
    discovery: usize,
    control_births: usize,
) -> (TunnelAgentState, usize, SourcePeak) {
    let temporary = source_future_bytes(&*source) + ownership::Ownership::attempt_bytes();
    let mut state = state(source);
    let charged = std::mem::size_of::<NativeDevice>()
        + actual_publication_destination_bytes()
        + publication::Life::bytes()
        + publication::TicketOwner::body_bytes()
        + metadata_lease::Lease::pair_bytes();
    state.inner.get_mut().unwrap().metadata_bytes = Arc::new(Semaphore::new(charged));
    let native_bytes = state.inner.native.structural_bytes()
        + reconnect::Ledger::structural_bytes()
        + scan::Outcomes::body_bytes()
        + TunnelAgentOwner::body_bytes()
        + TunnelAgentState::watch_bytes()
        + failure_slot::Counts::body_bytes();
    state.set_global_metadata_limit(
        native_bytes
            + retirement::Registry::incarnation_bytes()
            + charged
            + Lifecycle::initial_metadata_bytes(DEVICE)
            + record_copy_bytes()
            + route_body_bytes()
            + temporary
            + discovery
            + (4 + control_births) * ControlIdentity::body_bytes(),
    );
    let peak = SourcePeak::new(&state, temporary);
    (state, charged, peak)
}

#[tokio::test]
async fn metadata_lease_original_roots_survive_table_removal_and_native_joins_with_independent_refunds(
) {
    let expected = baseline();
    let source = LocalSource::new();
    let (state, charged, peak) = configured(source.clone(), 3);
    let mut prepared_direct_0 = state.prepare_cutover::<2>().unwrap();
    let mut prepared_direct_1 = state.prepare_cutover::<1>().unwrap();

    let record = publish(&state, Route::Wifi(None)).await.unwrap();
    peak.checked();
    let global = state.inner.global_metadata_bytes.clone();
    let publication = state.inner.metadata_bytes.clone();
    let (published_fee, global_fee) = {
        let table = state.inner.tunnels.read().await;
        (
            table[DEVICE]._metadata.clone(),
            table[DEVICE]._global_metadata.clone(),
        )
    };
    let p_pointer = &*published_fee as *const metadata_lease::Lease;
    let g_pointer = &*global_fee as *const metadata_lease::Lease;
    let p_fee = published_fee.num_permits();
    let g_fee = global_fee.num_permits();
    assert_eq!(global.available_permits(), 0);
    assert_eq!(publication.available_permits(), 0);
    state.clear(&mut prepared_direct_0).await;
    drop(prepared_direct_0);
    state.drain_native(OBSERVE).await.unwrap();
    assert!(source.handle().shutdown_status().is_ok());
    assert!(
        recovered(&source, expected).await,
        "original peer strict EOF/tasks did not recover"
    );
    assert_eq!(
        publication.available_permits(),
        actual_publication_destination_bytes()
            + publication::Life::bytes()
            + publication::TicketOwner::body_bytes()
    );
    let before_g = global.available_permits();
    let before_p = publication.available_permits();
    assert_eq!(&*global_fee as *const metadata_lease::Lease, g_pointer);
    assert_eq!(&*published_fee as *const metadata_lease::Lease, p_pointer);
    drop(global_fee);
    assert_eq!(global.available_permits(), before_g + g_fee);
    assert_eq!(
        publication.available_permits(),
        before_p + metadata_lease::Lease::body_bytes()
    );
    assert_eq!(&*published_fee as *const metadata_lease::Lease, p_pointer);
    drop(published_fee);
    assert_eq!(
        global.available_permits(),
        before_g + g_fee + metadata_lease::Lease::body_bytes()
    );
    assert_eq!(
        publication.available_permits(),
        before_p + metadata_lease::Lease::body_bytes() + p_fee
    );
    assert_eq!(publication.available_permits(), charged);
    assert_eq!(
        global.available_permits(),
        charged
            + route_body_bytes()
            + retirement::Registry::incarnation_bytes()
            + ControlIdentity::body_bytes()
    );
    drop(record);
    state.notify_shutdown(&mut prepared_direct_1);
    drop(prepared_direct_1);
}

#[tokio::test]
async fn native_metadata_charge_survives_table_removal_until_original_joins_and_eof() {
    let expected = baseline();
    let source = LocalSource::new();
    let (state, charged, peak) = configured(source.clone(), 3);
    let mut prepared_direct_0 = state.prepare_cutover::<2>().unwrap();
    let mut prepared_direct_1 = state.prepare_cutover::<1>().unwrap();

    let record = publish(&state, Route::Wifi(None)).await.unwrap();
    peak.checked();
    assert_eq!(state.inner.metadata_bytes.available_permits(), 0);
    assert_eq!(state.inner.global_metadata_bytes.available_permits(), 0);
    state.clear(&mut prepared_direct_0).await;
    drop(prepared_direct_0);
    assert_eq!(state.inner.native.snapshot().retired, 1);
    assert_eq!(
        state.inner.metadata_bytes.available_permits(),
        actual_publication_destination_bytes()
            + publication::Life::bytes()
            + publication::TicketOwner::body_bytes()
    );
    assert_eq!(
        state.inner.global_metadata_bytes.available_permits(),
        actual_publication_destination_bytes()
            + route_body_bytes()
            + ControlIdentity::body_bytes()
            + publication::Life::bytes()
            + publication::TicketOwner::body_bytes()
    );
    state.drain_native(OBSERVE).await.unwrap();
    assert_eq!(state.inner.metadata_bytes.available_permits(), charged);
    assert_eq!(
        state.inner.global_metadata_bytes.available_permits(),
        charged
            + route_body_bytes()
            + retirement::Registry::incarnation_bytes()
            + ControlIdentity::body_bytes()
    );
    assert!(source.handle().shutdown_status().is_ok());
    assert!(
        recovered(&source, expected).await,
        "native peer did not reach EOF or task baseline"
    );
    state.notify_shutdown(&mut prepared_direct_1);
    drop(prepared_direct_1);
    drop(record);
    assert_eq!(
        state.inner.global_metadata_bytes.available_permits(),
        charged
            + route_body_bytes()
            + record_copy_bytes()
            + retirement::Registry::incarnation_bytes()
            + 3 * ControlIdentity::body_bytes()
    );
}

#[tokio::test]
async fn native_metadata_charge_survives_certified_joins_while_table_still_owns_metadata() {
    let expected = baseline();
    let source = LocalSource::new();
    let (state, charged, peak) = configured(source.clone(), 2);
    let mut prepared_direct_0 = state.prepare_cutover::<1>().unwrap();
    let mut prepared_direct_1 = state.prepare_cutover::<1>().unwrap();

    let record = publish(&state, Route::Wifi(None)).await.unwrap();
    peak.checked();
    state.notify_shutdown(&mut prepared_direct_0);
    drop(prepared_direct_0);
    state.drain_native(OBSERVE).await.unwrap();
    assert!(state.inner.native.snapshot().closed_and_empty());
    assert_eq!(state.inner.tunnels.read().await.len(), 1);
    assert_eq!(state.inner.metadata_bytes.available_permits(), 0);
    assert_eq!(
        state.inner.global_metadata_bytes.available_permits(),
        ControlIdentity::body_bytes()
    );
    assert!(source.handle().shutdown_status().is_ok());
    assert!(recovered(&source, expected).await);
    state.clear(&mut prepared_direct_1).await;
    drop(prepared_direct_1);
    assert_eq!(state.inner.metadata_bytes.available_permits(), charged);
    assert_eq!(
        state.inner.global_metadata_bytes.available_permits(),
        charged
            + route_body_bytes()
            + retirement::Registry::incarnation_bytes()
            + 2 * ControlIdentity::body_bytes()
    );
    drop(record);
    assert_eq!(
        state.inner.global_metadata_bytes.available_permits(),
        charged
            + route_body_bytes()
            + record_copy_bytes()
            + retirement::Registry::incarnation_bytes()
            + 2 * ControlIdentity::body_bytes()
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
    let (state, charged, peak) = configured_with_discovery(source.clone(), discovery, 2);
    let mut prepared_direct_0 = state.prepare_cutover::<1>().unwrap();
    let mut prepared_direct_1 = state.prepare_cutover::<1>().unwrap();

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
        node_bytes + publication::Life::bytes() + publication::TicketOwner::body_bytes()
    );
    assert_eq!(
        state.inner.global_metadata_bytes.available_permits(),
        node_bytes
            + record_copy_bytes()
            + route_body_bytes()
            + publication::Life::bytes()
            + publication::TicketOwner::body_bytes()
            + discovery
    );
    state.drain_native(OBSERVE).await.unwrap();
    assert_eq!(
        state.inner.metadata_bytes.available_permits(),
        charged - backing
    );
    assert_eq!(
        state.inner.global_metadata_bytes.available_permits(),
        charged - backing
            + record_copy_bytes()
            + route_body_bytes()
            + discovery
            + retirement::Registry::incarnation_bytes()
    );
    assert!(source.handle().shutdown_status().is_ok());
    assert!(recovered(&source, expected).await);
    state.notify_shutdown(&mut prepared_direct_0);
    drop(prepared_direct_0);
    state.clear(&mut prepared_direct_1).await;
    drop(prepared_direct_1);
    assert_eq!(state.inner.metadata_bytes.available_permits(), charged);
}

#[tokio::test]
async fn global_metadata_rejected_native_publication_keeps_snapshot_charge_and_joins_owner() {
    let expected = baseline();
    let source = LocalSource::new();
    let (mut state, charged, old_peak) = configured(source.clone(), 0);
    drop(old_peak);
    let temporary = source_future_bytes(&*source) + ownership::Ownership::attempt_bytes();
    let devices = vec![source::UsbDevice {
        udid: DEVICE.into(),
        device_id: 7,
        usb: true,
    }];
    let snapshot_size =
        devices.capacity() * std::mem::size_of::<source::UsbDevice>() + devices[0].udid.capacity();
    let total = charged + snapshot_size + route_body_bytes() - 1;
    let native_bytes = state.inner.native.structural_bytes()
        + reconnect::Ledger::structural_bytes()
        + scan::Outcomes::body_bytes()
        + TunnelAgentOwner::body_bytes()
        + TunnelAgentState::watch_bytes()
        + failure_slot::Counts::body_bytes();
    state.set_global_metadata_limit(
        native_bytes
            + retirement::Registry::incarnation_bytes()
            + (total + Lifecycle::initial_metadata_bytes(DEVICE))
            + temporary
            + 6 * ControlIdentity::body_bytes(),
    );
    let mut terminal_inputs = PreparedTerminal::new(&state);
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
        charged - backing + route_body_bytes() - 1 + retirement::Registry::incarnation_bytes()
    );
    drop(snapshot);
    assert_eq!(
        state.inner.global_metadata_bytes.available_permits(),
        total - backing + retirement::Registry::incarnation_bytes()
    );
    terminal_inputs.shutdown(&state);
    terminal_inputs.clear(&state).await;
    drop(terminal_inputs);
}
