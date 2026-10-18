//! Two-budget body leases outlive the original data allocation, including Weak.
use std::ops::Deref;
use std::sync::Arc;
#[cfg(test)]
use std::sync::Weak as StdWeak;

use anyhow::Result;
use tokio::sync::{OwnedSemaphorePermit, Semaphore};

use super::shared_value::Shared;

struct Leases {
    first: OwnedSemaphorePermit,
    second: OwnedSemaphorePermit,
}

pub(super) struct Strong<T> {
    // Both strong and weak data references end BEFORE their shared body lease.
    data: Arc<T>,
    body: Shared<Leases>,
}
#[cfg(test)]
pub(super) struct Weak<T> {
    data: StdWeak<T>,
    body: Shared<Leases>,
}

impl<T> Strong<T> {
    pub(super) fn body_bytes() -> usize {
        std::mem::size_of::<T>() + std::mem::size_of::<Leases>()
    }

    pub(super) fn new(
        incoming_value: T,
        first_bank: &Arc<Semaphore>,
        second_bank: &Arc<Semaphore>,
        first: OwnedSemaphorePermit,
        second: OwnedSemaphorePermit,
    ) -> Result<Self> {
        let leases = Leases { first, second };
        let value = incoming_value;
        anyhow::ensure!(
            Arc::ptr_eq(leases.first.semaphore(), first_bank)
                && Arc::ptr_eq(leases.second.semaphore(), second_bank)
                && leases.first.num_permits() == Self::body_bytes()
                && leases.second.num_permits() == Self::body_bytes(),
            crate::output::resource_decision("tunnel dual shared body grant mismatch")
        );
        // BOTH budgets already include this guard's actual payload before
        // either Arc exists. Shared frees the guard allocation before refunds.
        let body = Shared::new(leases);
        Ok(Self {
            data: Arc::new(value),
            body,
        })
    }

    pub(super) fn banks_match(&self, first: &Arc<Semaphore>, second: &Arc<Semaphore>) -> bool {
        self.first_bank_matches(first) && Arc::ptr_eq(self.body.second.semaphore(), second)
    }

    pub(super) fn first_bank_matches(&self, first: &Arc<Semaphore>) -> bool {
        Arc::ptr_eq(self.body.first.semaphore(), first)
    }

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

    #[cfg(test)]
    pub(super) fn strong_count(&self) -> usize {
        Arc::strong_count(&self.data)
    }
}

impl<T> Clone for Strong<T> {
    fn clone(&self) -> Self {
        Self {
            data: self.data.clone(),
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
