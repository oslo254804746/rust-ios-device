//! Original native stamp keepers pay each actual foreign producer bank.
use std::sync::Arc;

use anyhow::Result;
use tokio::sync::{OwnedSemaphorePermit, Semaphore};

use super::{
    admission::known_metadata_charge,
    marker_identity::Identity,
    metadata_vec::{exact_backing, AdmittedVec},
};

// OwnershipStamp contains exactly two identities. This is its field count,
// not an arena, historical-origin, native-owner, or producer capacity limit.
pub(super) type Grants = [Option<OwnedSemaphorePermit>; 2];

pub(super) fn proof_grants(
    bank: &Arc<Semaphore>,
    bytes: usize,
    identities: [&Identity; 2],
) -> Result<Grants> {
    let mut grants: Grants = [None, None];
    for (index, identity) in identities.into_iter().enumerate() {
        let origin = identity.bank();
        if Arc::ptr_eq(origin, bank)
            || grants
                .iter()
                .flatten()
                .any(|grant| Arc::ptr_eq(grant.semaphore(), origin))
        {
            continue;
        }
        grants[index] = Some(known_metadata_charge(origin, bytes)?);
    }
    Ok(grants)
}

#[derive(Default)]
pub(super) struct Catalogue {
    grants: Vec<OwnedSemaphorePermit>,
    storage: Option<OwnedSemaphorePermit>,
}

pub(super) struct Prepared {
    grants: Grants,
    backing: Option<AdmittedVec<OwnedSemaphorePermit>>,
}

impl Catalogue {
    pub(super) fn prepare(
        &self,
        bank: &Arc<Semaphore>,
        backing_bytes: usize,
        identities: [&Identity; 2],
    ) -> Result<Prepared> {
        let mut grants: Grants = [None, None];
        for (index, identity) in identities.into_iter().enumerate() {
            let origin = identity.bank();
            if Arc::ptr_eq(origin, bank)
                || self
                    .grants
                    .iter()
                    .chain(grants.iter().flatten())
                    .any(|grant| Arc::ptr_eq(grant.semaphore(), origin))
            {
                continue;
            }
            grants[index] = Some(known_metadata_charge(origin, backing_bytes)?);
        }
        let additional = grants.iter().flatten().count();
        let required = self.grants.len().checked_add(additional).ok_or_else(|| {
            crate::output::resource_decision("native producer catalogue length overflow")
        })?;
        let backing = if required > self.grants.capacity() {
            let capacity = self
                .grants
                .capacity()
                .max(1)
                .saturating_mul(2)
                .max(required);
            Some(exact_backing(bank, capacity)?)
        } else {
            None
        };
        Ok(Prepared { grants, backing })
    }

    pub(super) fn commit(&mut self, prepared: Prepared) {
        if let Some(mut backing) = prepared.backing {
            backing.values.append(&mut self.grants);
            // Original A grants move without refund. Free the old actual B Vec
            // before returning its B fee; the new backing is already admitted.
            self.grants = backing.values;
            self.storage = Some(backing.storage);
        }
        for grant in prepared.grants.into_iter().flatten() {
            assert!(self.grants.len() < self.grants.capacity());
            self.grants.push(grant);
        }
    }

    #[cfg(test)]
    pub(super) fn is_empty(&self) -> bool {
        self.grants.is_empty()
    }

    #[cfg(test)]
    pub(super) fn backing_bytes(&self) -> usize {
        self.storage.as_ref().map_or(0, |grant| grant.num_permits())
    }
}
