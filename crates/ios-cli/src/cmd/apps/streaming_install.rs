//! Checked stderr progress with the existing diagnostic projection/bytes.

pub(super) fn write_progress(percent: u32, status: &str) -> std::io::Result<()> {
    write_progress_to(&mut std::io::stderr().lock(), percent, status)
}

fn write_progress_to(
    writer: &mut impl std::io::Write,
    percent: u32,
    status: &str,
) -> std::io::Result<()> {
    crate::output::write_error_line_to(
        writer,
        format_args!(
            "  [{percent}%] status={}",
            ios_core::redaction::diagnostic_id(status)
        ),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{self, Write};

    #[tokio::test]
    async fn local_decoder_failure_keeps_actual_safe_cli_envelope() {
        struct LocalIpa(std::path::PathBuf);
        impl Drop for LocalIpa {
            fn drop(&mut self) {
                let _ = std::fs::remove_dir_all(&self.0);
            }
        }
        let dir =
            std::env::temp_dir().join(format!("ios-owned-private-zip-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir(&dir).unwrap();
        let owner = LocalIpa(dir.clone());
        let path = dir.join("owned-private.ipa");
        std::fs::write(&path, [0; 12]).unwrap();
        let (mut service, _peer) = tokio::io::duplex(16);
        let native = ios_core::apps::install_ipa_with_checked_progress(&mut service, &path, None)
            .await
            .unwrap_err();
        let error = anyhow::Error::new(native).context("streaming zip conduit install failed");
        let source = error
            .chain()
            .find_map(|cause| cause.downcast_ref::<io::Error>())
            .expect("native decoder IO wrapper lost");
        assert_eq!(source.kind(), io::ErrorKind::InvalidInput);
        assert!(std::error::Error::source(source).is_some());
        let message = crate::output::operation_failure_message(
            crate::output::ErrorFormat::Json,
            "apps",
            &error,
        )
        .unwrap();
        let json: serde_json::Value = serde_json::from_str(&message).unwrap();
        assert_eq!(json["error"]["class"], "invalid_input");
        assert_eq!(json["error"]["phase"], "apps");
        assert_eq!(json["error"]["exit_code"], 1);
        assert_eq!(json["error"]["effects"], "unknown");
        assert!(!message.contains("owned-private"));
        assert!(!message.contains(&path.to_string_lossy().to_string()));
        drop(owner);
        assert!(!dir.exists());
    }

    #[test]
    fn progress_preserves_bytes_and_safe_native_flush_failure() {
        struct FlushFailure(Vec<u8>);
        impl Write for FlushFailure {
            fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
                self.0.extend_from_slice(bytes);
                Ok(bytes.len())
            }
            fn flush(&mut self) -> io::Result<()> {
                Err(io::Error::new(
                    io::ErrorKind::BrokenPipe,
                    "owned-private-flush-source",
                ))
            }
        }
        let status = "owned-private-status\nwith quotes\"";
        let expected = format!(
            "  [42%] status={}\n",
            ios_core::redaction::diagnostic_id(status)
        );
        let mut success = Vec::new();
        write_progress_to(&mut success, 42, status).unwrap();
        assert_eq!(success, expected.as_bytes());
        assert!(!String::from_utf8_lossy(&success).contains(status));
        let mut failed = FlushFailure(Vec::new());
        let native = write_progress_to(&mut failed, 42, status).unwrap_err();
        assert_eq!(failed.0, expected.as_bytes());
        assert_eq!(native.kind(), io::ErrorKind::BrokenPipe);
        let error = anyhow::Error::new(ios_core::apps::ZipConduitError::Io(native))
            .context("streaming zip conduit install failed");
        let message = crate::output::operation_failure_message(
            crate::output::ErrorFormat::Json,
            "apps",
            &error,
        )
        .unwrap();
        let json: serde_json::Value = serde_json::from_str(&message).unwrap();
        assert_eq!(json["error"]["class"], "disconnected");
        assert_eq!(json["error"]["phase"], "apps");
        assert_eq!(json["error"]["exit_code"], 1);
        assert_eq!(json["error"]["effects"], "unknown");
        assert!(!message.contains("owned-private"));
        assert_eq!(
            error
                .chain()
                .find_map(|cause| cause.downcast_ref::<io::Error>())
                .unwrap()
                .to_string(),
            "owned-private-flush-source"
        );
    }

    #[test]
    fn progress_write_failure_preserves_partial_line_and_stops_before_flush() {
        struct Partial {
            bytes: Vec<u8>,
            flushed: bool,
        }
        impl Write for Partial {
            fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
                if self.bytes.len() == 3 {
                    return Err(io::Error::from(io::ErrorKind::PermissionDenied));
                }
                let count = bytes.len().min(3 - self.bytes.len());
                self.bytes.extend_from_slice(&bytes[..count]);
                Ok(count)
            }
            fn flush(&mut self) -> io::Result<()> {
                self.flushed = true;
                Ok(())
            }
        }
        let mut output = Partial {
            bytes: vec![],
            flushed: false,
        };
        let error = write_progress_to(&mut output, 20, "private-status").unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::PermissionDenied);
        assert_eq!(
            ios_core::classify_error(&error),
            ios_core::ErrorClass::PermissionDenied
        );
        assert_eq!(output.bytes, b"  [");
        assert!(!output.flushed);
    }
}
