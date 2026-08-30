//! Structured OS trace relay helpers.
//!
//! Service: `com.apple.os_trace_relay`
//! References: go-ios `ios/ostrace/ostrace.go` and pymobiledevice3
//! `services/os_trace.py`.

use std::fmt;
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Seek, SeekFrom};
use std::path::{Component, Path, PathBuf};
use std::str::FromStr;
use std::time::Duration;

use regex::Regex;
use serde::Serialize;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tokio_stream::Stream;

pub const SERVICE_NAME: &str = "com.apple.os_trace_relay";
pub const SHIM_SERVICE_NAME: &str = "com.apple.os_trace_relay.shim.remote";

const START_ACTIVITY: &str = "StartActivity";
const PID_LIST: &str = "PidList";
const CREATE_ARCHIVE: &str = "CreateArchive";
const TRACE_FRAME_MAGIC: u8 = 0x02;
const ARCHIVE_RESPONSE_MARKER: u8 = 1;
const ARCHIVE_CHUNK_MAGIC: u8 = 3;
const MAX_ARCHIVE_CHUNK_SIZE: usize = 16 * 1024 * 1024;
/// A deliberately finite default protects callers that do not set a device
/// size limit from an accidentally unbounded diagnostic stream.
pub const DEFAULT_MAX_ARCHIVE_BYTES: u64 = 4 * 1024 * 1024 * 1024;
pub const DEFAULT_MAX_ARCHIVE_FILE_BYTES: u64 = 1024 * 1024 * 1024;
pub const DEFAULT_MAX_ARCHIVE_ENTRIES: usize = 100_000;
pub const DEFAULT_MAX_EXTRACTED_BYTES: u64 = 4 * 1024 * 1024 * 1024;
const TRACE_HEADER_SIZE: usize = 129;
const MAX_PLIST_SIZE: usize = 8 * 1024 * 1024;
/// A single activity record is length-prefixed by the device. Keep malformed
/// lengths from turning a stream into an unbounded allocation while leaving
/// room for large diagnostic messages and labels.
pub const MAX_TRACE_ENTRY_SIZE: usize = 16 * 1024 * 1024;
const MAX_HANDSHAKE_LENGTH_BYTES: u32 = 8;

pub const OS_TRACE_RELAY_MESSAGE_FILTER_ALL: u16 = MessageFilter::ALL.0;
pub const OS_TRACE_RELAY_STREAM_FLAGS_DEFAULT: u32 = StreamFlags::ALL.0;

service_error!(
    OsTraceError,
    after {
        /// The stream did not produce a complete response before the deadline.
        #[error("OS trace operation timed out")]
        Timeout,
    },
);

/// Severity byte carried by an os_trace activity record.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct LogLevel(pub u8);

impl LogLevel {
    pub const DEFAULT: Self = Self(0x00);
    pub const INFO: Self = Self(0x01);
    pub const DEBUG: Self = Self(0x02);
    pub const USER_ACTION: Self = Self(0x03);
    pub const ERROR: Self = Self(0x10);
    pub const FAULT: Self = Self(0x11);

    pub const fn name(self) -> &'static str {
        match self.0 {
            0x00 => "Default",
            0x01 => "Info",
            0x02 => "Debug",
            0x03 => "UserAction",
            0x10 => "Error",
            0x11 => "Fault",
            _ => "Unknown",
        }
    }

    pub const fn value(self) -> u8 {
        self.0
    }

    pub fn display_name(self) -> String {
        match self.0 {
            0x00 => "Default".into(),
            0x01 => "Info".into(),
            0x02 => "Debug".into(),
            0x03 => "UserAction".into(),
            0x10 => "Error".into(),
            0x11 => "Fault".into(),
            value => format!("Unknown({value})"),
        }
    }
}

impl fmt::Display for LogLevel {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.display_name())
    }
}

impl FromStr for LogLevel {
    type Err = String;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value.trim().to_ascii_lowercase().as_str() {
            "default" | "notice" => Ok(Self::DEFAULT),
            "info" => Ok(Self::INFO),
            "debug" => Ok(Self::DEBUG),
            "useraction" | "user-action" | "user_action" => Ok(Self::USER_ACTION),
            "error" => Ok(Self::ERROR),
            "fault" => Ok(Self::FAULT),
            value => Err(format!(
                "unknown log level {value:?}; valid levels: default, info, debug, error, fault"
            )),
        }
    }
}

/// Device-side activity record type mask.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize)]
pub struct MessageFilter(pub u16);

impl MessageFilter {
    pub const ACTIVITY_CREATE: Self = Self(1 << 0);
    pub const ACTIVITY_TRANSITION: Self = Self(1 << 1);
    pub const LOG_MESSAGE: Self = Self(1 << 2);
    pub const SIGNPOST: Self = Self(1 << 3);
    pub const ALL: Self = Self(u16::MAX);

    pub const fn bits(self) -> u16 {
        self.0
    }
}

impl std::ops::BitOr for MessageFilter {
    type Output = Self;

    fn bitor(self, rhs: Self) -> Self::Output {
        Self(self.0 | rhs.0)
    }
}

/// Device-side activity stream flags.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize)]
pub struct StreamFlags(pub u32);

impl StreamFlags {
    pub const PROCESS_ONLY: Self = Self(0x0000_0001);
    pub const SKIP_DECODE: Self = Self(0x0000_0002);
    pub const PAYLOAD: Self = Self(0x0000_0004);
    pub const HISTORICAL: Self = Self(0x0000_0008);
    pub const CALLSTACK: Self = Self(0x0000_0010);
    pub const DEBUG: Self = Self(0x0000_0020);
    pub const NO_SENSITIVE: Self = Self(0x0000_0080);
    pub const INFO: Self = Self(0x0000_0100);
    pub const PROMISCUOUS: Self = Self(0x0000_0200);

    /// The bit used by current iOS versions to enable Info and Debug records.
    pub const ALL: Self = Self::DEBUG;

    pub const fn bits(self) -> u32 {
        self.0
    }
}

impl std::ops::BitOr for StreamFlags {
    type Output = Self;

    fn bitor(self, rhs: Self) -> Self::Output {
        Self(self.0 | rhs.0)
    }
}

/// Device-side settings plus exact client-side severity filtering.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LevelFilter {
    pub message_filter: MessageFilter,
    pub stream_flags: StreamFlags,
    pub client_levels: Vec<LogLevel>,
}

impl Default for LevelFilter {
    fn default() -> Self {
        default_level_filter()
    }
}

pub fn default_level_filter() -> LevelFilter {
    LevelFilter {
        message_filter: MessageFilter::LOG_MESSAGE,
        stream_flags: StreamFlags::ALL,
        client_levels: Vec::new(),
    }
}

/// Parse a comma-separated, case-insensitive level list.
pub fn parse_level_filter(levels: &str) -> Result<LevelFilter, String> {
    if levels.trim().is_empty() {
        return Ok(default_level_filter());
    }

    let mut client_levels = Vec::new();
    for raw in levels.split(',') {
        if raw.trim().is_empty() {
            continue;
        }
        let level = LogLevel::from_str(raw)?;
        if !client_levels.contains(&level) {
            client_levels.push(level);
        }
    }

    let stream_flags = if client_levels
        .iter()
        .any(|level| *level == LogLevel::INFO || *level == LogLevel::DEBUG)
    {
        StreamFlags::ALL
    } else {
        StreamFlags::default()
    };
    Ok(LevelFilter {
        message_filter: MessageFilter::LOG_MESSAGE,
        stream_flags,
        client_levels,
    })
}

/// Timestamp from the device's wall-clock fields.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct TraceTimestamp {
    pub seconds: u64,
    pub microseconds: u32,
}

/// Structured subsystem/category label.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct LogLabel {
    pub subsystem: String,
    pub category: String,
}

/// One decoded binary activity-stream log record.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct LogEntry {
    pub record_type: u32,
    pub pid: u32,
    pub procid: u64,
    pub process_uuid: [u8; 16],
    pub activity_id: u64,
    pub parent_activity_id: u64,
    pub timestamp: TraceTimestamp,
    pub mach_timestamp: u64,
    pub level: LogLevel,
    pub thread_id: u32,
    pub image_uuid: [u8; 16],
    pub image_name: String,
    pub image_offset: u32,
    pub filename: String,
    pub message: String,
    pub label: Option<LogLabel>,
}

/// Client-side filters. These do not reduce device traffic.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct MessageFilterSpec {
    pub levels: Vec<LogLevel>,
    pub process: Option<String>,
    pub subsystem: Option<String>,
    pub category: Option<String>,
    pub contains: Option<String>,
    pub excludes: Option<String>,
}

impl MessageFilterSpec {
    pub fn matches(&self, entry: &LogEntry) -> bool {
        self.compile().matches(entry)
    }

    /// Compile this legacy single-value filter for efficient repeated use.
    pub fn compile(&self) -> CompiledMessageFilter {
        // Keep the historical infallible API truly infallible. The bounded
        // validation belongs to the new CLI/options API; applying it here
        // would turn a caller-supplied legacy term into a panic via `expect`.
        CompiledMessageFilter {
            levels: self.levels.clone(),
            process: self.process.clone(),
            subsystem: self.subsystem.clone(),
            category: self.category.clone(),
            matches: self.contains.clone().into_iter().collect(),
            excludes: self.excludes.clone().into_iter().collect(),
            regexes: Vec::new(),
            ignore_case: false,
        }
    }
}

/// Extended client-side filters. Device-side StartActivity filters cannot
/// express these text predicates, so they are applied after bounded frame
/// parsing on the host. Repeated `matches` terms are ANDed, repeated
/// `excludes` terms are ORed (any match rejects), and repeated regex terms are
/// ORed (any match accepts); all filter groups are then ANDed together.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct MessageFilterOptions {
    pub levels: Vec<LogLevel>,
    pub process: Option<String>,
    pub subsystem: Option<String>,
    pub category: Option<String>,
    pub matches: Vec<String>,
    pub excludes: Vec<String>,
    pub regex: Vec<String>,
    pub ignore_case: bool,
}

const MAX_FILTER_TERMS: usize = 64;
const MAX_FILTER_TERM_LENGTH: usize = 4096;
const MAX_FILTER_REGEX_SIZE: usize = 4 * 1024 * 1024;

impl MessageFilterOptions {
    pub fn compile(&self) -> Result<CompiledMessageFilter, String> {
        if self.matches.len() > MAX_FILTER_TERMS
            || self.excludes.len() > MAX_FILTER_TERMS
            || self.regex.len() > MAX_FILTER_TERMS
        {
            return Err(format!(
                "too many OS trace text filters; maximum is {MAX_FILTER_TERMS} per kind"
            ));
        }
        for (kind, terms) in [
            ("match", &self.matches),
            ("exclude", &self.excludes),
            ("regex", &self.regex),
        ] {
            if let Some(term) = terms
                .iter()
                .find(|term| term.len() > MAX_FILTER_TERM_LENGTH)
            {
                return Err(format!(
                    "OS trace {kind} filter is {} bytes; maximum is {MAX_FILTER_TERM_LENGTH}",
                    term.len()
                ));
            }
        }

        let regexes = self
            .regex
            .iter()
            .map(|pattern| {
                regex::RegexBuilder::new(pattern)
                    .case_insensitive(self.ignore_case)
                    .size_limit(MAX_FILTER_REGEX_SIZE)
                    .build()
                    .map_err(|error| format!("invalid OS trace regex {pattern:?}: {error}"))
            })
            .collect::<Result<Vec<_>, _>>()?;

        let lower = |value: &str| {
            if self.ignore_case {
                value.to_lowercase()
            } else {
                value.to_owned()
            }
        };

        Ok(CompiledMessageFilter {
            levels: self.levels.clone(),
            process: self.process.as_deref().map(lower),
            subsystem: self.subsystem.as_deref().map(lower),
            category: self.category.as_deref().map(lower),
            matches: self.matches.iter().map(|term| lower(term)).collect(),
            excludes: self.excludes.iter().map(|term| lower(term)).collect(),
            regexes,
            ignore_case: self.ignore_case,
        })
    }
}

/// A compiled, reusable client-side filter. Compile before connecting so
/// invalid regexes fail locally without opening a device service.
#[derive(Debug, Clone)]
pub struct CompiledMessageFilter {
    levels: Vec<LogLevel>,
    process: Option<String>,
    subsystem: Option<String>,
    category: Option<String>,
    matches: Vec<String>,
    excludes: Vec<String>,
    regexes: Vec<Regex>,
    ignore_case: bool,
}

impl CompiledMessageFilter {
    pub fn matches(&self, entry: &LogEntry) -> bool {
        if !self.levels.is_empty() && !self.levels.contains(&entry.level) {
            return false;
        }
        if let Some(process) = &self.process {
            let process_matches = [
                entry.filename.as_str(),
                entry.image_name.as_str(),
                base_process_name(&entry.filename),
                base_process_name(&entry.image_name),
            ]
            .into_iter()
            .any(|candidate| self.text_equals(candidate, process));
            if !process_matches {
                return false;
            }
        }
        if let Some(subsystem) = &self.subsystem {
            let Some(label) = entry.label.as_ref() else {
                return false;
            };
            if !self.text_contains(&label.subsystem, subsystem) {
                return false;
            }
        }
        if let Some(category) = &self.category {
            let Some(label) = entry.label.as_ref() else {
                return false;
            };
            if !self.text_contains(&label.category, category) {
                return false;
            }
        }
        if self
            .matches
            .iter()
            .any(|term| !self.text_contains(&entry.message, term))
        {
            return false;
        }
        if self
            .excludes
            .iter()
            .any(|term| self.text_contains(&entry.message, term))
        {
            return false;
        }
        if !self.regexes.is_empty()
            && !self
                .regexes
                .iter()
                .any(|regex| regex.is_match(&entry.message))
        {
            return false;
        }
        true
    }

    fn text_contains(&self, haystack: &str, needle: &str) -> bool {
        if self.ignore_case {
            haystack.to_lowercase().contains(needle)
        } else {
            haystack.contains(needle)
        }
    }

    fn text_equals(&self, left: &str, right: &str) -> bool {
        if self.ignore_case {
            left.to_lowercase() == right
        } else {
            left == right
        }
    }
}

/// Compatibility name used by the go-ios API terminology.
pub type ClientFilter = MessageFilterSpec;

fn base_process_name(value: &str) -> &str {
    value
        .rsplit('/')
        .next()
        .unwrap_or(value)
        .split('(')
        .next()
        .filter(|part| !part.is_empty())
        .unwrap_or(value)
}

/// StartActivity parameters for a structured trace stream.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TraceOptions {
    /// `-1` asks diagnosticd for all processes.
    pub pid: i32,
    pub message_filter: MessageFilter,
    pub stream_flags: StreamFlags,
}

impl Default for TraceOptions {
    fn default() -> Self {
        let filter = default_level_filter();
        Self {
            pid: -1,
            message_filter: filter.message_filter,
            stream_flags: filter.stream_flags,
        }
    }
}

/// Parameters understood by the device-side `CreateArchive` request.
///
/// `size_limit`, `age_limit`, and `start_time` are sent verbatim using the
/// names used by Apple's relay. The remaining limits are host-side safety
/// budgets and are never sent to the device.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ArchiveOptions {
    pub size_limit: Option<u64>,
    pub age_limit: Option<u64>,
    pub start_time: Option<i64>,
    pub max_total_bytes: u64,
    pub max_file_bytes: u64,
    pub max_extracted_bytes: u64,
    pub max_entries: usize,
}

impl Default for ArchiveOptions {
    fn default() -> Self {
        Self {
            size_limit: None,
            age_limit: None,
            start_time: None,
            max_total_bytes: DEFAULT_MAX_ARCHIVE_BYTES,
            max_file_bytes: DEFAULT_MAX_ARCHIVE_FILE_BYTES,
            max_extracted_bytes: DEFAULT_MAX_EXTRACTED_BYTES,
            max_entries: DEFAULT_MAX_ARCHIVE_ENTRIES,
        }
    }
}

/// Counters for one raw archive transfer.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct ArchiveStats {
    pub bytes: u64,
    pub chunks: u64,
}

/// Counters for a collected `.logarchive` directory.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct CollectStats {
    pub bytes: u64,
    pub chunks: u64,
    pub entries: usize,
    pub extracted_bytes: u64,
}

/// Raw stream after the StartActivity handshake.
pub struct TraceStream<S> {
    stream: S,
}

/// Explicit name for callers that prefer the service prefix.
pub type OsTraceStream<S> = TraceStream<S>;

impl<S: AsyncRead + AsyncWrite + Unpin> TraceStream<S> {
    pub async fn next_entry(&mut self) -> Result<Option<LogEntry>, OsTraceError> {
        let mut magic = [0u8; 1];
        let read = self.stream.read(&mut magic).await?;
        if read == 0 {
            return Ok(None);
        }
        if magic[0] != TRACE_FRAME_MAGIC {
            return Err(OsTraceError::Protocol(format!(
                "unexpected OS trace frame magic 0x{:02x}, expected 0x{TRACE_FRAME_MAGIC:02x}",
                magic[0]
            )));
        }

        let length = usize::try_from(self.stream.read_u32_le().await?).map_err(|_| {
            OsTraceError::Protocol("OS trace frame length does not fit usize".into())
        })?;
        if length == 0 {
            return Err(OsTraceError::Protocol(
                "OS trace frame has an empty entry".into(),
            ));
        }
        if length > MAX_TRACE_ENTRY_SIZE {
            return Err(OsTraceError::Protocol(format!(
                "OS trace entry length {length} exceeds max {MAX_TRACE_ENTRY_SIZE}"
            )));
        }
        let mut data = vec![0u8; length];
        self.stream.read_exact(&mut data).await?;
        parse_entry(&data).map(Some)
    }

    pub async fn next_entry_with_timeout(
        &mut self,
        timeout: Duration,
    ) -> Result<Option<LogEntry>, OsTraceError> {
        tokio::time::timeout(timeout, self.next_entry())
            .await
            .map_err(|_| OsTraceError::Timeout)?
    }

    pub fn into_stream(self) -> impl Stream<Item = Result<LogEntry, OsTraceError>> {
        async_stream::try_stream! {
            let mut stream = self;
            while let Some(entry) = stream.next_entry().await? {
                yield entry;
            }
        }
    }

    pub async fn next_filtered(
        &mut self,
        filter: &MessageFilterSpec,
    ) -> Result<Option<LogEntry>, OsTraceError> {
        let filter = filter.compile();
        self.next_filtered_compiled(&filter).await
    }

    pub async fn next_filtered_options(
        &mut self,
        filter: &MessageFilterOptions,
    ) -> Result<Option<LogEntry>, OsTraceError> {
        let filter = filter.compile().map_err(OsTraceError::Protocol)?;
        self.next_filtered_compiled(&filter).await
    }

    pub async fn next_filtered_compiled(
        &mut self,
        filter: &CompiledMessageFilter,
    ) -> Result<Option<LogEntry>, OsTraceError> {
        loop {
            let Some(entry) = self.next_entry().await? else {
                return Ok(None);
            };
            if filter.matches(&entry) {
                return Ok(Some(entry));
            }
        }
    }
}

pub struct OsTraceClient<S> {
    stream: S,
}

impl<S: AsyncRead + AsyncWrite + Unpin> OsTraceClient<S> {
    pub fn new(stream: S) -> Self {
        Self { stream }
    }

    pub async fn get_pid_list(&mut self) -> Result<plist::Dictionary, OsTraceError> {
        let request = plist::Dictionary::from_iter([(
            "Request".to_string(),
            plist::Value::String(PID_LIST.into()),
        )]);
        send_plist(&mut self.stream, &plist::Value::Dictionary(request)).await?;

        let _marker = self.stream.read_u8().await?;
        recv_prefixed_plist(&mut self.stream).await
    }

    /// Request the device's stored diagnostics and stream the raw PAX tar
    /// bytes to `writer`. The relay terminates the service connection after
    /// the final chunk; a clean EOF at a frame boundary is success.
    pub async fn create_archive<W: AsyncWrite + Unpin>(
        &mut self,
        writer: &mut W,
        options: ArchiveOptions,
    ) -> Result<ArchiveStats, OsTraceError> {
        let mut request = plist::Dictionary::from_iter([(
            "Request".to_string(),
            plist::Value::String(CREATE_ARCHIVE.into()),
        )]);
        if let Some(size_limit) = options.size_limit {
            request.insert(
                "SizeLimit".to_string(),
                plist::Value::Integer(plist::Integer::from(size_limit)),
            );
        }
        if let Some(age_limit) = options.age_limit {
            request.insert(
                "AgeLimit".to_string(),
                plist::Value::Integer(plist::Integer::from(age_limit)),
            );
        }
        if let Some(start_time) = options.start_time {
            request.insert(
                "StartTime".to_string(),
                plist::Value::Integer(plist::Integer::from(start_time)),
            );
        }
        send_plist(&mut self.stream, &plist::Value::Dictionary(request)).await?;

        let marker = self.stream.read_u8().await?;
        if marker != ARCHIVE_RESPONSE_MARKER {
            return Err(OsTraceError::Protocol(format!(
                "OS trace CreateArchive acknowledgement was 0x{marker:02x}, expected 0x{ARCHIVE_RESPONSE_MARKER:02x}"
            )));
        }
        let response = recv_prefixed_plist(&mut self.stream).await?;
        match response.get("Status").and_then(plist::Value::as_string) {
            Some("RequestSuccessful") => {}
            Some(status) => {
                return Err(OsTraceError::Protocol(format!(
                    "OS trace CreateArchive failed with status {status:?}"
                )))
            }
            None => {
                return Err(OsTraceError::Protocol(
                    "OS trace CreateArchive response missing Status".into(),
                ))
            }
        }

        let mut stats = ArchiveStats {
            bytes: 0,
            chunks: 0,
        };
        loop {
            let mut magic = [0u8; 1];
            let read = self.stream.read(&mut magic).await?;
            if read == 0 {
                break;
            }
            if magic[0] != ARCHIVE_CHUNK_MAGIC {
                return Err(OsTraceError::Protocol(format!(
                    "unexpected OS trace archive frame magic 0x{:02x}, expected 0x{ARCHIVE_CHUNK_MAGIC:02x}",
                    magic[0]
                )));
            }
            let length = usize::try_from(self.stream.read_u32_le().await?).map_err(|_| {
                OsTraceError::Protocol("OS trace archive chunk length does not fit usize".into())
            })?;
            if length > MAX_ARCHIVE_CHUNK_SIZE {
                return Err(OsTraceError::Protocol(format!(
                    "OS trace archive chunk length {length} exceeds max {MAX_ARCHIVE_CHUNK_SIZE}"
                )));
            }
            let length_u64 = u64::try_from(length).expect("usize fits u64");
            let new_total = stats.bytes.checked_add(length_u64).ok_or_else(|| {
                OsTraceError::Protocol("OS trace archive byte count overflow".into())
            })?;
            if new_total > options.max_total_bytes {
                return Err(OsTraceError::Protocol(format!(
                    "OS trace archive exceeds max size {} bytes",
                    options.max_total_bytes
                )));
            }
            let mut remaining = length;
            let mut buffer = vec![0u8; 64 * 1024];
            while remaining != 0 {
                let read_len = remaining.min(buffer.len());
                self.stream.read_exact(&mut buffer[..read_len]).await?;
                writer.write_all(&buffer[..read_len]).await?;
                remaining -= read_len;
            }
            stats.bytes = new_total;
            stats.chunks = stats.chunks.saturating_add(1);
        }
        writer.flush().await?;
        Ok(stats)
    }

    /// Stream and atomically save a raw PAX archive to `output`.
    pub async fn archive_to_path(
        &mut self,
        output: &Path,
        options: ArchiveOptions,
    ) -> Result<ArchiveStats, OsTraceError> {
        let parent = safe_output_parent(output)?;
        let (temp_path, std_file) = create_secure_temp_file(parent, output.file_name())?;
        let mut guard = TempFileGuard::new(temp_path.clone());
        let mut file = tokio::fs::File::from_std(std_file);
        let result = self.create_archive(&mut file, options).await?;
        file.sync_all().await?;
        drop(file);
        validate_pax_archive(&temp_path, options)?;
        atomic_replace(&temp_path, output)?;
        guard.disarm();
        Ok(result)
    }

    /// Fetch and safely extract a raw PAX archive into a new `.logarchive`
    /// directory. Extraction is staged and the destination is installed only
    /// after every entry has passed the path/type/size checks.
    pub async fn collect(
        &mut self,
        output: &Path,
        options: ArchiveOptions,
    ) -> Result<CollectStats, OsTraceError> {
        let parent = safe_output_parent(output)?;
        if fs::symlink_metadata(output).is_ok() {
            return Err(OsTraceError::Protocol(format!(
                "OS trace collect output already exists: {}",
                output.display()
            )));
        }
        let staging_path = create_secure_temp_dir(parent, output.file_name())?;
        let mut staging = TempDirGuard::new(staging_path.clone());
        let (tar_path, tar_file) =
            create_secure_temp_file(parent, Some(std::ffi::OsStr::new("os-trace.tar")))?;
        let mut tar_guard = TempFileGuard::new(tar_path.clone());
        let mut file = tokio::fs::File::from_std(tar_file);
        let archive = self.create_archive(&mut file, options).await?;
        file.sync_all().await?;
        drop(file);
        validate_pax_archive(&tar_path, options)?;

        // Keep parsing and extraction in this operation future. That way a
        // cancellation drops the cleanup guards without a detached task
        // racing the staging-directory removal.
        let extraction = extract_archive(&tar_path, &staging_path, options)?;
        atomic_replace(&staging_path, output)?;
        staging.disarm();
        tar_guard.remove_now();
        Ok(CollectStats {
            bytes: archive.bytes,
            chunks: archive.chunks,
            entries: extraction.entries,
            extracted_bytes: extraction.bytes,
        })
    }

    /// Start a structured activity stream and consume this client's transport.
    pub async fn start_activity(
        self,
        options: TraceOptions,
    ) -> Result<TraceStream<S>, OsTraceError> {
        let mut stream = self.stream;
        let request = plist::Dictionary::from_iter([
            (
                "Request".to_string(),
                plist::Value::String(START_ACTIVITY.into()),
            ),
            (
                "MessageFilter".to_string(),
                plist::Value::Integer(plist::Integer::from(options.message_filter.bits())),
            ),
            (
                "Pid".to_string(),
                plist::Value::Integer(plist::Integer::from(i64::from(options.pid))),
            ),
            (
                "StreamFlags".to_string(),
                plist::Value::Integer(plist::Integer::from(options.stream_flags.bits())),
            ),
        ]);
        send_plist(&mut stream, &plist::Value::Dictionary(request)).await?;
        read_start_activity_handshake(&mut stream).await?;
        Ok(TraceStream { stream })
    }

    /// Alias for callers that name the operation after the returned stream.
    pub async fn start_trace(self, options: TraceOptions) -> Result<TraceStream<S>, OsTraceError> {
        self.start_activity(options).await
    }

    /// StartActivity with a bounded handshake. A timeout cancels the pending
    /// read and the caller should drop the returned client/transport.
    pub async fn start_activity_with_timeout(
        self,
        options: TraceOptions,
        timeout: Duration,
    ) -> Result<TraceStream<S>, OsTraceError> {
        tokio::time::timeout(timeout, self.start_activity(options))
            .await
            .map_err(|_| OsTraceError::Timeout)?
    }
}

async fn send_plist<S: AsyncWrite + Unpin>(
    stream: &mut S,
    value: &plist::Value,
) -> Result<(), OsTraceError> {
    let mut buf = Vec::new();
    plist::to_writer_xml(&mut buf, value)?;
    let length = u32::try_from(buf.len()).map_err(|_| {
        OsTraceError::Protocol(format!("request plist length {} exceeds u32", buf.len()))
    })?;
    if buf.len() > MAX_PLIST_SIZE {
        return Err(OsTraceError::Protocol(format!(
            "request plist length {} exceeds max {MAX_PLIST_SIZE}",
            buf.len()
        )));
    }
    stream.write_all(&length.to_be_bytes()).await?;
    stream.write_all(&buf).await?;
    stream.flush().await?;
    Ok(())
}

async fn recv_prefixed_plist<S: AsyncRead + Unpin>(
    stream: &mut S,
) -> Result<plist::Dictionary, OsTraceError> {
    let len = stream.read_u32().await? as usize;
    if len > MAX_PLIST_SIZE {
        return Err(OsTraceError::Protocol(format!(
            "plist length {len} exceeds max {MAX_PLIST_SIZE}"
        )));
    }

    let mut buf = vec![0u8; len];
    stream.read_exact(&mut buf).await?;
    let value: plist::Value = plist::from_bytes(&buf)?;
    value
        .into_dictionary()
        .ok_or_else(|| OsTraceError::Protocol("OS trace response was not a dictionary".into()))
}

fn safe_output_parent(output: &Path) -> Result<&Path, OsTraceError> {
    let parent = output
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    let mut current = PathBuf::new();
    // A Windows prefix alone (`\\?\C:` from a canonicalized verbatim path)
    // is not a complete root and rejects metadata queries with
    // ERROR_INVALID_FUNCTION; it can never be a symlink, so the prefix is
    // accumulated without a metadata check and the walk resumes once the
    // root separator or a real component is appended.
    for component in parent.components() {
        match component {
            Component::Prefix(prefix) => {
                current.push(prefix.as_os_str());
                continue;
            }
            Component::RootDir => current.push(component.as_os_str()),
            Component::CurDir => {
                if current.as_os_str().is_empty() {
                    current.push(".");
                }
            }
            Component::Normal(part) => current.push(part),
            Component::ParentDir => {
                return Err(OsTraceError::Protocol(format!(
                    "OS trace output parent contains '..': {}",
                    parent.display()
                )))
            }
        }
        let metadata = fs::symlink_metadata(&current)?;
        if metadata.file_type().is_symlink() && !is_approved_system_alias(&current) {
            return Err(OsTraceError::Protocol(format!(
                "OS trace output parent contains a symlink: {}",
                current.display()
            )));
        }
        if !metadata.is_dir() {
            return Err(OsTraceError::Protocol(format!(
                "OS trace output parent is not a directory: {}",
                current.display()
            )));
        }
    }
    Ok(parent)
}

/// macOS exposes common system directories through root-owned aliases into
/// `/private`.  Accept only those exact aliases; user-controlled symlinked
/// output parents remain rejected by `safe_output_parent`.
fn is_approved_system_alias(path: &Path) -> bool {
    #[cfg(target_os = "macos")]
    {
        let target = match fs::read_link(path) {
            Ok(target) => target,
            Err(_) => return false,
        };
        return [
            (Path::new("/var"), Path::new("private/var")),
            (Path::new("/tmp"), Path::new("private/tmp")),
            (Path::new("/etc"), Path::new("private/etc")),
        ]
        .into_iter()
        .any(|(alias, expected)| path == alias && target == expected);
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = path;
        false
    }
}

fn temp_stem(name: Option<&std::ffi::OsStr>, suffix: &str) -> String {
    let name = name
        .map(|value| value.to_string_lossy())
        .filter(|value| !value.is_empty())
        .unwrap_or(std::borrow::Cow::Borrowed("archive"));
    format!(".{name}.{suffix}.{}", uuid::Uuid::new_v4())
}

fn create_secure_temp_file(
    parent: &Path,
    name: Option<&std::ffi::OsStr>,
) -> Result<(PathBuf, File), OsTraceError> {
    for _ in 0..4 {
        let path = parent.join(temp_stem(name, "tmp"));
        let mut options = OpenOptions::new();
        options.write(true).read(true).create_new(true);
        #[cfg(unix)]
        std::os::unix::fs::OpenOptionsExt::mode(&mut options, 0o600);
        match options.open(&path) {
            Ok(file) => return Ok((path, file)),
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(error.into()),
        }
    }
    Err(OsTraceError::Protocol(
        "unable to create a unique OS trace temporary file".into(),
    ))
}

fn create_secure_temp_dir(
    parent: &Path,
    name: Option<&std::ffi::OsStr>,
) -> Result<PathBuf, OsTraceError> {
    for _ in 0..4 {
        let path = parent.join(temp_stem(name, "staging"));
        #[cfg(unix)]
        let mut builder = fs::DirBuilder::new();
        #[cfg(not(unix))]
        let builder = fs::DirBuilder::new();
        #[cfg(unix)]
        std::os::unix::fs::DirBuilderExt::mode(&mut builder, 0o700);
        match builder.create(&path) {
            Ok(()) => return Ok(path),
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(error.into()),
        }
    }
    Err(OsTraceError::Protocol(
        "unable to create a unique OS trace staging directory".into(),
    ))
}

struct TempFileGuard {
    path: Option<PathBuf>,
}

impl TempFileGuard {
    fn new(path: PathBuf) -> Self {
        Self { path: Some(path) }
    }

    fn disarm(&mut self) {
        self.path = None;
    }

    fn remove_now(&mut self) {
        if let Some(path) = self.path.take() {
            if fs::remove_file(&path).is_err() {
                self.path = Some(path);
            }
        }
    }
}

impl Drop for TempFileGuard {
    fn drop(&mut self) {
        if let Some(path) = self.path.take() {
            let _ = fs::remove_file(path);
        }
    }
}

struct TempDirGuard {
    path: Option<PathBuf>,
}

impl TempDirGuard {
    fn new(path: PathBuf) -> Self {
        Self { path: Some(path) }
    }

    fn disarm(&mut self) {
        self.path = None;
    }
}

impl Drop for TempDirGuard {
    fn drop(&mut self) {
        if let Some(path) = self.path.take() {
            let _ = fs::remove_dir_all(path);
        }
    }
}

fn atomic_replace(source: &Path, destination: &Path) -> Result<(), OsTraceError> {
    let _ = safe_output_parent(destination)?;
    if let Ok(metadata) = fs::symlink_metadata(destination) {
        if metadata.file_type().is_symlink() {
            return Err(OsTraceError::Protocol(format!(
                "OS trace output is a symlink: {}",
                destination.display()
            )));
        }
    }
    crate::fs_replace::move_file_replace(source, destination).map_err(OsTraceError::from)
}

fn validate_pax_archive(path: &Path, options: ArchiveOptions) -> Result<(), OsTraceError> {
    let mut file = File::open(path)?;
    let mut header = [0u8; 512];
    file.read_exact(&mut header).map_err(|error| {
        OsTraceError::Protocol(format!(
            "OS trace archive is shorter than one tar header: {error}"
        ))
    })?;
    let magic = &header[257..263];
    if magic != b"ustar\0" && magic != b"ustar " {
        return Err(OsTraceError::Protocol(
            "OS trace archive does not have a PAX/ustar header".into(),
        ));
    }
    file.seek(SeekFrom::Start(0))?;
    let mut archive = tar::Archive::new(file);
    let mut entries = 0usize;
    let mut bytes = 0u64;
    for item in archive.entries()? {
        let mut entry = item.map_err(|error| {
            OsTraceError::Protocol(format!("invalid OS trace tar entry: {error}"))
        })?;
        entries = entries.checked_add(1).ok_or_else(|| {
            OsTraceError::Protocol("OS trace archive entry count overflow".into())
        })?;
        if entries > options.max_entries {
            return Err(OsTraceError::Protocol(format!(
                "OS trace archive has more than {} entries",
                options.max_entries
            )));
        }
        bytes = bytes.checked_add(entry.size()).ok_or_else(|| {
            OsTraceError::Protocol("OS trace archive extracted size overflow".into())
        })?;
        if entry.size() > options.max_file_bytes {
            return Err(OsTraceError::Protocol(format!(
                "OS trace archive file exceeds {} bytes",
                options.max_file_bytes
            )));
        }
        if bytes > options.max_extracted_bytes {
            return Err(OsTraceError::Protocol(format!(
                "OS trace archive extracted size exceeds {} bytes",
                options.max_extracted_bytes
            )));
        }
        std::io::copy(&mut entry, &mut std::io::sink())?;
    }
    Ok(())
}

struct ExtractionStats {
    entries: usize,
    bytes: u64,
}

fn validate_entry_path(path: &Path) -> Result<(), OsTraceError> {
    if path.as_os_str().to_string_lossy().contains('\\') {
        return Err(OsTraceError::Protocol(format!(
            "OS trace archive entry uses a backslash path separator: {}",
            path.display()
        )));
    }
    for component in path.components() {
        match component {
            Component::Normal(_) | Component::CurDir => {}
            Component::Prefix(_) | Component::RootDir | Component::ParentDir => {
                return Err(OsTraceError::Protocol(format!(
                    "OS trace archive entry escapes extraction root: {}",
                    path.display()
                )))
            }
        }
    }
    Ok(())
}

fn extract_archive(
    tar_path: &Path,
    output: &Path,
    options: ArchiveOptions,
) -> Result<ExtractionStats, OsTraceError> {
    let mut file = File::open(tar_path)?;
    let mut archive = tar::Archive::new(&mut file);
    let mut entries = 0usize;
    let mut bytes = 0u64;
    for item in archive.entries()? {
        let mut entry = item.map_err(|error| {
            OsTraceError::Protocol(format!("invalid OS trace tar entry: {error}"))
        })?;
        entries = entries.checked_add(1).ok_or_else(|| {
            OsTraceError::Protocol("OS trace archive entry count overflow".into())
        })?;
        if entries > options.max_entries {
            return Err(OsTraceError::Protocol(format!(
                "OS trace archive has more than {} entries",
                options.max_entries
            )));
        }
        let path = entry.path()?.into_owned();
        validate_entry_path(&path)?;
        let entry_type = entry.header().entry_type();
        if entry_type.is_symlink() || entry_type.is_hard_link() {
            return Err(OsTraceError::Protocol(format!(
                "OS trace archive refuses link entry: {}",
                path.display()
            )));
        }
        if !entry_type.is_file() && !entry_type.is_dir() {
            return Err(OsTraceError::Protocol(format!(
                "OS trace archive refuses special entry: {}",
                path.display()
            )));
        }
        bytes = bytes.checked_add(entry.size()).ok_or_else(|| {
            OsTraceError::Protocol("OS trace archive extracted size overflow".into())
        })?;
        if entry.size() > options.max_file_bytes {
            return Err(OsTraceError::Protocol(format!(
                "OS trace archive file exceeds {} bytes",
                options.max_file_bytes
            )));
        }
        if bytes > options.max_extracted_bytes {
            return Err(OsTraceError::Protocol(format!(
                "OS trace archive extracted size exceeds {} bytes",
                options.max_extracted_bytes
            )));
        }
        std::io::copy(&mut entry, &mut std::io::sink())?;
    }

    let file = File::open(tar_path)?;
    let mut archive = tar::Archive::new(file);
    archive.unpack(output).map_err(|error| {
        OsTraceError::Protocol(format!("failed to extract OS trace archive: {error}"))
    })?;
    lockdown_extracted_tree(output)?;
    Ok(ExtractionStats { entries, bytes })
}

/// Keep the extracted diagnostic tree private regardless of modes supplied by
/// the device archive. The archive's paths and entry types were validated in
/// the first pass; this second pass only tightens permissions and refuses a
/// filesystem object that was not part of the approved regular-file/directory
/// set before the staging tree is installed.
fn lockdown_extracted_tree(root: &Path) -> Result<(), OsTraceError> {
    let metadata = fs::symlink_metadata(root)?;
    if metadata.file_type().is_symlink() {
        return Err(OsTraceError::Protocol(format!(
            "OS trace extracted root is a symlink: {}",
            root.display()
        )));
    }
    if metadata.is_dir() {
        set_private_mode(root, 0o700)?;
        for child in fs::read_dir(root)? {
            let child = child?.path();
            lockdown_extracted_tree(&child)?;
        }
        return Ok(());
    }
    if metadata.is_file() {
        set_private_mode(root, 0o600)?;
        return Ok(());
    }
    Err(OsTraceError::Protocol(format!(
        "OS trace extraction produced a special file: {}",
        root.display()
    )))
}

fn set_private_mode(path: &Path, mode: u32) -> Result<(), OsTraceError> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut permissions = fs::metadata(path)?.permissions();
        permissions.set_mode(mode);
        fs::set_permissions(path, permissions)?;
    }
    #[cfg(windows)]
    {
        // Windows has no Unix mode bits; the staging directory is private to
        // the current user by creation policy and ACL inheritance.
        let _ = (path, mode);
    }
    Ok(())
}

async fn read_start_activity_handshake<S: AsyncRead + Unpin>(
    stream: &mut S,
) -> Result<(), OsTraceError> {
    let length_length = stream.read_u32_le().await?;
    if !(1..=MAX_HANDSHAKE_LENGTH_BYTES).contains(&length_length) {
        return Err(OsTraceError::Protocol(format!(
            "invalid OS trace handshake length-of-length {length_length}"
        )));
    }

    let mut length_bytes = vec![0u8; length_length as usize];
    stream.read_exact(&mut length_bytes).await?;
    length_bytes.reverse();
    let plist_len = length_bytes
        .into_iter()
        .fold(0u64, |value, byte| (value << 8) | u64::from(byte));
    let plist_len = usize::try_from(plist_len).map_err(|_| {
        OsTraceError::Protocol(format!(
            "OS trace handshake plist length {plist_len} is too large"
        ))
    })?;
    if plist_len > MAX_PLIST_SIZE {
        return Err(OsTraceError::Protocol(format!(
            "OS trace handshake plist length {plist_len} exceeds max {MAX_PLIST_SIZE}"
        )));
    }

    let mut response = vec![0u8; plist_len];
    stream.read_exact(&mut response).await?;
    let response: plist::Value = plist::from_bytes(&response)?;
    let response = response.into_dictionary().ok_or_else(|| {
        OsTraceError::Protocol("OS trace StartActivity response was not a dictionary".into())
    })?;
    match response.get("Status").and_then(plist::Value::as_string) {
        Some("RequestSuccessful") => Ok(()),
        Some(status) => Err(OsTraceError::Protocol(format!(
            "OS trace StartActivity failed with status {status:?}"
        ))),
        None => Err(OsTraceError::Protocol(
            "OS trace StartActivity response missing Status".into(),
        )),
    }
}

pub fn parse_entry(data: &[u8]) -> Result<LogEntry, OsTraceError> {
    if data.len() < TRACE_HEADER_SIZE {
        return Err(OsTraceError::Protocol(format!(
            "OS trace entry too short: {} bytes, need at least {TRACE_HEADER_SIZE}",
            data.len()
        )));
    }

    let record_type = read_u32(data, 1, "record type")?;
    let _header_size = read_u32(data, 5, "header size")?;
    let pid = read_u32(data, 9, "pid")?;
    if pid > 999_999 {
        return Err(OsTraceError::Protocol(format!(
            "OS trace pid {pid} exceeds sanity limit"
        )));
    }
    let procid = read_u64(data, 13, "procid")?;
    let process_uuid = read_array::<16>(data, 21, "process UUID")?;
    let procpath_len = usize::from(read_u16(data, 37, "process path length")?);
    let activity_id = read_u64(data, 39, "activity ID")?;
    let parent_activity_id = read_u64(data, 47, "parent activity ID")?;
    let seconds = read_u64(data, 55, "timestamp seconds")?;
    let microseconds = read_u32(data, 63, "timestamp microseconds")?;
    if microseconds >= 1_000_000 {
        return Err(OsTraceError::Protocol(format!(
            "OS trace timestamp microseconds out of range: {microseconds}"
        )));
    }
    let level = LogLevel(
        *data
            .get(68)
            .ok_or_else(|| OsTraceError::Protocol("OS trace entry missing level".into()))?,
    );
    let mach_timestamp = read_u64(data, 75, "mach timestamp")?;
    let thread_id = read_u32(data, 83, "thread ID")?;
    let image_uuid = read_array::<16>(data, 91, "image UUID")?;
    let imagepath_len = usize::from(read_u16(data, 107, "image path length")?);
    let message_len = usize::try_from(read_u32(data, 109, "message length")?)
        .map_err(|_| OsTraceError::Protocol("OS trace message length does not fit usize".into()))?;
    let image_offset = read_u32(data, 113, "image offset")?;
    let subsystem_len = usize::from(read_u16(data, 117, "subsystem length")?);
    let category_len = usize::from(read_u16(data, 121, "category length")?);

    let mut offset = TRACE_HEADER_SIZE;
    let filename = read_string_field(data, &mut offset, procpath_len, "process path")?;
    let image_name = read_string_field(data, &mut offset, imagepath_len, "image path")?;
    let message = read_string_field(data, &mut offset, message_len, "message")?;
    let label = if subsystem_len > 0 && category_len > 0 {
        Some(LogLabel {
            subsystem: read_string_field(data, &mut offset, subsystem_len, "subsystem")?,
            category: read_string_field(data, &mut offset, category_len, "category")?,
        })
    } else {
        None
    };

    Ok(LogEntry {
        record_type,
        pid,
        procid,
        process_uuid,
        activity_id,
        parent_activity_id,
        timestamp: TraceTimestamp {
            seconds,
            microseconds,
        },
        mach_timestamp,
        level,
        thread_id,
        image_uuid,
        image_name,
        image_offset,
        filename,
        message,
        label,
    })
}

/// Descriptive alias for callers parsing a captured activity record.
pub fn parse_log_entry(data: &[u8]) -> Result<LogEntry, OsTraceError> {
    parse_entry(data)
}

fn read_u16(data: &[u8], offset: usize, field: &str) -> Result<u16, OsTraceError> {
    let bytes = data.get(offset..offset + 2).ok_or_else(|| {
        OsTraceError::Protocol(format!("OS trace entry truncated while reading {field}"))
    })?;
    Ok(u16::from_le_bytes([bytes[0], bytes[1]]))
}

fn read_u32(data: &[u8], offset: usize, field: &str) -> Result<u32, OsTraceError> {
    let bytes = data.get(offset..offset + 4).ok_or_else(|| {
        OsTraceError::Protocol(format!("OS trace entry truncated while reading {field}"))
    })?;
    Ok(u32::from_le_bytes(
        bytes.try_into().expect("four-byte slice"),
    ))
}

fn read_u64(data: &[u8], offset: usize, field: &str) -> Result<u64, OsTraceError> {
    let bytes = data.get(offset..offset + 8).ok_or_else(|| {
        OsTraceError::Protocol(format!("OS trace entry truncated while reading {field}"))
    })?;
    Ok(u64::from_le_bytes(
        bytes.try_into().expect("eight-byte slice"),
    ))
}

fn read_array<const N: usize>(
    data: &[u8],
    offset: usize,
    field: &str,
) -> Result<[u8; N], OsTraceError> {
    data.get(offset..offset + N)
        .and_then(|bytes| bytes.try_into().ok())
        .ok_or_else(|| {
            OsTraceError::Protocol(format!("OS trace entry truncated while reading {field}"))
        })
}

fn read_string_field(
    data: &[u8],
    offset: &mut usize,
    length: usize,
    field: &str,
) -> Result<String, OsTraceError> {
    let end = offset
        .checked_add(length)
        .ok_or_else(|| OsTraceError::Protocol(format!("OS trace {field} length overflow")))?;
    let bytes = data.get(*offset..end).ok_or_else(|| {
        OsTraceError::Protocol(format!("OS trace entry truncated while reading {field}"))
    })?;
    *offset = end;
    let bytes = bytes.strip_suffix(&[0]).unwrap_or(bytes);
    Ok(String::from_utf8_lossy(bytes).into_owned())
}

#[cfg(test)]
mod tests {
    use tokio::io::AsyncWriteExt;
    use tokio_stream::StreamExt;

    use super::*;

    // macOS exposes its temporary directory through the `/var` system
    // symlink.  The output policy intentionally rejects symlinked parents, so
    // fixtures use the canonical spelling while still exercising the same
    // checks as callers on every host.
    fn test_temp_dir() -> PathBuf {
        std::fs::canonicalize(std::env::temp_dir()).expect("canonical temp directory")
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn safe_output_parent_accepts_macos_temp_alias() {
        let output = std::env::temp_dir().join(format!(
            "ios-trace-macos-alias-{}-{}",
            std::process::id(),
            uuid::Uuid::new_v4().simple()
        ));
        safe_output_parent(&output).expect("macOS temp alias");
    }

    fn build_entry(level: LogLevel, message: &str) -> Vec<u8> {
        let filename = "/usr/lib/Test(Helper)\0";
        let image = "Test\0";
        let subsystem = "com.example\0";
        let category = "network\0";
        let message = format!("{message}\0");
        let mut data = vec![0u8; TRACE_HEADER_SIZE];
        data[1..5].copy_from_slice(&0x0400u32.to_le_bytes());
        data[9..13].copy_from_slice(&1234u32.to_le_bytes());
        data[13..21].copy_from_slice(&77u64.to_le_bytes());
        data[37..39].copy_from_slice(&(filename.len() as u16).to_le_bytes());
        data[39..47].copy_from_slice(&9u64.to_le_bytes());
        data[47..55].copy_from_slice(&8u64.to_le_bytes());
        data[55..63].copy_from_slice(&1_705_312_200u64.to_le_bytes());
        data[63..67].copy_from_slice(&500_000u32.to_le_bytes());
        data[68] = level.0;
        data[75..83].copy_from_slice(&999u64.to_le_bytes());
        data[83..87].copy_from_slice(&4567u32.to_le_bytes());
        data[107..109].copy_from_slice(&(image.len() as u16).to_le_bytes());
        data[109..113].copy_from_slice(&(message.len() as u32).to_le_bytes());
        data[113..117].copy_from_slice(&0x1234u32.to_le_bytes());
        data[117..119].copy_from_slice(&(subsystem.len() as u16).to_le_bytes());
        data[121..123].copy_from_slice(&(category.len() as u16).to_le_bytes());
        data.extend_from_slice(filename.as_bytes());
        data.extend_from_slice(image.as_bytes());
        data.extend_from_slice(message.as_bytes());
        data.extend_from_slice(subsystem.as_bytes());
        data.extend_from_slice(category.as_bytes());
        data
    }

    fn frame(data: &[u8]) -> Vec<u8> {
        let mut frame = vec![TRACE_FRAME_MAGIC];
        frame.extend_from_slice(&(data.len() as u32).to_le_bytes());
        frame.extend_from_slice(data);
        frame
    }

    #[test]
    fn parses_structured_entry_and_unicode() {
        let data = build_entry(LogLevel::ERROR, "こんにちは 👋");
        let entry = parse_entry(&data).unwrap();
        assert_eq!(entry.pid, 1234);
        assert_eq!(entry.procid, 77);
        assert_eq!(entry.level, LogLevel::ERROR);
        assert_eq!(entry.thread_id, 4567);
        assert_eq!(entry.filename, "/usr/lib/Test(Helper)");
        assert_eq!(entry.image_name, "Test");
        assert_eq!(entry.message, "こんにちは 👋");
        assert_eq!(entry.label.as_ref().unwrap().category, "network");
    }

    #[test]
    fn parses_missing_label_and_unknown_record_type() {
        let mut data = build_entry(LogLevel(0xff), "hello");
        data[1..5].copy_from_slice(&0x9900u32.to_le_bytes());
        data[117..119].copy_from_slice(&0u16.to_le_bytes());
        data[121..123].copy_from_slice(&0u16.to_le_bytes());
        let entry = parse_entry(&data).unwrap();
        assert_eq!(entry.record_type, 0x9900);
        assert_eq!(entry.level.display_name(), "Unknown(255)");
        assert!(entry.label.is_none());
    }

    #[test]
    fn rejects_malformed_lengths_without_panicking() {
        let mut data = build_entry(LogLevel::INFO, "x");
        data[109..113].copy_from_slice(&u32::MAX.to_le_bytes());
        let error = parse_entry(&data).unwrap_err();
        assert!(error.to_string().contains("message"));
        assert!(parse_entry(&[0u8; TRACE_HEADER_SIZE - 1]).is_err());
    }

    #[test]
    fn parses_level_filter_case_insensitively_and_deduplicates() {
        let filter = parse_level_filter("Error, INFO, error").unwrap();
        assert_eq!(filter.message_filter, MessageFilter::LOG_MESSAGE);
        assert_eq!(filter.stream_flags, StreamFlags::ALL);
        assert_eq!(filter.client_levels, vec![LogLevel::ERROR, LogLevel::INFO]);
        assert!(parse_level_filter("bogus").is_err());
    }

    #[test]
    fn client_filter_matches_process_category_and_text() {
        let entry = parse_entry(&build_entry(LogLevel::ERROR, "connection timeout")).unwrap();
        let filter = MessageFilterSpec {
            levels: vec![LogLevel::ERROR],
            process: Some("Test".into()),
            subsystem: Some("com.example".into()),
            category: Some("network".into()),
            contains: Some("timeout".into()),
            excludes: Some("success".into()),
        };
        assert!(filter.matches(&entry));
        assert!(!MessageFilterSpec {
            excludes: Some("timeout".into()),
            ..filter
        }
        .matches(&entry));
    }

    #[test]
    fn legacy_filter_large_term_does_not_panic() {
        let entry = parse_entry(&build_entry(LogLevel::INFO, "x")).unwrap();
        let filter = MessageFilterSpec {
            contains: Some("x".repeat(MAX_FILTER_TERM_LENGTH + 1)),
            ..Default::default()
        };
        assert!(!filter.matches(&entry));
    }

    #[test]
    fn extended_filter_combines_unicode_terms_and_regexes() {
        let entry = parse_entry(&build_entry(LogLevel::ERROR, "Connection TIMEOUT 世界")).unwrap();
        let options = MessageFilterOptions {
            levels: vec![LogLevel::ERROR],
            matches: vec!["connection".into(), "世界".into()],
            excludes: vec!["blocked".into(), "denied".into()],
            regex: vec!["never-matches".into(), "世界".into()],
            ignore_case: true,
            ..Default::default()
        };
        let compiled = options.compile().unwrap();
        assert!(compiled.matches(&entry));

        let mut denied = options;
        denied.excludes = vec!["世界".into()];
        assert!(!denied.compile().unwrap().matches(&entry));
    }

    #[test]
    fn extended_filter_rejects_invalid_regex_before_streaming() {
        let options = MessageFilterOptions {
            regex: vec!["[".into()],
            ..Default::default()
        };
        let error = options.compile().unwrap_err();
        assert!(error.to_string().contains("invalid OS trace regex"));
    }

    #[test]
    fn extended_filter_limits_client_work() {
        let too_many = MessageFilterOptions {
            matches: vec![String::from("x"); MAX_FILTER_TERMS + 1],
            ..Default::default()
        };
        assert!(too_many.compile().unwrap_err().contains("too many"));

        let too_long = MessageFilterOptions {
            regex: vec!["x".repeat(MAX_FILTER_TERM_LENGTH + 1)],
            ..Default::default()
        };
        assert!(too_long.compile().unwrap_err().contains("maximum"));
    }

    #[tokio::test]
    async fn start_activity_writes_exact_request_and_reads_fragmented_frame() {
        let (client, mut server) = tokio::io::duplex(32 * 1024);
        let data = build_entry(LogLevel::INFO, "fragmented");
        let response = plist::Value::Dictionary(plist::Dictionary::from_iter([(
            "Status".to_string(),
            plist::Value::String("RequestSuccessful".into()),
        )]));
        let mut response_bytes = Vec::new();
        plist::to_writer_xml(&mut response_bytes, &response).unwrap();

        let server_task = tokio::spawn(async move {
            let mut prefix = [0u8; 4];
            server.read_exact(&mut prefix).await.unwrap();
            let request_len = u32::from_be_bytes(prefix) as usize;
            let mut request = vec![0u8; request_len];
            server.read_exact(&mut request).await.unwrap();
            let request: plist::Value = plist::from_bytes(&request).unwrap();
            let dict = request.as_dictionary().unwrap();
            assert_eq!(
                dict.get("Request").and_then(plist::Value::as_string),
                Some(START_ACTIVITY)
            );
            assert_eq!(
                dict.get("Pid").and_then(plist::Value::as_signed_integer),
                Some(-1)
            );
            assert_eq!(
                dict.get("MessageFilter")
                    .and_then(plist::Value::as_unsigned_integer),
                Some(u64::from(MessageFilter::LOG_MESSAGE.bits()))
            );

            server.write_all(&4u32.to_le_bytes()).await.unwrap();
            server
                .write_all(&[response_bytes.len() as u8, 0, 0, 0])
                .await
                .unwrap();
            server.write_all(&response_bytes).await.unwrap();
            let frame = frame(&data);
            for chunk in frame.chunks(3) {
                server.write_all(chunk).await.unwrap();
            }
        });

        let stream = OsTraceClient::new(client)
            .start_activity(TraceOptions::default())
            .await
            .unwrap();
        let mut stream = stream;
        let entry = stream.next_entry().await.unwrap().unwrap();
        assert_eq!(entry.message, "fragmented");
        assert!(stream.next_entry().await.unwrap().is_none());
        server_task.await.unwrap();
    }

    #[tokio::test]
    async fn stream_rejects_empty_and_oversized_frames() {
        let (client, mut server) = tokio::io::duplex(1024);
        server
            .write_all(&[TRACE_FRAME_MAGIC, 0, 0, 0, 0])
            .await
            .unwrap();
        let mut stream = TraceStream { stream: client };
        assert!(stream
            .next_entry()
            .await
            .unwrap_err()
            .to_string()
            .contains("empty"));

        let (client, mut server) = tokio::io::duplex(1024);
        server
            .write_all(&[
                TRACE_FRAME_MAGIC,
                (MAX_TRACE_ENTRY_SIZE as u32 + 1) as u8,
                ((MAX_TRACE_ENTRY_SIZE as u32 + 1) >> 8) as u8,
                0,
                1,
            ])
            .await
            .unwrap();
        let mut stream = TraceStream { stream: client };
        assert!(stream
            .next_entry()
            .await
            .unwrap_err()
            .to_string()
            .contains("exceeds max"));
    }

    #[tokio::test]
    async fn stream_reports_bad_magic_partial_frame_and_timeout() {
        let (client, mut server) = tokio::io::duplex(1024);
        server.write_all(&[0x99]).await.unwrap();
        let mut stream = TraceStream { stream: client };
        assert!(stream
            .next_entry()
            .await
            .unwrap_err()
            .to_string()
            .contains("magic"));

        let (client, mut server) = tokio::io::duplex(1024);
        server.write_all(&[TRACE_FRAME_MAGIC, 1]).await.unwrap();
        let mut stream = TraceStream { stream: client };
        assert!(matches!(
            stream
                .next_entry_with_timeout(Duration::from_millis(20))
                .await,
            Err(OsTraceError::Timeout)
        ));

        let (client, _server) = tokio::io::duplex(1024);
        let mut stream = TraceStream { stream: client };
        assert!(matches!(
            stream
                .next_entry_with_timeout(Duration::from_millis(1))
                .await,
            Err(OsTraceError::Timeout)
        ));
    }

    #[tokio::test]
    async fn start_activity_with_timeout_cancels_stalled_handshake() {
        let (client, _server) = tokio::io::duplex(4096);
        let result = OsTraceClient::new(client)
            .start_activity_with_timeout(TraceOptions::default(), Duration::from_millis(1))
            .await;
        assert!(matches!(result, Err(OsTraceError::Timeout)));
    }

    #[tokio::test]
    async fn handshake_rejects_malicious_length_and_reports_device_error() {
        let (mut client, mut server) = tokio::io::duplex(1024);
        server.write_all(&0u32.to_le_bytes()).await.unwrap();
        let error = read_start_activity_handshake(&mut client)
            .await
            .unwrap_err();
        assert!(error.to_string().contains("length-of-length"));

        let (mut client, mut server) = tokio::io::duplex(1024);
        let response = plist::Value::Dictionary(plist::Dictionary::from_iter([(
            "Status".to_string(),
            plist::Value::String("RequestDenied".into()),
        )]));
        let mut response_bytes = Vec::new();
        plist::to_writer_xml(&mut response_bytes, &response).unwrap();
        server.write_all(&1u32.to_le_bytes()).await.unwrap();
        server
            .write_all(&[u8::try_from(response_bytes.len()).unwrap()])
            .await
            .unwrap();
        server.write_all(&response_bytes).await.unwrap();
        let error = read_start_activity_handshake(&mut client)
            .await
            .unwrap_err();
        assert!(error.to_string().contains("RequestDenied"));
    }

    #[tokio::test]
    async fn into_stream_yields_entries_until_clean_eof() {
        let (client, mut server) = tokio::io::duplex(4096);
        server
            .write_all(&frame(&build_entry(LogLevel::DEFAULT, "one")))
            .await
            .unwrap();
        drop(server);
        let stream = TraceStream { stream: client }.into_stream();
        let entries: Vec<_> = stream.collect().await;
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].as_ref().unwrap().message, "one");
    }

    #[tokio::test]
    async fn next_filtered_compiled_skips_unmatched_frames() {
        let (client, mut server) = tokio::io::duplex(8192);
        let mut bytes = frame(&build_entry(LogLevel::INFO, "skip"));
        bytes.extend_from_slice(&frame(&build_entry(LogLevel::INFO, "Hello 世界")));
        server.write_all(&bytes).await.unwrap();
        drop(server);

        let filter = MessageFilterOptions {
            matches: vec!["hello".into(), "世界".into()],
            ignore_case: true,
            ..Default::default()
        }
        .compile()
        .unwrap();
        let mut stream = TraceStream { stream: client };
        let entry = stream
            .next_filtered_compiled(&filter)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(entry.message, "Hello 世界");
        assert!(stream.next_entry().await.unwrap().is_none());
    }

    fn archive_status(status: &str) -> Vec<u8> {
        let value = plist::Value::Dictionary(plist::Dictionary::from_iter([(
            "Status".to_string(),
            plist::Value::String(status.into()),
        )]));
        let mut bytes = Vec::new();
        plist::to_writer_xml(&mut bytes, &value).unwrap();
        let mut framed = vec![ARCHIVE_RESPONSE_MARKER];
        framed.extend_from_slice(&(bytes.len() as u32).to_be_bytes());
        framed.extend_from_slice(&bytes);
        framed
    }

    fn archive_tar() -> Vec<u8> {
        let mut output = std::io::Cursor::new(Vec::new());
        let mut builder = tar::Builder::new(&mut output);
        let payload = "diagnostic 世界\n".as_bytes();
        let mut header = tar::Header::new_gnu();
        header.set_path("logs/世界.log").unwrap();
        header.set_size(payload.len() as u64);
        // Deliberately use a permissive archive mode: collect must tighten it
        // before installing the staging directory.
        header.set_mode(0o777);
        header.set_cksum();
        builder.append(&header, payload).unwrap();
        builder.finish().unwrap();
        drop(builder);
        output.into_inner()
    }

    async fn archive_server(
        mut server: tokio::io::DuplexStream,
        chunks: Vec<Vec<u8>>,
        status: &str,
        expected_options: Option<(u64, u64, i64)>,
    ) {
        let mut length = [0u8; 4];
        server.read_exact(&mut length).await.unwrap();
        let mut request = vec![0u8; u32::from_be_bytes(length) as usize];
        server.read_exact(&mut request).await.unwrap();
        let request: plist::Value = plist::from_bytes(&request).unwrap();
        let request = request.as_dictionary().unwrap();
        assert_eq!(
            request.get("Request").and_then(plist::Value::as_string),
            Some(CREATE_ARCHIVE)
        );
        if let Some((size_limit, age_limit, start_time)) = expected_options {
            assert_eq!(
                request
                    .get("SizeLimit")
                    .and_then(plist::Value::as_unsigned_integer),
                Some(size_limit)
            );
            assert_eq!(
                request
                    .get("AgeLimit")
                    .and_then(plist::Value::as_unsigned_integer),
                Some(age_limit)
            );
            assert_eq!(
                request
                    .get("StartTime")
                    .and_then(plist::Value::as_signed_integer),
                Some(start_time)
            );
        }
        server.write_all(&archive_status(status)).await.unwrap();
        if status != "RequestSuccessful" {
            return;
        }
        for chunk in chunks {
            server.write_all(&[ARCHIVE_CHUNK_MAGIC]).await.unwrap();
            server
                .write_all(&(chunk.len() as u32).to_le_bytes())
                .await
                .unwrap();
            for part in chunk.chunks(3) {
                server.write_all(part).await.unwrap();
            }
        }
    }

    #[tokio::test]
    async fn create_archive_matches_wire_and_streams_fragmented_chunks() {
        let (client, server) = tokio::io::duplex(64 * 1024);
        let archive = archive_tar();
        let server_task = tokio::spawn(archive_server(
            server,
            vec![archive[..17].to_vec(), archive[17..].to_vec()],
            "RequestSuccessful",
            Some((1234, 7, 42)),
        ));
        let mut output = Vec::new();
        let stats = OsTraceClient::new(client)
            .create_archive(
                &mut TokioVecWriter(&mut output),
                ArchiveOptions {
                    size_limit: Some(1234),
                    age_limit: Some(7),
                    start_time: Some(42),
                    ..Default::default()
                },
            )
            .await
            .unwrap();
        assert_eq!(output, archive);
        assert_eq!(stats.bytes, archive.len() as u64);
        assert_eq!(stats.chunks, 2);
        server_task.await.unwrap();
    }

    struct TokioVecWriter<'a>(&'a mut Vec<u8>);

    impl AsyncWrite for TokioVecWriter<'_> {
        fn poll_write(
            self: std::pin::Pin<&mut Self>,
            _cx: &mut std::task::Context<'_>,
            data: &[u8],
        ) -> std::task::Poll<std::io::Result<usize>> {
            self.get_mut().0.extend_from_slice(data);
            std::task::Poll::Ready(Ok(data.len()))
        }

        fn poll_flush(
            self: std::pin::Pin<&mut Self>,
            _cx: &mut std::task::Context<'_>,
        ) -> std::task::Poll<std::io::Result<()>> {
            std::task::Poll::Ready(Ok(()))
        }

        fn poll_shutdown(
            self: std::pin::Pin<&mut Self>,
            _cx: &mut std::task::Context<'_>,
        ) -> std::task::Poll<std::io::Result<()>> {
            std::task::Poll::Ready(Ok(()))
        }
    }

    #[tokio::test]
    async fn create_archive_accepts_empty_chunk_and_rejects_errors() {
        let (client, server) = tokio::io::duplex(4096);
        let server_task = tokio::spawn(archive_server(
            server,
            vec![Vec::new()],
            "RequestSuccessful",
            None,
        ));
        let mut output = Vec::new();
        let stats = OsTraceClient::new(client)
            .create_archive(&mut TokioVecWriter(&mut output), ArchiveOptions::default())
            .await
            .unwrap();
        assert_eq!(
            stats,
            ArchiveStats {
                bytes: 0,
                chunks: 1
            }
        );
        server_task.await.unwrap();

        let (client, mut server) = tokio::io::duplex(4096);
        tokio::spawn(async move {
            let mut length = [0u8; 4];
            server.read_exact(&mut length).await.unwrap();
            let mut request = vec![0u8; u32::from_be_bytes(length) as usize];
            server.read_exact(&mut request).await.unwrap();
            server
                .write_all(&archive_status("RequestDenied"))
                .await
                .unwrap();
        });
        let error = OsTraceClient::new(client)
            .create_archive(
                &mut TokioVecWriter(&mut Vec::new()),
                ArchiveOptions::default(),
            )
            .await
            .unwrap_err();
        assert!(error.to_string().contains("RequestDenied"));
    }

    #[tokio::test]
    async fn archive_to_path_validates_tar_and_collect_is_staged() {
        let output = test_temp_dir().join(format!("ios-trace-{}.tar", uuid::Uuid::new_v4()));
        let (client, server) = tokio::io::duplex(64 * 1024);
        let archive = archive_tar();
        let server_task = tokio::spawn(archive_server(
            server,
            vec![archive],
            "RequestSuccessful",
            None,
        ));
        let stats = OsTraceClient::new(client)
            .archive_to_path(&output, ArchiveOptions::default())
            .await
            .unwrap();
        assert!(output.is_file());
        assert_eq!(stats.bytes, fs::metadata(&output).unwrap().len());
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                fs::metadata(&output).unwrap().permissions().mode() & 0o777,
                0o600
            );
        }
        server_task.await.unwrap();
        fs::remove_file(&output).unwrap();

        let collect_output =
            test_temp_dir().join(format!("ios-trace-{}.logarchive", uuid::Uuid::new_v4()));
        let (client, server) = tokio::io::duplex(64 * 1024);
        let archive = archive_tar();
        let server_task = tokio::spawn(archive_server(
            server,
            vec![archive],
            "RequestSuccessful",
            None,
        ));
        let stats = OsTraceClient::new(client)
            .collect(&collect_output, ArchiveOptions::default())
            .await
            .unwrap();
        assert_eq!(stats.entries, 1);
        assert_eq!(
            fs::read_to_string(collect_output.join("logs/世界.log")).unwrap(),
            "diagnostic 世界\n"
        );
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                fs::metadata(collect_output.join("logs/世界.log"))
                    .unwrap()
                    .permissions()
                    .mode()
                    & 0o777,
                0o600
            );
            assert_eq!(
                fs::metadata(collect_output.join("logs"))
                    .unwrap()
                    .permissions()
                    .mode()
                    & 0o777,
                0o700
            );
        }
        server_task.await.unwrap();
        fs::remove_dir_all(collect_output).unwrap();
    }

    #[tokio::test]
    async fn collect_refuses_existing_destination_before_request() {
        let output = test_temp_dir().join(format!("ios-trace-{}", uuid::Uuid::new_v4()));
        fs::create_dir(&output).unwrap();
        let (client, _server) = tokio::io::duplex(4096);
        let error = OsTraceClient::new(client)
            .collect(&output, ArchiveOptions::default())
            .await
            .unwrap_err();
        assert!(error.to_string().contains("already exists"));
        fs::remove_dir(output).unwrap();
    }

    /// R5: the archive replacement itself must overwrite an existing regular
    /// file (the previous local constant turned REPLACE_EXISTING into
    /// MOVEFILE_COPY_ALLOWED, which fails with ERROR_ALREADY_EXISTS).
    #[test]
    fn atomic_replace_overwrites_an_existing_regular_file() {
        let directory = test_temp_dir().join(format!(
            "ios-trace-replace-{}-{}",
            std::process::id(),
            uuid::Uuid::new_v4()
        ));
        fs::create_dir_all(&directory).unwrap();
        let destination = directory.join("archive.tar.gz");
        fs::write(&destination, b"old archive bytes").unwrap();
        let source = directory.join(".staging.tar.gz");
        fs::write(&source, b"new archive bytes").unwrap();

        atomic_replace(&source, &destination).expect("replacement");

        assert_eq!(fs::read(&destination).unwrap(), b"new archive bytes");
        assert!(!source.exists(), "the source must be consumed");
        fs::remove_dir_all(&directory).unwrap();
    }

    /// Canonical verbatim output parents (for example `\\?\C:\...`) must pass
    /// the parent walk: the bare prefix is never queried for metadata.
    #[test]
    fn safe_output_parent_accepts_canonical_verbatim_directories() {
        let directory = test_temp_dir().join(format!(
            "ios-trace-parent-{}-{}",
            std::process::id(),
            uuid::Uuid::new_v4()
        ));
        fs::create_dir_all(&directory).unwrap();
        let canonical = directory.canonicalize().unwrap();

        safe_output_parent(&canonical.join("output.log")).expect("verbatim parent");

        fs::remove_dir_all(&directory).unwrap();
    }

    #[tokio::test]
    async fn archive_timeout_drops_pending_read() {
        let (client, mut server) = tokio::io::duplex(4096);
        let server_task = tokio::spawn(async move {
            let mut length = [0u8; 4];
            server.read_exact(&mut length).await.unwrap();
            let mut request = vec![0u8; u32::from_be_bytes(length) as usize];
            server.read_exact(&mut request).await.unwrap();
            let _ = server.write_all(&archive_status("RequestSuccessful")).await;
            tokio::time::sleep(Duration::from_secs(1)).await;
        });
        let result = tokio::time::timeout(
            Duration::from_millis(20),
            OsTraceClient::new(client).create_archive(
                &mut TokioVecWriter(&mut Vec::new()),
                ArchiveOptions::default(),
            ),
        )
        .await;
        assert!(
            result.is_err(),
            "archive should be bounded by caller deadline"
        );
        server_task.abort();
    }

    #[tokio::test]
    async fn archive_rejects_bad_magic_and_size_budget() {
        let (client, mut server) = tokio::io::duplex(4096);
        tokio::spawn(async move {
            let mut length = [0u8; 4];
            server.read_exact(&mut length).await.unwrap();
            let mut request = vec![0u8; u32::from_be_bytes(length) as usize];
            server.read_exact(&mut request).await.unwrap();
            server
                .write_all(&archive_status("RequestSuccessful"))
                .await
                .unwrap();
            server.write_all(&[0x99]).await.unwrap();
        });
        let error = OsTraceClient::new(client)
            .create_archive(
                &mut TokioVecWriter(&mut Vec::new()),
                ArchiveOptions::default(),
            )
            .await
            .unwrap_err();
        assert!(error.to_string().contains("frame magic"));

        let (client, mut server) = tokio::io::duplex(4096);
        tokio::spawn(async move {
            let mut length = [0u8; 4];
            server.read_exact(&mut length).await.unwrap();
            let mut request = vec![0u8; u32::from_be_bytes(length) as usize];
            server.read_exact(&mut request).await.unwrap();
            server
                .write_all(&archive_status("RequestSuccessful"))
                .await
                .unwrap();
            server.write_all(&[ARCHIVE_CHUNK_MAGIC]).await.unwrap();
            server.write_all(&100u32.to_le_bytes()).await.unwrap();
        });
        let error = OsTraceClient::new(client)
            .create_archive(
                &mut TokioVecWriter(&mut Vec::new()),
                ArchiveOptions {
                    max_total_bytes: 10,
                    ..Default::default()
                },
            )
            .await
            .unwrap_err();
        assert!(error.to_string().contains("exceeds max size"));
    }
}
