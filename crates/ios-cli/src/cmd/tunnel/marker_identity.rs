//! Actual private control identity and its immutable producer-bank provenance.
//!
//! Every physical marker is prepared before its control activation. Its actual
//! allocation and authentic origin fees survive every clone and last Weak free.
use std::sync::Arc;

use super::birth_bank::BirthBank;
use super::shared_weak_allocation::{Prepared as AllocationDestination, Strong};
use anyhow::Result;
use tokio::sync::{OwnedSemaphorePermit, Semaphore};

type MarkerRoot = Strong<BirthBank, Option<OwnedSemaphorePermit>>;

/// Keeps only the physical marker block/fees, never its original strong payload.
pub(super) struct PhysicalKeepAlive {
    _root: super::shared_weak_allocation::Weak<BirthBank, Option<OwnedSemaphorePermit>>,
}

pub(super) trait ReleaseMarkers {
    fn keep(&mut self, identity: &Identity);
}

pub(super) trait BirthContext: ReleaseMarkers {
    fn place(&mut self, origin: &BirthBank) -> Identity;
}

/// Fixed actual operation-frame storage. Its owner must outlive the watch,
/// authority and destination guards, including when its borrowed work is dropped.
pub(super) struct ReleaseFence<const N: usize> {
    values: [Option<PhysicalKeepAlive>; N],
}

impl<const N: usize> ReleaseFence<N> {
    pub(super) fn new() -> Self {
        Self {
            values: [const { None }; N],
        }
    }
}

impl<const N: usize> ReleaseMarkers for ReleaseFence<N> {
    fn keep(&mut self, identity: &Identity) {
        *self
            .values
            .iter_mut()
            .find(|value| value.is_none())
            .expect("complete marker release field storage") = Some(identity.keep_physical());
    }
}

impl ReleaseMarkers for super::metadata_vec::KnownVec<PhysicalKeepAlive> {
    fn keep(&mut self, identity: &Identity) {
        self.push_reserved(identity.keep_physical());
    }
}

/// Exact operation storage for the CURRENT capacity-pruning set. Admission
/// prepares it before any Gate is dropped; a closed-phase caller can supply
/// the same real backing in advance. No fixed large array lives in Ticket's F.
pub(super) struct ReleaseBatch {
    values: Option<super::metadata_vec::AdmittedVec<PhysicalKeepAlive>>,
    pending: Option<OwnedSemaphorePermit>,
    bank: Arc<Semaphore>,
}

impl ReleaseBatch {
    pub(super) fn new(bank: &Arc<Semaphore>) -> Self {
        Self {
            values: None,
            pending: None,
            bank: bank.clone(),
        }
    }

    #[cfg(test)]
    pub(super) fn backing_bytes(count: usize) -> usize {
        count * std::mem::size_of::<PhysicalKeepAlive>()
    }

    pub(super) fn prepare(&mut self, count: usize) -> Result<()> {
        if let Some(values) = &self.values {
            assert!(
                values.values.is_empty() && values.values.capacity() >= count,
                "complete prepaid current pruning fields"
            );
            return Ok(());
        }
        assert!(
            self.pending.is_none(),
            "one actual release backing preparation"
        );
        let layout = std::alloc::Layout::array::<PhysicalKeepAlive>(count)
            .map_err(|_| crate::output::resource_decision("tunnel owned vector layout overflow"))?;
        self.pending = Some(super::admission::known_metadata_charge(
            &self.bank,
            layout.size(),
        )?);
        self.values = Some(super::metadata_vec::from_borrowed_grant(
            &self.bank,
            count,
            &mut self.pending,
        )?);
        Ok(())
    }
}

impl ReleaseMarkers for ReleaseBatch {
    fn keep(&mut self, identity: &Identity) {
        let values = &mut self
            .values
            .as_mut()
            .expect("prepared release backing")
            .values;
        assert!(
            values.len() < values.capacity(),
            "whole current release field stock"
        );
        values.push(identity.keep_physical());
    }
}

// Cell reset can retire epoch/version/pending/failure (four original fields),
// in addition to the new marker. This is operation storage, not an arena bound.
const RELEASE_FIELDS_PER_BIRTH: usize = 5;

/// Actual complete empty marker storage, never a logical reservation token.
pub(super) struct PreparedBirth {
    destination: AllocationDestination<BirthBank, Option<OwnedSemaphorePermit>>,
    origin: BirthBank,
    extra: Option<OwnedSemaphorePermit>,
}

impl PreparedBirth {
    fn from_pending_grants(
        origin: &BirthBank,
        bank: &Arc<Semaphore>,
        first: &mut Option<OwnedSemaphorePermit>,
        extra: &mut Option<OwnedSemaphorePermit>,
    ) -> Result<Self> {
        let foreign = !Arc::ptr_eq(origin.raw(), bank);
        anyhow::ensure!(
            extra.as_ref().map_or(!foreign, |grant| {
                foreign
                    && Arc::ptr_eq(grant.semaphore(), origin.raw())
                    && grant.num_permits() == Identity::body_bytes()
            }),
            crate::output::resource_decision("tunnel marker authentic origin grant mismatch")
        );
        let destination = AllocationDestination::from_borrowed_grant(bank, first)?;
        Ok(Self {
            destination,
            origin: origin.clone(),
            extra: extra.take(),
        })
    }

    fn from_grants(
        origin: &BirthBank,
        bank: &Arc<Semaphore>,
        first: OwnedSemaphorePermit,
        extra: Option<OwnedSemaphorePermit>,
    ) -> Result<Self> {
        let foreign = !Arc::ptr_eq(origin.raw(), bank);
        anyhow::ensure!(
            extra.as_ref().map_or(!foreign, |grant| {
                foreign
                    && Arc::ptr_eq(grant.semaphore(), origin.raw())
                    && grant.num_permits() == Identity::body_bytes()
            }),
            crate::output::resource_decision("tunnel marker authentic origin grant mismatch")
        );
        let destination = AllocationDestination::from_grant(bank, first)?;
        Ok(Self {
            destination,
            origin: origin.clone(),
            extra,
        })
    }

    pub(super) fn place(self, origin: &BirthBank) -> Identity {
        assert!(
            Arc::ptr_eq(origin.raw(), self.origin.raw()),
            "actual prepared marker producer"
        );
        let Self {
            destination,
            origin,
            extra,
        } = self;
        Identity {
            value: destination.place(origin, extra),
        }
    }
}

fn grants(
    origin: &BirthBank,
    bank: &Arc<Semaphore>,
    count: usize,
) -> Result<(OwnedSemaphorePermit, Option<OwnedSemaphorePermit>)> {
    let bytes = Identity::body_bytes().checked_mul(count).ok_or_else(|| {
        crate::output::resource_decision("tunnel marker destination size overflow")
    })?;
    let same = Arc::ptr_eq(origin.raw(), bank);
    let first = if same {
        origin.origin_charge(bytes)?
    } else {
        super::admission::known_metadata_charge(bank, bytes)?
    };
    let extra = if same {
        None
    } else {
        Some(origin.origin_charge(bytes)?)
    };
    Ok((first, extra))
}

fn split_birth(
    origin: &BirthBank,
    bank: &Arc<Semaphore>,
    first: &mut OwnedSemaphorePermit,
    extra: &mut Option<OwnedSemaphorePermit>,
) -> Result<PreparedBirth> {
    let body = first
        .split(Identity::body_bytes())
        .expect("whole marker destination grant");
    let mirror = extra.as_mut().map(|grant| {
        grant
            .split(Identity::body_bytes())
            .expect("whole marker origin destination grant")
    });
    PreparedBirth::from_grants(origin, bank, body, mirror)
}

/// One-shot inputs for the exact fresh births of one actual control activation.
/// This type has no Clone; nested activations cannot reuse the same storage.
pub(super) struct Cutover<const N: usize> {
    births: [Option<PreparedBirth>; N],
    retired: [[Option<PhysicalKeepAlive>; RELEASE_FIELDS_PER_BIRTH]; N],
    origin: BirthBank,
}

impl<const N: usize> Cutover<N> {
    pub(super) fn new(origin: &BirthBank, bank: &Arc<Semaphore>) -> Result<Self> {
        let (mut first, mut extra) = grants(origin, bank, N)?;
        let mut births = std::array::from_fn(|_| None);
        for birth in &mut births {
            *birth = Some(split_birth(origin, bank, &mut first, &mut extra)?);
        }
        Ok(Self {
            births,
            retired: std::array::from_fn(|_| std::array::from_fn(|_| None)),
            origin: origin.clone(),
        })
    }

    pub(super) fn place(&mut self, origin: &BirthBank) -> Identity {
        let identity = self.take(origin).place(origin);
        self.keep(&identity);
        identity
    }

    pub(super) fn take(&mut self, origin: &BirthBank) -> PreparedBirth {
        assert!(
            Arc::ptr_eq(origin.raw(), self.origin.raw()),
            "actual marker producer context"
        );
        self.births
            .iter_mut()
            .find_map(Option::take)
            .expect("one actual destination per fresh marker")
    }

    /// Split only an unused, fully prepaid group into independent one-shot
    /// activations. No root or physical keeper is shared or manufactured.
    pub(super) fn into_individual(self) -> [Cutover<1>; N] {
        let Self {
            births,
            retired,
            origin,
        } = self;
        assert!(retired.iter().flatten().all(Option::is_none));
        assert!(births.iter().all(Option::is_some));
        births.map(|birth| Cutover {
            births: [birth],
            retired: [std::array::from_fn(|_| None)],
            origin: origin.clone(),
        })
    }
}

impl<const N: usize> ReleaseMarkers for Cutover<N> {
    fn keep(&mut self, identity: &Identity) {
        *self
            .retired
            .iter_mut()
            .flatten()
            .find(|value| value.is_none())
            .expect("complete cutover marker release fields") = Some(identity.keep_physical());
    }
}

impl<const N: usize> BirthContext for Cutover<N> {
    fn place(&mut self, origin: &BirthBank) -> Identity {
        Cutover::place(self, origin)
    }
}

#[derive(Default)]
struct PendingGrants {
    whole_first: Option<OwnedSemaphorePermit>,
    whole_extra: Option<OwnedSemaphorePermit>,
    current_first: Option<OwnedSemaphorePermit>,
    current_extra: Option<OwnedSemaphorePermit>,
}

impl PendingGrants {
    fn acquire(&mut self, origin: &BirthBank, bank: &Arc<Semaphore>, count: usize) -> Result<()> {
        assert!(self.whole_first.is_none(), "one actual marker preparation");
        let bytes = Identity::body_bytes().checked_mul(count).ok_or_else(|| {
            crate::output::resource_decision("tunnel marker destination size overflow")
        })?;
        let same = Arc::ptr_eq(origin.raw(), bank);
        // Store G before attempting authentic A. A refusal must not refund G
        // under the caller's device guard, even before any root exists.
        self.whole_first = Some(if same {
            origin.origin_charge(bytes)?
        } else {
            super::admission::known_metadata_charge(bank, bytes)?
        });
        if !same {
            self.whole_extra = Some(origin.origin_charge(bytes)?);
        }
        Ok(())
    }

    fn next(&mut self, origin: &BirthBank, bank: &Arc<Semaphore>) -> Result<PreparedBirth> {
        assert!(self.current_first.is_none() && self.current_extra.is_none());
        self.current_first = Some(
            self.whole_first
                .as_mut()
                .expect("whole pending marker grant")
                .split(Identity::body_bytes())
                .expect("whole next marker grant"),
        );
        self.current_extra = self.whole_extra.as_mut().map(|grant| {
            grant
                .split(Identity::body_bytes())
                .expect("whole next origin marker grant")
        });
        PreparedBirth::from_pending_grants(
            origin,
            bank,
            &mut self.current_first,
            &mut self.current_extra,
        )
    }
}

/// An outer-owned constructor transaction. Actual partial empty destinations
/// and all unsplit/current grants survive a failed preparation until its owner
/// leaves the caller's guards. This is not a source of reusable marker slots.
pub(super) struct PendingCutover<const N: usize> {
    births: [Option<PreparedBirth>; N],
    grants: PendingGrants,
    origin: BirthBank,
    bank: Arc<Semaphore>,
}

impl<const N: usize> PendingCutover<N> {
    pub(super) fn new(origin: &BirthBank, bank: &Arc<Semaphore>) -> Self {
        Self {
            births: [const { None }; N],
            grants: PendingGrants::default(),
            origin: origin.clone(),
            bank: bank.clone(),
        }
    }

    pub(super) fn prepare(&mut self) -> Result<Cutover<N>> {
        self.grants.acquire(&self.origin, &self.bank, N)?;
        for birth in &mut self.births {
            *birth = Some(self.grants.next(&self.origin, &self.bank)?);
        }
        Ok(Cutover {
            births: std::mem::replace(&mut self.births, [const { None }; N]),
            retired: std::array::from_fn(|_| std::array::from_fn(|_| None)),
            origin: self.origin.clone(),
        })
    }
}

struct BirthCell {
    destination: Option<PreparedBirth>,
    retired: [Option<PhysicalKeepAlive>; RELEASE_FIELDS_PER_BIRTH],
}

/// Exact CURRENT stale-gate input stock, fully prepared before its first reset.
pub(super) struct BirthBatch {
    births: Option<super::metadata_vec::AdmittedVec<BirthCell>>,
    next: usize,
    retired: usize,
    origin: BirthBank,
}

impl BirthBatch {
    #[cfg(test)]
    pub(super) fn backing_bytes(count: usize) -> usize {
        count * std::mem::size_of::<BirthCell>()
    }

    #[cfg(test)]
    pub(super) fn new(origin: &BirthBank, bank: &Arc<Semaphore>, count: usize) -> Result<Self> {
        PendingBatch::new(origin, bank).prepare(count)
    }

    pub(super) fn place(&mut self, origin: &BirthBank) -> Identity {
        let identity = self.take(origin).place(origin);
        self.keep(&identity);
        identity
    }

    pub(super) fn take(&mut self, origin: &BirthBank) -> PreparedBirth {
        assert!(
            Arc::ptr_eq(origin.raw(), self.origin.raw()),
            "actual marker producer batch"
        );
        let birth = self
            .births
            .as_mut()
            .expect("nonempty prepared birth batch")
            .values[self.next]
            .destination
            .take()
            .expect("one current marker destination per batch field");
        self.next += 1;
        birth
    }

    pub(super) fn remaining(&self) -> usize {
        self.births.as_ref().map_or(0, |births| births.values.len()) - self.next
    }
}

impl ReleaseMarkers for BirthBatch {
    fn keep(&mut self, identity: &Identity) {
        let index = self.retired;
        let slot = &mut self
            .births
            .as_mut()
            .expect("nonempty marker release batch")
            .values[index / RELEASE_FIELDS_PER_BIRTH]
            .retired[index % RELEASE_FIELDS_PER_BIRTH];
        assert!(slot.is_none(), "unique batch marker release field");
        *slot = Some(identity.keep_physical());
        self.retired += 1;
    }
}

impl BirthContext for BirthBatch {
    fn place(&mut self, origin: &BirthBank) -> Identity {
        BirthBatch::place(self, origin)
    }
}

pub(super) struct PendingBatch {
    // All actual partial root backings end before this row backing's grant.
    births: Option<super::metadata_vec::AdmittedVec<BirthCell>>,
    row_grant: Option<OwnedSemaphorePermit>,
    grants: PendingGrants,
    origin: BirthBank,
    bank: Arc<Semaphore>,
}

impl PendingBatch {
    pub(super) fn new(origin: &BirthBank, bank: &Arc<Semaphore>) -> Self {
        Self {
            births: None,
            row_grant: None,
            grants: PendingGrants::default(),
            origin: origin.clone(),
            bank: bank.clone(),
        }
    }

    pub(super) fn prepare(&mut self, count: usize) -> Result<BirthBatch> {
        if count != 0 {
            assert!(self.births.is_none() && self.row_grant.is_none());
            let layout = std::alloc::Layout::array::<BirthCell>(count).map_err(|_| {
                crate::output::resource_decision("tunnel owned vector layout overflow")
            })?;
            self.row_grant = Some(super::admission::known_metadata_charge(
                &self.bank,
                layout.size(),
            )?);
            self.births = Some(super::metadata_vec::from_borrowed_grant(
                &self.bank,
                count,
                &mut self.row_grant,
            )?);
            self.grants.acquire(&self.origin, &self.bank, count)?;
            let births = self.births.as_mut().expect("actual prepared row backing");
            for _ in 0..count {
                births.values.push(BirthCell {
                    destination: Some(self.grants.next(&self.origin, &self.bank)?),
                    retired: std::array::from_fn(|_| None),
                });
            }
        }
        Ok(BirthBatch {
            births: self.births.take(),
            next: 0,
            retired: 0,
            origin: self.origin.clone(),
        })
    }
}

/// Actual independent captured-identity allocation, without retaining a Ticket.
pub(super) struct Owner(
    super::shared_weak_allocation::Strong<Identity, Option<tokio::sync::OwnedSemaphorePermit>>,
);

impl Owner {
    pub(super) fn body_bytes() -> usize {
        super::shared_weak_allocation::Strong::<
            Identity,
            Option<tokio::sync::OwnedSemaphorePermit>,
        >::body_bytes()
    }

    pub(super) fn new(identity: &Identity, bank: &Arc<Semaphore>) -> anyhow::Result<Self> {
        let bytes = Self::body_bytes();
        let storage = super::admission::known_metadata_charge(bank, bytes)?;
        let origin = if Arc::ptr_eq(identity.bank(), bank) {
            None
        } else {
            Some(identity.origin_charge(bytes)?)
        };
        Ok(Self(super::shared_weak_allocation::Strong::new(
            identity.clone(),
            bank,
            storage,
            origin,
        )?))
    }
}

impl Clone for Owner {
    fn clone(&self) -> Self {
        Self(self.0.clone())
    }
}

impl std::ops::Deref for Owner {
    type Target = Identity;

    fn deref(&self) -> &Identity {
        &self.0
    }
}

impl std::fmt::Debug for Owner {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        std::fmt::Debug::fmt(&**self, formatter)
    }
}

#[derive(Clone)]
pub(super) struct Identity {
    value: MarkerRoot,
}

impl Identity {
    pub(super) fn body_bytes() -> usize {
        MarkerRoot::body_bytes()
    }

    pub(super) fn ptr_eq(&self, other: &Self) -> bool {
        self.value.ptr_eq(&other.value)
    }

    pub(super) fn bank(&self) -> &Arc<Semaphore> {
        self.value.raw()
    }

    pub(super) fn origin_charge(
        &self,
        bytes: usize,
    ) -> anyhow::Result<tokio::sync::OwnedSemaphorePermit> {
        self.value.origin_charge(bytes)
    }

    pub(super) fn keep_physical(&self) -> PhysicalKeepAlive {
        PhysicalKeepAlive {
            _root: self.value.downgrade(),
        }
    }

    #[cfg(test)]
    pub(super) fn strong_count(&self) -> usize {
        self.value.strong_count()
    }

    #[cfg(test)]
    pub(super) fn downgrade(&self) -> Weak {
        Weak {
            value: self.value.downgrade(),
        }
    }
}

impl std::fmt::Debug for Identity {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("()")
    }
}

#[cfg(test)]
#[derive(Clone)]
pub(super) struct Weak {
    value: super::shared_weak_allocation::Weak<BirthBank, Option<OwnedSemaphorePermit>>,
}

#[cfg(test)]
impl Weak {
    pub(super) fn upgrade(&self) -> Option<Identity> {
        self.value.upgrade().map(|value| Identity { value })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn producer_identity_preserves_actual_value_and_bank_through_clone_and_weak_upgrade() {
        let bank_birth = crate::cmd::tunnel::birth_bank::BirthBank::new(2 * Identity::body_bytes());
        let bank = bank_birth.raw().clone();
        let mut births = Cutover::<2>::new(&bank_birth, &bank).unwrap();
        bank.close();
        let foreign_birth = crate::cmd::tunnel::birth_bank::BirthBank::new(Identity::body_bytes());
        let foreign = foreign_birth.raw().clone();
        let mut foreign_births = Cutover::<1>::new(&foreign_birth, &foreign).unwrap();
        let original = births.place(&bank_birth);
        let cloned = original.clone();
        let other = foreign_births.place(&foreign_birth);
        let fresh = births.place(&bank_birth);
        assert!(original.ptr_eq(&cloned));
        assert!(!original.ptr_eq(&other) && !original.ptr_eq(&fresh));
        assert!(Arc::ptr_eq(original.bank(), &bank));
        assert!(Arc::ptr_eq(cloned.bank(), &bank));
        assert!(Arc::ptr_eq(other.bank(), &foreign));
        assert_eq!(original.strong_count(), 2);
        let weak = original.downgrade();
        let second = weak.clone();
        let upgraded = weak.upgrade().unwrap();
        assert!(upgraded.ptr_eq(&original));
        assert!(Arc::ptr_eq(upgraded.bank(), &bank));
        drop(upgraded);
        drop(original);
        assert!(second.upgrade().is_some());
        drop(cloned);
        assert!(weak.upgrade().is_none() && second.upgrade().is_none());
        assert_eq!(bank.available_permits(), 0);
        // The same original provenance/control assertions now hold for real
        // paid physical roots, including closed-zero placement and Weak-only life.
        drop((fresh, other, weak, second, births, foreign_births));
        assert_eq!(bank.available_permits(), 2 * Identity::body_bytes());
        assert_eq!(foreign.available_permits(), Identity::body_bytes());
    }
}

#[cfg(test)]
mod owner_tests;
#[cfg(test)]
mod physical_birth_tests;
