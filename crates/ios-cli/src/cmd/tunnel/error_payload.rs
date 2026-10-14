//! Read-only accounting of retained original error payloads, without formatting.
//!
//! Known String capacities are a lower bound, never an estimate of opaque heap
//! allocations. Unknown nodes and active scans explicitly prevent a complete
//! measurement. This census is not a byte admission limit or an RSS measurement.
use std::collections::HashSet;
use std::error::Error;
use std::marker::PhantomData;

use ios_core::{CoreError, LockdownError, MuxError, PairRecordError, TunnelError, XpcError};

const MAX_NODES: usize = 65_536;
const MAX_DEPTH: usize = 32;

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub(super) struct Snapshot {
    pub(super) known_payload_capacity_bytes: usize,
    pub(super) opaque_nodes: usize,
    pub(super) visited_nodes: usize,
    pub(super) active_scans: usize,
    pub(super) unjoined_native_owners: usize,
    pub(super) truncated: bool,
}

impl Snapshot {
    pub(super) fn fully_measured_payloads(self) -> bool {
        self.opaque_nodes == 0
            && self.active_scans == 0
            && self.unjoined_native_owners == 0
            && !self.truncated
    }
}

#[derive(Default)]
struct Census<'a> {
    snapshot: Snapshot,
    // Kind separates inline transparent enum/source objects at the same address.
    nodes: HashSet<(usize, u8)>,
    buffers: HashSet<usize>,
    // Every root must outlive the census, including native status snapshots.
    borrowed: PhantomData<&'a (dyn Error + 'static)>,
}

impl<'a> Census<'a> {
    fn string(&mut self, value: &'a String) {
        if value.capacity() != 0 && self.buffers.insert(value.as_ptr() as usize) {
            match self
                .snapshot
                .known_payload_capacity_bytes
                .checked_add(value.capacity())
            {
                Some(total) => self.snapshot.known_payload_capacity_bytes = total,
                None => self.snapshot.truncated = true,
            }
        }
    }

    fn anyhow(&mut self, error: &'a anyhow::Error, depth: usize) {
        if depth >= MAX_DEPTH || self.nodes.len() >= MAX_NODES {
            self.snapshot.truncated = true;
            return;
        }
        if let Some(value) = error.downcast_ref::<String>() {
            self.string(value);
        }
        if let Some(context) = error.downcast_ref::<super::candidate_input::SourceInputContext>() {
            self.string(&context.input.udid);
            match &context.input.route {
                super::source::Route::Usbmux => {}
                super::source::Route::Usb(host) | super::source::Route::Wifi(host) => {
                    if let Some(host) = host {
                        self.string(host);
                    }
                }
                super::source::Route::Mobdev2(host) => self.string(host),
            }
        }
        // Anyhow contexts can have independent causes not exposed by source().
        if let Some(context) = error.downcast_ref::<super::bookkeeping::FailureContext>() {
            self.anyhow(context.secondary(), depth + 1);
        }
        if let Some(context) = error.downcast_ref::<super::scan::SourceContext>() {
            for source in [
                super::scan::Source::Usb,
                super::scan::Source::Paired,
                super::scan::Source::Coordinator,
                super::scan::Source::Freshness,
            ] {
                if let Some(error) = context.secondary(source) {
                    self.anyhow(error, depth + 1);
                }
            }
            if let Some(error) = context.completion() {
                self.anyhow(error, depth + 1);
            }
        }
        if let Some(context) = error.downcast_ref::<super::bookkeeping::SourceFailuresContext>() {
            for receipt in context.receipts() {
                self.anyhow(receipt.primary(), depth + 1);
                self.anyhow(receipt.latest(), depth + 1);
            }
        }
        if let Some(failure) = error.downcast_ref::<super::freshness::Failure>() {
            self.anyhow(failure.original(), depth + 1);
        }
        self.error(error.as_ref(), depth);
    }

    fn error(&mut self, error: &'a (dyn Error + 'static), depth: usize) {
        if depth >= MAX_DEPTH || self.nodes.len() >= MAX_NODES {
            self.snapshot.truncated = true;
            return;
        }
        let kind = if error.is::<CoreError>() {
            1
        } else if error.is::<MuxError>() {
            2
        } else if error.is::<LockdownError>() {
            3
        } else if error.is::<PairRecordError>() {
            4
        } else if error.is::<TunnelError>() {
            5
        } else if error.is::<XpcError>() {
            6
        } else if error.is::<std::io::Error>() {
            7
        } else if error.is::<ios_core::TunnelShutdownFailure>() {
            8
        } else {
            0
        };
        if !self
            .nodes
            .insert((error as *const dyn Error as *const () as usize, kind))
        {
            return;
        }
        self.snapshot.visited_nodes += 1;
        let mut known = true;
        if let Some(error) = error.downcast_ref::<CoreError>() {
            match error {
                CoreError::Protocol(value)
                | CoreError::DeviceNotFound(value)
                | CoreError::Unsupported(value)
                | CoreError::Other(value) => self.string(value),
                _ => {}
            }
        } else if let Some(MuxError::Protocol(value) | MuxError::DeviceNotFound(value)) =
            error.downcast_ref()
        {
            self.string(value);
        } else if let Some(LockdownError::Protocol(value)) = error.downcast_ref() {
            self.string(value);
        } else if let Some(error) = error.downcast_ref::<PairRecordError>() {
            match error {
                PairRecordError::NotFound(value) | PairRecordError::Parse(value) => {
                    self.string(value)
                }
                // PathBuf storage is OS-specific and not counted as UTF-8 bytes.
                PairRecordError::Read { .. } => known = false,
            }
        } else if let Some(TunnelError::Protocol(value) | TunnelError::TunDevice(value)) =
            error.downcast_ref()
        {
            self.string(value);
        } else if let Some(XpcError::ServiceNotFound(value) | XpcError::Tls(value)) =
            error.downcast_ref()
        {
            self.string(value);
        } else if let Some(error) = error.downcast_ref::<ios_core::TunnelShutdownFailure>() {
            // The snapshot's Vec<Cause> capacity and Arc/allocator backing are
            // not available here; retained payload accounting stays incomplete.
            known = false;
            self.error(error.primary(), depth + 1);
            for cause in error.cleanup() {
                self.error(cause, depth + 1);
            }
        } else if let Some(error) = error.downcast_ref::<std::io::Error>() {
            if let Some(payload) = error.get_ref() {
                // IO source() may skip the custom payload itself.
                self.error(payload, depth + 1);
            }
        } else if kind == 0 {
            // In particular JoinError can own an arbitrary panic payload. Its
            // node size/source/category cannot establish retained heap bytes.
            known = error.is::<tokio::time::error::Elapsed>()
                || error.is::<ios_core::OperationCancelled>();
        }
        if !known {
            self.snapshot.opaque_nodes += 1;
        }
        if let Some(source) = error.source() {
            self.error(source, depth + 1);
        }
    }
}

impl super::TunnelAgentState {
    pub(super) fn retained_error_payloads(&self) -> Snapshot {
        // Clone original owners under each short lock; inspect after release.
        // This is a sampled union, not a globally linearized admission decision.
        let receipts = self.inner.retry.payload_receipts();
        let (active_scans, scans) = self.inner.scan_outcomes.payload_history();
        let (unjoined_native_owners, native) = self.inner.native.payload_outcomes();
        let freshness = self.inner.native.payload_freshness();
        let receipts = match receipts {
            Ok(receipts) => receipts,
            Err(_) => {
                return Snapshot {
                    active_scans,
                    unjoined_native_owners,
                    truncated: true,
                    ..Snapshot::default()
                }
            }
        };
        let freshness = match freshness {
            Ok(roots) => roots,
            Err(_) => {
                return Snapshot {
                    active_scans,
                    unjoined_native_owners,
                    truncated: true,
                    ..Snapshot::default()
                }
            }
        };
        let mut census = Census::default();
        census.snapshot.active_scans = active_scans;
        census.snapshot.unjoined_native_owners = unjoined_native_owners;
        for receipt in receipts.as_slice() {
            census.anyhow(receipt.primary(), 0);
            census.anyhow(receipt.latest(), 0);
        }
        for scan in &scans {
            census.anyhow(scan.original(), 0);
        }
        for receipt in freshness.as_slice() {
            census.anyhow(receipt.original(), 0);
        }
        for error in &native {
            census.error(error.as_ref(), 0);
        }
        census.snapshot
    }
}

#[cfg(test)]
mod tests;
