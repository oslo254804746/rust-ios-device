use super::*;
use std::marker::PhantomPinned;
use std::sync::{
    atomic::{AtomicBool, AtomicUsize, Ordering},
    Barrier, Mutex,
};

#[repr(align(128))]
struct PinnedValue {
    address: AtomicUsize,
    drops: Arc<AtomicUsize>,
    first: Arc<Semaphore>,
    second: Arc<Semaphore>,
    _pin: PhantomPinned,
}
impl Drop for PinnedValue {
    fn drop(&mut self) {
        assert_eq!(
            self.address.load(Ordering::SeqCst),
            std::ptr::from_ref(self) as usize
        );
        assert_eq!(self.first.available_permits(), 0);
        assert_eq!(self.second.available_permits(), 0);
        assert_eq!(self.drops.fetch_add(1, Ordering::SeqCst), 0);
    }
}
#[test]
fn weak_allocation_actual_one_layout_is_prepaid_in_both_pools_and_pinned_data_drops_in_place() {
    let size = Strong::<PinnedValue, OwnedSemaphorePermit>::body_bytes();
    let first = Arc::new(Semaphore::new(size));
    let second = Arc::new(Semaphore::new(size));
    let a = first.clone().try_acquire_many_owned(size as u32).unwrap();
    let b = second.clone().try_acquire_many_owned(size as u32).unwrap();
    let drops = Arc::new(AtomicUsize::new(0));
    let seen = Arc::new(Mutex::new(Vec::new()));
    let record = seen.clone();
    let charged_a = first.clone();
    let charged_b = second.clone();
    let root = super::super::metadata_vec::with_allocation_hook(
        move |layout, pointer| {
            assert_eq!(
                layout,
                std::alloc::Layout::new::<Allocation<PinnedValue, OwnedSemaphorePermit>>()
            );
            assert_eq!(charged_a.available_permits(), 0);
            assert_eq!(charged_b.available_permits(), 0);
            record.lock().unwrap().push(pointer);
        },
        || {
            Strong::new(
                PinnedValue {
                    address: AtomicUsize::new(0),
                    drops: drops.clone(),
                    first: first.clone(),
                    second: second.clone(),
                    _pin: PhantomPinned,
                },
                &first,
                a,
                b,
            )
            .unwrap()
        },
    );
    assert_eq!(
        seen.lock().unwrap().as_slice(),
        [root.pointer.as_ptr() as usize]
    );
    let address = std::ptr::from_ref(&*root) as usize;
    assert_eq!(address % 128, 0);
    root.address.store(address, Ordering::SeqCst);
    let weak = root.downgrade();
    let last = weak.clone();
    first.close();
    second.close();
    drop(root);
    assert_eq!(drops.load(Ordering::SeqCst), 1);
    assert!(weak.upgrade().is_none());
    assert_eq!(first.available_permits(), 0);
    assert_eq!(second.available_permits(), 0);
    drop(weak);
    let freed = Arc::new(AtomicBool::new(false));
    let flag = freed.clone();
    let a = first.clone();
    let b = second.clone();
    after_free(
        move || {
            assert_eq!(a.available_permits(), 0);
            assert_eq!(b.available_permits(), 0);
            flag.store(true, Ordering::SeqCst);
        },
        || drop(last),
    );
    assert!(freed.load(Ordering::SeqCst));
    assert_eq!(first.available_permits(), size);
    assert_eq!(second.available_permits(), size);
}
#[test]
fn weak_allocation_prepared_complete_layout_precedes_value_and_places_same_pinned_body_at_closed_zero(
) {
    let size = Strong::<PinnedValue, OwnedSemaphorePermit>::body_bytes();
    let first = Arc::new(Semaphore::new(size));
    let second = Arc::new(Semaphore::new(size));
    let a = first.clone().try_acquire_many_owned(size as u32).unwrap();
    let b = second.clone().try_acquire_many_owned(size as u32).unwrap();
    let constructed = Arc::new(AtomicBool::new(false));
    let observed = constructed.clone();
    let prepared = super::super::metadata_vec::with_allocation_hook(
        move |layout, _| {
            assert_eq!(
                layout,
                std::alloc::Layout::new::<Allocation<PinnedValue, OwnedSemaphorePermit>>()
            );
            assert!(!observed.load(Ordering::SeqCst));
        },
        || Prepared::<PinnedValue, OwnedSemaphorePermit>::from_grant(&first, a).unwrap(),
    );
    assert_eq!(prepared.values.len(), 0);
    assert_eq!(prepared.values.capacity(), 1);
    let pointer = prepared.values.as_ptr() as usize;
    let drops = Arc::new(AtomicUsize::new(0));
    first.close();
    second.close();
    let root = super::super::metadata_vec::with_allocation_hook(
        |_, _| panic!("late place allocated"),
        || {
            constructed.store(true, Ordering::SeqCst);
            prepared.place(
                PinnedValue {
                    address: AtomicUsize::new(0),
                    drops: drops.clone(),
                    first: first.clone(),
                    second: second.clone(),
                    _pin: PhantomPinned,
                },
                b,
            )
        },
    );
    assert_eq!(root.pointer.as_ptr() as usize, pointer);
    root.address
        .store(std::ptr::from_ref(&*root) as usize, Ordering::SeqCst);
    let weak = root.downgrade();
    drop(root);
    assert_eq!(drops.load(Ordering::SeqCst), 1);
    assert!(weak.upgrade().is_none());
    let a = first.clone();
    let b = second.clone();
    after_free(
        move || {
            assert_eq!(a.available_permits(), 0);
            assert_eq!(b.available_permits(), 0);
        },
        || drop(weak),
    );
    assert_eq!(first.available_permits(), size);
    assert_eq!(second.available_permits(), size);
}

#[test]
fn weak_allocation_refused_preparation_drops_original_value_before_its_extra_fee() {
    struct Original {
        extra: Arc<Semaphore>,
        drops: Arc<AtomicUsize>,
    }
    impl Drop for Original {
        fn drop(&mut self) {
            assert_eq!(self.extra.available_permits(), 0);
            assert_eq!(self.drops.fetch_add(1, Ordering::SeqCst), 0);
        }
    }
    let bytes = Strong::<Original, OwnedSemaphorePermit>::body_bytes();
    let first = Arc::new(Semaphore::new(bytes));
    let second = Arc::new(Semaphore::new(bytes));
    let a = first.clone().try_acquire_many_owned(bytes as u32).unwrap();
    let b = second.clone().try_acquire_many_owned(bytes as u32).unwrap();
    let drops = Arc::new(AtomicUsize::new(0));
    let guard = super::super::metadata_vec::refuse_allocation(move |layout| {
        assert_eq!(layout.size(), bytes);
        true
    });
    assert!(Strong::new(
        Original {
            extra: second.clone(),
            drops: drops.clone()
        },
        &first,
        a,
        b
    )
    .is_err());
    drop(guard);
    assert_eq!(drops.load(Ordering::SeqCst), 1);
    assert_eq!(first.available_permits(), bytes);
    assert_eq!(second.available_permits(), bytes);
}

struct Panics(Arc<AtomicUsize>);
impl Drop for Panics {
    fn drop(&mut self) {
        assert_eq!(self.0.fetch_add(1, Ordering::SeqCst), 0);
        panic!("original weak payload unwind");
    }
}
#[test]
fn weak_allocation_implicit_guard_preserves_both_grants_during_original_destructor_unwind() {
    for explicit in [false, true] {
        let size = Strong::<Panics, OwnedSemaphorePermit>::body_bytes();
        let first = Arc::new(Semaphore::new(size));
        let second = Arc::new(Semaphore::new(size));
        let a = first.clone().try_acquire_many_owned(size as u32).unwrap();
        let b = second.clone().try_acquire_many_owned(size as u32).unwrap();
        let drops = Arc::new(AtomicUsize::new(0));
        let root = Strong::new(Panics(drops.clone()), &first, a, b).unwrap();
        let weak = explicit.then(|| root.downgrade());
        let freed = Arc::new(AtomicBool::new(false));
        let flag = freed.clone();
        let a = first.clone();
        let b = second.clone();
        after_free(
            move || {
                assert_eq!(a.available_permits(), 0);
                assert_eq!(b.available_permits(), 0);
                flag.store(true, Ordering::SeqCst);
            },
            || {
                assert!(
                    std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| drop(root))).is_err()
                );
                assert_eq!(drops.load(Ordering::SeqCst), 1);
                if let Some(weak) = weak {
                    assert!(!freed.load(Ordering::SeqCst));
                    assert!(weak.upgrade().is_none());
                    assert_eq!(first.available_permits(), 0);
                    assert_eq!(second.available_permits(), 0);
                    drop(weak);
                }
            },
        );
        assert!(freed.load(Ordering::SeqCst));
        assert_eq!(first.available_permits(), size);
        assert_eq!(second.available_permits(), size);
    }
}
#[test]
fn weak_allocation_unique_mutation_locks_downgrade_before_checking_strong_count() {
    let size = Strong::<usize, ()>::body_bytes();
    let bank = Arc::new(Semaphore::new(size));
    let fee = bank.clone().try_acquire_many_owned(size as u32).unwrap();
    let mut root = Strong::new(7usize, &bank, fee, ()).unwrap();
    let other = root.clone();
    let barrier = Arc::new(Barrier::new(2));
    let attempted = Arc::new(AtomicBool::new(false));
    let converted = Arc::new(AtomicBool::new(false));
    std::thread::scope(|scope| {
        let sync = barrier.clone();
        let entered = attempted.clone();
        let done = converted.clone();
        let worker = scope.spawn(move || {
            sync.wait();
            entered.store(true, Ordering::SeqCst);
            let weak = other.downgrade();
            done.store(true, Ordering::SeqCst);
            drop(other);
            weak
        });
        let sync = barrier.clone();
        let entered = attempted.clone();
        let done = converted.clone();
        let mutable = before_unique(
            move || {
                sync.wait();
                let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
                while !entered.load(Ordering::SeqCst) {
                    assert!(std::time::Instant::now() < deadline);
                    std::thread::yield_now();
                }
                for _ in 0..128 {
                    std::thread::yield_now();
                }
                assert!(
                    !done.load(Ordering::SeqCst),
                    "downgrade crossed the implicit-only weak lock"
                );
            },
            || root.get_mut().is_some(),
        );
        assert!(!mutable);
        let weak = worker.join().unwrap();
        assert!(root.get_mut().is_none());
        drop(weak);
    });
    *root.get_mut().unwrap() = 9;
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        before_unique(
            || panic!("uniqueness observation unwind"),
            || {
                root.get_mut();
            },
        )
    }));
    assert!(result.is_err());
    let weak = root.downgrade();
    assert_eq!(*weak.upgrade().unwrap(), 9);
    drop(weak);
    drop(root);
    assert_eq!(bank.available_permits(), size);
}
#[test]
fn weak_allocation_parallel_upgrades_and_last_weak_release_keep_one_original_value_and_no_revival()
{
    struct Value {
        drops: Arc<AtomicUsize>,
        number: usize,
    }
    impl Drop for Value {
        fn drop(&mut self) {
            assert_eq!(self.drops.fetch_add(1, Ordering::SeqCst), 0);
        }
    }
    let size = Strong::<Value, ()>::body_bytes();
    let bank = Arc::new(Semaphore::new(size));
    let drops = Arc::new(AtomicUsize::new(0));
    let fee = bank.clone().try_acquire_many_owned(size as u32).unwrap();
    let root = Strong::new(
        Value {
            drops: drops.clone(),
            number: 123,
        },
        &bank,
        fee,
        (),
    )
    .unwrap();
    let pointer = std::ptr::from_ref(&*root) as usize;
    let weak = root.downgrade();
    let sync = Arc::new(Barrier::new(9));
    bank.close();
    std::thread::scope(|scope| {
        for _ in 0..8 {
            let weak = weak.clone();
            let sync = sync.clone();
            scope.spawn(move || {
                sync.wait();
                for _ in 0..1024 {
                    if let Some(root) = weak.upgrade() {
                        assert_eq!(std::ptr::from_ref(&*root) as usize, pointer);
                        assert_eq!(root.number, 123);
                    }
                }
            });
        }
        sync.wait();
        drop(root);
    });
    assert_eq!(drops.load(Ordering::SeqCst), 1);
    assert!(weak.upgrade().is_none());
    assert_eq!(bank.available_permits(), 0);
    let sync = Arc::new(Barrier::new(9));
    std::thread::scope(|scope| {
        for _ in 0..8 {
            let weak = weak.clone();
            let sync = sync.clone();
            scope.spawn(move || {
                sync.wait();
                drop(weak);
            });
        }
        sync.wait();
        drop(weak);
    });
    assert_eq!(bank.available_permits(), size);
}
