//! iOS 17+ InstallCoordinationProxy service.
//!
//! `com.apple.remote.installcoordination_proxy` is a RemoteXPC service exposed
//! through the RSD tunnel.  The pinned pymobiledevice3 implementation only
//! implements the read-only `Query` request; install/uninstall/stash requests
//! use an out-of-band file-transfer protocol and are deliberately not
//! invented here.

use std::time::Duration;

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
    /// Set after a send/receive failure leaves the connection's message
    /// stream in an unknown state; further queries must not run on it.
    usable: bool,
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
        Self {
            client,
            timeout,
            usable: true,
        }
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
    ///
    /// The deadline starts before the request is sent: send, flow-control
    /// waits, and receive all draw from the same budget, so a stalled write
    /// end or a slow reply can at most consume the configured timeout, never
    /// exceed it.  A zero timeout returns a timeout error without sending the
    /// request.
    ///
    /// The daemon answers with an uncorrelated fresh message and the response
    /// stream is not part of the request contract: pymobiledevice3 consumes
    /// the next DATA message regardless of its stream id, so the reply is
    /// read from whichever stream it arrives on while message reassembly
    /// stays strictly per-stream.  Zero-length wrappers and empty dictionaries
    /// around the real response are skipped.  Once the input and deadline
    /// prechecks pass, the connection is pessimistically marked unusable before
    /// the first I/O await: a timeout, transport failure, or caller cancellation
    /// therefore requires reconnecting, so a late response cannot be misread
    /// as the answer to a later query.  A complete non-empty response restores
    /// reusability before business parsing, preserving the protocol-error
    /// contract that a consumed malformed response does not poison the stream.
    pub async fn query(
        &mut self,
        bundle_identifier: &str,
    ) -> Result<InstallRecord, InstallCoordinationError> {
        if !self.usable {
            return Err(InstallCoordinationError::Protocol(
                "install coordination connection is unusable after a previous failed query; \
                 reconnect required"
                    .into(),
            ));
        }
        validate_bundle_identifier(bundle_identifier)?;
        let request = build_query_request(bundle_identifier);
        // The daemon sends an uncorrelated fresh reply.  Do not use `call`
        // here: it adds WANTING_REPLY and waits on stream 3 for a matching
        // message id.  pymobiledevice3 similarly sends this request without
        // WANTING_REPLY and consumes the next response directly.
        let deadline = tokio::time::Instant::now()
            .checked_add(self.timeout)
            .ok_or_else(|| {
                InstallCoordinationError::Protocol(format!(
                    "query timeout {:#?} exceeds the representable instant range",
                    self.timeout
                ))
            })?;
        // A zero timeout (or a deadline the timer has already passed) must
        // never put a business request on the wire.  The tokio timer's
        // granularity alone cannot guarantee that, so check it explicitly.
        // Nothing was sent, so the connection stays in sync and remains
        // usable for a later query.
        if self.timeout.is_zero() || tokio::time::Instant::now() >= deadline {
            return Err(InstallCoordinationError::Timeout(self.timeout));
        }
        // The wire protocol has no request/response correlation. Once all
        // pre-I/O checks have passed, a caller may cancel this future at any
        // await point, including during a partial write. Mark the connection
        // unusable before the first I/O await so an abandoned request can
        // never be followed by a query that consumes its late response.
        self.usable = false;
        match tokio::time::timeout_at(deadline, self.client.send(request)).await {
            Ok(Ok(())) => {}
            Ok(Err(error)) => {
                self.usable = false;
                return Err(error.into());
            }
            Err(_elapsed) => {
                self.usable = false;
                return Err(InstallCoordinationError::Timeout(self.timeout));
            }
        }
        loop {
            let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
            if remaining.is_zero() {
                self.usable = false;
                return Err(InstallCoordinationError::Timeout(self.timeout));
            }
            let response = match tokio::time::timeout(remaining, self.client.recv_any()).await {
                Ok(Ok(message)) => message,
                Ok(Err(error)) => {
                    self.usable = false;
                    return Err(error.into());
                }
                Err(_elapsed) => {
                    self.usable = false;
                    return Err(InstallCoordinationError::Timeout(self.timeout));
                }
            };
            // RemoteXPC peers may emit zero-length wrappers or empty
            // dictionaries around the real response.  pmd3's receive_response
            // skips both; empty messages do not extend the deadline.
            if is_skippable_message(&response) {
                continue;
            }
            // A protocol error means a whole message was consumed and the
            // connection stays in sync, so it does not poison the client; a
            // later query simply consumes the next response.
            self.usable = true;
            return parse_query_response(response, bundle_identifier);
        }
    }
}

/// Empty XPC wrapper frames and empty dictionaries carry no business data.
fn is_skippable_message(response: &XpcMessage) -> bool {
    match &response.body {
        None => true,
        Some(XpcValue::Dictionary(values)) => values.is_empty(),
        Some(_) => false,
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
        .and_then(as_sequence_uint64)
        .ok_or_else(|| {
            InstallCoordinationError::Protocol("query response DBSequence is not an integer".into())
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

/// Accept both integer wire encodings for `DBSequence`.
///
/// The daemon sends the sequence as the signed int64 XPC type on real
/// devices (observed on iOS 18.7.9), not the unsigned type; pymobiledevice3
/// accepts either.  Negative values remain invalid for a sequence number.
fn as_sequence_uint64(value: &XpcValue) -> Option<u64> {
    match value {
        XpcValue::Uint64(value) => Some(*value),
        XpcValue::Int64(value) => u64::try_from(*value).ok(),
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
    use std::future::Future;
    use std::pin::Pin;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::Arc;
    use std::time::Instant;

    use bytes::Bytes;
    use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt, DuplexStream};

    use crate::xpc::message::encode_message;

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
    fn parses_device_shape_response_with_signed_db_sequence() {
        // Real devices (observed on iOS 18.7.9) encode DBSequence with the
        // signed int64 XPC type; the record must still parse.
        let record = parse_query_response(
            message(XpcValue::Dictionary(IndexMap::from([
                ("Success".into(), XpcValue::Bool(true)),
                (
                    "DBUUID".into(),
                    XpcValue::String("D1D20BD4-9669-47A6-B577-F6D62ED45B43".into()),
                ),
                ("DBSequence".into(), XpcValue::Int64(276)),
                (
                    "InstallPath".into(),
                    encode_url("file:///Applications/Preferences.app/"),
                ),
                (
                    "PersistentIdentifier".into(),
                    XpcValue::Data(Bytes::from_static(b"\0\x01")),
                ),
            ]))),
            "com.apple.Preferences",
        )
        .unwrap();
        assert_eq!(record.db_sequence, 276);

        // A negative sequence is not a valid integer sequence number.
        let negative = parse_query_response(
            message(XpcValue::Dictionary(IndexMap::from([
                ("Success".into(), XpcValue::Bool(true)),
                ("DBUUID".into(), XpcValue::String("db".into())),
                ("DBSequence".into(), XpcValue::Int64(-1)),
                (
                    "PersistentIdentifier".into(),
                    XpcValue::Data(Bytes::from_static(b"id")),
                ),
            ]))),
            "com.apple.Preferences",
        );
        assert!(negative.is_err());
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

    // ------------------------------------------------------------------
    // Fake-transport tests (X-01..X-07): a full RemoteXPC peer drives the
    // public `query` API over an in-memory H2 connection.
    // ------------------------------------------------------------------

    const FRAME_DATA: u8 = 0x00;
    const FRAME_HEADERS: u8 = 0x01;
    const FRAME_SETTINGS: u8 = 0x04;
    const FRAME_WINDOW_UPDATE: u8 = 0x08;
    const STREAM_INIT: u32 = 0;
    const STREAM_CLIENT_SERVER: u32 = 1;
    const STREAM_SERVER_CLIENT: u32 = 3;

    fn build_frame(frame_type: u8, frame_flags: u8, stream_id: u32, payload: &[u8]) -> Vec<u8> {
        let len = payload.len();
        let mut out = Vec::with_capacity(9 + len);
        out.push(((len >> 16) & 0xFF) as u8);
        out.push(((len >> 8) & 0xFF) as u8);
        out.push((len & 0xFF) as u8);
        out.push(frame_type);
        out.push(frame_flags);
        out.extend_from_slice(&(stream_id & 0x7FFF_FFFF).to_be_bytes());
        out.extend_from_slice(payload);
        out
    }

    fn data_frame(stream_id: u32, payload: &[u8]) -> Vec<u8> {
        build_frame(FRAME_DATA, 0, stream_id, payload)
    }

    fn window_update_frame(stream_id: u32, increment: u32) -> Vec<u8> {
        build_frame(FRAME_WINDOW_UPDATE, 0, stream_id, &increment.to_be_bytes())
    }

    fn encode_xpc(flags: u32, msg_id: u64, body: Option<XpcValue>) -> Vec<u8> {
        encode_message(&XpcMessage {
            flags,
            msg_id,
            body,
        })
        .expect("message should encode")
        .to_vec()
    }

    fn decode_xpc(payload: &[u8]) -> XpcMessage {
        crate::xpc::message::decode_message(Bytes::copy_from_slice(payload))
            .expect("message should decode")
    }

    async fn read_frame<S: AsyncRead + Unpin>(peer: &mut S) -> (u8, u8, u32, Vec<u8>) {
        let mut header = [0u8; 9];
        peer.read_exact(&mut header)
            .await
            .expect("peer should read a frame header");
        let len = ((header[0] as usize) << 16) | ((header[1] as usize) << 8) | (header[2] as usize);
        let mut payload = vec![0u8; len];
        if len > 0 {
            peer.read_exact(&mut payload)
                .await
                .expect("peer should read a frame payload");
        }
        (
            header[3],
            header[4],
            u32::from_be_bytes(header[5..9].try_into().unwrap()),
            payload,
        )
    }

    async fn read_request(peer: &mut DuplexStream) -> XpcMessage {
        let (frame_type, _, stream_id, payload) = read_frame(peer).await;
        assert_eq!(frame_type, FRAME_DATA, "requests arrive as DATA frames");
        assert_eq!(stream_id, STREAM_CLIENT_SERVER, "requests use stream 1");
        decode_xpc(&payload)
    }

    async fn send_on_stream(peer: &mut DuplexStream, stream_id: u32, message: &[u8]) {
        peer.write_all(&data_frame(stream_id, message))
            .await
            .expect("peer should write a data frame");
        peer.flush().await.expect("peer should flush");
    }

    async fn send_data_frame_on_stream(
        peer: &mut DuplexStream,
        stream_id: u32,
        frame_flags: u8,
        message: &[u8],
    ) {
        peer.write_all(&build_frame(FRAME_DATA, frame_flags, stream_id, message))
            .await
            .expect("peer should write a data frame");
        peer.flush().await.expect("peer should flush");
    }

    /// Complete the H2/RemoteXPC bootstrap by impersonating the daemon, then
    /// return with the connection ready for business requests.
    async fn bootstrap_peer(peer: &mut DuplexStream) {
        let mut preface = [0u8; 24];
        peer.read_exact(&mut preface).await.unwrap();
        assert_eq!(&preface, crate::xpc::h2_raw::H2_PREFACE);
        let mut settings = [0u8; 21];
        peer.read_exact(&mut settings).await.unwrap();
        let mut window_update = [0u8; 13];
        peer.read_exact(&mut window_update).await.unwrap();

        peer.write_all(&build_frame(FRAME_SETTINGS, 0, STREAM_INIT, &[]))
            .await
            .unwrap();
        peer.flush().await.unwrap();
        let mut ack = [0u8; 9];
        peer.read_exact(&mut ack).await.unwrap();
        assert_eq!(ack[3], FRAME_SETTINGS);

        let (frame_type, _, stream_id, _) = read_frame(peer).await;
        assert_eq!(
            (frame_type, stream_id),
            (FRAME_HEADERS, STREAM_CLIENT_SERVER)
        );
        let (frame_type, _, stream_id, payload) = read_frame(peer).await;
        assert_eq!((frame_type, stream_id), (FRAME_DATA, STREAM_CLIENT_SERVER));
        let msg1 = decode_xpc(&payload);
        assert_eq!(msg1.flags, crate::xpc::message::flags::ALWAYS_SET);

        let empty = encode_xpc(crate::xpc::message::flags::ALWAYS_SET, 0, None);
        send_on_stream(peer, STREAM_CLIENT_SERVER, &empty).await;

        let (frame_type, _, stream_id, _) = read_frame(peer).await;
        assert_eq!(
            (frame_type, stream_id),
            (FRAME_HEADERS, STREAM_SERVER_CLIENT)
        );
        let (frame_type, _, stream_id, _) = read_frame(peer).await;
        assert_eq!((frame_type, stream_id), (FRAME_DATA, STREAM_CLIENT_SERVER));

        // The client's second stream-1 discard only completes after a second
        // peer message on stream 1 (the connect_stream reference test sends
        // the same message twice).
        send_on_stream(peer, STREAM_CLIENT_SERVER, &empty).await;

        let (frame_type, _, stream_id, _) = read_frame(peer).await;
        assert_eq!((frame_type, stream_id), (FRAME_DATA, STREAM_SERVER_CLIENT));

        send_on_stream(peer, STREAM_SERVER_CLIENT, &empty).await;
    }

    async fn connect_with_timeout(
        timeout: Duration,
    ) -> (InstallCoordinationProxyClient, DuplexStream) {
        let (client_side, mut peer) = tokio::io::duplex(64 * 1024);
        let connect = XpcClient::connect_stream(client_side);
        let bootstrap = bootstrap_peer(&mut peer);
        let (client, ()) = tokio::join!(connect, bootstrap);
        let client = client.expect("connect should succeed");
        (
            InstallCoordinationProxyClient::with_timeout(client, timeout),
            peer,
        )
    }

    /// A duplex wrapper whose write side can be switched to hang forever,
    /// simulating a peer that accepts the connection but stops draining.
    struct GatedDuplex {
        inner: DuplexStream,
        open: Arc<AtomicBool>,
    }

    impl GatedDuplex {
        fn new(inner: DuplexStream) -> (Self, Arc<AtomicBool>) {
            let open = Arc::new(AtomicBool::new(true));
            (
                Self {
                    inner,
                    open: Arc::clone(&open),
                },
                open,
            )
        }
    }

    impl AsyncRead for GatedDuplex {
        fn poll_read(
            mut self: std::pin::Pin<&mut Self>,
            cx: &mut std::task::Context<'_>,
            buf: &mut tokio::io::ReadBuf<'_>,
        ) -> std::task::Poll<std::io::Result<()>> {
            std::pin::Pin::new(&mut self.inner).poll_read(cx, buf)
        }
    }

    impl AsyncWrite for GatedDuplex {
        fn poll_write(
            mut self: std::pin::Pin<&mut Self>,
            cx: &mut std::task::Context<'_>,
            buf: &[u8],
        ) -> std::task::Poll<std::io::Result<usize>> {
            if !self.open.load(Ordering::Acquire) {
                // Pending without registering a waker: the write side never
                // becomes ready again on its own, exactly like a stalled peer.
                return std::task::Poll::Pending;
            }
            std::pin::Pin::new(&mut self.inner).poll_write(cx, buf)
        }

        fn poll_flush(
            mut self: std::pin::Pin<&mut Self>,
            cx: &mut std::task::Context<'_>,
        ) -> std::task::Poll<std::io::Result<()>> {
            std::pin::Pin::new(&mut self.inner).poll_flush(cx)
        }

        fn poll_shutdown(
            mut self: std::pin::Pin<&mut Self>,
            cx: &mut std::task::Context<'_>,
        ) -> std::task::Poll<std::io::Result<()>> {
            std::pin::Pin::new(&mut self.inner).poll_shutdown(cx)
        }
    }

    /// A duplex wrapper that delays exactly the next write after it is armed.
    /// This makes the query's shared send/receive budget observable without
    /// delaying the XPC bootstrap itself.
    struct DelayedWriteDuplex {
        inner: DuplexStream,
        armed: Arc<AtomicBool>,
        delay: Duration,
        sleep: Option<Pin<Box<tokio::time::Sleep>>>,
    }

    impl DelayedWriteDuplex {
        fn new(inner: DuplexStream, delay: Duration) -> (Self, Arc<AtomicBool>) {
            let armed = Arc::new(AtomicBool::new(false));
            (
                Self {
                    inner,
                    armed: Arc::clone(&armed),
                    delay,
                    sleep: None,
                },
                armed,
            )
        }
    }

    impl AsyncRead for DelayedWriteDuplex {
        fn poll_read(
            mut self: Pin<&mut Self>,
            cx: &mut std::task::Context<'_>,
            buf: &mut tokio::io::ReadBuf<'_>,
        ) -> std::task::Poll<std::io::Result<()>> {
            Pin::new(&mut self.inner).poll_read(cx, buf)
        }
    }

    impl AsyncWrite for DelayedWriteDuplex {
        fn poll_write(
            mut self: Pin<&mut Self>,
            cx: &mut std::task::Context<'_>,
            buf: &[u8],
        ) -> std::task::Poll<std::io::Result<usize>> {
            if self.armed.load(Ordering::Acquire) {
                if self.sleep.is_none() {
                    self.sleep = Some(Box::pin(tokio::time::sleep(self.delay)));
                }
                if self
                    .sleep
                    .as_mut()
                    .expect("delay sleep should be initialized")
                    .as_mut()
                    .poll(cx)
                    .is_pending()
                {
                    return std::task::Poll::Pending;
                }
                self.sleep = None;
                self.armed.store(false, Ordering::Release);
            }
            Pin::new(&mut self.inner).poll_write(cx, buf)
        }

        fn poll_flush(
            mut self: Pin<&mut Self>,
            cx: &mut std::task::Context<'_>,
        ) -> std::task::Poll<std::io::Result<()>> {
            Pin::new(&mut self.inner).poll_flush(cx)
        }

        fn poll_shutdown(
            mut self: Pin<&mut Self>,
            cx: &mut std::task::Context<'_>,
        ) -> std::task::Poll<std::io::Result<()>> {
            Pin::new(&mut self.inner).poll_shutdown(cx)
        }
    }

    /// A duplex wrapper that writes a small prefix of the next enabled write,
    /// then stays pending. This models cancellation after a partial H2 frame
    /// has reached the peer.
    struct PartialWriteDuplex {
        inner: DuplexStream,
        armed: Arc<AtomicBool>,
        stalled: bool,
        wrote_partial: Arc<AtomicBool>,
    }

    impl PartialWriteDuplex {
        fn new(inner: DuplexStream) -> (Self, Arc<AtomicBool>, Arc<AtomicBool>) {
            let armed = Arc::new(AtomicBool::new(false));
            let wrote_partial = Arc::new(AtomicBool::new(false));
            (
                Self {
                    inner,
                    armed: Arc::clone(&armed),
                    stalled: false,
                    wrote_partial: Arc::clone(&wrote_partial),
                },
                armed,
                wrote_partial,
            )
        }
    }

    impl AsyncRead for PartialWriteDuplex {
        fn poll_read(
            mut self: Pin<&mut Self>,
            cx: &mut std::task::Context<'_>,
            buf: &mut tokio::io::ReadBuf<'_>,
        ) -> std::task::Poll<std::io::Result<()>> {
            Pin::new(&mut self.inner).poll_read(cx, buf)
        }
    }

    impl AsyncWrite for PartialWriteDuplex {
        fn poll_write(
            mut self: Pin<&mut Self>,
            cx: &mut std::task::Context<'_>,
            buf: &[u8],
        ) -> std::task::Poll<std::io::Result<usize>> {
            if self.armed.load(Ordering::Acquire) {
                if self.stalled {
                    return std::task::Poll::Pending;
                }
                let prefix_len = buf.len().min(4);
                let result = Pin::new(&mut self.inner).poll_write(cx, &buf[..prefix_len]);
                if matches!(result, std::task::Poll::Ready(Ok(count)) if count > 0) {
                    self.stalled = true;
                    self.wrote_partial.store(true, Ordering::Release);
                }
                return result;
            }
            Pin::new(&mut self.inner).poll_write(cx, buf)
        }

        fn poll_flush(
            mut self: Pin<&mut Self>,
            cx: &mut std::task::Context<'_>,
        ) -> std::task::Poll<std::io::Result<()>> {
            Pin::new(&mut self.inner).poll_flush(cx)
        }

        fn poll_shutdown(
            mut self: Pin<&mut Self>,
            cx: &mut std::task::Context<'_>,
        ) -> std::task::Poll<std::io::Result<()>> {
            Pin::new(&mut self.inner).poll_shutdown(cx)
        }
    }

    async fn connect_with_delayed_write(
        timeout: Duration,
        delay: Duration,
    ) -> (
        InstallCoordinationProxyClient,
        DuplexStream,
        Arc<AtomicBool>,
    ) {
        let (client_side, mut peer) = tokio::io::duplex(64 * 1024);
        let (delayed, armed) = DelayedWriteDuplex::new(client_side, delay);
        let connect = XpcClient::connect_stream(delayed);
        let bootstrap = bootstrap_peer(&mut peer);
        let (client, ()) = tokio::join!(connect, bootstrap);
        (
            InstallCoordinationProxyClient::with_timeout(
                client.expect("connect should succeed"),
                timeout,
            ),
            peer,
            armed,
        )
    }

    async fn connect_with_partial_write(
        timeout: Duration,
    ) -> (
        InstallCoordinationProxyClient,
        DuplexStream,
        Arc<AtomicBool>,
        Arc<AtomicBool>,
    ) {
        let (client_side, mut peer) = tokio::io::duplex(64 * 1024);
        let (partial, armed, wrote_partial) = PartialWriteDuplex::new(client_side);
        let connect = XpcClient::connect_stream(partial);
        let bootstrap = bootstrap_peer(&mut peer);
        let (client, ()) = tokio::join!(connect, bootstrap);
        (
            InstallCoordinationProxyClient::with_timeout(
                client.expect("connect should succeed"),
                timeout,
            ),
            peer,
            armed,
            wrote_partial,
        )
    }

    /// X-01: a permanently pending write end must be bounded by the query
    /// deadline itself, not by an outer watchdog.
    #[tokio::test]
    async fn x01_query_times_out_when_send_never_completes() {
        let (client_side, mut peer) = tokio::io::duplex(64 * 1024);
        let (gated, gate) = GatedDuplex::new(client_side);
        let connect = XpcClient::connect_stream(gated);
        let bootstrap = bootstrap_peer(&mut peer);
        let (client, ()) = tokio::join!(connect, bootstrap);
        let mut proxy = InstallCoordinationProxyClient::with_timeout(
            client.expect("connect should succeed"),
            Duration::from_millis(500),
        );

        gate.store(false, Ordering::Release);

        let started = Instant::now();
        let result = proxy.query("com.apple.Preferences").await;
        let elapsed = started.elapsed();
        match result {
            Err(InstallCoordinationError::Timeout(_)) => {}
            other => panic!("expected a query timeout, got {other:?}"),
        }
        assert!(
            elapsed < Duration::from_millis(2000),
            "the query deadline must bound a pending send, took {elapsed:?}"
        );
    }

    /// X-02 (shared budget): the request reaches the peer, and a reply after
    /// the remaining budget has been spent must time out instead of enjoying
    /// a fresh receive deadline.
    #[tokio::test]
    async fn x02_receive_uses_remaining_budget_not_a_fresh_one() {
        let (mut proxy, mut peer, send_gate) =
            connect_with_delayed_write(Duration::from_millis(700), Duration::from_millis(350))
                .await;
        // Spend part of the query budget before the request reaches the peer.
        // A receive-only deadline would incorrectly restart the full 700 ms
        // budget after this delayed write completes.
        send_gate.store(true, Ordering::Release);
        let responder = tokio::spawn(async move {
            let request = read_request(&mut peer).await;
            assert_eq!(
                request.flags,
                crate::xpc::message::flags::ALWAYS_SET | crate::xpc::message::flags::DATA,
                "the request must reach the peer without WANTING_REPLY"
            );
            // The delayed send takes about 350 ms, so the reply arrives at
            // about 850 ms. A receive-only 700 ms timeout would incorrectly
            // accept it after restarting the budget; the shared deadline
            // must time out first.
            tokio::time::sleep(Duration::from_millis(500)).await;
            send_on_stream(
                &mut peer,
                STREAM_CLIENT_SERVER,
                &encode_xpc(
                    crate::xpc::message::flags::ALWAYS_SET | crate::xpc::message::flags::DATA,
                    77,
                    Some(XpcValue::Dictionary(IndexMap::from([
                        ("Success".into(), XpcValue::Bool(true)),
                        ("DBUUID".into(), XpcValue::String("db".into())),
                        ("DBSequence".into(), XpcValue::Uint64(1)),
                        (
                            "PersistentIdentifier".into(),
                            XpcValue::Data(Bytes::from_static(b"id")),
                        ),
                    ]))),
                ),
            )
            .await;
        });

        let started = Instant::now();
        let result = proxy.query("com.apple.Preferences").await;
        let elapsed = started.elapsed();
        assert!(
            matches!(result, Err(InstallCoordinationError::Timeout(_))),
            "a reply past the shared deadline must time out, got {result:?}"
        );
        assert!(
            elapsed >= Duration::from_millis(600),
            "the query must draw from its full budget, took {elapsed:?}"
        );
        assert!(
            elapsed < Duration::from_millis(1100),
            "a fresh receive budget would still have succeeded at ~850ms, took {elapsed:?}"
        );
        responder.await.unwrap();
    }

    /// X-02 (empty messages): skipped wrappers and dictionaries must not
    /// reset or extend the deadline.
    #[tokio::test]
    async fn x02_empty_messages_do_not_reset_the_deadline() {
        let (mut proxy, mut peer) = connect_with_timeout(Duration::from_millis(700)).await;
        let always_set = crate::xpc::message::flags::ALWAYS_SET;
        let responder = tokio::spawn(async move {
            read_request(&mut peer).await;
            tokio::time::sleep(Duration::from_millis(400)).await;
            send_on_stream(
                &mut peer,
                STREAM_CLIENT_SERVER,
                &encode_xpc(always_set, 70, None),
            )
            .await;
            tokio::time::sleep(Duration::from_millis(400)).await;
            send_on_stream(
                &mut peer,
                STREAM_CLIENT_SERVER,
                &encode_xpc(
                    always_set | crate::xpc::message::flags::DATA,
                    71,
                    Some(XpcValue::Dictionary(IndexMap::new())),
                ),
            )
            .await;
            tokio::time::sleep(Duration::from_millis(400)).await;
            send_on_stream(
                &mut peer,
                STREAM_CLIENT_SERVER,
                &encode_xpc(
                    always_set | crate::xpc::message::flags::DATA,
                    72,
                    Some(XpcValue::Dictionary(IndexMap::from([
                        ("Success".into(), XpcValue::Bool(true)),
                        ("DBUUID".into(), XpcValue::String("db".into())),
                        ("DBSequence".into(), XpcValue::Uint64(1)),
                        (
                            "PersistentIdentifier".into(),
                            XpcValue::Data(Bytes::from_static(b"id")),
                        ),
                    ]))),
                ),
            )
            .await;
        });

        let started = Instant::now();
        let result = proxy.query("com.apple.Preferences").await;
        let elapsed = started.elapsed();
        assert!(
            matches!(result, Err(InstallCoordinationError::Timeout(_))),
            "empty messages must not extend the deadline, got {result:?}"
        );
        assert!(
            elapsed < Duration::from_millis(1050),
            "the deadline must survive empty messages, took {elapsed:?}"
        );
        responder.await.unwrap();
    }

    /// X-02 (zero timeout): the request is never sent.
    #[tokio::test]
    async fn x02_zero_timeout_sends_nothing() {
        let (client_side, mut peer) = tokio::io::duplex(64 * 1024);
        let connect = XpcClient::connect_stream(client_side);
        let bootstrap = bootstrap_peer(&mut peer);
        let (client, ()) = tokio::join!(connect, bootstrap);
        let mut proxy = InstallCoordinationProxyClient::with_timeout(
            client.expect("connect should succeed"),
            Duration::ZERO,
        );

        let started = Instant::now();
        let result = proxy.query("com.apple.Preferences").await;
        assert!(
            matches!(result, Err(InstallCoordinationError::Timeout(_))),
            "a zero timeout must fail without sending: {result:?}"
        );
        assert!(
            started.elapsed() < Duration::from_millis(500),
            "a zero timeout must not wait"
        );

        // The connection never carried a business request: after the client
        // drops its end the peer must observe EOF without ever seeing a
        // DATA frame.
        drop(proxy);
        let mut saw_data_frame = false;
        let drain = async {
            loop {
                let mut header = [0u8; 9];
                match peer.read_exact(&mut header).await {
                    Ok(_) => {
                        if header[3] == FRAME_DATA {
                            saw_data_frame = true;
                            break;
                        }
                        let len = ((header[0] as usize) << 16)
                            | ((header[1] as usize) << 8)
                            | (header[2] as usize);
                        let mut payload = vec![0u8; len];
                        peer.read_exact(&mut payload).await.unwrap();
                    }
                    Err(_) => break,
                }
            }
        };
        tokio::time::timeout(Duration::from_secs(2), drain)
            .await
            .expect("the peer should reach EOF");
        assert!(
            !saw_data_frame,
            "no business request may be written under a zero timeout"
        );
    }

    /// X-08: canceling a query after its request was sent must poison the
    /// connection, because the uncorrelated response may arrive later.
    #[tokio::test]
    async fn x08_external_cancel_after_send_rejects_reuse_without_a_new_request() {
        let (mut proxy, mut peer) = connect_with_timeout(Duration::from_secs(30)).await;
        let (request_seen_tx, request_seen_rx) = tokio::sync::oneshot::channel();
        let responder = tokio::spawn(async move {
            read_request(&mut peer).await;
            let _ = request_seen_tx.send(());
            // A reusable connection would let the next query put another
            // DATA frame on this peer. Keep the read bounded to prove that it
            // never happens after cancellation.
            tokio::time::timeout(Duration::from_millis(400), read_frame(&mut peer)).await
        });

        let canceled = tokio::time::timeout(
            Duration::from_millis(100),
            proxy.query("com.apple.Preferences"),
        )
        .await;
        assert!(
            canceled.is_err(),
            "the first query should be externally canceled"
        );
        tokio::time::timeout(Duration::from_secs(1), request_seen_rx)
            .await
            .expect("the peer should observe the sent request")
            .expect("request-seen signal should not be dropped");

        let next = tokio::time::timeout(
            Duration::from_millis(100),
            proxy.query("com.apple.Preferences"),
        )
        .await;
        assert!(
            matches!(next, Ok(Err(InstallCoordinationError::Protocol(ref message))) if message.contains("unusable")),
            "cancellation must reject reuse immediately: {next:?}"
        );

        let second_request = responder.await.expect("peer task should finish");
        assert!(
            second_request.is_err(),
            "cancellation must not send a second request: {second_request:?}"
        );
    }

    /// X-08: cancellation while the H2 write is blocked must have the same
    /// poison semantics as cancellation while waiting for a response.
    #[tokio::test]
    async fn x08_external_cancel_during_blocked_send_rejects_reuse() {
        let (client_side, mut peer) = tokio::io::duplex(64 * 1024);
        let (gated, gate) = GatedDuplex::new(client_side);
        let connect = XpcClient::connect_stream(gated);
        let bootstrap = bootstrap_peer(&mut peer);
        let (client, ()) = tokio::join!(connect, bootstrap);
        let mut proxy = InstallCoordinationProxyClient::with_timeout(
            client.expect("connect should succeed"),
            Duration::from_secs(30),
        );

        gate.store(false, Ordering::Release);
        let canceled = tokio::time::timeout(
            Duration::from_millis(100),
            proxy.query("com.apple.Preferences"),
        )
        .await;
        assert!(
            canceled.is_err(),
            "the blocked send should be externally canceled"
        );

        let next = tokio::time::timeout(
            Duration::from_millis(100),
            proxy.query("com.apple.Preferences"),
        )
        .await;
        assert!(
            matches!(next, Ok(Err(InstallCoordinationError::Protocol(ref message))) if message.contains("unusable")),
            "cancellation during send must reject reuse immediately: {next:?}"
        );
    }

    /// X-08: cancellation after a partial DATA frame has been written must
    /// also poison the connection and prevent a follow-up write.
    #[tokio::test]
    async fn x08_external_cancel_during_partial_send_rejects_reuse() {
        let (mut proxy, mut peer, send_gate, wrote_partial) =
            connect_with_partial_write(Duration::from_secs(30)).await;
        send_gate.store(true, Ordering::Release);

        let canceled = tokio::time::timeout(
            Duration::from_millis(100),
            proxy.query("com.apple.Preferences"),
        )
        .await;
        assert!(
            canceled.is_err(),
            "the partial send should be externally canceled"
        );
        assert!(
            wrote_partial.load(Ordering::Acquire),
            "the fake transport must write a partial frame before cancellation"
        );

        let next = tokio::time::timeout(
            Duration::from_millis(100),
            proxy.query("com.apple.Preferences"),
        )
        .await;
        assert!(
            matches!(next, Ok(Err(InstallCoordinationError::Protocol(ref message))) if message.contains("unusable")),
            "cancellation after partial send must reject reuse immediately: {next:?}"
        );
        drop(proxy);
        let _ = tokio::time::timeout(Duration::from_millis(100), async {
            let mut byte = [0u8; 1];
            let _ = peer.read(&mut byte).await;
        })
        .await;
    }

    /// X-03: a valid response on stream 1 with a fresh message id parses into
    /// an exact install record; the request carries no WANTING_REPLY.
    #[tokio::test]
    async fn x03_stream1_response_parses_with_fresh_id_and_no_wanting_reply() {
        let (mut proxy, mut peer) = connect_with_timeout(Duration::from_secs(5)).await;
        let responder = tokio::spawn(async move {
            let request = read_request(&mut peer).await;
            assert_eq!(
                request.flags,
                crate::xpc::message::flags::ALWAYS_SET | crate::xpc::message::flags::DATA,
                "the request must not carry WANTING_REPLY"
            );
            let request_body = request.body.as_ref().and_then(XpcValue::as_dict).unwrap();
            assert_eq!(
                request_body["BundleID"].as_str(),
                Some("com.apple.Preferences")
            );
            assert_eq!(request_body["RequestVersion"], XpcValue::Uint64(1));
            assert_eq!(request_body["ProtocolVersion"], XpcValue::Uint64(1));
            assert_eq!(request_body["RequestType"], XpcValue::Uint64(4));

            send_on_stream(
                &mut peer,
                STREAM_CLIENT_SERVER,
                &encode_xpc(
                    crate::xpc::message::flags::ALWAYS_SET | crate::xpc::message::flags::DATA,
                    77,
                    Some(XpcValue::Dictionary(IndexMap::from([
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
                    ]))),
                ),
            )
            .await;
        });

        let record = proxy.query("com.apple.Preferences").await.unwrap();
        responder.await.unwrap();
        assert_eq!(record.db_uuid, "D1D20BD4-9669-47A6-B577-F6D62ED45B43");
        assert_eq!(record.db_sequence, 276);
        assert_eq!(
            record.install_path.as_deref(),
            Some("file:///Applications/Preferences.app/")
        );
        assert_eq!(record.persistent_identifier, b"\0\x01");
        assert_eq!(record.extra["FutureField"].as_str(), Some("kept"));
    }

    /// X-04: the response stream is not part of the request contract.  A
    /// valid answer on stream 3 must be accepted even with interleaved
    /// non-DATA control frames, matching pymobiledevice3's receive behavior.
    #[tokio::test]
    async fn x04_stream3_response_with_interleaved_control_frames_is_accepted() {
        let (mut proxy, mut peer) = connect_with_timeout(Duration::from_secs(5)).await;
        let responder = tokio::spawn(async move {
            read_request(&mut peer).await;
            // Interleave a control frame before the real response.
            peer.write_all(&window_update_frame(STREAM_INIT, 64))
                .await
                .unwrap();
            peer.flush().await.unwrap();
            send_on_stream(
                &mut peer,
                STREAM_SERVER_CLIENT,
                &encode_xpc(
                    crate::xpc::message::flags::ALWAYS_SET | crate::xpc::message::flags::DATA,
                    42,
                    Some(XpcValue::Dictionary(IndexMap::from([
                        ("Success".into(), XpcValue::Bool(true)),
                        ("DBUUID".into(), XpcValue::String("db".into())),
                        ("DBSequence".into(), XpcValue::Uint64(9)),
                        (
                            "PersistentIdentifier".into(),
                            XpcValue::Data(Bytes::from_static(b"id")),
                        ),
                    ]))),
                ),
            )
            .await;
        });

        let record = proxy.query("com.apple.Preferences").await.unwrap();
        responder.await.unwrap();
        assert_eq!(record.db_sequence, 9);
    }

    #[tokio::test]
    async fn x04_zero_length_data_on_one_stream_does_not_block_other_response() {
        let (mut proxy, mut peer) = connect_with_timeout(Duration::from_secs(2)).await;
        let responder = tokio::spawn(async move {
            read_request(&mut peer).await;
            // This is legal keep-alive DATA and carries no XPC bytes. The
            // following stream-1 message is the actual uncorrelated reply.
            send_on_stream(&mut peer, STREAM_SERVER_CLIENT, &[]).await;
            send_on_stream(
                &mut peer,
                STREAM_CLIENT_SERVER,
                &encode_xpc(
                    crate::xpc::message::flags::ALWAYS_SET | crate::xpc::message::flags::DATA,
                    43,
                    Some(XpcValue::Dictionary(IndexMap::from([
                        ("Success".into(), XpcValue::Bool(true)),
                        ("DBUUID".into(), XpcValue::String("db".into())),
                        ("DBSequence".into(), XpcValue::Uint64(10)),
                        (
                            "PersistentIdentifier".into(),
                            XpcValue::Data(Bytes::from_static(b"id")),
                        ),
                    ]))),
                ),
            )
            .await;
        });

        let record = proxy.query("com.apple.Preferences").await.unwrap();
        responder.await.unwrap();
        assert_eq!(record.db_sequence, 10);
    }

    #[tokio::test]
    async fn x04_empty_end_stream_tombstone_does_not_block_other_response() {
        let (mut proxy, mut peer) = connect_with_timeout(Duration::from_secs(2)).await;
        let responder = tokio::spawn(async move {
            read_request(&mut peer).await;
            // A clean empty stream termination carries no XPC message. It is
            // retained only as a bounded tombstone and must not win over the
            // valid uncorrelated response that follows on stream 1.
            send_data_frame_on_stream(&mut peer, STREAM_SERVER_CLIENT, 0x01, &[]).await;
            send_on_stream(
                &mut peer,
                STREAM_CLIENT_SERVER,
                &encode_xpc(
                    crate::xpc::message::flags::ALWAYS_SET | crate::xpc::message::flags::DATA,
                    45,
                    Some(XpcValue::Dictionary(IndexMap::from([
                        ("Success".into(), XpcValue::Bool(true)),
                        ("DBUUID".into(), XpcValue::String("db".into())),
                        ("DBSequence".into(), XpcValue::Uint64(12)),
                        (
                            "PersistentIdentifier".into(),
                            XpcValue::Data(Bytes::from_static(b"id")),
                        ),
                    ]))),
                ),
            )
            .await;
        });

        let record = proxy.query("com.apple.Preferences").await.unwrap();
        responder.await.unwrap();
        assert_eq!(record.db_sequence, 12);
    }

    #[tokio::test]
    async fn x04_partial_message_on_one_stream_does_not_block_complete_other_response() {
        let (mut proxy, mut peer) = connect_with_timeout(Duration::from_secs(2)).await;
        let responder = tokio::spawn(async move {
            read_request(&mut peer).await;
            let data_flags =
                crate::xpc::message::flags::ALWAYS_SET | crate::xpc::message::flags::DATA;
            let response = encode_xpc(
                data_flags,
                44,
                Some(XpcValue::Dictionary(IndexMap::from([
                    ("Success".into(), XpcValue::Bool(true)),
                    ("DBUUID".into(), XpcValue::String("db".into())),
                    ("DBSequence".into(), XpcValue::Uint64(11)),
                    (
                        "PersistentIdentifier".into(),
                        XpcValue::Data(Bytes::from_static(b"id")),
                    ),
                ]))),
            );
            // Leave stream 3's XPC wrapper incomplete, then deliver a full
            // message on stream 1. The receiver must choose stream 1 rather
            // than waiting for more bytes on stream 3.
            send_on_stream(&mut peer, STREAM_SERVER_CLIENT, &response[..8]).await;
            send_on_stream(&mut peer, STREAM_CLIENT_SERVER, &response).await;
        });

        let record = proxy.query("com.apple.Preferences").await.unwrap();
        responder.await.unwrap();
        assert_eq!(record.db_sequence, 11);
    }

    /// X-05: empty wrapper, empty dictionary, same-stream fragmentation,
    /// single-frame multi-message, half a message followed by EOF.
    #[tokio::test]
    async fn x05_empty_wrapper_then_valid_response_is_skipped() {
        let (mut proxy, mut peer) = connect_with_timeout(Duration::from_secs(5)).await;
        let always_set = crate::xpc::message::flags::ALWAYS_SET;
        let data_flags = always_set | crate::xpc::message::flags::DATA;
        let responder = tokio::spawn(async move {
            read_request(&mut peer).await;
            send_on_stream(
                &mut peer,
                STREAM_CLIENT_SERVER,
                &encode_xpc(always_set, 70, None),
            )
            .await;
            send_on_stream(
                &mut peer,
                STREAM_CLIENT_SERVER,
                &encode_xpc(data_flags, 71, Some(XpcValue::Dictionary(IndexMap::new()))),
            )
            .await;
            send_on_stream(
                &mut peer,
                STREAM_CLIENT_SERVER,
                &encode_xpc(
                    data_flags,
                    72,
                    Some(XpcValue::Dictionary(IndexMap::from([
                        ("Success".into(), XpcValue::Bool(true)),
                        ("DBUUID".into(), XpcValue::String("db".into())),
                        ("DBSequence".into(), XpcValue::Uint64(5)),
                        (
                            "PersistentIdentifier".into(),
                            XpcValue::Data(Bytes::from_static(b"id")),
                        ),
                    ]))),
                ),
            )
            .await;
        });

        let record = proxy.query("com.apple.Preferences").await.unwrap();
        responder.await.unwrap();
        assert_eq!(record.db_sequence, 5);
    }

    #[tokio::test]
    async fn x05_fragmented_same_stream_message_is_reassembled() {
        let (mut proxy, mut peer) = connect_with_timeout(Duration::from_secs(5)).await;
        let data_flags = crate::xpc::message::flags::ALWAYS_SET | crate::xpc::message::flags::DATA;
        let responder = tokio::spawn(async move {
            read_request(&mut peer).await;
            let message = encode_xpc(
                data_flags,
                73,
                Some(XpcValue::Dictionary(IndexMap::from([
                    ("Success".into(), XpcValue::Bool(true)),
                    ("DBUUID".into(), XpcValue::String("db".into())),
                    ("DBSequence".into(), XpcValue::Uint64(6)),
                    (
                        "PersistentIdentifier".into(),
                        XpcValue::Data(Bytes::from_static(b"id")),
                    ),
                ]))),
            );
            let (head, tail) = message.split_at(message.len() / 2);
            peer.write_all(&data_frame(STREAM_CLIENT_SERVER, head))
                .await
                .unwrap();
            peer.flush().await.unwrap();
            tokio::time::sleep(Duration::from_millis(50)).await;
            peer.write_all(&data_frame(STREAM_CLIENT_SERVER, tail))
                .await
                .unwrap();
            peer.flush().await.unwrap();
        });

        let record = proxy.query("com.apple.Preferences").await.unwrap();
        responder.await.unwrap();
        assert_eq!(record.db_sequence, 6);
    }

    #[tokio::test]
    async fn x05_single_frame_with_two_messages_keeps_stream_boundaries() {
        let (mut proxy, mut peer) = connect_with_timeout(Duration::from_secs(5)).await;
        let data_flags = crate::xpc::message::flags::ALWAYS_SET | crate::xpc::message::flags::DATA;
        let responder = tokio::spawn(async move {
            read_request(&mut peer).await;
            let empty_dict =
                encode_xpc(data_flags, 80, Some(XpcValue::Dictionary(IndexMap::new())));
            let response = encode_xpc(
                data_flags,
                81,
                Some(XpcValue::Dictionary(IndexMap::from([
                    ("Success".into(), XpcValue::Bool(true)),
                    ("DBUUID".into(), XpcValue::String("db".into())),
                    ("DBSequence".into(), XpcValue::Uint64(7)),
                    (
                        "PersistentIdentifier".into(),
                        XpcValue::Data(Bytes::from_static(b"id")),
                    ),
                ]))),
            );
            let mut combined = empty_dict;
            combined.extend_from_slice(&response);
            send_on_stream(&mut peer, STREAM_CLIENT_SERVER, &combined).await;
        });

        let record = proxy.query("com.apple.Preferences").await.unwrap();
        responder.await.unwrap();
        assert_eq!(record.db_sequence, 7);
    }

    #[tokio::test]
    async fn x05_half_message_then_eof_fails_fast_with_transport_error() {
        let (mut proxy, mut peer) = connect_with_timeout(Duration::from_secs(5)).await;
        let data_flags = crate::xpc::message::flags::ALWAYS_SET | crate::xpc::message::flags::DATA;
        let responder = tokio::spawn(async move {
            read_request(&mut peer).await;
            let message = encode_xpc(
                data_flags,
                82,
                Some(XpcValue::Dictionary(IndexMap::from([(
                    "Success".into(),
                    XpcValue::Bool(true),
                )]))),
            );
            // Send only part of the header, then close the connection.
            peer.write_all(&data_frame(STREAM_CLIENT_SERVER, &message[..10]))
                .await
                .unwrap();
            peer.flush().await.unwrap();
            drop(peer);
        });

        let started = Instant::now();
        let result = proxy.query("com.apple.Preferences").await;
        let elapsed = started.elapsed();
        responder.await.unwrap();
        match result {
            Err(InstallCoordinationError::Xpc(_)) => {}
            other => panic!("expected a transport error, got {other:?}"),
        }
        assert!(
            elapsed < Duration::from_secs(4),
            "EOF must surface as an error instead of hanging, took {elapsed:?}"
        );
        // The connection is poisoned after the transport failure.
        let next = proxy.query("com.apple.Preferences").await;
        assert!(
            matches!(next, Err(InstallCoordinationError::Protocol(ref message)) if message.contains("unusable")),
            "a failed connection must not accept further queries: {next:?}"
        );
    }

    /// X-06: unknown bundle errors, unknown fields, invalid input, budget.
    #[tokio::test]
    async fn x06_unknown_bundle_reports_daemon_error_detail() {
        let (mut proxy, mut peer) = connect_with_timeout(Duration::from_secs(5)).await;
        let responder = tokio::spawn(async move {
            read_request(&mut peer).await;
            let error_data = plist::Value::Dictionary(plist::Dictionary::from_iter([(
                "$objects".to_string(),
                plist::Value::Array(vec![
                    plist::Value::String("$null".into()),
                    plist::Value::String("The operation could not be completed".into()),
                ]),
            )]));
            let mut encoded = Vec::new();
            plist::to_writer_binary(&mut encoded, &error_data).unwrap();
            send_on_stream(
                &mut peer,
                STREAM_CLIENT_SERVER,
                &encode_xpc(
                    crate::xpc::message::flags::ALWAYS_SET | crate::xpc::message::flags::DATA,
                    90,
                    Some(XpcValue::Dictionary(IndexMap::from([
                        ("Success".into(), XpcValue::Bool(false)),
                        ("ErrorData".into(), XpcValue::Data(Bytes::from(encoded))),
                    ]))),
                ),
            )
            .await;
        });

        let error = proxy
            .query("com.example.Unknown")
            .await
            .expect_err("unknown bundle must fail");
        responder.await.unwrap();
        let text = error.to_string();
        assert!(text.contains("com.example.Unknown"));
        assert!(text.contains("could not be completed"));
    }

    #[tokio::test]
    async fn x06_invalid_bundle_input_is_rejected_before_sending() {
        let (mut proxy, mut peer) = connect_with_timeout(Duration::from_secs(5)).await;

        assert!(proxy.query("").await.is_err());
        assert!(proxy
            .query(&"x".repeat(MAX_BUNDLE_ID_BYTES + 1))
            .await
            .is_err());
        assert!(proxy.query("bad\u{0007}control").await.is_err());

        drop(proxy);
        let mut saw_data_frame = false;
        let drain = async {
            loop {
                let mut header = [0u8; 9];
                match peer.read_exact(&mut header).await {
                    Ok(_) => {
                        if header[3] == FRAME_DATA {
                            saw_data_frame = true;
                            break;
                        }
                        let len = ((header[0] as usize) << 16)
                            | ((header[1] as usize) << 8)
                            | (header[2] as usize);
                        let mut payload = vec![0u8; len];
                        peer.read_exact(&mut payload).await.unwrap();
                    }
                    Err(_) => break,
                }
            }
        };
        tokio::time::timeout(Duration::from_secs(2), drain)
            .await
            .expect("the peer should reach EOF");
        assert!(
            !saw_data_frame,
            "invalid input must be rejected before any request is written"
        );
    }

    #[tokio::test]
    async fn x06_response_budget_overflow_is_rejected() {
        let (mut proxy, mut peer) = connect_with_timeout(Duration::from_secs(30)).await;
        let data_flags = crate::xpc::message::flags::ALWAYS_SET | crate::xpc::message::flags::DATA;
        // Hold the peer open until the query has finished parsing: a dropped
        // connection would surface as a transport error instead of the
        // protocol rejection under test.
        let (hold_tx, hold_rx) = tokio::sync::oneshot::channel::<()>();
        let responder = tokio::spawn(async move {
            read_request(&mut peer).await;
            let message = encode_xpc(
                data_flags,
                91,
                Some(XpcValue::Dictionary(IndexMap::from([
                    ("Success".into(), XpcValue::Bool(true)),
                    ("DBUUID".into(), XpcValue::String("db".into())),
                    ("DBSequence".into(), XpcValue::Uint64(1)),
                    (
                        "PersistentIdentifier".into(),
                        XpcValue::Data(Bytes::from_static(b"id")),
                    ),
                    (
                        "OversizedData".into(),
                        XpcValue::Data(Bytes::from(vec![0u8; MAX_RESPONSE_VALUE_BYTES + 1])),
                    ),
                ]))),
            );
            for chunk in message.chunks(16_384) {
                peer.write_all(&data_frame(STREAM_CLIENT_SERVER, chunk))
                    .await
                    .unwrap();
            }
            peer.flush().await.unwrap();
            let _ = hold_rx.await;
        });

        let result = proxy.query("com.apple.Preferences").await;
        let _ = hold_tx.send(());
        responder.await.unwrap();
        match result {
            Err(InstallCoordinationError::Protocol(message)) => {
                assert!(message.contains("exceeds maximum"), "{message}")
            }
            other => panic!("expected a budget rejection, got {other:?}"),
        }
    }

    /// X-07: two sequential queries reuse the connection, and a timed-out
    /// query poisons it so a late response cannot impersonate the next one.
    #[tokio::test]
    async fn x07_repeated_queries_work_until_a_timeout_poisons_the_connection() {
        let (mut proxy, mut peer) = connect_with_timeout(Duration::from_millis(300)).await;
        let responder = tokio::spawn(async move {
            for sequence in [10u64, 11] {
                read_request(&mut peer).await;
                send_on_stream(
                    &mut peer,
                    STREAM_CLIENT_SERVER,
                    &encode_xpc(
                        crate::xpc::message::flags::ALWAYS_SET | crate::xpc::message::flags::DATA,
                        sequence,
                        Some(XpcValue::Dictionary(IndexMap::from([
                            ("Success".into(), XpcValue::Bool(true)),
                            ("DBUUID".into(), XpcValue::String("db".into())),
                            ("DBSequence".into(), XpcValue::Uint64(sequence)),
                            (
                                "PersistentIdentifier".into(),
                                XpcValue::Data(Bytes::from_static(b"id")),
                            ),
                        ]))),
                    ),
                )
                .await;
            }
            // Third request: the reply is delayed past the query budget and
            // is only sent after the query must already have timed out.
            read_request(&mut peer).await;
            tokio::time::sleep(Duration::from_millis(600)).await;
            send_on_stream(
                &mut peer,
                STREAM_CLIENT_SERVER,
                &encode_xpc(
                    crate::xpc::message::flags::ALWAYS_SET | crate::xpc::message::flags::DATA,
                    99,
                    Some(XpcValue::Dictionary(IndexMap::from([
                        ("Success".into(), XpcValue::Bool(true)),
                        ("DBUUID".into(), XpcValue::String("late".into())),
                        ("DBSequence".into(), XpcValue::Uint64(99)),
                        (
                            "PersistentIdentifier".into(),
                            XpcValue::Data(Bytes::from_static(b"id")),
                        ),
                    ]))),
                ),
            )
            .await;
        });

        let first = proxy.query("com.apple.Preferences").await.unwrap();
        assert_eq!(first.db_sequence, 10);
        let second = proxy.query("com.apple.Preferences").await.unwrap();
        assert_eq!(second.db_sequence, 11);

        // The third query shares the 300ms budget; the reply only arrives
        // after 600ms, so the deadline must fire first.
        let third_started = Instant::now();
        let third =
            tokio::time::timeout(Duration::from_secs(5), proxy.query("com.apple.Preferences"))
                .await
                .expect("the query deadline must return without an outer watchdog");
        let third_elapsed = third_started.elapsed();
        match third {
            Err(InstallCoordinationError::Timeout(_)) => {
                assert!(
                    third_elapsed >= Duration::from_millis(250),
                    "the third query must draw from its budget, took {third_elapsed:?}"
                );
                assert!(
                    third_elapsed < Duration::from_millis(560),
                    "the third query must time out before the late reply, took {third_elapsed:?}"
                );
            }
            other => panic!("expected the third query to time out, got {other:?}"),
        }

        // The late response must not satisfy a fourth query: the connection
        // is unusable and the error is immediate.
        let fourth_started = Instant::now();
        let fourth = proxy.query("com.apple.Preferences").await;
        assert!(
            matches!(fourth, Err(InstallCoordinationError::Protocol(ref message)) if message.contains("unusable")),
            "the fourth query must be rejected without consuming the late response: {fourth:?}"
        );
        assert!(
            fourth_started.elapsed() < Duration::from_millis(500),
            "the unusable-connection error must be immediate"
        );
        responder.await.unwrap();
    }
}
