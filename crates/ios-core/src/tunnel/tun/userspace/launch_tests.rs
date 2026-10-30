//! Actual dormant transport/future/slot owners, without device or OS TUN IO.
use super::*;
use std::future::Future;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::pin::Pin;
use std::sync::atomic::AtomicUsize;
use std::task::{Context, Poll, Wake, Waker};
use tokio::io::{AsyncRead, AsyncWrite, DuplexStream, ReadBuf};

const TOTAL: usize = 16 * 1024 * 1024;
const OBSERVE: Duration = Duration::from_secs(2);

#[derive(Default)]
struct Traffic {
    reads: AtomicUsize,
    writes: AtomicUsize,
    drops: AtomicUsize,
}
struct Stream {
    stream: DuplexStream,
    traffic: Arc<Traffic>,
}
impl AsyncRead for Stream {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        bytes: &mut ReadBuf<'_>,
    ) -> Poll<std::io::Result<()>> {
        self.traffic.reads.fetch_add(1, Ordering::SeqCst);
        Pin::new(&mut self.stream).poll_read(cx, bytes)
    }
}
impl AsyncWrite for Stream {
    fn poll_write(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        bytes: &[u8],
    ) -> Poll<std::io::Result<usize>> {
        self.traffic.writes.fetch_add(1, Ordering::SeqCst);
        Pin::new(&mut self.stream).poll_write(cx, bytes)
    }
    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        Pin::new(&mut self.stream).poll_flush(cx)
    }
    fn poll_shutdown(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        Pin::new(&mut self.stream).poll_shutdown(cx)
    }
}
impl Drop for Stream {
    fn drop(&mut self) {
        self.traffic.drops.fetch_add(1, Ordering::SeqCst);
    }
}
fn stream() -> (Stream, DuplexStream, Arc<Traffic>) {
    let (stream, peer) = tokio::io::duplex(1280);
    let traffic = Arc::new(Traffic::default());
    (
        Stream {
            stream,
            traffic: traffic.clone(),
        },
        peer,
        traffic,
    )
}
fn alive() -> usize {
    tokio::runtime::Handle::current()
        .metrics()
        .num_alive_tasks()
}

fn snapshot(error: &TunnelError) -> &owner::TunnelShutdownFailure {
    let mut cause: &(dyn std::error::Error + 'static) = error;
    loop {
        if let Some(report) = cause.downcast_ref::<owner::TunnelShutdownFailure>() {
            return report;
        }
        cause = cause.source().expect("native launch completion report");
    }
}

async fn recovered(mut peer: DuplexStream, traffic: &Traffic, baseline: usize) {
    assert!(matches!(
        tokio::time::timeout(OBSERVE, peer.read(&mut [0; 1])).await,
        Ok(Ok(0))
    ));
    tokio::time::timeout(OBSERVE, async {
        while alive() != baseline {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    assert_eq!(traffic.drops.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn dormant_actual_four_futures_drop_unpolled_and_refund_before_retained_runtime_roots() {
    let baseline = alive();
    let bank = Arc::new(Semaphore::new(TOTAL));
    let (stream, peer, traffic) = stream();
    let (runtime, launch) = UserspaceTunDevice::prepare_validated_with_bank(
        "fd00::2",
        "fd00::1",
        ValidatedMtu::new(1280).unwrap(),
        stream,
        Some(bank.clone()),
    )
    .await
    .unwrap();
    assert_eq!(runtime.task_handles.len(), 0);
    assert!(!runtime.is_alive());
    assert_eq!(alive(), baseline);
    {
        let pending = runtime.shutdown_status().unwrap_err();
        let pending = snapshot(&pending);
        assert_eq!(pending.pending_joins(), 0);
        assert!(pending.launch_pending());
        assert!(!pending.joins_complete() && !pending.has_retained_outcomes());
    }
    let held = bank
        .clone()
        .try_acquire_many_owned(bank.available_permits() as u32)
        .unwrap();
    bank.close();
    drop(launch);
    runtime.shutdown_status().unwrap();
    assert_eq!(traffic.reads.load(Ordering::SeqCst), 0);
    assert_eq!(traffic.writes.load(Ordering::SeqCst), 0);
    recovered(peer, &traffic, baseline).await;
    drop(held);
    assert_eq!(bank.available_permits(), TOTAL - runtime_owner_bytes());
    drop(runtime);
    assert_eq!(bank.available_permits(), TOTAL);
}

#[tokio::test]
async fn stop_before_launch_returns_typed_cancel_without_worker_or_inner_poll_at_closed_zero() {
    let baseline = alive();
    let bank = Arc::new(Semaphore::new(TOTAL));
    let (stream, peer, traffic) = stream();
    let (runtime, launch) = UserspaceTunDevice::prepare_validated_with_bank(
        "fd00::2",
        "fd00::1",
        ValidatedMtu::new(1280).unwrap(),
        stream,
        Some(bank.clone()),
    )
    .await
    .unwrap();
    let held = bank
        .clone()
        .try_acquire_many_owned(bank.available_permits() as u32)
        .unwrap();
    bank.close();
    runtime.request_stop();
    let error = launch().unwrap_err();
    assert_eq!(crate::classify_error(&error), crate::ErrorClass::Cancelled);
    assert_eq!(runtime.task_handles.len(), 0);
    assert_eq!(traffic.reads.load(Ordering::SeqCst), 0);
    assert_eq!(traffic.writes.load(Ordering::SeqCst), 0);
    runtime.shutdown(OBSERVE).await.unwrap();
    recovered(peer, &traffic, baseline).await;
    drop((held, runtime));
    assert_eq!(bank.available_permits(), TOTAL);
}

#[tokio::test]
async fn cancelled_shutdown_wait_keeps_dormant_original_futures_until_their_launch_owner_drops() {
    let baseline = alive();
    let bank = Arc::new(Semaphore::new(TOTAL));
    let (stream, peer, traffic) = stream();
    let (runtime, launch) = UserspaceTunDevice::prepare_validated_with_bank(
        "fd00::2",
        "fd00::1",
        ValidatedMtu::new(1280).unwrap(),
        stream,
        Some(bank.clone()),
    )
    .await
    .unwrap();
    let mut shutdown = Box::pin(runtime.shutdown(OBSERVE));
    assert!(futures_util::poll!(shutdown.as_mut()).is_pending());
    let pending = runtime.shutdown_status().unwrap_err();
    assert!(snapshot(&pending).launch_pending());
    assert!(!snapshot(&pending).joins_complete());
    drop(pending);
    drop(shutdown);
    assert_eq!(traffic.drops.load(Ordering::SeqCst), 0);
    drop(launch);
    runtime.shutdown(OBSERVE).await.unwrap();
    assert_eq!(traffic.reads.load(Ordering::SeqCst), 0);
    recovered(peer, &traffic, baseline).await;
    drop(runtime);
    assert_eq!(bank.available_permits(), TOTAL);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn stop_during_partial_install_cannot_complete_before_later_original_join_slots_settle() {
    for panic_on_release in [false, true] {
        let baseline = alive();
        let bank = Arc::new(Semaphore::new(TOTAL));
        let (stream, peer, traffic) = stream();
        let (runtime, launch) = UserspaceTunDevice::prepare_validated_with_bank(
            "fd00::2",
            "fd00::1",
            ValidatedMtu::new(1280).unwrap(),
            stream,
            Some(bank.clone()),
        )
        .await
        .unwrap();
        let (entered_tx, entered_rx) = std::sync::mpsc::channel();
        let (release_tx, release_rx) = std::sync::mpsc::channel();
        let handle = tokio::runtime::Handle::current();
        let thread = std::thread::spawn(move || {
            let _entered = handle.enter();
            let hook = launch::on_installed(move |index| {
                if index == 0 {
                    entered_tx.send(()).unwrap();
                    release_rx.recv_timeout(OBSERVE).unwrap();
                    if panic_on_release {
                        std::panic::panic_any(InstallPanic);
                    }
                }
            });
            let result = launch();
            drop(hook);
            result
        });
        entered_rx.recv_timeout(OBSERVE).unwrap();
        let mut shutdown = Box::pin(runtime.shutdown(OBSERVE));
        let pending = futures_util::poll!(shutdown.as_mut()).is_pending();
        release_tx.send(()).unwrap();
        let stopped = thread.join();
        // The unwind path deliberately avoids a second arbitrary waker call.
        // Cancel this actual pending waiter, then retry using the same real slots.
        drop(shutdown);
        // Always release/join the synchronous fixture thread before assertions.
        assert!(pending);
        if panic_on_release {
            assert!(stopped.unwrap_err().is::<InstallPanic>());
            assert_eq!(runtime.task_handles.len(), 1);
        } else {
            assert_eq!(
                crate::classify_error(&stopped.unwrap().unwrap_err()),
                crate::ErrorClass::Cancelled
            );
            assert_eq!(runtime.task_handles.len(), 4);
        }
        runtime.shutdown(OBSERVE).await.unwrap();
        runtime.shutdown_status().unwrap();
        assert_eq!(traffic.reads.load(Ordering::SeqCst), 0);
        assert_eq!(traffic.writes.load(Ordering::SeqCst), 0);
        recovered(peer, &traffic, baseline).await;
        drop(runtime);
        assert_eq!(bank.available_permits(), TOTAL);
    }
}

#[derive(Debug)]
struct InstallPanic;

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn launch_each_installed_slot_survives_original_sync_panic_for_real_join_without_inner_poll()
{
    for at in 0..4 {
        let baseline = alive();
        let bank = Arc::new(Semaphore::new(TOTAL));
        let (stream, peer, traffic) = stream();
        let (runtime, launch) = UserspaceTunDevice::prepare_validated_with_bank(
            "fd00::2",
            "fd00::1",
            ValidatedMtu::new(1280).unwrap(),
            stream,
            Some(bank.clone()),
        )
        .await
        .unwrap();
        let hook = launch::on_installed(move |index| {
            if index == at {
                std::panic::panic_any(InstallPanic);
            }
        });
        let panic = catch_unwind(AssertUnwindSafe(launch)).unwrap_err();
        drop(hook);
        assert!(panic.is::<InstallPanic>());
        assert_eq!(runtime.task_handles.len(), at + 1);
        assert!(!runtime.launch.opened());
        runtime.shutdown(OBSERVE).await.unwrap();
        runtime.shutdown_status().unwrap();
        assert_eq!(traffic.reads.load(Ordering::SeqCst), 0);
        assert_eq!(traffic.writes.load(Ordering::SeqCst), 0);
        recovered(peer, &traffic, baseline).await;
        drop(runtime);
        assert_eq!(bank.available_permits(), TOTAL);
    }
}

#[derive(Debug)]
struct WakePanic;
struct PanickingWake(AtomicBool);
impl Wake for PanickingWake {
    fn wake(self: Arc<Self>) {
        self.wake_by_ref();
    }
    fn wake_by_ref(self: &Arc<Self>) {
        if self.0.swap(false, Ordering::SeqCst) {
            std::panic::panic_any(WakePanic);
        }
    }
}

#[tokio::test]
async fn launch_notify_original_waker_panic_still_aborts_and_joins_every_installed_original() {
    let baseline = alive();
    let bank = Arc::new(Semaphore::new(TOTAL));
    let (stream, peer, traffic) = stream();
    let (runtime, launch) = UserspaceTunDevice::prepare_validated_with_bank(
        "fd00::2",
        "fd00::1",
        ValidatedMtu::new(1280).unwrap(),
        stream,
        Some(bank.clone()),
    )
    .await
    .unwrap();
    let waker = Waker::from(Arc::new(PanickingWake(AtomicBool::new(true))));
    let control = runtime.launch.clone();
    let mut waiting = Box::pin(control.wait());
    assert!(waiting
        .as_mut()
        .poll(&mut Context::from_waker(&waker))
        .is_pending());
    let panic = catch_unwind(AssertUnwindSafe(launch)).unwrap_err();
    assert!(panic.is::<WakePanic>());
    assert_eq!(runtime.task_handles.len(), 4);
    assert!(!runtime.launch.opened());
    runtime.shutdown(OBSERVE).await.unwrap();
    assert_eq!(traffic.reads.load(Ordering::SeqCst), 0);
    assert_eq!(traffic.writes.load(Ordering::SeqCst), 0);
    recovered(peer, &traffic, baseline).await;
    drop(waiting);
    drop(control);
    drop(runtime);
    assert_eq!(bank.available_permits(), TOTAL);
}

#[tokio::test]
async fn all_original_four_joins_and_proxy_child_finish_while_runtime_roots_stay_held() {
    let baseline = alive();
    let bank = Arc::new(Semaphore::new(TOTAL));
    let (stream, mut peer, traffic) = stream();
    let (runtime, launch) = UserspaceTunDevice::prepare_validated_with_bank(
        "fd00::2",
        "fd00::1",
        ValidatedMtu::new(1280).unwrap(),
        stream,
        Some(bank.clone()),
    )
    .await
    .unwrap();
    launch().unwrap();
    let mut client =
        tokio::net::TcpStream::connect((std::net::Ipv4Addr::LOCALHOST, runtime.local_port))
            .await
            .unwrap();
    let mut preamble = [0; LOCAL_PROXY_PREAMBLE_LEN];
    preamble[..16].copy_from_slice(&std::net::Ipv6Addr::new(0xfd00, 0, 0, 0, 0, 0, 0, 1).octets());
    preamble[16..].copy_from_slice(&58783u32.to_le_bytes());
    client.write_all(&preamble).await.unwrap();
    // A real SYN proves the actual child consumed its complete preamble,
    // submitted its socket request and remains owned waiting for connection.
    let mut header = [0; IPV6_HEADER_LEN];
    tokio::time::timeout(OBSERVE, peer.read_exact(&mut header))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(header[0] >> 4, 6);
    let mut body = vec![0; u16::from_be_bytes([header[4], header[5]]) as usize];
    tokio::time::timeout(OBSERVE, peer.read_exact(&mut body))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(header[6], 6, "first packet must contain TCP");
    assert!(body.len() >= 20, "first packet has a truncated TCP header");
    assert_eq!(u16::from_be_bytes([body[2], body[3]]), 58783);
    assert_eq!(body[13] & 0x12, 0x02, "first packet must be a SYN");
    assert_eq!(runtime.task_handles.len(), 4);
    runtime.shutdown(OBSERVE).await.unwrap();
    runtime.shutdown_status().unwrap();
    let client_read = tokio::time::timeout(OBSERVE, client.read(&mut [0; 1])).await;
    // Cancellation closes the proxy child; neither result certifies graceful FIN.
    match client_read {
        Ok(Ok(0)) => eprintln!("cancelled proxy peer outcome: EOF"),
        Ok(Err(error)) if error.kind() == std::io::ErrorKind::ConnectionReset => {
            eprintln!("cancelled proxy peer outcome: {error:?}");
        }
        other => panic!("unexpected cancelled proxy peer outcome: {other:?}"),
    }
    recovered(peer, &traffic, baseline).await;
    assert!(bank.available_permits() < TOTAL);
    drop(runtime);
    assert_eq!(bank.available_permits(), TOTAL);
}

#[test]
fn actual_launch_control_last_byte_refusal_and_closed_last_physical_free_keep_complete_fee() {
    let bytes = LaunchControl::body_bytes();
    let short = Arc::new(Semaphore::new(bytes - 1));
    let error = crate::device::prepared_rsd::with_allocation_hook(
        |_, _| panic!("short actual launch root allocated"),
        || {
            LaunchControl::prepare(Some(&short), &Ledger::new())
                .err()
                .unwrap()
        },
    );
    assert_eq!(
        crate::classify_error(&error),
        crate::ErrorClass::ResourceLimit
    );
    assert_eq!(short.available_permits(), bytes - 1);
    let bank = Arc::new(Semaphore::new(bytes));
    let control = LaunchControl::prepare(Some(&bank), &Ledger::new()).unwrap();
    let kept = control.clone();
    bank.close();
    drop(control);
    let observed = bank.clone();
    mpsc::roots::after_free(
        move |layout, _| {
            assert_eq!(layout.size(), bytes);
            assert_eq!(observed.available_permits(), 0);
        },
        || drop(kept),
    );
    assert_eq!(bank.available_permits(), bytes);
}
