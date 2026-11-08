//! Independent full metadata fee roots, prepaid before Source can be polled.
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
        Owner::body_bytes()
    }

    pub(super) fn pair_bytes() -> usize {
        2 * Self::body_bytes()
    }
}

// Each empty destination deallocates BEFORE either of its original body fees
// can return credit. The two destinations remain independently owned on finish.
struct Destination {
    backing: super::metadata_vec::AdmittedVec<super::shared_value::Allocation<Lease>>,
    cross_body: OwnedSemaphorePermit,
}

pub(super) struct PreparedPair {
    publication: Destination,
    global: Destination,
}

impl PreparedPair {
    pub(super) fn new(
        global_bank: &Arc<Semaphore>,
        publication_bank: &Arc<Semaphore>,
    ) -> Result<Self> {
        let global = super::admission::known_metadata_charge(global_bank, Lease::pair_bytes())?;
        let publication =
            super::admission::known_metadata_charge(publication_bank, Lease::pair_bytes())?;
        Self::from_grants(global_bank, publication_bank, global, publication)
    }

    fn from_grants(
        global_bank: &Arc<Semaphore>,
        publication_bank: &Arc<Semaphore>,
        mut global: OwnedSemaphorePermit,
        mut publication: OwnedSemaphorePermit,
    ) -> Result<Self> {
        // Validate BOTH exact whole original-bank body grants before any alloc.
        anyhow::ensure!(
            Arc::ptr_eq(global.semaphore(), global_bank)
                && Arc::ptr_eq(publication.semaphore(), publication_bank)
                && global.num_permits() == Lease::pair_bytes()
                && publication.num_permits() == Lease::pair_bytes(),
            crate::output::resource_decision("tunnel metadata lease grant mismatch")
        );
        let publication_cross = global
            .split(Lease::body_bytes())
            .expect("validated global lease body grants");
        let global_cross = publication
            .split(Lease::body_bytes())
            .expect("validated publication lease body grants");
        let publication = Destination {
            backing: super::metadata_vec::from_grant(publication_bank, 1, publication)?,
            cross_body: publication_cross,
        };
        let global = Destination {
            backing: super::metadata_vec::from_grant(global_bank, 1, global)?,
            cross_body: global_cross,
        };
        Ok(Self {
            publication,
            global,
        })
    }

    /// Variable field grants join the two original prepaid body fees. Nothing
    /// allocates here, even when both banks are closed or have zero free bytes.
    pub(super) fn finish(
        self,
        global_fields: OwnedSemaphorePermit,
        publication_fields: OwnedSemaphorePermit,
    ) -> Result<(Owner, Owner)> {
        let valid = |destination: &Destination, fields: &OwnedSemaphorePermit| {
            Arc::ptr_eq(fields.semaphore(), destination.backing.storage.semaphore())
                && fields
                    .num_permits()
                    .checked_add(Lease::body_bytes())
                    .is_some_and(|bytes| bytes <= Semaphore::MAX_PERMITS)
        };
        anyhow::ensure!(
            valid(&self.global, &global_fields) && valid(&self.publication, &publication_fields),
            crate::output::resource_decision("tunnel metadata lease field grant mismatch")
        );
        let publication = self.publication.finish(publication_fields);
        let global = self.global.finish(global_fields);
        Ok((publication, global))
    }
}

impl Destination {
    fn finish(self, mut fields: OwnedSemaphorePermit) -> Owner {
        let super::metadata_vec::AdmittedVec { values, storage } = self.backing;
        // Same original bank and checked sum: merging cannot reject/overflow.
        fields.merge(storage);
        Owner::place(
            Lease {
                value: fields,
                _cross_body: self.cross_body,
            },
            values,
        )
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
