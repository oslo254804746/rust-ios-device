//! Actual Core prepared candidates from an explicit scripted authentication
//! boundary. No default NativeSource / device constructor is ever polled.
use super::*;
use ios_core::device::PreparedRsdOwner;
use ios_core::test_support::{
    prepare_scripted_tunnel_stream_at, scripted_tunnel_peer, ScriptedTunnelReport,
};
use std::sync::atomic::AtomicBool;

const CLOCK: Duration = Duration::from_secs(8);

#[derive(Default)]
struct ScriptedSource {
    handle: StdMutex<Option<Arc<TunnelHandle>>>,
    current: StdMutex<Option<PreparedRsdOwner>>,
    initial: StdMutex<Option<ios_core::OwnedRsdHandshake>>,
    peer: StdMutex<Option<tokio::task::JoinHandle<ScriptedTunnelReport>>>,
    held: StdMutex<Option<tokio::sync::OwnedSemaphorePermit>>,
    usb: StdMutex<Vec<source::UsbDevice>>,
}
impl ScriptedSource {
    fn handle(&self) -> Arc<TunnelHandle> {
        self.handle.lock().unwrap().as_ref().unwrap().clone()
    }
}
impl Source for ScriptedSource {
    fn prepare_owned<'a>(
        &'a self,
        udid: &'a str,
        _route: RouteRef<'a>,
        mode: TunMode,
        owner: &'a dyn TunnelOwner,
        bank: &'a Arc<Semaphore>,
        deadline: Option<&'a lifecycle::ConnectionDeadline>,
    ) -> Result<prepared_source::SourceFuture<'a>> {
        let prepared = owned_box::Destination::<ios_core::PreparedConnectedDevice>::new(bank)?;
        let output = owned_box::Destination::<ios_core::OwnedConnectedDevice>::new(bank)?;
        Ok(prepared_source::SourceFuture::Native(
            future_storage::Dynamic::new(
                async move {
                    assert_eq!(mode, TunMode::Userspace);
                    assert_eq!(udid, DEVICE);
                    let (stream, peer) = tokio::io::duplex(97);
                    let task = tokio::spawn(scripted_tunnel_peer(peer, DEVICE));
                    assert!(self.peer.lock().unwrap().replace(task).is_none());
                    let device = prepare_scripted_tunnel_stream_at(
                        ios_core::DeviceInfo {
                            udid: udid.into(),
                            device_id: 0,
                            connection_type: "Network".into(),
                            product_id: 0,
                        },
                        stream,
                        owner,
                        bank,
                        deadline.expect("same original caller clock").absolute_at(),
                    )
                    .await?;
                    assert!(!device.tunnel_handle().ready_for_publication());
                    *self.handle.lock().unwrap() = Some(device.tunnel_handle().clone());
                    *self.current.lock().unwrap() = Some(device.current_rsd_catalogue().clone());
                    *self.initial.lock().unwrap() = device.rsd().cloned();
                    Ok(prepared_source::Candidate::Native {
                        device: prepared.place(device),
                        output,
                    })
                },
                bank,
            )?,
        ))
    }
    fn connect<'a>(
        &'a self,
        _: &'a str,
        _: Route,
        _: TunMode,
    ) -> BoxFuture<'a, Result<ManagedDevice>> {
        panic!("scripted Source requires the actual Core preparation path")
    }
    fn usb_devices(&self) -> BoxFuture<'_, UsbSnapshot> {
        let list = self.usb.lock().unwrap().clone();
        Box::pin(async move { UsbSnapshot::Devices(list) })
    }
    fn paired_devices(&self) -> BoxFuture<'_, Result<Vec<ios_core::device::PairedMobdev2Device>>> {
        panic!("the explicit fixture never browses devices")
    }
}

fn scripted_state(source: Arc<ScriptedSource>) -> TunnelAgentState {
    TunnelAgentState::with_source(
        TunMode::Userspace,
        AccessControl::trusted_test_local(),
        Upstreams::default(),
        source,
        CLOCK,
    )
}
async fn connect_prepared(state: &TunnelAgentState) -> Result<PaidRecord> {
    let ticket = state.inner.lifecycle.ticket(DEVICE).await?;
    let deadline = lifecycle::ConnectionDeadline::new(CLOCK)?;
    let _serial = ticket.serial().await;
    ticket
        .run(deadline.run(state.connect_and_store(
            DEVICE,
            Route::Wifi(None),
            &ticket,
            None,
            Some(&deadline),
        )))
        .await
}
async fn collect(
    state: &TunnelAgentState,
    source: &ScriptedSource,
    terminal: &mut PreparedTerminal,
    expected: usize,
) {
    terminal.shutdown(state);
    state.drain_native(CLOCK).await.unwrap();
    terminal.clear(state).await;
    let peer = source.peer.lock().unwrap().take().unwrap();
    let report = tokio::time::timeout(CLOCK, peer).await.unwrap().unwrap();
    assert!(report.literal_eof);
    assert!(report
        .connections
        .iter()
        .all(|connection| connection.established && connection.uuid.is_some()));
    assert!(source.handle().shutdown_status().is_ok());
    assert!(state.inner.native.snapshot().closed_and_empty());
    drop(source.handle.lock().unwrap().take());
    drop(source.current.lock().unwrap().take());
    drop(source.initial.lock().unwrap().take());
    drop(source.held.lock().unwrap().take());
    tokio::time::timeout(CLOCK, async {
        while baseline() != expected {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn prepared_cli_stages_actual_directory_record_and_lease_before_same_listener_publication() {
    let expected = baseline();
    let source = Arc::new(ScriptedSource::default());
    let state = scripted_state(source.clone());
    let bank = state.inner.global_metadata_bytes.clone();
    let mut terminal = PreparedTerminal::new(&state);
    let staged = Arc::new(AtomicBool::new(false));
    let seen = staged.clone();
    let captured = source.clone();
    let _hook = record_owner::on_before_admission(move || {
        assert!(!captured.handle().ready_for_publication());
        let current = captured.current.lock().unwrap();
        assert_eq!(
            current.as_ref().unwrap().service_port("wire.checked.only"),
            Some(23456)
        );
        assert_eq!(
            captured
                .initial
                .lock()
                .unwrap()
                .as_ref()
                .unwrap()
                .get_port("wire.initial.only"),
            Some(12345)
        );
        seen.store(true, Ordering::SeqCst);
    });
    let record = connect_prepared(&state).await.unwrap();
    assert!(staged.load(Ordering::SeqCst));
    {
        let rows = state.inner.tunnels.read().await;
        let row = rows.get(DEVICE).unwrap();
        let handle = source.handle();
        assert!(Arc::ptr_eq(row.device.native_handle().unwrap(), &handle));
        assert!(handle.ready_for_publication());
        let current = source.current.lock().unwrap();
        let current = current.as_ref().unwrap();
        assert!(row
            ._fresh_metadata
            .as_ref()
            .unwrap()
            .catalogue
            .ptr_eq(current));
        assert!(row.device.freshness_catalogue().unwrap().ptr_eq(current));
        assert_eq!(record.userspace_port, handle.userspace_port);
    }
    drop(record);
    collect(&state, &source, &mut terminal, expected).await;
    drop(terminal);
    drop(_hook);
    drop(state);
    drop(source);
    assert_eq!(bank.available_permits(), MAX_GLOBAL_KNOWN_METADATA_BYTES);
}

#[tokio::test]
async fn prepared_cli_actual_record_refusal_keeps_public_gate_closed_and_joins_at_closed_zero() {
    let expected = baseline();
    let source = Arc::new(ScriptedSource::default());
    let state = scripted_state(source.clone());
    let bank = state.inner.global_metadata_bytes.clone();
    let mut terminal = PreparedTerminal::new(&state);
    let captured = source.clone();
    let charged = bank.clone();
    let _hook = record_owner::on_before_admission(move || {
        assert!(!captured.handle().ready_for_publication());
        *captured.held.lock().unwrap() = Some(
            charged
                .clone()
                .try_acquire_many_owned(charged.available_permits() as u32)
                .unwrap(),
        );
        charged.close();
    });
    let error = connect_prepared(&state).await.err().unwrap();
    assert!(error.is::<record_owner::CopyAdmission>());
    assert!(!source.handle().ready_for_publication());
    assert!(state.inner.tunnels.read().await.is_empty());
    assert!(bank.is_closed());
    drop(error);
    collect(&state, &source, &mut terminal, expected).await;
    drop(terminal);
    drop(_hook);
    drop(state);
    drop(source);
    assert_eq!(bank.available_permits(), MAX_GLOBAL_KNOWN_METADATA_BYTES);
}

#[tokio::test]
async fn prepared_cli_same_poll_epoch_clear_before_activation_never_opens_original_listener() {
    let expected = baseline();
    let source = Arc::new(ScriptedSource::default());
    let state = scripted_state(source.clone());
    let bank = state.inner.global_metadata_bytes.clone();
    let mut terminal = PreparedTerminal::new(&state);
    let mut cutover = state.prepare_cutover::<2>().unwrap();
    let inner = state.inner.clone();
    let captured = source.clone();
    let _hook = record_owner::on_before_admission(move || {
        assert!(!captured.handle().ready_for_publication());
        inner.lifecycle.clear(&mut cutover);
    });
    let error = connect_prepared(&state).await.err().unwrap();
    assert_eq!(
        crate::output::operation_error_class(error.as_ref()),
        ios_core::ErrorClass::Cancelled
    );
    assert!(!source.handle().ready_for_publication());
    assert!(state.inner.tunnels.read().await.is_empty());
    drop(error);
    collect(&state, &source, &mut terminal, expected).await;
    drop(terminal);
    drop(_hook);
    drop(state);
    drop(source);
    assert_eq!(bank.available_permits(), MAX_GLOBAL_KNOWN_METADATA_BYTES);
}

#[tokio::test]
async fn prepared_cli_post_activation_usb_detachment_stops_runtime_and_refunds() {
    let expected = baseline();
    let source = Arc::new(ScriptedSource::default());
    *source.usb.lock().unwrap() = vec![source::UsbDevice {
        udid: DEVICE.into(),
        device_id: 0,
        usb: true,
    }];
    let state = scripted_state(source.clone());
    let bank = state.inner.global_metadata_bytes.clone();
    let mut terminal = PreparedTerminal::new(&state);
    let captured = source.clone();
    let _hook = record_owner::on_before_admission(move || {
        assert!(!captured.handle().ready_for_publication());
        captured.usb.lock().unwrap().clear();
    });
    let ticket = state.inner.lifecycle.ticket(DEVICE).await.unwrap();
    let deadline = lifecycle::ConnectionDeadline::new(CLOCK).unwrap();
    let _serial = ticket.serial().await;
    let error = ticket
        .run(deadline.run(state.connect_and_store(
            DEVICE,
            Route::Usbmux,
            &ticket,
            None,
            Some(&deadline),
        )))
        .await
        .err()
        .unwrap();
    assert!(error.to_string().contains("tunnel attachment absent before publication"));
    assert!(!source.handle().ready_for_publication());
    assert!(state.inner.tunnels.read().await.is_empty());
    drop(error);
    drop(_serial);
    drop(ticket);
    drop(deadline);
    collect(&state, &source, &mut terminal, expected).await;
    drop(terminal);
    drop(_hook);
    drop(state);
    drop(source);
    assert_eq!(bank.available_permits(), MAX_GLOBAL_KNOWN_METADATA_BYTES);
}

#[tokio::test]
async fn prepared_cli_deadline_clamp_before_activation_aborts_and_cleans_up() {
    let expected = baseline();
    let source = Arc::new(ScriptedSource::default());
    let state = scripted_state(source.clone());
    let bank = state.inner.global_metadata_bytes.clone();
    let mut terminal = PreparedTerminal::new(&state);
    let ticket = state.inner.lifecycle.ticket(DEVICE).await.unwrap();
    let deadline = Arc::new(lifecycle::ConnectionDeadline::new(CLOCK).unwrap());
    let _serial = ticket.serial().await;
    let deadline_ref = deadline.clone();
    let captured = source.clone();
    let _hook = record_owner::on_before_admission(move || {
        assert!(!captured.handle().ready_for_publication());
        deadline_ref.clamp_retry(tokio::time::Instant::now() - Duration::from_millis(10));
    });
    let error = ticket
        .run(deadline.run(state.connect_and_store(
            DEVICE,
            Route::Wifi(None),
            &ticket,
            None,
            Some(&*deadline),
        )))
        .await
        .err()
        .unwrap();
    assert_eq!(
        crate::output::operation_error_class(error.as_ref()),
        ios_core::ErrorClass::Timeout
    );
    assert!(!source.handle().ready_for_publication());
    assert!(state.inner.tunnels.read().await.is_empty());
    drop(error);
    drop(_serial);
    drop(ticket);
    drop(deadline);
    collect(&state, &source, &mut terminal, expected).await;
    drop(terminal);
    drop(_hook);
    drop(state);
    drop(source);
    assert_eq!(bank.available_permits(), MAX_GLOBAL_KNOWN_METADATA_BYTES);
}
