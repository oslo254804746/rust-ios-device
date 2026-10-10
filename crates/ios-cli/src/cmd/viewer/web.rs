//! Web viewer HTTP / WebSocket application, interactive canvas, and media stream (F05-1).
#![allow(dead_code)]

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use anyhow::Result;
use axum::extract::{Json, State};
use axum::http::{header, HeaderMap, StatusCode};
use axum::response::{Html, Response};
use axum::routing::{get, post};
use axum::Router;
use serde::{Deserialize, Serialize};
use tokio::sync::{broadcast, Mutex, Notify, RwLock};

use super::clipboard::{
    ClipboardPayload, ClipboardSource, ClipboardSyncState, MAX_IMAGE_BYTES, MAX_TEXT_BYTES,
    UTI_JPEG, UTI_PLAIN_TEXT, UTI_PNG,
};
use super::vnc::{
    VncInputAction, BTN_HOME, BTN_LOCK, BTN_MUTE, BTN_SIRI, BTN_VOL_DOWN, BTN_VOL_UP,
};
use crate::access::{AccessControl, Permission};

pub const MAX_WEB_FRAME_BYTES: usize = 16 * 1024 * 1024; // 16 MiB

#[derive(Debug, Clone)]
pub struct WebFrame {
    pub mime_type: &'static str,
    pub data: Arc<Vec<u8>>,
    pub width: u32,
    pub height: u32,
    pub timestamp_ms: u64,
}

pub struct WebViewerState {
    pub frame: RwLock<Option<WebFrame>>,
    pub new_frame_notify: Arc<Notify>,
    pub input_tx: broadcast::Sender<VncInputAction>,
    pub clipboard_sync: Mutex<ClipboardSyncState>,
    pub access: AccessControl,
    pub stopped: AtomicBool,
    pub stop_notify: Arc<Notify>,
}

impl WebViewerState {
    pub fn new(access: AccessControl) -> Arc<Self> {
        let (input_tx, _) = broadcast::channel(512);
        Arc::new(Self {
            frame: RwLock::new(None),
            new_frame_notify: Arc::new(Notify::new()),
            input_tx,
            clipboard_sync: Mutex::new(ClipboardSyncState::new()),
            access,
            stopped: AtomicBool::new(false),
            stop_notify: Arc::new(Notify::new()),
        })
    }

    pub async fn push_frame(
        &self,
        mime: &'static str,
        data: Vec<u8>,
        width: u32,
        height: u32,
        ts: u64,
    ) {
        if data.len() > MAX_WEB_FRAME_BYTES {
            tracing::warn!(
                len = data.len(),
                "frame exceeds web viewer memory budget; dropped"
            );
            return;
        }
        let frame = WebFrame {
            mime_type: mime,
            data: Arc::new(data),
            width,
            height,
            timestamp_ms: ts,
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
    }
}

pub fn create_viewer_router(state: Arc<WebViewerState>) -> Router {
    Router::new()
        .route("/", get(handle_index))
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
        .with_state(state)
}

fn check_auth(headers: &HeaderMap, access: &AccessControl) -> Result<(), StatusCode> {
    let principal = access
        .authenticate(headers)
        .ok_or(StatusCode::UNAUTHORIZED)?;
    if principal.has_permission(Permission::Debug) || principal.has_permission(Permission::Connect)
    {
        Ok(())
    } else {
        Err(StatusCode::FORBIDDEN)
    }
}

// -----------------------------------------------------------------------------
// Route Handlers
// -----------------------------------------------------------------------------

async fn handle_index(
    State(state): State<Arc<WebViewerState>>,
    headers: HeaderMap,
) -> Result<Html<&'static str>, StatusCode> {
    check_auth(&headers, &state.access)?;
    Ok(Html(VIEWER_HTML))
}

async fn handle_stream(
    State(state): State<Arc<WebViewerState>>,
    headers: HeaderMap,
) -> Result<Response, StatusCode> {
    check_auth(&headers, &state.access)?;

    let boundary = "frame";
    let (tx, rx) = tokio::sync::mpsc::channel(16);
    let state_stream = state.clone();
    tokio::spawn(async move {
        while !state_stream.is_stopped() {
            let frame_opt = {
                let guard = state_stream.frame.read().await;
                guard.clone()
            };

            if let Some(frame) = frame_opt {
                let part_header = format!(
                    "--{}\r\nContent-Type: {}\r\nContent-Length: {}\r\nX-Timestamp: {}\r\n\r\n",
                    boundary,
                    frame.mime_type,
                    frame.data.len(),
                    frame.timestamp_ms
                );
                if tx
                    .send(Ok::<_, std::io::Error>(bytes::Bytes::from(part_header)))
                    .await
                    .is_err()
                {
                    break;
                }
                if tx
                    .send(Ok(bytes::Bytes::copy_from_slice(&frame.data)))
                    .await
                    .is_err()
                {
                    break;
                }
                if tx.send(Ok(bytes::Bytes::from("\r\n"))).await.is_err() {
                    break;
                }
            }

            tokio::select! {
                _ = state_stream.new_frame_notify.notified() => {}
                _ = tokio::time::sleep(Duration::from_millis(50)) => {}
            }
        }
    });

    let body = tokio_stream::wrappers::ReceiverStream::new(rx);

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
    headers: HeaderMap,
) -> Result<Json<serde_json::Value>, StatusCode> {
    check_auth(&headers, &state.access)?;
    state.trigger_stop();
    Ok(Json(serde_json::json!({
        "status": "stopped",
        "message": "web viewer stream cleanly terminated"
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
    headers: HeaderMap,
    Json(req): Json<PointerRequest>,
) -> Result<StatusCode, StatusCode> {
    check_auth(&headers, &state.access)?;
    let contact = match req.action.as_str() {
        "down" | "move" => true,
        "up" => false,
        _ => return Err(StatusCode::BAD_REQUEST),
    };
    let _ = state.input_tx.send(VncInputAction::Touch {
        contact,
        x: req.x,
        y: req.y,
    });
    Ok(StatusCode::OK)
}

#[derive(Deserialize)]
struct ButtonRequest {
    name: String, // "home", "lock", "vol_up", "vol_down", "mute", "siri"
}

async fn handle_button(
    State(state): State<Arc<WebViewerState>>,
    headers: HeaderMap,
    Json(req): Json<ButtonRequest>,
) -> Result<StatusCode, StatusCode> {
    check_auth(&headers, &state.access)?;
    let (page, code) = match req.name.to_lowercase().as_str() {
        "home" => BTN_HOME,
        "lock" => BTN_LOCK,
        "vol_up" => BTN_VOL_UP,
        "vol_down" => BTN_VOL_DOWN,
        "mute" => BTN_MUTE,
        "siri" => BTN_SIRI,
        _ => return Err(StatusCode::BAD_REQUEST),
    };

    let _ = state.input_tx.send(VncInputAction::Button {
        page,
        code,
        down: true,
    });
    tokio::time::sleep(Duration::from_millis(50)).await;
    let _ = state.input_tx.send(VncInputAction::Button {
        page,
        code,
        down: false,
    });
    Ok(StatusCode::OK)
}

#[derive(Deserialize)]
struct KeyRequest {
    usage: u8,
    down: bool,
}

async fn handle_key(
    State(state): State<Arc<WebViewerState>>,
    headers: HeaderMap,
    Json(req): Json<KeyRequest>,
) -> Result<StatusCode, StatusCode> {
    check_auth(&headers, &state.access)?;
    let _ = state.input_tx.send(VncInputAction::Key {
        usage: req.usage,
        down: req.down,
    });
    Ok(StatusCode::OK)
}

#[derive(Deserialize)]
struct OrientationRequest {
    direction: String, // "left", "right"
}

async fn handle_orientation(
    State(state): State<Arc<WebViewerState>>,
    headers: HeaderMap,
    Json(req): Json<OrientationRequest>,
) -> Result<StatusCode, StatusCode> {
    check_auth(&headers, &state.access)?;
    let dir = match req.direction.as_str() {
        "left" => "left",
        "right" => "right",
        _ => return Err(StatusCode::BAD_REQUEST),
    };
    let _ = state
        .input_tx
        .send(VncInputAction::Rotate { direction: dir });
    Ok(StatusCode::OK)
}

#[derive(Serialize)]
struct ClipboardResponse {
    text: Option<String>,
    uti: String,
    data_base64: Option<String>,
}

async fn handle_get_clipboard(
    State(state): State<Arc<WebViewerState>>,
    headers: HeaderMap,
) -> Result<Json<ClipboardResponse>, StatusCode> {
    check_auth(&headers, &state.access)?;
    let sync = state.clipboard_sync.lock().await;
    let text = sync.last_synced_hash().map(hex::encode);
    Ok(Json(ClipboardResponse {
        text,
        uti: UTI_PLAIN_TEXT.to_string(),
        data_base64: None,
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
    headers: HeaderMap,
    Json(req): Json<ClipboardWriteRequest>,
) -> Result<StatusCode, StatusCode> {
    check_auth(&headers, &state.access)?;
    let payload = if let Some(text) = req.text {
        ensure_within_limit(text.len(), MAX_TEXT_BYTES)?;
        ClipboardPayload::Text(text)
    } else if let Some(b64) = req.data_base64 {
        let raw = base64::Engine::decode(&base64::engine::general_purpose::STANDARD, &b64)
            .map_err(|_| StatusCode::BAD_REQUEST)?;
        ensure_within_limit(raw.len(), MAX_IMAGE_BYTES)?;
        match req.uti.as_deref() {
            Some(UTI_PNG) => ClipboardPayload::ImagePng(raw),
            Some(UTI_JPEG) => ClipboardPayload::ImageJpeg(raw),
            _ => return Err(StatusCode::BAD_REQUEST),
        }
    } else {
        return Err(StatusCode::BAD_REQUEST);
    };

    let mut sync = state.clipboard_sync.lock().await;
    if sync.should_sync(&payload, ClipboardSource::Host) {
        let _ = state.input_tx.send(VncInputAction::ClipboardOut(payload));
    }
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
    <button onclick="sendOrientation('left')">↺ Rotate CCW</button>
    <button onclick="sendOrientation('right')">↻ Rotate CW</button>
    <button onclick="sendButton('home')">Home</button>
    <button onclick="sendButton('lock')">Lock</button>
    <button onclick="sendButton('vol_up')">Vol +</button>
    <button onclick="sendButton('vol_down')">Vol -</button>
    <button onclick="sendButton('siri')">Siri</button>
    <button class="danger" onclick="stopStream()">■ Stop</button>
  </div>
</header>
<main>
  <img id="screen-frame" src="/stream" alt="Live Screen Stream" />
</main>
<footer>
  <span id="status">Status: Connected to live media stream</span>
  <span>Bounded HEVC / MJPEG Media Transport</span>
</footer>
<script>
  const img = document.getElementById('screen-frame');
  const statusEl = document.getElementById('status');

  async function postJson(url, data) {
    try {
      const resp = await fetch(url, {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify(data)
      });
      return resp.ok;
    } catch (e) {
      console.error(e);
      return false;
    }
  }

  function sendButton(name) { postJson('/input/button', { name }); }
  function sendOrientation(direction) { postJson('/orientation', { direction }); }
  async function stopStream() {
    await postJson('/stop', {});
    statusEl.textContent = 'Status: Stream stopped';
    img.src = '';
  }

  // Pointer / Touch mapping
  img.addEventListener('mousedown', (e) => handlePointer('down', e));
  img.addEventListener('mouseup', (e) => handlePointer('up', e));
  img.addEventListener('mousemove', (e) => {
    if (e.buttons === 1) handlePointer('move', e);
  });

  function handlePointer(action, e) {
    const rect = img.getBoundingClientRect();
    const x = Math.round(((e.clientX - rect.left) / rect.width) * (img.naturalWidth || rect.width));
    const y = Math.round(((e.clientY - rect.top) / rect.height) * (img.naturalHeight || rect.height));
    postJson('/input/pointer', { x, y, action });
  }
</script>
</body>
</html>
"#;
