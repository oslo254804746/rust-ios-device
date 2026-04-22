//! Screenshot service.
//!
//! Connects to `com.apple.mobile.screenshotr` and captures a screenshot.
//!
//! Protocol: plist-framed (same 4-byte BE length prefix as lockdown).
//! 1. Send DL message: {"MessageType":"DLMessageVersionExchange", "SupportedVersions":[1]}
//! 2. Recv version exchange response
//! 3. Send DL ready: {"MessageType":"DLMessageDeviceReady"}
//! 4. Recv: screenshot plist with "ScreenShotData" key (TIFF/PNG/JPEG bytes)
//!
//! Reference: libimobiledevice screenshotr protocol

use bytes::Bytes;
use serde::Serialize;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

pub const SERVICE_NAME: &str = "com.apple.mobile.screenshotr";

#[derive(Debug, thiserror::Error)]
pub enum ScreenshotError {
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
    #[error("plist error: {0}")]
    Plist(String),
    #[error("protocol error: {0}")]
    Protocol(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum ScreenshotFormat {
    Png,
    Jpeg,
    Tiff,
    Unknown,
}

impl ScreenshotFormat {
    pub fn mime_type(self) -> &'static str {
        match self {
            Self::Png => "image/png",
            Self::Jpeg => "image/jpeg",
            Self::Tiff => "image/tiff",
            Self::Unknown => "application/octet-stream",
        }
    }

    pub fn detect(bytes: &[u8]) -> Self {
        if bytes.starts_with(&[0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A]) {
            Self::Png
        } else if bytes.starts_with(&[0xFF, 0xD8, 0xFF]) {
            Self::Jpeg
        } else if bytes.starts_with(b"II*\0") || bytes.starts_with(b"MM\0*") {
            Self::Tiff
        } else {
            Self::Unknown
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScreenshotImage {
    pub data: Bytes,
    pub format: ScreenshotFormat,
}

impl ScreenshotImage {
    pub fn from_bytes(data: Bytes) -> Self {
        let format = ScreenshotFormat::detect(&data);
        Self { data, format }
    }

    pub fn mime_type(&self) -> &'static str {
        self.format.mime_type()
    }

    pub fn byte_len(&self) -> usize {
        self.data.len()
    }
}

#[derive(Serialize)]
#[serde(rename_all = "PascalCase")]
struct VersionExchangeRequest {
    message_type: &'static str,
    supported_versions: Vec<u64>,
}

#[derive(Serialize)]
#[serde(rename_all = "PascalCase")]
struct DeviceReadyRequest {
    message_type: &'static str,
}

/// Capture a screenshot from the device.
///
/// Returns raw image bytes plus detected format metadata.
pub async fn take_screenshot<S>(stream: &mut S) -> Result<ScreenshotImage, ScreenshotError>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    // 1. Send version exchange
    send_plist(
        stream,
        &VersionExchangeRequest {
            message_type: "DLMessageVersionExchange",
            supported_versions: vec![1],
        },
    )
    .await?;

    // 2. Recv version exchange response (ignore content)
    recv_plist_raw(stream).await?;

    // 3. Send device ready
    send_plist(
        stream,
        &DeviceReadyRequest {
            message_type: "DLMessageDeviceReady",
        },
    )
    .await?;

    // 4. Recv screenshot plist
    let data = recv_plist_raw(stream).await?;

    // Parse plist to find ScreenShotData
    let val: plist::Value =
        plist::from_bytes(&data).map_err(|e| ScreenshotError::Plist(e.to_string()))?;

    // The plist is an array: [MessageType, {ScreenShotData: <data>}]
    if let Some(arr) = val.as_array() {
        for item in arr {
            if let Some(dict) = item.as_dictionary() {
                if let Some(img) = dict.get("ScreenShotData") {
                    if let Some(bytes) = img.as_data() {
                        return Ok(ScreenshotImage::from_bytes(Bytes::copy_from_slice(bytes)));
                    }
                }
            }
        }
    }

    Err(ScreenshotError::Protocol(
        "ScreenShotData not found in response".into(),
    ))
}

// ── plist framing (same as lockdown: 4-byte BE length prefix) ─────────────────

async fn send_plist<S, T>(stream: &mut S, value: &T) -> Result<(), ScreenshotError>
where
    S: AsyncWrite + Unpin,
    T: Serialize,
{
    let mut buf = Vec::new();
    plist::to_writer_xml(&mut buf, value).map_err(|e| ScreenshotError::Plist(e.to_string()))?;
    stream.write_all(&(buf.len() as u32).to_be_bytes()).await?;
    stream.write_all(&buf).await?;
    stream.flush().await?;
    Ok(())
}

async fn recv_plist_raw<S>(stream: &mut S) -> Result<Vec<u8>, ScreenshotError>
where
    S: AsyncRead + Unpin,
{
    let mut len_buf = [0u8; 4];
    stream.read_exact(&mut len_buf).await?;
    let len = u32::from_be_bytes(len_buf) as usize;
    const MAX_PLIST_SIZE: usize = 4 * 1024 * 1024;
    if len > MAX_PLIST_SIZE {
        return Err(ScreenshotError::Protocol(format!(
            "plist length {len} exceeds maximum of {MAX_PLIST_SIZE}"
        )));
    }
    let mut buf = vec![0u8; len];
    stream.read_exact(&mut buf).await?;
    Ok(buf)
}

#[cfg(test)]
mod tests {
    use bytes::Bytes;

    use super::{ScreenshotFormat, ScreenshotImage};

    #[test]
    fn detects_png_signature() {
        let format = ScreenshotFormat::detect(&[0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A]);
        assert_eq!(format, ScreenshotFormat::Png);
        assert_eq!(format.mime_type(), "image/png");
    }

    #[test]
    fn detects_jpeg_signature() {
        let format = ScreenshotFormat::detect(&[0xFF, 0xD8, 0xFF, 0xE0]);
        assert_eq!(format, ScreenshotFormat::Jpeg);
        assert_eq!(format.mime_type(), "image/jpeg");
    }

    #[test]
    fn detects_tiff_signatures() {
        assert_eq!(
            ScreenshotFormat::detect(b"II*\0rest"),
            ScreenshotFormat::Tiff
        );
        assert_eq!(
            ScreenshotFormat::detect(b"MM\0*rest"),
            ScreenshotFormat::Tiff
        );
    }

    #[test]
    fn unknown_signature_falls_back_to_octet_stream() {
        let image = ScreenshotImage::from_bytes(Bytes::from_static(b"not-an-image"));
        assert_eq!(image.format, ScreenshotFormat::Unknown);
        assert_eq!(image.mime_type(), "application/octet-stream");
        assert_eq!(image.byte_len(), 12);
    }
}
