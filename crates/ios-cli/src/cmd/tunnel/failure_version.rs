//! Original retry/failure identity in a complete, independently paid root.
use super::shared_weak_allocation::Strong;
use anyhow::Result;
use std::sync::Arc;
use tokio::sync::{OwnedSemaphorePermit, Semaphore};

#[derive(Clone)]
pub(super) struct FailureVersion(Strong<(), ()>);
impl FailureVersion {
    pub(super) fn body_bytes() -> usize {
        Strong::<(), ()>::body_bytes()
    }
    pub(super) fn new(bank: &Arc<Semaphore>) -> Result<Self> {
        let storage = super::admission::known_metadata_charge(bank, Self::body_bytes())?;
        Self::from_grant(bank, storage)
    }
    fn from_grant(bank: &Arc<Semaphore>, storage: OwnedSemaphorePermit) -> Result<Self> {
        Ok(Self(Strong::new((), bank, storage, ())?))
    }
    pub(super) fn ptr_eq(&self, other: &Self) -> bool {
        self.0.ptr_eq(&other.0)
    }
    pub(super) fn is_from(&self, bank: &Arc<Semaphore>) -> bool {
        Arc::ptr_eq(self.0.first().semaphore(), bank)
    }
    #[cfg(test)]
    pub(super) fn downgrade(&self) -> super::shared_weak_allocation::Weak<(), ()> {
        self.0.downgrade()
    }
    #[cfg(test)]
    pub(super) fn as_ptr(&self) -> *const () {
        &*self.0
    }
}
impl std::fmt::Debug for FailureVersion {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("TunnelFailureVersion")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cmd::tunnel::{
        admission::known_metadata_charge, metadata_vec::with_allocation_hook,
    };
    use std::sync::{
        atomic::{AtomicUsize, Ordering},
        Barrier, Mutex,
    };

    #[test]
    fn failure_version_requires_exact_whole_original_bank_grant_before_any_backing() {
        let bytes = FailureVersion::body_bytes();
        let short = Arc::new(Semaphore::new(bytes - 1));
        let error = with_allocation_hook(
            |_, _| panic!("N-1 failure identity allocated a backing"),
            || FailureVersion::new(&short).unwrap_err(),
        );
        assert_eq!(
            crate::output::operation_error_class(error.as_ref()),
            ios_core::ErrorClass::ResourceLimit
        );
        assert_eq!(short.available_permits(), bytes - 1);
        for (foreign, amount) in [(true, bytes), (false, bytes - 1), (false, bytes + 1)] {
            let bank = Arc::new(Semaphore::new(2 * bytes));
            let other = Arc::new(Semaphore::new(2 * bytes));
            let grant =
                known_metadata_charge(if foreign { &other } else { &bank }, amount).unwrap();
            let error = with_allocation_hook(
                |_, _| panic!("foreign/short/long failure identity grant allocated"),
                || FailureVersion::from_grant(&bank, grant).unwrap_err(),
            );
            assert_eq!(
                crate::output::operation_error_class(error.as_ref()),
                ios_core::ErrorClass::ResourceLimit
            );
            assert_eq!(bank.available_permits(), 2 * bytes);
            assert_eq!(other.available_permits(), 2 * bytes);
        }
    }

    #[test]
    fn failure_version_closed_zero_pool_keeps_original_root_and_fee_through_last_weak_free() {
        let bytes = FailureVersion::body_bytes();
        let bank = Arc::new(Semaphore::new(bytes));
        let allocations = Arc::new(Mutex::new(Vec::new()));
        let recorded = allocations.clone();
        let credit = bank.clone();
        let version = with_allocation_hook(
            move |layout, pointer| {
                assert_eq!(credit.available_permits(), 0);
                recorded.lock().unwrap().push((layout.size(), pointer));
            },
            || FailureVersion::new(&bank).unwrap(),
        );
        let recorded = allocations.lock().unwrap();
        assert_eq!(recorded.len(), 1);
        assert_eq!(recorded[0].0, bytes);
        // The unit is zero-sized and may be the allocation's one-past field.
        let value = version.as_ptr() as usize;
        assert!(recorded[0].1 <= value && value <= recorded[0].1 + bytes);
        drop(recorded);
        let other_bank = Arc::new(Semaphore::new(bytes));
        let other = FailureVersion::new(&other_bank).unwrap();
        assert!(!version.ptr_eq(&other));
        assert!(version.is_from(&bank) && !version.is_from(&other_bank));
        bank.close();
        let (kept, weak) = with_allocation_hook(
            |_, _| panic!("closed failure identity clone allocated a backing"),
            || (version.clone(), version.downgrade()),
        );
        assert!(version.ptr_eq(&kept));
        assert_eq!(version.as_ptr(), kept.as_ptr());
        let restored = weak.upgrade().unwrap();
        assert!(restored.ptr_eq(&kept.0));
        drop((version, kept, restored));
        assert!(weak.upgrade().is_none());
        assert_eq!(bank.available_permits(), 0);
        let last = weak.clone();
        drop(weak);
        assert_eq!(bank.available_permits(), 0);
        let frees = Arc::new(AtomicUsize::new(0));
        let observed = frees.clone();
        let credit = bank.clone();
        super::super::shared_weak_allocation::after_free(
            move || {
                assert_eq!(
                    credit.available_permits(),
                    0,
                    "identity fee preceded actual root free"
                );
                observed.fetch_add(1, Ordering::SeqCst);
            },
            || drop(last),
        );
        assert_eq!(frees.load(Ordering::SeqCst), 1);
        assert_eq!(bank.available_permits(), bytes);
        assert_eq!(other_bank.available_permits(), 0);
        drop(other);
        assert_eq!(other_bank.available_permits(), bytes);
    }

    #[test]
    fn failure_version_parallel_original_clones_refund_once_after_the_last_owner() {
        let bytes = FailureVersion::body_bytes();
        let bank = Arc::new(Semaphore::new(bytes));
        let version = FailureVersion::new(&bank).unwrap();
        let pointer = version.as_ptr() as usize;
        let weak = version.downgrade();
        bank.close();
        let barrier = Arc::new(Barrier::new(9));
        let threads: Vec<_> = (0..8)
            .map(|_| {
                let version = version.clone();
                let bank = bank.clone();
                let barrier = barrier.clone();
                std::thread::spawn(move || {
                    assert_eq!(version.as_ptr() as usize, pointer);
                    assert!(version.is_from(&bank));
                    barrier.wait();
                    drop(version);
                })
            })
            .collect();
        drop(version);
        assert_eq!(bank.available_permits(), 0);
        barrier.wait();
        for thread in threads {
            thread.join().unwrap();
        }
        assert!(weak.upgrade().is_none());
        assert_eq!(bank.available_permits(), 0);
        drop(weak);
        assert_eq!(bank.available_permits(), bytes);
    }
}
