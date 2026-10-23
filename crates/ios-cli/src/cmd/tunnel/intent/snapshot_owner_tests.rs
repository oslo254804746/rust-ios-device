use super::super::{birth_bank::BirthBank, metadata_vec, shared_weak_allocation};
use super::*;
use std::sync::atomic::{AtomicUsize, Ordering};

#[test]
fn desired_snapshot_root_each_bank_refusal_precedes_backing_and_rolls_back_other_grants() {
    let bytes = SnapshotOwner::body_bytes();
    for denied in 0..3 {
        for closed in [false, true] {
            let global = Arc::new(Semaphore::new(bytes));
            let epoch = BirthBank::new(bytes);
            let version = BirthBank::new(bytes);
            let snapshot = Snapshot {
                epoch: ControlIdentity::new(&epoch),
                version: ControlIdentity::new(&version),
                desired: Desired::Stopped,
            };
            let banks = [&global, epoch.raw(), version.raw()];
            let held = if closed {
                None
            } else {
                Some(banks[denied].clone().try_acquire_owned().unwrap())
            };
            if closed {
                banks[denied].close();
            }
            let before = banks.map(|bank| bank.available_permits());
            let hook = metadata_vec::on_allocation(|_, _| panic!("unadmitted snapshot root"));
            let error = SnapshotOwner::new(&snapshot, &global).err().unwrap();
            drop(hook);
            assert_eq!(
                crate::output::operation_error_class(error.as_ref()),
                ios_core::ErrorClass::ResourceLimit
            );
            assert_eq!(banks.map(|bank| bank.available_permits()), before);
            assert_eq!(
                (
                    snapshot.epoch.strong_count(),
                    snapshot.version.strong_count()
                ),
                (1, 1)
            );
            drop(held);
        }
    }
}

#[test]
fn desired_snapshot_root_actual_span_deduplicates_producer_and_keeps_fees_until_physical_free() {
    let bytes = SnapshotOwner::body_bytes();
    let global = Arc::new(Semaphore::new(bytes));
    let origin = BirthBank::new(bytes);
    let snapshot = Snapshot {
        epoch: ControlIdentity::new(&origin),
        version: ControlIdentity::new(&origin),
        desired: Desired::Stopped,
    };
    let old_epoch = snapshot.epoch.downgrade();
    let old_version = snapshot.version.downgrade();
    let pointer = Arc::new(AtomicUsize::new(0));
    let observed = pointer.clone();
    let a = origin.raw().clone();
    let b = global.clone();
    let hook = metadata_vec::on_allocation(move |layout, address| {
        assert_eq!(layout.size(), bytes);
        assert_eq!((a.available_permits(), b.available_permits()), (0, 0));
        assert_eq!(observed.swap(address, Ordering::SeqCst), 0);
    });
    let root = SnapshotOwner::new(&snapshot, &global).unwrap();
    drop(hook);
    let start = pointer.load(Ordering::SeqCst);
    let value = &*root as *const Snapshot as usize;
    assert!(start <= value && value + std::mem::size_of::<Snapshot>() <= start + bytes);
    assert!(ControlIdentity::ptr_eq(&root.epoch, &snapshot.epoch));
    assert!(ControlIdentity::ptr_eq(&root.version, &snapshot.version));
    assert_eq!(root.0.extra().iter().flatten().count(), 1);
    let clone = root.clone();
    assert!(root.0.ptr_eq(&clone.0));
    origin.close();
    global.close();
    drop((root, snapshot));
    assert!(old_epoch.upgrade().is_some() && old_version.upgrade().is_some());
    let a = origin.raw().clone();
    let b = global.clone();
    shared_weak_allocation::after_free(
        move || assert_eq!((a.available_permits(), b.available_permits()), (0, 0)),
        || drop(clone),
    );
    assert!(old_epoch.upgrade().is_none() && old_version.upgrade().is_none());
    assert_eq!(
        (origin.available_permits(), global.available_permits()),
        (bytes, bytes)
    );
}
