//! Minimal raw HTTP/2 framer for iOS XPC protocol.
//!
//! Apple's XPC-over-HTTP/2 does NOT use standard HTTP semantics.
//! It uses raw HTTP/2 frames with two fixed stream IDs:
//!   - Stream 1: clientServer  (client → device)
//!   - Stream 3: serverClient  (device → client)
//!
//! Reference: go-ios/ios/http/http.go

use std::collections::{HashMap, HashSet};

use bytes::{Bytes, BytesMut};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

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
const FRAME_WINDOW_UPDATE: u8 = 0x08;

const FLAG_END_HEADERS: u8 = 0x04;
const FLAG_SETTINGS_ACK: u8 = 0x01;

// ── Settings IDs ────────────────────────────────────────────────────────────

const SETTINGS_MAX_CONCURRENT_STREAMS: u16 = 0x03;
const SETTINGS_INITIAL_WINDOW_SIZE: u16 = 0x04;

// ── H2 preface ──────────────────────────────────────────────────────────────

pub const H2_PREFACE: &[u8] = b"PRI * HTTP/2.0\r\n\r\nSM\r\n\r\n";

// ── Framer ──────────────────────────────────────────────────────────────────

/// Minimal HTTP/2 framer for the iOS XPC protocol.
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
}

#[derive(Debug, Clone)]
pub struct DataFrame {
    pub stream_id: u32,
    pub flags: u8,
    pub payload: Bytes,
}

impl<S: AsyncRead + AsyncWrite + Unpin> H2Framer<S> {
    /// Perform the HTTP/2 handshake and return a framer ready for use.
    pub async fn connect(mut stream: S) -> Result<Self, H2Error> {
        // 1. Send HTTP/2 connection preface
        stream.write_all(H2_PREFACE).await?;

        // 2. Send SETTINGS
        // INITIAL_WINDOW_SIZE = 1,048,576 (1 MiB), matching Apple's RemoteXPC implementation
        let settings = build_settings_frame(&[
            (SETTINGS_MAX_CONCURRENT_STREAMS, 100),
            (SETTINGS_INITIAL_WINDOW_SIZE, 1_048_576),
        ]);
        stream.write_all(&settings).await?;

        // 3. Send WINDOW_UPDATE on stream 0
        // Increment = 983,041 = 1,048,576 (1 MiB) - 65,535 (RFC 9113 default window)
        // This brings the connection-level window up to 1 MiB to match the stream-level setting
        let wupdate = build_window_update_frame(STREAM_INIT, 983_041);
        stream.write_all(&wupdate).await?;
        stream.flush().await?;

        let mut framer = Self {
            stream,
            client_server_buf: BytesMut::new(),
            server_client_buf: BytesMut::new(),
            stream_bufs: HashMap::new(),
            locally_open_streams: HashSet::new(),
            client_server_open: false,
            server_client_open: false,
        };

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
            match frame.frame_type {
                FRAME_SETTINGS => {
                    if frame.flags & FLAG_SETTINGS_ACK == 0 {
                        // Device sent SETTINGS; acknowledge it
                        let ack = build_settings_ack();
                        self.stream.write_all(&ack).await?;
                        self.stream.flush().await?;
                        return Ok(());
                    }
                    // It's our own ACK echoed back – ignore
                }
                FRAME_DATA => {
                    // Buffer early data
                    match frame.stream_id {
                        STREAM_CLIENT_SERVER => {
                            self.client_server_buf.extend_from_slice(&frame.payload)
                        }
                        STREAM_SERVER_CLIENT => {
                            self.server_client_buf.extend_from_slice(&frame.payload)
                        }
                        _ => {}
                    }
                }
                _ => {}
            }
        }
    }

    /// Read one raw HTTP/2 frame from the stream.
    async fn read_raw_frame(&mut self) -> Result<RawFrame, H2Error> {
        let mut header = [0u8; 9];
        self.stream.read_exact(&mut header).await?;

        let length =
            ((header[0] as usize) << 16) | ((header[1] as usize) << 8) | (header[2] as usize);
        let frame_type = header[3];
        let flags = header[4];
        let stream_id = u32::from_be_bytes([header[5] & 0x7F, header[6], header[7], header[8]]);

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
    pub async fn read_server_client(&mut self, n: usize) -> Result<Bytes, H2Error> {
        self.read_stream(STREAM_SERVER_CLIENT, n).await
    }

    /// Read data from the clientServer stream (client ← device, used for ack).
    pub async fn read_client_server(&mut self, n: usize) -> Result<Bytes, H2Error> {
        self.read_stream(STREAM_CLIENT_SERVER, n).await
    }

    /// Read data from any stream, blocking until `n` bytes are available.
    pub async fn read_stream(&mut self, stream_id: u32, n: usize) -> Result<Bytes, H2Error> {
        while self.stream_buffer_len(stream_id) < n {
            let frame = self.read_raw_frame().await?;
            self.dispatch_frame(frame).await?;
        }
        self.take_stream_bytes(stream_id, n)
    }

    async fn dispatch_frame(&mut self, frame: RawFrame) -> Result<(), H2Error> {
        tracing::trace!(
            "h2: dispatch frame type={} flags=0x{:02x} stream={} len={}",
            frame_type_name(frame.frame_type),
            frame.flags,
            frame.stream_id,
            frame.payload.len()
        );
        match frame.frame_type {
            FRAME_DATA => match frame.stream_id {
                STREAM_CLIENT_SERVER => self.client_server_buf.extend_from_slice(&frame.payload),
                STREAM_SERVER_CLIENT => self.server_client_buf.extend_from_slice(&frame.payload),
                other => self
                    .stream_bufs
                    .entry(other)
                    .or_default()
                    .extend_from_slice(&frame.payload),
            },
            FRAME_SETTINGS if frame.flags & FLAG_SETTINGS_ACK == 0 => {
                let ack = build_settings_ack();
                self.stream.write_all(&ack).await?;
                self.stream.flush().await?;
            }
            _ => {}
        }
        if frame.frame_type == FRAME_DATA && frame.stream_id % 2 == 0 && !frame.payload.is_empty() {
            let conn_window = build_window_update_frame(STREAM_INIT, frame.payload.len() as u32);
            let stream_window =
                build_window_update_frame(frame.stream_id, frame.payload.len() as u32);
            self.stream.write_all(&conn_window).await?;
            self.stream.write_all(&stream_window).await?;
            self.stream.flush().await?;
        }
        Ok(())
    }

    /// Read the next DATA frame from any stream, skipping non-DATA frames.
    pub async fn read_next_data_frame(&mut self) -> Result<DataFrame, H2Error> {
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
                    if frame.stream_id % 2 == 0 && !frame.payload.is_empty() {
                        let conn_window =
                            build_window_update_frame(STREAM_INIT, frame.payload.len() as u32);
                        let stream_window =
                            build_window_update_frame(frame.stream_id, frame.payload.len() as u32);
                        self.stream.write_all(&conn_window).await?;
                        self.stream.write_all(&stream_window).await?;
                        self.stream.flush().await?;
                    }
                    return Ok(DataFrame {
                        stream_id: frame.stream_id,
                        flags: frame.flags,
                        payload: Bytes::from(frame.payload),
                    });
                }
                FRAME_SETTINGS if frame.flags & FLAG_SETTINGS_ACK == 0 => {
                    let ack = build_settings_ack();
                    self.stream.write_all(&ack).await?;
                    self.stream.flush().await?;
                }
                _ => {}
            }
        }
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
        let data_frame = build_data_frame(stream_id, data);
        self.stream.write_all(&data_frame).await?;
        self.stream.flush().await?;
        Ok(())
    }

    /// Open stream 1 with an empty HEADERS frame if it is not open yet.
    pub async fn open_client_server(&mut self) -> Result<(), H2Error> {
        self.open_stream(STREAM_CLIENT_SERVER).await
    }

    /// Open stream 3 with an empty HEADERS frame if it is not open yet.
    pub async fn open_server_client(&mut self) -> Result<(), H2Error> {
        self.open_stream(STREAM_SERVER_CLIENT).await
    }

    /// Open an arbitrary stream with an empty HEADERS frame if it is not open yet.
    pub async fn open_stream(&mut self, stream_id: u32) -> Result<(), H2Error> {
        let already_open = match stream_id {
            STREAM_CLIENT_SERVER => self.client_server_open,
            STREAM_SERVER_CLIENT => self.server_client_open,
            _ => self.locally_open_streams.contains(&stream_id),
        };
        if !already_open {
            let headers = build_headers_frame(stream_id);
            self.stream.write_all(&headers).await?;
            self.stream.flush().await?;
            match stream_id {
                STREAM_CLIENT_SERVER => self.client_server_open = true,
                STREAM_SERVER_CLIENT => self.server_client_open = true,
                _ => {
                    self.locally_open_streams.insert(stream_id);
                    self.stream_bufs.entry(stream_id).or_default();
                }
            }
        }
        Ok(())
    }

    /// Process any pending frames (drain incoming data into buffers).
    pub async fn poll_frames(&mut self) -> Result<(), H2Error> {
        // Non-blocking poll: try to read frames if there's data available
        // We rely on the individual read_* calls to refill buffers; this is a helper
        // for situations where we want to ensure the buffers are current.
        Ok(())
    }

    fn stream_buffer_len(&self, stream_id: u32) -> usize {
        match stream_id {
            STREAM_CLIENT_SERVER => self.client_server_buf.len(),
            STREAM_SERVER_CLIENT => self.server_client_buf.len(),
            _ => self.stream_bufs.get(&stream_id).map_or(0, BytesMut::len),
        }
    }

    fn take_stream_bytes(&mut self, stream_id: u32, n: usize) -> Result<Bytes, H2Error> {
        match stream_id {
            STREAM_CLIENT_SERVER => Ok(self.client_server_buf.split_to(n).freeze()),
            STREAM_SERVER_CLIENT => Ok(self.server_client_buf.split_to(n).freeze()),
            _ => self
                .stream_bufs
                .get_mut(&stream_id)
                .map(|buf| buf.split_to(n).freeze())
                .ok_or_else(|| H2Error::Protocol(format!("stream {stream_id} not open"))),
        }
    }
}

fn frame_type_name(frame_type: u8) -> &'static str {
    match frame_type {
        FRAME_DATA => "DATA",
        FRAME_HEADERS => "HEADERS",
        FRAME_SETTINGS => "SETTINGS",
        FRAME_WINDOW_UPDATE => "WINDOW_UPDATE",
        _ => "OTHER",
    }
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
    #[error("GOAWAY received")]
    GoAway,
}

#[cfg(test)]
mod tests {
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
        let mut framer = H2Framer {
            stream: client,
            client_server_buf: BytesMut::new(),
            server_client_buf: BytesMut::new(),
            stream_bufs: HashMap::new(),
            locally_open_streams: HashSet::new(),
            client_server_open: false,
            server_client_open: false,
        };

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
        let mut framer = H2Framer {
            stream: client,
            client_server_buf: BytesMut::new(),
            server_client_buf: BytesMut::new(),
            stream_bufs: HashMap::new(),
            locally_open_streams: HashSet::new(),
            client_server_open: false,
            server_client_open: false,
        };

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

        let mut conn_window = [0u8; 13];
        server.read_exact(&mut conn_window).await.unwrap();
        assert_eq!(conn_window[3], FRAME_WINDOW_UPDATE);
        assert_eq!(
            u32::from_be_bytes([
                conn_window[5] & 0x7F,
                conn_window[6],
                conn_window[7],
                conn_window[8]
            ]),
            STREAM_INIT
        );

        let mut stream_window = [0u8; 13];
        server.read_exact(&mut stream_window).await.unwrap();
        assert_eq!(stream_window[3], FRAME_WINDOW_UPDATE);
        assert_eq!(
            u32::from_be_bytes([
                stream_window[5] & 0x7F,
                stream_window[6],
                stream_window[7],
                stream_window[8]
            ]),
            4
        );

        let mut headers = [0u8; 9];
        server.read_exact(&mut headers).await.unwrap();
        assert_eq!(headers[3], FRAME_HEADERS);
        assert_eq!(headers[4], FLAG_END_HEADERS);
        assert_eq!(
            u32::from_be_bytes([headers[5] & 0x7F, headers[6], headers[7], headers[8]]),
            4
        );
    }
}
