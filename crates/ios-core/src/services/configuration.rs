//! iOS 17+ CoreDevice configuration actions over RemoteXPC/RSD.
//!
//! The configuration daemon uses action-only CoreDevice envelopes. In
//! particular, these requests contain `CoreDevice.actionIdentifier` but do
//! not contain `CoreDevice.featureIdentifier` or `CoreDevice.action`; this
//! mirrors pymobiledevice3's `ConfigurationService` exactly.

use std::fmt;
use std::future::Future;
use std::pin::Pin;
use std::str::FromStr;

use indexmap::IndexMap;

use crate::xpc::{XpcClient, XpcError, XpcMessage, XpcValue};

pub use crate::services::coredevice::CoreDeviceEnvelopeMode;

/// RSD service name for CoreDevice configuration actions.
pub const SERVICE_NAME: &str = "com.apple.coredevice.configuration";

const ACTION_GET_USER_INTERFACE_STYLE: &str = "com.apple.coredevice.action.getuserinterfacestyle";
const ACTION_SET_USER_INTERFACE_STYLE: &str = "com.apple.coredevice.action.setuserinterfacestyle";
const ACTION_SET_LIQUID_GLASS_CONFIGURATION: &str =
    "com.apple.coredevice.action.setliquidglassconfiguration";
const ACTION_GET_COLOR_FILTER: &str = "com.apple.coredevice.action.getcolorfilter";
const ACTION_SET_COLOR_FILTER: &str = "com.apple.coredevice.action.setcolorfilter";
const ACTION_GET_DEVICE_TEXT_SIZE: &str = "com.apple.coredevice.action.getdevicetextsize";
const ACTION_SET_DEVICE_TEXT_SIZE: &str = "com.apple.coredevice.action.setdevicetextsize";
const ACTION_GET_REDUCE_MOTION: &str = "com.apple.coredevice.action.getreducemotion";
const ACTION_SET_REDUCE_MOTION: &str = "com.apple.coredevice.action.setreducemotion";
const ACTION_SET_INCREASE_CONTRAST: &str = "com.apple.coredevice.action.setdeviceincreasecontrast";
const ACTION_GET_SHOW_BORDERS: &str = "com.apple.coredevice.action.getshowborders";
const ACTION_SET_SHOW_BORDERS: &str = "com.apple.coredevice.action.setshowborders";
const ACTION_GET_REDUCE_TRANSPARENCY: &str = "com.apple.coredevice.action.getreducetransparency";
const ACTION_SET_REDUCE_TRANSPARENCY: &str = "com.apple.coredevice.action.setreducetransparency";

/// Errors returned by CoreDevice configuration actions.
#[derive(Debug, thiserror::Error)]
pub enum ConfigurationError {
    /// Underlying RemoteXPC transport failure.
    #[error("xpc error: {0}")]
    Xpc(#[from] XpcError),
    /// The daemon returned an unsupported response shape or value.
    #[error("configuration protocol error: {0}")]
    Protocol(String),
    /// Configuration actions are not available through the legacy envelope.
    #[error("CoreDevice configuration requires the modern envelope; legacy mode is unsupported")]
    LegacyUnsupported,
}

/// User-interface appearance returned by CoreDevice.
///
/// The daemon may add styles in a future OS release. Getters preserve such a
/// value in [`Self::Unknown`] so a newer device does not turn an otherwise
/// valid response into a protocol error. Setters intentionally accept only
/// the values known by this crate; callers should wait for an API update before
/// trying to set a new daemon value.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UserInterfaceStyle {
    Light,
    Dark,
    Unknown(String),
}

impl UserInterfaceStyle {
    /// Return the exact daemon string, including an unknown future value.
    pub fn as_str(&self) -> &str {
        match self {
            Self::Light => "light",
            Self::Dark => "dark",
            Self::Unknown(value) => value,
        }
    }

    fn known_name_for_set(&self) -> Result<&'static str, ConfigurationError> {
        match self {
            Self::Light => Ok("light"),
            Self::Dark => Ok("dark"),
            Self::Unknown(value) => Err(ConfigurationError::Protocol(format!(
                "cannot set unknown user-interface style {value:?}; use a supported style"
            ))),
        }
    }
}

impl FromStr for UserInterfaceStyle {
    type Err = String;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "light" => Ok(Self::Light),
            "dark" => Ok(Self::Dark),
            other if !other.is_empty() => Ok(Self::Unknown(other.to_string())),
            _ => Err("style must not be empty".into()),
        }
    }
}

impl serde::Serialize for UserInterfaceStyle {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        serializer.serialize_str(self.as_str())
    }
}

impl<'de> serde::Deserialize<'de> for UserInterfaceStyle {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        deserialize_wire_enum(deserializer)
    }
}

impl fmt::Display for UserInterfaceStyle {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

/// Color-filter preset returned by the CoreDevice configuration daemon.
///
/// `filterType` is a dictionary on the wire (`{"name": "Protanopia"}`),
/// while the public state and CLI JSON expose its name as one string. Unknown
/// names are retained by getters; setters reject [`Self::Unknown`] because
/// their semantics are not known to this crate.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ColorFilterType {
    Grayscale,
    Protanopia,
    Deuteranopia,
    Tritanopia,
    Unknown(String),
}

impl ColorFilterType {
    /// Return the exact daemon string, including an unknown future value.
    pub fn as_str(&self) -> &str {
        match self {
            Self::Grayscale => "Grayscale",
            Self::Protanopia => "Protanopia",
            Self::Deuteranopia => "Deuteranopia",
            Self::Tritanopia => "Tritanopia",
            Self::Unknown(value) => value,
        }
    }

    fn known_name_for_set(&self) -> Result<&'static str, ConfigurationError> {
        match self {
            Self::Grayscale => Ok("Grayscale"),
            Self::Protanopia => Ok("Protanopia"),
            Self::Deuteranopia => Ok("Deuteranopia"),
            Self::Tritanopia => Ok("Tritanopia"),
            Self::Unknown(value) => Err(ConfigurationError::Protocol(format!(
                "cannot set unknown color filter type {value:?}; use a supported filter"
            ))),
        }
    }
}

impl FromStr for ColorFilterType {
    type Err = String;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "Grayscale" | "grayscale" => Ok(Self::Grayscale),
            "Protanopia" | "protanopia" => Ok(Self::Protanopia),
            "Deuteranopia" | "deuteranopia" => Ok(Self::Deuteranopia),
            "Tritanopia" | "tritanopia" => Ok(Self::Tritanopia),
            other if !other.is_empty() => Ok(Self::Unknown(other.to_string())),
            _ => Err("color filter type must not be empty".into()),
        }
    }
}

impl serde::Serialize for ColorFilterType {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        serializer.serialize_str(self.as_str())
    }
}

impl<'de> serde::Deserialize<'de> for ColorFilterType {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        deserialize_wire_enum(deserializer)
    }
}

impl fmt::Display for ColorFilterType {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

/// Dynamic text-size value exposed by UIKit's content-size category enum.
/// Unknown values are retained when reading a newer device and rejected when
/// setting because their daemon semantics are not known to this crate.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DeviceTextSize {
    ExtraSmall,
    Small,
    Medium,
    Large,
    ExtraLarge,
    ExtraExtraLarge,
    ExtraExtraExtraLarge,
    AccessibilityMedium,
    AccessibilityLarge,
    AccessibilityExtraLarge,
    AccessibilityExtraExtraLarge,
    AccessibilityExtraExtraExtraLarge,
    Unknown(String),
}

impl DeviceTextSize {
    /// Return the exact daemon string, including an unknown future value.
    pub fn as_str(&self) -> &str {
        match self {
            Self::ExtraSmall => "extraSmall",
            Self::Small => "small",
            Self::Medium => "medium",
            Self::Large => "large",
            Self::ExtraLarge => "extraLarge",
            Self::ExtraExtraLarge => "extraExtraLarge",
            Self::ExtraExtraExtraLarge => "extraExtraExtraLarge",
            Self::AccessibilityMedium => "accessibilityMedium",
            Self::AccessibilityLarge => "accessibilityLarge",
            Self::AccessibilityExtraLarge => "accessibilityExtraLarge",
            Self::AccessibilityExtraExtraLarge => "accessibilityExtraExtraLarge",
            Self::AccessibilityExtraExtraExtraLarge => "accessibilityExtraExtraExtraLarge",
            Self::Unknown(value) => value,
        }
    }

    fn known_name_for_set(&self) -> Result<&'static str, ConfigurationError> {
        match self {
            Self::ExtraSmall => Ok("extraSmall"),
            Self::Small => Ok("small"),
            Self::Medium => Ok("medium"),
            Self::Large => Ok("large"),
            Self::ExtraLarge => Ok("extraLarge"),
            Self::ExtraExtraLarge => Ok("extraExtraLarge"),
            Self::ExtraExtraExtraLarge => Ok("extraExtraExtraLarge"),
            Self::AccessibilityMedium => Ok("accessibilityMedium"),
            Self::AccessibilityLarge => Ok("accessibilityLarge"),
            Self::AccessibilityExtraLarge => Ok("accessibilityExtraLarge"),
            Self::AccessibilityExtraExtraLarge => Ok("accessibilityExtraExtraLarge"),
            Self::AccessibilityExtraExtraExtraLarge => Ok("accessibilityExtraExtraExtraLarge"),
            Self::Unknown(value) => Err(ConfigurationError::Protocol(format!(
                "cannot set unknown device text size {value:?}; use a supported size"
            ))),
        }
    }
}

impl FromStr for DeviceTextSize {
    type Err = String;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "extraSmall" => Ok(Self::ExtraSmall),
            "small" => Ok(Self::Small),
            "medium" => Ok(Self::Medium),
            "large" => Ok(Self::Large),
            "extraLarge" => Ok(Self::ExtraLarge),
            "extraExtraLarge" => Ok(Self::ExtraExtraLarge),
            "extraExtraExtraLarge" => Ok(Self::ExtraExtraExtraLarge),
            "accessibilityMedium" => Ok(Self::AccessibilityMedium),
            "accessibilityLarge" => Ok(Self::AccessibilityLarge),
            "accessibilityExtraLarge" => Ok(Self::AccessibilityExtraLarge),
            "accessibilityExtraExtraLarge" => Ok(Self::AccessibilityExtraExtraLarge),
            "accessibilityExtraExtraExtraLarge" => Ok(Self::AccessibilityExtraExtraExtraLarge),
            other if !other.is_empty() => Ok(Self::Unknown(other.to_string())),
            _ => Err("device text size must not be empty".into()),
        }
    }
}

impl serde::Serialize for DeviceTextSize {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        serializer.serialize_str(self.as_str())
    }
}

impl<'de> serde::Deserialize<'de> for DeviceTextSize {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        deserialize_wire_enum(deserializer)
    }
}

impl fmt::Display for DeviceTextSize {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

/// Current color-filter state returned by CoreDevice.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct ColorFilterState {
    pub enabled: bool,
    #[serde(rename = "filterType", skip_serializing_if = "Option::is_none")]
    pub filter_type: Option<ColorFilterType>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub intensity: Option<f32>,
}

/// A client for `com.apple.coredevice.configuration`.
pub struct ConfigurationServiceClient {
    client: XpcClient,
    device_identifier: String,
    envelope_mode: CoreDeviceEnvelopeMode,
    service_features: Option<Vec<String>>,
}

/// Compatibility alias following pymobiledevice3's service naming.
pub type ConfigurationService = ConfigurationServiceClient;

/// Small transport seam shared by the real client and protocol-level tests.
/// Keeping request construction and response parsing above the socket layer
/// lets tests exercise a complete action round-trip without a device.
trait ActionTransport {
    fn call<'a>(
        &'a mut self,
        request: XpcValue,
    ) -> Pin<Box<dyn Future<Output = Result<XpcMessage, XpcError>> + 'a>>;
}

impl ActionTransport for XpcClient {
    fn call<'a>(
        &'a mut self,
        request: XpcValue,
    ) -> Pin<Box<dyn Future<Output = Result<XpcMessage, XpcError>> + 'a>> {
        Box::pin(XpcClient::call(self, request))
    }
}

impl ConfigurationServiceClient {
    /// Create a client using the modern CoreDevice action envelope.
    pub fn new(client: XpcClient, device_identifier: impl Into<String>) -> Self {
        Self::new_with_mode(client, device_identifier, CoreDeviceEnvelopeMode::Modern)
    }

    /// Create a modern client retaining the service's RSD feature metadata.
    ///
    /// Configuration actions are action-only requests and, like the reference
    /// client, intentionally do not require an advertised feature identifier.
    /// The metadata remains available to callers for diagnostics and routing.
    pub fn new_with_features<I, S>(
        client: XpcClient,
        device_identifier: impl Into<String>,
        service_features: I,
    ) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        Self {
            client,
            device_identifier: device_identifier.into(),
            envelope_mode: CoreDeviceEnvelopeMode::Modern,
            service_features: Some(service_features.into_iter().map(Into::into).collect()),
        }
    }

    /// Create a client with an explicit CoreDevice envelope mode.
    pub fn new_with_mode(
        client: XpcClient,
        device_identifier: impl Into<String>,
        envelope_mode: CoreDeviceEnvelopeMode,
    ) -> Self {
        Self {
            client,
            device_identifier: device_identifier.into(),
            envelope_mode,
            service_features: None,
        }
    }

    /// Return the envelope mode selected for this client.
    pub const fn envelope_mode(&self) -> CoreDeviceEnvelopeMode {
        self.envelope_mode
    }

    /// Return the exact RSD feature list used to route this service, if known.
    pub fn advertised_features(&self) -> Option<&[String]> {
        self.service_features.as_deref()
    }

    /// Read the active light/dark interface style.
    pub async fn get_user_interface_style(
        &mut self,
    ) -> Result<UserInterfaceStyle, ConfigurationError> {
        let output = self
            .invoke_action(ACTION_GET_USER_INTERFACE_STYLE, empty_input())
            .await?;
        let style = required_string(&output, "style", "get user interface style")?;
        style.parse().map_err(ConfigurationError::Protocol)
    }

    /// Set the active light/dark interface style.
    pub async fn set_user_interface_style(
        &mut self,
        style: UserInterfaceStyle,
    ) -> Result<(), ConfigurationError> {
        let style = style.known_name_for_set()?;
        self.invoke_action(
            ACTION_SET_USER_INTERFACE_STYLE,
            dictionary([("style", XpcValue::String(style.to_string()))]),
        )
        .await
        .map(|_| ())
    }

    /// Set liquid-glass opacity in the inclusive range 0.0..=1.0.
    ///
    /// The wire value is rounded through binary32 because the daemon decodes
    /// this slider as Swift `Float`, not `Double`.
    pub async fn set_liquid_glass_opacity(
        &mut self,
        opacity: f64,
    ) -> Result<(), ConfigurationError> {
        let opacity = checked_unit_float(opacity, "opacity")?;
        self.invoke_action(
            ACTION_SET_LIQUID_GLASS_CONFIGURATION,
            dictionary([(
                "configuration",
                dictionary([("opacity", XpcValue::Double(opacity as f64))]),
            )]),
        )
        .await
        .map(|_| ())
    }

    /// Read the current color-filter state.
    pub async fn get_color_filter(&mut self) -> Result<ColorFilterState, ConfigurationError> {
        let output = self
            .invoke_action(ACTION_GET_COLOR_FILTER, empty_input())
            .await?;
        parse_color_filter_state(&output)
    }

    /// Set color-filter state. A filter type is required when enabling it;
    /// intensity, when provided, must be in the inclusive range 0.0..=1.0.
    pub async fn set_color_filter(
        &mut self,
        enabled: bool,
        filter_type: Option<ColorFilterType>,
        intensity: Option<f64>,
    ) -> Result<(), ConfigurationError> {
        let mut body = IndexMap::from([(String::from("enabled"), XpcValue::Bool(enabled))]);
        if enabled {
            let filter_type = filter_type.ok_or_else(|| {
                ConfigurationError::Protocol(
                    "filter_type is required when color filter is enabled".into(),
                )
            })?;
            let filter_type = filter_type.known_name_for_set()?;
            body.insert(
                "filterType".into(),
                dictionary([("name", XpcValue::String(filter_type.to_string()))]),
            );
            if let Some(intensity) = intensity {
                body.insert(
                    "intensity".into(),
                    XpcValue::Double(checked_unit_float(intensity, "intensity")? as f64),
                );
            }
        } else if let Some(intensity) = intensity {
            checked_unit_float(intensity, "intensity")?;
        }
        self.invoke_action(
            ACTION_SET_COLOR_FILTER,
            dictionary([(String::from("colorFilter"), XpcValue::Dictionary(body))]),
        )
        .await
        .map(|_| ())
    }

    /// Read the dynamic text-size category.
    pub async fn get_device_text_size(&mut self) -> Result<DeviceTextSize, ConfigurationError> {
        let output = self
            .invoke_action(ACTION_GET_DEVICE_TEXT_SIZE, empty_input())
            .await?;
        let size = nested_dict(&output, "textSize", "get device text size")?;
        let size = nested_dict_from(size, "size", "get device text size")?;
        if size.len() != 1 {
            return Err(ConfigurationError::Protocol(format!(
                "get device text size expected one size variant, got {size:?}"
            )));
        }
        size.keys()
            .next()
            .ok_or_else(|| {
                ConfigurationError::Protocol("get device text size returned no size variant".into())
            })?
            .parse()
            .map_err(ConfigurationError::Protocol)
    }

    /// Set the dynamic text-size category.
    pub async fn set_device_text_size(
        &mut self,
        size: DeviceTextSize,
    ) -> Result<(), ConfigurationError> {
        let size = size.known_name_for_set()?;
        self.invoke_action(
            ACTION_SET_DEVICE_TEXT_SIZE,
            dictionary([(
                "textSize",
                dictionary([(
                    "size",
                    dictionary([(size, XpcValue::Dictionary(IndexMap::new()))]),
                )]),
            )]),
        )
        .await
        .map(|_| ())
    }

    /// Read whether Reduce Motion is enabled.
    pub async fn get_reduce_motion(&mut self) -> Result<bool, ConfigurationError> {
        self.get_enabled(ACTION_GET_REDUCE_MOTION, "reduceMotion")
            .await
    }

    /// Toggle Reduce Motion.
    pub async fn set_reduce_motion(&mut self, enabled: bool) -> Result<(), ConfigurationError> {
        self.set_enabled(ACTION_SET_REDUCE_MOTION, "reduceMotion", enabled)
            .await
    }

    /// Toggle Increase Contrast. The daemon exposes no symmetric getter.
    pub async fn set_increase_contrast(&mut self, enabled: bool) -> Result<(), ConfigurationError> {
        self.set_enabled(ACTION_SET_INCREASE_CONTRAST, "increaseContrast", enabled)
            .await
    }

    /// Read whether layout-debug borders are enabled.
    pub async fn get_show_borders(&mut self) -> Result<bool, ConfigurationError> {
        self.get_enabled(ACTION_GET_SHOW_BORDERS, "showBorders")
            .await
    }

    /// Toggle layout-debug borders.
    pub async fn set_show_borders(&mut self, enabled: bool) -> Result<(), ConfigurationError> {
        self.set_enabled(ACTION_SET_SHOW_BORDERS, "showBorders", enabled)
            .await
    }

    /// Read whether Reduce Transparency is enabled.
    pub async fn get_reduce_transparency(&mut self) -> Result<bool, ConfigurationError> {
        self.get_enabled(ACTION_GET_REDUCE_TRANSPARENCY, "reduceTransparency")
            .await
    }

    /// Toggle Reduce Transparency.
    pub async fn set_reduce_transparency(
        &mut self,
        enabled: bool,
    ) -> Result<(), ConfigurationError> {
        self.set_enabled(
            ACTION_SET_REDUCE_TRANSPARENCY,
            "reduceTransparency",
            enabled,
        )
        .await
    }

    async fn get_enabled(&mut self, action: &str, field: &str) -> Result<bool, ConfigurationError> {
        let output = self.invoke_action(action, empty_input()).await?;
        let nested = nested_dict(&output, field, action)?;
        nested.get("enabled").and_then(xpc_bool).ok_or_else(|| {
            ConfigurationError::Protocol(format!(
                "{action} output missing boolean {field}.enabled: {output:?}"
            ))
        })
    }

    async fn set_enabled(
        &mut self,
        action: &str,
        field: &str,
        enabled: bool,
    ) -> Result<(), ConfigurationError> {
        self.invoke_action(
            action,
            dictionary([(field, dictionary([("enabled", XpcValue::Bool(enabled))]))]),
        )
        .await
        .map(|_| ())
    }

    async fn invoke_action(
        &mut self,
        action_identifier: &str,
        input: XpcValue,
    ) -> Result<XpcValue, ConfigurationError> {
        invoke_action_with_transport(
            &mut self.client,
            &self.device_identifier,
            self.envelope_mode,
            action_identifier,
            input,
        )
        .await
    }
}

async fn invoke_action_with_transport<T: ActionTransport>(
    transport: &mut T,
    device_identifier: &str,
    envelope_mode: CoreDeviceEnvelopeMode,
    action_identifier: &str,
    input: XpcValue,
) -> Result<XpcValue, ConfigurationError> {
    ensure_modern(envelope_mode)?;
    let response = transport
        .call(crate::services::coredevice::build_action_request(
            device_identifier,
            action_identifier,
            input,
        ))
        .await?;
    crate::services::coredevice::parse_output(response).map_err(ConfigurationError::Protocol)
}

fn ensure_modern(mode: CoreDeviceEnvelopeMode) -> Result<(), ConfigurationError> {
    if mode == CoreDeviceEnvelopeMode::Legacy {
        return Err(ConfigurationError::LegacyUnsupported);
    }
    Ok(())
}

fn empty_input() -> XpcValue {
    XpcValue::Dictionary(IndexMap::new())
}

fn dictionary<const N: usize>(entries: [(impl Into<String>, XpcValue); N]) -> XpcValue {
    XpcValue::Dictionary(IndexMap::from_iter(
        entries.into_iter().map(|(key, value)| (key.into(), value)),
    ))
}

fn checked_unit_float(value: f64, name: &str) -> Result<f32, ConfigurationError> {
    if !value.is_finite() || !(0.0..=1.0).contains(&value) {
        return Err(ConfigurationError::Protocol(format!(
            "{name} must be finite and in the inclusive range [0.0, 1.0], got {value}"
        )));
    }
    Ok(value as f32)
}

fn deserialize_wire_enum<'de, D, T>(deserializer: D) -> Result<T, D::Error>
where
    D: serde::Deserializer<'de>,
    T: FromStr<Err = String>,
{
    let value = <String as serde::Deserialize>::deserialize(deserializer)?;
    value.parse().map_err(serde::de::Error::custom)
}

fn required_string<'a>(
    value: &'a XpcValue,
    key: &str,
    action: &str,
) -> Result<&'a str, ConfigurationError> {
    value
        .as_dict()
        .and_then(|dict| dict.get(key))
        .and_then(XpcValue::as_str)
        .ok_or_else(|| {
            ConfigurationError::Protocol(format!("{action} output missing string {key}: {value:?}"))
        })
}

fn nested_dict<'a>(
    value: &'a XpcValue,
    key: &str,
    action: &str,
) -> Result<&'a IndexMap<String, XpcValue>, ConfigurationError> {
    value
        .as_dict()
        .and_then(|dict| dict.get(key))
        .and_then(XpcValue::as_dict)
        .ok_or_else(|| {
            ConfigurationError::Protocol(format!(
                "{action} output missing dictionary {key}: {value:?}"
            ))
        })
}

fn xpc_bool(value: &XpcValue) -> Option<bool> {
    match value {
        XpcValue::Bool(value) => Some(*value),
        _ => None,
    }
}

fn nested_dict_from<'a>(
    dict: &'a IndexMap<String, XpcValue>,
    key: &str,
    action: &str,
) -> Result<&'a IndexMap<String, XpcValue>, ConfigurationError> {
    dict.get(key).and_then(XpcValue::as_dict).ok_or_else(|| {
        ConfigurationError::Protocol(format!(
            "{action} output missing dictionary {key}: {dict:?}"
        ))
    })
}

fn parse_color_filter_state(value: &XpcValue) -> Result<ColorFilterState, ConfigurationError> {
    let output = value.as_dict().ok_or_else(|| {
        ConfigurationError::Protocol(format!(
            "get color filter output is not a dictionary: {value:?}"
        ))
    })?;
    let dict = output
        .get("colorFilter")
        .and_then(XpcValue::as_dict)
        .ok_or_else(|| {
            ConfigurationError::Protocol(format!(
                "get color filter output missing dictionary colorFilter: {value:?}"
            ))
        })?;
    let enabled = dict.get("enabled").and_then(xpc_bool).ok_or_else(|| {
        ConfigurationError::Protocol(format!(
            "get color filter output missing boolean enabled: {value:?}"
        ))
    })?;
    let filter_type = dict
        .get("filterType")
        .map(|value| {
            value
                .as_dict()
                .and_then(|dict| dict.get("name"))
                .and_then(XpcValue::as_str)
                .ok_or_else(|| {
                    ConfigurationError::Protocol(format!(
                        "get color filter output has malformed filterType: {value:?}"
                    ))
                })?
                .parse()
                .map_err(ConfigurationError::Protocol)
        })
        .transpose()?;
    let intensity = dict
        .get("intensity")
        .map(|value| match value {
            XpcValue::Double(value) if value.is_finite() && (0.0..=1.0).contains(value) => {
                Ok(*value as f32)
            }
            _ => Err(ConfigurationError::Protocol(format!(
                "get color filter output has invalid intensity: {value:?}"
            ))),
        })
        .transpose()?;
    Ok(ColorFilterState {
        enabled,
        filter_type,
        intensity,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    struct FakeActionTransport {
        requests: Vec<XpcValue>,
        response: Option<Result<XpcMessage, XpcError>>,
    }

    impl ActionTransport for FakeActionTransport {
        fn call<'a>(
            &'a mut self,
            request: XpcValue,
        ) -> Pin<Box<dyn Future<Output = Result<XpcMessage, XpcError>> + 'a>> {
            self.requests.push(request);
            let response = self
                .response
                .take()
                .expect("fake action transport called more than once");
            Box::pin(async move { response })
        }
    }

    fn response_with_output(output: XpcValue) -> XpcMessage {
        XpcMessage {
            flags: 0,
            msg_id: 1,
            body: Some(dictionary([("CoreDevice.output", output)])),
        }
    }

    #[test]
    fn enums_roundtrip_wire_names() {
        assert_eq!(
            "dark".parse::<UserInterfaceStyle>().unwrap().as_str(),
            "dark"
        );
        assert_eq!(
            "Protanopia".parse::<ColorFilterType>().unwrap().as_str(),
            "Protanopia"
        );
        assert_eq!(
            "accessibilityExtraExtraLarge"
                .parse::<DeviceTextSize>()
                .unwrap()
                .as_str(),
            "accessibilityExtraExtraLarge"
        );
        assert_eq!(
            "future-style".parse::<UserInterfaceStyle>().unwrap(),
            UserInterfaceStyle::Unknown("future-style".into())
        );
        assert_eq!(
            "sepia".parse::<ColorFilterType>().unwrap(),
            ColorFilterType::Unknown("sepia".into())
        );
        assert_eq!(
            "accessibilityHuge".parse::<DeviceTextSize>().unwrap(),
            DeviceTextSize::Unknown("accessibilityHuge".into())
        );
        assert!("".parse::<ColorFilterType>().is_err());
    }

    #[test]
    fn unknown_configuration_values_are_preserved_in_api_and_json() {
        let style: UserInterfaceStyle = "automatic".parse().unwrap();
        let filter: ColorFilterType = "InvertColors".parse().unwrap();
        let text_size: DeviceTextSize = "accessibilityGigantic".parse().unwrap();

        assert_eq!(style.to_string(), "automatic");
        assert_eq!(filter.to_string(), "InvertColors");
        assert_eq!(text_size.to_string(), "accessibilityGigantic");
        assert_eq!(
            serde_json::to_value(&style).unwrap(),
            serde_json::json!("automatic")
        );
        assert_eq!(
            serde_json::to_value(&filter).unwrap(),
            serde_json::json!("InvertColors")
        );
        assert_eq!(
            serde_json::to_value(&text_size).unwrap(),
            serde_json::json!("accessibilityGigantic")
        );
        assert_eq!(
            serde_json::from_value::<UserInterfaceStyle>(serde_json::json!("automatic")).unwrap(),
            style
        );
        assert_eq!(
            serde_json::from_value::<ColorFilterType>(serde_json::json!("InvertColors")).unwrap(),
            filter
        );
        assert_eq!(
            serde_json::from_value::<DeviceTextSize>(serde_json::json!("accessibilityGigantic"))
                .unwrap(),
            text_size
        );

        assert!(style.known_name_for_set().is_err());
        assert!(filter.known_name_for_set().is_err());
        assert!(text_size.known_name_for_set().is_err());
    }

    #[test]
    fn unit_float_validation_rejects_nan_and_out_of_range() {
        assert_eq!(checked_unit_float(0.0, "x").unwrap(), 0.0);
        assert_eq!(checked_unit_float(1.0, "x").unwrap(), 1.0);
        assert!(checked_unit_float(-0.01, "x").is_err());
        assert!(checked_unit_float(1.01, "x").is_err());
        assert!(checked_unit_float(f64::NAN, "x").is_err());
        assert!(checked_unit_float(f64::INFINITY, "x").is_err());
    }

    #[test]
    fn parses_enabled_color_filter_and_disabled_minimal_shape() {
        let enabled = parse_color_filter_state(&dictionary([(
            "colorFilter",
            dictionary([
                ("enabled", XpcValue::Bool(true)),
                (
                    "filterType",
                    dictionary([("name", XpcValue::String("Tritanopia".into()))]),
                ),
                ("intensity", XpcValue::Double(0.5)),
            ]),
        )]))
        .unwrap();
        assert_eq!(enabled.filter_type, Some(ColorFilterType::Tritanopia));
        assert_eq!(enabled.intensity, Some(0.5));

        let future = parse_color_filter_state(&dictionary([(
            "colorFilter",
            dictionary([
                ("enabled", XpcValue::Bool(true)),
                (
                    "filterType",
                    dictionary([("name", XpcValue::String("InvertColors".into()))]),
                ),
            ]),
        )]))
        .unwrap();
        assert_eq!(
            future.filter_type,
            Some(ColorFilterType::Unknown("InvertColors".into()))
        );
        assert_eq!(
            serde_json::to_value(future).unwrap(),
            serde_json::json!({
                "enabled": true,
                "filterType": "InvertColors"
            })
        );
        let decoded: ColorFilterState = serde_json::from_value(serde_json::json!({
            "enabled": true,
            "filterType": "InvertColors"
        }))
        .unwrap();
        assert_eq!(
            decoded.filter_type,
            Some(ColorFilterType::Unknown("InvertColors".into()))
        );

        let disabled = parse_color_filter_state(&dictionary([(
            "colorFilter",
            dictionary([("enabled", XpcValue::Bool(false))]),
        )]))
        .unwrap();
        assert!(!disabled.enabled);
        assert_eq!(disabled.filter_type, None);
    }

    #[test]
    fn color_filter_json_omits_disabled_optional_wire_fields() {
        let disabled = ColorFilterState {
            enabled: false,
            filter_type: None,
            intensity: None,
        };
        let json = serde_json::to_value(disabled).unwrap();
        assert_eq!(json, serde_json::json!({"enabled": false}));
    }

    #[test]
    fn parses_nested_text_size_and_enabled_values() {
        let value = dictionary([(
            "textSize",
            dictionary([(
                "size",
                dictionary([("large", XpcValue::Dictionary(IndexMap::new()))]),
            )]),
        )]);
        let text_size = nested_dict(&value, "textSize", "test").unwrap();
        assert_eq!(
            nested_dict_from(text_size, "size", "test")
                .unwrap()
                .keys()
                .next(),
            Some(&"large".to_string())
        );
    }

    #[test]
    fn every_public_action_has_the_reference_identifier_and_input_shape() {
        let cases = [
            (ACTION_GET_USER_INTERFACE_STYLE, empty_input()),
            (
                ACTION_SET_USER_INTERFACE_STYLE,
                dictionary([("style", XpcValue::String("dark".into()))]),
            ),
            (
                ACTION_SET_LIQUID_GLASS_CONFIGURATION,
                dictionary([(
                    "configuration",
                    dictionary([("opacity", XpcValue::Double(0.5))]),
                )]),
            ),
            (ACTION_GET_COLOR_FILTER, empty_input()),
            (
                ACTION_SET_COLOR_FILTER,
                dictionary([(
                    "colorFilter",
                    dictionary([
                        ("enabled", XpcValue::Bool(true)),
                        (
                            "filterType",
                            dictionary([("name", XpcValue::String("Protanopia".into()))]),
                        ),
                        ("intensity", XpcValue::Double(0.5)),
                    ]),
                )]),
            ),
            (ACTION_GET_DEVICE_TEXT_SIZE, empty_input()),
            (
                ACTION_SET_DEVICE_TEXT_SIZE,
                dictionary([(
                    "textSize",
                    dictionary([(
                        "size",
                        dictionary([("large", XpcValue::Dictionary(IndexMap::new()))]),
                    )]),
                )]),
            ),
            (ACTION_GET_REDUCE_MOTION, empty_input()),
            (
                ACTION_SET_REDUCE_MOTION,
                dictionary([(
                    "reduceMotion",
                    dictionary([("enabled", XpcValue::Bool(false))]),
                )]),
            ),
            (
                ACTION_SET_INCREASE_CONTRAST,
                dictionary([(
                    "increaseContrast",
                    dictionary([("enabled", XpcValue::Bool(true))]),
                )]),
            ),
            (ACTION_GET_SHOW_BORDERS, empty_input()),
            (
                ACTION_SET_SHOW_BORDERS,
                dictionary([(
                    "showBorders",
                    dictionary([("enabled", XpcValue::Bool(true))]),
                )]),
            ),
            (ACTION_GET_REDUCE_TRANSPARENCY, empty_input()),
            (
                ACTION_SET_REDUCE_TRANSPARENCY,
                dictionary([(
                    "reduceTransparency",
                    dictionary([("enabled", XpcValue::Bool(false))]),
                )]),
            ),
        ];
        assert_eq!(cases.len(), 14);

        for (action, input) in cases {
            let request = crate::services::coredevice::build_action_request(
                "DEVICE-ID",
                action,
                input.clone(),
            );
            let dict = request
                .as_dict()
                .expect("action request should be a dictionary");
            assert_eq!(dict.len(), 6, "unexpected envelope keys for {action}");
            assert_eq!(dict["CoreDevice.actionIdentifier"].as_str(), Some(action));
            assert_eq!(dict["CoreDevice.input"], input);
            assert!(!dict.contains_key("CoreDevice.featureIdentifier"));
            assert!(!dict.contains_key("CoreDevice.action"));
        }
    }

    #[test]
    fn legacy_mode_is_rejected_before_transport() {
        assert!(matches!(
            ensure_modern(CoreDeviceEnvelopeMode::Legacy),
            Err(ConfigurationError::LegacyUnsupported)
        ));
        assert!(ensure_modern(CoreDeviceEnvelopeMode::Modern).is_ok());
    }

    #[tokio::test]
    async fn mock_transport_round_trip_checks_action_envelope_and_response() {
        assert_eq!(SERVICE_NAME, "com.apple.coredevice.configuration");
        let mut transport = FakeActionTransport {
            requests: Vec::new(),
            response: Some(Ok(response_with_output(dictionary([(
                "style",
                XpcValue::String("future-style".into()),
            )])))),
        };

        let output = invoke_action_with_transport(
            &mut transport,
            "device-udid",
            CoreDeviceEnvelopeMode::Modern,
            ACTION_GET_USER_INTERFACE_STYLE,
            empty_input(),
        )
        .await
        .unwrap();
        assert_eq!(
            output,
            dictionary([("style", XpcValue::String("future-style".into()))])
        );

        let request = transport.requests.pop().unwrap();
        let request = request.as_dict().unwrap();
        assert_eq!(request.len(), 6);
        assert_eq!(
            request["CoreDevice.actionIdentifier"].as_str(),
            Some(ACTION_GET_USER_INTERFACE_STYLE)
        );
        assert_eq!(request["CoreDevice.input"], empty_input());
        assert!(!request.contains_key("CoreDevice.featureIdentifier"));
        assert!(!request.contains_key("CoreDevice.action"));

        let mut error_transport = FakeActionTransport {
            requests: Vec::new(),
            response: Some(Ok(XpcMessage {
                flags: 0,
                msg_id: 2,
                body: Some(dictionary([(
                    "error",
                    XpcValue::String("configuration denied".into()),
                )])),
            })),
        };
        let error = invoke_action_with_transport(
            &mut error_transport,
            "device-udid",
            CoreDeviceEnvelopeMode::Modern,
            ACTION_GET_USER_INTERFACE_STYLE,
            empty_input(),
        )
        .await
        .unwrap_err();
        assert!(error.to_string().contains("configuration denied"));

        let mut legacy_transport = FakeActionTransport {
            requests: Vec::new(),
            response: Some(Ok(response_with_output(empty_input()))),
        };
        let legacy = invoke_action_with_transport(
            &mut legacy_transport,
            "device-udid",
            CoreDeviceEnvelopeMode::Legacy,
            ACTION_GET_USER_INTERFACE_STYLE,
            empty_input(),
        )
        .await
        .unwrap_err();
        assert!(matches!(legacy, ConfigurationError::LegacyUnsupported));
        assert!(legacy_transport.requests.is_empty());
    }
}
