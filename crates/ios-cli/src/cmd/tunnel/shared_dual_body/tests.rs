use super::super::{admission, metadata_vec};
use super::*;
use std::sync::atomic::{AtomicUsize, Ordering};

#[test]
fn dual_body_foreign_short_long_grants_in_either_pool_refund_both_and_drop_original_once() {
    struct Value(Arc<AtomicUsize>);
    impl Drop for Value {
        fn drop(&mut self) {
            self.0.fetch_add(1, Ordering::SeqCst);
        }
    }
    let bytes = Strong::<Value>::body_bytes();
    for pool in 0..2 {
        for mode in 0..3 {
            let first_bank = Arc::new(Semaphore::new(2 * bytes));
            let second_bank = Arc::new(Semaphore::new(2 * bytes));
            let foreign_bank = Arc::new(Semaphore::new(2 * bytes));
            let amount = match mode {
                0 => bytes - 1,
                1 => bytes + 1,
                _ => bytes,
            };
            let first = admission::known_metadata_charge(
                if pool == 0 && mode == 2 {
                    &foreign_bank
                } else {
                    &first_bank
                },
                if pool == 0 { amount } else { bytes },
            )
            .unwrap();
            let second = admission::known_metadata_charge(
                if pool == 1 && mode == 2 {
                    &foreign_bank
                } else {
                    &second_bank
                },
                if pool == 1 { amount } else { bytes },
            )
            .unwrap();
            let drops = Arc::new(AtomicUsize::new(0));
            let error = Strong::new(
                Value(drops.clone()),
                &first_bank,
                &second_bank,
                first,
                second,
            )
            .err()
            .unwrap();
            assert_eq!(
                crate::output::operation_error_class(error.as_ref()),
                ios_core::ErrorClass::ResourceLimit
            );
            assert_eq!(drops.load(Ordering::SeqCst), 1);
            assert_eq!(first_bank.available_permits(), 2 * bytes);
            assert_eq!(second_bank.available_permits(), 2 * bytes);
            assert_eq!(foreign_bank.available_permits(), 2 * bytes);
        }
    }
}

#[repr(align(128))]
struct Payload {
    first: Vec<u8>,
    second: Vec<u8>,
    first_bank: Arc<Semaphore>,
    second_bank: Arc<Semaphore>,
    drops: Arc<AtomicUsize>,
    _first_fields: OwnedSemaphorePermit,
    _second_fields: OwnedSemaphorePermit,
}
impl Drop for Payload {
    fn drop(&mut self) {
        assert_eq!(self.first_bank.available_permits(), 0);
        assert_eq!(self.second_bank.available_permits(), 0);
        assert_eq!(self.first.as_slice(), &[1; 11]);
        assert_eq!(self.second.as_slice(), &[2; 17]);
        self.drops.fetch_add(1, Ordering::SeqCst);
    }
}

#[test]
fn dual_body_parallel_weak_upgrades_end_fields_at_last_strong_and_both_body_leases_at_last_weak() {
    let bytes = Strong::<Payload>::body_bytes();
    let first_bank = Arc::new(Semaphore::new(bytes + 11));
    let second_bank = Arc::new(Semaphore::new(bytes + 17));
    let first_body = admission::known_metadata_charge(&first_bank, bytes).unwrap();
    let second_body = admission::known_metadata_charge(&second_bank, bytes).unwrap();
    let first = metadata_vec::exact_backing::<u8>(&first_bank, 11).unwrap();
    let second = metadata_vec::exact_backing::<u8>(&second_bank, 17).unwrap();
    let drops = Arc::new(AtomicUsize::new(0));
    let mut first_values = first.values;
    let mut second_values = second.values;
    first_values.resize(11, 1);
    second_values.resize(17, 2);
    let value = Strong::new(
        Payload {
            first: first_values,
            second: second_values,
            first_bank: first_bank.clone(),
            second_bank: second_bank.clone(),
            drops: drops.clone(),
            _first_fields: first.storage,
            _second_fields: second.storage,
        },
        &first_bank,
        &second_bank,
        first_body,
        second_body,
    )
    .unwrap();
    let pointer = &*value as *const Payload as usize;
    let first_pointer = value.first.as_ptr() as usize;
    let second_pointer = value.second.as_ptr() as usize;
    assert_eq!(pointer % 128, 0);
    assert!(value.banks_match(&first_bank, &second_bank));
    let weak = value.downgrade();
    let last = weak.clone();
    let retained = weak.upgrade().unwrap();
    assert!(retained.ptr_eq(&value));
    drop(retained);
    first_bank.close();
    second_bank.close();
    let barrier = Arc::new(std::sync::Barrier::new(9));
    std::thread::scope(|scope| {
        for _ in 0..8 {
            let weak = weak.clone();
            let barrier = barrier.clone();
            scope.spawn(move || {
                barrier.wait();
                for _ in 0..128 {
                    if let Some(value) = weak.upgrade() {
                        assert_eq!(&*value as *const Payload as usize, pointer);
                        assert_eq!(value.first.as_ptr() as usize, first_pointer);
                        assert_eq!(value.second.as_ptr() as usize, second_pointer);
                    }
                }
            });
        }
        barrier.wait();
        drop(value);
    });
    assert_eq!(drops.load(Ordering::SeqCst), 1);
    assert!(weak.upgrade().is_none());
    assert_eq!(first_bank.available_permits(), 11);
    assert_eq!(second_bank.available_permits(), 17);
    drop(weak);
    assert_eq!(first_bank.available_permits(), 11);
    assert_eq!(second_bank.available_permits(), 17);
    drop(last);
    assert_eq!(first_bank.available_permits(), bytes + 11);
    assert_eq!(second_bank.available_permits(), bytes + 17);
}
