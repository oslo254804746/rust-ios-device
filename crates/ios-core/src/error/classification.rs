//! Additive classification of existing errors; never rewrite their sources.

use std::error::Error;
use std::io::ErrorKind;

mod services;

/// Machine categories with stable string codes. Future categories may be added;
/// consumers must retain an unknown/default branch. A category alone does not
/// imply that retry is safe or that device cleanup/rollback has completed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum ErrorClass {
    Timeout,
    Cancelled,
    Disconnected,
    ServiceMissing,
    PermissionDenied,
    Protocol,
    InvalidInput,
    Unsupported,
    DeviceNotFound,
    NotFound,
    ResourceLimit,
    Conflict,
    Precondition,
    ServiceFailure,
    Io,
    Unknown,
}

impl ErrorClass {
    /// Stable code for machine output, independent of Display/Debug prose.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Timeout => "timeout",
            Self::Cancelled => "cancelled",
            Self::Disconnected => "disconnected",
            Self::ServiceMissing => "service_missing",
            Self::PermissionDenied => "permission_denied",
            Self::Protocol => "protocol",
            Self::InvalidInput => "invalid_input",
            Self::Unsupported => "unsupported",
            Self::DeviceNotFound => "device_not_found",
            Self::NotFound => "not_found",
            Self::ResourceLimit => "resource_limit",
            Self::Conflict => "conflict",
            Self::Precondition => "precondition",
            Self::ServiceFailure => "service_failure",
            Self::Io => "io",
            Self::Unknown => "unknown",
        }
    }
}

impl std::fmt::Display for ErrorClass {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

const MAX_CLASSIFICATION_SOURCES: usize = 32;

/// Inspect at most 32 typed sources, without formatting or matching messages.
/// Context wrappers and recognized IO/codec wrappers retain their original
/// chain. The first precise typed category wins; generic IO/codec categories
/// are fallbacks if no source supplies a more precise category. Source cycles
/// terminate at the same bound. Flattened legacy strings cannot recover lost
/// types: a protocol string remains Protocol and an opaque string is Unknown.
pub fn classify_error(error: &(dyn Error + 'static)) -> ErrorClass {
    let mut current = Some(error);
    let mut fallback = ErrorClass::Unknown;
    for _ in 0..MAX_CLASSIFICATION_SOURCES {
        let Some(error) = current else { break };
        if let Some(class) = direct_class(error) {
            return class;
        }
        if let Some(io) = error.downcast_ref::<std::io::Error>() {
            if let Some(class) = io_class(io.kind()) {
                return class;
            }
            fallback = ErrorClass::Io;
            // io::Error's custom payload may itself be a typed error. Visit it
            // directly so std's source forwarding cannot skip that category.
            current = io.get_ref().map(|source| source as &(dyn Error + 'static));
            if current.is_none() {
                current = error.source();
            }
            continue;
        }
        if let Some(json) = error.downcast_ref::<serde_json::Error>() {
            if let Some(kind) = json.io_error_kind() {
                if let Some(class) = io_class(kind) {
                    return class;
                }
                fallback = ErrorClass::Io;
            } else {
                fallback = ErrorClass::Protocol;
            }
        } else if error.is::<plist::Error>() {
            fallback = ErrorClass::Protocol;
        }
        current = error.source();
    }
    fallback
}

fn io_class(kind: ErrorKind) -> Option<ErrorClass> {
    Some(match kind {
        ErrorKind::TimedOut => ErrorClass::Timeout,
        ErrorKind::PermissionDenied => ErrorClass::PermissionDenied,
        ErrorKind::NotConnected
        | ErrorKind::ConnectionAborted
        | ErrorKind::ConnectionReset
        | ErrorKind::BrokenPipe => ErrorClass::Disconnected,
        ErrorKind::InvalidInput => ErrorClass::InvalidInput,
        ErrorKind::InvalidData => ErrorClass::Protocol,
        ErrorKind::NotFound => ErrorClass::NotFound,
        // EINTR is not an acknowledged cancellation. UnexpectedEof can be a
        // truncated host file, so neither is guessed to mean device disconnect.
        _ => return None,
    })
}

// Each adapter examines variants, never a Display/Debug string. Wrapping
// variants fall through to their source; untyped legacy failures stay unknown.
macro_rules! typed_class {
    ($error:expr, $type:path, {$($pattern:pat => $class:expr),* $(,)?}) => {{
        if let Some(value) = $error.downcast_ref::<$type>() {
            return match value {
                $($pattern => Some($class),)*
                _ => None,
            };
        }
    }};
}
use typed_class;

fn direct_class(error: &(dyn Error + 'static)) -> Option<ErrorClass> {
    if let Some(error) = error.downcast_ref::<super::context::RetainedCause>() {
        return Some(error.class());
    }
    #[cfg(feature = "mobileactivation")]
    if let Some(error) = error.downcast_ref::<crate::DeactivationFallbackError>() {
        // The earlier failure remains separately observable. A wrapper must not
        // replace an opaque final cause's Unknown with generic IO.
        return Some(error.fallback_class());
    }
    if error.is::<super::OperationCancelled>() {
        return Some(ErrorClass::Cancelled);
    }
    use crate::mux::MuxError;
    use crate::{CoreError, LockdownError, PairRecordError, TunnelError, XpcError};
    typed_class!(error, CoreError, {
        CoreError::Protocol(_) => ErrorClass::Protocol,
        CoreError::DeviceNotFound(_) => ErrorClass::DeviceNotFound,
        CoreError::Unsupported(_) => ErrorClass::Unsupported
    });
    typed_class!(error, MuxError, {
        MuxError::Protocol(_) => ErrorClass::Protocol,
        MuxError::DeviceNotFound(_) => ErrorClass::DeviceNotFound
    });
    typed_class!(error, LockdownError, {LockdownError::Protocol(_) => ErrorClass::Protocol});
    typed_class!(error, PairRecordError, {
        PairRecordError::NotFound(_) => ErrorClass::NotFound,
        PairRecordError::Parse(_) => ErrorClass::InvalidInput
    });
    typed_class!(error, TunnelError, {TunnelError::Protocol(_) => ErrorClass::Protocol});
    typed_class!(error, XpcError, {
        XpcError::NotConnected => ErrorClass::Disconnected,
        XpcError::ServiceNotFound(_) => ErrorClass::ServiceMissing,
        XpcError::Tls(_) => ErrorClass::Protocol
    });
    if error.is::<tokio::time::error::Elapsed>() {
        return Some(ErrorClass::Timeout);
    }
    if let Some(join) = error.downcast_ref::<tokio::task::JoinError>() {
        return join.is_cancelled().then_some(ErrorClass::Cancelled);
    }
    #[cfg(any(
        feature = "mobileactivation",
        feature = "imagemounter",
        feature = "restore"
    ))]
    if let Some(http) = error.downcast_ref::<reqwest::Error>() {
        if http.is_timeout() {
            return Some(ErrorClass::Timeout);
        }
        if http.is_builder() {
            return Some(ErrorClass::InvalidInput);
        }
    }
    #[cfg(all(feature = "tunnel", feature = "mdns"))]
    {
        use crate::PairingTransportError as Kind;
        typed_class!(error, Kind, {
            Kind::ServiceNotFound => ErrorClass::ServiceMissing,
            Kind::Rejected(_) => ErrorClass::PermissionDenied,
            Kind::MissingField(_) | Kind::UnexpectedType(_) | Kind::Xpc(_) | Kind::Rsd(_) => ErrorClass::Protocol
        });
    }
    #[cfg(any(
        feature = "tunnel",
        feature = "dproxy",
        feature = "fetchsymbols",
        feature = "restore"
    ))]
    {
        use crate::xpc::h2_raw::H2Error as Kind;
        typed_class!(error, Kind, {Kind::Protocol(_) => ErrorClass::Protocol});
    }
    #[cfg(feature = "tunnel")]
    {
        use crate::psk_tls::PskTlsError as Kind;
        typed_class!(error, Kind, {Kind::PskCipherUnsupported(_) => ErrorClass::Unsupported});
        use crate::lockdown::pairing::PairingError as Pairing;
        typed_class!(error, Pairing, {Pairing::Protocol(_) => ErrorClass::Protocol});
    }
    use crate::proto::nskeyedarchiver::ArchiveError as Archive;
    if let Some(archive) = error.downcast_ref::<Archive>() {
        return Some(match archive {
            Archive::TooDeep(_) => ErrorClass::ResourceLimit,
            Archive::Plist(_) | Archive::Invalid(_) | Archive::Integer(_) => ErrorClass::Protocol,
        });
    }
    use crate::proto::opack::OpackError as Opack;
    if let Some(opack) = error.downcast_ref::<Opack>() {
        return Some(match opack {
            Opack::TooDeep(_) => ErrorClass::ResourceLimit,
            Opack::UnexpectedEof | Opack::UnknownTag(_) | Opack::InvalidUtf8 | Opack::Encode(_) => {
                ErrorClass::Protocol
            }
        });
    }
    services::class(error)
}

#[cfg(test)]
mod tests;
