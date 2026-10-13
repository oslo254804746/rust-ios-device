use super::*;

#[test]
fn registry_metadata_exact_body_capacity_and_final_registry_owner_release() {
    let bank = Arc::new(Semaphore::new(65536));
    let registry = Registry::allocate(&bank, 3).unwrap();
    let state = registry.lock();
    let actual = std::mem::size_of::<Registry>()
        + state.slots.capacity() * std::mem::size_of::<Option<Entry>>();
    assert_eq!(state.slots.len(), 3);
    assert!(state.slots.iter().all(Option::is_none));
    drop(state);
    assert_eq!(registry.structural_bytes(), actual);
    assert_eq!(bank.available_permits(), 65536 - actual);
    let last = Arc::clone(&registry);
    drop(registry);
    assert_eq!(bank.available_permits(), 65536 - actual);
    drop(last);
    assert_eq!(bank.available_permits(), 65536);
}

#[test]
fn registry_metadata_one_byte_refusal_refunds_body_and_every_partial_backing() {
    let probe = Registry::allocate(&Arc::new(Semaphore::new(65536)), 2).unwrap();
    let bytes = probe.structural_bytes();
    drop(probe);
    for available in [std::mem::size_of::<Registry>() - 1, bytes - 1] {
        let bank = Arc::new(Semaphore::new(available));
        let error = Registry::allocate(&bank, 2).err().unwrap();
        assert_eq!(
            crate::output::operation_error_class(error.as_ref()),
            ErrorClass::ResourceLimit
        );
        assert_eq!(bank.available_permits(), available);
    }
    let bank = Arc::new(Semaphore::new(bytes));
    let exact = Registry::allocate(&bank, 2).unwrap();
    assert_eq!(bank.available_permits(), 0);
    drop(exact);
    assert_eq!(bank.available_permits(), bytes);
}

#[test]
fn registry_metadata_original_reservation_keeps_structural_bytes_after_external_registry_drop() {
    let bank = Arc::new(Semaphore::new(65536));
    let registry = Registry::allocate(&bank, 1).unwrap();
    let bytes = registry.structural_bytes();
    let reservation = registry
        .reserve(OwnershipStamp::new(Arc::new(()), Arc::new(())))
        .unwrap();
    assert_eq!(registry.snapshot().reserved, 1);
    drop(registry);
    assert_eq!(bank.available_permits(), 65536 - bytes);
    drop(reservation);
    assert_eq!(bank.available_permits(), 65536);
}

#[test]
fn registry_metadata_rebind_is_atomic_and_preserves_original_slots_and_namespace() {
    let old_bank = Arc::new(Semaphore::new(65536));
    let mut registry = Registry::allocate(&old_bank, 2).unwrap();
    let bytes = registry.structural_bytes();
    let pointer = registry.lock().slots.as_ptr();
    let denied = Arc::new(Semaphore::new(bytes - 1));
    assert!(Arc::get_mut(&mut registry)
        .unwrap()
        .rebind_empty_metadata(&denied)
        .is_err());
    assert_eq!(old_bank.available_permits(), 65536 - bytes);
    assert_eq!(denied.available_permits(), bytes - 1);
    assert_eq!(registry.lock().slots.as_ptr(), pointer);
    let next = Arc::new(Semaphore::new(bytes));
    Arc::get_mut(&mut registry)
        .unwrap()
        .rebind_empty_metadata(&next)
        .unwrap();
    assert_eq!(old_bank.available_permits(), 65536);
    assert_eq!(next.available_permits(), 0);
    assert_eq!(registry.lock().slots.as_ptr(), pointer);
    registry.close_and_stop_all();
    assert!(Arc::get_mut(&mut registry)
        .unwrap()
        .rebind_empty_metadata(&old_bank)
        .is_err());
    assert_eq!(next.available_permits(), 0);
    assert_eq!(old_bank.available_permits(), 65536);
    drop(registry);
    assert_eq!(next.available_permits(), bytes);
}
