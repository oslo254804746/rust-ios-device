//! AFC (Apple File Conduit) protocol – direct async I/O implementation.
//!
//! Wire protocol reference: go-ios/ios/afc/afc.go + client.go
//!
//! Frame structure:
//!   [AfcHeader: 40 bytes LE]
//!   [header_payload: (this_len - 40) bytes]  – null-terminated paths, file handles, status etc.
//!   [payload: (entire_len - this_len) bytes] – file data for reads/writes
//!
//! For ReadDir responses, the device puts filenames in the header_payload section.
//! For FileRead responses, the data is in the payload section.

use std::collections::HashMap;

use crate::proto::afc::{AfcHeader, AfcOpcode, AFC_MAGIC};
use bytes::{Bytes, BytesMut};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use zerocopy::{FromBytes, IntoBytes};

#[cfg(feature = "house_arrest")]
pub use super::house_arrest;

pub mod protocol; // kept for re-export compatibility

// ── error ─────────────────────────────────────────────────────────────────────

#[derive(Debug, thiserror::Error)]
pub enum AfcError {
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
    #[error("AFC error: {0}")]
    Status(AfcStatusCode),
    #[error("protocol error: {0}")]
    Protocol(String),
}

/// AFC status codes from the device (matches go-ios/ios/afc/errors.go).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AfcStatusCode {
    Success,
    Unknown,
    OperationHeaderInvalid,
    NoResources,
    ReadError,
    WriteError,
    UnknownPacketType,
    InvalidArgument,
    ObjectNotFound,
    ObjectIsDir,
    PermDenied,
    ServiceNotConnected,
    Timeout,
    TooMuchData,
    EndOfData,
    OpNotSupported,
    ObjectExists,
    ObjectBusy,
    NoSpaceLeft,
    OpWouldBlock,
    IoError,
    OpInterrupted,
    OpInProgress,
    InternalError,
    MuxError,
    NoMem,
    NotEnoughData,
    DirNotEmpty,
    Other(u64),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AfcFileInfo {
    pub name: Option<String>,
    pub file_type: Option<String>,
    pub size: Option<u64>,
    pub mode: Option<u32>,
    pub link_target: Option<String>,
    pub raw: HashMap<String, String>,
}

impl AfcStatusCode {
    pub fn from_u64(code: u64) -> Self {
        match code {
            0 => Self::Success,
            1 => Self::Unknown,
            2 => Self::OperationHeaderInvalid,
            3 => Self::NoResources,
            4 => Self::ReadError,
            5 => Self::WriteError,
            6 => Self::UnknownPacketType,
            7 => Self::InvalidArgument,
            8 => Self::ObjectNotFound,
            9 => Self::ObjectIsDir,
            10 => Self::PermDenied,
            11 => Self::ServiceNotConnected,
            12 => Self::Timeout,
            13 => Self::TooMuchData,
            14 => Self::EndOfData,
            15 => Self::OpNotSupported,
            16 => Self::ObjectExists,
            17 => Self::ObjectBusy,
            18 => Self::NoSpaceLeft,
            19 => Self::OpWouldBlock,
            20 => Self::IoError,
            21 => Self::OpInterrupted,
            22 => Self::OpInProgress,
            23 => Self::InternalError,
            30 => Self::MuxError,
            31 => Self::NoMem,
            32 => Self::NotEnoughData,
            33 => Self::DirNotEmpty,
            _ => Self::Other(code),
        }
    }
}

impl std::fmt::Display for AfcStatusCode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Success => write!(f, "success"),
            Self::Unknown => write!(f, "unknown error (1)"),
            Self::OperationHeaderInvalid => write!(f, "operation header invalid (2)"),
            Self::NoResources => write!(f, "no resources (3)"),
            Self::ReadError => write!(f, "read error (4)"),
            Self::WriteError => write!(f, "write error (5)"),
            Self::UnknownPacketType => write!(f, "unknown packet type (6)"),
            Self::InvalidArgument => write!(f, "invalid argument (7)"),
            Self::ObjectNotFound => write!(f, "object not found (8)"),
            Self::ObjectIsDir => write!(f, "object is directory (9)"),
            Self::PermDenied => write!(f, "permission denied (10)"),
            Self::ServiceNotConnected => write!(f, "service not connected (11)"),
            Self::Timeout => write!(f, "timeout (12)"),
            Self::TooMuchData => write!(f, "too much data (13)"),
            Self::EndOfData => write!(f, "end of data (14)"),
            Self::OpNotSupported => write!(f, "operation not supported (15)"),
            Self::ObjectExists => write!(f, "object exists (16)"),
            Self::ObjectBusy => write!(f, "object busy (17)"),
            Self::NoSpaceLeft => write!(f, "no space left (18)"),
            Self::OpWouldBlock => write!(f, "operation would block (19)"),
            Self::IoError => write!(f, "I/O error (20)"),
            Self::OpInterrupted => write!(f, "operation interrupted (21)"),
            Self::OpInProgress => write!(f, "operation in progress (22)"),
            Self::InternalError => write!(f, "internal error (23)"),
            Self::MuxError => write!(f, "mux error (30)"),
            Self::NoMem => write!(f, "no memory (31)"),
            Self::NotEnoughData => write!(f, "not enough data (32)"),
            Self::DirNotEmpty => write!(f, "directory not empty (33)"),
            Self::Other(code) => write!(f, "unknown status ({code})"),
        }
    }
}

// ── raw packet ────────────────────────────────────────────────────────────────

struct Packet {
    #[allow(dead_code)]
    opcode: u64,
    /// Embedded data within the "header" section (this_len - 40 bytes).
    /// Used for: directory listings, file info, status codes, file handles.
    header_payload: Bytes,
    /// Extra data after the header section (entire_len - this_len bytes).
    /// Used for: file read data.
    payload: Bytes,
}

// ── client ────────────────────────────────────────────────────────────────────

/// AFC file-system client.
///
/// `S` must be a full-duplex async stream (e.g. the raw usbmux stream from lockdown).
pub struct AfcClient<S> {
    stream: S,
    packet_num: u64,
}

impl<S: AsyncRead + AsyncWrite + Unpin> AfcClient<S> {
    pub const FILE_MODE_READ_ONLY: u64 = 0x00000001;
    pub const FILE_MODE_READ_WRITE: u64 = 0x00000002;
    pub const FILE_MODE_WRITE_ONLY_CREATE_TRUNC: u64 = 0x00000003;
    pub const LOCK_EXCLUSIVE: u64 = 2 | 4;
    pub const LOCK_UNLOCK: u64 = 8 | 4;

    pub fn new(stream: S) -> Self {
        // go-ios uses atomic.Add(1) which returns 1 on first call.
        // Devices may reject packet_num=0 for some operations.
        Self {
            stream,
            packet_num: 1,
        }
    }

    fn next_pnum(&mut self) -> u64 {
        let n = self.packet_num;
        self.packet_num += 1;
        n
    }

    // ── send ─────────────────────────────────────────────────────────────────

    async fn send(
        &mut self,
        opcode: AfcOpcode,
        header_payload: &[u8],
        payload: &[u8],
    ) -> Result<(), AfcError> {
        let pnum = self.next_pnum();
        let hdr = AfcHeader::new(pnum, opcode, header_payload.len(), payload.len());
        self.stream.write_all(hdr.as_bytes()).await?;
        if !header_payload.is_empty() {
            self.stream.write_all(header_payload).await?;
        }
        if !payload.is_empty() {
            self.stream.write_all(payload).await?;
        }
        self.stream.flush().await?;
        Ok(())
    }

    // ── recv ─────────────────────────────────────────────────────────────────

    async fn recv(&mut self) -> Result<Packet, AfcError> {
        let mut hdr_buf = [0u8; AfcHeader::SIZE];
        self.stream.read_exact(&mut hdr_buf).await?;

        let hdr = AfcHeader::ref_from_bytes(&hdr_buf)
            .map_err(|_| AfcError::Protocol("bad AFC header".into()))?;

        if hdr.magic.get() != AFC_MAGIC {
            return Err(AfcError::Protocol(format!(
                "bad AFC magic: 0x{:016X}",
                hdr.magic.get()
            )));
        }

        let entire_len = hdr.entire_len.get() as usize;
        let this_len = hdr.this_len.get() as usize;
        let opcode = hdr.operation.get();

        let header_payload_len = this_len.saturating_sub(AfcHeader::SIZE);
        let payload_len = entire_len.saturating_sub(this_len);

        // Sanity check against DoS
        const MAX_AFC_MSG: usize = 256 * 1024 * 1024; // 256 MiB
        if header_payload_len > MAX_AFC_MSG || payload_len > MAX_AFC_MSG {
            return Err(AfcError::Protocol(format!(
                "AFC frame too large: header_payload={header_payload_len} payload={payload_len}"
            )));
        }

        let mut header_payload = vec![0u8; header_payload_len];
        let mut payload = vec![0u8; payload_len];

        if header_payload_len > 0 {
            self.stream.read_exact(&mut header_payload).await?;
        }
        if payload_len > 0 {
            self.stream.read_exact(&mut payload).await?;
        }

        // Status opcode (1): header_payload[0..8] = LE u64 error code
        if opcode == AfcOpcode::Status as u64 {
            let code = AfcStatusCode::from_u64(if header_payload.len() >= 8 {
                u64::from_le_bytes(
                    header_payload[..8]
                        .try_into()
                        .map_err(|_| AfcError::Protocol("bad status code".into()))?,
                )
            } else {
                0
            });
            if code != AfcStatusCode::Success {
                return Err(AfcError::Status(code));
            }
        }

        Ok(Packet {
            opcode,
            header_payload: Bytes::from(header_payload),
            payload: Bytes::from(payload),
        })
    }

    // ── public API ────────────────────────────────────────────────────────────

    /// List directory entries (excludes "." and "..").
    ///
    /// The AFC device returns null-separated filenames in the payload section
    /// of the response frame (entire_len > this_len).
    pub async fn list_dir(&mut self, path: &str) -> Result<Vec<String>, AfcError> {
        let mut hp = path.as_bytes().to_vec();
        hp.push(0);
        self.send(AfcOpcode::ReadDir, &hp, &[]).await?;
        let pkt = self.recv().await?;
        // Filenames come in the payload section (consistent with go-ios pack.Payload)
        let entries = split_null_strings(&pkt.payload)
            .into_iter()
            .filter(|s| s != "." && s != "..")
            .collect();
        Ok(entries)
    }

    /// Get key-value file info for a path.
    pub async fn stat(&mut self, path: &str) -> Result<HashMap<String, String>, AfcError> {
        let mut hp = path.as_bytes().to_vec();
        hp.push(0);
        self.send(AfcOpcode::GetFileInfo, &hp, &[]).await?;
        let pkt = self.recv().await?;
        Ok(parse_kv_pairs(&pkt.payload))
    }

    /// Get parsed file info for a path.
    ///
    /// AFC reports `st_mode` as an octal string. This helper parses it into a
    /// `u32`, matching go-ios behavior.
    pub async fn stat_info(&mut self, path: &str) -> Result<AfcFileInfo, AfcError> {
        let raw = self.stat(path).await?;
        Ok(parse_file_info(path, raw))
    }

    /// Create a directory.
    pub async fn make_dir(&mut self, path: &str) -> Result<(), AfcError> {
        let mut hp = path.as_bytes().to_vec();
        hp.push(0);
        self.send(AfcOpcode::MakePath, &hp, &[]).await?;
        self.recv().await?;
        Ok(())
    }

    /// Remove a path (file or empty directory).
    pub async fn remove(&mut self, path: &str) -> Result<(), AfcError> {
        let mut hp = path.as_bytes().to_vec();
        hp.push(0);
        self.send(AfcOpcode::RemovePath, &hp, &[]).await?;
        self.recv().await?;
        Ok(())
    }

    /// Remove a path and all contents recursively.
    pub async fn remove_all(&mut self, path: &str) -> Result<(), AfcError> {
        let mut hp = path.as_bytes().to_vec();
        hp.push(0);
        self.send(AfcOpcode::RemovePathAndContents, &hp, &[])
            .await?;
        self.recv().await?;
        Ok(())
    }

    /// Rename / move a path.
    pub async fn rename(&mut self, from: &str, to: &str) -> Result<(), AfcError> {
        let mut hp = from.as_bytes().to_vec();
        hp.push(0);
        hp.extend_from_slice(to.as_bytes());
        hp.push(0);
        self.send(AfcOpcode::RenamePath, &hp, &[]).await?;
        self.recv().await?;
        Ok(())
    }

    /// Read an entire file into memory.
    pub async fn read_file(&mut self, path: &str) -> Result<Bytes, AfcError> {
        let fd = self.file_open(path, Self::FILE_MODE_READ_ONLY).await?; // READ_ONLY
        let mut data = BytesMut::new();
        let chunk = 65536u64;
        loop {
            let buf = self.file_read(fd, chunk).await?;
            if buf.is_empty() {
                break;
            }
            data.extend_from_slice(&buf);
        }
        self.file_close(fd).await?;
        Ok(data.freeze())
    }

    /// Read an entire file into memory, following a single AFC symlink.
    ///
    /// This matches go-ios PullSingleFile behavior: if the source path is a
    /// symlink, AFC reports the target in `st_linktarget` and the file is read
    /// from that target path instead.
    pub async fn read_file_follow_links(&mut self, path: &str) -> Result<Bytes, AfcError> {
        let target = self.resolve_read_path(path).await?;
        self.read_file(&target).await
    }

    /// Write data to a file (creates or truncates).
    pub async fn write_file(&mut self, path: &str, data: &[u8]) -> Result<(), AfcError> {
        let fd = self
            .file_open(path, Self::FILE_MODE_WRITE_ONLY_CREATE_TRUNC)
            .await?; // WRITE_ONLY_CREATE_TRUNC
        self.file_write(fd, data).await?;
        self.file_close(fd).await?;
        Ok(())
    }

    pub async fn open_file(&mut self, path: &str, mode: u64) -> Result<u64, AfcError> {
        self.file_open(path, mode).await
    }

    pub async fn lock_file(&mut self, fd: u64, operation: u64) -> Result<(), AfcError> {
        let mut hp = [0u8; 16];
        hp[..8].copy_from_slice(&fd.to_le_bytes());
        hp[8..].copy_from_slice(&operation.to_le_bytes());
        self.send(AfcOpcode::FileRefLock, &hp, &[]).await?;
        self.recv().await?;
        Ok(())
    }

    pub async fn close_file(&mut self, fd: u64) -> Result<(), AfcError> {
        self.file_close(fd).await
    }

    /// Get device filesystem info.
    pub async fn device_info(&mut self) -> Result<HashMap<String, String>, AfcError> {
        self.send(AfcOpcode::GetDeviceInfo, &[], &[]).await?;
        let pkt = self.recv().await?;
        Ok(parse_kv_pairs(&pkt.payload))
    }

    // ── file handle ops ───────────────────────────────────────────────────────

    async fn file_open(&mut self, path: &str, mode: u64) -> Result<u64, AfcError> {
        let mut hp = vec![0u8; 8];
        hp[..8].copy_from_slice(&mode.to_le_bytes());
        hp.extend_from_slice(path.as_bytes());
        hp.push(0);
        self.send(AfcOpcode::FileRefOpen, &hp, &[]).await?;
        let pkt = self.recv().await?;
        if pkt.header_payload.len() < 8 {
            return Err(AfcError::Protocol(
                "FileRefOpenResult: short response".into(),
            ));
        }
        let fd = u64::from_le_bytes(
            pkt.header_payload[..8]
                .try_into()
                .map_err(|_| AfcError::Protocol("bad file handle".into()))?,
        );
        Ok(fd)
    }

    async fn file_read(&mut self, fd: u64, size: u64) -> Result<Bytes, AfcError> {
        let mut hp = [0u8; 16];
        hp[..8].copy_from_slice(&fd.to_le_bytes());
        hp[8..].copy_from_slice(&size.to_le_bytes());
        self.send(AfcOpcode::FileRefRead, &hp, &[]).await?;
        let pkt = self.recv().await?;
        Ok(pkt.payload)
    }

    async fn file_write(&mut self, fd: u64, data: &[u8]) -> Result<(), AfcError> {
        let mut hp = [0u8; 8];
        hp.copy_from_slice(&fd.to_le_bytes());
        self.send(AfcOpcode::FileRefWrite, &hp, data).await?;
        self.recv().await?;
        Ok(())
    }

    async fn file_close(&mut self, fd: u64) -> Result<(), AfcError> {
        let hp = fd.to_le_bytes();
        self.send(AfcOpcode::FileRefClose, &hp, &[]).await?;
        self.recv().await?;
        Ok(())
    }

    async fn resolve_read_path(&mut self, path: &str) -> Result<String, AfcError> {
        let info = self.stat_info(path).await?;
        Ok(resolve_link_target(path, &info))
    }
}

// ── helpers ───────────────────────────────────────────────────────────────────

/// Split a null-byte-separated byte slice into owned strings, skipping empty.
fn split_null_strings(data: &[u8]) -> Vec<String> {
    data.split(|&b| b == 0)
        .filter(|s| !s.is_empty())
        .map(|s| String::from_utf8_lossy(s).into_owned())
        .collect()
}

/// Parse null-separated key\0value\0 pairs into a HashMap.
fn parse_kv_pairs(data: &[u8]) -> HashMap<String, String> {
    let parts = split_null_strings(data);
    let mut map = HashMap::new();
    let mut it = parts.into_iter();
    while let (Some(k), Some(v)) = (it.next(), it.next()) {
        map.insert(k, v);
    }
    map
}

fn parse_file_info(path: &str, raw: HashMap<String, String>) -> AfcFileInfo {
    let name = path
        .rsplit('/')
        .next()
        .filter(|s| !s.is_empty())
        .map(str::to_string);
    let file_type = raw.get("st_ifmt").cloned();
    let size = raw.get("st_size").and_then(|s| s.parse::<u64>().ok());
    let mode = raw
        .get("st_mode")
        .and_then(|s| u32::from_str_radix(s, 8).ok());
    let link_target = raw.get("st_linktarget").cloned().filter(|s| !s.is_empty());

    AfcFileInfo {
        name,
        file_type,
        size,
        mode,
        link_target,
        raw,
    }
}

fn resolve_link_target(path: &str, info: &AfcFileInfo) -> String {
    let is_link = matches!(info.file_type.as_deref(), Some("S_IFLNK"));
    if is_link {
        if let Some(target) = &info.link_target {
            return target.clone();
        }
    }
    path.to_string()
}

// ── file mode constants (for callers) ─────────────────────────────────────────
pub mod mode {
    pub const READ_ONLY: u64 = 0x00000001;
    pub const READ_WRITE_CREATE: u64 = 0x00000002;
    pub const WRITE_ONLY_CREATE_TRUNC: u64 = 0x00000003;
    pub const READ_WRITE_CREATE_TRUNC: u64 = 0x00000004;
    pub const WRITE_ONLY_CREATE_APPEND: u64 = 0x00000005;
    pub const READ_WRITE_CREATE_APPEND: u64 = 0x00000006;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_split_null_strings() {
        let data = b"foo\0bar\0baz\0";
        let result = split_null_strings(data);
        assert_eq!(result, vec!["foo", "bar", "baz"]);
    }

    #[test]
    fn test_parse_kv_pairs() {
        let data = b"st_size\x0012345\0st_ifmt\0S_IFREG\0";
        let map = parse_kv_pairs(data);
        assert_eq!(map["st_size"], "12345");
        assert_eq!(map["st_ifmt"], "S_IFREG");
    }

    #[test]
    fn test_afc_header_size() {
        assert_eq!(std::mem::size_of::<AfcHeader>(), 40);
    }

    #[test]
    fn test_afc_header_new() {
        let hdr = AfcHeader::new(7, AfcOpcode::ReadDir, 5, 10);
        assert_eq!(hdr.magic.get(), AFC_MAGIC);
        assert_eq!(hdr.packet_num.get(), 7);
        assert_eq!(hdr.this_len.get(), 45); // 40 + 5
        assert_eq!(hdr.entire_len.get(), 55); // 45 + 10
        assert_eq!(hdr.operation.get(), AfcOpcode::ReadDir as u64);
    }

    /// Simulate list_dir: the device returns filenames in the payload section.
    #[tokio::test]
    async fn test_list_dir_roundtrip() {
        use zerocopy::IntoBytes;

        // ReadDir response: filenames are in the payload section
        // (this_len = 40, entire_len = 40 + names.len())
        let names = b".\0..\0Photos\0Downloads\0";
        let hdr = AfcHeader::new(
            1,                  // packet_num
            AfcOpcode::ReadDir, // opcode
            0,                  // header_payload = 0 bytes (this_len = 40)
            names.len(),        // payload = filenames (entire_len = 40 + names.len())
        );
        let mut server_resp = hdr.as_bytes().to_vec();
        server_resp.extend_from_slice(names); // payload section follows header

        let (client_side, mut server_side) = tokio::io::duplex(4096);
        tokio::spawn(async move {
            use tokio::io::{AsyncReadExt, AsyncWriteExt};
            let mut buf = vec![0u8; 256];
            let _ = server_side.read(&mut buf).await;
            server_side.write_all(&server_resp).await.unwrap();
        });

        let mut afc = AfcClient::new(client_side);
        let entries = afc.list_dir("/").await.unwrap();
        // "." and ".." are filtered out
        assert_eq!(entries, vec!["Photos", "Downloads"]);
    }

    #[test]
    fn test_resolve_link_target_uses_st_linktarget_for_symlink() {
        let mut raw = HashMap::new();
        raw.insert("st_ifmt".to_string(), "S_IFLNK".to_string());
        raw.insert(
            "st_linktarget".to_string(),
            "/var/mobile/real-file".to_string(),
        );
        let info = parse_file_info("/var/mobile/link", raw);

        let resolved = resolve_link_target("/var/mobile/link", &info);
        assert_eq!(resolved, "/var/mobile/real-file");
    }

    #[test]
    fn test_resolve_link_target_keeps_original_path_for_regular_file() {
        let mut raw = HashMap::new();
        raw.insert("st_ifmt".to_string(), "S_IFREG".to_string());
        let info = parse_file_info("/var/mobile/file", raw);

        let resolved = resolve_link_target("/var/mobile/file", &info);
        assert_eq!(resolved, "/var/mobile/file");
    }

    #[test]
    fn test_parse_file_info_parses_st_mode_from_octal() {
        let mut raw = HashMap::new();
        raw.insert("st_ifmt".to_string(), "S_IFREG".to_string());
        raw.insert("st_mode".to_string(), "100644".to_string());
        raw.insert("st_size".to_string(), "12".to_string());

        let info = parse_file_info("/var/mobile/file.txt", raw);
        assert_eq!(info.name.as_deref(), Some("file.txt"));
        assert_eq!(info.file_type.as_deref(), Some("S_IFREG"));
        assert_eq!(info.size, Some(12));
        assert_eq!(info.mode, Some(0o100644));
    }

    #[test]
    fn test_afc_status_code_mapping_matches_go_ios_upper_status_codes() {
        assert_eq!(AfcStatusCode::from_u64(24), AfcStatusCode::Other(24));
        assert_eq!(AfcStatusCode::from_u64(25), AfcStatusCode::Other(25));
        assert_eq!(AfcStatusCode::from_u64(26), AfcStatusCode::Other(26));
        assert_eq!(AfcStatusCode::from_u64(27), AfcStatusCode::Other(27));
        assert_eq!(AfcStatusCode::from_u64(30), AfcStatusCode::MuxError);
        assert_eq!(AfcStatusCode::from_u64(31), AfcStatusCode::NoMem);
        assert_eq!(AfcStatusCode::from_u64(32), AfcStatusCode::NotEnoughData);
        assert_eq!(AfcStatusCode::from_u64(33), AfcStatusCode::DirNotEmpty);
    }
}
