//! Shared helpers for CoreDevice XPC feature services.
//!
//! CoreDevice services expose individual operations as feature identifiers. Each call is
//! wrapped in the same `CoreDevice.*` envelope: target device identifier, CoreDevice
//! protocol version, feature identifier, input payload, and a per-call invocation UUID.
//! Service modules keep their feature-specific input/output parsing local and use this
//! module only for the common envelope and error extraction.

use indexmap::IndexMap;

use crate::xpc::{XpcMessage, XpcValue};

const COREDEVICE_PROTOCOL_VERSION: i64 = 0;
const COREDEVICE_VERSION: &str = "325.3";

pub(crate) fn build_request(
    device_identifier: &str,
    feature_identifier: &str,
    input: XpcValue,
) -> XpcValue {
    // The version fields mirror reference CoreDevice clients. Apple appears to accept
    // this client version for the feature set implemented here, so keep the shape stable
    // unless a new reference trace shows a required version bump.
    let mut coredevice_version = IndexMap::new();
    coredevice_version.insert(
        "components".to_string(),
        XpcValue::Array(vec![XpcValue::Uint64(325), XpcValue::Uint64(3)]),
    );
    coredevice_version.insert("originalComponentsCount".to_string(), XpcValue::Int64(2));
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
        assert_eq!(
            dict["CoreDevice.deviceIdentifier"].as_str(),
            Some("DEVICE-ID")
        );
        assert_eq!(
            dict["CoreDevice.CoreDeviceDDIProtocolVersion"],
            XpcValue::Int64(0)
        );
        let version = dict["CoreDevice.coreDeviceVersion"].as_dict().unwrap();
        assert_eq!(
            version["components"],
            XpcValue::Array(vec![XpcValue::Uint64(325), XpcValue::Uint64(3)])
        );
        assert_eq!(version["originalComponentsCount"], XpcValue::Int64(2));
        assert_eq!(version["stringValue"].as_str(), Some("325.3"));
        assert!(dict["CoreDevice.invocationIdentifier"]
            .as_str()
            .unwrap()
            .contains('-'));
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
}
