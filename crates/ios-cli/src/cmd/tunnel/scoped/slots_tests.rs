use super::*;
use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Mutex as StdMutex,
};

const TOTAL: usize = super::super::MAX_GLOBAL_KNOWN_METADATA_BYTES;

#[test]
fn foreground_slots_original_four_owned_permits_return_to_same_semaphore_without_allocation() {
    let bank = Arc::new(Semaphore::new(TOTAL));
    let registry = Registry::with_metadata(&bank).unwrap();
    let scope = NativeScope::from_registry(registry.clone()).unwrap();
    let slots = scope.0.connections.clone();
    let pointer = &*slots.0 as *const Semaphore as usize;
    let before = bank.available_permits();
    let _hook =
        super::super::metadata_vec::on_allocation(|_, _| panic!("connection permit allocated"));
    let first = slots.clone().try_acquire_many_owned(3).unwrap();
    let last = slots.clone().try_acquire_owned().unwrap();
    assert_eq!(slots.available_permits(), 0);
    assert!(matches!(
        slots.clone().try_acquire_owned(),
        Err(TryAcquireError::NoPermits)
    ));
    assert_eq!(bank.available_permits(), before);
    assert!(slots.0.ptr_eq(&first.slots.0) && slots.0.ptr_eq(&last.slots.0));
    drop(first);
    assert_eq!(slots.available_permits(), 3);
    let all = slots.clone().try_acquire_many_owned(3).unwrap();
    assert_eq!(slots.available_permits(), 0);
    slots.close();
    assert!(matches!(
        slots.clone().try_acquire_owned(),
        Err(TryAcquireError::Closed)
    ));
    drop(all);
    drop(last);
    assert_eq!(slots.available_permits(), 4);
    assert_eq!(&*slots.0 as *const Semaphore as usize, pointer);
    assert_eq!(bank.available_permits(), before);
    drop(_hook);
    drop(slots);
    drop(scope);
    drop(registry);
    assert_eq!(bank.available_permits(), TOTAL);
}

#[test]
fn foreground_slots_live_owned_permit_keeps_only_its_paid_root_after_last_scope_closes_registry() {
    let bank = Arc::new(Semaphore::new(TOTAL));
    let registry = Registry::with_metadata(&bank).unwrap();
    let before = bank.available_permits();
    let scope = NativeScope::from_registry(registry.clone()).unwrap();
    let slots = scope.0.connections.clone();
    let permit = slots.clone().try_acquire_owned().unwrap();
    let pointer = &*slots.0 as *const Semaphore as usize;
    let slots_bytes = SlotsOwner::allocation_bytes();
    assert_eq!(slots.available_permits(), 3);
    assert_eq!(
        bank.available_permits(),
        before - slots_bytes - InnerOwner::allocation_bytes()
    );
    bank.close();
    drop(scope);
    assert!(
        registry.snapshot().closed_and_empty(),
        "live connection permit postponed original Inner Drop"
    );
    assert_eq!(slots.available_permits(), 3);
    assert_eq!(bank.available_permits(), before - slots_bytes);
    assert_eq!(slots.0.strong_count(), 2);
    drop(slots);
    assert_eq!(&*permit.slots.0 as *const Semaphore as usize, pointer);
    assert_eq!(bank.available_permits(), before - slots_bytes);
    let frees = Arc::new(AtomicUsize::new(0));
    let observed = frees.clone();
    let charged = bank.clone();
    super::super::shared_allocation::after_free(
        move || {
            assert_eq!(
                charged.available_permits(),
                before - slots_bytes,
                "slots fee returned before actual last owned-permit root free"
            );
            observed.fetch_add(1, Ordering::SeqCst);
        },
        || drop(permit),
    );
    assert_eq!(frees.load(Ordering::SeqCst), 1);
    assert_eq!(bank.available_permits(), before);
    drop(registry);
    assert_eq!(bank.available_permits(), TOTAL);
}

#[test]
fn foreground_slots_zero_and_many_permit_transfer_survive_parallel_drop_on_original_root() {
    let scope = NativeScope::new().unwrap();
    let bank = scope.0.registry.byte_bank();
    let slots = scope.0.connections.clone();
    let zero = slots.clone().try_acquire_many_owned(0).unwrap();
    assert_eq!(slots.available_permits(), 4);
    let permits = (0..4)
        .map(|_| slots.clone().try_acquire_owned().unwrap())
        .collect::<Vec<_>>();
    assert_eq!(slots.available_permits(), 0);
    assert!(matches!(
        slots.clone().try_acquire_many_owned(5),
        Err(TryAcquireError::NoPermits)
    ));
    let before = bank.available_permits();
    std::thread::scope(|threads| {
        for permit in permits {
            threads.spawn(move || drop(permit));
        }
    });
    assert_eq!(slots.available_permits(), 4);
    assert_eq!(bank.available_permits(), before);
    drop(zero);
    drop(slots);
    drop(scope);
    assert_eq!(bank.available_permits(), TOTAL);
}

#[test]
fn foreground_slots_empty_shared_destinations_validate_grants_and_keep_exact_original_backing() {
    use super::super::shared_allocation::{Destination, Shared};
    let bytes = Shared::<[u64; 17]>::allocation_bytes();
    for foreign in [false, true] {
        let bank = Arc::new(Semaphore::new(bytes));
        let other = Arc::new(Semaphore::new(bytes));
        let source = if foreign { &other } else { &bank };
        let count = if foreign { bytes } else { bytes - 1 };
        let fee = source.clone().try_acquire_many_owned(count as u32).unwrap();
        let _hook = super::super::metadata_vec::on_allocation(|_, _| {
            panic!("invalid shared destination allocated")
        });
        assert!(Destination::<[u64; 17]>::from_grant(&bank, fee).is_err());
        assert_eq!(bank.available_permits(), bytes);
        assert_eq!(other.available_permits(), bytes);
    }
    let bank = Arc::new(Semaphore::new(bytes));
    let seen = Arc::new(StdMutex::new(Vec::new()));
    let observed = seen.clone();
    let charged = bank.clone();
    let _hook = super::super::metadata_vec::on_allocation(move |layout, pointer| {
        assert_eq!(layout.size(), bytes);
        assert_eq!(charged.available_permits(), 0);
        observed.lock().unwrap().push(pointer);
    });
    let fee = bank.clone().try_acquire_many_owned(bytes as u32).unwrap();
    let destination = Destination::<[u64; 17]>::from_grant(&bank, fee).unwrap();
    let pointer = destination.as_ptr();
    assert_eq!(&*seen.lock().unwrap(), &[pointer]);
    bank.close();
    let value = destination.place([71; 17]);
    assert_eq!(
        &*seen.lock().unwrap(),
        &[pointer],
        "placement allocated a second body"
    );
    let data = &*value as *const [u64; 17] as usize;
    assert!(data >= pointer && data + std::mem::size_of::<[u64; 17]>() <= pointer + bytes);
    assert_eq!(*value, [71; 17]);
    drop(value);
    assert_eq!(bank.available_permits(), bytes);
}

#[test]
fn foreground_slots_second_destination_refusal_frees_first_without_constructing_inner_or_sealing_registry(
) {
    let bank = Arc::new(Semaphore::new(TOTAL));
    let registry = Registry::with_metadata(&bank).unwrap();
    let before = bank.available_permits();
    let whole = InnerOwner::allocation_bytes() + SlotsOwner::allocation_bytes();
    let held = bank
        .clone()
        .try_acquire_many_owned((before - whole) as u32)
        .unwrap();
    let attempts = Arc::new(AtomicUsize::new(0));
    let observed = attempts.clone();
    let original = registry.clone();
    let charged = bank.clone();
    let _refusal = super::super::metadata_vec::refuse_allocation(move |layout| {
        let call = observed.fetch_add(1, Ordering::SeqCst);
        assert_eq!(
            charged.available_permits(),
            0,
            "both actual roots must be prepaid before allocation"
        );
        assert!(!original.snapshot().sealed);
        assert_eq!(
            layout.size(),
            if call == 0 {
                InnerOwner::allocation_bytes()
            } else {
                SlotsOwner::allocation_bytes()
            }
        );
        call == 1
    });
    let allocations = Arc::new(AtomicUsize::new(0));
    let observed = allocations.clone();
    let _allocated = super::super::metadata_vec::on_allocation(move |layout, _| {
        assert_eq!(layout.size(), InnerOwner::allocation_bytes());
        observed.fetch_add(1, Ordering::SeqCst);
    });
    let error = NativeScope::from_registry(registry.clone())
        .err()
        .expect("second destination refusal ignored");
    assert_eq!(
        crate::output::operation_error_class(error.as_ref()),
        ios_core::ErrorClass::ResourceLimit
    );
    assert_eq!(attempts.load(Ordering::SeqCst), 2);
    assert_eq!(allocations.load(Ordering::SeqCst), 1);
    assert_eq!(bank.available_permits(), whole);
    assert!(
        !registry.snapshot().sealed,
        "fallible empty destination caused Inner Drop"
    );
    assert_eq!(registry.snapshot().occupied(), 0);
    drop(_allocated);
    drop(_refusal);
    assert_eq!(registry.strong_count(), 1);
    drop(held);
    drop(registry);
    assert_eq!(bank.available_permits(), TOTAL);
}

#[test]
fn foreground_slots_whole_paid_roots_place_after_bank_closes_during_second_allocation() {
    let bank = Arc::new(Semaphore::new(TOTAL));
    let registry = Registry::with_metadata(&bank).unwrap();
    let before = bank.available_permits();
    let whole = InnerOwner::allocation_bytes() + SlotsOwner::allocation_bytes();
    let held = bank
        .clone()
        .try_acquire_many_owned((before - whole) as u32)
        .unwrap();
    let allocations = Arc::new(AtomicUsize::new(0));
    let observed = allocations.clone();
    let charged = bank.clone();
    let _allocated = super::super::metadata_vec::on_allocation(move |_, _| {
        assert_eq!(charged.available_permits(), 0);
        let call = observed.fetch_add(1, Ordering::SeqCst);
        assert!(call < 2, "placement allocated another backing");
        if call == 1 {
            charged.close();
        }
    });
    let scope = NativeScope::from_registry(registry.clone()).unwrap();
    assert!(bank.is_closed());
    assert_eq!(bank.available_permits(), 0);
    assert_eq!(allocations.load(Ordering::SeqCst), 2);
    assert!(!registry.snapshot().sealed);
    let permit = scope
        .0
        .connections
        .clone()
        .try_acquire_many_owned(4)
        .unwrap();
    assert_eq!(scope.0.connections.available_permits(), 0);
    drop(permit);
    assert_eq!(scope.0.connections.available_permits(), 4);
    drop(_allocated);
    drop(scope);
    assert!(registry.snapshot().closed_and_empty());
    assert_eq!(bank.available_permits(), whole);
    drop(held);
    drop(registry);
    assert_eq!(bank.available_permits(), TOTAL);
}
