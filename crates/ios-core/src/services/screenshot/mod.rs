//! Screenshot service.
//!
//! Connects to `com.apple.mobile.screenshotr` and captures a screenshot.
//!
//! Protocol: plist-framed (same 4-byte BE length prefix as lockdown).
//! 1. Send DL message: `{"MessageType":"DLMessageVersionExchange", "SupportedVersions":[1]}`
//! 2. Recv version exchange response
//! 3. Send DL ready: {"MessageType":"DLMessageDeviceReady"}
//! 4. Recv: screenshot plist with "ScreenShotData" key (TIFF/PNG/JPEG bytes)
//!
//! Reference: libimobiledevice screenshotr protocol

use std::{
    future::Future,
    pin::Pin,
    task::{Context, Poll},
    time::Duration,
};

use bytes::Bytes;
use serde::Serialize;
use tokio::{
    io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt, ReadBuf},
    time::Instant,
};

pub const SERVICE_NAME: &str = "com.apple.mobile.screenshotr";

service_error!(ScreenshotError);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum ScreenshotFormat {
    Png,
    Jpeg,
    Tiff,
    Heif,
    Unknown,
}

impl ScreenshotFormat {
    pub fn mime_type(self) -> &'static str {
        match self {
            Self::Png => "image/png",
            Self::Jpeg => "image/jpeg",
            Self::Tiff => "image/tiff",
            Self::Heif => "image/heif",
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
        } else if bytes.len() >= 12 && &bytes[4..8] == b"ftyp" {
            Self::Heif
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

/// Screenshot service client with explicit operation deadline and connection state.
#[derive(Debug)]
pub struct ScreenshotClient<S> {
    pub(crate) stream: Option<S>,
    usable: bool,
    operation_timeout: Option<Duration>,
}

impl<S> ScreenshotClient<S> {
    pub fn new(stream: S) -> Self {
        Self {
            stream: Some(stream),
            usable: true,
            operation_timeout: None,
        }
    }

    pub fn is_usable(&self) -> bool {
        self.usable && self.stream.is_some()
    }

    pub fn set_operation_timeout(&mut self, timeout: Option<Duration>) {
        self.operation_timeout = timeout;
    }

    pub fn with_operation_timeout(mut self, timeout: Option<Duration>) -> Self {
        self.set_operation_timeout(timeout);
        self
    }

    fn ensure_usable(&self) -> Result<(), ScreenshotError> {
        if self.is_usable() {
            Ok(())
        } else {
            Err(ScreenshotError::Protocol(
                "screenshot connection is unusable; reconnect before another request".into(),
            ))
        }
    }

    fn deadline(&self) -> Result<Option<Instant>, ScreenshotError> {
        self.ensure_usable()?;
        match self.operation_timeout {
            Some(budget) if budget.is_zero() => Err(elapsed_budget()),
            Some(budget) => Instant::now().checked_add(budget).map(Some).ok_or_else(|| {
                std::io::Error::new(
                    std::io::ErrorKind::InvalidInput,
                    "screenshot operation deadline is too large",
                )
                .into()
            }),
            None => Ok(None),
        }
    }

    fn begin_request(
        &mut self,
        deadline: Option<Instant>,
    ) -> Result<ScreenshotRequest<'_, S>, ScreenshotError> {
        self.ensure_usable()?;
        let stream = self.stream.take();
        Ok(ScreenshotRequest {
            client: self,
            stream,
            deadline,
            progressed: false,
            failed: false,
        })
    }
}

impl<S: AsyncRead + AsyncWrite + Unpin> ScreenshotClient<S> {
    /// Capture a screenshot from the device.
    ///
    /// Version exchange, DeviceReady, and screenshot reception execute under
    /// a single shared operation deadline.
    pub async fn take_screenshot(&mut self) -> Result<ScreenshotImage, ScreenshotError> {
        let deadline = self.deadline()?;
        check_deadline(deadline, Ok(()))?;
        let mut request = self.begin_request(deadline)?;
        let result = await_operation(deadline, async {
            let mut io = request.io();
            // 1. Send version exchange
            send_plist(
                &mut io,
                &VersionExchangeRequest {
                    message_type: "DLMessageVersionExchange",
                    supported_versions: vec![1],
                },
            )
            .await?;

            // 2. Recv version exchange response (ignore content)
            recv_plist_raw(&mut io).await?;

            // 3. Send device ready
            send_plist(
                &mut io,
                &DeviceReadyRequest {
                    message_type: "DLMessageDeviceReady",
                },
            )
            .await?;

            // 4. Recv screenshot plist
            let data = recv_plist_raw(&mut io).await?;
            parse_screenshot_image(&data)
        })
        .await;
        request.finish(check_deadline(deadline, result))
    }
}

struct ScreenshotRequest<'a, S> {
    client: &'a mut ScreenshotClient<S>,
    stream: Option<S>,
    deadline: Option<Instant>,
    progressed: bool,
    failed: bool,
}

impl<S> ScreenshotRequest<'_, S> {
    fn io(&mut self) -> ScreenshotIo<'_, S> {
        ScreenshotIo {
            inner: self.stream.as_mut().expect("active screenshot stream"),
            deadline: self.deadline,
            progressed: &mut self.progressed,
            failed: &mut self.failed,
        }
    }

    fn finish<T>(mut self, result: Result<T, ScreenshotError>) -> Result<T, ScreenshotError> {
        if !self.failed && (result.is_ok() || !self.progressed) {
            self.client.stream = self.stream.take();
        } else {
            self.client.usable = false;
        }
        result
    }
}

impl<S> Drop for ScreenshotRequest<'_, S> {
    fn drop(&mut self) {
        if self.stream.is_none() {
            return;
        }
        if self.progressed || self.failed {
            self.client.usable = false;
        } else {
            self.client.stream = self.stream.take();
        }
    }
}

struct ScreenshotIo<'a, S> {
    inner: &'a mut S,
    deadline: Option<Instant>,
    progressed: &'a mut bool,
    failed: &'a mut bool,
}

impl<S> ScreenshotIo<'_, S> {
    fn expired(&self) -> bool {
        self.deadline
            .is_some_and(|deadline| Instant::now() >= deadline)
    }
}

impl<S: AsyncRead + Unpin> AsyncRead for ScreenshotIo<'_, S> {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<std::io::Result<()>> {
        if self.expired() {
            return Poll::Pending;
        }
        let before = buf.filled().len();
        let requested = buf.remaining() > 0;
        let result = Pin::new(&mut *self.inner).poll_read(cx, buf);
        *self.progressed |= buf.filled().len() > before;
        *self.failed |= matches!(result, Poll::Ready(Err(_)))
            || (requested && matches!(result, Poll::Ready(Ok(()))) && buf.filled().len() == before);
        result
    }
}

impl<S: AsyncWrite + Unpin> AsyncWrite for ScreenshotIo<'_, S> {
    fn poll_write(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        bytes: &[u8],
    ) -> Poll<std::io::Result<usize>> {
        if self.expired() {
            return Poll::Pending;
        }
        let result = Pin::new(&mut *self.inner).poll_write(cx, bytes);
        *self.progressed |= matches!(result, Poll::Ready(Ok(n)) if n > 0);
        *self.failed |= matches!(result, Poll::Ready(Err(_)))
            || (!bytes.is_empty() && matches!(result, Poll::Ready(Ok(0))));
        result
    }

    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        if self.expired() {
            return Poll::Pending;
        }
        let result = Pin::new(&mut *self.inner).poll_flush(cx);
        *self.failed |= matches!(result, Poll::Ready(Err(_)));
        result
    }

    fn poll_shutdown(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        if self.expired() {
            return Poll::Pending;
        }
        let result = Pin::new(&mut *self.inner).poll_shutdown(cx);
        *self.failed |= matches!(result, Poll::Ready(Err(_)));
        result
    }
}

fn elapsed_budget() -> ScreenshotError {
    std::io::Error::new(
        std::io::ErrorKind::TimedOut,
        "screenshot operation budget elapsed",
    )
    .into()
}

fn check_deadline<T>(
    deadline: Option<Instant>,
    result: Result<T, ScreenshotError>,
) -> Result<T, ScreenshotError> {
    if result.is_ok() && deadline.is_some_and(|deadline| Instant::now() >= deadline) {
        Err(elapsed_budget())
    } else {
        result
    }
}

async fn await_operation<T>(
    deadline: Option<Instant>,
    operation: impl Future<Output = Result<T, ScreenshotError>>,
) -> Result<T, ScreenshotError> {
    match deadline {
        Some(deadline) => tokio::time::timeout_at(deadline, operation)
            .await
            .map_err(|error| {
                ScreenshotError::Io(crate::error::context::retained_io(
                    std::io::ErrorKind::TimedOut,
                    "screenshot operation deadline",
                    error,
                ))
            })?,
        None => operation.await,
    }
}

fn parse_screenshot_image(data: &[u8]) -> Result<ScreenshotImage, ScreenshotError> {
    let val: plist::Value = plist::from_bytes(data)?;
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

/// Capture a screenshot from the device.
///
/// Returns raw image bytes plus detected format metadata.
pub async fn take_screenshot<S>(stream: &mut S) -> Result<ScreenshotImage, ScreenshotError>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    let mut client = ScreenshotClient::new(stream);
    client.take_screenshot().await
}

// ── plist framing (same as lockdown: 4-byte BE length prefix) ─────────────────

async fn send_plist<S, T>(stream: &mut S, value: &T) -> Result<(), ScreenshotError>
where
    S: AsyncWrite + Unpin,
    T: Serialize,
{
    let mut buf = Vec::new();
    plist::to_writer_xml(&mut buf, value)?;
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
    use std::time::Duration;
    use tokio::io::AsyncReadExt;

    use super::*;

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
    fn detects_heif_file_type_box() {
        let mut bytes = [0u8; 12];
        bytes[4..8].copy_from_slice(b"ftyp");
        assert_eq!(ScreenshotFormat::detect(&bytes), ScreenshotFormat::Heif);
        assert_eq!(ScreenshotFormat::Heif.mime_type(), "image/heif");
    }

    #[test]
    fn unknown_signature_falls_back_to_octet_stream() {
        let image = ScreenshotImage::from_bytes(Bytes::from_static(b"not-an-image"));
        assert_eq!(image.format, ScreenshotFormat::Unknown);
        assert_eq!(image.mime_type(), "application/octet-stream");
        assert_eq!(image.byte_len(), 12);
    }

    #[tokio::test]
    async fn screenshot_client_zero_budget_fails_without_io_and_retains_stream() {
        let stream = crate::test_util::MockStream::eof();
        let mut client = ScreenshotClient::new(stream);
        client.set_operation_timeout(Some(Duration::ZERO));
        assert!(client.is_usable());

        let err = client.take_screenshot().await.unwrap_err();
        assert_eq!(crate::classify_error(&err), crate::ErrorClass::Timeout);
        assert!(client.is_usable());
        assert!(client.stream.is_some());
    }

    #[tokio::test]
    async fn screenshot_client_handshake_shares_one_deadline_and_succeeds() {
        // Step 2 response (version exchange ack)
        let version_resp = plist::Value::Dictionary(plist::Dictionary::new());
        // Step 4 response (screenshot payload)
        let png_bytes = vec![0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A, 1, 2, 3];
        let screenshot_resp = plist::Value::Array(vec![
            plist::Value::String("DLMessage".into()),
            plist::Value::Dictionary(plist::Dictionary::from_iter([(
                "ScreenShotData".to_string(),
                plist::Value::Data(png_bytes.clone()),
            )])),
        ]);

        let stream =
            crate::test_util::MockStream::with_responses(vec![version_resp, screenshot_resp]);
        let mut client = ScreenshotClient::new(stream);
        client.set_operation_timeout(Some(Duration::from_secs(5)));
        let image = client.take_screenshot().await.unwrap();
        assert_eq!(image.format, ScreenshotFormat::Png);
        assert_eq!(image.data.as_ref(), &png_bytes[..]);
        assert!(client.is_usable());
    }

    #[tokio::test]
    async fn screenshot_client_progress_timeout_retires_stream() {
        let (client_side, mut server_side) = tokio::io::duplex(1024);
        let mut client = ScreenshotClient::new(client_side);
        client.set_operation_timeout(Some(Duration::from_millis(50)));

        tokio::spawn(async move {
            // Read version exchange request, then stall
            let mut buf = [0u8; 4];
            let _ = server_side.read_exact(&mut buf).await;
            tokio::time::sleep(Duration::from_millis(200)).await;
        });

        let err = client.take_screenshot().await.unwrap_err();
        assert_eq!(crate::classify_error(&err), crate::ErrorClass::Timeout);
        assert!(!client.is_usable());
        assert!(client.stream.is_none());
    }
}
