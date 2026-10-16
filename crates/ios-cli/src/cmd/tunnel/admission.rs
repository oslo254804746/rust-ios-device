//! Admission of manager selectors, retained Source snapshots and scan candidates.
//! Source allocation before return and allocator/RSS are separate boundaries.
use std::sync::Arc;

use anyhow::Result;
use ios_core::device::PairedMobdev2Device;

use super::source::UsbDevice;
use tokio::sync::{OwnedSemaphorePermit, Semaphore};

const MAX_RAW_ROWS: usize = 16_384;
const MAX_FIELD_BYTES: usize = 4096;
const MAX_RAW_TEXT_BYTES: usize = 16 * 1024 * 1024;
const MAX_RAW_STORAGE_BYTES: usize = 32 * 1024 * 1024;
const MAX_CANDIDATES: usize = 4096;
const MAX_CANDIDATE_TEXT_BYTES: usize = 16 * 1024 * 1024;

pub(super) fn selectors(udid: &str, route_host: Option<&str>) -> Result<()> {
    anyhow::ensure!(
        udid.len() <= MAX_FIELD_BYTES
            && route_host.map_or(true, |host| host.len() <= MAX_FIELD_BYTES),
        crate::output::resource_decision("tunnel selector metadata limit")
    );
    Ok(())
}

fn snapshot_limit() -> anyhow::Error {
    crate::output::resource_decision("tunnel discovery snapshot metadata limit")
}

fn charge_field(total: &mut usize, field: &String) -> Result<()> {
    anyhow::ensure!(field.len() <= MAX_FIELD_BYTES, snapshot_limit());
    *total = total
        .checked_add(field.capacity())
        .filter(|bytes| *bytes <= MAX_RAW_TEXT_BYTES)
        .ok_or_else(snapshot_limit)?;
    Ok(())
}

fn snapshot_storage(capacity: usize, element_size: usize, text: usize) -> Result<()> {
    anyhow::ensure!(
        capacity
            .checked_mul(element_size)
            .and_then(|bytes| bytes.checked_add(text))
            .is_some_and(|bytes| bytes <= MAX_RAW_STORAGE_BYTES),
        snapshot_limit()
    );
    Ok(())
}

pub(super) fn known_metadata_charge(
    bank: &Arc<Semaphore>,
    bytes: usize,
) -> Result<OwnedSemaphorePermit> {
    let count = u32::try_from(bytes).map_err(|_| snapshot_limit())?;
    bank.clone().try_acquire_many_owned(count).map_err(|_| {
        crate::output::resource_decision("tunnel global known metadata byte limit exceeded")
    })
}

pub(super) fn retain_usb_snapshot(
    bank: &Arc<Semaphore>,
    devices: &Vec<UsbDevice>,
) -> Result<OwnedSemaphorePermit> {
    usb_snapshot_fields(devices)?;
    let text = devices
        .iter()
        .map(|device| device.udid.capacity())
        .sum::<usize>();
    // The per-snapshot validator already checked these same capacities; the
    // borrowed input cannot change before this synchronous global admission.
    let bytes = devices.capacity() * std::mem::size_of::<UsbDevice>() + text;
    let storage = known_metadata_charge(bank, bytes)?;
    usb_snapshot_identities(devices, bank)?;
    Ok(storage)
}

pub(super) fn retain_paired_snapshot(
    bank: &Arc<Semaphore>,
    devices: &Vec<PairedMobdev2Device>,
) -> Result<OwnedSemaphorePermit> {
    paired_snapshot(devices)?;
    let text = devices
        .iter()
        .map(|device| device.udid.capacity() + device.host.capacity())
        .sum::<usize>();
    let bytes = devices.capacity() * std::mem::size_of::<PairedMobdev2Device>() + text;
    known_metadata_charge(bank, bytes)
}

fn usb_snapshot_fields(devices: &Vec<UsbDevice>) -> Result<()> {
    anyhow::ensure!(devices.len() <= MAX_RAW_ROWS, snapshot_limit());
    let mut bytes = 0;
    for device in devices {
        charge_field(&mut bytes, &device.udid)?;
    }
    snapshot_storage(devices.capacity(), std::mem::size_of::<UsbDevice>(), bytes)
}

fn usb_snapshot_identities(devices: &[UsbDevice], bank: &Arc<Semaphore>) -> Result<()> {
    let allocation = super::metadata_vec::exact_backing(bank, devices.len())?;
    let _storage = allocation.storage;
    let mut positions = allocation.values;
    positions.extend(0..devices.len());
    positions.sort_unstable_by(|a, b| devices[*a].udid.cmp(&devices[*b].udid));
    let mut last = None;
    let mut unique = 0;
    for index in positions {
        let identity = devices[index].udid.as_str();
        if last != Some(identity) {
            // Validate every raw identity, before selection or authorization.
            anyhow::ensure!(unique < MAX_CANDIDATES, snapshot_limit());
            unique += 1;
            last = Some(identity);
        }
    }
    Ok(())
}

// Logical validator tests have no production bank. All production consumers
// use retain_usb_snapshot, which admits raw storage and this temporary index.
#[cfg(test)]
fn usb_snapshot(devices: &Vec<UsbDevice>) -> Result<()> {
    usb_snapshot_fields(devices)?;
    usb_snapshot_identities(devices, &Arc::new(Semaphore::new(MAX_RAW_STORAGE_BYTES)))
}

pub(super) fn paired_snapshot(devices: &Vec<PairedMobdev2Device>) -> Result<()> {
    anyhow::ensure!(devices.len() <= MAX_RAW_ROWS, snapshot_limit());
    let mut bytes = 0;
    for device in devices {
        charge_field(&mut bytes, &device.udid)?;
        charge_field(&mut bytes, &device.host)?;
    }
    snapshot_storage(
        devices.capacity(),
        std::mem::size_of::<PairedMobdev2Device>(),
        bytes,
    )
}

#[cfg(test)]
mod paired_plan_tests;
#[cfg(test)]
mod tests;

#[derive(Clone, Copy, Default)]
pub(super) struct CandidateBudget {
    count: usize,
    bytes: usize,
}

pub(super) enum PairedCandidates {
    Complete {
        targets: Vec<PairedMobdev2Device>,
        indices: Vec<usize>,
        _storage: OwnedSemaphorePermit,
        observed: super::paired_observed::Index,
    },
    Incomplete(anyhow::Error),
}

impl CandidateBudget {
    pub(super) fn paired_candidates(
        &self,
        targets: Vec<PairedMobdev2Device>,
        usb_seen: impl Fn(&str) -> bool,
        permits: impl Fn(&str) -> bool,
        bank: &Arc<Semaphore>,
    ) -> PairedCandidates {
        match paired_snapshot(&targets).and_then(|()| {
            let plan = self.paired_plan(&targets, usb_seen, permits, bank)?;
            let storage = plan.1;
            let indices = plan.0;
            let observed = super::paired_observed::Index::new(&targets, bank)?;
            Ok((indices, storage, observed))
        }) {
            Ok((indices, storage, observed)) => PairedCandidates::Complete {
                targets,
                indices,
                _storage: storage,
                observed,
            },
            // Reject without retaining the input across USB candidate waits.
            Err(error) => PairedCandidates::Incomplete(error),
        }
    }

    pub(super) fn admit(&mut self, udid_bytes: usize, host_bytes: usize) -> Result<()> {
        let limit = || crate::output::resource_decision("tunnel scan candidate metadata limit");
        let count = self
            .count
            .checked_add(1)
            .filter(|count| *count <= MAX_CANDIDATES)
            .ok_or_else(limit)?;
        let bytes = self
            .bytes
            .checked_add(udid_bytes)
            .and_then(|bytes| bytes.checked_add(host_bytes))
            .filter(|bytes| *bytes <= MAX_CANDIDATE_TEXT_BYTES)
            .ok_or_else(limit)?;
        self.count = count;
        self.bytes = bytes;
        Ok(())
    }

    fn paired_plan(
        &self,
        targets: &[PairedMobdev2Device],
        usb_seen: impl Fn(&str) -> bool,
        permits: impl Fn(&str) -> bool,
        bank: &Arc<Semaphore>,
    ) -> Result<(Vec<usize>, OwnedSemaphorePermit)> {
        let mut budget = *self;
        let allocation = super::metadata_vec::exact_backing(bank, targets.len())?;
        let storage = allocation.storage;
        let mut indices = allocation.values;
        for (index, target) in targets.iter().enumerate() {
            if permits(&target.udid) && !usb_seen(&target.udid) {
                indices.push(index);
            }
        }
        indices.sort_unstable_by(|a, b| targets[*a].udid.cmp(&targets[*b].udid).then(a.cmp(b)));
        let mut read = 0;
        let mut write = 0;
        while read < indices.len() {
            let first = indices[read];
            let mut end = read + 1;
            while end < indices.len() && targets[indices[end]].udid == targets[first].udid {
                end += 1;
            }
            // Admit the full unique borrowed plan before any String clone or
            // ticket. The first matching raw row and original order are kept.
            budget.admit(targets[first].udid.len(), targets[first].host.len())?;
            indices[write] = first;
            write += 1;
            read = end;
        }
        indices.truncate(write);
        indices.sort_unstable();
        Ok((indices, storage))
    }
}
