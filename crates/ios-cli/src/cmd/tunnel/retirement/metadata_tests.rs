use super::*;

fn stamp(registry: &RegistryOwner) -> OwnershipStamp {
    let mut births = registry.prepare_births::<2>(&registry.byte_bank()).unwrap();
    OwnershipStamp::standalone(registry, &mut births)
}

#[test]
fn registry_owner_closed_pool_parallel_last_roots_keep_original_body_slots_and_refund_once() {
    let total = 65536;
    let bank_birth = crate::cmd::tunnel::birth_bank::BirthBank::new(total);
    let bank = bank_birth.raw().clone();
    let mut registry = Registry::allocate(&bank_birth, 3).unwrap();
    let bytes = registry.structural_bytes();
    assert_eq!(registry.storage().num_permits(), Registry::body_bytes());
    assert!(Arc::ptr_eq(registry.storage().semaphore(), &bank));
    let alias = registry.clone();
    assert!(registry.get_mut().is_none());
    drop(alias);
    assert!(registry.get_mut().is_some());
    let pointer = &*registry as *const Registry as usize;
    let slots = registry.lock().slots.as_ptr() as usize;
    let held = bank
        .clone()
        .try_acquire_many_owned(bank.available_permits() as u32)
        .unwrap();
    bank.close();
    assert_eq!(bank.available_permits(), 0);
    let barrier = Arc::new(std::sync::Barrier::new(8));
    std::thread::scope(|scope| {
        for _ in 0..7 {
            let owner = registry.clone();
            assert!(owner.ptr_eq(&registry));
            let barrier = barrier.clone();
            scope.spawn(move || {
                assert_eq!(&*owner as *const Registry as usize, pointer);
                assert_eq!(owner.lock().slots.as_ptr() as usize, slots);
                barrier.wait();
                drop(owner);
            });
        }
        barrier.wait();
        drop(registry);
    });
    assert_eq!(bank.available_permits(), bytes);
    drop(held);
    assert_eq!(bank.available_permits(), total);
}

#[test]
fn registry_metadata_exact_body_capacity_and_final_registry_owner_release() {
    let bank_birth = crate::cmd::tunnel::birth_bank::BirthBank::new(65536);
    let bank = bank_birth.raw().clone();
    let registry = Registry::allocate(&bank_birth, 3).unwrap();
    let state = registry.lock();
    let actual = Registry::body_bytes()
        + state.slots.capacity() * std::mem::size_of::<Option<Entry>>()
        + 2 * NativeReport::body_bytes();
    assert_eq!(state.slots.len(), 3);
    assert!(state.slots.iter().all(Option::is_none));
    drop(state);
    assert_eq!(registry.structural_bytes(), actual);
    assert_eq!(bank.available_permits(), 65536 - actual);
    let last = registry.clone();
    drop(registry);
    assert_eq!(bank.available_permits(), 65536 - actual);
    drop(last);
    assert_eq!(bank.available_permits(), 65536);
}

#[test]
fn registry_metadata_one_byte_refusal_refunds_body_and_every_partial_backing() {
    let probe =
        Registry::allocate(&crate::cmd::tunnel::birth_bank::BirthBank::new(65536), 2).unwrap();
    let bytes = probe.structural_bytes();
    drop(probe);
    for available in [Registry::body_bytes() - 1, bytes - 1] {
        let bank_birth = crate::cmd::tunnel::birth_bank::BirthBank::new(available);
        let bank = bank_birth.raw().clone();
        let error = Registry::allocate(&bank_birth, 2).err().unwrap();
        assert_eq!(
            crate::output::operation_error_class(error.as_ref()),
            ErrorClass::ResourceLimit
        );
        assert_eq!(bank.available_permits(), available);
    }
    let bank_birth = crate::cmd::tunnel::birth_bank::BirthBank::new(bytes);
    let bank = bank_birth.raw().clone();
    let exact = Registry::allocate(&bank_birth, 2).unwrap();
    assert_eq!(bank.available_permits(), 0);
    drop(exact);
    assert_eq!(bank.available_permits(), bytes);
}

#[test]
fn registry_metadata_original_reservation_keeps_structural_bytes_after_external_registry_drop() {
    let bank_birth = crate::cmd::tunnel::birth_bank::BirthBank::new(65536);
    let bank = bank_birth.raw().clone();
    let registry = Registry::allocate(&bank_birth, 1).unwrap();
    let prepared_stamp_0 = stamp(&registry);
    let bytes = registry.structural_bytes();
    let reservation = registry.reserve(prepared_stamp_0).unwrap();
    assert_eq!(registry.snapshot().reserved, 1);
    drop(registry);
    assert_eq!(
        bank.available_permits(),
        65536 - bytes - Registry::incarnation_bytes() - 2 * ControlIdentity::body_bytes()
    );
    drop(reservation);
    assert_eq!(bank.available_permits(), 65536);
}

#[test]
fn registry_metadata_rebind_is_atomic_and_preserves_original_slots_and_namespace() {
    let old_bank_birth = crate::cmd::tunnel::birth_bank::BirthBank::new(65536);
    let old_bank = old_bank_birth.raw().clone();
    let mut registry = Registry::allocate(&old_bank_birth, 2).unwrap();
    let bytes = registry.structural_bytes();
    let pointer = registry.lock().slots.as_ptr();
    let denied_birth = crate::cmd::tunnel::birth_bank::BirthBank::new(bytes - 1);
    let denied = denied_birth.raw().clone();
    assert!(registry.rebind_empty_metadata(&denied_birth).is_err());
    assert_eq!(old_bank.available_permits(), 65536 - bytes);
    assert_eq!(denied.available_permits(), bytes - 1);
    assert_eq!(registry.lock().slots.as_ptr(), pointer);
    let next_birth = crate::cmd::tunnel::birth_bank::BirthBank::new(bytes);
    let next = next_birth.raw().clone();
    registry.rebind_empty_metadata(&next_birth).unwrap();
    assert_eq!(old_bank.available_permits(), 65536);
    assert_eq!(next.available_permits(), 0);
    assert_eq!(registry.lock().slots.as_ptr(), pointer);
    registry.close_and_stop_all();
    assert!(registry.rebind_empty_metadata(&old_bank_birth).is_err());
    assert_eq!(next.available_permits(), 0);
    assert_eq!(old_bank.available_permits(), 65536);
    drop(registry);
    assert_eq!(next.available_permits(), bytes);
}

#[test]
fn incarnation_whole_original_body_precedes_slot_and_ordinal_and_survives_empty_reservation() {
    let total = 65536;
    let bank_birth = crate::cmd::tunnel::birth_bank::BirthBank::new(total);
    let bank = bank_birth.raw().clone();
    let registry = Registry::allocate(&bank_birth, 1).unwrap();
    let prepared_stamp_0 = stamp(&registry);
    let prepared_stamp_1 = stamp(&registry);
    let structural = registry.structural_bytes();
    let before = bank.available_permits();
    let bytes = Registry::incarnation_bytes();
    assert_eq!(bytes, Incarnation::allocation_bytes());
    assert!(
        bytes
            >= std::mem::size_of::<std::sync::atomic::AtomicUsize>()
                + std::mem::size_of::<OwnedSemaphorePermit>()
    );
    let slots_pointer = registry.lock().slots.as_ptr();
    for leave in [0, bytes - 1] {
        let held = bank
            .clone()
            .try_acquire_many_owned((before - leave) as u32)
            .unwrap();
        let error = super::super::metadata_vec::with_allocation_hook(
            |_, _| panic!("incarnation quota refusal allocated a backing"),
            || registry.reserve(prepared_stamp_0.clone()).err().unwrap(),
        );
        assert_eq!(
            crate::output::operation_error_class(error.as_ref()),
            ErrorClass::ResourceLimit
        );
        assert_eq!(bank.available_permits(), leave);
        let state = registry.lock();
        assert_eq!((state.issued, state.retired), (0, 0));
        assert_eq!(state.slots.as_ptr(), slots_pointer);
        assert!(state.slots.iter().all(Option::is_none));
        drop(state);
        drop(held);
        assert_eq!(bank.available_permits(), before);
    }
    let held = bank
        .clone()
        .try_acquire_many_owned((before - bytes) as u32)
        .unwrap();
    let allocations = Arc::new(StdMutex::new(Vec::new()));
    let captured = allocations.clone();
    let charged = bank.clone();
    let reservation = super::super::metadata_vec::with_allocation_hook(
        move |layout, pointer| {
            assert_eq!(charged.available_permits(), 0);
            captured.lock().unwrap().push((layout, pointer));
        },
        || registry.reserve(prepared_stamp_1.clone()).unwrap(),
    );
    let key = reservation.key.as_ref().unwrap().clone();
    let allocations = allocations.lock().unwrap();
    assert_eq!(
        allocations.len(),
        1,
        "one actual complete incarnation allocation"
    );
    assert_eq!(allocations[0].0.size(), bytes);
    let pointer = &*key.incarnation as *const () as usize;
    assert!(allocations[0].1 <= pointer && pointer < allocations[0].1 + bytes);
    drop(allocations);
    assert_eq!(key.incarnation.storage().num_permits(), bytes);
    assert!(Arc::ptr_eq(key.incarnation.storage().semaphore(), &bank));
    {
        let state = registry.lock();
        assert_eq!((state.issued, state.retired), (1, 0));
        assert_eq!(state.slots.as_ptr(), slots_pointer);
        assert!(state.slots[key.slot]
            .as_ref()
            .unwrap()
            .incarnation
            .ptr_eq(&key.incarnation));
    }
    drop(reservation);
    assert_eq!(registry.snapshot().occupied(), 0);
    assert_eq!(
        registry.lock().issued,
        1,
        "empty release never refunds lifetime ordinal"
    );
    assert_eq!(key.incarnation.strong_count(), 1);
    assert_eq!(
        bank.available_permits(),
        0,
        "stale Key keeps its original full body"
    );
    drop(key);
    assert_eq!(bank.available_permits(), bytes);
    drop((prepared_stamp_0, prepared_stamp_1));
    drop(held);
    assert_eq!(bank.available_permits(), total - structural);
    drop(registry);
    assert_eq!(bank.available_permits(), total);
}

#[test]
fn incarnation_closed_pool_parallel_last_keys_keep_original_body_and_refund_once() {
    let total = 65536;
    let bank_birth = crate::cmd::tunnel::birth_bank::BirthBank::new(total);
    let bank = bank_birth.raw().clone();
    let registry = Registry::allocate(&bank_birth, 1).unwrap();
    let prepared_stamp_0 = stamp(&registry);
    let structural = registry.structural_bytes();
    let reservation = registry.reserve(prepared_stamp_0).unwrap();
    let key = reservation.key.as_ref().unwrap().clone();
    let bytes = Registry::incarnation_bytes();
    let pointer = &*key.incarnation as *const () as usize;
    drop(reservation);
    assert_eq!(registry.snapshot().occupied(), 0);
    assert_eq!(key.incarnation.strong_count(), 1);
    assert_eq!(bank.available_permits(), total - structural - bytes);
    let held = bank
        .clone()
        .try_acquire_many_owned(bank.available_permits() as u32)
        .unwrap();
    bank.close();
    assert_eq!(bank.available_permits(), 0);
    let barrier = Arc::new(std::sync::Barrier::new(8));
    std::thread::scope(|scope| {
        for _ in 0..7 {
            let owner = key.clone();
            assert!(owner.incarnation.ptr_eq(&key.incarnation));
            let bank = bank.clone();
            let barrier = barrier.clone();
            scope.spawn(move || {
                assert_eq!(&*owner.incarnation as *const () as usize, pointer);
                assert_eq!(owner.incarnation.storage().num_permits(), bytes);
                assert!(Arc::ptr_eq(owner.incarnation.storage().semaphore(), &bank));
                barrier.wait();
                drop(owner);
            });
        }
        barrier.wait();
        drop(key);
    });
    assert_eq!(bank.available_permits(), bytes);
    assert_eq!(registry.lock().issued, 1);
    assert_eq!(registry.snapshot().occupied(), 0);
    drop(registry);
    assert_eq!(bank.available_permits(), structural + bytes);
    drop(held);
    assert_eq!(bank.available_permits(), total);
}
