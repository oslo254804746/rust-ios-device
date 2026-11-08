//! Exact original body allocation with two original grants held until last Weak.
use super::shared_weak_allocation as allocation;
use anyhow::Result;
use std::ops::Deref;
use std::sync::Arc;
use tokio::sync::{OwnedSemaphorePermit, Semaphore};

pub(super) struct Strong<T>(allocation::Strong<T, OwnedSemaphorePermit>);
#[cfg(test)]
pub(super) struct Weak<T>(allocation::Weak<T, OwnedSemaphorePermit>);
impl<T> Strong<T> {
    pub(super) fn body_bytes() -> usize {
        allocation::Strong::<T, OwnedSemaphorePermit>::body_bytes()
    }
    pub(super) fn new(
        incoming_value: T,
        first_bank: &Arc<Semaphore>,
        second_bank: &Arc<Semaphore>,
        first: OwnedSemaphorePermit,
        second: OwnedSemaphorePermit,
    ) -> Result<Self> {
        let leases = (first, second);
        let value = incoming_value;
        anyhow::ensure!(
            Arc::ptr_eq(leases.0.semaphore(), first_bank)
                && Arc::ptr_eq(leases.1.semaphore(), second_bank)
                && leases.0.num_permits() == Self::body_bytes()
                && leases.1.num_permits() == Self::body_bytes(),
            crate::output::resource_decision("tunnel dual shared body grant mismatch")
        );
        Ok(Self(allocation::Strong::new(
            value, first_bank, leases.0, leases.1,
        )?))
    }
    pub(super) fn banks_match(&self, first: &Arc<Semaphore>, second: &Arc<Semaphore>) -> bool {
        self.first_bank_matches(first) && Arc::ptr_eq(self.0.extra().semaphore(), second)
    }
    pub(super) fn first_bank_matches(&self, first: &Arc<Semaphore>) -> bool {
        Arc::ptr_eq(self.0.first().semaphore(), first)
    }
    pub(super) fn ptr_eq(&self, other: &Self) -> bool {
        self.0.ptr_eq(&other.0)
    }
    #[cfg(test)]
    pub(super) fn downgrade(&self) -> Weak<T> {
        Weak(self.0.downgrade())
    }
    #[cfg(test)]
    pub(super) fn strong_count(&self) -> usize {
        self.0.strong_count()
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
