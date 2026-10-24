use super::super::{metadata_vec, shared_weak_allocation};
use super::*;
use std::alloc::Layout;
use std::future::Future;
use std::sync::{
    atomic::{AtomicBool, AtomicUsize, Ordering},
    Mutex,
};
use std::task::{Context, Poll, Wake, Waker};

fn assert_span(layout: Layout, pointer: usize, identity: &Identity) {
    let end = pointer.checked_add(layout.size()).unwrap();
    assert_eq!(layout.size(), Identity::body_bytes());
    assert_eq!(pointer % layout.align(), 0);
    for (address, bytes) in [
        (
            std::ptr::from_ref(identity.value.first()) as usize,
            std::mem::size_of::<OwnedSemaphorePermit>(),
        ),
        (
            std::ptr::from_ref(identity.value.extra()) as usize,
            std::mem::size_of::<Option<OwnedSemaphorePermit>>(),
        ),
        (
            std::ptr::from_ref(&*identity.value) as usize,
            std::mem::size_of::<BirthBank>(),
        ),
    ] {
        assert!(pointer <= address && address.checked_add(bytes).unwrap() <= end);
    }
}

#[test]
fn marker_birth_whole_group_precedes_actual_backing_and_closed_zero_place_uses_same_spans() {
    let bytes = Identity::body_bytes();
    let total = 2 * bytes;
    let origin = BirthBank::new(total);
    let bank = Arc::new(Semaphore::new(total));
    let allocations = Arc::new(Mutex::new([None::<(Layout, usize)>; 2]));
    let spans = allocations.clone();
    let a = origin.raw().clone();
    let g = bank.clone();
    let mut cutover = metadata_vec::with_allocation_hook(
        move |layout, pointer| {
            assert_eq!(layout.size(), bytes);
            // Both complete roots, including this still-empty root, were
            // admitted in authentic A and foreign G before the first alloc.
            assert_eq!((a.available_permits(), g.available_permits()), (0, 0));
            let mut spans = spans.lock().unwrap();
            let slot = spans.iter_mut().find(|slot| slot.is_none()).unwrap();
            *slot = Some((layout, pointer));
        },
        || Cutover::<2>::new(&origin, &bank).unwrap(),
    );
    assert_eq!(origin.total(), total);
    assert!(cutover.births.iter().all(Option::is_some));
    let first = cutover.take(&origin);
    let second = cutover.take(&origin);
    assert!(cutover.births.iter().all(Option::is_none));
    origin.close();
    bank.close();
    let (first, second, cloned) = metadata_vec::with_allocation_hook(
        |_, _| panic!("placing or cloning an admitted marker allocated"),
        || {
            let first = first.place(&origin);
            let second = second.place(&origin);
            let cloned = first.clone();
            (first, second, cloned)
        },
    );
    let spans = *allocations.lock().unwrap();
    let [(first_layout, first_pointer), (second_layout, second_pointer)] =
        spans.map(Option::unwrap);
    assert_ne!(first_pointer, second_pointer);
    assert_span(first_layout, first_pointer, &first);
    assert_span(second_layout, second_pointer, &second);
    assert!(first.ptr_eq(&cloned));
    assert!(!first.ptr_eq(&second));
    assert_eq!(first.strong_count(), 2);
    for identity in [&first, &second] {
        assert!(Arc::ptr_eq(identity.bank(), origin.raw()));
        assert!(Arc::ptr_eq(identity.value.first().semaphore(), &bank));
        assert_eq!(identity.value.first().num_permits(), bytes);
        let extra = identity.value.extra().as_ref().unwrap();
        assert!(Arc::ptr_eq(extra.semaphore(), origin.raw()));
        assert_eq!(extra.num_permits(), bytes);
    }
    drop((cutover, first, cloned));
    assert_eq!(
        (origin.available_permits(), bank.available_permits()),
        (bytes, bytes)
    );
    drop(second);
    assert_eq!(
        (origin.available_permits(), bank.available_permits()),
        (total, total)
    );
}

#[test]
fn marker_birth_each_real_bank_n_minus_one_or_closed_refuses_before_root_backing() {
    let total = 2 * Identity::body_bytes();
    // The last case is one authentic producer bank serving both roles.
    for (same_bank, deny_origin) in [(false, false), (false, true), (true, true)] {
        for closed in [false, true] {
            let origin = BirthBank::new(total);
            let bank = if same_bank {
                origin.raw().clone()
            } else {
                Arc::new(Semaphore::new(total))
            };
            let denied = if deny_origin { origin.raw() } else { &bank };
            let held = if closed {
                denied.close();
                None
            } else {
                Some(denied.clone().try_acquire_owned().unwrap())
            };
            let before = (origin.available_permits(), bank.available_permits());
            assert_eq!(origin.total(), total);
            let error = metadata_vec::with_allocation_hook(
                |_, _| panic!("unadmitted marker backing"),
                || Cutover::<2>::new(&origin, &bank).err().unwrap(),
            );
            assert_eq!(
                crate::output::operation_error_class(error.as_ref()),
                ios_core::ErrorClass::ResourceLimit
            );
            assert_eq!(
                (origin.available_permits(), bank.available_permits()),
                before
            );
            // A's authentic original capacity survives both a held grant
            // and closure; current availability is never substituted for it.
            assert_eq!(origin.total(), total);
            drop(held);
            assert_eq!(
                (origin.available_permits(), bank.available_permits()),
                (total, total)
            );
        }
    }
}

#[test]
fn marker_birth_actual_zero_or_partial_allocator_refusal_refunds_cutover_and_batch() {
    let bytes = Identity::body_bytes();
    let stock = 2 * std::mem::size_of::<BirthCell>();
    for dynamic in [false, true] {
        for successful_roots in [0, 1] {
            let origin = BirthBank::new(2 * bytes);
            let total_g = 2 * bytes + if dynamic { stock } else { 0 };
            let bank = Arc::new(Semaphore::new(total_g));
            let attempts = Arc::new(AtomicUsize::new(0));
            let observed = attempts.clone();
            let a = origin.raw().clone();
            let g = bank.clone();
            let refusal = metadata_vec::refuse_allocation(move |layout| {
                let attempt = observed.fetch_add(1, Ordering::SeqCst);
                if dynamic && attempt == 0 {
                    assert_eq!(layout, Layout::array::<BirthCell>(2).unwrap());
                    return false;
                }
                assert_eq!(layout.size(), bytes);
                assert_eq!((a.available_permits(), g.available_permits()), (0, 0));
                attempt - usize::from(dynamic) == successful_roots
            });
            let allocated = Arc::new(AtomicUsize::new(0));
            let observed = allocated.clone();
            let a = origin.raw().clone();
            let g = bank.clone();
            let error = metadata_vec::with_allocation_hook(
                move |layout, _| {
                    let allocation = observed.fetch_add(1, Ordering::SeqCst);
                    if dynamic && allocation == 0 {
                        assert_eq!(layout, Layout::array::<BirthCell>(2).unwrap());
                        assert_eq!(
                            (a.available_permits(), g.available_permits()),
                            (2 * bytes, 2 * bytes)
                        );
                    } else {
                        assert_eq!(layout.size(), bytes);
                        assert_eq!((a.available_permits(), g.available_permits()), (0, 0));
                    }
                },
                || {
                    if dynamic {
                        BirthBatch::new(&origin, &bank, 2).err().unwrap()
                    } else {
                        Cutover::<2>::new(&origin, &bank).err().unwrap()
                    }
                },
            );
            drop(refusal);
            assert_eq!(
                crate::output::operation_error_class(error.as_ref()),
                ios_core::ErrorClass::ResourceLimit
            );
            assert_eq!(
                attempts.load(Ordering::SeqCst),
                usize::from(dynamic) + successful_roots + 1
            );
            assert_eq!(
                allocated.load(Ordering::SeqCst),
                usize::from(dynamic) + successful_roots
            );
            assert_eq!(
                (origin.available_permits(), bank.available_permits()),
                (2 * bytes, total_g)
            );
            // The hooks cover these actual owned backings, not all process
            // allocations or the semaphore's own standard Arc allocation.
        }
    }
}

#[test]
fn marker_birth_same_bank_deduplicates_and_foreign_last_weak_keeps_fees_after_payload_end() {
    let bytes = Identity::body_bytes();
    let same = BirthBank::new(bytes);
    let mut cutover = Cutover::<1>::new(&same, same.raw()).unwrap();
    let identity = cutover.place(&same);
    assert!(identity.value.extra().is_none());
    assert!(Arc::ptr_eq(identity.value.first().semaphore(), same.raw()));
    assert_eq!(identity.value.first().num_permits(), bytes);
    assert_eq!(same.available_permits(), 0);
    drop((identity, cutover));
    assert_eq!(same.available_permits(), bytes);

    let origin = BirthBank::new(bytes);
    let bank = Arc::new(Semaphore::new(bytes));
    let mut cutover = Cutover::<1>::new(&origin, &bank).unwrap();
    let identity = cutover.place(&origin);
    let cloned = identity.clone();
    let weak = identity.downgrade();
    let last = weak.clone();
    drop(cutover);
    origin.close();
    bank.close();
    let references = Arc::strong_count(origin.raw());
    drop(identity);
    assert_eq!(Arc::strong_count(origin.raw()), references);
    assert!(weak.upgrade().is_some());
    drop(cloned);
    // The actual BirthBank payload owns one original semaphore Arc. Its
    // reference ends with the last strong, while the mirrored fee remains.
    assert_eq!(Arc::strong_count(origin.raw()), references - 1);
    assert!(weak.upgrade().is_none() && last.upgrade().is_none());
    assert_eq!(
        (origin.available_permits(), bank.available_permits()),
        (0, 0)
    );
    drop(weak);
    assert_eq!(
        (origin.available_permits(), bank.available_permits()),
        (0, 0)
    );
    let freed = Arc::new(AtomicBool::new(false));
    let observed = freed.clone();
    let a = origin.raw().clone();
    let g = bank.clone();
    shared_weak_allocation::after_free(
        move || {
            assert_eq!((a.available_permits(), g.available_permits()), (0, 0));
            assert!(!observed.swap(true, Ordering::SeqCst));
        },
        || drop(last),
    );
    assert!(freed.load(Ordering::SeqCst));
    assert_eq!(
        (origin.available_permits(), bank.available_permits()),
        (bytes, bytes)
    );
    assert_eq!(origin.total(), bytes);
}

struct GuardWake {
    guard: Arc<Mutex<()>>,
    calls: AtomicUsize,
    all_unlocked: AtomicBool,
}

impl Wake for GuardWake {
    fn wake(self: Arc<Self>) {
        self.wake_by_ref();
    }

    fn wake_by_ref(self: &Arc<Self>) {
        let unlocked = self.guard.try_lock().is_ok();
        self.all_unlocked.fetch_and(unlocked, Ordering::SeqCst);
        self.calls.fetch_add(1, Ordering::SeqCst);
    }
}

#[test]
fn marker_cutover_weak_fence_refunds_after_guard_unlock_without_retaining_original_payload() {
    let bytes = Identity::body_bytes();
    let origin = BirthBank::new(bytes);
    let bank = Arc::new(Semaphore::new(bytes));
    // The context owner is created outside, and outlives, the original guard.
    let mut cutover = Cutover::<1>::new(&origin, &bank).unwrap();
    let identity = cutover.place(&origin);
    let weak = identity.downgrade();
    assert!(!origin.is_closed() && !bank.is_closed());
    assert_eq!(
        (origin.available_permits(), bank.available_permits()),
        (0, 0)
    );
    let guard = Arc::new(Mutex::new(()));
    let observed = Arc::new(GuardWake {
        guard: guard.clone(),
        calls: AtomicUsize::new(0),
        all_unlocked: AtomicBool::new(true),
    });
    let waker = Waker::from(observed.clone());
    let mut context = Context::from_waker(&waker);
    let mut waiter = std::pin::pin!(bank.clone().acquire_many_owned(bytes as u32));
    assert!(waiter.as_mut().poll(&mut context).is_pending());
    assert_eq!(observed.calls.load(Ordering::SeqCst), 0);
    let references = Arc::strong_count(origin.raw());
    {
        let _held = guard.lock().unwrap();
        drop(identity);
        // The original BirthBank T ends now, even though its physical block
        // and fees are protected by the operation context's private Weak.
        assert_eq!(Arc::strong_count(origin.raw()), references - 1);
        assert!(weak.upgrade().is_none());
        drop(weak);
        assert_eq!(
            (origin.available_permits(), bank.available_permits()),
            (0, 0)
        );
        assert!(waiter.as_mut().poll(&mut context).is_pending());
        assert_eq!(observed.calls.load(Ordering::SeqCst), 0);
    }
    let freed = Arc::new(AtomicBool::new(false));
    let flag = freed.clone();
    let a = origin.raw().clone();
    let g = bank.clone();
    let wake = observed.clone();
    shared_weak_allocation::after_free(
        move || {
            assert_eq!((a.available_permits(), g.available_permits()), (0, 0));
            assert_eq!(wake.calls.load(Ordering::SeqCst), 0);
            assert!(!flag.swap(true, Ordering::SeqCst));
        },
        || drop(cutover),
    );
    assert!(freed.load(Ordering::SeqCst));
    assert!(observed.calls.load(Ordering::SeqCst) > 0);
    assert!(observed.all_unlocked.load(Ordering::SeqCst));
    let permit = match waiter.as_mut().poll(&mut context) {
        Poll::Ready(Ok(permit)) => permit,
        _ => panic!("the actual refunded whole body did not satisfy its queued waiter"),
    };
    assert_eq!(permit.num_permits(), bytes);
    assert!(Arc::ptr_eq(permit.semaphore(), &bank));
    assert_eq!(
        (origin.available_permits(), bank.available_permits()),
        (bytes, 0)
    );
    drop(permit);
    assert_eq!(
        (origin.available_permits(), bank.available_permits()),
        (bytes, bytes)
    );
    // This covers the context primitive's synchronous refund separation.
    // Actual Cell/Ticket/Registry guard paths require their own fixtures.
}

#[test]
fn pending_foreign_cutover_keeps_original_g_grant_after_a_refusal_until_outer_guard_ends() {
    let bytes = 2 * Identity::body_bytes();
    let origin = BirthBank::new(bytes);
    let bank = Arc::new(Semaphore::new(bytes));
    let short = origin.raw().clone().try_acquire_owned().unwrap();
    let mut preparation = PendingCutover::<2>::new(&origin, &bank);
    let guard = Arc::new(Mutex::new(()));
    let observed = Arc::new(GuardWake {
        guard: guard.clone(),
        calls: AtomicUsize::new(0),
        all_unlocked: AtomicBool::new(true),
    });
    let waker = Waker::from(observed.clone());
    let mut cx = Context::from_waker(&waker);
    let mut waiter = std::pin::pin!(bank.clone().acquire_many_owned(bytes as u32));
    {
        let _held = guard.lock().unwrap();
        let error = metadata_vec::with_allocation_hook(
            |_, _| panic!("authentic A refusal allocated a marker root"),
            || preparation.prepare().err().unwrap(),
        );
        assert_eq!(
            crate::output::operation_error_class(error.as_ref()),
            ios_core::ErrorClass::ResourceLimit
        );
        assert!(!bank.is_closed());
        assert_eq!(bank.available_permits(), 0);
        assert_eq!(origin.available_permits(), bytes - 1);
        assert!(waiter.as_mut().poll(&mut cx).is_pending());
        assert_eq!(observed.calls.load(Ordering::SeqCst), 0);
    }
    drop(preparation);
    assert!(observed.calls.load(Ordering::SeqCst) > 0);
    assert!(observed.all_unlocked.load(Ordering::SeqCst));
    let permit = match waiter.as_mut().poll(&mut cx) {
        Poll::Ready(Ok(permit)) => permit,
        _ => panic!("the authentic pending G grant was not refunded after A refusal"),
    };
    assert_eq!(permit.num_permits(), bytes);
    drop((permit, short));
    assert_eq!(
        (origin.available_permits(), bank.available_permits()),
        (bytes, bytes)
    );
    assert_eq!(origin.total(), bytes);
}
