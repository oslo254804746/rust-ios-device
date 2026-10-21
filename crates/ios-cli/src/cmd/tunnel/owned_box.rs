//! An original value Box, prepaid while empty and paid through final destruction.
use std::ops::{Deref, DerefMut};
use std::sync::Arc;

use anyhow::Result;
use tokio::sync::{OwnedSemaphorePermit, Semaphore};

use super::metadata_vec::{exact_backing, AdmittedVec};

pub(super) struct Destination<T> {
    values: Vec<T>,
    #[cfg(test)]
    after_free: AfterFree,
    storage: OwnedSemaphorePermit,
}
impl<T> Destination<T> {
    pub(super) fn new(bank: &Arc<Semaphore>) -> Result<Self> {
        Ok(Self::backing(exact_backing(bank, 1)?))
    }

    pub(super) fn from_grant(bank: &Arc<Semaphore>, storage: OwnedSemaphorePermit) -> Result<Self> {
        Ok(Self::backing(super::metadata_vec::from_grant(
            bank, 1, storage,
        )?))
    }

    fn backing(backing: AdmittedVec<T>) -> Self {
        Self {
            values: backing.values,
            #[cfg(test)]
            after_free: AfterFree,
            storage: backing.storage,
        }
    }

    #[cfg(test)]
    pub(super) fn as_ptr(&self) -> *const T {
        self.values.as_ptr()
    }

    pub(super) fn place(self, value: T) -> Owner<T> {
        let Self {
            mut values,
            storage,
            #[cfg(test)]
            after_free,
        } = self;
        assert!(values.is_empty());
        assert!(std::mem::size_of::<T>() == 0 || values.capacity() == 1);
        values.push(value);
        let pointer = values.as_mut_ptr();
        std::mem::forget(values);
        // SAFETY: this exact capacity-one allocation now owns one initialized T.
        // Array<T>(1) and T have the same nonzero layout. ZST's original aligned
        // dangling pointer needs no deallocation. No fallible step intervenes.
        let data = unsafe { Box::from_raw(pointer) };
        Owner {
            data,
            #[cfg(test)]
            after_free,
            _storage: storage,
        }
    }
}

pub(super) struct Owner<T> {
    data: Box<T>,
    #[cfg(test)]
    after_free: AfterFree,
    // Outside Box: its entire actual allocation is freed before refund.
    _storage: OwnedSemaphorePermit,
}
impl<T> Owner<T> {
    pub(super) fn into_inner(self) -> T {
        let Self {
            data,
            #[cfg(test)]
            after_free,
            _storage: storage,
        } = self;
        let pointer = Box::into_raw(data);
        // SAFETY: this sealed owner contains exactly one initialized T in the
        // original global allocation. Array<T>(1) has exactly T's size/alignment;
        // for ZST the original pointer is aligned and needs no allocation. The
        // Box has relinquished ownership, and this sole Vec now owns the same
        // pointer. pop moves T out once; dropping the empty Vec frees the body.
        let mut values = unsafe { Vec::from_raw_parts(pointer, 1, 1) };
        let value = values.pop().expect("original owned Box contains one value");
        drop(values);
        #[cfg(test)]
        drop(after_free);
        drop(storage);
        value
    }

    pub(super) fn bytes_in(&self, bank: &Arc<Semaphore>) -> Result<usize> {
        anyhow::ensure!(
            Arc::ptr_eq(self._storage.semaphore(), bank),
            crate::output::resource_decision("tunnel original value bank mismatch")
        );
        Ok(self._storage.num_permits())
    }
}
impl<T> Deref for Owner<T> {
    type Target = T;
    fn deref(&self) -> &T {
        &self.data
    }
}
impl<T> DerefMut for Owner<T> {
    fn deref_mut(&mut self) -> &mut T {
        &mut self.data
    }
}

#[cfg(test)]
struct AfterFree;
#[cfg(test)]
impl Drop for AfterFree {
    fn drop(&mut self) {
        AFTER_FREE.with(|hook| {
            if let Some(callback) = hook.borrow_mut().as_mut() {
                callback();
            }
        });
    }
}
#[cfg(test)]
type FreeHook = std::cell::RefCell<Option<Box<dyn FnMut()>>>;
#[cfg(test)]
thread_local! { static AFTER_FREE: FreeHook = const { std::cell::RefCell::new(None) }; }
#[cfg(test)]
pub(super) fn after_free<R>(hook: impl FnMut() + 'static, work: impl FnOnce() -> R) -> R {
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
mod tests;
