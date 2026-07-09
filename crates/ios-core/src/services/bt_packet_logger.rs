//! Bluetooth HCI packet capture via `com.apple.bluetooth.BTPacketLogger`.
//!
//! Reference: pymobiledevice3 `services/bt_packet_logger.py`.

use std::io::Write;

use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite};

pub const SERVICE_NAME: &str = "com.apple.bluetooth.BTPacketLogger";

const PACKETLOGGER_RECORD_HEADER_SIZE: usize = 13;
const SERVICE_PACKET_SIZE_HEADER: usize = 2;
const MAX_PACKETLOGGER_RECORD_SIZE: usize = 16 * 1024 * 1024;
pub const PCAPNG_LINKTYPE_BLUETOOTH_HCI_H4_WITH_PHDR: u16 = 201;

service_error!(BtPacketLoggerError);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum PacketLoggerPacketType {
    HciCommand = 0x00,
    HciEvent = 0x01,
    SentAclData = 0x02,
    RecvAclData = 0x03,
    SentScoData = 0x08,
    RecvScoData = 0x09,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PacketLoggerRecord {
    pub length: u32,
    pub seconds: u32,
    pub microseconds: u32,
    pub packet_type: u8,
    pub payload: Vec<u8>,
}

pub struct BtPacketLoggerClient<S> {
    stream: S,
}

impl<S> BtPacketLoggerClient<S> {
    pub fn new(stream: S) -> Self {
        Self { stream }
    }
}

impl<S: AsyncRead + AsyncWrite + Unpin> BtPacketLoggerClient<S> {
    pub async fn next_packetlogger_record(&mut self) -> Result<Vec<u8>, BtPacketLoggerError> {
        loop {
            let mut len = [0u8; SERVICE_PACKET_SIZE_HEADER];
            self.stream.read_exact(&mut len).await?;
            let len = u16::from_le_bytes(len) as usize;
            if len == 0 {
                continue;
            }
            if len > MAX_PACKETLOGGER_RECORD_SIZE {
                return Err(BtPacketLoggerError::Protocol(format!(
                    "BTPacketLogger record length {len} exceeds maximum {MAX_PACKETLOGGER_RECORD_SIZE}"
                )));
            }
            let mut record = vec![0u8; len];
            self.stream.read_exact(&mut record).await?;
            return Ok(record);
        }
    }

    pub async fn next_record(&mut self) -> Result<PacketLoggerRecord, BtPacketLoggerError> {
        let raw = self.next_packetlogger_record().await?;
        parse_packetlogger_record(&raw)
    }
}

pub fn parse_packetlogger_record(data: &[u8]) -> Result<PacketLoggerRecord, BtPacketLoggerError> {
    if data.len() < PACKETLOGGER_RECORD_HEADER_SIZE {
        return Err(BtPacketLoggerError::Protocol(format!(
            "packet logger record too short: {} bytes",
            data.len()
        )));
    }

    Ok(PacketLoggerRecord {
        length: u32::from_be_bytes(data[0..4].try_into().unwrap()),
        seconds: u32::from_be_bytes(data[4..8].try_into().unwrap()),
        microseconds: u32::from_be_bytes(data[8..12].try_into().unwrap()),
        packet_type: data[12],
        payload: data[PACKETLOGGER_RECORD_HEADER_SIZE..].to_vec(),
    })
}

pub fn packetlogger_record_to_hci_h4_phdr(record: &PacketLoggerRecord) -> Option<Vec<u8>> {
    let (hci_h4_type, direction_in) = match record.packet_type {
        value if value == PacketLoggerPacketType::HciCommand as u8 => (0x01, 0u32),
        value if value == PacketLoggerPacketType::SentAclData as u8 => (0x02, 0u32),
        value if value == PacketLoggerPacketType::RecvAclData as u8 => (0x02, 1u32),
        value if value == PacketLoggerPacketType::SentScoData as u8 => (0x03, 0u32),
        value if value == PacketLoggerPacketType::RecvScoData as u8 => (0x03, 1u32),
        value if value == PacketLoggerPacketType::HciEvent as u8 => (0x04, 1u32),
        _ => return None,
    };

    let mut frame = Vec::with_capacity(5 + record.payload.len());
    frame.extend_from_slice(&direction_in.to_be_bytes());
    frame.push(hci_h4_type);
    frame.extend_from_slice(&record.payload);
    Some(frame)
}

pub fn write_packetlogger_record<W: Write>(
    writer: &mut W,
    raw_record: &[u8],
) -> Result<(), BtPacketLoggerError> {
    writer.write_all(raw_record)?;
    writer.flush()?;
    Ok(())
}

pub fn write_pcapng_header<W: Write>(writer: &mut W) -> Result<(), BtPacketLoggerError> {
    let mut section = Vec::new();
    section.extend_from_slice(&0x1A2B3C4Du32.to_le_bytes());
    section.extend_from_slice(&1u16.to_le_bytes());
    section.extend_from_slice(&0u16.to_le_bytes());
    section.extend_from_slice(&(-1i64).to_le_bytes());
    write_options_end(&mut section);
    write_pcapng_block(writer, 0x0A0D0D0A, &section)?;

    let mut interface = Vec::new();
    interface.extend_from_slice(&PCAPNG_LINKTYPE_BLUETOOTH_HCI_H4_WITH_PHDR.to_le_bytes());
    interface.extend_from_slice(&0u16.to_le_bytes());
    interface.extend_from_slice(&u32::MAX.to_le_bytes());
    write_options_end(&mut interface);
    write_pcapng_block(writer, 0x00000001, &interface)?;
    Ok(())
}

pub fn write_pcapng_record<W: Write>(
    writer: &mut W,
    record: &PacketLoggerRecord,
    tz_offset_seconds: f64,
) -> Result<bool, BtPacketLoggerError> {
    let Some(frame) = packetlogger_record_to_hci_h4_phdr(record) else {
        return Ok(false);
    };
    let timestamp_microseconds = packetlogger_timestamp_microseconds(record, tz_offset_seconds);
    let captured_len = u32::try_from(frame.len()).map_err(|_| {
        BtPacketLoggerError::Protocol(format!("pcapng frame too large: {}", frame.len()))
    })?;

    let mut packet = Vec::new();
    packet.extend_from_slice(&0u32.to_le_bytes());
    packet.extend_from_slice(&((timestamp_microseconds >> 32) as u32).to_le_bytes());
    packet.extend_from_slice(&(timestamp_microseconds as u32).to_le_bytes());
    packet.extend_from_slice(&captured_len.to_le_bytes());
    packet.extend_from_slice(&captured_len.to_le_bytes());
    packet.extend_from_slice(&frame);
    pad_to_32bit(&mut packet);
    write_options_end(&mut packet);
    write_pcapng_block(writer, 0x00000006, &packet)?;
    Ok(true)
}

pub fn packetlogger_timestamp_microseconds(
    record: &PacketLoggerRecord,
    tz_offset_seconds: f64,
) -> u64 {
    let timestamp =
        ((record.seconds as f64 - tz_offset_seconds) * 1_000_000.0) + record.microseconds as f64;
    if timestamp.is_sign_negative() {
        0
    } else {
        timestamp as u64
    }
}

fn write_pcapng_block<W: Write>(
    writer: &mut W,
    block_type: u32,
    body: &[u8],
) -> Result<(), BtPacketLoggerError> {
    let total_length = u32::try_from(body.len() + 12).map_err(|_| {
        BtPacketLoggerError::Protocol(format!("pcapng block too large: {}", body.len()))
    })?;
    writer.write_all(&block_type.to_le_bytes())?;
    writer.write_all(&total_length.to_le_bytes())?;
    writer.write_all(body)?;
    writer.write_all(&total_length.to_le_bytes())?;
    writer.flush()?;
    Ok(())
}

fn write_options_end(buf: &mut Vec<u8>) {
    buf.extend_from_slice(&0u16.to_le_bytes());
    buf.extend_from_slice(&0u16.to_le_bytes());
}

fn pad_to_32bit(buf: &mut Vec<u8>) {
    let padding = (4 - (buf.len() % 4)) % 4;
    buf.extend(std::iter::repeat(0).take(padding));
}

#[cfg(test)]
mod tests {
    use super::*;

    fn packetlogger_record(
        packet_type: PacketLoggerPacketType,
        payload: &[u8],
        seconds: u32,
        microseconds: u32,
    ) -> Vec<u8> {
        let length = 9 + payload.len() as u32;
        let mut record = Vec::new();
        record.extend_from_slice(&length.to_be_bytes());
        record.extend_from_slice(&seconds.to_be_bytes());
        record.extend_from_slice(&microseconds.to_be_bytes());
        record.push(packet_type as u8);
        record.extend_from_slice(payload);
        record
    }

    #[test]
    fn parses_packetlogger_record_header() {
        let raw = packetlogger_record(
            PacketLoggerPacketType::HciEvent,
            &[1, 2, 3, 4],
            0x01020304,
            0x05060708,
        );

        let record = parse_packetlogger_record(&raw).unwrap();

        assert_eq!(record.length, 13);
        assert_eq!(record.seconds, 0x01020304);
        assert_eq!(record.microseconds, 0x05060708);
        assert_eq!(record.packet_type, PacketLoggerPacketType::HciEvent as u8);
        assert_eq!(record.payload, vec![1, 2, 3, 4]);
    }

    #[test]
    fn rejects_short_packetlogger_record() {
        let err = parse_packetlogger_record(&[0; 12]).unwrap_err();
        assert!(err.to_string().contains("too short"));
    }

    #[test]
    fn maps_packetlogger_records_to_hci_h4_phdr() {
        let cases = [
            (PacketLoggerPacketType::HciCommand, 0u32, 0x01),
            (PacketLoggerPacketType::SentAclData, 0u32, 0x02),
            (PacketLoggerPacketType::RecvAclData, 1u32, 0x02),
            (PacketLoggerPacketType::SentScoData, 0u32, 0x03),
            (PacketLoggerPacketType::RecvScoData, 1u32, 0x03),
            (PacketLoggerPacketType::HciEvent, 1u32, 0x04),
        ];

        for (packet_type, direction, h4_type) in cases {
            let record =
                parse_packetlogger_record(&packetlogger_record(packet_type, &[0xaa, 0xbb], 1, 2))
                    .unwrap();
            let frame = packetlogger_record_to_hci_h4_phdr(&record).unwrap();
            assert_eq!(&frame[0..4], &direction.to_be_bytes());
            assert_eq!(frame[4], h4_type);
            assert_eq!(&frame[5..], &[0xaa, 0xbb]);
        }
    }

    #[test]
    fn skips_unknown_packetlogger_record_type() {
        let mut raw = packetlogger_record(PacketLoggerPacketType::HciEvent, &[0], 1, 2);
        raw[12] = 0xfc;
        let record = parse_packetlogger_record(&raw).unwrap();
        assert_eq!(packetlogger_record_to_hci_h4_phdr(&record), None);
    }

    #[test]
    fn pcapng_timestamp_subtracts_device_timezone_offset() {
        let record = parse_packetlogger_record(&packetlogger_record(
            PacketLoggerPacketType::HciEvent,
            &[0],
            10_000,
            25,
        ))
        .unwrap();

        assert_eq!(
            packetlogger_timestamp_microseconds(&record, 3_600.0),
            6_400_000_025
        );
    }

    #[test]
    fn writes_minimal_pcapng_header_and_enhanced_packet() {
        let record = parse_packetlogger_record(&packetlogger_record(
            PacketLoggerPacketType::RecvAclData,
            &[0x11, 0x22],
            1,
            2,
        ))
        .unwrap();
        let mut out = Vec::new();

        write_pcapng_header(&mut out).unwrap();
        assert!(write_pcapng_record(&mut out, &record, 0.0).unwrap());

        assert_eq!(&out[0..4], &0x0A0D0D0Au32.to_le_bytes());
        assert!(out
            .windows(4)
            .any(|window| window == 0x00000006u32.to_le_bytes()));
        assert!(out
            .windows(7)
            .any(|window| { window == [0, 0, 0, 1, 0x02, 0x11, 0x22] }));
    }
}
