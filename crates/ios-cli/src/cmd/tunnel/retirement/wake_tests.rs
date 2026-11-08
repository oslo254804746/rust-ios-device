use super::super::{metadata_vec, ownership::Ownership};
use super::*;
use ios_core::TunnelOwner;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::task::{Context, Poll, Wake, Waker};

struct RegistryWake {
    registry: RegistryOwner,
    calls: AtomicUsize,
    unlocked: AtomicBool,
}

impl Wake for RegistryWake {
    fn wake(self: Arc<Self>) {
        self.wake_by_ref();
    }

    fn wake_by_ref(self: &Arc<Self>) {
        let unlocked =
            self.registry.state.try_lock().is_ok() && self.registry.origins.try_lock().is_ok();
        self.unlocked.fetch_and(unlocked, Ordering::SeqCst);
        self.calls.fetch_add(1, Ordering::SeqCst);
    }
}

#[tokio::test]
async fn completed_owner_metadata_refund_wakes_only_after_registry_unlock() {
    use super::super::{admission, metadata_lease};
    let registry = Registry::small(NonZeroUsize::new(1).unwrap());
    let global = registry.byte_bank();
    let published = Arc::new(Semaphore::new(65536));
    let mut births = registry.prepare_births::<2>(&global).unwrap();
    let stamp = OwnershipStamp::standalone(&registry, &mut births);
    let (handle, _receiver) = TunnelHandle::new(
        "metadata-refund-fixture".into(),
        ios_core::TunnelInfo {
            client_address: "fd00::1".into(),
            server_address: "fd00::2".into(),
            client_mtu: 1280,
            server_rsd_port: 58783,
        },
        None,
    );
    let handle = Arc::new(handle);
    let lease = registry
        .reserve(stamp.clone())
        .unwrap()
        .attach(handle.clone());
    let key = lease.key.clone();
    let (metadata, global_metadata) = metadata_lease::PreparedPair::new(&global, &published)
        .unwrap()
        .finish(
            admission::known_metadata_charge(&global, 7).unwrap(),
            admission::known_metadata_charge(&published, 11).unwrap(),
        )
        .unwrap();
    lease.retain_metadata(metadata.clone()).unwrap();
    lease
        .retain_global_metadata(global_metadata.clone())
        .unwrap();
    lease.publish(&stamp).unwrap();
    drop((metadata, global_metadata, lease));
    // Sender-only fixture: no native IO exists, and real empty joins complete.
    // This tests the actual metadata destructors and semaphore wake boundary.
    handle.shutdown(Duration::from_secs(1)).await.unwrap();
    assert!(!handle.has_pending_native_cleanup());
    let held = published
        .clone()
        .try_acquire_many_owned(published.available_permits() as u32)
        .unwrap();
    let observed = Arc::new(RegistryWake {
        registry: registry.clone(),
        calls: AtomicUsize::new(0),
        unlocked: AtomicBool::new(true),
    });
    let waker = Waker::from(observed.clone());
    let mut cx = Context::from_waker(&waker);
    let mut waiter = std::pin::pin!(published.clone().acquire_owned());
    assert!(waiter.as_mut().poll(&mut cx).is_pending());
    assert!(registry.certify_completed(&key, &handle));
    assert!(observed.calls.load(Ordering::SeqCst) > 0);
    assert!(
        observed.unlocked.load(Ordering::SeqCst),
        "completed native metadata refunded under the Registry mutex"
    );
    let permit = match waiter.as_mut().poll(&mut cx) {
        Poll::Ready(Ok(permit)) => permit,
        _ => panic!("actual completed metadata fees did not wake the waiter"),
    };
    assert_eq!(registry.snapshot().occupied(), 0);
    drop((permit, held));
    assert_eq!(published.available_permits(), 65536);
}

#[test]
fn empty_entry_marker_refund_wakes_public_original_bank_after_registry_guard_release() {
    let registry = Registry::small(NonZeroUsize::new(1).unwrap());
    // Obtain the supported original byte bank through the real public trait
    // adapter. Neither this probe nor the target entry creates a native handle.
    let bank = {
        let scope = Ownership::standalone(registry.clone()).unwrap();
        let attempt = scope.reserve_owned().unwrap();
        let bank = attempt.byte_bank().unwrap();
        assert!(Arc::ptr_eq(&bank, &registry.byte_bank()));
        drop(attempt);
        drop(scope);
        bank
    };
    assert_eq!(registry.snapshot().occupied(), 0);
    let before = bank.available_permits();
    let marker_bytes = 2 * ControlIdentity::body_bytes();
    let mut births = registry.prepare_births::<2>(&bank).unwrap();
    let stamp = OwnershipStamp::standalone(&registry, &mut births);
    let reservation = registry.reserve(stamp).unwrap();
    drop(births);
    // Entry is the sole strong owner of both target markers. There is no
    // observer Weak or context keeper which could mask the real refund point.
    assert_eq!(
        bank.available_permits(),
        before - marker_bytes - Registry::incarnation_bytes()
    );
    let held = bank
        .clone()
        .try_acquire_many_owned(bank.available_permits() as u32)
        .unwrap();
    assert!(!bank.is_closed());
    assert_eq!(bank.available_permits(), 0);
    let observed = Arc::new(RegistryWake {
        registry: registry.clone(),
        calls: AtomicUsize::new(0),
        unlocked: AtomicBool::new(true),
    });
    let waker = Waker::from(observed.clone());
    let mut cx = Context::from_waker(&waker);
    let mut waiter = std::pin::pin!(bank.clone().acquire_many_owned(marker_bytes as u32));
    assert!(waiter.as_mut().poll(&mut cx).is_pending());
    metadata_vec::with_allocation_hook(
        |_, _| panic!("empty entry retirement allocated a backing"),
        || drop(reservation),
    );
    assert!(observed.calls.load(Ordering::SeqCst) > 0);
    assert!(observed.unlocked.load(Ordering::SeqCst));
    assert_eq!(registry.snapshot().occupied(), 0);
    let permit = match waiter.as_mut().poll(&mut cx) {
        Poll::Ready(Ok(permit)) => permit,
        _ => panic!("the two actual entry marker refunds did not satisfy the waiter"),
    };
    assert_eq!(permit.num_permits(), marker_bytes);
    assert_eq!(bank.available_permits(), Registry::incarnation_bytes());
    drop((permit, held));
    assert_eq!(bank.available_permits(), before);
    drop((observed, waker, registry));
    assert_eq!(
        bank.available_permits(),
        super::super::MAX_GLOBAL_KNOWN_METADATA_BYTES
    );
}
