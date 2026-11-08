//! Authentic immutable capacity of the byte bank which creates identities.
//! Ordinary allocators borrow its original semaphore; availability is not TOTAL.
use std::sync::Arc;

use anyhow::Result;
use tokio::sync::{OwnedSemaphorePermit, Semaphore};

#[derive(Clone)]
pub(super) struct BirthBank {
    raw: Arc<Semaphore>,
    total: usize,
}

impl BirthBank {
    pub(super) fn new(total: usize) -> Self {
        Self {
            raw: Arc::new(Semaphore::new(total)),
            total,
        }
    }

    pub(super) fn raw(&self) -> &Arc<Semaphore> {
        &self.raw
    }

    pub(super) fn origin_charge(&self, bytes: usize) -> Result<OwnedSemaphorePermit> {
        super::admission::known_metadata_charge_with_total(&self.raw, bytes, self.total)
    }

    #[cfg(test)]
    pub(super) fn total(&self) -> usize {
        self.total
    }
}

impl std::ops::Deref for BirthBank {
    type Target = Arc<Semaphore>;

    fn deref(&self) -> &Self::Target {
        &self.raw
    }
}

#[cfg(test)]
mod tests;
