//! iOS 17+ CoreDevice screenshot service.
//!
//! This service is distinct from the legacy lockdown `screenshotr` service and
//! from the DTX Instruments screenshot helper.  It returns the image bytes
//! directly in the CoreDevice output dictionary together with display and
//! format metadata.

use bytes::Bytes;
use indexmap::IndexMap;

use crate::xpc::{XpcClient, XpcError, XpcMessage, XpcValue};

/// RSD service name for CoreDevice screen capture.
pub const SERVICE_NAME: &str = "com.apple.coredevice.screencaptureservice";
/// Feature identifier used by screenshot requests.
pub const CAPTURE_SCREENSHOT_FEATURE: &str = "com.apple.coredevice.feature.capturescreenshot";
/// Compatibility spelling used by other CoreDevice service modules.
pub const FEATURE_CAPTURE_SCREENSHOT: &str = CAPTURE_SCREENSHOT_FEATURE;
/// Action identifier used by screenshot requests.
pub const CAPTURE_SCREENSHOT_ACTION: &str = "com.apple.coredevice.action.capturescreenshot";

/// Maximum image payload accepted from CoreDevice.
///
/// This is intentionally larger than normal screenshots to allow high-density
/// HEIF output, while still bounding a malformed XPC response before callers
/// write it to disk.
pub const MAX_SCREENSHOT_BYTES: usize = 64 * 1024 * 1024;

/// Image returned by the CoreDevice screenshot service.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScreenCaptureImage {
    /// Raw image bytes (PNG, HEIF/HEIC, JPEG, or another device-supported format).
    pub data: Bytes,
    /// Device display identifier, when the daemon returned one.
    pub display_unique_id: Option<String>,
    /// Device-reported image format (usually `png`).
    pub image_format: String,
    /// Pixel dimensions when the daemon includes them in its response.
    pub pixel_size: Option<(u64, u64)>,
}

impl ScreenCaptureImage {
    /// Return a stable MIME type for the reported image format.
    pub fn mime_type(&self) -> &'static str {
        image_format_mime_type(&self.image_format)
    }

    /// Number of bytes in the image payload.
    pub fn byte_len(&self) -> usize {
        self.data.len()
    }
}

/// Errors returned by CoreDevice screen capture.
#[derive(Debug, thiserror::Error)]
pub enum ScreenCaptureError {
    /// Underlying RemoteXPC transport failure.
    #[error("xpc error: {0}")]
    Xpc(#[from] XpcError),
    /// The daemon returned an invalid response or unsupported argument.
    #[error("screen capture protocol error: {0}")]
    Protocol(String),
    /// ScreenCaptureService only has a modern CoreDevice envelope.
    #[error("CoreDevice screen capture requires the modern envelope; legacy mode is unsupported")]
    LegacyUnsupported,
}

impl ScreenCaptureError {
    /// Whether retrying another screenshot transport is meaningful because the
    /// CoreDevice endpoint itself was absent. Protocol and permission errors
    /// deliberately return false so callers cannot mask them with fallback.
    pub fn is_service_unavailable(&self) -> bool {
        matches!(
            self,
            Self::Xpc(XpcError::ServiceNotFound(_)) | Self::Xpc(XpcError::Io(_))
        )
    }
}

pub use crate::services::coredevice::CoreDeviceEnvelopeMode;

/// Client for `com.apple.coredevice.screencaptureservice`.
pub struct ScreenCaptureServiceClient {
    client: XpcClient,
    envelope_mode: CoreDeviceEnvelopeMode,
    service_features: Option<Vec<String>>,
}

/// Compatibility alias matching the upstream service name.
pub type ScreenCaptureService = ScreenCaptureServiceClient;

impl ScreenCaptureServiceClient {
    /// Create a modern CoreDevice screen-capture client.
    pub fn new(client: XpcClient) -> Self {
        Self::new_with_mode(client, CoreDeviceEnvelopeMode::Modern)
    }

    /// Create a modern client retaining the feature list from the resolved RSD
    /// service descriptor.
    pub fn new_with_features<I, S>(client: XpcClient, service_features: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        Self {
            client,
            envelope_mode: CoreDeviceEnvelopeMode::Modern,
            service_features: Some(service_features.into_iter().map(Into::into).collect()),
        }
    }

    /// Create a client with an explicitly selected envelope.
    pub fn new_with_mode(client: XpcClient, envelope_mode: CoreDeviceEnvelopeMode) -> Self {
        Self {
            client,
            envelope_mode,
            service_features: None,
        }
    }

    /// Return whether the feature advertised by this client's resolved RSD
    /// entry can capture a display. Missing/empty metadata remains permissive
    /// for compatibility with older RSD handshakes.
    pub fn supports_capture_screenshot(&self) -> bool {
        feature_is_advertised(self.service_features.as_deref())
    }

    fn ensure_available(&self) -> Result<(), ScreenCaptureError> {
        if self.envelope_mode == CoreDeviceEnvelopeMode::Legacy {
            return Err(ScreenCaptureError::LegacyUnsupported);
        }
        if !self.supports_capture_screenshot() {
            return Err(ScreenCaptureError::Protocol(format!(
                "CoreDevice feature {CAPTURE_SCREENSHOT_FEATURE} is not advertised by RSD"
            )));
        }
        Ok(())
    }

    /// Capture the selected display in the requested format.
    ///
    /// The upstream protocol currently supports PNG. Other formats are
    /// accepted here because newer devices may advertise HEIF/JPEG support;
    /// the returned payload is validated against its actual magic bytes.
    pub async fn capture_screenshot(
        &mut self,
        display_unique_id: Option<&str>,
        requested_format: &str,
    ) -> Result<ScreenCaptureImage, ScreenCaptureError> {
        self.ensure_available()?;
        validate_requested_format(requested_format)?;
        let response = self
            .client
            .call(crate::services::coredevice::build_request_with_action(
                "",
                CAPTURE_SCREENSHOT_FEATURE,
                CAPTURE_SCREENSHOT_ACTION,
                XpcValue::Dictionary(IndexMap::from([
                    (
                        "displayUniqueID".to_string(),
                        display_unique_id
                            .map(|value| XpcValue::String(value.to_string()))
                            .unwrap_or(XpcValue::Null),
                    ),
                    (
                        "requestedFormat".to_string(),
                        // Preserve the caller's spelling on the wire. The
                        // reference client passes this value through
                        // unchanged; validation above is case-insensitive but
                        // must not silently alter an explicitly requested
                        // format.
                        XpcValue::String(requested_format.to_string()),
                    ),
                ])),
            ))
            .await?;
        parse_response(response)
    }
}

fn validate_requested_format(format: &str) -> Result<(), ScreenCaptureError> {
    if !matches!(
        format.to_ascii_lowercase().as_str(),
        "png" | "heif" | "heic" | "jpeg" | "jpg" | "tiff"
    ) {
        return Err(ScreenCaptureError::Protocol(format!(
            "unsupported requested screenshot format {format:?}"
        )));
    }
    Ok(())
}

fn feature_is_advertised(features: Option<&[String]>) -> bool {
    features.map_or(true, |features| {
        features.is_empty()
            || features
                .iter()
                .any(|feature| feature == CAPTURE_SCREENSHOT_FEATURE)
    })
}

fn parse_response(response: XpcMessage) -> Result<ScreenCaptureImage, ScreenCaptureError> {
    let output = crate::services::coredevice::parse_output(response)
        .map_err(ScreenCaptureError::Protocol)?;
    let output = output.as_dict().ok_or_else(|| {
        ScreenCaptureError::Protocol(format!(
            "screen capture output is not a dictionary: {output:?}"
        ))
    })?;
    let data = match output.get("image") {
        Some(XpcValue::Data(data)) if !data.is_empty() => data.clone(),
        Some(other) => {
            return Err(ScreenCaptureError::Protocol(format!(
                "screen capture image is not non-empty data: {other:?}"
            )))
        }
        None => {
            return Err(ScreenCaptureError::Protocol(
                "screen capture output missing image".into(),
            ))
        }
    };
    if data.len() > MAX_SCREENSHOT_BYTES {
        return Err(ScreenCaptureError::Protocol(format!(
            "screen capture image length {} exceeds maximum {MAX_SCREENSHOT_BYTES}",
            data.len()
        )));
    }
    let image_format = output
        .get("imageFormat")
        .and_then(XpcValue::as_str)
        .ok_or_else(|| {
            ScreenCaptureError::Protocol("screen capture output missing imageFormat".into())
        })?
        .to_ascii_lowercase();
    validate_image_magic(&image_format, &data)?;
    let display_unique_id = match output.get("displayUniqueID") {
        Some(XpcValue::String(value)) => Some(value.clone()),
        Some(XpcValue::Null) | None => None,
        Some(other) => {
            return Err(ScreenCaptureError::Protocol(format!(
                "displayUniqueID is not a string or null: {other:?}"
            )))
        }
    };
    let pixel_size = match (
        optional_u64(output, &["pixelWidth", "width"]),
        optional_u64(output, &["pixelHeight", "height"]),
    ) {
        (Some(width), Some(height)) => Some((width, height)),
        (None, None) => None,
        _ => {
            return Err(ScreenCaptureError::Protocol(
                "screen capture output has incomplete pixel dimensions".into(),
            ))
        }
    };

    Ok(ScreenCaptureImage {
        data,
        display_unique_id,
        image_format,
        pixel_size,
    })
}

fn optional_u64(dict: &IndexMap<String, XpcValue>, keys: &[&str]) -> Option<u64> {
    keys.iter().find_map(|key| match dict.get(*key) {
        Some(XpcValue::Uint64(value)) => Some(*value),
        Some(XpcValue::Int64(value)) if *value >= 0 => Some(*value as u64),
        _ => None,
    })
}

fn validate_image_magic(format: &str, data: &[u8]) -> Result<(), ScreenCaptureError> {
    let valid = match format {
        "png" | "public.png" => data.starts_with(&[0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a]),
        "jpeg" | "jpg" | "public.jpeg" => data.starts_with(&[0xff, 0xd8, 0xff]),
        "tiff" | "public.tiff" => data.starts_with(b"II*\0") || data.starts_with(b"MM\0*"),
        "heif" | "heic" | "public.heif" | "public.heic" => {
            data.len() >= 12 && &data[4..8] == b"ftyp"
        }
        _ => false,
    };
    if !valid {
        return Err(ScreenCaptureError::Protocol(format!(
            "screen capture image does not match declared format {format:?}"
        )));
    }
    Ok(())
}

fn image_format_mime_type(format: &str) -> &'static str {
    match format.to_ascii_lowercase().as_str() {
        "png" | "public.png" => "image/png",
        "jpeg" | "jpg" | "public.jpeg" => "image/jpeg",
        "tiff" | "public.tiff" => "image/tiff",
        "heif" | "heic" | "public.heif" | "public.heic" => "image/heif",
        _ => "application/octet-stream",
    }
}

#[cfg(test)]
mod tests {
    use bytes::Bytes;

    use super::*;

    fn response(image: &[u8], format: &str) -> XpcMessage {
        XpcMessage {
            flags: 0,
            msg_id: 1,
            body: Some(XpcValue::Dictionary(IndexMap::from([(
                "CoreDevice.output".into(),
                XpcValue::Dictionary(IndexMap::from([
                    (
                        "image".into(),
                        XpcValue::Data(Bytes::copy_from_slice(image)),
                    ),
                    (
                        "displayUniqueID".into(),
                        XpcValue::String("main-display".into()),
                    ),
                    ("imageFormat".into(), XpcValue::String(format.into())),
                    ("pixelWidth".into(), XpcValue::Uint64(1170)),
                    ("pixelHeight".into(), XpcValue::Uint64(2532)),
                ])),
            )]))),
        }
    }

    #[test]
    fn request_has_coredevice_feature_and_action_shape() {
        let request = crate::services::coredevice::build_request_with_action(
            "ignored",
            CAPTURE_SCREENSHOT_FEATURE,
            CAPTURE_SCREENSHOT_ACTION,
            XpcValue::Dictionary(IndexMap::from([
                ("displayUniqueID".into(), XpcValue::Null),
                ("requestedFormat".into(), XpcValue::String("png".into())),
            ])),
        );
        let dict = request.as_dict().unwrap();
        assert_eq!(
            dict["CoreDevice.featureIdentifier"].as_str(),
            Some(CAPTURE_SCREENSHOT_FEATURE)
        );
        assert_eq!(
            dict["CoreDevice.actionIdentifier"].as_str(),
            Some(CAPTURE_SCREENSHOT_ACTION)
        );
        assert_eq!(
            dict["CoreDevice.action"],
            XpcValue::Dictionary(IndexMap::new())
        );
        assert_eq!(
            dict["CoreDevice.input"].as_dict().unwrap()["displayUniqueID"],
            XpcValue::Null
        );
    }

    #[test]
    fn parse_png_and_heif_metadata_and_reject_invalid_magic() {
        let png = [0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a];
        let image = parse_response(response(&png, "png")).unwrap();
        assert_eq!(image.display_unique_id.as_deref(), Some("main-display"));
        assert_eq!(image.mime_type(), "image/png");
        assert_eq!(image.pixel_size, Some((1170, 2532)));

        let mut heif = vec![0u8; 12];
        heif[4..8].copy_from_slice(b"ftyp");
        let image = parse_response(response(&heif, "heif")).unwrap();
        assert_eq!(image.mime_type(), "image/heif");

        assert!(matches!(
            parse_response(response(b"not-an-image", "png")),
            Err(ScreenCaptureError::Protocol(_))
        ));
    }

    #[test]
    fn rejects_oversized_screenshot_payload_before_magic_decode() {
        let mut oversized = vec![0u8; MAX_SCREENSHOT_BYTES + 1];
        oversized[..8].copy_from_slice(&[0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a]);
        assert!(matches!(
            parse_response(response(&oversized, "png")),
            Err(ScreenCaptureError::Protocol(message)) if message.contains("exceeds maximum")
        ));
    }

    #[test]
    fn validates_requested_formats() {
        assert!(validate_requested_format("PNG").is_ok());
        assert!(validate_requested_format("heic").is_ok());
        assert!(validate_requested_format("bmp").is_err());
    }

    #[test]
    fn feature_selection_is_permissive_only_when_metadata_is_missing() {
        assert!(feature_is_advertised(None));
        assert!(feature_is_advertised(Some(&[])));
        assert!(feature_is_advertised(Some(&[
            CAPTURE_SCREENSHOT_FEATURE.to_string(),
        ])));
        assert!(!feature_is_advertised(Some(&["other-feature".to_string()])));
    }
}
