//! Actual Vec backing admission, including both allocations during growth.
use std::alloc::{alloc, Layout};
use std::ptr::NonNull;
use std::sync::Arc;

use anyhow::Result;
use tokio::sync::{OwnedSemaphorePermit, Semaphore};

/// Dropping an unused result deallocates values before returning its grant.
/// When moving fields to locals, bind storage first for reverse local Drop.
pub(super) struct AdmittedVec<T> {
    pub(super) values: Vec<T>,
    pub(super) storage: OwnedSemaphorePermit,
}

pub(super) fn exact_backing<T>(bank: &Arc<Semaphore>, capacity: usize) -> Result<AdmittedVec<T>> {
    let layout = Layout::array::<T>(capacity)
        .map_err(|_| crate::output::resource_decision("tunnel owned vector layout overflow"))?;
    let storage = super::admission::known_metadata_charge(bank, layout.size())?;
    allocate_backing(capacity, layout, storage)
}

pub(super) fn from_grant<T>(
    bank: &Arc<Semaphore>,
    capacity: usize,
    storage: OwnedSemaphorePermit,
) -> Result<AdmittedVec<T>> {
    let layout = Layout::array::<T>(capacity)
        .map_err(|_| crate::output::resource_decision("tunnel owned vector layout overflow"))?;
    anyhow::ensure!(
        Arc::ptr_eq(storage.semaphore(), bank) && storage.num_permits() == layout.size(),
        crate::output::resource_decision("tunnel prepaid vector grant mismatch")
    );
    allocate_backing(capacity, layout, storage)
}

/// The caller retains the authentic grant across allocation refusal/unwind.
/// This lets an outer transaction refund only after its enclosing guards end.
pub(super) fn from_borrowed_grant<T>(
    bank: &Arc<Semaphore>,
    capacity: usize,
    storage: &mut Option<OwnedSemaphorePermit>,
) -> Result<AdmittedVec<T>> {
    let layout = Layout::array::<T>(capacity)
        .map_err(|_| crate::output::resource_decision("tunnel owned vector layout overflow"))?;
    let grant = storage.as_ref().expect("original pending vector grant");
    anyhow::ensure!(
        Arc::ptr_eq(grant.semaphore(), bank) && grant.num_permits() == layout.size(),
        crate::output::resource_decision("tunnel prepaid vector grant mismatch")
    );
    let values = allocate_values(capacity, layout)?;
    Ok(AdmittedVec {
        values,
        storage: storage.take().expect("successful original vector grant"),
    })
}

fn allocate_backing<T>(
    capacity: usize,
    layout: Layout,
    storage: OwnedSemaphorePermit,
) -> Result<AdmittedVec<T>> {
    let values = allocate_values(capacity, layout)?;
    Ok(AdmittedVec { values, storage })
}

fn allocate_values<T>(capacity: usize, layout: Layout) -> Result<Vec<T>> {
    #[cfg(test)]
    if layout.size() != 0
        && REFUSE_ALLOCATION.with(|hook| {
            hook.borrow_mut()
                .as_mut()
                .is_some_and(|refuse| refuse(layout))
        })
    {
        return Err(crate::output::resource_decision(
            "tunnel owned vector allocation failed",
        ));
    }
    let values = if layout.size() == 0 {
        Vec::with_capacity(capacity)
    } else {
        let pointer = NonNull::new(unsafe { alloc(layout) }.cast::<T>()).ok_or_else(|| {
            crate::output::resource_decision("tunnel owned vector allocation failed")
        })?;
        // SAFETY: the exact nonzero T array layout is admitted before alloc.
        // No T is initialized; Vec owns and frees this same pointer/layout.
        unsafe { Vec::from_raw_parts(pointer.as_ptr(), 0, capacity) }
    };
    #[cfg(test)]
    if layout.size() != 0 {
        ALLOCATED.with(|hook| {
            if let Some(callback) = hook.borrow_mut().as_mut() {
                callback(layout, values.as_ptr() as usize);
            }
        });
    }
    Ok(values)
}

#[cfg(test)]
type AllocationRefusal = std::cell::RefCell<Option<Box<dyn FnMut(Layout) -> bool>>>;
#[cfg(test)]
thread_local! {
    static REFUSE_ALLOCATION: AllocationRefusal = const { std::cell::RefCell::new(None) };
}
#[cfg(test)]
pub(super) struct RefusalGuard;
#[cfg(test)]
impl Drop for RefusalGuard {
    fn drop(&mut self) {
        REFUSE_ALLOCATION.with(|hook| hook.borrow_mut().take());
    }
}
#[cfg(test)]
pub(super) fn refuse_allocation(hook: impl FnMut(Layout) -> bool + 'static) -> RefusalGuard {
    REFUSE_ALLOCATION.with(|pending| {
        let mut pending = pending.borrow_mut();
        assert!(
            pending.is_none(),
            "allocation refusal hook already installed"
        );
        *pending = Some(Box::new(hook));
    });
    RefusalGuard
}

#[cfg(test)]
type AllocationHook = std::cell::RefCell<Option<Box<dyn FnMut(Layout, usize)>>>;
#[cfg(test)]
thread_local! {
    static ALLOCATED: AllocationHook = const { std::cell::RefCell::new(None) };
}
#[cfg(test)]
pub(super) fn with_allocation_hook<R>(
    hook: impl FnMut(Layout, usize) + 'static,
    operation: impl FnOnce() -> R,
) -> R {
    let _reset = on_allocation(hook);
    operation()
}

#[cfg(test)]
pub(super) struct AllocationGuard;
#[cfg(test)]
impl Drop for AllocationGuard {
    fn drop(&mut self) {
        ALLOCATED.with(|pending| pending.borrow_mut().take());
    }
}
#[cfg(test)]
pub(super) fn on_allocation(hook: impl FnMut(Layout, usize) + 'static) -> AllocationGuard {
    ALLOCATED.with(|pending| {
        let mut pending = pending.borrow_mut();
        assert!(
            pending.is_none(),
            "vector allocation hook already installed"
        );
        *pending = Some(Box::new(hook));
    });
    AllocationGuard
}

pub(super) struct KnownVec<T> {
    values: Vec<T>,
    storage: Option<OwnedSemaphorePermit>,
    bank: Arc<Semaphore>,
    ceiling: usize,
}

impl<T> KnownVec<T> {
    pub(super) fn new(bank: Arc<Semaphore>, ceiling: usize) -> Self {
        Self {
            values: Vec::new(),
            storage: None,
            bank,
            ceiling,
        }
    }

    pub(super) fn reserve(&mut self, additional: usize) -> Result<()> {
        let required = self
            .values
            .len()
            .checked_add(additional)
            .filter(|required| *required <= self.ceiling)
            .ok_or_else(|| {
                crate::output::resource_decision("tunnel metadata vector entry limit")
            })?;
        if required <= self.values.capacity() {
            return Ok(());
        }
        let capacity = self
            .values
            .capacity()
            .max(1)
            .saturating_mul(2)
            .max(required)
            .min(self.ceiling);
        let allocation = exact_backing(&self.bank, capacity)?;
        let storage = allocation.storage;
        let mut replacement = allocation.values;
        replacement.append(&mut self.values);
        self.values = replacement;
        self.storage = Some(storage);
        Ok(())
    }

    pub(super) fn push(&mut self, value: T) -> Result<()> {
        self.reserve(1)?;
        self.values.push(value);
        Ok(())
    }

    pub(super) fn as_slice(&self) -> &[T] {
        &self.values
    }
    pub(super) fn as_mut_slice(&mut self) -> &mut [T] {
        &mut self.values
    }
    #[cfg(test)]
    pub(super) fn capacity(&self) -> usize {
        self.values.capacity()
    }
    /// The caller reserved this slot before acquiring an irreplaceable value.
    pub(super) fn push_reserved(&mut self, value: T) {
        assert!(self.values.len() < self.values.capacity());
        assert!(self.values.len() < self.ceiling);
        self.values.push(value);
    }
    pub(super) fn remove(&mut self, index: usize) -> T {
        self.values.remove(index)
    }
    /// Restore a removal without allocation; the original backing is retained.
    pub(super) fn insert_reserved(&mut self, index: usize, value: T) {
        assert!(self.values.len() < self.values.capacity());
        assert!(self.values.len() < self.ceiling);
        self.values.insert(index, value);
    }
    pub(super) fn len(&self) -> usize {
        self.values.len()
    }
    pub(super) fn retain(&mut self, keep: impl FnMut(&T) -> bool) {
        self.values.retain(keep);
    }

    pub(super) fn dedup_by(&mut self, same: impl FnMut(&mut T, &mut T) -> bool) {
        self.values.dedup_by(same);
    }

    pub(super) fn extend_copy(&mut self, values: &[T]) -> Result<()>
    where
        T: Copy,
    {
        self.reserve(values.len())?;
        self.values.extend_from_slice(values);
        Ok(())
    }

    pub(super) fn take(&mut self) -> Self {
        let empty = Self::new(Arc::clone(&self.bank), self.ceiling);
        std::mem::replace(self, empty)
    }
}

pub(super) struct IntoIter<T> {
    values: std::vec::IntoIter<T>,
    _storage: Option<OwnedSemaphorePermit>,
}

impl<T> Iterator for IntoIter<T> {
    type Item = T;
    fn next(&mut self) -> Option<T> {
        self.values.next()
    }
    fn size_hint(&self) -> (usize, Option<usize>) {
        self.values.size_hint()
    }
}

impl<T> ExactSizeIterator for IntoIter<T> {}

impl<T> IntoIterator for KnownVec<T> {
    type Item = T;
    type IntoIter = IntoIter<T>;
    fn into_iter(self) -> Self::IntoIter {
        IntoIter {
            values: self.values.into_iter(),
            _storage: self.storage,
        }
    }
}

#[cfg(test)]
mod tests;
