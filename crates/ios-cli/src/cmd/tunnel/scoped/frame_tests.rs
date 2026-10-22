use super::*;
use std::marker::PhantomPinned;
use std::sync::{
    atomic::{AtomicBool, AtomicUsize, Ordering},
    Mutex as StdMutex,
};
use std::task::Poll;
use tokio::io::{AsyncReadExt, DuplexStream};

const KEY: &str = "foreground-frame-fixture";
const OBSERVE: Duration = Duration::from_secs(2);
const TOTAL: usize = super::super::MAX_GLOBAL_KNOWN_METADATA_BYTES;

struct Probe {
    bank: Arc<Semaphore>,
    calls: AtomicUsize,
    polls: AtomicUsize,
    connector_drops: AtomicUsize,
    device_drops: AtomicUsize,
    address: AtomicUsize,
    drop_credit: AtomicUsize,
    device_drop_credit: AtomicUsize,
    release: AtomicBool,
    failure: StdMutex<Option<anyhow::Error>>,
    native: Option<Arc<TunnelHandle>>,
}
impl Probe {
    fn new(scope: &NativeScope, native: Option<Arc<TunnelHandle>>) -> Arc<Self> {
        Arc::new(Self {
            bank: scope.0.registry.byte_bank(),
            calls: AtomicUsize::new(0),
            polls: AtomicUsize::new(0),
            connector_drops: AtomicUsize::new(0),
            device_drops: AtomicUsize::new(0),
            address: AtomicUsize::new(0),
            drop_credit: AtomicUsize::new(usize::MAX),
            device_drop_credit: AtomicUsize::new(usize::MAX),
            release: AtomicBool::new(false),
            failure: StdMutex::new(None),
            native,
        })
    }
}

#[repr(align(128))]
struct LocalConnector {
    probe: Arc<Probe>,
    original: [u8; 129],
    _pinned: PhantomPinned,
}
impl LocalConnector {
    fn new(probe: &Arc<Probe>) -> Self {
        Self {
            probe: probe.clone(),
            original: [37; 129],
            _pinned: PhantomPinned,
        }
    }
}
impl Drop for LocalConnector {
    fn drop(&mut self) {
        self.probe
            .drop_credit
            .store(self.probe.bank.available_permits(), Ordering::SeqCst);
        self.probe.connector_drops.fetch_add(1, Ordering::SeqCst);
    }
}
struct LocalDevice(Arc<Probe>);
impl Device for LocalDevice {
    fn tunnel_handle(&self) -> Option<&Arc<TunnelHandle>> {
        self.0.native.as_ref()
    }
}
impl Drop for LocalDevice {
    fn drop(&mut self) {
        let expected = self.0.device_drop_credit.load(Ordering::SeqCst);
        if expected != usize::MAX {
            assert_eq!(self.0.bank.available_permits(), expected);
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
    ) -> Result<Self::Device> {
        assert_eq!(udid, KEY);
        assert!(matches!(options.tun_mode, TunMode::Userspace));
        assert!(!options.skip_tunnel && options.pair_record_path.is_none());
        self.probe.calls.fetch_add(1, Ordering::SeqCst);
        let mut reservation = owner.reserve_owned()?;
        assert!(Arc::ptr_eq(
            &reservation.byte_bank().expect("original native bank"),
            &self.probe.bank
        ));
        if let Some(native) = &self.probe.native {
            reservation.attach(native.clone())?;
        }
        std::future::poll_fn(|_| {
            assert!(self.original.iter().all(|byte| *byte == 37));
            let address = self as *const Self as usize;
            assert_eq!(address % 128, 0);
            let old = self.probe.address.swap(address, Ordering::SeqCst);
            assert!(old == 0 || old == address);
            self.probe.polls.fetch_add(1, Ordering::SeqCst);
            if self.probe.release.load(Ordering::SeqCst) {
                Poll::Ready(())
            } else {
                Poll::Pending
            }
        })
        .await;
        if let Some(original) = self.probe.failure.lock().unwrap().take() {
            self.probe.bank.close();
            return Err(original);
        }
        if self.probe.native.is_some() {
            reservation.complete();
        }
        Ok(LocalDevice(self.probe.clone()))
    }
}

fn frame_bytes<C: Connector>(scope: &NativeScope, connector: C) -> usize {
    let permit = scope.0.connections.clone().try_acquire_owned().unwrap();
    let future = scope.connect_admitted(KEY, TunMode::Userspace, permit, connector);
    let bytes = std::mem::size_of_val(&future);
    drop(future); // Constructor only: no Ownership, connector or SDK poll.
    bytes
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
async fn strict_eof_and_baseline(peer: &mut DuplexStream, expected: usize) {
    tokio::time::timeout(OBSERVE, async {
        assert_eq!(peer.read(&mut [0; 1]).await.unwrap(), 0);
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

#[test]
fn foreground_frame_original_selector_and_connection_prefix_precede_native_byte_refusal() {
    for closed in [false, true] {
        let scope = NativeScope::new().unwrap();
        let bank = scope.0.registry.byte_bank();
        let bytes = frame_bytes(&scope, NativeConnector);
        let leave = if closed { 0 } else { bytes - 1 };
        let held = bank
            .clone()
            .try_acquire_many_owned((bank.available_permits() - leave) as u32)
            .unwrap();
        if closed {
            bank.close();
        }
        let _hook = super::super::metadata_vec::on_allocation(|_, _| {
            panic!("refused native frame allocated before admission");
        });
        let selector = "x".repeat(4097);
        let error = scope
            .prepare_connect(&selector, TunMode::Userspace, NativeConnector)
            .err()
            .unwrap();
        assert_eq!(class(&error), ios_core::ErrorClass::ResourceLimit);
        assert!(error.to_string().contains("selector metadata limit"));
        let all = scope
            .0
            .connections
            .clone()
            .try_acquire_many_owned(4)
            .unwrap();
        let error = scope
            .prepare_connect(KEY, TunMode::Userspace, NativeConnector)
            .err()
            .unwrap();
        assert_eq!(error.to_string(), "foreground native connection capacity");
        drop(all);
        let error = scope
            .prepare_connect(KEY, TunMode::Userspace, NativeConnector)
            .err()
            .unwrap();
        assert_eq!(class(&error), ios_core::ErrorClass::ResourceLimit);
        assert_eq!(scope.0.registry.snapshot().occupied(), 0);
        assert!(!scope.0.registry.snapshot().sealed);
        assert_eq!(scope.0.connections.available_permits(), 4);
        assert_eq!(bank.available_permits(), leave);
        drop(held);
        drop(scope);
        assert_eq!(bank.available_permits(), TOTAL);
    }
}

#[test]
fn foreground_frame_actual_native_unpolled_drop_has_no_stamp_or_sdk_work_and_refunds() {
    let scope = NativeScope::new().unwrap();
    let bank = scope.0.registry.byte_bank();
    let before = bank.available_permits();
    let bytes = frame_bytes(&scope, NativeConnector);
    let observed = Arc::new(AtomicUsize::new(0));
    let seen = observed.clone();
    let _hook = super::super::metadata_vec::on_allocation(move |layout, _| {
        assert_eq!(layout.size(), bytes);
        seen.fetch_add(1, Ordering::SeqCst);
    });
    let future = scope
        .prepare_connect(KEY, TunMode::Userspace, NativeConnector)
        .unwrap();
    assert_eq!(observed.load(Ordering::SeqCst), 1);
    assert_eq!(bank.available_permits(), before - bytes);
    assert_eq!(scope.0.connections.available_permits(), 3);
    assert_eq!(scope.0.registry.snapshot().occupied(), 0);
    bank.close();
    drop(future); // The actual native/Core future is never polled.
    assert_eq!(bank.available_permits(), before);
    assert_eq!(scope.0.connections.available_permits(), 4);
    assert!(!scope.0.registry.snapshot().sealed);
    drop(scope);
    assert_eq!(bank.available_permits(), TOTAL);
}

#[tokio::test]
async fn foreground_frame_concrete_aligned_pending_connector_stays_inside_same_paid_allocation() {
    let scope = NativeScope::new().unwrap();
    let probe = Probe::new(&scope, None);
    let bytes = frame_bytes(&scope, LocalConnector::new(&probe));
    let before_drops = probe.connector_drops.load(Ordering::SeqCst);
    let allocations = Arc::new(StdMutex::new(Vec::new()));
    let observed = allocations.clone();
    let _hook = super::super::metadata_vec::on_allocation(move |layout, pointer| {
        assert_eq!(layout.size(), bytes);
        assert_eq!(layout.align(), 128);
        observed.lock().unwrap().push(pointer);
    });
    let future = scope
        .prepare_connect(KEY, TunMode::Userspace, LocalConnector::new(&probe))
        .unwrap();
    drop(_hook);
    let pointer = allocations.lock().unwrap()[0];
    assert_eq!(allocations.lock().unwrap().len(), 1);
    let mut future = Box::pin(future);
    assert!(futures_util::poll!(&mut future).is_pending());
    let address = probe.address.load(Ordering::SeqCst);
    assert!(
        address >= pointer && address + std::mem::size_of::<LocalConnector>() <= pointer + bytes
    );
    let held = probe
        .bank
        .clone()
        .try_acquire_many_owned(probe.bank.available_permits() as u32)
        .unwrap();
    probe.bank.close();
    assert!(futures_util::poll!(&mut future).is_pending());
    assert_eq!(probe.calls.load(Ordering::SeqCst), 1);
    assert_eq!(probe.polls.load(Ordering::SeqCst), 2);
    assert_eq!(probe.address.load(Ordering::SeqCst), address);
    assert_eq!(probe.bank.available_permits(), 0);
    assert_eq!(scope.0.registry.snapshot().inflight, 1);
    // End the actual paid F, not merely a pinned reference to it.
    drop(future);
    assert_eq!(
        probe.connector_drops.load(Ordering::SeqCst),
        before_drops + 1
    );
    assert!(
        probe.drop_credit.load(Ordering::SeqCst)
            <= Ownership::attempt_bytes() + Registry::incarnation_bytes()
    );
    assert_eq!(
        probe.bank.available_permits(),
        bytes + Ownership::attempt_bytes() + Registry::incarnation_bytes()
    );
    assert_eq!(scope.0.connections.available_permits(), 4);
    assert_eq!(scope.0.registry.snapshot().occupied(), 0);
    drop(held);
    drop(scope);
    assert_eq!(probe.bank.available_permits(), TOTAL);
}

#[tokio::test]
async fn foreground_frame_caller_drop_retires_original_native_at_closed_zero_before_joins() {
    let expected = tokio::runtime::Handle::current()
        .metrics()
        .num_alive_tasks();
    let (handle, mut peer) = native().await;
    let scope = NativeScope::new().unwrap();
    let probe = Probe::new(&scope, Some(handle.clone()));
    let bytes = frame_bytes(&scope, LocalConnector::new(&probe));
    let before_drops = probe.connector_drops.load(Ordering::SeqCst);
    let mut future = Box::pin(
        scope
            .prepare_connect(KEY, TunMode::Userspace, LocalConnector::new(&probe))
            .unwrap(),
    );
    assert!(futures_util::poll!(&mut future).is_pending());
    let held = probe
        .bank
        .clone()
        .try_acquire_many_owned(probe.bank.available_permits() as u32)
        .unwrap();
    probe.bank.close();
    drop(future);
    assert_eq!(
        probe.connector_drops.load(Ordering::SeqCst),
        before_drops + 1
    );
    assert!(probe.drop_credit.load(Ordering::SeqCst) <= Ownership::attempt_bytes());
    assert_eq!(
        probe.bank.available_permits(),
        bytes + Ownership::attempt_bytes()
    );
    assert_eq!(scope.0.connections.available_permits(), 4);
    assert_eq!(scope.0.registry.snapshot().retired, 1);
    assert!(!handle.is_alive());
    assert_eq!(probe.device_drops.load(Ordering::SeqCst), 0);
    scope.close(OBSERVE).await.unwrap();
    assert!(handle.shutdown_status().is_ok());
    assert!(scope.0.registry.snapshot().closed_and_empty());
    strict_eof_and_baseline(&mut peer, expected).await;
    assert_eq!(
        probe.bank.available_permits(),
        bytes + Ownership::attempt_bytes() + Registry::incarnation_bytes()
    );
    drop(held);
    drop(scope);
    assert_eq!(probe.bank.available_permits(), TOTAL);
}

#[tokio::test]
async fn foreground_frame_success_keeps_exact_runtime_lease_and_output_permit_until_device_drop() {
    let expected = tokio::runtime::Handle::current()
        .metrics()
        .num_alive_tasks();
    let (handle, mut peer) = native().await;
    let scope = NativeScope::new().unwrap();
    let probe = Probe::new(&scope, Some(handle.clone()));
    let before = probe.bank.available_permits();
    probe.release.store(true, Ordering::SeqCst);
    let device = scope
        .prepare_connect(KEY, TunMode::Userspace, LocalConnector::new(&probe))
        .unwrap()
        .await
        .unwrap();
    assert!(Arc::ptr_eq(device.tunnel_handle().unwrap(), &handle));
    assert!(device.lease.as_ref().unwrap().matches_runtime(&handle));
    assert_eq!(probe.calls.load(Ordering::SeqCst), 1);
    assert_eq!(probe.device_drops.load(Ordering::SeqCst), 0);
    assert_eq!(scope.0.connections.available_permits(), 3);
    assert_eq!(
        probe.bank.available_permits(),
        before - Registry::incarnation_bytes()
    );
    assert_eq!(scope.0.registry.snapshot().unpublished, 1);
    scope.close(OBSERVE).await.unwrap();
    assert!(scope.0.registry.snapshot().closed_and_empty());
    assert!(handle.shutdown_status().is_ok());
    strict_eof_and_baseline(&mut peer, expected).await;
    assert_eq!(
        probe.bank.available_permits(),
        before - Registry::incarnation_bytes()
    );
    probe
        .device_drop_credit
        .store(before - Registry::incarnation_bytes(), Ordering::SeqCst);
    drop(device);
    assert_eq!(probe.device_drops.load(Ordering::SeqCst), 1);
    assert_eq!(scope.0.connections.available_permits(), 4);
    assert_eq!(probe.bank.available_permits(), before);
    drop(scope);
    assert_eq!(probe.bank.available_permits(), TOTAL);
}

#[tokio::test]
async fn foreground_frame_ready_original_io_survives_closed_bank_and_original_native_retirement() {
    #[derive(Debug)]
    struct Original(std::io::Error);
    impl std::fmt::Display for Original {
        fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            formatter.write_str("original foreground connector failure")
        }
    }
    impl std::error::Error for Original {
        fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
            Some(&self.0)
        }
    }
    let expected = tokio::runtime::Handle::current()
        .metrics()
        .num_alive_tasks();
    let (handle, mut peer) = native().await;
    let scope = NativeScope::new().unwrap();
    let probe = Probe::new(&scope, Some(handle.clone()));
    let original = anyhow::Error::new(Original(std::io::Error::new(
        std::io::ErrorKind::PermissionDenied,
        "original-foreground-io",
    )));
    let pointer = original.downcast_ref::<Original>().unwrap() as *const Original as usize;
    let io_pointer = &original.downcast_ref::<Original>().unwrap().0 as *const _ as usize;
    *probe.failure.lock().unwrap() = Some(original);
    let mut future = Box::pin(
        scope
            .prepare_connect(KEY, TunMode::Userspace, LocalConnector::new(&probe))
            .unwrap(),
    );
    assert!(futures_util::poll!(&mut future).is_pending());
    let held = probe
        .bank
        .clone()
        .try_acquire_many_owned(probe.bank.available_permits() as u32)
        .unwrap();
    probe.bank.close();
    probe.release.store(true, Ordering::SeqCst);
    let error = future.await.err().unwrap();
    assert_eq!(
        error.downcast_ref::<Original>().unwrap() as *const _ as usize,
        pointer
    );
    assert_eq!(
        &error.downcast_ref::<Original>().unwrap().0 as *const _ as usize,
        io_pointer
    );
    assert_eq!(class(&error), ios_core::ErrorClass::PermissionDenied);
    assert_eq!(probe.calls.load(Ordering::SeqCst), 1);
    assert_eq!(scope.0.connections.available_permits(), 4);
    assert_eq!(scope.0.registry.snapshot().retired, 1);
    assert!(!handle.is_alive());
    assert_eq!(probe.device_drops.load(Ordering::SeqCst), 0);
    scope.close(OBSERVE).await.unwrap();
    assert!(handle.shutdown_status().is_ok());
    strict_eof_and_baseline(&mut peer, expected).await;
    drop(scope);
    drop(held);
    assert_eq!(probe.bank.available_permits(), TOTAL);
}
