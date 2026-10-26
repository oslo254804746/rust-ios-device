//! Bidirectional clipboard synchronization, format validation, and loop suppression (F05-3).
#![allow(dead_code)]

use anyhow::{ensure, Context, Result};
use ios_core::flate2::read::ZlibDecoder;
use ios_core::flate2::write::ZlibEncoder;
use ios_core::flate2::Compression;
use sha2::{Digest, Sha256};
use std::io::{Read, Write};

pub const MAX_TEXT_BYTES: usize = 1024 * 1024;
pub const MAX_IMAGE_BYTES: usize = 2 * 1024 * 1024;

// Extended Clipboard pseudo-encoding (0xC0A1E5CE as int32)
pub const ENC_EXTENDED_CLIPBOARD: i32 = -1063131698;

pub const FORMAT_TEXT: u32 = 1 << 0;
pub const FORMAT_RTF: u32 = 1 << 1;
pub const FORMAT_HTML: u32 = 1 << 2;
pub const FORMAT_DIB: u32 = 1 << 3;
pub const FORMAT_FILES: u32 = 1 << 4;
// Historical private codec only; RFB bit 3 is DIB, bit 4 is files.
pub const FORMAT_IMAGE_PNG: u32 = 1 << 3;
pub const FORMAT_IMAGE_JPEG: u32 = 1 << 4;
pub const _FORMAT_MASK: u32 = 0xFFFF;

pub const ACTION_CAPS: u32 = 1 << 24;
pub const ACTION_REQUEST: u32 = 1 << 25;
pub const ACTION_PEEK: u32 = 1 << 26;
pub const ACTION_NOTIFY: u32 = 1 << 27;
pub const ACTION_PROVIDE: u32 = 1 << 28;

pub const SERVER_CUT_TEXT: u8 = 3;
pub const CLIENT_CUT_TEXT: u8 = 6;

pub const UTI_PLAIN_TEXT: &str = "public.utf8-plain-text";
pub const UTI_PNG: &str = "public.png";
pub const UTI_JPEG: &str = "public.jpeg";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClipboardSource {
    Host,
    Device,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ClipboardPayload {
    Text(String),
    ImagePng(Vec<u8>),
    ImageJpeg(Vec<u8>),
}

impl ClipboardPayload {
    pub fn byte_len(&self) -> usize {
        match self {
            Self::Text(s) => s.len(),
            Self::ImagePng(b) => b.len(),
            Self::ImageJpeg(b) => b.len(),
        }
    }

    pub fn uti(&self) -> &'static str {
        match self {
            Self::Text(_) => UTI_PLAIN_TEXT,
            Self::ImagePng(_) => UTI_PNG,
            Self::ImageJpeg(_) => UTI_JPEG,
        }
    }

    pub fn sha256_hash(&self) -> [u8; 32] {
        let mut hasher = Sha256::new();
        hasher.update(self.uti().as_bytes());
        hasher.update([0]);
        match self {
            Self::Text(s) => hasher.update(s.replace("\r\n", "\n").as_bytes()),
            Self::ImagePng(b) => {
                // PNG metadata/compression changes on the RFB DIB round trip.
                // Fingerprint the same bounded pixels that this adapter transfers.
                match png_to_dib(b) {
                    Ok(dib) => hasher.update(&dib),
                    Err(_) => hasher.update(b), // invalid/untransmittable payloads retain a stable identity
                }
            }
            Self::ImageJpeg(b) => hasher.update(b),
        }
        hasher.finalize().into()
    }

    pub fn validate_bounds(&self) -> Result<()> {
        match self {
            Self::Text(s) => {
                ensure!(
                    s.len() <= MAX_TEXT_BYTES,
                    "clipboard text length {} exceeds max text budget {}",
                    s.len(),
                    MAX_TEXT_BYTES
                );
            }
            Self::ImagePng(b) => {
                ensure!(
                    b.len() <= MAX_IMAGE_BYTES,
                    "clipboard PNG image bytes {} exceeds max image budget {}",
                    b.len(),
                    MAX_IMAGE_BYTES
                );
            }
            Self::ImageJpeg(b) => {
                ensure!(
                    b.len() <= MAX_IMAGE_BYTES,
                    "clipboard JPEG image bytes {} exceeds max image budget {}",
                    b.len(),
                    MAX_IMAGE_BYTES
                );
            }
        }
        Ok(())
    }
}

/// State tracker for bidirectional clipboard synchronization to suppress loop echoes.
#[derive(Debug, Default)]
pub struct ClipboardSyncState {
    last_synced_hash: Option<[u8; 32]>,
    last_source: Option<ClipboardSource>,
    sync_count: u64,
    suppression_count: u64,
}

impl ClipboardSyncState {
    pub fn new() -> Self {
        Self::default()
    }

    /// Evaluates incoming payload from a source.
    /// Returns `true` if the payload is fresh and should be dispatched to the opposite target,
    /// or `false` if it is an echo from the recent sync and must be suppressed.
    pub fn should_sync(&mut self, payload: &ClipboardPayload, source: ClipboardSource) -> bool {
        let hash = payload.sha256_hash();
        if let (Some(last_hash), Some(last_src)) = (self.last_synced_hash, self.last_source) {
            // If the hash is identical and the message comes from the opposite side of what we just wrote,
            // it's an echo from the device or host confirming our write.
            if last_hash == hash && last_src != source {
                self.suppression_count += 1;
                return false;
            }
            // If the hash is identical and the source is the same, it's a duplicate notification.
            if last_hash == hash && last_src == source {
                return false;
            }
        }

        self.last_synced_hash = Some(hash);
        self.last_source = Some(source);
        self.sync_count += 1;
        true
    }

    pub fn last_synced_hash(&self) -> Option<[u8; 32]> {
        self.last_synced_hash
    }

    pub fn sync_count(&self) -> u64 {
        self.sync_count
    }

    pub fn suppression_count(&self) -> u64 {
        self.suppression_count
    }
}

// -----------------------------------------------------------------------------
// RFB Cut Text formatting (Classic and Extended Clipboard)
// -----------------------------------------------------------------------------

/// Encode classic RFC 6143 ServerCutText (Latin-1 encoded).
pub fn encode_classic_server_cut_text(text: &str) -> Vec<u8> {
    let mut out = Vec::with_capacity(8 + text.len());
    out.push(SERVER_CUT_TEXT);
    out.extend_from_slice(&[0, 0, 0]); // 3 bytes padding
    let latin1: Vec<u8> = text.chars().map(|c| (c as u32).min(255) as u8).collect();
    let len = latin1.len() as u32;
    out.extend_from_slice(&len.to_be_bytes());
    out.extend_from_slice(&latin1);
    out
}

/// Decode classic ClientCutText payload (Latin-1).
pub fn decode_classic_cut_text(bytes: &[u8]) -> Result<String> {
    ensure!(bytes.len() <= MAX_TEXT_BYTES, "cut text exceeds max limit");
    let s: String = bytes.iter().map(|&b| b as char).collect();
    Ok(s)
}

/// Extended Clipboard caps advertisement.
pub fn encode_extended_caps() -> Vec<u8> {
    let flags = ACTION_CAPS
        | ACTION_REQUEST
        | ACTION_PEEK
        | ACTION_NOTIFY
        | ACTION_PROVIDE
        | FORMAT_TEXT
        | FORMAT_DIB;
    let mut body = flags.to_be_bytes().to_vec();
    body.extend_from_slice(&0u32.to_be_bytes()); // Text unsolicited size 0
    body.extend_from_slice(&0u32.to_be_bytes()); // DIB unsolicited size 0
    let len_field = -(body.len() as i32);

    let mut out = Vec::with_capacity(8 + body.len());
    out.push(SERVER_CUT_TEXT);
    out.extend_from_slice(&[0, 0, 0]);
    out.extend_from_slice(&len_field.to_be_bytes());
    out.extend_from_slice(&body);
    out
}

/// Extended Clipboard notify message.
pub fn encode_extended_notify(has_text: bool, has_png: bool) -> Vec<u8> {
    let mut flags = ACTION_NOTIFY;
    if has_text {
        flags |= FORMAT_TEXT;
    }
    if has_png {
        flags |= FORMAT_IMAGE_PNG;
    }
    let body = flags.to_be_bytes();
    let len_field = -(body.len() as i32);

    let mut out = Vec::with_capacity(8 + body.len());
    out.push(SERVER_CUT_TEXT);
    out.extend_from_slice(&[0, 0, 0]);
    out.extend_from_slice(&len_field.to_be_bytes());
    out.extend_from_slice(&body);
    out
}

/// Historical private PNG/JPEG codec, retained only for the existing codec regression.
/// It is not RFB Extended Clipboard and must never reach a production VNC peer.
#[cfg(test)]
pub fn encode_extended_provide_image(image_bytes: &[u8], is_png: bool) -> Result<Vec<u8>> {
    ensure!(
        image_bytes.len() <= MAX_IMAGE_BYTES,
        "extended image exceeds budget"
    );
    let mut uncompressed = Vec::with_capacity(4 + image_bytes.len());
    uncompressed.extend_from_slice(&(image_bytes.len() as u32).to_be_bytes());
    uncompressed.extend_from_slice(image_bytes);

    let mut encoder = ZlibEncoder::new(Vec::new(), Compression::default());
    encoder
        .write_all(&uncompressed)
        .context("zlib compression failed")?;
    let compressed = encoder.finish().context("zlib finish failed")?;

    let format_flag = if is_png {
        FORMAT_IMAGE_PNG
    } else {
        FORMAT_IMAGE_JPEG
    };
    let flags = ACTION_PROVIDE | format_flag;
    let mut body = Vec::with_capacity(4 + compressed.len());
    body.extend_from_slice(&flags.to_be_bytes());
    body.extend_from_slice(&compressed);

    let len_field = -(body.len() as i32);
    let mut out = Vec::with_capacity(8 + body.len());
    out.push(SERVER_CUT_TEXT);
    out.extend_from_slice(&[0, 0, 0]);
    out.extend_from_slice(&len_field.to_be_bytes());
    out.extend_from_slice(&body);
    Ok(out)
}

/// Extended Clipboard provide text (UTF-8 compressed).
pub fn encode_extended_provide_text(text: &str) -> Result<Vec<u8>> {
    ensure!(text.len() <= MAX_TEXT_BYTES, "extended text exceeds budget");
    let mut raw_data = text
        .replace("\r\n", "\n")
        .replace('\n', "\r\n")
        .into_bytes();
    raw_data.push(0); // NUL terminated

    let mut uncompressed = Vec::with_capacity(4 + raw_data.len());
    uncompressed.extend_from_slice(&(raw_data.len() as u32).to_be_bytes());
    uncompressed.extend_from_slice(&raw_data);

    let mut encoder = ZlibEncoder::new(Vec::new(), Compression::default());
    encoder
        .write_all(&uncompressed)
        .context("zlib compression failed")?;
    let compressed = encoder.finish().context("zlib finish failed")?;

    let flags = ACTION_PROVIDE | FORMAT_TEXT;
    let mut body = Vec::with_capacity(4 + compressed.len());
    body.extend_from_slice(&flags.to_be_bytes());
    body.extend_from_slice(&compressed);

    let len_field = -(body.len() as i32);
    let mut out = Vec::with_capacity(8 + body.len());
    out.push(SERVER_CUT_TEXT);
    out.extend_from_slice(&[0, 0, 0]);
    out.extend_from_slice(&len_field.to_be_bytes());
    out.extend_from_slice(&body);
    Ok(out)
}

#[derive(Debug)]
pub struct ExtendedCutTextMessage {
    pub flags: u32,
    pub text: Option<String>,
    pub image_png: Option<Vec<u8>>,
    pub image_jpeg: Option<Vec<u8>>,
}

impl ExtendedCutTextMessage {
    pub fn action(&self) -> u32 {
        if self.flags & ACTION_CAPS != 0 {
            ACTION_CAPS
        } else {
            self.flags & !0xFFFF
        }
    }
}

/// Parse extended clipboard message body where length was negative.
pub fn parse_extended_cut_text(body: &[u8]) -> Result<ExtendedCutTextMessage> {
    ensure!(body.len() >= 4, "extended clipboard body too short");
    let flags = u32::from_be_bytes(body[..4].try_into().unwrap());
    let mut msg = ExtendedCutTextMessage {
        flags,
        text: None,
        image_png: None,
        image_jpeg: None,
    };

    if (flags & ACTION_PROVIDE != 0) && body.len() > 4 {
        let compressed = &body[4..];
        let mut decompressed = Vec::new();
        let decoder = ZlibDecoder::new(compressed);
        decoder
            .take((MAX_IMAGE_BYTES + 5) as u64)
            .read_to_end(&mut decompressed)
            .context("zlib decompression failed")?;

        ensure!(
            decompressed.len() >= 4 && decompressed.len() <= MAX_IMAGE_BYTES + 4,
            "decompressed clipboard payload exceeds bounds"
        );
        let reported_len = u32::from_be_bytes(decompressed[..4].try_into().unwrap()) as usize;
        let payload_bytes = &decompressed[4..];
        ensure!(
            payload_bytes.len() == reported_len,
            "decompressed payload length mismatch"
        );

        if flags & FORMAT_TEXT != 0 {
            let text_bytes = &payload_bytes[..reported_len];
            let clean = text_bytes
                .strip_suffix(b"\0")
                .context("RFB text lacks terminating NUL")?;
            ensure!(
                clean.len() <= MAX_TEXT_BYTES,
                "RFB text exceeds byte budget"
            );
            let s = std::str::from_utf8(clean)
                .context("RFB text is not UTF-8")?
                .replace("\r\n", "\n");
            msg.text = Some(s);
        } else if flags & FORMAT_IMAGE_PNG != 0 {
            msg.image_png = Some(payload_bytes[..reported_len].to_vec());
        } else if flags & FORMAT_IMAGE_JPEG != 0 {
            msg.image_jpeg = Some(payload_bytes[..reported_len].to_vec());
        }
    }

    Ok(msg)
}

/// Convert PNG image bytes into a standard Microsoft Windows Device Independent Bitmap (DIB).
/// RFB requires a 124-byte BITMAPV5HEADER, without a BMP file header, followed by bottom-up BGR rows
/// padded to 4-byte boundaries.
pub fn png_to_dib(png_bytes: &[u8]) -> Result<Vec<u8>> {
    ensure!(
        png_bytes.len() <= MAX_IMAGE_BYTES,
        "PNG bytes exceed max image budget"
    );
    let mut decoder = png::Decoder::new(std::io::Cursor::new(png_bytes));
    decoder.set_limits(png::Limits {
        bytes: MAX_IMAGE_BYTES * 4,
    });
    decoder.set_transformations(png::Transformations::EXPAND | png::Transformations::STRIP_16);
    let mut reader = decoder
        .read_info()
        .context("read PNG info for DIB conversion")?;
    let (width, height) = (reader.info().width, reader.info().height);
    ensure!(
        width > 0 && height > 0 && width <= 16384 && height <= 16384,
        "invalid image dimensions for DIB"
    );

    let row_stride = (width as usize * 3 + 3) & !3;
    let image_size = row_stride * height as usize;
    ensure!(
        124 + image_size <= MAX_IMAGE_BYTES && reader.output_buffer_size() <= MAX_IMAGE_BYTES * 4,
        "decoded DIB exceeds image budget"
    );
    let mut raw_pixels = vec![0u8; reader.output_buffer_size()];
    let info = reader
        .next_frame(&mut raw_pixels)
        .context("decode PNG for DIB conversion")?;
    raw_pixels.truncate(info.buffer_size());

    let channels = match info.color_type {
        png::ColorType::Rgba => 4,
        png::ColorType::Rgb => 3,
        png::ColorType::GrayscaleAlpha => 2,
        png::ColorType::Grayscale => 1,
        png::ColorType::Indexed => anyhow::bail!("unexpanded indexed PNG"),
    };

    let w = width as usize;
    let h = height as usize;
    ensure!(
        raw_pixels.len() == w * h * channels,
        "decoded PNG buffer length mismatch"
    );

    let row_stride = (w * 3 + 3) & !3;
    let image_size = row_stride * h;
    let mut dib = Vec::with_capacity(124 + image_size);

    // BITMAPV5HEADER (all little-endian; 24-bit BI_RGB subset).
    dib.extend_from_slice(&124u32.to_le_bytes()); // biSize
    dib.extend_from_slice(&(width as i32).to_le_bytes()); // biWidth
    dib.extend_from_slice(&(height as i32).to_le_bytes()); // biHeight (positive = bottom-up)
    dib.extend_from_slice(&1u16.to_le_bytes()); // biPlanes
    dib.extend_from_slice(&24u16.to_le_bytes()); // biBitCount (24-bit BGR)
    dib.extend_from_slice(&0u32.to_le_bytes()); // biCompression (BI_RGB)
    dib.extend_from_slice(&(image_size as u32).to_le_bytes()); // biSizeImage
    dib.extend_from_slice(&0i32.to_le_bytes()); // biXPelsPerMeter
    dib.extend_from_slice(&0i32.to_le_bytes()); // biYPelsPerMeter
    dib.extend_from_slice(&0u32.to_le_bytes()); // biClrUsed
    dib.extend_from_slice(&0u32.to_le_bytes()); // biClrImportant

    dib.extend_from_slice(&[0; 16]); // masks unused for BI_RGB
    dib.extend_from_slice(&0x73524742u32.to_le_bytes()); // LCS_sRGB
    dib.extend_from_slice(&[0; 48]); // endpoints and gamma unused for sRGB
    dib.extend_from_slice(&4u32.to_le_bytes()); // LCS_GM_IMAGES
    dib.extend_from_slice(&[0; 12]); // no embedded profile; reserved = 0

    // Pixel rows bottom-up: from row h-1 down to 0
    let padding_len = row_stride - (w * 3);
    let padding = [0u8; 4];
    for y in (0..h).rev() {
        for x in 0..w {
            let src_idx = (y * w + x) * channels;
            let (r, g, b) = match channels {
                4 | 3 => (
                    raw_pixels[src_idx],
                    raw_pixels[src_idx + 1],
                    raw_pixels[src_idx + 2],
                ),
                2 | 1 => (
                    raw_pixels[src_idx],
                    raw_pixels[src_idx],
                    raw_pixels[src_idx],
                ),
                _ => unreachable!(),
            };
            dib.push(b);
            dib.push(g);
            dib.push(r);
        }
        dib.extend_from_slice(&padding[..padding_len]);
    }

    Ok(dib)
}

/// Convert a standard Microsoft Windows Device Independent Bitmap (DIB) into PNG image bytes.
pub fn dib_to_png(dib_bytes: &[u8]) -> Result<Vec<u8>> {
    ensure!(
        dib_bytes.len() >= 124 && dib_bytes.len() <= MAX_IMAGE_BYTES,
        "DIB v5 bytes outside header/image bounds"
    );
    let bi_size = u32::from_le_bytes(dib_bytes[0..4].try_into().unwrap());
    ensure!(bi_size == 124, "RFB requires a 124-byte BITMAPV5HEADER");
    let bi_width = i32::from_le_bytes(dib_bytes[4..8].try_into().unwrap());
    let bi_height = i32::from_le_bytes(dib_bytes[8..12].try_into().unwrap());
    let bi_planes = u16::from_le_bytes(dib_bytes[12..14].try_into().unwrap());
    let bi_bit_count = u16::from_le_bytes(dib_bytes[14..16].try_into().unwrap());
    let bi_compression = u32::from_le_bytes(dib_bytes[16..20].try_into().unwrap());

    ensure!(bi_planes == 1, "DIB biPlanes must be 1");
    ensure!(
        bi_compression == 0 || (bi_compression == 3 && bi_bit_count == 32),
        "only BI_RGB or 32-bit BI_BITFIELDS DIB v5 is supported"
    );
    ensure!(
        bi_bit_count == 24 || bi_bit_count == 32,
        "unsupported DIB bit count (expected 24 or 32 bpp)"
    );
    ensure!(bi_width > 0 && bi_width <= 16384, "invalid DIB width");
    ensure!(
        bi_height != 0 && bi_height.unsigned_abs() <= 16384,
        "invalid DIB height"
    );

    let width = bi_width as usize;
    let (height, bottom_up) = if bi_height > 0 {
        (bi_height as usize, true)
    } else {
        (bi_height.unsigned_abs() as usize, false)
    };

    ensure!(
        u32::from_le_bytes(dib_bytes[32..36].try_into().unwrap()) == 0,
        "DIB palettes are not supported"
    );
    ensure!(
        dib_bytes[112..120] == [0; 8],
        "embedded DIB profiles are not supported"
    );
    let alpha = bi_compression == 3;
    if alpha {
        let masks: Vec<_> = dib_bytes[40..56]
            .chunks_exact(4)
            .map(|b| u32::from_le_bytes(b.try_into().unwrap()))
            .collect();
        ensure!(
            masks == [0x00ff0000, 0x0000ff00, 0x000000ff, 0xff000000],
            "unsupported DIB channel masks"
        );
    }
    ensure!(
        width * height * 4 <= MAX_IMAGE_BYTES * 4,
        "decoded DIB exceeds pixel budget"
    );
    let bpp_bytes = (bi_bit_count / 8) as usize;
    let row_stride = (width * bpp_bytes + 3) & !3;
    let pixel_data = &dib_bytes[124..];
    ensure!(
        pixel_data.len() >= row_stride * height,
        "DIB pixel data truncated"
    );

    let mut rgb_pixels = Vec::with_capacity(width * height * if alpha { 4 } else { 3 });
    for row in 0..height {
        let y = if bottom_up { height - 1 - row } else { row };
        let row_start = y * row_stride;
        let row_bytes = &pixel_data[row_start..row_start + width * bpp_bytes];
        for x in 0..width {
            let px = &row_bytes[x * bpp_bytes..x * bpp_bytes + bpp_bytes];
            let b = px[0];
            let g = px[1];
            let r = px[2];
            rgb_pixels.push(r);
            rgb_pixels.push(g);
            rgb_pixels.push(b);
            if alpha {
                rgb_pixels.push(px[3]);
            }
        }
    }

    let mut png_bytes = Vec::new();
    {
        let mut encoder = png::Encoder::new(&mut png_bytes, width as u32, height as u32);
        encoder.set_color(if alpha {
            png::ColorType::Rgba
        } else {
            png::ColorType::Rgb
        });
        encoder.set_depth(png::BitDepth::Eight);
        let mut writer = encoder.write_header().context("write PNG header")?;
        writer
            .write_image_data(&rgb_pixels)
            .context("write PNG image data")?;
    }

    Ok(png_bytes)
}

/// Encode extended provide DIB message for RFB clients.
pub fn encode_extended_provide_dib(png_bytes: &[u8]) -> Result<Vec<u8>> {
    let dib = png_to_dib(png_bytes)?;
    ensure!(dib.len() <= MAX_IMAGE_BYTES, "DIB payload exceeds budget");

    let mut uncompressed = Vec::with_capacity(4 + dib.len());
    uncompressed.extend_from_slice(&(dib.len() as u32).to_be_bytes());
    uncompressed.extend_from_slice(&dib);

    let mut encoder = ZlibEncoder::new(Vec::new(), Compression::default());
    encoder
        .write_all(&uncompressed)
        .context("zlib compression failed")?;
    let compressed = encoder.finish().context("zlib finish failed")?;

    let flags = ACTION_PROVIDE | FORMAT_DIB;
    let mut body = Vec::with_capacity(4 + compressed.len());
    body.extend_from_slice(&flags.to_be_bytes());
    body.extend_from_slice(&compressed);

    let len_field = -(body.len() as i32);
    let mut out = Vec::with_capacity(8 + body.len());
    out.push(SERVER_CUT_TEXT);
    out.extend_from_slice(&[0, 0, 0]);
    out.extend_from_slice(&len_field.to_be_bytes());
    out.extend_from_slice(&body);
    Ok(out)
}

/// RFB Extended Clipboard accepts standard text and DIB formats.
/// Spec: rfbproto/rfbproto.rst, Extended Clipboard Pseudo-encoding.
pub fn parse_rfb_extended_cut_text(body: &[u8]) -> Result<ExtendedCutTextMessage> {
    ensure!(body.len() >= 4, "extended clipboard body too short");
    let flags = u32::from_be_bytes(body[..4].try_into().unwrap());
    if flags & ACTION_CAPS != 0 {
        let formats = (flags & 0xffff).count_ones() as usize;
        ensure!(
            body.len() == 4 + 4 * formats,
            "invalid RFB clipboard capability sizes"
        );
        return Ok(ExtendedCutTextMessage {
            flags,
            text: None,
            image_png: None,
            image_jpeg: None,
        });
    }
    let action = flags & !0xffff;
    ensure!(
        matches!(
            action,
            ACTION_REQUEST | ACTION_PEEK | ACTION_NOTIFY | ACTION_PROVIDE
        ),
        "invalid RFB clipboard action"
    );
    let format_mask = flags & 0xffff;
    if format_mask & !(FORMAT_TEXT | FORMAT_DIB) != 0 {
        return Err(std::io::Error::new(
            std::io::ErrorKind::Unsupported,
            "RFB clipboard format unavailable: files/RTF/HTML are not supported",
        )
        .into());
    }
    if action == ACTION_PROVIDE {
        if flags & FORMAT_DIB != 0 && body.len() <= 4 {
            return Err(std::io::Error::new(
                std::io::ErrorKind::Unsupported,
                "DIB provide requires DIB payload data",
            )
            .into());
        }
        ensure!(body.len() > 4, "RFB provide requires text or DIB data");
    } else {
        ensure!(body.len() == 4, "unexpected RFB clipboard action payload");
    }

    if (flags & ACTION_PROVIDE != 0) && body.len() > 4 {
        let compressed = &body[4..];
        let mut decompressed = Vec::new();
        let decoder = ZlibDecoder::new(compressed);
        decoder
            .take((MAX_IMAGE_BYTES + MAX_TEXT_BYTES + 9) as u64)
            .read_to_end(&mut decompressed)
            .context("zlib decompression failed")?;

        ensure!(
            decompressed.len() >= 4 && decompressed.len() <= MAX_IMAGE_BYTES + MAX_TEXT_BYTES + 8,
            "decompressed clipboard payload exceeds bounds"
        );
        let mut remaining = decompressed.as_slice();
        let mut message = ExtendedCutTextMessage {
            flags,
            text: None,
            image_png: None,
            image_jpeg: None,
        };
        for format in [FORMAT_TEXT, FORMAT_DIB] {
            if flags & format == 0 {
                continue;
            }
            ensure!(remaining.len() >= 4, "missing RFB format size");
            let size = u32::from_be_bytes(remaining[..4].try_into().unwrap()) as usize;
            remaining = &remaining[4..];
            ensure!(
                size <= remaining.len(),
                "decompressed payload length mismatch"
            );
            let (payload, rest) = remaining.split_at(size);
            remaining = rest;
            if format == FORMAT_TEXT {
                let text = payload
                    .strip_suffix(b"\0")
                    .context("RFB text lacks terminating NUL")?;
                ensure!(text.len() <= MAX_TEXT_BYTES, "RFB text exceeds byte budget");
                message.text = Some(
                    std::str::from_utf8(text)
                        .context("RFB text is not UTF-8")?
                        .replace("\r\n", "\n"),
                );
            } else {
                message.image_png = Some(dib_to_png(payload)?);
            }
        }
        ensure!(
            remaining.is_empty(),
            "unexpected trailing RFB clipboard data"
        );
        return Ok(message);
    }

    Ok(ExtendedCutTextMessage {
        flags,
        text: None,
        image_png: None,
        image_jpeg: None,
    })
}
