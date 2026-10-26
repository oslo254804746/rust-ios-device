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
            Self::Text(s) => hasher.update(s.as_bytes()),
            Self::ImagePng(b) => hasher.update(b),
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
    let flags =
        ACTION_CAPS | ACTION_REQUEST | ACTION_PEEK | ACTION_NOTIFY | ACTION_PROVIDE | FORMAT_TEXT;
    let mut body = flags.to_be_bytes().to_vec();
    body.extend_from_slice(&0u32.to_be_bytes()); // Unsolicited size 0
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

/// RFB Extended Clipboard accepts only the implemented UTF-8 text format.
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
    if flags & 0xffff & !FORMAT_TEXT != 0 {
        return Err(std::io::Error::new(
            std::io::ErrorKind::Unsupported,
            "RFB clipboard format unavailable: DIB/files/RTF/HTML are not PNG/JPEG",
        )
        .into());
    }
    if action == ACTION_PROVIDE {
        ensure!(
            flags & FORMAT_TEXT != 0 && body.len() > 4,
            "RFB provide requires text data"
        );
    } else {
        ensure!(body.len() == 4, "unexpected RFB clipboard action payload");
    }
    let message = parse_extended_cut_text(body)?;
    if let Some(text) = &message.text {
        ensure!(
            text.len() <= MAX_TEXT_BYTES,
            "RFB clipboard text exceeds budget"
        );
    }
    Ok(message)
}
