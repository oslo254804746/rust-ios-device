//! Admission of manager selectors, retained Source snapshots and scan candidates.
//! Source allocation before return and allocator/RSS are separate boundaries.
use std::collections::HashSet;
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

pub(super) fn known_vec_backing<T>(
    bank: &Arc<Semaphore>,
    values: &Vec<T>,
) -> Result<OwnedSemaphorePermit> {
    let bytes = values
        .capacity()
        .checked_mul(std::mem::size_of::<T>())
        .ok_or_else(|| {
            crate::output::resource_decision("tunnel known vector backing size overflow")
        })?;
    known_metadata_charge(bank, bytes)
}

pub(super) fn retain_usb_snapshot(
    bank: &Arc<Semaphore>,
    devices: &Vec<UsbDevice>,
) -> Result<OwnedSemaphorePermit> {
    usb_snapshot(devices)?;
    let text = devices
        .iter()
        .map(|device| device.udid.capacity())
        .sum::<usize>();
    // The per-snapshot validator already checked these same capacities; the
    // borrowed input cannot change before this synchronous global admission.
    let bytes = devices.capacity() * std::mem::size_of::<UsbDevice>() + text;
    known_metadata_charge(bank, bytes)
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

pub(super) fn usb_snapshot(devices: &Vec<UsbDevice>) -> Result<()> {
    anyhow::ensure!(devices.len() <= MAX_RAW_ROWS, snapshot_limit());
    let mut bytes = 0;
    let mut unique = HashSet::new();
    for device in devices {
        charge_field(&mut bytes, &device.udid)?;
        if !unique.contains(device.udid.as_str()) {
            // Count all raw identities before selection/authorization or clones.
            anyhow::ensure!(unique.len() < MAX_CANDIDATES, snapshot_limit());
            unique.insert(device.udid.as_str());
        }
    }
    snapshot_storage(devices.capacity(), std::mem::size_of::<UsbDevice>(), bytes)
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
    },
    Incomplete(anyhow::Error),
}

impl CandidateBudget {
    pub(super) fn paired_candidates(
        &self,
        targets: Vec<PairedMobdev2Device>,
        usb_seen: &HashSet<String>,
        permits: impl Fn(&str) -> bool,
    ) -> PairedCandidates {
        match paired_snapshot(&targets).and_then(|()| self.paired_plan(&targets, usb_seen, permits))
        {
            Ok(indices) => PairedCandidates::Complete { targets, indices },
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

    pub(super) fn paired_plan(
        &self,
        targets: &[PairedMobdev2Device],
        usb_seen: &HashSet<String>,
        permits: impl Fn(&str) -> bool,
    ) -> Result<Vec<usize>> {
        let mut budget = *self;
        let mut planned = HashSet::new();
        let mut indices = Vec::new();
        for (index, target) in targets.iter().enumerate() {
            if permits(&target.udid)
                && !usb_seen.contains(&target.udid)
                && !planned.contains(target.udid.as_str())
            {
                // Admit the entire borrowed plan before any paired String clone
                // or ticket. Permission rechecks may only remove planned rows.
                budget.admit(target.udid.len(), target.host.len())?;
                planned.insert(target.udid.as_str());
                indices.push(index);
            }
        }
        Ok(indices)
    }
}
