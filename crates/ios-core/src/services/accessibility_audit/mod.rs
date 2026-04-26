use std::time::Duration;

use crate::proto::nskeyedarchiver_encode;
use plist::{Dictionary, Value};
use serde::Serialize;
use serde_json::{Map, Value as JsonValue};
use tokio::io::{AsyncRead, AsyncWrite};

use crate::services::dtx::primitive_enc::archived_object;
use crate::services::dtx::{DtxConnection, DtxError, DtxPayload, NSObject};

pub const SERVICE_NAME: &str = "com.apple.accessibility.axAuditDaemon.remoteserver";
pub const RSD_SERVICE_NAME: &str = "com.apple.accessibility.axAuditDaemon.remoteserver.shim.remote";

const PUBLISH_CAPABILITIES_SELECTOR: &str = "_notifyOfPublishedCapabilities:";
const EVENT_AUDIT_COMPLETE: &str = "hostDeviceDidCompleteAuditCategoriesWithAuditIssues:";
const EVENT_FOCUS_CHANGED: &str = "hostInspectorCurrentElementChanged:";
const EVENT_MONITORED_EVENT_TYPE_CHANGED: &str = "hostInspectorMonitoredEventTypeChanged:";

#[derive(Debug, thiserror::Error)]
pub enum AccessibilityAuditError {
    #[error("DTX error: {0}")]
    Dtx(#[from] DtxError),
    #[error("protocol error: {0}")]
    Protocol(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MoveDirection {
    Previous = 3,
    Next = 4,
    First = 5,
    Last = 6,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AccessibilityAuditHandshake {
    PublishCapabilities,
    SkipInitialCapabilities,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct AuditEvent {
    pub selector: String,
    pub data: JsonValue,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct FocusElement {
    pub platform_identifier: String,
    pub estimated_uid: String,
    pub caption: Option<String>,
    pub spoken_description: Option<String>,
}

impl FocusElement {
    fn try_from_event_payload(
        payload: &JsonValue,
    ) -> Result<Option<Self>, AccessibilityAuditError> {
        let normalized = deserialize_ax_json(payload);
        let object = find_first_object(&normalized).ok_or_else(|| {
            AccessibilityAuditError::Protocol(format!(
                "focus payload was not a JSON object or object list: {}",
                json_debug_snippet(&normalized)
            ))
        })?;
        let Some(platform_bytes) = extract_platform_element_bytes(object) else {
            return Ok(None);
        };
        if platform_bytes.len() < 16 {
            return Err(AccessibilityAuditError::Protocol(format!(
                "platform element identifier too short: {} bytes; payload: {}",
                platform_bytes.len(),
                json_debug_snippet(&normalized)
            )));
        }

        let platform_identifier = hex::encode_upper(&platform_bytes);
        let estimated_uid = format!(
            "{}-0000-0000-{}-000000000000",
            hex::encode_upper(&platform_bytes[12..16]),
            hex::encode_upper(&platform_bytes[0..2])
        );

        Ok(Some(Self {
            platform_identifier,
            estimated_uid,
            caption: get_optional_string(object, "CaptionTextValue_v1"),
            spoken_description: get_optional_string(object, "SpokenDescriptionValue_v1"),
        }))
    }

    pub fn from_event_payload(payload: &JsonValue) -> Result<Self, AccessibilityAuditError> {
        Self::try_from_event_payload(payload)?.ok_or_else(|| {
            AccessibilityAuditError::Protocol(format!(
                "focus payload did not contain PlatformElementValue_v1 bytes: {}",
                json_debug_snippet(&deserialize_ax_json(payload))
            ))
        })
    }
}

pub struct AccessibilityAuditClient<S> {
    conn: DtxConnection<S>,
    product_major_version: u64,
    handshake: AccessibilityAuditHandshake,
    published_capabilities: bool,
    initial_messages_flushed: bool,
    initial_messages_to_flush: usize,
}

impl<S> AccessibilityAuditClient<S>
where
    S: AsyncRead + AsyncWrite + Unpin + Send,
{
    pub fn new(stream: S, product_major_version: u64) -> Self {
        Self::new_with_handshake(
            stream,
            product_major_version,
            AccessibilityAuditHandshake::SkipInitialCapabilities,
        )
    }

    pub fn new_rsd(stream: S, product_major_version: u64) -> Self {
        Self::new_with_handshake(
            stream,
            product_major_version,
            AccessibilityAuditHandshake::SkipInitialCapabilities,
        )
    }

    pub fn new_with_handshake(
        stream: S,
        product_major_version: u64,
        handshake: AccessibilityAuditHandshake,
    ) -> Self {
        Self {
            conn: DtxConnection::new(stream),
            product_major_version,
            handshake,
            published_capabilities: handshake
                == AccessibilityAuditHandshake::SkipInitialCapabilities,
            initial_messages_flushed: false,
            initial_messages_to_flush: if product_major_version >= 15 { 2 } else { 1 },
        }
    }

    async fn ensure_ready(&mut self) -> Result<(), AccessibilityAuditError> {
        if !self.published_capabilities {
            self.publish_capabilities().await?;
        }
        if self.initial_messages_flushed {
            return Ok(());
        }

        for _ in 0..self.initial_messages_to_flush {
            let message =
                match tokio::time::timeout(Duration::from_millis(300), self.conn.recv()).await {
                    Ok(result) => result?,
                    Err(_) => continue, // slot timed out; keep iterating remaining slots
                };
            if message.expects_reply {
                self.conn.send_ack(&message).await?;
            }
        }

        self.initial_messages_flushed = true;
        Ok(())
    }

    pub async fn publish_capabilities(&mut self) -> Result<(), AccessibilityAuditError> {
        if self.handshake == AccessibilityAuditHandshake::SkipInitialCapabilities {
            self.published_capabilities = true;
            return Ok(());
        }

        let payload = nskeyedarchiver_encode::archive_dict(vec![
            (
                "com.apple.private.DTXBlockCompression".to_string(),
                Value::Integer(2.into()),
            ),
            (
                "com.apple.private.DTXConnection".to_string(),
                Value::Integer(1.into()),
            ),
        ]);
        self.conn
            .method_call_async(
                0,
                PUBLISH_CAPABILITIES_SELECTOR,
                &[archived_object(payload)],
            )
            .await?;
        self.published_capabilities = true;
        Ok(())
    }

    pub async fn capabilities(&mut self) -> Result<Vec<String>, AccessibilityAuditError> {
        self.ensure_ready().await?;
        let response = self.conn.method_call(0, "deviceCapabilities", &[]).await?;
        extract_string_vec_response(response.payload)
    }

    pub async fn api_version(&mut self) -> Result<u64, AccessibilityAuditError> {
        self.ensure_ready().await?;
        let response = self.conn.method_call(0, "deviceApiVersion", &[]).await?;
        extract_u64_response(response.payload)
    }

    pub async fn supported_audit_types(&mut self) -> Result<JsonValue, AccessibilityAuditError> {
        self.ensure_ready().await?;
        let selector = if self.product_major_version >= 15 {
            "deviceAllSupportedAuditTypes"
        } else {
            "deviceAllAuditCaseIDs"
        };
        let response = self.conn.method_call(0, selector, &[]).await?;
        extract_json_response(response.payload)
    }

    pub async fn settings(&mut self) -> Result<JsonValue, AccessibilityAuditError> {
        self.ensure_ready().await?;
        let response = self
            .conn
            .method_call(0, "deviceAccessibilitySettings", &[])
            .await?;
        extract_json_response(response.payload).map(|value| deserialize_ax_json(&value))
    }

    pub async fn set_app_monitoring_enabled(
        &mut self,
        enabled: bool,
    ) -> Result<(), AccessibilityAuditError> {
        self.ensure_ready().await?;
        self.conn
            .method_call_async(
                0,
                "deviceSetAppMonitoringEnabled:",
                &[archived_object(nskeyedarchiver_encode::archive_bool(
                    enabled,
                ))],
            )
            .await?;
        Ok(())
    }

    pub async fn set_monitored_event_type(
        &mut self,
        event_type: u64,
    ) -> Result<(), AccessibilityAuditError> {
        self.ensure_ready().await?;
        self.conn
            .method_call_async(
                0,
                "deviceInspectorSetMonitoredEventType:",
                &[archived_object(nskeyedarchiver_encode::archive_int(
                    event_type as i64,
                ))],
            )
            .await?;
        Ok(())
    }

    pub async fn set_show_ignored_elements(
        &mut self,
        enabled: bool,
    ) -> Result<(), AccessibilityAuditError> {
        self.ensure_ready().await?;
        self.conn
            .method_call_async(
                0,
                "deviceInspectorShowIgnoredElements:",
                &[archived_object(nskeyedarchiver_encode::archive_bool(
                    enabled,
                ))],
            )
            .await?;
        Ok(())
    }

    pub async fn set_show_visuals(&mut self, enabled: bool) -> Result<(), AccessibilityAuditError> {
        self.ensure_ready().await?;
        self.conn
            .method_call_async(
                0,
                "deviceInspectorShowVisuals:",
                &[archived_object(nskeyedarchiver_encode::archive_bool(
                    enabled,
                ))],
            )
            .await?;
        Ok(())
    }

    pub async fn set_audit_target_pid(&mut self, pid: u64) -> Result<(), AccessibilityAuditError> {
        self.ensure_ready().await?;
        self.conn
            .method_call_async(
                0,
                "deviceSetAuditTargetPid:",
                &[archived_object(nskeyedarchiver_encode::archive_int(
                    pid as i64,
                ))],
            )
            .await?;
        Ok(())
    }

    pub async fn focus_on_element(&mut self) -> Result<(), AccessibilityAuditError> {
        self.ensure_ready().await?;
        self.conn
            .method_call_async(
                0,
                "deviceInspectorFocusOnElement:",
                &[archived_object(nskeyedarchiver_encode::archive_null())],
            )
            .await?;
        Ok(())
    }

    pub async fn preview_on_element(&mut self) -> Result<(), AccessibilityAuditError> {
        self.ensure_ready().await?;
        self.conn
            .method_call_async(
                0,
                "deviceInspectorPreviewOnElement:",
                &[archived_object(nskeyedarchiver_encode::archive_null())],
            )
            .await?;
        Ok(())
    }

    pub async fn highlight_issue(&mut self) -> Result<(), AccessibilityAuditError> {
        self.ensure_ready().await?;
        self.conn
            .method_call_async(
                0,
                "deviceHighlightIssue:",
                &[archived_object(nskeyedarchiver_encode::archive_dict(
                    vec![],
                ))],
            )
            .await?;
        Ok(())
    }

    pub async fn move_focus(
        &mut self,
        direction: MoveDirection,
    ) -> Result<(), AccessibilityAuditError> {
        self.ensure_ready().await?;
        self.conn
            .method_call_async(
                0,
                "deviceInspectorMoveWithOptions:",
                &[archived_object(nskeyedarchiver_encode::archive_dict(vec![
                    (
                        "ObjectType".to_string(),
                        Value::String("passthrough".to_string()),
                    ),
                    (
                        "Value".to_string(),
                        Value::Dictionary(Dictionary::from_iter([
                            (
                                "allowNonAX".to_string(),
                                passthrough_value(Value::Boolean(false)),
                            ),
                            (
                                "direction".to_string(),
                                passthrough_value(Value::Integer((direction as i64).into())),
                            ),
                            (
                                "includeContainers".to_string(),
                                passthrough_value(Value::Boolean(true)),
                            ),
                        ])),
                    ),
                ]))],
            )
            .await?;
        Ok(())
    }

    pub async fn next_event(&mut self) -> Result<AuditEvent, AccessibilityAuditError> {
        self.ensure_ready().await?;
        loop {
            let message = self.conn.recv().await?;
            if message.expects_reply {
                self.conn.send_ack(&message).await?;
            }

            if let DtxPayload::MethodInvocation { selector, args } = message.payload {
                let data = if args.len() == 1 {
                    deserialize_ax_json(&nsobject_to_json(&args[0]))
                } else {
                    deserialize_ax_json(&JsonValue::Array(
                        args.iter().map(nsobject_to_json).collect(),
                    ))
                };
                return Ok(AuditEvent { selector, data });
            }
        }
    }

    pub async fn wait_for_event(
        &mut self,
        selector: &str,
    ) -> Result<AuditEvent, AccessibilityAuditError> {
        loop {
            let event = self.next_event().await?;
            if event.selector == selector {
                return Ok(event);
            }
        }
    }

    pub async fn wait_for_monitored_event_type_changed(
        &mut self,
    ) -> Result<(), AccessibilityAuditError> {
        let _ = self
            .wait_for_event(EVENT_MONITORED_EVENT_TYPE_CHANGED)
            .await?;
        Ok(())
    }

    pub async fn run_audit(
        &mut self,
        audit_types: &[String],
    ) -> Result<JsonValue, AccessibilityAuditError> {
        self.ensure_ready().await?;
        let selector = if self.product_major_version >= 15 {
            "deviceBeginAuditTypes:"
        } else {
            "deviceBeginAuditCaseIDs:"
        };
        let payload = nskeyedarchiver_encode::archive_array(
            audit_types
                .iter()
                .cloned()
                .map(Value::String)
                .collect::<Vec<_>>(),
        );
        self.conn
            .method_call_async(0, selector, &[archived_object(payload)])
            .await?;

        loop {
            let event = self.next_event().await?;
            if event.selector == EVENT_AUDIT_COMPLETE {
                return Ok(extract_audit_complete_issues(&event.data));
            }
        }
    }

    pub async fn next_focus_change(&mut self) -> Result<FocusElement, AccessibilityAuditError> {
        self.ensure_ready().await?;
        loop {
            let event = self.next_event().await?;
            if event.selector != EVENT_FOCUS_CHANGED {
                continue;
            }
            if let Some(focus) = FocusElement::try_from_event_payload(&event.data)? {
                return Ok(focus);
            }
        }
    }

    pub async fn next_focus_change_with_idle_timeout(
        &mut self,
        idle_timeout: Duration,
    ) -> Result<Option<FocusElement>, AccessibilityAuditError> {
        self.ensure_ready().await?;
        loop {
            let event = match tokio::time::timeout(idle_timeout, self.next_event()).await {
                Ok(event) => event?,
                Err(_) => return Ok(None),
            };
            if event.selector != EVENT_FOCUS_CHANGED {
                continue;
            }
            if let Some(focus) = FocusElement::try_from_event_payload(&event.data)? {
                return Ok(Some(focus));
            }
        }
    }

    /// Navigate focus in the given direction and return the newly focused element.
    pub async fn navigate(
        &mut self,
        direction: MoveDirection,
        timeout: Duration,
    ) -> Result<Option<FocusElement>, AccessibilityAuditError> {
        self.move_focus(direction).await?;
        self.next_focus_change_with_idle_timeout(timeout).await
    }

    /// Perform the Activate action on the currently focused element.
    ///
    /// `element_bytes` should be the raw platform identifier bytes from `FocusElement`.
    pub async fn perform_action_activate(
        &mut self,
        element_bytes: &[u8],
    ) -> Result<(), AccessibilityAuditError> {
        self.ensure_ready().await?;

        // Build AXAuditElement_v1 wrapper for the element
        let element_payload = nskeyedarchiver_encode::archive_dict(vec![
            (
                "ObjectType".to_string(),
                Value::String("passthrough".to_string()),
            ),
            (
                "Value".to_string(),
                Value::Dictionary(Dictionary::from_iter([(
                    "AXAuditElement_v1".to_string(),
                    Value::Dictionary(Dictionary::from_iter([(
                        "PlatformElementValue_v1".to_string(),
                        passthrough_value(Value::Data(element_bytes.to_vec())),
                    )])),
                )])),
            ),
        ]);

        // Build AXAuditElementAttribute_v1 for Activate action
        let action_payload = nskeyedarchiver_encode::archive_dict(vec![
            (
                "ObjectType".to_string(),
                Value::String("cycler".to_string()),
            ),
            (
                "Value".to_string(),
                Value::Dictionary(Dictionary::from_iter([
                    (
                        "AttributeNameValue_v1".to_string(),
                        passthrough_value(Value::String("AXAction-2010".to_string())),
                    ),
                    (
                        "HumanReadableNameValue_v1".to_string(),
                        passthrough_value(Value::String("Activate".to_string())),
                    ),
                    (
                        "PerformsActionValue_v1".to_string(),
                        passthrough_value(Value::Boolean(true)),
                    ),
                    (
                        "SettableValue_v1".to_string(),
                        passthrough_value(Value::Boolean(false)),
                    ),
                    (
                        "ValueTypeValue_v1".to_string(),
                        passthrough_value(Value::Integer(1.into())),
                    ),
                    (
                        "DisplayAsTree_v1".to_string(),
                        passthrough_value(Value::Boolean(false)),
                    ),
                    (
                        "DisplayInlineValue_v1".to_string(),
                        passthrough_value(Value::Boolean(false)),
                    ),
                    (
                        "IsInternal_v1".to_string(),
                        passthrough_value(Value::Boolean(false)),
                    ),
                ])),
            ),
        ]);

        // Build empty value dict
        let value_payload = nskeyedarchiver_encode::archive_dict(vec![]);

        self.conn
            .method_call_async(
                0,
                "deviceElement:performAction:withValue:",
                &[
                    archived_object(element_payload),
                    archived_object(action_payload),
                    archived_object(value_payload),
                ],
            )
            .await?;
        Ok(())
    }
}

pub fn deserialize_ax_object(value: &Value) -> JsonValue {
    deserialize_ax_json(&plist_to_json(value))
}

fn plist_to_json(value: &Value) -> JsonValue {
    match value {
        Value::Boolean(v) => JsonValue::Bool(*v),
        Value::Data(bytes) => {
            JsonValue::Array(bytes.iter().copied().map(JsonValue::from).collect())
        }
        Value::Date(date) => JsonValue::String(date.to_xml_format()),
        Value::Integer(v) => v
            .as_signed()
            .map(JsonValue::from)
            .or_else(|| v.as_unsigned().map(JsonValue::from))
            .unwrap_or(JsonValue::Null),
        Value::Real(v) => serde_json::Number::from_f64(*v)
            .map(JsonValue::Number)
            .unwrap_or(JsonValue::Null),
        Value::String(v) => JsonValue::String(v.clone()),
        Value::Uid(v) => JsonValue::from(v.get()),
        Value::Array(values) => JsonValue::Array(values.iter().map(plist_to_json).collect()),
        Value::Dictionary(dict) => JsonValue::Object(
            dict.iter()
                .map(|(key, value)| (key.clone(), plist_to_json(value)))
                .collect(),
        ),
        _ => JsonValue::Null,
    }
}

fn deserialize_ax_json(value: &JsonValue) -> JsonValue {
    match value {
        JsonValue::Array(items) => {
            JsonValue::Array(items.iter().map(deserialize_ax_json).collect())
        }
        JsonValue::Object(object) => {
            if let Some(object_type) = object.get("ObjectType").and_then(JsonValue::as_str) {
                if object_type == "passthrough" {
                    return object
                        .get("Value")
                        .map(deserialize_ax_json)
                        .unwrap_or(JsonValue::Null);
                }
                if let Some(inner) = object.get("Value") {
                    return deserialize_ax_json(inner);
                }
            }

            JsonValue::Object(
                object
                    .iter()
                    .map(|(key, value)| (key.clone(), deserialize_ax_json(value)))
                    .collect(),
            )
        }
        other => other.clone(),
    }
}

fn extract_string_vec_response(
    payload: DtxPayload,
) -> Result<Vec<String>, AccessibilityAuditError> {
    match payload {
        DtxPayload::Response(NSObject::Array(items)) => items
            .into_iter()
            .map(|item| match item {
                NSObject::String(value) => Ok(value),
                other => Err(AccessibilityAuditError::Protocol(format!(
                    "expected string array item, got {other:?}"
                ))),
            })
            .collect(),
        other => Err(AccessibilityAuditError::Protocol(format!(
            "expected array response, got {other:?}"
        ))),
    }
}

fn extract_u64_response(payload: DtxPayload) -> Result<u64, AccessibilityAuditError> {
    match payload {
        DtxPayload::Response(NSObject::Int(value)) if value >= 0 => Ok(value as u64),
        DtxPayload::Response(NSObject::Uint(value)) => Ok(value),
        other => Err(AccessibilityAuditError::Protocol(format!(
            "expected integer response, got {other:?}"
        ))),
    }
}

fn extract_json_response(payload: DtxPayload) -> Result<JsonValue, AccessibilityAuditError> {
    match payload {
        DtxPayload::Response(value) => Ok(nsobject_to_json(&value)),
        other => Err(AccessibilityAuditError::Protocol(format!(
            "expected response payload, got {other:?}"
        ))),
    }
}

fn nsobject_to_json(value: &NSObject) -> JsonValue {
    match value {
        NSObject::Int(v) => JsonValue::from(*v),
        NSObject::Uint(v) => JsonValue::from(*v),
        NSObject::Double(v) => serde_json::Number::from_f64(*v)
            .map(JsonValue::Number)
            .unwrap_or(JsonValue::Null),
        NSObject::Bool(v) => JsonValue::Bool(*v),
        NSObject::String(v) => JsonValue::String(v.clone()),
        NSObject::Data(bytes) => {
            JsonValue::Array(bytes.iter().copied().map(JsonValue::from).collect())
        }
        NSObject::Array(items) => JsonValue::Array(items.iter().map(nsobject_to_json).collect()),
        NSObject::Dict(dict) => JsonValue::Object(
            dict.iter()
                .map(|(key, value)| (key.clone(), nsobject_to_json(value)))
                .collect::<Map<String, JsonValue>>(),
        ),
        NSObject::Null => JsonValue::Null,
    }
}

fn get_optional_string(object: &Map<String, JsonValue>, field: &str) -> Option<String> {
    object.get(field).and_then(json_string_to_owned)
}

fn find_first_object(value: &JsonValue) -> Option<&Map<String, JsonValue>> {
    match value {
        JsonValue::Object(object) => Some(object),
        JsonValue::Array(items) => items.iter().find_map(find_first_object),
        _ => None,
    }
}

fn extract_platform_element_bytes(object: &Map<String, JsonValue>) -> Option<Vec<u8>> {
    if let Some(bytes) = object
        .get("PlatformElementValue_v1")
        .and_then(json_bytes_to_vec)
    {
        return Some(bytes);
    }

    if let Some(bytes) = object
        .get("ElementValue_v1")
        .and_then(JsonValue::as_object)
        .and_then(|element| element.get("PlatformElementValue_v1"))
        .and_then(json_bytes_to_vec)
    {
        return Some(bytes);
    }

    object
        .values()
        .filter_map(JsonValue::as_object)
        .find_map(extract_platform_element_bytes)
}

fn extract_audit_complete_issues(payload: &JsonValue) -> JsonValue {
    match payload {
        JsonValue::Array(items) if items.len() == 1 => {
            if let Some(value) = items[0]
                .as_object()
                .and_then(|item| item.get("value").or_else(|| item.get("Value")))
            {
                return value.clone();
            }
            if let JsonValue::Array(inner) = &items[0] {
                return JsonValue::Array(inner.clone());
            }
            payload.clone()
        }
        _ => payload.clone(),
    }
}

fn json_bytes_to_vec(value: &JsonValue) -> Option<Vec<u8>> {
    match value {
        JsonValue::Array(items) => items
            .iter()
            .map(|item| item.as_u64().map(|v| v as u8))
            .collect(),
        JsonValue::Object(object) => object
            .get("Value")
            .and_then(json_bytes_to_vec)
            .or_else(|| object.values().find_map(json_bytes_to_vec)),
        _ => None,
    }
}

fn json_string_to_owned(value: &JsonValue) -> Option<String> {
    match value {
        JsonValue::String(value) => Some(value.clone()),
        JsonValue::Object(object) => object
            .get("Value")
            .and_then(json_string_to_owned)
            .or_else(|| object.values().find_map(json_string_to_owned)),
        _ => None,
    }
}

fn json_debug_snippet(value: &JsonValue) -> String {
    let text = serde_json::to_string(value).unwrap_or_else(|_| "<unserializable>".to_string());
    debug_snippet(&text)
}

fn debug_snippet(text: &str) -> String {
    const LIMIT: usize = 1200;
    if text.len() <= LIMIT {
        text.to_string()
    } else {
        format!("{}...", &text[..LIMIT])
    }
}

fn passthrough_value(value: Value) -> Value {
    Value::Dictionary(Dictionary::from_iter([
        (
            "ObjectType".to_string(),
            Value::String("passthrough".to_string()),
        ),
        ("Value".to_string(), value),
    ]))
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;
    use crate::services::dtx::{DtxPayload, NSObject};

    #[test]
    fn extract_string_vec_response_rejects_non_string_entries() {
        let err = extract_string_vec_response(DtxPayload::Response(NSObject::Array(vec![
            NSObject::String("ok".into()),
            NSObject::Bool(true),
        ])))
        .expect_err("mixed response types must fail");

        assert!(err
            .to_string()
            .contains("expected string array item, got Bool(true)"));
    }

    #[test]
    fn extract_u64_response_accepts_signed_and_unsigned_values() {
        assert_eq!(
            extract_u64_response(DtxPayload::Response(NSObject::Int(17))).unwrap(),
            17
        );
        assert_eq!(
            extract_u64_response(DtxPayload::Response(NSObject::Uint(42))).unwrap(),
            42
        );
    }

    #[test]
    fn focus_element_rejects_short_platform_identifier() {
        let err = FocusElement::from_event_payload(&json!({
            "PlatformElementValue_v1": [1, 2, 3],
        }))
        .expect_err("short platform identifiers must fail");

        assert!(err
            .to_string()
            .contains("platform element identifier too short: 3 bytes"));
    }

    #[test]
    fn focus_element_try_from_event_payload_skips_metadata_only_events() {
        let focus = FocusElement::try_from_event_payload(&json!({
            "CaptionTextValue_v1": "",
            "InspectorSectionsValue_v1": [],
            "SpokenDescriptionValue_v1": ""
        }))
        .expect("metadata-only focus event should parse");

        assert!(focus.is_none());
    }

    #[test]
    fn extract_platform_element_bytes_finds_nested_value_recursively() {
        let value = json!({
            "Outer": {
                "Inner": {
                    "PlatformElementValue_v1": [1, 2, 3, 4]
                }
            }
        });

        let object = value.as_object().expect("json object");
        assert_eq!(
            extract_platform_element_bytes(object),
            Some(vec![1, 2, 3, 4])
        );
    }

    #[test]
    fn extract_platform_element_bytes_unwraps_nested_value_wrappers() {
        let value = json!({
            "ElementValue_v1": {
                "Value": {
                    "Value": {
                        "PlatformElementValue_v1": {
                            "Value": [1, 2, 3, 4]
                        }
                    }
                }
            }
        });

        let object = value.as_object().expect("json object");
        assert_eq!(
            extract_platform_element_bytes(object),
            Some(vec![1, 2, 3, 4])
        );
    }

    #[test]
    fn get_optional_string_unwraps_nested_value_wrappers() {
        let value = json!({
            "SpokenDescriptionValue_v1": {
                "Value": "Play button"
            }
        });

        let object = value.as_object().expect("json object");
        assert_eq!(
            get_optional_string(object, "SpokenDescriptionValue_v1"),
            Some("Play button".to_string())
        );
    }

    #[test]
    fn passthrough_value_wraps_value_with_expected_marker() {
        let wrapped = passthrough_value(Value::Boolean(true));
        let dict = wrapped.as_dictionary().expect("wrapped dictionary");
        assert_eq!(
            dict.get("ObjectType").and_then(Value::as_string),
            Some("passthrough")
        );
        assert_eq!(dict.get("Value").and_then(Value::as_boolean), Some(true));
    }

    #[test]
    fn extract_audit_complete_issues_unwraps_single_value_wrapper() {
        let payload = json!([
            {
                "value": [
                    {
                        "IssueClassificationValue_v1": 12
                    }
                ]
            }
        ]);

        assert_eq!(
            extract_audit_complete_issues(&payload),
            json!([
                {
                    "IssueClassificationValue_v1": 12
                }
            ])
        );
    }
}
