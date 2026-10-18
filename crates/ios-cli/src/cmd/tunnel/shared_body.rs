//! Paired strong/weak references retain the original allocation body grant.
//! Data fields end at last strong; data allocation and its paid guard payload
//! end before the last body refund. Private Arc headers remain separate.
use std::ops::Deref;
use std::sync::Arc;
#[cfg(test)]
use std::sync::Weak as StdWeak;

use anyhow::Result;
use tokio::sync::{OwnedSemaphorePermit, Semaphore};

use super::shared_value::Shared;

pub(super) struct Strong<T> {
    // Field order is essential: raw data reference ends BEFORE its body lease.
    data: Arc<T>,
    body: Shared<OwnedSemaphorePermit>,
}
#[cfg(test)]
pub(super) struct Weak<T> {
    data: StdWeak<T>,
    body: Shared<OwnedSemaphorePermit>,
}
impl<T> Strong<T> {
    pub(super) fn body_bytes() -> usize {
        std::mem::size_of::<T>() + std::mem::size_of::<OwnedSemaphorePermit>()
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
        // The guard's OWN known payload is included before either Arc exists.
        // Its sealed strong-only holder frees that allocation before permit Drop.
        let body = Shared::new(storage);
        Ok(Self {
            data: Arc::new(value),
            body,
        })
    }
    pub(super) fn ptr_eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.data, &other.data)
    }
    #[cfg(test)]
    pub(super) fn get_mut(&mut self) -> Option<&mut T> {
        Arc::get_mut(&mut self.data)
    }
    #[cfg(test)]
    pub(super) fn rebind_body(&mut self, bank: &Arc<Semaphore>) -> Result<()> {
        anyhow::ensure!(
            self.get_mut().is_some(),
            crate::output::resource_decision("configure shared body before sharing")
        );
        let storage = super::admission::known_metadata_charge(bank, Self::body_bytes())?;
        // The existing data allocation remains paid by its old guard while the
        // new guard is created. Install the prepaid replacement BEFORE freeing
        // the old guard and returning its original bank credit.
        let old = std::mem::replace(&mut self.body, Shared::new(storage));
        drop(old);
        Ok(())
    }
    #[cfg(test)]
    pub(super) fn downgrade(&self) -> Weak<T> {
        Weak {
            data: Arc::downgrade(&self.data),
            body: self.body.clone(),
        }
    }
}
impl<T> Clone for Strong<T> {
    fn clone(&self) -> Self {
        Self {
            data: Arc::clone(&self.data),
            body: self.body.clone(),
        }
    }
}
impl<T> Deref for Strong<T> {
    type Target = T;
    fn deref(&self) -> &T {
        &self.data
    }
}
impl<T: std::fmt::Debug> std::fmt::Debug for Strong<T> {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.data.fmt(formatter)
    }
}
impl<T: std::fmt::Display> std::fmt::Display for Strong<T> {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.data.fmt(formatter)
    }
}
#[cfg(test)]
impl<T> Weak<T> {
    pub(super) fn upgrade(&self) -> Option<Strong<T>> {
        self.data.upgrade().map(|data| Strong {
            data,
            body: self.body.clone(),
        })
    }
}
#[cfg(test)]
impl<T> Clone for Weak<T> {
    fn clone(&self) -> Self {
        Self {
            data: self.data.clone(),
            body: self.body.clone(),
        }
    }
}

#[cfg(test)]
mod tests;
