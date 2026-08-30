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

use std::collections::VecDeque;
use std::time::Duration;

use bytes::Bytes;
use futures_core::Stream;
use futures_util::StreamExt;
use indexmap::IndexMap;
use uuid::Uuid;

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
pub const DATA_COMMAND: &str = "DATA";
pub const PUSH_COMMAND: &str = "PUSH";
pub const AUTONOTIFY_COMMAND: &str = "AUTONOTIFY";
pub const RESOLVE_COMMAND: &str = "RESOLVE";

/// Standard Uniform Type Identifiers supported by the convenience helpers.
pub const UTI_UTF8_PLAIN_TEXT: &str = "public.utf8-plain-text";
pub const UTI_PLAIN_TEXT: &str = "public.plain-text";
pub const UTI_TEXT: &str = "public.text";
pub const UTI_URL: &str = "public.url";

/// The upper bound for one PULL or SET round trip.
pub const REQUEST_TIMEOUT: Duration = Duration::from_secs(10);

/// Upper bounds applied while decoding or constructing pasteboard messages.
///
/// The XPC decoder already limits the complete control body, but a pasteboard
/// snapshot can contain many independent allocations (UTIs, metadata and
/// representations). Keeping a service-local budget makes those limits
/// explicit and lets callers choose a smaller ceiling on constrained hosts.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PasteboardLimits {
    pub max_items: usize,
    pub max_representations: usize,
    pub max_data_bytes: usize,
    pub max_metadata_bytes: usize,
    pub max_events: usize,
}

impl Default for PasteboardLimits {
    fn default() -> Self {
        Self {
            max_items: 256,
            max_representations: 2048,
            max_data_bytes: 64 * 1024 * 1024,
            max_metadata_bytes: 1024 * 1024,
            max_events: 128,
        }
    }
}

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
    /// A caller supplied an invalid field or the decoded value exceeded a
    /// pasteboard-specific resource budget.
    #[error("pasteboard limit/input error: {0}")]
    Limit(String),
}

/// Controls which item representations the device includes in a PULL/PUSH
/// snapshot. The spelling and nesting match Apple's Codable
/// `PasteboardDataInclusionPolicy` representation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DataInclusionPolicy {
    /// Resolve and inline every representation.
    AllResolved,
    /// Return promises for every representation.
    AllPromised,
    /// Preserve the source application's inclusion policy.
    MatchSource,
    /// Inline the primary representation and promise secondary ones.
    PromiseSecondary,
    /// Inline representations smaller than the supplied byte threshold.
    Threshold(i64),
}

/// Short alias used by callers that prefer the service's terminology.
pub type DataPolicy = DataInclusionPolicy;
/// Name used by the corresponding CoreDevice Swift type.
pub type PasteboardDataInclusionPolicy = DataInclusionPolicy;

impl DataInclusionPolicy {
    /// Encode this policy as the exact XPC dictionary used by CoreDevice.
    pub fn to_xpc(self) -> XpcValue {
        let (key, value) = match self {
            Self::AllResolved => ("allResolved", XpcValue::Dictionary(IndexMap::new())),
            Self::AllPromised => ("allPromised", XpcValue::Dictionary(IndexMap::new())),
            Self::MatchSource => ("matchSource", XpcValue::Dictionary(IndexMap::new())),
            Self::PromiseSecondary => ("promiseSecondary", XpcValue::Dictionary(IndexMap::new())),
            Self::Threshold(bytes) => (
                "thresholdData",
                XpcValue::Dictionary(IndexMap::from([(
                    // Swift Codable's synthesized key for the unlabeled
                    // associated Int64 value is `_0` on the wire.
                    "_0".to_string(),
                    XpcValue::Int64(bytes),
                )])),
            ),
        };
        XpcValue::Dictionary(IndexMap::from([(key.to_string(), value)]))
    }

    fn validate(self) -> Result<(), PasteboardError> {
        if let Self::Threshold(bytes) = self {
            if bytes < 0 {
                return Err(PasteboardError::Limit(format!(
                    "data policy threshold must be non-negative, got {bytes}"
                )));
            }
        }
        Ok(())
    }
}

/// A parsed item representation. Immediate bytes are copied into a bounded
/// `Bytes` value; promised entries contain the device's advertised size when
/// available; an `Error` preserves a diagnostic returned by the device.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PasteboardPayload {
    Inline(Bytes),
    Promised { size: Option<i64> },
    Error(String),
}

/// One UTI representation in a parsed snapshot.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PasteboardEntry {
    pub uti: String,
    pub payload: PasteboardPayload,
}

/// A single item in a parsed PULL_REPLY or PUSH snapshot.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PasteboardSnapshotItem {
    /// Zero-based item index used by the RESOLVE request.
    pub index: usize,
    pub types: Vec<String>,
    pub data: Vec<PasteboardEntry>,
}

/// Compatibility alias for code that calls parsed entries `ParsedItem`.
pub type ParsedPasteboardItem = PasteboardSnapshotItem;

/// A parsed PULL_REPLY/PUSH snapshot, retaining metadata needed for later
/// promise resolution and clipboard synchronization.
#[derive(Debug, Clone, PartialEq)]
pub struct PasteboardSnapshot {
    pub command: Option<String>,
    pub pasteboard_name: Option<String>,
    pub change_count: Option<i64>,
    pub uuid: Option<[u8; 16]>,
    pub metadata: Option<XpcValue>,
    pub source_metadata: Option<XpcValue>,
    pub items: Vec<PasteboardSnapshotItem>,
}

/// A promised `(item index, UTI)` pair that can be fetched with RESOLVE.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PromisedItem {
    pub item_index: i64,
    pub uti: String,
    pub size: Option<i64>,
}

/// DATA response returned for a promise. A null `data` is a valid response
/// when the source pasteboard changed or the provider withdrew the promise.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PasteboardData {
    pub pasteboard_name: Option<String>,
    pub item_index: Option<i64>,
    pub uti: Option<String>,
    pub uuid: Option<[u8; 16]>,
    pub data: Option<Bytes>,
    pub error: Option<String>,
}

/// A change notification delivered by AUTONOTIFY/PUSH.
///
/// # Experimental
///
/// AUTONOTIFY/PUSH is not implemented by the pinned go-ios or
/// pymobiledevice3 reference clients.
#[derive(Debug, Clone, PartialEq)]
pub struct PasteboardPush {
    pub snapshot: PasteboardSnapshot,
}

/// Events exposed by a subscribed pasteboard session. Unknown commands are
/// rejected as protocol errors rather than silently discarded.
///
/// # Experimental
///
/// See [`PasteboardPush`] for the upstream support status.
#[derive(Debug, Clone, PartialEq)]
pub enum PasteboardEvent {
    Push(PasteboardPush),
    Data(PasteboardData),
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

impl PasteboardSnapshot {
    /// Decode a PULL_REPLY or PUSH body with the default resource budgets.
    pub fn from_xpc(reply: &XpcValue) -> Result<Self, PasteboardError> {
        Self::from_xpc_with_limits(reply, PasteboardLimits::default())
    }

    /// Decode a snapshot using caller-provided item, representation, metadata
    /// and data ceilings.
    pub fn from_xpc_with_limits(
        reply: &XpcValue,
        limits: PasteboardLimits,
    ) -> Result<Self, PasteboardError> {
        parse_snapshot(reply, limits)
    }

    /// Best-effort UTF-8 text from the first inline text representation.
    pub fn text(&self) -> Option<String> {
        for item in &self.items {
            for uti in TEXT_UTIS {
                let Some(entry) = item.data.iter().find(|entry| entry.uti == uti) else {
                    continue;
                };
                let PasteboardPayload::Inline(bytes) = &entry.payload else {
                    continue;
                };
                if let Ok(text) = std::str::from_utf8(bytes.as_ref()) {
                    return Some(text.to_owned());
                }
            }
        }
        None
    }

    /// Return the first inline representation matching `uti`.
    pub fn data_for_uti(&self, uti: &str) -> Option<&Bytes> {
        self.items
            .iter()
            .flat_map(|item| &item.data)
            .find_map(|entry| {
                (entry.uti == uti).then_some(match &entry.payload {
                    PasteboardPayload::Inline(bytes) => bytes,
                    PasteboardPayload::Promised { .. } | PasteboardPayload::Error(_) => {
                        return None
                    }
                })
            })
    }

    /// The promised entries, each addressable with one RESOLVE request.
    pub fn promised_items(&self) -> Vec<PromisedItem> {
        self.items
            .iter()
            .flat_map(|item| {
                item.data.iter().filter_map(move |entry| {
                    let PasteboardPayload::Promised { size } = entry.payload else {
                        return None;
                    };
                    Some(PromisedItem {
                        item_index: i64::try_from(item.index).ok()?,
                        uti: entry.uti.clone(),
                        size,
                    })
                })
            })
            .collect()
    }

    /// Return all inline and promised representations in item order.
    pub fn entries(&self) -> impl Iterator<Item = (usize, &PasteboardEntry)> {
        self.items
            .iter()
            .flat_map(|item| std::iter::repeat(item.index).zip(item.data.iter()))
    }
}

fn parse_snapshot(
    reply: &XpcValue,
    limits: PasteboardLimits,
) -> Result<PasteboardSnapshot, PasteboardError> {
    let root = reply.as_dict().ok_or_else(|| {
        PasteboardError::Protocol("pasteboard snapshot is not a dictionary".into())
    })?;
    let command = root
        .get("command")
        .map(|value| {
            value.as_str().map(str::to_owned).ok_or_else(|| {
                PasteboardError::Protocol("pasteboard command is not a string".into())
            })
        })
        .transpose()?;
    let snapshot = root
        .get("pasteboard")
        .and_then(XpcValue::as_dict)
        .unwrap_or(root);

    let metadata = snapshot.get("metadata").cloned();
    let source_metadata = snapshot
        .get("sourceMetadata")
        .or_else(|| root.get("sourceMetadata"))
        .and_then(|value| (!matches!(value, XpcValue::Null)).then_some(value))
        .cloned();

    let metadata_bytes = metadata
        .as_ref()
        .map(|value| estimate_value_size(value, 0))
        .transpose()?
        .unwrap_or_default();
    let source_metadata_bytes = source_metadata
        .as_ref()
        .map(|value| estimate_value_size(value, 0))
        .transpose()?
        .unwrap_or_default();
    let metadata_total = metadata_bytes
        .checked_add(source_metadata_bytes)
        .ok_or_else(|| PasteboardError::Limit("metadata size overflow".into()))?;
    if metadata_total > limits.max_metadata_bytes {
        return Err(PasteboardError::Limit(format!(
            "metadata size {metadata_total} exceeds limit {}",
            limits.max_metadata_bytes
        )));
    }

    let uuid = parse_optional_uuid(snapshot, root)?;
    let pasteboard_name = snapshot
        .get("pasteboardName")
        .or_else(|| {
            metadata
                .as_ref()
                .and_then(XpcValue::as_dict)
                .and_then(|dict| dict.get("pasteboardName"))
        })
        .map(|value| {
            value
                .as_str()
                .map(str::to_owned)
                .ok_or_else(|| PasteboardError::Protocol("pasteboardName is not a string".into()))
        })
        .transpose()?;
    let change_count = metadata
        .as_ref()
        .and_then(XpcValue::as_dict)
        .and_then(|dict| dict.get("changeCount"))
        .map(parse_i64)
        .transpose()?;

    let item_values = match snapshot.get("items") {
        None | Some(XpcValue::Null) => &[][..],
        Some(XpcValue::Array(items)) => items.as_slice(),
        Some(_) => {
            return Err(PasteboardError::Protocol(
                "pasteboard items is not an array".into(),
            ));
        }
    };
    if item_values.len() > limits.max_items {
        return Err(PasteboardError::Limit(format!(
            "item count {} exceeds limit {}",
            item_values.len(),
            limits.max_items
        )));
    }

    let mut total_representations = 0usize;
    let mut total_data_bytes = 0usize;
    let mut items = Vec::with_capacity(item_values.len());
    for (index, value) in item_values.iter().enumerate() {
        let item = value.as_dict().ok_or_else(|| {
            PasteboardError::Protocol(format!("pasteboard item {index} is not a dictionary"))
        })?;
        let types = parse_string_array(item.get("types"), "item types")?;
        let empty_data = IndexMap::new();
        let data_map = match item.get("data") {
            None | Some(XpcValue::Null) => &empty_data,
            Some(XpcValue::Dictionary(data)) => data,
            Some(_) => {
                return Err(PasteboardError::Protocol(format!(
                    "pasteboard item {index} data is not a dictionary"
                )));
            }
        };
        total_representations = total_representations
            .checked_add(data_map.len())
            .and_then(|count| count.checked_add(types.len()))
            .ok_or_else(|| PasteboardError::Limit("representation count overflow".into()))?;
        if total_representations > limits.max_representations {
            return Err(PasteboardError::Limit(format!(
                "representation count {total_representations} exceeds limit {}",
                limits.max_representations
            )));
        }
        let mut data = Vec::with_capacity(data_map.len());
        for (uti, datum) in data_map {
            if uti.is_empty() {
                return Err(PasteboardError::Protocol(format!(
                    "pasteboard item {index} contains an empty UTI"
                )));
            }
            let payload = parse_payload(datum, index, uti, &mut total_data_bytes, limits)?;
            data.push(PasteboardEntry {
                uti: uti.clone(),
                payload,
            });
        }
        items.push(PasteboardSnapshotItem { index, types, data });
    }

    Ok(PasteboardSnapshot {
        command,
        pasteboard_name,
        change_count,
        uuid,
        metadata,
        source_metadata,
        items,
    })
}

fn parse_string_array(
    value: Option<&XpcValue>,
    field: &str,
) -> Result<Vec<String>, PasteboardError> {
    let Some(value) = value else {
        return Ok(Vec::new());
    };
    let XpcValue::Array(values) = value else {
        return Err(PasteboardError::Protocol(format!(
            "{field} is not an array"
        )));
    };
    values
        .iter()
        .enumerate()
        .map(|(index, value)| {
            value.as_str().map(str::to_owned).ok_or_else(|| {
                PasteboardError::Protocol(format!("{field}[{index}] is not a string"))
            })
        })
        .collect()
}

fn parse_payload(
    value: &XpcValue,
    item_index: usize,
    uti: &str,
    total_data_bytes: &mut usize,
    limits: PasteboardLimits,
) -> Result<PasteboardPayload, PasteboardError> {
    let dict = value.as_dict().ok_or_else(|| {
        PasteboardError::Protocol(format!(
            "payload for item {item_index} UTI {uti:?} is not a dictionary"
        ))
    })?;
    if let Some(data) = dict.get("data") {
        let XpcValue::Data(data) = data else {
            return Err(PasteboardError::Protocol(format!(
                "payload for item {item_index} UTI {uti:?} has non-data `data`"
            )));
        };
        *total_data_bytes = total_data_bytes
            .checked_add(data.len())
            .ok_or_else(|| PasteboardError::Limit("pasteboard data size overflow".into()))?;
        if *total_data_bytes > limits.max_data_bytes {
            return Err(PasteboardError::Limit(format!(
                "inline data size {} exceeds limit {}",
                *total_data_bytes, limits.max_data_bytes
            )));
        }
        return Ok(PasteboardPayload::Inline(data.clone()));
    }
    if let Some(error) = dict.get("error") {
        return Ok(PasteboardPayload::Error(format_scalar_bounded(error)));
    }
    let is_promised = dict
        .get("isPromised")
        .map(|value| {
            bool_value(value).ok_or_else(|| {
                PasteboardError::Protocol(format!(
                    "promise marker for item {item_index} UTI {uti:?} is not a boolean"
                ))
            })
        })
        .transpose()?;
    let is_available = dict
        .get("isAvailable")
        .map(|value| {
            bool_value(value).ok_or_else(|| {
                PasteboardError::Protocol(format!(
                    "availability marker for item {item_index} UTI {uti:?} is not a boolean"
                ))
            })
        })
        .transpose()?;
    if is_promised == Some(false) {
        return Err(PasteboardError::Protocol(format!(
            "item {item_index} UTI {uti:?} is marked non-promised but has no data"
        )));
    }
    if is_available == Some(true) {
        return Err(PasteboardError::Protocol(format!(
            "item {item_index} UTI {uti:?} is marked available but has no data"
        )));
    }
    let size = dict.get("size").map(parse_i64).transpose()?;
    if size.is_none() && is_promised != Some(true) && is_available != Some(false) {
        return Err(PasteboardError::Protocol(format!(
            "payload for item {item_index} UTI {uti:?} is neither inline data nor a promise"
        )));
    }
    if let Some(size) = size {
        if size < 0 {
            return Err(PasteboardError::Protocol(format!(
                "promised size for item {item_index} UTI {uti:?} is negative: {size}"
            )));
        }
        if usize::try_from(size)
            .ok()
            .is_some_and(|size| size > limits.max_data_bytes)
        {
            return Err(PasteboardError::Limit(format!(
                "promised size {size} for item {item_index} UTI {uti:?} exceeds limit {}",
                limits.max_data_bytes
            )));
        }
    }
    Ok(PasteboardPayload::Promised { size })
}

fn parse_i64(value: &XpcValue) -> Result<i64, PasteboardError> {
    match value {
        XpcValue::Int64(value) => Ok(*value),
        XpcValue::Uint64(value) => i64::try_from(*value)
            .map_err(|_| PasteboardError::Protocol(format!("integer {value} does not fit in i64"))),
        other => Err(PasteboardError::Protocol(format!(
            "expected signed integer, got {}",
            xpc_value_kind(other)
        ))),
    }
}

fn bool_value(value: &XpcValue) -> Option<bool> {
    match value {
        XpcValue::Bool(value) => Some(*value),
        _ => None,
    }
}

fn parse_optional_uuid(
    snapshot: &IndexMap<String, XpcValue>,
    root: &IndexMap<String, XpcValue>,
) -> Result<Option<[u8; 16]>, PasteboardError> {
    for key in ["UUID", "uuid", "pasteboardUUID"] {
        if let Some(value) = snapshot.get(key).or_else(|| root.get(key)) {
            return parse_uuid(value).map(Some);
        }
    }
    Ok(None)
}

fn parse_uuid(value: &XpcValue) -> Result<[u8; 16], PasteboardError> {
    match value {
        XpcValue::Uuid(bytes) => Ok(*bytes),
        XpcValue::String(value) => {
            Uuid::parse_str(value)
                .map(|uuid| *uuid.as_bytes())
                .map_err(|_| {
                    PasteboardError::Protocol(format!(
                        "invalid pasteboard UUID {:?}",
                        bounded_text(value)
                    ))
                })
        }
        other => Err(PasteboardError::Protocol(format!(
            "pasteboard UUID has unexpected type {}",
            xpc_value_kind(other)
        ))),
    }
}

fn estimate_value_size(value: &XpcValue, depth: usize) -> Result<usize, PasteboardError> {
    const MAX_DEPTH: usize = 64;
    if depth > MAX_DEPTH {
        return Err(PasteboardError::Limit(format!(
            "metadata nesting exceeds {MAX_DEPTH} levels"
        )));
    }
    let size = match value {
        XpcValue::Null
        | XpcValue::Bool(_)
        | XpcValue::Int64(_)
        | XpcValue::Uint64(_)
        | XpcValue::Double(_)
        | XpcValue::Date(_) => 16,
        XpcValue::Data(bytes) => bytes.len(),
        XpcValue::String(value) => value.len(),
        XpcValue::Uuid(_) => 16,
        XpcValue::Array(values) => {
            let mut total = values.len();
            for value in values {
                total = total
                    .checked_add(estimate_value_size(value, depth + 1)?)
                    .ok_or_else(|| PasteboardError::Limit("metadata size overflow".into()))?;
            }
            total
        }
        XpcValue::Dictionary(values) => {
            let mut total = values.len();
            for (key, value) in values {
                total = total
                    .checked_add(key.len())
                    .ok_or_else(|| PasteboardError::Limit("metadata size overflow".into()))?;
                total = total
                    .checked_add(estimate_value_size(value, depth + 1)?)
                    .ok_or_else(|| PasteboardError::Limit("metadata size overflow".into()))?;
            }
            total
        }
        XpcValue::FileTransfer { data, .. } => estimate_value_size(data, depth + 1)?,
    };
    Ok(size)
}

fn xpc_value_kind(value: &XpcValue) -> &'static str {
    match value {
        XpcValue::Null => "null",
        XpcValue::Bool(_) => "bool",
        XpcValue::Int64(_) => "int64",
        XpcValue::Uint64(_) => "uint64",
        XpcValue::Double(_) => "double",
        XpcValue::Date(_) => "date",
        XpcValue::Data(_) => "data",
        XpcValue::String(_) => "string",
        XpcValue::Uuid(_) => "uuid",
        XpcValue::Array(_) => "array",
        XpcValue::Dictionary(_) => "dictionary",
        XpcValue::FileTransfer { .. } => "file-transfer",
    }
}

fn bounded_text(value: &str) -> String {
    const MAX_DIAGNOSTIC_BYTES: usize = 512;
    if value.len() <= MAX_DIAGNOSTIC_BYTES {
        return value.to_owned();
    }
    let mut end = MAX_DIAGNOSTIC_BYTES;
    while end > 0 && !value.is_char_boundary(end) {
        end -= 1;
    }
    let mut rendered = value[..end].to_owned();
    rendered.push('…');
    rendered
}

fn format_scalar_bounded(value: &XpcValue) -> String {
    let rendered = match value {
        XpcValue::String(value) => value.clone(),
        XpcValue::Int64(value) => value.to_string(),
        XpcValue::Uint64(value) => value.to_string(),
        XpcValue::Bool(value) => value.to_string(),
        XpcValue::Double(value) => value.to_string(),
        XpcValue::Date(value) => value.to_string(),
        XpcValue::Uuid(value) => Uuid::from_bytes(*value).to_string(),
        XpcValue::Null => "null".into(),
        XpcValue::Data(value) => format!("<data: {} bytes>", value.len()),
        XpcValue::Array(values) => format!("<array: {} values>", values.len()),
        XpcValue::Dictionary(values) => format!("<dictionary: {} entries>", values.len()),
        XpcValue::FileTransfer { msg_id, .. } => format!("<file-transfer: {msg_id}>"),
    };
    bounded_text(&rendered)
}

/// A write-side item with any number of UTI representations. This additive
/// type lets callers send binary/multi-item pasteboards without changing the
/// original [`PasteboardItem`] convenience struct.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PasteboardWriteItem {
    pub types: Vec<String>,
    pub data: IndexMap<String, Bytes>,
}

impl PasteboardWriteItem {
    pub fn new(types: Vec<String>, data: IndexMap<String, Bytes>) -> Self {
        Self { types, data }
    }

    pub fn data(uti: impl Into<String>, data: impl AsRef<[u8]>) -> Self {
        let uti = uti.into();
        Self {
            types: vec![uti.clone()],
            data: IndexMap::from([(uti, Bytes::copy_from_slice(data.as_ref()))]),
        }
    }

    pub fn text(text: impl AsRef<str>, utis: &[&str]) -> Self {
        let bytes = Bytes::copy_from_slice(text.as_ref().as_bytes());
        let types = utis.iter().map(|uti| (*uti).to_owned()).collect();
        let data = utis
            .iter()
            .map(|uti| ((*uti).to_owned(), bytes.clone()))
            .collect();
        Self { types, data }
    }
}

impl From<&PasteboardItem> for PasteboardWriteItem {
    fn from(item: &PasteboardItem) -> Self {
        Self {
            types: item.types.clone(),
            data: item.data.clone(),
        }
    }
}

impl From<PasteboardItem> for PasteboardWriteItem {
    fn from(item: PasteboardItem) -> Self {
        Self {
            types: item.types,
            data: item.data,
        }
    }
}

/// A client for `com.apple.coredevice.pasteboardservice`.
pub struct PasteboardClient {
    // Keeping this optional lets us drop the underlying TCP stream when a
    // read times out. A RemoteXPC read can be half-way through a frame, so a
    // timed-out connection must not be reused for a later request.
    client: Option<XpcClient>,
    timeout: Duration,
    limits: PasteboardLimits,
}

/// Compatibility alias following pymobiledevice3's service naming.
pub type PasteboardService = PasteboardClient;

impl PasteboardClient {
    /// Wrap an initialized XPC client connected to [`SERVICE_NAME`].
    pub fn new(client: XpcClient) -> Self {
        Self {
            client: Some(client),
            timeout: REQUEST_TIMEOUT,
            limits: PasteboardLimits::default(),
        }
    }

    /// Construct a client with a custom timeout, primarily useful for tests.
    pub fn with_timeout(client: XpcClient, timeout: Duration) -> Self {
        Self {
            client: Some(client),
            timeout,
            limits: PasteboardLimits::default(),
        }
    }

    /// Construct a client with both a request deadline and pasteboard memory
    /// limits. The timeout applies to the whole request, including a reply
    /// split across multiple XPC/H2 frames.
    pub fn with_timeout_and_limits(
        client: XpcClient,
        timeout: Duration,
        limits: PasteboardLimits,
    ) -> Self {
        Self {
            client: Some(client),
            timeout,
            limits,
        }
    }

    /// Replace the default parser/resource limits for this client.
    pub fn set_limits(&mut self, limits: PasteboardLimits) {
        self.limits = limits;
    }

    pub fn limits(&self) -> PasteboardLimits {
        self.limits
    }

    /// Pull the named pasteboard and return its raw XPC reply dictionary.
    pub async fn get(&mut self) -> Result<XpcValue, PasteboardError> {
        self.get_named(GENERAL_PASTEBOARD).await
    }

    /// Pull a named pasteboard and return its raw XPC reply dictionary.
    pub async fn get_named(&mut self, pasteboard_name: &str) -> Result<XpcValue, PasteboardError> {
        self.get_named_with_policy(pasteboard_name, DataInclusionPolicy::AllResolved)
            .await
    }

    /// Pull a named pasteboard and return its raw XPC reply dictionary while
    /// selecting the device's data inclusion policy.  This is the raw
    /// counterpart to [`Self::get_with_policy`], useful to callers that need
    /// to preserve fields unknown to this crate.
    pub async fn get_named_with_policy(
        &mut self,
        pasteboard_name: &str,
        policy: DataInclusionPolicy,
    ) -> Result<XpcValue, PasteboardError> {
        policy.validate()?;
        let reply = self
            .send_receive(build_pull_request_with_policy(pasteboard_name, policy))
            .await?;
        validate_reply_command(&reply, PULL_REPLY_COMMAND)?;
        Ok(reply)
    }

    /// Pull a named pasteboard with an explicit data-inclusion policy and
    /// parse its bounded snapshot.
    pub async fn get_with_policy(
        &mut self,
        pasteboard_name: &str,
        policy: DataInclusionPolicy,
    ) -> Result<PasteboardSnapshot, PasteboardError> {
        policy.validate()?;
        let reply = self
            .send_receive(build_pull_request_with_policy(pasteboard_name, policy))
            .await?;
        PasteboardSnapshot::from_xpc_with_limits(&reply, self.limits)
    }

    /// Typed alias for [`Self::get_with_policy`].
    pub async fn get_snapshot(
        &mut self,
        pasteboard_name: &str,
        policy: DataInclusionPolicy,
    ) -> Result<PasteboardSnapshot, PasteboardError> {
        self.get_with_policy(pasteboard_name, policy).await
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
        let reply = self
            .send_receive(try_build_set_request(
                pasteboard_name,
                items,
                source_metadata,
                self.limits,
            )?)
            .await?;
        validate_reply_command(&reply, SET_REPLY_COMMAND)?;
        Ok(reply)
    }

    /// Replace a named pasteboard with arbitrary multi-representation items.
    pub async fn set_items(
        &mut self,
        pasteboard_name: &str,
        items: &[PasteboardWriteItem],
        source_metadata: Option<XpcValue>,
    ) -> Result<XpcValue, PasteboardError> {
        let reply = self
            .send_receive(try_build_set_request_for_write_items(
                pasteboard_name,
                items,
                source_metadata,
                self.limits,
            )?)
            .await?;
        validate_reply_command(&reply, SET_REPLY_COMMAND)?;
        Ok(reply)
    }

    /// Resolve one promised `(item index, UTI)` pair and return only its
    /// inline bytes. A valid DATA response may carry a null `data` field.
    ///
    /// # Experimental
    ///
    /// The verb is listed by CoreDevice, but is not implemented by the
    /// go-ios or pymobiledevice3 reference clients at the pinned revisions.
    pub async fn resolve(
        &mut self,
        pasteboard_name: &str,
        item_index: i64,
        uti: &str,
    ) -> Result<Option<Bytes>, PasteboardError> {
        Ok(self
            .resolve_data(pasteboard_name, item_index, uti)
            .await?
            .data)
    }

    /// Resolve a promise using the same `(item index, UTI)` identifiers that
    /// appeared in a parsed snapshot.
    ///
    /// # Experimental
    ///
    /// See [`Self::resolve`] for the upstream support status.
    pub async fn resolve_promise(
        &mut self,
        pasteboard_name: &str,
        promise: &PromisedItem,
    ) -> Result<PasteboardData, PasteboardError> {
        self.resolve_data(pasteboard_name, promise.item_index, &promise.uti)
            .await
    }

    /// Resolve a promise selected from `snapshot`, checking the snapshot
    /// identity when the service includes a UUID in its DATA response. Some
    /// device builds omit that optional field, so those replies retain the
    /// older index/UTI-only compatibility behavior.
    ///
    /// # Experimental
    ///
    /// See [`Self::resolve`] for the upstream support status.
    pub async fn resolve_data_for_snapshot(
        &mut self,
        pasteboard_name: &str,
        snapshot: &PasteboardSnapshot,
        item_index: i64,
        uti: &str,
    ) -> Result<PasteboardData, PasteboardError> {
        let data = self.resolve_data(pasteboard_name, item_index, uti).await?;
        validate_snapshot_uuid(snapshot.uuid, &data)?;
        Ok(data)
    }

    /// Snapshot-aware variant of [`Self::resolve_promise`].
    ///
    /// # Experimental
    ///
    /// See [`Self::resolve`] for the upstream support status.
    pub async fn resolve_promise_for_snapshot(
        &mut self,
        pasteboard_name: &str,
        snapshot: &PasteboardSnapshot,
        promise: &PromisedItem,
    ) -> Result<PasteboardData, PasteboardError> {
        self.resolve_data_for_snapshot(pasteboard_name, snapshot, promise.item_index, &promise.uti)
            .await
    }

    /// Return the full DATA response, including its optional error/metadata.
    ///
    /// # Experimental
    ///
    /// See [`Self::resolve`] for the upstream support status.
    pub async fn resolve_data(
        &mut self,
        pasteboard_name: &str,
        item_index: i64,
        uti: &str,
    ) -> Result<PasteboardData, PasteboardError> {
        validate_resolve_arguments(pasteboard_name, item_index, uti)?;
        let reply = self
            .send_receive(build_resolve_request(pasteboard_name, item_index, uti))
            .await?;
        let data = parse_data_reply(&reply, self.limits)?;
        validate_data_identity(&data, pasteboard_name, item_index, uti)?;
        Ok(data)
    }

    /// Subscribe to pasteboard changes. The returned session owns this XPC
    /// connection, so a timed-out or cancelled subscription cannot be reused
    /// for a later request.
    ///
    /// # Experimental
    ///
    /// AUTONOTIFY/PUSH is listed by CoreDevice, but is not implemented by the
    /// go-ios or pymobiledevice3 reference clients at the pinned revisions.
    pub async fn subscribe(
        &mut self,
        pasteboard_name: &str,
        policy: Option<DataInclusionPolicy>,
    ) -> Result<PasteboardSubscription, PasteboardError> {
        if let Some(policy) = policy {
            policy.validate()?;
        }
        let mut client = self.client.take().ok_or(PasteboardError::Closed)?;
        let timeout = self.timeout;
        let request = build_autonotify_request(true, pasteboard_name, policy);
        let result = tokio::time::timeout(timeout, client.send(request)).await;
        match result {
            Ok(Ok(())) => Ok(PasteboardSubscription {
                client: Some(client),
                pasteboard_name: pasteboard_name.to_owned(),
                timeout,
                limits: self.limits,
                closed: false,
                pending: VecDeque::new(),
            }),
            Ok(Err(error)) => Err(PasteboardError::Xpc(error)),
            Err(_) => Err(PasteboardError::Timeout {
                seconds: timeout.as_secs(),
            }),
        }
    }

    /// Alias matching the service's change-notification terminology.
    ///
    /// # Experimental
    ///
    /// See [`Self::subscribe`] for the upstream support status.
    pub async fn listen_for_changes(
        &mut self,
        pasteboard_name: &str,
        policy: Option<DataInclusionPolicy>,
    ) -> Result<PasteboardSubscription, PasteboardError> {
        self.subscribe(pasteboard_name, policy).await
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
    build_pull_request_with_policy(pasteboard_name, DataInclusionPolicy::AllResolved)
}

/// Build a PULL request with an explicit data-inclusion policy.
pub fn build_pull_request_with_policy(
    pasteboard_name: &str,
    policy: DataInclusionPolicy,
) -> XpcValue {
    XpcValue::Dictionary(IndexMap::from([
        (
            "command".to_string(),
            XpcValue::String(PULL_COMMAND.to_string()),
        ),
        (
            "pasteboardName".to_string(),
            XpcValue::String(pasteboard_name.to_string()),
        ),
        ("dataPolicy".to_string(), policy.to_xpc()),
    ]))
}

/// Build an AUTONOTIFY request. This command is intentionally one-way: the
/// device starts delivering PUSH messages on the root XPC stream.
///
/// # Experimental
///
/// The pinned go-ios and pymobiledevice3 clients expose no AUTONOTIFY
/// implementation; callers should gate use behind an explicit opt-in.
pub fn build_autonotify_request(
    enable: bool,
    pasteboard_name: &str,
    policy: Option<DataInclusionPolicy>,
) -> XpcValue {
    XpcValue::Dictionary(IndexMap::from([
        (
            "command".to_string(),
            XpcValue::String(AUTONOTIFY_COMMAND.to_string()),
        ),
        ("enable".to_string(), XpcValue::Bool(enable)),
        (
            "pasteboardName".to_string(),
            XpcValue::String(pasteboard_name.to_string()),
        ),
        (
            "dataPolicy".to_string(),
            policy
                .map(DataInclusionPolicy::to_xpc)
                .unwrap_or(XpcValue::Null),
        ),
    ]))
}

/// Build the exact RESOLVE request used for a promised item.
///
/// # Experimental
///
/// The pinned go-ios and pymobiledevice3 clients expose no RESOLVE
/// implementation; callers should gate use behind an explicit opt-in.
pub fn build_resolve_request(pasteboard_name: &str, item_index: i64, uti: &str) -> XpcValue {
    XpcValue::Dictionary(IndexMap::from([
        (
            "command".to_string(),
            XpcValue::String(RESOLVE_COMMAND.to_string()),
        ),
        (
            "pasteboardName".to_string(),
            XpcValue::String(pasteboard_name.to_string()),
        ),
        ("itemIndex".to_string(), XpcValue::Int64(item_index)),
        ("type".to_string(), XpcValue::String(uti.to_string())),
    ]))
}

/// Build the exact direct-dictionary SET request used by CoreDevice.
pub fn build_set_request(
    pasteboard_name: &str,
    items: &[PasteboardItem],
    source_metadata: Option<XpcValue>,
) -> XpcValue {
    let write_items: Vec<PasteboardWriteItem> =
        items.iter().map(PasteboardWriteItem::from).collect();
    build_set_request_for_write_items(pasteboard_name, &write_items, source_metadata)
}

/// Build a SET request for arbitrary multi-item/multi-UTI data. The checked
/// variant is used by the client; this compatibility builder remains useful
/// for callers that only need to inspect a wire dictionary.
pub fn build_set_request_for_write_items(
    pasteboard_name: &str,
    items: &[PasteboardWriteItem],
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

fn try_build_set_request(
    pasteboard_name: &str,
    items: &[PasteboardItem],
    source_metadata: Option<XpcValue>,
    limits: PasteboardLimits,
) -> Result<XpcValue, PasteboardError> {
    let write_items: Vec<PasteboardWriteItem> =
        items.iter().map(PasteboardWriteItem::from).collect();
    try_build_set_request_for_write_items(pasteboard_name, &write_items, source_metadata, limits)
}

fn try_build_set_request_for_write_items(
    pasteboard_name: &str,
    items: &[PasteboardWriteItem],
    source_metadata: Option<XpcValue>,
    limits: PasteboardLimits,
) -> Result<XpcValue, PasteboardError> {
    if items.len() > limits.max_items {
        return Err(PasteboardError::Limit(format!(
            "item count {} exceeds limit {}",
            items.len(),
            limits.max_items
        )));
    }
    let metadata_size = source_metadata
        .as_ref()
        .filter(|value| !matches!(value, XpcValue::Null))
        .map(|value| estimate_value_size(value, 0))
        .transpose()?
        .unwrap_or_default();
    if metadata_size > limits.max_metadata_bytes {
        return Err(PasteboardError::Limit(format!(
            "source metadata size {metadata_size} exceeds limit {}",
            limits.max_metadata_bytes
        )));
    }

    let mut representations = 0usize;
    let mut data_bytes = 0usize;
    for (index, item) in items.iter().enumerate() {
        representations = representations
            .checked_add(item.types.len())
            .and_then(|count| count.checked_add(item.data.len()))
            .ok_or_else(|| PasteboardError::Limit("representation count overflow".into()))?;
        if representations > limits.max_representations {
            return Err(PasteboardError::Limit(format!(
                "representation count {representations} exceeds limit {}",
                limits.max_representations
            )));
        }
        for uti in &item.types {
            if uti.is_empty() {
                return Err(PasteboardError::Limit(format!(
                    "item {index} contains an empty UTI"
                )));
            }
        }
        for (uti, bytes) in &item.data {
            if uti.is_empty() {
                return Err(PasteboardError::Limit(format!(
                    "item {index} contains an empty data UTI"
                )));
            }
            data_bytes = data_bytes
                .checked_add(bytes.len())
                .ok_or_else(|| PasteboardError::Limit("pasteboard data size overflow".into()))?;
            if data_bytes > limits.max_data_bytes {
                return Err(PasteboardError::Limit(format!(
                    "outgoing data size {data_bytes} exceeds limit {}",
                    limits.max_data_bytes
                )));
            }
        }
    }
    Ok(build_set_request_for_write_items(
        pasteboard_name,
        items,
        source_metadata,
    ))
}

fn validate_resolve_arguments(
    pasteboard_name: &str,
    item_index: i64,
    uti: &str,
) -> Result<(), PasteboardError> {
    if pasteboard_name.is_empty() {
        return Err(PasteboardError::Limit("pasteboard name is empty".into()));
    }
    if item_index < 0 {
        return Err(PasteboardError::Limit(format!(
            "promise item index must be non-negative, got {item_index}"
        )));
    }
    if uti.is_empty() {
        return Err(PasteboardError::Limit("promise UTI is empty".into()));
    }
    Ok(())
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
        PasteboardError::Protocol(format!(
            "pasteboard reply is not a dictionary ({})",
            xpc_value_kind(&body)
        ))
    })?;
    if let Some(error) = dictionary.get("error") {
        return Err(parse_service_error(error));
    }
    Ok(body)
}

/// Keep the compatibility path for devices that omit `command`, but reject a
/// populated response belonging to a different pasteboard verb. This matters
/// because the direct service uses one shared reply stream and unsolicited or
/// stale non-empty dictionaries must never be accepted as a successful PULL
/// or SET result.
fn validate_reply_command(reply: &XpcValue, expected: &str) -> Result<(), PasteboardError> {
    let Some(command) = reply.as_dict().and_then(|dict| dict.get("command")) else {
        return Ok(());
    };
    let Some(command) = command.as_str() else {
        return Err(PasteboardError::Protocol(
            "pasteboard reply command is not a string".into(),
        ));
    };
    if command != expected {
        return Err(PasteboardError::Protocol(format!(
            "expected pasteboard command {expected:?}, got {command:?}"
        )));
    }
    Ok(())
}

fn parse_data_reply(
    reply: &XpcValue,
    limits: PasteboardLimits,
) -> Result<PasteboardData, PasteboardError> {
    let dictionary = reply
        .as_dict()
        .ok_or_else(|| PasteboardError::Protocol("DATA reply is not a dictionary".into()))?;
    if let Some(command) = dictionary.get("command") {
        if command.as_str() != Some(DATA_COMMAND) {
            return Err(PasteboardError::Protocol(format!(
                "RESOLVE returned unexpected command {:?}",
                command.as_str().unwrap_or("<non-string>")
            )));
        }
    }
    let data = match dictionary.get("data") {
        None | Some(XpcValue::Null) => None,
        Some(XpcValue::Data(data)) => {
            if data.len() > limits.max_data_bytes {
                return Err(PasteboardError::Limit(format!(
                    "resolved data size {} exceeds limit {}",
                    data.len(),
                    limits.max_data_bytes
                )));
            }
            Some(data.clone())
        }
        Some(other) => {
            return Err(PasteboardError::Protocol(format!(
                "DATA reply has unexpected data value {}",
                format_scalar_bounded(other)
            )));
        }
    };
    let item_index = dictionary.get("itemIndex").map(parse_i64).transpose()?;
    if let Some(item_index) = item_index {
        if item_index < 0 {
            return Err(PasteboardError::Protocol(format!(
                "DATA itemIndex is negative: {item_index}"
            )));
        }
    }
    let uti = dictionary
        .get("type")
        .or_else(|| dictionary.get("uti"))
        .map(|value| {
            value
                .as_str()
                .map(str::to_owned)
                .ok_or_else(|| PasteboardError::Protocol("DATA type is not a string".into()))
        })
        .transpose()?;
    let pasteboard_name = dictionary
        .get("pasteboardName")
        .map(|value| {
            value.as_str().map(str::to_owned).ok_or_else(|| {
                PasteboardError::Protocol("DATA pasteboardName is not a string".into())
            })
        })
        .transpose()?;
    let uuid = parse_optional_uuid(dictionary, dictionary)?;
    let error = dictionary.get("error").map(format_scalar_bounded);
    if data.is_none() && error.is_none() && !dictionary.contains_key("data") {
        return Err(PasteboardError::Protocol(
            "DATA reply contains neither data nor an explicit null data field".into(),
        ));
    }
    Ok(PasteboardData {
        pasteboard_name,
        item_index,
        uti,
        uuid,
        data,
        error,
    })
}

fn validate_data_identity(
    data: &PasteboardData,
    pasteboard_name: &str,
    item_index: i64,
    uti: &str,
) -> Result<(), PasteboardError> {
    if let Some(reply_name) = data.pasteboard_name.as_deref() {
        if reply_name != pasteboard_name {
            return Err(PasteboardError::Protocol(format!(
                "DATA reply is for pasteboard {reply_name:?}, requested {pasteboard_name:?}"
            )));
        }
    }
    if let Some(reply_index) = data.item_index {
        if reply_index != item_index {
            return Err(PasteboardError::Protocol(format!(
                "DATA reply is for item {reply_index}, requested {item_index}"
            )));
        }
    }
    if let Some(reply_uti) = data.uti.as_deref() {
        if reply_uti != uti {
            return Err(PasteboardError::Protocol(format!(
                "DATA reply is for UTI {reply_uti:?}, requested {uti:?}"
            )));
        }
    }
    Ok(())
}

fn validate_snapshot_uuid(
    expected: Option<[u8; 16]>,
    data: &PasteboardData,
) -> Result<(), PasteboardError> {
    if let (Some(expected), Some(actual)) = (expected, data.uuid) {
        if expected != actual {
            return Err(PasteboardError::Protocol(format!(
                "DATA reply UUID {} does not match snapshot UUID {}",
                Uuid::from_bytes(actual),
                Uuid::from_bytes(expected)
            )));
        }
    }
    Ok(())
}

fn parse_event(
    body: XpcValue,
    limits: PasteboardLimits,
) -> Result<PasteboardEvent, PasteboardError> {
    let command = body
        .as_dict()
        .and_then(|dict| dict.get("command"))
        .and_then(XpcValue::as_str)
        .ok_or_else(|| PasteboardError::Protocol("pasteboard event is missing command".into()))?;
    match command {
        PUSH_COMMAND => Ok(PasteboardEvent::Push(PasteboardPush {
            snapshot: PasteboardSnapshot::from_xpc_with_limits(&body, limits)?,
        })),
        DATA_COMMAND => Ok(PasteboardEvent::Data(parse_data_reply(&body, limits)?)),
        other => Err(PasteboardError::Protocol(format!(
            "unexpected pasteboard event command {:?}",
            bounded_text(other)
        ))),
    }
}

/// A live AUTONOTIFY session. It owns the XPC connection moved out of the
/// original client, making cancellation and timeout cleanup unambiguous.
///
/// # Experimental
///
/// AUTONOTIFY/PUSH is not implemented by the pinned go-ios or
/// pymobiledevice3 reference clients.
pub struct PasteboardSubscription {
    client: Option<XpcClient>,
    pasteboard_name: String,
    timeout: Duration,
    limits: PasteboardLimits,
    closed: bool,
    pending: VecDeque<PasteboardEvent>,
}

/// Compatibility alias for callers that call a subscription a listener.
pub type PasteboardListener = PasteboardSubscription;

impl PasteboardSubscription {
    pub fn pasteboard_name(&self) -> &str {
        &self.pasteboard_name
    }

    pub fn is_closed(&self) -> bool {
        self.closed || self.client.is_none()
    }

    async fn read_wire_event(&mut self) -> Result<PasteboardEvent, PasteboardError> {
        loop {
            let timeout = self.timeout;
            let result = {
                let client = self.client.as_mut().ok_or(PasteboardError::Closed)?;
                tokio::time::timeout(timeout, client.recv_client_server()).await
            };
            let message = match result {
                Ok(Ok(message)) => message,
                Ok(Err(error)) => {
                    self.client.take();
                    self.closed = true;
                    return Err(PasteboardError::Xpc(error));
                }
                Err(_) => {
                    self.client.take();
                    self.closed = true;
                    return Err(PasteboardError::Timeout {
                        seconds: timeout.as_secs(),
                    });
                }
            };
            let Some(body) = message.body else {
                continue;
            };
            if matches!(&body, XpcValue::Dictionary(dict) if dict.is_empty()) {
                continue;
            }
            let event = parse_event(body, self.limits);
            if event.is_err() {
                // A malformed/unknown event means the root stream can no
                // longer be safely interpreted.  Do not leave a caller with
                // a connection that might be reused at a frame boundary.
                self.client.take();
                self.closed = true;
            }
            return event;
        }
    }

    /// Read the next PUSH or DATA event from the root XPC stream. Empty
    /// bootstrap/control messages are ignored. A timeout consumes the
    /// connection because its frame boundary may be half-read.
    pub async fn next_event(&mut self) -> Result<PasteboardEvent, PasteboardError> {
        if let Some(event) = self.pending.pop_front() {
            return Ok(event);
        }
        self.read_wire_event().await
    }

    /// Read the next change notification, ignoring DATA events that belong to
    /// a concurrent/previous resolve request. DATA events are retained in a
    /// bounded queue so they are not silently lost.
    pub async fn next_push(&mut self) -> Result<PasteboardSnapshot, PasteboardError> {
        let mut deferred = VecDeque::new();
        loop {
            let event = match self.next_event().await {
                Ok(event) => event,
                Err(error) => {
                    self.restore_deferred(&mut deferred);
                    return Err(error);
                }
            };
            match event {
                PasteboardEvent::Push(push) => {
                    self.restore_deferred(&mut deferred);
                    return Ok(push.snapshot);
                }
                PasteboardEvent::Data(data) => {
                    if deferred.len() >= self.limits.max_events {
                        self.closed = true;
                        self.client.take();
                        return Err(PasteboardError::Limit(format!(
                            "pending event count reached limit {}",
                            self.limits.max_events
                        )));
                    }
                    deferred.push_back(PasteboardEvent::Data(data));
                }
            }
        }
    }

    fn restore_deferred(&mut self, deferred: &mut VecDeque<PasteboardEvent>) {
        while let Some(event) = deferred.pop_back() {
            self.pending.push_front(event);
        }
    }

    /// Return a stream view over this listener. Reading remains single-owner;
    /// dropping the stream does not spawn a task or leave an unbounded queue.
    pub fn stream(&mut self) -> impl Stream<Item = Result<PasteboardEvent, PasteboardError>> + '_ {
        async_stream::try_stream! {
            loop {
                yield self.next_event().await?;
            }
        }
    }

    /// Resolve a promise while keeping the AUTONOTIFY connection alive. Reply
    /// DATA travels on the reply stream, while PUSH notifications remain on
    /// the root stream and are consumed by [`Self::next_event`].
    ///
    /// # Experimental
    ///
    /// See [`PasteboardClient::resolve`] for the upstream support status.
    pub async fn resolve(
        &mut self,
        pasteboard_name: &str,
        item_index: i64,
        uti: &str,
    ) -> Result<PasteboardData, PasteboardError> {
        validate_resolve_arguments(pasteboard_name, item_index, uti)?;
        let timeout = self.timeout;
        let result = {
            let client = self.client.as_mut().ok_or(PasteboardError::Closed)?;
            let responses =
                client.stream_invoke(build_resolve_request(pasteboard_name, item_index, uti));
            futures_util::pin_mut!(responses);
            tokio::time::timeout(timeout, receive_reply(responses)).await
        };
        let reply = match result {
            Ok(Ok(reply)) => reply,
            Ok(Err(error)) => return Err(error),
            Err(_) => {
                self.client.take();
                self.closed = true;
                return Err(PasteboardError::Timeout {
                    seconds: timeout.as_secs(),
                });
            }
        };
        let data = parse_data_reply(&reply, self.limits)?;
        validate_data_identity(&data, pasteboard_name, item_index, uti)?;
        Ok(data)
    }

    /// # Experimental
    ///
    /// See [`PasteboardClient::resolve`] for the upstream support status.
    pub async fn resolve_promise(
        &mut self,
        promise: &PromisedItem,
    ) -> Result<PasteboardData, PasteboardError> {
        self.resolve(
            &self.pasteboard_name.clone(),
            promise.item_index,
            &promise.uti,
        )
        .await
    }

    /// Resolve a promise selected from a PUSH snapshot and validate the
    /// optional snapshot UUID returned by the service.
    ///
    /// # Experimental
    ///
    /// See [`PasteboardClient::resolve`] for the upstream support status.
    pub async fn resolve_for_snapshot(
        &mut self,
        snapshot: &PasteboardSnapshot,
        item_index: i64,
        uti: &str,
    ) -> Result<PasteboardData, PasteboardError> {
        let data = self
            .resolve(&self.pasteboard_name.clone(), item_index, uti)
            .await?;
        validate_snapshot_uuid(snapshot.uuid, &data)?;
        Ok(data)
    }

    /// Snapshot-aware variant of [`Self::resolve_promise`].
    ///
    /// # Experimental
    ///
    /// See [`PasteboardClient::resolve`] for the upstream support status.
    pub async fn resolve_promise_for_snapshot(
        &mut self,
        snapshot: &PasteboardSnapshot,
        promise: &PromisedItem,
    ) -> Result<PasteboardData, PasteboardError> {
        self.resolve_for_snapshot(snapshot, promise.item_index, &promise.uti)
            .await
    }

    /// Queue an event encountered while another operation was in progress.
    /// This is deliberately bounded; a consumer that never drains the queue
    /// receives a diagnostic error instead of allowing memory growth.
    pub fn enqueue(&mut self, event: PasteboardEvent) -> Result<(), PasteboardError> {
        if self.closed {
            return Err(PasteboardError::Closed);
        }
        if self.pending.len() >= self.limits.max_events {
            self.closed = true;
            self.client.take();
            return Err(PasteboardError::Limit(format!(
                "pending event count reached limit {}",
                self.limits.max_events
            )));
        }
        self.pending.push_back(event);
        Ok(())
    }

    /// Disable notifications. Repeated calls are idempotent.
    pub async fn unsubscribe(&mut self) -> Result<(), PasteboardError> {
        if self.closed {
            return Ok(());
        }
        let Some(mut client) = self.client.take() else {
            self.closed = true;
            return Ok(());
        };
        let timeout = self.timeout;
        let request = build_autonotify_request(false, &self.pasteboard_name, None);
        let result = tokio::time::timeout(timeout, client.send(request)).await;
        self.closed = true;
        match result {
            Ok(Ok(())) => Ok(()),
            Ok(Err(error)) => Err(PasteboardError::Xpc(error)),
            Err(_) => Err(PasteboardError::Timeout {
                seconds: timeout.as_secs(),
            }),
        }
    }

    pub async fn close(&mut self) -> Result<(), PasteboardError> {
        self.unsubscribe().await
    }
}

impl Drop for PasteboardSubscription {
    fn drop(&mut self) {
        if self.closed {
            return;
        }
        let Some(mut client) = self.client.take() else {
            return;
        };
        let request = build_autonotify_request(false, &self.pasteboard_name, None);
        let timeout = self.timeout;
        let Ok(handle) = tokio::runtime::Handle::try_current() else {
            return;
        };
        // Drop must never block. Moving the connection into a bounded task
        // gives the daemon a best-effort unsubscribe while still dropping it
        // promptly when the runtime is unavailable or shutting down.
        handle.spawn(async move {
            let _ = tokio::time::timeout(timeout, client.send(request)).await;
        });
    }
}

fn parse_service_error(error: &XpcValue) -> PasteboardError {
    let Some(error) = error.as_dict() else {
        return PasteboardError::Service {
            domain: "unknown".into(),
            code: "unknown".into(),
            description: format!("malformed error value: {}", format_scalar_bounded(error)),
        };
    };

    let domain = error
        .get("domain")
        .and_then(XpcValue::as_str)
        .map(bounded_text)
        .unwrap_or_else(|| "unknown".into());
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
        .map(bounded_text)
        .unwrap_or_else(|| "unknown service error".into());

    PasteboardError::Service {
        domain,
        code,
        description,
    }
}

fn format_scalar(value: &XpcValue) -> String {
    format_scalar_bounded(value)
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
    use tokio::io::{duplex, AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

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

    const FRAME_DATA: u8 = 0x00;
    const FRAME_HEADERS: u8 = 0x01;
    const FRAME_SETTINGS: u8 = 0x04;
    const FRAME_WINDOW_UPDATE: u8 = 0x08;
    const FLAG_SETTINGS_ACK: u8 = 0x01;
    const STREAM_INIT: u32 = 0;
    const STREAM_CLIENT_SERVER: u32 = 1;
    const STREAM_SERVER_CLIENT: u32 = 3;

    fn frame(frame_type: u8, flags: u8, stream_id: u32, payload: &[u8]) -> Vec<u8> {
        let length = payload.len();
        let mut output = Vec::with_capacity(9 + length);
        output.push(((length >> 16) & 0xff) as u8);
        output.push(((length >> 8) & 0xff) as u8);
        output.push((length & 0xff) as u8);
        output.push(frame_type);
        output.push(flags);
        output.extend_from_slice(&(stream_id & 0x7fff_ffff).to_be_bytes());
        output.extend_from_slice(payload);
        output
    }

    async fn read_frame<S: AsyncRead + Unpin>(stream: &mut S) -> (u8, u8, u32, Vec<u8>) {
        let mut header = [0u8; 9];
        stream.read_exact(&mut header).await.unwrap();
        let length =
            ((header[0] as usize) << 16) | ((header[1] as usize) << 8) | header[2] as usize;
        let mut payload = vec![0u8; length];
        if length > 0 {
            stream.read_exact(&mut payload).await.unwrap();
        }
        (
            header[3],
            header[4],
            u32::from_be_bytes([header[5], header[6], header[7], header[8]]) & 0x7fff_ffff,
            payload,
        )
    }

    fn encoded_message(flags: u32, msg_id: u64, body: Option<XpcValue>) -> Bytes {
        crate::xpc::message::encode_message(&XpcMessage {
            flags,
            msg_id,
            body,
        })
        .unwrap()
    }

    async fn finish_fake_xpc_handshake<S: AsyncRead + AsyncWrite + Unpin>(server: &mut S) {
        let mut preface = [0u8; 24];
        server.read_exact(&mut preface).await.unwrap();
        assert_eq!(&preface, crate::xpc::h2_raw::H2_PREFACE);
        let (frame_type, _, stream_id, _) = read_frame(server).await;
        assert_eq!((frame_type, stream_id), (FRAME_SETTINGS, STREAM_INIT));
        let (frame_type, _, stream_id, _) = read_frame(server).await;
        assert_eq!((frame_type, stream_id), (FRAME_WINDOW_UPDATE, STREAM_INIT));

        server
            .write_all(&frame(FRAME_SETTINGS, 0, STREAM_INIT, &[]))
            .await
            .unwrap();
        server.flush().await.unwrap();
        let (frame_type, frame_flags, stream_id, _) = read_frame(server).await;
        assert_eq!(
            (frame_type, frame_flags, stream_id),
            (FRAME_SETTINGS, FLAG_SETTINGS_ACK, STREAM_INIT)
        );

        let (frame_type, _, stream_id, _) = read_frame(server).await;
        assert_eq!(
            (frame_type, stream_id),
            (FRAME_HEADERS, STREAM_CLIENT_SERVER)
        );
        let (frame_type, _, stream_id, payload) = read_frame(server).await;
        assert_eq!((frame_type, stream_id), (FRAME_DATA, STREAM_CLIENT_SERVER));
        let first = crate::xpc::message::decode_message(Bytes::from(payload)).unwrap();
        assert_eq!(first.flags, crate::xpc::message::flags::ALWAYS_SET);
        assert!(matches!(
            first.body,
            Some(XpcValue::Dictionary(ref values)) if values.is_empty()
        ));
        server
            .write_all(&frame(
                FRAME_DATA,
                0,
                STREAM_CLIENT_SERVER,
                &encoded_message(crate::xpc::message::flags::ALWAYS_SET, 0, None),
            ))
            .await
            .unwrap();
        server.flush().await.unwrap();

        let (frame_type, _, stream_id, _) = read_frame(server).await;
        assert_eq!(
            (frame_type, stream_id),
            (FRAME_HEADERS, STREAM_SERVER_CLIENT)
        );
        let (frame_type, _, stream_id, payload) = read_frame(server).await;
        assert_eq!((frame_type, stream_id), (FRAME_DATA, STREAM_CLIENT_SERVER));
        let second = crate::xpc::message::decode_message(Bytes::from(payload)).unwrap();
        assert_eq!(second.flags, crate::xpc::message::flags::ALWAYS_SET | 0x200);
        assert!(second.body.is_none());
        server
            .write_all(&frame(
                FRAME_DATA,
                0,
                STREAM_CLIENT_SERVER,
                &encoded_message(crate::xpc::message::flags::ALWAYS_SET, 0, None),
            ))
            .await
            .unwrap();
        server.flush().await.unwrap();

        let (frame_type, _, stream_id, payload) = read_frame(server).await;
        assert_eq!((frame_type, stream_id), (FRAME_DATA, STREAM_SERVER_CLIENT));
        let third = crate::xpc::message::decode_message(Bytes::from(payload)).unwrap();
        assert_eq!(
            third.flags,
            crate::xpc::message::flags::INIT_HANDSHAKE | crate::xpc::message::flags::ALWAYS_SET
        );
        assert!(third.body.is_none());
        server
            .write_all(&frame(
                FRAME_DATA,
                0,
                STREAM_SERVER_CLIENT,
                &encoded_message(crate::xpc::message::flags::ALWAYS_SET, 0, None),
            ))
            .await
            .unwrap();
        server.flush().await.unwrap();
    }

    async fn read_xpc_on_stream<S: AsyncRead + Unpin>(
        stream: &mut S,
        wanted_stream: u32,
    ) -> XpcMessage {
        loop {
            let (frame_type, _, stream_id, payload) = read_frame(stream).await;
            if frame_type != FRAME_DATA || stream_id != wanted_stream {
                continue;
            }
            return crate::xpc::message::decode_message(Bytes::from(payload)).unwrap();
        }
    }

    fn snapshot_body(items: Vec<XpcValue>) -> XpcValue {
        XpcValue::Dictionary(IndexMap::from([(
            "pasteboard".into(),
            XpcValue::Dictionary(IndexMap::from([
                (
                    "pasteboardName".into(),
                    XpcValue::String(GENERAL_PASTEBOARD.into()),
                ),
                ("items".into(), XpcValue::Array(items)),
            ])),
        )]))
    }

    fn snapshot_item(types: &[&str], data: IndexMap<String, XpcValue>) -> XpcValue {
        XpcValue::Dictionary(IndexMap::from([
            (
                "types".into(),
                XpcValue::Array(
                    types
                        .iter()
                        .map(|value| XpcValue::String((*value).into()))
                        .collect(),
                ),
            ),
            ("data".into(), XpcValue::Dictionary(data)),
        ]))
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
    fn policies_match_swift_codable_wire_shape() {
        for (policy, key) in [
            (DataInclusionPolicy::AllResolved, "allResolved"),
            (DataInclusionPolicy::AllPromised, "allPromised"),
            (DataInclusionPolicy::MatchSource, "matchSource"),
            (DataInclusionPolicy::PromiseSecondary, "promiseSecondary"),
        ] {
            let encoded = policy.to_xpc();
            let value = dict(&encoded);
            assert_eq!(value[key], XpcValue::Dictionary(IndexMap::new()));
        }
        let encoded = DataInclusionPolicy::Threshold(4096).to_xpc();
        let value = dict(&encoded);
        let threshold = dict(&value["thresholdData"]);
        assert_eq!(threshold["_0"], XpcValue::Int64(4096));
        assert!(!threshold.contains_key("bytes"));
        assert!(DataInclusionPolicy::Threshold(-1).validate().is_err());
    }

    #[test]
    fn autonotify_and_resolve_requests_preserve_exact_fields() {
        let encoded =
            build_autonotify_request(true, "剪贴板", Some(DataInclusionPolicy::AllPromised));
        let request = dict(&encoded);
        assert_eq!(request["command"].as_str(), Some(AUTONOTIFY_COMMAND));
        assert_eq!(request["enable"], XpcValue::Bool(true));
        assert_eq!(request["pasteboardName"].as_str(), Some("剪贴板"));
        assert_eq!(
            dict(&request["dataPolicy"])["allPromised"],
            XpcValue::Dictionary(IndexMap::new())
        );

        let encoded = build_autonotify_request(false, "general", None);
        let request = dict(&encoded);
        assert_eq!(request["dataPolicy"], XpcValue::Null);

        let encoded = build_resolve_request("general", 7, "public.data");
        let request = dict(&encoded);
        assert_eq!(request["command"].as_str(), Some(RESOLVE_COMMAND));
        assert_eq!(request["itemIndex"], XpcValue::Int64(7));
        assert_eq!(request["type"].as_str(), Some("public.data"));
    }

    #[test]
    fn set_builder_supports_multiple_items_and_representations() {
        let first = PasteboardWriteItem::new(
            vec!["public.text".into(), "public.utf8-plain-text".into()],
            IndexMap::from([
                ("public.text".into(), Bytes::from_static(b"hello")),
                (
                    "public.utf8-plain-text".into(),
                    Bytes::from_static(b"hello"),
                ),
            ]),
        );
        let second = PasteboardWriteItem::data("public.data", [0, 255]);
        let encoded = build_set_request_for_write_items("general", &[first, second], None);
        let request = dict(&encoded);
        let items = as_array(&request["items"]).expect("items array");
        assert_eq!(items.len(), 2);
        assert_eq!(
            dict(&items[1])["types"],
            XpcValue::Array(vec![XpcValue::String("public.data".into())])
        );
        assert_eq!(
            dict(&dict(&items[1])["data"])["public.data"],
            XpcValue::Dictionary(IndexMap::from([(
                "data".into(),
                XpcValue::Data(Bytes::from_static(&[0, 255])),
            )]))
        );
    }

    #[test]
    fn snapshot_parser_decodes_inline_promised_error_metadata_and_uuid() {
        let mut inline = IndexMap::new();
        inline.insert("data".into(), XpcValue::Data(Bytes::new()));
        let mut promised = IndexMap::new();
        promised.insert("isPromised".into(), XpcValue::Bool(true));
        promised.insert("isAvailable".into(), XpcValue::Bool(false));
        promised.insert("size".into(), XpcValue::Int64(42));
        let mut error = IndexMap::new();
        error.insert(
            "error".into(),
            XpcValue::String("provider unavailable".into()),
        );
        let item = snapshot_item(
            &["public.text", "public.data", "public.error"],
            IndexMap::from([
                ("public.text".into(), XpcValue::Dictionary(inline)),
                ("public.data".into(), XpcValue::Dictionary(promised)),
                ("public.error".into(), XpcValue::Dictionary(error)),
            ]),
        );
        let root = XpcValue::Dictionary(IndexMap::from([
            (
                "pasteboard".into(),
                XpcValue::Dictionary(IndexMap::from([
                    (
                        "UUID".into(),
                        XpcValue::String("00112233-4455-6677-8899-aabbccddeeff".into()),
                    ),
                    (
                        "metadata".into(),
                        XpcValue::Dictionary(IndexMap::from([
                            ("pasteboardName".into(), XpcValue::String("general".into())),
                            ("changeCount".into(), XpcValue::Int64(9)),
                        ])),
                    ),
                    ("items".into(), XpcValue::Array(vec![item])),
                ])),
            ),
            (
                "sourceMetadata".into(),
                XpcValue::Dictionary(IndexMap::from([(
                    "source".into(),
                    XpcValue::String("com.example.test".into()),
                )])),
            ),
        ]));
        let snapshot = PasteboardSnapshot::from_xpc(&root).unwrap();
        assert_eq!(snapshot.items.len(), 1);
        assert_eq!(snapshot.items[0].index, 0);
        assert_eq!(snapshot.text().as_deref(), Some(""));
        assert_eq!(
            snapshot.promised_items(),
            vec![PromisedItem {
                item_index: 0,
                uti: "public.data".into(),
                size: Some(42),
            }]
        );
        assert_eq!(snapshot.change_count, Some(9));
        assert_eq!(
            snapshot.uuid,
            Some([
                0x00, 0x11, 0x22, 0x33, 0x44, 0x55, 0x66, 0x77, 0x88, 0x99, 0xaa, 0xbb, 0xcc, 0xdd,
                0xee, 0xff,
            ])
        );
        assert!(snapshot.source_metadata.is_some());
    }

    #[test]
    fn snapshot_parser_rejects_malformed_promises_and_enforces_budgets() {
        let malformed = snapshot_body(vec![snapshot_item(
            &["public.data"],
            IndexMap::from([("public.data".into(), XpcValue::Dictionary(IndexMap::new()))]),
        )]);
        assert!(matches!(
            PasteboardSnapshot::from_xpc(&malformed),
            Err(PasteboardError::Protocol(_))
        ));

        let mut too_large_payload = IndexMap::new();
        too_large_payload.insert("data".into(), XpcValue::Data(Bytes::from_static(b"12")));
        let too_large = snapshot_body(vec![snapshot_item(
            &["public.data"],
            IndexMap::from([(
                "public.data".into(),
                XpcValue::Dictionary(too_large_payload),
            )]),
        )]);
        assert!(matches!(
            PasteboardSnapshot::from_xpc_with_limits(
                &too_large,
                PasteboardLimits {
                    max_data_bytes: 1,
                    ..PasteboardLimits::default()
                }
            ),
            Err(PasteboardError::Limit(_))
        ));
        assert!(matches!(
            PasteboardSnapshot::from_xpc_with_limits(
                &too_large,
                PasteboardLimits {
                    max_items: 0,
                    ..PasteboardLimits::default()
                }
            ),
            Err(PasteboardError::Limit(_))
        ));
    }

    #[test]
    fn data_reply_accepts_zero_length_and_null_but_rejects_negative_index() {
        let reply = XpcValue::Dictionary(IndexMap::from([
            ("command".into(), XpcValue::String(DATA_COMMAND.into())),
            ("itemIndex".into(), XpcValue::Int64(0)),
            ("type".into(), XpcValue::String("public.data".into())),
            ("data".into(), XpcValue::Data(Bytes::new())),
        ]));
        let parsed = parse_data_reply(&reply, PasteboardLimits::default()).unwrap();
        assert_eq!(parsed.data, Some(Bytes::new()));
        assert!(validate_data_identity(&parsed, "general", 0, "public.data").is_ok());

        let null = XpcValue::Dictionary(IndexMap::from([("data".into(), XpcValue::Null)]));
        assert_eq!(
            parse_data_reply(&null, PasteboardLimits::default())
                .unwrap()
                .data,
            None
        );

        let negative = XpcValue::Dictionary(IndexMap::from([
            ("itemIndex".into(), XpcValue::Int64(-1)),
            ("data".into(), XpcValue::Null),
        ]));
        assert!(matches!(
            parse_data_reply(&negative, PasteboardLimits::default()),
            Err(PasteboardError::Protocol(_))
        ));
    }

    #[test]
    fn snapshot_uuid_mismatch_is_rejected_when_data_reply_provides_one() {
        let data = PasteboardData {
            pasteboard_name: None,
            item_index: Some(0),
            uti: Some(UTI_URL.into()),
            uuid: Some([0x22; 16]),
            data: Some(Bytes::from_static(b"https://example.test")),
            error: None,
        };
        assert!(matches!(
            validate_snapshot_uuid(Some([0x11; 16]), &data),
            Err(PasteboardError::Protocol(message)) if message.contains("UUID")
        ));
        assert!(validate_snapshot_uuid(Some([0x22; 16]), &data).is_ok());
        // A legacy DATA response without a UUID remains usable.
        assert!(
            validate_snapshot_uuid(Some([0x11; 16]), &PasteboardData { uuid: None, ..data })
                .is_ok()
        );
    }

    #[tokio::test]
    async fn next_push_preserves_interleaved_data_in_bounded_order() {
        let push = PasteboardSnapshot {
            command: Some(PUSH_COMMAND.into()),
            pasteboard_name: Some("general".into()),
            change_count: Some(2),
            uuid: None,
            metadata: None,
            source_metadata: None,
            items: Vec::new(),
        };
        let data = PasteboardData {
            pasteboard_name: Some("general".into()),
            item_index: Some(0),
            uti: Some("public.data".into()),
            uuid: None,
            data: Some(Bytes::from_static(b"data")),
            error: None,
        };
        let mut subscription = PasteboardSubscription {
            client: None,
            pasteboard_name: "general".into(),
            timeout: Duration::from_millis(1),
            limits: PasteboardLimits {
                max_events: 2,
                ..PasteboardLimits::default()
            },
            closed: false,
            pending: VecDeque::from([
                PasteboardEvent::Data(data.clone()),
                PasteboardEvent::Push(PasteboardPush {
                    snapshot: push.clone(),
                }),
            ]),
        };
        assert_eq!(subscription.next_push().await.unwrap(), push);
        assert_eq!(
            subscription.next_event().await.unwrap(),
            PasteboardEvent::Data(data)
        );
    }

    #[test]
    fn event_parser_rejects_unknown_command_and_queue_is_bounded() {
        let unknown = XpcValue::Dictionary(IndexMap::from([(
            "command".into(),
            XpcValue::String("FUTURE".into()),
        )]));
        assert!(matches!(
            parse_event(unknown, PasteboardLimits::default()),
            Err(PasteboardError::Protocol(_))
        ));
        let mut subscription = PasteboardSubscription {
            client: None,
            pasteboard_name: "general".into(),
            timeout: Duration::from_secs(1),
            limits: PasteboardLimits {
                max_events: 1,
                ..PasteboardLimits::default()
            },
            closed: false,
            pending: VecDeque::new(),
        };
        let event = PasteboardEvent::Data(PasteboardData {
            pasteboard_name: None,
            item_index: None,
            uti: None,
            uuid: None,
            data: None,
            error: Some("gone".into()),
        });
        subscription.enqueue(event.clone()).unwrap();
        assert!(matches!(
            subscription.enqueue(event),
            Err(PasteboardError::Limit(_))
        ));
        assert!(subscription.is_closed());
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
    async fn fake_xpc_pull_round_trip_uses_direct_policy_dictionary() {
        let (client_stream, mut server_stream) = duplex(64 * 1024);
        let server_task = tokio::spawn(async move {
            finish_fake_xpc_handshake(&mut server_stream).await;
            let request = read_xpc_on_stream(&mut server_stream, STREAM_CLIENT_SERVER).await;
            let body = request.body.expect("pasteboard request body");
            let request = dict(&body);
            assert_eq!(request["command"].as_str(), Some(PULL_COMMAND));
            assert_eq!(request["pasteboardName"].as_str(), Some("general"));
            let policy = dict(&request["dataPolicy"]);
            assert_eq!(dict(&policy["thresholdData"])["_0"], XpcValue::Int64(32));

            let reply = XpcValue::Dictionary(IndexMap::from([
                (
                    "command".into(),
                    XpcValue::String(PULL_REPLY_COMMAND.into()),
                ),
                (
                    "pasteboard".into(),
                    XpcValue::Dictionary(IndexMap::from([
                        (
                            "metadata".into(),
                            XpcValue::Dictionary(IndexMap::from([
                                ("pasteboardName".into(), XpcValue::String("general".into())),
                                ("changeCount".into(), XpcValue::Int64(3)),
                            ])),
                        ),
                        ("items".into(), XpcValue::Array(Vec::new())),
                    ])),
                ),
            ]));
            server_stream
                .write_all(&frame(
                    FRAME_DATA,
                    0,
                    STREAM_SERVER_CLIENT,
                    &encoded_message(
                        crate::xpc::message::flags::ALWAYS_SET
                            | crate::xpc::message::flags::REPLY
                            | crate::xpc::message::flags::DATA,
                        1,
                        Some(reply),
                    ),
                ))
                .await
                .unwrap();
            server_stream.flush().await.unwrap();
        });

        let xpc = tokio::time::timeout(
            Duration::from_secs(1),
            XpcClient::connect_stream(client_stream),
        )
        .await
        .expect("XPC connect timed out")
        .unwrap();
        let mut client = PasteboardClient::with_timeout(xpc, Duration::from_secs(1));
        let snapshot = client
            .get_with_policy("general", DataInclusionPolicy::Threshold(32))
            .await
            .unwrap();
        assert_eq!(snapshot.command.as_deref(), Some(PULL_REPLY_COMMAND));
        assert_eq!(snapshot.change_count, Some(3));
        server_task.await.unwrap();
    }

    #[tokio::test]
    async fn fake_xpc_resolve_round_trip_matches_item_and_uti() {
        let (client_stream, mut server_stream) = duplex(64 * 1024);
        let server_task = tokio::spawn(async move {
            finish_fake_xpc_handshake(&mut server_stream).await;
            let request = read_xpc_on_stream(&mut server_stream, STREAM_CLIENT_SERVER).await;
            let body = request.body.expect("RESOLVE request body");
            let request = dict(&body);
            assert_eq!(request["command"].as_str(), Some(RESOLVE_COMMAND));
            assert_eq!(request["pasteboardName"].as_str(), Some("general"));
            assert_eq!(request["itemIndex"], XpcValue::Int64(2));
            assert_eq!(request["type"].as_str(), Some("public.data"));

            let data = XpcValue::Dictionary(IndexMap::from([
                ("command".into(), XpcValue::String(DATA_COMMAND.into())),
                ("pasteboardName".into(), XpcValue::String("general".into())),
                ("itemIndex".into(), XpcValue::Int64(2)),
                ("type".into(), XpcValue::String("public.data".into())),
                (
                    "data".into(),
                    XpcValue::Data(Bytes::from_static(b"resolved")),
                ),
            ]));
            server_stream
                .write_all(&frame(
                    FRAME_DATA,
                    0,
                    STREAM_SERVER_CLIENT,
                    &encoded_message(
                        crate::xpc::message::flags::ALWAYS_SET
                            | crate::xpc::message::flags::REPLY
                            | crate::xpc::message::flags::DATA,
                        1,
                        Some(data),
                    ),
                ))
                .await
                .unwrap();
            server_stream.flush().await.unwrap();
        });

        let xpc = tokio::time::timeout(
            Duration::from_secs(1),
            XpcClient::connect_stream(client_stream),
        )
        .await
        .expect("XPC connect timed out")
        .unwrap();
        let mut client = PasteboardClient::with_timeout(xpc, Duration::from_secs(1));
        let result = client
            .resolve_data("general", 2, "public.data")
            .await
            .unwrap();
        assert_eq!(result.data, Some(Bytes::from_static(b"resolved")));
        server_task.await.unwrap();
    }

    #[tokio::test]
    async fn fake_xpc_autonotify_delivers_push_and_unsubscribes() {
        let (client_stream, mut server_stream) = duplex(64 * 1024);
        let server_task = tokio::spawn(async move {
            finish_fake_xpc_handshake(&mut server_stream).await;
            let request = read_xpc_on_stream(&mut server_stream, STREAM_CLIENT_SERVER).await;
            let body = request.body.expect("AUTONOTIFY request body");
            let request = dict(&body);
            assert_eq!(request["command"].as_str(), Some(AUTONOTIFY_COMMAND));
            assert_eq!(request["enable"], XpcValue::Bool(true));
            assert_eq!(request["dataPolicy"], XpcValue::Null);

            let push = XpcValue::Dictionary(IndexMap::from([
                ("command".into(), XpcValue::String(PUSH_COMMAND.into())),
                (
                    "pasteboard".into(),
                    XpcValue::Dictionary(IndexMap::from([(
                        "items".into(),
                        XpcValue::Array(Vec::new()),
                    )])),
                ),
            ]));
            server_stream
                .write_all(&frame(
                    FRAME_DATA,
                    0,
                    STREAM_CLIENT_SERVER,
                    &encoded_message(
                        crate::xpc::message::flags::ALWAYS_SET | crate::xpc::message::flags::DATA,
                        0,
                        Some(push),
                    ),
                ))
                .await
                .unwrap();
            server_stream.flush().await.unwrap();

            let request = read_xpc_on_stream(&mut server_stream, STREAM_CLIENT_SERVER).await;
            let body = request.body.expect("unsubscribe request body");
            let request = dict(&body);
            assert_eq!(request["command"].as_str(), Some(AUTONOTIFY_COMMAND));
            assert_eq!(request["enable"], XpcValue::Bool(false));
        });

        let xpc = tokio::time::timeout(
            Duration::from_secs(1),
            XpcClient::connect_stream(client_stream),
        )
        .await
        .expect("XPC connect timed out")
        .unwrap();
        let mut client = PasteboardClient::with_timeout(xpc, Duration::from_secs(1));
        let mut subscription = client.subscribe("general", None).await.unwrap();
        let event = subscription.next_event().await.unwrap();
        assert!(matches!(event, PasteboardEvent::Push(_)));
        subscription.unsubscribe().await.unwrap();
        server_task.await.unwrap();
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

    #[test]
    fn reply_command_must_match_requested_verb_when_present() {
        let reply = XpcValue::Dictionary(IndexMap::from([(
            "command".into(),
            XpcValue::String(SET_REPLY_COMMAND.into()),
        )]));
        assert!(matches!(
            validate_reply_command(&reply, PULL_REPLY_COMMAND),
            Err(PasteboardError::Protocol(message)) if message.contains("SET_REPLY")
        ));

        // Older service builds may omit the discriminator; retain that
        // compatibility while still rejecting an explicit mismatch.
        let legacy = XpcValue::Dictionary(IndexMap::new());
        assert!(validate_reply_command(&legacy, PULL_REPLY_COMMAND).is_ok());
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
