//! Issuer-specific device-operation deadline evidence.

use std::{error::Error, time::Duration};

use super::Mobilebackup2Error;

/// The device-side MobileBackup2 operation whose total budget expired.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum DeviceOperation {
    Unback,
    Extract,
}

/// The part of the operation that was awaiting progress when its budget expired.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum DeviceOperationTimeoutPhase {
    /// Validation, host layout, negotiation, request or transfer loop.
    Operation,
    /// Best-effort DeviceLink disconnect using the same absolute deadline.
    Disconnect,
}

/// Evidence of an actual MobileBackup2-issued Tokio timeout.
///
/// The standard source chain follows the final `Elapsed`. If disconnect also
/// times out after an operation failure, [`Self::operation_error`] retains that
/// separate failure. Neither cause establishes remote cleanup or safe reuse.
#[derive(thiserror::Error)]
#[error("device-side {operation:?} exceeded the total timeout of {} seconds ({phase:?})", .duration.as_secs())]
pub struct DeviceOperationTimeout {
    pub(super) operation: DeviceOperation,
    pub(super) phase: DeviceOperationTimeoutPhase,
    pub(super) duration: Duration,
    #[source]
    pub(super) elapsed: tokio::time::error::Elapsed,
    pub(super) operation_error: Option<Box<Mobilebackup2Error>>,
}
crate::redaction::error_debug!(DeviceOperationTimeout);

impl DeviceOperationTimeout {
    /// Device-side operation that owns this total budget.
    pub fn operation(&self) -> DeviceOperation {
        self.operation
    }

    /// Awaiting operation or disconnect when this particular timer expired.
    pub fn phase(&self) -> DeviceOperationTimeoutPhase {
        self.phase
    }

    /// Total configured budget, shared by operation and disconnect.
    pub fn duration(&self) -> Duration {
        self.duration
    }

    /// Earlier operation failure, when a later disconnect timeout replaced it.
    /// Native sources on this branch remain available through `Error::source`.
    pub fn operation_error(&self) -> Option<&Mobilebackup2Error> {
        self.operation_error.as_deref()
    }
}

impl Mobilebackup2Error {
    /// Find this service's issued deadline within at most 32 typed sources.
    /// Generic IO timeouts and unrelated `Elapsed` values return `None`.
    pub fn device_operation_timeout(&self) -> Option<&DeviceOperationTimeout> {
        let mut next: Option<&(dyn Error + 'static)> = Some(self);
        for _ in 0..32 {
            let Some(error) = next else { break };
            if let Some(timeout) = error.downcast_ref::<DeviceOperationTimeout>() {
                return Some(timeout);
            }
            next = error.source();
        }
        None
    }
}

pub(super) fn timeout_error(
    operation: DeviceOperation,
    phase: DeviceOperationTimeoutPhase,
    duration: Duration,
    elapsed: tokio::time::error::Elapsed,
    operation_error: Option<Mobilebackup2Error>,
) -> Mobilebackup2Error {
    Mobilebackup2Error::Io(crate::error::context::retained_io(
        std::io::ErrorKind::TimedOut,
        "MobileBackup2 device operation",
        DeviceOperationTimeout {
            operation,
            phase,
            duration,
            elapsed,
            operation_error: operation_error.map(Box::new),
        },
    ))
}
