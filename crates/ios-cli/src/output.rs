//! Shared CLI helpers.

use anyhow::Result;

struct ForceRequired {
    operation: String,
    consequence: String,
}

impl std::fmt::Display for ForceRequired {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "refusing to {} without --force: {}",
            self.operation, self.consequence
        )
    }
}

impl std::fmt::Debug for ForceRequired {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("ForceRequired { .. }")
    }
}

impl std::error::Error for ForceRequired {}

/// Format an operation error without printing arbitrary context/source text.
/// Callers supply a fixed phase; command dispatch uses the canonical clap name.
pub fn error_message(error: &anyhow::Error) -> String {
    let hint = if error.chain().any(|source| source.is::<ForceRequired>()) {
        "; --force is required to confirm this operation"
    } else {
        ""
    };
    format!(
        "{}{hint}",
        ios_core::redaction::diagnostic_error(error.as_ref())
    )
}

pub fn argument_error_message(error: &clap::Error) -> String {
    format!(
        "argument parsing failed ({:?}); {}. Use 'ios --help' or 'ios <command> --help'",
        error.kind(),
        ios_core::redaction::diagnostic_error(error)
    )
}

/// Refuse a destructive operation unless the caller opted in with `--force`.
///
/// Commands that reboot, overwrite device data, or install a device-wide
/// profile all route through this so the guard reads the same everywhere.
pub fn require_force(force: bool, operation: &str, consequence: &str) -> Result<()> {
    if force {
        return Ok(());
    }
    Err(ForceRequired {
        operation: operation.into(),
        consequence: consequence.into(),
    }
    .into())
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::Parser;

    #[test]
    fn passes_through_when_forced() {
        assert!(require_force(true, "erase device", "all data is lost").is_ok());
    }

    #[test]
    fn explains_what_the_flag_would_have_done() {
        let err = require_force(false, "erase device", "all data is lost").unwrap_err();
        let message = err.to_string();
        assert!(message.contains("--force"), "{message}");
        assert!(message.contains("erase device"), "{message}");
        assert!(message.contains("all data is lost"), "{message}");
    }

    #[test]
    fn diagnostic_boundary_hides_context_sources_and_preserves_force_hint() {
        let secret = "synthetic-private-key-UDID-device-data";
        let error = anyhow::Error::new(std::io::Error::new(
            std::io::ErrorKind::PermissionDenied,
            secret,
        ))
        .context(format!("https://user:{secret}@example.invalid/{secret}"));
        assert!(format!("{error:#} {error:?}").contains(secret));
        let message = error_message(&error);
        assert!(!message.contains(secret));
        assert!(!message.contains("https://"));
        assert!(message.contains("io/PermissionDenied error:"));
        let guard = require_force(false, secret, secret)
            .unwrap_err()
            .context(secret);
        assert!(format!("{guard:#}").contains(secret));
        let message = error_message(&guard);
        assert!(!message.contains(secret));
        assert!(message.contains("--force is required"));
    }

    #[test]
    fn argument_boundary_hides_invalid_values_but_retains_error_kind() {
        let secret = "synthetic-token-and-identifier";
        let error = crate::Cli::try_parse_from(["ios", "forward", secret, "62078"])
            .err()
            .expect("port must be invalid");
        assert!(error.to_string().contains(secret));
        let message = argument_error_message(&error);
        assert!(!message.contains(secret));
        assert!(message.contains("ValueValidation"));
        assert!(message.contains("--help"));
    }
}
