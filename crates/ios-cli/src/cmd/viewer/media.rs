//! One bounded PNG media producer, shared by the HTTP and RFB consumers.
//! CoreDevice screencapture supplies PNG; this does not claim HEVC/RTP decoding.
use std::io::Cursor;
use std::sync::Arc;

use anyhow::{ensure, Context, Result};
use bytes::Bytes;
use tokio::sync::watch;

pub const MAX_ENCODED_BYTES: usize = 2 * 1024 * 1024;
pub const MAX_PIXEL_BYTES: usize = 12 * 1024 * 1024;

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
}
