use super::*;
use platform::local::{LocalDevice, LocalStep};
use std::net::Ipv6Addr;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

fn make_ipv6_packet(src: Ipv6Addr, dst: Ipv6Addr, payload: &[u8]) -> Vec<u8> {
    let mut pkt = Vec::with_capacity(40 + payload.len());
    pkt.extend_from_slice(&[0x60, 0x00, 0x00, 0x00]);
    pkt.extend_from_slice(&(payload.len() as u16).to_be_bytes());
    pkt.push(58);
    pkt.push(64);
    pkt.extend_from_slice(&src.octets());
    pkt.extend_from_slice(&dst.octets());
    pkt.extend_from_slice(payload);
    pkt
}

const TOTAL: usize = 32 * 1024 * 1024;
const OBSERVE: Duration = Duration::from_secs(3);

#[derive(Debug)]
struct OriginalIo(Arc<io::Error>);
impl std::fmt::Display for OriginalIo {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}
impl Error for OriginalIo {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        Some(self.0.as_ref())
    }
}

fn plan() -> Plan {
    Plan {
        client: "fd00::2".parse().unwrap(),
        server: "fd00::1".parse().unwrap(),
        mtu: 1280,
        prefix: 64,
    }
}

#[tokio::test]
async fn actual_local_refusal_is_primary_before_stop_and_all_real_joins_at_closed_zero() {
    let socket = tokio::net::TcpSocket::new_v4().unwrap();
    socket.bind("127.0.0.1:0".parse().unwrap()).unwrap();
    let actual = tokio::net::TcpStream::connect(socket.local_addr().unwrap())
        .await
        .unwrap_err();
    assert_eq!(actual.kind(), io::ErrorKind::ConnectionRefused);
    let original = Arc::new(actual);
    let factory_error = NativeError::Native(io::Error::other(OriginalIo(original.clone())));
    let bank = Arc::new(Semaphore::new(TOTAL));
    let (stream, mut peer) = tokio::io::duplex(1280);
    let (engine, launch) = UserspaceTunDevice::prepare_private_validated_with_bank(
        "fd00::2",
        "fd00::1",
        super::super::ValidatedMtu::new(1280).unwrap(),
        stream,
        bank.clone(),
        1,
        true,
    )
    .await
    .unwrap();
    launch().unwrap();
    // Only this explicit failed native operation is injected. Neither a real
    // TUN/utun constructor nor any SDK/default Source future is invoked.
    let native = KernelRuntime::prepare_with(
        &engine,
        bank.clone(),
        plan(),
        Instant::now() + OBSERVE,
        move |_| Err(factory_error),
    )
    .unwrap();
    let port = engine.quiesce_and_flush().await.unwrap();
    let held = bank
        .clone()
        .try_acquire_many_owned(bank.available_permits() as u32)
        .unwrap();
    bank.close();
    let error = native.activate(port).await.unwrap_err();
    let TunnelError::Phases(report) = error else {
        panic!("actual native failure must retain phase evidence")
    };
    let shared = report.primary_shared().unwrap();
    let mut source: &(dyn Error + 'static) = shared.as_ref();
    loop {
        if let Some(io) = source.downcast_ref::<io::Error>() {
            if io.kind() == io::ErrorKind::ConnectionRefused {
                assert!(std::ptr::eq(io, original.as_ref()));
                break;
            }
        }
        source = source
            .source()
            .expect("original refused IO remains a deep source");
    }
    tokio::join!(native.join_work(), engine.collect_original());
    tokio::time::timeout(OBSERVE, native.undo_at(Instant::now() + OBSERVE))
        .await
        .unwrap();
    assert!(native.is_terminal());
    let final_report = native
        .control
        .hub
        .status(
            engine.ledger(),
            Some(native.native_ledger()),
            Some(native.undo_ledger()),
        )
        .unwrap_err();
    assert!(final_report.joins_complete());
    engine.release_original();
    native.release_flows();
    assert_eq!(
        tokio::time::timeout(OBSERVE, peer.read(&mut [0; 1]))
            .await
            .unwrap()
            .unwrap(),
        0
    );
    drop((final_report, report, native, engine, original, socket, held));
    assert_eq!(bank.available_permits(), TOTAL);
}

#[tokio::test]
async fn expired_cleanup_clock_does_not_consume_prepaid_call_then_live_clock_joins_it_at_closed_zero(
) {
    let bank = Arc::new(Semaphore::new(TOTAL));
    let (stream, mut peer) = tokio::io::duplex(1280);
    let (engine, launch) = UserspaceTunDevice::prepare_private_validated_with_bank(
        "fd00::2",
        "fd00::1",
        super::super::ValidatedMtu::new(1280).unwrap(),
        stream,
        bank.clone(),
        1,
        true,
    )
    .await
    .unwrap();
    launch().unwrap();
    let calls = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let observed = calls.clone();
    let native = KernelRuntime::prepare_with(
        &engine,
        bank.clone(),
        plan(),
        Instant::now() + OBSERVE,
        move |_| {
            observed.fetch_add(1, Ordering::SeqCst);
            Err(NativeError::Conflict("unpolled native constructor trap"))
        },
    )
    .unwrap();
    let held = bank
        .clone()
        .try_acquire_many_owned(bank.available_permits() as u32)
        .unwrap();
    bank.close();
    native.request_stop();
    engine.request_stop();
    tokio::join!(native.join_work(), engine.collect_original());
    native
        .undo_at(Instant::now() - Duration::from_millis(1))
        .await;
    assert!(!native.undo.installed());
    assert!(!native.is_terminal());
    assert!(native.launch.lock().unwrap().undo.is_some());
    tokio::time::timeout(OBSERVE, native.undo_at(Instant::now() + OBSERVE))
        .await
        .unwrap();
    assert!(native.undo.installed() && native.undo.is_finished());
    assert!(native.is_terminal());
    assert_eq!(calls.load(Ordering::SeqCst), 0);
    engine.release_original();
    native.release_flows();
    assert_eq!(
        tokio::time::timeout(OBSERVE, peer.read(&mut [0; 1]))
            .await
            .unwrap()
            .unwrap(),
        0
    );
    drop((native, engine, held));
    assert_eq!(bank.available_permits(), TOTAL);
}

#[tokio::test]
async fn renewed_clock_keeps_the_same_actual_blocking_join_after_real_elapsed_at_closed_zero() {
    let bank = Arc::new(Semaphore::new(TOTAL));
    let clock = RuntimeRoot::new(CleanupClock::new(), Some(&bank)).unwrap();
    let slot = RuntimeRoot::new(BlockingSlot::new(), Some(&bank)).unwrap();
    let ledger = Ledger::with_metadata(Some(&bank)).unwrap();
    let entered = Arc::new(Notify::new());
    let body_clock = clock.clone();
    let body_entered = entered.clone();
    let prepared = PreparedBlocking::prepare(&bank, || {
        move || {
            let first = body_clock.live();
            body_entered.notify_one();
            while Instant::now() < first {
                body_clock.retry(first);
            }
            let renewed = body_clock.live();
            assert!(renewed > first);
            Ok(())
        }
    })
    .unwrap();
    let held = bank
        .clone()
        .try_acquire_many_owned(bank.available_permits() as u32)
        .unwrap();
    bank.close();
    let first = Instant::now() + Duration::from_millis(40);
    clock.set(first);
    prepared.start(&slot, ParentRole::Smoltcp, ledger.clone());
    entered.notified().await;
    let original_elapsed = tokio::time::timeout_at(first, slot.join())
        .await
        .unwrap_err();
    assert!(!slot.is_finished());
    assert!(slot.installed());
    // This updates only the same concrete mailbox and wakes the same original
    // blocking body. No F, handle, slot or response backing is allocated.
    clock.set(Instant::now() + OBSERVE);
    tokio::time::timeout(OBSERVE, slot.join()).await.unwrap();
    assert!(slot.is_finished());
    ledger.result(None).unwrap();
    drop((original_elapsed, clock, slot, ledger, entered, held));
    assert_eq!(bank.available_permits(), TOTAL);
}

#[tokio::test]
async fn successful_activation_and_bidirectional_packet_transfer() {
    let bank = Arc::new(Semaphore::new(TOTAL));
    let (stream, mut peer_stream) = tokio::io::duplex(1280);
    let (engine, launch) = UserspaceTunDevice::prepare_private_validated_with_bank(
        "fd00::2",
        "fd00::1",
        super::super::ValidatedMtu::new(1280).unwrap(),
        stream,
        bank.clone(),
        1,
        true,
    )
    .await
    .unwrap();
    launch().unwrap();

    let (device, handle) = LocalDevice::create(plan()).unwrap();
    let native = KernelRuntime::prepare_with(
        &engine,
        bank.clone(),
        plan(),
        Instant::now() + OBSERVE,
        move |_| Ok(Device::from_local(device)),
    )
    .unwrap();

    let port = engine.quiesce_and_flush().await.unwrap();
    native.activate(port).await.unwrap();
    assert!(native.ready());
    assert!(native.is_alive());

    let peer = handle.peer.try_clone().unwrap();
    peer.set_nonblocking(true).unwrap();
    let local_peer = tokio::net::UnixDatagram::from_std(peer).unwrap();

    // 1. Packet from iOS device to Host:
    let payload_to_host = b"test packet to host";
    let pkt_to_host = make_ipv6_packet(
        "fd00::1".parse().unwrap(),
        "fd00::2".parse().unwrap(),
        payload_to_host,
    );
    peer_stream.write_all(&pkt_to_host).await.unwrap();

    let mut recv_host = vec![0u8; 1500];
    let n = tokio::time::timeout(OBSERVE, local_peer.recv(&mut recv_host))
        .await
        .expect("receive packet on local native socket timed out")
        .unwrap();
    assert_eq!(&recv_host[..n], &pkt_to_host);

    // 2. Packet from Host to iOS device:
    let payload_to_device = b"test packet to device";
    let pkt_to_device = make_ipv6_packet(
        "fd00::2".parse().unwrap(),
        "fd00::1".parse().unwrap(),
        payload_to_device,
    );
    local_peer.send(&pkt_to_device).await.unwrap();

    let mut recv_device = vec![0u8; pkt_to_device.len()];
    tokio::time::timeout(OBSERVE, peer_stream.read_exact(&mut recv_device))
        .await
        .expect("read packet on peer stream timed out")
        .unwrap();
    assert_eq!(&recv_device, &pkt_to_device);

    // Orderly teardown:
    native.request_stop();
    engine.request_stop();
    tokio::join!(native.join_work(), engine.collect_original());
    tokio::time::timeout(OBSERVE, native.undo_at(Instant::now() + OBSERVE))
        .await
        .unwrap();
    assert!(native.is_terminal());

    native
        .control
        .hub
        .status(
            engine.ledger(),
            Some(native.native_ledger()),
            Some(native.undo_ledger()),
        )
        .unwrap();

    engine.release_original();
    native.release_flows();

    assert_eq!(
        tokio::time::timeout(OBSERVE, peer_stream.read(&mut [0; 1]))
            .await
            .unwrap()
            .unwrap(),
        0
    );

    let steps = handle.executed_steps();
    assert!(steps.contains(&LocalStep::ResolveIdentity));
    assert!(steps.contains(&LocalStep::MakeInert));
    assert!(steps.contains(&LocalStep::InspectInert));
    assert!(steps.contains(&LocalStep::ConfigureMtu));
    assert!(steps.contains(&LocalStep::PreparePacketIo));
    assert!(steps.contains(&LocalStep::StartSession));
    assert!(steps.contains(&LocalStep::AddAddress));
    assert!(steps.contains(&LocalStep::AddRoute));
    assert!(steps.contains(&LocalStep::AddressReady));
    assert!(steps.contains(&LocalStep::UndoRoute));
    assert!(steps.contains(&LocalStep::UndoAddress));
    assert!(steps.contains(&LocalStep::UndoSession));
    assert!(steps.contains(&LocalStep::Close));

    drop((native, engine, local_peer, handle));
    assert_eq!(bank.available_permits(), TOTAL);
}

#[tokio::test]
async fn init_boundary_failure_and_cancellation_unwinds() {
    for fail_step in [
        LocalStep::ConfigureMtu,
        LocalStep::AddAddress,
        LocalStep::AddRoute,
    ] {
        let bank = Arc::new(Semaphore::new(TOTAL));
        let (stream, mut peer_stream) = tokio::io::duplex(1280);
        let (engine, launch) = UserspaceTunDevice::prepare_private_validated_with_bank(
            "fd00::2",
            "fd00::1",
            super::super::ValidatedMtu::new(1280).unwrap(),
            stream,
            bank.clone(),
            1,
            true,
        )
        .await
        .unwrap();
        launch().unwrap();

        let (device, handle) = LocalDevice::create(plan()).unwrap();
        handle.set_fail_at(fail_step, NativeError::Conflict("scripted init failure"));

        let native = KernelRuntime::prepare_with(
            &engine,
            bank.clone(),
            plan(),
            Instant::now() + OBSERVE,
            move |_| Ok(Device::from_local(device)),
        )
        .unwrap();

        let port = engine.quiesce_and_flush().await.unwrap();
        let err = native.activate(port).await.unwrap_err();
        let TunnelError::Phases(report) = err else {
            panic!("init failure must report phase outcomes");
        };
        let shared = report.primary_shared().unwrap();
        assert!(format!("{shared}").contains("scripted init failure"));

        tokio::join!(native.join_work(), engine.collect_original());
        tokio::time::timeout(OBSERVE, native.undo_at(Instant::now() + OBSERVE))
            .await
            .unwrap();
        assert!(native.is_terminal());

        engine.release_original();
        native.release_flows();

        assert_eq!(
            tokio::time::timeout(OBSERVE, peer_stream.read(&mut [0; 1]))
                .await
                .unwrap()
                .unwrap(),
            0
        );

        let steps = handle.executed_steps();
        assert!(steps.contains(&LocalStep::Close));
        drop((native, engine, report));
        assert_eq!(bank.available_permits(), TOTAL);
    }
}

#[tokio::test]
async fn address_dad_tentative_blocks_readiness_and_conflict_fails() {
    // Part A: Tentative initially, then becomes Preferred
    {
        let bank = Arc::new(Semaphore::new(TOTAL));
        let (stream, mut peer_stream) = tokio::io::duplex(1280);
        let (engine, launch) = UserspaceTunDevice::prepare_private_validated_with_bank(
            "fd00::2",
            "fd00::1",
            super::super::ValidatedMtu::new(1280).unwrap(),
            stream,
            bank.clone(),
            1,
            true,
        )
        .await
        .unwrap();
        launch().unwrap();

        let (device, handle) = LocalDevice::create(plan()).unwrap();
        handle.set_address_readiness(AddressReadiness::Tentative);

        let native = KernelRuntime::prepare_with(
            &engine,
            bank.clone(),
            plan(),
            Instant::now() + OBSERVE,
            move |_| Ok(Device::from_local(device)),
        )
        .unwrap();

        let port = engine.quiesce_and_flush().await.unwrap();
        let hook_handle = handle.hook.clone();
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(50)).await;
            hook_handle.lock().unwrap().address_readiness = Some(AddressReadiness::Preferred);
        });

        native.activate(port).await.unwrap();
        assert!(native.ready());

        native.request_stop();
        engine.request_stop();
        tokio::join!(native.join_work(), engine.collect_original());
        native.undo_at(Instant::now() + OBSERVE).await;
        assert!(native.is_terminal());

        engine.release_original();
        native.release_flows();
        assert_eq!(
            tokio::time::timeout(OBSERVE, peer_stream.read(&mut [0; 1]))
                .await
                .unwrap()
                .unwrap(),
            0
        );
        drop((native, engine));
        assert_eq!(bank.available_permits(), TOTAL);
    }

    // Part B: Conflict fails immediately
    {
        let bank = Arc::new(Semaphore::new(TOTAL));
        let (stream, mut peer_stream) = tokio::io::duplex(1280);
        let (engine, launch) = UserspaceTunDevice::prepare_private_validated_with_bank(
            "fd00::2",
            "fd00::1",
            super::super::ValidatedMtu::new(1280).unwrap(),
            stream,
            bank.clone(),
            1,
            true,
        )
        .await
        .unwrap();
        launch().unwrap();

        let (device, handle) = LocalDevice::create(plan()).unwrap();
        handle.set_address_readiness(AddressReadiness::Conflict);

        let native = KernelRuntime::prepare_with(
            &engine,
            bank.clone(),
            plan(),
            Instant::now() + OBSERVE,
            move |_| Ok(Device::from_local(device)),
        )
        .unwrap();

        let port = engine.quiesce_and_flush().await.unwrap();
        let err = native.activate(port).await.unwrap_err();
        let TunnelError::Phases(report) = err else {
            panic!("conflict must report phase outcomes");
        };
        let shared = report.primary_shared().unwrap();
        assert!(format!("{shared}").contains("DAD/conflicting"));

        tokio::join!(native.join_work(), engine.collect_original());
        native.undo_at(Instant::now() + OBSERVE).await;
        assert!(native.is_terminal());

        engine.release_original();
        native.release_flows();
        assert_eq!(
            tokio::time::timeout(OBSERVE, peer_stream.read(&mut [0; 1]))
                .await
                .unwrap()
                .unwrap(),
            0
        );
        drop((native, engine, report));
        assert_eq!(bank.available_permits(), TOTAL);
    }
}

#[tokio::test]
async fn packet_backpressure_and_strict_eof_under_full_pipe() {
    let bank = Arc::new(Semaphore::new(TOTAL));
    let (stream, peer_stream) = tokio::io::duplex(1280);
    let (engine, launch) = UserspaceTunDevice::prepare_private_validated_with_bank(
        "fd00::2",
        "fd00::1",
        super::super::ValidatedMtu::new(1280).unwrap(),
        stream,
        bank.clone(),
        1,
        true,
    )
    .await
    .unwrap();
    launch().unwrap();

    let (device, handle) = LocalDevice::create(plan()).unwrap();
    let native = KernelRuntime::prepare_with(
        &engine,
        bank.clone(),
        plan(),
        Instant::now() + OBSERVE,
        move |_| Ok(Device::from_local(device)),
    )
    .unwrap();

    let port = engine.quiesce_and_flush().await.unwrap();
    native.activate(port).await.unwrap();
    assert!(native.ready());

    // Drop peer stream to trigger EOF on reader:
    drop(peer_stream);

    // Bridge and transport detect EOF and signal stop:
    tokio::time::timeout(OBSERVE, async {
        tokio::join!(native.join_work(), engine.collect_original())
    })
    .await
    .unwrap();

    native.undo_at(Instant::now() + OBSERVE).await;
    assert!(native.is_terminal());

    engine.release_original();
    native.release_flows();
    drop((native, engine, handle));
    assert_eq!(bank.available_permits(), TOTAL);
}

#[tokio::test]
async fn stop_signal_and_orderly_reverse_join() {
    let bank = Arc::new(Semaphore::new(TOTAL));
    let (stream, mut peer_stream) = tokio::io::duplex(1280);
    let (engine, launch) = UserspaceTunDevice::prepare_private_validated_with_bank(
        "fd00::2",
        "fd00::1",
        super::super::ValidatedMtu::new(1280).unwrap(),
        stream,
        bank.clone(),
        1,
        true,
    )
    .await
    .unwrap();
    launch().unwrap();

    let (device, handle) = LocalDevice::create(plan()).unwrap();
    let native = KernelRuntime::prepare_with(
        &engine,
        bank.clone(),
        plan(),
        Instant::now() + OBSERVE,
        move |_| Ok(Device::from_local(device)),
    )
    .unwrap();

    let port = engine.quiesce_and_flush().await.unwrap();
    native.activate(port).await.unwrap();
    assert!(native.ready());

    // Explicitly call request_stop()
    native.request_stop();
    engine.request_stop();

    // Verify native workers join before undo_at runs
    tokio::time::timeout(OBSERVE, async {
        tokio::join!(native.join_work(), engine.collect_original())
    })
    .await
    .unwrap();

    assert!(native.control.work_joined.load(Ordering::Acquire));
    assert!(!native.control.undo_joined.load(Ordering::Acquire));

    // Now run undo_at
    native.undo_at(Instant::now() + OBSERVE).await;
    assert!(native.control.undo_joined.load(Ordering::Acquire));
    assert!(native.is_terminal());

    let steps = handle.executed_steps();
    let undo_route_idx = steps
        .iter()
        .position(|s| *s == LocalStep::UndoRoute)
        .unwrap();
    let undo_addr_idx = steps
        .iter()
        .position(|s| *s == LocalStep::UndoAddress)
        .unwrap();
    let undo_session_idx = steps
        .iter()
        .position(|s| *s == LocalStep::UndoSession)
        .unwrap();
    let close_idx = steps.iter().position(|s| *s == LocalStep::Close).unwrap();

    // Strict reverse order: UndoRoute -> UndoAddress -> UndoSession -> Close
    assert!(undo_route_idx < undo_addr_idx);
    assert!(undo_addr_idx < undo_session_idx);
    assert!(undo_session_idx < close_idx);

    engine.release_original();
    native.release_flows();

    assert_eq!(
        tokio::time::timeout(OBSERVE, peer_stream.read(&mut [0; 1]))
            .await
            .unwrap()
            .unwrap(),
        0
    );

    drop((native, engine, handle));
    assert_eq!(bank.available_permits(), TOTAL);
}
