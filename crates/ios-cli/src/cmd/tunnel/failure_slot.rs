//! One original count reservation, with its shared payload byte-admitted first.
use std::ops::Deref;
use std::sync::Arc;

use anyhow::Result;
use tokio::sync::{OwnedSemaphorePermit, Semaphore};

pub(super) struct Slot {
    permit: OwnedSemaphorePermit,
}
pub(super) type Owner = super::shared_allocation::Shared<Slot>;

impl Slot {
    pub(super) fn body_bytes() -> usize {
        Owner::allocation_bytes()
    }

    pub(super) fn reserve(counts: &Arc<Semaphore>, bytes: &Arc<Semaphore>) -> Result<Owner> {
        // Preserve the original count-capacity refusal priority. A subsequent
        // byte refusal drops this exact reservation before returning the error.
        let permit = counts
            .clone()
            .try_acquire_owned()
            .map_err(|_| crate::output::resource_decision("tunnel failure record capacity"))?;
        let body = super::admission::known_metadata_charge(bytes, Self::body_bytes())?;
        Owner::from_grant(Self { permit }, bytes, body)
    }
}
impl Deref for Slot {
    type Target = OwnedSemaphorePermit;
    fn deref(&self) -> &Self::Target {
        &self.permit
    }
}
impl std::fmt::Debug for Slot {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("FailureSlot").finish_non_exhaustive()
    }
}

#[cfg(test)]
mod tests;
