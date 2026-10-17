//! An original value Box, prepaid while empty and paid through final destruction.
use std::ops::{Deref, DerefMut};
use std::sync::Arc;

use anyhow::Result;
use tokio::sync::{OwnedSemaphorePermit, Semaphore};

use super::metadata_vec::{exact_backing, AdmittedVec};

pub(super) struct Destination<T>(AdmittedVec<T>);
impl<T> Destination<T> {
    pub(super) fn new(bank: &Arc<Semaphore>) -> Result<Self> {
        Ok(Self(exact_backing(bank, 1)?))
    }

    pub(super) fn place(self, value: T) -> Owner<T> {
        let storage = self.0.storage;
        let mut values = self.0.values;
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
            _storage: storage,
        }
    }
}

pub(super) struct Owner<T> {
    data: Box<T>,
    // Outside Box: its entire actual allocation is freed before refund.
    _storage: OwnedSemaphorePermit,
}
impl<T> Owner<T> {
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
mod tests;
