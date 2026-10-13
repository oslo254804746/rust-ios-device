//! Owned scan-start keys and publication identities in the shared metadata bank.
use std::sync::Arc;

use anyhow::Result;
use tokio::sync::{OwnedSemaphorePermit, Semaphore};

use super::{metadata_vec::KnownVec, MAX_FAILURE_RECORDS};

struct Row {
    udid: String,
    publication: Arc<()>,
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
        entries: impl ExactSizeIterator<Item = (&'a String, &'a Arc<()>)>,
    ) -> Result<Self> {
        let mut rows = KnownVec::new(Arc::clone(bank), MAX_FAILURE_RECORDS);
        rows.reserve(entries.len())?;
        for (udid, publication) in entries {
            let udid = udid.clone();
            let key_storage = super::admission::known_metadata_charge(bank, udid.capacity())?;
            rows.push_reserved(Row {
                udid,
                publication: Arc::clone(publication),
                _key_storage: key_storage,
            });
        }
        Ok(Self { rows })
    }

    pub(super) fn get(&self, udid: &str) -> Option<&Arc<()>> {
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
        let publication = Arc::new(());
        let entries = [(key, publication.clone())];
        let snapshot =
            InitialPublications::capture(&bank, entries.iter().map(|(key, value)| (key, value)))
                .unwrap();
        let row = &snapshot.rows.as_slice()[0];
        let actual = snapshot.rows.capacity() * std::mem::size_of::<Row>() + row.udid.capacity();
        assert_eq!(bank.available_permits(), 4096 - actual);
        assert!(Arc::ptr_eq(snapshot.get("fixture").unwrap(), &publication));
        assert_eq!(
            entries[0].0.capacity(),
            8192,
            "original table key was not replaced"
        );
        let shared = Arc::new(snapshot);
        let last = shared.clone();
        drop(shared);
        assert_eq!(bank.available_permits(), 4096 - actual);
        drop(last);
        assert_eq!(bank.available_permits(), 4096);
        assert_eq!(Arc::strong_count(&publication), 2);
    }

    #[test]
    fn initial_publication_one_byte_refusal_refunds_backing_and_preserves_original_identity() {
        let key = "fixture".to_owned();
        let publication = Arc::new(());
        let total = InitialPublications::first_bytes(&key) - 1;
        let bank = Arc::new(Semaphore::new(total));
        assert!(InitialPublications::capture(&bank, [(&key, &publication)].into_iter()).is_err());
        assert_eq!(bank.available_permits(), total);
        assert_eq!(key, "fixture");
        assert_eq!(Arc::strong_count(&publication), 1);
        let empty = InitialPublications::capture(&bank, std::iter::empty()).unwrap();
        assert!(empty.get("fixture").is_none());
        assert_eq!(bank.available_permits(), total);
    }
}
