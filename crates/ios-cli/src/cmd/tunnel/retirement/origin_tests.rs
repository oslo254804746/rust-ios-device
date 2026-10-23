use super::*;
use std::sync::atomic::{AtomicUsize, Ordering};
use tokio::io::{AsyncReadExt, DuplexStream};

const TOTAL: usize = super::super::MAX_GLOBAL_KNOWN_METADATA_BYTES;
const OBSERVE: Duration = Duration::from_secs(2);

fn registry() -> RegistryOwner {
    Registry::small(NonZeroUsize::new(1).unwrap())
}
fn stamp(bank: &Arc<Semaphore>) -> OwnershipStamp {
    OwnershipStamp::new(ControlIdentity::new(bank), ControlIdentity::new(bank))
}

#[test]
fn native_origin_historical_producers_outnumber_current_slots_and_reuse_keeps_whole_fee() {
    let registry = registry();
    let bank = registry.byte_bank();
    let mirror = registry._slots_storage.num_permits();
    let before = bank.available_permits();
    let mut producers = Vec::new();
    for _ in 0..3 {
        let producer = Arc::new(Semaphore::new(mirror));
        drop(registry.reserve(stamp(&producer)).unwrap());
        assert_eq!(registry.snapshot().occupied(), 0);
        assert_eq!(producer.available_permits(), 0);
        producer.close();
        // The original whole-backing mirror is already funded, even though
        // this different actual epoch is created after producer bank closure.
        drop(registry.reserve(stamp(&producer)).unwrap());
        assert_eq!(producer.available_permits(), 0);
        producers.push(producer);
    }
    assert_eq!(registry.lock().issued, 6);
    assert_eq!(registry.snapshot().capacity, 1);
    let catalogue = registry.origins.lock().unwrap().backing_bytes();
    assert_eq!(catalogue, 4 * std::mem::size_of::<OwnedSemaphorePermit>());
    assert_eq!(bank.available_permits(), before - catalogue);
    drop(registry);
    assert!(producers
        .iter()
        .all(|producer| producer.available_permits() == mirror));
    assert_eq!(bank.available_permits(), TOTAL);
}

#[test]
fn native_origin_growth_refusal_keeps_old_catalogue_and_success_pays_both_actual_backings() {
    let registry = registry();
    let bank = registry.byte_bank();
    let mirror = registry._slots_storage.num_permits();
    let first = Arc::new(Semaphore::new(mirror));
    let second = Arc::new(Semaphore::new(mirror));
    drop(registry.reserve(stamp(&first)).unwrap());
    drop(registry.reserve(stamp(&second)).unwrap());
    let old_bytes = 2 * std::mem::size_of::<OwnedSemaphorePermit>();
    let next_bytes = 2 * old_bytes;
    assert_eq!(registry.origins.lock().unwrap().backing_bytes(), old_bytes);
    let third = Arc::new(Semaphore::new(mirror));
    let leave = Registry::incarnation_bytes() + next_bytes - 1;
    let held = bank
        .clone()
        .try_acquire_many_owned((bank.available_permits() - leave) as u32)
        .unwrap();
    let error = registry.reserve(stamp(&third)).err().unwrap();
    assert_eq!(
        crate::output::operation_error_class(error.as_ref()),
        ios_core::ErrorClass::ResourceLimit
    );
    assert_eq!(bank.available_permits(), leave);
    assert_eq!(third.available_permits(), mirror);
    assert_eq!(first.available_permits(), 0);
    assert_eq!(second.available_permits(), 0);
    assert_eq!(registry.lock().issued, 2);
    assert_eq!(registry.snapshot().occupied(), 0);
    assert_eq!(registry.origins.lock().unwrap().backing_bytes(), old_bytes);
    drop(held);
    let before = bank.available_permits(); // Includes the still-paid old Vec.
    let calls = Arc::new(AtomicUsize::new(0));
    let observed = calls.clone();
    let charged = bank.clone();
    let origin = third.clone();
    let old_first = first.clone();
    let old_second = second.clone();
    let hook = super::super::metadata_vec::on_allocation(move |layout, _| {
        let previous = observed.fetch_add(1, Ordering::SeqCst);
        assert_eq!(
            layout.size(),
            if previous == 0 {
                Registry::incarnation_bytes()
            } else {
                next_bytes
            }
        );
        assert_eq!(
            charged.available_permits(),
            before - Registry::incarnation_bytes() - if previous == 0 { 0 } else { next_bytes }
        );
        assert_eq!(
            origin.available_permits(),
            if previous == 0 { mirror } else { 0 }
        );
        assert_eq!(old_first.available_permits(), 0);
        assert_eq!(old_second.available_permits(), 0);
    });
    let reservation = registry.reserve(stamp(&third)).unwrap();
    drop(hook);
    assert_eq!(calls.load(Ordering::SeqCst), 2);
    assert_eq!(
        bank.available_permits(),
        before - Registry::incarnation_bytes() - next_bytes + old_bytes
    );
    drop(reservation);
    assert_eq!(registry.origins.lock().unwrap().backing_bytes(), next_bytes);
    assert_eq!(third.available_permits(), 0);
    drop(registry);
    assert_eq!(
        (
            first.available_permits(),
            second.available_permits(),
            third.available_permits()
        ),
        (mirror, mirror, mirror)
    );
    assert_eq!(bank.available_permits(), TOTAL);
}

#[test]
fn native_origin_mixed_two_producers_roll_back_together_and_original_capacity_still_refuses_first()
{
    let registry = registry();
    let bank = registry.byte_bank();
    let before = bank.available_permits();
    let mirror = registry._slots_storage.num_permits();
    let first = Arc::new(Semaphore::new(mirror));
    let second = Arc::new(Semaphore::new(mirror));
    let held = second.clone().try_acquire_owned().unwrap();
    let mixed = || OwnershipStamp::new(ControlIdentity::new(&first), ControlIdentity::new(&second));
    assert!(registry.reserve(mixed()).is_err());
    assert_eq!(first.available_permits(), mirror);
    assert_eq!(second.available_permits(), mirror - 1);
    assert_eq!(bank.available_permits(), before);
    assert_eq!(registry.lock().issued, 0);
    assert!(registry.origins.lock().unwrap().is_empty());
    drop(held);
    let reservation = registry.reserve(mixed()).unwrap();
    assert_eq!(
        (first.available_permits(), second.available_permits()),
        (0, 0)
    );
    let denied = Arc::new(Semaphore::new(0));
    denied.close();
    let hook = super::super::metadata_vec::on_allocation(|_, _| {
        panic!("capacity refusal allocated a new owner")
    });
    let error = registry.reserve(stamp(&denied)).err().unwrap();
    drop(hook);
    assert_eq!(error.to_string(), "native tunnel owner capacity exhausted");
    assert_eq!(registry.lock().issued, 1);
    drop(reservation);
    assert_eq!(
        (first.available_permits(), second.available_permits()),
        (0, 0)
    );
    drop(registry);
    assert_eq!(
        (first.available_permits(), second.available_permits()),
        (mirror, mirror)
    );
}

async fn native() -> (Arc<TunnelHandle>, DuplexStream) {
    let (stream, peer) = tokio::io::duplex(1280);
    let runtime = ios_core::UserspaceTunDevice::start("fd00::2", "fd00::1", 1280, stream)
        .await
        .unwrap();
    (
        Arc::new(TunnelHandle::new_userspace(
            "native-origin-proof".into(),
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
fn metadata(
    bank: &Arc<Semaphore>,
    publication: &Arc<Semaphore>,
) -> (
    super::super::metadata_lease::Owner,
    super::super::metadata_lease::Owner,
) {
    super::super::metadata_lease::PreparedPair::new(bank, publication)
        .unwrap()
        .finish(
            super::super::admission::known_metadata_charge(bank, 0).unwrap(),
            super::super::admission::known_metadata_charge(publication, 0).unwrap(),
        )
        .unwrap()
}
async fn join(
    registry: &Registry,
    handle: &TunnelHandle,
    peer: &mut DuplexStream,
    expected: usize,
) {
    registry
        .drain_retired(CleanupClock::new(OBSERVE).unwrap())
        .await
        .unwrap();
    assert_eq!(registry.snapshot().occupied(), 0);
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
}

#[tokio::test]
async fn native_origin_foreign_proofs_pay_independently_after_slot_reuse_and_original_native_joins()
{
    for distinct in [false, true] {
        let expected = tokio::runtime::Handle::current()
            .metrics()
            .num_alive_tasks();
        let registry = registry();
        let bank = registry.byte_bank();
        let mirror = registry._slots_storage.num_permits();
        let proof_bytes = std::mem::size_of::<RefreshProof>();
        let total = mirror + 2 * proof_bytes;
        let origin = Arc::new(Semaphore::new(total));
        let other = if distinct {
            Arc::new(Semaphore::new(total))
        } else {
            origin.clone()
        };
        let make_stamp =
            || OwnershipStamp::new(ControlIdentity::new(&origin), ControlIdentity::new(&other));
        let destination = Arc::new(Semaphore::new(2 * proof_bytes));
        let publication = Arc::new(Semaphore::new(65536));
        let first_stamp = make_stamp();
        let (first, mut first_peer) = native().await;
        let first_lease = registry
            .reserve(first_stamp.clone())
            .unwrap()
            .attach(first.clone());
        let (first_metadata, first_global) = metadata(&bank, &publication);
        first_lease.retain_metadata(first_metadata.clone()).unwrap();
        first_lease
            .retain_global_metadata(first_global.clone())
            .unwrap();
        first_lease.publish(&first_stamp).unwrap();
        let first_proof = first_lease
            .capture_refresh(
                first_stamp.clone(),
                &first,
                &first_metadata,
                &first_global,
                None,
                &destination,
            )
            .unwrap();
        assert!(Arc::ptr_eq(&first_proof.handle, &first));
        assert!(first_proof.metadata.ptr_eq(&first_metadata));
        assert!(first_proof.global_metadata.ptr_eq(&first_global));
        assert_eq!(origin.available_permits(), proof_bytes);
        let held = origin
            .clone()
            .try_acquire_many_owned(proof_bytes as u32)
            .unwrap();
        let refused_destination = Arc::new(Semaphore::new(proof_bytes));
        let error = first_lease
            .capture_refresh(
                first_stamp.clone(),
                &first,
                &first_metadata,
                &first_global,
                None,
                &refused_destination,
            )
            .err()
            .unwrap();
        assert_eq!(
            crate::output::operation_error_class(error.as_ref()),
            ios_core::ErrorClass::ResourceLimit
        );
        assert_eq!(origin.available_permits(), 0);
        assert_eq!(refused_destination.available_permits(), proof_bytes);
        assert_eq!(registry.snapshot().active, 1);
        assert!(first.is_alive());
        first_proof.check().unwrap();
        drop(held);
        if distinct {
            assert_eq!(other.available_permits(), proof_bytes);
            let held_other = other
                .clone()
                .try_acquire_many_owned(proof_bytes as u32)
                .unwrap();
            let before = refused_destination.available_permits();
            let error = first_lease
                .capture_refresh(
                    first_stamp.clone(),
                    &first,
                    &first_metadata,
                    &first_global,
                    None,
                    &refused_destination,
                )
                .err()
                .unwrap();
            assert_eq!(
                crate::output::operation_error_class(error.as_ref()),
                ios_core::ErrorClass::ResourceLimit
            );
            // A was successfully staged before the second actual producer refused.
            assert_eq!(origin.available_permits(), proof_bytes);
            assert_eq!(other.available_permits(), 0);
            assert_eq!(refused_destination.available_permits(), before);
            assert!(first.is_alive());
            first_proof.check().unwrap();
            drop(held_other);
        }
        first_lease.retire();
        drop(first_lease);
        join(&registry, &first, &mut first_peer, expected).await;
        drop(first_stamp);

        let second_stamp = make_stamp();
        let (second, mut second_peer) = native().await;
        let second_lease = registry
            .reserve(second_stamp.clone())
            .unwrap()
            .attach(second.clone());
        let (second_metadata, second_global) = metadata(&bank, &publication);
        second_lease
            .retain_metadata(second_metadata.clone())
            .unwrap();
        second_lease
            .retain_global_metadata(second_global.clone())
            .unwrap();
        second_lease.publish(&second_stamp).unwrap();
        let second_proof = second_lease
            .capture_refresh(
                second_stamp.clone(),
                &second,
                &second_metadata,
                &second_global,
                None,
                &destination,
            )
            .unwrap();
        assert!(!first_proof.stamp.global.ptr_eq(&second_proof.stamp.global));
        assert!(!first_proof.stamp.device.ptr_eq(&second_proof.stamp.device));
        assert_eq!(origin.available_permits(), 0);
        origin.close();
        other.close();
        second_lease.retire();
        drop(second_lease);
        join(&registry, &second, &mut second_peer, expected).await;
        drop(second_stamp);
        drop((first_metadata, first_global, second_metadata, second_global));
        drop(registry);
        assert_eq!(origin.available_permits(), 0);
        drop(first_proof);
        assert_eq!(origin.available_permits(), proof_bytes);
        let weak = second_proof.stamp.global.downgrade();
        let observer = origin.clone();
        let other_observer = other.clone();
        let frees = Arc::new(AtomicUsize::new(0));
        let calls = frees.clone();
        super::super::shared_allocation::after_free(
            move || {
                let previous = calls.fetch_add(1, Ordering::SeqCst);
                // Original registry/key/stamp Drop order is unchanged. After B's
                // body+two report roots free, its mirror returns; the original Key
                // then frees BEFORE stamp. The proof's own A fee is still held.
                assert_eq!(
                    observer.available_permits(),
                    if previous < 3 {
                        proof_bytes
                    } else {
                        total - proof_bytes
                    }
                );
                assert_eq!(
                    other_observer.available_permits(),
                    if previous < 3 {
                        proof_bytes
                    } else {
                        total - proof_bytes
                    }
                );
                assert!(weak.upgrade().is_some());
            },
            || drop(second_proof),
        );
        assert_eq!(frees.load(Ordering::SeqCst), 4);
        assert_eq!(origin.available_permits(), total);
        assert_eq!(other.available_permits(), total);
        assert_eq!(destination.available_permits(), 2 * proof_bytes);
        assert_eq!(publication.available_permits(), 65536);
        assert_eq!(bank.available_permits(), TOTAL);
        assert!(first.shutdown_status().is_ok() && second.shutdown_status().is_ok());
    }
}
