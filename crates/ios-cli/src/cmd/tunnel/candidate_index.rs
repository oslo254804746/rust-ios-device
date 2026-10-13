//! Borrow the original candidate identities across reconciliation waits.
//! No cloned host buffer or unmeasured hash table is retained here.
use std::sync::Arc;

use anyhow::Result;
use tokio::sync::{OwnedSemaphorePermit, Semaphore};

use super::{Route, Ticket};

pub(super) struct Candidate {
    pub(super) input: Arc<super::candidate_input::Input>,
    pub(super) ticket: Ticket,
    pub(super) native_id: Option<u32>,
}

#[cfg(test)]
impl Candidate {
    pub(super) fn first_usb_bytes(udid: &str) -> usize {
        2 * std::mem::size_of::<Self>() + super::candidate_input::Input::bytes(udid, &Route::Usbmux)
            - udid.len()
            + super::scan_seen::Seen::first_bytes()
    }
}

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
        positions.sort_unstable_by(|a, b| {
            candidates[*a]
                .input
                .udid
                .cmp(&candidates[*b].input.udid)
                .then(a.cmp(b))
        });
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
            .partition_point(|index| self.candidates[*index].input.udid.as_str() <= udid);
        let index = *self.positions.get(end.checked_sub(1)?)?;
        let candidate = &self.candidates[index];
        (candidate.input.udid == udid).then_some((&candidate.input.route, candidate.native_id))
    }
}

#[cfg(test)]
mod tests;
