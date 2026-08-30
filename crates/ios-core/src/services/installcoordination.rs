//! iOS 17+ InstallCoordinationProxy service.
//!
//! `com.apple.remote.installcoordination_proxy` is a RemoteXPC service exposed
//! through the RSD tunnel.  The pinned pymobiledevice3 implementation only
//! implements the read-only `Query` request; install/uninstall/stash requests
//! use an out-of-band file-transfer protocol and are deliberately not
//! invented here.

use std::time::Duration;

use futures_util::StreamExt;
use indexmap::IndexMap;

use crate::xpc::{XpcClient, XpcError, XpcMessage, XpcValue};

/// Canonical RSD service name.  The upstream service has no classic lockdown
/// or `.shim.remote` route.
pub const SERVICE_NAME: &str = "com.apple.remote.installcoordination_proxy";

/// Request and protocol version gates required by the daemon.
pub const REQUEST_VERSION: u64 = 1;
pub const PROTOCOL_VERSION: u64 = 1;

/// Request type constants defined by the daemon protocol.  Only query is
/// implemented here; the other operations require an out-of-band payload
/// transfer that the pinned upstream client also leaves unimplemented.
pub const REQUEST_TYPE_INSTALL: u64 = 1;
/// Request type reserved for reverting an installation stash.
pub const REQUEST_TYPE_REVERT_STASH: u64 = 2;
/// Request type reserved for uninstalling an application.
pub const REQUEST_TYPE_UNINSTALL: u64 = 3;
/// Read-only install record query request type.
pub const REQUEST_TYPE_QUERY: u64 = 4;

/// CoreFoundation marker used to encode an NSURL in XPC.
pub const CFURL_MAGIC: uuid::Uuid = uuid::Uuid::from_u128(0xc3853dcc97764114b6c1fd9f51944a6d);

const DEFAULT_TIMEOUT: Duration = Duration::from_secs(30);
const MAX_BUNDLE_ID_BYTES: usize = 1024;
const MAX_DB_UUID_BYTES: usize = 256;
const MAX_INSTALL_PATH_BYTES: usize = 16 * 1024;
const MAX_PERSISTENT_IDENTIFIER_BYTES: usize = 4096;
const MAX_ERROR_DATA_BYTES: usize = 1024 * 1024;
const MAX_ERROR_MESSAGE_BYTES: usize = 4096;
const MAX_RESPONSE_DEPTH: usize = 32;
const MAX_RESPONSE_ITEMS: usize = 4096;
const MAX_RESPONSE_VALUE_BYTES: usize = 4 * 1024 * 1024;

/// Install-coordination protocol errors.
#[derive(Debug, thiserror::Error)]
pub enum InstallCoordinationError {
    /// Underlying RemoteXPC transport failure.
    #[error("install coordination XPC error: {0}")]
    Xpc(#[from] XpcError),
    /// The daemon returned an invalid or unsupported response.
    #[error("install coordination protocol error: {0}")]
    Protocol(String),
    /// The request exceeded its configured deadline.
    #[error("install coordination request timed out after {0:?}")]
    Timeout(Duration),
}

/// The LaunchServices install record returned by `Query`.
#[derive(Debug, Clone, PartialEq)]
pub struct InstallRecord {
    pub db_uuid: String,
    pub db_sequence: u64,
    pub install_path: Option<String>,
    pub persistent_identifier: Vec<u8>,
    /// Future response fields, retained without changing the known record
    /// interpretation.  Query currently returns a small flat dictionary.
    pub extra: IndexMap<String, XpcValue>,
}

/// Encode a CoreFoundation NSURL exactly as the reference client does.
pub fn encode_url(url: &str) -> XpcValue {
    XpcValue::Dictionary(IndexMap::from([
        (
            "com.apple.CFURL.magic".to_string(),
            XpcValue::Uuid(*CFURL_MAGIC.as_bytes()),
        ),
        ("com.apple.CFURL.base".to_string(), XpcValue::Null),
        (
            "com.apple.CFURL.string".to_string(),
            XpcValue::String(url.to_string()),
        ),
    ]))
}

/// Decode the string component of a CoreFoundation NSURL, if present.
pub fn decode_url(value: &XpcValue) -> Option<String> {
    value
        .as_dict()?
        .get("com.apple.CFURL.string")
        .and_then(XpcValue::as_str)
        .map(ToOwned::to_owned)
}

/// Client for `com.apple.remote.installcoordination_proxy`.
pub struct InstallCoordinationProxyClient {
    client: XpcClient,
    timeout: Duration,
}

/// Compatibility alias matching the upstream service class name.
pub type InstallCoordinationProxy = InstallCoordinationProxyClient;
/// Explicit service suffix alias for callers mirroring pymobiledevice3 names.
pub type InstallCoordinationProxyService = InstallCoordinationProxyClient;

impl InstallCoordinationProxyClient {
    /// Construct a client with the standard request deadline.
    pub fn new(client: XpcClient) -> Self {
        Self::with_timeout(client, DEFAULT_TIMEOUT)
    }

    /// Construct a client with a bounded request deadline.
    pub fn with_timeout(client: XpcClient, timeout: Duration) -> Self {
        Self { client, timeout }
    }

    pub fn timeout(&self) -> Duration {
        self.timeout
    }

    /// Connect through the canonical RSD entry selected for this request.
    ///
    /// `ConnectedDevice` rejects `.shim.remote` entries for XPC services, and
    /// this extra equality check makes the RSD-only contract explicit here.
    pub async fn connect(
        device: &crate::ConnectedDevice,
    ) -> Result<Self, InstallCoordinationError> {
        // `connect_xpc_service_with_metadata` includes the H2/XPC bootstrap,
        // whose read side can otherwise wait forever on a peer that accepts a
        // TCP connection but never speaks RemoteXPC.  Keep the library entry
        // point bounded just like the query itself; callers that need a
        // shorter total deadline can still wrap this future externally.
        let (client, metadata) = tokio::time::timeout(
            DEFAULT_TIMEOUT,
            device.connect_xpc_service_with_metadata(SERVICE_NAME),
        )
        .await
        .map_err(|_| InstallCoordinationError::Timeout(DEFAULT_TIMEOUT))?
        .map_err(|error| {
            InstallCoordinationError::Protocol(format!(
                "failed to connect to {SERVICE_NAME}: {error}"
            ))
        })?;
        if metadata.resolved_service_name != SERVICE_NAME {
            return Err(InstallCoordinationError::Protocol(format!(
                "resolved unsupported InstallCoordinationProxy service {}",
                metadata.resolved_service_name
            )));
        }
        Ok(Self::new(client))
    }

    /// Query one bundle's install record.
    pub async fn query(
        &mut self,
        bundle_identifier: &str,
    ) -> Result<InstallRecord, InstallCoordinationError> {
        validate_bundle_identifier(bundle_identifier)?;
        let request = build_query_request(bundle_identifier);
        // The reference service sends the reply as the next fresh message
        // rather than correlating it with the request id.  `call` deliberately
        // waits for a matching id, so use the shared stream primitive with the
        // same no-WANTING_REPLY flags as pymobiledevice3.
        let responses = self.client.stream_invoke_with_flags(request, 0);
        tokio::pin!(responses);
        let deadline = tokio::time::Instant::now()
            .checked_add(self.timeout)
            .ok_or_else(|| {
                InstallCoordinationError::Protocol(format!(
                    "query timeout {:#?} exceeds the representable instant range",
                    self.timeout
                ))
            })?;
        loop {
            let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
            if remaining.is_zero() {
                return Err(InstallCoordinationError::Timeout(self.timeout));
            }
            let response = tokio::time::timeout(remaining, responses.next())
                .await
                .map_err(|_| InstallCoordinationError::Timeout(self.timeout))?;
            let response = response.ok_or_else(|| {
                InstallCoordinationError::Protocol(
                    "InstallCoordinationProxy response stream ended before a response".into(),
                )
            })??;
            // RemoteXPC peers may emit a zero-length data frame after the
            // request. pmd3's receive_response skips those frames; do the
            // same before parsing the first actual dictionary.
            if response.body.is_none() {
                continue;
            }
            return parse_query_response(response, bundle_identifier);
        }
    }
}

fn validate_bundle_identifier(bundle_identifier: &str) -> Result<(), InstallCoordinationError> {
    if bundle_identifier.is_empty() {
        return Err(InstallCoordinationError::Protocol(
            "bundle identifier must not be empty".into(),
        ));
    }
    if bundle_identifier.len() > MAX_BUNDLE_ID_BYTES {
        return Err(InstallCoordinationError::Protocol(format!(
            "bundle identifier length {} exceeds maximum {MAX_BUNDLE_ID_BYTES}",
            bundle_identifier.len()
        )));
    }
    if bundle_identifier.chars().any(char::is_control) {
        return Err(InstallCoordinationError::Protocol(
            "bundle identifier must not contain control characters".into(),
        ));
    }
    Ok(())
}

fn build_query_request(bundle_identifier: &str) -> XpcValue {
    XpcValue::Dictionary(IndexMap::from([
        (
            "RequestVersion".to_string(),
            XpcValue::Uint64(REQUEST_VERSION),
        ),
        (
            "ProtocolVersion".to_string(),
            XpcValue::Uint64(PROTOCOL_VERSION),
        ),
        (
            "RequestType".to_string(),
            XpcValue::Uint64(REQUEST_TYPE_QUERY),
        ),
        (
            "BundleID".to_string(),
            XpcValue::String(bundle_identifier.to_string()),
        ),
    ]))
}

fn parse_query_response(
    response: XpcMessage,
    bundle_identifier: &str,
) -> Result<InstallRecord, InstallCoordinationError> {
    let body = response.body.ok_or_else(|| {
        InstallCoordinationError::Protocol("query response is missing a body".into())
    })?;
    let body = body.as_dict().ok_or_else(|| {
        InstallCoordinationError::Protocol("query response body is not a dictionary".into())
    })?;
    let mut response_budget = 0;
    validate_response_dict(body, 0, &mut response_budget)?;
    let success = body.get("Success").and_then(as_bool).ok_or_else(|| {
        InstallCoordinationError::Protocol("query response is missing boolean Success".into())
    })?;
    if !success {
        let detail = body
            .get("ErrorData")
            .and_then(|value| match value {
                XpcValue::Data(data) => Some(describe_error_data(data)),
                _ => None,
            })
            .unwrap_or_else(|| "no error detail".into());
        return Err(InstallCoordinationError::Protocol(format!(
            "query failed for {bundle_identifier}: {detail}"
        )));
    }

    let db_uuid = bounded_string(body, "DBUUID", MAX_DB_UUID_BYTES)?;
    let db_sequence = body
        .get("DBSequence")
        .and_then(XpcValue::as_uint64)
        .ok_or_else(|| {
            InstallCoordinationError::Protocol("query response DBSequence is not uint64".into())
        })?;
    let install_path = match body.get("InstallPath") {
        None | Some(XpcValue::Null) => None,
        Some(value) => {
            let path = decode_url(value).ok_or_else(|| {
                InstallCoordinationError::Protocol(
                    "query response InstallPath is not a CoreFoundation URL".into(),
                )
            })?;
            if path.len() > MAX_INSTALL_PATH_BYTES {
                return Err(InstallCoordinationError::Protocol(format!(
                    "install path length {} exceeds maximum {MAX_INSTALL_PATH_BYTES}",
                    path.len()
                )));
            }
            Some(path)
        }
    };
    let persistent_identifier = match body.get("PersistentIdentifier") {
        Some(XpcValue::Data(data)) => {
            if data.len() > MAX_PERSISTENT_IDENTIFIER_BYTES {
                return Err(InstallCoordinationError::Protocol(format!(
                    "persistent identifier length {} exceeds maximum {MAX_PERSISTENT_IDENTIFIER_BYTES}",
                    data.len()
                )));
            }
            data.to_vec()
        }
        Some(_) => {
            return Err(InstallCoordinationError::Protocol(
                "query response PersistentIdentifier is not data".into(),
            ))
        }
        None => {
            return Err(InstallCoordinationError::Protocol(
                "query response is missing PersistentIdentifier".into(),
            ))
        }
    };

    let mut extra = IndexMap::new();
    for (key, value) in body {
        if !matches!(
            key.as_str(),
            "Success" | "DBUUID" | "DBSequence" | "InstallPath" | "PersistentIdentifier"
        ) {
            extra.insert(key.clone(), value.clone());
        }
    }

    Ok(InstallRecord {
        db_uuid,
        db_sequence,
        install_path,
        persistent_identifier,
        extra,
    })
}

fn validate_response_dict(
    values: &IndexMap<String, XpcValue>,
    depth: usize,
    budget: &mut usize,
) -> Result<(), InstallCoordinationError> {
    if values.len() > MAX_RESPONSE_ITEMS {
        return Err(InstallCoordinationError::Protocol(format!(
            "query response dictionary has {} items, maximum is {MAX_RESPONSE_ITEMS}",
            values.len()
        )));
    }
    for (key, value) in values {
        add_response_bytes(budget, key.len())?;
        validate_response_value(value, depth + 1, budget)?;
    }
    Ok(())
}

fn validate_response_value(
    value: &XpcValue,
    depth: usize,
    budget: &mut usize,
) -> Result<(), InstallCoordinationError> {
    if depth > MAX_RESPONSE_DEPTH {
        return Err(InstallCoordinationError::Protocol(format!(
            "query response nesting exceeds maximum {MAX_RESPONSE_DEPTH}"
        )));
    }
    match value {
        XpcValue::String(value) => add_response_bytes(budget, value.len())?,
        XpcValue::Data(value) => add_response_bytes(budget, value.len())?,
        XpcValue::Array(values) => {
            if values.len() > MAX_RESPONSE_ITEMS {
                return Err(InstallCoordinationError::Protocol(format!(
                    "query response array has {} items, maximum is {MAX_RESPONSE_ITEMS}",
                    values.len()
                )));
            }
            for value in values {
                validate_response_value(value, depth + 1, budget)?;
            }
        }
        XpcValue::Dictionary(values) => validate_response_dict(values, depth, budget)?,
        XpcValue::FileTransfer { data, .. } => {
            validate_response_value(data, depth + 1, budget)?;
        }
        _ => {}
    }
    Ok(())
}

fn add_response_bytes(
    budget: &mut usize,
    additional: usize,
) -> Result<(), InstallCoordinationError> {
    *budget = budget
        .checked_add(additional)
        .ok_or_else(|| InstallCoordinationError::Protocol("query response size overflow".into()))?;
    if *budget > MAX_RESPONSE_VALUE_BYTES {
        return Err(InstallCoordinationError::Protocol(format!(
            "query response value budget {budget} exceeds maximum {MAX_RESPONSE_VALUE_BYTES}"
        )));
    }
    Ok(())
}

fn bounded_string(
    body: &IndexMap<String, XpcValue>,
    key: &str,
    limit: usize,
) -> Result<String, InstallCoordinationError> {
    let value = body.get(key).and_then(XpcValue::as_str).ok_or_else(|| {
        InstallCoordinationError::Protocol(format!("query response {key} is not a string"))
    })?;
    if value.len() > limit {
        return Err(InstallCoordinationError::Protocol(format!(
            "query response {key} length {} exceeds maximum {limit}",
            value.len()
        )));
    }
    Ok(value.to_string())
}

fn as_bool(value: &XpcValue) -> Option<bool> {
    match value {
        XpcValue::Bool(value) => Some(*value),
        _ => None,
    }
}

fn describe_error_data(error_data: &[u8]) -> String {
    if error_data.len() > MAX_ERROR_DATA_BYTES {
        return format!(
            "<undecodable NSError, {} bytes exceeds maximum {MAX_ERROR_DATA_BYTES}>",
            error_data.len()
        );
    }
    let Ok(plist::Value::Dictionary(archive)) = plist::from_bytes(error_data) else {
        return format!("<undecodable NSError, {} bytes>", error_data.len());
    };
    let Some(plist::Value::Array(objects)) = archive.get("$objects") else {
        return format!("<undecodable NSError, {} bytes>", error_data.len());
    };
    let Some(longest) = objects
        .iter()
        .filter_map(plist::Value::as_string)
        .filter(|value| value.contains(' '))
        .max_by_key(|value| value.len())
    else {
        return format!("<undecodable NSError, {} bytes>", error_data.len());
    };
    truncate_message(longest)
}

fn truncate_message(message: &str) -> String {
    if message.len() <= MAX_ERROR_MESSAGE_BYTES {
        return message.to_string();
    }
    let mut end = MAX_ERROR_MESSAGE_BYTES.saturating_sub(3);
    while end > 0 && !message.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}...", &message[..end])
}

#[cfg(test)]
mod tests {
    use bytes::Bytes;

    use super::*;

    fn message(body: XpcValue) -> XpcMessage {
        XpcMessage {
            flags: 0,
            msg_id: 1,
            body: Some(body),
        }
    }

    fn successful_response() -> XpcMessage {
        message(XpcValue::Dictionary(IndexMap::from([
            ("Success".into(), XpcValue::Bool(true)),
            (
                "DBUUID".into(),
                XpcValue::String("D1D20BD4-9669-47A6-B577-F6D62ED45B43".into()),
            ),
            ("DBSequence".into(), XpcValue::Uint64(276)),
            (
                "InstallPath".into(),
                encode_url("file:///Applications/Preferences.app/"),
            ),
            (
                "PersistentIdentifier".into(),
                XpcValue::Data(Bytes::from_static(b"\0\x01")),
            ),
            ("FutureField".into(), XpcValue::String("kept".into())),
        ])))
    }

    #[test]
    fn query_request_contains_both_version_gates_and_exact_wire_keys() {
        let request = build_query_request("com.apple.Preferences");
        let request = request.as_dict().unwrap();
        assert_eq!(request["RequestVersion"], XpcValue::Uint64(1));
        assert_eq!(request["ProtocolVersion"], XpcValue::Uint64(1));
        assert_eq!(request["RequestType"], XpcValue::Uint64(4));
        assert_eq!(request["BundleID"].as_str(), Some("com.apple.Preferences"));
    }

    #[test]
    fn parses_install_record_and_preserves_unknown_fields() {
        let record = parse_query_response(successful_response(), "com.apple.Preferences").unwrap();
        assert_eq!(record.db_uuid, "D1D20BD4-9669-47A6-B577-F6D62ED45B43");
        assert_eq!(record.db_sequence, 276);
        assert_eq!(
            record.install_path.as_deref(),
            Some("file:///Applications/Preferences.app/")
        );
        assert_eq!(record.persistent_identifier, b"\0\x01");
        assert_eq!(record.extra["FutureField"].as_str(), Some("kept"));
    }

    #[test]
    fn url_round_trip_and_non_url_decode_match_reference() {
        let encoded = encode_url("file:///Applications/Preferences.app/");
        assert_eq!(
            encoded.as_dict().unwrap()["com.apple.CFURL.magic"],
            XpcValue::Uuid(*CFURL_MAGIC.as_bytes())
        );
        assert_eq!(
            decode_url(&encoded).as_deref(),
            Some("file:///Applications/Preferences.app/")
        );
        assert!(decode_url(&XpcValue::String("not a URL".into())).is_none());
    }

    #[test]
    fn malformed_success_and_error_responses_are_bounded() {
        let missing = message(XpcValue::Dictionary(IndexMap::from([(
            "Success".into(),
            XpcValue::Bool(true),
        )])));
        assert!(parse_query_response(missing, "com.example.missing").is_err());

        let oversized = vec![b'x'; MAX_ERROR_DATA_BYTES + 1];
        let error = message(XpcValue::Dictionary(IndexMap::from([
            ("Success".into(), XpcValue::Bool(false)),
            ("ErrorData".into(), XpcValue::Data(Bytes::from(oversized))),
        ])));
        let error = parse_query_response(error, "com.example.missing")
            .unwrap_err()
            .to_string();
        assert!(error.contains("exceeds maximum"));
        assert!(!error.contains("xxxxxxxx"));

        let error_data = plist::Value::Dictionary(plist::Dictionary::from_iter([(
            "$objects".to_string(),
            plist::Value::Array(vec![
                plist::Value::String("$null".into()),
                plist::Value::String("The operation could not be completed".into()),
            ]),
        )]));
        let mut encoded_error = Vec::new();
        plist::to_writer_binary(&mut encoded_error, &error_data).unwrap();
        let error = message(XpcValue::Dictionary(IndexMap::from([
            ("Success".into(), XpcValue::Bool(false)),
            (
                "ErrorData".into(),
                XpcValue::Data(Bytes::from(encoded_error)),
            ),
        ])));
        let error = parse_query_response(error, "com.example.missing")
            .unwrap_err()
            .to_string();
        assert!(error.contains("could not be completed"));
        assert!(error.contains("com.example.missing"));

        let no_detail = message(XpcValue::Dictionary(IndexMap::from([(
            "Success".into(),
            XpcValue::Bool(false),
        )])));
        assert!(parse_query_response(no_detail, "com.example.missing")
            .unwrap_err()
            .to_string()
            .contains("no error detail"));
    }

    #[test]
    fn request_input_and_response_limits_reject_boundary_overflows() {
        assert!(validate_bundle_identifier("").is_err());
        assert!(validate_bundle_identifier(&"x".repeat(MAX_BUNDLE_ID_BYTES + 1)).is_err());
        assert!(validate_bundle_identifier(&"x".repeat(MAX_BUNDLE_ID_BYTES)).is_ok());

        let mut response = successful_response();
        if let Some(XpcValue::Dictionary(body)) = response.body.as_mut() {
            body.insert(
                "PersistentIdentifier".into(),
                XpcValue::Data(Bytes::from(vec![0; MAX_PERSISTENT_IDENTIFIER_BYTES + 1])),
            );
        }
        assert!(parse_query_response(response, "com.example.missing").is_err());

        let mut response = successful_response();
        if let Some(XpcValue::Dictionary(body)) = response.body.as_mut() {
            body.insert(
                "FutureData".into(),
                XpcValue::Data(Bytes::from(vec![0; MAX_RESPONSE_VALUE_BYTES + 1])),
            );
        }
        assert!(parse_query_response(response, "com.example.missing").is_err());

        let mut nested = XpcValue::String("leaf".into());
        for _ in 0..=MAX_RESPONSE_DEPTH {
            nested = XpcValue::Dictionary(IndexMap::from([("next".into(), nested)]));
        }
        let nested = message(XpcValue::Dictionary(IndexMap::from([
            ("Success".into(), XpcValue::Bool(true)),
            ("DBUUID".into(), XpcValue::String("db".into())),
            ("DBSequence".into(), XpcValue::Uint64(1)),
            (
                "PersistentIdentifier".into(),
                XpcValue::Data(Bytes::from_static(b"id")),
            ),
            ("Nested".into(), nested),
        ])));
        assert!(parse_query_response(nested, "com.example.missing").is_err());
    }
}
