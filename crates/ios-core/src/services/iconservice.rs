//! iOS 17+ CoreDevice application icon service.
//!
//! This is intentionally separate from [`crate::apps::appservice`].  On recent
//! systems `dtappserviced` does not expose the icon feature; icons are served
//! by `com.apple.coredevice.iconservice` instead.

use bytes::Bytes;
use indexmap::IndexMap;

use crate::xpc::{XpcClient, XpcError, XpcMessage, XpcValue};

/// RSD service name for CoreDevice icon requests.
pub const SERVICE_NAME: &str = "com.apple.coredevice.iconservice";
/// Feature identifier accepted by the icon service.
pub const FETCH_APP_ICONS_FEATURE: &str = "com.apple.coredevice.feature.fetchappicons";
/// Compatibility spelling used by other CoreDevice service modules.
pub const FEATURE_FETCH_APP_ICONS: &str = FETCH_APP_ICONS_FEATURE;

/// Maximum icon payload accepted from a device.
///
/// Icons are normally a few hundred KiB.  The larger bound leaves room for
/// unusual high-density icons while ensuring a malformed XPC response cannot
/// allocate an unbounded image buffer.
pub const MAX_ICON_DATA_BYTES: usize = 16 * 1024 * 1024;

/// One rendered application icon.
#[derive(Debug, Clone, PartialEq)]
pub struct AppIcon {
    /// PNG bytes returned by CoreDevice.
    pub png_data: Bytes,
    /// Rendered dimensions in pixels.
    pub pixel_size: (f64, f64),
    /// Rendered dimensions in points.
    pub size: (f64, f64),
    /// Render scale.
    pub scale: f64,
    /// Whether CoreDevice returned a generic placeholder icon.
    pub is_placeholder: bool,
}

impl AppIcon {
    /// Borrow the encoded PNG bytes.
    pub fn data(&self) -> &Bytes {
        &self.png_data
    }
}

/// Errors returned by CoreDevice icon requests.
#[derive(Debug, thiserror::Error)]
pub enum IconServiceError {
    /// Underlying RemoteXPC transport failure.
    #[error("xpc error: {0}")]
    Xpc(#[from] XpcError),
    /// The icon daemon returned an invalid response or unsupported argument.
    #[error("icon service protocol error: {0}")]
    Protocol(String),
    /// IconService only has a modern CoreDevice envelope.
    #[error("CoreDevice icon service requires the modern envelope; legacy mode is unsupported")]
    LegacyUnsupported,
}

pub use crate::services::coredevice::CoreDeviceEnvelopeMode;

/// Client for `com.apple.coredevice.iconservice`.
pub struct IconServiceClient {
    client: XpcClient,
    envelope_mode: CoreDeviceEnvelopeMode,
    service_features: Option<Vec<String>>,
}

/// Compatibility alias matching the upstream service name.
pub type IconService = IconServiceClient;

impl IconServiceClient {
    /// Create a modern CoreDevice icon client.
    pub fn new(client: XpcClient) -> Self {
        Self::new_with_mode(client, CoreDeviceEnvelopeMode::Modern)
    }

    /// Create a modern client retaining the feature list from the resolved RSD
    /// descriptor.  An empty list means capability metadata was omitted and is
    /// therefore permissive, just like the RSD resolver.
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
    /// entry can serve icons. An omitted or empty feature list is permissive,
    /// matching the resolver's handling of older RSD descriptors.
    pub fn supports_fetch_app_icons(&self) -> bool {
        feature_is_advertised(self.service_features.as_deref())
    }

    fn ensure_available(&self) -> Result<(), IconServiceError> {
        if self.envelope_mode == CoreDeviceEnvelopeMode::Legacy {
            return Err(IconServiceError::LegacyUnsupported);
        }
        if !self.supports_fetch_app_icons() {
            return Err(IconServiceError::Protocol(format!(
                "CoreDevice feature {FETCH_APP_ICONS_FEATURE} is not advertised by RSD"
            )));
        }
        Ok(())
    }

    /// Fetch one icon by bundle identifier or on-device app path.
    ///
    /// Exactly one of `bundle_identifier` and `app_path` must be supplied.
    /// `width`, `height`, and `scale` are encoded with the same Float32
    /// rounding used by the Swift `FetchAppIconParams` decoder.
    pub async fn fetch_icon(
        &mut self,
        bundle_identifier: Option<&str>,
        app_path: Option<&str>,
        width: f64,
        height: f64,
        scale: f64,
        allow_placeholder: bool,
    ) -> Result<AppIcon, IconServiceError> {
        self.ensure_available()?;
        validate_identifiers(bundle_identifier, app_path)?;
        validate_dimension("width", width)?;
        validate_dimension("height", height)?;
        validate_dimension("scale", scale)?;
        let response = self
            .client
            .call(crate::services::coredevice::build_request(
                "",
                FETCH_APP_ICONS_FEATURE,
                build_fetch_icon_input(
                    bundle_identifier,
                    app_path,
                    width,
                    height,
                    scale,
                    allow_placeholder,
                )?,
            ))
            .await?;
        parse_response(response)
    }

    /// Fetch icons for multiple bundle identifiers.
    ///
    /// The wire protocol is a single-app request. This convenience method
    /// performs one request per bundle in order and never invents a batch wire
    /// shape the daemon does not define.
    pub async fn fetch_icons(
        &mut self,
        bundle_identifiers: &[&str],
        width: f64,
        height: f64,
        scale: f64,
        allow_placeholder: bool,
    ) -> Result<Vec<AppIcon>, IconServiceError> {
        let mut icons = Vec::with_capacity(bundle_identifiers.len());
        for bundle_identifier in bundle_identifiers {
            icons.push(
                self.fetch_icon(
                    Some(bundle_identifier),
                    None,
                    width,
                    height,
                    scale,
                    allow_placeholder,
                )
                .await?,
            );
        }
        Ok(icons)
    }
}

fn validate_identifiers(
    bundle_identifier: Option<&str>,
    app_path: Option<&str>,
) -> Result<(), IconServiceError> {
    if bundle_identifier.is_some() == app_path.is_some() {
        return Err(IconServiceError::Protocol(
            "exactly one of bundle_identifier or app_path must be supplied".into(),
        ));
    }
    if bundle_identifier.is_some_and(str::is_empty) || app_path.is_some_and(str::is_empty) {
        return Err(IconServiceError::Protocol(
            "bundle_identifier and app_path must not be empty".into(),
        ));
    }
    Ok(())
}

fn validate_dimension(name: &str, value: f64) -> Result<(), IconServiceError> {
    if !value.is_finite() || value <= 0.0 {
        return Err(IconServiceError::Protocol(format!(
            "{name} must be finite and greater than zero"
        )));
    }
    let rounded = value as f32;
    if !rounded.is_finite() || rounded <= 0.0 {
        return Err(IconServiceError::Protocol(format!(
            "{name} is outside the CoreDevice Float32 range"
        )));
    }
    Ok(())
}

fn feature_is_advertised(features: Option<&[String]>) -> bool {
    features.map_or(true, |features| {
        features.is_empty()
            || features
                .iter()
                .any(|feature| feature == FETCH_APP_ICONS_FEATURE)
    })
}

fn float32(value: f64) -> f64 {
    value as f32 as f64
}

fn build_fetch_icon_input(
    bundle_identifier: Option<&str>,
    app_path: Option<&str>,
    width: f64,
    height: f64,
    scale: f64,
    allow_placeholder: bool,
) -> Result<XpcValue, IconServiceError> {
    Ok(XpcValue::Dictionary(IndexMap::from([
        (
            "bundleIdentifier".to_string(),
            bundle_identifier
                .map(|value| XpcValue::String(value.to_string()))
                .unwrap_or(XpcValue::Null),
        ),
        (
            "appPath".to_string(),
            app_path
                .map(|value| XpcValue::String(value.to_string()))
                .unwrap_or(XpcValue::Null),
        ),
        ("width".to_string(), XpcValue::Double(float32(width))),
        ("height".to_string(), XpcValue::Double(float32(height))),
        ("scale".to_string(), XpcValue::Double(float32(scale))),
        (
            "allowPlaceholder".to_string(),
            XpcValue::Bool(allow_placeholder),
        ),
    ])))
}

fn parse_response(response: XpcMessage) -> Result<AppIcon, IconServiceError> {
    let output =
        crate::services::coredevice::parse_output(response).map_err(IconServiceError::Protocol)?;
    let dict = output.as_dict().ok_or_else(|| {
        IconServiceError::Protocol(format!("icon output is not a dictionary: {output:?}"))
    })?;
    let info = dict.get("appIconInfo").ok_or_else(|| {
        IconServiceError::Protocol(format!("icon output missing appIconInfo: {output:?}"))
    })?;
    let info = info.as_dict().ok_or_else(|| {
        IconServiceError::Protocol(format!("appIconInfo is not a dictionary: {info:?}"))
    })?;
    let png_data = match info.get("pngData") {
        Some(XpcValue::Data(data)) if !data.is_empty() => data.clone(),
        Some(other) => {
            return Err(IconServiceError::Protocol(format!(
                "appIconInfo.pngData is not non-empty data: {other:?}"
            )))
        }
        None => {
            return Err(IconServiceError::Protocol(
                "appIconInfo missing pngData".into(),
            ))
        }
    };
    if png_data.len() > MAX_ICON_DATA_BYTES {
        return Err(IconServiceError::Protocol(format!(
            "icon data length {} exceeds maximum {MAX_ICON_DATA_BYTES}",
            png_data.len()
        )));
    }
    if !png_data.starts_with(&[0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a]) {
        return Err(IconServiceError::Protocol(
            "appIconInfo.pngData is not a PNG image".into(),
        ));
    }
    let pixel_size = parse_pair(info, "pixelSize")?;
    let size = parse_pair(info, "size")?;
    let scale = number(info, "scale")?;
    if scale <= 0.0 {
        return Err(IconServiceError::Protocol(
            "appIconInfo.scale must be greater than zero".into(),
        ));
    }
    let is_placeholder = info
        .get("isAppIconPlaceholder")
        .and_then(as_bool)
        .ok_or_else(|| {
            IconServiceError::Protocol("appIconInfo missing boolean isAppIconPlaceholder".into())
        })?;

    Ok(AppIcon {
        png_data,
        pixel_size,
        size,
        scale,
        is_placeholder,
    })
}

fn parse_pair(
    dict: &IndexMap<String, XpcValue>,
    key: &str,
) -> Result<(f64, f64), IconServiceError> {
    let value = dict
        .get(key)
        .ok_or_else(|| IconServiceError::Protocol(format!("appIconInfo missing {key}")))?;
    let values = match value {
        XpcValue::Array(values) if values.len() == 2 => values,
        _ => {
            return Err(IconServiceError::Protocol(format!(
                "appIconInfo.{key} must contain exactly two numbers: {value:?}"
            )))
        }
    };
    let first = as_f64(&values[0]).ok_or_else(|| {
        IconServiceError::Protocol(format!("appIconInfo.{key}[0] is not a finite number"))
    })?;
    let second = as_f64(&values[1]).ok_or_else(|| {
        IconServiceError::Protocol(format!("appIconInfo.{key}[1] is not a finite number"))
    })?;
    if first < 0.0 || second < 0.0 {
        return Err(IconServiceError::Protocol(format!(
            "appIconInfo.{key} cannot contain negative dimensions"
        )));
    }
    Ok((first, second))
}

fn number(dict: &IndexMap<String, XpcValue>, key: &str) -> Result<f64, IconServiceError> {
    let value = dict
        .get(key)
        .ok_or_else(|| IconServiceError::Protocol(format!("appIconInfo missing {key}")))?;
    as_f64(value).ok_or_else(|| {
        IconServiceError::Protocol(format!("appIconInfo.{key} is not a finite number"))
    })
}

fn as_f64(value: &XpcValue) -> Option<f64> {
    let number = match value {
        XpcValue::Double(value) => *value,
        XpcValue::Int64(value) => *value as f64,
        XpcValue::Uint64(value) => *value as f64,
        _ => return None,
    };
    number.is_finite().then_some(number)
}

fn as_bool(value: &XpcValue) -> Option<bool> {
    match value {
        XpcValue::Bool(value) => Some(*value),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use bytes::Bytes;

    use super::*;

    const PNG: &[u8] = &[0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a];

    #[test]
    fn request_has_exact_upstream_shape_and_null_alternative() {
        let request = crate::services::coredevice::build_request(
            "ignored",
            FETCH_APP_ICONS_FEATURE,
            build_fetch_icon_input(Some("com.example.App"), None, 60.123456789, 61.0, 2.0, true)
                .unwrap(),
        );
        let dict = request.as_dict().unwrap();
        assert_eq!(
            dict["CoreDevice.featureIdentifier"].as_str(),
            Some(FETCH_APP_ICONS_FEATURE)
        );
        let input = dict["CoreDevice.input"].as_dict().unwrap();
        assert_eq!(input["bundleIdentifier"].as_str(), Some("com.example.App"));
        assert_eq!(input["appPath"], XpcValue::Null);
        assert_eq!(input["width"], XpcValue::Double(float32(60.123456789)));
        assert_eq!(input["height"], XpcValue::Double(61.0));
        assert_eq!(input["scale"], XpcValue::Double(2.0));
        assert_eq!(input["allowPlaceholder"], XpcValue::Bool(true));
    }

    #[test]
    fn parse_icon_info_and_reject_bad_png_or_metadata() {
        let info = XpcValue::Dictionary(IndexMap::from([
            ("pngData".into(), XpcValue::Data(Bytes::from_static(PNG))),
            (
                "pixelSize".into(),
                XpcValue::Array(vec![XpcValue::Double(120.0), XpcValue::Double(120.0)]),
            ),
            (
                "size".into(),
                XpcValue::Array(vec![XpcValue::Double(60.0), XpcValue::Double(60.0)]),
            ),
            ("scale".into(), XpcValue::Double(2.0)),
            ("isAppIconPlaceholder".into(), XpcValue::Bool(false)),
        ]));
        let output = XpcValue::Dictionary(IndexMap::from([("appIconInfo".into(), info)]));
        let response = XpcMessage {
            flags: 0,
            msg_id: 1,
            body: Some(XpcValue::Dictionary(IndexMap::from([(
                "CoreDevice.output".into(),
                output,
            )]))),
        };
        let icon = parse_response(response).unwrap();
        assert_eq!(icon.pixel_size, (120.0, 120.0));
        assert_eq!(icon.size, (60.0, 60.0));
        assert!(!icon.is_placeholder);

        let bad_info = XpcValue::Dictionary(IndexMap::from([(
            "pngData".into(),
            XpcValue::Data(Bytes::from_static(b"not png")),
        )]));
        let bad_output = XpcValue::Dictionary(IndexMap::from([("appIconInfo".into(), bad_info)]));
        let bad = XpcMessage {
            flags: 0,
            msg_id: 1,
            body: Some(XpcValue::Dictionary(IndexMap::from([(
                "CoreDevice.output".into(),
                bad_output,
            )]))),
        };
        assert!(matches!(
            parse_response(bad),
            Err(IconServiceError::Protocol(_))
        ));
    }

    #[test]
    fn rejects_oversized_icon_payload_before_metadata_decode() {
        let mut oversized = vec![0u8; MAX_ICON_DATA_BYTES + 1];
        oversized[..PNG.len()].copy_from_slice(PNG);
        let info = XpcValue::Dictionary(IndexMap::from([(
            "pngData".into(),
            XpcValue::Data(Bytes::from(oversized)),
        )]));
        let response = XpcMessage {
            flags: 0,
            msg_id: 1,
            body: Some(XpcValue::Dictionary(IndexMap::from([(
                "CoreDevice.output".into(),
                XpcValue::Dictionary(IndexMap::from([("appIconInfo".into(), info)])),
            )]))),
        };
        assert!(matches!(
            parse_response(response),
            Err(IconServiceError::Protocol(message)) if message.contains("exceeds maximum")
        ));
    }

    #[test]
    fn validates_identifier_and_dimensions() {
        assert!(validate_identifiers(Some("com.example.App"), None).is_ok());
        assert!(validate_identifiers(None, Some("/Applications/App.app")).is_ok());
        assert!(validate_identifiers(None, None).is_err());
        assert!(validate_identifiers(Some("a"), Some("b")).is_err());
        assert!(validate_dimension("width", 0.0).is_err());
        assert!(validate_dimension("width", f64::NAN).is_err());
        assert!(validate_dimension("width", f64::INFINITY).is_err());
    }

    #[test]
    fn feature_selection_is_permissive_only_when_metadata_is_missing() {
        assert!(feature_is_advertised(None));
        assert!(feature_is_advertised(Some(&[])));
        assert!(feature_is_advertised(Some(&[
            FETCH_APP_ICONS_FEATURE.to_string(),
        ])));
        assert!(!feature_is_advertised(Some(&["other-feature".to_string()])));
    }
}
