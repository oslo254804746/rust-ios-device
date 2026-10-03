//! Per-push events and independently owned decoding failures.

use std::{error::Error, fmt};

use super::{Direction, DproxyError, ProxyEvent};
use crate::ErrorClass;

/// The decoder step that produced an observation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum DecodePhase {
    Buffer,
    Lockdown,
    Dtx,
    H2Frame,
    XpcStream,
    XpcMessage,
    XpcEndStream,
}

/// Native errors and source-free local decisions are distinct.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum DecodeFailureKind {
    Error,
    ResourceLimit,
    IncompleteMessage,
    InvalidState,
}

/// A failed decoding step; it need not terminate recording or forwarding.
/// Debug includes metadata only. Explicit sources may contain private data.
pub struct DecodeFailure {
    pub(super) direction: Direction,
    pub(super) phase: DecodePhase,
    pub(super) kind: DecodeFailureKind,
    pub(super) event_index: Option<usize>,
    pub(super) stream_id: Option<u32>,
    pub(super) error: Option<Box<dyn Error + Send + Sync>>,
}

impl DecodeFailure {
    pub fn direction(&self) -> Direction {
        self.direction
    }

    pub fn phase(&self) -> DecodePhase {
        self.phase
    }

    pub fn kind(&self) -> DecodeFailureKind {
        self.kind
    }

    /// Index in this push's event list; fatal guards have no corresponding event.
    pub fn event_index(&self) -> Option<usize> {
        self.event_index
    }

    pub fn stream_id(&self) -> Option<u32> {
        self.stream_id
    }

    /// The original native error; local guards have no fabricated source.
    pub fn error(&self) -> Option<&(dyn Error + Send + Sync + 'static)> {
        self.error.as_deref()
    }

    /// Classify the original error only; source-free guards return None.
    pub fn error_class(&self) -> Option<ErrorClass> {
        self.error
            .as_ref()
            .map(|error| crate::classify_error(error.as_ref()))
    }

    pub fn into_error(self) -> Option<Box<dyn Error + Send + Sync>> {
        self.error
    }
}

impl fmt::Debug for DecodeFailure {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("DecodeFailure")
            .field("direction", &self.direction)
            .field("phase", &self.phase)
            .field("kind", &self.kind)
            .field("event_index", &self.event_index)
            .field("stream_id", &self.stream_id)
            .field("error_class", &self.error_class())
            .finish_non_exhaustive()
    }
}

/// A push can decode events, observe nonfatal failures, and then fail fatally.
/// Those independent results are retained together, not one Error::source chain.
pub struct DecodeOutcome {
    pub(super) events: Vec<ProxyEvent>,
    pub(super) failures: Vec<DecodeFailure>,
    pub(super) fatal_error: Option<DproxyError>,
    pub(super) input_ignored: bool,
}

impl DecodeOutcome {
    pub fn events(&self) -> &[ProxyEvent] {
        &self.events
    }

    pub fn failures(&self) -> &[DecodeFailure] {
        &self.failures
    }

    pub fn fatal_error(&self) -> Option<&DproxyError> {
        self.fatal_error.as_ref()
    }

    /// Classify the unchanged legacy fatal error, independently of guard kinds.
    pub fn fatal_error_class(&self) -> Option<ErrorClass> {
        self.fatal_error
            .as_ref()
            .map(|error| crate::classify_error(error))
    }

    /// True only when a previous DTX failure caused this input to be ignored.
    /// No events alone does not imply ignored input or complete decoding.
    pub fn input_ignored(&self) -> bool {
        self.input_ignored
    }

    /// Deliberately discard observations and, on fatal error, preceding events.
    pub fn into_result(self) -> Result<Vec<ProxyEvent>, DproxyError> {
        match self.fatal_error {
            Some(error) => Err(error),
            None => Ok(self.events),
        }
    }

    /// Keep partial events, independent native sources and the fatal error.
    pub fn into_parts(self) -> (Vec<ProxyEvent>, Vec<DecodeFailure>, Option<DproxyError>) {
        (self.events, self.failures, self.fatal_error)
    }

    pub(super) fn new() -> Self {
        Self {
            events: Vec::new(),
            failures: Vec::new(),
            fatal_error: None,
            input_ignored: false,
        }
    }

    pub(super) fn observe_native(
        &mut self,
        preserve: bool,
        direction: Direction,
        phase: DecodePhase,
        stream_id: Option<u32>,
        error: impl Error + Send + Sync + 'static,
    ) {
        if preserve {
            self.failures.push(DecodeFailure {
                direction,
                phase,
                kind: DecodeFailureKind::Error,
                event_index: Some(self.events.len()),
                stream_id,
                error: Some(Box::new(error)),
            });
        }
    }

    pub(super) fn observe_guard(
        &mut self,
        preserve: bool,
        direction: Direction,
        phase: DecodePhase,
        kind: DecodeFailureKind,
        stream_id: Option<u32>,
        has_event: bool,
    ) {
        if preserve {
            self.failures.push(DecodeFailure {
                direction,
                phase,
                kind,
                event_index: has_event.then_some(self.events.len()),
                stream_id,
                error: None,
            });
        }
    }
}

impl fmt::Debug for DecodeOutcome {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("DecodeOutcome")
            .field("event_count", &self.events.len())
            .field("failure_count", &self.failures.len())
            .field("fatal_error_class", &self.fatal_error_class())
            .field("input_ignored", &self.input_ignored)
            .finish_non_exhaustive()
    }
}
