use std::collections::{HashMap, HashSet};
use std::time::{Duration, Instant};

use serde::Serialize;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite};
use tokio_stream::Stream;

use crate::services::afc::{AfcClient, AfcError, AfcFileInfo, AfcStatusCode};

pub const CRASHREPORT_MOVER_SERVICE: &str = "com.apple.crashreportmover";
pub const CRASHREPORT_COPY_MOBILE_SERVICE: &str = "com.apple.crashreportcopymobile";
pub const RSD_CRASHREPORT_MOVER_SERVICE: &str = "com.apple.crashreportmover.shim.remote";
pub const RSD_CRASHREPORT_COPY_MOBILE_SERVICE: &str = "com.apple.crashreportcopymobile.shim.remote";

/// Maximum report size accepted by the structured parser.  Crash reports are
/// diagnostic input, so a corrupt device entry must not turn a CLI invocation
/// into an unbounded allocation.
pub const MAX_CRASH_REPORT_BYTES: usize = 16 * 1024 * 1024;
const MAX_CRASH_REPORT_LINES: usize = 200_000;
const MAX_CRASH_REPORT_LINE_BYTES: usize = 1024 * 1024;
const MAX_WATCH_PARSE_RETRIES: u8 = 3;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CrashReportEntry {
    pub path: String,
    pub size: Option<u64>,
    pub modified: Option<String>,
}

#[derive(Debug, thiserror::Error)]
pub enum CrashReportError {
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
    #[error("AFC error: {0}")]
    Afc(#[from] AfcError),
    #[error("protocol error: {0}")]
    Protocol(String),
    #[error("crash report operation timed out")]
    Timeout,
    #[error("invalid pattern '{pattern}': {message}")]
    InvalidPattern { pattern: String, message: String },
}

/// The small, stable subset of an Apple crash report that is useful to
/// callers.  `raw` retains the complete bounded JSON object (or legacy text)
/// so new Apple fields do not get silently discarded.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ParsedCrashReport {
    pub path: String,
    pub format: String,
    pub incident_id: Option<String>,
    pub timestamp: Option<String>,
    pub process: Option<String>,
    pub bundle_id: Option<String>,
    /// Executable path reported by the crash payload, when present.
    pub process_path: Option<String>,
    pub pid: Option<u64>,
    pub exception: Option<String>,
    pub termination: Option<String>,
    pub triggered_thread: Option<u64>,
    pub threads: Vec<CrashThread>,
    pub images: Vec<CrashImage>,
    pub raw: serde_json::Value,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct CrashThread {
    pub id: Option<u64>,
    pub name: Option<String>,
    pub crashed: bool,
    pub frames: Vec<CrashFrame>,
    pub raw: serde_json::Value,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct CrashFrame {
    pub image: Option<String>,
    pub symbol: Option<String>,
    pub raw: serde_json::Value,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct CrashImage {
    pub name: Option<String>,
    pub path: Option<String>,
    pub uuid: Option<String>,
    pub raw: serde_json::Value,
}

/// Bounded polling options for [`CrashReportClient::watch_reports`].
#[derive(Debug, Clone)]
pub struct CrashWatchOptions {
    pub poll_interval: Duration,
    pub timeout: Option<Duration>,
    pub max_reports: usize,
}

impl Default for CrashWatchOptions {
    fn default() -> Self {
        Self {
            poll_interval: Duration::from_secs(1),
            timeout: None,
            max_reports: 1_000,
        }
    }
}

pub struct CrashReportClient<S> {
    afc: AfcClient<S>,
}

impl<S: AsyncRead + AsyncWrite + Unpin> CrashReportClient<S> {
    pub fn new(stream: S) -> Self {
        Self {
            afc: AfcClient::new(stream),
        }
    }

    pub async fn list_reports(
        &mut self,
        pattern: Option<&str>,
    ) -> Result<Vec<CrashReportEntry>, CrashReportError> {
        let mut dirs = vec![".".to_string()];
        let mut entries = Vec::new();
        let compiled = compile_pattern(pattern.unwrap_or("*"))?;

        while let Some(dir) = dirs.pop() {
            for name in self.afc.list_dir(&dir).await? {
                let path = join_path(&dir, &name);
                let info = self.afc.stat_info(&path).await?;
                if is_dir(&info) {
                    dirs.push(path);
                    continue;
                }
                if !compiled.matches(&name) {
                    continue;
                }
                entries.push(CrashReportEntry {
                    path,
                    size: info.size,
                    modified: modified_time(&info),
                });
            }
        }

        sort_reports(&mut entries);
        Ok(entries)
    }

    pub async fn remove_reports(
        &mut self,
        pattern: Option<&str>,
    ) -> Result<usize, CrashReportError> {
        let reports = self.list_reports(pattern).await?;
        for report in &reports {
            self.afc.remove(&report.path).await?;
        }
        Ok(reports.len())
    }

    pub async fn read_report(&mut self, report: &str) -> Result<Vec<u8>, CrashReportError> {
        let path = self.resolve_report_path(report).await?;
        Ok(self.afc.read_file(&path).await?.to_vec())
    }

    /// Read and parse one report from the device.
    pub async fn parse_report(
        &mut self,
        report: &str,
    ) -> Result<ParsedCrashReport, CrashReportError> {
        let path = self.resolve_report_path(report).await?;
        let data = self.read_report_for_parse(&path).await?;
        parse_report_bytes(&path, &data)
    }

    /// Alias matching the crash-report managers in the reference clients.
    pub async fn parse(&mut self, report: &str) -> Result<ParsedCrashReport, CrashReportError> {
        self.parse_report(report).await
    }

    /// Parse reports and return the newest `count` by event timestamp.
    pub async fn parse_latest(
        &mut self,
        pattern: Option<&str>,
        count: usize,
    ) -> Result<Vec<ParsedCrashReport>, CrashReportError> {
        if count == 0 {
            return Err(CrashReportError::Protocol(
                "parse_latest count must be greater than zero".into(),
            ));
        }
        let entries = self.list_reports(pattern).await?;
        let mut reports = Vec::with_capacity(entries.len());
        for entry in entries {
            if !is_parseable_report_path(&entry.path) {
                continue;
            }
            let data = self.read_report_for_parse(&entry.path).await?;
            reports.push(parse_report_bytes(&entry.path, &data)?);
        }
        if reports.is_empty() {
            return Err(CrashReportError::Protocol("no crash reports found".into()));
        }
        sort_parsed_reports(&mut reports);
        reports.truncate(count);
        Ok(reports)
    }

    /// Remove all entries below `path` without removing the path itself.
    /// `path` is relative to the crash-report AFC jail; `/` means its root.
    pub async fn clear_reports(&mut self, path: &str) -> Result<usize, CrashReportError> {
        let root = match path {
            "" | "." | "./" | "/" => ".".to_string(),
            _ => normalize_report_path(path)?,
        };
        let entries = self.afc.list_dir(&root).await?;
        let mut removed = 0;
        for name in entries {
            let child = join_path(&root, &name);
            match self.afc.remove_all(&child).await {
                Ok(()) => removed += 1,
                // iOS may recreate this bookkeeping directory while a clear
                // is in flight.  Match pmd3's documented exception, but do
                // not hide failures for arbitrary crash-report entries.
                Err(AfcError::Status(AfcStatusCode::DirNotEmpty))
                    if root == "." && path_basename(&child) == "com.apple.appstored" => {}
                Err(error) => return Err(error.into()),
            }
        }
        Ok(removed)
    }

    /// Alias matching pymobiledevice3's manager API.
    pub async fn clear(&mut self, path: &str) -> Result<usize, CrashReportError> {
        self.clear_reports(path).await
    }

    /// Watch for files appearing in the crash-report AFC jail.
    ///
    /// The pinned go-ios mover has no notification stream. This API therefore
    /// uses a bounded directory snapshot poll, seeded before the first poll so
    /// existing reports are not replayed. A report is only read after its
    /// `(size, modified)` signature is unchanged for one poll interval.
    pub fn watch_reports<'a>(
        &'a mut self,
        pattern: Option<&'a str>,
        options: CrashWatchOptions,
    ) -> impl Stream<Item = Result<ParsedCrashReport, CrashReportError>> + 'a {
        async_stream::try_stream! {
            let compiled = compile_pattern(pattern.unwrap_or("*"))?;
            if options.max_reports == 0 {
                Err::<(), CrashReportError>(CrashReportError::Protocol("watch max_reports must be greater than zero".into()))?;
            }
            let mut known = HashMap::<String, (Option<u64>, Option<String>)>::new();
            let mut pending = HashMap::<String, ((Option<u64>, Option<String>), u8)>::new();
            let started = Instant::now();
            let deadline = options
                .timeout
                .and_then(|timeout| tokio::time::Instant::now().checked_add(timeout));
            let initial = if let Some(deadline) = deadline {
                tokio::time::timeout_at(deadline, self.list_reports(pattern))
                    .await
                    .map_err(|_| CrashReportError::Timeout)??
            } else {
                self.list_reports(pattern).await?
            };
            for report in initial {
                known.insert(report.path, (report.size, report.modified));
            }
            let mut yielded = 0usize;
            loop {
                if let Some(timeout) = options.timeout {
                    if started.elapsed() >= timeout {
                        break;
                    }
                }
                let sleep_for = options.timeout
                    .and_then(|timeout| timeout.checked_sub(started.elapsed()))
                    .map(|remaining| options.poll_interval.min(remaining))
                    .unwrap_or(options.poll_interval);
                tokio::time::sleep(sleep_for).await;
                let reports = if let Some(deadline) = deadline {
                    tokio::time::timeout_at(
                        deadline,
                        self.list_reports(Some(compiled.0.as_str())),
                    )
                    .await
                    .map_err(|_| CrashReportError::Timeout)??
                } else {
                    self.list_reports(Some(compiled.0.as_str())).await?
                };
                let mut current = HashSet::new();
                for report in reports {
                    if !compiled.matches(path_basename(&report.path)) {
                        continue;
                    }
                    if !is_parseable_report_path(&report.path) {
                        continue;
                    }
                    current.insert(report.path.clone());
                    let signature = (report.size, report.modified.clone());
                    if known.get(&report.path) == Some(&signature) {
                        continue;
                    }
                    // A path may be recreated after a report is consumed.  Its
                    // new AFC signature must be treated as a new event, while
                    // an unchanged snapshot remains deduplicated above.
                    known.remove(&report.path);
                    if pending.len() >= options.max_reports && !pending.contains_key(&report.path) {
                        continue;
                    }
                    if pending
                        .get(&report.path)
                        .map(|(pending_signature, _)| pending_signature)
                        != Some(&signature)
                    {
                        pending.insert(report.path.clone(), (signature, 0));
                        continue;
                    }
                    let data = if let Some(deadline) = deadline {
                        tokio::time::timeout_at(deadline, self.afc.read_file(&report.path))
                            .await
                            .map_err(|_| CrashReportError::Timeout)??
                            .to_vec()
                    } else {
                        self.afc.read_file(&report.path).await?.to_vec()
                    };
                    let parsed = match parse_report_bytes(&report.path, &data) {
                        Ok(parsed) => parsed,
                        Err(error @ CrashReportError::Protocol(_)) => {
                            let Some((_, retries)) = pending.get_mut(&report.path) else {
                                continue;
                            };
                            if *retries >= MAX_WATCH_PARSE_RETRIES {
                                Err::<(), CrashReportError>(error)?;
                                unreachable!("watch error propagation returned unexpectedly");
                            }
                            // AFC can expose the name before the producer has
                            // finished writing the two-line report. Retry a
                            // few times instead of losing the event or ending
                            // the stream on a transient JSON decode error.
                            *retries += 1;
                            continue;
                        }
                        Err(error) => {
                            Err::<(), CrashReportError>(error)?;
                            unreachable!("watch error propagation returned unexpectedly");
                        }
                    };
                    known.insert(report.path.clone(), signature);
                    pending.remove(&report.path);
                    yielded += 1;
                    yield parsed;
                    if yielded >= options.max_reports {
                        break;
                    }
                }
                known.retain(|path, _| current.contains(path));
                pending.retain(|path, _| current.contains(path));
                if yielded >= options.max_reports {
                    break;
                }
            }
        }
    }

    async fn resolve_report_path(&mut self, report: &str) -> Result<String, CrashReportError> {
        if report.contains('/') {
            return normalize_report_path(report);
        }

        let reports = self.list_reports(Some("*")).await?;
        resolve_report_path_from_entries(report, &reports)
    }

    /// Avoid handing a parser-bound report to AFC's much larger generic
    /// in-memory reader.  The stat is advisory (the file may grow), while the
    /// parser's final length check remains authoritative.
    async fn read_report_for_parse(&mut self, path: &str) -> Result<Vec<u8>, CrashReportError> {
        if let Some(size) = self.afc.stat_info(path).await?.size {
            if size > MAX_CRASH_REPORT_BYTES as u64 {
                return Err(CrashReportError::Protocol(format!(
                    "crash report {path} is {size} bytes; limit is {MAX_CRASH_REPORT_BYTES}"
                )));
            }
        }
        Ok(self.afc.read_file(path).await?.to_vec())
    }

    /// Alias matching the reference clients' watch operation.
    pub fn watch<'a>(
        &'a mut self,
        pattern: Option<&'a str>,
        options: CrashWatchOptions,
    ) -> impl Stream<Item = Result<ParsedCrashReport, CrashReportError>> + 'a {
        self.watch_reports(pattern, options)
    }
}

/// Complete the crashreport mover handshake with one absolute timeout.
pub async fn flush_reports<S>(stream: &mut S, timeout: Duration) -> Result<(), CrashReportError>
where
    S: AsyncRead + Unpin,
{
    let deadline = tokio::time::Instant::now()
        .checked_add(timeout)
        .ok_or(CrashReportError::Timeout)?;
    flush_reports_at(stream, deadline).await
}

/// Complete the RSD crashreport mover handshake.  RSD's lockdown shim sends
/// the five-byte `ping\0` acknowledgement used by pymobiledevice3, whereas
/// the classic lockdown service sends only the four bytes used by go-ios.
pub async fn flush_reports_rsd<S>(stream: &mut S, timeout: Duration) -> Result<(), CrashReportError>
where
    S: AsyncRead + Unpin,
{
    let deadline = tokio::time::Instant::now()
        .checked_add(timeout)
        .ok_or(CrashReportError::Timeout)?;
    flush_reports_rsd_at(stream, deadline).await
}

/// Complete the classic mover handshake before an absolute deadline.
///
/// This variant is useful when opening the service is part of the same
/// operation budget: callers can pass the original deadline instead of
/// converting the remaining time into a fresh relative timeout.
pub async fn flush_reports_at<S>(
    stream: &mut S,
    deadline: tokio::time::Instant,
) -> Result<(), CrashReportError>
where
    S: AsyncRead + Unpin,
{
    tokio::time::timeout_at(deadline, prepare_reports(stream))
        .await
        .map_err(|_| CrashReportError::Timeout)??;
    Ok(())
}

/// Complete the RSD mover handshake (`ping\0`) before an absolute deadline.
pub async fn flush_reports_rsd_at<S>(
    stream: &mut S,
    deadline: tokio::time::Instant,
) -> Result<(), CrashReportError>
where
    S: AsyncRead + Unpin,
{
    tokio::time::timeout_at(deadline, prepare_reports_rsd(stream))
        .await
        .map_err(|_| CrashReportError::Timeout)??;
    Ok(())
}

/// Parse an Apple `.ips` (header JSON followed by body JSON) or legacy
/// `.crash` text report without requiring a connected device.
pub fn parse_report_bytes(path: &str, data: &[u8]) -> Result<ParsedCrashReport, CrashReportError> {
    if data.len() > MAX_CRASH_REPORT_BYTES {
        return Err(CrashReportError::Protocol(format!(
            "crash report exceeds {MAX_CRASH_REPORT_BYTES} byte limit"
        )));
    }
    let extension = path
        .rsplit('.')
        .next()
        .unwrap_or_default()
        .to_ascii_lowercase();
    if extension == "crash" {
        return parse_legacy_crash(path, data);
    }
    parse_ips(path, data)
}

fn parse_ips(path: &str, data: &[u8]) -> Result<ParsedCrashReport, CrashReportError> {
    let text = std::str::from_utf8(data)
        .map_err(|_| CrashReportError::Protocol("crash report is not UTF-8 JSON".into()))?;
    let (header_text, body_text) = text
        .split_once('\n')
        .ok_or_else(|| CrashReportError::Protocol(".ips report is missing its JSON body".into()))?;
    let header: serde_json::Value = serde_json::from_str(header_text.trim())
        .map_err(|e| CrashReportError::Protocol(format!("invalid .ips header JSON: {e}")))?;
    let body: serde_json::Value = serde_json::from_str(body_text.trim())
        .map_err(|e| CrashReportError::Protocol(format!("invalid .ips body JSON: {e}")))?;
    if !header.is_object() || !body.is_object() {
        return Err(CrashReportError::Protocol(
            ".ips header and body must be JSON objects".into(),
        ));
    }
    let sources = [&body, &header];
    let mut parsed = ParsedCrashReport {
        path: path.to_string(),
        format: "ips".into(),
        incident_id: first_string(&sources, &["incident_id", "incidentId"]),
        timestamp: first_string(&sources, &["timestamp", "date"]),
        process: first_string(&sources, &["name", "processName", "procName"]),
        bundle_id: first_string(&sources, &["bundleID", "bundle_id", "identifier"]),
        process_path: first_string(&sources, &["path", "procPath", "executablePath"]),
        pid: first_u64(&sources, &["pid", "processId"]),
        exception: nested_string(&sources, &["exception"], &["type", "exceptionType"])
            .or_else(|| first_string(&sources, &["exceptionType"])),
        termination: nested_string(&sources, &["termination"], &["reason", "by"])
            .or_else(|| first_string(&sources, &["terminationReason"])),
        triggered_thread: first_u64(&sources, &["triggeredThread", "triggered_thread"]),
        threads: parse_threads(&body),
        images: parse_images(&body),
        raw: serde_json::json!({"header": header, "body": body}),
    };
    if parsed.triggered_thread.is_none() {
        parsed.triggered_thread = parsed
            .threads
            .iter()
            .find(|thread| thread.crashed)
            .and_then(|thread| thread.id);
    }
    Ok(parsed)
}

fn parse_legacy_crash(path: &str, data: &[u8]) -> Result<ParsedCrashReport, CrashReportError> {
    let text = std::str::from_utf8(data)
        .map_err(|_| CrashReportError::Protocol("legacy crash report is not UTF-8".into()))?;
    let mut report = ParsedCrashReport {
        path: path.to_string(),
        format: "crash".into(),
        incident_id: None,
        timestamp: None,
        process: None,
        bundle_id: None,
        process_path: None,
        pid: None,
        exception: None,
        termination: None,
        triggered_thread: None,
        threads: Vec::new(),
        images: Vec::new(),
        raw: serde_json::json!({"text": text}),
    };
    let mut current_thread: Option<CrashThread> = None;
    for (line_number, line) in text.lines().enumerate() {
        if line_number >= MAX_CRASH_REPORT_LINES {
            return Err(CrashReportError::Protocol(
                "legacy crash report has too many lines".into(),
            ));
        }
        if line.len() > MAX_CRASH_REPORT_LINE_BYTES {
            return Err(CrashReportError::Protocol(
                "legacy crash report line is too long".into(),
            ));
        }
        let (key, value) = line
            .split_once(':')
            .map(|(k, v)| (k.trim(), v.trim()))
            .unwrap_or(("", ""));
        match key {
            "Process" => report.process = nonempty(value),
            "Identifier" => report.bundle_id = nonempty(value),
            "Path" => report.process_path = nonempty(value),
            "Date/Time" => report.timestamp = nonempty(value),
            "Exception Type" => report.exception = nonempty(value),
            "Termination Reason" => report.termination = nonempty(value),
            "Triggered by Thread" => report.triggered_thread = value.parse().ok(),
            _ => {}
        }
        if let Some(rest) = line.strip_prefix("Thread ") {
            if let Some(thread) = current_thread.take() {
                report.threads.push(thread);
            }
            let id = rest
                .split_whitespace()
                .next()
                .and_then(|value| value.parse().ok());
            let crashed = rest.contains("Crashed");
            current_thread = Some(CrashThread {
                id,
                name: None,
                crashed,
                frames: Vec::new(),
                raw: serde_json::json!({"header": line}),
            });
        } else if let Some(thread) = current_thread.as_mut() {
            if line
                .trim_start()
                .chars()
                .next()
                .is_some_and(|c| c.is_ascii_digit())
            {
                thread.frames.push(CrashFrame {
                    image: None,
                    symbol: Some(line.trim().to_string()),
                    raw: serde_json::json!({"text": line}),
                });
            }
        }
    }
    if let Some(thread) = current_thread {
        report.threads.push(thread);
    }
    Ok(report)
}

fn nonempty(value: &str) -> Option<String> {
    (!value.is_empty()).then(|| value.to_string())
}

fn is_parseable_report_path(path: &str) -> bool {
    matches!(
        path.rsplit('.')
            .next()
            .map(str::to_ascii_lowercase)
            .as_deref(),
        Some("ips") | Some("panic") | Some("crash")
    )
}

fn first_string(values: &[&serde_json::Value], keys: &[&str]) -> Option<String> {
    values.iter().find_map(|value| {
        keys.iter()
            .find_map(|key| value.get(*key).and_then(value_to_string))
    })
}

fn first_u64(values: &[&serde_json::Value], keys: &[&str]) -> Option<u64> {
    values.iter().find_map(|value| {
        keys.iter()
            .find_map(|key| value.get(*key).and_then(value_to_u64))
    })
}

fn nested_string(values: &[&serde_json::Value], objects: &[&str], keys: &[&str]) -> Option<String> {
    values.iter().find_map(|value| {
        objects.iter().find_map(|object| {
            value.get(*object).and_then(|nested| {
                keys.iter()
                    .find_map(|key| nested.get(*key).and_then(value_to_string))
            })
        })
    })
}

fn value_to_string(value: &serde_json::Value) -> Option<String> {
    match value {
        serde_json::Value::String(value) => Some(value.clone()),
        serde_json::Value::Number(value) => Some(value.to_string()),
        _ => None,
    }
}

fn value_to_u64(value: &serde_json::Value) -> Option<u64> {
    value.as_u64().or_else(|| value.as_str()?.parse().ok())
}

fn parse_threads(body: &serde_json::Value) -> Vec<CrashThread> {
    body.get("threads")
        .and_then(serde_json::Value::as_array)
        .into_iter()
        .flatten()
        .map(|thread| CrashThread {
            id: thread.get("id").and_then(value_to_u64),
            name: thread.get("name").and_then(value_to_string),
            crashed: thread
                .get("crashed")
                .and_then(|value| value.as_bool())
                .unwrap_or(false),
            frames: thread
                .get("frames")
                .and_then(serde_json::Value::as_array)
                .into_iter()
                .flatten()
                .map(|frame| CrashFrame {
                    image: frame
                        .get("imageIndex")
                        .and_then(value_to_string)
                        .or_else(|| frame.get("image").and_then(value_to_string)),
                    symbol: frame
                        .get("symbol")
                        .and_then(value_to_string)
                        .or_else(|| frame.get("symbolLocation").and_then(value_to_string)),
                    raw: frame.clone(),
                })
                .collect(),
            raw: thread.clone(),
        })
        .collect()
}

fn parse_images(body: &serde_json::Value) -> Vec<CrashImage> {
    body.get("usedImages")
        .or_else(|| body.get("images"))
        .and_then(serde_json::Value::as_array)
        .into_iter()
        .flatten()
        .map(|image| CrashImage {
            name: image.get("name").and_then(value_to_string),
            path: image.get("path").and_then(value_to_string),
            uuid: image.get("uuid").and_then(value_to_string),
            raw: image.clone(),
        })
        .collect()
}

/// Sort parsed reports by their event timestamp when it has a comparable
/// Apple ISO-like representation, then use path as a deterministic tie-break.
pub fn sort_parsed_reports(reports: &mut [ParsedCrashReport]) {
    reports.sort_by(|a, b| {
        match (
            a.timestamp.as_deref().and_then(timestamp_sort_key),
            b.timestamp.as_deref().and_then(timestamp_sort_key),
        ) {
            (Some(a), Some(b)) => b.cmp(&a),
            (Some(_), None) => std::cmp::Ordering::Less,
            (None, Some(_)) => std::cmp::Ordering::Greater,
            (None, None) => b
                .timestamp
                .as_deref()
                .unwrap_or("")
                .cmp(a.timestamp.as_deref().unwrap_or("")),
        }
        .then_with(|| a.path.cmp(&b.path))
    });
}

fn timestamp_sort_key(value: &str) -> Option<i128> {
    let date = value.get(..10)?;
    let year = date.get(..4)?.parse::<i128>().ok()?;
    let month = date.get(5..7)?.parse::<i128>().ok()?;
    let day = date.get(8..10)?.parse::<i128>().ok()?;
    if date.as_bytes().get(4) != Some(&b'-')
        || date.as_bytes().get(7) != Some(&b'-')
        || !(1..=12).contains(&month)
        || !(1..=31).contains(&day)
    {
        return None;
    }
    let time = value.get(10..)?.trim_start_matches(['T', ' ']);
    let hour = time.get(..2)?.parse::<i128>().ok()?;
    let minute = time.get(3..5)?.parse::<i128>().ok()?;
    let second = time.get(6..8)?.parse::<i128>().ok()?;
    if time.as_bytes().get(2) != Some(&b':')
        || time.as_bytes().get(5) != Some(&b':')
        || hour > 23
        || minute > 59
        || second > 60
    {
        return None;
    }
    let mut offset = 0i128;
    let rest = &time[8..];
    if let Some(index) = rest.find(['+', '-']) {
        let sign = if rest.as_bytes()[index] == b'+' {
            1
        } else {
            -1
        };
        let zone = &rest[index + 1..];
        let zone = zone.trim_end_matches('Z');
        let zone_hour = zone.get(..2)?.parse::<i128>().ok()?;
        let zone_minute = if zone.as_bytes().get(2) == Some(&b':') {
            zone.get(3..5)?.parse::<i128>().ok()?
        } else {
            zone.get(2..4).unwrap_or("00").parse::<i128>().ok()?
        };
        if zone_hour > 23 || zone_minute > 59 {
            return None;
        }
        offset = sign * (zone_hour * 3_600 + zone_minute * 60);
    }
    Some(days_from_civil(year, month, day) * 86_400 + hour * 3_600 + minute * 60 + second - offset)
}

fn days_from_civil(year: i128, month: i128, day: i128) -> i128 {
    let adjusted_year = year - i128::from(month <= 2);
    let era = (if adjusted_year >= 0 {
        adjusted_year
    } else {
        adjusted_year - 399
    })
    .div_euclid(400);
    let year_of_era = adjusted_year - era * 400;
    let month_prime = month + if month > 2 { -3 } else { 9 };
    let day_of_year = (153 * month_prime + 2) / 5 + day - 1;
    let day_of_era = year_of_era * 365 + year_of_era / 4 - year_of_era / 100 + day_of_year;
    era * 146_097 + day_of_era - 719_468
}

pub async fn prepare_reports<S>(stream: &mut S) -> Result<(), CrashReportError>
where
    S: AsyncRead + Unpin,
{
    let mut ping = [0u8; 4];
    stream.read_exact(&mut ping).await?;
    if &ping != b"ping" {
        return Err(CrashReportError::Protocol(format!(
            "crashreport mover did not return ping: {:02x?}",
            ping
        )));
    }
    Ok(())
}

/// Complete the RSD crashreport mover handshake (`ping\0`).
pub async fn prepare_reports_rsd<S>(stream: &mut S) -> Result<(), CrashReportError>
where
    S: AsyncRead + Unpin,
{
    let mut ping = [0u8; 5];
    stream.read_exact(&mut ping).await?;
    if &ping != b"ping\0" {
        return Err(CrashReportError::Protocol(format!(
            "RSD crashreport mover did not return ping\\0: {:02x?}",
            ping
        )));
    }
    Ok(())
}

pub fn matches_pattern(path: &str, pattern: &str) -> Result<bool, CrashReportError> {
    Ok(compile_pattern(pattern)?.matches(path_basename(path)))
}

pub fn sort_reports(entries: &mut [CrashReportEntry]) {
    entries.sort_by(|a, b| match (&a.modified, &b.modified) {
        (Some(a_modified), Some(b_modified)) => {
            b_modified.cmp(a_modified).then_with(|| a.path.cmp(&b.path))
        }
        (Some(_), None) => std::cmp::Ordering::Less,
        (None, Some(_)) => std::cmp::Ordering::Greater,
        (None, None) => a.path.cmp(&b.path),
    });
}

fn compile_pattern(pattern: &str) -> Result<Pattern, CrashReportError> {
    validate_pattern(pattern)?;
    Ok(Pattern(pattern.to_string()))
}

fn modified_time(info: &AfcFileInfo) -> Option<String> {
    info.raw
        .get("st_mtime")
        .or_else(|| info.raw.get("st_birthtime"))
        .map(|raw| format_human_readable_timestamp(raw))
}

fn format_human_readable_timestamp(raw: &str) -> String {
    match RawTimestamp::parse(raw) {
        Some(timestamp) => timestamp.format_utc(),
        None => raw.to_string(),
    }
}

fn is_dir(info: &AfcFileInfo) -> bool {
    matches!(info.file_type.as_deref(), Some("S_IFDIR"))
}

fn path_basename(path: &str) -> &str {
    path.rsplit('/').next().unwrap_or(path)
}

fn join_path(dir: &str, name: &str) -> String {
    if dir == "." {
        format!("./{name}")
    } else {
        format!("{}/{}", dir.trim_end_matches('/'), name)
    }
}

/// Turn a caller-supplied report path into one rooted at the crash-log jail.
///
/// The leading `/` was already stripped, but `..` was not, and the result is
/// sent verbatim over `crashreportcopymobile`, where the device-side AFC has
/// historically honoured it. Reject traversal rather than normalising it away,
/// matching how `backup2::sanitize_relative_path` treats device-supplied paths.
fn normalize_report_path(report: &str) -> Result<String, CrashReportError> {
    let trimmed = report.trim_start_matches('/');
    let relative = trimmed.strip_prefix("./").unwrap_or(trimmed);

    if relative.is_empty() {
        return Err(CrashReportError::Protocol("empty crash report path".into()));
    }
    if relative.split(['/', '\\']).any(|part| part == "..") {
        return Err(CrashReportError::Protocol(format!(
            "crash report path must stay inside the crash log directory: {report}"
        )));
    }

    Ok(format!("./{relative}"))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct RawTimestamp {
    seconds: i128,
}

impl RawTimestamp {
    fn parse(raw: &str) -> Option<Self> {
        let value = raw.trim().parse::<i128>().ok()?;
        for divisor in [1_000_000_000_i128, 1_000_000, 1_000, 1] {
            let seconds = value.div_euclid(divisor);
            if plausible_year(seconds) {
                return Some(Self { seconds });
            }
        }

        Some(Self { seconds: value })
    }

    fn format_utc(self) -> String {
        let total_seconds = self.seconds;
        let days = total_seconds.div_euclid(86_400);
        let seconds_of_day = total_seconds.rem_euclid(86_400) as u32;
        let (year, month, day) = civil_from_days(days);
        let hour = seconds_of_day / 3_600;
        let minute = (seconds_of_day % 3_600) / 60;
        let second = seconds_of_day % 60;

        format!("{year:04}-{month:02}-{day:02} {hour:02}:{minute:02}:{second:02} UTC")
    }
}

fn plausible_year(seconds: i128) -> bool {
    let days = seconds.div_euclid(86_400);
    let (year, _, _) = civil_from_days(days);
    (1970..=2500).contains(&year)
}

fn civil_from_days(days: i128) -> (i128, i128, i128) {
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = mp + if mp < 10 { 3 } else { -9 };
    let year = y + if month <= 2 { 1 } else { 0 };

    (year, month, day)
}

fn resolve_report_path_from_entries(
    report: &str,
    reports: &[CrashReportEntry],
) -> Result<String, CrashReportError> {
    let mut matches = reports
        .iter()
        .filter(|entry| path_basename(&entry.path) == report)
        .map(|entry| entry.path.clone())
        .collect::<Vec<_>>();

    match matches.len() {
        0 => Err(CrashReportError::Protocol(format!(
            "crash report '{report}' not found"
        ))),
        1 => Ok(matches.pop().unwrap()),
        _ => Err(CrashReportError::Protocol(format!(
            "crash report '{report}' is ambiguous"
        ))),
    }
}

struct Pattern(String);

impl Pattern {
    fn matches(&self, candidate: &str) -> bool {
        wildcard_match(self.0.as_bytes(), candidate.as_bytes())
    }
}

fn validate_pattern(pattern: &str) -> Result<(), CrashReportError> {
    for ch in ['[', ']', '{', '}'] {
        if pattern.contains(ch) {
            return Err(CrashReportError::InvalidPattern {
                pattern: pattern.to_string(),
                message: format!("unsupported pattern syntax '{ch}'"),
            });
        }
    }
    Ok(())
}

fn wildcard_match(pattern: &[u8], candidate: &[u8]) -> bool {
    let mut p = 0usize;
    let mut c = 0usize;
    let mut star = None;
    let mut star_match = 0usize;

    while c < candidate.len() {
        if p < pattern.len() && (pattern[p] == b'?' || pattern[p] == candidate[c]) {
            p += 1;
            c += 1;
        } else if p < pattern.len() && pattern[p] == b'*' {
            star = Some(p);
            p += 1;
            star_match = c;
        } else if let Some(star_pos) = star {
            p = star_pos + 1;
            star_match += 1;
            c = star_match;
        } else {
            return false;
        }
    }

    while p < pattern.len() && pattern[p] == b'*' {
        p += 1;
    }

    p == pattern.len()
}

#[cfg(test)]
mod tests {
    use crate::proto::afc::{AfcHeader, AfcOpcode};
    use tokio::io::{duplex, AsyncReadExt, AsyncWriteExt};
    use zerocopy::{FromBytes, IntoBytes};

    use super::*;

    #[tokio::test]
    async fn prepare_reports_accepts_ping() {
        let (mut client, mut server) = duplex(16);
        tokio::spawn(async move {
            server.write_all(b"ping").await.unwrap();
        });

        prepare_reports(&mut client).await.unwrap();
    }

    #[tokio::test]
    async fn prepare_reports_accepts_fragmented_classic_ping() {
        let (mut client, mut server) = duplex(16);
        tokio::spawn(async move {
            server.write_all(b"pi").await.unwrap();
            tokio::task::yield_now().await;
            server.write_all(b"ng").await.unwrap();
        });

        prepare_reports(&mut client).await.unwrap();
    }

    #[tokio::test]
    async fn prepare_reports_rejects_non_ping() {
        let (mut client, mut server) = duplex(16);
        tokio::spawn(async move {
            server.write_all(b"pong").await.unwrap();
        });

        let err = prepare_reports(&mut client).await.unwrap_err();
        assert!(err.to_string().contains("ping"));
    }

    #[tokio::test]
    async fn prepare_reports_rsd_requires_ping_nul() {
        let (mut client, mut server) = duplex(16);
        tokio::spawn(async move {
            server.write_all(b"ping\0").await.unwrap();
        });

        prepare_reports_rsd(&mut client).await.unwrap();

        let (mut client, mut server) = duplex(16);
        tokio::spawn(async move {
            server.write_all(b"pong\0").await.unwrap();
        });
        let err = prepare_reports_rsd(&mut client).await.unwrap_err();
        assert!(err.to_string().contains("ping\\0"));
    }

    #[tokio::test]
    async fn prepare_reports_rsd_accepts_fragmented_ping_nul() {
        let (mut client, mut server) = duplex(16);
        tokio::spawn(async move {
            server.write_all(b"pin").await.unwrap();
            tokio::task::yield_now().await;
            server.write_all(b"g\0").await.unwrap();
        });

        prepare_reports_rsd(&mut client).await.unwrap();
    }

    #[tokio::test]
    async fn flush_reports_has_a_total_timeout() {
        let (mut client, _server) = duplex(8);
        let err = flush_reports(&mut client, Duration::from_millis(1))
            .await
            .unwrap_err();
        assert!(matches!(err, CrashReportError::Timeout));
    }

    #[tokio::test]
    async fn flush_reports_rsd_has_a_total_timeout() {
        let (mut client, _server) = duplex(8);
        let err = flush_reports_rsd(&mut client, Duration::from_millis(1))
            .await
            .unwrap_err();
        assert!(matches!(err, CrashReportError::Timeout));
    }

    #[tokio::test]
    async fn absolute_flush_deadline_is_not_restarted_after_service_connect() {
        let (mut client, _server) = duplex(8);
        let deadline = tokio::time::Instant::now();
        let err = flush_reports_at(&mut client, deadline).await.unwrap_err();
        assert!(matches!(err, CrashReportError::Timeout));

        let (mut client, _server) = duplex(8);
        let deadline = tokio::time::Instant::now();
        let err = flush_reports_rsd_at(&mut client, deadline)
            .await
            .unwrap_err();
        assert!(matches!(err, CrashReportError::Timeout));
    }

    #[test]
    fn matches_pattern_uses_basename() {
        assert!(matches_pattern("./foo/bar/Test.ips", "*.ips").unwrap());
        assert!(!matches_pattern("./foo/bar/Test.ips", "foo*").unwrap());
    }

    #[test]
    fn sort_reports_prefers_modified_descending() {
        let mut entries = vec![
            CrashReportEntry {
                path: "./B.ips".into(),
                size: Some(20),
                modified: Some("2026-04-01 10:00:00 UTC".into()),
            },
            CrashReportEntry {
                path: "./A.ips".into(),
                size: Some(10),
                modified: Some("2026-04-02 10:00:00 UTC".into()),
            },
            CrashReportEntry {
                path: "./C.ips".into(),
                size: Some(5),
                modified: None,
            },
        ];

        sort_reports(&mut entries);
        assert_eq!(entries[0].path, "./A.ips");
        assert_eq!(entries[1].path, "./B.ips");
        assert_eq!(entries[2].path, "./C.ips");
    }

    #[test]
    fn modified_time_formats_raw_afc_timestamp() {
        let info = AfcFileInfo {
            name: Some("Example.ips".into()),
            file_type: Some("S_IFREG".into()),
            size: Some(1),
            mode: None,
            link_target: None,
            raw: std::iter::once(("st_mtime".into(), "86400000000000".into())).collect(),
        };

        assert_eq!(modified_time(&info), Some("1970-01-02 00:00:00 UTC".into()));
    }

    #[test]
    fn parse_ips_extracts_header_body_and_unicode() {
        let data = "{\"bug_type\":\"999\",\"incident_id\":\"abc\",\"timestamp\":\"2026-01-02 03:04:05 +0000\",\"name\":\"Demo\",\"pid\":42}\n{\"exception\":{\"type\":\"EXC_BAD_ACCESS\"},\"termination\":{\"reason\":\"signal 11\"},\"threads\":[{\"id\":7,\"crashed\":true,\"frames\":[{\"symbol\":\"δemo\"}]}],\"usedImages\":[{\"name\":\"Demo\",\"uuid\":\"u\"}],\"newField\":\"保留\"}".as_bytes();
        let report = parse_report_bytes("Demo.ips", data).unwrap();
        assert_eq!(report.incident_id.as_deref(), Some("abc"));
        assert_eq!(report.process.as_deref(), Some("Demo"));
        assert_eq!(report.pid, Some(42));
        assert_eq!(report.exception.as_deref(), Some("EXC_BAD_ACCESS"));
        assert_eq!(report.termination.as_deref(), Some("signal 11"));
        assert_eq!(report.triggered_thread, Some(7));
        assert_eq!(report.threads[0].frames[0].symbol.as_deref(), Some("δemo"));
        assert_eq!(report.raw["body"]["newField"], "保留");
    }

    #[test]
    fn parse_legacy_crash_extracts_basic_fields() {
        let data = b"Process: Demo\nPath: /Applications/Demo.app/Demo\nIdentifier: com.example.demo\nDate/Time: 2026-01-02 03:04:05 +0000\nException Type: EXC_CRASH\nTermination Reason: SIGNAL 6\nTriggered by Thread: 3\n\nThread 3 Crashed:\n0   Demo 0x0000 symbol\n";
        let report = parse_report_bytes("Demo.crash", data).unwrap();
        assert_eq!(report.process.as_deref(), Some("Demo"));
        assert_eq!(report.bundle_id.as_deref(), Some("com.example.demo"));
        assert_eq!(
            report.process_path.as_deref(),
            Some("/Applications/Demo.app/Demo")
        );
        assert_eq!(report.triggered_thread, Some(3));
        assert_eq!(report.threads.len(), 1);
        assert!(report.threads[0].crashed);
        assert_eq!(report.threads[0].frames.len(), 1);
    }

    #[test]
    fn parse_legacy_crash_rejects_invalid_utf8() {
        let err = parse_report_bytes("Demo.crash", b"Process: Demo\n\x80").unwrap_err();
        assert!(err.to_string().contains("not UTF-8"));
    }

    #[test]
    fn panic_reports_use_the_ips_parser() {
        let report = parse_report_bytes("Demo.panic", b"{}\n{\"name\":\"Demo\"}").unwrap();
        assert_eq!(report.format, "ips");
        assert_eq!(report.process.as_deref(), Some("Demo"));
    }

    #[test]
    fn parse_ips_rejects_invalid_body_and_budget() {
        let err = parse_report_bytes("bad.ips", b"{}\nnot-json").unwrap_err();
        assert!(err.to_string().contains("body JSON"));
        let oversized = vec![b'x'; MAX_CRASH_REPORT_BYTES + 1];
        let err = parse_report_bytes("bad.ips", &oversized).unwrap_err();
        assert!(err.to_string().contains("byte limit"));
    }

    #[test]
    fn parsed_reports_sort_by_event_timestamp() {
        let mut reports = vec![
            parse_report_bytes("new.ips", b"{}\n{\"timestamp\":\"2026-02-01\"}").unwrap(),
            parse_report_bytes("old.ips", b"{}\n{\"timestamp\":\"2026-01-01\"}").unwrap(),
        ];
        sort_parsed_reports(&mut reports);
        assert_eq!(reports[0].path, "new.ips");
    }

    #[test]
    fn parsed_reports_normalize_timezone_offsets_for_latest() {
        let mut reports = vec![
            parse_report_bytes(
                "utc.ips",
                b"{}\n{\"timestamp\":\"2026-01-01 23:30:00 +0000\"}",
            )
            .unwrap(),
            parse_report_bytes(
                "east.ips",
                b"{}\n{\"timestamp\":\"2026-01-02 00:00:00 +0100\"}",
            )
            .unwrap(),
        ];
        sort_parsed_reports(&mut reports);
        assert_eq!(reports[0].path, "utc.ips");
    }

    #[test]
    fn resolve_report_path_from_entries_uses_basename_match() {
        let reports = vec![CrashReportEntry {
            path: "./foo/Example.ips".into(),
            size: Some(1),
            modified: None,
        }];

        let resolved = resolve_report_path_from_entries("Example.ips", &reports).unwrap();
        assert_eq!(resolved, "./foo/Example.ips");
    }

    #[test]
    fn resolve_report_path_from_entries_rejects_ambiguous_basename() {
        let reports = vec![
            CrashReportEntry {
                path: "./foo/Example.ips".into(),
                size: Some(1),
                modified: None,
            },
            CrashReportEntry {
                path: "./bar/Example.ips".into(),
                size: Some(2),
                modified: None,
            },
        ];

        let err = resolve_report_path_from_entries("Example.ips", &reports).unwrap_err();
        assert!(err.to_string().contains("ambiguous"));
    }

    #[test]
    fn clear_and_read_paths_reject_unix_and_windows_traversal() {
        for path in [
            "../outside.ips",
            r"..\outside.ips",
            "./nested/../../outside.ips",
        ] {
            assert!(normalize_report_path(path).is_err(), "accepted {path}");
        }
        assert_eq!(
            normalize_report_path("/nested/report.ips").unwrap(),
            "./nested/report.ips"
        );
    }

    #[tokio::test]
    async fn remove_reports_removes_only_matching_reports() {
        let (client_side, mut server_side) = duplex(4096);
        let removed_paths = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let removed_paths_server = removed_paths.clone();

        tokio::spawn(async move {
            let stat_names = ["B.log", "A.ips", "C.ips"];
            let mut removed = 0usize;

            loop {
                let mut hdr_buf = [0u8; AfcHeader::SIZE];
                if server_side.read_exact(&mut hdr_buf).await.is_err() {
                    break;
                }
                let hdr = AfcHeader::ref_from_bytes(&hdr_buf).unwrap();
                let entire_len = hdr.entire_len.get() as usize;
                let this_len = hdr.this_len.get() as usize;
                let header_payload_len = this_len.saturating_sub(AfcHeader::SIZE);
                let payload_len = entire_len.saturating_sub(this_len);
                let mut header_payload = vec![0u8; header_payload_len];
                let mut payload = vec![0u8; payload_len];

                if header_payload_len > 0 {
                    server_side.read_exact(&mut header_payload).await.unwrap();
                }
                if payload_len > 0 {
                    server_side.read_exact(&mut payload).await.unwrap();
                }

                match hdr.operation.get() {
                    x if x == AfcOpcode::ReadDir as u64 => {
                        assert_eq!(trim_c_string(&header_payload), ".");
                        let names = stat_names.join("\0") + "\0";
                        let resp = AfcHeader::new(
                            hdr.packet_num.get(),
                            AfcOpcode::ReadDir,
                            0,
                            names.len(),
                        );
                        server_side.write_all(resp.as_bytes()).await.unwrap();
                        server_side.write_all(names.as_bytes()).await.unwrap();
                    }
                    x if x == AfcOpcode::GetFileInfo as u64 => {
                        let path = trim_c_string(&header_payload);
                        let basename = path_basename(&path);
                        let payload = match basename {
                            "B.log" => b"st_ifmt\0S_IFREG\0st_size\x001\0".as_slice(),
                            "A.ips" => b"st_ifmt\0S_IFREG\0st_size\x001\0".as_slice(),
                            "C.ips" => b"st_ifmt\0S_IFREG\0st_size\x001\0".as_slice(),
                            other => panic!("unexpected stat path: {other}"),
                        };
                        let resp = AfcHeader::new(
                            hdr.packet_num.get(),
                            AfcOpcode::GetFileInfo,
                            0,
                            payload.len(),
                        );
                        server_side.write_all(resp.as_bytes()).await.unwrap();
                        server_side.write_all(payload).await.unwrap();
                    }
                    x if x == AfcOpcode::RemovePath as u64 => {
                        let path = trim_c_string(&header_payload);
                        removed_paths_server.lock().unwrap().push(path);
                        removed += 1;
                        let resp = AfcHeader::new(hdr.packet_num.get(), AfcOpcode::Status, 8, 0);
                        server_side.write_all(resp.as_bytes()).await.unwrap();
                        server_side.write_all(&0u64.to_le_bytes()).await.unwrap();
                        if removed == 2 {
                            break;
                        }
                    }
                    other => panic!("unexpected AFC opcode: {other}"),
                }
            }
        });

        let mut client = CrashReportClient::new(client_side);
        let removed = client.remove_reports(Some("*.ips")).await.unwrap();

        assert_eq!(removed, 2);
        assert_eq!(
            removed_paths.lock().unwrap().as_slice(),
            &["./A.ips".to_string(), "./C.ips".to_string()]
        );
    }

    #[tokio::test]
    async fn remove_reports_returns_zero_for_no_matches() {
        let (client_side, mut server_side) = duplex(4096);

        tokio::spawn(async move {
            loop {
                let mut hdr_buf = [0u8; AfcHeader::SIZE];
                if server_side.read_exact(&mut hdr_buf).await.is_err() {
                    break;
                }
                let hdr = AfcHeader::ref_from_bytes(&hdr_buf).unwrap();
                let entire_len = hdr.entire_len.get() as usize;
                let this_len = hdr.this_len.get() as usize;
                let header_payload_len = this_len.saturating_sub(AfcHeader::SIZE);
                let payload_len = entire_len.saturating_sub(this_len);
                let mut header_payload = vec![0u8; header_payload_len];
                let mut payload = vec![0u8; payload_len];

                if header_payload_len > 0 {
                    server_side.read_exact(&mut header_payload).await.unwrap();
                }
                if payload_len > 0 {
                    server_side.read_exact(&mut payload).await.unwrap();
                }

                match hdr.operation.get() {
                    x if x == AfcOpcode::ReadDir as u64 => {
                        let names = b"Only.log\0".to_vec();
                        let resp = AfcHeader::new(
                            hdr.packet_num.get(),
                            AfcOpcode::ReadDir,
                            0,
                            names.len(),
                        );
                        server_side.write_all(resp.as_bytes()).await.unwrap();
                        server_side.write_all(&names).await.unwrap();
                    }
                    x if x == AfcOpcode::GetFileInfo as u64 => {
                        let payload = b"st_ifmt\0S_IFREG\0st_size\0\x31\0";
                        let resp = AfcHeader::new(
                            hdr.packet_num.get(),
                            AfcOpcode::GetFileInfo,
                            0,
                            payload.len(),
                        );
                        server_side.write_all(resp.as_bytes()).await.unwrap();
                        server_side.write_all(payload).await.unwrap();
                    }
                    other => panic!("unexpected AFC opcode: {other}"),
                }
            }
        });

        let mut client = CrashReportClient::new(client_side);
        let removed = client.remove_reports(Some("*.ips")).await.unwrap();

        assert_eq!(removed, 0);
    }

    fn trim_c_string(bytes: &[u8]) -> String {
        String::from_utf8_lossy(bytes)
            .trim_end_matches('\0')
            .to_string()
    }
}
