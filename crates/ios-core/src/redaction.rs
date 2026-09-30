//! Diagnostic identifiers are process-local pseudonyms, never raw device IDs.
//!
//! Explicit device-selection/API output retains its identifiers. Use this
//! wrapper in diagnostics only; it cannot sanitize arbitrary device payloads.

use std::fmt;
use std::sync::OnceLock;

use rand::RngCore;
use sha2::{Digest, Sha256};

static PROCESS_KEY: OnceLock<Option<[u8; 32]>> = OnceLock::new();

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
        let key = PROCESS_KEY.get_or_init(|| {
            let mut key = [0; 32];
            rand::rngs::OsRng.try_fill_bytes(&mut key).ok().map(|_| key)
        });
        match key {
            Some(key) => write!(formatter, "id:{}", encode_id(key, self.0)),
            None => formatter.write_str("id:redacted"),
        }
    }
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
}
