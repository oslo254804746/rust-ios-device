//! Keep original sources through the existing public pairing error family.

use super::PairingTransportError;
use crate::{lockdown::pairing::PairingError, xpc::h2_raw::H2Error, XpcError};
use std::{error::Error, io};

pub(super) fn native(
    kind: io::ErrorKind,
    phase: &'static str,
    source: impl Error + Send + Sync + 'static,
) -> PairingTransportError {
    PairingTransportError::Io(crate::device::error_context::retained_io(
        kind, phase, source,
    ))
}

pub(super) fn pairing(phase: &'static str, source: PairingError) -> PairingTransportError {
    let kind = match &source {
        PairingError::Io(error) => error.kind(),
        _ => io::ErrorKind::Other,
    };
    native(kind, phase, source)
}

pub(super) fn h2(phase: &'static str, source: H2Error) -> PairingTransportError {
    let kind = match &source {
        H2Error::Io(error) => error.kind(),
        H2Error::Protocol(_) => io::ErrorKind::InvalidData,
    };
    native(kind, phase, source)
}

pub(super) fn xpc(phase: &'static str, source: XpcError) -> PairingTransportError {
    let kind = match &source {
        XpcError::Io(error) => error.kind(),
        _ => io::ErrorKind::Other,
    };
    native(kind, phase, source)
}

pub(super) fn dial_timeout(source: tokio::time::error::Elapsed) -> PairingTransportError {
    native(
        io::ErrorKind::TimedOut,
        "dial manual pairing service",
        source,
    )
}
