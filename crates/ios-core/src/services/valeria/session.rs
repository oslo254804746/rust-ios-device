use std::fs::File;
use std::io::Write;
use std::path::Path;
use std::time::{Duration, Instant};

use super::coremedia::parse_feed_payload;
use super::protocol::{self, Packet, Tag};
use super::{CaptureOptions, H264Frame, ValeriaError};

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, serde::Serialize)]
pub struct CaptureSummary {
    pub frames: u64,
    pub bytes: u64,
    pub width: u32,
    pub height: u32,
    pub dropped_frames: u64,
}

pub struct ValeriaSession {
    _options: CaptureOptions,
    summary: CaptureSummary,
    device_video_clock_ref: Option<u64>,
    device_audio_clock_ref: Option<u64>,
    local_video_clock_ref: u64,
    local_audio_clock_ref: u64,
    local_clock_ref: u64,
    started: Instant,
    video_stream_started: bool,
}

pub(crate) trait ValeriaTransport {
    fn read_packet(&mut self) -> Result<Packet, ValeriaError>;
    fn write_bytes(&mut self, bytes: &[u8]) -> Result<(), ValeriaError>;
}

impl ValeriaSession {
    pub fn new(options: CaptureOptions) -> Self {
        Self {
            _options: options,
            summary: CaptureSummary::default(),
            device_video_clock_ref: None,
            device_audio_clock_ref: None,
            local_video_clock_ref: 0x7669_6465_6f00_0001,
            local_audio_clock_ref: 0x6175_6469_6f00_0001,
            local_clock_ref: 0x636c_6f63_6b00_0001,
            started: Instant::now(),
            video_stream_started: false,
        }
    }

    pub fn summary(&self) -> CaptureSummary {
        self.summary
    }

    #[cfg(test)]
    pub(crate) fn run_until_first_frame<T: ValeriaTransport>(
        &mut self,
        transport: &mut T,
    ) -> Result<H264Frame, ValeriaError> {
        loop {
            if let Some(frame) = self.next_frame(transport)? {
                return Ok(frame);
            }
        }
    }

    pub(crate) fn record_annex_b<T: ValeriaTransport>(
        &mut self,
        transport: &mut T,
        output: &Path,
        duration_secs: u64,
    ) -> Result<CaptureSummary, ValeriaError> {
        let mut file = File::create(output)?;
        let deadline =
            (duration_secs > 0).then(|| Instant::now() + Duration::from_secs(duration_secs));

        loop {
            if deadline.is_some_and(|deadline| Instant::now() >= deadline) {
                break;
            }

            match self.next_frame(transport) {
                Ok(Some(frame)) => {
                    let annex_b = frame.to_annex_b()?;
                    file.write_all(&annex_b)?;
                    self.summary.bytes = self.summary.bytes.saturating_add(annex_b.len() as u64);
                }
                Ok(None) => {}
                Err(ValeriaError::Stopped) => break,
                Err(err) => return Err(err),
            }
        }

        file.flush()?;
        Ok(self.summary)
    }

    pub(crate) fn close<T: ValeriaTransport>(
        &mut self,
        transport: &mut T,
    ) -> Result<(), ValeriaError> {
        if let Some(clock_ref) = self.device_audio_clock_ref {
            let stop = protocol::encode_stop_audio(clock_ref);
            transport.write_bytes(&stop)?;
        }
        if self.video_stream_started || self.device_video_clock_ref.is_some() {
            let stop = protocol::encode_stop_video();
            transport.write_bytes(&stop)?;
        }
        Ok(())
    }

    #[cfg(test)]
    fn set_device_video_clock_for_test(&mut self, clock_ref: u64) {
        self.device_video_clock_ref = Some(clock_ref);
        self.video_stream_started = true;
    }

    #[cfg(test)]
    fn set_device_audio_clock_for_test(&mut self, clock_ref: u64) {
        self.device_audio_clock_ref = Some(clock_ref);
    }

    fn next_frame<T: ValeriaTransport>(
        &mut self,
        transport: &mut T,
    ) -> Result<Option<H264Frame>, ValeriaError> {
        let packet = transport.read_packet()?;
        match packet.tag {
            Tag::Ping => {
                transport.write_bytes(&protocol::encode_ping())?;
                Ok(None)
            }
            Tag::Sync => {
                self.handle_sync(transport, packet)?;
                Ok(None)
            }
            Tag::Asyn => self.handle_asyn(transport, packet),
            _ => Ok(None),
        }
    }

    fn handle_asyn<T: ValeriaTransport>(
        &mut self,
        transport: &mut T,
        packet: Packet,
    ) -> Result<Option<H264Frame>, ValeriaError> {
        if packet.payload.len() < 12 {
            return Err(ValeriaError::Protocol(
                "ASYN payload shorter than clock/subtype header".into(),
            ));
        }

        let subtype = Tag::from_bytes(&packet.payload[8..12]).ok_or_else(|| {
            ValeriaError::Protocol(format!(
                "unknown ASYN subtype {:02x?}",
                &packet.payload[8..12]
            ))
        })?;
        if subtype != Tag::Feed {
            return Ok(None);
        }

        let sample = parse_feed_payload(&packet.payload)?;
        let frame = sample.into_h264_frame()?;
        self.summary.frames = self.summary.frames.saturating_add(1);
        self.summary.width = frame.width;
        self.summary.height = frame.height;

        if let Some(clock_ref) = self.device_video_clock_ref {
            let need = protocol::encode_asyn_simple(Tag::Need, clock_ref)?;
            transport.write_bytes(&need)?;
        }

        Ok(Some(frame))
    }

    fn handle_sync<T: ValeriaTransport>(
        &mut self,
        transport: &mut T,
        packet: Packet,
    ) -> Result<(), ValeriaError> {
        if packet.payload.len() < 20 {
            return Err(ValeriaError::Protocol(
                "SYNC payload shorter than clock/subtype/correlation header".into(),
            ));
        }

        let subtype = Tag::from_bytes(&packet.payload[8..12]).ok_or_else(|| {
            ValeriaError::Protocol(format!(
                "unknown SYNC subtype {:02x?}",
                &packet.payload[8..12]
            ))
        })?;
        let correlation = read_u64_le(&packet.payload[12..20], "SYNC correlation")?;

        match subtype {
            Tag::Cvrp => {
                let device_clock = read_optional_u64_le(&packet.payload[20..])
                    .unwrap_or_else(|| read_u64_le(&packet.payload[0..8], "SYNC clock"))
                    .map_err(|err| ValeriaError::Protocol(format!("CVRP clock: {err}")))?;
                self.device_video_clock_ref = Some(device_clock);
                let need = protocol::encode_asyn_simple(Tag::Need, device_clock)?;
                transport.write_bytes(&need)?;
                transport.write_bytes(&protocol::encode_reply_u64(
                    correlation,
                    self.local_video_clock_ref,
                ))?;
            }
            Tag::Cwpa => {
                let device_clock = read_optional_u64_le(&packet.payload[20..])
                    .unwrap_or_else(|| read_u64_le(&packet.payload[0..8], "SYNC clock"))
                    .map_err(|err| ValeriaError::Protocol(format!("CWPA clock: {err}")))?;
                self.device_audio_clock_ref = Some(device_clock);
                let start_video = protocol::encode_start_video();
                transport.write_bytes(&start_video)?;
                transport.write_bytes(&start_video)?;
                self.video_stream_started = true;
                transport.write_bytes(&protocol::encode_reply_u64(
                    correlation,
                    self.local_audio_clock_ref,
                ))?;
                transport.write_bytes(&protocol::encode_start_audio(device_clock))?;
            }
            Tag::Clok => {
                transport.write_bytes(&protocol::encode_reply_u64(
                    correlation,
                    self.local_clock_ref,
                ))?;
            }
            Tag::Time => {
                transport.write_bytes(&protocol::encode_reply_cmtime(
                    correlation,
                    self.elapsed_ns(),
                ))?;
            }
            Tag::Skew => {
                transport.write_bytes(&protocol::encode_reply_f64(correlation, 48_000.0))?;
            }
            Tag::Afmt => {
                transport.write_bytes(&protocol::encode_afmt_reply(correlation))?;
            }
            Tag::Og | Tag::Stop => {
                transport.write_bytes(&protocol::encode_reply_status_ok(correlation))?;
            }
            _ => {}
        }

        Ok(())
    }

    fn elapsed_ns(&self) -> i64 {
        let nanos = self.started.elapsed().as_nanos();
        nanos.min(i64::MAX as u128) as i64
    }
}

fn read_u64_le(bytes: &[u8], what: &str) -> Result<u64, ValeriaError> {
    if bytes.len() < 8 {
        return Err(ValeriaError::Protocol(format!(
            "{what} requires 8 bytes, got {}",
            bytes.len()
        )));
    }
    Ok(u64::from_le_bytes(
        bytes[0..8].try_into().expect("slice length checked"),
    ))
}

fn read_optional_u64_le(bytes: &[u8]) -> Option<Result<u64, ValeriaError>> {
    (bytes.len() >= 8).then(|| read_u64_le(bytes, "optional clock"))
}

#[cfg(test)]
mod tests {
    use std::collections::VecDeque;

    use bytes::BufMut;

    use super::*;
    use crate::services::valeria::protocol::{decode_packet, encode_ping, Packet, Tag};

    #[derive(Default)]
    struct ScriptedTransport {
        incoming: VecDeque<Packet>,
        written: Vec<Vec<u8>>,
    }

    impl ValeriaTransport for ScriptedTransport {
        fn read_packet(&mut self) -> Result<Packet, ValeriaError> {
            self.incoming.pop_front().ok_or(ValeriaError::Stopped)
        }

        fn write_bytes(&mut self, bytes: &[u8]) -> Result<(), ValeriaError> {
            self.written.push(bytes.to_vec());
            Ok(())
        }
    }

    #[test]
    fn replies_to_ping_before_session_negotiation() {
        let mut transport = ScriptedTransport {
            incoming: VecDeque::from([Packet {
                tag: Tag::Ping,
                payload: bytes::Bytes::from_static(&[0, 0, 0, 0, 1, 0, 0, 0]),
            }]),
            written: Vec::new(),
        };

        let mut session = ValeriaSession::new(CaptureOptions::default());
        let err = session
            .run_until_first_frame(&mut transport)
            .expect_err("no feed fixture");
        assert!(matches!(err, ValeriaError::Stopped));
        assert_eq!(transport.written, vec![encode_ping().to_vec()]);
    }

    #[test]
    fn feed_packet_yields_h264_frame_and_sends_need_when_clock_is_known() {
        let feed = decode_packet(include_bytes!(
            "../../../tests/fixtures/valeria/asyn-feed.bin"
        ))
        .unwrap();

        let mut transport = ScriptedTransport {
            incoming: VecDeque::from([feed]),
            written: Vec::new(),
        };

        let mut session = ValeriaSession::new(CaptureOptions::default());
        session.set_device_video_clock_for_test(0x0102030405060708);
        let frame = session.run_until_first_frame(&mut transport).unwrap();

        assert!(!frame.nalu_data.is_empty());
        assert_eq!(session.summary().frames, 1);
        assert!(transport
            .written
            .iter()
            .any(|bytes| bytes.ends_with(b"deen")));
    }

    #[test]
    fn cvrp_sync_records_video_clock_and_requests_feed() {
        let mut payload = bytes::BytesMut::new();
        payload.put_u64_le(0x1111_2222_3333_4444);
        payload.put_slice(Tag::Cvrp.bytes());
        payload.put_u64_le(0x0101_0101_0101_0101);
        payload.put_u64_le(0x0102_0304_0506_0708);

        let mut transport = ScriptedTransport {
            incoming: VecDeque::from([Packet {
                tag: Tag::Sync,
                payload: payload.freeze(),
            }]),
            written: Vec::new(),
        };

        let mut session = ValeriaSession::new(CaptureOptions::default());
        let err = session
            .run_until_first_frame(&mut transport)
            .expect_err("sync does not include a frame");

        assert!(matches!(err, ValeriaError::Stopped));
        assert!(transport
            .written
            .iter()
            .any(|bytes| bytes.get(4..8) == Some(Tag::Reply.bytes().as_slice())));
        assert!(transport
            .written
            .iter()
            .any(|bytes| bytes.ends_with(b"deen")));
    }

    #[test]
    fn reply_packets_include_padding_before_values() {
        let correlation = 0x0102_0304_0506_0708;
        let value = 0x1112_1314_1516_1718;
        let bytes = protocol::encode_reply_u64(correlation, value);

        assert_eq!(&bytes[0..4], &[0x1c, 0, 0, 0]);
        assert_eq!(&bytes[4..8], b"ylpr");
        assert_eq!(
            &bytes[8..16],
            &[0x08, 0x07, 0x06, 0x05, 0x04, 0x03, 0x02, 0x01]
        );
        assert_eq!(&bytes[16..20], &[0, 0, 0, 0]);
        assert_eq!(
            &bytes[20..28],
            &[0x18, 0x17, 0x16, 0x15, 0x14, 0x13, 0x12, 0x11]
        );
    }

    #[test]
    fn status_replies_include_zero_status_after_padding() {
        let og = protocol::encode_reply_status_ok(0x0000_0001_02d3_2f30);
        let stop = protocol::encode_reply_status_ok(0x0000_0001_02fd_4910);

        assert_eq!(
            og.as_ref(),
            include_bytes!("../../../tests/fixtures/valeria/og-reply.bin")
        );
        assert_eq!(
            stop.as_ref(),
            include_bytes!("../../../tests/fixtures/valeria/stop-reply.bin")
        );
    }

    #[test]
    fn afmt_sync_replies_with_zero_error_dictionary() {
        let mut payload = bytes::BytesMut::new();
        payload.put_u64_le(0x0000_7fa6_6ce2_0cb0);
        payload.put_slice(Tag::Afmt.bytes());
        payload.put_u64_le(0x0000_0001_1322_9d80);

        let mut transport = ScriptedTransport {
            incoming: VecDeque::from([Packet {
                tag: Tag::Sync,
                payload: payload.freeze(),
            }]),
            written: Vec::new(),
        };

        let mut session = ValeriaSession::new(CaptureOptions::default());
        let err = session
            .run_until_first_frame(&mut transport)
            .expect_err("sync does not include a frame");

        assert!(matches!(err, ValeriaError::Stopped));
        assert_eq!(
            transport.written[0],
            include_bytes!("../../../tests/fixtures/valeria/afmt-reply.bin")
        );
    }

    #[test]
    fn cwpa_sync_starts_video_and_audio_streams() {
        let mut payload = bytes::BytesMut::new();
        payload.put_u64_le(1);
        payload.put_slice(Tag::Cwpa.bytes());
        payload.put_u64_le(0x0101_0101_0101_0101);
        payload.put_u64_le(0x0000_0001_1453_92f0);

        let mut transport = ScriptedTransport {
            incoming: VecDeque::from([Packet {
                tag: Tag::Sync,
                payload: payload.freeze(),
            }]),
            written: Vec::new(),
        };

        let mut session = ValeriaSession::new(CaptureOptions::default());
        let err = session
            .run_until_first_frame(&mut transport)
            .expect_err("sync does not include a frame");

        assert!(matches!(err, ValeriaError::Stopped));
        assert_eq!(
            transport.written[0],
            include_bytes!("../../../tests/fixtures/valeria/asyn-hpd1.bin")
        );
        assert_eq!(
            transport.written[1],
            include_bytes!("../../../tests/fixtures/valeria/asyn-hpd1.bin")
        );
        assert_eq!(&transport.written[2][0..4], &[0x1c, 0, 0, 0]);
        assert_eq!(&transport.written[2][4..8], b"ylpr");
        assert_eq!(&transport.written[2][16..20], &[0, 0, 0, 0]);
        assert_eq!(
            transport.written[3],
            include_bytes!("../../../tests/fixtures/valeria/asyn-hpa1.bin")
        );
    }

    #[test]
    fn record_annex_b_writes_frames_until_transport_stops() {
        let feed = decode_packet(include_bytes!(
            "../../../tests/fixtures/valeria/asyn-feed.bin"
        ))
        .unwrap();
        let mut transport = ScriptedTransport {
            incoming: VecDeque::from([feed]),
            written: Vec::new(),
        };
        let path =
            std::env::temp_dir().join(format!("ios-core-valeria-{}.h264", std::process::id()));
        let _ = std::fs::remove_file(&path);

        let mut session = ValeriaSession::new(CaptureOptions::default());
        let summary = session.record_annex_b(&mut transport, &path, 0).unwrap();
        let bytes = std::fs::read(&path).unwrap();
        let _ = std::fs::remove_file(&path);

        assert_eq!(summary.frames, 1);
        assert_eq!(summary.bytes, bytes.len() as u64);
        assert!(bytes.starts_with(b"\x00\x00\x00\x01"));
    }

    #[test]
    fn close_sends_video_and_audio_stop_packets_when_clocks_are_known() {
        let mut transport = ScriptedTransport::default();
        let mut session = ValeriaSession::new(CaptureOptions::default());
        session.set_device_video_clock_for_test(0x0102_0304_0506_0708);
        session.set_device_audio_clock_for_test(0x1112_1314_1516_1718);

        session.close(&mut transport).unwrap();

        assert!(transport
            .written
            .iter()
            .any(|bytes| bytes.ends_with(b"0dph")));
        assert!(transport
            .written
            .iter()
            .any(|bytes| bytes.ends_with(b"0aph")));
    }
}
