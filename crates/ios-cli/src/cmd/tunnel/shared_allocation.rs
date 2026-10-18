//! Exact application-owned shared allocation, including its control block.
//! No raw pointer, Weak, mutation or pinning interface escapes. The original
//! layout is admitted once, freed before value/permit Drop, and never guessed
//! from a standard library Arc implementation.
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

#[repr(C)]
struct Allocation<T> {
    strong: AtomicUsize,
    value: ManuallyDrop<T>,
    storage: ManuallyDrop<OwnedSemaphorePermit>,
}

pub(super) struct Shared<T: Unpin> {
    pointer: NonNull<Allocation<T>>,
    // This root owns T despite storing a raw allocation pointer. Keep dropck,
    // variance and auto-trait ownership tied to the actual declared allocation.
    _allocation: PhantomData<Allocation<T>>,
}
// SAFETY: roots only share immutable T, as Arc does. Clones count atomically;
// a single final root moves T only after all other owning roots have ended.
unsafe impl<T: Send + Sync + Unpin> Send for Shared<T> {}
unsafe impl<T: Send + Sync + Unpin> Sync for Shared<T> {}

impl<T: Unpin> Shared<T> {
    pub(super) fn allocation_bytes() -> usize {
        mem::size_of::<Allocation<T>>()
    }
    pub(super) fn from_grant(
        value: T,
        bank: &Arc<Semaphore>,
        storage: OwnedSemaphorePermit,
    ) -> Result<Self> {
        // The existing exact allocator validates the original bank and WHOLE
        // declared layout before alloc. No std Arc/Box is allocated alongside it.
        let AdmittedVec {
            mut values,
            storage,
        } = from_grant::<Allocation<T>>(bank, 1, storage)?;
        values.push(Allocation {
            strong: AtomicUsize::new(1),
            value: ManuallyDrop::new(value),
            storage: ManuallyDrop::new(storage),
        });
        let pointer = NonNull::from(&mut values[0]);
        mem::forget(values);
        Ok(Self {
            pointer,
            _allocation: PhantomData,
        })
    }
    #[cfg(test)]
    pub(super) fn ptr_eq(&self, other: &Self) -> bool {
        self.pointer == other.pointer
    }
    fn allocation(&self) -> &Allocation<T> {
        // SAFETY: every live Shared owns one positive count. Its allocation
        // stays initialized at the original address until the final Drop.
        unsafe { self.pointer.as_ref() }
    }
    #[cfg(test)]
    pub(super) fn storage(&self) -> &OwnedSemaphorePermit {
        &self.allocation().storage
    }
}
impl<T: Unpin> Clone for Shared<T> {
    fn clone(&self) -> Self {
        // Owning self already protects the allocation; increment needs no
        // synchronization of T. Prevent reference-count overflow like Arc.
        if self.allocation().strong.fetch_add(1, Ordering::Relaxed) >= isize::MAX as usize {
            std::process::abort();
        }
        Self {
            pointer: self.pointer,
            _allocation: PhantomData,
        }
    }
}
impl<T: Unpin> Deref for Shared<T> {
    type Target = T;
    fn deref(&self) -> &T {
        &self.allocation().value
    }
}
impl<T: std::fmt::Debug + Unpin> std::fmt::Debug for Shared<T> {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.deref().fmt(formatter)
    }
}
impl<T: Unpin> Drop for Shared<T> {
    fn drop(&mut self) {
        if self.allocation().strong.fetch_sub(1, Ordering::Release) != 1 {
            return;
        }
        // The final root observes prior owners' releases before reading/moving
        // the original value. No Weak or mutable/pinned reference can escape.
        fence(Ordering::Acquire);
        let pointer = self.pointer.as_ptr();
        // SAFETY: count became zero exactly once, and no live root/borrow can
        // reach this allocation. Both ManuallyDrop fields are initialized and
        // moved once. Bind storage first so unwinding drops the value before it.
        let storage = unsafe { ManuallyDrop::take(&mut (*pointer).storage) };
        let value = unsafe { ManuallyDrop::take(&mut (*pointer).value) };
        // SAFETY: from_grant allocated EXACTLY Array<Allocation<T>>(1). Both
        // owning fields have moved out. Length zero drops no fields; capacity
        // one deallocates the SAME original nonzero layout (counter is inline).
        unsafe {
            drop(Vec::from_raw_parts(pointer, 0, 1));
        }
        #[cfg(test)]
        AFTER_FREE.with(|hook| {
            if let Some(callback) = hook.borrow_mut().as_mut() {
                callback();
            }
        });
        drop(value);
        drop(storage);
    }
}

#[cfg(test)]
type FreeHook = std::cell::RefCell<Option<Box<dyn FnMut()>>>;
#[cfg(test)]
thread_local! { static AFTER_FREE: FreeHook = const { std::cell::RefCell::new(None) }; }
#[cfg(test)]
fn after_free<R>(hook: impl FnMut() + 'static, work: impl FnOnce() -> R) -> R {
    struct Reset;
    impl Drop for Reset {
        fn drop(&mut self) {
            AFTER_FREE.with(|pending| pending.borrow_mut().take());
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
mod tests;
