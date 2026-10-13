//! Complete paired views index original UID/host buffers before any new ticket.
use std::sync::Arc;

use anyhow::Result;
use ios_core::device::PairedMobdev2Device;
use tokio::sync::{OwnedSemaphorePermit, Semaphore};

pub(super) struct Index {
    positions: Vec<usize>,
    _storage: OwnedSemaphorePermit,
}

impl Index {
    pub(super) fn new(targets: &[PairedMobdev2Device], bank: &Arc<Semaphore>) -> Result<Self> {
        let mut positions = Vec::new();
        positions.try_reserve_exact(targets.len()).map_err(|_| {
            crate::output::resource_decision("tunnel paired observed index allocation failed")
        })?;
        let storage = super::admission::known_vec_backing(bank, &positions)?;
        positions.extend(0..targets.len());
        positions.sort_unstable_by(|a, b| {
            let a = &targets[*a];
            let b = &targets[*b];
            a.udid.cmp(&b.udid).then(a.host.cmp(&b.host))
        });
        Ok(Self {
            positions,
            _storage: storage,
        })
    }

    pub(super) fn contains(&self, targets: &[PairedMobdev2Device], udid: &str, host: &str) -> bool {
        self.positions
            .binary_search_by(|index| {
                let target = &targets[*index];
                target
                    .udid
                    .as_str()
                    .cmp(udid)
                    .then(target.host.as_str().cmp(host))
            })
            .is_ok()
    }
}

#[cfg(test)]
mod tests;
