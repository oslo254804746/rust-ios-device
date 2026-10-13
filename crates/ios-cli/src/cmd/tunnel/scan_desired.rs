//! Scan indexes borrow admitted desired keys/routes and own actual Vec backing.
use std::sync::Arc;

use anyhow::Result;
use tokio::sync::Semaphore;

use super::{intent, lifecycle::DesiredRow, metadata_vec::KnownVec, Ticket, MAX_FAILURE_RECORDS};

type ManualValue<'a> = (&'a Ticket, &'a Arc<intent::RetainedRoute>);
type ManualEntry<'a> = (&'a str, ManualValue<'a>);

pub(super) struct Manual<'a> {
    rows: KnownVec<&'a DesiredRow>,
    positions: KnownVec<usize>,
}

impl<'a> Manual<'a> {
    pub(super) fn new(bank: &Arc<Semaphore>) -> Self {
        Self {
            rows: KnownVec::new(Arc::clone(bank), MAX_FAILURE_RECORDS),
            positions: KnownVec::new(Arc::clone(bank), MAX_FAILURE_RECORDS),
        }
    }

    pub(super) fn insert(&mut self, row: &'a DesiredRow) -> Result<()> {
        self.rows.reserve(1)?;
        self.positions.reserve(1)?;
        self.positions.push_reserved(self.rows.len());
        self.rows.push_reserved(row);
        Ok(())
    }

    pub(super) fn sort(&mut self) {
        let rows = self.rows.as_slice();
        self.positions
            .as_mut_slice()
            .sort_unstable_by(|a, b| rows[*a].udid.cmp(&rows[*b].udid));
    }

    pub(super) fn is_empty(&self) -> bool {
        self.rows.len() == 0
    }

    pub(super) fn get(&self, udid: &str) -> Option<ManualValue<'a>> {
        let rows = self.rows.as_slice();
        let positions = self.positions.as_slice();
        let found = positions
            .binary_search_by(|index| rows[*index].udid.as_str().cmp(udid))
            .ok()?;
        let row = rows[positions[found]];
        let intent::Desired::Manual { route, .. } = &row.snapshot.desired else {
            unreachable!("manual scan row")
        };
        Some((&row.ticket, route))
    }

    pub(super) fn contains_key(&self, udid: &str) -> bool {
        self.get(udid).is_some()
    }

    pub(super) fn iter(&self) -> impl Iterator<Item = ManualEntry<'a>> + '_ {
        self.rows.as_slice().iter().map(|row| {
            let intent::Desired::Manual { route, .. } = &row.snapshot.desired else {
                unreachable!("manual scan row")
            };
            (row.udid.as_str(), (&row.ticket, route))
        })
    }

    #[cfg(test)]
    pub(super) fn storage_bytes(&self) -> usize {
        self.rows.capacity() * std::mem::size_of::<&DesiredRow>()
            + self.positions.capacity() * std::mem::size_of::<usize>()
    }
}

pub(super) struct Suppressed<'a> {
    keys: KnownVec<&'a str>,
}
impl<'a> Suppressed<'a> {
    pub(super) fn new(bank: &Arc<Semaphore>) -> Self {
        Self {
            keys: KnownVec::new(Arc::clone(bank), MAX_FAILURE_RECORDS),
        }
    }
    pub(super) fn insert(&mut self, key: &'a str) -> Result<()> {
        self.keys.push(key)
    }
    pub(super) fn sort(&mut self) {
        self.keys.as_mut_slice().sort_unstable();
    }
    pub(super) fn contains(&self, key: &str) -> bool {
        self.keys.as_slice().binary_search(&key).is_ok()
    }
}
