//! Diagnostic identifiers are process-local pseudonyms, never raw device IDs.
//!
//! Explicit device-selection/API output retains its identifiers. Use this
//! wrapper in diagnostics only; it cannot sanitize arbitrary device payloads.

use std::error::Error;
use std::fmt;
use std::fmt::Write;
use std::sync::OnceLock;

use rand::RngCore;
use sha2::{Digest, Sha256};

static PROCESS_KEY: OnceLock<Option<[u8; 32]>> = OnceLock::new();

#[cfg(test)]
pub(crate) mod test_support;

// Internal implementation shared by owned error types. Preserve typed fields,
// variants, Display and sources; Debug is always an automatic diagnostic view.
macro_rules! error_debug {
    ($name:ty) => {
        impl std::fmt::Debug for $name {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                f.debug_struct(stringify!($name))
                    .field("diagnostic", &$crate::redaction::diagnostic_error(self))
                    .finish_non_exhaustive()
            }
        }
    };
}
pub(crate) use error_debug;

fn process_key() -> &'static Option<[u8; 32]> {
    PROCESS_KEY.get_or_init(|| {
        let mut key = [0; 32];
        rand::rngs::OsRng.try_fill_bytes(&mut key).ok().map(|_| key)
    })
}

/// Borrowed identifier formatted as an opaque process-local correlation value.
pub struct DiagnosticId<'a>(&'a str);

/// Format a device/host identifier without disclosing the original value.
///
/// Correlation is valid only within the process. If system entropy is
/// unavailable, formatting falls back to a fixed redacted label.
pub fn diagnostic_id(value: &str) -> DiagnosticId<'_> {
    DiagnosticId(value)
}

impl fmt::Display for DiagnosticId<'_> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match process_key() {
            Some(key) => write!(formatter, "id:{}", encode_id(key, self.0)),
            None => formatter.write_str("id:redacted"),
        }
    }
}

/// Safe diagnostic view of an error and its source chain.
///
/// Pair this with a caller-owned phase (for example, `"tunnel dial failed"`).
/// The view emits a typed category/IO kind and a process-local correlation ID,
/// never error text, Debug payloads, URLs or credentials. Error fields and
/// sources remain available through the original typed error to explicit API
/// callers. Correlation hashes at most 4096 formatted bytes and eight sources
/// without constructing an intermediate string. Source cycles are bounded.
pub fn diagnostic_error<'a>(error: &'a (dyn Error + 'static)) -> DiagnosticError<'a> {
    DiagnosticError(error)
}

/// Borrowed error formatted for logs, error replies and exceptions.
pub struct DiagnosticError<'a>(&'a (dyn Error + 'static));

const MAX_ERROR_SOURCES: usize = 8;
const MAX_ERROR_HASH_BYTES: usize = 4096;

impl fmt::Display for DiagnosticError<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let (category, kind) = error_category(self.0);
        write!(f, "{category}")?;
        if let Some(kind) = kind {
            write!(f, "/{kind:?}")?;
        }
        match process_key() {
            Some(key) => write!(f, " error:{}", error_id(key, self.0)),
            None => f.write_str(" error:redacted"),
        }
    }
}

impl fmt::Debug for DiagnosticError<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(self, f)
    }
}

fn error_category(error: &(dyn Error + 'static)) -> (&'static str, Option<std::io::ErrorKind>) {
    let mut current = Some(error);
    for _ in 0..MAX_ERROR_SOURCES {
        let Some(error) = current else { break };
        if let Some(error) = error.downcast_ref::<std::io::Error>() {
            return ("io", Some(error.kind()));
        }
        if let Some(error) = error.downcast_ref::<crate::CoreError>() {
            use crate::CoreError;
            return match error {
                CoreError::Io(error) => ("io", Some(error.kind())),
                CoreError::Mux(_) => ("usbmux", None),
                CoreError::Lockdown(_) => ("lockdown", None),
                CoreError::PairRecord(_) => ("pair_record", None),
                CoreError::Tunnel(_) => ("tunnel", None),
                CoreError::Xpc(_) => ("xpc", None),
                CoreError::Json(_) => ("json", None),
                CoreError::Protocol(_) => ("protocol", None),
                CoreError::DeviceNotFound(_) => ("device_not_found", None),
                CoreError::Unsupported(_) => ("unsupported", None),
                CoreError::Other(_) => ("operation", None),
            };
        }
        if error.is::<serde_json::Error>() {
            return ("json", None);
        }
        if error.is::<crate::lockdown::pair_record::PairRecordError>() {
            return ("pair_record", None);
        }
        if error.is::<crate::mux::MuxError>() {
            return ("usbmux", None);
        }
        current = error.source();
    }
    ("operation", None)
}

struct ErrorHash {
    hash: Sha256,
    remaining: usize,
}

impl fmt::Write for ErrorHash {
    fn write_str(&mut self, text: &str) -> fmt::Result {
        let count = text.len().min(self.remaining);
        self.hash.update(&text.as_bytes()[..count]);
        self.remaining -= count;
        if count < text.len() {
            Err(fmt::Error)
        } else {
            Ok(())
        }
    }
}

fn error_id(key: &[u8; 32], error: &(dyn Error + 'static)) -> String {
    let mut hash = Sha256::new();
    hash.update(key);
    hash.update(b"ios diagnostic error v1\0");
    let mut sink = ErrorHash {
        hash,
        remaining: MAX_ERROR_HASH_BYTES,
    };
    let mut current = Some(error);
    for _ in 0..MAX_ERROR_SOURCES {
        let Some(error) = current else { break };
        if write!(&mut sink, "{error}\0").is_err() || sink.remaining == 0 {
            break;
        }
        current = error.source();
    }
    hex::encode(&sink.hash.finalize()[..8])
}

impl fmt::Debug for DiagnosticId<'_> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(self, formatter)
    }
}

fn encode_id(key: &[u8; 32], value: &str) -> String {
    let mut hash = Sha256::new();
    hash.update(key);
    hash.update(value.as_bytes());
    hex::encode(&hash.finalize()[..8])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn owned_error_debug_hides_parser_protocol_and_source_payloads() {
        let secret = "synthetic-private-credential-device-data";
        let errors: Vec<Box<dyn Error>> = vec![
            Box::new(crate::lockdown::LockdownError::Protocol(secret.into())),
            Box::new(crate::xpc::XpcError::Tls(secret.into())),
            Box::new(crate::tunnel::TunnelError::Protocol(secret.into())),
            Box::new(crate::proto::opack::OpackError::Encode(secret.into())),
            Box::new(crate::proto::nskeyedarchiver::ArchiveError::Invalid(
                secret.into(),
            )),
        ];
        for error in errors {
            assert!(error.to_string().contains(secret));
            let output = format!("{error:?} {error:#?}");
            assert!(!output.contains(secret));
            assert!(output.contains("diagnostic"));
            assert!(output.contains("error:"));
        }
    }

    #[cfg(not(feature = "tunnel"))]
    #[test]
    fn fallback_tunnel_info_debug_hides_addresses_without_tunnel_feature() {
        let secret = "synthetic-private-tunnel-address";
        let info = crate::tunnel::TunnelInfo {
            server_address: secret.into(),
            server_rsd_port: 1234,
            client_address: secret.into(),
            client_mtu: 1280,
        };
        let output = format!("{info:?} {info:#?}");
        assert!(!output.contains(secret));
        assert!(output.contains("id:"));
        assert!(output.contains("1280"));
    }

    #[test]
    fn correlation_is_consistent_without_raw_identifiers() {
        let secret = "synthetic-unique-device-identifier";
        let output = format!("{} {:?}", diagnostic_id(secret), diagnostic_id(secret));
        assert!(!output.contains(secret));
        assert_eq!(
            format!("{}", diagnostic_id(secret)),
            format!("{:?}", diagnostic_id(secret))
        );
    }

    #[test]
    fn correlation_changes_with_process_key_and_identifier() {
        assert_ne!(
            encode_id(&[1; 32], "device-a"),
            encode_id(&[2; 32], "device-a")
        );
        assert_ne!(
            encode_id(&[1; 32], "device-a"),
            encode_id(&[1; 32], "device-b")
        );
    }

    #[test]
    fn diagnostic_error_checks_original_and_safe_views_separately() {
        let secret = "synthetic-UDID-key-token-device-data";
        let error = std::io::Error::new(std::io::ErrorKind::PermissionDenied, secret);
        assert!(
            format!("{error} {error:?}").contains(secret),
            "the raw fixture must really expose the secret"
        );
        let output = format!(
            "{} {:?} {:#?}",
            diagnostic_error(&error),
            diagnostic_error(&error),
            diagnostic_error(&error)
        );
        assert!(!output.contains(secret));
        assert!(output.contains("io/PermissionDenied error:"));
        assert_eq!(
            diagnostic_error(&error).to_string(),
            format!("{:?}", diagnostic_error(&error))
        );
        assert_ne!(error_id(&[1; 32], &error), error_id(&[2; 32], &error));
    }

    #[test]
    fn diagnostic_error_bounds_source_cycles_and_formatted_bytes() {
        #[derive(Debug)]
        struct Cyclic;
        impl fmt::Display for Cyclic {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str("synthetic cyclic secret")
            }
        }
        impl Error for Cyclic {
            fn source(&self) -> Option<&(dyn Error + 'static)> {
                Some(self)
            }
        }
        assert!(diagnostic_error(&Cyclic)
            .to_string()
            .starts_with("operation error:"));
        let error = crate::CoreError::Protocol("synthetic secret".repeat(100_000));
        let output = diagnostic_error(&error).to_string();
        assert!(output.starts_with("protocol error:"));
        assert!(output.len() < 64);
        let mut sink = ErrorHash {
            hash: Sha256::new(),
            remaining: 3,
        };
        assert!(sink.write_str("世界").is_err());
        assert_eq!(sink.remaining, 0);
        assert!(sink.write_str("ignored").is_err());
    }
}
