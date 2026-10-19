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
        // The zero-field API keeps its strict exact-layout validation.
        let backing = from_grant::<Allocation<T>>(bank, 1, storage)?;
        Ok(Self::place(value, backing))
    }
    /// One original grant covers this declared allocation and separately owned
    /// known field buffers. Callers must admit those buffers before creating
    /// them and supply their actual controlled capacities, never opaque guesses.
    pub(super) fn from_grant_with_fields(
        value: T,
        bank: &Arc<Semaphore>,
        incoming_storage: OwnedSemaphorePermit,
        field_bytes: usize,
    ) -> Result<Self> {
        // Bind storage before value: every validation/allocation refusal drops
        // original field buffers BEFORE returning their still-held byte credit.
        let mut storage = incoming_storage;
        let value = value;
        let total = Self::allocation_bytes()
            .checked_add(field_bytes)
            .ok_or_else(|| {
                crate::output::resource_decision("tunnel shared fields layout overflow")
            })?;
        anyhow::ensure!(
            Arc::ptr_eq(storage.semaphore(), bank) && storage.num_permits() == total,
            crate::output::resource_decision("tunnel shared fields grant mismatch")
        );
        let body = storage
            .split(Self::allocation_bytes())
            .expect("validated whole allocation grant");
        // The existing allocator independently checks the exact body layout.
        // The field portion remains held locally until the same grant is merged.
        let mut backing = from_grant::<Allocation<T>>(bank, 1, body)?;
        // Same bank and checked total <= original grant. merge cannot reject or
        // overflow; no fallible step follows before both parts enter the root.
        backing.storage.merge(storage);
        Ok(Self::place(value, backing))
    }
    fn place(value: T, backing: AdmittedVec<Allocation<T>>) -> Self {
        let AdmittedVec {
            mut values,
            storage,
        } = backing;
        values.push(Allocation {
            strong: AtomicUsize::new(1),
            value: ManuallyDrop::new(value),
            storage: ManuallyDrop::new(storage),
        });
        let pointer = NonNull::from(&mut values[0]);
        mem::forget(values);
        Self {
            pointer,
            _allocation: PhantomData,
        }
    }
    pub(super) fn ptr_eq(&self, other: &Self) -> bool {
        self.pointer == other.pointer
    }
    fn allocation(&self) -> &Allocation<T> {
        // SAFETY: every live Shared owns one positive count. Its allocation
        // stays initialized at the original address until the final Drop.
        unsafe { self.pointer.as_ref() }
    }
    pub(super) fn strong_count(&self) -> usize {
        self.allocation().strong.load(Ordering::Acquire)
    }
    #[cfg(test)]
    pub(super) fn get_mut(&mut self) -> Option<&mut T> {
        if self.strong_count() != 1 {
            return None;
        }
        // SAFETY: self is exclusively borrowed, there is only one strong root,
        // and this sealed allocation has no Weak/raw/pin escape. Other owners'
        // releases have been acquired, and no borrowed T can outlive its root.
        Some(unsafe { &mut (*self.pointer.as_ptr()).value })
    }
    #[cfg(test)]
    pub(super) fn replace_storage(
        &mut self,
        bank: &Arc<Semaphore>,
        incoming: OwnedSemaphorePermit,
    ) -> Result<OwnedSemaphorePermit> {
        anyhow::ensure!(
            self.strong_count() == 1
                && Arc::ptr_eq(incoming.semaphore(), bank)
                && incoming.num_permits() == self.storage().num_permits(),
            crate::output::resource_decision(
                "configure unique shared allocation with its whole grant"
            )
        );
        // SAFETY: same uniqueness proof as get_mut. The new whole grant already
        // exists. Move out the OLD grant so callers can retain it until every
        // related fixture field has acquired its original-bank replacement.
        Ok(ManuallyDrop::into_inner(unsafe {
            std::mem::replace(
                &mut (*self.pointer.as_ptr()).storage,
                ManuallyDrop::new(incoming),
            )
        }))
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
impl<T: Unpin> AsRef<T> for Shared<T> {
    fn as_ref(&self) -> &T {
        self.deref()
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
