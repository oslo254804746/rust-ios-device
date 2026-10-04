//! Shared CLI helpers.

use anyhow::Result;

/// Opt-in machine errors leave the existing result stream and human default
/// unchanged. The command parser owns selection and input validation.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, clap::ValueEnum)]
pub(crate) enum ErrorFormat {
    #[default]
    Human,
    Json,
}

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

const MAX_CONFIRMATION_SOURCES: usize = 32;

fn confirmation_required(error: &anyhow::Error) -> bool {
    error
        .chain()
        .take(MAX_CONFIRMATION_SOURCES)
        .any(|source| source.is::<ForceRequired>())
}

#[derive(serde::Serialize)]
struct MachineFailure<'a> {
    schema_version: u8,
    error: MachineFailureDetails<'a>,
}

#[derive(serde::Serialize)]
struct MachineFailureDetails<'a> {
    class: ios_core::ErrorClass,
    phase: &'a str,
    diagnostic: &'a str,
    exit_code: u8,
    confirmation_required: bool,
    effects: &'a str,
}

fn machine_failure(
    class: ios_core::ErrorClass,
    phase: &str,
    diagnostic: &str,
    exit_code: u8,
    confirmation_required: bool,
    effects: &str,
) -> std::io::Result<String> {
    serde_json::to_string(&MachineFailure {
        schema_version: 1,
        error: MachineFailureDetails {
            class,
            phase,
            diagnostic,
            exit_code,
            confirmation_required,
            effects,
        },
    })
    .map_err(|error| std::io::Error::new(std::io::ErrorKind::InvalidData, error))
}

/// The phase must be a fixed label or the parser's canonical command name.
/// Unknown effects means the failure alone cannot authorize an automatic retry.
pub(crate) fn operation_failure_message(
    format: ErrorFormat,
    phase: &str,
    error: &anyhow::Error,
) -> std::io::Result<String> {
    let diagnostic = error_message(error);
    if format == ErrorFormat::Human {
        return Ok(format!("{phase} failed: {diagnostic}"));
    }
    let confirmation_required = confirmation_required(error);
    let class = if confirmation_required {
        ios_core::ErrorClass::Precondition
    } else {
        ios_core::classify_error(error.as_ref())
    };
    machine_failure(
        class,
        phase,
        &diagnostic,
        1,
        confirmation_required,
        "unknown",
    )
}

pub(crate) fn argument_failure_message(
    format: ErrorFormat,
    error: &clap::Error,
) -> std::io::Result<String> {
    let diagnostic = argument_error_message(error);
    if format == ErrorFormat::Human {
        return Ok(diagnostic);
    }
    let class = if error.kind() == clap::error::ErrorKind::Io {
        // Preserve a recognized underlying IO category if Clap exposes it;
        // the typed parser kind is a generic IO fallback, never a text guess.
        match ios_core::classify_error(error) {
            ios_core::ErrorClass::Unknown => ios_core::ErrorClass::Io,
            class => class,
        }
    } else {
        ios_core::ErrorClass::InvalidInput
    };
    machine_failure(
        class,
        "argument_parsing",
        &diagnostic,
        2,
        false,
        "not_started",
    )
}

/// Stderr is best effort once an operation already failed; reporting failure
/// must not replace its original exit status or panic on a closed pipe.
pub(crate) fn report_failure(message: std::io::Result<String>) {
    if let Ok(message) = message {
        let _ = write_error_line(format_args!("{message}"));
    }
}

/// Format an operation error without printing arbitrary context/source text.
/// Callers supply a fixed phase; command dispatch uses the canonical clap name.
pub fn error_message(error: &anyhow::Error) -> String {
    let hint = if confirmation_required(error) {
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

    #[test]
    fn machine_failure_keeps_source_class_and_hides_context_and_details() {
        let secret = "synthetic-private-machine-error";
        let error = anyhow::Error::new(std::io::Error::new(
            std::io::ErrorKind::PermissionDenied,
            secret,
        ))
        .context(secret);
        let message = operation_failure_message(ErrorFormat::Json, "list", &error).unwrap();
        assert!(!message.contains(secret));
        assert!(!message.contains('\n'));
        let body: serde_json::Value = serde_json::from_str(&message).unwrap();
        assert_eq!(body["schema_version"], 1);
        assert_eq!(body["error"]["class"], "permission_denied");
        assert_eq!(body["error"]["phase"], "list");
        assert_eq!(body["error"]["exit_code"], 1);
        assert_eq!(body["error"]["effects"], "unknown");
        assert_eq!(body["error"]["confirmation_required"], false);
        assert!(error.chain().any(|source| {
            source
                .downcast_ref::<std::io::Error>()
                .is_some_and(|source| {
                    source.kind() == std::io::ErrorKind::PermissionDenied
                        && source.to_string() == secret
                })
        }));
        assert_eq!(
            operation_failure_message(ErrorFormat::Human, "list", &error).unwrap(),
            format!("list failed: {}", error_message(&error))
        );
    }

    #[test]
    fn machine_failure_keeps_opaque_errors_unknown_and_force_typed() {
        let opaque = anyhow::anyhow!("permission denied timeout cancelled");
        let body: serde_json::Value = serde_json::from_str(
            &operation_failure_message(ErrorFormat::Json, "dispatch", &opaque).unwrap(),
        )
        .unwrap();
        assert_eq!(body["error"]["class"], "unknown");
        let force = require_force(false, "private-operation", "private-consequence").unwrap_err();
        let message = operation_failure_message(ErrorFormat::Json, "dispatch", &force).unwrap();
        assert!(!message.contains("private-"));
        let body: serde_json::Value = serde_json::from_str(&message).unwrap();
        assert_eq!(body["error"]["class"], "precondition");
        assert_eq!(body["error"]["confirmation_required"], true);
    }

    #[test]
    fn machine_argument_failure_is_typed_before_dispatch_without_private_values() {
        let secret = "synthetic-private-invalid-port";
        let error = crate::Cli::try_parse_from(["ios", "forward", secret, "62078"])
            .err()
            .unwrap();
        let message = argument_failure_message(ErrorFormat::Json, &error).unwrap();
        assert!(!message.contains(secret));
        let body: serde_json::Value = serde_json::from_str(&message).unwrap();
        assert_eq!(body["error"]["class"], "invalid_input");
        assert_eq!(body["error"]["phase"], "argument_parsing");
        assert_eq!(body["error"]["exit_code"], 2);
        assert_eq!(body["error"]["effects"], "not_started");
        assert_eq!(
            argument_failure_message(ErrorFormat::Human, &error).unwrap(),
            argument_error_message(&error)
        );
    }

    #[test]
    fn final_failure_bounds_cyclic_confirmation_sources_without_secret_output() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        use std::sync::Arc;
        #[derive(Debug)]
        struct Cyclic(Arc<AtomicUsize>);
        impl std::fmt::Display for Cyclic {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                f.write_str("synthetic-private-cyclic-error")
            }
        }
        impl std::error::Error for Cyclic {
            fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
                // The safety valve makes the regression fail instead of hanging
                // if a frontend accidentally reintroduces an unlimited scan.
                (self.0.fetch_add(1, Ordering::SeqCst) < 512).then_some(self)
            }
        }
        let calls = Arc::new(AtomicUsize::new(0));
        let error = anyhow::Error::new(Cyclic(calls.clone()));
        calls.store(0, Ordering::SeqCst);
        let message = operation_failure_message(ErrorFormat::Json, "dispatch", &error).unwrap();
        assert!(calls.load(Ordering::SeqCst) <= 128);
        assert!(!message.contains("synthetic-private-cyclic-error"));
        let body: serde_json::Value = serde_json::from_str(&message).unwrap();
        assert_eq!(body["error"]["class"], "unknown");
        assert_eq!(body["error"]["confirmation_required"], false);
    }

    #[test]
    fn confirmation_hints_and_machine_fields_observe_the_same_bounded_sources() {
        for (depth, observed) in [(24, true), (40, false)] {
            let mut error =
                require_force(false, "synthetic-private-op", "synthetic-private-effect")
                    .unwrap_err();
            for _ in 0..depth {
                error = error.context("synthetic-private-context");
            }
            assert_eq!(
                error_message(&error).contains("--force is required"),
                observed
            );
            let body: serde_json::Value = serde_json::from_str(
                &operation_failure_message(ErrorFormat::Json, "dispatch", &error).unwrap(),
            )
            .unwrap();
            assert_eq!(body["error"]["confirmation_required"], observed);
            assert_eq!(
                body["error"]["class"],
                if observed { "precondition" } else { "unknown" }
            );
        }
    }
}

/// Write one result line, including its terminating newline, without panicking
/// on a closed or failed output stream.
pub(crate) fn write_line(arguments: std::fmt::Arguments<'_>) -> std::io::Result<()> {
    let stdout = std::io::stdout();
    write_line_to(&mut stdout.lock(), arguments)
}

pub(crate) fn write_line_to(
    writer: &mut impl std::io::Write,
    arguments: std::fmt::Arguments<'_>,
) -> std::io::Result<()> {
    writer.write_fmt(arguments)?;
    writer.write_all(b"\n")
}

/// Error reporting uses stderr and never turns an existing failure into a panic.
pub(crate) fn write_error_line(arguments: std::fmt::Arguments<'_>) -> std::io::Result<()> {
    let stderr = std::io::stderr();
    let mut writer = stderr.lock();
    write_error_line_to(&mut writer, arguments)
}

pub(crate) fn write_error_line_to(
    writer: &mut impl std::io::Write,
    arguments: std::fmt::Arguments<'_>,
) -> std::io::Result<()> {
    write_line_to(writer, arguments)?;
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
