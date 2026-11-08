//! Borrow the original candidate identities across reconciliation waits.
//! No cloned host buffer or unmeasured hash table is retained here.
use std::sync::Arc;

use anyhow::Result;
use tokio::sync::{OwnedSemaphorePermit, Semaphore};

use super::{lifecycle::TicketOwner, Route};

pub(super) struct Candidate {
    pub(super) input: super::candidate_input::Owner,
    pub(super) ticket: TicketOwner,
    pub(super) native_id: Option<u32>,
    pub(super) node: super::failure_table::OwnedNode,
    pub(super) control: super::marker_identity::Cutover<3>,
}

#[cfg(test)]
impl Candidate {
    pub(super) fn first_usb_bytes(udid: &str) -> usize {
        2 * std::mem::size_of::<Self>() + super::candidate_input::Input::bytes(udid, &Route::Usbmux)
            - udid.len()
            + super::scan_seen::Seen::first_bytes()
            + super::failure_table::Node::input_bytes(&Route::Usbmux)
            + TicketOwner::body_bytes()
            + 3 * super::ControlIdentity::body_bytes()
    }
}

pub(super) struct Index<'a> {
    candidates: &'a [Candidate],
    positions: Vec<usize>,
    _storage: OwnedSemaphorePermit,
}

impl<'a> Index<'a> {
    pub(super) fn new(candidates: &'a [Candidate], bank: &Arc<Semaphore>) -> Result<Self> {
        let allocation = super::metadata_vec::exact_backing(bank, candidates.len())?;
        let storage = allocation.storage;
        let mut positions = allocation.values;
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
