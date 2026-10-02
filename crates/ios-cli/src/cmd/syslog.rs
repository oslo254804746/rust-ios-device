use std::time::Duration;

use anyhow::Result;
use regex::Regex;
use tokio_stream::StreamExt;

#[derive(clap::Args)]
pub struct SyslogCmd {
    #[arg(long, help = "Only show lines containing this string")]
    filter: Option<String>,
    #[arg(
        long = "regex",
        help = "Only show lines matching this regex; repeat to accept any matching expression"
    )]
    regex: Vec<String>,
    #[arg(
        long = "insensitive-regex",
        help = "Only show lines matching this regex case-insensitively; repeat to accept any matching expression"
    )]
    insensitive_regex: Vec<String>,
    #[arg(short = 'p', long, help = "Only show logs from this process")]
    process: Option<String>,
    #[arg(long, help = "Only show logs from this PID")]
    pid: Option<u32>,
    #[arg(long, help = "Print parsed log fields instead of the raw line")]
    parse: bool,
    #[arg(long, help = "Stop after receiving this many matching log lines")]
    count: Option<u64>,
    #[arg(long, help = "Overall timeout in seconds")]
    timeout: Option<u64>,
}

impl SyslogCmd {
    fn matches_filters(&self, entry: &ios_core::syslog::LogEntry) -> bool {
        if let Some(f) = &self.filter {
            if !entry.raw.contains(f.as_str()) {
                return false;
            }
        }
        if !matches_any_regex(&entry.raw, &self.regex, false) {
            return false;
        }
        if !matches_any_regex(&entry.raw, &self.insensitive_regex, true) {
            return false;
        }
        if let Some(proc_filter) = &self.process {
            let matches_process = entry.process.as_deref().is_some_and(|process| {
                process == proc_filter || Self::base_process_name(process) == proc_filter
            });
            if !matches_process {
                return false;
            }
        }
        if let Some(pid) = self.pid {
            if entry.pid != Some(pid) {
                return false;
            }
        }
        true
    }

    /// Syslog annotates os_log senders with the emitting library, for example
    /// `SpringBoard(FrontBoard)[52]`. Keep the annotation available for exact
    /// matches, while allowing `--process SpringBoard` to match that common form.
    fn base_process_name(process: &str) -> &str {
        if let Some(open) = process.find('(') {
            if open > 0 && process.ends_with(')') {
                return &process[..open];
            }
        }
        process
    }

    pub async fn run(self, udid: Option<String>, json_output: bool) -> Result<()> {
        let udid = udid.ok_or_else(|| anyhow::anyhow!("--udid required for syslog"))?;

        let opts = ios_core::device::ConnectOptions {
            tun_mode: ios_core::TunMode::Userspace,
            pair_record_path: None,
            skip_tunnel: true,
        };
        let device = ios_core::connect(&udid, opts).await?;
        let stream = device
            .connect_service(ios_core::syslog::SERVICE_NAME)
            .await?;

        crate::output::write_error_line(format_args!("Streaming syslog (Ctrl+C to stop)..."))?;

        let log_stream = ios_core::syslog::into_stream(stream);
        self.consume_logs(log_stream, json_output, |record| {
            crate::output::write_formatted(format_args!("{record}"))
        })
        .await
    }

    async fn consume_logs<S, F>(&self, log_stream: S, json_output: bool, mut emit: F) -> Result<()>
    where
        S: tokio_stream::Stream<Item = std::io::Result<String>>,
        F: FnMut(&str) -> std::io::Result<()>,
    {
        tokio::pin!(log_stream);

        let deadline = self
            .timeout
            .map(|secs| tokio::time::Instant::now() + Duration::from_secs(secs));
        let mut received = 0u64;

        loop {
            let result = match deadline {
                Some(deadline) => {
                    let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
                    if remaining.is_zero() {
                        break;
                    }
                    match tokio::time::timeout(remaining, log_stream.next()).await {
                        Ok(Some(result)) => result,
                        Ok(None) => break,
                        Err(_) => break,
                    }
                }
                None => match log_stream.next().await {
                    Some(result) => result,
                    None => break,
                },
            };

            let entry = ios_core::syslog::LogEntry::parse(result?);
            if !self.matches_filters(&entry) {
                continue;
            }
            emit(&render_log_entry(&entry, json_output, self.parse)?)?;
            received += 1;
            if self.count.is_some_and(|count| received >= count) {
                break;
            }
        }
        Ok(())
    }
}

fn render_log_entry(
    entry: &ios_core::syslog::LogEntry,
    json_output: bool,
    parse: bool,
) -> Result<String> {
    if json_output {
        let mut record = serde_json::to_string(&log_entry_to_json(entry))?;
        record.push('\n');
        Ok(record)
    } else if parse {
        Ok(format!("{}\n", format_parsed_entry(entry)))
    } else {
        Ok(entry.raw.clone())
    }
}

fn matches_any_regex(line: &str, patterns: &[String], case_insensitive: bool) -> bool {
    if patterns.is_empty() {
        return true;
    }

    patterns.iter().any(|pattern| {
        let pattern = if case_insensitive {
            format!("(?i){pattern}")
        } else {
            pattern.clone()
        };
        Regex::new(&pattern)
            .map(|regex| regex.is_match(line))
            .unwrap_or(false)
    })
}

fn log_entry_to_json(entry: &ios_core::syslog::LogEntry) -> serde_json::Value {
    serde_json::json!({
        "raw": entry.raw,
        "timestamp": entry.timestamp,
        "device": entry.device,
        "process": entry.process,
        "pid": entry.pid,
        "level": entry.level,
        "message": entry.message,
        "parse_success": entry.parse_success,
        "parse_error": entry.parse_error,
    })
}

fn format_parsed_entry(entry: &ios_core::syslog::LogEntry) -> String {
    if !entry.parse_success {
        if let Some(error) = &entry.parse_error {
            return format!("parse_failed({error}): {}", entry.raw);
        }
        return format!("parse_failed: {}", entry.raw);
    }

    let mut prefix_parts = Vec::new();
    if let Some(timestamp) = &entry.timestamp {
        prefix_parts.push(timestamp.clone());
    }
    if let Some(device) = &entry.device {
        prefix_parts.push(device.clone());
    }

    let mut proc_part = entry
        .process
        .clone()
        .unwrap_or_else(|| "unknown".to_string());
    if let Some(pid) = entry.pid {
        proc_part.push('[');
        proc_part.push_str(&pid.to_string());
        proc_part.push(']');
    }
    prefix_parts.push(proc_part);
    let prefix = prefix_parts.join(" ");

    match (entry.level.as_deref(), entry.message.as_deref()) {
        (Some(level), Some(message)) => format!("{prefix} {level}: {message}"),
        (Some(level), None) => format!("{prefix} {level}: {}", entry.raw),
        (None, Some(message)) => format!("{prefix}: {message}"),
        (None, None) => entry.raw.clone(),
    }
}

#[cfg(test)]
mod tests {
    use clap::Parser;
    use std::io::{self, Write};
    use std::pin::Pin;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;
    use std::task::{Context, Poll};
    use tokio::io::{AsyncRead, AsyncReadExt, ReadBuf};

    use super::*;

    #[derive(Parser)]
    struct TestCli {
        #[command(flatten)]
        command: SyslogCmd,
    }

    struct OwnedReader {
        input: Vec<u8>,
        offset: usize,
        terminal_error: Option<io::ErrorKind>,
        drops: Arc<AtomicUsize>,
    }

    impl OwnedReader {
        fn new(input: Vec<u8>, terminal_error: Option<io::ErrorKind>) -> Self {
            Self {
                input,
                offset: 0,
                terminal_error,
                drops: Arc::new(AtomicUsize::new(0)),
            }
        }
    }

    impl AsyncRead for OwnedReader {
        fn poll_read(
            mut self: Pin<&mut Self>,
            _cx: &mut Context<'_>,
            buf: &mut ReadBuf<'_>,
        ) -> Poll<io::Result<()>> {
            if self.offset == self.input.len() {
                return Poll::Ready(match self.terminal_error {
                    Some(kind) => Err(io::Error::new(kind, "synthetic-private-syslog-source")),
                    None => Ok(()),
                });
            }
            let end = (self.offset + buf.remaining()).min(self.input.len());
            buf.put_slice(&self.input[self.offset..end]);
            self.offset = end;
            Poll::Ready(Ok(()))
        }
    }

    impl Drop for OwnedReader {
        fn drop(&mut self) {
            self.drops.fetch_add(1, Ordering::SeqCst);
        }
    }

    fn command(args: &[&str]) -> SyslogCmd {
        TestCli::parse_from(std::iter::once("syslog").chain(args.iter().copied())).command
    }

    fn assert_failure(error: &anyhow::Error, kind: io::ErrorKind, class: ios_core::ErrorClass) {
        let source = error
            .chain()
            .find_map(|source| source.downcast_ref::<io::Error>())
            .unwrap();
        assert_eq!(source.kind(), kind);
        assert_eq!(ios_core::classify_error(error.as_ref()), class);
        let message = crate::output::operation_failure_message(
            crate::output::ErrorFormat::Json,
            "syslog",
            error,
        )
        .unwrap();
        assert!(!message.contains("synthetic-private-syslog-source"));
        let body: serde_json::Value = serde_json::from_str(&message).unwrap();
        assert_eq!(body["error"]["class"], class.as_str());
        assert_eq!(body["error"]["phase"], "syslog");
        assert_eq!(body["error"]["exit_code"], 1);
        assert_eq!(body["error"]["effects"], "unknown");
    }

    #[tokio::test]
    async fn partial_json_stream_retains_native_failure_and_complete_records() {
        for (kind, class) in [
            (io::ErrorKind::TimedOut, ios_core::ErrorClass::Timeout),
            (
                io::ErrorKind::PermissionDenied,
                ios_core::ErrorClass::PermissionDenied,
            ),
            (
                io::ErrorKind::ConnectionReset,
                ios_core::ErrorClass::Disconnected,
            ),
            (io::ErrorKind::InvalidData, ios_core::ErrorClass::Protocol),
        ] {
            let first = "Mar 17 12:34:56 iPhone SpringBoard[58] <Notice>: first\n";
            let second =
                "Mar 17 12:34:57 iPhone SpringBoard[58] <Notice>: quoted \"内容\"\ncontinued\n";
            let reader = OwnedReader::new(format!("{first}\0{second}\0").into_bytes(), Some(kind));
            let drops = Arc::clone(&reader.drops);
            let mut output = String::new();
            let error = command(&[])
                .consume_logs(ios_core::syslog::into_stream(reader), true, |record| {
                    output.push_str(record);
                    Ok(())
                })
                .await
                .unwrap_err();
            assert_failure(&error, kind, class);
            assert_eq!(
                error.downcast_ref::<io::Error>().unwrap().to_string(),
                "synthetic-private-syslog-source"
            );
            assert!(output.ends_with('\n'));
            let records: Vec<serde_json::Value> = output
                .lines()
                .map(|line| serde_json::from_str(line).unwrap())
                .collect();
            assert_eq!(records.len(), 2);
            assert_eq!(records[0]["raw"], first);
            assert_eq!(records[1]["raw"], second);
            assert_eq!(drops.load(Ordering::SeqCst), 1);
        }
    }

    #[tokio::test]
    async fn relay_eof_fails_without_emitting_unterminated_record() {
        let line = "Mar 17 12:34:56 iPhone SpringBoard[58] <Notice>: complete";
        let reader = OwnedReader::new(format!("{line}\0unfinished record").into_bytes(), None);
        let drops = Arc::clone(&reader.drops);
        let mut output = String::new();
        let error = command(&[])
            .consume_logs(ios_core::syslog::into_stream(reader), true, |record| {
                output.push_str(record);
                Ok(())
            })
            .await
            .unwrap_err();
        assert_failure(
            &error,
            io::ErrorKind::UnexpectedEof,
            ios_core::ErrorClass::Io,
        );
        assert_eq!(output.lines().count(), 1);
        let record: serde_json::Value = serde_json::from_str(output.trim_end()).unwrap();
        assert_eq!(record["raw"], line);
        assert!(!output.contains("unfinished record"));
        assert_eq!(drops.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn relay_message_limit_fails_after_prior_ndjson_record() {
        let mut wire = b"complete\0".to_vec();
        wire.resize(wire.len() + 1024 * 1024 + 1, b'x');
        wire.push(0);
        let reader = OwnedReader::new(wire, None);
        let drops = Arc::clone(&reader.drops);
        let mut output = String::new();
        let error = command(&[])
            .consume_logs(ios_core::syslog::into_stream(reader), true, |record| {
                output.push_str(record);
                Ok(())
            })
            .await
            .unwrap_err();
        assert_failure(
            &error,
            io::ErrorKind::InvalidData,
            ios_core::ErrorClass::Protocol,
        );
        let records: Vec<serde_json::Value> = output
            .lines()
            .map(|line| serde_json::from_str(line).unwrap())
            .collect();
        assert_eq!(records.len(), 1);
        assert_eq!(records[0]["raw"], "complete");
        assert_eq!(drops.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn matching_count_is_success_and_drops_reader_before_terminal_failure() {
        let reader = OwnedReader::new(
            b"discarded\0selected first\0selected second\0".to_vec(),
            Some(io::ErrorKind::TimedOut),
        );
        let drops = Arc::clone(&reader.drops);
        let mut output = String::new();
        command(&["--filter", "selected", "--count", "2"])
            .consume_logs(ios_core::syslog::into_stream(reader), true, |record| {
                output.push_str(record);
                Ok(())
            })
            .await
            .unwrap();
        let records: Vec<serde_json::Value> = output
            .lines()
            .map(|line| serde_json::from_str(line).unwrap())
            .collect();
        assert_eq!(records.len(), 2);
        assert_eq!(records[0]["raw"], "selected first");
        assert_eq!(records[1]["raw"], "selected second");
        assert_eq!(drops.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn selected_capture_timeout_is_success_and_closes_owned_stream() {
        let (reader, mut peer) = tokio::io::duplex(64);
        let mut emitted = 0;
        tokio::time::timeout(
            Duration::from_secs(3),
            command(&["--timeout", "1"]).consume_logs(
                ios_core::syslog::into_stream(reader),
                true,
                |_| {
                    emitted += 1;
                    Ok(())
                },
            ),
        )
        .await
        .unwrap()
        .unwrap();
        assert_eq!(emitted, 0);
        assert_eq!(peer.read(&mut [0; 1]).await.unwrap(), 0);
    }

    struct FailingOutput {
        bytes: Vec<u8>,
        remaining: usize,
        fail_flush: bool,
    }

    impl Write for FailingOutput {
        fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
            if self.remaining == 0 {
                return Err(io::Error::new(
                    io::ErrorKind::BrokenPipe,
                    "synthetic-private-syslog-source",
                ));
            }
            let count = self.remaining.min(bytes.len());
            self.bytes.extend_from_slice(&bytes[..count]);
            self.remaining -= count;
            Ok(count)
        }
        fn flush(&mut self) -> io::Result<()> {
            if self.fail_flush {
                Err(io::Error::new(
                    io::ErrorKind::BrokenPipe,
                    "synthetic-private-syslog-source",
                ))
            } else {
                Ok(())
            }
        }
    }

    #[tokio::test]
    async fn partial_write_or_flush_failure_stops_stream_with_original_io() {
        for fail_flush in [false, true] {
            let reader = OwnedReader::new(b"first\0second\0".to_vec(), None);
            let drops = Arc::clone(&reader.drops);
            let mut output = FailingOutput {
                bytes: Vec::new(),
                remaining: if fail_flush { usize::MAX } else { 7 },
                fail_flush,
            };
            let mut emitted = 0;
            let error = command(&[])
                .consume_logs(ios_core::syslog::into_stream(reader), true, |record| {
                    emitted += 1;
                    output.write_all(record.as_bytes())?;
                    output.flush()
                })
                .await
                .unwrap_err();
            assert_failure(
                &error,
                io::ErrorKind::BrokenPipe,
                ios_core::ErrorClass::Disconnected,
            );
            let first = render_log_entry(
                &ios_core::syslog::LogEntry::parse("first".into()),
                true,
                false,
            )
            .unwrap();
            assert_eq!(
                output.bytes,
                if fail_flush {
                    first.as_bytes()
                } else {
                    &first.as_bytes()[..7]
                }
            );
            assert_eq!(emitted, 1);
            assert_eq!(drops.load(Ordering::SeqCst), 1);
        }
    }

    #[tokio::test]
    async fn human_records_preserve_raw_and_parsed_bytes() {
        let raw = "Mar 17 12:34:56 iPhone SpringBoard[58] <Notice>: ready\n";
        for parse in [false, true] {
            let reader = OwnedReader::new(format!("{raw}\0").into_bytes(), None);
            let mut output = String::new();
            let cmd = command(if parse {
                &["--parse", "--count", "1"]
            } else {
                &["--count", "1"]
            });
            cmd.consume_logs(ios_core::syslog::into_stream(reader), false, |record| {
                output.push_str(record);
                Ok(())
            })
            .await
            .unwrap();
            assert_eq!(
                output,
                if parse {
                    "Mar 17 12:34:56 iPhone SpringBoard[58] Notice: ready\n\n"
                } else {
                    raw
                }
            );
        }
    }

    #[test]
    fn parses_syslog_count_and_timeout_flags() {
        let cmd = TestCli::parse_from([
            "syslog",
            "--process",
            "SpringBoard",
            "--regex",
            "lock",
            "--insensitive-regex",
            "NOTICE",
            "--pid",
            "58",
            "--parse",
            "--filter",
            "lock",
            "--count",
            "3",
            "--timeout",
            "10",
        ]);

        assert_eq!(cmd.command.process.as_deref(), Some("SpringBoard"));
        assert_eq!(cmd.command.regex, vec!["lock".to_string()]);
        assert_eq!(cmd.command.insensitive_regex, vec!["NOTICE".to_string()]);
        assert_eq!(cmd.command.pid, Some(58));
        assert!(cmd.command.parse);
        assert_eq!(cmd.command.filter.as_deref(), Some("lock"));
        assert_eq!(cmd.command.count, Some(3));
        assert_eq!(cmd.command.timeout, Some(10));
    }

    #[test]
    fn pid_filter_matches_only_requested_pid() {
        let cmd = SyslogCmd {
            filter: None,
            regex: Vec::new(),
            insensitive_regex: Vec::new(),
            process: None,
            pid: Some(58),
            parse: false,
            count: Some(1),
            timeout: Some(5),
        };
        let matching = ios_core::syslog::LogEntry::parse(
            "Mar 17 12:34:56 iPhone SpringBoard[58] <Notice>: lock".to_string(),
        );
        let other = ios_core::syslog::LogEntry::parse(
            "Mar 17 12:34:56 iPhone SpringBoard[99] <Notice>: lock".to_string(),
        );

        assert!(cmd.matches_filters(&matching));
        assert!(!cmd.matches_filters(&other));
    }

    #[test]
    fn process_filter_matches_os_log_library_annotation() {
        let cmd = SyslogCmd {
            filter: None,
            regex: Vec::new(),
            insensitive_regex: Vec::new(),
            process: Some("SpringBoard".into()),
            pid: None,
            parse: false,
            count: Some(1),
            timeout: Some(5),
        };
        let annotated = ios_core::syslog::LogEntry::parse(
            "Mar 17 12:34:56 iPhone SpringBoard(FrontBoard)[52] <Notice>: ready".into(),
        );
        let different = ios_core::syslog::LogEntry::parse(
            "Mar 17 12:34:56 iPhone SpringBoardExtension(Foundation)[52] <Notice>: ready".into(),
        );

        assert!(cmd.matches_filters(&annotated));
        assert!(!cmd.matches_filters(&different));
    }

    #[test]
    fn process_filter_accepts_full_annotated_name() {
        let cmd = SyslogCmd {
            filter: None,
            regex: Vec::new(),
            insensitive_regex: Vec::new(),
            process: Some("SpringBoard(FrontBoard)".into()),
            pid: None,
            parse: false,
            count: Some(1),
            timeout: Some(5),
        };
        let annotated = ios_core::syslog::LogEntry::parse(
            "Mar 17 12:34:56 iPhone SpringBoard(FrontBoard)[52] <Notice>: ready".into(),
        );

        assert!(cmd.matches_filters(&annotated));
    }

    #[test]
    fn regex_filter_matches_when_any_pattern_matches() {
        let cmd = SyslogCmd {
            filter: None,
            regex: vec!["daemon".into(), "worker".into()],
            insensitive_regex: Vec::new(),
            process: None,
            pid: None,
            parse: false,
            count: Some(1),
            timeout: Some(5),
        };
        let entry = ios_core::syslog::LogEntry::parse(
            "Mar 17 12:34:56 iPhone SpringBoard[58] <Notice>: worker ready".to_string(),
        );

        assert!(cmd.matches_filters(&entry));
    }

    #[test]
    fn insensitive_regex_filter_matches_case_insensitively() {
        let cmd = SyslogCmd {
            filter: None,
            regex: Vec::new(),
            insensitive_regex: vec!["READY".into()],
            process: None,
            pid: None,
            parse: false,
            count: Some(1),
            timeout: Some(5),
        };
        let entry = ios_core::syslog::LogEntry::parse(
            "Mar 17 12:34:56 iPhone SpringBoard[58] <Notice>: worker ready".to_string(),
        );

        assert!(cmd.matches_filters(&entry));
    }

    #[test]
    fn invalid_regex_does_not_match_lines() {
        let cmd = SyslogCmd {
            filter: None,
            regex: vec!["(".into()],
            insensitive_regex: Vec::new(),
            process: None,
            pid: None,
            parse: false,
            count: Some(1),
            timeout: Some(5),
        };
        let entry = ios_core::syslog::LogEntry::parse(
            "Mar 17 12:34:56 iPhone SpringBoard[58] <Notice>: worker ready".to_string(),
        );

        assert!(!cmd.matches_filters(&entry));
    }

    #[test]
    fn log_entry_to_json_preserves_parsed_fields() {
        let entry = ios_core::syslog::LogEntry::parse(
            "Mar 17 12:34:56 iPhone SpringBoard[58] <Notice>: lock".to_string(),
        );

        let value = log_entry_to_json(&entry);
        assert_eq!(value["timestamp"], "Mar 17 12:34:56");
        assert_eq!(value["device"], "iPhone");
        assert_eq!(value["process"], "SpringBoard");
        assert_eq!(value["pid"], 58);
        assert_eq!(value["level"], "Notice");
        assert_eq!(value["message"], "lock");
        assert_eq!(value["parse_success"], true);
        assert!(value["parse_error"].is_null());
        assert!(value["raw"].as_str().unwrap().contains("SpringBoard[58]"));
    }

    #[test]
    fn format_parsed_entry_prefers_structured_fields() {
        let entry = ios_core::syslog::LogEntry::parse(
            "Mar 17 12:34:56 iPhone SpringBoard[58] <Notice>: lock".to_string(),
        );

        assert_eq!(
            format_parsed_entry(&entry),
            "Mar 17 12:34:56 iPhone SpringBoard[58] Notice: lock"
        );
    }

    #[test]
    fn format_parsed_entry_reports_parse_failures() {
        let entry =
            ios_core::syslog::LogEntry::parse("totally unstructured syslog payload".to_string());

        let rendered = format_parsed_entry(&entry);
        assert!(rendered.contains("parse_failed"));
        assert!(rendered.contains("totally unstructured syslog payload"));
    }
}
