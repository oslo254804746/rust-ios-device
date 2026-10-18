//! Owned scan-start keys and publication identities in the shared metadata bank.
use std::sync::Arc;

use anyhow::Result;
use tokio::sync::{OwnedSemaphorePermit, Semaphore};

use super::{metadata_vec::KnownVec, MAX_FAILURE_RECORDS};

struct Row {
    udid: String,
    publication: super::publication::LifeOwner,
    _key_storage: OwnedSemaphorePermit,
}

pub(super) struct InitialPublications {
    rows: KnownVec<Row>,
}

impl InitialPublications {
    /// Called before discovery. The iterator borrows the publication table only
    /// during this synchronous capture; no network operation holds its lock.
    pub(super) fn capture<'a>(
        bank: &Arc<Semaphore>,
        entries: impl ExactSizeIterator<Item = (&'a String, &'a super::publication::LifeOwner)>,
    ) -> Result<Self> {
        let mut rows = KnownVec::new(Arc::clone(bank), MAX_FAILURE_RECORDS);
        rows.reserve(entries.len())?;
        for (udid, publication) in entries {
            anyhow::ensure!(
                publication.belongs_to(bank),
                crate::output::resource_decision("tunnel publication snapshot bank mismatch")
            );
            let allocation = super::metadata_vec::exact_backing(bank, udid.len())?;
            let key_storage = allocation.storage;
            let mut bytes = allocation.values;
            bytes.extend_from_slice(udid.as_bytes());
            let udid = String::from_utf8(bytes).expect("copied publication key remains UTF-8");
            rows.push_reserved(Row {
                udid,
                publication: publication.clone(),
                _key_storage: key_storage,
            });
        }
        Ok(Self { rows })
    }

    pub(super) fn get(&self, udid: &str) -> Option<&super::publication::LifeOwner> {
        self.rows
            .as_slice()
            .iter()
            .find(|row| row.udid == udid)
            .map(|row| &row.publication)
    }

    #[cfg(test)]
    pub(super) fn first_bytes(udid: &str) -> usize {
        2 * std::mem::size_of::<Row>() + udid.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn initial_publication_snapshot_charges_actual_backing_and_copy_until_last_snapshot_owner() {
        let bank = Arc::new(Semaphore::new(4096));
        let mut key = String::with_capacity(8192);
        key.push_str("fixture");
        let published = Arc::new(Semaphore::new(4096));
        let publication = super::super::publication::Life::new(&bank, &published).unwrap();
        let before = bank.available_permits();
        let entries = [(key, publication.clone())];
        let allocated = Arc::new(std::sync::Mutex::new(Vec::new()));
        let captured = allocated.clone();
        let charged = bank.clone();
        let snapshot = super::super::metadata_vec::with_allocation_hook(
            move |layout, pointer| {
                let mut seen = captured.lock().unwrap();
                let prior: usize = seen.iter().map(|(size, _)| size).sum();
                assert_eq!(charged.available_permits(), before - prior - layout.size());
                seen.push((layout.size(), pointer));
            },
            || {
                InitialPublications::capture(&bank, entries.iter().map(|(key, value)| (key, value)))
                    .unwrap()
            },
        );
        let row = &snapshot.rows.as_slice()[0];
        assert_eq!(allocated.lock().unwrap().len(), 2);
        assert_eq!(
            allocated.lock().unwrap()[1],
            ("fixture".len(), row.udid.as_ptr() as usize)
        );
        let actual = snapshot.rows.capacity() * std::mem::size_of::<Row>() + row.udid.capacity();
        assert_eq!(bank.available_permits(), before - actual);
        assert!(snapshot.get("fixture").unwrap().ptr_eq(&publication));
        assert_eq!(
            entries[0].0.capacity(),
            8192,
            "original table key was not replaced"
        );
        let shared = Arc::new(snapshot);
        let last = shared.clone();
        drop(shared);
        assert_eq!(bank.available_permits(), before - actual);
        drop(last);
        assert_eq!(bank.available_permits(), before);
        assert_eq!(publication.strong_count(), 2);
        drop(entries);
        drop(publication);
        assert_eq!(bank.available_permits(), 4096);
        assert_eq!(published.available_permits(), 4096);
    }

    #[test]
    fn initial_publication_one_byte_refusal_refunds_backing_and_preserves_original_identity() {
        let key = "fixture".to_owned();
        let total = InitialPublications::first_bytes(&key) - 1;
        let bank = Arc::new(Semaphore::new(
            total + super::super::publication::Life::bytes(),
        ));
        let published = Arc::new(Semaphore::new(4096));
        let publication = super::super::publication::Life::new(&bank, &published).unwrap();
        assert!(InitialPublications::capture(&bank, [(&key, &publication)].into_iter()).is_err());
        assert_eq!(bank.available_permits(), total);
        assert_eq!(key, "fixture");
        assert_eq!(publication.strong_count(), 1);
        let empty = InitialPublications::capture(&bank, std::iter::empty()).unwrap();
        assert!(empty.get("fixture").is_none());
        assert_eq!(bank.available_permits(), total);
        drop(empty);
        drop(publication);
        assert_eq!(
            bank.available_permits(),
            total + super::super::publication::Life::bytes()
        );
        assert_eq!(published.available_permits(), 4096);
    }

    #[test]
    fn publication_lifetime_snapshot_foreign_bank_refuses_and_keeps_original_owner() {
        let global = Arc::new(Semaphore::new(4096));
        let published = Arc::new(Semaphore::new(4096));
        let life = super::super::publication::Life::new(&global, &published).unwrap();
        let before = global.available_permits();
        let foreign = Arc::new(Semaphore::new(4096));
        let key = String::from("fixture");
        let error = InitialPublications::capture(&foreign, [(&key, &life)].into_iter())
            .err()
            .unwrap();
        assert_eq!(
            crate::output::operation_error_class(error.as_ref()),
            ios_core::ErrorClass::ResourceLimit
        );
        assert_eq!(foreign.available_permits(), 4096);
        assert_eq!(global.available_permits(), before);
        assert!(life.check().is_ok());
        drop(life);
        assert_eq!(global.available_permits(), 4096);
        assert_eq!(published.available_permits(), 4096);
    }
}
