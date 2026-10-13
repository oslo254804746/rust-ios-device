//! Actual Vec backing admission, including both allocations during growth.
use std::sync::Arc;

use anyhow::Result;
use tokio::sync::{OwnedSemaphorePermit, Semaphore};

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
        let mut replacement = Vec::new();
        replacement.try_reserve_exact(capacity).map_err(|_| {
            crate::output::resource_decision("tunnel metadata vector allocation failed")
        })?;
        let storage = super::admission::known_vec_backing(&self.bank, &replacement)?;
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
