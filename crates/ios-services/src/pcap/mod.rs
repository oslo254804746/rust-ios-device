//! Minimal packet capture client for `com.apple.pcapd`.
//!
//! The service sends lockdown plist frames whose payload is a `Data` blob containing
//! an iOS-specific packet header followed by the captured packet bytes.

use plist::Value;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite};

pub const SERVICE_NAME: &str = "com.apple.pcapd";
const DEFAULT_HEADER_SIZE: usize = 95;
const FAKE_ETHERNET_HEADER: [u8; 14] = [
    0xbe, 0xfe, 0xbe, 0xfe, 0xbe, 0xfe, 0xbe, 0xfe, 0xbe, 0xfe, 0xbe, 0xfe, 0x08, 0x00,
];

#[derive(Debug, thiserror::Error)]
pub enum PcapError {
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
    #[error("plist error: {0}")]
    Plist(String),
    #[error("protocol error: {0}")]
    Protocol(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CapturedPacket {
    pub ts_sec: u32,
    pub ts_usec: u32,
    pub interface_name: String,
    pub pid: i32,
    pub pid2: i32,
    pub proc_name: String,
    pub proc_name2: String,
    pub payload: Vec<u8>,
}

pub struct PcapClient<S> {
    stream: S,
}

impl<S: AsyncRead + AsyncWrite + Unpin> PcapClient<S> {
    pub fn new(stream: S) -> Self {
        Self { stream }
    }

    pub async fn next_packet(&mut self) -> Result<CapturedPacket, PcapError> {
        let mut len_buf = [0u8; 4];
        self.stream.read_exact(&mut len_buf).await?;
        let len = u32::from_be_bytes(len_buf) as usize;
        const MAX_PLIST_SIZE: usize = 4 * 1024 * 1024;
        if len > MAX_PLIST_SIZE {
            return Err(PcapError::Protocol(format!(
                "plist length {len} exceeds maximum of {MAX_PLIST_SIZE}"
            )));
        }

        let mut buf = vec![0u8; len];
        self.stream.read_exact(&mut buf).await?;
        let payload = plist::from_bytes::<Value>(&buf)
            .map_err(|e| PcapError::Plist(e.to_string()))?
            .into_data()
            .ok_or_else(|| PcapError::Protocol("pcap plist payload was not data".into()))?;

        decode_packet(&payload)
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PacketFilter {
    pub pid: Option<i32>,
    pub process_prefix: Option<String>,
}

impl PacketFilter {
    pub fn matches(&self, packet: &CapturedPacket) -> bool {
        if let Some(pid) = self.pid {
            if packet.pid != pid && packet.pid2 != pid {
                return false;
            }
        }

        if let Some(prefix) = &self.process_prefix {
            if !packet.proc_name.starts_with(prefix) && !packet.proc_name2.starts_with(prefix) {
                return false;
            }
        }

        true
    }
}

pub fn write_global_header<W: std::io::Write>(writer: &mut W) -> Result<(), PcapError> {
    writer.write_all(&[
        0xd4, 0xc3, 0xb2, 0xa1, 0x02, 0x00, 0x04, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
        0x00, 0xff, 0xff, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00,
    ])?;
    Ok(())
}

pub fn write_packet_record<W: std::io::Write>(
    writer: &mut W,
    packet: &CapturedPacket,
) -> Result<(), PcapError> {
    let length = packet.payload.len() as u32;
    writer.write_all(&packet.ts_sec.to_le_bytes())?;
    writer.write_all(&packet.ts_usec.to_le_bytes())?;
    writer.write_all(&length.to_le_bytes())?;
    writer.write_all(&length.to_le_bytes())?;
    writer.write_all(&packet.payload)?;
    Ok(())
}

fn decode_packet(buf: &[u8]) -> Result<CapturedPacket, PcapError> {
    if buf.len() < DEFAULT_HEADER_SIZE {
        return Err(PcapError::Protocol(format!(
            "pcap frame too short for header: {}",
            buf.len()
        )));
    }

    let hdr_size = be_u32(buf, 0)? as usize;
    if hdr_size < DEFAULT_HEADER_SIZE {
        return Err(PcapError::Protocol(format!(
            "pcap header too small: {hdr_size}"
        )));
    }
    if buf.len() < hdr_size {
        return Err(PcapError::Protocol(format!(
            "pcap frame shorter than header size: {} < {hdr_size}",
            buf.len()
        )));
    }

    let frame_pre_length = be_u32(buf, 17)?;
    let interface_name = parse_fixed_string(buf, 25, 16)?;
    let pid = le_i32(buf, 41)?;
    let proc_name = parse_fixed_string(buf, 45, 17)?;
    let pid2 = le_i32(buf, 66)?;
    let proc_name2 = parse_fixed_string(buf, 70, 17)?;
    let ts_sec = be_u32(buf, 87)?;
    let ts_usec = be_u32(buf, 91)?;

    let payload = &buf[hdr_size..];
    let payload = if frame_pre_length == 0 {
        let mut packet = Vec::with_capacity(FAKE_ETHERNET_HEADER.len() + payload.len());
        packet.extend_from_slice(&FAKE_ETHERNET_HEADER);
        packet.extend_from_slice(payload);
        packet
    } else {
        payload.to_vec()
    };

    Ok(CapturedPacket {
        ts_sec,
        ts_usec,
        interface_name,
        pid,
        pid2,
        proc_name,
        proc_name2,
        payload,
    })
}

fn be_u32(buf: &[u8], offset: usize) -> Result<u32, PcapError> {
    let bytes = buf
        .get(offset..offset + 4)
        .ok_or_else(|| PcapError::Protocol(format!("missing u32 at offset {offset}")))?;
    // Safety: .get(offset..offset+4) returns exactly 4 bytes, so try_into::<[u8; 4]>() is infallible.
    Ok(u32::from_be_bytes(bytes.try_into().unwrap()))
}

fn le_i32(buf: &[u8], offset: usize) -> Result<i32, PcapError> {
    let bytes = buf
        .get(offset..offset + 4)
        .ok_or_else(|| PcapError::Protocol(format!("missing i32 at offset {offset}")))?;
    // Safety: .get(offset..offset+4) returns exactly 4 bytes, so try_into::<[u8; 4]>() is infallible.
    Ok(i32::from_le_bytes(bytes.try_into().unwrap()))
}

fn parse_fixed_string(buf: &[u8], offset: usize, len: usize) -> Result<String, PcapError> {
    let bytes = buf
        .get(offset..offset + len)
        .ok_or_else(|| PcapError::Protocol(format!("missing string at offset {offset}")))?;
    let trimmed = bytes
        .iter()
        .copied()
        .take_while(|byte| *byte != 0)
        .collect::<Vec<_>>();
    String::from_utf8(trimmed).map_err(|e| PcapError::Protocol(format!("invalid string: {e}")))
}

#[cfg(test)]
mod tests {
    use std::pin::Pin;
    use std::task::{Context, Poll};

    use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};

    use super::*;

    #[derive(Default)]
    struct MockStream {
        read_buf: Vec<u8>,
        read_pos: usize,
    }

    impl MockStream {
        fn with_packet_data(data: Vec<u8>) -> Self {
            let mut plist_buf = Vec::new();
            plist::to_writer_xml(&mut plist_buf, &Value::Data(data)).unwrap();
            let mut read_buf = Vec::new();
            read_buf.extend_from_slice(&(plist_buf.len() as u32).to_be_bytes());
            read_buf.extend_from_slice(&plist_buf);
            Self {
                read_buf,
                read_pos: 0,
            }
        }
    }

    impl AsyncRead for MockStream {
        fn poll_read(
            mut self: Pin<&mut Self>,
            _cx: &mut Context<'_>,
            buf: &mut ReadBuf<'_>,
        ) -> Poll<std::io::Result<()>> {
            let remaining = self.read_buf.len().saturating_sub(self.read_pos);
            if remaining == 0 {
                return Poll::Ready(Err(std::io::Error::new(
                    std::io::ErrorKind::UnexpectedEof,
                    "no more test data",
                )));
            }
            let to_copy = remaining.min(buf.remaining());
            let start = self.read_pos;
            let end = start + to_copy;
            buf.put_slice(&self.read_buf[start..end]);
            self.read_pos = end;
            Poll::Ready(Ok(()))
        }
    }

    impl AsyncWrite for MockStream {
        fn poll_write(
            self: Pin<&mut Self>,
            _cx: &mut Context<'_>,
            buf: &[u8],
        ) -> Poll<std::io::Result<usize>> {
            Poll::Ready(Ok(buf.len()))
        }

        fn poll_flush(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
            Poll::Ready(Ok(()))
        }

        fn poll_shutdown(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
            Poll::Ready(Ok(()))
        }
    }

    fn sample_header(frame_pre_length: u32) -> Vec<u8> {
        let mut buf = vec![0u8; DEFAULT_HEADER_SIZE];
        buf[0..4].copy_from_slice(&(DEFAULT_HEADER_SIZE as u32).to_be_bytes());
        buf[17..21].copy_from_slice(&frame_pre_length.to_be_bytes());
        buf[25..29].copy_from_slice(b"en0\0");
        buf[41..45].copy_from_slice(&1234i32.to_le_bytes());
        buf[45..52].copy_from_slice(b"Safari\0");
        buf[66..70].copy_from_slice(&4321i32.to_le_bytes());
        buf[70..77].copy_from_slice(b"WebKit\0");
        buf[87..91].copy_from_slice(&123u32.to_be_bytes());
        buf[91..95].copy_from_slice(&456u32.to_be_bytes());
        buf
    }

    #[test]
    fn decode_packet_prepends_fake_ethernet_header_for_ip_payloads() {
        let mut raw = sample_header(0);
        raw.extend_from_slice(&[0x45, 0x00, 0x00, 0x14]);

        let packet = decode_packet(&raw).unwrap();
        assert_eq!(packet.ts_sec, 123);
        assert_eq!(packet.ts_usec, 456);
        assert_eq!(packet.interface_name, "en0");
        assert_eq!(packet.pid, 1234);
        assert_eq!(packet.proc_name, "Safari");
        assert_eq!(&packet.payload[..14], &FAKE_ETHERNET_HEADER);
        assert_eq!(&packet.payload[14..], &[0x45, 0x00, 0x00, 0x14]);
    }

    #[tokio::test]
    async fn next_packet_roundtrips_plist_data_frame() {
        let mut raw = sample_header(14);
        raw.extend_from_slice(&[0xaa, 0xbb, 0xcc, 0xdd]);
        let stream = MockStream::with_packet_data(raw);
        let mut client = PcapClient::new(stream);

        let packet = client.next_packet().await.unwrap();
        assert_eq!(packet.ts_sec, 123);
        assert_eq!(packet.ts_usec, 456);
        assert_eq!(packet.pid2, 4321);
        assert_eq!(packet.proc_name2, "WebKit");
        assert_eq!(packet.payload, vec![0xaa, 0xbb, 0xcc, 0xdd]);
    }

    #[test]
    fn write_global_header_writes_standard_pcap_magic() {
        let mut buf = Vec::new();
        write_global_header(&mut buf).unwrap();
        assert_eq!(&buf[..4], &[0xd4, 0xc3, 0xb2, 0xa1]);
        assert_eq!(buf.len(), 24);
    }

    #[test]
    fn packet_filter_matches_on_either_pid_field() {
        let packet = CapturedPacket {
            ts_sec: 0,
            ts_usec: 0,
            interface_name: "en0".into(),
            pid: 111,
            pid2: 222,
            proc_name: "Safari".into(),
            proc_name2: "WebKit".into(),
            payload: vec![1, 2, 3],
        };

        assert!(PacketFilter {
            pid: Some(111),
            process_prefix: None
        }
        .matches(&packet));
        assert!(PacketFilter {
            pid: Some(222),
            process_prefix: None
        }
        .matches(&packet));
        assert!(!PacketFilter {
            pid: Some(333),
            process_prefix: None
        }
        .matches(&packet));
    }

    #[test]
    fn packet_filter_matches_on_either_process_name_field() {
        let packet = CapturedPacket {
            ts_sec: 0,
            ts_usec: 0,
            interface_name: "en0".into(),
            pid: 111,
            pid2: 222,
            proc_name: "Safari".into(),
            proc_name2: "WebKit.Networking".into(),
            payload: vec![1, 2, 3],
        };

        assert!(PacketFilter {
            pid: None,
            process_prefix: Some("Saf".into())
        }
        .matches(&packet));
        assert!(PacketFilter {
            pid: None,
            process_prefix: Some("WebKit".into())
        }
        .matches(&packet));
        assert!(!PacketFilter {
            pid: None,
            process_prefix: Some("SpringBoard".into())
        }
        .matches(&packet));
    }
}
