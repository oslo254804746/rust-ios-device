//! Borrow the original candidate identities across reconciliation waits.
//! No cloned host buffer or unmeasured hash table is retained here.
use std::sync::Arc;

use anyhow::Result;
use tokio::sync::{OwnedSemaphorePermit, Semaphore};

use super::{Route, Ticket};

pub(super) type Candidate = (String, Route, Ticket, Option<u32>);

pub(super) struct Index<'a> {
    candidates: &'a [Candidate],
    positions: Vec<usize>,
    _storage: OwnedSemaphorePermit,
}

impl<'a> Index<'a> {
    pub(super) fn new(candidates: &'a [Candidate], bank: &Arc<Semaphore>) -> Result<Self> {
        let mut positions = Vec::new();
        positions.try_reserve_exact(candidates.len()).map_err(|_| {
            crate::output::resource_decision("tunnel candidate index allocation failed")
        })?;
        let storage = super::admission::known_vec_backing(bank, &positions)?;
        positions.extend(0..candidates.len());
        positions.sort_unstable_by(|a, b| candidates[*a].0.cmp(&candidates[*b].0).then(a.cmp(b)));
        Ok(Self {
            candidates,
            positions,
            _storage: storage,
        })
    }

    pub(super) fn get(&self, udid: &str) -> Option<(&Route, Option<u32>)> {
        // Preserve the old map's last-entry rule even for a duplicate private
        // input; production's seen set normally makes all these keys unique.
        let end = self
            .positions
            .partition_point(|index| self.candidates[*index].0.as_str() <= udid);
        let index = *self.positions.get(end.checked_sub(1)?)?;
        let (key, route, _, native_id) = &self.candidates[index];
        (key == udid).then_some((route, *native_id))
    }
}

#[cfg(test)]
mod tests;
