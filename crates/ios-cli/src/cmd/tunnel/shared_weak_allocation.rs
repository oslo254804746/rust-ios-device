//! Declared shared allocation with strong/weak lifetime and original body fees.
//! Fields end at last strong; body fees end AFTER the last weak deallocation.
//! No standard Arc layout assumptions or secondary guard allocation.
use super::metadata_vec::{from_grant, AdmittedVec};
use anyhow::Result;
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
use tokio::sync::{OwnedSemaphorePermit, Semaphore};

struct Fees<F> {
    first: OwnedSemaphorePermit,
    extra: F,
}
// Keep fees before the value so an over-aligned T does not leave trailing fee
// padding. Every byte, including alignment, is measured from this actual type.
#[repr(C)]
struct Allocation<T, F> {
    strong: AtomicUsize,
    weak: AtomicUsize,
    fees: ManuallyDrop<Fees<F>>,
    value: ManuallyDrop<T>,
}
pub(super) struct Strong<T, F: Unpin> {
    pointer: NonNull<Allocation<T, F>>,
    _allocation: PhantomData<Allocation<T, F>>,
}

/// The actual complete allocation, still empty until its original value exists.
pub(super) struct Prepared<T, F: Unpin> {
    values: Vec<Allocation<T, F>>,
    // Unused empty storage is physically freed before this fee returns.
    storage: OwnedSemaphorePermit,
}

impl<T, F: Unpin> Prepared<T, F> {
    pub(super) fn from_borrowed_grant(
        bank: &Arc<Semaphore>,
        first: &mut Option<OwnedSemaphorePermit>,
    ) -> Result<Self> {
        let AdmittedVec { values, storage } =
            super::metadata_vec::from_borrowed_grant::<Allocation<T, F>>(bank, 1, first)?;
        Ok(Self { values, storage })
    }

    pub(super) fn from_grant(bank: &Arc<Semaphore>, first: OwnedSemaphorePermit) -> Result<Self> {
        let AdmittedVec { values, storage } = from_grant::<Allocation<T, F>>(bank, 1, first)?;
        Ok(Self { values, storage })
    }

    pub(super) fn place(self, value: T, extra: F) -> Strong<T, F> {
        let Self {
            mut values,
            storage,
        } = self;
        values.push(Allocation {
            strong: AtomicUsize::new(1),
            weak: AtomicUsize::new(1),
            fees: ManuallyDrop::new(Fees {
                first: storage,
                extra,
            }),
            value: ManuallyDrop::new(value),
        });
        let pointer = NonNull::from(&mut values[0]);
        mem::forget(values);
        Strong {
            pointer,
            _allocation: PhantomData,
        }
    }
}
pub(super) struct Weak<T, F: Unpin> {
    pointer: NonNull<Allocation<T, F>>,
    _allocation: PhantomData<Allocation<T, F>>,
}
// SAFETY: owning references count atomically, immutable T is only accessible
// while strong>0, and the implicit weak protects its in-place destructor.
unsafe impl<T: Send + Sync, F: Send + Sync + Unpin> Send for Strong<T, F> {}
unsafe impl<T: Send + Sync, F: Send + Sync + Unpin> Sync for Strong<T, F> {}
unsafe impl<T: Send + Sync, F: Send + Sync + Unpin> Send for Weak<T, F> {}
unsafe impl<T: Send + Sync, F: Send + Sync + Unpin> Sync for Weak<T, F> {}

impl<T, F: Unpin> Strong<T, F> {
    pub(super) fn body_bytes() -> usize {
        mem::size_of::<Allocation<T, F>>()
    }
    pub(super) fn new(
        value: T,
        bank: &Arc<Semaphore>,
        first: OwnedSemaphorePermit,
        extra: F,
    ) -> Result<Self> {
        // Other-bank validation belongs to the two-bank caller and precedes
        // this exact first-bank allocator. Extra fees remain held on failure.
        let extra = extra;
        let value = value;
        let prepared = Prepared::from_grant(bank, first)?;
        Ok(prepared.place(value, extra))
    }
    fn allocation(&self) -> &Allocation<T, F> {
        // SAFETY: this strong reference keeps the whole allocation and T live.
        unsafe { self.pointer.as_ref() }
    }
    pub(super) fn first(&self) -> &OwnedSemaphorePermit {
        &self.allocation().fees.first
    }
    pub(super) fn extra(&self) -> &F {
        &self.allocation().fees.extra
    }
    pub(super) fn ptr_eq(&self, other: &Self) -> bool {
        self.pointer == other.pointer
    }
    #[cfg(test)]
    pub(super) fn strong_count(&self) -> usize {
        self.allocation().strong.load(Ordering::Acquire)
    }
    pub(super) fn downgrade(&self) -> Weak<T, F> {
        let mut count = self.allocation().weak.load(Ordering::Relaxed);
        loop {
            // get_mut briefly locks the implicit-only weak count. A concurrent
            // strong downgrade must wait until its uniqueness check ends.
            if count == usize::MAX {
                std::hint::spin_loop();
                count = self.allocation().weak.load(Ordering::Relaxed);
                continue;
            }
            if count >= isize::MAX as usize {
                std::process::abort();
            }
            // Acquire pairs with the uniqueness lock's Release unlock, so
            // its prior strong-count observation precedes this downgrade.
            match self.allocation().weak.compare_exchange_weak(
                count,
                count + 1,
                Ordering::Acquire,
                Ordering::Relaxed,
            ) {
                Ok(_) => {
                    return Weak {
                        pointer: self.pointer,
                        _allocation: PhantomData,
                    }
                }
                Err(actual) => count = actual,
            }
        }
    }
    #[cfg(test)]
    pub(super) fn get_mut(&mut self) -> Option<&mut T> {
        if self
            .allocation()
            .weak
            .compare_exchange(1, usize::MAX, Ordering::Acquire, Ordering::Relaxed)
            .is_err()
        {
            return None;
        }
        let locked = WeakLock {
            pointer: self.pointer,
        };
        BEFORE_UNIQUE.with(|hook| {
            if let Some(callback) = hook.borrow_mut().as_mut() {
                callback();
            }
        });
        let unique = self.allocation().strong.load(Ordering::Acquire) == 1;
        drop(locked);
        if !unique {
            return None;
        }
        // SAFETY: exclusive self, strong1 and no explicit weak atomically
        // verified above. T has never been exposed mutably by another root.
        Some(unsafe { &mut (*self.pointer.as_ptr()).value })
    }
    #[cfg(test)]
    pub(super) fn replace_first(
        &mut self,
        bank: &Arc<Semaphore>,
        incoming: OwnedSemaphorePermit,
    ) -> Result<OwnedSemaphorePermit> {
        anyhow::ensure!(
            self.get_mut().is_some()
                && Arc::ptr_eq(incoming.semaphore(), bank)
                && incoming.num_permits() == Self::body_bytes(),
            crate::output::resource_decision(
                "configure unique weak allocation with whole body grant"
            )
        );
        // SAFETY: uniqueness is preserved by exclusive self and no weak can
        // appear after get_mut succeeds. The NEW whole original grant is held.
        let fees: &mut Fees<F> = unsafe { &mut (*self.pointer.as_ptr()).fees };
        Ok(mem::replace(&mut fees.first, incoming))
    }
}
impl<T, F: Unpin> Clone for Strong<T, F> {
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
impl<T, F: Unpin> Deref for Strong<T, F> {
    type Target = T;
    fn deref(&self) -> &T {
        &self.allocation().value
    }
}
impl<T: std::fmt::Debug, F: Unpin> std::fmt::Debug for Strong<T, F> {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.deref().fmt(formatter)
    }
}
impl<T: std::fmt::Display, F: Unpin> std::fmt::Display for Strong<T, F> {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.deref().fmt(formatter)
    }
}
impl<T, F: Unpin> Drop for Strong<T, F> {
    fn drop(&mut self) {
        if self.allocation().strong.fetch_sub(1, Ordering::Release) != 1 {
            return;
        }
        fence(Ordering::Acquire);
        // The implicit weak is released even if the ORIGINAL T destructor
        // unwinds. It protects the allocation throughout that destructor.
        let implicit = ImplicitWeak {
            pointer: self.pointer,
        };
        // SAFETY: strong became zero exactly once, upgrades cannot revive it.
        // Drop T in place (including !Unpin); explicit Weak cannot access T.
        unsafe {
            ManuallyDrop::drop(&mut (*self.pointer.as_ptr()).value);
        }
        drop(implicit);
    }
}
#[cfg(test)]
struct WeakLock<T, F: Unpin> {
    pointer: NonNull<Allocation<T, F>>,
}
#[cfg(test)]
impl<T, F: Unpin> Drop for WeakLock<T, F> {
    fn drop(&mut self) {
        // SAFETY: get_mut owns the implicit-only weak lock and its strong
        // reference keeps the allocation live, including on hook unwind.
        unsafe { &(*self.pointer.as_ptr()).weak }.store(1, Ordering::Release);
    }
}
struct ImplicitWeak<T, F: Unpin> {
    pointer: NonNull<Allocation<T, F>>,
}
impl<T, F: Unpin> Drop for ImplicitWeak<T, F> {
    fn drop(&mut self) {
        release_weak(self.pointer);
    }
}
fn release_weak<T, F: Unpin>(pointer: NonNull<Allocation<T, F>>) {
    // SAFETY: the caller owns one weak count. Only borrow its still-live
    // counter field: T may already have been destroyed, so no &Allocation<T,F>
    // (and therefore no reference to a logically dead T) is formed here.
    let weak = unsafe { &(*pointer.as_ptr()).weak };
    if weak.fetch_sub(1, Ordering::Release) != 1 {
        return;
    }
    fence(Ordering::Acquire);
    // SAFETY: no strong or weak remains, T has completed in-place destruction
    // before its implicit weak was released. Move unpinned original fees once.
    let fees = unsafe { ManuallyDrop::take(&mut (*pointer.as_ptr()).fees) };
    // SAFETY: same exact capacity-one allocation from from_grant. Length zero
    // drops no already-destroyed T/fee; capacity one frees the ORIGINAL layout.
    unsafe {
        drop(Vec::from_raw_parts(pointer.as_ptr(), 0, 1));
    }
    #[cfg(test)]
    AFTER_FREE.with(|hook| {
        if let Some(callback) = hook.borrow_mut().as_mut() {
            callback();
        }
    });
    drop(fees);
}
#[cfg(test)]
impl<T, F: Unpin> Weak<T, F> {
    pub(super) fn upgrade(&self) -> Option<Strong<T, F>> {
        // SAFETY: this explicit weak keeps the counter field live, including
        // after T has ended. Never borrow the whole allocation through Weak.
        let strong = unsafe { &(*self.pointer.as_ptr()).strong };
        let mut count = strong.load(Ordering::Relaxed);
        loop {
            if count == 0 {
                return None;
            }
            if count >= isize::MAX as usize {
                std::process::abort();
            }
            match strong.compare_exchange_weak(
                count,
                count + 1,
                Ordering::Acquire,
                Ordering::Relaxed,
            ) {
                Ok(_) => {
                    return Some(Strong {
                        pointer: self.pointer,
                        _allocation: PhantomData,
                    })
                }
                Err(actual) => count = actual,
            }
        }
    }
}
impl<T, F: Unpin> Clone for Weak<T, F> {
    fn clone(&self) -> Self {
        // SAFETY: explicit weak means weak>=2 or strong=0; get_mut cannot have
        // locked the implicit-only count. The block stays live through clone.
        if unsafe { &(*self.pointer.as_ptr()).weak }.fetch_add(1, Ordering::Relaxed)
            >= isize::MAX as usize
        {
            std::process::abort();
        }
        Self {
            pointer: self.pointer,
            _allocation: PhantomData,
        }
    }
}
impl<T, F: Unpin> Drop for Weak<T, F> {
    fn drop(&mut self) {
        release_weak(self.pointer);
    }
}
#[cfg(test)]
type FreeHook = std::cell::RefCell<Option<Box<dyn FnMut()>>>;
#[cfg(test)]
thread_local! {
    static AFTER_FREE:FreeHook=const{std::cell::RefCell::new(None)};
    static BEFORE_UNIQUE:FreeHook=const{std::cell::RefCell::new(None)};
}
#[cfg(test)]
pub(super) fn after_free<R>(callback: impl FnMut() + 'static, work: impl FnOnce() -> R) -> R {
    struct Reset;
    impl Drop for Reset {
        fn drop(&mut self) {
            AFTER_FREE.with(|hook| hook.borrow_mut().take());
        }
    }
    AFTER_FREE.with(|hook| {
        assert!(hook.borrow().is_none());
        *hook.borrow_mut() = Some(Box::new(callback));
    });
    let _reset = Reset;
    work()
}
#[cfg(test)]
fn before_unique<R>(callback: impl FnMut() + 'static, work: impl FnOnce() -> R) -> R {
    struct Reset;
    impl Drop for Reset {
        fn drop(&mut self) {
            BEFORE_UNIQUE.with(|hook| hook.borrow_mut().take());
        }
    }
    BEFORE_UNIQUE.with(|hook| {
        assert!(hook.borrow().is_none());
        *hook.borrow_mut() = Some(Box::new(callback));
    });
    let _reset = Reset;
    work()
}
#[cfg(test)]
mod tests;
