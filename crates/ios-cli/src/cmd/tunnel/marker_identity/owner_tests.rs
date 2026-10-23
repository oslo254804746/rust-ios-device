use super::super::{birth_bank::BirthBank, metadata_vec, shared_weak_allocation};
use super::*;
use std::sync::atomic::{AtomicUsize, Ordering};

#[test]
fn failure_global_root_each_bank_refusal_has_no_backing_and_rolls_back_before_clone() {
    let bytes = Owner::body_bytes();
    for origin_short in [false, true] {
        for closed in [false, true] {
            let origin = BirthBank::new(2 * bytes);
            let bank = Arc::new(Semaphore::new(2 * bytes));
            let identity = Identity::new(&origin);
            let denied = if origin_short { origin.raw() } else { &bank };
            let leave = bytes - usize::from(!closed);
            let held = denied
                .clone()
                .try_acquire_many_owned((2 * bytes - leave) as u32)
                .unwrap();
            if closed {
                denied.close();
            }
            let before = (origin.available_permits(), bank.available_permits());
            let hook = metadata_vec::on_allocation(|_, _| panic!("unadmitted epoch root"));
            let error = Owner::new(&identity, &bank).err().unwrap();
            drop(hook);
            assert_eq!(
                crate::output::operation_error_class(error.as_ref()),
                ios_core::ErrorClass::ResourceLimit
            );
            assert_eq!(
                (origin.available_permits(), bank.available_permits()),
                before
            );
            assert_eq!(identity.strong_count(), 1);
            drop(held);
        }
    }
}

#[test]
fn failure_global_root_actual_span_same_identity_and_last_free_keep_both_real_fees() {
    let bytes = Owner::body_bytes();
    let origin = BirthBank::new(bytes);
    let raw = origin.raw().clone();
    let bank = Arc::new(Semaphore::new(bytes));
    let identity = Identity::new(&origin);
    let weak = identity.downgrade();
    let address = Arc::new(AtomicUsize::new(0));
    let observed = address.clone();
    let a = raw.clone();
    let b = bank.clone();
    let hook = metadata_vec::on_allocation(move |layout, pointer| {
        assert_eq!(layout.size(), bytes);
        assert_eq!((a.available_permits(), b.available_permits()), (0, 0));
        assert_eq!(observed.swap(pointer, Ordering::SeqCst), 0);
    });
    let root = Owner::new(&identity, &bank).unwrap();
    drop(hook);
    let value = &*root as *const Identity as usize;
    let pointer = address.load(Ordering::SeqCst);
    assert!(pointer <= value && value + std::mem::size_of::<Identity>() <= pointer + bytes);
    assert!(root.ptr_eq(&identity));
    let kept = root.clone();
    assert_eq!(&*kept as *const Identity, &*root as *const Identity);
    raw.close();
    bank.close();
    drop((root, identity, origin));
    assert!(weak.upgrade().is_some());
    let a = raw.clone();
    let b = bank.clone();
    shared_weak_allocation::after_free(
        move || assert_eq!((a.available_permits(), b.available_permits()), (0, 0)),
        || drop(kept),
    );
    assert!(weak.upgrade().is_none());
    assert_eq!(
        (raw.available_permits(), bank.available_permits()),
        (bytes, bytes)
    );
}

#[test]
fn failure_global_root_same_producer_is_covered_by_single_actual_grant() {
    let bytes = Owner::body_bytes();
    let bank = BirthBank::new(bytes);
    let identity = Identity::new(&bank);
    let root = Owner::new(&identity, bank.raw()).unwrap();
    assert_eq!(bank.available_permits(), 0);
    assert!(root.0.extra().is_none());
    drop(root);
    assert_eq!(bank.available_permits(), bytes);
}
