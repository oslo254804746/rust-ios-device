use super::*;
use tokio::io::AsyncWriteExt;

static SIGNAL_ALLOWED: AtomicBool = AtomicBool::new(false);
static SIGNAL_CALLS: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
struct SignalRecovery<'a>(&'a InterruptEvent);
impl Drop for SignalRecovery<'_> {
    fn drop(&mut self) {
        let _ = self.0.force_signal();
    }
}
unsafe extern "system" fn scripted_signal(handle: windows_sys::Win32::Foundation::HANDLE) -> i32 {
    SIGNAL_CALLS.fetch_add(1, Ordering::SeqCst);
    if !SIGNAL_ALLOWED.load(Ordering::SeqCst) {
        // Only the result of this narrow callback is scripted. The same event
        // remains valid and physically unsignalled until the actual retry API.
        unsafe { windows_sys::Win32::Foundation::SetLastError(5) };
        return 0;
    }
    unsafe { windows_sys::Win32::System::Threading::SetEvent(handle) }
}

#[tokio::test]
async fn retrying_failed_native_stop_signal_keeps_original_io_and_actual_blocked_join() {
    use super::super::native::windows::scripted_runtime::{Operation as Op, Script};
    use crate::tunnel::owner::phase_outcomes::join_until;
    const TOTAL: usize = 32 * 1024 * 1024;
    const OBSERVE: Duration = Duration::from_secs(3);
    SIGNAL_ALLOWED.store(false, Ordering::SeqCst);
    SIGNAL_CALLS.store(0, Ordering::SeqCst);
    let bank = Arc::new(Semaphore::new(TOTAL));
    let script = Script::new().unwrap();
    let (stream, mut peer) = tokio::io::duplex(1280);
    let plan = windows::Plan {
        client: "fd00::2".parse().unwrap(),
        server: "fd00::1".parse().unwrap(),
        mtu: 1280,
        prefix: 64,
    };
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
    let native = KernelRuntime::prepare_with_event(
        &engine,
        bank.clone(),
        plan,
        Instant::now() + OBSERVE,
        script.allocator(),
        || InterruptEvent::with_signal(scripted_signal),
    )
    .unwrap();
    native
        .activate(engine.quiesce_and_flush().await.unwrap())
        .await
        .unwrap();
    let recovery = SignalRecovery(native.control.stop_event.get().unwrap());
    tokio::time::timeout(OBSERVE, script.wait_for(Op::Read))
        .await
        .unwrap();
    let held = bank
        .clone()
        .try_acquire_many_owned(bank.available_permits() as u32)
        .unwrap();
    bank.close();
    native.request_stop();
    engine.request_stop();
    let collect = || async {
        tokio::join!(native.join_work(), engine.collect_original());
        native.undo().await;
        engine.release_original();
        native.release_flows();
    };
    let first = join_until(
        Ok(Instant::now() + Duration::from_millis(30)),
        &native.control.hub,
        engine.ledger(),
        native.native_ledger(),
        native.undo_ledger(),
        collect(),
    )
    .await
    .unwrap_err();
    let TunnelError::Phases(first) = first else {
        panic!("same native signal phase failure")
    };
    assert!(first.elapsed().is_some());
    assert!(!first.joins_complete());
    assert_eq!(script.count(Op::Close), 0);
    let original = first.primary_shared().unwrap().clone();
    let NativeError::Call(call) = first.primary().downcast_ref::<NativeError>().unwrap() else {
        panic!("original event API status")
    };
    assert_eq!(call.operation(), "SetEvent");
    assert_eq!(call.original().raw_os_error(), Some(5));
    let io_pointer = call.original() as *const io::Error as usize;
    SIGNAL_ALLOWED.store(true, Ordering::SeqCst);
    let later = join_until(
        Ok(Instant::now() + Duration::from_secs(1)),
        &native.control.hub,
        engine.ledger(),
        native.native_ledger(),
        native.undo_ledger(),
        collect(),
    )
    .await
    .unwrap_err();
    let TunnelError::Phases(later) = later else {
        panic!("same signal failure after retry")
    };
    let recovered = later.joins_complete() && later.elapsed().is_none();
    // A failed old version is recovered through the actual same valid event,
    // then every real join is collected before asserting the regression.
    native
        .control
        .stop_event
        .get()
        .unwrap()
        .force_signal()
        .unwrap();
    collect().await;
    assert!(Arc::ptr_eq(later.primary_shared().unwrap(), &original));
    let NativeError::Call(call) = later.primary().downcast_ref::<NativeError>().unwrap() else {
        panic!("same event API status after actual join")
    };
    assert_eq!(call.original() as *const io::Error as usize, io_pointer);
    assert_eq!(script.count(Op::Close), 1);
    assert!(later.additional_unretained_outcomes() > 0);
    assert!(SIGNAL_CALLS.load(Ordering::SeqCst) > 0);
    assert!(matches!(
        tokio::time::timeout(
            OBSERVE,
            tokio::io::AsyncReadExt::read(&mut peer, &mut [0; 1])
        )
        .await,
        Ok(Ok(0))
    ));
    drop(recovery);
    drop((first, later, original, native, engine, held));
    assert_eq!(bank.available_permits(), TOTAL);
    assert!(
        recovered,
        "stopped flag prevented retry of the same failed native stop signal"
    );
}

#[tokio::test]
async fn original_peer_eof_wakes_native_jobs_while_native_packet_send_is_backpressured() {
    use super::super::native::windows::scripted_runtime::{Operation as Op, Script};
    const TOTAL: usize = 32 * 1024 * 1024;
    const OBSERVE: Duration = Duration::from_secs(3);
    let bank = Arc::new(Semaphore::new(TOTAL));
    let script = Script::new().unwrap();
    script.fail(
        Op::AllocateSend,
        windows_sys::Win32::Foundation::ERROR_BUFFER_OVERFLOW,
    );
    let (stream, mut peer) = tokio::io::duplex(1280);
    let plan = windows::Plan {
        client: "fd00::2".parse().unwrap(),
        server: "fd00::1".parse().unwrap(),
        mtu: 1280,
        prefix: 64,
    };
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
    let native = KernelRuntime::prepare_scripted(
        &engine,
        bank.clone(),
        plan,
        Instant::now() + OBSERVE,
        script.allocator(),
    )
    .unwrap();
    let port = engine.quiesce_and_flush().await.unwrap();
    native.activate(port).await.unwrap();
    let mut packet = [0u8; 48];
    packet[0] = 0x60;
    packet[5] = 8;
    packet[6] = 17;
    packet[7] = 64;
    packet[8..24].copy_from_slice(&plan.server.octets());
    packet[24..40].copy_from_slice(&plan.client.octets());
    packet[40..].copy_from_slice(&[0x51, 0x52, 0x53, 0x54, 0, 8, 0x55, 0x56]);
    for _ in 0..3 {
        peer.write_all(&packet).await.unwrap();
    }
    tokio::time::timeout(OBSERVE, async {
        loop {
            let queued = native
                .control
                .to_native
                .state
                .lock()
                .unwrap()
                .packet
                .is_some();
            if queued
                && native
                    .control
                    .to_native
                    .full_send_observed
                    .load(Ordering::Acquire)
                && script.count(Op::AllocateSend) > 0
            {
                break;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    assert!(script.count(Op::AllocateSend) > 0);
    let held = bank
        .clone()
        .try_acquire_many_owned(bank.available_permits() as u32)
        .unwrap();
    bank.close();
    peer.shutdown().await.unwrap();
    let observed = tokio::time::timeout(Duration::from_secs(1), async {
        while !native.work.read.is_finished() || !native.work.write.is_finished() {
            tokio::task::yield_now().await;
        }
    })
    .await
    .is_ok();
    // A failed old-version observation still ends with genuine native joins.
    native.request_stop();
    engine.request_stop();
    tokio::join!(native.join_work(), engine.collect_original());
    native.undo().await;
    engine.release_original();
    native.release_flows();
    let error = native.work_result().unwrap_err();
    let TunnelError::Phases(report) = error else {
        panic!("same original transport failure")
    };
    assert!(report.joins_complete());
    assert_eq!(script.count(Op::Close), 1);
    drop((report, native, engine, held));
    assert_eq!(bank.available_permits(), TOTAL);
    assert!(
        observed,
        "original EOF was hidden behind the actual full native packet pipe"
    );
}

#[tokio::test]
async fn paid_single_packet_pipe_preserves_cancelled_pending_slot_and_closed_buffer() {
    let bank = Arc::new(Semaphore::new(65536));
    let total = bank.available_permits();
    let pipe = Pipe::prepare(&bank).unwrap();
    pipe.put_blocking(OwnedBuffer::zeroed(32, Some(&bank)).unwrap())
        .unwrap();
    let mut pending = Some(OwnedBuffer::zeroed(64, Some(&bank)).unwrap());
    let pointer = pending.as_ref().unwrap().as_ptr();
    bank.close();
    let mut send = Box::pin(pipe.send_from(&mut pending));
    assert!(futures_util::poll!(send.as_mut()).is_pending());
    drop(send);
    assert_eq!(pending.as_ref().unwrap().as_ptr(), pointer);
    pipe.close_state();
    assert!(pipe.send_from(&mut pending).await.is_err());
    assert_eq!(pending.as_ref().unwrap().as_ptr(), pointer);
    pipe.release();
    drop(pending);
    drop(pipe);
    assert_eq!(bank.available_permits(), total);
}

#[test]
fn actual_pipe_root_refuses_one_byte_short_before_packet_or_task() {
    let bytes = RuntimeRoot::<Pipe>::body_bytes();
    let bank = Arc::new(Semaphore::new(bytes - 1));
    let error = Pipe::prepare(&bank).err().unwrap();
    assert_eq!(
        crate::classify_error(&error),
        crate::ErrorClass::ResourceLimit
    );
    assert_eq!(bank.available_permits(), bytes - 1);
}
