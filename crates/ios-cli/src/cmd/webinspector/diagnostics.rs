//! Diagnostic protocol failures, separate from explicitly requested runtime data.
use serde_json::{json, Value as JsonValue};

// Only the top-level protocol failure is a diagnostic. Runtime results,
// exceptionDetails, console events and page data are explicitly requested
// debugger data and keep their protocol shape.
pub(super) fn diagnostic_cdp_response(mut message: JsonValue) -> JsonValue {
    struct ProtocolReplyError(JsonValue);
    impl std::fmt::Display for ProtocolReplyError {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            std::fmt::Display::fmt(&self.0, f)
        }
    }
    impl std::fmt::Debug for ProtocolReplyError {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            std::fmt::Display::fmt(&ios_core::redaction::diagnostic_error(self), f)
        }
    }
    impl std::error::Error for ProtocolReplyError {}
    if let Some(object) = message.as_object_mut() {
        if let Some(error) = object.remove("error") {
            let code = error
                .get("code")
                .and_then(JsonValue::as_i64)
                .unwrap_or(-32000);
            object.insert(
                "error".into(),
                json!({
                    "code": code,
                    "message": format!("WebInspector protocol request failed: {}",
                        ios_core::redaction::diagnostic_error(&ProtocolReplyError(error))),
                }),
            );
        }
    }
    message
}
