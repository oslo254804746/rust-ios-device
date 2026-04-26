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
            if entry.process.as_deref() != Some(proc_filter.as_str()) {
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

    pub async fn run(self, udid: Option<String>, json_output: bool) -> Result<()> {
        let udid = udid.ok_or_else(|| anyhow::anyhow!("--udid required for syslog"))?;

        let opts = ios_core::device::ConnectOptions {
            tun_mode: ios_core::tunnel::TunMode::Userspace,
            pair_record_path: None,
            skip_tunnel: true,
        };
        let device = ios_core::connect(&udid, opts).await?;
        let stream = device
            .connect_service(ios_core::syslog::SERVICE_NAME)
            .await?;

        eprintln!("Streaming syslog (Ctrl+C to stop)...");

        let log_stream = ios_core::syslog::into_stream(stream);
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

            match result {
                Ok(line) => {
                    let entry = ios_core::syslog::LogEntry::parse(line.clone());
                    if !self.matches_filters(&entry) {
                        continue;
                    }
                    if json_output {
                        println!("{}", serde_json::to_string(&log_entry_to_json(&entry))?);
                    } else if self.parse {
                        println!("{}", format_parsed_entry(&entry));
                    } else {
                        print!("{line}");
                    }
                    received += 1;
                    if self.count.is_some_and(|count| received >= count) {
                        break;
                    }
                }
                Err(e) => {
                    eprintln!("syslog error: {e}");
                    break;
                }
            }
        }
        Ok(())
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

    use super::*;

    #[derive(Parser)]
    struct TestCli {
        #[command(flatten)]
        command: SyslogCmd,
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
