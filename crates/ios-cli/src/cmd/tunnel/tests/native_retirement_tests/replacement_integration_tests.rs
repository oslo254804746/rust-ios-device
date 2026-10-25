//! End-to-end integration tests for Package 2 exact old-row recovery and replacement.
use super::*;
use ios_core::device::PreparedRsdOwner;
use ios_core::test_support::{
    prepare_scripted_tunnel_stream_at, scripted_tunnel_peer, ScriptedTunnelReport,
};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

const CLOCK: Duration = Duration::from_secs(8);

struct ReplacementSource {
    handles: StdMutex<Vec<Arc<TunnelHandle>>>,
    currents: StdMutex<Vec<PreparedRsdOwner>>,
    peers: StdMutex<Vec<tokio::task::JoinHandle<ScriptedTunnelReport>>>,
    refresh_fails: AtomicBool,
    candidate_fails: AtomicBool,
    prepare_count: AtomicUsize,
    usb: StdMutex<Vec<source::UsbDevice>>,
}

impl Default for ReplacementSource {
    fn default() -> Self {
        Self {
            handles: StdMutex::new(Vec::new()),
            currents: StdMutex::new(Vec::new()),
            peers: StdMutex::new(Vec::new()),
            refresh_fails: AtomicBool::new(false),
            candidate_fails: AtomicBool::new(false),
            prepare_count: AtomicUsize::new(0),
            usb: StdMutex::new(vec![source::UsbDevice {
                udid: DEVICE.into(),
                device_id: 0,
                usb: true,
            }]),
        }
    }
}

impl ReplacementSource {
    fn handle(&self, index: usize) -> Arc<TunnelHandle> {
        self.handles.lock().unwrap()[index].clone()
    }
}

impl Source for ReplacementSource {
    fn observes_freshness(&self) -> bool {
        true
    }

    fn refresh_existing_rsd<'a>(
        &'a self,
        _route: ios_core::device::ExistingRsdRoute,
        _deadline: tokio::time::Instant,
        bank: &'a Arc<Semaphore>,
    ) -> Result<
        future_storage::Dynamic<'a, Result<ios_core::device::PreparedRsdOwner>>,
    > {
        let should_fail = self.refresh_fails.load(Ordering::SeqCst);
        let current = self.currents.lock().unwrap().first().cloned();
        future_storage::Dynamic::new(
            async move {
                if should_fail {
                    Err(anyhow::anyhow!("scripted RSD refresh probe failed"))
                } else {
                    current.ok_or_else(|| anyhow::anyhow!("no current RSD catalogue"))
                }
            },
            bank,
        )
    }

    fn prepare_owned<'a>(
        &'a self,
        udid: &'a str,
        _route: RouteRef<'a>,
        mode: TunMode,
        owner: &'a dyn TunnelOwner,
        bank: &'a Arc<Semaphore>,
        deadline: Option<&'a lifecycle::ConnectionDeadline>,
    ) -> Result<prepared_source::SourceFuture<'a>> {
        self.prepare_count.fetch_add(1, Ordering::SeqCst);
        if self.candidate_fails.load(Ordering::SeqCst) {
            return Err(anyhow::anyhow!("scripted candidate connection refusal"));
        }
        let prepared = owned_box::Destination::<ios_core::PreparedConnectedDevice>::new(bank)?;
        let output = owned_box::Destination::<ios_core::OwnedConnectedDevice>::new(bank)?;
        Ok(prepared_source::SourceFuture::Native(
            future_storage::Dynamic::new(
                async move {
                    assert_eq!(mode, TunMode::Userspace);
                    assert_eq!(udid, DEVICE);
                    let (stream, peer) = tokio::io::duplex(97);
                    let task = tokio::spawn(scripted_tunnel_peer(peer, DEVICE));
                    self.peers.lock().unwrap().push(task);
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
                    self.handles.lock().unwrap().push(device.tunnel_handle().clone());
                    self.currents.lock().unwrap().push(device.current_rsd_catalogue().clone());
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
        Box::pin(async { Ok(Vec::new()) })
    }
}

fn replacement_state(source: Arc<ReplacementSource>) -> TunnelAgentState {
    TunnelAgentState::with_source(
        TunMode::Userspace,
        AccessControl::trusted_test_local(),
        Upstreams::default(),
        source,
        CLOCK,
    )
}

#[tokio::test]
async fn probe_failure_triggers_exact_replacement_returning_new_paid_record() {
    let source = Arc::new(ReplacementSource::default());
    let state = replacement_state(source.clone());
    let mut terminal = PreparedTerminal::new(&state);
    let bank = state.inner.global_metadata_bytes.clone();

    // 1. Initial publication of tunnel #1.
    let record1 = state.ensure_tunnel(DEVICE, None, None).await.unwrap();
    let handle1 = source.handle(0);
    assert_eq!(source.prepare_count.load(Ordering::SeqCst), 1);
    assert!(handle1.is_alive());

    let (initial_proof, initial_incarnation) = {
        let tunnels = state.inner.tunnels.read().await;
        tunnels.proof(DEVICE).unwrap()
    };

    // 2. Configure probe failure so refresh fails.
    source.refresh_fails.store(true, Ordering::SeqCst);

    // 3. Ensure tunnel again: probe fails -> replaces with candidate #2 in-place.
    let record2 = state.ensure_tunnel(DEVICE, None, None).await.unwrap();
    let handle2 = source.handle(1);
    assert_eq!(source.prepare_count.load(Ordering::SeqCst), 2);
    assert!(handle2.is_alive());
    assert!(!Arc::ptr_eq(&handle1, &handle2));

    // Verify record returned is NEW.
    assert_eq!(record2.udid, record1.udid);

    // Verify publication table still has exactly one entry at the same slot with new incarnation.
    let (new_proof, new_incarnation) = {
        let tunnels = state.inner.tunnels.read().await;
        assert_eq!(tunnels.len(), 1);
        tunnels.proof(DEVICE).unwrap()
    };
    assert_eq!(initial_proof, new_proof, "same table slot was preserved");
    assert!(new_incarnation > initial_incarnation, "incarnation monotonically increased");

    // Old runtime was cleanly retired; new runtime is active.
    assert_eq!(state.inner.native.snapshot().active, 1);

    // Clean up via terminal.
    terminal.shutdown(&state);
    let _ = state.drain_native(CLOCK).await;
    terminal.clear(&state).await;
    assert_eq!(state.inner.tunnels.read().await.len(), 0);

    for peer in source.peers.lock().unwrap().drain(..) {
        peer.abort();
    }
    drop(record1);
    drop(record2);
    drop(handle1);
    drop(handle2);
    drop(source);
    drop(terminal);
    drop(state);
    assert_eq!(bank.available_permits(), MAX_GLOBAL_KNOWN_METADATA_BYTES);
}

#[tokio::test]
async fn natural_runtime_death_triggers_exact_replacement() {
    let source = Arc::new(ReplacementSource::default());
    let state = replacement_state(source.clone());
    let mut terminal = PreparedTerminal::new(&state);
    let bank = state.inner.global_metadata_bytes.clone();

    // 1. Initial publication of tunnel #1.
    let record1 = state.ensure_tunnel(DEVICE, None, None).await.unwrap();
    let handle1 = source.handle(0);
    assert!(handle1.is_alive());

    // 2. Simulate natural death of runtime #1 by aborting its duplex peer.
    source.peers.lock().unwrap()[0].abort();
    // Yield to let EOF propagate to handle1's worker tasks.
    for _ in 0..50 {
        if !handle1.is_alive() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    assert!(!handle1.is_alive());

    // 3. Ensure tunnel: detects ended runtime -> replaces with candidate #2.
    let record2 = state.ensure_tunnel(DEVICE, None, None).await.unwrap();
    let handle2 = source.handle(1);
    assert!(handle2.is_alive());
    assert!(!Arc::ptr_eq(&handle1, &handle2));

    // Table has new tunnel active.
    {
        let tunnels = state.inner.tunnels.read().await;
        assert_eq!(tunnels.len(), 1);
        assert!(tunnels[DEVICE].is_alive());
    }

    terminal.shutdown(&state);
    let _ = state.drain_native(CLOCK).await;
    terminal.clear(&state).await;

    for peer in source.peers.lock().unwrap().drain(..) {
        peer.abort();
    }
    drop(record1);
    drop(record2);
    drop(handle1);
    drop(handle2);
    drop(source);
    drop(terminal);
    drop(state);
    assert_eq!(bank.available_permits(), MAX_GLOBAL_KNOWN_METADATA_BYTES);
}

#[tokio::test]
async fn candidate_failure_during_replacement_preserves_exact_old_row() {
    let source = Arc::new(ReplacementSource::default());
    let state = replacement_state(source.clone());
    let mut terminal = PreparedTerminal::new(&state);
    let bank = state.inner.global_metadata_bytes.clone();

    // 1. Initial publication.
    let record1 = state.ensure_tunnel(DEVICE, None, None).await.unwrap();
    let handle1 = source.handle(0);
    assert!(handle1.is_alive());

    let (initial_proof, initial_incarnation) = {
        let tunnels = state.inner.tunnels.read().await;
        tunnels.proof(DEVICE).unwrap()
    };

    // 2. Both probe and candidate fail.
    source.refresh_fails.store(true, Ordering::SeqCst);
    source.candidate_fails.store(true, Ordering::SeqCst);

    // 3. Ensure tunnel fails because candidate refused.
    let err = state.ensure_tunnel(DEVICE, None, None).await.unwrap_err();
    let message = format!("{err:#}");
    assert!(
        message.contains("scripted RSD refresh probe failed")
            || message.contains("scripted candidate connection refusal")
    );

    // 4. Old row was NOT displaced or lost.
    {
        let tunnels = state.inner.tunnels.read().await;
        assert_eq!(tunnels.len(), 1);
        let (current_proof, current_incarnation) = tunnels.proof(DEVICE).unwrap();
        assert_eq!(initial_proof, current_proof);
        assert_eq!(initial_incarnation, current_incarnation);
        assert!(tunnels[DEVICE].is_alive());
    }

    terminal.shutdown(&state);
    let _ = state.drain_native(CLOCK).await;
    terminal.clear(&state).await;

    for peer in source.peers.lock().unwrap().drain(..) {
        peer.abort();
    }
    drop(record1);
    drop(handle1);
    drop(source);
    drop(terminal);
    drop(state);
    drop(err);
    assert_eq!(bank.available_permits(), MAX_GLOBAL_KNOWN_METADATA_BYTES);
}

#[tokio::test]
async fn connect_limit_of_one_does_not_deadlock_during_replacement() {
    let source = Arc::new(ReplacementSource::default());
    let state = replacement_state(source.clone());
    let mut terminal = PreparedTerminal::new(&state);
    let bank = state.inner.global_metadata_bytes.clone();

    // Exhaust all connects permits except 1.
    let mut drained = Vec::new();
    while state.inner.lifecycle.connects.available_permits() > 1 {
        drained.push(state.inner.lifecycle.connects.try_acquire().unwrap());
    }
    assert_eq!(state.inner.lifecycle.connects.available_permits(), 1);

    // 1. Initial publication succeeds with exactly 1 permit limit.
    let record1 = state.ensure_tunnel(DEVICE, None, None).await.unwrap();
    assert_eq!(state.inner.lifecycle.connects.available_permits(), 1);

    // 2. Probe failure triggers replacement. Probe permit must be dropped before
    // candidate acquires permit, otherwise with max=1 it would deadlock!
    source.refresh_fails.store(true, Ordering::SeqCst);

    // Run ensure_tunnel with timeout to verify no deadlock.
    let timeout = Duration::from_secs(5);
    let record2 = tokio::time::timeout(
        timeout,
        state.ensure_tunnel(DEVICE, None, None),
    )
    .await
    .expect("did not deadlock on single permit limit")
    .unwrap();

    assert_eq!(state.inner.lifecycle.connects.available_permits(), 1);

    // Drop drained permits to allow shutdown.
    drop(drained);

    terminal.shutdown(&state);
    let _ = state.drain_native(CLOCK).await;
    terminal.clear(&state).await;

    for peer in source.peers.lock().unwrap().drain(..) {
        peer.abort();
    }
    drop(record1);
    drop(record2);
    drop(source);
    drop(terminal);
    drop(state);
    assert_eq!(bank.available_permits(), MAX_GLOBAL_KNOWN_METADATA_BYTES);
}

#[test]
fn scan_replaces_probe_failed_tunnel_with_candidate() {
    std::thread::Builder::new()
        .stack_size(16 * 1024 * 1024)
        .spawn(|| {
            tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .unwrap()
                .block_on(async {
                    let source = Arc::new(ReplacementSource::default());
                    let state = replacement_state(source.clone());
                    let mut terminal = PreparedTerminal::new(&state);
                    let bank = state.inner.global_metadata_bytes.clone();

                    // 1. Initial publication of tunnel #1 (alive).
                    let record1 = state.ensure_tunnel(DEVICE, None, None).await.unwrap();
                    let handle1 = source.handle(0);
                    assert!(handle1.is_alive());

                    let (initial_proof, initial_incarnation) = {
                        let tunnels = state.inner.tunnels.read().await;
                        tunnels.proof(DEVICE).unwrap()
                    };

                    // 2. Configure RSD probe failure during scan.
                    source.refresh_fails.store(true, Ordering::SeqCst);

                    // 3. Run scan refresh_once: RSD probe fails, session retains the error, scan replaces in-place with candidate.
                    let err = state.refresh_once().await.unwrap_err();
                    assert!(format!("{err:#}").contains("scripted RSD refresh probe failed"));
                    let handle2 = source.handle(1);
                    assert!(handle2.is_alive());
                    assert!(!Arc::ptr_eq(&handle1, &handle2));

                    // Verify publication table still has exactly one entry at the same slot with new incarnation.
                    let (new_proof, new_incarnation) = {
                        let tunnels = state.inner.tunnels.read().await;
                        assert_eq!(tunnels.len(), 1);
                        tunnels.proof(DEVICE).unwrap()
                    };
                    assert_eq!(initial_proof, new_proof, "same table slot was preserved");
                    assert!(new_incarnation > initial_incarnation, "incarnation monotonically increased");

                    terminal.shutdown(&state);
                    let _ = state.drain_native(CLOCK).await;
                    terminal.clear(&state).await;

                    for peer in source.peers.lock().unwrap().drain(..) {
                        peer.abort();
                    }
                    drop(record1);
                    drop(handle1);
                    drop(handle2);
                    drop(source);
                    drop(terminal);
                    drop(state);
                    drop(err);
                    assert_eq!(bank.available_permits(), MAX_GLOBAL_KNOWN_METADATA_BYTES);
                });
        })
        .unwrap()
        .join()
        .unwrap();
}

#[tokio::test]
async fn epoch_mismatch_during_replacement_rejects_and_rolls_back() {
    let source = Arc::new(ReplacementSource::default());
    let state = replacement_state(source.clone());
    let mut terminal = PreparedTerminal::new(&state);
    let bank = state.inner.global_metadata_bytes.clone();

    // 1. Initial publication.
    let record1 = state.ensure_tunnel(DEVICE, None, None).await.unwrap();
    let handle1 = source.handle(0);
    assert!(handle1.is_alive());

    // 2. Probe fails to trigger replacement.
    source.refresh_fails.store(true, Ordering::SeqCst);

    // 3. Before publication write lock, clear/bump lifecycle epoch to invalidate ticket.
    let inner = state.inner.clone();
    let mut cutover = state.prepare_cutover::<2>().unwrap();
    let _hook = record_owner::on_before_admission(move || {
        inner.lifecycle.clear(&mut cutover);
    });

    // 4. Ensure tunnel must reject replacement.
    let err = state.ensure_tunnel(DEVICE, None, None).await.unwrap_err();
    assert!(err.is::<ios_core::OperationCancelled>());

    terminal.shutdown(&state);
    let _ = state.drain_native(CLOCK).await;
    terminal.clear(&state).await;

    for peer in source.peers.lock().unwrap().drain(..) {
        peer.abort();
    }
    drop(record1);
    drop(handle1);
    drop(source);
    drop(terminal);
    drop(state);
    drop(err);
    assert_eq!(bank.available_permits(), MAX_GLOBAL_KNOWN_METADATA_BYTES);
}
