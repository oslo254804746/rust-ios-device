//! Shared helpers for CoreDevice XPC feature services.
//!
//! CoreDevice services expose individual operations as feature identifiers. Each call is
//! wrapped in the same `CoreDevice.*` envelope: the current DDI protocol/version,
//! per-call device and invocation UUIDs, feature identifier, and input payload.
//! A separate legacy builder preserves the older protocol for callers that have
//! device-specific evidence requiring it.
//! Service modules keep their feature-specific input/output parsing local and use this
//! module only for the common envelope and error extraction.

#[cfg(feature = "tunnel")]
use std::fmt::{self, Write as _};

use indexmap::IndexMap;

#[cfg(feature = "tunnel")]
use crate::xpc::{XpcClient, XpcError};
use crate::xpc::{XpcMessage, XpcValue};

#[cfg(feature = "tunnel")]
use futures_util::StreamExt;

// CoreDevice's current DDI envelope.  This is the shape emitted by
// pymobiledevice3's CoreDeviceService (protocol 2 / version 629.3): both
// identifiers are per-invocation UUIDs, not the device UDID.
const COREDEVICE_PROTOCOL_VERSION: i64 = 2;
const COREDEVICE_VERSION: &str = "629.3";
const COREDEVICE_VERSION_COMPONENTS: &[u64] = &[629, 3];

// Keep the pre-629 envelope available for callers that have independent
// evidence that a device only accepts the older DDI.  There is no reliable
// version field in every RSD handshake, so callers must opt into this mode;
// ordinary calls use the current reference envelope above.
const LEGACY_COREDEVICE_PROTOCOL_VERSION: i64 = 0;
const LEGACY_COREDEVICE_VERSION: &str = "325.3";
const LEGACY_COREDEVICE_VERSION_COMPONENTS: &[u64] = &[325, 3];

/// Select the CoreDevice request envelope used for a feature invocation.
///
/// `Modern` is the current DDI protocol. `Legacy` is retained for the few
/// existing callers that have device-specific evidence for the pre-629
/// envelope; new CoreDevice services must reject it when their daemon only
/// accepts the modern contract.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum CoreDeviceEnvelopeMode {
    #[default]
    Modern,
    Legacy,
}

#[cfg(feature = "tunnel")]
const STREAM_STATUS_KEY: &str = "CoreDevice.XPCMessageKey.sideChannelStatus";
#[cfg(feature = "tunnel")]
const STREAM_PUSHING_KEY: &str = "pushing";
#[cfg(feature = "tunnel")]
const STREAM_ELEMENTS_KEY: &str = "elements";
#[cfg(feature = "tunnel")]
const STREAM_FINISH_KEY: &str = "finishStreaming";
#[cfg(feature = "tunnel")]
const STREAM_RECEIVED_ERROR_KEY: &str = "receivedError";
#[cfg(feature = "tunnel")]
const STREAM_ERROR_LIMIT: usize = 512;

pub(crate) fn build_request(
    _device_identifier: &str,
    feature_identifier: &str,
    input: XpcValue,
) -> XpcValue {
    build_modern_request(feature_identifier, input)
}

fn build_modern_request(feature_identifier: &str, input: XpcValue) -> XpcValue {
    let request_device_identifier = uuid::Uuid::new_v4().to_string();
    build_request_with_version(
        &request_device_identifier,
        feature_identifier,
        input,
        COREDEVICE_PROTOCOL_VERSION,
        COREDEVICE_VERSION_COMPONENTS,
        COREDEVICE_VERSION,
    )
}

/// Build an action-only CoreDevice invocation.
///
/// ConfigurationService uses `CoreDevice.actionIdentifier` and deliberately
/// omits `CoreDevice.featureIdentifier`/`CoreDevice.action`; this is the exact
/// action-only shape emitted by pymobiledevice3's `CoreDeviceService.invoke`.
pub(crate) fn build_action_request(
    _device_identifier: &str,
    action_identifier: &str,
    input: XpcValue,
) -> XpcValue {
    let request_device_identifier = uuid::Uuid::new_v4().to_string();
    let mut coredevice_version = IndexMap::new();
    coredevice_version.insert(
        "components".to_string(),
        XpcValue::Array(
            COREDEVICE_VERSION_COMPONENTS
                .iter()
                .copied()
                .map(XpcValue::Uint64)
                .collect(),
        ),
    );
    coredevice_version.insert(
        "originalComponentsCount".to_string(),
        XpcValue::Int64(COREDEVICE_VERSION_COMPONENTS.len() as i64),
    );
    coredevice_version.insert(
        "stringValue".to_string(),
        XpcValue::String(COREDEVICE_VERSION.to_string()),
    );

    XpcValue::Dictionary(IndexMap::from([
        (
            "CoreDevice.CoreDeviceDDIProtocolVersion".to_string(),
            XpcValue::Int64(COREDEVICE_PROTOCOL_VERSION),
        ),
        (
            "CoreDevice.actionIdentifier".to_string(),
            XpcValue::String(action_identifier.to_string()),
        ),
        (
            "CoreDevice.coreDeviceVersion".to_string(),
            XpcValue::Dictionary(coredevice_version),
        ),
        (
            "CoreDevice.deviceIdentifier".to_string(),
            XpcValue::String(request_device_identifier),
        ),
        ("CoreDevice.input".to_string(), input),
        (
            "CoreDevice.invocationIdentifier".to_string(),
            XpcValue::String(uuid::Uuid::new_v4().to_string()),
        ),
    ]))
}

/// Build the pre-modern CoreDevice envelope for a caller that has explicitly
/// selected legacy compatibility.  The old wire contract uses the supplied
/// device identifier and protocol 0; keeping it separate prevents an
/// accidental downgrade after a modern request has had side effects.
pub(crate) fn build_legacy_request(
    device_identifier: &str,
    feature_identifier: &str,
    input: XpcValue,
) -> XpcValue {
    build_request_with_version(
        device_identifier,
        feature_identifier,
        input,
        LEGACY_COREDEVICE_PROTOCOL_VERSION,
        LEGACY_COREDEVICE_VERSION_COMPONENTS,
        LEGACY_COREDEVICE_VERSION,
    )
}

fn build_request_with_version(
    device_identifier: &str,
    feature_identifier: &str,
    input: XpcValue,
    protocol_version: i64,
    version_components: &[u64],
    version_string: &str,
) -> XpcValue {
    // The version fields mirror reference CoreDevice clients. Keep this one
    // constructor shared by ordinary and streaming requests so their envelope
    // dictionaries cannot drift apart.
    let mut coredevice_version = IndexMap::new();
    coredevice_version.insert(
        "components".to_string(),
        XpcValue::Array(
            version_components
                .iter()
                .copied()
                .map(XpcValue::Uint64)
                .collect(),
        ),
    );
    coredevice_version.insert(
        "originalComponentsCount".to_string(),
        XpcValue::Int64(version_components.len() as i64),
    );
    coredevice_version.insert(
        "stringValue".to_string(),
        XpcValue::String(version_string.to_string()),
    );

    XpcValue::Dictionary(IndexMap::from([
        (
            "CoreDevice.CoreDeviceDDIProtocolVersion".to_string(),
            XpcValue::Int64(protocol_version),
        ),
        (
            "CoreDevice.action".to_string(),
            XpcValue::Dictionary(IndexMap::new()),
        ),
        (
            "CoreDevice.coreDeviceVersion".to_string(),
            XpcValue::Dictionary(coredevice_version),
        ),
        (
            "CoreDevice.deviceIdentifier".to_string(),
            XpcValue::String(device_identifier.to_string()),
        ),
        (
            "CoreDevice.featureIdentifier".to_string(),
            XpcValue::String(feature_identifier.to_string()),
        ),
        ("CoreDevice.input".to_string(), input),
        (
            "CoreDevice.invocationIdentifier".to_string(),
            XpcValue::String(uuid::Uuid::new_v4().to_string()),
        ),
    ]))
}

/// Build the CoreDevice input container used by side-channel streaming
/// features. The ordinary list request is deliberately kept separate: on
/// iOS 26, sending that request to a service advertising `streamapplist` can
/// leave the daemon waiting forever.
#[cfg(feature = "tunnel")]
pub(crate) fn build_stream_request(
    _device_identifier: &str,
    feature_identifier: &str,
    input: XpcValue,
) -> XpcValue {
    let input = XpcValue::Dictionary(IndexMap::from([
        ("actualInput".to_string(), input),
        (
            "streamProxy".to_string(),
            XpcValue::Dictionary(IndexMap::from([(
                "sideChannel".to_string(),
                XpcValue::Uuid(*uuid::Uuid::new_v4().as_bytes()),
            )])),
        ),
    ]));
    // `build_modern_request` supplies the per-invocation client UUID, not the
    // device UDID. The service endpoint already identifies the target device;
    // reusing the UDID here causes the streaming DDI envelope to differ from
    // the accepted wire shape.
    build_modern_request(feature_identifier, input)
}

/// Start a CoreDevice side-channel invocation and yield each pushed element.
///
/// The low-level [`XpcClient::stream_invoke`] owns only the transport loop. This
/// layer owns the CoreDevice stream envelope and status protocol so other
/// streaming CoreDevice features can reuse the same implementation.
#[cfg(feature = "tunnel")]
pub(crate) fn stream_invoke<'a>(
    client: &'a mut XpcClient,
    device_identifier: &'a str,
    feature_identifier: &'a str,
    input: XpcValue,
) -> impl futures_core::Stream<Item = Result<XpcValue, XpcError>> + 'a {
    let request = build_stream_request(device_identifier, feature_identifier, input);

    async_stream::try_stream! {
        let mut messages = Box::pin(client.stream_invoke(request));
        while let Some(message) = messages.next().await {
            let message = message?;
            // RemoteXPC can interleave bodyless acknowledgements (or an empty
            // dictionary control message) before the side-channel status. The
            // reference receive_response loop discards these frames; they are
            // transport control, not an unknown stream status.
            if message.body.as_ref().map_or(true, |body| {
                matches!(body, XpcValue::Dictionary(values) if values.is_empty())
            }) {
                continue;
            }
            match parse_stream_status(message)? {
                StreamStatus::Elements(elements) => {
                    for element in elements {
                        yield element;
                    }
                }
                StreamStatus::Finished => return,
            }
        }

        // A CoreDevice stream is terminated by an explicit finishStreaming
        // status. Treat a transport EOF as a protocol failure instead of
        // silently returning a partial list.
        Err(XpcError::Tls(
            "CoreDevice stream ended before finishStreaming".to_string(),
        ))?;
    }
}

#[cfg(feature = "tunnel")]
#[derive(Debug, PartialEq)]
enum StreamStatus {
    Elements(Vec<XpcValue>),
    Finished,
}

#[cfg(feature = "tunnel")]
fn parse_stream_status(message: XpcMessage) -> Result<StreamStatus, XpcError> {
    let body = message
        .body
        .ok_or_else(|| XpcError::Tls("CoreDevice stream response is missing a body".to_string()))?;
    let body = body.as_dict().ok_or_else(|| {
        XpcError::Tls("CoreDevice stream response body is not a dictionary".to_string())
    })?;
    let status = body.get(STREAM_STATUS_KEY).ok_or_else(|| {
        XpcError::Tls(format!(
            "CoreDevice stream response is missing {STREAM_STATUS_KEY}"
        ))
    })?;
    let status = status.as_dict().ok_or_else(|| {
        XpcError::Tls(format!(
            "CoreDevice stream status {STREAM_STATUS_KEY} is not a dictionary"
        ))
    })?;

    if let Some(error) = status.get(STREAM_RECEIVED_ERROR_KEY) {
        const PREFIX: &str = "CoreDevice stream receivedError: ";
        return Err(XpcError::Tls(format!(
            "{PREFIX}{}",
            bounded_debug(error, STREAM_ERROR_LIMIT.saturating_sub(PREFIX.len()))
        )));
    }
    if status.contains_key(STREAM_FINISH_KEY) {
        return Ok(StreamStatus::Finished);
    }

    let pushing = status
        .get(STREAM_PUSHING_KEY)
        .and_then(XpcValue::as_dict)
        .ok_or_else(|| {
            XpcError::Tls(format!(
                "CoreDevice stream status has neither {STREAM_PUSHING_KEY} nor {STREAM_FINISH_KEY}"
            ))
        })?;
    let elements = pushing
        .get(STREAM_ELEMENTS_KEY)
        .and_then(|value| match value {
            XpcValue::Array(elements) => Some(elements.clone()),
            _ => None,
        })
        .ok_or_else(|| {
            XpcError::Tls(format!(
                "CoreDevice stream {STREAM_PUSHING_KEY} status is missing an elements array"
            ))
        })?;

    Ok(StreamStatus::Elements(elements))
}

#[cfg(feature = "tunnel")]
fn bounded_debug(value: &XpcValue, limit: usize) -> String {
    struct BoundedFormatter {
        output: String,
        limit: usize,
        truncated: bool,
    }

    impl fmt::Write for BoundedFormatter {
        fn write_str(&mut self, value: &str) -> fmt::Result {
            const MARKER_LEN: usize = 3;
            let limit = self.limit.saturating_sub(MARKER_LEN);
            if self.output.len() >= limit {
                self.truncated = true;
                return Err(fmt::Error);
            }

            let remaining = limit - self.output.len();
            if value.len() <= remaining {
                self.output.push_str(value);
                return Ok(());
            }

            let mut end = remaining;
            while end > 0 && !value.is_char_boundary(end) {
                end -= 1;
            }
            self.output.push_str(&value[..end]);
            self.truncated = true;
            Err(fmt::Error)
        }
    }

    let mut formatter = BoundedFormatter {
        output: String::new(),
        limit,
        truncated: false,
    };
    let _ = write!(&mut formatter, "{value:?}");
    if formatter.truncated {
        formatter.output.push_str("...");
    }
    formatter.output
}

pub(crate) fn parse_output(response: XpcMessage) -> Result<XpcValue, String> {
    let body = response
        .body
        .ok_or_else(|| "missing CoreDevice response body".to_string())?;
    let dict = body
        .as_dict()
        .ok_or_else(|| format!("CoreDevice response body is not a dictionary: {body:?}"))?;

    if let Some(output) = dict.get("CoreDevice.output") {
        return Ok(output.clone());
    }

    ensure_no_error(&body)?;

    Err(format!(
        "CoreDevice response missing CoreDevice.output: {body:?}"
    ))
}

pub(crate) fn output(value: &XpcValue) -> Option<&XpcValue> {
    value.as_dict()?.get("CoreDevice.output")
}

pub(crate) fn ensure_no_error(value: &XpcValue) -> Result<(), String> {
    if let Some(message) = error_message(value) {
        return Err(message);
    }
    Ok(())
}

pub(crate) fn error_message(value: &XpcValue) -> Option<String> {
    let dict = value.as_dict()?;
    // CoreDevice errors can arrive at several nesting levels depending on the feature.
    // Search the common envelopes first, then recurse through userInfo/wrapped errors
    // to surface the human-readable description instead of a raw dictionary dump.
    for key in ["CoreDevice.error", "error", "Error", "NSError", "userInfo"] {
        if let Some(found) = dict.get(key) {
            if let Some(message) = nested_error_message(found) {
                return Some(message);
            }
            return Some(format!("{found:?}"));
        }
    }
    None
}

fn nested_error_message(value: &XpcValue) -> Option<String> {
    match value {
        XpcValue::String(message) => Some(message.clone()),
        XpcValue::Dictionary(dict) => {
            for key in [
                "message",
                "localizedDescription",
                "LocalizedDescription",
                "NSLocalizedDescription",
                "description",
            ] {
                if let Some(XpcValue::String(message)) = dict.get(key) {
                    return Some(message.clone());
                }
            }
            for key in ["userInfo", "wrappedError", "underlyingError"] {
                if let Some(message) = dict.get(key).and_then(nested_error_message) {
                    return Some(message);
                }
            }
            None
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use indexmap::IndexMap;

    use super::*;
    use crate::xpc::{XpcMessage, XpcValue};

    #[test]
    fn build_request_wraps_feature_invocation() {
        let request = build_request(
            "DEVICE-ID",
            "com.apple.coredevice.feature.test",
            XpcValue::Dictionary(IndexMap::new()),
        );
        let dict = request.as_dict().expect("request should be a dictionary");

        assert_eq!(
            dict["CoreDevice.featureIdentifier"].as_str(),
            Some("com.apple.coredevice.feature.test")
        );
        let device_identifier = dict["CoreDevice.deviceIdentifier"]
            .as_str()
            .expect("modern device identifier should be a string");
        assert!(uuid::Uuid::parse_str(device_identifier).is_ok());
        assert_ne!(device_identifier, "DEVICE-ID");
        assert_eq!(
            dict["CoreDevice.CoreDeviceDDIProtocolVersion"],
            XpcValue::Int64(COREDEVICE_PROTOCOL_VERSION)
        );
        let version = dict["CoreDevice.coreDeviceVersion"].as_dict().unwrap();
        assert_eq!(
            version["components"],
            XpcValue::Array(vec![XpcValue::Uint64(629), XpcValue::Uint64(3)])
        );
        assert_eq!(version["originalComponentsCount"], XpcValue::Int64(2));
        assert_eq!(version["stringValue"].as_str(), Some(COREDEVICE_VERSION));
        assert!(dict["CoreDevice.invocationIdentifier"]
            .as_str()
            .and_then(|value| uuid::Uuid::parse_str(value).ok())
            .is_some());
        assert_ne!(
            dict["CoreDevice.deviceIdentifier"],
            dict["CoreDevice.invocationIdentifier"]
        );
    }

    #[test]
    fn modern_request_uses_fresh_device_and_invocation_identifiers() {
        let first = build_request("same-device", "feature", XpcValue::Null);
        let second = build_request("same-device", "feature", XpcValue::Null);
        let first = first.as_dict().unwrap();
        let second = second.as_dict().unwrap();

        assert_ne!(
            first["CoreDevice.deviceIdentifier"],
            second["CoreDevice.deviceIdentifier"]
        );
        assert_ne!(
            first["CoreDevice.invocationIdentifier"],
            second["CoreDevice.invocationIdentifier"]
        );
    }

    #[test]
    fn action_request_matches_configuration_envelope_exactly() {
        let request = build_action_request(
            "DEVICE-ID",
            "com.apple.coredevice.action.setuserinterfacestyle",
            XpcValue::Dictionary(IndexMap::from([(
                "style".into(),
                XpcValue::String("dark".into()),
            )])),
        );
        let dict = request.as_dict().unwrap();
        assert_eq!(dict.len(), 6);
        assert_eq!(
            dict["CoreDevice.CoreDeviceDDIProtocolVersion"],
            XpcValue::Int64(2)
        );
        assert_eq!(
            dict["CoreDevice.actionIdentifier"].as_str(),
            Some("com.apple.coredevice.action.setuserinterfacestyle")
        );
        assert!(dict["CoreDevice.actionIdentifier"].as_str().is_some());
        assert!(!dict.contains_key("CoreDevice.action"));
        assert!(!dict.contains_key("CoreDevice.featureIdentifier"));
        assert_eq!(
            dict["CoreDevice.input"].as_dict().unwrap()["style"].as_str(),
            Some("dark")
        );
        let version = dict["CoreDevice.coreDeviceVersion"].as_dict().unwrap();
        assert_eq!(
            version["components"],
            XpcValue::Array(vec![XpcValue::Uint64(629), XpcValue::Uint64(3)])
        );
        assert_eq!(version["originalComponentsCount"], XpcValue::Int64(2));
        assert_eq!(version["stringValue"].as_str(), Some("629.3"));
        assert!(
            uuid::Uuid::parse_str(dict["CoreDevice.deviceIdentifier"].as_str().unwrap()).is_ok()
        );
        assert!(
            uuid::Uuid::parse_str(dict["CoreDevice.invocationIdentifier"].as_str().unwrap())
                .is_ok()
        );
    }

    #[test]
    fn legacy_request_preserves_old_dictionary_route() {
        let request = build_legacy_request(
            "DEVICE-ID",
            "com.apple.coredevice.feature.test",
            XpcValue::Null,
        );
        let dict = request.as_dict().unwrap();

        assert_eq!(
            dict["CoreDevice.deviceIdentifier"].as_str(),
            Some("DEVICE-ID")
        );
        assert_eq!(
            dict["CoreDevice.CoreDeviceDDIProtocolVersion"],
            XpcValue::Int64(LEGACY_COREDEVICE_PROTOCOL_VERSION)
        );
        let version = dict["CoreDevice.coreDeviceVersion"].as_dict().unwrap();
        assert_eq!(
            version["components"],
            XpcValue::Array(vec![XpcValue::Uint64(325), XpcValue::Uint64(3)])
        );
        assert_eq!(
            version["stringValue"].as_str(),
            Some(LEGACY_COREDEVICE_VERSION)
        );
    }

    #[test]
    fn parse_output_extracts_coredevice_output() {
        let response = XpcMessage {
            flags: 0,
            msg_id: 1,
            body: Some(XpcValue::Dictionary(IndexMap::from([(
                "CoreDevice.output".to_string(),
                XpcValue::String("ok".into()),
            )]))),
        };

        assert_eq!(
            parse_output(response).unwrap(),
            XpcValue::String("ok".into())
        );
    }

    #[test]
    fn ensure_no_error_reads_nested_localized_description() {
        let body = XpcValue::Dictionary(IndexMap::from([(
            "CoreDevice.error".to_string(),
            XpcValue::Dictionary(IndexMap::from([(
                "userInfo".to_string(),
                XpcValue::Dictionary(IndexMap::from([(
                    "NSLocalizedDescription".to_string(),
                    XpcValue::String("denied".into()),
                )])),
            )])),
        )]));

        assert_eq!(ensure_no_error(&body).unwrap_err(), "denied");
    }

    #[cfg(feature = "tunnel")]
    #[test]
    fn build_stream_request_wraps_actual_input_and_side_channel_proxy() {
        let request = build_stream_request(
            "DEVICE-ID",
            "com.apple.coredevice.feature.streamapplist",
            XpcValue::Dictionary(IndexMap::from([
                ("includeAppClips".into(), XpcValue::Bool(true)),
                ("requireContainerAccess".into(), XpcValue::Bool(false)),
            ])),
        );
        let dict = request.as_dict().expect("request should be a dictionary");
        assert_eq!(
            dict["CoreDevice.featureIdentifier"].as_str(),
            Some("com.apple.coredevice.feature.streamapplist")
        );
        let request_device_identifier = dict["CoreDevice.deviceIdentifier"]
            .as_str()
            .expect("stream device identifier should be a string");
        assert!(uuid::Uuid::parse_str(request_device_identifier).is_ok());
        assert_eq!(
            dict["CoreDevice.CoreDeviceDDIProtocolVersion"],
            XpcValue::Int64(COREDEVICE_PROTOCOL_VERSION)
        );
        let version = dict["CoreDevice.coreDeviceVersion"]
            .as_dict()
            .expect("stream version should be a dictionary");
        assert_eq!(
            version["components"],
            XpcValue::Array(vec![XpcValue::Uint64(629), XpcValue::Uint64(3)])
        );
        assert_eq!(version["originalComponentsCount"], XpcValue::Int64(2));
        assert_eq!(version["stringValue"].as_str(), Some(COREDEVICE_VERSION));

        let input = dict["CoreDevice.input"]
            .as_dict()
            .expect("stream input should be a dictionary");
        assert!(input["actualInput"].as_dict().is_some());
        let proxy = input["streamProxy"]
            .as_dict()
            .expect("stream proxy should be a dictionary");
        assert!(matches!(proxy["sideChannel"], XpcValue::Uuid(_)));
        assert!(dict["CoreDevice.action"].as_dict().is_some());
        assert!(dict["CoreDevice.invocationIdentifier"]
            .as_str()
            .and_then(|value| uuid::Uuid::parse_str(value).ok())
            .is_some());
        assert_ne!(
            dict["CoreDevice.deviceIdentifier"],
            dict["CoreDevice.invocationIdentifier"]
        );
    }

    #[cfg(feature = "tunnel")]
    fn stream_message(status: XpcValue) -> XpcMessage {
        XpcMessage {
            flags: 0,
            msg_id: 1,
            body: Some(XpcValue::Dictionary(IndexMap::from([(
                STREAM_STATUS_KEY.to_string(),
                status,
            )]))),
        }
    }

    #[cfg(feature = "tunnel")]
    #[test]
    fn parse_stream_status_yields_multiple_batches_and_finish() {
        let first = parse_stream_status(stream_message(XpcValue::Dictionary(IndexMap::from([(
            STREAM_PUSHING_KEY.to_string(),
            XpcValue::Dictionary(IndexMap::from([(
                STREAM_ELEMENTS_KEY.to_string(),
                XpcValue::Array(vec![XpcValue::String("first".into())]),
            )])),
        )]))))
        .unwrap();
        assert_eq!(
            first,
            StreamStatus::Elements(vec![XpcValue::String("first".into())])
        );

        let second = parse_stream_status(stream_message(XpcValue::Dictionary(IndexMap::from([(
            STREAM_PUSHING_KEY.to_string(),
            XpcValue::Dictionary(IndexMap::from([(
                STREAM_ELEMENTS_KEY.to_string(),
                XpcValue::Array(vec![
                    XpcValue::String("second".into()),
                    XpcValue::String("third".into()),
                ]),
            )])),
        )]))))
        .unwrap();
        assert_eq!(
            second,
            StreamStatus::Elements(vec![
                XpcValue::String("second".into()),
                XpcValue::String("third".into()),
            ])
        );

        let finish = parse_stream_status(stream_message(XpcValue::Dictionary(IndexMap::from([(
            STREAM_FINISH_KEY.to_string(),
            XpcValue::Dictionary(IndexMap::new()),
        )]))))
        .unwrap();
        assert_eq!(finish, StreamStatus::Finished);
    }

    #[cfg(feature = "tunnel")]
    #[test]
    fn parse_stream_status_surfaces_received_error_with_bounded_text() {
        let message = stream_message(XpcValue::Dictionary(IndexMap::from([(
            STREAM_RECEIVED_ERROR_KEY.to_string(),
            XpcValue::String("x".repeat(4_096)),
        )])));

        let error = parse_stream_status(message).unwrap_err();
        let XpcError::Tls(error) = error else {
            panic!("stream errors should be protocol errors");
        };
        assert!(error.starts_with("CoreDevice stream receivedError:"));
        assert!(error.len() <= 512);
        assert!(error.ends_with("..."));
    }

    #[cfg(feature = "tunnel")]
    #[test]
    fn parse_stream_status_rejects_eof_and_unknown_status_shapes() {
        let missing_body = parse_stream_status(XpcMessage {
            flags: 0,
            msg_id: 1,
            body: None,
        })
        .unwrap_err();
        assert!(missing_body.to_string().contains("missing a body"));

        let unknown =
            parse_stream_status(stream_message(XpcValue::Dictionary(IndexMap::from([(
                "futureStatus".into(),
                XpcValue::Null,
            )]))))
            .unwrap_err();
        assert!(unknown
            .to_string()
            .contains("neither pushing nor finishStreaming"));

        let missing_elements =
            parse_stream_status(stream_message(XpcValue::Dictionary(IndexMap::from([(
                STREAM_PUSHING_KEY.to_string(),
                XpcValue::Dictionary(IndexMap::new()),
            )]))))
            .unwrap_err();
        assert!(missing_elements
            .to_string()
            .contains("missing an elements array"));
    }
}
