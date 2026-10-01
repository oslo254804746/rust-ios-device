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

/// Write one result line, including its terminating newline, without panicking
/// on a closed or failed output stream.
pub(crate) fn write_line(arguments: std::fmt::Arguments<'_>) -> std::io::Result<()> {
    let stdout = std::io::stdout();
    write_line_to(&mut stdout.lock(), arguments)
}

fn write_line_to(
    writer: &mut impl std::io::Write,
    arguments: std::fmt::Arguments<'_>,
) -> std::io::Result<()> {
    writer.write_fmt(arguments)?;
    writer.write_all(b"\n")
}

/// Error reporting uses stderr and never turns an existing failure into a panic.
pub(crate) fn write_error_line(arguments: std::fmt::Arguments<'_>) -> std::io::Result<()> {
    use std::io::Write;
    let stderr = std::io::stderr();
    let mut writer = stderr.lock();
    write_line_to(&mut writer, arguments)?;
    writer.flush()
}

/// Prompts and other output without a newline explicitly flush too, so an
/// error cannot be deferred to an unobserved global stdout destructor.
pub(crate) fn write_formatted(arguments: std::fmt::Arguments<'_>) -> std::io::Result<()> {
    let stdout = std::io::stdout();
    write_formatted_to(&mut stdout.lock(), arguments)
}

fn write_formatted_to(
    writer: &mut impl std::io::Write,
    arguments: std::fmt::Arguments<'_>,
) -> std::io::Result<()> {
    writer.write_fmt(arguments)?;
    writer.flush()
}

/// Preserve the historical XML bytes and extra newline, and report flush errors.
pub(crate) fn write_plist_xml(value: &plist::Value) -> Result<()> {
    let stdout = std::io::stdout();
    write_plist_xml_to(&mut stdout.lock(), value)
}

fn write_plist_xml_to(writer: &mut impl std::io::Write, value: &plist::Value) -> Result<()> {
    plist::to_writer_xml(&mut *writer, value)?;
    writer.write_all(b"\n")?;
    writer.flush()?;
    Ok(())
}

macro_rules! outln {
    () => { $crate::output::write_line(format_args!(""))? };
    ($($argument:tt)*) => { $crate::output::write_line(format_args!($($argument)*))? };
}
pub(crate) use outln;

macro_rules! out {
    ($($argument:tt)*) => { $crate::output::write_formatted(format_args!($($argument)*))? };
}
pub(crate) use out;

#[cfg(test)]
mod result_writer_tests {
    use super::*;
    use std::io::{self, Write};

    struct FailingOutput {
        bytes: Vec<u8>,
        remaining: usize,
    }

    impl Write for FailingOutput {
        fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
            if self.remaining == 0 {
                return Err(io::Error::new(
                    io::ErrorKind::BrokenPipe,
                    "synthetic output failure",
                ));
            }
            let count = bytes.len().min(self.remaining);
            self.bytes.extend_from_slice(&bytes[..count]);
            self.remaining -= count;
            Ok(count)
        }

        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    #[test]
    fn partial_result_failure_returns_io_error_without_success_or_panic() {
        let mut output = FailingOutput {
            bytes: Vec::new(),
            remaining: 2,
        };
        let error = write_line_to(&mut output, format_args!("{}", "result"))
            .expect_err("partial output must fail");
        assert_eq!(error.kind(), io::ErrorKind::BrokenPipe);
        assert_eq!(output.bytes, b"re");
    }

    #[test]
    fn trailing_newline_failure_is_part_of_the_result_contract() {
        let mut output = FailingOutput {
            bytes: Vec::new(),
            remaining: 6,
        };
        let error = write_line_to(&mut output, format_args!("{}", "result"))
            .expect_err("missing final newline must fail");
        assert_eq!(error.kind(), io::ErrorKind::BrokenPipe);
        assert_eq!(output.bytes, b"result");
        let mut complete = Vec::new();
        write_line_to(&mut complete, format_args!("{:04x}", 42)).unwrap();
        assert_eq!(complete, b"002a\n");
    }

    #[test]
    fn output_without_newline_reports_flush_failure() {
        struct FlushFailure(Vec<u8>);
        impl Write for FlushFailure {
            fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
                self.0.extend_from_slice(bytes);
                Ok(bytes.len())
            }
            fn flush(&mut self) -> io::Result<()> {
                Err(io::Error::new(
                    io::ErrorKind::BrokenPipe,
                    "synthetic flush failure",
                ))
            }
        }
        let mut output = FlushFailure(Vec::new());
        let error = write_formatted_to(&mut output, format_args!("prompt> ")).unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::BrokenPipe);
        assert_eq!(output.0, b"prompt> ");
    }

    #[test]
    fn xml_result_preserves_bytes_and_reports_partial_write_source() {
        let value = plist::Value::String("requested & <data>".into());
        let expected = concat!(
            "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n",
            "<!DOCTYPE plist PUBLIC \"-//Apple//DTD PLIST 1.0//EN\" ",
            "\"http://www.apple.com/DTDs/PropertyList-1.0.dtd\">\n",
            "<plist version=\"1.0\">\n",
            "<string>requested &amp; &lt;data&gt;</string>\n</plist>\n"
        );
        let mut complete = Vec::new();
        write_plist_xml_to(&mut complete, &value).unwrap();
        assert_eq!(complete, expected.as_bytes());

        let mut partial = FailingOutput {
            bytes: Vec::new(),
            remaining: 2,
        };
        let error = write_plist_xml_to(&mut partial, &value).unwrap_err();
        let source = error
            .chain()
            .find_map(|source| source.downcast_ref::<io::Error>())
            .expect("XML serialization must retain the output IO source");
        assert_eq!(source.kind(), io::ErrorKind::BrokenPipe);
        assert_eq!(partial.bytes, b"<?");
    }
}
