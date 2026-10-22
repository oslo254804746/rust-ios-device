//! One original count reservation, with its shared payload byte-admitted first.
use std::sync::Arc;

use anyhow::Result;
use tokio::sync::{Semaphore, TryAcquireError};

#[derive(Clone)]
pub(super) struct Counts(super::shared_allocation::Shared<Semaphore>);

impl Counts {
    pub(super) fn body_bytes() -> usize {
        super::shared_allocation::Shared::<Semaphore>::allocation_bytes()
    }

    pub(super) fn new(capacity: usize, bank: &Arc<Semaphore>) -> Result<Self> {
        let storage = super::admission::known_metadata_charge(bank, Self::body_bytes())?;
        let destination = super::shared_allocation::Destination::from_grant(bank, storage)?;
        Ok(Self(destination.place(Semaphore::new(capacity))))
    }

    fn try_acquire_owned(self) -> std::result::Result<Permit, TryAcquireError> {
        // Keep the original Tokio count decision. Transfer its successful one
        // count immediately into this independent root owner, without failure.
        self.0.try_acquire()?.forget();
        Ok(Permit { counts: self })
    }

    #[cfg(test)]
    pub(super) fn fixture(capacity: usize) -> Self {
        Self::new(
            capacity,
            &Arc::new(Semaphore::new(super::MAX_GLOBAL_KNOWN_METADATA_BYTES)),
        )
        .unwrap()
    }

    #[cfg(test)]
    pub(super) fn available_permits(&self) -> usize {
        self.0.available_permits()
    }

    #[cfg(test)]
    pub(super) fn close(&self) {
        self.0.close();
    }

    #[cfg(test)]
    fn ptr_eq(&self, other: &Self) -> bool {
        self.0.ptr_eq(&other.0)
    }

    #[cfg(test)]
    pub(super) fn rebind_empty_metadata(&mut self, bank: &Arc<Semaphore>) -> Result<()> {
        anyhow::ensure!(
            self.0.strong_count() == 1 && !self.0.is_closed(),
            crate::output::resource_decision("configure unique unused failure count root")
        );
        let storage = super::admission::known_metadata_charge(bank, Self::body_bytes())?;
        drop(self.0.replace_storage(bank, storage)?);
        Ok(())
    }
}

struct Permit {
    counts: Counts,
}
impl Drop for Permit {
    fn drop(&mut self) {
        self.counts.0.add_permits(1);
        // Counts is independent of State. Return the count before releasing
        // its actual root, without keeping manager/native owners alive.
    }
}

pub(super) struct Slot {
    _permit: Permit,
}
pub(super) type Owner = super::shared_allocation::Shared<Slot>;

impl Slot {
    pub(super) fn body_bytes() -> usize {
        Owner::allocation_bytes()
    }

    pub(super) fn reserve(counts: &Counts, bytes: &Arc<Semaphore>) -> Result<Owner> {
        // Preserve the original count-capacity refusal priority. A subsequent
        // byte refusal drops this exact reservation before returning the error.
        let permit = counts
            .clone()
            .try_acquire_owned()
            .map_err(|_| crate::output::resource_decision("tunnel failure record capacity"))?;
        let body = super::admission::known_metadata_charge(bytes, Self::body_bytes())?;
        Owner::from_grant(Self { _permit: permit }, bytes, body)
    }

    #[cfg(test)]
    fn num_permits(&self) -> usize {
        1
    }

    #[cfg(test)]
    fn counts(&self) -> &Counts {
        &self._permit.counts
    }
}
impl std::fmt::Debug for Slot {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("FailureSlot").finish_non_exhaustive()
    }
}

#[cfg(test)]
mod tests;
