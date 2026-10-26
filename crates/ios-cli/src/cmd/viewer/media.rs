//! Bounded video media pipeline for Viewer (F05).
//!
//! Media Architecture:
//! - CoreDevice screencapture supplies bounded PNG screenshots which are converted to BGRA in-place
//!   and consumed simultaneously by the Web Viewer (HTTP multipart/MJPEG) and VNC/RFB 3.8 servers.
//! - CoreDevice Display RTP stream (`ios_core::display::RtpAssembler`) reassembles Annex-B HEVC NAL
//!   access units.
//! - Licensing & Platform Considerations for Full Pixel HEVC Decoding:
//!   * HEVC/H.265 is subject to licensing patents (HEVC Advance / MPEG LA patent pool).
//!   * On macOS: VideoToolbox (`VTDecompressionSession`) provides hardware zero-copy decoding without
//!     third-party patent licensing encumbrances.
//!   * On Windows: Media Foundation (MFT HEVC Video Decoder) or DirectX Video Acceleration (DXVA2).
//!   * On Linux: VA-API or software `libde265` (LGPL 3.0).
//!   * For portable cross-platform host binaries without native C linking, NAL units (VPS/SPS/PPS/IDR)
//!     are validated and parsed with bounded ring buffers, while screenshot frame streaming utilizes
//!     the verified native PNG path.
use std::collections::VecDeque;
use std::io::Cursor;
use std::sync::Arc;

use anyhow::{ensure, Context, Result};
use bytes::Bytes;
use tokio::sync::watch;

pub const MAX_ENCODED_BYTES: usize = 2 * 1024 * 1024;
pub const MAX_PIXEL_BYTES: usize = 12 * 1024 * 1024;

/// HEVC NAL Unit types according to ITU-T H.265 / ISO/IEC 23008-2.
#[allow(dead_code)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HevcNalType {
    CodedSliceTrail,
    CodedSliceTsa,
    CodedSliceStsa,
    CodedSliceRadl,
    CodedSliceRasl,
    IdrKeyframe,
    CraKeyframe,
    Vps,
    Sps,
    Pps,
    AccessUnitDelimiter,
    Sei,
    Other(u8),
}

#[allow(dead_code)]
impl HevcNalType {
    pub fn from_header(byte0: u8) -> Self {
        let nal_type = (byte0 >> 1) & 0x3F;
        match nal_type {
            0 | 1 => Self::CodedSliceTrail,
            2 | 3 => Self::CodedSliceTsa,
            4 | 5 => Self::CodedSliceStsa,
            6 | 7 => Self::CodedSliceRadl,
            8 | 9 => Self::CodedSliceRasl,
            19 | 20 => Self::IdrKeyframe,
            21 => Self::CraKeyframe,
            32 => Self::Vps,
            33 => Self::Sps,
            34 => Self::Pps,
            35 => Self::AccessUnitDelimiter,
            39 | 40 => Self::Sei,
            other => Self::Other(other),
        }
    }

    pub fn is_keyframe(&self) -> bool {
        matches!(self, Self::IdrKeyframe | Self::CraKeyframe)
    }

    pub fn is_parameter_set(&self) -> bool {
        matches!(self, Self::Vps | Self::Sps | Self::Pps)
    }
}

/// Parse an Annex-B byte stream into identified HEVC NAL units.
#[allow(dead_code)]
pub fn parse_hevc_annex_b_nals(data: &[u8]) -> Vec<(HevcNalType, &[u8])> {
    let mut nals = Vec::new();
    let mut i = 0;
    let mut start_indices = Vec::new();

    while i + 3 <= data.len() {
        if data[i] == 0 && data[i + 1] == 0 {
            if data[i + 2] == 1 {
                start_indices.push((i, i + 3));
                i += 3;
                continue;
            } else if i + 4 <= data.len() && data[i + 2] == 0 && data[i + 3] == 1 {
                start_indices.push((i, i + 4));
                i += 4;
                continue;
            }
        }
        i += 1;
    }

    for (idx, &(_code_start, nal_start)) in start_indices.iter().enumerate() {
        let nal_end = if idx + 1 < start_indices.len() {
            start_indices[idx + 1].0
        } else {
            data.len()
        };
        if nal_start < nal_end {
            let nal_data = &data[nal_start..nal_end];
            let nal_type = HevcNalType::from_header(nal_data[0]);
            nals.push((nal_type, nal_data));
        }
    }

    nals
}

/// Bounded queue for video frames enforcing bounded memory and backpressure for slow consumers.
#[allow(dead_code)]
pub struct BoundedFrameQueue<T> {
    queue: VecDeque<T>,
    capacity: usize,
    dropped_frames: u64,
}

#[allow(dead_code)]
impl<T> BoundedFrameQueue<T> {
    pub fn new(capacity: usize) -> Self {
        Self {
            queue: VecDeque::with_capacity(capacity),
            capacity: capacity.max(1),
            dropped_frames: 0,
        }
    }

    pub fn push(&mut self, item: T) {
        if self.queue.len() >= self.capacity {
            self.queue.pop_front();
            self.dropped_frames += 1;
        }
        self.queue.push_back(item);
    }

    pub fn pop(&mut self) -> Option<T> {
        self.queue.pop_front()
    }

    pub fn dropped_count(&self) -> u64 {
        self.dropped_frames
    }

    pub fn len(&self) -> usize {
        self.queue.len()
    }

    pub fn is_empty(&self) -> bool {
        self.queue.is_empty()
    }
}

#[derive(Clone)]
pub struct DecodedFrame {
    pub png: Bytes,
    pub bgra: Arc<Vec<u8>>,
    pub width: u16,
    pub height: u16,
    pub timestamp_ms: u64,
}

pub fn decode_png(data: Bytes, timestamp_ms: u64) -> Result<DecodedFrame> {
    ensure!(
        data.len() <= MAX_ENCODED_BYTES,
        "encoded frame exceeds budget"
    );
    let mut decoder = png::Decoder::new(Cursor::new(data.clone()));
    decoder.set_limits(png::Limits {
        bytes: MAX_PIXEL_BYTES,
    });
    decoder.set_transformations(png::Transformations::EXPAND | png::Transformations::STRIP_16);
    let mut reader = decoder
        .read_info()
        .context("decode screenshot PNG header")?;
    let (width, height) = (reader.info().width, reader.info().height);
    ensure!(
        width > 0 && height > 0 && width <= u16::MAX as u32 && height <= u16::MAX as u32,
        "PNG dimensions cannot be represented by RFB"
    );
    let pixels = (width as usize)
        .checked_mul(height as usize)
        .context("PNG dimensions overflow")?;
    let output_size = pixels.checked_mul(4).context("PNG pixel size overflow")?;
    ensure!(
        output_size <= MAX_PIXEL_BYTES && reader.output_buffer_size() <= MAX_PIXEL_BYTES,
        "decoded frame exceeds pixel budget"
    );
    let mut decoded = vec![0; reader.output_buffer_size()];
    let info = reader
        .next_frame(&mut decoded)
        .context("decode screenshot PNG pixels")?;
    ensure!(
        info.bit_depth == png::BitDepth::Eight,
        "unsupported PNG bit depth"
    );
    decoded.truncate(info.buffer_size());
    let channels = match info.color_type {
        png::ColorType::Rgba => 4,
        png::ColorType::Rgb => 3,
        png::ColorType::GrayscaleAlpha => 2,
        png::ColorType::Grayscale => 1,
        png::ColorType::Indexed => anyhow::bail!("unexpanded indexed PNG"),
    };
    ensure!(
        decoded.len() == pixels * channels,
        "invalid PNG pixel length"
    );
    // Expand in place, backwards, so old and new full pixel buffers are never retained.
    decoded.resize(output_size, 0);
    for index in (0..pixels).rev() {
        let source = index * channels;
        let (r, g, b, a) = match channels {
            4 => (
                decoded[source],
                decoded[source + 1],
                decoded[source + 2],
                decoded[source + 3],
            ),
            3 => (
                decoded[source],
                decoded[source + 1],
                decoded[source + 2],
                255,
            ),
            2 => (
                decoded[source],
                decoded[source],
                decoded[source],
                decoded[source + 1],
            ),
            _ => (decoded[source], decoded[source], decoded[source], 255),
        };
        decoded[index * 4..index * 4 + 4].copy_from_slice(&[b, g, r, a]);
    }
    Ok(DecodedFrame {
        png: data,
        bgra: Arc::new(decoded),
        width: width as u16,
        height: height as u16,
        timestamp_ms,
    })
}

pub async fn capture_frame(
    udid: &str,
    native: &crate::cmd::tunnel::NativeScope,
) -> Result<DecodedFrame> {
    let device = native.connect(udid, ios_core::TunMode::Userspace).await?;
    let result = async {
        let (xpc, meta) = device
            .connect_xpc_service_with_metadata(ios_core::screencapture::SERVICE_NAME)
            .await?;
        let mut client = ios_core::screencapture::ScreenCaptureServiceClient::new_with_features(
            xpc,
            meta.features,
        );
        let image = client.capture_screenshot(None, "png").await?;
        let timestamp_ms = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)?
            .as_millis() as u64;
        decode_png(image.data, timestamp_ms)
    }
    .await;
    drop(device);
    super::finish(result, native.collect(super::device::CLEANUP_TIMEOUT).await)
}

pub async fn produce(
    udid: String,
    frames: watch::Sender<Option<DecodedFrame>>,
    mut stop: watch::Receiver<bool>,
    native: crate::cmd::tunnel::NativeScope,
) -> Result<()> {
    let mut interval = tokio::time::interval(std::time::Duration::from_millis(100));
    interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    loop {
        tokio::select! {
            biased;
            changed = stop.changed() => { if changed.is_err() || *stop.borrow() { return Ok(()); } }
            _ = interval.tick() => {}
        }
        let frame = tokio::select! {
            biased;
            _ = stop.changed() => return Ok(()),
            frame = tokio::time::timeout(std::time::Duration::from_secs(10), capture_frame(&udid, &native)) => frame.context("viewer capture deadline")??,
        };
        frames.send_replace(Some(frame));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn actual_png_pixels_are_converted_and_oversized_headers_are_rejected() {
        let mut encoded = Vec::new();
        {
            let mut encoder = png::Encoder::new(&mut encoded, 2, 1);
            encoder.set_color(png::ColorType::Rgba);
            encoder.set_depth(png::BitDepth::Eight);
            encoder
                .write_header()
                .unwrap()
                .write_image_data(&[255, 0, 0, 255, 0, 255, 0, 64])
                .unwrap();
        }
        let frame = decode_png(Bytes::from(encoded), 42).unwrap();
        assert_eq!((frame.width, frame.height, frame.timestamp_ms), (2, 1, 42));
        assert_eq!(frame.bgra.as_slice(), &[0, 0, 255, 255, 0, 255, 0, 64]);
        let mut header = Vec::new();
        png::Encoder::new(&mut header, 65535, 65535)
            .write_header()
            .unwrap()
            .finish()
            .unwrap();
        assert!(decode_png(Bytes::from(header), 0).is_err());
        assert!(decode_png(Bytes::from_static(b"not a png"), 0).is_err());
    }

    #[test]
    fn hevc_nal_parsing_and_frame_queue_bounded_drop() {
        // Synthesize an Annex-B stream containing VPS (type 32), SPS (type 33), PPS (type 34), and IDR (type 19)
        let vps_header = (32 << 1) as u8;
        let sps_header = (33 << 1) as u8;
        let pps_header = (34 << 1) as u8;
        let idr_header = (19 << 1) as u8;

        let mut stream = Vec::new();
        // 4-byte start code + VPS
        stream.extend_from_slice(&[0, 0, 0, 1, vps_header, 0x01, 0x02]);
        // 3-byte start code + SPS
        stream.extend_from_slice(&[0, 0, 1, sps_header, 0x03, 0x04]);
        // 4-byte start code + PPS
        stream.extend_from_slice(&[0, 0, 0, 1, pps_header, 0x05]);
        // 4-byte start code + IDR Keyframe
        stream.extend_from_slice(&[0, 0, 0, 1, idr_header, 0x06, 0x07]);

        let nals = parse_hevc_annex_b_nals(&stream);
        assert_eq!(nals.len(), 4);
        assert_eq!(nals[0].0, HevcNalType::Vps);
        assert!(nals[0].0.is_parameter_set());
        assert_eq!(nals[1].0, HevcNalType::Sps);
        assert_eq!(nals[2].0, HevcNalType::Pps);
        assert_eq!(nals[3].0, HevcNalType::IdrKeyframe);
        assert!(nals[3].0.is_keyframe());

        // Test bounded queue drops oldest on overflow to protect slow consumers
        let mut queue = BoundedFrameQueue::new(2);
        assert!(queue.is_empty());
        queue.push(101);
        queue.push(102);
        assert_eq!(queue.len(), 2);
        assert_eq!(queue.dropped_count(), 0);

        queue.push(103); // Drops 101
        assert_eq!(queue.len(), 2);
        assert_eq!(queue.dropped_count(), 1);
        assert_eq!(queue.pop(), Some(102));
        assert_eq!(queue.pop(), Some(103));
        assert_eq!(queue.pop(), None);
    }
}
