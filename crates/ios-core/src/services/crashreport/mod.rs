use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite};

use crate::services::afc::{AfcClient, AfcError, AfcFileInfo};

pub const CRASHREPORT_MOVER_SERVICE: &str = "com.apple.crashreportmover";
pub const CRASHREPORT_COPY_MOBILE_SERVICE: &str = "com.apple.crashreportcopymobile";

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
    #[error("invalid pattern '{pattern}': {message}")]
    InvalidPattern { pattern: String, message: String },
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

    async fn resolve_report_path(&mut self, report: &str) -> Result<String, CrashReportError> {
        if report.contains('/') {
            return Ok(normalize_report_path(report));
        }

        let reports = self.list_reports(Some("*")).await?;
        resolve_report_path_from_entries(report, &reports)
    }
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

fn normalize_report_path(report: &str) -> String {
    if report.starts_with("./") {
        report.to_string()
    } else {
        format!("./{}", report.trim_start_matches('/'))
    }
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
    async fn prepare_reports_rejects_non_ping() {
        let (mut client, mut server) = duplex(16);
        tokio::spawn(async move {
            server.write_all(b"pong").await.unwrap();
        });

        let err = prepare_reports(&mut client).await.unwrap_err();
        assert!(err.to_string().contains("ping"));
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
