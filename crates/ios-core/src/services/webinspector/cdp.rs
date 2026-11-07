//! Shared CDP bridge contracts for WebInspector page and browser sessions.
//!
//! This module consolidates the remaining F04 CDP bridge boundaries:
//! - Primitive-only `Console.messageAdded` (`console.log`/`info`/`warn`/`error`/`debug` without
//!   `objectId` parameters) synthesis of `Runtime.consoleAPICalled` to the active main-frame
//!   default (`normal`) `contextId`, while preserving `isolated world` and `injectedScriptId`
//!   generation/retirement semantics.
//! - Controlled `Input.dispatchMouseEvent`, `Input.dispatchKeyEvent`, `Input.insertText`, and
//!   `Input.dispatchTouchEvent` translation into verifiable page-target `Runtime.evaluate` DOM
//!   event synthesis expressions.
//! - Controlled `Browser.setDownloadBehavior` and `Page.setDownloadBehavior` policy validation
//!   and state tracking.
//! - `Performance.enable`, `Performance.disable`, and `Performance.getMetrics` runtime timing
//!   and heap metric synthesis contracts.

use std::collections::{HashMap, HashSet};
use std::time::{SystemTime, UNIX_EPOCH};

use serde_json::{json, Value};

pub use super::cdp_browser::{
    BrowserCdpAction, BrowserCdpRefresh, BrowserCdpState, BrowserDiscoveryKnowledge,
    JsContextCdpAdapter,
};
pub use super::cdp_page::{PageCdpAdapter, PageCdpOutput};

/// Controlled download behavior policy recorded for `Browser.setDownloadBehavior`
/// and `Page.setDownloadBehavior`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DownloadBehaviorPolicy {
    pub behavior: String,
    pub download_path: Option<String>,
    pub browser_context_id: Option<String>,
    pub events_enabled: bool,
}

/// Validate and normalize parameters for `Browser.setDownloadBehavior` or
/// `Page.setDownloadBehavior`.
pub fn validate_download_behavior_params(params: &Value) -> Result<DownloadBehaviorPolicy, String> {
    let behavior = params
        .get("behavior")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| "setDownloadBehavior requires a non-empty string behavior".to_string())?;

    if !matches!(behavior, "deny" | "allow" | "allowAndName" | "default") {
        return Err(format!(
            "unsupported setDownloadBehavior behavior `{behavior}`; expected deny, allow, allowAndName, or default"
        ));
    }

    let download_path = match params.get("downloadPath") {
        None | Some(Value::Null) => None,
        Some(Value::String(path)) => {
            if path.trim().is_empty() && matches!(behavior, "allow" | "allowAndName") {
                return Err(
                    "setDownloadBehavior downloadPath must not be empty when behavior allows downloads"
                        .to_string(),
                );
            }
            Some(path.clone())
        }
        Some(_) => {
            return Err("setDownloadBehavior downloadPath must be a string".to_string());
        }
    };

    let browser_context_id = match params.get("browserContextId") {
        None | Some(Value::Null) => None,
        Some(Value::String(id)) => Some(id.clone()),
        Some(_) => {
            return Err("setDownloadBehavior browserContextId must be a string".to_string());
        }
    };

    let events_enabled = match params.get("eventsEnabled") {
        None | Some(Value::Null) => false,
        Some(Value::Bool(flag)) => *flag,
        Some(_) => {
            return Err("setDownloadBehavior eventsEnabled must be a boolean".to_string());
        }
    };

    Ok(DownloadBehaviorPolicy {
        behavior: behavior.to_string(),
        download_path,
        browser_context_id,
        events_enabled,
    })
}

/// Returns whether a CDP `Input.*` method has a controlled page-target DOM synthesis contract.
pub fn is_supported_input_method(method: &str) -> bool {
    matches!(
        method,
        "Input.dispatchMouseEvent"
            | "Input.dispatchKeyEvent"
            | "Input.insertText"
            | "Input.dispatchTouchEvent"
    )
}

/// Returns whether a WebKit `Console.messageAdded` source/level pair corresponds to a
/// standard console API method (`log`, `info`, `warn`, `error`, `debug`) eligible for
/// primitive-only `Runtime.consoleAPICalled` synthesis.
pub fn is_primitive_console_api_message(source: &str, level: &str) -> bool {
    matches!(source, "console-api" | "javascript" | "other" | "")
        && matches!(
            level,
            "log" | "info" | "warning" | "warn" | "error" | "debug"
        )
}

/// Normalize WebKit console level strings into CDP `Runtime.consoleAPICalled` type values.
pub fn map_console_api_type(level: &str) -> &'static str {
    match level {
        "warning" | "warn" => "warning",
        "error" => "error",
        "debug" => "debug",
        "info" => "info",
        _ => "log",
    }
}

/// Returns true if all WebKit `Console.ChannelMessage.parameters` entries are primitive
/// remote objects (no `objectId` handle).
pub fn console_parameters_are_primitive_only(message: &Value) -> bool {
    let Some(parameters) = message.get("parameters").and_then(Value::as_array) else {
        return true;
    };
    parameters.iter().all(|param| {
        param
            .get("objectId")
            .and_then(Value::as_str)
            .map(str::trim)
            .unwrap_or("")
            .is_empty()
    })
}

/// Resolve a WebKit `Console.messageAdded` event into a CDP `Runtime.consoleAPICalled` event:
/// - When `objectId` parameters are present, extracts and validates `injectedScriptId` so
///   isolated worlds (`user` / `internal`) and `normal` worlds preserve exact realm and
///   retirement/reuse semantics.
/// - When parameters are primitive-only (no `objectId`) and the call is a standard
///   `console.log`/`info`/`warn`/`error`/`debug`, synthesizes `Runtime.consoleAPICalled`
///   against the active main-frame default (`normal`) `contextId`.
pub fn resolve_page_console_api_called(
    event: &Value,
    main_frame_id: Option<&str>,
    frame_contexts: &HashMap<String, i64>,
    announced_contexts: &HashMap<i64, String>,
    reused_realm_ids: &HashSet<i64>,
) -> Option<Value> {
    let message = &event["params"]["message"];
    if message["source"] != "console-api" {
        return None;
    }
    let message_type = message["type"].as_str()?;
    let kind = match message_type {
        "log" => match message["level"].as_str()? {
            "log" => "log",
            "info" => "info",
            "warning" | "warn" => "warning",
            "error" => "error",
            "debug" => "debug",
            _ => return None,
        },
        "trace" => "trace",
        "dir" => "dir",
        "dirxml" => "dirxml",
        "table" => "table",
        "clear" => "clear",
        "assert" => "assert",
        _ => return None,
    };
    let timestamp = message["timestamp"]
        .as_f64()
        .filter(|time| time.is_finite() && *time >= 0.0)?
        * 1000.0;
    if !timestamp.is_finite() {
        return None;
    }

    let mut object_context: Option<i64> = None;
    let mut saw_object_id = false;
    let args: Vec<Value> = if let Some(params) = message["parameters"]
        .as_array()
        .filter(|args| !args.is_empty())
    {
        for arg in params {
            if !arg.is_object() {
                return None;
            }
            if let Some(object_id) = arg.get("objectId") {
                saw_object_id = true;
                let object_id = object_id.as_str().filter(|value| value.len() <= 1024)?;
                let parsed: Value = serde_json::from_str(object_id).ok()?;
                let object = parsed.as_object()?;
                if object.len() != 2 || object.get("id")?.as_u64().is_none() {
                    return None;
                }
                let id = object.get("injectedScriptId")?.as_i64()?;
                if object_context.is_some_and(|previous| previous != id) {
                    return None;
                }
                object_context = Some(id);
            }
        }
        params.clone()
    } else {
        let text = message["text"].as_str()?;
        vec![json!({
            "type": "string",
            "value": text,
        })]
    };

    let context = if saw_object_id {
        object_context?
    } else {
        if message_type != "log" || !matches!(kind, "log" | "info" | "warning" | "error" | "debug")
        {
            return None;
        }
        let main_frame = main_frame_id?;
        *frame_contexts.get(main_frame)?
    };

    if !announced_contexts.contains_key(&context) || reused_realm_ids.contains(&context) {
        return None;
    }

    Some(json!({
        "method": "Runtime.consoleAPICalled",
        "params": {
            "type": kind,
            "args": args,
            "executionContextId": context,
            "timestamp": timestamp
        }
    }))
}

/// Synthesize a CDP `Runtime.consoleAPICalled` event payload for a WebKit `Console.messageAdded`
/// message bound to a verified execution context ID.
pub fn synthesize_console_api_called_event(message: &Value, context_id: u64) -> Value {
    let level = message
        .get("level")
        .and_then(Value::as_str)
        .unwrap_or("log");
    let cdp_type = map_console_api_type(level);
    let text = message
        .get("text")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let args = message
        .get("parameters")
        .and_then(Value::as_array)
        .filter(|params| !params.is_empty())
        .cloned()
        .unwrap_or_else(|| {
            vec![json!({
                "type": "string",
                "value": text,
            })]
        });
    let timestamp = message
        .get("timestamp")
        .and_then(Value::as_f64)
        .map(|ts| if ts < 1.0e12 { ts * 1000.0 } else { ts })
        .unwrap_or_else(now_epoch_millis);

    let mut params = json!({
        "type": cdp_type,
        "args": args,
        "executionContextId": context_id,
        "timestamp": timestamp,
    });
    if let Some(stack_trace) = build_console_stack_trace(message) {
        if let Some(obj) = params.as_object_mut() {
            obj.insert("stackTrace".to_string(), stack_trace);
        }
    }

    json!({
        "method": "Runtime.consoleAPICalled",
        "params": params,
    })
}

fn build_console_stack_trace(message: &Value) -> Option<Value> {
    if let Some(frames) = message.get("stackTrace").and_then(|st| {
        st.get("callFrames")
            .and_then(Value::as_array)
            .or_else(|| st.as_array())
    }) {
        let mapped: Vec<Value> = frames
            .iter()
            .map(|frame| {
                json!({
                    "functionName": frame.get("functionName").and_then(Value::as_str).unwrap_or(""),
                    "scriptId": frame.get("scriptId").and_then(Value::as_str).unwrap_or("0"),
                    "url": frame.get("url").and_then(Value::as_str).unwrap_or(""),
                    "lineNumber": frame.get("lineNumber").and_then(Value::as_i64).unwrap_or(0).saturating_sub(1).max(0),
                    "columnNumber": frame.get("columnNumber").and_then(Value::as_i64).unwrap_or(0).saturating_sub(1).max(0),
                })
            })
            .collect();
        return Some(json!({ "callFrames": mapped }));
    }
    None
}

fn now_epoch_millis() -> f64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs_f64() * 1000.0)
        .unwrap_or(1_700_000_000_000.0)
}

/// Build a controlled, self-contained JavaScript expression that dispatches DOM input events
/// inside the active page context for `Input.dispatchMouseEvent`, `Input.dispatchKeyEvent`,
/// `Input.insertText`, and `Input.dispatchTouchEvent`.
pub fn build_input_dispatch_expression(method: &str, params: &Value) -> Result<String, String> {
    match method {
        "Input.dispatchMouseEvent" => build_mouse_event_expression(params),
        "Input.dispatchKeyEvent" => build_key_event_expression(params),
        "Input.insertText" => build_insert_text_expression(params),
        "Input.dispatchTouchEvent" => build_touch_event_expression(params),
        _ => Err(format!("unsupported CDP Input method `{method}`")),
    }
}

fn build_mouse_event_expression(params: &Value) -> Result<String, String> {
    let event_type = params
        .get("type")
        .and_then(Value::as_str)
        .ok_or_else(|| "Input.dispatchMouseEvent requires a string `type` parameter".to_string())?;
    if !matches!(
        event_type,
        "mousePressed" | "mouseReleased" | "mouseMoved" | "mouseWheel"
    ) {
        return Err(format!(
            "Input.dispatchMouseEvent unsupported type `{event_type}`"
        ));
    }

    let x = params
        .get("x")
        .and_then(Value::as_f64)
        .filter(|v| v.is_finite())
        .ok_or_else(|| {
            "Input.dispatchMouseEvent requires a finite numeric `x` parameter".to_string()
        })?;
    let y = params
        .get("y")
        .and_then(Value::as_f64)
        .filter(|v| v.is_finite())
        .ok_or_else(|| {
            "Input.dispatchMouseEvent requires a finite numeric `y` parameter".to_string()
        })?;

    let button_name = params
        .get("button")
        .and_then(Value::as_str)
        .unwrap_or("none");
    let button_code: i64 = match button_name {
        "none" | "left" => 0,
        "middle" => 1,
        "right" => 2,
        "back" => 3,
        "forward" => 4,
        other => {
            return Err(format!(
                "Input.dispatchMouseEvent unsupported button `{other}`"
            ));
        }
    };
    let buttons = params.get("buttons").and_then(Value::as_u64).unwrap_or({
        match (event_type, button_name) {
            ("mousePressed", "left") => 1,
            ("mousePressed", "right") => 2,
            ("mousePressed", "middle") => 4,
            _ => 0,
        }
    });
    let modifiers = params.get("modifiers").and_then(Value::as_u64).unwrap_or(0);
    let click_count = params.get("clickCount").and_then(Value::as_u64).unwrap_or(
        if matches!(event_type, "mousePressed" | "mouseReleased") {
            1
        } else {
            0
        },
    );
    let delta_x = params.get("deltaX").and_then(Value::as_f64).unwrap_or(0.0);
    let delta_y = params.get("deltaY").and_then(Value::as_f64).unwrap_or(0.0);

    let payload = json!({
        "type": event_type,
        "x": x,
        "y": y,
        "button": button_code,
        "buttonName": button_name,
        "buttons": buttons,
        "modifiers": modifiers,
        "clickCount": click_count,
        "deltaX": delta_x,
        "deltaY": delta_y,
    });

    Ok(format!(
        "/*cdp-input:mouse*/(() => {{
            const p = {payload};
            const doc = typeof document !== 'undefined' ? document : null;
            const target = (doc && typeof doc.elementFromPoint === 'function' && doc.elementFromPoint(p.x, p.y))
                || (doc && (doc.body || doc.documentElement))
                || (typeof window !== 'undefined' ? window : null);
            const init = {{
                bubbles: true,
                cancelable: true,
                composed: true,
                clientX: p.x,
                clientY: p.y,
                screenX: p.x,
                screenY: p.y,
                button: p.button,
                buttons: p.buttons,
                detail: p.clickCount,
                altKey: !!(p.modifiers & 1),
                ctrlKey: !!(p.modifiers & 2),
                metaKey: !!(p.modifiers & 4),
                shiftKey: !!(p.modifiers & 8)
            }};
            if (target && typeof target.dispatchEvent === 'function') {{
                if (p.type === 'mouseMoved') {{
                    if (typeof PointerEvent === 'function') target.dispatchEvent(new PointerEvent('pointermove', init));
                    if (typeof MouseEvent === 'function') target.dispatchEvent(new MouseEvent('mousemove', init));
                }} else if (p.type === 'mousePressed') {{
                    if (typeof PointerEvent === 'function') target.dispatchEvent(new PointerEvent('pointerdown', init));
                    if (typeof MouseEvent === 'function') target.dispatchEvent(new MouseEvent('mousedown', init));
                    if (typeof target.focus === 'function') {{ try {{ target.focus(); }} catch (_) {{}} }}
                }} else if (p.type === 'mouseReleased') {{
                    if (typeof PointerEvent === 'function') target.dispatchEvent(new PointerEvent('pointerup', init));
                    if (typeof MouseEvent === 'function') target.dispatchEvent(new MouseEvent('mouseup', init));
                    if (p.clickCount >= 1 && (p.buttonName === 'left' || p.buttonName === 'none') && typeof MouseEvent === 'function') {{
                        target.dispatchEvent(new MouseEvent('click', init));
                        if (p.clickCount === 2) target.dispatchEvent(new MouseEvent('dblclick', init));
                    }}
                }} else if (p.type === 'mouseWheel') {{
                    if (typeof WheelEvent === 'function') {{
                        target.dispatchEvent(new WheelEvent('wheel', Object.assign({{ deltaX: p.deltaX, deltaY: p.deltaY }}, init)));
                    }}
                }}
            }}
            return true;
        }})()"
    ))
}

fn build_key_event_expression(params: &Value) -> Result<String, String> {
    let event_type = params
        .get("type")
        .and_then(Value::as_str)
        .ok_or_else(|| "Input.dispatchKeyEvent requires a string `type` parameter".to_string())?;
    if !matches!(event_type, "keyDown" | "keyUp" | "rawKeyDown" | "char") {
        return Err(format!(
            "Input.dispatchKeyEvent unsupported type `{event_type}`"
        ));
    }

    let key = params.get("key").and_then(Value::as_str).unwrap_or("");
    let code = params.get("code").and_then(Value::as_str).unwrap_or("");
    let text = params
        .get("text")
        .and_then(Value::as_str)
        .or_else(|| params.get("unmodifiedText").and_then(Value::as_str))
        .unwrap_or("");
    let key_code = params
        .get("windowsVirtualKeyCode")
        .and_then(Value::as_u64)
        .or_else(|| params.get("nativeVirtualKeyCode").and_then(Value::as_u64))
        .unwrap_or(0);
    let modifiers = params.get("modifiers").and_then(Value::as_u64).unwrap_or(0);

    let payload = json!({
        "type": event_type,
        "key": key,
        "code": code,
        "text": text,
        "keyCode": key_code,
        "modifiers": modifiers,
    });

    Ok(format!(
        "/*cdp-input:key*/(() => {{
            const p = {payload};
            const doc = typeof document !== 'undefined' ? document : null;
            const target = (doc && (doc.activeElement || doc.body || doc.documentElement))
                || (typeof window !== 'undefined' ? window : null);
            const domType = p.type === 'keyUp' ? 'keyup' : (p.type === 'char' ? 'keypress' : 'keydown');
            const init = {{
                bubbles: true,
                cancelable: true,
                composed: true,
                key: p.key || p.text || '',
                code: p.code || '',
                keyCode: p.keyCode,
                which: p.keyCode,
                altKey: !!(p.modifiers & 1),
                ctrlKey: !!(p.modifiers & 2),
                metaKey: !!(p.modifiers & 4),
                shiftKey: !!(p.modifiers & 8)
            }};
            if (target && typeof target.dispatchEvent === 'function' && typeof KeyboardEvent === 'function') {{
                target.dispatchEvent(new KeyboardEvent(domType, init));
            }}
            if ((p.type === 'char' || (p.type === 'keyDown' && p.text)) && p.text && target) {{
                if (typeof target.value === 'string') {{
                    target.value += p.text;
                }}
                if (typeof target.dispatchEvent === 'function' && typeof Event === 'function') {{
                    const ev = typeof InputEvent === 'function'
                        ? new InputEvent('input', {{ bubbles: true, data: p.text, inputType: 'insertText' }})
                        : new Event('input', {{ bubbles: true }});
                    target.dispatchEvent(ev);
                }}
            }}
            return true;
        }})()"
    ))
}

fn build_insert_text_expression(params: &Value) -> Result<String, String> {
    let text = params
        .get("text")
        .and_then(Value::as_str)
        .ok_or_else(|| "Input.insertText requires a string `text` parameter".to_string())?;
    let payload = json!({ "text": text });

    Ok(format!(
        "/*cdp-input:insertText*/(() => {{
            const p = {payload};
            const doc = typeof document !== 'undefined' ? document : null;
            const target = (doc && (doc.activeElement || doc.body || doc.documentElement))
                || (typeof window !== 'undefined' ? window : null);
            if (target) {{
                if (typeof target.value === 'string') {{
                    const start = typeof target.selectionStart === 'number' ? target.selectionStart : target.value.length;
                    const end = typeof target.selectionEnd === 'number' ? target.selectionEnd : target.value.length;
                    target.value = target.value.slice(0, start) + p.text + target.value.slice(end);
                }} else if (target.isContentEditable && doc && typeof doc.execCommand === 'function') {{
                    try {{ doc.execCommand('insertText', false, p.text); }} catch (_) {{}}
                }}
                if (typeof target.dispatchEvent === 'function' && typeof Event === 'function') {{
                    const ev = typeof InputEvent === 'function'
                        ? new InputEvent('input', {{ bubbles: true, data: p.text, inputType: 'insertText' }})
                        : new Event('input', {{ bubbles: true }});
                    target.dispatchEvent(ev);
                }}
            }}
            return true;
        }})()"
    ))
}

fn build_touch_event_expression(params: &Value) -> Result<String, String> {
    let event_type = params
        .get("type")
        .and_then(Value::as_str)
        .ok_or_else(|| "Input.dispatchTouchEvent requires a string `type` parameter".to_string())?;
    if !matches!(
        event_type,
        "touchStart" | "touchEnd" | "touchMove" | "touchCancel"
    ) {
        return Err(format!(
            "Input.dispatchTouchEvent unsupported type `{event_type}`"
        ));
    }

    let touch_points = params
        .get("touchPoints")
        .and_then(Value::as_array)
        .ok_or_else(|| {
            "Input.dispatchTouchEvent requires an array `touchPoints` parameter".to_string()
        })?;
    if matches!(event_type, "touchStart" | "touchMove") && touch_points.is_empty() {
        return Err(format!(
            "Input.dispatchTouchEvent `{event_type}` requires at least one touchPoint"
        ));
    }

    let mut normalized_points = Vec::with_capacity(touch_points.len());
    for (idx, point) in touch_points.iter().enumerate() {
        let x = point
            .get("x")
            .and_then(Value::as_f64)
            .filter(|v| v.is_finite())
            .ok_or_else(|| {
                format!("Input.dispatchTouchEvent touchPoints[{idx}] requires a finite numeric `x`")
            })?;
        let y = point
            .get("y")
            .and_then(Value::as_f64)
            .filter(|v| v.is_finite())
            .ok_or_else(|| {
                format!("Input.dispatchTouchEvent touchPoints[{idx}] requires a finite numeric `y`")
            })?;
        let id = point
            .get("id")
            .and_then(Value::as_i64)
            .unwrap_or(idx as i64);
        normalized_points.push(json!({
            "x": x,
            "y": y,
            "id": id,
        }));
    }

    let modifiers = params.get("modifiers").and_then(Value::as_u64).unwrap_or(0);
    let payload = json!({
        "type": event_type,
        "touchPoints": normalized_points,
        "modifiers": modifiers,
    });

    Ok(format!(
        "/*cdp-input:touch*/(() => {{
            const p = {payload};
            const doc = typeof document !== 'undefined' ? document : null;
            const first = p.touchPoints[0] || {{ x: 0, y: 0, id: 0 }};
            const target = (doc && typeof doc.elementFromPoint === 'function' && doc.elementFromPoint(first.x, first.y))
                || (doc && (doc.body || doc.documentElement))
                || (typeof window !== 'undefined' ? window : null);
            const domType = p.type === 'touchStart' ? 'touchstart'
                : (p.type === 'touchMove' ? 'touchmove'
                : (p.type === 'touchCancel' ? 'touchcancel' : 'touchend'));
            const pointerType = p.type === 'touchStart' ? 'pointerdown'
                : (p.type === 'touchMove' ? 'pointermove'
                : (p.type === 'touchCancel' ? 'pointercancel' : 'pointerup'));
            if (target && typeof target.dispatchEvent === 'function') {{
                if (typeof PointerEvent === 'function') {{
                    target.dispatchEvent(new PointerEvent(pointerType, {{
                        bubbles: true,
                        cancelable: true,
                        composed: true,
                        pointerId: first.id + 1,
                        pointerType: 'touch',
                        isPrimary: true,
                        clientX: first.x,
                        clientY: first.y
                    }}));
                }}
                if (typeof Event === 'function') {{
                    target.dispatchEvent(new Event(domType, {{ bubbles: true, cancelable: true }}));
                }}
            }}
            return true;
        }})()"
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn download_behavior_validation_accepts_controlled_modes_and_rejects_invalid_params() {
        let allow = validate_download_behavior_params(&json!({
            "behavior": "allowAndName",
            "downloadPath": "C:/tmp/downloads",
            "eventsEnabled": true
        }))
        .expect("allowAndName should be accepted");
        assert_eq!(allow.behavior, "allowAndName");
        assert_eq!(allow.download_path.as_deref(), Some("C:/tmp/downloads"));
        assert!(allow.events_enabled);

        let deny = validate_download_behavior_params(&json!({
            "behavior": "deny"
        }))
        .expect("deny should be accepted");
        assert_eq!(deny.behavior, "deny");
        assert!(deny.download_path.is_none());

        assert!(validate_download_behavior_params(&json!({})).is_err());
        assert!(validate_download_behavior_params(&json!({ "behavior": "unknown" })).is_err());
        assert!(validate_download_behavior_params(&json!({
            "behavior": "allow",
            "downloadPath": "   "
        }))
        .is_err());
    }

    #[test]
    fn input_dispatch_expressions_validate_parameters_and_tag_commands() {
        let mouse = build_input_dispatch_expression(
            "Input.dispatchMouseEvent",
            &json!({ "type": "mousePressed", "x": 24, "y": 48, "button": "left", "clickCount": 1 }),
        )
        .expect("mouse expression");
        assert!(mouse.starts_with("/*cdp-input:mouse*/"));

        let key = build_input_dispatch_expression(
            "Input.dispatchKeyEvent",
            &json!({ "type": "keyDown", "key": "a", "text": "a" }),
        )
        .expect("key expression");
        assert!(key.starts_with("/*cdp-input:key*/"));

        let insert =
            build_input_dispatch_expression("Input.insertText", &json!({ "text": "hello" }))
                .expect("insertText expression");
        assert!(insert.starts_with("/*cdp-input:insertText*/"));

        let touch = build_input_dispatch_expression(
            "Input.dispatchTouchEvent",
            &json!({
                "type": "touchStart",
                "touchPoints": [{ "x": 12.0, "y": 34.0 }]
            }),
        )
        .expect("touch expression");
        assert!(touch.starts_with("/*cdp-input:touch*/"));

        assert!(build_input_dispatch_expression(
            "Input.dispatchMouseEvent",
            &json!({ "type": "mouseMoved" })
        )
        .is_err());
        assert!(build_input_dispatch_expression(
            "Input.dispatchTouchEvent",
            &json!({ "type": "touchStart", "touchPoints": [] })
        )
        .is_err());
    }

    #[test]
    fn primitive_console_api_synthesis_binds_to_active_main_frame_default_context() {
        let mut frame_contexts = HashMap::new();
        frame_contexts.insert("main".to_string(), 31);
        frame_contexts.insert("child".to_string(), 41);
        let mut announced_contexts = HashMap::new();
        announced_contexts.insert(31, "main".to_string());
        announced_contexts.insert(32, "main".to_string()); // isolated world on main
        announced_contexts.insert(41, "child".to_string());
        let reused = HashSet::new();

        for (level, expected_type) in [
            ("log", "log"),
            ("info", "info"),
            ("warning", "warning"),
            ("warn", "warning"),
            ("error", "error"),
            ("debug", "debug"),
        ] {
            let event = json!({
                "method": "Console.messageAdded",
                "params": {
                    "message": {
                        "source": "console-api",
                        "type": "log",
                        "level": level,
                        "text": "primitive-msg",
                        "timestamp": 1700000000.5,
                        "parameters": [
                            { "type": "string", "value": "primitive-msg" },
                            { "type": "number", "value": 99 }
                        ]
                    }
                }
            });
            let synthesized = resolve_page_console_api_called(
                &event,
                Some("main"),
                &frame_contexts,
                &announced_contexts,
                &reused,
            )
            .expect("primitive console message should synthesize to main default context");
            assert_eq!(synthesized["method"], "Runtime.consoleAPICalled");
            assert_eq!(synthesized["params"]["type"], expected_type);
            assert_eq!(synthesized["params"]["executionContextId"], 31);
            assert_eq!(synthesized["params"]["timestamp"], 1_700_000_000_500.0);
        }

        // Isolated world objectId still binds to its isolated context (32) rather than main default (31).
        let isolated_obj_event = json!({
            "method": "Console.messageAdded",
            "params": {
                "message": {
                    "source": "console-api",
                    "type": "log",
                    "level": "log",
                    "text": "isolated-obj",
                    "timestamp": 12.5,
                    "parameters": [
                        { "type": "object", "objectId": "{\"injectedScriptId\":32,\"id\":1}" }
                    ]
                }
            }
        });
        let isolated = resolve_page_console_api_called(
            &isolated_obj_event,
            Some("main"),
            &frame_contexts,
            &announced_contexts,
            &reused,
        )
        .expect("object console message should bind to isolated world");
        assert_eq!(isolated["params"]["executionContextId"], 32);
    }
}
