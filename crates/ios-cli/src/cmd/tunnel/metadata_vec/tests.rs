use super::*;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Mutex;

#[test]
fn metadata_vector_refusal_precedes_actual_allocation_and_keeps_original_growth_owner() {
    let width = std::mem::size_of::<usize>();
    let bank = Arc::new(Semaphore::new(3 * width));
    let mut rows = KnownVec::new(Arc::clone(&bank), 8);
    rows.push(11_usize).unwrap();
    rows.push(22).unwrap();
    let pointer = rows.values.as_ptr();
    let allocated = Arc::new(AtomicUsize::new(0));
    let seen = Arc::clone(&allocated);
    with_allocation_hook(
        move |_, _| {
            seen.fetch_add(1, Ordering::SeqCst);
        },
        || {
            assert!(rows.push(33).is_err());
        },
    );
    assert_eq!(allocated.load(Ordering::SeqCst), 0);
    assert_eq!(rows.values.as_ptr(), pointer);
    assert_eq!(rows.as_slice(), &[11, 22]);
    assert_eq!(bank.available_permits(), width);
    drop(rows);
    assert_eq!(bank.available_permits(), 3 * width);
}

#[test]
fn metadata_vector_actual_allocation_observes_full_new_and_old_layout_grants() {
    let width = std::mem::size_of::<usize>();
    let bank = Arc::new(Semaphore::new(32 * width));
    let mut rows = KnownVec::new(Arc::clone(&bank), 8);
    rows.push(11_usize).unwrap();
    rows.push(22).unwrap();
    let observed = Arc::new(Mutex::new(Vec::new()));
    let captured = Arc::clone(&observed);
    let charged = Arc::clone(&bank);
    with_allocation_hook(
        move |layout, pointer| {
            captured
                .lock()
                .unwrap()
                .push((layout.size(), pointer, charged.available_permits()));
        },
        || {
            rows.push(33).unwrap();
        },
    );
    let observed = observed.lock().unwrap();
    assert_eq!(observed.len(), 1);
    assert_eq!(
        observed[0],
        (4 * width, rows.values.as_ptr() as usize, 26 * width)
    );
    assert_eq!(bank.available_permits(), 28 * width);
    drop(rows);
    assert_eq!(bank.available_permits(), 32 * width);
}

#[test]
fn metadata_vector_layout_overflow_and_empty_bank_allocate_nothing() {
    let bank = Arc::new(Semaphore::new(0));
    let allocated = Arc::new(AtomicUsize::new(0));
    let seen = Arc::clone(&allocated);
    with_allocation_hook(
        move |_, _| {
            seen.fetch_add(1, Ordering::SeqCst);
        },
        || {
            assert!(exact_backing::<usize>(&bank, usize::MAX).is_err());
            assert!(exact_backing::<u8>(&bank, 1).is_err());
        },
    );
    assert_eq!(allocated.load(Ordering::SeqCst), 0);
    assert_eq!(bank.available_permits(), 0);
}

#[test]
fn metadata_vector_alignment_and_element_drop_precede_last_backing_refund() {
    #[repr(align(128))]
    struct Element(Arc<Semaphore>, Arc<AtomicUsize>);
    impl Drop for Element {
        fn drop(&mut self) {
            assert_eq!(self.0.available_permits(), 0);
            self.1.fetch_add(1, Ordering::SeqCst);
        }
    }
    let width = std::mem::size_of::<Element>();
    let bank = Arc::new(Semaphore::new(3 * width));
    let drops = Arc::new(AtomicUsize::new(0));
    let mut allocation = exact_backing::<Element>(&bank, 3).unwrap();
    assert_eq!(allocation.values.capacity(), 3);
    assert_eq!(allocation.values.as_ptr() as usize % 128, 0);
    for _ in 0..3 {
        allocation
            .values
            .push(Element(Arc::clone(&bank), Arc::clone(&drops)));
    }
    drop(allocation);
    assert_eq!(drops.load(Ordering::SeqCst), 3);
    assert_eq!(bank.available_permits(), 3 * width);
}

#[test]
fn metadata_vector_zero_size_elements_keep_logical_ceiling_without_allocation() {
    let bank = Arc::new(Semaphore::new(0));
    let allocated = Arc::new(AtomicUsize::new(0));
    let seen = Arc::clone(&allocated);
    with_allocation_hook(
        move |_, _| {
            seen.fetch_add(1, Ordering::SeqCst);
        },
        || {
            let mut rows = KnownVec::new(Arc::clone(&bank), 3);
            for _ in 0..3 {
                rows.push(()).unwrap();
            }
            assert!(rows.push(()).is_err());
            assert_eq!(rows.len(), 3);
            drop(rows);
            drop(exact_backing::<()>(&bank, 3).unwrap());
        },
    );
    assert_eq!(allocated.load(Ordering::SeqCst), 0);
}

#[test]
fn native_registry_backing_refusal_precedes_slot_allocation_and_refunds_body() {
    use super::super::retirement::Registry;
    let full = Arc::new(Semaphore::new(128 * 1024 * 1024));
    let slot_bytes = Arc::new(AtomicUsize::new(0));
    let captured = Arc::clone(&slot_bytes);
    let native = with_allocation_hook(
        move |layout, _| {
            captured.store(layout.size(), Ordering::SeqCst);
        },
        || Registry::with_metadata(&full).unwrap(),
    );
    drop(native);
    let bytes = std::mem::size_of::<Registry>() + slot_bytes.load(Ordering::SeqCst) - 1;
    let bank = Arc::new(Semaphore::new(bytes));
    let allocated = Arc::new(AtomicUsize::new(0));
    let captured = Arc::clone(&allocated);
    with_allocation_hook(
        move |_, _| {
            captured.fetch_add(1, Ordering::SeqCst);
        },
        || {
            assert!(Registry::with_metadata(&bank).is_err());
        },
    );
    assert_eq!(allocated.load(Ordering::SeqCst), 0);
    assert_eq!(bank.available_permits(), bytes);
}

#[test]
fn metadata_vec_growth_refusal_preserves_original_rows_and_last_iterator_release() {
    let bank = Arc::new(Semaphore::new(3 * std::mem::size_of::<usize>()));
    let mut rows = KnownVec::new(Arc::clone(&bank), 8);
    rows.push(1_usize).unwrap();
    rows.push(2).unwrap();
    assert!(rows.push(3).is_err());
    assert_eq!(rows.as_slice(), &[1, 2]);
    assert_eq!(bank.available_permits(), std::mem::size_of::<usize>());
    let mut iterator = rows.into_iter();
    assert_eq!(iterator.next(), Some(1));
    assert_eq!(bank.available_permits(), std::mem::size_of::<usize>());
    drop(iterator);
    assert_eq!(bank.available_permits(), 3 * std::mem::size_of::<usize>());
}

#[test]
fn metadata_vec_actual_capacity_and_count_ceiling_refund_all_storage() {
    let bank = Arc::new(Semaphore::new(1024));
    let mut rows = KnownVec::new(Arc::clone(&bank), 3);
    for index in 0..3_u64 {
        rows.push(index).unwrap();
    }
    assert!(rows.push(4).is_err());
    assert_eq!(rows.len(), 3);
    assert_eq!(
        bank.available_permits(),
        1024 - rows.values.capacity() * std::mem::size_of::<u64>()
    );
    drop(rows);
    assert_eq!(bank.available_permits(), 1024);
}
