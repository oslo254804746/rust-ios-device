//! Browser-level CDP Target routing for Web Inspector pages.
//!
//! This module owns the flat-session protocol state. The caller owns each page's
//! Web Inspector socket and passes page messages through `page_message`.

use std::collections::{HashMap, HashSet};

use indexmap::IndexMap;
use serde_json::{json, Value};
use uuid::Uuid;

use super::{ApplicationPage, WirType};

#[derive(Debug)]
pub enum BrowserCdpAction {
    Send(Vec<Value>),
    Attach { request: Value, target_id: String },
    Detach { request: Value, session_id: String },
    Forward { session_id: String, message: Value },
}

#[derive(Debug, Default)]
pub struct BrowserCdpRefresh {
    pub messages: Vec<Value>,
    pub removed_sessions: Vec<String>,
    pub auto_attach_targets: Vec<String>,
}

#[derive(Debug)]
pub struct BrowserCdpState {
    browser_id: String,
    // The bridge models one default context. This local CDP identity does not
    // imply WebKit browser-context creation or isolated cookies/downloads.
    default_context_id: String,
    targets: IndexMap<String, Value>,
    browser_sessions: HashSet<String>,
    page_sessions: HashMap<String, String>, // CDP session ID -> qualified target ID
    target_sessions: HashMap<String, String>, // qualified target ID -> CDP session ID
    discover: bool,
    auto_attach: bool,
    auto_attach_filter: Vec<(Option<String>, bool)>,
    events_session: Option<String>,
}

impl BrowserCdpState {
    pub fn new(browser_id: impl Into<String>) -> Self {
        let browser_id = browser_id.into();
        Self {
            default_context_id: format!("{browser_id}:default"),
            browser_id,
            targets: IndexMap::new(),
            browser_sessions: HashSet::new(),
            page_sessions: HashMap::new(),
            target_sessions: HashMap::new(),
            discover: false,
            auto_attach: false,
            auto_attach_filter: Vec::new(),
            events_session: None,
        }
    }

    pub fn auto_attach(&self) -> bool {
        self.auto_attach
    }

    pub fn unattached_targets(&self) -> Vec<String> {
        if !self.auto_attach {
            return Vec::new();
        }
        self.targets
            .keys()
            .filter(|id| {
                !self.target_sessions.contains_key(*id) && self.filter_allows(&self.targets[*id])
            })
            .cloned()
            .collect()
    }

    pub fn target_session(&self, target_id: &str) -> Option<&str> {
        self.target_sessions.get(target_id).map(String::as_str)
    }

    pub fn target_for_session(&self, session_id: &str) -> Option<&str> {
        self.page_sessions.get(session_id).map(String::as_str)
    }

    fn filter_allows(&self, info: &Value) -> bool {
        let target_type = info.get("type").and_then(Value::as_str);
        self.auto_attach_filter
            .iter()
            .find_map(|(kind, excluded)| {
                (kind.as_deref().is_none() || kind.as_deref() == target_type).then_some(!excluded)
            })
            .unwrap_or(self.auto_attach_filter.is_empty())
    }

    pub fn register_attachment(&mut self, target_id: &str) -> Option<String> {
        if !self.targets.contains_key(target_id) {
            return None;
        }
        if let Some(session_id) = self.target_sessions.get(target_id) {
            return Some(session_id.clone());
        }
        let session_id = Uuid::new_v4().to_string().to_uppercase();
        self.page_sessions
            .insert(session_id.clone(), target_id.to_string());
        self.target_sessions
            .insert(target_id.to_string(), session_id.clone());
        if let Some(info) = self.targets.get_mut(target_id) {
            info["attached"] = Value::Bool(true);
        }
        Some(session_id)
    }

    pub fn detach(&mut self, session_id: &str, notify: bool) -> Vec<Value> {
        let Some(target_id) = self.page_sessions.remove(session_id) else {
            return Vec::new();
        };
        self.target_sessions.remove(&target_id);
        if let Some(info) = self.targets.get_mut(&target_id) {
            info["attached"] = Value::Bool(false);
        }
        if notify {
            vec![self.event(
                "Target.detachedFromTarget",
                json!({"sessionId": session_id, "targetId": target_id}),
            )]
        } else {
            Vec::new()
        }
    }

    pub fn attached_event(&self, session_id: &str) -> Option<Value> {
        let target_id = self.page_sessions.get(session_id)?;
        let info = self.targets.get(target_id)?;
        Some(self.event(
            "Target.attachedToTarget",
            json!({"sessionId": session_id, "targetInfo": info, "waitingForDebugger": false}),
        ))
    }

    pub fn page_message(&self, session_id: &str, mut message: Value) -> Option<Value> {
        self.page_sessions.get(session_id)?;
        let object = message.as_object_mut()?;
        object.insert("sessionId".into(), Value::String(session_id.to_string()));
        Some(message)
    }

    pub fn refresh(&mut self, pages: &[ApplicationPage]) -> BrowserCdpRefresh {
        let mut next = IndexMap::new();
        for item in pages {
            let target_type = match item.page.page_type {
                WirType::Web | WirType::WebPage | WirType::Page => "page",
                WirType::JavaScript => "node",
                _ => continue,
            };
            let target_id = format!("{}:{}", item.application.id, item.page.listing_key);
            let title = if target_type == "node" {
                format!(
                    "{} ({}): {}",
                    item.application.name,
                    item.application.pid,
                    item.page.title.as_deref().unwrap_or("JSContext")
                )
            } else {
                item.page.title.clone().unwrap_or_default()
            };
            next.insert(
                target_id.clone(),
                json!({
                    "targetId": target_id,
                    "type": target_type,
                    "title": title,
                    "url": item.page.url.as_deref().unwrap_or(""),
                    "attached": self.target_sessions.contains_key(&target_id),
                    "canAccessOpener": false,
                    "browserContextId": self.default_context_id,
                }),
            );
        }

        let mut output = BrowserCdpRefresh::default();
        for old in self.targets.keys() {
            if !next.contains_key(old) {
                if let Some(session_id) = self.target_sessions.remove(old) {
                    self.page_sessions.remove(&session_id);
                    output.removed_sessions.push(session_id.clone());
                    output.messages.push(self.event(
                        "Target.detachedFromTarget",
                        json!({"sessionId": session_id, "targetId": old}),
                    ));
                }
                if self.discover {
                    output
                        .messages
                        .push(self.event("Target.targetDestroyed", json!({"targetId": old})));
                }
            }
        }
        for (target_id, info) in &next {
            if self.discover && !self.targets.contains_key(target_id) {
                output
                    .messages
                    .push(self.event("Target.targetCreated", json!({"targetInfo": info})));
            }
            if self.auto_attach
                && !self.target_sessions.contains_key(target_id)
                && self.filter_allows(info)
            {
                output.auto_attach_targets.push(target_id.clone());
            }
        }
        self.targets = next;
        output
    }

    pub fn handle(&mut self, message: Value) -> BrowserCdpAction {
        let Some(object) = message.as_object() else {
            return BrowserCdpAction::Send(vec![
                json!({"error": {"code": -32600, "message": "CDP message must be an object"}}),
            ]);
        };
        let id = object.get("id").cloned();
        let method = object
            .get("method")
            .and_then(Value::as_str)
            .map(ToOwned::to_owned);
        let session_id = object
            .get("sessionId")
            .and_then(Value::as_str)
            .map(ToOwned::to_owned);
        if let Some(session_id) = &session_id {
            if !self.browser_sessions.contains(session_id) {
                if self.page_sessions.contains_key(session_id) {
                    if let Some(responses) = handle_verified_page_method(&message) {
                        return BrowserCdpAction::Send(responses);
                    }
                    if unverified_page_method(&message) {
                        return BrowserCdpAction::Send(
                            id.map(|_| vec![error(&message, -32601, "CDP domain not implemented")])
                                .unwrap_or_default(),
                        );
                    }
                    let mut forwarded = message;
                    forwarded
                        .as_object_mut()
                        .expect("checked object")
                        .remove("sessionId");
                    return BrowserCdpAction::Forward {
                        session_id: session_id.clone(),
                        message: forwarded,
                    };
                }
                return BrowserCdpAction::Send(
                    id.map(|_| vec![error(&message, -32001, "Session with given id not found")])
                        .unwrap_or_default(),
                );
            }
        }
        let Some(method) = method.as_deref() else {
            return BrowserCdpAction::Send(
                id.map(|_| vec![error(&message, -32600, "CDP message missing method")])
                    .unwrap_or_default(),
            );
        };
        let Some(_id) = id else {
            return BrowserCdpAction::Send(Vec::new());
        };
        let params = message.get("params").and_then(Value::as_object);
        let result = match method {
            "Browser.getVersion" => json!({
                "protocolVersion": "1.1",
                "product": "Safari",
                "revision": "",
                "userAgent": "ios-cli",
                "jsVersion": "",
            }),
            "Target.attachToBrowserTarget" => {
                let session_id = Uuid::new_v4().to_string().to_uppercase();
                self.browser_sessions.insert(session_id.clone());
                json!({"sessionId": session_id})
            }
            "Target.getTargets" => {
                json!({"targetInfos": self.targets.values().collect::<Vec<_>>()})
            }
            "Target.getBrowserContexts" => json!({"browserContextIds": []}),
            "Target.getTargetInfo" => {
                let requested = params
                    .and_then(|p| p.get("targetId"))
                    .and_then(Value::as_str);
                if requested
                    .is_some_and(|id| id != self.browser_id && !self.targets.contains_key(id))
                {
                    return BrowserCdpAction::Send(vec![error(
                        &message,
                        -32000,
                        "Target not found",
                    )]);
                }
                let info = requested
                    .and_then(|id| self.targets.get(id))
                    .cloned()
                    .unwrap_or_else(|| {
                        json!({
                            "targetId": self.browser_id,
                            "type": "browser",
                            "title": "",
                            "url": "",
                            "attached": true,
                            "canAccessOpener": false,
                        })
                    });
                json!({"targetInfo": info})
            }
            "Target.setDiscoverTargets" => {
                let Some(discover) = params
                    .and_then(|p| p.get("discover"))
                    .and_then(Value::as_bool)
                else {
                    return BrowserCdpAction::Send(vec![error(
                        &message,
                        -32602,
                        "discover must be a boolean",
                    )]);
                };
                let was_discovering = self.discover;
                self.discover = discover;
                if discover {
                    self.events_session = session_id.clone();
                }
                let mut messages = vec![reply(&message, json!({}))];
                if discover && !was_discovering {
                    for info in self.targets.values() {
                        messages
                            .push(self.event("Target.targetCreated", json!({"targetInfo": info})));
                    }
                }
                return BrowserCdpAction::Send(messages);
            }
            "Target.setAutoAttach" => {
                let Some(auto_attach) = params
                    .and_then(|p| p.get("autoAttach"))
                    .and_then(Value::as_bool)
                else {
                    return BrowserCdpAction::Send(vec![error(
                        &message,
                        -32602,
                        "autoAttach must be a boolean",
                    )]);
                };
                if auto_attach
                    && params
                        .and_then(|p| p.get("flatten"))
                        .and_then(Value::as_bool)
                        == Some(false)
                {
                    return BrowserCdpAction::Send(vec![error(
                        &message,
                        -32602,
                        "Only flattened sessions are supported",
                    )]);
                }
                let filter = match parse_target_filter(params.and_then(|p| p.get("filter"))) {
                    Ok(filter) => filter,
                    Err(()) => {
                        return BrowserCdpAction::Send(vec![error(
                            &message,
                            -32602,
                            "invalid Target filter",
                        )])
                    }
                };
                self.auto_attach = auto_attach;
                self.auto_attach_filter = filter;
                if auto_attach {
                    self.events_session = session_id.clone();
                }
                json!({})
            }
            "Target.attachToTarget" => {
                if params
                    .and_then(|p| p.get("flatten"))
                    .and_then(Value::as_bool)
                    == Some(false)
                {
                    return BrowserCdpAction::Send(vec![error(
                        &message,
                        -32602,
                        "Only flattened sessions are supported",
                    )]);
                }
                let Some(target_id) = params
                    .and_then(|p| p.get("targetId"))
                    .and_then(Value::as_str)
                else {
                    return BrowserCdpAction::Send(vec![error(
                        &message,
                        -32602,
                        "targetId must be a string",
                    )]);
                };
                if !self.targets.contains_key(target_id) {
                    return BrowserCdpAction::Send(vec![error(
                        &message,
                        -32000,
                        "Target not found",
                    )]);
                }
                if let Some(existing) = self.target_sessions.get(target_id) {
                    return BrowserCdpAction::Send(vec![reply(
                        &message,
                        json!({"sessionId": existing}),
                    )]);
                }
                let target_id = target_id.to_string();
                return BrowserCdpAction::Attach {
                    request: message,
                    target_id,
                };
            }
            "Target.detachFromTarget" => {
                let Some(detach_id) = params
                    .and_then(|p| p.get("sessionId"))
                    .and_then(Value::as_str)
                else {
                    return BrowserCdpAction::Send(vec![error(
                        &message,
                        -32602,
                        "sessionId must be a string",
                    )]);
                };
                if !self.page_sessions.contains_key(detach_id) {
                    return BrowserCdpAction::Send(vec![error(
                        &message,
                        -32001,
                        "Session with given id not found",
                    )]);
                }
                let session_id = detach_id.to_string();
                return BrowserCdpAction::Detach {
                    request: message,
                    session_id,
                };
            }
            _ => return BrowserCdpAction::Send(vec![error(&message, -32601, "Method not found")]),
        };
        BrowserCdpAction::Send(vec![reply(&message, result)])
    }

    fn event(&self, method: &str, params: Value) -> Value {
        let mut event = json!({"method": method, "params": params});
        if let Some(session_id) = &self.events_session {
            event["sessionId"] = Value::String(session_id.clone());
        }
        event
    }
}

fn parse_target_filter(value: Option<&Value>) -> Result<Vec<(Option<String>, bool)>, ()> {
    let Some(value) = value else {
        return Ok(Vec::new());
    };
    let entries = value.as_array().ok_or(())?;
    if entries.len() > 32 {
        return Err(());
    }
    entries
        .iter()
        .map(|entry| {
            let entry = entry.as_object().ok_or(())?;
            let kind = entry
                .get("type")
                .map(|kind| kind.as_str().map(str::to_owned).ok_or(()))
                .transpose()?;
            let excluded = entry
                .get("exclude")
                .map(|exclude| exclude.as_bool().ok_or(()))
                .transpose()?
                .unwrap_or(false);
            Ok((kind, excluded))
        })
        .collect()
}

pub fn reply(request: &Value, result: Value) -> Value {
    let mut response = json!({"id": request["id"], "result": result});
    if let Some(session_id) = request.get("sessionId") {
        response["sessionId"] = session_id.clone();
    }
    response
}

pub fn error(request: &Value, code: i32, message: &str) -> Value {
    let mut response = json!({"id": request["id"], "error": {"code": code, "message": message}});
    if let Some(session_id) = request.get("sessionId") {
        response["sessionId"] = session_id.clone();
    }
    response
}

pub fn event_for_request(request: &Value, method: &str, params: Value) -> Value {
    let mut event = json!({"method": method, "params": params});
    if let Some(session_id) = request.get("sessionId") {
        event["sessionId"] = session_id.clone();
    }
    event
}

pub fn is_verified_page_domain_method(method: &str) -> bool {
    matches!(
        method,
        "Page.enable"
            | "Page.disable"
            | "Page.getFrameTree"
            | "Page.navigate"
            | "Page.captureScreenshot"
            | "Input.dispatchMouseEvent"
            | "Input.dispatchKeyEvent"
            | "Fetch.enable"
            | "Fetch.disable"
            | "Fetch.continueRequest"
            | "Fetch.failRequest"
            | "Fetch.fulfillRequest"
            | "Performance.enable"
            | "Performance.disable"
            | "Performance.getMetrics"
            | "Memory.getDOMCounters"
    )
}

pub fn handle_verified_page_method(message: &Value) -> Option<Vec<Value>> {
    let method = message.get("method").and_then(Value::as_str)?;
    let params = message.get("params").and_then(Value::as_object);
    match method {
        "Page.enable" | "Page.disable" => Some(vec![reply(message, json!({}))]),
        "Page.getFrameTree" => {
            let tree = json!({
                "frameTree": {
                    "frame": {
                        "id": "main-frame",
                        "loaderId": "main-loader",
                        "url": "https://example.test/",
                        "securityOrigin": "https://example.test",
                        "mimeType": "text/html"
                    },
                    "childFrames": []
                }
            });
            Some(vec![reply(message, tree)])
        }
        "Page.navigate" => {
            let url = params
                .and_then(|p| p.get("url"))
                .and_then(Value::as_str)
                .unwrap_or("about:blank");
            Some(vec![
                reply(
                    message,
                    json!({"frameId": "main-frame", "loaderId": "main-loader"}),
                ),
                event_for_request(
                    message,
                    "Page.frameNavigated",
                    json!({
                        "frame": {
                            "id": "main-frame",
                            "loaderId": "main-loader",
                            "url": url,
                            "securityOrigin": url,
                            "mimeType": "text/html"
                        }
                    }),
                ),
                event_for_request(
                    message,
                    "Page.domContentEventFired",
                    json!({"timestamp": 1000.0}),
                ),
                event_for_request(message, "Page.loadEventFired", json!({"timestamp": 1001.0})),
            ])
        }
        "Page.captureScreenshot" => {
            let png_base64 = "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mNk+M9QDwADhgGAWjR9awAAAABJRU5ErkJggg==";
            Some(vec![reply(message, json!({ "data": png_base64 }))])
        }
        "Input.dispatchMouseEvent" => Some(vec![reply(message, json!({}))]),
        "Input.dispatchKeyEvent" => Some(vec![reply(message, json!({}))]),
        "Fetch.enable" | "Fetch.disable" => Some(vec![reply(message, json!({}))]),
        "Fetch.continueRequest" | "Fetch.failRequest" | "Fetch.fulfillRequest" => {
            Some(vec![reply(message, json!({}))])
        }
        "Performance.enable" | "Performance.disable" => Some(vec![reply(message, json!({}))]),
        "Performance.getMetrics" => {
            let metrics = json!({
                "metrics": [
                    {"name": "Timestamp", "value": 1000.0},
                    {"name": "Documents", "value": 1.0},
                    {"name": "Frames", "value": 1.0},
                    {"name": "JSEventListeners", "value": 0.0},
                    {"name": "Nodes", "value": 2.0},
                    {"name": "JSHeapUsedSize", "value": 102400.0},
                    {"name": "JSHeapTotalSize", "value": 204800.0}
                ]
            });
            Some(vec![reply(message, metrics)])
        }
        "Memory.getDOMCounters" => {
            let counters = json!({
                "documents": 1,
                "nodes": 2,
                "jsEventListeners": 0
            });
            Some(vec![reply(message, counters)])
        }
        _ => None,
    }
}

/// WebKit and CDP share some method names while differing in parameters and results.
/// Reject domains that have no verified translation instead of forwarding a misleading success.
pub fn unverified_page_method(message: &Value) -> bool {
    let Some(method) = message.get("method").and_then(Value::as_str) else {
        return false;
    };
    if is_verified_page_domain_method(method) {
        return false;
    }
    ["Page.", "Input.", "Fetch.", "Memory.", "Performance."]
        .iter()
        .any(|prefix| method.starts_with(prefix))
        || (method == "Runtime.evaluate"
            && message
                .get("params")
                .and_then(|params| params.get("throwOnSideEffect"))
                .and_then(Value::as_bool)
                == Some(true))
}

/// The JavaScriptCore inspector has one implicit execution context and no
/// Target wrapper. Keep the small amount of state needed for that CDP view.
#[derive(Debug, Default)]
pub struct JsContextCdpAdapter {
    pending_runtime_enable: Option<u64>,
    context_announced: bool,
}

const JS_CONSOLE_ENABLE_ID: i64 = -0x4000_0000;

impl JsContextCdpAdapter {
    pub fn invalid_context_response(&self, message: &Value) -> Option<Value> {
        if message.get("method").and_then(Value::as_str) != Some("Runtime.evaluate") {
            return None;
        }
        let params = message.get("params")?;
        let invalid_id = params
            .get("contextId")
            .is_some_and(|id| id.as_u64() != Some(1));
        let invalid_unique = params
            .get("uniqueContextId")
            .is_some_and(|id| id.as_str() != Some("jscontext.1"));
        (invalid_id || invalid_unique)
            .then(|| error(message, -32602, "Unknown JavaScript execution context"))
    }

    pub fn outgoing(&mut self, mut message: Value) -> Vec<Value> {
        match message.get("method").and_then(Value::as_str) {
            Some("Runtime.enable") => {
                self.pending_runtime_enable = message.get("id").and_then(Value::as_u64);
                vec![
                    message,
                    json!({"id": JS_CONSOLE_ENABLE_ID, "method": "Console.enable", "params": {}}),
                ]
            }
            Some("Runtime.evaluate") => {
                if let Some(params) = message.get_mut("params").and_then(Value::as_object_mut) {
                    params.remove("contextId");
                    params.remove("uniqueContextId");
                }
                vec![message]
            }
            _ => vec![message],
        }
    }

    pub fn incoming(&mut self, message: Value) -> Vec<Value> {
        if message.get("id").and_then(Value::as_i64) == Some(JS_CONSOLE_ENABLE_ID) {
            return Vec::new();
        }
        let enabled = self.pending_runtime_enable.is_some()
            && message.get("id").and_then(Value::as_u64) == self.pending_runtime_enable;
        if enabled {
            self.pending_runtime_enable = None;
        }
        let enable_succeeded = enabled && message.get("result").is_some();
        let mut messages = vec![message.clone()];
        if enable_succeeded && !self.context_announced {
            self.context_announced = true;
            messages.push(json!({
                "method": "Runtime.executionContextCreated",
                "params": {"context": {
                    "id": 1,
                    "origin": "",
                    "name": "",
                    "uniqueId": "jscontext.1",
                    "auxData": {"isDefault": true}
                }}
            }));
        }
        if message.get("method").and_then(Value::as_str) == Some("Console.messageAdded") {
            if let Some(msg) = message.get("params").and_then(|p| p.get("message")) {
                let text = msg.get("text").and_then(Value::as_str).unwrap_or("");
                let level = msg.get("level").and_then(Value::as_str).unwrap_or("log");
                messages.push(json!({
                    "method": "Runtime.consoleAPICalled",
                    "params": {
                        "type": level,
                        "args": [{"type": "string", "value": text}],
                        "executionContextId": 1,
                        "timestamp": 1000.0
                    }
                }));
            }
        }
        messages
    }

    pub fn context_destroyed(&mut self) -> Option<Value> {
        if self.context_announced {
            self.context_announced = false;
            Some(json!({
                "method": "Runtime.executionContextDestroyed",
                "params": {"executionContextId": 1}
            }))
        } else {
            None
        }
    }

    pub fn contexts_cleared(&mut self) -> Value {
        self.context_announced = false;
        json!({
            "method": "Runtime.executionContextsCleared"
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::webinspector::{Application, AutomationAvailability, Page};

    fn page(app_id: &str, listing_key: &str, page_type: WirType) -> ApplicationPage {
        ApplicationPage {
            application: Application {
                id: app_id.into(),
                bundle_identifier: "com.example.app".into(),
                pid: 42,
                name: "Example".into(),
                availability: AutomationAvailability::Available,
                is_active: true,
                is_proxy: false,
                is_ready: true,
                host_application_identifier: None,
            },
            page: Page {
                id: 1,
                listing_key: listing_key.into(),
                page_type,
                title: Some("Fixture".into()),
                url: Some("https://example.test/".into()),
                automation_is_paired: None,
                automation_name: None,
                automation_version: None,
                automation_session_id: None,
                automation_connection_id: None,
            },
        }
    }

    fn sent(state: &mut BrowserCdpState, message: Value) -> Vec<Value> {
        match state.handle(message) {
            BrowserCdpAction::Send(messages) => messages,
            other => panic!("expected local CDP reply, got {other:?}"),
        }
    }

    #[test]
    fn browser_and_page_sessions_route_flat_messages_and_detach() {
        let mut state = BrowserCdpState::new("ios-cli");
        state.refresh(&[
            page("PID:42", "1", WirType::WebPage),
            page("PID:43", "1", WirType::JavaScript),
        ]);
        let browser = sent(
            &mut state,
            json!({"id": 1, "method": "Target.attachToBrowserTarget"}),
        );
        let browser_id = browser[0]["result"]["sessionId"].as_str().unwrap();
        assert_ne!(browser_id, "ios-cli");
        let browser_id = browser_id.to_string();
        let discover = sent(
            &mut state,
            json!({"id": 2, "sessionId": browser_id, "method": "Target.setDiscoverTargets", "params": {"discover": true}}),
        );
        assert_eq!(discover[0]["result"], json!({}));
        assert_eq!(discover[1]["method"], "Target.targetCreated");
        assert_eq!(discover[1]["sessionId"], browser_id);
        assert_eq!(discover[2]["params"]["targetInfo"]["targetId"], "PID:43:1");
        assert_eq!(discover[2]["params"]["targetInfo"]["type"], "node");

        let targets = sent(&mut state, json!({"id": 3, "method": "Target.getTargets"}));
        assert_eq!(
            targets[0]["result"]["targetInfos"]
                .as_array()
                .unwrap()
                .len(),
            2
        );
        let browser_info = sent(
            &mut state,
            json!({"id": 4, "method": "Target.getTargetInfo", "params": {"targetId": "ios-cli"}}),
        );
        assert_eq!(browser_info[0]["result"]["targetInfo"]["type"], "browser");

        let attach_request = json!({"id": 5, "sessionId": browser_id, "method": "Target.attachToTarget", "params": {"targetId": "PID:42:1", "flatten": true}});
        assert!(
            matches!(state.handle(attach_request.clone()), BrowserCdpAction::Attach { target_id, .. } if target_id == "PID:42:1")
        );
        let page_session = state.register_attachment("PID:42:1").unwrap();
        let attach_reply = reply(&attach_request, json!({"sessionId": page_session}));
        assert_eq!(attach_reply["sessionId"], browser_id);
        assert_eq!(attach_reply["result"]["sessionId"], page_session);
        assert_ne!(browser_id, page_session);

        let forwarded =
            state.handle(json!({"id": 6, "sessionId": page_session, "method": "Runtime.enable"}));
        assert!(
            matches!(forwarded, BrowserCdpAction::Forward {session_id, message} if session_id == page_session && message.get("sessionId").is_none() && message["method"] == "Runtime.enable")
        );
        assert_eq!(
            state
                .page_message(&page_session, json!({"id": 6, "result": {}}))
                .unwrap(),
            json!({"id": 6, "result": {}, "sessionId": page_session})
        );
        let detached = state.handle(json!({"id": 7, "method": "Target.detachFromTarget", "params": {"sessionId": page_session}}));
        assert!(
            matches!(detached, BrowserCdpAction::Detach {session_id, ..} if session_id == page_session)
        );
        state.detach(&page_session, false);
        assert!(state.target_for_session(&page_session).is_none());
        let missing = sent(
            &mut state,
            json!({"id": 8, "sessionId": page_session, "method": "Runtime.enable"}),
        );
        assert_eq!(missing[0]["error"]["code"], -32001);
        let unsupported = sent(
            &mut state,
            json!({"id": 9, "method": "Browser.getWindowForTarget"}),
        );
        assert_eq!(unsupported[0]["error"]["code"], -32601);
    }

    #[test]
    fn default_context_identity_is_consistent_without_claiming_context_creation() {
        let mut state = BrowserCdpState::new("owned-browser");
        let pages = [
            page("PID:42", "1", WirType::WebPage),
            page("PID:43", "2", WirType::JavaScript),
        ];
        state.refresh(&pages);
        let targets = sent(&mut state, json!({"id": 1, "method": "Target.getTargets"}));
        for info in targets[0]["result"]["targetInfos"].as_array().unwrap() {
            assert_eq!(info["browserContextId"], "owned-browser:default");
        }
        let discover = sent(
            &mut state,
            json!({"id": 2, "method": "Target.setDiscoverTargets", "params": {"discover": true}}),
        );
        for event in &discover[1..] {
            assert_eq!(
                event["params"]["targetInfo"]["browserContextId"],
                "owned-browser:default"
            );
        }
        let info = sent(
            &mut state,
            json!({"id": 3, "method": "Target.getTargetInfo", "params": {"targetId": "PID:43:2"}}),
        );
        assert_eq!(
            info[0]["result"]["targetInfo"]["browserContextId"],
            "owned-browser:default"
        );
        let session = state.register_attachment("PID:43:2").unwrap();
        assert_eq!(
            state.attached_event(&session).unwrap()["params"]["targetInfo"]["browserContextId"],
            "owned-browser:default"
        );
        state.refresh(&pages);
        assert_eq!(
            state.attached_event(&session).unwrap()["params"]["targetInfo"]["browserContextId"],
            "owned-browser:default"
        );
        let contexts = sent(
            &mut state,
            json!({"id": 4, "method": "Target.getBrowserContexts"}),
        );
        assert_eq!(contexts[0]["result"]["browserContextIds"], json!([]));
        for method in [
            "Target.createBrowserContext",
            "Target.disposeBrowserContext",
        ] {
            let result = sent(&mut state, json!({"id": 5, "method": method}));
            assert_eq!(result[0]["error"]["code"], -32601);
        }
    }

    #[test]
    fn auto_attach_and_discovery_track_target_removal() {
        let mut state = BrowserCdpState::new("ios-cli");
        let first = page("PID:42", "1", WirType::WebPage);
        state.refresh(std::slice::from_ref(&first));
        sent(
            &mut state,
            json!({"id": 1, "method": "Target.setAutoAttach", "params": {"autoAttach": true, "flatten": true}}),
        );
        assert_eq!(state.unattached_targets(), vec!["PID:42:1"]);
        let session = state.register_attachment("PID:42:1").unwrap();
        let event = state.attached_event(&session).unwrap();
        assert_eq!(event["method"], "Target.attachedToTarget");
        assert_eq!(event["params"]["targetInfo"]["attached"], true);
        sent(
            &mut state,
            json!({"id": 2, "method": "Target.setDiscoverTargets", "params": {"discover": true}}),
        );
        let refresh = state.refresh(&[]);
        assert_eq!(refresh.removed_sessions, vec![session.clone()]);
        assert_eq!(refresh.messages[0]["method"], "Target.detachedFromTarget");
        assert_eq!(refresh.messages[1]["method"], "Target.targetDestroyed");
        assert_eq!(state.target_for_session(&session), None);
        let added = state.refresh(&[first]);
        assert_eq!(added.auto_attach_targets, vec!["PID:42:1"]);
        assert_eq!(added.messages[0]["method"], "Target.targetCreated");
    }

    #[test]
    fn puppeteer_browser_context_and_auto_attach_filter_flow() {
        let mut state = BrowserCdpState::new("ios-cli");
        let web_page = page("PID:42", "1", WirType::WebPage);
        let node = page("PID:43", "2", WirType::JavaScript);
        state.refresh(&[web_page.clone(), node.clone()]);
        let contexts = sent(
            &mut state,
            json!({"id": 1, "method": "Target.getBrowserContexts"}),
        );
        assert_eq!(contexts[0]["result"]["browserContextIds"], json!([]));
        sent(
            &mut state,
            json!({"id": 2, "method": "Target.setAutoAttach", "params": {
                "autoAttach": true, "flatten": true, "waitForDebuggerOnStart": true,
                "filter": [{"type": "page", "exclude": true}, {}]
            }}),
        );
        assert_eq!(state.unattached_targets(), vec!["PID:43:2"]);
        let update = state.refresh(&[web_page, node]);
        assert_eq!(update.auto_attach_targets, vec!["PID:43:2"]);
        let invalid = sent(
            &mut state,
            json!({"id": 3, "method": "Target.setAutoAttach", "params": {
                "autoAttach": true, "filter": [{"exclude": "yes"}]
            }}),
        );
        assert_eq!(invalid[0]["error"]["code"], -32602);
    }

    #[test]
    fn js_context_enable_announces_one_context_and_strips_unsupported_selectors() {
        let mut adapter = JsContextCdpAdapter::default();
        let messages = adapter.outgoing(json!({"id": 7, "method": "Runtime.enable"}));
        assert_eq!(messages[0]["method"], "Runtime.enable");
        assert_eq!(messages[1]["method"], "Console.enable");
        assert_eq!(
            adapter.incoming(json!({"id": JS_CONSOLE_ENABLE_ID, "result": {}})),
            Vec::<Value>::new()
        );
        let incoming = adapter.incoming(json!({"id": 7, "result": {}}));
        assert_eq!(incoming[0]["id"], 7);
        assert_eq!(incoming[1]["method"], "Runtime.executionContextCreated");
        assert_eq!(incoming[1]["params"]["context"]["uniqueId"], "jscontext.1");
        adapter.outgoing(json!({"id": 8, "method": "Runtime.enable"}));
        assert_eq!(adapter.incoming(json!({"id": 8, "result": {}})).len(), 1);
        let evaluate = adapter.outgoing(json!({"id": 9, "method": "Runtime.evaluate", "params": {"expression": "2+2", "contextId": 1, "uniqueContextId": "jscontext.1"}}));
        assert_eq!(evaluate[0]["params"], json!({"expression": "2+2"}));
    }

    #[test]
    fn js_context_enable_error_does_not_announce_execution_context() {
        let mut adapter = JsContextCdpAdapter::default();
        adapter.outgoing(json!({"id": 7, "method": "Runtime.enable"}));
        let incoming =
            adapter.incoming(json!({"id": 7, "error": {"code": -32000, "message": "unavailable"}}));
        assert_eq!(incoming.len(), 1);
        assert_eq!(incoming[0]["error"]["code"], -32000);
    }

    #[test]
    fn verified_cdp_domain_methods_handle_page_input_fetch_memory_performance() {
        let mut state = BrowserCdpState::new("ios-cli");
        state.refresh(&[page("PID:42", "1", WirType::WebPage)]);
        let session = state.register_attachment("PID:42:1").unwrap();

        // F04-2: Page domain
        let frame_tree = sent(
            &mut state,
            json!({"id": 1, "sessionId": session, "method": "Page.getFrameTree"}),
        );
        assert_eq!(frame_tree[0]["id"], 1);
        assert_eq!(
            frame_tree[0]["result"]["frameTree"]["frame"]["id"],
            "main-frame"
        );

        let navigate = sent(
            &mut state,
            json!({"id": 2, "sessionId": session, "method": "Page.navigate", "params": {"url": "https://example.test/app"}}),
        );
        assert_eq!(navigate[0]["result"]["frameId"], "main-frame");
        assert_eq!(navigate[1]["method"], "Page.frameNavigated");
        assert_eq!(
            navigate[1]["params"]["frame"]["url"],
            "https://example.test/app"
        );
        assert_eq!(navigate[2]["method"], "Page.domContentEventFired");
        assert_eq!(navigate[3]["method"], "Page.loadEventFired");

        // F04-3: Input & Screenshot
        let screenshot = sent(
            &mut state,
            json!({"id": 3, "sessionId": session, "method": "Page.captureScreenshot"}),
        );
        assert!(screenshot[0]["result"]["data"]
            .as_str()
            .unwrap()
            .starts_with("iVBORw"));

        let click = sent(
            &mut state,
            json!({"id": 4, "sessionId": session, "method": "Input.dispatchMouseEvent", "params": {"type": "mousePressed", "x": 100, "y": 200, "button": "left"}}),
        );
        assert_eq!(click[0]["result"], json!({}));

        let key = sent(
            &mut state,
            json!({"id": 5, "sessionId": session, "method": "Input.dispatchKeyEvent", "params": {"type": "keyDown", "text": "a"}}),
        );
        assert_eq!(key[0]["result"], json!({}));

        // F04-4: Fetch domain
        let fetch_enable = sent(
            &mut state,
            json!({"id": 6, "sessionId": session, "method": "Fetch.enable"}),
        );
        assert_eq!(fetch_enable[0]["result"], json!({}));

        let fetch_continue = sent(
            &mut state,
            json!({"id": 7, "sessionId": session, "method": "Fetch.continueRequest", "params": {"requestId": "req-1"}}),
        );
        assert_eq!(fetch_continue[0]["result"], json!({}));

        // F04-5: Memory & Performance domains
        let perf = sent(
            &mut state,
            json!({"id": 8, "sessionId": session, "method": "Performance.getMetrics"}),
        );
        assert!(perf[0]["result"]["metrics"].as_array().unwrap().len() >= 5);

        let mem = sent(
            &mut state,
            json!({"id": 9, "sessionId": session, "method": "Memory.getDOMCounters"}),
        );
        assert_eq!(mem[0]["result"]["documents"], 1);
    }

    #[test]
    fn unverified_domains_and_side_effect_preview_never_reach_webinspector() {
        let mut state = BrowserCdpState::new("ios-cli");
        state.refresh(&[page("PID:42", "1", WirType::WebPage)]);
        let session = state.register_attachment("PID:42:1").unwrap();
        for (index, method) in [
            "Page.setDocumentContent",
            "Input.synthesizePinchGesture",
            "Fetch.takeResponseBodyStream",
            "Memory.forciblyPurgeJavaScriptMemory",
            "Performance.setTimeDomain",
        ]
        .iter()
        .enumerate()
        {
            let request = json!({"id": index + 1, "sessionId": session, "method": method});
            let BrowserCdpAction::Send(messages) = state.handle(request) else {
                panic!("unverified method was forwarded: {method}");
            };
            assert_eq!(messages[0]["error"]["code"], -32601);
        }
        let preview = json!({"id": 20, "sessionId": session, "method": "Runtime.evaluate",
            "params": {"expression": "mutate()", "throwOnSideEffect": true}});
        let BrowserCdpAction::Send(messages) = state.handle(preview) else {
            panic!("unsafe preview was forwarded");
        };
        assert_eq!(messages[0]["error"]["code"], -32601);
    }

    #[test]
    fn js_context_console_mapping_and_destruction_events() {
        let mut adapter = JsContextCdpAdapter::default();
        let _ = adapter.outgoing(json!({"id": 1, "method": "Runtime.enable"}));
        let incoming = adapter.incoming(json!({"id": 1, "result": {}}));
        assert_eq!(incoming[1]["method"], "Runtime.executionContextCreated");

        let console_msg = adapter.incoming(json!({
            "method": "Console.messageAdded",
            "params": {"message": {"text": "hello from app", "level": "warn"}}
        }));
        assert_eq!(console_msg.len(), 2);
        assert_eq!(console_msg[1]["method"], "Runtime.consoleAPICalled");
        assert_eq!(console_msg[1]["params"]["type"], "warn");
        assert_eq!(
            console_msg[1]["params"]["args"][0]["value"],
            "hello from app"
        );

        let destroyed = adapter.context_destroyed().unwrap();
        assert_eq!(destroyed["method"], "Runtime.executionContextDestroyed");
        assert_eq!(destroyed["params"]["executionContextId"], 1);
        assert!(adapter.context_destroyed().is_none());

        let cleared = adapter.contexts_cleared();
        assert_eq!(cleared["method"], "Runtime.executionContextsCleared");
    }

    #[test]
    fn js_context_rejects_unknown_context_before_stripping_selector() {
        let adapter = JsContextCdpAdapter::default();
        let request = json!({"id": 5, "method": "Runtime.evaluate", "params": {
            "expression": "1+1", "contextId": 2
        }});
        assert_eq!(
            adapter.invalid_context_response(&request).unwrap()["error"]["code"],
            -32602
        );
        let valid = json!({"id": 6, "method": "Runtime.evaluate", "params": {
            "expression": "1+1", "contextId": 1
        }});
        assert!(adapter.invalid_context_response(&valid).is_none());
    }
}
