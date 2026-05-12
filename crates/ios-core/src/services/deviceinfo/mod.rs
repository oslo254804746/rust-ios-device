//! iOS 17+ CoreDevice device info service via XPC/RSD.

use indexmap::IndexMap;

use crate::xpc::{XpcClient, XpcError, XpcValue};

pub const SERVICE_NAME: &str = "com.apple.coredevice.deviceinfo";

const FEATURE_GET_DEVICE_INFO: &str = "com.apple.coredevice.feature.getdeviceinfo";
const FEATURE_GET_DISPLAY_INFO: &str = "com.apple.coredevice.feature.getdisplayinfo";
const FEATURE_QUERY_MOBILEGESTALT: &str = "com.apple.coredevice.feature.querymobilegestalt";
const FEATURE_GET_LOCKSTATE: &str = "com.apple.coredevice.feature.getlockstate";

#[derive(Debug, thiserror::Error)]
pub enum DeviceInfoError {
    #[error("xpc error: {0}")]
    Xpc(#[from] XpcError),
    #[error("protocol error: {0}")]
    Protocol(String),
}

pub struct DeviceInfoClient {
    client: XpcClient,
    device_identifier: String,
}

impl DeviceInfoClient {
    pub fn new(client: XpcClient, device_identifier: impl Into<String>) -> Self {
        Self {
            client,
            device_identifier: device_identifier.into(),
        }
    }

    pub async fn get_device_info(&mut self) -> Result<XpcValue, DeviceInfoError> {
        self.invoke(
            FEATURE_GET_DEVICE_INFO,
            XpcValue::Dictionary(IndexMap::new()),
        )
        .await
    }

    pub async fn get_display_info(&mut self) -> Result<XpcValue, DeviceInfoError> {
        self.invoke(
            FEATURE_GET_DISPLAY_INFO,
            XpcValue::Dictionary(IndexMap::new()),
        )
        .await
    }

    pub async fn query_mobilegestalt(
        &mut self,
        keys: &[&str],
    ) -> Result<XpcValue, DeviceInfoError> {
        self.invoke(
            FEATURE_QUERY_MOBILEGESTALT,
            build_query_mobilegestalt_input(keys),
        )
        .await
    }

    pub async fn get_lockstate(&mut self) -> Result<XpcValue, DeviceInfoError> {
        self.invoke(FEATURE_GET_LOCKSTATE, XpcValue::Dictionary(IndexMap::new()))
            .await
    }

    pub async fn invoke(
        &mut self,
        feature_identifier: &str,
        input: XpcValue,
    ) -> Result<XpcValue, DeviceInfoError> {
        let response = self
            .client
            .call(build_request(
                &self.device_identifier,
                feature_identifier,
                input,
            ))
            .await?;
        crate::services::coredevice::parse_output(response).map_err(DeviceInfoError::Protocol)
    }
}

fn build_query_mobilegestalt_input(keys: &[&str]) -> XpcValue {
    XpcValue::Dictionary(IndexMap::from([(
        "keys".to_string(),
        XpcValue::Array(
            keys.iter()
                .map(|key| XpcValue::String((*key).to_string()))
                .collect(),
        ),
    )]))
}

fn build_request(device_identifier: &str, feature_identifier: &str, input: XpcValue) -> XpcValue {
    crate::services::coredevice::build_request(device_identifier, feature_identifier, input)
}

pub fn xpc_value_to_plist(value: &XpcValue) -> plist::Value {
    match value {
        XpcValue::Null => plist::Value::String("null".into()),
        XpcValue::Bool(value) => plist::Value::Boolean(*value),
        XpcValue::Int64(value) => plist::Value::Integer((*value).into()),
        XpcValue::Uint64(value) => plist::Value::Integer((*value).into()),
        XpcValue::Double(value) => plist::Value::Real(*value),
        XpcValue::Date(value) => plist::Value::Integer((*value).into()),
        XpcValue::Data(bytes) => plist::Value::Data(bytes.to_vec()),
        XpcValue::String(value) => plist::Value::String(value.clone()),
        XpcValue::Uuid(bytes) => plist::Value::String(uuid::Uuid::from_bytes(*bytes).to_string()),
        XpcValue::Array(values) => {
            plist::Value::Array(values.iter().map(xpc_value_to_plist).collect())
        }
        XpcValue::Dictionary(values) => plist::Value::Dictionary(
            values
                .iter()
                .map(|(key, value)| (key.clone(), xpc_value_to_plist(value)))
                .collect(),
        ),
        XpcValue::FileTransfer { msg_id, data } => {
            plist::Value::Dictionary(plist::Dictionary::from_iter([
                (
                    "msg_id".to_string(),
                    plist::Value::Integer((*msg_id).into()),
                ),
                ("data".to_string(), xpc_value_to_plist(data)),
            ]))
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::xpc::{XpcMessage, XpcValue};

    use super::*;

    #[test]
    fn build_query_mobilegestalt_request_uses_coredevice_envelope() {
        let request = build_request(
            "DEVICE-ID",
            FEATURE_QUERY_MOBILEGESTALT,
            build_query_mobilegestalt_input(&["ProductVersion", "MainScreenCanvasSizes"]),
        );
        let dict = request.as_dict().expect("request should be a dictionary");

        assert_eq!(
            dict["CoreDevice.featureIdentifier"].as_str(),
            Some(FEATURE_QUERY_MOBILEGESTALT)
        );
        assert_eq!(
            dict["CoreDevice.deviceIdentifier"].as_str(),
            Some("DEVICE-ID")
        );

        let input = dict["CoreDevice.input"]
            .as_dict()
            .expect("CoreDevice.input should be a dictionary");
        assert_eq!(
            input["keys"],
            XpcValue::Array(vec![
                XpcValue::String("ProductVersion".into()),
                XpcValue::String("MainScreenCanvasSizes".into()),
            ])
        );
    }

    #[test]
    fn coredevice_output_extracts_success_payload() {
        let response = XpcMessage {
            flags: 0,
            msg_id: 1,
            body: Some(XpcValue::Dictionary(IndexMap::from([(
                "CoreDevice.output".to_string(),
                XpcValue::Dictionary(IndexMap::from([(
                    "ProductVersion".to_string(),
                    XpcValue::String("17.4".into()),
                )])),
            )]))),
        };

        let output =
            crate::services::coredevice::parse_output(response).expect("output should parse");
        let dict = output.as_dict().expect("output should be a dictionary");
        assert_eq!(dict["ProductVersion"].as_str(), Some("17.4"));
    }

    #[test]
    fn coredevice_output_reports_error_payload() {
        let response = XpcMessage {
            flags: 0,
            msg_id: 1,
            body: Some(XpcValue::Dictionary(IndexMap::from([(
                "CoreDevice.error".to_string(),
                XpcValue::Dictionary(IndexMap::from([(
                    "localizedDescription".to_string(),
                    XpcValue::String("denied".into()),
                )])),
            )]))),
        };

        let err = crate::services::coredevice::parse_output(response)
            .expect_err("errors should be surfaced");
        assert!(err.to_string().contains("denied"));
    }

    #[test]
    fn xpc_value_to_plist_preserves_mobilegestalt_dictionary_shape() {
        let value = XpcValue::Dictionary(IndexMap::from([
            (
                "ProductVersion".to_string(),
                XpcValue::String("17.4".into()),
            ),
            (
                "MainScreenCanvasSizes".to_string(),
                XpcValue::Array(vec![XpcValue::Dictionary(IndexMap::from([(
                    "Width".to_string(),
                    XpcValue::Uint64(1290),
                )]))]),
            ),
        ]));

        let plist = xpc_value_to_plist(&value);
        let dict = plist
            .as_dictionary()
            .expect("converted mobilegestalt should be a dictionary");
        assert_eq!(
            dict.get("ProductVersion").and_then(plist::Value::as_string),
            Some("17.4")
        );
        assert!(dict
            .get("MainScreenCanvasSizes")
            .and_then(plist::Value::as_array)
            .is_some());
    }
}
