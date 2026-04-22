//! Wire protocol for `com.apple.mobile.mobile_image_mounter`.
//!
//! Commands: LookupImage, ReceiveBytes, MountImage, QueryPersonalizationIdentifiers,
//!           QueryPersonalizationManifest (QueryNonce), Hangup
//!
//! Reference: go-ios/ios/imagemounter/imagemounter.go

use std::collections::HashMap;

use plist::Value;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

pub const SERVICE_NAME: &str = "com.apple.mobile.mobile_image_mounter";

#[derive(Debug, thiserror::Error)]
pub enum ImageMounterError {
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
    #[error("plist error: {0}")]
    Plist(String),
    #[error("protocol error: {0}")]
    Protocol(String),
    #[error("device error: {0}")]
    DeviceError(String),
    #[error("TSS error: {0}")]
    Tss(String),
    #[error("download error: {0}")]
    Download(String),
}

/// High-level image mounter client.
pub struct ImageMounterClient<S> {
    stream: S,
}

impl<S: AsyncRead + AsyncWrite + Unpin + Send> ImageMounterClient<S> {
    pub fn new(stream: S) -> Self {
        Self { stream }
    }

    /// Return raw mounted image entries reported by mobile_image_mounter.
    pub async fn copy_devices(&mut self) -> Result<Vec<plist::Dictionary>, ImageMounterError> {
        let req = plist::Dictionary::from_iter([(
            "Command".to_string(),
            Value::String("CopyDevices".into()),
        )]);
        send_plist(&mut self.stream, &Value::Dictionary(req)).await?;
        let resp = recv_plist(&mut self.stream).await?;
        check_error(&resp)?;

        match resp.get("EntryList") {
            Some(Value::Array(items)) => items
                .iter()
                .map(|value| {
                    value.as_dictionary().cloned().ok_or_else(|| {
                        ImageMounterError::Protocol("CopyDevices entry was not a dictionary".into())
                    })
                })
                .collect(),
            None => Ok(Vec::new()),
            Some(_) => Err(ImageMounterError::Protocol(
                "CopyDevices EntryList had unexpected type".into(),
            )),
        }
    }

    /// Check if a developer image is already mounted.
    pub async fn is_image_mounted(&mut self) -> Result<bool, ImageMounterError> {
        Ok(!self.lookup_image_signatures("Developer").await?.is_empty()
            || !self
                .lookup_image_signatures("Personalized")
                .await?
                .is_empty())
    }

    /// Return mounted image signatures for an image type.
    pub async fn lookup_image_signatures(
        &mut self,
        image_type: &str,
    ) -> Result<Vec<Vec<u8>>, ImageMounterError> {
        let req = plist::Dictionary::from_iter([
            ("Command".to_string(), Value::String("LookupImage".into())),
            ("ImageType".to_string(), Value::String(image_type.into())),
        ]);
        send_plist(&mut self.stream, &Value::Dictionary(req)).await?;
        let resp = recv_plist(&mut self.stream).await?;
        check_error(&resp)?;

        match resp.get("ImageSignature") {
            Some(Value::Array(items)) => items
                .iter()
                .map(|value| {
                    value.as_data().map(|bytes| bytes.to_vec()).ok_or_else(|| {
                        ImageMounterError::Protocol(
                            "LookupImage ImageSignature entry was not data".into(),
                        )
                    })
                })
                .collect(),
            Some(Value::Data(bytes)) => Ok(vec![bytes.clone()]),
            None => Ok(Vec::new()),
            Some(_) => Err(ImageMounterError::Protocol(
                "LookupImage ImageSignature had unexpected type".into(),
            )),
        }
    }

    /// Mount a standard (pre-iOS 17) developer disk image.
    ///
    /// `image_bytes`: the DeveloperDiskImage.dmg contents
    /// `signature`: the DeveloperDiskImage.dmg.signature contents
    pub async fn mount_standard(
        &mut self,
        image_bytes: &[u8],
        signature: &[u8],
    ) -> Result<(), ImageMounterError> {
        // 1. Upload image via ReceiveBytes
        self.upload_image(image_bytes, signature).await?;

        // 2. Mount
        let mount_req = plist::Dictionary::from_iter([
            ("Command".to_string(), Value::String("MountImage".into())),
            ("ImageType".to_string(), Value::String("Developer".into())),
            (
                "ImagePath".to_string(),
                Value::String("/private/var/mobile/Media/PublicStaging/staging.dimage".into()),
            ),
            (
                "ImageSignature".to_string(),
                Value::Data(signature.to_vec()),
            ),
        ]);
        send_plist(&mut self.stream, &Value::Dictionary(mount_req)).await?;
        let resp = recv_plist(&mut self.stream).await?;
        check_error(&resp)?;
        Ok(())
    }

    /// Mount a personalized (iOS 17+) developer disk image.
    ///
    /// `trustcache`: the trust cache data
    /// `build_manifest`: the BuildManifest.plist bytes
    /// `image_bytes`: the personalized disk image
    /// `ticket`: the TSS ticket (ApImg4Ticket)
    pub async fn mount_personalized(
        &mut self,
        trustcache: &[u8],
        build_manifest: &[u8],
        image_bytes: &[u8],
        ticket: &[u8],
    ) -> Result<(), ImageMounterError> {
        // 1. Query personalization identifiers
        let ids = self.query_personalization_identifiers().await?;
        tracing::debug!(
            "personalization identifiers: {:?}",
            ids.keys().collect::<Vec<_>>()
        );

        // 2. Query nonce
        let nonce = self.query_nonce().await?;
        tracing::debug!("personalization nonce: {} bytes", nonce.len());

        // 3. Upload image
        self.upload_personalized_image(image_bytes, trustcache, build_manifest)
            .await?;

        // 4. Mount with ticket
        let mount_req = plist::Dictionary::from_iter([
            ("Command".to_string(), Value::String("MountImage".into())),
            (
                "ImageType".to_string(),
                Value::String("Personalized".into()),
            ),
            (
                "ImagePath".to_string(),
                Value::String("/private/var/mobile/Media/PublicStaging/staging.dimage".into()),
            ),
            ("ImageSignature".to_string(), Value::Data(ticket.to_vec())),
        ]);
        send_plist(&mut self.stream, &Value::Dictionary(mount_req)).await?;
        let resp = recv_plist(&mut self.stream).await?;
        check_error(&resp)?;
        Ok(())
    }

    /// Query personalization identifiers (board ID, chip ID, etc.)
    pub async fn query_personalization_identifiers(
        &mut self,
    ) -> Result<HashMap<String, Value>, ImageMounterError> {
        self.query_personalization_identifiers_with_type("DeveloperDiskImage")
            .await
    }

    /// Query personalization identifiers for a specific personalized image type.
    pub async fn query_personalization_identifiers_with_type(
        &mut self,
        personalized_image_type: &str,
    ) -> Result<HashMap<String, Value>, ImageMounterError> {
        let req = plist::Dictionary::from_iter([
            (
                "Command".to_string(),
                Value::String("QueryPersonalizationIdentifiers".into()),
            ),
            (
                "PersonalizedImageType".to_string(),
                Value::String(personalized_image_type.into()),
            ),
        ]);
        send_plist(&mut self.stream, &Value::Dictionary(req)).await?;
        let resp = recv_plist(&mut self.stream).await?;
        check_error(&resp)?;

        let ids = resp
            .get("PersonalizationIdentifiers")
            .and_then(|v| v.as_dictionary())
            .ok_or_else(|| {
                ImageMounterError::Protocol("missing PersonalizationIdentifiers".into())
            })?;

        Ok(ids.iter().map(|(k, v)| (k.clone(), v.clone())).collect())
    }

    /// Query the personalization manifest associated with a mounted personalized image.
    pub async fn query_personalization_manifest(
        &mut self,
        personalized_image_type: &str,
        image_signature: &[u8],
    ) -> Result<Vec<u8>, ImageMounterError> {
        let req = plist::Dictionary::from_iter([
            (
                "Command".to_string(),
                Value::String("QueryPersonalizationManifest".into()),
            ),
            (
                "PersonalizedImageType".to_string(),
                Value::String(personalized_image_type.into()),
            ),
            (
                "ImageType".to_string(),
                Value::String(personalized_image_type.into()),
            ),
            (
                "ImageSignature".to_string(),
                Value::Data(image_signature.to_vec()),
            ),
        ]);
        send_plist(&mut self.stream, &Value::Dictionary(req)).await?;
        let resp = recv_plist(&mut self.stream).await?;
        check_error(&resp)?;

        let manifest = resp
            .get("ImageSignature")
            .and_then(|v| v.as_data())
            .ok_or_else(|| ImageMounterError::Protocol("missing ImageSignature".into()))?;

        Ok(manifest.to_vec())
    }

    /// Query the personalization nonce.
    pub async fn query_nonce(&mut self) -> Result<Vec<u8>, ImageMounterError> {
        self.query_nonce_with_type("DeveloperDiskImage").await
    }

    /// Query the personalization nonce for a specific personalized image type.
    pub async fn query_nonce_with_type(
        &mut self,
        personalized_image_type: &str,
    ) -> Result<Vec<u8>, ImageMounterError> {
        let req = plist::Dictionary::from_iter([
            ("Command".to_string(), Value::String("QueryNonce".into())),
            (
                "PersonalizedImageType".to_string(),
                Value::String(personalized_image_type.into()),
            ),
        ]);
        send_plist(&mut self.stream, &Value::Dictionary(req)).await?;
        let resp = recv_plist(&mut self.stream).await?;
        check_error(&resp)?;

        let nonce = resp
            .get("PersonalizationNonce")
            .and_then(|v| v.as_data())
            .ok_or_else(|| ImageMounterError::Protocol("missing PersonalizationNonce".into()))?;

        Ok(nonce.to_vec())
    }

    /// Query whether developer mode is enabled on the device.
    pub async fn query_developer_mode_status(&mut self) -> Result<bool, ImageMounterError> {
        let req = plist::Dictionary::from_iter([(
            "Command".to_string(),
            Value::String("QueryDeveloperModeStatus".into()),
        )]);
        send_plist(&mut self.stream, &Value::Dictionary(req)).await?;
        let resp = recv_plist(&mut self.stream).await?;
        check_error(&resp)?;

        Ok(resp
            .get("DeveloperModeStatus")
            .and_then(|v| v.as_boolean())
            .unwrap_or(false))
    }

    /// Unmount a mounted image at a mount path such as `/Developer` or `/System/Developer`.
    pub async fn unmount_image(&mut self, mount_path: &str) -> Result<(), ImageMounterError> {
        let req = plist::Dictionary::from_iter([
            ("Command".to_string(), Value::String("UnmountImage".into())),
            ("MountPath".to_string(), Value::String(mount_path.into())),
        ]);
        send_plist(&mut self.stream, &Value::Dictionary(req)).await?;
        let resp = recv_plist(&mut self.stream).await?;
        check_error(&resp)?;
        Ok(())
    }

    async fn upload_image(
        &mut self,
        image_bytes: &[u8],
        signature: &[u8],
    ) -> Result<(), ImageMounterError> {
        let req = plist::Dictionary::from_iter([
            ("Command".to_string(), Value::String("ReceiveBytes".into())),
            ("ImageType".to_string(), Value::String("Developer".into())),
            (
                "ImageSize".to_string(),
                Value::Integer((image_bytes.len() as i64).into()),
            ),
            (
                "ImageSignature".to_string(),
                Value::Data(signature.to_vec()),
            ),
        ]);
        send_plist(&mut self.stream, &Value::Dictionary(req)).await?;
        let resp = recv_plist(&mut self.stream).await?;

        let status = resp.get("Status").and_then(|v| v.as_string()).unwrap_or("");

        if status == "ReceiveBytesAck" {
            // Device wants the image bytes
            self.stream.write_all(image_bytes).await?;
            self.stream.flush().await?;
            let resp2 = recv_plist(&mut self.stream).await?;
            check_error(&resp2)?;
        } else {
            check_error(&resp)?;
        }
        Ok(())
    }

    async fn upload_personalized_image(
        &mut self,
        image_bytes: &[u8],
        trustcache: &[u8],
        build_manifest: &[u8],
    ) -> Result<(), ImageMounterError> {
        let req = plist::Dictionary::from_iter([
            ("Command".to_string(), Value::String("ReceiveBytes".into())),
            (
                "ImageType".to_string(),
                Value::String("Personalized".into()),
            ),
            (
                "ImageSize".to_string(),
                Value::Integer((image_bytes.len() as i64).into()),
            ),
            (
                "ImageTrustCache".to_string(),
                Value::Data(trustcache.to_vec()),
            ),
            (
                "BuildManifest".to_string(),
                Value::Data(build_manifest.to_vec()),
            ),
        ]);
        send_plist(&mut self.stream, &Value::Dictionary(req)).await?;
        let resp = recv_plist(&mut self.stream).await?;

        let status = resp.get("Status").and_then(|v| v.as_string()).unwrap_or("");

        if status == "ReceiveBytesAck" {
            self.stream.write_all(image_bytes).await?;
            self.stream.flush().await?;
            let resp2 = recv_plist(&mut self.stream).await?;
            check_error(&resp2)?;
        } else {
            check_error(&resp)?;
        }
        Ok(())
    }

    /// Send Hangup to close the session.
    pub async fn hangup(&mut self) -> Result<(), ImageMounterError> {
        let req =
            plist::Dictionary::from_iter([("Command".to_string(), Value::String("Hangup".into()))]);
        send_plist(&mut self.stream, &Value::Dictionary(req)).await?;
        Ok(())
    }
}

fn check_error(resp: &plist::Dictionary) -> Result<(), ImageMounterError> {
    if let Some(err) = resp.get("Error") {
        let msg = err.as_string().unwrap_or("unknown error");
        return Err(ImageMounterError::DeviceError(msg.to_string()));
    }
    Ok(())
}

// ── plist framing (4-byte BE length prefix) ──────────────────────────────────

async fn send_plist<S: AsyncWrite + Unpin>(
    stream: &mut S,
    value: &Value,
) -> Result<(), ImageMounterError> {
    let mut buf = Vec::new();
    plist::to_writer_xml(&mut buf, value).map_err(|e| ImageMounterError::Plist(e.to_string()))?;
    stream.write_all(&(buf.len() as u32).to_be_bytes()).await?;
    stream.write_all(&buf).await?;
    stream.flush().await?;
    Ok(())
}

async fn recv_plist<S: AsyncRead + Unpin>(
    stream: &mut S,
) -> Result<plist::Dictionary, ImageMounterError> {
    let mut len_buf = [0u8; 4];
    stream.read_exact(&mut len_buf).await?;
    let len = u32::from_be_bytes(len_buf) as usize;
    const MAX_PLIST_SIZE: usize = 4 * 1024 * 1024;
    if len > MAX_PLIST_SIZE {
        return Err(ImageMounterError::Protocol(format!(
            "plist length {len} exceeds maximum of {MAX_PLIST_SIZE}"
        )));
    }
    let mut buf = vec![0u8; len];
    stream.read_exact(&mut buf).await?;
    let val: plist::Value =
        plist::from_bytes(&buf).map_err(|e| ImageMounterError::Plist(e.to_string()))?;
    val.into_dictionary()
        .ok_or_else(|| ImageMounterError::Protocol("expected plist dictionary".into()))
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
        written: Vec<u8>,
        read_pos: usize,
    }

    impl MockStream {
        fn with_response(value: Value) -> Self {
            let mut payload = Vec::new();
            plist::to_writer_xml(&mut payload, &value).unwrap();
            let mut read_buf = Vec::new();
            read_buf.extend_from_slice(&(payload.len() as u32).to_be_bytes());
            read_buf.extend_from_slice(&payload);
            Self {
                read_buf,
                written: Vec::new(),
                read_pos: 0,
            }
        }

        fn with_responses(values: Vec<Value>) -> Self {
            let mut read_buf = Vec::new();
            for value in values {
                let mut payload = Vec::new();
                plist::to_writer_xml(&mut payload, &value).unwrap();
                read_buf.extend_from_slice(&(payload.len() as u32).to_be_bytes());
                read_buf.extend_from_slice(&payload);
            }
            Self {
                read_buf,
                written: Vec::new(),
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
            mut self: Pin<&mut Self>,
            _cx: &mut Context<'_>,
            buf: &[u8],
        ) -> Poll<std::io::Result<usize>> {
            self.written.extend_from_slice(buf);
            Poll::Ready(Ok(buf.len()))
        }

        fn poll_flush(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
            Poll::Ready(Ok(()))
        }

        fn poll_shutdown(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
            Poll::Ready(Ok(()))
        }
    }

    #[tokio::test]
    async fn query_developer_mode_status_roundtrips_boolean() {
        let response = Value::Dictionary(plist::Dictionary::from_iter([(
            "DeveloperModeStatus".to_string(),
            Value::Boolean(true),
        )]));
        let mut stream = MockStream::with_response(response);
        let mut client = ImageMounterClient::new(&mut stream);

        let enabled = client.query_developer_mode_status().await.unwrap();
        assert!(enabled);

        let len = u32::from_be_bytes(stream.written[..4].try_into().unwrap()) as usize;
        let payload = &stream.written[4..4 + len];
        let dict: plist::Dictionary = plist::from_bytes(payload).unwrap();
        assert_eq!(
            dict.get("Command").and_then(|v| v.as_string()),
            Some("QueryDeveloperModeStatus")
        );
    }

    #[tokio::test]
    async fn lookup_image_signatures_roundtrips_data_array() {
        let response = Value::Dictionary(plist::Dictionary::from_iter([(
            "ImageSignature".to_string(),
            Value::Array(vec![Value::Data(vec![0xde, 0xad, 0xbe, 0xef])]),
        )]));
        let mut stream = MockStream::with_response(response);
        let mut client = ImageMounterClient::new(&mut stream);

        let signatures = client.lookup_image_signatures("Developer").await.unwrap();
        assert_eq!(signatures, vec![vec![0xde, 0xad, 0xbe, 0xef]]);

        let len = u32::from_be_bytes(stream.written[..4].try_into().unwrap()) as usize;
        let payload = &stream.written[4..4 + len];
        let dict: plist::Dictionary = plist::from_bytes(payload).unwrap();
        assert_eq!(
            dict.get("Command").and_then(|v| v.as_string()),
            Some("LookupImage")
        );
        assert_eq!(
            dict.get("ImageType").and_then(|v| v.as_string()),
            Some("Developer")
        );
    }

    #[tokio::test]
    async fn is_image_mounted_checks_both_image_types() {
        let mut stream = MockStream::with_responses(vec![
            Value::Dictionary(plist::Dictionary::new()),
            Value::Dictionary(plist::Dictionary::from_iter([(
                "ImageSignature".to_string(),
                Value::Array(vec![Value::Data(vec![1, 2, 3])]),
            )])),
        ]);
        let mut client = ImageMounterClient::new(&mut stream);

        let mounted = client.is_image_mounted().await.unwrap();
        assert!(mounted);
    }

    #[tokio::test]
    async fn unmount_image_sends_mount_path() {
        let response = Value::Dictionary(plist::Dictionary::new());
        let mut stream = MockStream::with_response(response);
        let mut client = ImageMounterClient::new(&mut stream);

        client.unmount_image("/System/Developer").await.unwrap();

        let len = u32::from_be_bytes(stream.written[..4].try_into().unwrap()) as usize;
        let payload = &stream.written[4..4 + len];
        let dict: plist::Dictionary = plist::from_bytes(payload).unwrap();
        assert_eq!(
            dict.get("Command").and_then(|v| v.as_string()),
            Some("UnmountImage")
        );
        assert_eq!(
            dict.get("MountPath").and_then(|v| v.as_string()),
            Some("/System/Developer")
        );
    }

    #[tokio::test]
    async fn query_nonce_uses_query_nonce_command_and_personalization_nonce() {
        let response = Value::Dictionary(plist::Dictionary::from_iter([(
            "PersonalizationNonce".to_string(),
            Value::Data(vec![0xde, 0xad, 0xbe, 0xef]),
        )]));
        let mut stream = MockStream::with_response(response);
        let mut client = ImageMounterClient::new(&mut stream);

        let nonce = client.query_nonce().await.unwrap();
        assert_eq!(nonce, vec![0xde, 0xad, 0xbe, 0xef]);

        let len = u32::from_be_bytes(stream.written[..4].try_into().unwrap()) as usize;
        let payload = &stream.written[4..4 + len];
        let dict: plist::Dictionary = plist::from_bytes(payload).unwrap();
        assert_eq!(
            dict.get("Command").and_then(|v| v.as_string()),
            Some("QueryNonce")
        );
        assert_eq!(
            dict.get("PersonalizedImageType")
                .and_then(|v| v.as_string()),
            Some("DeveloperDiskImage")
        );
    }

    #[tokio::test]
    async fn copy_devices_roundtrips_entry_list() {
        let response = Value::Dictionary(plist::Dictionary::from_iter([(
            "EntryList".to_string(),
            Value::Array(vec![Value::Dictionary(plist::Dictionary::from_iter([
                (
                    "ImageType".to_string(),
                    Value::String("Personalized".into()),
                ),
                ("ImageSignature".to_string(), Value::Data(vec![0xaa, 0xbb])),
            ]))]),
        )]));
        let mut stream = MockStream::with_response(response);
        let mut client = ImageMounterClient::new(&mut stream);

        let entries = client.copy_devices().await.unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(
            entries[0].get("ImageType").and_then(|v| v.as_string()),
            Some("Personalized")
        );
        assert_eq!(
            entries[0].get("ImageSignature").and_then(|v| v.as_data()),
            Some([0xaa, 0xbb].as_slice())
        );

        let len = u32::from_be_bytes(stream.written[..4].try_into().unwrap()) as usize;
        let payload = &stream.written[4..4 + len];
        let dict: plist::Dictionary = plist::from_bytes(payload).unwrap();
        assert_eq!(
            dict.get("Command").and_then(|v| v.as_string()),
            Some("CopyDevices")
        );
    }

    #[tokio::test]
    async fn query_personalization_manifest_roundtrips_request_and_manifest_bytes() {
        let response = Value::Dictionary(plist::Dictionary::from_iter([(
            "ImageSignature".to_string(),
            Value::Data(vec![0xfa, 0xce]),
        )]));
        let mut stream = MockStream::with_response(response);
        let mut client = ImageMounterClient::new(&mut stream);

        let manifest = client
            .query_personalization_manifest("DeveloperDiskImage", &[0xaa, 0xbb])
            .await
            .unwrap();
        assert_eq!(manifest, vec![0xfa, 0xce]);

        let len = u32::from_be_bytes(stream.written[..4].try_into().unwrap()) as usize;
        let payload = &stream.written[4..4 + len];
        let dict: plist::Dictionary = plist::from_bytes(payload).unwrap();
        assert_eq!(
            dict.get("Command").and_then(|v| v.as_string()),
            Some("QueryPersonalizationManifest")
        );
        assert_eq!(
            dict.get("PersonalizedImageType")
                .and_then(|v| v.as_string()),
            Some("DeveloperDiskImage")
        );
        assert_eq!(
            dict.get("ImageType").and_then(|v| v.as_string()),
            Some("DeveloperDiskImage")
        );
        assert_eq!(
            dict.get("ImageSignature").and_then(|v| v.as_data()),
            Some([0xaa, 0xbb].as_slice())
        );
    }

    #[tokio::test]
    async fn query_nonce_with_custom_image_type_uses_provided_personalized_image_type() {
        let response = Value::Dictionary(plist::Dictionary::from_iter([(
            "PersonalizationNonce".to_string(),
            Value::Data(vec![0xde, 0xad]),
        )]));
        let mut stream = MockStream::with_response(response);
        let mut client = ImageMounterClient::new(&mut stream);

        let nonce = client.query_nonce_with_type("Cryptex").await.unwrap();
        assert_eq!(nonce, vec![0xde, 0xad]);

        let len = u32::from_be_bytes(stream.written[..4].try_into().unwrap()) as usize;
        let payload = &stream.written[4..4 + len];
        let dict: plist::Dictionary = plist::from_bytes(payload).unwrap();
        assert_eq!(
            dict.get("Command").and_then(|v| v.as_string()),
            Some("QueryNonce")
        );
        assert_eq!(
            dict.get("PersonalizedImageType")
                .and_then(|v| v.as_string()),
            Some("Cryptex")
        );
    }

    #[tokio::test]
    async fn query_personalization_identifiers_with_custom_type_uses_provided_image_type() {
        let response = Value::Dictionary(plist::Dictionary::from_iter([(
            "PersonalizationIdentifiers".to_string(),
            Value::Dictionary(plist::Dictionary::from_iter([(
                "BoardId".to_string(),
                Value::Integer(12.into()),
            )])),
        )]));
        let mut stream = MockStream::with_response(response);
        let mut client = ImageMounterClient::new(&mut stream);

        let identifiers = client
            .query_personalization_identifiers_with_type("Cryptex")
            .await
            .unwrap();
        assert_eq!(
            identifiers
                .get("BoardId")
                .and_then(|v| v.as_unsigned_integer()),
            Some(12)
        );

        let len = u32::from_be_bytes(stream.written[..4].try_into().unwrap()) as usize;
        let payload = &stream.written[4..4 + len];
        let dict: plist::Dictionary = plist::from_bytes(payload).unwrap();
        assert_eq!(
            dict.get("Command").and_then(|v| v.as_string()),
            Some("QueryPersonalizationIdentifiers")
        );
        assert_eq!(
            dict.get("PersonalizedImageType")
                .and_then(|v| v.as_string()),
            Some("Cryptex")
        );
    }
}
