//! Shared CLI helpers.

use anyhow::Result;

/// Refuse a destructive operation unless the caller opted in with `--force`.
///
/// Commands that reboot, overwrite device data, or install a device-wide
/// profile all route through this so the guard reads the same everywhere.
pub fn require_force(force: bool, operation: &str, consequence: &str) -> Result<()> {
    if force {
        return Ok(());
    }
    Err(anyhow::anyhow!(
        "refusing to {operation} without --force: {consequence}"
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

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
}
