//! Exact single-bank body allocation retained through the last explicit Weak.
//! Original T fields drop at last strong, body credit after actual deallocation.
use super::shared_weak_allocation as allocation;
use anyhow::Result;
use std::ops::Deref;
use std::sync::Arc;
use tokio::sync::{OwnedSemaphorePermit, Semaphore};

pub(super) struct Strong<T>(allocation::Strong<T, ()>);
#[cfg(test)]
pub(super) struct Weak<T>(allocation::Weak<T, ()>);
impl<T> Strong<T> {
    pub(super) fn body_bytes() -> usize {
        allocation::Strong::<T, ()>::body_bytes()
    }
    pub(super) fn new(
        incoming_value: T,
        bank: &Arc<Semaphore>,
        incoming_storage: OwnedSemaphorePermit,
    ) -> Result<Self> {
        let storage = incoming_storage;
        let value = incoming_value;
        anyhow::ensure!(
            Arc::ptr_eq(storage.semaphore(), bank) && storage.num_permits() == Self::body_bytes(),
            crate::output::resource_decision("tunnel shared body grant mismatch")
        );
        Ok(Self(allocation::Strong::new(value, bank, storage, ())?))
    }
    pub(super) fn ptr_eq(&self, other: &Self) -> bool {
        self.0.ptr_eq(&other.0)
    }
    #[cfg(test)]
    pub(super) fn get_mut(&mut self) -> Option<&mut T> {
        self.0.get_mut()
    }
    #[cfg(test)]
    pub(super) fn rebind_body(&mut self, bank: &Arc<Semaphore>) -> Result<()> {
        anyhow::ensure!(
            self.get_mut().is_some(),
            crate::output::resource_decision("configure shared body before sharing")
        );
        let storage = super::admission::known_metadata_charge(bank, Self::body_bytes())?;
        let old = self.0.replace_first(bank, storage)?;
        drop(old);
        Ok(())
    }
    #[cfg(test)]
    pub(super) fn downgrade(&self) -> Weak<T> {
        Weak(self.0.downgrade())
    }
}
impl<T> Clone for Strong<T> {
    fn clone(&self) -> Self {
        Self(self.0.clone())
    }
}
impl<T> Deref for Strong<T> {
    type Target = T;
    fn deref(&self) -> &T {
        &self.0
    }
}
impl<T: std::fmt::Debug> std::fmt::Debug for Strong<T> {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.deref().fmt(formatter)
    }
}
impl<T: std::fmt::Display> std::fmt::Display for Strong<T> {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.deref().fmt(formatter)
    }
}
#[cfg(test)]
impl<T> Weak<T> {
    pub(super) fn upgrade(&self) -> Option<Strong<T>> {
        self.0.upgrade().map(Strong)
    }
}
#[cfg(test)]
impl<T> Clone for Weak<T> {
    fn clone(&self) -> Self {
        Self(self.0.clone())
    }
}
#[cfg(test)]
mod tests;
