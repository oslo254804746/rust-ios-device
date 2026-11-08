use super::LockdownError;
use std::{error::Error, io};

pub(super) fn native(
    kind: io::ErrorKind,
    phase: &'static str,
    source: impl Error + Send + Sync + 'static,
) -> LockdownError {
    LockdownError::Io(crate::error::context::retained_io(kind, phase, source))
}
