use super::*;
use std::sync::atomic::{AtomicUsize, Ordering};

fn bank() -> crate::cmd::tunnel::birth_bank::BirthBank {
    crate::cmd::tunnel::birth_bank::BirthBank::new(super::super::MAX_GLOBAL_KNOWN_METADATA_BYTES)
}

#[test]
fn attempt_storage_first_short_and_closed_refuse_before_native_reservation_or_backing() {
    for closed in [false, true] {
        let bank_birth = bank();
        let bank = bank_birth.raw().clone();
        let registry = Registry::with_metadata(&bank_birth).unwrap();
        let bytes = Ownership::attempt_bytes();
        let leave = if closed { 0 } else { bytes - 1 };
        let held = bank
            .clone()
            .try_acquire_many_owned((bank.available_permits() - leave) as u32)
            .unwrap();
        if closed {
            bank.close();
        }
        let allocations = Arc::new(AtomicUsize::new(0));
        let observed = allocations.clone();
        let _hook = super::super::metadata_vec::on_allocation(move |_, _| {
            observed.fetch_add(1, Ordering::SeqCst);
        });
        let error = Ownership::standalone(registry.clone())
            .err()
            .expect("unpaid Attempt accepted");
        assert_eq!(
            crate::output::operation_error_class(error.as_ref()),
            ios_core::ErrorClass::ResourceLimit
        );
        assert_eq!(allocations.load(Ordering::SeqCst), 0);
        assert_eq!(registry.snapshot().occupied(), 0);
        assert_eq!(bank.available_permits(), leave);
        drop(held);
        drop(registry);
        assert_eq!(
            bank.available_permits(),
            super::super::MAX_GLOBAL_KNOWN_METADATA_BYTES
        );
    }
}

#[test]
fn attempt_storage_prepaid_first_uses_exact_original_box_at_closed_zero_then_refunds() {
    let bank_birth = bank();
    let bank = bank_birth.raw().clone();
    let registry = Registry::with_metadata(&bank_birth).unwrap();
    let scope = Ownership::standalone(registry.clone()).unwrap();
    let held = bank
        .clone()
        .try_acquire_many_owned(bank.available_permits() as u32)
        .unwrap();
    bank.close();
    let pointer = scope
        .initial_attempt
        .lock()
        .unwrap()
        .as_ref()
        .unwrap()
        .values
        .as_ptr() as usize;
    let bytes = Ownership::attempt_bytes();
    let _hook = super::super::metadata_vec::on_allocation(move |_, _| {
        panic!("prepaid first Attempt allocated again");
    });
    let reservation = scope.reserve_owned().unwrap();
    assert_eq!(&*reservation as *const _ as *const () as usize, pointer);
    assert_eq!(registry.snapshot().inflight, 1);
    assert_eq!(bank.available_permits(), 0);
    drop(reservation);
    assert_eq!(registry.snapshot().occupied(), 0);
    assert_eq!(
        bank.available_permits(),
        bytes + Registry::incarnation_bytes()
    );
    drop(scope);
    assert_eq!(
        bank.available_permits(),
        bytes + Registry::incarnation_bytes()
    );
    drop(held);
    drop(registry);
    assert_eq!(
        bank.available_permits(),
        super::super::MAX_GLOBAL_KNOWN_METADATA_BYTES
    );
}

#[test]
fn attempt_storage_additional_short_refuses_before_slot_and_each_freed_box_refunds() {
    let bank_birth = bank();
    let bank = bank_birth.raw().clone();
    let registry = Registry::with_metadata(&bank_birth).unwrap();
    let scope = Ownership::standalone(registry.clone()).unwrap();
    let first = scope.reserve_owned().unwrap();
    let first_pointer = &*first as *const _ as *const () as usize;
    let before = bank.available_permits();
    let second = scope.reserve_owned().unwrap();
    assert_ne!(&*second as *const _ as *const () as usize, first_pointer);
    let bytes = Ownership::attempt_bytes() + Registry::incarnation_bytes();
    assert_eq!(bank.available_permits(), before - bytes);
    drop(second);
    assert_eq!(bank.available_permits(), before);
    let leave = Ownership::attempt_bytes() - 1;
    let held = bank
        .clone()
        .try_acquire_many_owned((bank.available_permits() - leave) as u32)
        .unwrap();
    let allocations = Arc::new(AtomicUsize::new(0));
    let observed = allocations.clone();
    let _hook = super::super::metadata_vec::on_allocation(move |_, _| {
        observed.fetch_add(1, Ordering::SeqCst);
    });
    let error = scope
        .reserve_owned()
        .err()
        .expect("additional unpaid Attempt accepted");
    let error: anyhow::Error = error.into();
    assert_eq!(
        crate::output::operation_error_class(error.as_ref()),
        ios_core::ErrorClass::ResourceLimit
    );
    assert_eq!(allocations.load(Ordering::SeqCst), 0);
    assert_eq!(registry.snapshot().inflight, 1);
    assert_eq!(&*first as *const _ as *const () as usize, first_pointer);
    assert_eq!(bank.available_permits(), leave);
    drop(first);
    assert_eq!(registry.snapshot().occupied(), 0);
    assert_eq!(bank.available_permits(), leave + bytes);
    drop(scope);
    drop(held);
    drop(registry);
    assert_eq!(
        bank.available_permits(),
        super::super::MAX_GLOBAL_KNOWN_METADATA_BYTES
    );
}

#[test]
fn attempt_storage_legacy_original_box_keeps_fee_in_borrowed_scope_until_scope_drop() {
    let bank_birth = bank();
    let bank = bank_birth.raw().clone();
    let registry = Registry::with_metadata(&bank_birth).unwrap();
    let baseline = bank.available_permits();
    let scope = Ownership::standalone(registry.clone()).unwrap();
    let first = scope.reserve().unwrap();
    let second = scope.reserve().unwrap();
    let bytes = Ownership::attempt_bytes();
    let root_bytes = LegacyStorageOwner::allocation_bytes();
    assert_eq!(
        bank.available_permits(),
        baseline - 2 * (bytes + Registry::incarnation_bytes()) - root_bytes
    );
    assert_eq!(
        scope
            .legacy_attempts
            .lock()
            .unwrap()
            .as_ref()
            .unwrap()
            .strong_count(),
        3,
        "scope and both original Boxes must retain the same whole fee root"
    );
    drop(first);
    assert_eq!(
        scope
            .legacy_attempts
            .lock()
            .unwrap()
            .as_ref()
            .unwrap()
            .strong_count(),
        2
    );
    drop(second);
    assert_eq!(
        scope
            .legacy_attempts
            .lock()
            .unwrap()
            .as_ref()
            .unwrap()
            .strong_count(),
        1
    );
    assert_eq!(registry.snapshot().occupied(), 0);
    assert_eq!(bank.available_permits(), baseline - 2 * bytes - root_bytes);
    bank.close();
    drop(scope);
    assert_eq!(bank.available_permits(), baseline);
    drop(registry);
    assert_eq!(
        bank.available_permits(),
        super::super::MAX_GLOBAL_KNOWN_METADATA_BYTES
    );
}

#[tokio::test]
async fn attempt_storage_ticket_first_admission_and_stale_reserve_keep_original_fence() {
    let bank_birth = bank();
    let bank = bank_birth.raw().clone();
    let lifecycle = super::super::lifecycle::Lifecycle::with_metadata(bank_birth.clone()).unwrap();
    let registry = Registry::with_metadata(&bank_birth).unwrap();
    let ticket = lifecycle.ticket("attempt-storage-fixture").await.unwrap();
    let leave = Ownership::attempt_bytes() - 1;
    let held = bank
        .clone()
        .try_acquire_many_owned((bank.available_permits() - leave) as u32)
        .unwrap();
    let _hook = super::super::metadata_vec::on_allocation(|_, _| {
        panic!("short pre-Source Attempt allocated");
    });
    let error = Ownership::admitted(registry.clone(), &ticket)
        .err()
        .expect("unpaid pre-Source owner accepted");
    assert!(error.is::<Admission>());
    assert_eq!(
        crate::output::operation_error_class(error.as_ref()),
        ios_core::ErrorClass::ResourceLimit
    );
    assert_eq!(registry.snapshot().occupied(), 0);
    assert_eq!(bank.available_permits(), leave);
    ticket.commit(|| Ok(())).unwrap();
    drop(_hook);
    drop(held);
    let scope = Ownership::admitted(registry.clone(), &ticket).unwrap();
    let pointer = scope
        .initial_attempt
        .lock()
        .unwrap()
        .as_ref()
        .unwrap()
        .values
        .as_ptr() as usize;
    lifecycle.close();
    let held = bank
        .clone()
        .try_acquire_many_owned(bank.available_permits() as u32)
        .unwrap();
    bank.close();
    let _hook = super::super::metadata_vec::on_allocation(|_, _| {
        panic!("stale Source allocated a destination");
    });
    let error: anyhow::Error = scope
        .reserve_owned()
        .err()
        .expect("stale attempt accepted")
        .into();
    assert_eq!(
        crate::output::operation_error_class(error.as_ref()),
        ios_core::ErrorClass::Cancelled
    );
    assert_eq!(
        scope
            .initial_attempt
            .lock()
            .unwrap()
            .as_ref()
            .unwrap()
            .values
            .as_ptr() as usize,
        pointer
    );
    assert_eq!(registry.snapshot().reserved, 1);
    assert_eq!(bank.available_permits(), 0);
    drop(scope);
    assert_eq!(registry.snapshot().occupied(), 0);
    assert_eq!(
        bank.available_permits(),
        Ownership::attempt_bytes() + Registry::incarnation_bytes()
    );
    drop(ticket);
    drop(lifecycle);
    drop(held);
    drop(registry);
    assert_eq!(
        bank.available_permits(),
        super::super::MAX_GLOBAL_KNOWN_METADATA_BYTES
    );
}
