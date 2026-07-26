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
    #[error("packet checksum mismatch: device sent {expected:#04x}, computed {computed:#04x}")]
    Checksum { expected: u8, computed: u8 },
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
    format!("{:02x}", checksum_byte(payload.as_bytes()))
}

pub fn format_packet(payload: &str) -> String {
    format!("+${payload}#{}", checksum(payload))
}

/// Parse the first complete packet in `data`, ignoring damaged frames.
///
/// Prefer [`try_parse_packet`] on a transport: a `None` here cannot be told apart
/// from "the buffer does not hold a whole frame yet".
pub fn parse_packet(data: &[u8]) -> Option<ParsedPacket> {
    try_parse_packet(data).ok().flatten()
}

/// Parse the first complete packet in `data`.
///
/// `Ok(None)` means the frame is still incomplete and the caller should read more
/// bytes; an error means the frame arrived damaged and must not be handed on as if
/// it were the device's reply.
pub fn try_parse_packet(data: &[u8]) -> Result<Option<ParsedPacket>, DebugserverError> {
    const PACKET_SUFFIX_LEN: usize = 3; // "#xx"

    let Some(start) = data.iter().position(|&b| b == b'$') else {
        return Ok(None);
    };
    // Both the escape and the run-length encodings keep '#' out of the body, so the
    // first '#' in the buffer always terminates the frame.
    let Some(end) = data.iter().position(|&b| b == b'#') else {
        return Ok(None);
    };
    if end < start {
        return Ok(None);
    }
    if data.len() < end + PACKET_SUFFIX_LEN {
        return Ok(None);
    }

    let body = &data[start + 1..end];
    let expected = parse_hex_byte(&data[end + 1..end + PACKET_SUFFIX_LEN])
        .ok_or(DebugserverError::InvalidPayload)?;
    // The checksum covers the body as transmitted, i.e. before escapes and run-length
    // runs are expanded.
    let computed = checksum_byte(body);
    if computed != expected {
        return Err(DebugserverError::Checksum { expected, computed });
    }

    let payload = String::from_utf8(decode_body(body))?;
    Ok(Some(ParsedPacket {
        payload,
        consumed: end + PACKET_SUFFIX_LEN,
    }))
}

fn checksum_byte(payload: &[u8]) -> u8 {
    payload
        .iter()
        .fold(0u8, |acc, &byte| acc.wrapping_add(byte))
}

fn parse_hex_byte(digits: &[u8]) -> Option<u8> {
    fn nibble(byte: u8) -> Option<u8> {
        match byte {
            b'0'..=b'9' => Some(byte - b'0'),
            b'a'..=b'f' => Some(byte - b'a' + 10),
            b'A'..=b'F' => Some(byte - b'A' + 10),
            _ => None,
        }
    }

    match digits {
        [high, low] => Some((nibble(*high)? << 4) | nibble(*low)?),
        _ => None,
    }
}

/// Expand a packet body: `}` escapes and `*` run-length runs.
///
/// Both are decoded in one left-to-right pass because they interleave: a repeat
/// count is an arbitrary printable byte and may itself be `}`, so it has to be
/// consumed as a count rather than re-read as an escape prefix.
fn decode_body(body: &[u8]) -> Vec<u8> {
    const ESCAPE: u8 = b'}';
    const REPEAT: u8 = b'*';
    // Repeat counts are biased so they stay printable on the wire.
    const REPEAT_BIAS: u8 = 29;

    let mut out = Vec::with_capacity(body.len());
    let mut bytes = body.iter().copied();
    while let Some(byte) = bytes.next() {
        match byte {
            // The escape flips 0x20 so '#', '$', '*' and '}' can travel inside a
            // payload without breaking the framing.
            ESCAPE => match bytes.next() {
                Some(escaped) => out.push(escaped ^ 0x20),
                None => out.push(ESCAPE),
            },
            REPEAT => match (out.last().copied(), bytes.next()) {
                // The count byte encodes how many *extra* copies of the preceding byte
                // follow.
                (Some(previous), Some(count)) => {
                    let extra = count.saturating_sub(REPEAT_BIAS) as usize;
                    out.resize(out.len() + extra, previous);
                }
                // Malformed run (nothing to repeat, or no count byte). The frame already
                // passed its checksum, so keep the bytes instead of dropping them.
                (_, count) => {
                    out.push(REPEAT);
                    out.extend(count);
                }
            },
            other => out.push(other),
        }
    }
    out
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
        const ACK: u8 = b'+';
        const NACK: u8 = b'-';

        let mut scratch = [0u8; 1024];
        loop {
            let parsed = try_parse_packet(&self.read_buf);
            match parsed {
                Ok(Some(packet)) => {
                    self.read_buf.drain(..packet.consumed);
                    // This client never negotiates QStartNoAckMode, so acks stay on for
                    // the whole session and the device waits for one before sending
                    // anything else. `format_packet`'s leading '+' only arrives with the
                    // *next* request, which is too late for anything but a strict
                    // request/response exchange.
                    self.write_ack(ACK).await?;
                    return Ok(packet.payload);
                }
                Ok(None) => {}
                Err(err) => {
                    // A nak is what the protocol expects for a damaged frame, so the
                    // device is not left waiting on an ack. The call still fails: this
                    // client keeps no retransmission state, and looping on a device that
                    // never resends would hang the caller.
                    self.read_buf.clear();
                    self.write_ack(NACK).await?;
                    return Err(err);
                }
            }

            let read = self.stream.read(&mut scratch).await?;
            if read == 0 {
                return Err(DebugserverError::InvalidPayload);
            }
            self.read_buf.extend_from_slice(&scratch[..read]);
        }
    }

    async fn write_ack(&mut self, ack: u8) -> Result<(), DebugserverError> {
        self.stream.write_all(&[ack]).await?;
        self.stream.flush().await?;
        Ok(())
    }

    pub async fn request(&mut self, payload: &str) -> Result<String, DebugserverError> {
        self.send(payload).await?;
        self.recv().await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Frame a body exactly as the device would, checksum included.
    fn framed(body: &[u8]) -> Vec<u8> {
        let mut packet = vec![b'$'];
        packet.extend_from_slice(body);
        packet.extend_from_slice(format!("#{:02x}", checksum_byte(body)).as_bytes());
        packet
    }

    fn parsed(body: &[u8]) -> String {
        try_parse_packet(&framed(body))
            .expect("packet should be well formed")
            .expect("packet should be complete")
            .payload
    }

    #[test]
    fn rejects_a_corrupted_checksum() {
        let err = try_parse_packet(b"$OK#00").unwrap_err();
        assert!(
            matches!(
                err,
                DebugserverError::Checksum {
                    expected: 0x00,
                    computed: 0x9a
                }
            ),
            "unexpected error: {err}"
        );
        assert!(parse_packet(b"$OK#00").is_none());
    }

    #[test]
    fn reports_an_incomplete_frame_without_an_error() {
        assert!(try_parse_packet(b"").unwrap().is_none());
        assert!(try_parse_packet(b"$OK#9").unwrap().is_none());
    }

    #[test]
    fn unescapes_bytes_that_would_break_framing() {
        // '#', '$', '*' and '}' can only appear in a payload as `}` + byte ^ 0x20.
        assert_eq!(parsed(b"}\x03}\x04}\x0a}]"), "#$*}");
    }

    #[test]
    fn expands_run_length_encoded_runs() {
        // The GDB spec's own example: '0' plus a count of 0x20 is four zeros.
        assert_eq!(parsed(b"0* "), "0000");
    }

    #[test]
    fn treats_a_repeat_count_as_a_count_not_an_escape() {
        // '}' is a legal repeat count (125 - 29 = 96 extra copies); reading it as an
        // escape prefix instead would swallow the rest of the payload.
        assert_eq!(parsed(b"z*}"), "z".repeat(97));
    }

    #[test]
    fn repeats_an_escaped_byte() {
        assert_eq!(parsed(b"}]*!"), "}}}}}");
    }

    #[tokio::test]
    async fn recv_acks_an_accepted_packet() {
        let (mut device, host) = tokio::io::duplex(64);
        device.write_all(b"$OK#9a").await.unwrap();

        let mut client = GdbRemoteClient::new(host);
        assert_eq!(client.recv().await.unwrap(), "OK");

        let mut ack = [0u8; 1];
        device.read_exact(&mut ack).await.unwrap();
        assert_eq!(&ack, b"+");
    }

    #[tokio::test]
    async fn recv_naks_a_damaged_packet() {
        let (mut device, host) = tokio::io::duplex(64);
        device.write_all(b"$OK#00").await.unwrap();

        let mut client = GdbRemoteClient::new(host);
        let err = client.recv().await.unwrap_err();
        assert!(
            matches!(err, DebugserverError::Checksum { .. }),
            "unexpected error: {err}"
        );

        let mut nack = [0u8; 1];
        device.read_exact(&mut nack).await.unwrap();
        assert_eq!(&nack, b"-");
    }
}
