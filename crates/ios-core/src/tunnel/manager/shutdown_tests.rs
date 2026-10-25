//! Host-only owned kernel forwarding fixtures using native Tokio tasks.
//! Kernel TUN creation and privileged teardown require separate validation.

use std::error::Error;
use std::pin::Pin;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::task::{Context, Poll};
use std::time::Duration;

use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt, DuplexStream, ReadBuf};
use tokio::sync::Notify;

use super::{TunnelHandle, TunnelInfo, TunnelRuntime};
use crate::tunnel::forward::forward_packets_inner_owned;
use crate::tunnel::tun::TunDevice;
use crate::tunnel::{TunnelError, TunnelShutdownFailure, ValidatedMtu};

const OBSERVE: Duration = Duration::from_secs(1);

#[derive(Default)]
struct Probe {
    read_bytes: AtomicUsize,
    drops: AtomicUsize,
    read_pending: Notify,
}

struct Tracked {
    stream: DuplexStream,
    probe: Arc<Probe>,
}

impl AsyncRead for Tracked {
    fn poll_read(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<std::io::Result<()>> {
        let this = self.get_mut();
        let before = buf.filled().len();
        let result = Pin::new(&mut this.stream).poll_read(cx, buf);
        this.probe
            .read_bytes
            .fetch_add(buf.filled().len() - before, Ordering::SeqCst);
        if result.is_pending() {
            this.probe.read_pending.notify_one();
        }
        result
    }
}

impl AsyncWrite for Tracked {
    fn poll_write(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        bytes: &[u8],
    ) -> Poll<std::io::Result<usize>> {
        Pin::new(&mut self.get_mut().stream).poll_write(cx, bytes)
    }
    fn poll_flush(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        Pin::new(&mut self.get_mut().stream).poll_flush(cx)
    }
    fn poll_shutdown(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        Pin::new(&mut self.get_mut().stream).poll_shutdown(cx)
    }
}

impl Drop for Tracked {
    fn drop(&mut self) {
        self.probe.drops.fetch_add(1, Ordering::SeqCst);
    }
}

enum Tun {
    Idle,
    PanicWrite(Arc<Notify>),
}

struct NativeTunPanic;
impl std::fmt::Debug for NativeTunPanic {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("NativeTunPanic")
    }
}

impl TunDevice for Tun {
    async fn read_packet(&mut self) -> Result<Vec<u8>, TunnelError> {
        std::future::pending().await
    }
    async fn write_packet(&mut self, _packet: &[u8]) -> Result<(), TunnelError> {
        if let Self::PanicWrite(entered) = self {
            entered.notify_one();
            std::panic::panic_any(NativeTunPanic);
        }
        Ok(())
    }
}

fn info() -> TunnelInfo {
    TunnelInfo {
        server_address: "fd00::1".into(),
        server_rsd_port: 58783,
        client_address: "fd00::2".into(),
        client_mtu: 1280,
    }
}

fn packet() -> Vec<u8> {
    let mut bytes = vec![0; 40];
    bytes[0] = 0x60;
    bytes
}

fn stream() -> (DuplexStream, Tracked, Arc<Probe>) {
    let (peer, stream) = tokio::io::duplex(128);
    let probe = Arc::new(Probe::default());
    (
        peer,
        Tracked {
            stream,
            probe: Arc::clone(&probe),
        },
        probe,
    )
}

fn owned_forward(stream: Tracked, tun: Tun) -> TunnelHandle {
    TunnelHandle::new_owned_kernel(
        "native-owned-kernel-fixture".into(),
        info(),
        move |cancel, outcomes, readers| {
            tokio::spawn(forward_packets_inner_owned(
                stream,
                tun,
                ValidatedMtu::new(1280).unwrap(),
                cancel,
                outcomes,
                readers,
            ))
        },
    )
}

fn snapshot(error: &TunnelError) -> Option<&TunnelShutdownFailure> {
    let mut source: Option<&(dyn Error + 'static)> = Some(error);
    for _ in 0..32 {
        let value = source?;
        if let Some(report) = value.downcast_ref::<TunnelShutdownFailure>() {
            return Some(report);
        }
        source = value.source();
    }
    None
}

async fn recovered(mut peer: DuplexStream, probe: &Probe, baseline: usize) -> (bool, bool) {
    let mut bytes = Vec::new();
    let eof = matches!(
        tokio::time::timeout(OBSERVE, peer.read_to_end(&mut bytes)).await,
        Ok(Ok(_))
    );
    drop(peer);
    let recovered = tokio::time::timeout(OBSERVE, async {
        while probe.drops.load(Ordering::SeqCst) != 1
            || tokio::runtime::Handle::current()
                .metrics()
                .num_alive_tasks()
                != baseline
        {
            tokio::task::yield_now().await;
        }
    })
    .await
    .is_ok();
    (eof, recovered)
}

#[tokio::test]
async fn native_kernel_owner_actual_factory_joins_partial_reader() {
    let baseline = tokio::runtime::Handle::current()
        .metrics()
        .num_alive_tasks();
    let (mut peer, stream, probe) = stream();
    peer.write_all(&packet()[..20]).await.unwrap();
    let handle = owned_forward(stream, Tun::Idle);
    let retained =
        matches!(&handle._runtime, TunnelRuntime::Kernel { tasks, .. } if tasks.len() == 1);
    let reached = tokio::time::timeout(OBSERVE, probe.read_pending.notified())
        .await
        .is_ok();
    let active = tokio::runtime::Handle::current()
        .metrics()
        .num_alive_tasks()
        .saturating_sub(baseline);
    let result = handle.shutdown(OBSERVE).await;
    let joined_drops = probe.drops.load(Ordering::SeqCst);
    let joined_tasks = tokio::runtime::Handle::current()
        .metrics()
        .num_alive_tasks();
    let terminal = !handle.is_alive();
    let repeated = handle.shutdown(OBSERVE).await;
    drop(handle);
    let (eof, recovery) = recovered(peer, &probe, baseline).await;
    assert!(retained && reached && terminal && recovery && eof);
    assert_eq!(active, 2);
    assert_eq!(joined_drops, 1);
    assert_eq!(joined_tasks, baseline);
    assert_eq!(probe.read_bytes.load(Ordering::SeqCst), 20);
    result.unwrap();
    repeated.unwrap();
}

#[tokio::test]
async fn native_kernel_owner_parent_panic_retains_and_joins_reader_set() {
    let baseline = tokio::runtime::Handle::current()
        .metrics()
        .num_alive_tasks();
    let (mut peer, stream, probe) = stream();
    peer.write_all(&packet()).await.unwrap();
    let entered = Arc::new(Notify::new());
    let handle = owned_forward(stream, Tun::PanicWrite(Arc::clone(&entered)));
    let reached = tokio::time::timeout(OBSERVE, entered.notified())
        .await
        .is_ok();
    let result = handle.shutdown(OBSERVE).await;
    let joined_drops = probe.drops.load(Ordering::SeqCst);
    let joined_tasks = tokio::runtime::Handle::current()
        .metrics()
        .num_alive_tasks();
    drop(handle);
    let (eof, recovery) = recovered(peer, &probe, baseline).await;
    assert!(reached && eof && recovery);
    assert_eq!(joined_drops, 1);
    assert_eq!(joined_tasks, baseline);
    assert_eq!(probe.read_bytes.load(Ordering::SeqCst), 40);
    let error = result.unwrap_err();
    let report = snapshot(&error).unwrap();
    assert_eq!(report.pending_joins(), 0);
    assert_eq!(crate::classify_error(&error), crate::ErrorClass::Unknown);
    let native = report
        .primary()
        .downcast_ref::<tokio::task::JoinError>()
        .unwrap();
    assert!(native.is_panic());
    assert!(!native.is_cancelled());
}

struct CleanupGate {
    handle: TunnelHandle,
    peer: DuplexStream,
    probe: Arc<Probe>,
    acquired: Arc<Notify>,
    release: Arc<Notify>,
    primary: Arc<Mutex<Option<Arc<TunnelError>>>>,
}

// Controlled cleanup after actual native read_exact EOF, using the actual
// Kernel owner factory and a real native child holding the split writer.
// The Notify delay is a private fixture; it is not a real kernel/TUN teardown.
async fn cleanup_gate() -> CleanupGate {
    let (mut peer, stream, probe) = stream();
    peer.write_all(&packet()[..20]).await.unwrap();
    peer.shutdown().await.unwrap();
    let acquired = Arc::new(Notify::new());
    let release = Arc::new(Notify::new());
    let primary = Arc::new(Mutex::new(None));
    let task_acquired = Arc::clone(&acquired);
    let task_release = Arc::clone(&release);
    let task_primary = Arc::clone(&primary);
    let handle = TunnelHandle::new_owned_kernel(
        "native-owned-gate-fixture".into(),
        info(),
        move |cancel, outcomes, children| {
            tokio::spawn(async move {
                let _cancel = cancel;
                let (mut reader, writer) = tokio::io::split(stream);
                let mut children = children.lock().await;
                children.spawn(async move {
                    let _writer = writer;
                    std::future::pending::<()>().await;
                });
                let mut header = [0; 40];
                if let Err(error) = reader.read_exact(&mut header).await {
                    let error = Arc::new(TunnelError::Io(error));
                    *task_primary.lock().unwrap() = Some(Arc::clone(&error));
                    outcomes.record_work(0, error);
                }
                task_acquired.notify_one();
                task_release.notified().await;
                children.abort_all();
                while children.join_next().await.is_some() {}
                outcomes.work_result()
            })
        },
    );
    CleanupGate {
        handle,
        peer,
        probe,
        acquired,
        release,
        primary,
    }
}

#[tokio::test]
async fn native_kernel_owner_actual_eof_primary_survives_expiry_then_retry() {
    let baseline = tokio::runtime::Handle::current()
        .metrics()
        .num_alive_tasks();
    let gate = cleanup_gate().await;
    let reached = tokio::time::timeout(OBSERVE, gate.acquired.notified())
        .await
        .is_ok();
    let expired = gate.handle.shutdown(Duration::from_millis(30)).await;
    let held_drops = gate.probe.drops.load(Ordering::SeqCst);
    let held_tasks = tokio::runtime::Handle::current()
        .metrics()
        .num_alive_tasks()
        .saturating_sub(baseline);
    gate.release.notify_one();
    let retried = gate.handle.shutdown(OBSERVE).await;
    let primary = gate.primary.lock().unwrap().clone();
    drop(gate.handle);
    let (eof, recovery) = recovered(gate.peer, &gate.probe, baseline).await;
    assert!(reached && eof && recovery);
    assert_eq!(held_drops, 0);
    assert_eq!(held_tasks, 2);
    assert_eq!(gate.probe.read_bytes.load(Ordering::SeqCst), 20);
    let expired = expired.unwrap_err();
    let retried = retried.unwrap_err();
    let original = primary.unwrap();
    for error in [&expired, &retried] {
        assert_eq!(crate::classify_error(error), crate::ErrorClass::Io);
        let report = snapshot(error).unwrap();
        let retained = report.primary().downcast_ref::<TunnelError>().unwrap();
        assert!(std::ptr::eq(retained, original.as_ref()));
        let native = retained
            .source()
            .unwrap()
            .downcast_ref::<std::io::Error>()
            .unwrap();
        assert_eq!(native.kind(), std::io::ErrorKind::UnexpectedEof);
    }
    let expired = snapshot(&expired).unwrap();
    assert_eq!(expired.pending_joins(), 2);
    assert!(expired
        .cleanup()
        .any(|cause| cause.is::<tokio::time::error::Elapsed>()));
    let retried = snapshot(&retried).unwrap();
    assert_eq!(retried.pending_joins(), 0);
    assert!(!retried
        .cleanup()
        .any(|cause| cause.is::<tokio::time::error::Elapsed>()));
}

#[tokio::test]
async fn native_shutdown_status_preserves_acquired_eof_after_outer_future_drop() {
    let baseline = tokio::runtime::Handle::current()
        .metrics()
        .num_alive_tasks();
    let gate = cleanup_gate().await;
    let reached = tokio::time::timeout(OBSERVE, gate.acquired.notified())
        .await
        .is_ok();
    let before = gate.handle.shutdown_status();
    let live_before = gate.handle.is_alive();
    let mut cleanup = Box::pin(gate.handle.shutdown(OBSERVE));
    let interrupted = tokio::time::timeout(Duration::from_millis(30), &mut cleanup)
        .await
        .is_err();
    drop(cleanup);
    let after = gate.handle.shutdown_status();
    let original = gate.primary.lock().unwrap().clone();
    gate.release.notify_one();
    let retried = gate.handle.shutdown(OBSERVE).await;
    drop(gate.handle);
    let (eof, recovery) = recovered(gate.peer, &gate.probe, baseline).await;
    assert!(reached && live_before && interrupted && eof && recovery);
    let original = original.unwrap();
    let before = before.unwrap_err();
    let after = after.unwrap_err();
    let retried = retried.unwrap_err();
    for error in [&before, &after, &retried] {
        let report = snapshot(error).unwrap();
        assert!(report.has_retained_outcomes());
        assert!(std::ptr::eq(
            report.primary().downcast_ref::<TunnelError>().unwrap(),
            original.as_ref()
        ));
        assert!(!report
            .cleanup()
            .any(|cause| cause.is::<tokio::time::error::Elapsed>()));
    }
    assert_eq!(snapshot(&before).unwrap().pending_joins(), 2);
    assert_eq!(snapshot(&after).unwrap().pending_joins(), 2);
    assert!(!snapshot(&after).unwrap().joins_complete());
    assert!(snapshot(&retried).unwrap().joins_complete());
    assert_eq!(snapshot(&before).unwrap().pending_joins(), 2);
}

#[tokio::test]
async fn native_kernel_owner_dropped_join_future_can_retry_native_primary() {
    let baseline = tokio::runtime::Handle::current()
        .metrics()
        .num_alive_tasks();
    let gate = cleanup_gate().await;
    let reached = tokio::time::timeout(OBSERVE, gate.acquired.notified())
        .await
        .is_ok();
    let mut cleanup = Box::pin(gate.handle.shutdown(OBSERVE));
    let interrupted = tokio::time::timeout(Duration::from_millis(30), &mut cleanup)
        .await
        .is_err();
    drop(cleanup);
    let terminal = !gate.handle.is_alive();
    let held_drops = gate.probe.drops.load(Ordering::SeqCst);
    gate.release.notify_one();
    let retried = gate.handle.shutdown(OBSERVE).await;
    drop(gate.handle);
    let (eof, recovery) = recovered(gate.peer, &gate.probe, baseline).await;
    assert!(reached && interrupted && terminal && eof && recovery);
    assert_eq!(held_drops, 0);
    let error = retried.unwrap_err();
    assert_eq!(snapshot(&error).unwrap().pending_joins(), 0);
    let original = snapshot(&error)
        .unwrap()
        .primary()
        .downcast_ref::<TunnelError>()
        .unwrap();
    assert_eq!(
        original
            .source()
            .unwrap()
            .downcast_ref::<std::io::Error>()
            .unwrap()
            .kind(),
        std::io::ErrorKind::UnexpectedEof
    );
}

// v2 adds a direct original-reader-join retention control after native panic.

struct NativeDropGate {
    entered: Notify,
    released: Mutex<bool>,
    changed: std::sync::Condvar,
}

impl NativeDropGate {
    fn new() -> Arc<Self> {
        Arc::new(Self {
            entered: Notify::new(),
            released: Mutex::new(false),
            changed: std::sync::Condvar::new(),
        })
    }

    fn release(&self) {
        let mut released = self
            .released
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        *released = true;
        self.changed.notify_all();
    }
}

// A test-body unwind must release the native worker before Tokio runtime Drop.
struct ReleaseNativeDrop(Arc<NativeDropGate>);

impl Drop for ReleaseNativeDrop {
    fn drop(&mut self) {
        self.0.release();
    }
}

struct BlockingDropStream {
    stream: Tracked,
    gate: Arc<NativeDropGate>,
}

impl AsyncRead for BlockingDropStream {
    fn poll_read(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<std::io::Result<()>> {
        Pin::new(&mut self.get_mut().stream).poll_read(cx, buf)
    }
}

impl AsyncWrite for BlockingDropStream {
    fn poll_write(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        bytes: &[u8],
    ) -> Poll<std::io::Result<usize>> {
        Pin::new(&mut self.get_mut().stream).poll_write(cx, bytes)
    }

    fn poll_flush(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        Pin::new(&mut self.get_mut().stream).poll_flush(cx)
    }

    fn poll_shutdown(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        Pin::new(&mut self.get_mut().stream).poll_shutdown(cx)
    }
}

impl Drop for BlockingDropStream {
    fn drop(&mut self) {
        self.gate.entered.notify_one();
        let released = self
            .gate
            .released
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        let _released = self
            .gate
            .changed
            .wait_while(released, |released| !*released)
            .unwrap_or_else(|error| error.into_inner());
        // Tracked and the real duplex stream drop only after this gate returns.
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn native_kernel_owner_parent_panic_still_waits_for_original_reader_join() {
    // One worker may block in the controlled native task destructor; the other
    // remains available for Tokio's timer. The test thread releases it on every
    // normal/unwind path before runtime destruction.
    let baseline = tokio::runtime::Handle::current()
        .metrics()
        .num_alive_tasks();
    let (mut peer, stream, probe) = stream();
    peer.write_all(&packet()).await.unwrap();
    let gate = NativeDropGate::new();
    let release_on_unwind = ReleaseNativeDrop(Arc::clone(&gate));
    let stream = BlockingDropStream {
        stream,
        gate: Arc::clone(&gate),
    };
    let handle = TunnelHandle::new_owned_kernel(
        "native-owned-panic-reader-fixture".into(),
        info(),
        move |cancel, outcomes, readers| {
            tokio::spawn(forward_packets_inner_owned(
                stream,
                Tun::PanicWrite(Arc::new(Notify::new())),
                ValidatedMtu::new(1280).unwrap(),
                cancel,
                outcomes,
                readers,
            ))
        },
    );
    let reader_drop_entered = tokio::time::timeout(OBSERVE, gate.entered.notified())
        .await
        .is_ok();
    let expired = tokio::time::timeout(OBSERVE, handle.shutdown(Duration::from_millis(30))).await;
    let held_drops = probe.drops.load(Ordering::SeqCst);
    gate.release();
    drop(release_on_unwind);
    let retried = handle.shutdown(OBSERVE).await;
    drop(handle);
    let (eof, recovery) = recovered(peer, &probe, baseline).await;
    // A local JoinSet Drop may abort and eventually release the stream too.
    // It cannot certify pending native child join + deadline while Drop blocks.
    assert!(reader_drop_entered && eof && recovery);
    assert_eq!(held_drops, 0);
    assert_eq!(probe.read_bytes.load(Ordering::SeqCst), 40);
    let expired = expired.unwrap().unwrap_err();
    let retried = retried.unwrap_err();
    for error in [&expired, &retried] {
        assert_eq!(crate::classify_error(error), crate::ErrorClass::Unknown);
        let report = snapshot(error).unwrap();
        let parent = report
            .primary()
            .downcast_ref::<tokio::task::JoinError>()
            .unwrap();
        assert!(parent.is_panic() && !parent.is_cancelled());
    }
    let expired = snapshot(&expired).unwrap();
    assert_eq!(expired.pending_joins(), 1);
    assert!(expired
        .cleanup()
        .any(|cause| cause.is::<tokio::time::error::Elapsed>()));
    let retried = snapshot(&retried).unwrap();
    assert_eq!(retried.pending_joins(), 0);
    assert!(!retried
        .cleanup()
        .any(|cause| cause.is::<tokio::time::error::Elapsed>()));
}

struct NativeReaderPanic;

impl std::fmt::Debug for NativeReaderPanic {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("NativeKernelReaderFixturePanic")
    }
}

struct PanicAfterPartialRead {
    stream: Tracked,
    entered: Arc<Notify>,
}

impl AsyncRead for PanicAfterPartialRead {
    fn poll_read(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<std::io::Result<()>> {
        let this = self.get_mut();
        let result = Pin::new(&mut this.stream).poll_read(cx, buf);
        if result.is_pending() {
            // The real duplex prefix has been consumed. Panic occurs inside
            // the actual native read_stream_packets child, before another byte.
            this.entered.notify_one();
            std::panic::panic_any(NativeReaderPanic);
        }
        result
    }
}

impl AsyncWrite for PanicAfterPartialRead {
    fn poll_write(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        bytes: &[u8],
    ) -> Poll<std::io::Result<usize>> {
        Pin::new(&mut self.get_mut().stream).poll_write(cx, bytes)
    }

    fn poll_flush(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        Pin::new(&mut self.get_mut().stream).poll_flush(cx)
    }

    fn poll_shutdown(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        Pin::new(&mut self.get_mut().stream).poll_shutdown(cx)
    }
}

#[tokio::test]
async fn native_kernel_owner_reader_panic_keeps_native_primary_without_work_promotion() {
    let baseline = tokio::runtime::Handle::current()
        .metrics()
        .num_alive_tasks();
    let (mut peer, stream, probe) = stream();
    peer.write_all(&packet()[..20]).await.unwrap();
    let entered = Arc::new(Notify::new());
    let stream = PanicAfterPartialRead {
        stream,
        entered: Arc::clone(&entered),
    };
    let handle = TunnelHandle::new_owned_kernel(
        "native-owned-reader-report-fixture".into(),
        info(),
        move |cancel, outcomes, readers| {
            tokio::spawn(forward_packets_inner_owned(
                stream,
                Tun::Idle,
                ValidatedMtu::new(1280).unwrap(),
                cancel,
                outcomes,
                readers,
            ))
        },
    );
    let reader_panicked = tokio::time::timeout(OBSERVE, entered.notified())
        .await
        .is_ok();
    // The sender is still alive and no stop is requested. Native child panic
    // closes packet_rx, and the real parent must finish naturally through None.
    let parent_completed = tokio::time::timeout(OBSERVE, async {
        loop {
            if matches!(&handle._runtime, TunnelRuntime::Kernel { tasks, .. }
                if tasks.first().is_some_and(|task| task.is_finished()))
            {
                break;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .is_ok();
    let preliminary = match &handle._runtime {
        TunnelRuntime::Kernel { outcomes, .. } => outcomes.work_result(),
        #[cfg(feature = "tunnel-userspace")]
        TunnelRuntime::Userspace { .. } => Ok(()),
        #[cfg(feature = "tunnel-private-ip")]
        TunnelRuntime::Prepared { .. } => panic!("fixture must own the original kernel parent"),
    };
    let result = handle.shutdown(OBSERVE).await;
    let joined_drops = probe.drops.load(Ordering::SeqCst);
    let joined_tasks = tokio::runtime::Handle::current()
        .metrics()
        .num_alive_tasks();
    let repeated = handle.shutdown(OBSERVE).await;
    drop(handle);
    // Recover old/no-provenance controls before diagnosing their wrong error.
    let (eof, recovery) = recovered(peer, &probe, baseline).await;
    assert!(reader_panicked && parent_completed && eof && recovery);
    assert_eq!(joined_drops, 1);
    assert_eq!(joined_tasks, baseline);
    assert_eq!(probe.read_bytes.load(Ordering::SeqCst), 20);
    let preliminary = preliminary.unwrap_err();
    let native = snapshot(&preliminary)
        .unwrap()
        .primary()
        .downcast_ref::<tokio::task::JoinError>()
        .unwrap();
    assert!(native.is_panic() && !native.is_cancelled());
    for error in [result.unwrap_err(), repeated.unwrap_err()] {
        assert_eq!(crate::classify_error(&error), crate::ErrorClass::Unknown);
        let report = snapshot(&error).unwrap();
        let retained = report
            .primary()
            .downcast_ref::<tokio::task::JoinError>()
            .unwrap();
        assert!(std::ptr::eq(retained, native));
        assert!(retained.is_panic() && !retained.is_cancelled());
        assert_eq!(report.pending_joins(), 0);
        assert!(report.joins_complete());
        assert_eq!(report.cleanup().count(), 0);
        assert_eq!(report.additional_unretained_outcomes(), 0);
    }
}
