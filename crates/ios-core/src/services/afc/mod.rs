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

crate::redaction::error_debug!(AfcError);
crate::redaction::error_debug!(AfcReadLimit);

use std::collections::HashMap;
use std::pin::Pin;
use std::task::{Context, Poll};
use std::time::Duration;

use crate::proto::afc::{AfcHeader, AfcOpcode, AFC_MAGIC};
use bytes::{Bytes, BytesMut};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt, ReadBuf};
use zerocopy::{FromBytes, IntoBytes};

#[cfg(feature = "house_arrest")]
pub use super::house_arrest;

pub mod protocol; // kept for re-export compatibility

// ── error ─────────────────────────────────────────────────────────────────────

#[derive(thiserror::Error)]
pub enum AfcError {
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
    #[error("AFC error: {0}")]
    Status(AfcStatusCode),
    #[error("protocol error: {0}")]
    Protocol(String),
}

/// A local bounded-read admission decision, retained under the existing Io
/// variant so callers can keep exhaustively matching AfcError.
#[derive(thiserror::Error)]
#[error("AFC {scope} exceeds the {limit} byte limit (received {received} bytes)")]
pub(crate) struct AfcReadLimit {
    scope: &'static str,
    limit: usize,
    received: u64,
}

fn bounded_read_limit(scope: &'static str, limit: usize, received: u64) -> AfcError {
    AfcError::Io(std::io::Error::other(AfcReadLimit {
        scope,
        limit,
        received,
    }))
}

#[derive(Clone, Copy)]
struct ReadResponseLimit {
    body: usize,
    payload: usize,
}

impl ReadResponseLimit {
    // File handles and status replies each contain one u64 and no data payload.
    const CONTROL: Self = Self {
        body: 8,
        payload: 0,
    };

    fn data(requested: usize) -> Self {
        Self {
            body: requested.max(8),
            payload: requested,
        }
    }
}

fn header_error(source: zerocopy::CastError<&[u8], AfcHeader>) -> AfcError {
    // The production caller reads exactly the fixed 40-byte header. Only an
    // error owns a bounded copy; the successful borrowed decode stays unchanged.
    let source = source.map_src(|bytes| bytes.to_vec().into_boxed_slice());
    AfcError::Io(crate::error::context::received_io(
        "decode AFC header",
        None,
        source,
    ))
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
    stream: AfcStream<S>,
    packet_num: u64,
    usable: bool,
    request_timeout: Option<Duration>,
}

struct AfcStream<S> {
    inner: S,
    progressed: bool,
}

impl<S: AsyncRead + Unpin> AsyncRead for AfcStream<S> {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<std::io::Result<()>> {
        let before = buf.filled().len();
        let result = Pin::new(&mut self.inner).poll_read(cx, buf);
        self.progressed |= buf.filled().len() > before;
        result
    }
}

impl<S: AsyncWrite + Unpin> AsyncWrite for AfcStream<S> {
    fn poll_write(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        bytes: &[u8],
    ) -> Poll<std::io::Result<usize>> {
        let result = Pin::new(&mut self.inner).poll_write(cx, bytes);
        if matches!(result, Poll::Ready(Ok(n)) if n > 0) {
            self.progressed = true;
        }
        result
    }
    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        Pin::new(&mut self.inner).poll_flush(cx)
    }
    fn poll_shutdown(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        Pin::new(&mut self.inner).poll_shutdown(cx)
    }
}

struct AfcRequest<'a, S> {
    client: &'a mut AfcClient<S>,
    completed: bool,
}

impl<S> Drop for AfcRequest<'_, S> {
    fn drop(&mut self) {
        if !self.completed && self.client.stream.progressed {
            self.client.usable = false;
        }
    }
}

struct AfcTransfer<'a, S> {
    client: &'a mut AfcClient<S>,
    completed: bool,
}

impl<S> Drop for AfcTransfer<'_, S> {
    fn drop(&mut self) {
        // An open remote file belongs to this transfer even between requests.
        // Drop cannot await close; require a fresh connection after cancellation.
        if !self.completed {
            self.client.usable = false;
        }
    }
}

const FILE_CLEANUP_BUDGET: Duration = Duration::from_secs(5);

struct FileCleanupFailure {
    primary: AfcError,
    cleanup: AfcError,
}

impl std::fmt::Display for FileCleanupFailure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("AFC transfer failed; file cleanup also failed")
    }
}

impl std::fmt::Debug for FileCleanupFailure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("FileCleanupFailure")
            .field("primary_class", &crate::classify_error(&self.primary))
            .field("cleanup_class", &crate::classify_error(&self.cleanup))
            .finish_non_exhaustive()
    }
}

impl std::error::Error for FileCleanupFailure {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        Some(&self.primary)
    }
}

fn finish_file_transfer<T>(
    operation: Result<T, AfcError>,
    cleanup: Result<(), AfcError>,
) -> Result<T, AfcError> {
    match (operation, cleanup) {
        (Ok(value), Ok(())) => Ok(value),
        (Ok(_), Err(cleanup)) => Err(cleanup),
        (Err(primary), Ok(())) => Err(primary),
        (Err(primary), Err(cleanup)) => {
            Err(AfcError::Io(std::io::Error::other(FileCleanupFailure {
                primary,
                cleanup,
            })))
        }
    }
}

impl<S: AsyncRead + AsyncWrite + Unpin> AfcClient<S> {
    pub const FILE_MODE_READ_ONLY: u64 = 0x00000001;
    pub const FILE_MODE_READ_WRITE: u64 = 0x00000002;
    pub const FILE_MODE_WRITE_ONLY_CREATE_TRUNC: u64 = 0x00000003;
    pub const LOCK_EXCLUSIVE: u64 = 2 | 4;
    pub const LOCK_UNLOCK: u64 = 8 | 4;

    /// Ceiling for the whole-file-into-memory helpers.
    pub const MAX_IN_MEMORY_FILE: usize = 256 * 1024 * 1024;

    pub fn new(stream: S) -> Self {
        // go-ios uses atomic.Add(1) which returns 1 on first call.
        // Devices may reject packet_num=0 for some operations.
        Self {
            stream: AfcStream {
                inner: stream,
                progressed: false,
            },
            packet_num: 1,
            usable: true,
            request_timeout: None,
        }
    }

    /// Whether framing and request ownership still permit another operation.
    /// This is local state, not acknowledgment of remote cleanup or retry safety.
    pub fn is_usable(&self) -> bool {
        self.usable
    }

    /// Set one shared send/reply budget for each AFC request. None preserves
    /// legacy unbounded timing. This is not a whole-file or host-reader budget.
    /// Zero and unrepresentable budgets fail before request I/O.
    pub fn set_request_timeout(&mut self, budget: Option<Duration>) {
        self.request_timeout = budget;
    }

    fn next_pnum(&mut self) -> Result<u64, AfcError> {
        let n = self.packet_num;
        self.packet_num = n
            .checked_add(1)
            .ok_or_else(|| AfcError::Protocol("AFC packet identifier exhausted".into()))?;
        Ok(n)
    }

    async fn request(
        &mut self,
        opcode: AfcOpcode,
        header_payload: &[u8],
        payload: &[u8],
    ) -> Result<Packet, AfcError> {
        self.request_with_response_limit(opcode, header_payload, payload, None)
            .await
    }

    async fn request_with_response_limit(
        &mut self,
        opcode: AfcOpcode,
        header_payload: &[u8],
        payload: &[u8],
        response_limit: Option<ReadResponseLimit>,
    ) -> Result<Packet, AfcError> {
        if !self.usable {
            return Err(AfcError::Protocol(
                "AFC connection is unusable; reconnect before another request".into(),
            ));
        }
        let deadline = match self.request_timeout {
            Some(budget) if budget.is_zero() => {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::TimedOut,
                    "AFC request budget is zero",
                )
                .into())
            }
            Some(budget) => Some(tokio::time::Instant::now().checked_add(budget).ok_or_else(
                || {
                    std::io::Error::new(
                        std::io::ErrorKind::InvalidInput,
                        "AFC request deadline is too large",
                    )
                },
            )?),
            None => None,
        };
        self.stream.progressed = false;
        let mut request = AfcRequest {
            client: self,
            completed: false,
        };
        let operation = async {
            request.client.send(opcode, header_payload, payload).await?;
            match response_limit {
                Some(limit) => request.client.recv_with_response_limit(Some(limit)).await,
                None => request.client.recv().await,
            }
        };
        let result = match deadline {
            Some(deadline) => match tokio::time::timeout_at(deadline, operation).await {
                Ok(result) => result,
                Err(elapsed) => {
                    // The cancelled operation's guard decides whether any
                    // bytes were transferred. A zero-byte wait is reusable.
                    return Err(std::io::Error::new(std::io::ErrorKind::TimedOut, elapsed).into());
                }
            },
            None => operation.await,
        };
        // A complete device Status consumes its entire frame and leaves the
        // request boundary intact. IO/codec errors cannot certify that boundary.
        request.completed = true;
        if result.is_err() && !matches!(result, Err(AfcError::Status(_))) {
            request.client.usable = false;
        }
        result
    }

    // ── send ─────────────────────────────────────────────────────────────────

    async fn send(
        &mut self,
        opcode: AfcOpcode,
        header_payload: &[u8],
        payload: &[u8],
    ) -> Result<(), AfcError> {
        let pnum = self.next_pnum()?;
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
        self.recv_with_response_limit(None).await
    }

    async fn recv_with_response_limit(
        &mut self,
        response_limit: Option<ReadResponseLimit>,
    ) -> Result<Packet, AfcError> {
        let mut hdr_buf = [0u8; AfcHeader::SIZE];
        self.stream.read_exact(&mut hdr_buf).await?;

        let hdr = AfcHeader::ref_from_bytes(&hdr_buf).map_err(header_error)?;

        if hdr.magic.get() != AFC_MAGIC {
            return Err(AfcError::Protocol(format!(
                "bad AFC magic: 0x{:016X}",
                hdr.magic.get()
            )));
        }

        let entire_len_raw = hdr.entire_len.get();
        let this_len_raw = hdr.this_len.get();
        let opcode = hdr.operation.get();

        // Validate the wire lengths before converting them to usize or
        // deriving allocation sizes.  In particular, saturating subtraction
        // would turn malformed frames into apparently empty packets.
        let header_size = AfcHeader::SIZE as u64;
        if this_len_raw < header_size {
            return Err(AfcError::Protocol(format!(
                "AFC this_len is smaller than the header: {this_len_raw}"
            )));
        }
        if this_len_raw > entire_len_raw {
            return Err(AfcError::Protocol(format!(
                "AFC this_len exceeds entire_len: this_len={this_len_raw} entire_len={entire_len_raw}"
            )));
        }

        // Admit the peer's declared lengths before usize conversion or either
        // body allocation. A rejected frame is not drained or reused.
        if let Some(limit) = response_limit {
            let body_len = entire_len_raw - header_size;
            let payload_len = entire_len_raw - this_len_raw;
            if body_len > limit.body as u64 {
                return Err(bounded_read_limit("reply body", limit.body, body_len));
            }
            if payload_len > limit.payload as u64 {
                return Err(bounded_read_limit("reply data", limit.payload, payload_len));
            }
        }

        let entire_len = usize::try_from(entire_len_raw).map_err(|error| {
            AfcError::Io(crate::error::context::received_io(
                "convert AFC entire_len",
                Some(entire_len_raw),
                error,
            ))
        })?;
        let this_len = usize::try_from(this_len_raw).map_err(|error| {
            AfcError::Io(crate::error::context::received_io(
                "convert AFC this_len",
                Some(this_len_raw),
                error,
            ))
        })?;

        // Sanity check against DoS.  Bound the complete frame body because
        // both vectors below are allocated before the operation is decoded.
        const MAX_AFC_MSG: usize = 256 * 1024 * 1024; // 256 MiB
        let frame_body_len = entire_len - AfcHeader::SIZE;
        if frame_body_len > MAX_AFC_MSG {
            return Err(AfcError::Protocol(format!(
                "AFC frame too large: body={frame_body_len} limit={MAX_AFC_MSG}"
            )));
        }

        let header_payload_len = this_len - AfcHeader::SIZE;
        let payload_len = entire_len - this_len;

        if opcode == AfcOpcode::Status as u64 && header_payload_len < 8 {
            return Err(AfcError::Protocol(
                "AFC Status response is missing its 8-byte status code".into(),
            ));
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
            let code = AfcStatusCode::from_u64(u64::from_le_bytes(
                header_payload[..8].try_into().map_err(|error| {
                    AfcError::Io(crate::error::context::received_io(
                        "decode AFC status",
                        None,
                        error,
                    ))
                })?,
            ));
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
        let pkt = self.request(AfcOpcode::ReadDir, &hp, &[]).await?;
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
        let pkt = self.request(AfcOpcode::GetFileInfo, &hp, &[]).await?;
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
        self.request(AfcOpcode::MakePath, &hp, &[]).await?;
        Ok(())
    }

    /// Remove a path (file or empty directory).
    pub async fn remove(&mut self, path: &str) -> Result<(), AfcError> {
        let mut hp = path.as_bytes().to_vec();
        hp.push(0);
        self.request(AfcOpcode::RemovePath, &hp, &[]).await?;
        Ok(())
    }

    /// Remove a path and all contents recursively.
    pub async fn remove_all(&mut self, path: &str) -> Result<(), AfcError> {
        let mut hp = path.as_bytes().to_vec();
        hp.push(0);
        self.request(AfcOpcode::RemovePathAndContents, &hp, &[])
            .await?;
        Ok(())
    }

    /// Rename / move a path.
    pub async fn rename(&mut self, from: &str, to: &str) -> Result<(), AfcError> {
        let mut hp = from.as_bytes().to_vec();
        hp.push(0);
        hp.extend_from_slice(to.as_bytes());
        hp.push(0);
        self.request(AfcOpcode::RenamePath, &hp, &[]).await?;
        Ok(())
    }

    /// Read an entire file into memory.
    ///
    /// Refuses files above [`Self::MAX_IN_MEMORY_FILE`]; the device decides how
    /// much it sends, and go-ios streams straight to disk for exactly that
    /// reason. Use [`Self::file_read`] in a loop for anything larger.
    pub async fn read_file(&mut self, path: &str) -> Result<Bytes, AfcError> {
        let fd = self.file_open(path, Self::FILE_MODE_READ_ONLY).await?; // READ_ONLY
        let mut transfer = AfcTransfer {
            client: self,
            completed: false,
        };
        let result = async {
            let mut data = BytesMut::new();
            let chunk = 65536u64;
            loop {
                let buf = transfer.client.file_read(fd, chunk).await?;
                if buf.is_empty() {
                    break;
                }
                if data.len() + buf.len() > Self::MAX_IN_MEMORY_FILE {
                    return Err(AfcError::Protocol(format!(
                        "{path} exceeds the {} byte in-memory read limit",
                        Self::MAX_IN_MEMORY_FILE
                    )));
                }
                data.extend_from_slice(&buf);
            }
            Ok(data.freeze())
        }
        .await;
        let cleanup = transfer
            .client
            .close_transfer_file(fd, FILE_CLEANUP_BUDGET)
            .await;
        if cleanup.is_err() {
            transfer.client.usable = false;
        }
        transfer.completed = true;
        finish_file_transfer(result, cleanup)
    }

    /// Read a complete file with a caller-supplied byte ceiling.
    ///
    /// The effective ceiling is the smaller of `max_bytes` and
    /// [`Self::MAX_IN_MEMORY_FILE`]. A larger file returns a local resource-limit
    /// error under the existing [`AfcError::Io`] variant, never a truncated
    /// success. Zero permits only an empty file: a one-byte EOF probe detects
    /// non-empty input. No file size metadata is trusted to establish EOF.
    ///
    /// Open and close replies are admitted against eight body bytes; each read
    /// reply is admitted against its requested chunk (at most 64 KiB, or eight
    /// bytes for a status reply) before body allocation. One extra byte may be
    /// received to detect a file exceeding the ceiling, but is not appended to
    /// the result. These are logical byte bounds, not an allocator/RSS guarantee.
    ///
    /// The configured request timeout applies separately to open, each read,
    /// and close. Cleanup has the existing independent five-second ceiling and
    /// preserves the primary error if cleanup also fails. This method adds no
    /// whole-file deadline or async Drop: cancellation after an open/committed
    /// request invalidates reuse, and the caller must drop the client.
    pub async fn read_file_with_limit(
        &mut self,
        path: &str,
        max_bytes: usize,
    ) -> Result<Bytes, AfcError> {
        let limit = max_bytes.min(Self::MAX_IN_MEMORY_FILE);
        let fd = self
            .file_open_with_response_limit(
                path,
                Self::FILE_MODE_READ_ONLY,
                Some(ReadResponseLimit::CONTROL),
            )
            .await?;
        let mut transfer = AfcTransfer {
            client: self,
            completed: false,
        };
        let result = async {
            let mut data = BytesMut::new();
            loop {
                // The fixed in-memory ceiling makes this addition representable
                // even when the caller supplied usize::MAX.
                let remaining = limit - data.len();
                let requested = (remaining + 1).min(65_536);
                let mut hp = [0u8; 16];
                hp[..8].copy_from_slice(&fd.to_le_bytes());
                hp[8..].copy_from_slice(&(requested as u64).to_le_bytes());
                let packet = transfer
                    .client
                    .request_with_response_limit(
                        AfcOpcode::FileRefRead,
                        &hp,
                        &[],
                        Some(ReadResponseLimit::data(requested)),
                    )
                    .await?;
                if packet.payload.is_empty() {
                    break;
                }
                if packet.payload.len() > remaining {
                    return Err(bounded_read_limit(
                        "file data",
                        limit,
                        data.len() as u64 + packet.payload.len() as u64,
                    ));
                }
                data.extend_from_slice(&packet.payload);
            }
            Ok(data.freeze())
        }
        .await;
        let cleanup = transfer
            .client
            .close_transfer_file_with_response_limit(
                fd,
                FILE_CLEANUP_BUDGET,
                Some(ReadResponseLimit::CONTROL),
            )
            .await;
        if cleanup.is_err() {
            transfer.client.usable = false;
        }
        transfer.completed = true;
        finish_file_transfer(result, cleanup)
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
        let mut reader = std::io::Cursor::new(data);
        self.write_file_from_reader(path, &mut reader).await
    }

    /// Write data from an async reader to a file (creates or truncates).
    pub async fn write_file_from_reader<R>(
        &mut self,
        path: &str,
        reader: &mut R,
    ) -> Result<(), AfcError>
    where
        R: AsyncRead + Unpin,
    {
        let fd = self
            .file_open(path, Self::FILE_MODE_WRITE_ONLY_CREATE_TRUNC)
            .await?;
        let mut transfer = AfcTransfer {
            client: self,
            completed: false,
        };
        let result = async {
            let mut buf = vec![0u8; 1024 * 1024];
            loop {
                let n = reader.read(&mut buf).await?;
                if n == 0 {
                    break;
                }
                transfer.client.file_write(fd, &buf[..n]).await?;
            }
            Ok::<(), AfcError>(())
        }
        .await;
        let cleanup = transfer
            .client
            .close_transfer_file(fd, FILE_CLEANUP_BUDGET)
            .await;
        if cleanup.is_err() {
            transfer.client.usable = false;
        }
        transfer.completed = true;
        finish_file_transfer(result, cleanup)
    }

    pub async fn open_file(&mut self, path: &str, mode: u64) -> Result<u64, AfcError> {
        self.file_open(path, mode).await
    }

    pub async fn lock_file(&mut self, fd: u64, operation: u64) -> Result<(), AfcError> {
        let mut hp = [0u8; 16];
        hp[..8].copy_from_slice(&fd.to_le_bytes());
        hp[8..].copy_from_slice(&operation.to_le_bytes());
        self.request(AfcOpcode::FileRefLock, &hp, &[]).await?;
        Ok(())
    }

    pub async fn close_file(&mut self, fd: u64) -> Result<(), AfcError> {
        self.file_close(fd).await
    }

    /// Get device filesystem info.
    pub async fn device_info(&mut self) -> Result<HashMap<String, String>, AfcError> {
        let pkt = self.request(AfcOpcode::GetDeviceInfo, &[], &[]).await?;
        Ok(parse_kv_pairs(&pkt.payload))
    }

    // ── file handle ops ───────────────────────────────────────────────────────

    async fn file_open(&mut self, path: &str, mode: u64) -> Result<u64, AfcError> {
        self.file_open_with_response_limit(path, mode, None).await
    }

    async fn file_open_with_response_limit(
        &mut self,
        path: &str,
        mode: u64,
        response_limit: Option<ReadResponseLimit>,
    ) -> Result<u64, AfcError> {
        let mut hp = vec![0u8; 8];
        hp[..8].copy_from_slice(&mode.to_le_bytes());
        hp.extend_from_slice(path.as_bytes());
        hp.push(0);
        let pkt = self
            .request_with_response_limit(AfcOpcode::FileRefOpen, &hp, &[], response_limit)
            .await?;
        if pkt.header_payload.len() < 8 {
            self.usable = false;
            return Err(AfcError::Protocol(
                "FileRefOpenResult: short response".into(),
            ));
        }
        let fd = u64::from_le_bytes(pkt.header_payload[..8].try_into().map_err(|error| {
            AfcError::Io(crate::error::context::received_io(
                "decode AFC file handle",
                None,
                error,
            ))
        })?);
        Ok(fd)
    }

    async fn file_read(&mut self, fd: u64, size: u64) -> Result<Bytes, AfcError> {
        let mut hp = [0u8; 16];
        hp[..8].copy_from_slice(&fd.to_le_bytes());
        hp[8..].copy_from_slice(&size.to_le_bytes());
        let pkt = self.request(AfcOpcode::FileRefRead, &hp, &[]).await?;
        Ok(pkt.payload)
    }

    async fn file_write(&mut self, fd: u64, data: &[u8]) -> Result<(), AfcError> {
        let mut hp = [0u8; 8];
        hp.copy_from_slice(&fd.to_le_bytes());
        self.request(AfcOpcode::FileRefWrite, &hp, data).await?;
        Ok(())
    }

    async fn file_close(&mut self, fd: u64) -> Result<(), AfcError> {
        self.file_close_with_response_limit(fd, None).await
    }

    async fn file_close_with_response_limit(
        &mut self,
        fd: u64,
        response_limit: Option<ReadResponseLimit>,
    ) -> Result<(), AfcError> {
        let hp = fd.to_le_bytes();
        self.request_with_response_limit(AfcOpcode::FileRefClose, &hp, &[], response_limit)
            .await?;
        Ok(())
    }

    async fn close_transfer_file(&mut self, fd: u64, budget: Duration) -> Result<(), AfcError> {
        self.close_transfer_file_with_response_limit(fd, budget, None)
            .await
    }

    async fn close_transfer_file_with_response_limit(
        &mut self,
        fd: u64,
        budget: Duration,
        response_limit: Option<ReadResponseLimit>,
    ) -> Result<(), AfcError> {
        match tokio::time::timeout(
            budget,
            self.file_close_with_response_limit(fd, response_limit),
        )
        .await
        {
            Ok(result) => result,
            Err(elapsed) => Err(std::io::Error::new(std::io::ErrorKind::TimedOut, elapsed).into()),
        }
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
            if target.starts_with('/') {
                return target.clone();
            }
            // A relative target is relative to the link's own directory, not to
            // the AFC root (pymobiledevice3 `resolve_path` joins it the same way).
            return match path.rfind('/') {
                Some(0) => format!("/{target}"),
                Some(slash) => format!("{}/{target}", &path[..slash]),
                None => target.clone(),
            };
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
mod bounded_read_tests;

#[cfg(test)]
mod tests {
    async fn lifecycle_read_request(
        peer: &mut tokio::io::DuplexStream,
        opcode: AfcOpcode,
    ) -> (u64, Vec<u8>, Vec<u8>) {
        let mut bytes = [0; AfcHeader::SIZE];
        peer.read_exact(&mut bytes).await.unwrap();
        let header = AfcHeader::ref_from_bytes(&bytes).unwrap();
        assert_eq!(header.magic.get(), AFC_MAGIC);
        assert_eq!(header.operation.get(), opcode as u64);
        let mut hp = vec![0; header.this_len.get() as usize - AfcHeader::SIZE];
        let mut data = vec![0; (header.entire_len.get() - header.this_len.get()) as usize];
        peer.read_exact(&mut hp).await.unwrap();
        peer.read_exact(&mut data).await.unwrap();
        (header.packet_num.get(), hp, data)
    }

    async fn lifecycle_reply(
        peer: &mut tokio::io::DuplexStream,
        id: u64,
        opcode: AfcOpcode,
        hp: &[u8],
        data: &[u8],
    ) {
        let header = AfcHeader::new(id, opcode, hp.len(), data.len());
        peer.write_all(header.as_bytes()).await.unwrap();
        peer.write_all(hp).await.unwrap();
        peer.write_all(data).await.unwrap();
    }

    fn lifecycle_elapsed(error: &AfcError) -> &tokio::time::error::Elapsed {
        let AfcError::Io(io) = error else {
            panic!("native timeout flattened")
        };
        assert_eq!(io.kind(), std::io::ErrorKind::TimedOut);
        io.get_ref().unwrap().downcast_ref().unwrap()
    }

    #[tokio::test]
    async fn lifecycle_afc_zero_overflow_unpolled_have_no_io() {
        use std::error::Error;
        let (stream, mut peer) = tokio::io::duplex(4096);
        let mut client = AfcClient::new(stream);
        for (budget, kind) in [
            (Duration::ZERO, std::io::ErrorKind::TimedOut),
            (Duration::MAX, std::io::ErrorKind::InvalidInput),
        ] {
            client.set_request_timeout(Some(budget));
            let error = client.stat("owned-private-path").await.unwrap_err();
            let AfcError::Io(io) = error else {
                panic!("typed local decision lost")
            };
            assert_eq!(io.kind(), kind);
            assert!(io.source().is_none());
            assert!(client.is_usable());
            assert_eq!(client.packet_num, 1);
        }
        client.set_request_timeout(None);
        drop(client.list_dir("owned-private-path"));
        assert!(client.is_usable());
        assert_eq!(client.packet_num, 1);
        assert!(
            tokio::time::timeout(Duration::from_millis(10), peer.read(&mut [0; 1]))
                .await
                .is_err()
        );
        client.packet_num = u64::MAX;
        assert!(matches!(
            client.list_dir("/").await,
            Err(AfcError::Protocol(_))
        ));
        assert_eq!(client.packet_num, u64::MAX);
        assert!(!client.is_usable());
        assert!(
            tokio::time::timeout(Duration::from_millis(10), peer.read(&mut [0; 1]))
                .await
                .is_err()
        );
    }

    struct LifecycleStalledWrite {
        stream: tokio::io::DuplexStream,
        stalled: std::sync::Arc<std::sync::atomic::AtomicBool>,
    }
    impl AsyncRead for LifecycleStalledWrite {
        fn poll_read(
            mut self: Pin<&mut Self>,
            cx: &mut Context<'_>,
            buf: &mut ReadBuf<'_>,
        ) -> Poll<std::io::Result<()>> {
            Pin::new(&mut self.stream).poll_read(cx, buf)
        }
    }
    impl AsyncWrite for LifecycleStalledWrite {
        fn poll_write(
            mut self: Pin<&mut Self>,
            cx: &mut Context<'_>,
            bytes: &[u8],
        ) -> Poll<std::io::Result<usize>> {
            if self.stalled.load(std::sync::atomic::Ordering::Relaxed) {
                Poll::Pending
            } else {
                Pin::new(&mut self.stream).poll_write(cx, bytes)
            }
        }
        fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
            Pin::new(&mut self.stream).poll_flush(cx)
        }
        fn poll_shutdown(
            mut self: Pin<&mut Self>,
            cx: &mut Context<'_>,
        ) -> Poll<std::io::Result<()>> {
            Pin::new(&mut self.stream).poll_shutdown(cx)
        }
    }

    #[tokio::test]
    async fn lifecycle_afc_zero_byte_deadline_and_caller_cancel_keep_reuse() {
        for own_budget in [true, false] {
            let (stream, mut peer) = tokio::io::duplex(4096);
            let stalled = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(true));
            let mut client = AfcClient::new(LifecycleStalledWrite {
                stream,
                stalled: stalled.clone(),
            });
            if own_budget {
                client.set_request_timeout(Some(Duration::from_millis(20)));
                lifecycle_elapsed(&client.list_dir("owned path").await.unwrap_err());
            } else {
                assert!(tokio::time::timeout(
                    Duration::from_millis(20),
                    client.list_dir("owned path")
                )
                .await
                .is_err());
            }
            assert!(client.is_usable());
            assert_eq!(client.packet_num, 2); // no wire progress, identifier skipped
            assert!(
                tokio::time::timeout(Duration::from_millis(10), peer.read(&mut [0; 1]))
                    .await
                    .is_err()
            );
            stalled.store(false, std::sync::atomic::Ordering::Relaxed);
            client.set_request_timeout(None);
            let caller = client.list_dir("owned path");
            let server = async {
                let (id, hp, data) = lifecycle_read_request(&mut peer, AfcOpcode::ReadDir).await;
                assert_eq!(id, 2);
                assert_eq!(hp, b"owned path\0");
                assert!(data.is_empty());
                lifecycle_reply(&mut peer, id, AfcOpcode::Data, &[], b"entry\0").await;
            };
            let (result, ()) = tokio::time::timeout(Duration::from_secs(1), async {
                tokio::join!(caller, server)
            })
            .await
            .unwrap();
            assert_eq!(result.unwrap(), vec!["entry"]);
            assert!(client.is_usable());
        }
    }

    #[tokio::test]
    async fn lifecycle_afc_cancel_after_send_rejects_all_requests_and_leaves_late_reply() {
        let (stream, mut peer) = tokio::io::duplex(4096);
        let mut client = AfcClient::new(stream);
        let (cancelled, after_cancel) = tokio::sync::oneshot::channel();
        let caller = async {
            assert!(tokio::time::timeout(
                Duration::from_millis(20),
                client.list_dir("owned-private-path")
            )
            .await
            .is_err());
            assert!(!client.is_usable());
            cancelled.send(()).unwrap();
            assert!(matches!(
                client.list_dir("/").await,
                Err(AfcError::Protocol(_))
            ));
            assert!(matches!(client.stat("/").await, Err(AfcError::Protocol(_))));
            assert!(matches!(
                client.stat_info("/").await,
                Err(AfcError::Protocol(_))
            ));
            assert!(matches!(
                client.make_dir("/").await,
                Err(AfcError::Protocol(_))
            ));
            assert!(matches!(
                client.remove("/").await,
                Err(AfcError::Protocol(_))
            ));
            assert!(matches!(
                client.remove_all("/").await,
                Err(AfcError::Protocol(_))
            ));
            assert!(matches!(
                client.rename("/a", "/b").await,
                Err(AfcError::Protocol(_))
            ));
            assert!(matches!(
                client.device_info().await,
                Err(AfcError::Protocol(_))
            ));
            assert!(matches!(
                client.open_file("/a", 1).await,
                Err(AfcError::Protocol(_))
            ));
            assert!(matches!(
                client.close_file(7).await,
                Err(AfcError::Protocol(_))
            ));
            assert!(matches!(
                client.lock_file(7, 6).await,
                Err(AfcError::Protocol(_))
            ));
            assert!(matches!(
                client.read_file("/a").await,
                Err(AfcError::Protocol(_))
            ));
            assert!(matches!(
                client.read_file_follow_links("/a").await,
                Err(AfcError::Protocol(_))
            ));
            assert!(matches!(
                client.write_file("/a", b"data").await,
                Err(AfcError::Protocol(_))
            ));
        };
        let server = async {
            let (id, hp, _) = lifecycle_read_request(&mut peer, AfcOpcode::ReadDir).await;
            assert_eq!(id, 1);
            assert_eq!(hp, b"owned-private-path\0");
            after_cancel.await.unwrap();
            let late = afc_frame(AfcOpcode::Data, &[], b"late entry\0");
            peer.write_all(&late).await.unwrap();
            assert!(
                tokio::time::timeout(Duration::from_millis(10), peer.read(&mut [0; 1]))
                    .await
                    .is_err()
            );
            late
        };
        let ((), late) = tokio::time::timeout(Duration::from_secs(1), async {
            tokio::join!(caller, server)
        })
        .await
        .unwrap();
        // Inspect the owned raw transport only after the public client has
        // rejected reuse: the complete late reply remains unread.
        let mut unread = vec![0; late.len()];
        client.stream.inner.read_exact(&mut unread).await.unwrap();
        assert_eq!(unread, late);
    }

    #[tokio::test]
    async fn lifecycle_afc_partial_header_body_eof_and_bad_magic_invalidate() {
        for case in 0..4 {
            let (stream, mut peer) = tokio::io::duplex(4096);
            let mut client = AfcClient::new(stream);
            client.set_request_timeout(Some(Duration::from_millis(20)));
            let caller = client.list_dir("/");
            let server = async {
                lifecycle_read_request(&mut peer, AfcOpcode::ReadDir).await;
                let mut frame = afc_frame(AfcOpcode::Data, &[], b"names\0");
                match case {
                    0 => peer.write_all(&frame[..13]).await.unwrap(),
                    1 => peer.write_all(&frame[..AfcHeader::SIZE + 1]).await.unwrap(),
                    2 => {
                        drop(peer);
                        return None;
                    }
                    _ => {
                        frame[0] ^= 1;
                        peer.write_all(&frame).await.unwrap();
                    }
                }
                Some(peer)
            };
            let (result, _peer) = tokio::time::timeout(Duration::from_secs(1), async {
                tokio::join!(caller, server)
            })
            .await
            .unwrap();
            let error = result.unwrap_err();
            if case < 2 {
                lifecycle_elapsed(&error);
            } else if case == 2 {
                assert!(
                    matches!(&error, AfcError::Io(io) if io.kind() == std::io::ErrorKind::UnexpectedEof)
                );
            } else {
                assert!(matches!(&error, AfcError::Protocol(_)));
            }
            assert!(!client.is_usable());
            assert!(matches!(client.stat("/").await, Err(AfcError::Protocol(_))));
            assert!(!format!("{error:?}").contains("names"));
        }
    }

    #[tokio::test]
    async fn lifecycle_afc_blocked_partial_send_shares_request_budget() {
        let (stream, mut peer) = tokio::io::duplex(16);
        let mut client = AfcClient::new(stream);
        client.set_request_timeout(Some(Duration::from_millis(20)));
        lifecycle_elapsed(&client.list_dir("owned path").await.unwrap_err());
        assert!(!client.is_usable());
        assert!(matches!(
            client.make_dir("/").await,
            Err(AfcError::Protocol(_))
        ));
        drop(client);
        let mut prefix = Vec::new();
        peer.read_to_end(&mut prefix).await.unwrap();
        assert_eq!(prefix.len(), 16);
        assert_eq!(
            prefix,
            AfcHeader::new(1, AfcOpcode::ReadDir, b"owned path\0".len(), 0).as_bytes()[..16]
        );
    }

    #[tokio::test]
    async fn lifecycle_afc_complete_device_status_remains_reusable() {
        let (stream, mut peer) = tokio::io::duplex(4096);
        let mut client = AfcClient::new(stream);
        client.set_request_timeout(Some(Duration::from_secs(1)));
        let caller = async {
            assert!(matches!(
                client.remove("owned path").await,
                Err(AfcError::Status(AfcStatusCode::ObjectNotFound))
            ));
            assert!(client.is_usable());
            assert_eq!(client.list_dir("/").await.unwrap(), vec!["entry"]);
            assert!(client.is_usable());
            assert!(matches!(
                client.open_file("owned path", 1).await,
                Err(AfcError::Protocol(_))
            ));
            assert!(!client.is_usable());
        };
        let server = async {
            let (id, hp, _) = lifecycle_read_request(&mut peer, AfcOpcode::RemovePath).await;
            assert_eq!(hp, b"owned path\0");
            lifecycle_reply(&mut peer, id, AfcOpcode::Status, &8u64.to_le_bytes(), &[]).await;
            let (next, _, _) = lifecycle_read_request(&mut peer, AfcOpcode::ReadDir).await;
            assert_eq!(next, id + 1);
            lifecycle_reply(&mut peer, next, AfcOpcode::Data, &[], b"entry\0").await;
            let (id, _, _) = lifecycle_read_request(&mut peer, AfcOpcode::FileRefOpen).await;
            assert_eq!(id, next + 1);
            lifecycle_reply(&mut peer, id, AfcOpcode::FileRefOpenResult, &[0; 7], &[]).await;
        };
        tokio::time::timeout(Duration::from_secs(2), async {
            tokio::join!(caller, server)
        })
        .await
        .unwrap();
    }

    #[tokio::test]
    async fn lifecycle_afc_read_failure_closes_handle_and_retains_both_statuses() {
        for close_fails in [false, true] {
            let (stream, mut peer) = tokio::io::duplex(4096);
            let mut client = AfcClient::new(stream);
            let caller = client.read_file("owned-private-path");
            let server = async {
                let (id, hp, _) = lifecycle_read_request(&mut peer, AfcOpcode::FileRefOpen).await;
                assert_eq!(
                    &hp[..8],
                    &AfcClient::<tokio::io::DuplexStream>::FILE_MODE_READ_ONLY.to_le_bytes()
                );
                lifecycle_reply(
                    &mut peer,
                    id,
                    AfcOpcode::FileRefOpenResult,
                    &7u64.to_le_bytes(),
                    &[],
                )
                .await;
                let (id, hp, _) = lifecycle_read_request(&mut peer, AfcOpcode::FileRefRead).await;
                assert_eq!(&hp[..8], &7u64.to_le_bytes());
                assert_eq!(&hp[8..], &65536u64.to_le_bytes());
                lifecycle_reply(&mut peer, id, AfcOpcode::Status, &8u64.to_le_bytes(), &[]).await;
                let (id, hp, _) = lifecycle_read_request(&mut peer, AfcOpcode::FileRefClose).await;
                assert_eq!(hp, 7u64.to_le_bytes());
                lifecycle_reply(
                    &mut peer,
                    id,
                    AfcOpcode::Status,
                    &(if close_fails { 10u64 } else { 0 }).to_le_bytes(),
                    &[],
                )
                .await;
            };
            let (result, ()) = tokio::time::timeout(Duration::from_secs(2), async {
                tokio::join!(caller, server)
            })
            .await
            .unwrap();
            let error = result.unwrap_err();
            assert_eq!(crate::classify_error(&error), crate::ErrorClass::NotFound);
            assert_eq!(client.is_usable(), !close_fails);
            if close_fails {
                let AfcError::Io(io) = &error else {
                    panic!("cleanup cause lost")
                };
                let failure = io
                    .get_ref()
                    .unwrap()
                    .downcast_ref::<FileCleanupFailure>()
                    .unwrap();
                assert!(matches!(
                    failure.primary,
                    AfcError::Status(AfcStatusCode::ObjectNotFound)
                ));
                assert!(matches!(
                    failure.cleanup,
                    AfcError::Status(AfcStatusCode::PermDenied)
                ));
                assert!(!format!("{failure:?}").contains("owned-private"));
            } else {
                assert!(matches!(
                    error,
                    AfcError::Status(AfcStatusCode::ObjectNotFound)
                ));
            }
        }
    }

    #[tokio::test]
    async fn lifecycle_afc_read_success_and_cleanup_only_failure() {
        for close_fails in [false, true] {
            let (stream, mut peer) = tokio::io::duplex(4096);
            let mut client = AfcClient::new(stream);
            let caller = client.read_file("owned path");
            let server = async {
                let (id, _, _) = lifecycle_read_request(&mut peer, AfcOpcode::FileRefOpen).await;
                lifecycle_reply(
                    &mut peer,
                    id,
                    AfcOpcode::FileRefOpenResult,
                    &7u64.to_le_bytes(),
                    &[],
                )
                .await;
                for data in [b"content".as_slice(), b"".as_slice()] {
                    let (id, _, _) =
                        lifecycle_read_request(&mut peer, AfcOpcode::FileRefRead).await;
                    lifecycle_reply(&mut peer, id, AfcOpcode::Data, &[], data).await;
                }
                let (id, _, _) = lifecycle_read_request(&mut peer, AfcOpcode::FileRefClose).await;
                lifecycle_reply(
                    &mut peer,
                    id,
                    AfcOpcode::Status,
                    &(if close_fails { 10u64 } else { 0 }).to_le_bytes(),
                    &[],
                )
                .await;
            };
            let (result, ()) = tokio::time::timeout(Duration::from_secs(2), async {
                tokio::join!(caller, server)
            })
            .await
            .unwrap();
            if close_fails {
                assert!(matches!(
                    result,
                    Err(AfcError::Status(AfcStatusCode::PermDenied))
                ));
            } else {
                assert_eq!(result.unwrap().as_ref(), b"content");
            }
            assert_eq!(client.is_usable(), !close_fails);
        }
    }

    struct LifecycleFailingReader(Option<std::io::Error>);
    impl AsyncRead for LifecycleFailingReader {
        fn poll_read(
            mut self: Pin<&mut Self>,
            _: &mut Context<'_>,
            _: &mut ReadBuf<'_>,
        ) -> Poll<std::io::Result<()>> {
            Poll::Ready(Err(self.0.take().unwrap()))
        }
    }

    #[tokio::test]
    async fn lifecycle_afc_host_reader_failure_closes_file_without_flattening() {
        for close_fails in [false, true] {
            let native = std::fs::File::open("").unwrap_err();
            let code = native.raw_os_error();
            let mut reader = LifecycleFailingReader(Some(native));
            let (stream, mut peer) = tokio::io::duplex(4096);
            let mut client = AfcClient::new(stream);
            let caller = client.write_file_from_reader("owned-private-path", &mut reader);
            let server = async {
                let (id, hp, _) = lifecycle_read_request(&mut peer, AfcOpcode::FileRefOpen).await;
                assert_eq!(
                    &hp[..8],
                    &AfcClient::<tokio::io::DuplexStream>::FILE_MODE_WRITE_ONLY_CREATE_TRUNC
                        .to_le_bytes()
                );
                lifecycle_reply(
                    &mut peer,
                    id,
                    AfcOpcode::FileRefOpenResult,
                    &7u64.to_le_bytes(),
                    &[],
                )
                .await;
                let (id, hp, data) =
                    lifecycle_read_request(&mut peer, AfcOpcode::FileRefClose).await;
                assert_eq!(hp, 7u64.to_le_bytes());
                assert!(data.is_empty());
                lifecycle_reply(
                    &mut peer,
                    id,
                    AfcOpcode::Status,
                    &(if close_fails { 10u64 } else { 0 }).to_le_bytes(),
                    &[],
                )
                .await;
            };
            let (result, ()) = tokio::time::timeout(Duration::from_secs(2), async {
                tokio::join!(caller, server)
            })
            .await
            .unwrap();
            let error = result.unwrap_err();
            let AfcError::Io(io) = &error else {
                panic!("native host IO lost")
            };
            let original = if close_fails {
                let failure = io
                    .get_ref()
                    .unwrap()
                    .downcast_ref::<FileCleanupFailure>()
                    .unwrap();
                assert!(matches!(
                    failure.cleanup,
                    AfcError::Status(AfcStatusCode::PermDenied)
                ));
                let AfcError::Io(primary) = &failure.primary else {
                    panic!("native primary lost")
                };
                primary
            } else {
                io
            };
            assert_eq!(original.kind(), std::io::ErrorKind::NotFound);
            assert_eq!(original.raw_os_error(), code);
            assert_eq!(crate::classify_error(&error), crate::ErrorClass::NotFound);
            assert_eq!(client.is_usable(), !close_fails);
        }
    }

    #[tokio::test]
    async fn lifecycle_afc_public_read_cleanup_has_independent_five_second_budget() {
        let (stream, mut peer) = tokio::io::duplex(4096);
        let mut client = AfcClient::new(stream); // legacy requests are unbounded
        let caller = client.read_file("owned path");
        let server = async {
            let (id, _, _) = lifecycle_read_request(&mut peer, AfcOpcode::FileRefOpen).await;
            lifecycle_reply(
                &mut peer,
                id,
                AfcOpcode::FileRefOpenResult,
                &7u64.to_le_bytes(),
                &[],
            )
            .await;
            let (id, _, _) = lifecycle_read_request(&mut peer, AfcOpcode::FileRefRead).await;
            lifecycle_reply(&mut peer, id, AfcOpcode::Status, &8u64.to_le_bytes(), &[]).await;
            let (_, hp, _) = lifecycle_read_request(&mut peer, AfcOpcode::FileRefClose).await;
            assert_eq!(hp, 7u64.to_le_bytes());
            peer // keep the transport alive, deliberately withhold close reply
        };
        let (result, mut peer) = tokio::time::timeout(Duration::from_secs(6), async {
            tokio::join!(caller, server)
        })
        .await
        .unwrap();
        let error = result.unwrap_err();
        let AfcError::Io(io) = &error else {
            panic!("dual failure lost")
        };
        let failure = io
            .get_ref()
            .unwrap()
            .downcast_ref::<FileCleanupFailure>()
            .unwrap();
        assert!(matches!(
            failure.primary,
            AfcError::Status(AfcStatusCode::ObjectNotFound)
        ));
        lifecycle_elapsed(&failure.cleanup);
        assert_eq!(crate::classify_error(&error), crate::ErrorClass::NotFound);
        assert!(!client.is_usable());
        assert!(matches!(
            client.close_file(7).await,
            Err(AfcError::Protocol(_))
        ));
        drop(client);
        assert_eq!(peer.read(&mut [0; 1]).await.unwrap(), 0);
    }

    #[tokio::test]
    async fn lifecycle_afc_transfer_cancel_between_requests_invalidates_owner() {
        let (stream, mut peer) = tokio::io::duplex(4096);
        let (mut reader, _reader_peer) = tokio::io::duplex(64);
        let mut client = AfcClient::new(stream);
        let caller = tokio::time::timeout(
            Duration::from_millis(20),
            client.write_file_from_reader("owned path", &mut reader),
        );
        let server = async {
            let (id, _, _) = lifecycle_read_request(&mut peer, AfcOpcode::FileRefOpen).await;
            lifecycle_reply(
                &mut peer,
                id,
                AfcOpcode::FileRefOpenResult,
                &7u64.to_le_bytes(),
                &[],
            )
            .await;
            peer
        };
        let (result, mut peer) = tokio::time::timeout(Duration::from_secs(1), async {
            tokio::join!(caller, server)
        })
        .await
        .unwrap();
        assert!(result.is_err());
        assert!(!client.is_usable());
        assert!(matches!(
            client.list_dir("/").await,
            Err(AfcError::Protocol(_))
        ));
        assert!(
            tokio::time::timeout(Duration::from_millis(10), peer.read(&mut [0; 1]))
                .await
                .is_err()
        );
        drop(client);
        assert_eq!(peer.read(&mut [0; 1]).await.unwrap(), 0);
    }

    #[test]
    fn guarded_sources_afc_cast_owns_original_short_source_and_keeps_protocol_role() {
        use super::*;
        use std::error::Error;
        let error = {
            let bytes = [0x81; 39];
            header_error(AfcHeader::ref_from_bytes(&bytes).unwrap_err())
        };
        assert_eq!(crate::classify_error(&error), crate::ErrorClass::Protocol);
        let AfcError::Io(io) = &error else {
            panic!("original cast lost")
        };
        assert_eq!(io.kind(), std::io::ErrorKind::InvalidData);
        let cast = io
            .source()
            .unwrap()
            .downcast_ref::<zerocopy::CastError<Box<[u8]>, AfcHeader>>()
            .unwrap();
        assert!(matches!(cast, zerocopy::ConvertError::Size(_)));
        assert_eq!(cast.clone().into_src().as_ref(), &[0x81; 39]);
        let raw_source = format!("{:?}", cast.clone().into_src());
        // A process-keyed correlation can contain a short numeric substring;
        // check the actual source representation rather than three digits.
        assert!(!format!("{error:?} {:?}", io.get_ref()).contains(&raw_source));
        // Production reads exactly 40 bytes and this header has alignment 1:
        // the short-source fixture tests ownership, not a reachable recv fault.
        assert_eq!(std::mem::size_of::<AfcHeader>(), AfcHeader::SIZE);
        assert_eq!(std::mem::align_of::<AfcHeader>(), 1);
    }

    #[tokio::test]
    async fn guarded_sources_afc_huge_length_reaches_host_specific_rejection() {
        use std::error::Error;
        let mut header = AfcHeader::new(1, AfcOpcode::Data, 0, 0);
        header.entire_len = zerocopy::byteorder::U64::new(u64::MAX);
        let mut stream = MockStream::new(header.as_bytes().to_vec());
        let mut client = AfcClient::new(&mut stream);
        let error = match client.recv().await {
            Err(error) => error,
            Ok(_) => panic!("unbounded frame admitted"),
        };
        assert_eq!(crate::classify_error(&error), crate::ErrorClass::Protocol);
        if usize::BITS < 64 {
            let AfcError::Io(io) = &error else {
                panic!("native conversion lost")
            };
            assert!(io.source().unwrap().is::<std::num::TryFromIntError>());
        } else {
            assert!(matches!(error, AfcError::Protocol(_)));
            assert!(error.to_string().contains("frame too large"));
        }
        assert_eq!(stream.read_pos, AfcHeader::SIZE);
        assert!(stream.written.is_empty());
    }
    use crate::test_util::MockStream;

    use super::*;
    use zerocopy::FromBytes;

    fn afc_frame(opcode: AfcOpcode, header_payload: &[u8], payload: &[u8]) -> Vec<u8> {
        let header = AfcHeader::new(1, opcode, header_payload.len(), payload.len());
        let mut frame = header.as_bytes().to_vec();
        frame.extend_from_slice(header_payload);
        frame.extend_from_slice(payload);
        frame
    }

    fn afc_frame_with_lengths(
        opcode: AfcOpcode,
        entire_len: u64,
        this_len: u64,
        body: &[u8],
    ) -> Vec<u8> {
        let mut frame = afc_frame(opcode, &[], body);
        frame[8..16].copy_from_slice(&entire_len.to_le_bytes());
        frame[16..24].copy_from_slice(&this_len.to_le_bytes());
        frame
    }

    fn afc_status_success_frame() -> Vec<u8> {
        afc_frame(AfcOpcode::Status, &0u64.to_le_bytes(), &[])
    }

    fn afc_open_result_frame(fd: u64) -> Vec<u8> {
        afc_frame(AfcOpcode::FileRefOpenResult, &fd.to_le_bytes(), &[])
    }

    fn afc_write_success_responses(write_count: usize) -> Vec<u8> {
        let mut responses = afc_open_result_frame(42);
        for _ in 0..write_count {
            responses.extend_from_slice(&afc_status_success_frame());
        }
        responses.extend_from_slice(&afc_status_success_frame());
        responses
    }

    fn file_write_payloads(written: &[u8]) -> Vec<Vec<u8>> {
        let mut pos = 0;
        let mut payloads = Vec::new();
        while pos + AfcHeader::SIZE <= written.len() {
            let header = AfcHeader::ref_from_bytes(&written[pos..pos + AfcHeader::SIZE]).unwrap();
            let entire_len = header.entire_len.get() as usize;
            let this_len = header.this_len.get() as usize;
            let payload_start = pos + this_len;
            let payload_end = pos + entire_len;
            if header.operation.get() == AfcOpcode::FileRefWrite as u64 {
                payloads.push(written[payload_start..payload_end].to_vec());
            }
            pos += entire_len;
        }
        assert_eq!(pos, written.len());
        payloads
    }

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

    #[tokio::test]
    async fn recv_rejects_this_len_smaller_than_header() {
        // The extra byte lets the old saturating-subtraction code return Ok
        // instead of failing while trying to read a missing payload byte.
        let frame = afc_frame_with_lengths(AfcOpcode::ReadDir, 40, 39, &[0]);
        let mut stream = MockStream::new(frame);
        let mut client = AfcClient::new(&mut stream);

        let err = match client.recv().await {
            Ok(_) => panic!("malformed AFC frame was accepted"),
            Err(err) => err,
        };
        assert!(
            matches!(err, AfcError::Protocol(message) if message.contains("smaller than the header"))
        );
    }

    #[tokio::test]
    async fn recv_rejects_this_len_larger_than_entire_len() {
        // The extra byte lets the old saturating-subtraction code return Ok
        // even though this_len claims bytes beyond the end of the frame.
        let frame = afc_frame_with_lengths(AfcOpcode::ReadDir, 40, 41, &[0]);
        let mut stream = MockStream::new(frame);
        let mut client = AfcClient::new(&mut stream);

        let err = match client.recv().await {
            Ok(_) => panic!("malformed AFC frame was accepted"),
            Err(err) => err,
        };
        assert!(
            matches!(err, AfcError::Protocol(message) if message.contains("exceeds entire_len"))
        );
    }

    #[tokio::test]
    async fn recv_rejects_huge_frame_without_reading_or_allocating_body() {
        // Only the 40-byte header is present.  The frame must be rejected from
        // its declared size, without attempting a giant allocation or waiting
        // for a body that cannot be present in this fake transport.
        let frame = afc_frame_with_lengths(AfcOpcode::ReadDir, u64::MAX, u64::MAX, &[]);
        let mut stream = MockStream::new(frame);
        let mut client = AfcClient::new(&mut stream);

        let err = match client.recv().await {
            Ok(_) => panic!("oversized AFC frame was accepted"),
            Err(err) => err,
        };
        assert!(matches!(err, AfcError::Protocol(_)));
    }

    #[tokio::test]
    async fn recv_rejects_status_without_a_complete_code() {
        for header_payload_len in 0..8 {
            let header_payload = vec![0u8; header_payload_len];
            let frame = afc_frame(AfcOpcode::Status, &header_payload, &[]);
            let mut stream = MockStream::new(frame);
            let mut client = AfcClient::new(&mut stream);

            let err = match client.recv().await {
                Ok(_) => panic!("short AFC Status response was treated as success"),
                Err(err) => err,
            };
            assert!(
                matches!(err, AfcError::Protocol(message) if message.contains("8-byte status code")),
                "header payload length {header_payload_len}"
            );
        }
    }

    #[tokio::test]
    async fn recv_rejects_short_status_before_reading_declared_payload() {
        let declared_payload_len = 1024 * 1024;
        let frame = afc_frame_with_lengths(
            AfcOpcode::Status,
            (AfcHeader::SIZE + declared_payload_len) as u64,
            AfcHeader::SIZE as u64,
            &[0],
        );
        let mut stream = MockStream::new(frame);
        let mut client = AfcClient::new(&mut stream);

        let err = match client.recv().await {
            Ok(_) => panic!("short AFC Status response was accepted"),
            Err(err) => err,
        };
        assert!(
            matches!(err, AfcError::Protocol(message) if message.contains("8-byte status code"))
        );
        assert_eq!(stream.read_pos, AfcHeader::SIZE);
    }

    #[tokio::test]
    async fn recv_preserves_success_and_object_not_found_statuses() {
        let mut success_stream = MockStream::new(afc_status_success_frame());
        let mut success_client = AfcClient::new(&mut success_stream);
        success_client.recv().await.unwrap();

        let mut not_found_stream =
            MockStream::new(afc_frame(AfcOpcode::Status, &8u64.to_le_bytes(), &[]));
        let mut not_found_client = AfcClient::new(&mut not_found_stream);
        let err = match not_found_client.recv().await {
            Ok(_) => panic!("AFC status 8 was accepted as success"),
            Err(err) => err,
        };
        assert!(matches!(
            err,
            AfcError::Status(AfcStatusCode::ObjectNotFound)
        ));
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
    fn test_resolve_link_target_joins_relative_target_to_the_links_directory() {
        let mut raw = HashMap::new();
        raw.insert("st_ifmt".to_string(), "S_IFLNK".to_string());
        raw.insert("st_linktarget".to_string(), "real-file".to_string());
        let info = parse_file_info("/var/mobile/link", raw);

        let resolved = resolve_link_target("/var/mobile/link", &info);
        assert_eq!(resolved, "/var/mobile/real-file");
    }

    #[test]
    fn test_resolve_link_target_joins_relative_target_at_the_root() {
        let mut raw = HashMap::new();
        raw.insert("st_ifmt".to_string(), "S_IFLNK".to_string());
        raw.insert("st_linktarget".to_string(), "real-file".to_string());
        let info = parse_file_info("/link", raw);

        let resolved = resolve_link_target("/link", &info);
        assert_eq!(resolved, "/real-file");
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

    #[tokio::test]
    async fn write_file_from_reader_sends_chunks_without_prebuffering() {
        let payload = vec![0xAB; 1024 * 1024 + 17];
        let mut reader = std::io::Cursor::new(payload.clone());
        let mut stream = MockStream::new(afc_write_success_responses(2));
        let mut client = AfcClient::new(&mut stream);

        client
            .write_file_from_reader("/PublicStaging/app.ipa", &mut reader)
            .await
            .unwrap();

        let payloads = file_write_payloads(&stream.written);
        assert_eq!(payloads.len(), 2);
        assert_eq!(payloads[0].len(), 1024 * 1024);
        assert_eq!(payloads[1].len(), 17);
        assert_eq!(
            payloads.concat(),
            payload,
            "AFC file writes should preserve the streamed payload"
        );
    }
}
