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
    #[cfg(test)]
    pub(super) fn ptr_eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.data, &other.data)
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
