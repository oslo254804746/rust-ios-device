//! Independent metadata fee roots, each with its actual body paid in both pools.
use std::ops::Deref;
use std::sync::Arc;

use anyhow::Result;
use tokio::sync::{OwnedSemaphorePermit, Semaphore};

pub(super) struct Lease {
    value: OwnedSemaphorePermit,
    _cross_body: OwnedSemaphorePermit,
}
pub(super) type Owner = super::shared_value::Shared<Lease>;

impl Lease {
    pub(super) fn body_bytes() -> usize {
        std::mem::size_of::<Self>()
    }

    pub(super) fn pair_bytes() -> usize {
        2 * Self::body_bytes()
    }

    /// Both original whole grants include BOTH lease payloads before either
    /// allocation. Split the cross-pool body fees without coupling root lives.
    pub(super) fn from_grants(
        global_bank: &Arc<Semaphore>,
        publication_bank: &Arc<Semaphore>,
        mut global: OwnedSemaphorePermit,
        mut publication: OwnedSemaphorePermit,
    ) -> Result<(Owner, Owner)> {
        anyhow::ensure!(
            Arc::ptr_eq(global.semaphore(), global_bank)
                && Arc::ptr_eq(publication.semaphore(), publication_bank)
                && global.num_permits() >= Self::pair_bytes()
                && publication.num_permits() >= Self::pair_bytes(),
            crate::output::resource_decision("tunnel metadata lease grant mismatch")
        );
        let publication_cross = global
            .split(Self::body_bytes())
            .expect("validated global lease body grants");
        let global_cross = publication
            .split(Self::body_bytes())
            .expect("validated publication lease body grants");
        // No fallible step or allocation separates validation from these paid
        // roots. Shared frees each allocation BEFORE its two fee fields drop.
        let publication = Owner::new(Self {
            value: publication,
            _cross_body: publication_cross,
        });
        let global = Owner::new(Self {
            value: global,
            _cross_body: global_cross,
        });
        Ok((publication, global))
    }
}

impl Deref for Lease {
    type Target = OwnedSemaphorePermit;
    fn deref(&self) -> &Self::Target {
        &self.value
    }
}

#[cfg(test)]
mod tests;
