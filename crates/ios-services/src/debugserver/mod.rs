//! Minimal debugserver transport helpers.
//!
//! Reference: go-ios/ios/debugserver/*

use semver::Version;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

pub const LEGACY_SERVICE_NAME: &str = "com.apple.debugserver";
pub const SECURE_SERVICE_NAME: &str = "com.apple.debugserver.DVTSecureSocketProxy";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedPacket {
    pub payload: String,
    pub consumed: usize,
}

#[derive(Debug, thiserror::Error)]
pub enum DebugserverError {
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
    #[error("invalid debugserver payload")]
    InvalidPayload,
    #[error("invalid UTF-8 payload: {0}")]
    Utf8(#[from] std::string::FromUtf8Error),
}

pub fn select_service_name(version: &Version) -> &'static str {
    if version.major >= 15 {
        SECURE_SERVICE_NAME
    } else {
        LEGACY_SERVICE_NAME
    }
}

pub fn checksum(payload: &str) -> String {
    format!(
        "{:02x}",
        payload
            .bytes()
            .fold(0u8, |acc, byte| acc.wrapping_add(byte))
    )
}

pub fn format_packet(payload: &str) -> String {
    format!("+${payload}#{}", checksum(payload))
}

pub fn parse_packet(data: &[u8]) -> Option<ParsedPacket> {
    const PACKET_SUFFIX_LEN: usize = 3; // "#xx"

    let start = data.iter().position(|&b| b == b'$')?;
    let end = data.iter().position(|&b| b == b'#')?;
    if end < start {
        return None;
    }
    if data.len() < end + PACKET_SUFFIX_LEN {
        return None;
    }

    let payload = String::from_utf8(data[start + 1..end].to_vec()).ok()?;
    Some(ParsedPacket {
        payload,
        consumed: end + PACKET_SUFFIX_LEN,
    })
}

pub struct GdbRemoteClient<S> {
    stream: S,
    read_buf: Vec<u8>,
}

impl<S> GdbRemoteClient<S>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    pub fn new(stream: S) -> Self {
        Self {
            stream,
            read_buf: Vec::with_capacity(4096),
        }
    }

    pub fn into_inner(self) -> S {
        self.stream
    }

    pub async fn send(&mut self, payload: &str) -> Result<(), DebugserverError> {
        self.stream
            .write_all(format_packet(payload).as_bytes())
            .await?;
        self.stream.flush().await?;
        Ok(())
    }

    pub async fn recv(&mut self) -> Result<String, DebugserverError> {
        let mut scratch = [0u8; 1024];
        loop {
            if let Some(packet) = parse_packet(&self.read_buf) {
                self.read_buf.drain(..packet.consumed);
                return Ok(packet.payload);
            }

            let read = self.stream.read(&mut scratch).await?;
            if read == 0 {
                return Err(DebugserverError::InvalidPayload);
            }
            self.read_buf.extend_from_slice(&scratch[..read]);
        }
    }

    pub async fn request(&mut self, payload: &str) -> Result<String, DebugserverError> {
        self.send(payload).await?;
        self.recv().await
    }
}
