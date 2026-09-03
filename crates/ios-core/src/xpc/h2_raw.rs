//! Minimal raw HTTP/2 framer for iOS XPC protocol.
//!
//! Apple's XPC-over-HTTP/2 does NOT use standard HTTP semantics.
//! It uses raw HTTP/2 frames with two fixed stream IDs:
//!   - Stream 1: clientServer  (client → device)
//!   - Stream 3: serverClient  (device → client)
//!
//! Reference: go-ios/ios/http/http.go

use std::collections::{HashMap, HashSet, VecDeque};

use bytes::{Bytes, BytesMut};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

use crate::xpc::message::{checked_xpc_body_len, xpc_body_limit_for_flags, WRAPPER_MAGIC};

// ── Stream IDs ──────────────────────────────────────────────────────────────
// Apple's XPC-over-HTTP/2 uses only odd-numbered client-initiated streams.
// Stream 0 is the HTTP/2 connection control stream (per RFC 9113).
// Stream 2 is skipped because HTTP/2 even-numbered streams are reserved for
// server-initiated (push) streams, which this protocol does not use.

pub const STREAM_INIT: u32 = 0; // HTTP/2 connection-level control stream
pub const STREAM_CLIENT_SERVER: u32 = 1; // Client → device data stream
pub const STREAM_SERVER_CLIENT: u32 = 3; // Device → client data stream

// ── Frame types ─────────────────────────────────────────────────────────────

const FRAME_DATA: u8 = 0x00;
const FRAME_HEADERS: u8 = 0x01;
const FRAME_SETTINGS: u8 = 0x04;
const FRAME_RST_STREAM: u8 = 0x03;
const FRAME_GOAWAY: u8 = 0x07;
const FRAME_WINDOW_UPDATE: u8 = 0x08;

const FLAG_END_STREAM: u8 = 0x01;
const FLAG_END_HEADERS: u8 = 0x04;
const FLAG_SETTINGS_ACK: u8 = 0x01;

// ── Settings IDs ────────────────────────────────────────────────────────────

const SETTINGS_MAX_CONCURRENT_STREAMS: u16 = 0x03;
const SETTINGS_INITIAL_WINDOW_SIZE: u16 = 0x04;
const SETTINGS_MAX_FRAME_SIZE: u16 = 0x05;

// ── Flow control ────────────────────────────────────────────────────────────

/// Per-stream receive window we advertise, matching pymobiledevice3's
/// `DEFAULT_SETTINGS_INITIAL_WINDOW_SIZE`.
const INITIAL_WINDOW_SIZE: u32 = 16 * 1024 * 1024;

/// HTTP/2's default peer receive window for data we send.
const DEFAULT_PEER_WINDOW_SIZE: i64 = 65_535;

/// HTTP/2 flow-control windows are limited to a signed 31-bit value. A
/// SETTINGS_INITIAL_WINDOW_SIZE update may temporarily make an existing
/// stream's window negative, so stream windows are stored as i64 below.
const MAX_FLOW_CONTROL_WINDOW: i64 = 0x7fff_ffff;

/// The peer's SETTINGS_MAX_FRAME_SIZE may grow the frame size, but never past
/// the 24-bit length field. The inbound limit remains MAX_FRAME_PAYLOAD because
/// it is the limit advertised by this client.
const MAX_NEGOTIATED_FRAME_SIZE: usize = 0x00ff_ffff;

/// Connection-window bump needed to reach [`INITIAL_WINDOW_SIZE`] from the
/// RFC 9113 default of 65535.
const INITIAL_WINDOW_INCREMENT: u32 = INITIAL_WINDOW_SIZE - 65_535;

/// Consume this much on a window before spending a WINDOW_UPDATE frame on it,
/// matching pymobiledevice3's `WINDOW_UPDATE_THRESHOLD`. With the 16 MiB initial
/// window this keeps every window above 15 MiB without a frame pair per DATA.
const WINDOW_UPDATE_THRESHOLD: u32 = 1024 * 1024;

/// Largest frame payload we will accept.
///
/// We never advertise `SETTINGS_MAX_FRAME_SIZE`, so RFC 9113 fixes the peer's
/// limit at the 16 KiB default. Without this check the 24-bit length field lets
/// a device drive repeated 16 MiB allocations.
pub(crate) const MAX_FRAME_PAYLOAD: usize = 16_384;

/// Bound bytes retained while a caller is waiting for another stream. DATA
/// frames are consumed one at a time by `read_next_data_frame` or by the XPC
/// layer's chunked body reader; this budget only protects the demultiplexed
/// buffers used by `read_stream`.
pub(crate) const MAX_BUFFERED_BYTES_PER_STREAM: usize = 16 * 1024 * 1024;

/// Connection-wide budget for demultiplexed H2 DATA buffers. It permits a
/// control stream plus several active file/control streams without allowing a
/// peer to grow memory indefinitely by interleaving streams.
pub(crate) const MAX_TOTAL_BUFFERED_BYTES: usize = 64 * 1024 * 1024;

/// Maximum number of peer-created streams that may have buffered data. Streams
/// opened by this client are tracked separately and are not charged against
/// this unknown-stream flood limit.
pub(crate) const MAX_UNKNOWN_BUFFERED_STREAMS: usize = 32;

/// RemoteXPC uses even-numbered peer streams for file-transfer side channels.
/// Their first DATA frame is a body-less XPC FILE_TX_STREAM_REQUEST wrapper;
/// remoted intentionally leaves those streams open while the corresponding
/// control response is consumed on stream 1 or 3. Once that preamble has been
/// validated, keep the stream in a bounded protocol registry instead of
/// charging it as an unrecognised active stream forever.
const MAX_ANNOUNCED_FILE_STREAMS: usize = 4096;
const FILE_STREAM_PREAMBLE_LEN: usize = 24;

/// DATA frames observed while a writer is waiting for flow-control credit are
/// retained until the single framer owner asks for them. This count bounds
/// zero-length frames, which do not consume the byte budget.
const MAX_PENDING_DATA_FRAMES: usize = 1024;

/// Bound unknown EOF/reset tombstones. Empty END_STREAM frames otherwise
/// provide a cheap way to grow a set indefinitely without contributing to the
/// byte budget; known XPC/local streams are pinned separately.
const MAX_CLOSED_STREAMS: usize = MAX_UNKNOWN_BUFFERED_STREAMS;

// ── H2 preface ──────────────────────────────────────────────────────────────

pub const H2_PREFACE: &[u8] = b"PRI * HTTP/2.0\r\n\r\nSM\r\n\r\n";

// ── Framer ──────────────────────────────────────────────────────────────────

/// Minimal HTTP/2 framer for the iOS XPC protocol.
///
/// The framer is a single-owner reader/writer: every operation takes
/// `&mut self`, and `write_stream` may consume peer control frames while
/// waiting for outbound flow-control credit. Callers must therefore serialize
/// operations on one framer and must not run an independent reader against its
/// stream.
pub struct H2Framer<S> {
    stream: S,
    // Accumulated data from stream 1 (clientServer)
    client_server_buf: BytesMut,
    // Accumulated data from stream 3 (serverClient)
    server_client_buf: BytesMut,
    // Accumulated data from arbitrary additional streams.
    stream_bufs: HashMap<u32, BytesMut>,
    // Streams for which the client has already sent HEADERS.
    locally_open_streams: HashSet<u32>,
    // Whether HEADERS have been sent on each stream
    client_server_open: bool,
    server_client_open: bool,
    // Streams for which the peer has sent END_STREAM. This is separate from
    // buffered bytes so a reader cannot wait forever after a clean HTTP/2 EOF.
    closed_streams: HashSet<u32>,
    // Unknown-stream EOF tombstones in arrival order. Fixed XPC streams and
    // streams opened by this client are kept in `closed_streams` indefinitely;
    // only this queue is eligible for bounded eviction.
    closed_unknown_order: VecDeque<u32>,
    // Bytes consumed per window (keyed by stream id, `STREAM_INIT` = connection)
    // since the last WINDOW_UPDATE we sent for it.
    pending_window_updates: HashMap<u32, u32>,
    // Bytes retained in the three primary/auxiliary stream buffers above.
    buffered_bytes: usize,
    // Peer-created streams which have not sent END_STREAM yet. This is kept
    // independently of `stream_bufs` because `read_next_data_frame` returns
    // DATA directly and therefore does not need a per-stream buffer.
    peer_open_streams: HashSet<u32>,
    // Even peer streams whose first DATA frame was validated as the
    // RemoteXPC file-transfer preamble. These streams are deliberately not
    // required to send H2 END_STREAM; their registry is still bounded.
    announced_file_streams: HashSet<u32>,
    // HEADERS on an even peer stream arrive before its XPC preamble. Keep a
    // small bounded candidate set so idle/malformed peers cannot bypass the
    // unknown-stream limit by sending headers without data.
    file_stream_candidates: HashSet<u32>,
    // A peer is allowed to split the 24-byte side-channel preamble across
    // DATA frames. Keep only the bounded prefix needed to validate it.
    file_stream_candidate_data: HashMap<u32, BytesMut>,
    // Flow-control state for DATA frames sent by this client. The connection
    // starts at HTTP/2's default 65535-byte window; peer SETTINGS and
    // WINDOW_UPDATE frames change these values.
    outbound_connection_window: i64,
    peer_initial_window_size: i64,
    outbound_stream_windows: HashMap<u32, i64>,
    outbound_max_frame_size: usize,
    // DATA frames consumed by the writer's flow-control reader. Keeping the
    // frame boundary and flags here prevents a later read_next_data_frame from
    // losing business DATA that arrived while write_stream was blocked.
    pending_data_frames: VecDeque<DataFrame>,
    // Stream-local and connection-wide remote shutdown state.
    reset_streams: HashMap<u32, u32>,
    // Unknown reset tombstones in arrival order. As with clean EOF, known
    // streams are never displaced by an unknown-stream flood.
    reset_unknown_order: VecDeque<u32>,
    // Highest additional stream ID observed from the peer. This prevents an
    // evicted unknown tombstone from being reused as a fresh stream.
    highest_peer_stream_id: u32,
    goaway: Option<GoAwayState>,
    connection_error: Option<String>,
}

#[derive(Debug, Clone, Copy)]
struct GoAwayState {
    last_stream_id: u32,
    error_code: u32,
}

#[derive(Debug, Clone)]
pub struct DataFrame {
    pub stream_id: u32,
    #[allow(dead_code)]
    pub flags: u8,
    pub payload: Bytes,
}

impl DataFrame {
    #[allow(dead_code)]
    pub fn is_end_stream(&self) -> bool {
        self.flags & FLAG_END_STREAM != 0
    }

    #[allow(dead_code)]
    pub fn is_remote_xpc_control_stream(&self) -> bool {
        matches!(self.stream_id, STREAM_CLIENT_SERVER | STREAM_SERVER_CLIENT)
    }
}

impl<S: AsyncRead + AsyncWrite + Unpin> H2Framer<S> {
    fn new(stream: S) -> Self {
        Self {
            stream,
            client_server_buf: BytesMut::new(),
            server_client_buf: BytesMut::new(),
            stream_bufs: HashMap::new(),
            locally_open_streams: HashSet::new(),
            client_server_open: false,
            server_client_open: false,
            closed_streams: HashSet::new(),
            closed_unknown_order: VecDeque::new(),
            pending_window_updates: HashMap::new(),
            buffered_bytes: 0,
            peer_open_streams: HashSet::new(),
            announced_file_streams: HashSet::new(),
            file_stream_candidates: HashSet::new(),
            file_stream_candidate_data: HashMap::new(),
            outbound_connection_window: DEFAULT_PEER_WINDOW_SIZE,
            peer_initial_window_size: DEFAULT_PEER_WINDOW_SIZE,
            outbound_stream_windows: HashMap::new(),
            outbound_max_frame_size: MAX_FRAME_PAYLOAD,
            pending_data_frames: VecDeque::new(),
            reset_streams: HashMap::new(),
            reset_unknown_order: VecDeque::new(),
            highest_peer_stream_id: 0,
            goaway: None,
            connection_error: None,
        }
    }

    /// Perform the HTTP/2 handshake and return a framer ready for use.
    pub async fn connect(mut stream: S) -> Result<Self, H2Error> {
        // 1. Send HTTP/2 connection preface
        stream.write_all(H2_PREFACE).await?;

        // 2. Send SETTINGS
        let settings = build_settings_frame(&[
            (SETTINGS_MAX_CONCURRENT_STREAMS, 100),
            (SETTINGS_INITIAL_WINDOW_SIZE, INITIAL_WINDOW_SIZE),
        ]);
        stream.write_all(&settings).await?;

        // 3. Bring the connection-level window up to match the stream-level setting.
        let wupdate = build_window_update_frame(STREAM_INIT, INITIAL_WINDOW_INCREMENT);
        stream.write_all(&wupdate).await?;
        stream.flush().await?;

        let mut framer = Self::new(stream);

        // 4. Read server SETTINGS, send ACK
        framer.read_until_settings_ack_needed().await?;

        Ok(framer)
    }

    async fn read_until_settings_ack_needed(&mut self) -> Result<(), H2Error> {
        loop {
            let frame = self.read_raw_frame().await?;
            tracing::trace!(
                "h2: handshake frame type={} flags=0x{:02x} stream={} len={}",
                frame_type_name(frame.frame_type),
                frame.flags,
                frame.stream_id,
                frame.payload.len()
            );
            let is_settings =
                frame.frame_type == FRAME_SETTINGS && frame.flags & FLAG_SETTINGS_ACK == 0;
            self.dispatch_frame(frame).await?;
            if is_settings {
                return Ok(());
            }
        }
    }

    /// Read one raw HTTP/2 frame from the stream.
    async fn read_raw_frame(&mut self) -> Result<RawFrame, H2Error> {
        self.ensure_connection_open()?;
        let mut header = [0u8; 9];
        self.stream.read_exact(&mut header).await?;

        let length =
            ((header[0] as usize) << 16) | ((header[1] as usize) << 8) | (header[2] as usize);
        let frame_type = header[3];
        let flags = header[4];
        let raw_stream_id = u32::from_be_bytes([header[5], header[6], header[7], header[8]]);
        // The R bit is reserved and MUST be zero when sending, but RFC 9113
        // requires receivers to ignore it. Do not turn a peer's non-zero R
        // bit into a connection error while decoding the 31-bit identifier.
        let stream_id = raw_stream_id & 0x7FFF_FFFF;

        if length > MAX_FRAME_PAYLOAD {
            return Err(self.connection_protocol_error(format!(
                "frame payload {length} exceeds max frame size {MAX_FRAME_PAYLOAD}"
            )));
        }

        let mut payload = vec![0u8; length];
        if length > 0 {
            self.stream.read_exact(&mut payload).await?;
        }

        Ok(RawFrame {
            frame_type,
            flags,
            stream_id,
            payload,
        })
    }

    /// Read data from the serverClient stream (device → client).
    /// Blocks until `n` bytes are available.
    #[cfg(feature = "tunnel")]
    pub async fn read_server_client(&mut self, n: usize) -> Result<Bytes, H2Error> {
        self.read_stream(STREAM_SERVER_CLIENT, n).await
    }

    /// Read data from the clientServer stream (client ← device, used for ack).
    #[cfg(feature = "tunnel")]
    pub async fn read_client_server(&mut self, n: usize) -> Result<Bytes, H2Error> {
        self.read_stream(STREAM_CLIENT_SERVER, n).await
    }

    /// Read data from any stream, blocking until `n` bytes are available.
    pub async fn read_stream(&mut self, stream_id: u32, n: usize) -> Result<Bytes, H2Error> {
        self.ensure_stream_available(stream_id)?;
        self.ensure_peer_stream_not_reused(stream_id)?;
        self.drain_pending_data_frames();
        while self.stream_buffer_len(stream_id) < n {
            self.ensure_stream_available(stream_id)?;
            if self.closed_streams.contains(&stream_id) {
                return Err(H2Error::Protocol(format!(
                    "stream {stream_id} ended before {n} bytes were available"
                )));
            }
            self.ensure_peer_stream_not_reused(stream_id)?;
            let frame = self.read_raw_frame().await?;
            self.dispatch_frame(frame).await?;
        }
        self.take_stream_bytes(stream_id, n)
    }

    async fn dispatch_frame(&mut self, frame: RawFrame) -> Result<(), H2Error> {
        self.ensure_connection_open()?;
        tracing::trace!(
            "h2: dispatch frame type={} flags=0x{:02x} stream={} len={}",
            frame_type_name(frame.frame_type),
            frame.flags,
            frame.stream_id,
            frame.payload.len()
        );
        self.dispatch_frame_inner(frame).await
    }

    async fn dispatch_frame_inner(&mut self, frame: RawFrame) -> Result<(), H2Error> {
        match frame.frame_type {
            FRAME_DATA => {
                if frame.stream_id == STREAM_INIT {
                    return Err(self
                        .connection_protocol_error("DATA frame is not valid on stream 0".into()));
                }
                self.ensure_incoming_stream_available(frame.stream_id)?;
                self.observe_peer_data_stream(frame.stream_id, &frame.payload)?;
                self.append_stream_data(frame.stream_id, &frame.payload)?;
                self.replenish_receive_window(frame.stream_id, frame.payload.len())
                    .await?;
                self.mark_end_stream(frame.stream_id, frame.flags);
            }
            FRAME_HEADERS => {
                if frame.stream_id == STREAM_INIT {
                    return Err(self.connection_protocol_error(
                        "HEADERS frame is not valid on stream 0".into(),
                    ));
                }
                self.ensure_incoming_stream_available(frame.stream_id)?;
                self.observe_peer_headers(frame.stream_id)?;
                self.mark_end_stream(frame.stream_id, frame.flags);
            }
            FRAME_SETTINGS => {
                if frame.stream_id != STREAM_INIT {
                    return Err(
                        self.connection_protocol_error("SETTINGS frame must use stream 0".into())
                    );
                }
                self.process_settings(frame.flags, &frame.payload).await?;
            }
            FRAME_WINDOW_UPDATE => {
                self.process_window_update(frame.stream_id, &frame.payload)?;
            }
            FRAME_RST_STREAM => {
                if frame.stream_id == STREAM_INIT {
                    return Err(self.connection_protocol_error(
                        "RST_STREAM frame is not valid on stream 0".into(),
                    ));
                }
                if frame.payload.len() != 4 {
                    return Err(self.connection_protocol_error(format!(
                        "RST_STREAM payload must be 4 bytes, got {}",
                        frame.payload.len()
                    )));
                }
                if !self.is_known_stream(frame.stream_id)
                    && !self.peer_open_streams.contains(&frame.stream_id)
                    && !self.file_stream_candidates.contains(&frame.stream_id)
                    && !self.closed_streams.contains(&frame.stream_id)
                    && !self.reset_streams.contains_key(&frame.stream_id)
                {
                    return Err(self.connection_protocol_error(format!(
                        "RST_STREAM received for idle stream {}",
                        frame.stream_id
                    )));
                }
                let error_code = u32::from_be_bytes(frame.payload[..4].try_into().unwrap());
                self.clear_stream_buffer(frame.stream_id);
                self.outbound_stream_windows.remove(&frame.stream_id);
                self.record_reset_stream(frame.stream_id, error_code);
            }
            FRAME_GOAWAY => {
                self.process_goaway(frame.stream_id, &frame.payload)?;
            }
            _ => {}
        }
        Ok(())
    }

    fn ensure_connection_open(&self) -> Result<(), H2Error> {
        self.connection_error
            .as_ref()
            .map_or(Ok(()), |message| Err(H2Error::Protocol(message.clone())))
    }

    fn connection_protocol_error(&mut self, message: String) -> H2Error {
        if self.connection_error.is_none() {
            self.connection_error = Some(message.clone());
        }
        H2Error::Protocol(message)
    }

    fn ensure_stream_available(&self, stream_id: u32) -> Result<(), H2Error> {
        self.ensure_connection_open()?;
        if let Some(error_code) = self.reset_streams.get(&stream_id) {
            return Err(H2Error::Protocol(format!(
                "stream {stream_id} was reset by peer (error code {error_code})"
            )));
        }
        if let Some(goaway) = self.goaway {
            if stream_id != STREAM_INIT && stream_id > goaway.last_stream_id {
                return Err(H2Error::Protocol(format!(
                    "stream {stream_id} was refused by peer GOAWAY (last stream {}, error code {})",
                    goaway.last_stream_id, goaway.error_code
                )));
            }
        }
        Ok(())
    }

    fn ensure_incoming_stream_available(&self, stream_id: u32) -> Result<(), H2Error> {
        self.ensure_stream_available(stream_id)?;
        if self.closed_streams.contains(&stream_id) {
            return Err(H2Error::Protocol(format!(
                "frame received after stream {stream_id} ended"
            )));
        }
        self.ensure_peer_stream_not_reused(stream_id)?;
        Ok(())
    }

    fn ensure_peer_stream_not_reused(&self, stream_id: u32) -> Result<(), H2Error> {
        if !self.is_known_stream(stream_id)
            && !self.peer_open_streams.contains(&stream_id)
            && !self.announced_file_streams.contains(&stream_id)
            && !self.file_stream_candidates.contains(&stream_id)
            && !self.closed_streams.contains(&stream_id)
            && stream_id <= self.highest_peer_stream_id
        {
            return Err(H2Error::Protocol(format!(
                "peer stream {stream_id} was already opened and cannot be reused"
            )));
        }
        Ok(())
    }

    fn record_reset_stream(&mut self, stream_id: u32, error_code: u32) {
        // A reset stream cannot be reused, so retain enough state for a later
        // caller to receive the reset instead of reopening it. Keep the
        // tombstones bounded: a peer can send validly-shaped RST_STREAM frames
        // for arbitrary stream IDs without allocating DATA buffers.
        if !self.reset_streams.contains_key(&stream_id) && !self.is_known_stream(stream_id) {
            if self.reset_unknown_order.len() >= MAX_CLOSED_STREAMS {
                if let Some(oldest) = self.reset_unknown_order.pop_front() {
                    self.reset_streams.remove(&oldest);
                }
            }
            self.reset_unknown_order.push_back(stream_id);
        }
        self.reset_streams.insert(stream_id, error_code);
    }

    async fn process_settings(&mut self, flags: u8, payload: &[u8]) -> Result<(), H2Error> {
        if flags & FLAG_SETTINGS_ACK != 0 {
            if !payload.is_empty() {
                return Err(self.connection_protocol_error(
                    "SETTINGS ACK frame must have an empty payload".into(),
                ));
            }
            return Ok(());
        }
        if payload.len() % 6 != 0 {
            return Err(self.connection_protocol_error(format!(
                "SETTINGS payload length must be a multiple of 6, got {}",
                payload.len()
            )));
        }

        let mut initial_window = None;
        let mut max_frame_size = None;
        for setting in payload.chunks_exact(6) {
            let id = u16::from_be_bytes([setting[0], setting[1]]);
            let value = u32::from_be_bytes([setting[2], setting[3], setting[4], setting[5]]);
            match id {
                SETTINGS_INITIAL_WINDOW_SIZE => {
                    if i64::from(value) > MAX_FLOW_CONTROL_WINDOW {
                        return Err(self.connection_protocol_error(format!(
                            "SETTINGS_INITIAL_WINDOW_SIZE {value} exceeds {MAX_FLOW_CONTROL_WINDOW}"
                        )));
                    }
                    initial_window = Some(i64::from(value));
                }
                SETTINGS_MAX_FRAME_SIZE => {
                    let value = value as usize;
                    if !(MAX_FRAME_PAYLOAD..=MAX_NEGOTIATED_FRAME_SIZE).contains(&value) {
                        return Err(self.connection_protocol_error(format!(
                            "SETTINGS_MAX_FRAME_SIZE {value} is outside {}..={MAX_NEGOTIATED_FRAME_SIZE}",
                            MAX_FRAME_PAYLOAD
                        )));
                    }
                    max_frame_size = Some(value);
                }
                // MAX_CONCURRENT_STREAMS and unknown settings do not affect the
                // framing state maintained here.
                _ => {}
            }
        }

        if let Some(new_initial_window) = initial_window {
            let delta = new_initial_window - self.peer_initial_window_size;
            let mut updated = Vec::with_capacity(self.outbound_stream_windows.len());
            for (&stream_id, &window) in &self.outbound_stream_windows {
                let Some(value) = window.checked_add(delta) else {
                    return Err(self.connection_protocol_error(format!(
                        "stream {stream_id} outbound window overflow applying SETTINGS_INITIAL_WINDOW_SIZE delta {delta}"
                    )));
                };
                if !(-MAX_FLOW_CONTROL_WINDOW..=MAX_FLOW_CONTROL_WINDOW).contains(&value) {
                    return Err(self.connection_protocol_error(format!(
                        "stream {stream_id} outbound window {value} is outside flow-control range"
                    )));
                }
                updated.push((stream_id, value));
            }
            for (stream_id, value) in updated {
                self.outbound_stream_windows.insert(stream_id, value);
            }
            self.peer_initial_window_size = new_initial_window;
        }
        if let Some(max_frame_size) = max_frame_size {
            self.outbound_max_frame_size = max_frame_size;
        }

        // A SETTINGS frame is acknowledged only after all values have been
        // validated and applied. This also makes a waiting writer observe the
        // new window/frame limits before it resumes.
        let ack = build_settings_ack();
        self.stream.write_all(&ack).await?;
        self.stream.flush().await?;
        Ok(())
    }

    fn process_window_update(&mut self, stream_id: u32, payload: &[u8]) -> Result<(), H2Error> {
        if payload.len() != 4 {
            return Err(self.connection_protocol_error(format!(
                "WINDOW_UPDATE payload must be 4 bytes, got {}",
                payload.len()
            )));
        }
        let increment =
            u32::from_be_bytes(payload[..4].try_into().unwrap()) & MAX_FLOW_CONTROL_WINDOW as u32;
        if increment == 0 {
            let message = format!("WINDOW_UPDATE for stream {stream_id} has a zero increment");
            let stream_is_known = stream_id != STREAM_INIT
                && (self.is_known_stream(stream_id)
                    || self.peer_open_streams.contains(&stream_id)
                    || self.closed_streams.contains(&stream_id)
                    || self.reset_streams.contains_key(&stream_id)
                    || self.outbound_stream_windows.contains_key(&stream_id));
            return if stream_id == STREAM_INIT || !stream_is_known {
                Err(self.connection_protocol_error(message))
            } else {
                // RFC 9113 §6.9: a zero increment is a connection error on
                // stream 0, but only a stream error for a known non-zero
                // stream. An idle stream is a connection error regardless of
                // the increment value.
                Err(H2Error::Protocol(message))
            };
        }

        if stream_id == STREAM_INIT {
            let value = self
                .outbound_connection_window
                .checked_add(i64::from(increment))
                .ok_or_else(|| {
                    self.connection_protocol_error(format!(
                        "connection outbound window overflow adding {increment}"
                    ))
                })?;
            if value > MAX_FLOW_CONTROL_WINDOW {
                return Err(self.connection_protocol_error(format!(
                    "connection outbound window {value} exceeds {MAX_FLOW_CONTROL_WINDOW}"
                )));
            }
            self.outbound_connection_window = value;
            return Ok(());
        }

        let Some(window) = self.outbound_stream_windows.get(&stream_id).copied() else {
            if self.closed_streams.contains(&stream_id)
                || self.reset_streams.contains_key(&stream_id)
            {
                return Err(H2Error::Protocol(format!(
                    "WINDOW_UPDATE received for closed stream {stream_id}"
                )));
            }
            if self.peer_open_streams.contains(&stream_id)
                || self.announced_file_streams.contains(&stream_id)
                || self.file_stream_candidates.contains(&stream_id)
            {
                // This framer never sends DATA on a peer-created stream, so
                // there is no outbound counter to update. It is nevertheless
                // an open stream, and RFC 9113 permits WINDOW_UPDATE in this
                // state; do not misclassify it as an idle-stream error.
                return Ok(());
            }
            return Err(self.connection_protocol_error(format!(
                "WINDOW_UPDATE received for unopened stream {stream_id}"
            )));
        };
        let Some(value) = window.checked_add(i64::from(increment)) else {
            return Err(self.connection_protocol_error(format!(
                "stream {stream_id} outbound window overflow adding {increment}"
            )));
        };
        if value > MAX_FLOW_CONTROL_WINDOW {
            return Err(self.connection_protocol_error(format!(
                "stream {stream_id} outbound window {value} exceeds {MAX_FLOW_CONTROL_WINDOW}"
            )));
        }
        self.outbound_stream_windows.insert(stream_id, value);
        Ok(())
    }

    fn process_goaway(&mut self, stream_id: u32, payload: &[u8]) -> Result<(), H2Error> {
        if stream_id != STREAM_INIT {
            return Err(self.connection_protocol_error("GOAWAY frame must use stream 0".into()));
        }
        if payload.len() < 8 {
            return Err(self.connection_protocol_error(format!(
                "GOAWAY payload must be at least 8 bytes, got {}",
                payload.len()
            )));
        }
        let last_stream_id =
            u32::from_be_bytes(payload[..4].try_into().unwrap()) & MAX_FLOW_CONTROL_WINDOW as u32;
        let error_code = u32::from_be_bytes(payload[4..8].try_into().unwrap());
        if let Some(previous) = self.goaway {
            if last_stream_id > previous.last_stream_id {
                return Err(self.connection_protocol_error(format!(
                    "GOAWAY last stream {last_stream_id} exceeds previous {}",
                    previous.last_stream_id
                )));
            }
        }
        match self.goaway {
            Some(previous) if previous.last_stream_id <= last_stream_id => {}
            _ => {
                self.goaway = Some(GoAwayState {
                    last_stream_id,
                    error_code,
                });
            }
        }
        if error_code != 0 {
            return Err(self.connection_protocol_error(format!(
                "peer sent GOAWAY with error code {error_code}"
            )));
        }
        Ok(())
    }

    /// Give the peer back the receive window that `consumed` bytes of DATA used.
    ///
    /// Applies to every stream, not just even-numbered ones. The two primary XPC
    /// data streams (clientServer = 1, serverClient = 3) are odd, and HTTP/2
    /// windows are cumulative for the life of the connection, so skipping them
    /// meant any service streaming more than [`INITIAL_WINDOW_SIZE`] in total
    /// stalled forever with no timeout at this layer.
    ///
    /// Updates are batched at [`WINDOW_UPDATE_THRESHOLD`], as the reference does,
    /// so a busy stream does not pay for a frame pair per DATA frame.
    async fn replenish_receive_window(
        &mut self,
        stream_id: u32,
        consumed: usize,
    ) -> Result<(), H2Error> {
        if consumed == 0 {
            return Ok(());
        }
        let consumed = u32::try_from(consumed).map_err(|_| {
            self.connection_protocol_error(format!(
                "received DATA length {consumed} does not fit in a WINDOW_UPDATE"
            ))
        })?;

        let mut frames = Vec::new();
        // The connection window is shared by every stream, so it always accrues.
        let connection_pending = self
            .pending_window_updates
            .get(&STREAM_INIT)
            .copied()
            .unwrap_or_default();
        let connection_total = connection_pending.checked_add(consumed).ok_or_else(|| {
            self.connection_protocol_error(format!(
                "connection WINDOW_UPDATE pending count overflow: {connection_pending} + {consumed}"
            ))
        })?;

        let stream_pending = if stream_id == STREAM_INIT {
            0
        } else {
            self.pending_window_updates
                .get(&stream_id)
                .copied()
                .unwrap_or_default()
        };
        let stream_total = if stream_id == STREAM_INIT {
            0
        } else {
            stream_pending.checked_add(consumed).ok_or_else(|| {
                self.connection_protocol_error(format!(
                    "stream {stream_id} WINDOW_UPDATE pending count overflow: \
                     {stream_pending} + {consumed}"
                ))
            })?
        };

        if connection_total >= WINDOW_UPDATE_THRESHOLD {
            frames.push(build_window_update_frame(STREAM_INIT, connection_total));
            self.pending_window_updates.insert(STREAM_INIT, 0);
        } else {
            self.pending_window_updates
                .insert(STREAM_INIT, connection_total);
        }

        if stream_id != STREAM_INIT {
            if stream_total >= WINDOW_UPDATE_THRESHOLD {
                frames.push(build_window_update_frame(stream_id, stream_total));
                self.pending_window_updates.insert(stream_id, 0);
            } else {
                self.pending_window_updates.insert(stream_id, stream_total);
            }
        }

        if frames.is_empty() {
            return Ok(());
        }
        for frame in &frames {
            self.stream.write_all(frame).await?;
        }
        self.stream.flush().await?;
        Ok(())
    }

    /// Read the next DATA frame from any stream, skipping non-DATA frames.
    #[allow(dead_code)]
    pub async fn read_next_data_frame(&mut self) -> Result<DataFrame, H2Error> {
        self.ensure_connection_open()?;
        if let Some(frame) = self.pending_data_frames.pop_front() {
            self.buffered_bytes = self.buffered_bytes.saturating_sub(frame.payload.len());
            return Ok(frame);
        }

        loop {
            let frame = self.read_raw_frame().await?;
            tracing::trace!(
                "h2: next data frame type={} flags=0x{:02x} stream={} len={}",
                frame_type_name(frame.frame_type),
                frame.flags,
                frame.stream_id,
                frame.payload.len()
            );
            match frame.frame_type {
                FRAME_DATA => {
                    if frame.stream_id == STREAM_INIT {
                        return Err(self.connection_protocol_error(
                            "DATA frame is not valid on stream 0".into(),
                        ));
                    }
                    self.ensure_incoming_stream_available(frame.stream_id)?;
                    self.observe_peer_data_stream(frame.stream_id, &frame.payload)?;
                    self.replenish_receive_window(frame.stream_id, frame.payload.len())
                        .await?;
                    self.mark_end_stream(frame.stream_id, frame.flags);
                    return Ok(DataFrame {
                        stream_id: frame.stream_id,
                        flags: frame.flags,
                        payload: Bytes::from(frame.payload),
                    });
                }
                FRAME_GOAWAY => {
                    self.dispatch_frame(frame).await?;
                    if let Some(goaway) = self.goaway {
                        if goaway.error_code != 0 {
                            return Err(H2Error::Protocol(format!(
                                "peer sent GOAWAY with error code {}",
                                goaway.error_code
                            )));
                        }
                    }
                }
                FRAME_RST_STREAM => {
                    let stream_id = frame.stream_id;
                    self.dispatch_frame(frame).await?;
                    let error_code = self
                        .reset_streams
                        .get(&stream_id)
                        .copied()
                        .unwrap_or_default();
                    return Err(H2Error::Protocol(format!(
                        "stream {stream_id} was reset by peer (error code {error_code})"
                    )));
                }
                _ => self.dispatch_frame(frame).await?,
            }
        }
    }

    /// Dispatch raw frames until one stream has a complete XPC message in its
    /// per-stream buffer, then return that stream's id.
    ///
    /// Unlike [`Self::read_next_data_frame`], the returned frame's payload
    /// stays in the per-stream buffer, so a caller can reassemble an XPC
    /// message from that stream with [`Self::read_stream`] while data on
    /// other streams remains buffered in place.  Non-DATA frames (settings,
    /// headers, acks, window updates) are dispatched normally.  DATA frames
    /// queued while waiting for outbound capacity are flushed into their
    /// stream buffers first.
    ///
    /// Bytes already buffered for a stream are inspected before new wire
    /// frames. A partial message on one stream does not block a complete
    /// message on another stream; each stream is accumulated independently.
    /// A FILE_TX message larger than the H2 per-stream retention limit is
    /// returned once its 24-byte wrapper is buffered so the XPC layer can
    /// consume the body in bounded chunks.
    pub async fn buffer_next_data_stream(&mut self) -> Result<u32, H2Error> {
        self.ensure_connection_open()?;
        if !self.pending_data_frames.is_empty() {
            self.drain_pending_data_frames();
        }
        if let Some(stream_id) = self.first_ready_xpc_stream()? {
            return Ok(stream_id);
        }
        if let Some(stream_id) = self.first_unfinished_closed_stream()? {
            return Ok(stream_id);
        }
        loop {
            let frame = self.read_raw_frame().await?;
            self.dispatch_frame(frame).await?;
            if let Some(stream_id) = self.first_ready_xpc_stream()? {
                return Ok(stream_id);
            }
            if let Some(stream_id) = self.first_unfinished_closed_stream()? {
                return Ok(stream_id);
            }
        }
    }

    fn buffered_stream_ids(&self) -> Vec<u32> {
        let mut stream_ids = Vec::new();
        if !self.client_server_buf.is_empty() {
            stream_ids.push(STREAM_CLIENT_SERVER);
        }
        if !self.server_client_buf.is_empty() {
            stream_ids.push(STREAM_SERVER_CLIENT);
        }
        stream_ids.extend(
            self.stream_bufs
                .iter()
                .filter(|(_, buffer)| !buffer.is_empty())
                .map(|(stream_id, _)| *stream_id),
        );
        stream_ids.sort_unstable();
        stream_ids
    }

    /// Return whether the stream has enough buffered bytes for one XPC
    /// message. Large FILE_TX bodies are intentionally considered ready once
    /// their wrapper is buffered; the XPC reader then consumes their body in
    /// bounded H2-sized chunks instead of retaining it all here.
    fn stream_has_ready_xpc_message(&self, stream_id: u32) -> Result<bool, H2Error> {
        let Some(buffer) = self.stream_buffer(stream_id) else {
            return Ok(false);
        };
        if buffer.len() < 24 {
            return Ok(false);
        }

        let magic = u32::from_le_bytes(buffer[..4].try_into().unwrap());
        // Let the XPC decoder report malformed magic as soon as a complete
        // wrapper header is available instead of waiting for an arbitrary
        // body length from untrusted bytes.
        if magic != WRAPPER_MAGIC {
            return Ok(true);
        }
        let flags = u32::from_le_bytes(buffer[4..8].try_into().unwrap());
        let declared_body_len = u64::from_le_bytes(buffer[8..16].try_into().unwrap());
        let body_len = checked_xpc_body_len(declared_body_len, xpc_body_limit_for_flags(flags))
            .map_err(H2Error::Protocol)?;
        let total_len = 24usize.checked_add(body_len).ok_or_else(|| {
            H2Error::Protocol("XPC message length overflow while selecting a stream".into())
        })?;
        // Leave one maximum DATA frame of headroom for bytes belonging to the
        // next message. Without this margin a message exactly at the H2
        // retention limit can arrive in the same final frame as a following
        // wrapper and trip reserve_stream_buffer before the message is read.
        let buffered_message_limit = MAX_BUFFERED_BYTES_PER_STREAM - MAX_FRAME_PAYLOAD;
        Ok(buffer.len() >= total_len || total_len > buffered_message_limit)
    }

    fn stream_buffer(&self, stream_id: u32) -> Option<&BytesMut> {
        match stream_id {
            STREAM_CLIENT_SERVER => Some(&self.client_server_buf),
            STREAM_SERVER_CLIENT => Some(&self.server_client_buf),
            other => self.stream_bufs.get(&other),
        }
    }

    fn first_ready_xpc_stream(&self) -> Result<Option<u32>, H2Error> {
        for stream_id in self.buffered_stream_ids() {
            if self.stream_has_ready_xpc_message(stream_id)? {
                return Ok(Some(stream_id));
            }
        }
        Ok(None)
    }

    /// Return a stream that ended before a complete XPC message arrived. A
    /// complete message always wins, so an empty END_STREAM marker on one
    /// stream cannot hide a response already buffered on another stream.
    fn first_unfinished_closed_stream(&self) -> Result<Option<u32>, H2Error> {
        let mut stream_ids: Vec<u32> = self.closed_streams.iter().copied().collect();
        stream_ids.sort_unstable();
        for stream_id in stream_ids {
            // An empty END_STREAM marker is a harmless closed-stream
            // tombstone. It carries no partial XPC message and must not
            // prevent a later response on another stream from being chosen.
            // Only retained bytes prove that a message was started and then
            // cut short.
            if self
                .stream_buffer(stream_id)
                .map_or(true, BytesMut::is_empty)
            {
                continue;
            }
            if !self.stream_has_ready_xpc_message(stream_id)? {
                return Ok(Some(stream_id));
            }
        }
        Ok(None)
    }

    /// Write data to the clientServer stream (client → device).
    pub async fn write_client_server(&mut self, data: &[u8]) -> Result<(), H2Error> {
        self.write_stream(STREAM_CLIENT_SERVER, data).await
    }

    /// Write data to the serverClient stream (client → device, for acks/replies).
    pub async fn write_server_client(&mut self, data: &[u8]) -> Result<(), H2Error> {
        self.write_stream(STREAM_SERVER_CLIENT, data).await
    }

    /// Write data to any stream, opening it with an empty HEADERS frame first.
    pub async fn write_stream(&mut self, stream_id: u32, data: &[u8]) -> Result<(), H2Error> {
        self.open_stream(stream_id).await?;
        self.ensure_stream_available(stream_id)?;
        if data.is_empty() {
            self.stream
                .write_all(&build_data_frame(stream_id, data))
                .await?;
        } else {
            let mut offset = 0;
            while offset < data.len() {
                self.ensure_stream_available(stream_id)?;
                let budget = self.outbound_budget(stream_id)?;
                if budget == 0 {
                    self.wait_for_outbound_capacity(stream_id).await?;
                    continue;
                }
                let end = offset
                    .checked_add(budget.min(data.len() - offset))
                    .ok_or_else(|| {
                        self.connection_protocol_error("outbound DATA offset overflow".to_string())
                    })?;
                let chunk = &data[offset..end];
                self.stream
                    .write_all(&build_data_frame(stream_id, chunk))
                    .await?;
                self.consume_outbound_window(stream_id, chunk.len())?;
                offset = end;
            }
        }
        self.stream.flush().await?;
        Ok(())
    }

    /// Open an arbitrary stream with an empty HEADERS frame if it is not open yet.
    pub async fn open_stream(&mut self, stream_id: u32) -> Result<(), H2Error> {
        self.ensure_connection_open()?;
        if stream_id == STREAM_INIT || stream_id > MAX_FLOW_CONTROL_WINDOW as u32 {
            return Err(
                self.connection_protocol_error(format!("invalid local stream ID {stream_id}"))
            );
        }
        self.ensure_stream_available(stream_id)?;
        if self.closed_streams.contains(&stream_id) {
            return Err(H2Error::Protocol(format!(
                "cannot open stream {stream_id} after peer END_STREAM"
            )));
        }
        let already_open = match stream_id {
            STREAM_CLIENT_SERVER => self.client_server_open,
            STREAM_SERVER_CLIENT => self.server_client_open,
            _ => self.locally_open_streams.contains(&stream_id),
        };
        if !already_open {
            if let Some(goaway) = self.goaway {
                return Err(H2Error::Protocol(format!(
                    "cannot open stream {stream_id} after peer GOAWAY (last stream {}, error code {})",
                    goaway.last_stream_id, goaway.error_code
                )));
            }
            let headers = build_headers_frame(stream_id);
            self.stream.write_all(&headers).await?;
            self.stream.flush().await?;
            self.outbound_stream_windows
                .insert(stream_id, self.peer_initial_window_size);
            match stream_id {
                STREAM_CLIENT_SERVER => self.client_server_open = true,
                STREAM_SERVER_CLIENT => self.server_client_open = true,
                _ => {
                    self.locally_open_streams.insert(stream_id);
                    self.peer_open_streams.remove(&stream_id);
                    self.announced_file_streams.remove(&stream_id);
                    self.file_stream_candidates.remove(&stream_id);
                    self.file_stream_candidate_data.remove(&stream_id);
                    self.stream_bufs.entry(stream_id).or_default();
                }
            }
        }
        Ok(())
    }

    fn outbound_budget(&self, stream_id: u32) -> Result<usize, H2Error> {
        self.ensure_stream_available(stream_id)?;
        let stream_window = self
            .outbound_stream_windows
            .get(&stream_id)
            .copied()
            .ok_or_else(|| {
                H2Error::Protocol(format!("stream {stream_id} is not open for sending"))
            })?;
        if self.outbound_connection_window <= 0 || stream_window <= 0 {
            return Ok(0);
        }
        let connection = usize::try_from(self.outbound_connection_window).map_err(|_| {
            H2Error::Protocol(format!(
                "invalid outbound connection window {}",
                self.outbound_connection_window
            ))
        })?;
        let stream = usize::try_from(stream_window).map_err(|_| {
            H2Error::Protocol(format!(
                "invalid outbound stream {stream_id} window {stream_window}"
            ))
        })?;
        Ok(self.outbound_max_frame_size.min(connection).min(stream))
    }

    fn consume_outbound_window(&mut self, stream_id: u32, amount: usize) -> Result<(), H2Error> {
        let amount = i64::try_from(amount).map_err(|_| {
            self.connection_protocol_error(format!("outbound DATA length {amount} exceeds i64"))
        })?;
        let stream_window = self
            .outbound_stream_windows
            .get_mut(&stream_id)
            .ok_or_else(|| {
                H2Error::Protocol(format!("stream {stream_id} is not open for sending"))
            })?;
        if amount > self.outbound_connection_window || amount > *stream_window {
            return Err(self.connection_protocol_error(format!(
                "outbound DATA length {amount} exceeds stream {stream_id} or connection window"
            )));
        }
        self.outbound_connection_window -= amount;
        *stream_window -= amount;
        Ok(())
    }

    async fn wait_for_outbound_capacity(&mut self, stream_id: u32) -> Result<(), H2Error> {
        loop {
            self.ensure_stream_available(stream_id)?;
            if self.outbound_budget(stream_id)? > 0 {
                return Ok(());
            }

            // Flush DATA already written before waiting for the peer. This
            // keeps a full socket from hiding the frame that caused the peer
            // to send its next WINDOW_UPDATE.
            self.stream.flush().await?;
            let frame = self.read_raw_frame().await?;
            if frame.frame_type == FRAME_DATA {
                self.queue_incoming_data_frame(frame).await?;
            } else {
                self.dispatch_frame(frame).await?;
            }
        }
    }

    fn is_known_stream(&self, stream_id: u32) -> bool {
        matches!(
            stream_id,
            STREAM_INIT | STREAM_CLIENT_SERVER | STREAM_SERVER_CLIENT
        ) || self.locally_open_streams.contains(&stream_id)
            || self.announced_file_streams.contains(&stream_id)
    }

    /// Observe a peer HEADERS frame. The only peer-created streams used by
    /// RemoteXPC are even file-transfer side channels; defer admitting those
    /// streams to the protocol registry until their first DATA preamble is
    /// validated. Other streams remain subject to the ordinary unknown-stream
    /// flood limit immediately.
    fn observe_peer_headers(&mut self, stream_id: u32) -> Result<(), H2Error> {
        if self.is_known_stream(stream_id)
            || self.peer_open_streams.contains(&stream_id)
            || self.file_stream_candidates.contains(&stream_id)
        {
            return Ok(());
        }
        if stream_id.is_multiple_of(2) {
            self.register_file_stream_candidate(stream_id)
        } else {
            self.track_peer_stream(stream_id)
        }
    }

    /// Observe the first DATA frame for a peer-created stream. A valid
    /// body-less FILE_TX_STREAM_REQUEST preamble identifies a RemoteXPC
    /// side-channel; it is moved out of the unknown active set and retained in
    /// a bounded registry because iOS leaves it open without END_STREAM.
    fn observe_peer_data_stream(&mut self, stream_id: u32, payload: &[u8]) -> Result<(), H2Error> {
        if self.is_known_stream(stream_id) || self.peer_open_streams.contains(&stream_id) {
            return Ok(());
        }

        if self.file_stream_candidates.contains(&stream_id) {
            let valid = {
                let candidate = self
                    .file_stream_candidate_data
                    .entry(stream_id)
                    .or_default();
                let prefix_len = FILE_STREAM_PREAMBLE_LEN.saturating_sub(candidate.len());
                if prefix_len != 0 {
                    candidate.extend_from_slice(&payload[..payload.len().min(prefix_len)]);
                }
                if candidate.len() < FILE_STREAM_PREAMBLE_LEN {
                    return Ok(());
                }
                is_file_stream_preamble_prefix(stream_id, candidate)
            };
            if !valid {
                // The first fragment may already be retained in the generic
                // stream buffer. Clear it before returning the protocol error
                // so a malformed candidate cannot retain byte-budget state.
                self.clear_stream_buffer(stream_id);
                return Err(H2Error::Protocol(format!(
                    "invalid RemoteXPC file-stream preamble on stream {stream_id}"
                )));
            }
            self.file_stream_candidates.remove(&stream_id);
            self.file_stream_candidate_data.remove(&stream_id);
            return self.adopt_file_stream(stream_id);
        }

        if is_file_stream_preamble(stream_id, payload) {
            return self.adopt_file_stream(stream_id);
        }

        self.track_peer_stream(stream_id)
    }

    fn register_file_stream_candidate(&mut self, stream_id: u32) -> Result<(), H2Error> {
        if stream_id <= self.highest_peer_stream_id {
            return Err(H2Error::Protocol(format!(
                "peer stream {stream_id} was already opened and cannot be reused"
            )));
        }
        if self.unknown_active_stream_count() >= MAX_UNKNOWN_BUFFERED_STREAMS {
            return Err(H2Error::Protocol(format!(
                "too many pending RemoteXPC file streams: limit {MAX_UNKNOWN_BUFFERED_STREAMS}"
            )));
        }
        self.highest_peer_stream_id = stream_id;
        self.file_stream_candidates.insert(stream_id);
        self.file_stream_candidate_data
            .insert(stream_id, BytesMut::new());
        Ok(())
    }

    fn adopt_file_stream(&mut self, stream_id: u32) -> Result<(), H2Error> {
        if self.announced_file_streams.contains(&stream_id) {
            return Ok(());
        }
        if self.announced_file_streams.len() >= MAX_ANNOUNCED_FILE_STREAMS {
            return Err(H2Error::Protocol(format!(
                "too many announced RemoteXPC file streams: limit {MAX_ANNOUNCED_FILE_STREAMS}"
            )));
        }
        self.highest_peer_stream_id = self.highest_peer_stream_id.max(stream_id);
        self.announced_file_streams.insert(stream_id);
        Ok(())
    }

    fn track_peer_stream(&mut self, stream_id: u32) -> Result<(), H2Error> {
        if self.is_known_stream(stream_id)
            || self.peer_open_streams.contains(&stream_id)
            || self.file_stream_candidates.contains(&stream_id)
        {
            return Ok(());
        }
        if stream_id <= self.highest_peer_stream_id {
            return Err(H2Error::Protocol(format!(
                "peer stream {stream_id} was already opened and cannot be reused"
            )));
        }
        if self.unknown_active_stream_count() >= MAX_UNKNOWN_BUFFERED_STREAMS {
            return Err(H2Error::Protocol(format!(
                "too many unknown H2 streams: limit {MAX_UNKNOWN_BUFFERED_STREAMS}"
            )));
        }
        self.highest_peer_stream_id = stream_id;
        self.peer_open_streams.insert(stream_id);
        Ok(())
    }

    fn unknown_active_stream_count(&self) -> usize {
        self.peer_open_streams.len() + self.file_stream_candidates.len()
    }

    fn clear_stream_buffer(&mut self, stream_id: u32) {
        let mut removed = match stream_id {
            STREAM_CLIENT_SERVER => self.client_server_buf.split().len(),
            STREAM_SERVER_CLIENT => self.server_client_buf.split().len(),
            _ => self
                .stream_bufs
                .remove(&stream_id)
                .map_or(0, |buffer| buffer.len()),
        };
        self.pending_data_frames.retain(|frame| {
            if frame.stream_id == stream_id {
                removed = removed.saturating_add(frame.payload.len());
                false
            } else {
                true
            }
        });
        self.buffered_bytes = self.buffered_bytes.saturating_sub(removed);
        self.remove_closed_stream_tombstone(stream_id);
        self.peer_open_streams.remove(&stream_id);
        self.announced_file_streams.remove(&stream_id);
        self.file_stream_candidates.remove(&stream_id);
        self.file_stream_candidate_data.remove(&stream_id);
        self.pending_window_updates.remove(&stream_id);
    }

    fn remove_closed_stream_tombstone(&mut self, stream_id: u32) {
        if self.closed_streams.remove(&stream_id) && !self.is_known_stream(stream_id) {
            self.closed_unknown_order.retain(|&id| id != stream_id);
        }
    }

    fn append_stream_data(&mut self, stream_id: u32, payload: &[u8]) -> Result<(), H2Error> {
        if payload.is_empty() {
            return Ok(());
        }

        self.reserve_stream_buffer(stream_id, payload.len())?;
        match stream_id {
            STREAM_CLIENT_SERVER => self.client_server_buf.extend_from_slice(payload),
            STREAM_SERVER_CLIENT => self.server_client_buf.extend_from_slice(payload),
            other => self
                .stream_bufs
                .entry(other)
                .or_default()
                .extend_from_slice(payload),
        }
        Ok(())
    }

    fn reserve_stream_buffer(&mut self, stream_id: u32, incoming: usize) -> Result<(), H2Error> {
        if incoming == 0 {
            return Ok(());
        }

        let known = self.is_known_stream(stream_id);
        if !known
            && !self.stream_bufs.contains_key(&stream_id)
            && !self.peer_open_streams.contains(&stream_id)
            && self.peer_open_streams.len() >= MAX_UNKNOWN_BUFFERED_STREAMS
        {
            self.remove_closed_stream_tombstone(stream_id);
            return Err(H2Error::Protocol(format!(
                "too many unknown H2 streams with buffered data: limit {MAX_UNKNOWN_BUFFERED_STREAMS}"
            )));
        }

        let current = self
            .stream_buffer_len(stream_id)
            .checked_add(self.pending_stream_buffer_len(stream_id))
            .ok_or_else(|| {
                self.clear_stream_buffer(stream_id);
                H2Error::Protocol(format!(
                    "H2 stream {stream_id} buffered length overflow while adding {} pending bytes",
                    incoming
                ))
            })?;
        let requested = current.checked_add(incoming).ok_or_else(|| {
            self.clear_stream_buffer(stream_id);
            H2Error::Protocol(format!(
                "H2 stream {stream_id} buffer length overflow: current {current}, incoming {}",
                incoming
            ))
        })?;
        if requested > MAX_BUFFERED_BYTES_PER_STREAM {
            self.clear_stream_buffer(stream_id);
            return Err(H2Error::Protocol(format!(
                "H2 stream {stream_id} buffer {requested} exceeds per-stream limit {MAX_BUFFERED_BYTES_PER_STREAM}"
            )));
        }

        let total = self.buffered_bytes.checked_add(incoming).ok_or_else(|| {
            self.clear_stream_buffer(stream_id);
            H2Error::Protocol(format!(
                "H2 buffered byte count overflow: current {}, incoming {}",
                self.buffered_bytes, incoming
            ))
        })?;
        if total > MAX_TOTAL_BUFFERED_BYTES {
            self.clear_stream_buffer(stream_id);
            return Err(H2Error::Protocol(format!(
                "H2 buffered bytes {total} exceed connection limit {MAX_TOTAL_BUFFERED_BYTES}"
            )));
        }

        self.buffered_bytes = total;
        Ok(())
    }

    fn pending_stream_buffer_len(&self, stream_id: u32) -> usize {
        self.pending_data_frames
            .iter()
            .filter(|frame| frame.stream_id == stream_id)
            .map(|frame| frame.payload.len())
            .fold(0, usize::saturating_add)
    }

    async fn queue_incoming_data_frame(&mut self, frame: RawFrame) -> Result<(), H2Error> {
        if frame.stream_id == STREAM_INIT {
            return Err(
                self.connection_protocol_error("DATA frame is not valid on stream 0".into())
            );
        }
        self.ensure_incoming_stream_available(frame.stream_id)?;
        self.observe_peer_data_stream(frame.stream_id, &frame.payload)?;
        if self.pending_data_frames.len() >= MAX_PENDING_DATA_FRAMES {
            return Err(self.connection_protocol_error(format!(
                "too many pending DATA frames while waiting for outbound capacity: limit {MAX_PENDING_DATA_FRAMES}"
            )));
        }
        let stream_id = frame.stream_id;
        let flags = frame.flags;
        let payload_len = frame.payload.len();
        self.reserve_stream_buffer(stream_id, payload_len)?;
        self.pending_data_frames.push_back(DataFrame {
            stream_id,
            flags,
            payload: Bytes::from(frame.payload),
        });
        self.replenish_receive_window(stream_id, payload_len)
            .await?;
        self.mark_end_stream(stream_id, flags);
        Ok(())
    }

    fn drain_pending_data_frames(&mut self) {
        while let Some(frame) = self.pending_data_frames.pop_front() {
            if frame.payload.is_empty() {
                continue;
            }
            match frame.stream_id {
                STREAM_CLIENT_SERVER => self.client_server_buf.extend_from_slice(&frame.payload),
                STREAM_SERVER_CLIENT => self.server_client_buf.extend_from_slice(&frame.payload),
                stream_id => self
                    .stream_bufs
                    .entry(stream_id)
                    .or_default()
                    .extend_from_slice(&frame.payload),
            }
        }
    }

    fn mark_end_stream(&mut self, stream_id: u32, flags: u8) {
        if flags & FLAG_END_STREAM == 0 {
            return;
        }
        // A side-channel preamble is the protocol-level opening marker, but
        // an actual END_STREAM still closes it and releases the live registry
        // entry. Retain the normal bounded tombstone so a later frame cannot
        // reopen the stream ID.
        self.announced_file_streams.remove(&stream_id);
        self.file_stream_candidates.remove(&stream_id);
        self.file_stream_candidate_data.remove(&stream_id);
        // Retain EOF for every stream, including an empty unknown stream, so a
        // reader cannot wait forever after a clean END_STREAM. Keep this set
        // bounded because an attacker can otherwise send unlimited empty
        // END_STREAM frames without consuming the byte budget.
        if !self.closed_streams.contains(&stream_id) && !self.is_known_stream(stream_id) {
            if self.closed_unknown_order.len() >= MAX_CLOSED_STREAMS {
                if let Some(oldest) = self.closed_unknown_order.pop_front() {
                    self.closed_streams.remove(&oldest);
                }
            }
            self.closed_unknown_order.push_back(stream_id);
        }
        self.closed_streams.insert(stream_id);
        self.peer_open_streams.remove(&stream_id);
        self.pending_window_updates.remove(&stream_id);
    }

    fn stream_buffer_len(&self, stream_id: u32) -> usize {
        match stream_id {
            STREAM_CLIENT_SERVER => self.client_server_buf.len(),
            STREAM_SERVER_CLIENT => self.server_client_buf.len(),
            _ => self.stream_bufs.get(&stream_id).map_or(0, BytesMut::len),
        }
    }

    fn take_stream_bytes(&mut self, stream_id: u32, n: usize) -> Result<Bytes, H2Error> {
        if self.stream_buffer_len(stream_id) < n {
            return Err(H2Error::Protocol(format!(
                "stream {stream_id} has fewer than {n} buffered bytes"
            )));
        }
        let bytes = match stream_id {
            STREAM_CLIENT_SERVER => self.client_server_buf.split_to(n).freeze(),
            STREAM_SERVER_CLIENT => self.server_client_buf.split_to(n).freeze(),
            _ => self
                .stream_bufs
                .get_mut(&stream_id)
                .map(|buf| buf.split_to(n).freeze())
                .ok_or_else(|| H2Error::Protocol(format!("stream {stream_id} not open")))?,
        };
        self.buffered_bytes = self.buffered_bytes.saturating_sub(n);
        if stream_id != STREAM_CLIENT_SERVER
            && stream_id != STREAM_SERVER_CLIENT
            && self
                .stream_bufs
                .get(&stream_id)
                .is_some_and(BytesMut::is_empty)
            && !self.locally_open_streams.contains(&stream_id)
        {
            self.stream_bufs.remove(&stream_id);
        }
        Ok(bytes)
    }
}

fn frame_type_name(frame_type: u8) -> &'static str {
    match frame_type {
        FRAME_DATA => "DATA",
        FRAME_HEADERS => "HEADERS",
        FRAME_RST_STREAM => "RST_STREAM",
        FRAME_SETTINGS => "SETTINGS",
        FRAME_GOAWAY => "GOAWAY",
        FRAME_WINDOW_UPDATE => "WINDOW_UPDATE",
        _ => "OTHER",
    }
}

/// Recognise the one wire shape used by remoted for a device-created file
/// side-channel. This intentionally validates the complete XPC wrapper rather
/// than treating every even stream (or every 24-byte payload) as trusted.
fn is_file_stream_preamble(stream_id: u32, payload: &[u8]) -> bool {
    if payload.len() != FILE_STREAM_PREAMBLE_LEN {
        return false;
    }
    is_file_stream_preamble_prefix(stream_id, payload)
}

fn is_file_stream_preamble_prefix(stream_id: u32, payload: &[u8]) -> bool {
    if stream_id == STREAM_INIT
        || !stream_id.is_multiple_of(2)
        || payload.len() < FILE_STREAM_PREAMBLE_LEN
    {
        return false;
    }
    let magic = u32::from_le_bytes(payload[..4].try_into().unwrap());
    let flags = u32::from_le_bytes(payload[4..8].try_into().unwrap());
    let body_len = u64::from_le_bytes(payload[8..16].try_into().unwrap());
    magic == WRAPPER_MAGIC
        && flags
            == (crate::xpc::message::flags::ALWAYS_SET
                | crate::xpc::message::flags::FILE_TX_STREAM_REQUEST)
        && body_len == 0
}

// ── RawFrame ─────────────────────────────────────────────────────────────────

struct RawFrame {
    frame_type: u8,
    flags: u8,
    stream_id: u32,
    payload: Vec<u8>,
}

// ── Frame builders ────────────────────────────────────────────────────────────

fn build_frame(frame_type: u8, flags: u8, stream_id: u32, payload: &[u8]) -> Vec<u8> {
    let len = payload.len();
    let mut out = Vec::with_capacity(9 + len);
    out.push(((len >> 16) & 0xFF) as u8);
    out.push(((len >> 8) & 0xFF) as u8);
    out.push((len & 0xFF) as u8);
    out.push(frame_type);
    out.push(flags);
    out.extend_from_slice(&(stream_id & 0x7FFFFFFF).to_be_bytes());
    out.extend_from_slice(payload);
    out
}

fn build_settings_frame(settings: &[(u16, u32)]) -> Vec<u8> {
    let mut payload = Vec::new();
    for (id, val) in settings {
        payload.extend_from_slice(&id.to_be_bytes());
        payload.extend_from_slice(&val.to_be_bytes());
    }
    build_frame(FRAME_SETTINGS, 0, STREAM_INIT, &payload)
}

fn build_settings_ack() -> Vec<u8> {
    build_frame(FRAME_SETTINGS, FLAG_SETTINGS_ACK, STREAM_INIT, &[])
}

fn build_window_update_frame(stream_id: u32, increment: u32) -> Vec<u8> {
    build_frame(
        FRAME_WINDOW_UPDATE,
        0,
        stream_id,
        &(increment & 0x7FFFFFFF).to_be_bytes(),
    )
}

fn build_headers_frame(stream_id: u32) -> Vec<u8> {
    // Empty HEADERS frame with END_HEADERS flag (opens the stream)
    build_frame(FRAME_HEADERS, FLAG_END_HEADERS, stream_id, &[])
}

fn build_data_frame(stream_id: u32, data: &[u8]) -> Vec<u8> {
    build_frame(FRAME_DATA, 0, stream_id, data)
}

// ── Error type ────────────────────────────────────────────────────────────────

#[derive(Debug, thiserror::Error)]
pub enum H2Error {
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
    #[error("H2 protocol error: {0}")]
    Protocol(String),
}

#[cfg(test)]
mod tests {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::time::{timeout, Duration};

    use super::*;

    #[test]
    fn test_settings_frame_layout() {
        let frame = build_settings_frame(&[
            (SETTINGS_MAX_CONCURRENT_STREAMS, 100),
            (SETTINGS_INITIAL_WINDOW_SIZE, 1_048_576),
        ]);
        // 9-byte header + 2×6 bytes = 21 bytes
        assert_eq!(frame.len(), 9 + 12);
        assert_eq!(frame[3], FRAME_SETTINGS); // type
        assert_eq!(frame[4], 0); // no flags
    }

    #[test]
    fn test_window_update_frame() {
        let frame = build_window_update_frame(0, 983_041);
        assert_eq!(frame.len(), 9 + 4);
        assert_eq!(frame[3], FRAME_WINDOW_UPDATE);
    }

    #[test]
    fn test_data_frame() {
        let data = b"hello XPC";
        let frame = build_data_frame(STREAM_CLIENT_SERVER, data);
        assert_eq!(frame.len(), 9 + data.len());
        assert_eq!(frame[3], FRAME_DATA);
        assert_eq!(&frame[9..], data);
        // Stream ID 1
        let sid = u32::from_be_bytes([frame[5] & 0x7F, frame[6], frame[7], frame[8]]);
        assert_eq!(sid, STREAM_CLIENT_SERVER);
    }

    #[tokio::test]
    async fn test_dispatch_frame_acknowledges_settings_immediately() {
        let (client, mut server) = tokio::io::duplex(1024);
        let mut framer = H2Framer::new(client);

        framer
            .dispatch_frame(RawFrame {
                frame_type: FRAME_SETTINGS,
                flags: 0,
                stream_id: STREAM_INIT,
                payload: vec![],
            })
            .await
            .unwrap();

        let mut ack = [0u8; 9];
        server.read_exact(&mut ack).await.unwrap();
        assert_eq!(ack[3], FRAME_SETTINGS);
        assert_eq!(ack[4], FLAG_SETTINGS_ACK);
    }

    #[tokio::test]
    async fn test_open_stream_still_sends_headers_after_remote_data_buffered() {
        let (client, mut server) = tokio::io::duplex(1024);
        let mut framer = H2Framer::new(client);

        framer
            .dispatch_frame(RawFrame {
                frame_type: FRAME_DATA,
                flags: 0,
                stream_id: 4,
                payload: vec![1, 2, 3],
            })
            .await
            .unwrap();

        framer.open_stream(4).await.unwrap();

        // 3 bytes is far below the batching threshold, so HEADERS is the first
        // thing on the wire.
        let mut headers = [0u8; 9];
        server.read_exact(&mut headers).await.unwrap();
        assert_eq!(headers[3], FRAME_HEADERS);
        assert_eq!(headers[4], FLAG_END_HEADERS);
        assert_eq!(
            u32::from_be_bytes([headers[5] & 0x7F, headers[6], headers[7], headers[8]]),
            4
        );
    }

    fn test_framer<S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin>(
        stream: S,
    ) -> H2Framer<S> {
        H2Framer::new(stream)
    }

    fn read_window_update(frame: &[u8]) -> (u32, u32) {
        assert_eq!(frame[3], FRAME_WINDOW_UPDATE);
        let stream_id = u32::from_be_bytes([frame[5] & 0x7F, frame[6], frame[7], frame[8]]);
        let increment = u32::from_be_bytes([frame[9], frame[10], frame[11], frame[12]]);
        (stream_id, increment)
    }

    async fn read_wire_frame<S: tokio::io::AsyncRead + Unpin>(stream: &mut S) -> RawFrame {
        let mut header = [0u8; 9];
        stream.read_exact(&mut header).await.unwrap();
        let length =
            ((header[0] as usize) << 16) | ((header[1] as usize) << 8) | header[2] as usize;
        let mut payload = vec![0u8; length];
        if length != 0 {
            stream.read_exact(&mut payload).await.unwrap();
        }
        RawFrame {
            frame_type: header[3],
            flags: header[4],
            stream_id: u32::from_be_bytes([header[5] & 0x7F, header[6], header[7], header[8]]),
            payload,
        }
    }

    fn setting_payload(id: u16, value: u32) -> Vec<u8> {
        let mut payload = Vec::with_capacity(6);
        payload.extend_from_slice(&id.to_be_bytes());
        payload.extend_from_slice(&value.to_be_bytes());
        payload
    }

    fn rst_stream_frame(stream_id: u32, error_code: u32) -> Vec<u8> {
        build_frame(FRAME_RST_STREAM, 0, stream_id, &error_code.to_be_bytes())
    }

    fn file_stream_preamble(msg_id: u64) -> Vec<u8> {
        let mut payload = vec![0u8; FILE_STREAM_PREAMBLE_LEN];
        payload[..4].copy_from_slice(&WRAPPER_MAGIC.to_le_bytes());
        payload[4..8].copy_from_slice(
            &(crate::xpc::message::flags::ALWAYS_SET
                | crate::xpc::message::flags::FILE_TX_STREAM_REQUEST)
                .to_le_bytes(),
        );
        payload[16..24].copy_from_slice(&msg_id.to_le_bytes());
        payload
    }

    fn goaway_payload(last_stream_id: u32, error_code: u32) -> Vec<u8> {
        let mut payload = Vec::with_capacity(8);
        payload.extend_from_slice(&(last_stream_id & 0x7FFF_FFFF).to_be_bytes());
        payload.extend_from_slice(&error_code.to_be_bytes());
        payload
    }

    /// Streams 1 and 3 carry all XPC traffic and are odd; the old `% 2 == 0`
    /// guard meant their windows were never given back, so a service streaming
    /// more than the initial window in total stalled forever.
    #[tokio::test]
    async fn replenishes_receive_window_for_the_odd_xpc_streams() {
        let (client, mut server) = tokio::io::duplex(64 * 1024);
        let mut framer = test_framer(client);

        // Stay just under the threshold: nothing should go out yet.
        let below = (WINDOW_UPDATE_THRESHOLD as usize) / MAX_FRAME_PAYLOAD - 1;
        for _ in 0..below {
            framer
                .replenish_receive_window(STREAM_SERVER_CLIENT, MAX_FRAME_PAYLOAD)
                .await
                .unwrap();
        }
        assert_eq!(
            framer.pending_window_updates[&STREAM_SERVER_CLIENT],
            (below * MAX_FRAME_PAYLOAD) as u32
        );

        // Crossing it gives back both the connection window and the stream window.
        framer
            .replenish_receive_window(STREAM_SERVER_CLIENT, MAX_FRAME_PAYLOAD)
            .await
            .unwrap();

        let mut frames = [0u8; 26];
        server.read_exact(&mut frames).await.unwrap();
        assert_eq!(
            read_window_update(&frames[..13]),
            (STREAM_INIT, WINDOW_UPDATE_THRESHOLD)
        );
        assert_eq!(
            read_window_update(&frames[13..]),
            (STREAM_SERVER_CLIENT, WINDOW_UPDATE_THRESHOLD)
        );
        assert_eq!(framer.pending_window_updates[&STREAM_SERVER_CLIENT], 0);
        assert_eq!(framer.pending_window_updates[&STREAM_INIT], 0);
    }

    #[tokio::test]
    async fn rejects_frames_larger_than_the_negotiated_max_frame_size() {
        let (client, mut server) = tokio::io::duplex(1024);
        let mut framer = test_framer(client);

        let oversized = MAX_FRAME_PAYLOAD + 1;
        let mut header = vec![
            ((oversized >> 16) & 0xFF) as u8,
            ((oversized >> 8) & 0xFF) as u8,
            (oversized & 0xFF) as u8,
            FRAME_DATA,
            0,
        ];
        header.extend_from_slice(&STREAM_SERVER_CLIENT.to_be_bytes());
        server.write_all(&header).await.unwrap();

        let err = match framer.read_raw_frame().await {
            Ok(_) => panic!("oversized frame was accepted"),
            Err(err) => err,
        };
        match err {
            H2Error::Protocol(message) => {
                assert!(message.contains("exceeds max frame size"), "{message}")
            }
            other => panic!("unexpected error: {other:?}"),
        }
    }

    #[tokio::test]
    async fn read_next_data_frame_preserves_stream_id_and_flags() {
        let (client, mut server) = tokio::io::duplex(1024);
        let mut framer = H2Framer::new(client);

        server
            .write_all(&build_frame(FRAME_DATA, 0x01, 6, b"chunk"))
            .await
            .unwrap();

        let frame = framer.read_next_data_frame().await.unwrap();

        assert_eq!(frame.stream_id, 6);
        assert_eq!(frame.flags, 0x01);
        assert_eq!(frame.payload, Bytes::from_static(b"chunk"));
    }

    #[tokio::test]
    async fn ignores_reserved_stream_id_bit_when_receiving_a_frame() {
        let (client, mut server) = tokio::io::duplex(1024);
        let mut framer = H2Framer::new(client);
        let mut frame = build_frame(FRAME_DATA, 0, STREAM_SERVER_CLIENT, b"chunk");
        frame[5] |= 0x80;
        server.write_all(&frame).await.unwrap();

        let received = framer.read_next_data_frame().await.unwrap();

        assert_eq!(received.stream_id, STREAM_SERVER_CLIENT);
        assert!(framer.connection_error.is_none());
    }

    #[tokio::test]
    async fn read_stream_reports_clean_end_stream_instead_of_waiting_forever() {
        let (client, mut server) = tokio::io::duplex(1024);
        let mut framer = test_framer(client);

        server
            .write_all(&build_frame(
                FRAME_DATA,
                FLAG_END_STREAM,
                STREAM_SERVER_CLIENT,
                &[],
            ))
            .await
            .unwrap();

        let error = framer
            .read_stream(STREAM_SERVER_CLIENT, 24)
            .await
            .unwrap_err();
        assert!(error.to_string().contains("ended before 24 bytes"));
    }

    #[tokio::test]
    async fn large_file_transfer_header_is_selected_before_h2_buffer_limit() {
        let (client, mut server) = tokio::io::duplex(64 * 1024);
        let mut framer = H2Framer::new(client);
        let body_len = MAX_BUFFERED_BYTES_PER_STREAM - 24;
        let mut header = [0u8; 24];
        header[..4].copy_from_slice(&crate::xpc::message::WRAPPER_MAGIC.to_le_bytes());
        header[4..8]
            .copy_from_slice(&crate::xpc::message::flags::FILE_TX_STREAM_RESPONSE.to_le_bytes());
        header[8..16].copy_from_slice(&(body_len as u64).to_le_bytes());
        header[16..24].copy_from_slice(&17u64.to_le_bytes());

        let mut next = [0u8; 24];
        next[..4].copy_from_slice(&crate::xpc::message::WRAPPER_MAGIC.to_le_bytes());
        next[4..8].copy_from_slice(&crate::xpc::message::flags::ALWAYS_SET.to_le_bytes());
        next[16..24].copy_from_slice(&18u64.to_le_bytes());

        // The final DATA frame contains the tail of the large body and the
        // complete next wrapper. A reader that waits for the first message to
        // reach the 16 MiB H2 limit would reject this frame before consuming
        // either message; the FILE_TX path must select the header and drain
        // the body in bounded chunks instead.
        let mut payload = vec![0u8; body_len + next.len()];
        payload[body_len..].copy_from_slice(&next);
        // Keep the wrapper header in its own DATA frame. The body and next
        // wrapper then total exactly 16 MiB, making their final frame contain
        // 16,360 body bytes followed by the next 24-byte wrapper.
        let mut wire = build_frame(FRAME_DATA, 0, STREAM_SERVER_CLIENT, &header);
        for chunk in payload.chunks(MAX_FRAME_PAYLOAD) {
            wire.extend_from_slice(&build_frame(FRAME_DATA, 0, STREAM_SERVER_CLIENT, chunk));
        }
        let (hold_tx, hold_rx) = tokio::sync::oneshot::channel();
        let sender = tokio::spawn(async move {
            server.write_all(&wire).await.unwrap();
            server.flush().await.unwrap();
            let _ = hold_rx.await;
        });

        let stream_id = timeout(Duration::from_secs(5), framer.buffer_next_data_stream())
            .await
            .expect("large FILE_TX header selection timed out")
            .unwrap();
        assert_eq!(stream_id, STREAM_SERVER_CLIENT);
        assert_eq!(
            framer.read_stream(STREAM_SERVER_CLIENT, 24).await.unwrap(),
            &header[..]
        );

        let first_chunk = MAX_BUFFERED_BYTES_PER_STREAM - MAX_FRAME_PAYLOAD;
        let first = framer
            .read_stream(STREAM_SERVER_CLIENT, first_chunk)
            .await
            .unwrap();
        assert!(first.iter().all(|byte| *byte == 0));
        let second_len = body_len - first_chunk;
        let second = framer
            .read_stream(STREAM_SERVER_CLIENT, second_len)
            .await
            .unwrap();
        assert!(second.iter().all(|byte| *byte == 0));

        let next_stream = timeout(Duration::from_secs(5), framer.buffer_next_data_stream())
            .await
            .expect("next wrapper selection timed out")
            .unwrap();
        assert_eq!(next_stream, STREAM_SERVER_CLIENT);
        assert_eq!(
            framer.read_stream(STREAM_SERVER_CLIENT, 24).await.unwrap(),
            &next[..]
        );
        let _ = hold_tx.send(());
        sender.await.unwrap();
    }

    #[tokio::test]
    async fn read_stream_reports_clean_end_stream_for_unknown_empty_stream() {
        let (client, mut server) = tokio::io::duplex(1024);
        let mut framer = test_framer(client);

        server
            .write_all(&build_frame(FRAME_DATA, FLAG_END_STREAM, 6, &[]))
            .await
            .unwrap();

        let error = framer.read_stream(6, 1).await.unwrap_err();
        assert!(error.to_string().contains("stream 6 ended before 1 bytes"));
    }

    #[tokio::test]
    async fn closed_stream_tombstone_survives_buffer_consumption() {
        let (client, mut server) = tokio::io::duplex(1024);
        let mut framer = test_framer(client);

        server
            .write_all(&build_frame(FRAME_DATA, FLAG_END_STREAM, 6, b"chunk"))
            .await
            .unwrap();
        assert_eq!(framer.read_stream(6, 5).await.unwrap(), b"chunk"[..]);
        assert!(framer.closed_streams.contains(&6));

        let error = timeout(Duration::from_millis(100), framer.read_stream(6, 1))
            .await
            .expect("closed stream must not wait for a reused stream ID")
            .unwrap_err();
        assert!(error.to_string().contains("stream 6 ended before 1 bytes"));
    }

    #[tokio::test]
    async fn unknown_eof_flood_cannot_evict_fixed_xpc_tombstones() {
        let (client, _server) = tokio::io::duplex(1024);
        let mut framer = test_framer(client);

        for stream_id in [STREAM_CLIENT_SERVER, STREAM_SERVER_CLIENT] {
            framer
                .dispatch_frame(RawFrame {
                    frame_type: FRAME_DATA,
                    flags: FLAG_END_STREAM,
                    stream_id,
                    payload: Vec::new(),
                })
                .await
                .unwrap();
        }
        framer.open_stream(5).await.unwrap();
        framer
            .dispatch_frame(RawFrame {
                frame_type: FRAME_DATA,
                flags: FLAG_END_STREAM,
                stream_id: 5,
                payload: Vec::new(),
            })
            .await
            .unwrap();

        for stream_id in 100..100 + MAX_CLOSED_STREAMS as u32 + 1 {
            framer
                .dispatch_frame(RawFrame {
                    frame_type: FRAME_DATA,
                    flags: FLAG_END_STREAM,
                    stream_id,
                    payload: Vec::new(),
                })
                .await
                .unwrap();
        }

        assert!(framer.closed_streams.contains(&STREAM_CLIENT_SERVER));
        assert!(framer.closed_streams.contains(&STREAM_SERVER_CLIENT));
        assert!(framer.closed_streams.contains(&5));
        assert_eq!(framer.closed_unknown_order.len(), MAX_CLOSED_STREAMS);
        assert!(!framer.closed_streams.contains(&100));
        assert_eq!(framer.closed_unknown_order.front(), Some(&101));

        for stream_id in [STREAM_CLIENT_SERVER, STREAM_SERVER_CLIENT, 5] {
            let error = framer.read_stream(stream_id, 1).await.unwrap_err();
            assert!(error.to_string().contains("ended before 1 bytes"));
        }

        // Eviction never turns an old stream ID into a reusable stream.
        let error = framer
            .dispatch_frame(RawFrame {
                frame_type: FRAME_HEADERS,
                flags: FLAG_END_HEADERS,
                stream_id: 100,
                payload: Vec::new(),
            })
            .await
            .unwrap_err();
        assert!(error.to_string().contains("cannot be reused"));

        let next_stream = 100 + MAX_CLOSED_STREAMS as u32 + 1;
        framer
            .dispatch_frame(RawFrame {
                frame_type: FRAME_HEADERS,
                flags: FLAG_END_HEADERS,
                stream_id: next_stream,
                payload: Vec::new(),
            })
            .await
            .unwrap();
        assert!(framer.peer_open_streams.contains(&next_stream));
    }

    #[tokio::test]
    async fn unknown_reset_flood_cannot_evict_open_xpc_tombstones() {
        let (client, _server) = tokio::io::duplex(1024);
        let mut framer = test_framer(client);

        for stream_id in [STREAM_CLIENT_SERVER, STREAM_SERVER_CLIENT] {
            framer
                .dispatch_frame(RawFrame {
                    frame_type: FRAME_RST_STREAM,
                    flags: 0,
                    stream_id,
                    payload: 0x11u32.to_be_bytes().to_vec(),
                })
                .await
                .unwrap();
        }
        framer.open_stream(5).await.unwrap();
        framer
            .dispatch_frame(RawFrame {
                frame_type: FRAME_RST_STREAM,
                flags: 0,
                stream_id: 5,
                payload: 0x33u32.to_be_bytes().to_vec(),
            })
            .await
            .unwrap();

        for stream_id in 100..100 + MAX_CLOSED_STREAMS as u32 + 1 {
            framer
                .dispatch_frame(RawFrame {
                    frame_type: FRAME_HEADERS,
                    flags: FLAG_END_HEADERS,
                    stream_id,
                    payload: Vec::new(),
                })
                .await
                .unwrap();
            framer
                .dispatch_frame(RawFrame {
                    frame_type: FRAME_RST_STREAM,
                    flags: 0,
                    stream_id,
                    payload: 0x22u32.to_be_bytes().to_vec(),
                })
                .await
                .unwrap();
        }

        assert_eq!(framer.reset_unknown_order.len(), MAX_CLOSED_STREAMS);
        assert!(!framer.reset_streams.contains_key(&100));
        assert_eq!(framer.reset_unknown_order.front(), Some(&101));
        assert_eq!(framer.reset_streams.get(&STREAM_CLIENT_SERVER), Some(&0x11));
        assert_eq!(framer.reset_streams.get(&STREAM_SERVER_CLIENT), Some(&0x11));
        assert_eq!(framer.reset_streams.get(&5), Some(&0x33));

        for stream_id in [STREAM_CLIENT_SERVER, STREAM_SERVER_CLIENT, 5] {
            let error = framer.read_stream(stream_id, 1).await.unwrap_err();
            assert!(error.to_string().contains("was reset by peer"));
        }

        let error = framer
            .dispatch_frame(RawFrame {
                frame_type: FRAME_HEADERS,
                flags: FLAG_END_HEADERS,
                stream_id: 100,
                payload: Vec::new(),
            })
            .await
            .unwrap_err();
        assert!(error.to_string().contains("cannot be reused"));
    }

    #[tokio::test]
    async fn frames_after_end_stream_are_stream_errors_not_connection_fatal() {
        let (client, _server) = tokio::io::duplex(1024);
        let mut framer = test_framer(client);

        framer
            .dispatch_frame(RawFrame {
                frame_type: FRAME_DATA,
                flags: FLAG_END_STREAM,
                stream_id: 6,
                payload: Vec::new(),
            })
            .await
            .unwrap();
        let error = framer
            .dispatch_frame(RawFrame {
                frame_type: FRAME_DATA,
                flags: 0,
                stream_id: 6,
                payload: vec![1],
            })
            .await
            .unwrap_err();
        assert!(error.to_string().contains("after stream 6 ended"));
        assert!(framer.connection_error.is_none());
    }

    #[tokio::test]
    async fn read_next_data_frame_rejects_data_after_end_stream() {
        let (client, mut server) = tokio::io::duplex(1024);
        let mut framer = test_framer(client);
        let mut frames = build_frame(FRAME_DATA, FLAG_END_STREAM, 6, b"done");
        frames.extend_from_slice(&build_frame(FRAME_DATA, 0, 6, b"late"));
        server.write_all(&frames).await.unwrap();

        assert_eq!(
            framer.read_next_data_frame().await.unwrap().payload,
            b"done"[..]
        );
        let error = framer.read_next_data_frame().await.unwrap_err();

        assert!(error.to_string().contains("after stream 6 ended"));
        assert!(framer.connection_error.is_none());
    }

    #[tokio::test]
    async fn write_stream_splits_oversized_data_frames() {
        let (client, mut server) = tokio::io::duplex(64 * 1024);
        let mut framer = test_framer(client);
        let payload = vec![0xA5; MAX_FRAME_PAYLOAD + 1];

        framer.write_stream(5, &payload).await.unwrap();

        let mut headers = [0u8; 9];
        server.read_exact(&mut headers).await.unwrap();
        assert_eq!(headers[3], FRAME_HEADERS);
        assert_eq!(headers[4], FLAG_END_HEADERS);

        let mut first = vec![0u8; 9 + MAX_FRAME_PAYLOAD];
        server.read_exact(&mut first).await.unwrap();
        assert_eq!(first[3], FRAME_DATA);
        assert_eq!(
            ((first[0] as usize) << 16) | ((first[1] as usize) << 8) | first[2] as usize,
            MAX_FRAME_PAYLOAD
        );
        assert_eq!(&first[9..], &payload[..MAX_FRAME_PAYLOAD]);

        let mut second = [0u8; 10];
        server.read_exact(&mut second).await.unwrap();
        assert_eq!(second[3], FRAME_DATA);
        assert_eq!(&second[9..], &payload[MAX_FRAME_PAYLOAD..]);
    }

    #[tokio::test]
    async fn default_peer_window_allows_exactly_65535_bytes() {
        let (client, mut peer) = tokio::io::duplex(128 * 1024);
        let mut framer = test_framer(client);
        let payload = vec![0x5A; DEFAULT_PEER_WINDOW_SIZE as usize];
        let expected = payload.clone();

        let peer_task = tokio::spawn(async move {
            let headers = read_wire_frame(&mut peer).await;
            assert_eq!(headers.frame_type, FRAME_HEADERS);
            assert_eq!(headers.stream_id, 5);

            let mut received = Vec::new();
            while received.len() < DEFAULT_PEER_WINDOW_SIZE as usize {
                let frame = read_wire_frame(&mut peer).await;
                assert_eq!(frame.frame_type, FRAME_DATA);
                assert_eq!(frame.stream_id, 5);
                assert!(frame.payload.len() <= MAX_FRAME_PAYLOAD);
                received.extend_from_slice(&frame.payload);
            }
            assert_eq!(received, expected);
        });

        framer.write_stream(5, &payload).await.unwrap();
        peer_task.await.unwrap();
    }

    #[tokio::test]
    async fn write_stream_waits_for_split_connection_and_stream_updates() {
        let (client, mut peer) = tokio::io::duplex(16 * 1024);
        let mut framer = test_framer(client);
        framer.open_stream(5).await.unwrap();
        let headers = read_wire_frame(&mut peer).await;
        assert_eq!(headers.frame_type, FRAME_HEADERS);

        // Exercise min(connection window, stream window, max frame size) and
        // force the writer through several asynchronous update waits.
        framer.outbound_connection_window = 4;
        framer.outbound_stream_windows.insert(5, 2);
        let payload: Vec<u8> = (0..10).collect();
        let expected = payload.clone();
        let peer_task = tokio::spawn(async move {
            let first = read_wire_frame(&mut peer).await;
            assert_eq!(first.frame_type, FRAME_DATA);
            assert_eq!(first.stream_id, 5);
            assert_eq!(first.payload, expected[..2]);

            // A connection update alone leaves the stream as the limiting
            // window. The second update pair then exercises both counters.
            peer.write_all(&build_window_update_frame(STREAM_INIT, 2))
                .await
                .unwrap();
            peer.write_all(&build_window_update_frame(5, 1))
                .await
                .unwrap();
            peer.flush().await.unwrap();

            let second = read_wire_frame(&mut peer).await;
            assert_eq!(second.payload, expected[2..3]);

            peer.write_all(&build_window_update_frame(STREAM_INIT, 3))
                .await
                .unwrap();
            peer.write_all(&build_window_update_frame(5, 3))
                .await
                .unwrap();
            peer.flush().await.unwrap();

            let third = read_wire_frame(&mut peer).await;
            assert_eq!(third.payload, expected[3..6]);

            peer.write_all(&build_window_update_frame(STREAM_INIT, 4))
                .await
                .unwrap();
            peer.write_all(&build_window_update_frame(5, 4))
                .await
                .unwrap();
            peer.flush().await.unwrap();

            let fourth = read_wire_frame(&mut peer).await;
            assert_eq!(fourth.payload, expected[6..]);
        });

        framer.write_stream(5, &payload).await.unwrap();
        peer_task.await.unwrap();
    }

    #[tokio::test]
    async fn writer_flow_control_wait_preserves_business_data_frames() {
        let (client, mut peer) = tokio::io::duplex(16 * 1024);
        let mut framer = test_framer(client);
        framer.open_stream(5).await.unwrap();
        assert_eq!(read_wire_frame(&mut peer).await.frame_type, FRAME_HEADERS);
        framer.outbound_connection_window = 0;
        framer.outbound_stream_windows.insert(5, 0);

        let peer_task = tokio::spawn(async move {
            peer.write_all(&build_frame(
                FRAME_DATA,
                0,
                STREAM_SERVER_CLIENT,
                b"business",
            ))
            .await
            .unwrap();
            peer.write_all(&build_window_update_frame(STREAM_INIT, 3))
                .await
                .unwrap();
            peer.write_all(&build_window_update_frame(5, 3))
                .await
                .unwrap();
            peer.flush().await.unwrap();

            let outgoing = read_wire_frame(&mut peer).await;
            assert_eq!(outgoing.frame_type, FRAME_DATA);
            assert_eq!(outgoing.stream_id, 5);
            assert_eq!(outgoing.payload, b"out");
        });

        framer.write_stream(5, b"out").await.unwrap();
        let incoming = framer.read_next_data_frame().await.unwrap();
        assert_eq!(incoming.stream_id, STREAM_SERVER_CLIENT);
        assert_eq!(incoming.payload, &b"business"[..]);
        peer_task.await.unwrap();
    }

    #[tokio::test]
    async fn peer_settings_apply_delta_to_existing_stream_and_max_frame_size() {
        let (client, mut peer) = tokio::io::duplex(64 * 1024);
        let mut framer = test_framer(client);
        framer.open_stream(5).await.unwrap();
        assert_eq!(read_wire_frame(&mut peer).await.frame_type, FRAME_HEADERS);
        framer.consume_outbound_window(5, 100).unwrap();

        let mut payload = setting_payload(SETTINGS_INITIAL_WINDOW_SIZE, 65_540);
        payload.extend_from_slice(&setting_payload(SETTINGS_MAX_FRAME_SIZE, 65_535));
        framer.process_settings(0, &payload).await.unwrap();
        let ack = read_wire_frame(&mut peer).await;
        assert_eq!(ack.frame_type, FRAME_SETTINGS);
        assert_eq!(ack.flags, FLAG_SETTINGS_ACK);
        assert_eq!(framer.peer_initial_window_size, 65_540);
        assert_eq!(framer.outbound_stream_windows[&5], 65_440);
        assert_eq!(framer.outbound_max_frame_size, 65_535);

        // The negotiated frame limit is used for outbound DATA, while the
        // connection and stream windows remain independently accounted for.
        let data = vec![0xC3; 20_000];
        framer.write_stream(5, &data).await.unwrap();
        let frame = read_wire_frame(&mut peer).await;
        assert_eq!(frame.frame_type, FRAME_DATA);
        assert_eq!(frame.payload, data);
    }

    #[tokio::test]
    async fn validates_max_frame_size_boundaries() {
        let cases = [
            (MAX_FRAME_PAYLOAD as u32 - 1, false),
            (MAX_FRAME_PAYLOAD as u32, true),
            (u16::MAX as u32, true),
            (u16::MAX as u32 + 1, true),
            (MAX_NEGOTIATED_FRAME_SIZE as u32, true),
            (MAX_NEGOTIATED_FRAME_SIZE as u32 + 1, false),
            (u32::MAX, false),
        ];

        for (value, valid) in cases {
            let (client, _peer) = tokio::io::duplex(1024);
            let mut framer = test_framer(client);
            let result = framer
                .process_settings(0, &setting_payload(SETTINGS_MAX_FRAME_SIZE, value))
                .await;
            assert_eq!(result.is_ok(), valid, "SETTINGS_MAX_FRAME_SIZE={value}");
        }
    }

    #[tokio::test]
    async fn rejects_initial_window_delta_that_exceeds_flow_control_range() {
        let (client, _peer) = tokio::io::duplex(1024);
        let mut framer = test_framer(client);
        framer.peer_initial_window_size = 0;
        framer
            .outbound_stream_windows
            .insert(5, MAX_FLOW_CONTROL_WINDOW);

        let error = framer
            .process_settings(
                0,
                &setting_payload(SETTINGS_INITIAL_WINDOW_SIZE, MAX_FLOW_CONTROL_WINDOW as u32),
            )
            .await
            .unwrap_err();
        assert!(error.to_string().contains("outside flow-control range"));
        assert!(framer.connection_error.is_some());
    }

    #[tokio::test]
    async fn rejects_zero_and_overflowing_window_updates() {
        let (client, _peer) = tokio::io::duplex(1024);
        let mut framer = test_framer(client);
        let error = framer
            .process_window_update(STREAM_INIT, &0u32.to_be_bytes())
            .unwrap_err();
        assert!(error.to_string().contains("zero increment"));

        let (client, _peer) = tokio::io::duplex(1024);
        let mut framer = test_framer(client);
        framer.outbound_connection_window = MAX_FLOW_CONTROL_WINDOW - 1;
        let error = framer
            .process_window_update(STREAM_INIT, &2u32.to_be_bytes())
            .unwrap_err();
        assert!(error.to_string().contains("exceeds"));

        let (client, _peer) = tokio::io::duplex(1024);
        let mut framer = test_framer(client);
        framer.outbound_connection_window = 0;
        framer
            .process_window_update(STREAM_INIT, &u32::MAX.to_be_bytes())
            .unwrap();
        assert_eq!(framer.outbound_connection_window, MAX_FLOW_CONTROL_WINDOW);
    }

    #[test]
    fn zero_stream_window_update_is_a_stream_error() {
        let (client, _peer) = tokio::io::duplex(1024);
        let mut framer = test_framer(client);
        framer
            .outbound_stream_windows
            .insert(5, DEFAULT_PEER_WINDOW_SIZE);

        let error = framer
            .process_window_update(5, &0u32.to_be_bytes())
            .unwrap_err();

        assert!(error.to_string().contains("zero increment"));
        assert!(framer.connection_error.is_none());
    }

    #[test]
    fn zero_window_update_for_idle_stream_is_connection_error() {
        let (client, _peer) = tokio::io::duplex(1024);
        let mut framer = test_framer(client);

        let error = framer
            .process_window_update(5, &0u32.to_be_bytes())
            .unwrap_err();

        assert!(error.to_string().contains("zero increment"));
        assert!(framer.connection_error.is_some());
    }

    #[test]
    fn window_update_for_peer_open_stream_is_not_an_idle_error() {
        let (client, _peer) = tokio::io::duplex(1024);
        let mut framer = test_framer(client);
        framer.peer_open_streams.insert(6);

        framer
            .process_window_update(6, &1u32.to_be_bytes())
            .unwrap();

        assert!(framer.connection_error.is_none());
    }

    #[tokio::test]
    async fn rst_stream_on_idle_stream_is_connection_error() {
        let (client, _peer) = tokio::io::duplex(1024);
        let mut framer = test_framer(client);

        let error = framer
            .dispatch_frame(RawFrame {
                frame_type: FRAME_RST_STREAM,
                flags: 0,
                stream_id: 9,
                payload: 0u32.to_be_bytes().to_vec(),
            })
            .await
            .unwrap_err();

        assert!(error.to_string().contains("idle stream 9"));
        assert!(framer.connection_error.is_some());
    }

    #[tokio::test]
    async fn rst_stream_fails_a_writer_waiting_for_window_capacity() {
        let (client, mut peer) = tokio::io::duplex(1024);
        let mut framer = test_framer(client);
        framer.open_stream(5).await.unwrap();
        assert_eq!(read_wire_frame(&mut peer).await.frame_type, FRAME_HEADERS);
        framer.outbound_connection_window = 0;
        peer.write_all(&rst_stream_frame(5, 0x8)).await.unwrap();

        let error = timeout(Duration::from_secs(1), framer.write_stream(5, b"data"))
            .await
            .expect("writer stayed blocked after RST_STREAM")
            .unwrap_err();
        assert!(error.to_string().contains("stream 5 was reset"));
        assert!(!framer.outbound_stream_windows.contains_key(&5));
        assert!(framer.open_stream(5).await.is_err());
    }

    #[tokio::test]
    async fn rst_stream_clears_data_queued_while_writer_waited() {
        let (client, mut peer) = tokio::io::duplex(4096);
        let mut framer = test_framer(client);
        framer.open_stream(5).await.unwrap();
        assert_eq!(read_wire_frame(&mut peer).await.frame_type, FRAME_HEADERS);
        framer.outbound_connection_window = 0;

        let mut inbound = build_frame(FRAME_DATA, 0, 5, b"queued");
        inbound.extend_from_slice(&rst_stream_frame(5, 0x8));
        peer.write_all(&inbound).await.unwrap();

        let error = timeout(Duration::from_secs(1), framer.write_stream(5, b"out"))
            .await
            .expect("writer stayed blocked after RST_STREAM")
            .unwrap_err();

        assert!(error.to_string().contains("stream 5 was reset"));
        assert!(framer.pending_data_frames.is_empty());
        assert_eq!(framer.buffered_bytes, 0);
    }

    #[tokio::test]
    async fn goaway_rejects_new_and_affected_streams_but_allows_existing_streams() {
        let (client, _peer) = tokio::io::duplex(4096);
        let mut framer = test_framer(client);
        framer.open_stream(1).await.unwrap();
        framer.open_stream(5).await.unwrap();
        framer
            .process_goaway(STREAM_INIT, &goaway_payload(3, 0))
            .unwrap();

        assert!(framer.write_stream(1, b"ok").await.is_ok());
        let affected = framer.write_stream(5, b"no").await.unwrap_err();
        assert!(affected.to_string().contains("refused by peer GOAWAY"));
        let new_stream = framer.open_stream(3).await.unwrap_err();
        assert!(new_stream.to_string().contains("cannot open stream 3"));
    }

    #[test]
    fn goaway_last_stream_id_cannot_increase() {
        let (client, _peer) = tokio::io::duplex(1024);
        let mut framer = test_framer(client);
        framer
            .process_goaway(STREAM_INIT, &goaway_payload(3, 0))
            .unwrap();

        let error = framer
            .process_goaway(STREAM_INIT, &goaway_payload(5, 0))
            .unwrap_err();

        assert!(error.to_string().contains("exceeds previous 3"));
        assert!(framer.connection_error.is_some());
    }

    #[tokio::test]
    async fn fragmented_data_uses_one_budget_and_releases_it_when_consumed() {
        let (client, _server) = tokio::io::duplex(1024);
        let mut framer = test_framer(client);

        framer
            .dispatch_frame(RawFrame {
                frame_type: FRAME_DATA,
                flags: 0,
                stream_id: 6,
                payload: vec![1, 2],
            })
            .await
            .unwrap();
        framer
            .dispatch_frame(RawFrame {
                frame_type: FRAME_DATA,
                flags: 0,
                stream_id: 6,
                payload: vec![3, 4],
            })
            .await
            .unwrap();
        assert_eq!(framer.buffered_bytes, 4);

        assert_eq!(
            framer.take_stream_bytes(6, 3).unwrap(),
            Bytes::from_static(&[1, 2, 3])
        );
        assert_eq!(framer.buffered_bytes, 1);
        assert_eq!(
            framer.take_stream_bytes(6, 1).unwrap(),
            Bytes::from_static(&[4])
        );
        assert_eq!(framer.buffered_bytes, 0);
        assert!(!framer.stream_bufs.contains_key(&6));
    }

    #[tokio::test]
    async fn rejects_unknown_stream_flood_before_inserting_the_offending_stream() {
        let (client, _server) = tokio::io::duplex(1024);
        let mut framer = test_framer(client);

        for stream_id in 100..100 + MAX_UNKNOWN_BUFFERED_STREAMS as u32 {
            framer
                .dispatch_frame(RawFrame {
                    frame_type: FRAME_DATA,
                    flags: 0,
                    stream_id,
                    payload: vec![stream_id as u8],
                })
                .await
                .unwrap();
        }
        assert_eq!(framer.stream_bufs.len(), MAX_UNKNOWN_BUFFERED_STREAMS);

        let error = framer
            .dispatch_frame(RawFrame {
                frame_type: FRAME_DATA,
                flags: 0,
                stream_id: 100 + MAX_UNKNOWN_BUFFERED_STREAMS as u32,
                payload: vec![0xFF],
            })
            .await
            .unwrap_err();
        assert!(error.to_string().contains("too many unknown H2 streams"));
        assert_eq!(framer.stream_bufs.len(), MAX_UNKNOWN_BUFFERED_STREAMS);
    }

    #[tokio::test]
    async fn validated_file_side_channels_may_exceed_unknown_stream_limit() {
        let (client, _server) = tokio::io::duplex(1024);
        let mut framer = test_framer(client);

        // iOS sends these peer-created even streams without END_STREAM/RST;
        // each is nevertheless a complete, body-less FILE_TX preamble.
        for index in 0..(MAX_UNKNOWN_BUFFERED_STREAMS * 2) {
            let stream_id = 2 * (index as u32 + 1);
            framer
                .dispatch_frame(RawFrame {
                    frame_type: FRAME_HEADERS,
                    flags: FLAG_END_HEADERS,
                    stream_id,
                    payload: Vec::new(),
                })
                .await
                .unwrap();
            framer
                .dispatch_frame(RawFrame {
                    frame_type: FRAME_DATA,
                    flags: 0,
                    stream_id,
                    payload: file_stream_preamble(index as u64 + 1),
                })
                .await
                .unwrap();
        }

        assert_eq!(
            framer.announced_file_streams.len(),
            MAX_UNKNOWN_BUFFERED_STREAMS * 2
        );
        assert!(framer.peer_open_streams.is_empty());
        assert!(framer.file_stream_candidates.is_empty());
    }

    #[tokio::test]
    async fn unknown_active_stream_budget_is_shared_with_file_candidates() {
        let (client, _server) = tokio::io::duplex(1024);
        let mut framer = test_framer(client);

        for index in 0..(MAX_UNKNOWN_BUFFERED_STREAMS / 2) {
            let candidate_id = 2 + (index as u32) * 4;
            framer
                .dispatch_frame(RawFrame {
                    frame_type: FRAME_HEADERS,
                    flags: FLAG_END_HEADERS,
                    stream_id: candidate_id,
                    payload: Vec::new(),
                })
                .await
                .unwrap();
            // Stream 3 is the fixed serverClient XPC stream, so use the next
            // odd ID outside the primary pair for the genuinely unknown
            // peer-created stream.
            let unknown_id = candidate_id + 3;
            framer
                .dispatch_frame(RawFrame {
                    frame_type: FRAME_DATA,
                    flags: 0,
                    stream_id: unknown_id,
                    payload: vec![0xAA],
                })
                .await
                .unwrap();
        }

        assert_eq!(
            framer.unknown_active_stream_count(),
            MAX_UNKNOWN_BUFFERED_STREAMS
        );
        let error = framer
            .dispatch_frame(RawFrame {
                frame_type: FRAME_HEADERS,
                flags: FLAG_END_HEADERS,
                stream_id: 66,
                payload: Vec::new(),
            })
            .await
            .unwrap_err();
        assert!(error
            .to_string()
            .contains("too many pending RemoteXPC file streams"));
    }

    #[tokio::test]
    async fn file_side_channel_preamble_can_be_fragmented_and_is_validated() {
        let (client, _server) = tokio::io::duplex(1024);
        let mut framer = test_framer(client);
        let preamble = file_stream_preamble(7);

        framer
            .dispatch_frame(RawFrame {
                frame_type: FRAME_HEADERS,
                flags: FLAG_END_HEADERS,
                stream_id: 2,
                payload: Vec::new(),
            })
            .await
            .unwrap();
        framer
            .dispatch_frame(RawFrame {
                frame_type: FRAME_DATA,
                flags: 0,
                stream_id: 2,
                payload: preamble[..10].to_vec(),
            })
            .await
            .unwrap();
        assert!(framer.file_stream_candidates.contains(&2));
        framer
            .dispatch_frame(RawFrame {
                frame_type: FRAME_DATA,
                flags: 0,
                stream_id: 2,
                payload: preamble[10..].to_vec(),
            })
            .await
            .unwrap();
        assert!(framer.announced_file_streams.contains(&2));
        assert!(framer.file_stream_candidate_data.get(&2).is_none());
    }

    #[tokio::test]
    async fn end_and_reset_release_file_side_channel_state() {
        let (client, _server) = tokio::io::duplex(1024);
        let mut framer = test_framer(client);

        // END_STREAM on a candidate closes it before it is ever adopted.
        framer
            .dispatch_frame(RawFrame {
                frame_type: FRAME_HEADERS,
                flags: FLAG_END_HEADERS,
                stream_id: 2,
                payload: Vec::new(),
            })
            .await
            .unwrap();
        framer
            .dispatch_frame(RawFrame {
                frame_type: FRAME_HEADERS,
                flags: FLAG_END_STREAM,
                stream_id: 2,
                payload: Vec::new(),
            })
            .await
            .unwrap();
        assert!(!framer.file_stream_candidates.contains(&2));
        assert!(framer.closed_streams.contains(&2));

        // A validated stream is removed from the live registry on END_STREAM.
        framer
            .dispatch_frame(RawFrame {
                frame_type: FRAME_DATA,
                flags: 0,
                stream_id: 4,
                payload: file_stream_preamble(8),
            })
            .await
            .unwrap();
        framer
            .dispatch_frame(RawFrame {
                frame_type: FRAME_DATA,
                flags: FLAG_END_STREAM,
                stream_id: 4,
                payload: b"tail".to_vec(),
            })
            .await
            .unwrap();
        assert!(!framer.announced_file_streams.contains(&4));
        assert!(framer.closed_streams.contains(&4));
        // END_STREAM makes the bytes eligible for a final read; consuming
        // them releases the per-stream and connection byte accounting.
        assert_eq!(
            framer
                .take_stream_bytes(4, FILE_STREAM_PREAMBLE_LEN + 4)
                .unwrap()
                .len(),
            28
        );

        // RST_STREAM clears buffered bytes and the adopted registry entry.
        framer
            .dispatch_frame(RawFrame {
                frame_type: FRAME_DATA,
                flags: 0,
                stream_id: 6,
                payload: file_stream_preamble(9),
            })
            .await
            .unwrap();
        framer
            .dispatch_frame(RawFrame {
                frame_type: FRAME_RST_STREAM,
                flags: 0,
                stream_id: 6,
                payload: 5u32.to_be_bytes().to_vec(),
            })
            .await
            .unwrap();
        assert!(!framer.announced_file_streams.contains(&6));
        assert!(!framer.stream_bufs.contains_key(&6));
        assert_eq!(framer.buffered_bytes, 0);
        assert_eq!(framer.reset_streams.get(&6), Some(&5));
    }

    #[tokio::test]
    async fn invalid_file_side_channel_preamble_is_a_protocol_error() {
        let (client, _server) = tokio::io::duplex(1024);
        let mut framer = test_framer(client);
        framer
            .dispatch_frame(RawFrame {
                frame_type: FRAME_HEADERS,
                flags: FLAG_END_HEADERS,
                stream_id: 2,
                payload: Vec::new(),
            })
            .await
            .unwrap();
        let error = framer
            .dispatch_frame(RawFrame {
                frame_type: FRAME_DATA,
                flags: 0,
                stream_id: 2,
                payload: vec![0u8; FILE_STREAM_PREAMBLE_LEN],
            })
            .await
            .unwrap_err();
        assert!(error
            .to_string()
            .contains("invalid RemoteXPC file-stream preamble"));
        assert!(!framer.file_stream_candidates.contains(&2));
        assert!(!framer.file_stream_candidate_data.contains_key(&2));

        // A malformed fragmented candidate must also release the first
        // fragment retained before the decoder can reject the full preamble.
        let (client, _server) = tokio::io::duplex(1024);
        let mut framer = test_framer(client);
        framer
            .dispatch_frame(RawFrame {
                frame_type: FRAME_HEADERS,
                flags: FLAG_END_HEADERS,
                stream_id: 4,
                payload: Vec::new(),
            })
            .await
            .unwrap();
        framer
            .dispatch_frame(RawFrame {
                frame_type: FRAME_DATA,
                flags: 0,
                stream_id: 4,
                payload: vec![0u8; 10],
            })
            .await
            .unwrap();
        let error = framer
            .dispatch_frame(RawFrame {
                frame_type: FRAME_DATA,
                flags: 0,
                stream_id: 4,
                payload: vec![0u8; 14],
            })
            .await
            .unwrap_err();
        assert!(error
            .to_string()
            .contains("invalid RemoteXPC file-stream preamble"));
        assert_eq!(framer.buffered_bytes, 0);
        assert!(!framer.stream_bufs.contains_key(&4));
        assert!(!framer.file_stream_candidates.contains(&4));
    }

    #[tokio::test]
    async fn per_stream_budget_error_clears_the_stream_buffer() {
        let (client, _server) = tokio::io::duplex(1024);
        let mut framer = test_framer(client);
        let payload = vec![0u8; MAX_BUFFERED_BYTES_PER_STREAM];

        framer
            .dispatch_frame(RawFrame {
                frame_type: FRAME_DATA,
                flags: 0,
                stream_id: 8,
                payload,
            })
            .await
            .unwrap();
        assert_eq!(framer.buffered_bytes, MAX_BUFFERED_BYTES_PER_STREAM);

        let error = framer
            .dispatch_frame(RawFrame {
                frame_type: FRAME_DATA,
                flags: 0,
                stream_id: 8,
                payload: vec![1],
            })
            .await
            .unwrap_err();
        assert!(error.to_string().contains("per-stream limit"));
        assert_eq!(framer.buffered_bytes, 0);
        assert!(!framer.stream_bufs.contains_key(&8));
    }

    #[tokio::test]
    async fn connection_buffer_budget_rejects_after_exact_total_and_releases_on_consume() {
        let (client, _server) = tokio::io::duplex(1024);
        let mut framer = test_framer(client);
        let stream_ids = [8, 10, 12, 14];

        for stream_id in stream_ids {
            framer
                .dispatch_frame(RawFrame {
                    frame_type: FRAME_DATA,
                    flags: 0,
                    stream_id,
                    payload: vec![0u8; MAX_BUFFERED_BYTES_PER_STREAM],
                })
                .await
                .unwrap();
        }
        assert_eq!(framer.buffered_bytes, MAX_TOTAL_BUFFERED_BYTES);

        let error = framer
            .dispatch_frame(RawFrame {
                frame_type: FRAME_DATA,
                flags: 0,
                stream_id: 16,
                payload: vec![1],
            })
            .await
            .unwrap_err();
        assert!(error.to_string().contains("connection limit"));
        assert_eq!(framer.buffered_bytes, MAX_TOTAL_BUFFERED_BYTES);

        for stream_id in stream_ids {
            framer
                .take_stream_bytes(stream_id, MAX_BUFFERED_BYTES_PER_STREAM)
                .unwrap();
        }
        assert_eq!(framer.buffered_bytes, 0);
    }
}
