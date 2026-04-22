//! Syslog relay service.
//!
//! Connects to `com.apple.syslog_relay` (or `.shim.remote` over tunnel)
//! and streams null-byte-terminated log messages.
//!
//! Reference: go-ios/ios/syslog/syslog.go

use tokio::io::{AsyncRead, AsyncReadExt};
use tokio_stream::Stream;

pub const SERVICE_NAME: &str = "com.apple.syslog_relay";
pub const SHIM_SERVICE_NAME: &str = "com.apple.syslog_relay.shim.remote";

/// Syslog stream that yields raw log message strings (null-byte terminated by the device).
pub struct SyslogStream<S> {
    stream: S,
    buf: Vec<u8>,
}

impl<S: AsyncRead + Unpin> SyslogStream<S> {
    pub fn new(stream: S) -> Self {
        Self {
            stream,
            buf: Vec::with_capacity(4096),
        }
    }

    /// Read one null-terminated log message (blocking until available).
    pub async fn next_message(&mut self) -> Result<String, std::io::Error> {
        let mut byte = [0u8; 1];
        loop {
            self.stream.read_exact(&mut byte).await?;
            if byte[0] == 0 {
                let msg = String::from_utf8_lossy(&self.buf).into_owned();
                self.buf.clear();
                return Ok(msg);
            }
            self.buf.push(byte[0]);
        }
    }
}

/// Convert a syslog stream into an async Stream<Item = Result<String, io::Error>>.
pub fn into_stream<S: AsyncRead + Unpin + Send + 'static>(
    stream: S,
) -> impl Stream<Item = Result<String, std::io::Error>> {
    async_stream::try_stream! {
        let mut syslog = SyslogStream::new(stream);
        loop {
            let msg = syslog.next_message().await?;
            if !msg.is_empty() {
                yield msg;
            }
        }
    }
}

/// Parsed syslog entry.
#[derive(Debug, Clone)]
pub struct LogEntry {
    pub raw: String,
    pub timestamp: Option<String>,
    pub device: Option<String>,
    pub process: Option<String>,
    pub pid: Option<u32>,
    pub level: Option<String>,
    pub message: Option<String>,
    pub parse_success: bool,
    pub parse_error: Option<String>,
}

impl LogEntry {
    /// Try to parse a raw syslog line into structured fields.
    /// Preserves partial fields and makes parse failures explicit.
    pub fn parse(raw: String) -> Self {
        let mut entry = Self {
            raw,
            timestamp: None,
            device: None,
            process: None,
            pid: None,
            level: None,
            message: None,
            parse_success: false,
            parse_error: None,
        };

        entry.level = extract_level(&entry.raw);
        entry.message = extract_message(&entry.raw);

        let Some((timestamp, device, remainder)) = extract_prefix(&entry.raw) else {
            entry.parse_error = Some("missing syslog prefix (timestamp/device)".to_string());
            return entry;
        };

        entry.timestamp = Some(timestamp);
        entry.device = Some(device);

        let Some((process, pid)) = extract_process_token(remainder) else {
            entry.parse_error = Some("missing process segment after device".to_string());
            return entry;
        };

        entry.process = Some(process);
        entry.pid = pid;
        entry.parse_success = true;
        entry
    }
}

fn extract_prefix(s: &str) -> Option<(String, String, &str)> {
    let mut cursor = 0;
    let month = take_token(s, &mut cursor)?;
    let day = take_token(s, &mut cursor)?;
    let time = take_token(s, &mut cursor)?;
    let device = take_token(s, &mut cursor)?;
    let remainder = s[cursor..].trim_start();
    if remainder.is_empty() {
        return None;
    }

    Some((
        format!("{month} {day} {time}"),
        device.to_string(),
        remainder,
    ))
}

fn take_token<'a>(s: &'a str, cursor: &mut usize) -> Option<&'a str> {
    let bytes = s.as_bytes();
    while *cursor < bytes.len() && bytes[*cursor].is_ascii_whitespace() {
        *cursor += 1;
    }
    if *cursor >= bytes.len() {
        return None;
    }

    let start = *cursor;
    while *cursor < bytes.len() && !bytes[*cursor].is_ascii_whitespace() {
        *cursor += 1;
    }
    Some(&s[start..*cursor])
}

fn extract_process_token(s: &str) -> Option<(String, Option<u32>)> {
    let token = s.split_whitespace().next()?.trim();
    if token.is_empty() {
        return None;
    }

    if let Some(open) = token.rfind('[') {
        if token.ends_with(']') && open > 0 {
            let pid = token[open + 1..token.len() - 1].parse().ok();
            let process = token[..open].trim();
            if !process.is_empty() {
                return Some((process.to_string(), pid));
            }
        }
    }

    Some((token.to_string(), None))
}

fn extract_level(s: &str) -> Option<String> {
    let start = s.find('<')? + 1;
    let end = s[start..].find('>')? + start;
    Some(s[start..end].to_string())
}

fn extract_message(s: &str) -> Option<String> {
    if let Some(pos) = s.find(">: ") {
        return Some(s[pos + 3..].to_string());
    }
    if let Some(pos) = s.find(">:") {
        return Some(s[pos + 2..].trim_start().to_string());
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_syslog_stream_null_terminated() {
        // Two null-terminated messages
        let data = b"message one\0message two\0";
        let mut stream = std::io::Cursor::new(data);
        let mut syslog = SyslogStream::new(&mut stream);
        assert_eq!(syslog.next_message().await.unwrap(), "message one");
        assert_eq!(syslog.next_message().await.unwrap(), "message two");
    }

    #[test]
    fn test_log_entry_parse_level() {
        let raw = "Mar 17 12:34:56 iPhone kernel[0] <Notice>: boot message";
        let entry = LogEntry::parse(raw.to_string());
        assert_eq!(entry.timestamp.as_deref(), Some("Mar 17 12:34:56"));
        assert_eq!(entry.device.as_deref(), Some("iPhone"));
        assert_eq!(entry.process.as_deref(), Some("kernel"));
        assert_eq!(entry.level.as_deref(), Some("Notice"));
        assert_eq!(entry.pid, Some(0));
        assert_eq!(entry.message.as_deref(), Some("boot message"));
        assert!(entry.parse_success);
        assert_eq!(entry.parse_error, None);
    }

    #[test]
    fn test_log_entry_parse_failure_is_explicit() {
        let raw = "totally unstructured syslog payload";
        let entry = LogEntry::parse(raw.to_string());

        assert_eq!(entry.raw, raw);
        assert_eq!(entry.timestamp, None);
        assert_eq!(entry.device, None);
        assert_eq!(entry.process, None);
        assert_eq!(entry.pid, None);
        assert_eq!(entry.level, None);
        assert_eq!(entry.message, None);
        assert!(!entry.parse_success);
        assert!(entry.parse_error.as_deref().is_some());
    }
}
