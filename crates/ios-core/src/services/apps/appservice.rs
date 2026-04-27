//! iOS 17+ CoreDevice appservice helpers for running processes and app lifecycle.

use crate::xpc::{XpcClient, XpcError, XpcMessage, XpcValue};
use bytes::Bytes;
use indexmap::IndexMap;

const COREDEVICE_PROTOCOL_VERSION: i64 = 0;
const COREDEVICE_VERSION: &str = "325.3";
const FEATURE_LIST_PROCESSES: &str = "com.apple.coredevice.feature.listprocesses";
const FEATURE_LAUNCH_APPLICATION: &str = "com.apple.coredevice.feature.launchapplication";
const FEATURE_SEND_SIGNAL: &str = "com.apple.coredevice.feature.sendsignaltoprocess";
const SIGKILL: i64 = 9;

#[derive(Debug, thiserror::Error)]
pub enum AppServiceError {
    #[error("xpc error: {0}")]
    Xpc(#[from] XpcError),
    #[error("protocol error: {0}")]
    Protocol(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RunningAppProcess {
    pub pid: u64,
    pub bundle_id: Option<String>,
    pub name: String,
    pub executable: Option<String>,
    pub is_application: Option<bool>,
}

pub struct AppServiceClient {
    client: XpcClient,
    device_identifier: String,
}

impl AppServiceClient {
    pub fn new(client: XpcClient, _device_identifier: impl Into<String>) -> Self {
        Self {
            client,
            device_identifier: invocation_identifier(),
        }
    }

    pub async fn list_processes(&mut self) -> Result<Vec<RunningAppProcess>, AppServiceError> {
        let response = self
            .client
            .call(build_request(
                &self.device_identifier,
                FEATURE_LIST_PROCESSES,
                XpcValue::Dictionary(IndexMap::new()),
            ))
            .await?;
        parse_processes(&response)
    }

    pub async fn kill_process(&mut self, pid: u64) -> Result<(), AppServiceError> {
        self.send_signal(pid, SIGKILL).await
    }

    pub async fn send_signal(&mut self, pid: u64, signal: i64) -> Result<(), AppServiceError> {
        let response = self
            .client
            .call(build_request(
                &self.device_identifier,
                FEATURE_SEND_SIGNAL,
                build_send_signal_input(pid, signal),
            ))
            .await?;
        ensure_no_error(&response)?;
        Ok(())
    }

    pub async fn launch_application(
        &mut self,
        bundle_id: &str,
    ) -> Result<Option<u64>, AppServiceError> {
        let response = self
            .client
            .call(build_request(
                &self.device_identifier,
                FEATURE_LAUNCH_APPLICATION,
                build_launch_application_input(bundle_id)?,
            ))
            .await?;
        ensure_no_error(&response)?;
        Ok(parse_pid(response.body.as_ref()))
    }
}

fn build_send_signal_input(pid: u64, signal: i64) -> XpcValue {
    XpcValue::Dictionary(IndexMap::from([
        (
            "process".to_string(),
            XpcValue::Dictionary(IndexMap::from([(
                "processIdentifier".to_string(),
                XpcValue::Int64(pid as i64),
            )])),
        ),
        ("signal".to_string(), XpcValue::Int64(signal)),
    ]))
}

fn build_launch_application_input(bundle_id: &str) -> Result<XpcValue, AppServiceError> {
    let mut platform_specific_options = Vec::new();
    plist::to_writer_binary(
        &mut platform_specific_options,
        &plist::Value::Dictionary(plist::Dictionary::new()),
    )
    .map_err(|error| {
        AppServiceError::Protocol(format!("failed to encode platformSpecificOptions: {error}"))
    })?;

    Ok(XpcValue::Dictionary(IndexMap::from([
        (
            "applicationSpecifier".to_string(),
            XpcValue::Dictionary(IndexMap::from([(
                "bundleIdentifier".to_string(),
                XpcValue::Dictionary(IndexMap::from([(
                    "_0".to_string(),
                    XpcValue::String(bundle_id.to_string()),
                )])),
            )])),
        ),
        (
            "options".to_string(),
            XpcValue::Dictionary(IndexMap::from([
                ("arguments".to_string(), XpcValue::Array(Vec::new())),
                (
                    "environmentVariables".to_string(),
                    XpcValue::Dictionary(IndexMap::new()),
                ),
                (
                    "standardIOUsesPseudoterminals".to_string(),
                    XpcValue::Bool(true),
                ),
                ("startStopped".to_string(), XpcValue::Bool(false)),
                ("terminateExisting".to_string(), XpcValue::Bool(false)),
                (
                    "user".to_string(),
                    XpcValue::Dictionary(IndexMap::from([
                        ("active".to_string(), XpcValue::Bool(true)),
                        (
                            "shortName".to_string(),
                            XpcValue::String("mobile".to_string()),
                        ),
                    ])),
                ),
                (
                    "platformSpecificOptions".to_string(),
                    XpcValue::Data(Bytes::from(platform_specific_options)),
                ),
            ])),
        ),
        (
            "standardIOIdentifiers".to_string(),
            XpcValue::Dictionary(IndexMap::new()),
        ),
    ])))
}

fn build_request(device_identifier: &str, feature_identifier: &str, input: XpcValue) -> XpcValue {
    let mut coredevice_version = IndexMap::new();
    coredevice_version.insert(
        "components".to_string(),
        XpcValue::Array(vec![
            XpcValue::Uint64(325),
            XpcValue::Uint64(3),
            XpcValue::Uint64(0),
            XpcValue::Uint64(0),
            XpcValue::Uint64(0),
        ]),
    );
    coredevice_version.insert("originalComponentsCount".to_string(), XpcValue::Int64(2));
    coredevice_version.insert(
        "stringValue".to_string(),
        XpcValue::String(COREDEVICE_VERSION.to_string()),
    );

    let mut body = IndexMap::new();
    body.insert(
        "CoreDevice.CoreDeviceDDIProtocolVersion".to_string(),
        XpcValue::Int64(COREDEVICE_PROTOCOL_VERSION),
    );
    body.insert(
        "CoreDevice.action".to_string(),
        XpcValue::Dictionary(IndexMap::new()),
    );
    body.insert(
        "CoreDevice.coreDeviceVersion".to_string(),
        XpcValue::Dictionary(coredevice_version),
    );
    body.insert(
        "CoreDevice.deviceIdentifier".to_string(),
        XpcValue::String(device_identifier.to_string()),
    );
    body.insert(
        "CoreDevice.featureIdentifier".to_string(),
        XpcValue::String(feature_identifier.to_string()),
    );
    body.insert("CoreDevice.input".to_string(), input);
    body.insert(
        "CoreDevice.invocationIdentifier".to_string(),
        XpcValue::String(invocation_identifier()),
    );
    XpcValue::Dictionary(body)
}

fn invocation_identifier() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};

    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let raw = format!("{nanos:032x}");
    format!(
        "{}-{}-{}-{}-{}",
        &raw[0..8],
        &raw[8..12],
        &raw[12..16],
        &raw[16..20],
        &raw[20..32]
    )
}

fn parse_processes(response: &XpcMessage) -> Result<Vec<RunningAppProcess>, AppServiceError> {
    ensure_no_error(response)?;
    let body = response
        .body
        .as_ref()
        .ok_or_else(|| AppServiceError::Protocol("missing response body".into()))?;
    let payload = coredevice_output(body).unwrap_or(body);

    let items = process_items(payload).ok_or_else(|| {
        AppServiceError::Protocol(format!("unexpected process list payload: {payload:?}"))
    })?;

    Ok(items.iter().filter_map(parse_process).collect())
}

fn coredevice_output(value: &XpcValue) -> Option<&XpcValue> {
    value.as_dict()?.get("CoreDevice.output")
}

fn process_items(value: &XpcValue) -> Option<&[XpcValue]> {
    match value {
        XpcValue::Array(items) => Some(items.as_slice()),
        XpcValue::Dictionary(dict) => {
            for key in ["processTokens", "processes", "items"] {
                if let Some(XpcValue::Array(items)) = dict.get(key) {
                    return Some(items.as_slice());
                }
            }
            None
        }
        _ => None,
    }
}

fn parse_process(value: &XpcValue) -> Option<RunningAppProcess> {
    let dict = value.as_dict()?;
    let pid = dict
        .get("processIdentifier")
        .and_then(as_u64)
        .or_else(|| dict.get("pid").and_then(as_u64))?;
    let name = string_field(
        dict,
        &[
            "localizedName",
            "name",
            "executableDisplayName",
            "bundleIdentifier",
        ],
    )?;
    let bundle_id = string_field(dict, &["bundleIdentifier", "bundleIdentifierKey"]);
    let executable = string_field(dict, &["executableName", "name"]);
    let is_application = dict.get("isApplication").and_then(as_bool);

    Some(RunningAppProcess {
        pid,
        bundle_id,
        name,
        executable,
        is_application,
    })
}

fn ensure_no_error(response: &XpcMessage) -> Result<(), AppServiceError> {
    if let Some(body) = response.body.as_ref() {
        if let Some(message) = error_message(body) {
            return Err(AppServiceError::Protocol(message));
        }
    }
    Ok(())
}

fn error_message(value: &XpcValue) -> Option<String> {
    let dict = value.as_dict()?;
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
        XpcValue::String(s) => Some(s.clone()),
        XpcValue::Dictionary(dict) => {
            for key in [
                "message",
                "localizedDescription",
                "NSLocalizedDescription",
                "description",
            ] {
                if let Some(XpcValue::String(s)) = dict.get(key) {
                    return Some(s.clone());
                }
            }
            None
        }
        _ => None,
    }
}

fn parse_pid(value: Option<&XpcValue>) -> Option<u64> {
    match value {
        Some(XpcValue::Uint64(pid)) => Some(*pid),
        Some(XpcValue::Int64(pid)) if *pid >= 0 => Some(*pid as u64),
        Some(XpcValue::Dictionary(dict)) => {
            for key in ["processIdentifier", "pid"] {
                if let Some(pid) = dict.get(key).and_then(as_u64) {
                    return Some(pid);
                }
            }
            for key in [
                "CoreDevice.output",
                "processToken",
                "process",
                "launchedProcess",
            ] {
                if let Some(pid) = parse_pid(dict.get(key)) {
                    return Some(pid);
                }
            }
            None
        }
        _ => None,
    }
}

fn string_field(dict: &IndexMap<String, XpcValue>, keys: &[&str]) -> Option<String> {
    keys.iter().find_map(|key| {
        dict.get(*key)
            .and_then(|v| v.as_str())
            .map(ToOwned::to_owned)
    })
}

fn as_u64(value: &XpcValue) -> Option<u64> {
    match value {
        XpcValue::Uint64(n) => Some(*n),
        XpcValue::Int64(n) if *n >= 0 => Some(*n as u64),
        _ => None,
    }
}

fn as_bool(value: &XpcValue) -> Option<bool> {
    match value {
        XpcValue::Bool(v) => Some(*v),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn build_request_wraps_coredevice_envelope() {
        let request = build_request(
            "DEVICE-ID",
            FEATURE_SEND_SIGNAL,
            XpcValue::Dictionary(IndexMap::from([
                ("processIdentifier".to_string(), XpcValue::Uint64(42)),
                ("signal".to_string(), XpcValue::Int64(SIGKILL)),
            ])),
        );

        let dict = request.as_dict().unwrap();
        assert_eq!(
            dict["CoreDevice.featureIdentifier"].as_str(),
            Some(FEATURE_SEND_SIGNAL)
        );
        assert_eq!(
            dict["CoreDevice.deviceIdentifier"].as_str(),
            Some("DEVICE-ID")
        );
        assert!(dict["CoreDevice.invocationIdentifier"]
            .as_str()
            .unwrap()
            .contains('-'));
    }

    #[test]
    fn build_send_signal_input_nests_process_identifier() {
        let input = build_send_signal_input(42, SIGKILL);
        let dict = input.as_dict().unwrap();
        let process = dict["process"].as_dict().unwrap();

        assert_eq!(process["processIdentifier"], XpcValue::Int64(42));
        assert_eq!(dict["signal"], XpcValue::Int64(SIGKILL));
    }

    #[test]
    fn build_launch_application_input_matches_reference_shape() {
        let input = build_launch_application_input("com.example.App").unwrap();
        let dict = input.as_dict().unwrap();
        let application_specifier = dict["applicationSpecifier"].as_dict().unwrap();
        let bundle_identifier = application_specifier["bundleIdentifier"].as_dict().unwrap();
        let options = dict["options"].as_dict().unwrap();
        let user = options["user"].as_dict().unwrap();

        assert_eq!(bundle_identifier["_0"].as_str(), Some("com.example.App"));
        assert_eq!(options["arguments"], XpcValue::Array(Vec::new()));
        assert_eq!(
            options["environmentVariables"],
            XpcValue::Dictionary(IndexMap::new())
        );
        assert_eq!(
            options["standardIOUsesPseudoterminals"],
            XpcValue::Bool(true)
        );
        assert_eq!(options["startStopped"], XpcValue::Bool(false));
        assert_eq!(options["terminateExisting"], XpcValue::Bool(false));
        assert_eq!(user["active"], XpcValue::Bool(true));
        assert_eq!(user["shortName"].as_str(), Some("mobile"));
        assert_eq!(
            dict["standardIOIdentifiers"],
            XpcValue::Dictionary(IndexMap::new())
        );

        let XpcValue::Data(platform_specific_options) = &options["platformSpecificOptions"] else {
            panic!("platformSpecificOptions should be XPC data");
        };
        let decoded: plist::Value =
            plist::from_bytes(platform_specific_options).expect("binary plist decode");
        assert_eq!(decoded, plist::Value::Dictionary(plist::Dictionary::new()));
    }

    #[test]
    fn parse_processes_reads_coredevice_output_envelope() {
        let process = XpcValue::Dictionary(IndexMap::from([
            ("processIdentifier".to_string(), XpcValue::Uint64(99)),
            (
                "bundleIdentifier".to_string(),
                XpcValue::String("com.example.App".into()),
            ),
            (
                "localizedName".to_string(),
                XpcValue::String("Example".into()),
            ),
            (
                "executableName".to_string(),
                XpcValue::String("ExampleBin".into()),
            ),
            ("isApplication".to_string(), XpcValue::Bool(true)),
        ]));
        let response = XpcMessage {
            flags: 0,
            msg_id: 1,
            body: Some(XpcValue::Dictionary(IndexMap::from([(
                "CoreDevice.output".to_string(),
                XpcValue::Dictionary(IndexMap::from([(
                    "processTokens".to_string(),
                    XpcValue::Array(vec![process]),
                )])),
            )]))),
        };

        let parsed = parse_processes(&response).unwrap();
        assert_eq!(parsed.len(), 1);
        assert_eq!(parsed[0].pid, 99);
        assert_eq!(parsed[0].bundle_id.as_deref(), Some("com.example.App"));
    }

    #[test]
    fn ensure_no_error_reads_coredevice_error_envelope() {
        let response = XpcMessage {
            flags: 0,
            msg_id: 1,
            body: Some(XpcValue::Dictionary(IndexMap::from([(
                "CoreDevice.error".to_string(),
                XpcValue::Dictionary(IndexMap::from([(
                    "localizedDescription".to_string(),
                    XpcValue::String("boom".into()),
                )])),
            )]))),
        };

        let err = ensure_no_error(&response).unwrap_err();
        assert!(matches!(err, AppServiceError::Protocol(message) if message == "boom"));
    }

    #[test]
    fn parse_pid_accepts_coredevice_output_process_token() {
        let pid = parse_pid(Some(&XpcValue::Dictionary(IndexMap::from([(
            "CoreDevice.output".to_string(),
            XpcValue::Dictionary(IndexMap::from([(
                "processToken".to_string(),
                XpcValue::Dictionary(IndexMap::from([(
                    "processIdentifier".to_string(),
                    XpcValue::Uint64(31337),
                )])),
            )])),
        )]))));

        assert_eq!(pid, Some(31337));
    }
}
