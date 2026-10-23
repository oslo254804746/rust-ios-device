//! Real userspace workers and native results; no physical device or OS TUN.
use super::*;
use std::io;
use std::pin::Pin;
use std::sync::{Condvar, Mutex as StdMutex};
use std::task::{Context, Poll};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, DuplexStream, ReadBuf};

const OBSERVE: Duration = Duration::from_secs(2);

#[tokio::test]
async fn native_report_prepaid_history_and_current_join_snapshots_survive_closed_zero_pool() {
    let baseline = tokio::runtime::Handle::current()
        .metrics()
        .num_alive_tasks();
    let registry = Registry::small(NonZeroUsize::new(1).unwrap());
    let structural = registry.structural_bytes();
    let bank = registry.byte_bank.clone();
    let (stream, peer) = tokio::io::duplex(1280);
    let native = runtime(stream).await;
    let lease = registry
        .reserve(stamp(&registry))
        .unwrap()
        .attach(native.clone());
    drop(peer);
    assert!(inactive(&native).await);
    let original = native.shutdown_status().unwrap_err();
    let pointer = work(&original) as *const TunnelError;
    let held = bank
        .clone()
        .try_acquire_many_owned(bank.available_permits() as u32)
        .unwrap();
    bank.close();
    drop(lease);
    let first = registry
        .drain_retired(CleanupClock::new(Duration::ZERO).unwrap())
        .await
        .unwrap_err();
    let first_error = first.primary().downcast_ref::<TunnelError>().unwrap();
    let pending = shutdown_report(first_error).unwrap().pending_joins();
    assert!(pending > 0);
    assert_eq!(work(first_error) as *const _, pointer);
    assert!(matches!(&first.primary, Some(PayloadOutcome::History(_))));
    assert!(first.elapsed().is_some());
    let history = registry.historical_failure().unwrap();
    assert!(std::ptr::eq(history.primary(), first_error));
    assert_eq!(bank.available_permits(), 0);
    let second = registry
        .drain_retired(CleanupClock::new(OBSERVE).unwrap())
        .await
        .unwrap_err();
    assert!(matches!(&second.primary, Some(PayloadOutcome::Current(_))));
    let second_error = second.primary().downcast_ref::<TunnelError>().unwrap();
    assert!(shutdown_joins_certified(second_error));
    assert_eq!(work(second_error) as *const _, pointer);
    let report_pointer = second_error as *const TunnelError as usize;
    let frame_pointer = &second as *const DrainFailure as usize;
    assert!(
        report_pointer >= frame_pointer
            && report_pointer < frame_pointer + std::mem::size_of::<DrainFailure>()
    );
    assert_eq!(
        shutdown_report(first.primary().downcast_ref::<TunnelError>().unwrap())
            .unwrap()
            .pending_joins(),
        pending
    );
    assert_eq!(
        shutdown_report(history.primary()).unwrap().pending_joins(),
        pending
    );
    assert_eq!(registry.snapshot().occupied(), 0);
    assert!(recovered(baseline).await);
    assert_eq!(bank.available_permits(), Registry::incarnation_bytes());
    drop(registry);
    assert_eq!(
        bank.available_permits(),
        structural - NativeReport::body_bytes() + Registry::incarnation_bytes()
    );
    drop(history);
    assert_eq!(
        bank.available_permits(),
        structural - NativeReport::body_bytes() + Registry::incarnation_bytes()
    );
    drop(first);
    assert_eq!(
        bank.available_permits(),
        structural + Registry::incarnation_bytes()
    );
    assert_eq!(
        work(second.primary().downcast_ref::<TunnelError>().unwrap()) as *const _,
        pointer
    );
    drop((second, held));
    assert_eq!(
        bank.available_permits(),
        super::super::MAX_GLOBAL_KNOWN_METADATA_BYTES
    );
}

#[tokio::test]
async fn native_census_current_snapshot_is_inside_prepaid_backing_and_stays_frozen_through_joins() {
    let baseline = tokio::runtime::Handle::current()
        .metrics()
        .num_alive_tasks();
    let registry = Registry::small(NonZeroUsize::new(1).unwrap());
    let (stream, peer) = tokio::io::duplex(1280);
    let native = runtime(stream).await;
    let owner = stamp(&registry);
    let lease = registry
        .reserve(owner.clone())
        .unwrap()
        .attach(native.clone());
    lease.publish(&owner).unwrap();
    drop(peer);
    assert!(inactive(&native).await);
    let original = native.shutdown_status().unwrap_err();
    let original_pointer = work(&original) as *const TunnelError;
    let bank = registry.byte_bank.clone();
    let before = bank.available_permits();
    let layouts = Arc::new(StdMutex::new(Vec::new()));
    let seen = layouts.clone();
    let charged = bank.clone();
    let (unjoined, roots) = super::super::metadata_vec::with_allocation_hook(
        move |layout, pointer| {
            let mut layouts = seen.lock().unwrap();
            let prior: usize = layouts.iter().map(|(bytes, _)| bytes).sum();
            assert_eq!(charged.available_permits(), before - prior - layout.size());
            layouts.push((layout.size(), pointer));
        },
        || registry.payload_outcomes().unwrap(),
    );
    assert_eq!(unjoined, 1);
    assert_eq!(roots.len(), 1);
    assert!(matches!(roots.as_slice()[0], PayloadOutcome::Current(_)));
    let layouts = layouts.lock().unwrap().clone();
    assert_eq!(layouts.len(), 2);
    let (root_bytes, root_pointer) = layouts[0];
    assert_eq!(
        root_bytes,
        roots.capacity() * std::mem::size_of::<PayloadOutcome>()
    );
    let value_pointer = roots.as_slice()[0].as_ref() as *const TunnelError as usize;
    assert!(value_pointer >= root_pointer && value_pointer < root_pointer + root_bytes);
    assert_eq!(work(&roots.as_slice()[0]) as *const _, original_pointer);
    drop(roots);
    assert_eq!(bank.available_permits(), before);
    let needed: usize = layouts.iter().map(|(bytes, _)| bytes).sum();
    let held = bank
        .clone()
        .try_acquire_many_owned((before - needed + 1) as u32)
        .unwrap();
    let refused = registry.payload_outcomes().err().unwrap();
    assert_eq!(
        crate::output::operation_error_class(refused.as_ref()),
        ErrorClass::ResourceLimit
    );
    assert_eq!(bank.available_permits(), needed - 1);
    assert_eq!(registry.snapshot().active, 1);
    assert_eq!(
        work(&native.shutdown_status().unwrap_err()) as *const _,
        original_pointer
    );
    drop((held, refused));
    let held = bank
        .clone()
        .try_acquire_many_owned((before - needed) as u32)
        .unwrap();
    let (unjoined, frozen) = registry.payload_outcomes().unwrap();
    assert_eq!(unjoined, 1);
    let pending = shutdown_report(&frozen.as_slice()[0])
        .unwrap()
        .pending_joins();
    assert!(pending > 0);
    let spare = bank
        .clone()
        .try_acquire_many_owned(bank.available_permits() as u32)
        .unwrap();
    bank.close();
    assert_eq!(bank.available_permits(), 0);
    drop(lease);
    let failure = registry
        .drain_retired(CleanupClock::new(OBSERVE).unwrap())
        .await
        .unwrap_err();
    let joined = failure.primary().downcast_ref::<TunnelError>().unwrap();
    assert!(shutdown_joins_certified(joined));
    assert_eq!(work(joined) as *const _, original_pointer);
    assert!(recovered(baseline).await);
    assert_eq!(
        shutdown_report(&frozen.as_slice()[0])
            .unwrap()
            .pending_joins(),
        pending
    );
    assert_eq!(work(&frozen.as_slice()[0]) as *const _, original_pointer);
    assert_eq!(bank.available_permits(), Registry::incarnation_bytes());
    drop(frozen);
    assert_eq!(
        bank.available_permits(),
        root_bytes + Registry::incarnation_bytes()
    );
    drop((spare, held));
    assert_eq!(
        bank.available_permits(),
        before + Registry::incarnation_bytes()
    );
}

#[tokio::test]
async fn registry_owner_original_native_lease_outlives_caller_and_joins_at_closed_zero_quota() {
    let baseline = tokio::runtime::Handle::current()
        .metrics()
        .num_alive_tasks();
    let registry = Registry::small(NonZeroUsize::new(1).unwrap());
    let bank = registry.byte_bank.clone();
    let total = super::super::MAX_GLOBAL_KNOWN_METADATA_BYTES;
    let structural = registry.structural_bytes();
    let pointer = &*registry as *const Registry;
    let (stream, mut peer) = tokio::io::duplex(1280);
    let native = runtime(stream).await;
    let lease = registry
        .reserve(stamp(&registry))
        .unwrap()
        .attach(native.clone());
    assert!(lease.registry.ptr_eq(&registry));
    assert_eq!(
        bank.available_permits(),
        total - structural - Registry::incarnation_bytes()
    );
    let held = bank
        .clone()
        .try_acquire_many_owned(bank.available_permits() as u32)
        .unwrap();
    bank.close();
    drop(registry);
    assert_eq!(bank.available_permits(), 0);
    assert!(native.is_alive());
    let retained = lease.registry.clone();
    assert_eq!(&*retained as *const Registry, pointer);
    drop(lease);
    assert!(
        !native.is_alive(),
        "original lease Drop must request stop before drain"
    );
    let report = retained
        .drain_retired(CleanupClock::new(OBSERVE).unwrap())
        .await
        .unwrap();
    assert_eq!(
        (report.selected, report.joined, report.snapshot.occupied()),
        (1, 1, 0)
    );
    let mut byte = [0];
    assert_eq!(
        tokio::time::timeout(OBSERVE, peer.read(&mut byte))
            .await
            .unwrap()
            .unwrap(),
        0
    );
    assert!(native.shutdown_status().is_ok());
    assert!(recovered(baseline).await);
    assert_eq!(
        bank.available_permits(),
        Registry::incarnation_bytes(),
        "only the last incarnation key ended; the registry body/backing remain live"
    );
    drop(retained);
    assert_eq!(
        bank.available_permits(),
        structural + Registry::incarnation_bytes()
    );
    drop(held);
    assert_eq!(bank.available_permits(), total);
}

#[tokio::test]
async fn native_ordinal_cleanup_at_zero_pool_joins_original_workers_and_strict_eof() {
    let baseline = tokio::runtime::Handle::current()
        .metrics()
        .num_alive_tasks();
    let registry = Registry::small(NonZeroUsize::new(1).unwrap());
    let (stream, mut peer) = tokio::io::duplex(1280);
    let native = runtime(stream).await;
    let lease = registry
        .reserve(stamp(&registry))
        .unwrap()
        .attach(Arc::clone(&native));
    let bank = Arc::clone(&registry.byte_bank);
    let before = bank.available_permits();
    let held = Arc::clone(&bank)
        .try_acquire_many_owned(before.try_into().unwrap())
        .unwrap();
    drop(lease);
    let drain = registry.drain_retired(CleanupClock::new(OBSERVE).unwrap());
    assert!(
        !native.is_alive(),
        "original stop must precede polling the drain"
    );
    let report = drain.await.unwrap();
    let mut byte = [0];
    assert_eq!(
        tokio::time::timeout(OBSERVE, peer.read(&mut byte))
            .await
            .unwrap()
            .unwrap(),
        0
    );
    assert!(recovered(baseline).await);
    assert_eq!(
        (report.selected, report.joined, report.snapshot.occupied()),
        (1, 1, 0)
    );
    assert!(native.shutdown_status().is_ok());
    assert_eq!(bank.available_permits(), Registry::incarnation_bytes());
    drop(held);
    assert_eq!(
        bank.available_permits(),
        before + Registry::incarnation_bytes()
    );
}

#[tokio::test]
async fn native_census_keeps_active_original_io_on_quota_refusal_and_last_root_drop() {
    let baseline = tokio::runtime::Handle::current()
        .metrics()
        .num_alive_tasks();
    let registry = Registry::small(NonZeroUsize::new(1).unwrap());
    let owner = stamp(&registry);
    let (stream, peer) = tokio::io::duplex(1280);
    let native = runtime(stream).await;
    let lease = registry
        .reserve(owner.clone())
        .unwrap()
        .attach(Arc::clone(&native));
    lease.publish(&owner).unwrap();
    drop(peer);
    assert!(inactive(&native).await);
    let original_report = native.shutdown_status().unwrap_err();
    let original = work(&original_report) as *const TunnelError;
    let bank = Arc::clone(&registry.byte_bank);
    let before = bank.available_permits();
    let held = Arc::clone(&bank)
        .try_acquire_many_owned(before.try_into().unwrap())
        .unwrap();
    assert!(registry.payload_outcomes().is_err());
    assert_eq!(registry.snapshot().active, 1);
    assert_eq!(
        work(&native.shutdown_status().unwrap_err()) as *const _,
        original
    );
    drop(held);
    let (unjoined, roots) = registry.payload_outcomes().unwrap();
    assert_eq!(unjoined, 1);
    assert_eq!(roots.as_slice().len(), 1);
    assert_eq!(work(&roots.as_slice()[0]) as *const _, original);
    assert!(bank.available_permits() < before);
    drop(roots);
    assert_eq!(bank.available_permits(), before);
    drop(lease);
    let failure = registry
        .drain_retired(CleanupClock::new(OBSERVE).unwrap())
        .await
        .unwrap_err();
    let joined = failure.primary().downcast_ref::<TunnelError>().unwrap();
    assert!(shutdown_joins_certified(joined));
    assert_eq!(work(joined) as *const _, original);
    assert!(recovered(baseline).await);
    assert_eq!(registry.snapshot().occupied(), 0);
}

fn stamp(registry: &RegistryOwner) -> OwnershipStamp {
    OwnershipStamp::standalone(registry)
}

async fn runtime<S: AsyncRead + AsyncWrite + Unpin + Send + 'static>(
    stream: S,
) -> Arc<TunnelHandle> {
    let runtime = ios_core::UserspaceTunDevice::start("fd00::2", "fd00::1", 1280, stream)
        .await
        .unwrap();
    Arc::new(TunnelHandle::new_userspace(
        "native-registry-fixture".into(),
        ios_core::TunnelInfo {
            server_address: "fd00::1".into(),
            server_rsd_port: 58783,
            client_address: "fd00::2".into(),
            client_mtu: 1280,
        },
        runtime,
    ))
}

async fn inactive(native: &TunnelHandle) -> bool {
    tokio::time::timeout(OBSERVE, async {
        while native.is_alive() {
            tokio::task::yield_now().await;
        }
    })
    .await
    .is_ok()
}

async fn recovered(baseline: usize) -> bool {
    tokio::time::timeout(OBSERVE, async {
        while tokio::runtime::Handle::current()
            .metrics()
            .num_alive_tasks()
            != baseline
        {
            tokio::task::yield_now().await;
        }
    })
    .await
    .is_ok()
}

fn work(error: &TunnelError) -> &TunnelError {
    shutdown_report(error)
        .unwrap()
        .primary()
        .downcast_ref::<TunnelError>()
        .unwrap()
}

#[tokio::test]
async fn native_retirement_joins_real_workers_with_another_handle_arc_held() {
    let baseline = tokio::runtime::Handle::current()
        .metrics()
        .num_alive_tasks();
    let registry = Registry::small(NonZeroUsize::new(1).unwrap());
    let (stream, mut peer) = tokio::io::duplex(1280);
    let native = runtime(stream).await;
    let lease = registry
        .reserve(stamp(&registry))
        .unwrap()
        .attach(Arc::clone(&native));
    let retained = Arc::clone(&native);
    drop(lease);
    let result = registry
        .drain_retired(CleanupClock::new(OBSERVE).unwrap())
        .await;
    let eof = tokio::time::timeout(OBSERVE, peer.read_u8()).await;
    let tasks = recovered(baseline).await;
    let report = result.unwrap();
    assert!(Arc::ptr_eq(&native, &retained));
    assert!(!native.is_alive());
    assert_eq!(
        (report.selected, report.joined, report.snapshot.occupied()),
        (1, 1, 0)
    );
    assert!(native.shutdown_status().is_ok());
    assert!(eof
        .unwrap()
        .is_err_and(|error| error.kind() == io::ErrorKind::UnexpectedEof));
    assert!(tasks);
}

#[tokio::test]
async fn native_retirement_fully_joined_eof_releases_slot_and_preserves_history_for_terminal_return(
) {
    let baseline = tokio::runtime::Handle::current()
        .metrics()
        .num_alive_tasks();
    let registry = Registry::small(NonZeroUsize::new(1).unwrap());
    let (stream, peer) = tokio::io::duplex(1280);
    let native = runtime(stream).await;
    let lease = registry
        .reserve(stamp(&registry))
        .unwrap()
        .attach(Arc::clone(&native));
    drop(peer);
    let finished = inactive(&native).await;
    drop(lease);
    let first = registry
        .drain_retired(CleanupClock::new(OBSERVE).unwrap())
        .await;
    let empty = registry
        .drain_retired(CleanupClock::new(OBSERVE).unwrap())
        .await;
    registry.close_and_stop_all();
    let terminal =
        super::super::server::drain_native_terminal(&registry, CleanupClock::new(OBSERVE).unwrap())
            .await;
    let status = native.shutdown_status();
    let tasks = recovered(baseline).await;
    assert!(finished && tasks);
    let first = first.unwrap_err();
    let native_error = first.primary().downcast_ref::<TunnelError>().unwrap();
    assert!(shutdown_joins_certified(native_error));
    let original = work(native_error);
    let TunnelError::Io(io) = original else {
        panic!("original native IO missing")
    };
    assert_eq!(io.kind(), io::ErrorKind::UnexpectedEof);
    assert_eq!(empty.unwrap().selected, 0);
    assert_eq!(registry.snapshot().occupied(), 0);
    let history = registry.historical_failure().unwrap();
    assert!(std::ptr::eq(work(history.primary()), original));
    assert!(terminal
        .unwrap_err()
        .is::<super::super::server::TerminalCleanupFailure>());
    assert!(std::ptr::eq(work(&status.unwrap_err()), original));
}

#[derive(Default)]
struct Gate {
    released: StdMutex<bool>,
    condition: Condvar,
}
impl Gate {
    fn release(&self) {
        *self.released.lock().unwrap() = true;
        self.condition.notify_all();
    }
    fn wait(&self) {
        let mut released = self.released.lock().unwrap();
        while !*released {
            released = self.condition.wait(released).unwrap();
        }
    }
}

struct FaultStream {
    stream: DuplexStream,
    error: Option<io::Error>,
    gate: Arc<Gate>,
}

struct PanickingDropStream {
    stream: DuplexStream,
    gate: Arc<Gate>,
}
impl AsyncRead for PanickingDropStream {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        bytes: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        Pin::new(&mut self.stream).poll_read(cx, bytes)
    }
}
impl AsyncWrite for PanickingDropStream {
    fn poll_write(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        bytes: &[u8],
    ) -> Poll<io::Result<usize>> {
        Pin::new(&mut self.stream).poll_write(cx, bytes)
    }
    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.stream).poll_flush(cx)
    }
    fn poll_shutdown(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.stream).poll_shutdown(cx)
    }
}
impl Drop for PanickingDropStream {
    fn drop(&mut self) {
        self.gate.wait();
        std::panic::panic_any("synthetic-private-native-drop-panic");
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn native_retirement_later_join_panic_replaces_same_owner_pure_expiry_without_losing_timer() {
    let baseline = tokio::runtime::Handle::current()
        .metrics()
        .num_alive_tasks();
    let registry = Registry::small(NonZeroUsize::new(1).unwrap());
    let (stream, mut peer) = tokio::io::duplex(1280);
    let gate = Arc::new(Gate::default());
    let native = runtime(PanickingDropStream {
        stream,
        gate: gate.clone(),
    })
    .await;
    let lease = registry
        .reserve(stamp(&registry))
        .unwrap()
        .attach(native.clone());
    let key = lease.key.clone();
    drop(lease);
    // Supply an actual independently returned Core expiry to the registry's
    // bounded history. The later production drain observes this same runtime's
    // original JoinError after its controlled destructor is released.
    let first = native
        .shutdown(Duration::from_millis(30))
        .await
        .err()
        .map(|error| registry.record_failure(&key, error));
    let before = registry.historical_failure();
    gate.release();
    let retried = registry
        .drain_retired(CleanupClock::new(OBSERVE).unwrap())
        .await;
    let eof = tokio::time::timeout(OBSERVE, peer.read_u8()).await;
    let tasks = recovered(baseline).await;
    let history = registry.historical_failure();
    assert!(tasks);
    assert!(eof
        .unwrap()
        .is_err_and(|error| error.kind() == io::ErrorKind::UnexpectedEof));
    let first = first.expect("actual first Core expiry missing");
    assert!(shutdown_primary_is_elapsed(&first) && !shutdown_joins_certified(&first));
    assert!(shutdown_primary_is_elapsed(before.unwrap().primary()));
    let retry = retried.unwrap_err();
    let error = retry.primary().downcast_ref::<TunnelError>().unwrap();
    let report = shutdown_report(error).unwrap();
    let join = report
        .primary()
        .downcast_ref::<tokio::task::JoinError>()
        .unwrap();
    assert!(report.joins_complete() && join.is_panic() && !join.is_cancelled());
    assert_eq!(registry.snapshot().occupied(), 0);
    assert_eq!(registry.snapshot().additional_failed_owners, 0);
    let history = history.unwrap();
    assert!(std::ptr::eq(
        shutdown_report(history.primary()).unwrap().primary(),
        report.primary()
    ));
    assert!(std::ptr::eq(history.expiry().unwrap(), first.as_ref()));
    let mut attempt = AttemptEvidence::default();
    attempt.record(first);
    attempt.record(PayloadOutcome::Current(
        native.shutdown_status().unwrap_err(),
    ));
    assert!(!shutdown_primary_is_elapsed(
        attempt.first.as_ref().unwrap()
    ));
    assert_eq!(attempt.additional, 1);
}
impl AsyncRead for FaultStream {
    fn poll_read(
        mut self: Pin<&mut Self>,
        _: &mut Context<'_>,
        _: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        match self.error.take() {
            Some(error) => Poll::Ready(Err(error)),
            None => Poll::Pending,
        }
    }
}
impl AsyncWrite for FaultStream {
    fn poll_write(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        bytes: &[u8],
    ) -> Poll<io::Result<usize>> {
        Pin::new(&mut self.stream).poll_write(cx, bytes)
    }
    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.stream).poll_flush(cx)
    }
    fn poll_shutdown(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.stream).poll_shutdown(cx)
    }
}
impl Drop for FaultStream {
    fn drop(&mut self) {
        self.gate.wait();
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn native_retirement_outer_expiry_captures_acquired_io_before_internal_shutdown_returns() {
    let baseline = tokio::runtime::Handle::current()
        .metrics()
        .num_alive_tasks();
    let registry = Registry::small(NonZeroUsize::new(1).unwrap());
    let (stream, mut peer) = tokio::io::duplex(1280);
    let gate = Arc::new(Gate::default());
    let native = runtime(FaultStream {
        stream,
        error: Some(io::Error::from(io::ErrorKind::BrokenPipe)),
        gate: Arc::clone(&gate),
    })
    .await;
    let lease = registry
        .reserve(stamp(&registry))
        .unwrap()
        .attach(Arc::clone(&native));
    let finished = inactive(&native).await;
    drop(lease);
    let expired = registry
        .drain_retired(CleanupClock::new(Duration::from_millis(30)).unwrap())
        .await;
    let held = registry.snapshot();
    let history = registry.historical_failure();
    gate.release();
    let retried = registry
        .drain_retired(CleanupClock::new(OBSERVE).unwrap())
        .await;
    let eof = tokio::time::timeout(OBSERVE, peer.read_u8()).await;
    let tasks = recovered(baseline).await;
    assert!(finished && tasks);
    assert!(eof
        .unwrap()
        .is_err_and(|error| error.kind() == io::ErrorKind::UnexpectedEof));
    let expired = expired.unwrap_err();
    assert!(expired.elapsed().is_some());
    assert_eq!(held.occupied(), 1);
    assert_eq!(held.retired, 1);
    let original = expired.primary().downcast_ref::<TunnelError>().unwrap();
    assert!(shutdown_report(original).unwrap().has_retained_outcomes());
    assert!(!shutdown_joins_certified(original));
    assert_eq!(ios_core::classify_error(original), ErrorClass::Disconnected);
    let TunnelError::Io(io) = work(original) else {
        panic!("acquired native IO missing")
    };
    assert_eq!(io.kind(), io::ErrorKind::BrokenPipe);
    assert!(std::ptr::eq(
        work(history.unwrap().primary()),
        work(original)
    ));
    let retried = retried.unwrap_err();
    assert_eq!(registry.snapshot().occupied(), 0);
    assert!(shutdown_joins_certified(
        retried.primary().downcast_ref::<TunnelError>().unwrap()
    ));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn native_retirement_false_outer_report_never_uses_completed_foreign_inner_certificate() {
    let baseline = tokio::runtime::Handle::current()
        .metrics()
        .num_alive_tasks();
    let (foreign_stream, foreign_peer) = tokio::io::duplex(1280);
    let foreign = runtime(foreign_stream).await;
    drop(foreign_peer);
    let foreign_ready = inactive(&foreign).await;
    let foreign_result = foreign.shutdown(OBSERVE).await;
    let foreign_certified = foreign_result
        .as_ref()
        .err()
        .is_some_and(shutdown_joins_certified);
    let (stream, mut peer) = tokio::io::duplex(1280);
    let gate = Arc::new(Gate::default());
    let native = runtime(FaultStream {
        stream,
        error: Some(io::Error::other(foreign_result.unwrap_err())),
        gate: Arc::clone(&gate),
    })
    .await;
    let registry = Registry::small(NonZeroUsize::new(1).unwrap());
    let lease = registry
        .reserve(stamp(&registry))
        .unwrap()
        .attach(Arc::clone(&native));
    let finished = inactive(&native).await;
    drop(lease);
    let expired = registry
        .drain_retired(CleanupClock::new(Duration::from_millis(30)).unwrap())
        .await;
    let held = registry.snapshot();
    gate.release();
    let retried = registry
        .drain_retired(CleanupClock::new(OBSERVE).unwrap())
        .await;
    let eof = tokio::time::timeout(OBSERVE, peer.read_u8()).await;
    let tasks = recovered(baseline).await;
    assert!(foreign_ready && foreign_certified && finished && tasks);
    assert!(eof.unwrap().is_err());
    let expired = expired.unwrap_err();
    let outer = expired.primary().downcast_ref::<TunnelError>().unwrap();
    assert!(!shutdown_joins_certified(outer));
    let TunnelError::Io(io) = work(outer) else {
        panic!("foreign report's original wrapper missing")
    };
    let inner = io.get_ref().unwrap().downcast_ref::<TunnelError>().unwrap();
    assert!(shutdown_joins_certified(inner));
    assert_eq!(held.occupied(), 1);
    assert_eq!(registry.snapshot().occupied(), 0);
    assert!(shutdown_joins_certified(
        retried
            .unwrap_err()
            .primary()
            .downcast_ref::<TunnelError>()
            .unwrap()
    ));
}
