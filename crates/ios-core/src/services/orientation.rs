//! iOS 17+ CoreDevice device orientation control over RemoteXPC/RSD.
//!
//! Unlike configuration actions, orientation uses the remote device-control
//! dictionary protocol directly. The request intentionally has no
//! `CoreDevice.*` wrapper; this is the wire shape used by pymobiledevice3's
//! `OrientationService`.

use std::fmt;
use std::future::Future;
use std::pin::Pin;
use std::str::FromStr;

use indexmap::IndexMap;

use crate::xpc::{XpcClient, XpcError, XpcMessage, XpcValue};

pub use crate::services::coredevice::CoreDeviceEnvelopeMode;

/// RSD service name for device-control orientation actions.
pub const SERVICE_NAME: &str = "com.apple.coredevice.devicecontrol";
/// Feature identifier used by the orientation request dictionary.
pub const ORIENTATION_FEATURE: &str =
    "com.apple.coredevice.feature.remote.devicecontrol.orientation";

const MESSAGE_TYPE: &str = "OrientationRequest";

/// Errors returned by orientation control.
#[derive(Debug, thiserror::Error)]
pub enum OrientationError {
    /// Underlying RemoteXPC transport failure.
    #[error("xpc error: {0}")]
    Xpc(#[from] XpcError),
    /// The daemon returned an unsupported response shape or value.
    #[error("orientation protocol error: {0}")]
    Protocol(String),
    /// Orientation uses the modern remote device-control protocol only.
    #[error("CoreDevice orientation requires the modern envelope; legacy mode is unsupported")]
    LegacyUnsupported,
}

/// Direction for one 90-degree orientation step.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum RotationDirection {
    #[default]
    Left,
    Right,
}

impl RotationDirection {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Left => "left",
            Self::Right => "right",
        }
    }
}

impl FromStr for RotationDirection {
    type Err = String;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "left" => Ok(Self::Left),
            "right" => Ok(Self::Right),
            _ => Err(format!(
                "direction must be 'left' or 'right', got {value:?}"
            )),
        }
    }
}

impl fmt::Display for RotationDirection {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

/// Orientation values returned by the device-control daemon.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DeviceOrientation {
    Portrait,
    LandscapeLeft,
    PortraitUpsideDown,
    LandscapeRight,
    FaceUp,
    FaceDown,
    /// Preserve a future device value instead of losing the response.
    Unknown(String),
}

impl serde::Serialize for DeviceOrientation {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        serializer.serialize_str(self.to_string().as_str())
    }
}

impl<'de> serde::Deserialize<'de> for DeviceOrientation {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let value = <String as serde::Deserialize>::deserialize(deserializer)?;
        value.parse().map_err(serde::de::Error::custom)
    }
}

impl FromStr for DeviceOrientation {
    type Err = String;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        Ok(match value {
            "portrait" => Self::Portrait,
            "landscapeLeft" => Self::LandscapeLeft,
            "portraitUpsideDown" => Self::PortraitUpsideDown,
            "landscapeRight" => Self::LandscapeRight,
            "faceUp" => Self::FaceUp,
            "faceDown" => Self::FaceDown,
            other if !other.is_empty() => Self::Unknown(other.to_string()),
            _ => return Err("orientation value must not be empty".into()),
        })
    }
}

impl fmt::Display for DeviceOrientation {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Portrait => formatter.write_str("portrait"),
            Self::LandscapeLeft => formatter.write_str("landscapeLeft"),
            Self::PortraitUpsideDown => formatter.write_str("portraitUpsideDown"),
            Self::LandscapeRight => formatter.write_str("landscapeRight"),
            Self::FaceUp => formatter.write_str("faceUp"),
            Self::FaceDown => formatter.write_str("faceDown"),
            Self::Unknown(value) => formatter.write_str(value),
        }
    }
}

/// Resulting orientation state returned after rotation.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct OrientationState {
    #[serde(rename = "currentDeviceOrientation")]
    pub current_device_orientation: DeviceOrientation,
    #[serde(rename = "currentDeviceNonFlatOrientation")]
    pub current_device_non_flat_orientation: DeviceOrientation,
    #[serde(rename = "currentDeviceOrientationLocked")]
    pub current_device_orientation_locked: bool,
}

/// A client for `com.apple.coredevice.devicecontrol` orientation requests.
pub struct OrientationServiceClient {
    client: XpcClient,
    envelope_mode: CoreDeviceEnvelopeMode,
    service_features: Option<Vec<String>>,
}

/// Compatibility alias following pymobiledevice3's service naming.
pub type OrientationService = OrientationServiceClient;

/// Small transport seam shared by the real client and protocol-level tests.
/// It keeps the request/response contract testable without requiring a device.
trait OrientationTransport {
    fn call<'a>(
        &'a mut self,
        request: XpcValue,
    ) -> Pin<Box<dyn Future<Output = Result<XpcMessage, XpcError>> + 'a>>;
}

impl OrientationTransport for XpcClient {
    fn call<'a>(
        &'a mut self,
        request: XpcValue,
    ) -> Pin<Box<dyn Future<Output = Result<XpcMessage, XpcError>> + 'a>> {
        Box::pin(XpcClient::call(self, request))
    }
}

impl OrientationServiceClient {
    /// Create a client using the modern orientation request protocol.
    pub fn new(client: XpcClient) -> Self {
        Self {
            client,
            envelope_mode: CoreDeviceEnvelopeMode::Modern,
            service_features: None,
        }
    }

    /// Create a client using RSD features from the exact resolved service.
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

    /// Create a client with an explicit envelope mode.
    pub fn new_with_mode(client: XpcClient, envelope_mode: CoreDeviceEnvelopeMode) -> Self {
        Self {
            client,
            envelope_mode,
            service_features: None,
        }
    }

    /// Return whether this client's resolved RSD feature list includes
    /// orientation. An empty list remains permissive, matching RSD semantics.
    pub fn supports_orientation(&self) -> bool {
        orientation_feature_is_advertised(self.service_features.as_deref())
    }

    /// Rotate the device once by 90 degrees and return its resulting state.
    pub async fn rotate(
        &mut self,
        direction: RotationDirection,
    ) -> Result<OrientationState, OrientationError> {
        rotate_with_transport(
            &mut self.client,
            self.envelope_mode,
            self.service_features.as_deref(),
            direction,
        )
        .await
    }
}

async fn rotate_with_transport<T: OrientationTransport>(
    transport: &mut T,
    envelope_mode: CoreDeviceEnvelopeMode,
    service_features: Option<&[String]>,
    direction: RotationDirection,
) -> Result<OrientationState, OrientationError> {
    ensure_modern(envelope_mode)?;
    if !orientation_feature_is_advertised(service_features) {
        return Err(OrientationError::Protocol(format!(
            "CoreDevice feature {ORIENTATION_FEATURE} is not advertised by RSD"
        )));
    }

    let response = transport.call(build_rotate_request(direction)).await?;
    parse_orientation_response(response.body)
}

fn ensure_modern(mode: CoreDeviceEnvelopeMode) -> Result<(), OrientationError> {
    if mode == CoreDeviceEnvelopeMode::Legacy {
        return Err(OrientationError::LegacyUnsupported);
    }
    Ok(())
}

fn orientation_feature_is_advertised(features: Option<&[String]>) -> bool {
    features.map_or(true, |features| {
        features.is_empty()
            || features
                .iter()
                .any(|feature| feature == ORIENTATION_FEATURE)
    })
}

fn build_rotate_request(direction: RotationDirection) -> XpcValue {
    XpcValue::Dictionary(IndexMap::from([
        (
            "featureIdentifier".to_string(),
            XpcValue::String(ORIENTATION_FEATURE.to_string()),
        ),
        (
            "messageType".to_string(),
            XpcValue::String(MESSAGE_TYPE.to_string()),
        ),
        (
            "payload".to_string(),
            XpcValue::Dictionary(IndexMap::from([(
                "rotate".to_string(),
                XpcValue::Dictionary(IndexMap::from([(
                    "_0".to_string(),
                    XpcValue::String(direction.as_str().to_string()),
                )])),
            )])),
        ),
    ]))
}

fn parse_orientation_response(
    body: Option<XpcValue>,
) -> Result<OrientationState, OrientationError> {
    let body = body.ok_or_else(|| {
        OrientationError::Protocol("orientation response is missing a body".into())
    })?;
    crate::services::coredevice::ensure_no_error(&body).map_err(OrientationError::Protocol)?;
    let dict = body.as_dict().ok_or_else(|| {
        OrientationError::Protocol(format!(
            "orientation response is not a dictionary: {body:?}"
        ))
    })?;
    let current_device_orientation = required_orientation(dict, "currentDeviceOrientation")?;
    let current_device_non_flat_orientation =
        required_orientation(dict, "currentDeviceNonFlatOrientation")?;
    let current_device_orientation_locked = dict
        .get("currentDeviceOrientationLocked")
        .and_then(xpc_bool)
        .ok_or_else(|| {
            OrientationError::Protocol(format!(
                "orientation response missing boolean currentDeviceOrientationLocked: {body:?}"
            ))
        })?;
    Ok(OrientationState {
        current_device_orientation,
        current_device_non_flat_orientation,
        current_device_orientation_locked,
    })
}

fn xpc_bool(value: &XpcValue) -> Option<bool> {
    match value {
        XpcValue::Bool(value) => Some(*value),
        _ => None,
    }
}

fn required_orientation(
    dict: &IndexMap<String, XpcValue>,
    key: &str,
) -> Result<DeviceOrientation, OrientationError> {
    let value = dict
        .get(key)
        .and_then(XpcValue::as_str)
        .ok_or_else(|| OrientationError::Protocol(format!("orientation response missing {key}")))?;
    value.parse().map_err(OrientationError::Protocol)
}

#[cfg(test)]
mod tests {
    use super::*;

    struct FakeOrientationTransport {
        requests: Vec<XpcValue>,
        response: Option<Result<XpcMessage, XpcError>>,
    }

    impl OrientationTransport for FakeOrientationTransport {
        fn call<'a>(
            &'a mut self,
            request: XpcValue,
        ) -> Pin<Box<dyn Future<Output = Result<XpcMessage, XpcError>> + 'a>> {
            self.requests.push(request);
            let response = self
                .response
                .take()
                .expect("fake orientation transport called more than once");
            Box::pin(async move { response })
        }
    }

    fn orientation_response(orientation: &str) -> XpcMessage {
        XpcMessage {
            flags: 0,
            msg_id: 1,
            body: Some(XpcValue::Dictionary(IndexMap::from([
                (
                    "currentDeviceOrientation".into(),
                    XpcValue::String(orientation.into()),
                ),
                (
                    "currentDeviceNonFlatOrientation".into(),
                    XpcValue::String(orientation.into()),
                ),
                (
                    "currentDeviceOrientationLocked".into(),
                    XpcValue::Bool(false),
                ),
            ]))),
        }
    }

    #[test]
    fn rotation_request_matches_reference_shape() {
        let request = build_rotate_request(RotationDirection::Left);
        let dict = request.as_dict().unwrap();
        assert_eq!(
            dict["featureIdentifier"].as_str(),
            Some(ORIENTATION_FEATURE)
        );
        assert_eq!(dict["messageType"].as_str(), Some(MESSAGE_TYPE));
        let direction = dict["payload"].as_dict().unwrap()["rotate"]
            .as_dict()
            .unwrap()["_0"]
            .as_str();
        assert_eq!(direction, Some("left"));
        assert!(!dict.contains_key("CoreDevice.input"));
    }

    #[test]
    fn direction_validation_is_strict() {
        assert_eq!(
            "right".parse::<RotationDirection>().unwrap(),
            RotationDirection::Right
        );
        assert!("up".parse::<RotationDirection>().is_err());
    }

    #[test]
    fn parses_orientation_response_and_service_errors() {
        let body = Some(XpcValue::Dictionary(IndexMap::from([
            (
                "currentDeviceOrientation".into(),
                XpcValue::String("landscapeLeft".into()),
            ),
            (
                "currentDeviceNonFlatOrientation".into(),
                XpcValue::String("landscapeLeft".into()),
            ),
            (
                "currentDeviceOrientationLocked".into(),
                XpcValue::Bool(false),
            ),
        ])));
        let state = parse_orientation_response(body).unwrap();
        assert_eq!(
            state.current_device_orientation,
            DeviceOrientation::LandscapeLeft
        );
        assert!(!state.current_device_orientation_locked);

        let error = parse_orientation_response(Some(XpcValue::Dictionary(IndexMap::from([(
            "error".into(),
            XpcValue::String("not supported".into()),
        )]))))
        .unwrap_err();
        assert!(error.to_string().contains("not supported"));
    }

    #[test]
    fn serializes_unknown_orientation_as_the_wire_string() {
        let value = serde_json::to_value(DeviceOrientation::Unknown("tilted".into())).unwrap();
        assert_eq!(value, serde_json::json!("tilted"));
        let decoded: DeviceOrientation =
            serde_json::from_value(serde_json::json!("tilted")).unwrap();
        assert_eq!(decoded, DeviceOrientation::Unknown("tilted".into()));
    }

    #[test]
    fn feature_and_envelope_routing_are_conservative() {
        let advertised = vec![ORIENTATION_FEATURE.to_string()];
        assert!(orientation_feature_is_advertised(Some(&advertised)));
        assert!(orientation_feature_is_advertised(Some(&[])));
        assert!(orientation_feature_is_advertised(None));
        assert!(!orientation_feature_is_advertised(Some(&[
            "other".to_string()
        ])));
        assert!(matches!(
            ensure_modern(CoreDeviceEnvelopeMode::Legacy),
            Err(OrientationError::LegacyUnsupported)
        ));
    }

    #[tokio::test]
    async fn mock_transport_round_trip_checks_orientation_request_and_response() {
        assert_eq!(SERVICE_NAME, "com.apple.coredevice.devicecontrol");
        let mut transport = FakeOrientationTransport {
            requests: Vec::new(),
            response: Some(Ok(orientation_response("futureOrientation"))),
        };
        let features = [ORIENTATION_FEATURE.to_string()];
        let state = rotate_with_transport(
            &mut transport,
            CoreDeviceEnvelopeMode::Modern,
            Some(&features),
            RotationDirection::Right,
        )
        .await
        .unwrap();
        assert_eq!(
            state.current_device_orientation,
            DeviceOrientation::Unknown("futureOrientation".into())
        );

        let request = transport.requests.pop().unwrap();
        assert_eq!(request, build_rotate_request(RotationDirection::Right));
        let request = request.as_dict().unwrap();
        assert_eq!(
            request["featureIdentifier"].as_str(),
            Some(ORIENTATION_FEATURE)
        );
        assert_eq!(request["messageType"].as_str(), Some(MESSAGE_TYPE));
        assert!(!request.contains_key("CoreDevice.input"));

        let mut error_transport = FakeOrientationTransport {
            requests: Vec::new(),
            response: Some(Ok(XpcMessage {
                flags: 0,
                msg_id: 2,
                body: Some(XpcValue::Dictionary(IndexMap::from([(
                    "error".into(),
                    XpcValue::String("rotation denied".into()),
                )]))),
            })),
        };
        let error = rotate_with_transport(
            &mut error_transport,
            CoreDeviceEnvelopeMode::Modern,
            Some(&features),
            RotationDirection::Left,
        )
        .await
        .unwrap_err();
        assert!(error.to_string().contains("rotation denied"));

        let mut unsupported_transport = FakeOrientationTransport {
            requests: Vec::new(),
            response: Some(Ok(orientation_response("portrait"))),
        };
        let unsupported = rotate_with_transport(
            &mut unsupported_transport,
            CoreDeviceEnvelopeMode::Modern,
            Some(&[String::from("other-feature")]),
            RotationDirection::Left,
        )
        .await
        .unwrap_err();
        assert!(unsupported.to_string().contains(ORIENTATION_FEATURE));
        assert!(unsupported_transport.requests.is_empty());

        let mut legacy_transport = FakeOrientationTransport {
            requests: Vec::new(),
            response: Some(Ok(orientation_response("portrait"))),
        };
        let legacy = rotate_with_transport(
            &mut legacy_transport,
            CoreDeviceEnvelopeMode::Legacy,
            Some(&features),
            RotationDirection::Left,
        )
        .await
        .unwrap_err();
        assert!(matches!(legacy, OrientationError::LegacyUnsupported));
        assert!(legacy_transport.requests.is_empty());
    }
}
