//! Web viewer HTTP / WebSocket application, interactive canvas, and media stream (F05-1).
#![allow(dead_code)]

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;

use anyhow::Result;
use axum::extract::{Json, Request, State};
use axum::http::{header, StatusCode};
use axum::response::{Html, Response};
use axum::routing::{get, post};
use axum::{Extension, Router};
use serde::{Deserialize, Serialize};
use tokio::sync::{mpsc, oneshot, watch, Mutex, Notify, RwLock, Semaphore};

use super::clipboard::{
    ClipboardPayload, ClipboardSource, ClipboardSyncState, MAX_IMAGE_BYTES, MAX_TEXT_BYTES,
    UTI_PLAIN_TEXT,
};
use super::vnc::{
    VncInputAction, BTN_HOME, BTN_LOCK, BTN_MUTE, BTN_SIRI, BTN_VOL_DOWN, BTN_VOL_UP,
};
use crate::access::{AccessControl, Admission, Permission, Principal};

pub const MAX_WEB_FRAME_BYTES: usize = super::media::MAX_PNG_BYTES;

#[derive(Debug, Clone)]
pub struct WebFrame {
    pub sequence: u64,
    pub mime_type: &'static str,
    pub data: bytes::Bytes,
    pub width: u32,
    pub height: u32,
    pub timestamp_ms: u64,
    pub timestamp_clock: &'static str,
}

pub struct WebViewerState {
    sequence: AtomicU64,
    pub frame: RwLock<Option<WebFrame>>,
    pub new_frame_notify: Arc<Notify>,
    pub backend: RwLock<Option<mpsc::Sender<DeviceRequest>>>,
    pub clipboard_sync: Mutex<ClipboardSyncState>,
    pub access: AccessControl,
    pub udid: String,
    pub share_clipboard: bool,
    pub port: u16,
    pub clients: Arc<Semaphore>,
    pub stopped: AtomicBool,
    pub stop_notify: Arc<Notify>,
    pub stop_signal: watch::Sender<bool>,
}

impl WebViewerState {
    pub fn new(access: AccessControl, udid: String, port: u16, share_clipboard: bool) -> Arc<Self> {
        let (stop_signal, _) = watch::channel(false);
        Arc::new(Self {
            sequence: AtomicU64::new(0),
            frame: RwLock::new(None),
            new_frame_notify: Arc::new(Notify::new()),
            backend: RwLock::new(None),
            clipboard_sync: Mutex::new(ClipboardSyncState::new()),
            access,
            udid,
            share_clipboard,
            port,
            clients: Arc::new(Semaphore::new(2)),
            stopped: AtomicBool::new(false),
            stop_notify: Arc::new(Notify::new()),
            stop_signal,
        })
    }

    pub async fn push_frame(
        &self,
        mime: &'static str,
        data: impl Into<bytes::Bytes>,
        width: u32,
        height: u32,
        ts: u64,
    ) {
        self.push_timed_frame(mime, data, width, height, ts, "host-unix-ms")
            .await;
    }

    pub async fn push_timed_frame(
        &self,
        mime: &'static str,
        data: impl Into<bytes::Bytes>,
        width: u32,
        height: u32,
        ts: u64,
        clock: &'static str,
    ) {
        let data = data.into();
        if data.len() > MAX_WEB_FRAME_BYTES {
            tracing::warn!(
                len = data.len(),
                "frame exceeds web viewer memory budget; dropped"
            );
            return;
        }
        let frame = WebFrame {
            sequence: self.sequence.fetch_add(1, Ordering::Relaxed),
            mime_type: mime,
            data,
            width,
            height,
            timestamp_ms: ts,
            timestamp_clock: clock,
        };
        {
            let mut guard = self.frame.write().await;
            *guard = Some(frame);
        }
        self.new_frame_notify.notify_waiters();
    }

    pub fn is_stopped(&self) -> bool {
        self.stopped.load(Ordering::SeqCst)
    }

    pub fn trigger_stop(&self) {
        self.stopped.store(true, Ordering::SeqCst);
        self.stop_notify.notify_waiters();
        self.stop_signal.send_replace(true);
    }

    async fn submit(
        &self,
        command: DeviceCommand,
        principal: Principal,
    ) -> Result<Option<ClipboardPayload>, StatusCode> {
        principal
            .require(Permission::Debug, Some(&self.udid), None)
            .map_err(|_| StatusCode::FORBIDDEN)?;
        principal
            .require(Permission::Connect, Some(&self.udid), Some(self.port))
            .map_err(|_| StatusCode::FORBIDDEN)?;
        let backend = self
            .backend
            .read()
            .await
            .clone()
            .ok_or(StatusCode::SERVICE_UNAVAILABLE)?;
        let (tx, rx) = oneshot::channel();
        backend
            .try_send(DeviceRequest {
                command,
                principal: principal.clone(),
                reply: tx,
            })
            .map_err(|_| StatusCode::SERVICE_UNAVAILABLE)?;
        let result = principal
            .run(tokio::time::timeout(super::device::OPERATION_TIMEOUT, rx))
            .await
            .map_err(|_| StatusCode::FORBIDDEN)?
            .map_err(|_| StatusCode::GATEWAY_TIMEOUT)?
            .map_err(|_| StatusCode::SERVICE_UNAVAILABLE)?;
        result.map_err(|error| {
            tracing::debug!(error = %error, "viewer device operation failed");
            StatusCode::SERVICE_UNAVAILABLE
        })
    }
}

pub enum DeviceCommand {
    Input(VncInputAction),
    ReadClipboard,
}
pub struct DeviceRequest {
    pub command: DeviceCommand,
    pub principal: Principal,
    pub reply: oneshot::Sender<anyhow::Result<Option<ClipboardPayload>>>,
}

pub fn create_viewer_router(state: Arc<WebViewerState>) -> Router {
    let admission = Admission::debug(
        state.access.clone(),
        crate::http_diagnostics::ErrorShape::Manager,
        state.udid.clone(),
    );
    let protected = Router::new()
        .route("/", get(handle_index))
        .route("/frame", get(handle_frame))
        .route("/stream", get(handle_stream))
        .route("/stop", post(handle_stop))
        .route("/input/pointer", post(handle_pointer))
        .route("/input/button", post(handle_button))
        .route("/input/key", post(handle_key))
        .route("/orientation", post(handle_orientation))
        .route(
            "/clipboard",
            get(handle_get_clipboard).post(handle_set_clipboard),
        )
        .route("/auth/reload", post(handle_reload))
        .with_state(state.clone())
        .layer(axum::middleware::from_fn_with_state(state, viewer_scope))
        .merge(admission.ticket_routes())
        .layer(axum::middleware::from_fn_with_state(
            admission,
            crate::access::admission,
        ));
    // This page contains no device state. Every frame/control call still passes admission.
    Router::new()
        .route("/login", get(|| async { Html(VIEWER_HTML) }))
        .merge(protected)
}

async fn handle_frame(
    State(state): State<Arc<WebViewerState>>,
    Extension(_principal): Extension<Principal>,
) -> Result<Response, StatusCode> {
    let frame = state
        .frame
        .read()
        .await
        .clone()
        .ok_or(StatusCode::SERVICE_UNAVAILABLE)?;
    Response::builder()
        .header(header::CONTENT_TYPE, frame.mime_type)
        .header("X-Timestamp-Clock", frame.timestamp_clock)
        .header("X-Timestamp", frame.timestamp_ms.to_string())
        .header(header::CACHE_CONTROL, "no-store")
        .body(axum::body::Body::from(frame.data))
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)
}

async fn viewer_scope(
    State(state): State<Arc<WebViewerState>>,
    request: Request,
    next: axum::middleware::Next,
) -> Result<Response, StatusCode> {
    if request.uri().path() != "/auth/reload" {
        let principal = request
            .extensions()
            .get::<Principal>()
            .ok_or(StatusCode::UNAUTHORIZED)?;
        principal
            .require(Permission::Connect, Some(&state.udid), Some(state.port))
            .map_err(|_| StatusCode::FORBIDDEN)?;
    }
    Ok(next.run(request).await)
}

async fn handle_reload(
    State(state): State<Arc<WebViewerState>>,
    Extension(principal): Extension<Principal>,
) -> Result<StatusCode, StatusCode> {
    state
        .access
        .reload(&principal)
        .await
        .map_err(|_| StatusCode::FORBIDDEN)?;
    Ok(StatusCode::NO_CONTENT)
}

// -----------------------------------------------------------------------------
// Route Handlers
// -----------------------------------------------------------------------------

async fn handle_index(
    State(state): State<Arc<WebViewerState>>,
    Extension(_principal): Extension<Principal>,
) -> Result<Html<&'static str>, StatusCode> {
    let _ = state;
    Ok(Html(VIEWER_HTML))
}

async fn handle_stream(
    State(state): State<Arc<WebViewerState>>,
    Extension(principal): Extension<Principal>,
) -> Result<Response, StatusCode> {
    let permit = state
        .clients
        .clone()
        .try_acquire_owned()
        .map_err(|_| StatusCode::TOO_MANY_REQUESTS)?;
    let boundary = "frame";
    // The response owns the stream and its permit. No detached producer or per-client pixel copies.
    let body = futures_util::stream::unfold(
        (
            state,
            principal,
            permit,
            None,
            std::collections::VecDeque::<bytes::Bytes>::new(),
        ),
        |(state, principal, permit, mut last, mut parts)| async move {
            loop {
                if state.is_stopped() || !principal.current() {
                    return None;
                }
                if let Some(part) = parts.pop_front() {
                    return Some((
                        Ok::<_, std::io::Error>(part),
                        (state, principal, permit, last, parts),
                    ));
                }
                if let Some(frame) = state.frame.read().await.clone() {
                    if last != Some(frame.sequence) {
                        last = Some(frame.sequence);
                        parts.push_back(bytes::Bytes::from(format!("--frame\r\nContent-Type: {}\r\nContent-Length: {}\r\nX-Timestamp: {}\r\nX-Timestamp-Clock: {}\r\n\r\n", frame.mime_type, frame.data.len(), frame.timestamp_ms,frame.timestamp_clock)));
                        parts.push_back(frame.data);
                        parts.push_back(bytes::Bytes::from_static(b"\r\n"));
                        continue;
                    }
                }
                let wait = async {
                    tokio::select! { _ = state.new_frame_notify.notified() => {}, _ = state.stop_notify.notified() => {}, _ = tokio::time::sleep(Duration::from_millis(100)) => {} }
                };
                if principal.run(wait).await.is_err() {
                    return None;
                }
            }
        },
    );

    let response = Response::builder()
        .status(StatusCode::OK)
        .header(
            header::CONTENT_TYPE,
            format!("multipart/x-mixed-replace; boundary={boundary}"),
        )
        .header(header::CACHE_CONTROL, "no-cache, no-store, must-revalidate")
        .body(axum::body::Body::from_stream(body))
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    Ok(response)
}

async fn handle_stop(
    State(state): State<Arc<WebViewerState>>,
    Extension(_principal): Extension<Principal>,
) -> Result<Json<serde_json::Value>, StatusCode> {
    state.trigger_stop();
    Ok(Json(serde_json::json!({
        "status": "stopped",
        "message": "web viewer shutdown requested"
    })))
}

#[derive(Deserialize)]
struct PointerRequest {
    x: u16,
    y: u16,
    action: String, // "down", "up", "move"
}

async fn handle_pointer(
    State(state): State<Arc<WebViewerState>>,
    Extension(principal): Extension<Principal>,
    Json(req): Json<PointerRequest>,
) -> Result<StatusCode, StatusCode> {
    let contact = match req.action.as_str() {
        "down" | "move" => true,
        "up" => false,
        _ => return Err(StatusCode::BAD_REQUEST),
    };
    state
        .submit(
            DeviceCommand::Input(VncInputAction::Touch {
                contact,
                x: req.x,
                y: req.y,
            }),
            principal,
        )
        .await?;
    Ok(StatusCode::OK)
}

#[derive(Deserialize)]
struct ButtonRequest {
    name: String, // "home", "lock", "vol_up", "vol_down", "mute", "siri"
}

async fn handle_button(
    State(state): State<Arc<WebViewerState>>,
    Extension(principal): Extension<Principal>,
    Json(req): Json<ButtonRequest>,
) -> Result<StatusCode, StatusCode> {
    let (page, code) = match req.name.to_lowercase().as_str() {
        "home" => BTN_HOME,
        "lock" => BTN_LOCK,
        "vol_up" => BTN_VOL_UP,
        "vol_down" => BTN_VOL_DOWN,
        "mute" => BTN_MUTE,
        "siri" => BTN_SIRI,
        _ => return Err(StatusCode::BAD_REQUEST),
    };

    state
        .submit(
            DeviceCommand::Input(VncInputAction::Button {
                page,
                code,
                down: true,
            }),
            principal.clone(),
        )
        .await?;
    tokio::time::sleep(Duration::from_millis(50)).await;
    state
        .submit(
            DeviceCommand::Input(VncInputAction::Button {
                page,
                code,
                down: false,
            }),
            principal,
        )
        .await?;
    Ok(StatusCode::OK)
}

#[derive(Deserialize)]
struct KeyRequest {
    usage: u8,
    down: bool,
}

async fn handle_key(
    State(state): State<Arc<WebViewerState>>,
    Extension(principal): Extension<Principal>,
    Json(req): Json<KeyRequest>,
) -> Result<StatusCode, StatusCode> {
    state
        .submit(
            DeviceCommand::Input(VncInputAction::Key {
                usage: req.usage,
                down: req.down,
            }),
            principal,
        )
        .await?;
    Ok(StatusCode::OK)
}

#[derive(Deserialize)]
struct OrientationRequest {
    direction: String, // "left", "right"
}

async fn handle_orientation(
    State(state): State<Arc<WebViewerState>>,
    Extension(principal): Extension<Principal>,
    Json(req): Json<OrientationRequest>,
) -> Result<StatusCode, StatusCode> {
    let dir = match req.direction.as_str() {
        "left" => "left",
        "right" => "right",
        _ => return Err(StatusCode::BAD_REQUEST),
    };
    state
        .submit(
            DeviceCommand::Input(super::vnc::VncInputAction::Rotate { direction: dir }),
            principal,
        )
        .await?;
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Serialize)]
struct ClipboardResponse {
    text: Option<String>,
    uti: String,
    data_base64: Option<String>,
}

async fn handle_get_clipboard(
    State(state): State<Arc<WebViewerState>>,
    Extension(principal): Extension<Principal>,
) -> Result<Json<ClipboardResponse>, StatusCode> {
    if !state.share_clipboard {
        return Err(StatusCode::FORBIDDEN);
    }
    let (text, uti, data_base64) = match state
        .submit(DeviceCommand::ReadClipboard, principal)
        .await?
    {
        Some(ClipboardPayload::Text(text)) => (Some(text), UTI_PLAIN_TEXT.to_string(), None),
        Some(ClipboardPayload::ImagePng(png)) => (
            None,
            super::clipboard::UTI_PNG.to_string(),
            Some(base64::Engine::encode(
                &base64::engine::general_purpose::STANDARD,
                &png,
            )),
        ),
        Some(ClipboardPayload::ImageJpeg(jpeg)) => (
            None,
            super::clipboard::UTI_JPEG.to_string(),
            Some(base64::Engine::encode(
                &base64::engine::general_purpose::STANDARD,
                &jpeg,
            )),
        ),
        None => (None, UTI_PLAIN_TEXT.to_string(), None),
    };
    Ok(Json(ClipboardResponse {
        text,
        uti,
        data_base64,
    }))
}

#[derive(Deserialize)]
struct ClipboardWriteRequest {
    text: Option<String>,
    uti: Option<String>,
    data_base64: Option<String>,
}

async fn handle_set_clipboard(
    State(state): State<Arc<WebViewerState>>,
    Extension(principal): Extension<Principal>,
    Json(req): Json<ClipboardWriteRequest>,
) -> Result<StatusCode, StatusCode> {
    if !state.share_clipboard {
        return Err(StatusCode::FORBIDDEN);
    }
    let payload = if let Some(text) = req.text {
        ensure_within_limit(text.len(), MAX_TEXT_BYTES)?;
        ClipboardPayload::Text(text)
    } else if let Some(b64) = req.data_base64 {
        let raw = base64::Engine::decode(&base64::engine::general_purpose::STANDARD, &b64)
            .map_err(|_| StatusCode::BAD_REQUEST)?;
        ensure_within_limit(raw.len(), MAX_IMAGE_BYTES)?;
        let uti = req.uti.as_deref().unwrap_or(super::clipboard::UTI_PNG);
        match uti {
            super::clipboard::UTI_PNG => ClipboardPayload::ImagePng(raw),
            super::clipboard::UTI_JPEG => ClipboardPayload::ImageJpeg(raw),
            _ => return Err(StatusCode::BAD_REQUEST),
        }
    } else {
        return Err(StatusCode::BAD_REQUEST);
    };

    state
        .submit(
            DeviceCommand::Input(VncInputAction::ClipboardOut(payload.clone())),
            principal,
        )
        .await?;
    state
        .clipboard_sync
        .lock()
        .await
        .should_sync(&payload, ClipboardSource::Host);
    Ok(StatusCode::OK)
}

fn ensure_within_limit(actual: usize, max: usize) -> Result<(), StatusCode> {
    if actual > max {
        Err(StatusCode::PAYLOAD_TOO_LARGE)
    } else {
        Ok(())
    }
}

// -----------------------------------------------------------------------------
// Interactive HTML5 Viewer UI
// -----------------------------------------------------------------------------

const VIEWER_HTML: &str = r#"<!DOCTYPE html>
<html lang="en">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<title>iOS Device Web Viewer</title>
<style>
  body {
    margin: 0;
    padding: 0;
    background: #121214;
    color: #e4e4e7;
    font-family: -apple-system, BlinkMacSystemFont, "Segoe UI", Roboto, Helvetica, Arial, sans-serif;
    display: flex;
    flex-direction: column;
    align-items: center;
    height: 100vh;
  }
  header {
    width: 100%;
    padding: 12px 24px;
    background: #18181b;
    border-bottom: 1px solid #27272a;
    display: flex;
    justify-content: space-between;
    align-items: center;
    box-sizing: border-box;
  }
  .title { font-weight: 600; font-size: 16px; }
  .controls { display: flex; gap: 8px; flex-wrap: wrap; }
  button {
    background: #27272a;
    color: #fafafa;
    border: 1px solid #3f3f46;
    border-radius: 6px;
    padding: 6px 12px;
    font-size: 13px;
    cursor: pointer;
    transition: background 0.15s;
  }
  button:hover { background: #3f3f46; }
  button:active { background: #52525b; }
  button.danger { background: #7f1d1d; border-color: #991b1b; }
  button.danger:hover { background: #991b1b; }
  main {
    flex: 1;
    display: flex;
    justify-content: center;
    align-items: center;
    padding: 16px;
    overflow: hidden;
    position: relative;
  }
  #screen-frame {
    max-width: 100%;
    max-height: 100%;
    object-fit: contain;
    border-radius: 16px;
    box-shadow: 0 20px 40px rgba(0,0,0,0.6);
    background: #000;
    cursor: crosshair;
  }
  footer {
    width: 100%;
    padding: 8px 24px;
    background: #18181b;
    border-top: 1px solid #27272a;
    font-size: 12px;
    color: #a1a1aa;
    display: flex;
    justify-content: space-between;
    box-sizing: border-box;
  }
</style>
</head>
<body>
<header>
  <div class="title">iOS Device Screen Viewer (F05)</div>
  <div class="controls">
    <input id="credential" type="password" placeholder="Scoped token (empty for local mode)" autocomplete="off" />
    <button onclick="connectViewer()">Connect</button>
    <button onclick="sendButton('home')">Home</button>
    <button onclick="sendButton('lock')">Lock</button>
    <button onclick="sendButton('vol_up')">Vol +</button>
    <button onclick="sendButton('vol_down')">Vol -</button>
    <button onclick="sendButton('siri')">Siri</button>
    <button class="danger" onclick="stopStream()">■ Stop</button>
  </div>
</header>
<main>
  <img id="screen-frame" tabindex="0" alt="Connect to view the device screen" />
</main>
<footer>
  <span id="status">Status: Disconnected</span>
  <span>PNG screenshots · device operations report service failures</span>
</footer>
<script>
  const img = document.getElementById('screen-frame');
  const statusEl = document.getElementById('status');

  let bearer = '', active = false, imageUrl = null;
  let commands = Promise.resolve();
  const heldKeys = new Map();
  function headers() { return bearer ? { Authorization: 'Bearer ' + bearer } : {}; }
  async function request(url, options = {}) {
    const response = await fetch(url, { ...options, headers: { ...headers(), ...options.headers } });
    if (!response.ok) throw new Error('Device request failed (HTTP ' + response.status + ')');
    return response;
  }
  async function connectViewer() {
    bearer = document.getElementById('credential').value.trim();
    document.getElementById('credential').value = '';
    if (active) return;
    active = true;
    try {
      while (active) {
        const response = await request('/frame');
        const url = URL.createObjectURL(await response.blob());
        const previous = imageUrl;
        imageUrl = url; img.src = url;
        if (previous) URL.revokeObjectURL(previous);
        statusEl.textContent = 'Status: Connected';
        await new Promise(resolve => setTimeout(resolve, 100));
      }
    } catch (error) { active = false; bearer = ''; statusEl.textContent = error.message; }
  }
  async function postJson(url, data) {
    try {
      await request(url, { method: 'POST', headers: { 'Content-Type': 'application/json' }, body: JSON.stringify(data) });
      return true;
    } catch (error) { statusEl.textContent = error.message; return false; }
  }
  function sendButton(name) { postJson('/input/button', { name }); }
  async function stopStream() {
    if (await postJson('/stop', {})) statusEl.textContent = 'Status: Shutdown requested';
    active = false; bearer = ''; img.removeAttribute('src');
    if (imageUrl) { URL.revokeObjectURL(imageUrl); imageUrl = null; }
  }
  img.addEventListener('mousedown', e => { if (e.button === 0) { img.focus(); handlePointer('down', e); } });
  window.addEventListener('mouseup', e => { if (e.button === 0 && img.naturalWidth) handlePointer('up', e); });
  img.addEventListener('mousemove', e => { if (e.buttons & 1) handlePointer('move', e); });
  function handlePointer(action, e) {
    if (!active || !img.naturalWidth) return;
    const rect = img.getBoundingClientRect();
    const x = Math.max(0, Math.min(img.naturalWidth - 1, Math.floor((e.clientX - rect.left) / rect.width * img.naturalWidth)));
    const y = Math.max(0, Math.min(img.naturalHeight - 1, Math.floor((e.clientY - rect.top) / rect.height * img.naturalHeight)));
    commands = commands.then(() => postJson('/input/pointer', { x, y, action }));
  }
  function usage(code) {
    if (/^Key[A-Z]$/.test(code)) return code.charCodeAt(3) - 65 + 4;
    if (/^Digit[1-9]$/.test(code)) return code.charCodeAt(5) - 49 + 0x1e;
    return { Digit0:0x27, Enter:0x28, Escape:0x29, Backspace:0x2a, Tab:0x2b, Space:0x2c,
      ArrowRight:0x4f, ArrowLeft:0x50, ArrowDown:0x51, ArrowUp:0x52,
      ControlLeft:0xe0, ShiftLeft:0xe1, AltLeft:0xe2, MetaLeft:0xe3,
      ControlRight:0xe4, ShiftRight:0xe5, AltRight:0xe6, MetaRight:0xe7 }[code];
  }
  img.addEventListener('keydown', e => {
    const key = usage(e.code); if (key === undefined || !active) return;
    e.preventDefault(); if (heldKeys.has(e.code)) return;
    heldKeys.set(e.code, key); commands = commands.then(() => postJson('/input/key', { usage:key, down:true }));
  });
  window.addEventListener('keyup', e => {
    const key = heldKeys.get(e.code); if (key === undefined) return;
    e.preventDefault(); heldKeys.delete(e.code);
    commands = commands.then(() => postJson('/input/key', { usage:key, down:false }));
  });
  window.addEventListener('blur', () => {
    for (const key of heldKeys.values()) commands = commands.then(() => postJson('/input/key', { usage:key, down:false }));
    heldKeys.clear();
  });

</script>
</body>
</html>
"#;
