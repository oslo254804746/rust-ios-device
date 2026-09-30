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
    targets: IndexMap<String, Value>,
    browser_sessions: HashSet<String>,
    page_sessions: HashMap<String, String>, // CDP session ID -> qualified target ID
    target_sessions: HashMap<String, String>, // qualified target ID -> CDP session ID
    discover: bool,
    auto_attach: bool,
    events_session: Option<String>,
}

impl BrowserCdpState {
    pub fn new(browser_id: impl Into<String>) -> Self {
        Self {
            browser_id: browser_id.into(),
            targets: IndexMap::new(),
            browser_sessions: HashSet::new(),
            page_sessions: HashMap::new(),
            target_sessions: HashMap::new(),
            discover: false,
            auto_attach: false,
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
            .filter(|id| !self.target_sessions.contains_key(*id))
            .cloned()
            .collect()
    }

    pub fn target_session(&self, target_id: &str) -> Option<&str> {
        self.target_sessions.get(target_id).map(String::as_str)
    }

    pub fn target_for_session(&self, session_id: &str) -> Option<&str> {
        self.page_sessions.get(session_id).map(String::as_str)
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
            if self.auto_attach && !self.target_sessions.contains_key(target_id) {
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
                self.auto_attach = auto_attach;
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

/// The JavaScriptCore inspector has one implicit execution context and no
/// Target wrapper. Keep the small amount of state needed for that CDP view.
#[derive(Debug, Default)]
pub struct JsContextCdpAdapter {
    pending_runtime_enable: Option<u64>,
    context_announced: bool,
}

const JS_CONSOLE_ENABLE_ID: i64 = -0x4000_0000;

impl JsContextCdpAdapter {
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
        let mut messages = vec![message];
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
        messages
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
}
