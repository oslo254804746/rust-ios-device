//! iOS 17+ CoreDevice diagnostics service via XPC/RSD.
//!
//! This module currently covers the CoreDevice sysdiagnose feature. The service
//! returns metadata and an XPC file-transfer token; consumers that need the full
//! archive must use the returned transfer information with a compatible data path.

use indexmap::IndexMap;

use crate::xpc::{XpcClient, XpcError, XpcMessage, XpcValue};

/// RSD service name for CoreDevice diagnostics.
pub const SERVICE_NAME: &str = "com.apple.coredevice.diagnosticsservice";

const FEATURE_CAPTURE_SYSDIAGNOSE: &str = "com.apple.coredevice.feature.capturesysdiagnose";

/// Errors returned by CoreDevice diagnostics operations.
#[derive(Debug, thiserror::Error)]
pub enum DiagnosticsServiceError {
    /// Underlying XPC transport or encoding error.
    #[error("xpc error: {0}")]
    Xpc(#[from] XpcError),
    /// Diagnosticsservice response did not match the expected protocol shape.
    #[error("protocol error: {0}")]
    Protocol(String),
}

/// Metadata returned by the CoreDevice sysdiagnose feature.
#[derive(Debug, Clone, PartialEq)]
pub struct SysdiagnoseResponse {
    /// Filename preferred by the device for the sysdiagnose archive.
    pub preferred_filename: String,
    /// Archive size reported by CoreDevice.
    pub file_size: u64,
    /// Raw XPC file-transfer object for consumers that implement the transfer path.
    pub file_transfer: XpcValue,
}

/// Client for the CoreDevice diagnostics service.
pub struct DiagnosticsServiceClient {
    client: XpcClient,
    device_identifier: String,
}

impl DiagnosticsServiceClient {
    /// Create a diagnostics client from an initialized XPC client and device identifier.
    pub fn new(client: XpcClient, device_identifier: impl Into<String>) -> Self {
        Self {
            client,
            device_identifier: device_identifier.into(),
        }
    }

    /// Request a sysdiagnose capture.
    ///
    /// When `dry_run` is true, the device validates and describes the capture without
    /// collecting the full archive.
    pub async fn capture_sysdiagnose(
        &mut self,
        dry_run: bool,
    ) -> Result<SysdiagnoseResponse, DiagnosticsServiceError> {
        let response = self
            .client
            .call(build_request(
                &self.device_identifier,
                build_capture_sysdiagnose_input(dry_run),
            ))
            .await?;
        parse_capture_sysdiagnose_response(response)
    }
}

fn build_capture_sysdiagnose_input(is_dry_run: bool) -> XpcValue {
    XpcValue::Dictionary(IndexMap::from([
        (
            "options".to_string(),
            XpcValue::Dictionary(IndexMap::from([(
                "collectFullLogs".to_string(),
                XpcValue::Bool(true),
            )])),
        ),
        ("isDryRun".to_string(), XpcValue::Bool(is_dry_run)),
    ]))
}

fn build_request(device_identifier: &str, input: XpcValue) -> XpcValue {
    crate::services::coredevice::build_request(
        device_identifier,
        FEATURE_CAPTURE_SYSDIAGNOSE,
        input,
    )
}

fn parse_capture_sysdiagnose_response(
    response: XpcMessage,
) -> Result<SysdiagnoseResponse, DiagnosticsServiceError> {
    let output = crate::services::coredevice::parse_output(response)
        .map_err(DiagnosticsServiceError::Protocol)?;
    let dict = output.as_dict().ok_or_else(|| {
        DiagnosticsServiceError::Protocol(format!(
            "capture sysdiagnose output is not a dictionary: {output:?}"
        ))
    })?;
    let preferred_filename = dict
        .get("preferredFilename")
        .and_then(XpcValue::as_str)
        .ok_or_else(|| {
            DiagnosticsServiceError::Protocol(format!(
                "capture sysdiagnose output missing preferredFilename: {output:?}"
            ))
        })?
        .to_string();
    let file_transfer = dict.get("fileTransfer").cloned().ok_or_else(|| {
        DiagnosticsServiceError::Protocol(format!(
            "capture sysdiagnose output missing fileTransfer: {output:?}"
        ))
    })?;
    let file_size = parse_file_transfer_size(&file_transfer)?;

    Ok(SysdiagnoseResponse {
        preferred_filename,
        file_size,
        file_transfer,
    })
}

fn parse_file_transfer_size(value: &XpcValue) -> Result<u64, DiagnosticsServiceError> {
    if let Some((_, transfer)) = value.as_file_transfer() {
        return transfer
            .as_dict()
            .and_then(|dict| dict.get("s"))
            .and_then(as_u64)
            .ok_or_else(|| {
                DiagnosticsServiceError::Protocol("fileTransfer missing transfer size field".into())
            });
    }

    let dict = value.as_dict().ok_or_else(|| {
        DiagnosticsServiceError::Protocol(format!("unsupported fileTransfer shape: {value:?}"))
    })?;
    if let Some(size) = dict.get("expectedLength").and_then(as_u64) {
        return Ok(size);
    }
    dict.get("xpcFileTransfer")
        .ok_or_else(|| {
            DiagnosticsServiceError::Protocol(format!(
                "fileTransfer missing expectedLength/xpcFileTransfer: {value:?}"
            ))
        })
        .and_then(parse_file_transfer_size)
}

fn as_u64(value: &XpcValue) -> Option<u64> {
    match value {
        XpcValue::Uint64(value) => Some(*value),
        XpcValue::Int64(value) if *value >= 0 => Some(*value as u64),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use indexmap::IndexMap;

    use crate::xpc::{XpcMessage, XpcValue};

    use super::*;

    #[test]
    fn build_capture_sysdiagnose_input_matches_reference_shape() {
        let input = build_capture_sysdiagnose_input(true);
        let dict = input.as_dict().expect("input should be a dictionary");

        assert_eq!(dict["isDryRun"], XpcValue::Bool(true));
        let options = dict["options"].as_dict().expect("options should be a dict");
        assert_eq!(options["collectFullLogs"], XpcValue::Bool(true));
    }

    #[test]
    fn build_request_wraps_capture_sysdiagnose_feature() {
        let request = build_request("DEVICE-ID", build_capture_sysdiagnose_input(true));
        let dict = request.as_dict().expect("request should be a dictionary");

        assert_eq!(
            dict["CoreDevice.featureIdentifier"].as_str(),
            Some(FEATURE_CAPTURE_SYSDIAGNOSE)
        );
        assert_eq!(
            dict["CoreDevice.deviceIdentifier"].as_str(),
            Some("DEVICE-ID")
        );
    }

    #[test]
    fn parse_capture_sysdiagnose_response_reads_metadata() {
        let response = XpcMessage {
            flags: 0,
            msg_id: 1,
            body: Some(XpcValue::Dictionary(IndexMap::from([(
                "CoreDevice.output".to_string(),
                XpcValue::Dictionary(IndexMap::from([
                    (
                        "preferredFilename".to_string(),
                        XpcValue::String("sysdiagnose_2026.tar.gz".into()),
                    ),
                    (
                        "fileTransfer".to_string(),
                        XpcValue::Dictionary(IndexMap::from([(
                            "expectedLength".to_string(),
                            XpcValue::Uint64(4096),
                        )])),
                    ),
                ])),
            )]))),
        };

        let parsed = parse_capture_sysdiagnose_response(response).unwrap();

        assert_eq!(parsed.preferred_filename, "sysdiagnose_2026.tar.gz");
        assert_eq!(parsed.file_size, 4096);
    }

    #[test]
    fn parse_capture_sysdiagnose_response_accepts_nested_xpc_file_transfer() {
        let response = XpcMessage {
            flags: 0,
            msg_id: 1,
            body: Some(XpcValue::Dictionary(IndexMap::from([(
                "CoreDevice.output".to_string(),
                XpcValue::Dictionary(IndexMap::from([
                    (
                        "preferredFilename".to_string(),
                        XpcValue::String("sysdiagnose.tar.gz".into()),
                    ),
                    (
                        "fileTransfer".to_string(),
                        XpcValue::Dictionary(IndexMap::from([(
                            "xpcFileTransfer".to_string(),
                            XpcValue::FileTransfer {
                                msg_id: 7,
                                data: Box::new(XpcValue::Dictionary(IndexMap::from([(
                                    "s".to_string(),
                                    XpcValue::Int64(8192),
                                )]))),
                            },
                        )])),
                    ),
                ])),
            )]))),
        };

        let parsed = parse_capture_sysdiagnose_response(response).unwrap();

        assert_eq!(parsed.file_size, 8192);
    }
}
