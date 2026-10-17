//! Seen identities share admitted inputs, including coordinator-rejected rows.
use std::sync::Arc;

use anyhow::Result;
use tokio::sync::Semaphore;

#[cfg(test)]
use super::candidate_input::Input;
use super::{metadata_vec::KnownVec, MAX_FAILURE_RECORDS};

pub(super) struct Seen {
    rows: KnownVec<super::candidate_input::Owner>,
}

impl Seen {
    pub(super) fn new(bank: &Arc<Semaphore>) -> Self {
        Self {
            rows: KnownVec::new(Arc::clone(bank), MAX_FAILURE_RECORDS),
        }
    }

    fn position(&self, udid: &str) -> Result<usize, usize> {
        self.rows
            .as_slice()
            .binary_search_by(|input| input.udid.as_str().cmp(udid))
    }

    pub(super) fn contains(&self, udid: &str) -> bool {
        self.position(udid).is_ok()
    }

    pub(super) fn reserve(&mut self, additional: usize) -> Result<()> {
        self.rows.reserve(additional)
    }

    pub(super) fn insert(&mut self, input: &super::candidate_input::Owner) -> Result<bool> {
        if self.contains(&input.udid) {
            return Ok(false);
        }
        self.reserve(1)?;
        Ok(self.insert_reserved(input))
    }

    // Paired preparation reserved all slots before creating any new ticket.
    pub(super) fn insert_reserved(&mut self, input: &super::candidate_input::Owner) -> bool {
        let Err(position) = self.position(&input.udid) else {
            return false;
        };
        self.rows.insert_reserved(position, input.clone());
        true
    }

    #[cfg(test)]
    pub(super) fn first_bytes() -> usize {
        2 * std::mem::size_of::<super::candidate_input::Owner>()
    }
}

#[cfg(test)]
mod tests;
