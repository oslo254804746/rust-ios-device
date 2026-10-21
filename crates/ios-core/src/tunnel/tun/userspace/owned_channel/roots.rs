//! Complete channel and slot roots, preserving Tokio's original permit queue.
use super::TunnelError;
use std::{
    marker::PhantomData,
    mem::{self, ManuallyDrop},
    ops::Deref,
    ptr::NonNull,
    sync::{
        atomic::{fence, AtomicUsize, Ordering},
        Arc,
    },
};
use tokio::sync::{
    AcquireError, OwnedSemaphorePermit, Semaphore, SemaphorePermit, TryAcquireError,
};

#[repr(C)]
struct Allocation<T> {
    strong: AtomicUsize,
    value: ManuallyDrop<T>,
    storage: ManuallyDrop<OwnedSemaphorePermit>,
}
pub(in crate::tunnel::tun::userspace) struct Root<T> {
    pointer: NonNull<Allocation<T>>,
    _allocation: PhantomData<Allocation<T>>,
}
// SAFETY: roots share only immutable T, whose own locks/atomics guard mutation.
// Final T destruction is in place; no Weak/raw/pin/mutable export exists.
unsafe impl<T: Send + Sync> Send for Root<T> {}
unsafe impl<T: Send + Sync> Sync for Root<T> {}

pub(super) struct Destination<T> {
    values: Vec<Allocation<T>>,
    storage: OwnedSemaphorePermit,
}
impl<T> Destination<T> {
    pub(super) fn from_grant(
        bank: &Arc<Semaphore>,
        storage: OwnedSemaphorePermit,
    ) -> Result<Self, TunnelError> {
        let (storage, values) = crate::device::prepared_rsd::exact_vec_from_grant(bank, 1, storage)
            .map_err(|source| TunnelError::Io(std::io::Error::other(source)))?;
        Ok(Self { values, storage })
    }
    pub(super) fn finish(self, value: T) -> Root<T> {
        assert!(self.values.is_empty() && self.values.capacity() == 1);
        let Self {
            mut values,
            storage,
        } = self;
        values.push(Allocation {
            strong: AtomicUsize::new(1),
            value: ManuallyDrop::new(value),
            storage: ManuallyDrop::new(storage),
        });
        let pointer = NonNull::from(&mut values[0]);
        mem::forget(values);
        Root {
            pointer,
            _allocation: PhantomData,
        }
    }
}
impl<T> Root<T> {
    pub(super) fn body_bytes() -> usize {
        mem::size_of::<Allocation<T>>()
    }
    #[cfg(test)]
    pub(super) fn allocation_address(&self) -> usize {
        self.pointer.as_ptr() as usize
    }
    fn allocation(&self) -> &Allocation<T> {
        // SAFETY: a live root retains a positive count at this stable address.
        unsafe { self.pointer.as_ref() }
    }
}
impl<T> Clone for Root<T> {
    fn clone(&self) -> Self {
        if self.allocation().strong.fetch_add(1, Ordering::Relaxed) >= isize::MAX as usize {
            std::process::abort();
        }
        Self {
            pointer: self.pointer,
            _allocation: PhantomData,
        }
    }
}
impl<T> Deref for Root<T> {
    type Target = T;
    fn deref(&self) -> &T {
        &self.allocation().value
    }
}
struct Release<T> {
    pointer: NonNull<Allocation<T>>,
    _storage: OwnedSemaphorePermit,
}
impl<T> Drop for Release<T> {
    fn drop(&mut self) {
        // SAFETY: the unique original T finished its in-place drop glue, also
        // on unwind. Empty Vec frees precisely the same complete allocation.
        unsafe { drop(Vec::from_raw_parts(self.pointer.as_ptr(), 0, 1)) };
        #[cfg(test)]
        AFTER_FREE.with(|hook| {
            if let Some(callback) = hook.borrow_mut().as_mut() {
                callback(
                    std::alloc::Layout::new::<Allocation<T>>(),
                    self.pointer.as_ptr() as usize,
                );
            }
        });
    }
}
impl<T> Drop for Root<T> {
    fn drop(&mut self) {
        if self.allocation().strong.fetch_sub(1, Ordering::Release) != 1 {
            return;
        }
        fence(Ordering::Acquire);
        let pointer = self.pointer.as_ptr();
        // SAFETY: final ownership acquired all prior releases. Move ONLY the
        // permit into a stack guard, never T, before its in-place destruction.
        let storage = unsafe { ManuallyDrop::take(&mut (*pointer).storage) };
        let _release = Release {
            pointer: self.pointer,
            _storage: storage,
        };
        unsafe { ManuallyDrop::drop(&mut (*pointer).value) };
    }
}

#[derive(Clone)]
pub(super) struct Slots(Root<Semaphore>);
impl Slots {
    pub(super) fn body_bytes() -> usize {
        Root::<Semaphore>::body_bytes()
    }
    pub(super) fn place(destination: Destination<Semaphore>, capacity: usize) -> Self {
        Self(destination.finish(Semaphore::new(capacity)))
    }
    pub(super) async fn acquire_owned(self) -> Result<SlotPermit, AcquireError> {
        let permit = self.0.acquire().await?;
        // SAFETY: this permit was acquired from this exact stable root.
        Ok(unsafe { Self::retain(permit, &self) })
    }
    pub(super) fn try_acquire_owned(self) -> Result<SlotPermit, TryAcquireError> {
        let permit = self.0.try_acquire()?;
        // SAFETY: this permit was acquired from this exact stable root.
        Ok(unsafe { Self::retain(permit, &self) })
    }
    /// Safety: permit must belong to this exact owner's original Semaphore.
    unsafe fn retain(permit: SemaphorePermit<'_>, owner: &Self) -> SlotPermit {
        // Clone the same immutable stable root before extending this private
        // borrowed permit's lifetime. No native permit escapes SlotPermit.
        let owner = owner.clone();
        // SAFETY: the original Semaphore never moves out of its Root. The
        // retained owner keeps it alive, and _permit drops before _owner on
        // normal destruction and unwind. Moving the wrapper moves no Semaphore.
        let permit =
            unsafe { mem::transmute::<SemaphorePermit<'_>, SemaphorePermit<'static>>(permit) };
        SlotPermit {
            _permit: permit,
            _owner: owner,
        }
    }
    pub(super) fn close(&self) {
        self.0.close();
    }
    #[cfg(test)]
    pub(super) fn allocation_address(&self) -> usize {
        self.0.allocation_address()
    }
}

pub(super) struct SlotPermit {
    // Field order returns the ORIGINAL Tokio permit while its stable root is
    // still alive; only then may the last root free its Semaphore and fee.
    _permit: SemaphorePermit<'static>,
    _owner: Slots,
}

#[cfg(test)]
type FreeHook = std::cell::RefCell<Option<Box<dyn FnMut(std::alloc::Layout, usize)>>>;
#[cfg(test)]
thread_local! { static AFTER_FREE: FreeHook = const { std::cell::RefCell::new(None) }; }
#[cfg(test)]
pub(super) fn after_free<T>(
    hook: impl FnMut(std::alloc::Layout, usize) + 'static,
    work: impl FnOnce() -> T,
) -> T {
    struct Reset;
    impl Drop for Reset {
        fn drop(&mut self) {
            AFTER_FREE.with(|hook| hook.borrow_mut().take());
        }
    }
    AFTER_FREE.with(|pending| {
        assert!(pending.borrow().is_none());
        *pending.borrow_mut() = Some(Box::new(hook));
    });
    let _reset = Reset;
    work()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::device::prepared_rsd::{charge, with_allocation_hook};
    use std::{
        alloc::Layout,
        marker::PhantomPinned,
        panic::{catch_unwind, AssertUnwindSafe},
        sync::{Barrier, Mutex},
    };

    #[test]
    fn channel_roots_refuse_foreign_short_long_grants_before_any_backing() {
        let bytes = Root::<Semaphore>::body_bytes();
        for (foreign, amount) in [(true, bytes), (false, bytes - 1), (false, bytes + 1)] {
            let bank = Arc::new(Semaphore::new(2 * bytes));
            let other = Arc::new(Semaphore::new(2 * bytes));
            let grant = charge(if foreign { &other } else { &bank }, amount).unwrap();
            let error = with_allocation_hook(
                |_, _| panic!("invalid root grant allocated"),
                || match Destination::<Semaphore>::from_grant(&bank, grant) {
                    Err(error) => error,
                    Ok(_) => panic!("invalid root accepted"),
                },
            );
            assert_eq!(
                crate::classify_error(&error),
                crate::ErrorClass::ResourceLimit
            );
            assert_eq!(bank.available_permits(), 2 * bytes);
            assert_eq!(other.available_permits(), 2 * bytes);
        }
    }

    struct Probe {
        bank: Arc<Semaphore>,
        events: Arc<Mutex<Vec<usize>>>,
        panic: bool,
        _pinned: PhantomPinned,
    }
    struct OriginalPanic;
    impl Drop for Probe {
        fn drop(&mut self) {
            assert_eq!(
                self.bank.available_permits(),
                0,
                "root fee preceded original value Drop"
            );
            let pointer = self as *mut Self as usize;
            self.events.lock().unwrap().push(pointer);
            if self.panic {
                std::panic::panic_any(OriginalPanic);
            }
        }
    }
    #[test]
    fn channel_root_keeps_non_unpin_value_in_place_through_original_drop_and_unwind_before_actual_free_credit(
    ) {
        for panic_value in [false, true] {
            let bytes = Root::<Probe>::body_bytes();
            let bank = Arc::new(Semaphore::new(bytes));
            let events = Arc::new(Mutex::new(Vec::new()));
            let root = Destination::from_grant(&bank, charge(&bank, bytes).unwrap())
                .unwrap()
                .finish(Probe {
                    bank: bank.clone(),
                    events: events.clone(),
                    panic: panic_value,
                    _pinned: PhantomPinned,
                });
            let value = (&*root as *const Probe) as usize;
            let pointer = root.allocation_address();
            bank.close();
            let clone = root.clone();
            assert_eq!(clone.allocation_address(), pointer);
            drop(root);
            let seen = events.clone();
            let credit = bank.clone();
            let result = after_free(
                move |layout, address| {
                    assert_eq!(layout, Layout::new::<Allocation<Probe>>());
                    assert_eq!(address, pointer);
                    assert_eq!(&*seen.lock().unwrap(), &[value]);
                    assert_eq!(
                        credit.available_permits(),
                        0,
                        "fee preceded actual deallocation"
                    );
                },
                || catch_unwind(AssertUnwindSafe(|| drop(clone))),
            );
            if panic_value {
                assert!(result
                    .expect_err("original panic lost")
                    .is::<OriginalPanic>());
            } else {
                result.unwrap();
            }
            assert_eq!(bank.available_permits(), bytes);
        }
    }

    #[tokio::test]
    async fn channel_native_slot_permit_keeps_original_stable_semaphore_after_other_root_ends() {
        let bytes = Slots::body_bytes();
        let bank = Arc::new(Semaphore::new(bytes));
        let slots = Slots::place(
            Destination::from_grant(&bank, charge(&bank, bytes).unwrap()).unwrap(),
            1,
        );
        let pointer = slots.allocation_address();
        let semaphore = (&*slots.0 as *const Semaphore) as usize;
        bank.close();
        let permit = slots.clone().acquire_owned().await.unwrap();
        assert_eq!(permit._owner.allocation_address(), pointer);
        assert_eq!((&*permit._owner.0 as *const Semaphore) as usize, semaphore);
        assert_eq!(slots.0.available_permits(), 0);
        assert!(matches!(
            slots.clone().try_acquire_owned(),
            Err(TryAcquireError::NoPermits)
        ));
        drop(permit);
        assert_eq!(slots.0.available_permits(), 1);
        let kept = slots.clone().try_acquire_owned().unwrap();
        slots.close();
        assert!(matches!(
            slots.clone().try_acquire_owned(),
            Err(TryAcquireError::Closed)
        ));
        drop(slots);
        assert_eq!(bank.available_permits(), 0);
        let credit = bank.clone();
        after_free(
            move |_, address| {
                assert_eq!(address, pointer);
                assert_eq!(credit.available_permits(), 0);
            },
            || drop(kept),
        );
        assert_eq!(bank.available_permits(), bytes);
    }

    #[test]
    fn channel_parallel_last_roots_keep_same_declared_semaphore_and_refund_once() {
        let bytes = Slots::body_bytes();
        let bank = Arc::new(Semaphore::new(bytes));
        let slots = Slots::place(
            Destination::from_grant(&bank, charge(&bank, bytes).unwrap()).unwrap(),
            1,
        );
        let address = slots.allocation_address();
        bank.close();
        let barrier = Arc::new(Barrier::new(8));
        std::thread::scope(|scope| {
            for _ in 0..7 {
                let kept = slots.clone();
                let barrier = barrier.clone();
                let bank = bank.clone();
                scope.spawn(move || {
                    assert_eq!(kept.allocation_address(), address);
                    assert_eq!(bank.available_permits(), 0);
                    barrier.wait();
                    drop(kept);
                });
            }
            barrier.wait();
            drop(slots);
        });
        assert_eq!(bank.available_permits(), bytes);
    }
}
