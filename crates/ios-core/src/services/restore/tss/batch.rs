//! Owned observations of independent signing attempts, without changing legacy results.

use std::fmt;

use super::{RestoreError, TssBatchResult};
use crate::ErrorClass;

/// The step that produced a batch or per-rider failure.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum TssBatchPhase {
    BuildCombinedRequest,
    SignCombinedRequest,
    SignIndividualRequest,
}

/// A failed call and a successful response without a usable ticket are distinct.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum TssBatchFailureKind {
    Error,
    MissingTicket,
}

/// One failed attempt, including attempts later recovered by individual signing.
///
/// Independent attempts have independent errors; they are not one source chain.
/// Debug includes only typed metadata, never signing fields or error payloads.
pub struct TssBatchFailure {
    phase: TssBatchPhase,
    rider_index: Option<usize>,
    error: Option<RestoreError>,
}

impl TssBatchFailure {
    /// The request-building or signing step that failed.
    pub fn phase(&self) -> TssBatchPhase {
        self.phase
    }

    /// Whether an error was returned or a successful response lacked a ticket.
    pub fn kind(&self) -> TssBatchFailureKind {
        if self.error.is_some() {
            TssBatchFailureKind::Error
        } else {
            TssBatchFailureKind::MissingTicket
        }
    }

    /// Index in the supplied riders; None means the whole combined request failed.
    /// Indices distinguish duplicate updater names without exposing identifiers.
    pub fn rider_index(&self) -> Option<usize> {
        self.rider_index
    }

    /// The original returned error and its source chain, when the attempt failed.
    /// Missing/empty/non-data tickets have no fabricated native error.
    pub fn error(&self) -> Option<&RestoreError> {
        self.error.as_ref()
    }

    /// Classify the original error; missing-ticket observations return None.
    pub fn error_class(&self) -> Option<ErrorClass> {
        self.error
            .as_ref()
            .map(|error| crate::classify_error(error))
    }

    /// Take the original error, retaining ownership of its native sources.
    pub fn into_error(self) -> Option<RestoreError> {
        self.error
    }

    pub(super) fn new(
        phase: TssBatchPhase,
        rider_index: Option<usize>,
        error: Option<RestoreError>,
    ) -> Self {
        Self {
            phase,
            rider_index,
            error,
        }
    }
}

impl fmt::Debug for TssBatchFailure {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("TssBatchFailure")
            .field("phase", &self.phase())
            .field("kind", &self.kind())
            .field("rider_index", &self.rider_index())
            .field("error_class", &self.error_class())
            .finish_non_exhaustive()
    }
}

/// Tickets and ordered failure observations from batch signing and its fallback.
///
/// A nonempty failures list can accompany complete success after fallback. Final
/// failures are still reported by result().failed_updaters. Empty input performs
/// no signing and produces an empty result and observations.
pub struct TssBatchOutcome {
    result: TssBatchResult,
    failures: Vec<TssBatchFailure>,
}

impl TssBatchOutcome {
    /// Borrow the same tickets and final failed updater names as the legacy API.
    pub fn result(&self) -> &TssBatchResult {
        &self.result
    }

    /// Failed attempts in encounter order, including intermediate missing tickets.
    pub fn failures(&self) -> &[TssBatchFailure] {
        &self.failures
    }

    /// Return the legacy result, deliberately dropping the failure observations.
    pub fn into_result(self) -> TssBatchResult {
        self.result
    }

    /// Keep the result and all independently owned failure observations.
    pub fn into_parts(self) -> (TssBatchResult, Vec<TssBatchFailure>) {
        (self.result, self.failures)
    }

    pub(super) fn new(result: TssBatchResult, failures: Vec<TssBatchFailure>) -> Self {
        Self { result, failures }
    }
}

impl fmt::Debug for TssBatchOutcome {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("TssBatchOutcome")
            .field("ticket_count", &self.result.tickets.len())
            .field("failed_updater_count", &self.result.failed_updaters.len())
            .field("failure_count", &self.failures.len())
            .finish_non_exhaustive()
    }
}
