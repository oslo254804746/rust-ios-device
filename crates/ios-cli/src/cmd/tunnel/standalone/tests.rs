use super::*;
use std::error::Error;
use std::marker::PhantomPinned;
use std::pin::Pin;
use std::sync::{
    atomic::{AtomicBool, AtomicUsize, Ordering},
    Mutex,
};
use std::task::{Context, Poll};
use tokio::io::{AsyncReadExt, DuplexStream};
use tokio::sync::{OwnedSemaphorePermit, Semaphore};

const KEY: &str = "standalone-frame-fixture";
const OBSERVE: Duration = Duration::from_secs(2);
const TOTAL: usize = super::super::MAX_GLOBAL_KNOWN_METADATA_BYTES;

fn request(mode: ios_core::TunMode, script_mode: bool) -> Request<'static> {
    Request {
        udid: KEY,
        opts: ConnectOptions {
            tun_mode: mode,
            pair_record_path: None,
            skip_tunnel: false,
        },
        mode: match mode {
            ios_core::TunMode::Userspace => "userspace",
            ios_core::TunMode::Kernel => "kernel",
        },
        script_mode,
    }
}

struct Probe {
    bank: Arc<Semaphore>,
    native: Option<Arc<TunnelHandle>>,
    calls: AtomicUsize,
    reports: AtomicUsize,
    signals: AtomicUsize,
    connector_drops: AtomicUsize,
    signal_drops: AtomicUsize,
    device_drops: AtomicUsize,
    connector_address: AtomicUsize,
    signal_address: AtomicUsize,
    drop_credit: AtomicUsize,
    release_source: AtomicBool,
    release_signal: AtomicBool,
    source_error: Mutex<Option<std::io::Error>>,
    report_error: Mutex<Option<std::io::Error>>,
    signal_error: Mutex<Option<std::io::Error>>,
}
impl Probe {
    fn new(registry: &retirement::RegistryOwner, native: Option<Arc<TunnelHandle>>) -> Arc<Self> {
        Arc::new(Self {
            bank: registry.byte_bank(),
            native,
            calls: AtomicUsize::new(0),
            reports: AtomicUsize::new(0),
            signals: AtomicUsize::new(0),
            connector_drops: AtomicUsize::new(0),
            signal_drops: AtomicUsize::new(0),
            device_drops: AtomicUsize::new(0),
            connector_address: AtomicUsize::new(0),
            signal_address: AtomicUsize::new(0),
            drop_credit: AtomicUsize::new(usize::MAX),
            release_source: AtomicBool::new(false),
            release_signal: AtomicBool::new(false),
            source_error: Mutex::new(None),
            report_error: Mutex::new(None),
            signal_error: Mutex::new(None),
        })
    }
    fn hold_and_close(&self) -> OwnedSemaphorePermit {
        let held = self
            .bank
            .clone()
            .try_acquire_many_owned(self.bank.available_permits() as u32)
            .unwrap();
        self.bank.close();
        held
    }
}

#[repr(align(64))]
struct LocalConnector(Arc<Probe>, [u8; 65], PhantomPinned);
impl LocalConnector {
    fn new(probe: &Arc<Probe>) -> Self {
        Self(probe.clone(), [37; 65], PhantomPinned)
    }
}
impl Drop for LocalConnector {
    fn drop(&mut self) {
        self.0
            .drop_credit
            .store(self.0.bank.available_permits(), Ordering::SeqCst);
        self.0.connector_drops.fetch_add(1, Ordering::SeqCst);
    }
}
struct LocalDevice(Arc<Probe>);
impl Device for LocalDevice {
    fn tunnel_handle(&self) -> Option<&Arc<TunnelHandle>> {
        self.0.native.as_ref()
    }
    fn server_address(&self) -> Option<&str> {
        Some("fd00::1")
    }
    fn rsd_port(&self) -> Option<u16> {
        Some(58783)
    }
    fn userspace_port(&self) -> Option<u16> {
        None
    }
}
impl Drop for LocalDevice {
    fn drop(&mut self) {
        // Original standalone locals drop the completed lease before device.
        if let Some(handle) = &self.0.native {
            assert!(!handle.is_alive());
        }
        self.0.device_drops.fetch_add(1, Ordering::SeqCst);
    }
}
impl Connector for LocalConnector {
    type Device = LocalDevice;
    async fn connect<'a>(
        &'a self,
        udid: &'a str,
        options: ConnectOptions,
        owner: &'a dyn TunnelOwner,
    ) -> std::result::Result<Self::Device, CoreError> {
        assert_eq!(udid, KEY);
        assert!(matches!(options.tun_mode, ios_core::TunMode::Userspace));
        assert!(!options.skip_tunnel && options.pair_record_path.is_none());
        self.0.calls.fetch_add(1, Ordering::SeqCst);
        let mut reservation = owner.reserve_owned()?;
        assert!(Arc::ptr_eq(&reservation.byte_bank().unwrap(), &self.0.bank));
        if let Some(handle) = &self.0.native {
            reservation.attach(handle.clone())?;
        }
        std::future::poll_fn(|_| {
            assert!(self.1.iter().all(|byte| *byte == 37));
            let address = self as *const Self as usize;
            assert_eq!(address % 64, 0);
            let old = self.0.connector_address.swap(address, Ordering::SeqCst);
            assert!(old == 0 || old == address);
            if self.0.release_source.load(Ordering::SeqCst) {
                Poll::Ready(())
            } else {
                Poll::Pending
            }
        })
        .await;
        if let Some(error) = self.0.source_error.lock().unwrap().take() {
            return Err(CoreError::Io(error));
        }
        if self.0.native.is_some() {
            reservation.complete();
        }
        Ok(LocalDevice(self.0.clone()))
    }
}
struct LocalReporter(Arc<Probe>);
impl Reporter for LocalReporter {
    fn ready(
        &self,
        udid: &str,
        mode: &str,
        script: bool,
        device: &impl Device,
        address: &str,
        port: u16,
    ) -> std::io::Result<()> {
        assert_eq!(
            (udid, mode, script, address, port),
            (KEY, "userspace", true, "fd00::1", 58783)
        );
        assert!(Arc::ptr_eq(
            device.tunnel_handle().unwrap(),
            self.0.native.as_ref().unwrap()
        ));
        assert_eq!(self.0.signals.load(Ordering::SeqCst), 0);
        self.0.reports.fetch_add(1, Ordering::SeqCst);
        self.0
            .report_error
            .lock()
            .unwrap()
            .take()
            .map_or(Ok(()), Err)
    }
}
#[repr(align(128))]
struct LocalSignal(Arc<Probe>, [u8; 129], PhantomPinned);
impl LocalSignal {
    fn new(probe: &Arc<Probe>) -> Self {
        Self(probe.clone(), [41; 129], PhantomPinned)
    }
}
impl Future for LocalSignal {
    type Output = std::io::Result<()>;
    fn poll(self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<Self::Output> {
        let this = self.as_ref().get_ref();
        assert!(this.1.iter().all(|byte| *byte == 41));
        assert_eq!(this.0.reports.load(Ordering::SeqCst), 1);
        let address = this as *const Self as usize;
        assert_eq!(address % 128, 0);
        let old = this.0.signal_address.swap(address, Ordering::SeqCst);
        assert!(old == 0 || old == address);
        this.0.signals.fetch_add(1, Ordering::SeqCst);
        if this.0.release_signal.load(Ordering::SeqCst) {
            Poll::Ready(
                this.0
                    .signal_error
                    .lock()
                    .unwrap()
                    .take()
                    .map_or(Ok(()), Err),
            )
        } else {
            Poll::Pending
        }
    }
}
impl Drop for LocalSignal {
    fn drop(&mut self) {
        self.0.signal_drops.fetch_add(1, Ordering::SeqCst);
    }
}
fn local_frame_bytes(registry: &retirement::RegistryOwner, probe: &Arc<Probe>) -> usize {
    let future = run_admitted(
        registry.clone(),
        request(ios_core::TunMode::Userspace, true),
        LocalConnector::new(probe),
        LocalReporter(probe.clone()),
        LocalSignal::new(probe),
    );
    let bytes = std::mem::size_of_val(&future);
    drop(future);
    bytes
}
fn local(
    registry: &retirement::RegistryOwner,
    probe: &Arc<Probe>,
) -> impl Future<Output = Result<()>> + use<> {
    prepare(
        registry.clone(),
        request(ios_core::TunMode::Userspace, true),
        LocalConnector::new(probe),
        LocalReporter(probe.clone()),
        LocalSignal::new(probe),
    )
    .unwrap()
}
fn class(error: &anyhow::Error) -> ios_core::ErrorClass {
    crate::output::operation_error_class(error.as_ref())
}
async fn native() -> (Arc<TunnelHandle>, DuplexStream) {
    let (stream, peer) = tokio::io::duplex(1280);
    let runtime = ios_core::UserspaceTunDevice::start("fd00::2", "fd00::1", 1280, stream)
        .await
        .unwrap();
    (
        Arc::new(TunnelHandle::new_userspace(
            KEY.into(),
            ios_core::TunnelInfo {
                server_address: "fd00::1".into(),
                server_rsd_port: 58783,
                client_address: "fd00::2".into(),
                client_mtu: 1280,
            },
            runtime,
        )),
        peer,
    )
}
async fn baseline(expected: usize) {
    tokio::time::timeout(OBSERVE, async {
        while tokio::runtime::Handle::current()
            .metrics()
            .num_alive_tasks()
            != expected
        {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
}
async fn strict_eof(peer: &mut DuplexStream, expected: usize) {
    assert_eq!(
        tokio::time::timeout(OBSERVE, peer.read(&mut [0; 1]))
            .await
            .unwrap()
            .unwrap(),
        0
    );
    baseline(expected).await;
}

#[test]
fn standalone_actual_native_frame_refuses_before_allocation_and_unpolled_drop_never_enters_sdk() {
    for mode in [ios_core::TunMode::Userspace, ios_core::TunMode::Kernel] {
        for script in [false, true] {
            for refusal in [None, Some(false), Some(true)] {
                let registry = retirement::Registry::new().unwrap();
                let bank = registry.byte_bank();
                let before = bank.available_permits();
                let unpolled = run_admitted(
                    registry.clone(),
                    request(mode, script),
                    NativeConnector,
                    NativeReporter,
                    tokio::signal::ctrl_c(),
                );
                let bytes = std::mem::size_of_val(&unpolled);
                drop(unpolled); // The actual SDK/CtrlC constructors are never polled.
                let held = refusal.map(|closed| {
                    let leave = if closed { 0 } else { bytes - 1 };
                    let held = bank
                        .clone()
                        .try_acquire_many_owned((before - leave) as u32)
                        .unwrap();
                    if closed {
                        bank.close();
                    }
                    held
                });
                let allocations = Arc::new(AtomicUsize::new(0));
                let observed = allocations.clone();
                let _hook = super::super::metadata_vec::on_allocation(move |layout, _| {
                    assert!(refusal.is_none(), "refused frame allocated");
                    assert_eq!(layout.size(), bytes);
                    observed.fetch_add(1, Ordering::SeqCst);
                });
                let prepared = prepare(
                    registry.clone(),
                    request(mode, script),
                    NativeConnector,
                    NativeReporter,
                    tokio::signal::ctrl_c(),
                );
                match refusal {
                    None => {
                        assert_eq!(bank.available_permits(), before - bytes);
                        assert_eq!(allocations.load(Ordering::SeqCst), 1);
                        bank.close();
                        drop(prepared.unwrap());
                        assert_eq!(bank.available_permits(), before);
                    }
                    Some(closed) => {
                        assert_eq!(
                            class(&prepared.err().unwrap()),
                            ios_core::ErrorClass::ResourceLimit
                        );
                        assert_eq!(allocations.load(Ordering::SeqCst), 0);
                        assert_eq!(bank.available_permits(), if closed { 0 } else { bytes - 1 });
                    }
                }
                assert!(!registry.snapshot().sealed);
                assert_eq!(registry.snapshot().occupied(), 0);
                drop(held);
                drop(registry);
                assert_eq!(bank.available_permits(), TOTAL);
            }
        }
    }
}

#[tokio::test]
async fn standalone_aligned_pending_source_and_signal_live_inside_same_paid_frame() {
    let registry = retirement::Registry::new().unwrap();
    let probe = Probe::new(&registry, None);
    let bytes = local_frame_bytes(&registry, &probe);
    let old_connector = probe.connector_drops.load(Ordering::SeqCst);
    let old_signal = probe.signal_drops.load(Ordering::SeqCst);
    let address = Arc::new(AtomicUsize::new(0));
    let observed = address.clone();
    let hook = super::super::metadata_vec::on_allocation(move |layout, pointer| {
        assert_eq!((layout.size(), layout.align()), (bytes, 128));
        observed.store(pointer, Ordering::SeqCst);
    });
    let mut future = Box::pin(local(&registry, &probe));
    drop(hook);
    assert!(futures_util::poll!(&mut future).is_pending());
    let start = address.load(Ordering::SeqCst);
    let connector = probe.connector_address.load(Ordering::SeqCst);
    assert!(
        connector >= start && connector + std::mem::size_of::<LocalConnector>() <= start + bytes
    );
    let held = probe.hold_and_close();
    assert!(futures_util::poll!(&mut future).is_pending());
    assert_eq!(probe.calls.load(Ordering::SeqCst), 1);
    assert_eq!(probe.reports.load(Ordering::SeqCst), 0);
    assert_eq!(probe.signals.load(Ordering::SeqCst), 0);
    assert_eq!(probe.bank.available_permits(), 0);
    drop(future); // Destroy the owner of F, not only Pin<&mut F>.
    assert_eq!(
        probe.connector_drops.load(Ordering::SeqCst),
        old_connector + 1
    );
    assert_eq!(probe.signal_drops.load(Ordering::SeqCst), old_signal + 1);
    assert!(
        probe.drop_credit.load(Ordering::SeqCst)
            <= ownership::Ownership::attempt_bytes() + retirement::Registry::incarnation_bytes()
    );
    assert_eq!(
        probe.bank.available_permits(),
        bytes + ownership::Ownership::attempt_bytes() + retirement::Registry::incarnation_bytes()
    );
    assert!(registry.snapshot().closed_and_empty());
    drop(held);
    drop(registry);
    assert_eq!(probe.bank.available_permits(), TOTAL);
}

#[tokio::test]
async fn standalone_caller_drop_at_source_or_signal_retires_original_native_and_joins_at_closed_zero(
) {
    for at_signal in [false, true] {
        let expected = tokio::runtime::Handle::current()
            .metrics()
            .num_alive_tasks();
        let (handle, mut peer) = native().await;
        let registry = retirement::Registry::new().unwrap();
        let probe = Probe::new(&registry, Some(handle.clone()));
        let bytes = local_frame_bytes(&registry, &probe);
        probe.release_source.store(at_signal, Ordering::SeqCst);
        let mut future = Box::pin(local(&registry, &probe));
        assert!(futures_util::poll!(&mut future).is_pending());
        assert_eq!(probe.reports.load(Ordering::SeqCst), usize::from(at_signal));
        let held = probe.hold_and_close();
        drop(future);
        assert!(!handle.is_alive());
        assert!(registry.snapshot().sealed);
        assert_eq!(registry.snapshot().retired, 1);
        assert_eq!(
            probe.device_drops.load(Ordering::SeqCst),
            usize::from(at_signal)
        );
        let refunded = bytes
            + if at_signal {
                0
            } else {
                ownership::Ownership::attempt_bytes()
            };
        assert_eq!(probe.bank.available_permits(), refunded);
        server::drain_native_terminal(&registry, retirement::CleanupClock::new(OBSERVE).unwrap())
            .await
            .unwrap();
        assert!(handle.shutdown_status().is_ok());
        assert!(registry.snapshot().closed_and_empty());
        strict_eof(&mut peer, expected).await;
        assert_eq!(
            probe.bank.available_permits(),
            refunded + retirement::Registry::incarnation_bytes()
        );
        drop(held);
        drop(registry);
        assert_eq!(probe.bank.available_permits(), TOTAL);
    }
}

#[tokio::test]
async fn standalone_success_preserves_report_signal_and_lease_before_device_drop_order() {
    let expected = tokio::runtime::Handle::current()
        .metrics()
        .num_alive_tasks();
    let (handle, mut peer) = native().await;
    let registry = retirement::Registry::new().unwrap();
    let probe = Probe::new(&registry, Some(handle.clone()));
    let bytes = local_frame_bytes(&registry, &probe);
    let address = Arc::new(AtomicUsize::new(0));
    let observed = address.clone();
    let hook = super::super::metadata_vec::on_allocation(move |layout, pointer| {
        assert_eq!(layout.size(), bytes);
        observed.store(pointer, Ordering::SeqCst);
    });
    let mut future = Box::pin(local(&registry, &probe));
    drop(hook);
    probe.release_source.store(true, Ordering::SeqCst);
    assert!(futures_util::poll!(&mut future).is_pending());
    let start = address.load(Ordering::SeqCst);
    let signal = probe.signal_address.load(Ordering::SeqCst);
    assert!(signal >= start && signal + std::mem::size_of::<LocalSignal>() <= start + bytes);
    assert!(handle.is_alive());
    assert_eq!(registry.snapshot().unpublished, 1);
    tokio::time::pause();
    tokio::time::advance(Duration::from_secs(31)).await;
    assert!(futures_util::poll!(&mut future).is_pending());
    assert!(handle.is_alive());
    assert_eq!(probe.reports.load(Ordering::SeqCst), 1);
    tokio::time::resume();
    let held = probe.hold_and_close();
    probe.release_signal.store(true, Ordering::SeqCst);
    future.await.unwrap();
    assert_eq!(probe.calls.load(Ordering::SeqCst), 1);
    assert_eq!(probe.reports.load(Ordering::SeqCst), 1);
    assert_eq!(probe.device_drops.load(Ordering::SeqCst), 1);
    assert!(registry.snapshot().closed_and_empty());
    assert!(handle.shutdown_status().is_ok());
    strict_eof(&mut peer, expected).await;
    assert_eq!(
        probe.bank.available_permits(),
        bytes + retirement::Registry::incarnation_bytes()
    );
    drop(held);
    drop(registry);
    assert_eq!(probe.bank.available_permits(), TOTAL);
}

#[tokio::test]
async fn standalone_first_poll_ownership_refusal_keeps_original_early_owner_drop_without_source() {
    let registry = retirement::Registry::new().unwrap();
    let probe = Probe::new(&registry, None);
    let bytes = local_frame_bytes(&registry, &probe);
    let future = local(&registry, &probe);
    assert!(!registry.snapshot().sealed);
    let held = probe.hold_and_close();
    let error = future.await.unwrap_err();
    assert_eq!(class(&error), ios_core::ErrorClass::ResourceLimit);
    assert!(error.downcast_ref::<server::CleanupFailure>().is_none());
    assert_eq!(probe.calls.load(Ordering::SeqCst), 0);
    assert_eq!(probe.reports.load(Ordering::SeqCst), 0);
    assert_eq!(probe.signals.load(Ordering::SeqCst), 0);
    assert!(registry.snapshot().closed_and_empty());
    assert_eq!(probe.bank.available_permits(), bytes);
    drop(held);
    drop(registry);
    assert_eq!(probe.bank.available_permits(), TOTAL);
}

#[tokio::test(start_paused = true)]
async fn standalone_original_thirty_second_clock_starts_on_first_poll_and_only_covers_source() {
    let registry = retirement::Registry::new().unwrap();
    let probe = Probe::new(&registry, None);
    let mut future = Box::pin(local(&registry, &probe));
    tokio::time::advance(Duration::from_secs(40)).await;
    assert_eq!(probe.calls.load(Ordering::SeqCst), 0);
    assert_eq!(registry.snapshot().occupied(), 0);
    assert!(futures_util::poll!(&mut future).is_pending());
    tokio::time::advance(Duration::from_secs(29)).await;
    assert!(futures_util::poll!(&mut future).is_pending());
    tokio::time::advance(Duration::from_secs(2)).await;
    let error = future.await.unwrap_err();
    assert_eq!(error.to_string(), "tunnel connect timed out after 30s — the device may not support CoreDevice tunneling or the handshake is blocked");
    assert_eq!(probe.calls.load(Ordering::SeqCst), 1);
    assert_eq!(probe.reports.load(Ordering::SeqCst), 0);
    assert_eq!(probe.signals.load(Ordering::SeqCst), 0);
    assert!(registry.snapshot().closed_and_empty());
    drop(registry);
    assert_eq!(probe.bank.available_permits(), TOTAL);
}

#[derive(Debug)]
struct Original(std::io::Error);
impl std::fmt::Display for Original {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("original standalone IO payload")
    }
}
impl Error for Original {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        Some(&self.0)
    }
}
fn original() -> (std::io::Error, usize, usize) {
    let error = std::io::Error::new(
        std::io::ErrorKind::PermissionDenied,
        Original(std::io::Error::new(
            std::io::ErrorKind::PermissionDenied,
            "standalone-deep-io",
        )),
    );
    let payload = error.get_ref().unwrap().downcast_ref::<Original>().unwrap();
    let pointer = payload as *const Original as usize;
    let io_pointer = &payload.0 as *const _ as usize;
    (error, pointer, io_pointer)
}
fn find<'a, T: Error + 'static>(mut error: &'a (dyn Error + 'static)) -> &'a T {
    for _ in 0..32 {
        if let Some(found) = error.downcast_ref::<T>() {
            return found;
        }
        error = if let Some(io) = error.downcast_ref::<std::io::Error>() {
            io.get_ref()
                .map(|inner| inner as &(dyn Error + 'static))
                .or_else(|| io.source())
                .unwrap()
        } else {
            error.source().unwrap()
        };
    }
    panic!(
        "bounded typed error chain missing {}",
        std::any::type_name::<T>()
    );
}

async fn errors_preserved(broken_native: bool) {
    for phase in 0..3 {
        let expected = tokio::runtime::Handle::current()
            .metrics()
            .num_alive_tasks();
        let (handle, peer) = native().await;
        let mut peer = Some(peer);
        let native_error = if broken_native {
            drop(peer.take());
            tokio::time::timeout(OBSERVE, async {
                while handle.is_alive() {
                    tokio::task::yield_now().await;
                }
            })
            .await
            .unwrap();
            Some(handle.shutdown_status().unwrap_err())
        } else {
            None
        };
        let work_pointer = native_error.as_ref().map(|error| {
            let report = find::<ios_core::TunnelShutdownFailure>(error);
            // Finished workers still belong to the original unjoined owners.
            assert!(!report.joins_complete());
            report.primary() as *const dyn Error as *const () as usize
        });
        let registry = retirement::Registry::new().unwrap();
        let probe = Probe::new(&registry, Some(handle.clone()));
        let (error, pointer, io_pointer) = original();
        *match phase {
            0 => &probe.source_error,
            1 => &probe.report_error,
            _ => &probe.signal_error,
        }
        .lock()
        .unwrap() = Some(error);
        let mut future = Box::pin(local(&registry, &probe));
        assert!(futures_util::poll!(&mut future).is_pending());
        let held = probe.hold_and_close();
        probe.release_source.store(true, Ordering::SeqCst);
        probe.release_signal.store(true, Ordering::SeqCst);
        let error = future.await.unwrap_err();
        let primary = if broken_native {
            let dual = error.downcast_ref::<server::CleanupFailure>().unwrap();
            let terminal = dual
                .cleanup
                .downcast_ref::<server::TerminalCleanupFailure>()
                .unwrap();
            let report = find::<ios_core::TunnelShutdownFailure>(terminal.primary());
            assert!(report.joins_complete());
            assert_eq!(
                report.primary() as *const dyn Error as *const () as usize,
                work_pointer.unwrap()
            );
            &dual.primary
        } else {
            &error
        };
        if phase == 0 {
            assert!(primary.downcast_ref::<CoreError>().is_some());
        } else {
            assert!(primary.downcast_ref::<std::io::Error>().is_some());
        }
        let payload = find::<Original>(primary.as_ref());
        assert_eq!(payload as *const Original as usize, pointer);
        assert_eq!(&payload.0 as *const _ as usize, io_pointer);
        assert_eq!(class(primary), ios_core::ErrorClass::PermissionDenied);
        assert_eq!(probe.calls.load(Ordering::SeqCst), 1);
        assert_eq!(
            probe.reports.load(Ordering::SeqCst),
            usize::from(phase != 0)
        );
        assert_eq!(
            probe.signals.load(Ordering::SeqCst),
            usize::from(phase == 2)
        );
        assert!(registry.snapshot().closed_and_empty());
        if let Some(peer) = &mut peer {
            assert!(handle.shutdown_status().is_ok());
            strict_eof(peer, expected).await;
        } else {
            assert!(handle.shutdown_status().is_err());
            baseline(expected).await;
        }
        drop(error);
        drop(native_error);
        drop(registry);
        drop(held);
        assert_eq!(probe.bank.available_permits(), TOTAL);
    }
}
#[tokio::test]
async fn standalone_original_source_report_and_signal_io_survive_closed_zero_and_native_joins() {
    errors_preserved(false).await;
}
#[tokio::test]
async fn standalone_dual_failure_preserves_original_primary_and_actual_native_cleanup_work() {
    errors_preserved(true).await;
}
