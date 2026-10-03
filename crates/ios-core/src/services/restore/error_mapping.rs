//! Fixed owner context with original transport, codec and local package causes.

use std::{error::Error, io::ErrorKind};

use super::RestoreError;
use crate::xpc::{h2_raw::H2Error, XpcError};

pub(super) fn native_error(
    kind: ErrorKind,
    phase: &'static str,
    source: impl Error + Send + Sync + 'static,
) -> RestoreError {
    RestoreError::Io(crate::error::context::retained_io(kind, phase, source))
}

pub(super) fn native_io(phase: &'static str, source: std::io::Error) -> RestoreError {
    native_error(source.kind(), phase, source)
}

pub(super) fn native_h2(phase: &'static str, source: H2Error) -> RestoreError {
    let kind = match &source {
        H2Error::Io(error) => error.kind(),
        H2Error::Protocol(_) => ErrorKind::InvalidData,
    };
    native_error(kind, phase, source)
}

pub(super) fn native_xpc(phase: &'static str, source: XpcError) -> RestoreError {
    let kind = match &source {
        XpcError::Io(error) => error.kind(),
        _ => ErrorKind::Other,
    };
    native_error(kind, phase, source)
}

pub(super) fn native_http(phase: &'static str, source: reqwest::Error) -> RestoreError {
    let kind = if source.is_timeout() {
        ErrorKind::TimedOut
    } else if source.is_builder() {
        ErrorKind::InvalidInput
    } else {
        ErrorKind::Other
    };
    native_error(kind, phase, source)
}

pub(super) fn native_zip(phase: &'static str, source: zip::result::ZipError) -> RestoreError {
    let kind = match &source {
        zip::result::ZipError::Io(error) => error.kind(),
        // IPSW is the caller-supplied local package, unlike a received service frame.
        _ => ErrorKind::InvalidInput,
    };
    native_error(kind, phase, source)
}
