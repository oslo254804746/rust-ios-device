use super::*;
use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Mutex as StdMutex,
};
use tokio::io::AsyncReadExt;

const TOTAL: usize = super::super::MAX_GLOBAL_KNOWN_METADATA_BYTES;
const OBSERVE: Duration = Duration::from_secs(2);

#[test]
fn foreground_root_short_and_closed_refusal_precedes_scope_allocation_or_registry_close() {
    for closed in [false, true] {
        let bank = Arc::new(Semaphore::new(TOTAL));
        let registry = Registry::with_metadata(&bank).unwrap();
        let leave = if closed {
            0
        } else {
            InnerOwner::allocation_bytes() + SlotsOwner::allocation_bytes() - 1
        };
        let held = bank
            .clone()
            .try_acquire_many_owned((bank.available_permits() - leave) as u32)
            .unwrap();
        if closed {
            bank.close();
        }
        let _hook = super::super::metadata_vec::on_allocation(|_, _| {
            panic!("unadmitted foreground root allocated");
        });
        let error = NativeScope::from_registry(registry.clone())
            .err()
            .expect("unpaid scope root accepted");
        assert_eq!(
            crate::output::operation_error_class(error.as_ref()),
            ios_core::ErrorClass::ResourceLimit
        );
        assert_eq!(bank.available_permits(), leave);
        assert!(!registry.snapshot().sealed);
        assert_eq!(registry.snapshot().occupied(), 0);
        assert_eq!(registry.strong_count(), 1);
        drop(held);
        drop(registry);
        assert_eq!(bank.available_permits(), TOTAL);
    }
}

#[test]
fn foreground_root_exact_original_bank_and_same_shared_body_survive_clones_until_last_free() {
    let bank = Arc::new(Semaphore::new(TOTAL));
    let registry = Registry::with_metadata(&bank).unwrap();
    let before = bank.available_permits();
    let inner_bytes = InnerOwner::allocation_bytes();
    let slots_bytes = SlotsOwner::allocation_bytes();
    let bytes = inner_bytes + slots_bytes;
    let observed = Arc::new(StdMutex::new(Vec::new()));
    let captured = observed.clone();
    let charged = bank.clone();
    let held = bank
        .clone()
        .try_acquire_many_owned((before - bytes) as u32)
        .unwrap();
    let _hook = super::super::metadata_vec::on_allocation(move |layout, pointer| {
        assert!(layout.size() == inner_bytes || layout.size() == slots_bytes);
        assert_eq!(charged.available_permits(), 0);
        captured.lock().unwrap().push(pointer);
    });
    let scope = NativeScope::from_registry(registry.clone()).unwrap();
    drop(_hook);
    let pointer = observed.lock().unwrap()[0];
    assert_eq!(observed.lock().unwrap().len(), 2);
    let body = &*scope.0 as *const Inner as usize;
    assert!(body >= pointer && body + std::mem::size_of::<Inner>() <= pointer + inner_bytes);
    assert!(Arc::ptr_eq(&scope.0.registry.byte_bank(), &bank));
    let second = scope.clone();
    let third = second.clone();
    assert!(scope.0.ptr_eq(&second.0) && second.0.ptr_eq(&third.0));
    assert_eq!(scope.0.strong_count(), 3);
    bank.close();
    drop(scope);
    drop(second);
    assert_eq!(third.0.strong_count(), 1);
    assert_eq!(bank.available_permits(), 0);
    assert!(!registry.snapshot().sealed);
    let freed = Arc::new(AtomicUsize::new(0));
    let count = freed.clone();
    let original = registry.clone();
    let charged = bank.clone();
    super::super::shared_allocation::after_free(
        move || {
            assert_eq!(
                charged.available_permits(),
                0,
                "root fee returned before physical free"
            );
            let previous = count.fetch_add(1, Ordering::SeqCst);
            assert_eq!(
                original.snapshot().sealed,
                previous == 1,
                "Inner root must free before its Drop, then slots root frees after close"
            );
        },
        || drop(third),
    );
    assert_eq!(freed.load(Ordering::SeqCst), 2);
    assert!(registry.snapshot().closed_and_empty());
    assert_eq!(bank.available_permits(), bytes);
    drop(held);
    drop(registry);
    assert_eq!(bank.available_permits(), TOTAL);
}

#[tokio::test]
async fn foreground_root_actual_cleanup_failure_retains_scope_and_original_native_until_error_drop()
{
    let expected = tokio::runtime::Handle::current()
        .metrics()
        .num_alive_tasks();
    let bank = Arc::new(Semaphore::new(TOTAL));
    let registry = Registry::with_metadata(&bank).unwrap();
    let scope = NativeScope::from_registry(registry.clone()).unwrap();
    let ownership = Ownership::standalone(registry.clone()).unwrap();
    let mut reservation = ownership.reserve_owned().unwrap();
    let (stream, mut peer) = tokio::io::duplex(1280);
    let runtime = ios_core::UserspaceTunDevice::start("fd00::2", "fd00::1", 1280, stream)
        .await
        .unwrap();
    let handle = Arc::new(TunnelHandle::new_userspace(
        "foreground-root-fixture".into(),
        ios_core::TunnelInfo {
            server_address: "fd00::1".into(),
            server_rsd_port: 58783,
            client_address: "fd00::2".into(),
            client_mtu: 1280,
        },
        runtime,
    ));
    reservation.attach(handle.clone()).unwrap();
    reservation.complete();
    drop(reservation);
    let lease = ownership.take_completed(&handle).unwrap();
    drop(ownership);
    let error = scope
        .close(Duration::ZERO)
        .await
        .expect_err("zero cleanup unexpectedly joined pending native workers");
    let retained = error
        .chain()
        .find_map(|cause| cause.downcast_ref::<RetainedScopeFailure>())
        .expect("original retained scope missing");
    assert!(retained.scope.0.ptr_eq(&scope.0));
    assert!(lease.matches_runtime(&handle));
    assert!(!handle.is_alive());
    assert_eq!(scope.0.strong_count(), 2);
    let before = bank.available_permits();
    drop(scope);
    assert_eq!(
        bank.available_permits(),
        before,
        "cleanup error lost the original scope fee"
    );
    bank.close();
    drop(error);
    assert_eq!(
        bank.available_permits(),
        before + InnerOwner::allocation_bytes() + SlotsOwner::allocation_bytes()
    );
    let clock = CleanupClock::new(OBSERVE).unwrap();
    registry.drain_retired(clock).await.unwrap();
    assert!(registry.snapshot().closed_and_empty());
    assert!(handle.shutdown_status().is_ok());
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
    drop(lease);
    drop(registry);
    assert_eq!(bank.available_permits(), TOTAL);
}
