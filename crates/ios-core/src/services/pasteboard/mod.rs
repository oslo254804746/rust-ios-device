//! iOS 17+ CoreDevice pasteboard service.
//!
//! The pasteboard service is a RemoteXPC service, but unlike most CoreDevice
//! services it accepts plain dictionaries directly.  There is no
//! `CoreDevice.featureIdentifier`/`CoreDevice.input` envelope: the `command`
//! field (`PULL` or `SET`) selects the operation.
//!
//! The implementation intentionally keeps the wire-facing request and reply
//! values as [`XpcValue`].  This preserves metadata and future UTI types for
//! callers while the convenience helpers cover the common UTF-8 text and URL
//! cases.

use std::time::Duration;

use bytes::Bytes;
use futures_core::Stream;
use futures_util::StreamExt;
use indexmap::IndexMap;

use crate::xpc::{XpcClient, XpcError};
use crate::{XpcMessage, XpcValue};

/// RSD service name for the iOS 17+ pasteboard service.
pub const SERVICE_NAME: &str = "com.apple.coredevice.pasteboardservice";

/// The default system pasteboard.
pub const GENERAL_PASTEBOARD: &str = "general";

/// Pasteboard command verbs.
pub const PULL_COMMAND: &str = "PULL";
pub const PULL_REPLY_COMMAND: &str = "PULL_REPLY";
pub const SET_COMMAND: &str = "SET";
pub const SET_REPLY_COMMAND: &str = "SET_REPLY";

/// Standard Uniform Type Identifiers supported by the convenience helpers.
pub const UTI_UTF8_PLAIN_TEXT: &str = "public.utf8-plain-text";
pub const UTI_PLAIN_TEXT: &str = "public.plain-text";
pub const UTI_TEXT: &str = "public.text";
pub const UTI_URL: &str = "public.url";

/// The upper bound for one PULL or SET round trip.
pub const REQUEST_TIMEOUT: Duration = Duration::from_secs(10);

const TEXT_UTIS: [&str; 3] = [UTI_UTF8_PLAIN_TEXT, UTI_PLAIN_TEXT, UTI_TEXT];

/// Errors returned by the pasteboard service.
#[derive(Debug, thiserror::Error)]
pub enum PasteboardError {
    /// Underlying RemoteXPC/XPC transport failure.
    #[error("xpc error: {0}")]
    Xpc(#[from] XpcError),
    /// The daemon returned a malformed or unsupported response.
    #[error("pasteboard protocol error: {0}")]
    Protocol(String),
    /// The daemon returned its structured service error dictionary.
    #[error("pasteboard service error: {description} (domain {domain}, code {code})")]
    Service {
        domain: String,
        code: String,
        description: String,
    },
    /// The daemon did not provide a populated response within the deadline.
    #[error("pasteboard request timed out after {seconds}s; the connection was closed")]
    Timeout { seconds: u64 },
    /// The client was consumed after a timeout or was otherwise unavailable.
    #[error("pasteboard connection is closed")]
    Closed,
}

/// A pasteboard item containing raw bytes keyed by UTI.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PasteboardItem {
    /// Ordered UTI list from the wire item's `types` field.
    pub types: Vec<String>,
    /// Immediate data keyed by UTI. Promised data is not included here.
    pub data: IndexMap<String, Bytes>,
}

impl PasteboardItem {
    /// Build a text item with the three standard text UTIs.
    pub fn text(text: impl AsRef<str>) -> Self {
        let bytes = Bytes::copy_from_slice(text.as_ref().as_bytes());
        let mut data = IndexMap::new();
        for uti in TEXT_UTIS {
            data.insert(uti.to_string(), bytes.clone());
        }
        Self {
            types: TEXT_UTIS.iter().map(|uti| (*uti).to_string()).collect(),
            data,
        }
    }

    /// Build a URL item using the standard URL UTI and UTF-8 bytes.
    pub fn url(url: impl AsRef<str>) -> Self {
        Self::data(UTI_URL, url.as_ref().as_bytes())
    }

    /// Build an item with one arbitrary UTI and immediate bytes.
    pub fn data(uti: impl Into<String>, data: impl AsRef<[u8]>) -> Self {
        let uti = uti.into();
        let mut values = IndexMap::new();
        values.insert(uti.clone(), Bytes::copy_from_slice(data.as_ref()));
        Self {
            types: vec![uti],
            data: values,
        }
    }
}

/// Build a text pasteboard item. This mirrors the helper name used by the
/// reference Python client while keeping [`PasteboardItem`] as the Rust type.
pub fn text_item(text: impl AsRef<str>) -> PasteboardItem {
    PasteboardItem::text(text)
}

/// Build an immediate-data pasteboard item for one UTI.
pub fn data_item(uti: impl Into<String>, data: impl AsRef<[u8]>) -> PasteboardItem {
    PasteboardItem::data(uti, data)
}

/// A client for `com.apple.coredevice.pasteboardservice`.
pub struct PasteboardClient {
    // Keeping this optional lets us drop the underlying TCP stream when a
    // read times out. A RemoteXPC read can be half-way through a frame, so a
    // timed-out connection must not be reused for a later request.
    client: Option<XpcClient>,
    timeout: Duration,
}

/// Compatibility alias following pymobiledevice3's service naming.
pub type PasteboardService = PasteboardClient;

impl PasteboardClient {
    /// Wrap an initialized XPC client connected to [`SERVICE_NAME`].
    pub fn new(client: XpcClient) -> Self {
        Self {
            client: Some(client),
            timeout: REQUEST_TIMEOUT,
        }
    }

    /// Construct a client with a custom timeout, primarily useful for tests.
    pub fn with_timeout(client: XpcClient, timeout: Duration) -> Self {
        Self {
            client: Some(client),
            timeout,
        }
    }

    /// Pull the named pasteboard and return its raw XPC reply dictionary.
    pub async fn get(&mut self) -> Result<XpcValue, PasteboardError> {
        self.get_named(GENERAL_PASTEBOARD).await
    }

    /// Pull a named pasteboard and return its raw XPC reply dictionary.
    pub async fn get_named(&mut self, pasteboard_name: &str) -> Result<XpcValue, PasteboardError> {
        self.send_receive(build_pull_request(pasteboard_name)).await
    }

    /// Pull the general pasteboard and return the first inline UTF-8 text item.
    pub async fn get_text(&mut self) -> Result<Option<String>, PasteboardError> {
        self.get_text_named(GENERAL_PASTEBOARD).await
    }

    /// Pull a named pasteboard and return the first inline UTF-8 text item.
    pub async fn get_text_named(
        &mut self,
        pasteboard_name: &str,
    ) -> Result<Option<String>, PasteboardError> {
        let reply = self.get_named(pasteboard_name).await?;
        Ok(snapshot_text(&reply))
    }

    /// Pull the general pasteboard and return the first inline UTF-8 URL item.
    pub async fn get_url(&mut self) -> Result<Option<String>, PasteboardError> {
        self.get_url_named(GENERAL_PASTEBOARD).await
    }

    /// Pull a named pasteboard and return the first inline UTF-8 URL item.
    pub async fn get_url_named(
        &mut self,
        pasteboard_name: &str,
    ) -> Result<Option<String>, PasteboardError> {
        let reply = self.get_named(pasteboard_name).await?;
        Ok(snapshot_uti_text(&reply, UTI_URL))
    }

    /// Replace the general pasteboard with one UTF-8 text item.
    pub async fn set_text(&mut self, text: impl AsRef<str>) -> Result<XpcValue, PasteboardError> {
        self.set_text_named(GENERAL_PASTEBOARD, text).await
    }

    /// Replace a named pasteboard with one UTF-8 text item.
    pub async fn set_text_named(
        &mut self,
        pasteboard_name: &str,
        text: impl AsRef<str>,
    ) -> Result<XpcValue, PasteboardError> {
        self.set_named(pasteboard_name, &[PasteboardItem::text(text)])
            .await
    }

    /// Replace the general pasteboard with one UTF-8 URL item.
    pub async fn set_url(&mut self, url: impl AsRef<str>) -> Result<XpcValue, PasteboardError> {
        self.set_url_named(GENERAL_PASTEBOARD, url).await
    }

    /// Replace a named pasteboard with one UTF-8 URL item.
    pub async fn set_url_named(
        &mut self,
        pasteboard_name: &str,
        url: impl AsRef<str>,
    ) -> Result<XpcValue, PasteboardError> {
        self.set_named(pasteboard_name, &[PasteboardItem::url(url)])
            .await
    }

    /// Replace the general pasteboard with arbitrary immediate items.
    pub async fn set(&mut self, items: &[PasteboardItem]) -> Result<XpcValue, PasteboardError> {
        self.set_named(GENERAL_PASTEBOARD, items).await
    }

    /// Replace a named pasteboard with arbitrary immediate items.
    pub async fn set_named(
        &mut self,
        pasteboard_name: &str,
        items: &[PasteboardItem],
    ) -> Result<XpcValue, PasteboardError> {
        self.set_named_with_source_metadata(pasteboard_name, items, None)
            .await
    }

    /// Replace a named pasteboard with items and optional source metadata.
    ///
    /// CoreDevice accepts `null` for the common case. Callers that need to
    /// preserve an originating application or other metadata can pass an XPC
    /// dictionary here; it is emitted as-is under `sourceMetadata`.
    pub async fn set_named_with_source_metadata(
        &mut self,
        pasteboard_name: &str,
        items: &[PasteboardItem],
        source_metadata: Option<XpcValue>,
    ) -> Result<XpcValue, PasteboardError> {
        self.send_receive(build_set_request(pasteboard_name, items, source_metadata))
            .await
    }

    async fn send_receive(&mut self, request: XpcValue) -> Result<XpcValue, PasteboardError> {
        let timeout = self.timeout;
        let result = {
            let client = self.client.as_mut().ok_or(PasteboardError::Closed)?;
            let responses = client.stream_invoke(request);
            futures_util::pin_mut!(responses);
            tokio::time::timeout(timeout, receive_reply(responses)).await
        };

        match result {
            Ok(result) => result,
            Err(_) => {
                // Dropping XpcClient drops the underlying stream. Do this after
                // the timed future has been dropped so no reader can race it.
                self.client.take();
                Err(PasteboardError::Timeout {
                    seconds: timeout.as_secs(),
                })
            }
        }
    }
}

/// Build the exact direct-dictionary PULL request used by CoreDevice.
pub fn build_pull_request(pasteboard_name: &str) -> XpcValue {
    XpcValue::Dictionary(IndexMap::from([
        (
            "command".to_string(),
            XpcValue::String(PULL_COMMAND.to_string()),
        ),
        (
            "pasteboardName".to_string(),
            XpcValue::String(pasteboard_name.to_string()),
        ),
        (
            "dataPolicy".to_string(),
            XpcValue::Dictionary(IndexMap::from([(
                "allResolved".to_string(),
                XpcValue::Dictionary(IndexMap::new()),
            )])),
        ),
    ]))
}

/// Build the exact direct-dictionary SET request used by CoreDevice.
pub fn build_set_request(
    pasteboard_name: &str,
    items: &[PasteboardItem],
    source_metadata: Option<XpcValue>,
) -> XpcValue {
    let wire_items = items
        .iter()
        .map(|item| {
            let types = XpcValue::Array(
                item.types
                    .iter()
                    .map(|uti| XpcValue::String(uti.clone()))
                    .collect(),
            );
            let data = XpcValue::Dictionary(
                item.data
                    .iter()
                    .map(|(uti, bytes)| {
                        (
                            uti.clone(),
                            XpcValue::Dictionary(IndexMap::from([(
                                "data".to_string(),
                                XpcValue::Data(bytes.clone()),
                            )])),
                        )
                    })
                    .collect(),
            );
            XpcValue::Dictionary(IndexMap::from([
                ("types".to_string(), types),
                ("data".to_string(), data),
            ]))
        })
        .collect();

    XpcValue::Dictionary(IndexMap::from([
        (
            "command".to_string(),
            XpcValue::String(SET_COMMAND.to_string()),
        ),
        (
            "pasteboardName".to_string(),
            XpcValue::String(pasteboard_name.to_string()),
        ),
        ("items".to_string(), XpcValue::Array(wire_items)),
        (
            "sourceMetadata".to_string(),
            source_metadata.unwrap_or(XpcValue::Null),
        ),
    ]))
}

/// Extract the first non-empty reply from a stream of XPC messages.
///
/// RemoteXPC may put an empty acknowledgement/control message ahead of the
/// actual pasteboard reply. This helper intentionally ignores both a body-less
/// message and an empty dictionary, matching go-ios/pymobiledevice3 behavior.
async fn receive_reply<S>(mut responses: S) -> Result<XpcValue, PasteboardError>
where
    S: Stream<Item = Result<XpcMessage, XpcError>> + Unpin,
{
    while let Some(message) = responses.next().await {
        let message = message?;
        let Some(body) = message.body else {
            continue;
        };
        if matches!(&body, XpcValue::Dictionary(dict) if dict.is_empty()) {
            continue;
        }
        return parse_reply(body);
    }

    Err(PasteboardError::Protocol(
        "pasteboard service closed before sending a populated reply".into(),
    ))
}

fn parse_reply(body: XpcValue) -> Result<XpcValue, PasteboardError> {
    let dictionary = body.as_dict().ok_or_else(|| {
        PasteboardError::Protocol(format!("pasteboard reply is not a dictionary: {body:?}"))
    })?;
    if let Some(error) = dictionary.get("error") {
        return Err(parse_service_error(error));
    }
    Ok(body)
}

fn parse_service_error(error: &XpcValue) -> PasteboardError {
    let Some(error) = error.as_dict() else {
        return PasteboardError::Service {
            domain: "unknown".into(),
            code: "unknown".into(),
            description: format!("malformed error value: {error:?}"),
        };
    };

    let domain = error
        .get("domain")
        .and_then(XpcValue::as_str)
        .unwrap_or("unknown")
        .to_string();
    let code = error
        .get("code")
        .map(format_scalar)
        .unwrap_or_else(|| "unknown".into());
    let description = error
        .get("userInfo")
        .and_then(XpcValue::as_dict)
        .and_then(|user_info| {
            user_info
                .get("NSLocalizedDescription")
                .or_else(|| user_info.get("NSDebugDescription"))
        })
        .and_then(XpcValue::as_str)
        .unwrap_or("unknown service error")
        .to_string();

    PasteboardError::Service {
        domain,
        code,
        description,
    }
}

fn format_scalar(value: &XpcValue) -> String {
    match value {
        XpcValue::String(value) => value.clone(),
        XpcValue::Int64(value) => value.to_string(),
        XpcValue::Uint64(value) => value.to_string(),
        XpcValue::Bool(value) => value.to_string(),
        XpcValue::Null => "null".into(),
        other => format!("{other:?}"),
    }
}

/// Extract text from a PULL/SET reply snapshot.
pub fn snapshot_text(reply: &XpcValue) -> Option<String> {
    snapshot_uti_texts(reply, &TEXT_UTIS)
}

/// Extract a UTF-8 value for one UTI from a PULL/SET reply snapshot.
pub fn snapshot_uti_text(reply: &XpcValue, uti: &str) -> Option<String> {
    snapshot_uti_texts(reply, &[uti])
}

fn snapshot_uti_texts(reply: &XpcValue, utis: &[&str]) -> Option<String> {
    let snapshot = snapshot_dictionary(reply)?;
    let items = as_array(snapshot.get("items")?)?;
    for item in items {
        let Some(item) = item.as_dict() else {
            continue;
        };
        let Some(data) = item.get("data").and_then(XpcValue::as_dict) else {
            continue;
        };
        for uti in utis {
            let Some(datum) = data.get(*uti).and_then(XpcValue::as_dict) else {
                continue;
            };
            let Some(bytes) = datum.get("data").and_then(|value| match value {
                XpcValue::Data(bytes) => Some(bytes),
                _ => None,
            }) else {
                continue;
            };
            // Match the reference clients: an empty immediate value is not
            // considered a decodable text payload, while SET still preserves it.
            if bytes.is_empty() {
                continue;
            }
            let Ok(value) = std::str::from_utf8(bytes.as_ref()) else {
                continue;
            };
            return Some(value.to_string());
        }
    }
    None
}

fn as_array(value: &XpcValue) -> Option<&[XpcValue]> {
    match value {
        XpcValue::Array(values) => Some(values),
        _ => None,
    }
}

fn snapshot_dictionary(reply: &XpcValue) -> Option<&IndexMap<String, XpcValue>> {
    let dictionary = reply.as_dict()?;
    dictionary
        .get("pasteboard")
        .and_then(XpcValue::as_dict)
        .or(Some(dictionary))
}

#[cfg(test)]
mod tests {
    use bytes::Bytes;
    use futures_util::stream;

    use super::*;

    fn dict(value: &XpcValue) -> &IndexMap<String, XpcValue> {
        value.as_dict().expect("expected dictionary")
    }

    fn message(body: Option<XpcValue>) -> Result<XpcMessage, XpcError> {
        Ok(XpcMessage {
            flags: 0,
            msg_id: 1,
            body,
        })
    }

    #[test]
    fn pull_request_matches_direct_protocol_shape() {
        let request = build_pull_request(GENERAL_PASTEBOARD);
        let request = dict(&request);
        assert_eq!(request["command"].as_str(), Some(PULL_COMMAND));
        assert_eq!(request["pasteboardName"].as_str(), Some(GENERAL_PASTEBOARD));
        assert_eq!(
            dict(&request["dataPolicy"])["allResolved"],
            XpcValue::Dictionary(IndexMap::new())
        );
        assert!(!request.contains_key("CoreDevice.featureIdentifier"));
        assert!(!request.contains_key("CoreDevice.input"));
    }

    #[test]
    fn text_set_request_preserves_unicode_and_source_metadata_null() {
        let request = build_set_request(
            GENERAL_PASTEBOARD,
            &[PasteboardItem::text("こんにちは 👋 café")],
            None,
        );
        let request = dict(&request);
        assert_eq!(request["command"].as_str(), Some(SET_COMMAND));
        assert_eq!(request["sourceMetadata"], XpcValue::Null);

        let item = &as_array(&request["items"]).unwrap()[0];
        let item = dict(item);
        assert_eq!(
            item["types"],
            XpcValue::Array(
                TEXT_UTIS
                    .iter()
                    .map(|uti| XpcValue::String((*uti).into()))
                    .collect()
            )
        );
        let expected = Bytes::from("こんにちは 👋 café".as_bytes().to_vec());
        let data = dict(&item["data"]);
        for uti in TEXT_UTIS {
            assert_eq!(dict(&data[uti])["data"], XpcValue::Data(expected.clone()));
        }
    }

    #[test]
    fn url_set_request_uses_url_uti_and_utf8_data() {
        let mut source_metadata = IndexMap::new();
        source_metadata.insert("app".into(), XpcValue::String("com.example.test".into()));
        let request = build_set_request(
            GENERAL_PASTEBOARD,
            &[PasteboardItem::url("https://例.example/path?q=1")],
            Some(XpcValue::Dictionary(source_metadata.clone())),
        );
        let request = dict(&request);
        assert_eq!(
            request["sourceMetadata"],
            XpcValue::Dictionary(source_metadata)
        );
        let item = dict(&as_array(&request["items"]).unwrap()[0]);
        assert_eq!(
            item["types"],
            XpcValue::Array(vec![XpcValue::String(UTI_URL.into())])
        );
        assert_eq!(
            dict(&dict(&item["data"])[UTI_URL])["data"],
            XpcValue::Data(Bytes::from(
                "https://例.example/path?q=1".as_bytes().to_vec()
            ))
        );
    }

    #[test]
    fn empty_text_is_encoded_as_zero_length_immediate_data() {
        let request = build_set_request(GENERAL_PASTEBOARD, &[PasteboardItem::text("")], None);
        let request = dict(&request);
        let item = dict(&as_array(&request["items"]).unwrap()[0]);
        let data = dict(&item["data"]);
        for uti in TEXT_UTIS {
            assert_eq!(dict(&data[uti])["data"], XpcValue::Data(Bytes::new()));
        }
    }

    #[test]
    fn snapshot_extracts_nested_unicode_text_and_url() {
        let mut url_datum = IndexMap::new();
        url_datum.insert(
            "data".into(),
            XpcValue::Data(Bytes::from("https://example.test/☃".as_bytes().to_vec())),
        );
        let mut data = IndexMap::new();
        data.insert(UTI_URL.into(), XpcValue::Dictionary(url_datum));
        let mut item = IndexMap::new();
        item.insert(
            "types".into(),
            XpcValue::Array(vec![XpcValue::String(UTI_URL.into())]),
        );
        item.insert("data".into(), XpcValue::Dictionary(data));
        let mut pasteboard = IndexMap::new();
        pasteboard.insert(
            "items".into(),
            XpcValue::Array(vec![XpcValue::Dictionary(item)]),
        );
        let reply = XpcValue::Dictionary(IndexMap::from_iter([(
            "pasteboard".into(),
            XpcValue::Dictionary(pasteboard),
        )]));
        assert_eq!(snapshot_text(&reply), None);
        assert_eq!(
            snapshot_uti_text(&reply, UTI_URL).as_deref(),
            Some("https://example.test/☃")
        );
    }

    #[tokio::test]
    async fn receive_reply_skips_empty_frames_and_empty_dictionary() {
        let responses = stream::iter([
            message(None),
            message(Some(XpcValue::Dictionary(IndexMap::new()))),
            message(Some(XpcValue::Dictionary(IndexMap::from([(
                "command".into(),
                XpcValue::String(PULL_REPLY_COMMAND.into()),
            )])))),
        ]);
        let reply = receive_reply(responses).await.unwrap();
        assert_eq!(dict(&reply)["command"].as_str(), Some(PULL_REPLY_COMMAND));
    }

    #[tokio::test]
    async fn receive_reply_surfaces_structured_service_error() {
        let mut user_info = IndexMap::new();
        user_info.insert(
            "NSLocalizedDescription".into(),
            XpcValue::String("array required here".into()),
        );
        let mut error_dict = IndexMap::new();
        error_dict.insert(
            "domain".into(),
            XpcValue::String("NSCocoaErrorDomain".into()),
        );
        error_dict.insert("code".into(), XpcValue::Int64(4864));
        error_dict.insert("userInfo".into(), XpcValue::Dictionary(user_info));
        let mut reply = IndexMap::new();
        reply.insert("error".into(), XpcValue::Dictionary(error_dict));
        let responses = stream::iter([message(Some(XpcValue::Dictionary(reply)))]);
        let error = receive_reply(responses).await.unwrap_err();
        let rendered = error.to_string();
        assert!(rendered.contains("NSCocoaErrorDomain"), "{rendered}");
        assert!(rendered.contains("4864"), "{rendered}");
        assert!(rendered.contains("array required here"), "{rendered}");
    }

    #[tokio::test]
    async fn receive_reply_reports_closed_stream() {
        let error = receive_reply(stream::empty::<Result<XpcMessage, XpcError>>())
            .await
            .unwrap_err();
        assert!(error.to_string().contains("closed"));
    }

    #[tokio::test]
    async fn receive_reply_can_be_bounded_by_total_timeout() {
        let result = tokio::time::timeout(
            Duration::from_millis(5),
            receive_reply(stream::pending::<Result<XpcMessage, XpcError>>()),
        )
        .await;
        assert!(
            result.is_err(),
            "pending reply must hit the request deadline"
        );
    }
}
