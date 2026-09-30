use crate::lockdown::pair_record::PairRecordError;

/// Aggregated error type for ios-core operations.
#[derive(thiserror::Error)]
pub enum CoreError {
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
    #[error("usbmuxd error: {0}")]
    Mux(#[from] crate::mux::MuxError),
    #[error("lockdown error: {0}")]
    Lockdown(#[from] crate::lockdown::LockdownError),
    #[error("pair record error: {0}")]
    PairRecord(#[from] PairRecordError),
    #[error("tunnel error: {0}")]
    Tunnel(#[from] crate::tunnel::TunnelError),
    #[error("XPC error: {0}")]
    Xpc(#[from] crate::xpc::XpcError),
    #[error("JSON error: {0}")]
    Json(#[from] serde_json::Error),
    #[error("protocol error: {0}")]
    Protocol(String),
    #[error("device not found: {}", crate::redaction::diagnostic_id(.0))]
    DeviceNotFound(String),
    #[error("operation not supported: {0}")]
    Unsupported(String),
    #[error("{0}")]
    Other(String),
}

// Error fields and sources remain available to explicit callers. Debug is a
// diagnostic surface and must not render arbitrary protocol/credential data.
impl std::fmt::Debug for CoreError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let name = match self {
            Self::Io(_) => "CoreError::Io",
            Self::Mux(_) => "CoreError::Mux",
            Self::Lockdown(_) => "CoreError::Lockdown",
            Self::PairRecord(_) => "CoreError::PairRecord",
            Self::Tunnel(_) => "CoreError::Tunnel",
            Self::Xpc(_) => "CoreError::Xpc",
            Self::Json(_) => "CoreError::Json",
            Self::Protocol(_) => "CoreError::Protocol",
            Self::DeviceNotFound(_) => "CoreError::DeviceNotFound",
            Self::Unsupported(_) => "CoreError::Unsupported",
            Self::Other(_) => "CoreError::Other",
        };
        let mut output = f.debug_struct(name);
        if let Self::DeviceNotFound(identifier) = self {
            output.field("device", &crate::redaction::diagnostic_id(identifier));
        }
        if let Self::Io(error) = self {
            output.field("kind", &error.kind());
            output.field("os_code", &error.raw_os_error());
        }
        output.finish_non_exhaustive()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn core_error_debug_hides_identifiers_and_arbitrary_source_data() {
        let secret = "synthetic-credential-and-device-identifier";
        let errors = [
            CoreError::DeviceNotFound(secret.into()),
            CoreError::Protocol(secret.into()),
            CoreError::Other(secret.into()),
            CoreError::Io(std::io::Error::other(secret)),
            CoreError::Mux(crate::mux::MuxError::Protocol(secret.into())),
            CoreError::PairRecord(PairRecordError::Parse(secret.into())),
        ];
        for error in errors {
            let debug = format!("{error:?} {error:#?}");
            assert!(!debug.contains(secret));
            assert!(debug.contains("CoreError::"));
        }
        let error = CoreError::DeviceNotFound(secret.into());
        assert!(!error.to_string().contains(secret));
        assert!(error.to_string().contains("device not found: id:"));
        assert!(matches!(error, CoreError::DeviceNotFound(value) if value == secret));
        let mux = crate::mux::MuxError::DeviceNotFound(secret.into());
        assert!(!format!("{mux} {mux:?} {mux:#?}").contains(secret));
    }
}
