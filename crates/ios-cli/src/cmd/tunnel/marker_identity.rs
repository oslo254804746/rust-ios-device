//! Actual private control identity and its immutable producer-bank provenance.
//!
//! The raw Arc control allocation still needs the separate physical arena work.
//! This type prevents a foreign keeper from relabelling an existing identity.
use std::sync::Arc;

use super::birth_bank::BirthBank;
use tokio::sync::Semaphore;

#[derive(Clone)]
pub(super) struct Identity {
    value: Arc<()>,
    bank: BirthBank,
}

impl Identity {
    pub(super) fn new(bank: &BirthBank) -> Self {
        Self {
            value: Arc::new(()),
            bank: bank.clone(),
        }
    }

    pub(super) fn ptr_eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.value, &other.value)
    }

    pub(super) fn bank(&self) -> &Arc<Semaphore> {
        self.bank.raw()
    }

    pub(super) fn origin_charge(
        &self,
        bytes: usize,
    ) -> anyhow::Result<tokio::sync::OwnedSemaphorePermit> {
        self.bank.origin_charge(bytes)
    }

    #[cfg(test)]
    pub(super) fn strong_count(&self) -> usize {
        Arc::strong_count(&self.value)
    }

    #[cfg(test)]
    pub(super) fn downgrade(&self) -> Weak {
        Weak {
            value: Arc::downgrade(&self.value),
            bank: self.bank.clone(),
        }
    }
}

impl std::fmt::Debug for Identity {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        std::fmt::Debug::fmt(&self.value, formatter)
    }
}

#[cfg(test)]
#[derive(Clone)]
pub(super) struct Weak {
    value: std::sync::Weak<()>,
    bank: BirthBank,
}

#[cfg(test)]
impl Weak {
    pub(super) fn upgrade(&self) -> Option<Identity> {
        self.value.upgrade().map(|value| Identity {
            value,
            bank: self.bank.clone(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn producer_identity_preserves_actual_value_and_bank_through_clone_and_weak_upgrade() {
        let bank_birth = crate::cmd::tunnel::birth_bank::BirthBank::new(0);
        let bank = bank_birth.raw().clone();
        bank.close();
        let foreign_birth = crate::cmd::tunnel::birth_bank::BirthBank::new(0);
        let foreign = foreign_birth.raw().clone();
        let original = Identity::new(&bank_birth);
        let cloned = original.clone();
        let other = Identity::new(&foreign_birth);
        let fresh = Identity::new(&bank_birth);
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
        // This observes provenance/control semantics, not paid Arc allocation.
    }
}
