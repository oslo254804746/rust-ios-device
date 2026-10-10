//! CDP HTTP/WebSocket adaptation and bridge task ownership.
//! WebInspector wire/Target state remains in ios-core sessions.
use super::connection::InspectorConnection;
use super::diagnostics::diagnostic_cdp_response;
use super::http::{cdp_reload_policy, serve, server_state, AppError, ServerState};
use super::tasks::control::Budget;
use super::workflows::{find_page_by_id, is_inspectable_page_type, page_target_id};
use super::PAGE_DISCOVERY_IDLE;
use crate::access::{Admission, HttpBoundaryArgs, Principal, ServerAccessArgs};
use anyhow::{anyhow, Result};
use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::{Path, State};
use axum::response::IntoResponse;
use axum::routing::{get, post};
use axum::{Extension, Json, Router};
use futures_util::{SinkExt, StreamExt};
use ios_core::webinspector::cdp_browser::{
    self, BrowserCdpAction, BrowserCdpState, JsContextCdpAdapter,
};
use ios_core::webinspector::{ApplicationPage, InspectorSession, WebInspectorClient, WirType};
use serde::Serialize;
use serde_json::{json, Value as JsonValue};
use std::collections::HashMap;
use std::sync::{Arc, OnceLock};
use std::time::Duration;
use tokio::sync::{mpsc, watch, Mutex, OwnedMutexGuard};

mod browser_tasks;
#[cfg(test)]
use browser_tasks::stop_browser_page;
use browser_tasks::{report_cleanup, stop_browser_pages, BrowserPageCleanup, BrowserPageSession};

const MAX_CDP_CLIENT_MESSAGE_BYTES: usize = 1024 * 1024;
const CDP_TARGET_POLL: Duration = Duration::from_secs(2);
const CDP_PAGE_CHANNEL: usize = 64;
const CDP_PAGE_CLEANUP_TIMEOUT: Duration = Duration::from_secs(2);
const MAX_CDP_PAGE_LOCKS: usize = 4096;
static CDP_PAGE_LOCKS: OnceLock<Mutex<HashMap<String, Arc<Mutex<()>>>>> = OnceLock::new();

pub(super) async fn lock_cdp_page(
    target_id: &str,
    timeout_duration: Duration,
) -> Result<OwnedMutexGuard<()>> {
    let locks = CDP_PAGE_LOCKS.get_or_init(|| Mutex::new(HashMap::new()));
    let mut map = locks.lock().await;
    if !map.contains_key(target_id) && map.len() >= MAX_CDP_PAGE_LOCKS {
        map.retain(|_, lock| Arc::strong_count(lock) > 1);
    }
    if !map.contains_key(target_id) && map.len() >= MAX_CDP_PAGE_LOCKS {
        return Err(anyhow!("too many concurrent CDP page locks"));
    }
    let lock = map.entry(target_id.to_string()).or_default().clone();
    drop(map);
    Ok(tokio::time::timeout(timeout_duration, lock.lock_owned()).await?)
}

#[derive(Debug, Clone, Serialize)]
struct CdpTargetDescriptor {
    description: String,
    id: String,
    title: String,
    #[serde(rename = "type")]
    target_type: String,
    url: String,
    #[serde(rename = "webSocketDebuggerUrl")]
    web_socket_debugger_url: String,
    #[serde(rename = "devtoolsFrontendUrl")]
    #[serde(skip_serializing_if = "Option::is_none")]
    devtools_frontend_url: Option<String>,
}

pub(super) async fn run_cdp_server(
    udid: &str,
    host: String,
    port: u16,
    timeout: Duration,
    access: ServerAccessArgs,
    boundary: HttpBoundaryArgs,
) -> Result<()> {
    let (listener, state, boundary) =
        server_state(udid, &host, port, timeout, access, boundary).await?;
    serve(
        listener,
        boundary.protect(cdp_router(state), crate::http_diagnostics::ErrorShape::Cdp),
    )
    .await
}

pub(super) fn cdp_router(state: ServerState) -> Router {
    let admission = Admission::debug(
        state.access.clone(),
        crate::http_diagnostics::ErrorShape::Cdp,
        state.udid.clone(),
    );
    Router::new()
        .route("/json", get(cdp_targets))
        .route("/json/list", get(cdp_targets))
        .route("/json/version", get(cdp_version))
        .route("/json/version/", get(cdp_version))
        .route("/devtools/page/:page_id", get(cdp_page_ws))
        .route("/devtools/browser/:browser_id", get(cdp_browser_ws))
        .route("/auth/reload", post(cdp_reload_policy))
        .with_state(state)
        .merge(admission.ticket_routes())
        .layer(axum::middleware::from_fn_with_state(
            crate::http_diagnostics::ErrorShape::Cdp,
            crate::http_diagnostics::rejections,
        ))
        .layer(axum::middleware::from_fn_with_state(
            admission,
            crate::access::admission,
        ))
}

async fn cdp_targets(
    State(state): State<ServerState>,
) -> Result<Json<Vec<CdpTargetDescriptor>>, AppError> {
    let pages = load_open_pages(&state).await?;
    Ok(Json(cdp_target_descriptors(
        &pages,
        &state.cdp_host,
        state.cdp_port,
    )))
}

pub(super) async fn cdp_version(State(state): State<ServerState>) -> Json<JsonValue> {
    Json(json!({
        "Browser": "Safari",
        "Protocol-Version": "1.1",
        "User-Agent": "ios-cli",
        "webSocketDebuggerUrl": format!("ws://{}:{}/devtools/browser/ios-cli", state.cdp_host, state.cdp_port),
    }))
}

async fn cdp_page_ws(
    ws: WebSocketUpgrade,
    State(state): State<ServerState>,
    Extension(principal): Extension<Principal>,
    Path(page_id): Path<String>,
) -> Result<impl IntoResponse, AppError> {
    Ok(ws
        .max_frame_size(MAX_CDP_CLIENT_MESSAGE_BYTES)
        .max_message_size(MAX_CDP_CLIENT_MESSAGE_BYTES)
        .on_upgrade(move |socket| async move {
            if let Err(error) =
                handle_cdp_socket(socket, &state, &page_id, principal.revoked()).await
            {
                let _ = crate::output::write_error_line(format_args!(
                    "cdp bridge closed with error: {}",
                    crate::output::error_message(&error)
                ));
            }
        }))
}

async fn handle_cdp_socket(
    socket: WebSocket,
    server: &ServerState,
    page_id: &str,
    stop: impl std::future::Future<Output = ()> + Send,
) -> Result<()> {
    let signal = async {
        stop.await;
        Ok::<(), std::io::Error>(())
    };
    tokio::pin!(signal);
    let timeout = server.timeout;
    let budget = Budget::new(timeout, "CDP page setup")?;
    let (_page_guard, _device, mut client, mut session, mut js_adapter) = budget
        .drive(
            Box::pin(async {
                let guard = lock_cdp_page(page_id, budget.remaining()?).await?;
                let InspectorConnection { device, stream } = server
                    .source
                    .connect(&server.udid, budget.remaining()?)
                    .await?;
                let mut client = WebInspectorClient::new(stream);
                let remaining = budget.cap(&mut client)?;
                client.start_preserving_errors(remaining).await?;
                let (application_id, page) =
                    discover_cdp_page(&mut client, page_id, budget).await?;
                let mut session = InspectorSession::new(application_id, page.id);
                let js_adapter =
                    (page.page_type == WirType::JavaScript).then(JsContextCdpAdapter::default);
                let remaining = budget.cap(&mut client)?;
                session
                    .attach_unpaused(
                        &mut client,
                        page.page_type != WirType::JavaScript,
                        remaining,
                    )
                    .await?;
                client.set_operation_timeout(None);
                Ok((guard, device, client, session, js_adapter))
            }),
            &mut signal,
        )
        .await?;

    let (mut sender, mut receiver) = socket.split();
    let bridge_result = tokio::select! {
        biased;
        _ = &mut signal => Err(anyhow::Error::new(ios_core::OperationCancelled::new())
            .context("CDP page authorization revoked")),
        result = async {
        loop {
            // Each message turn yields back to the directly owned lease
            // control while preserving the existing unbiased traffic choice.
            tokio::task::yield_now().await;
            tokio::select! {
                inbound = receiver.next() => {
                    match inbound {
                        Some(Ok(Message::Text(text))) => {
                            let message: JsonValue = serde_json::from_str(text.as_ref())?;
                            if let Some(responses) = cdp_browser::handle_verified_page_method(&message) {
                                for resp in responses {
                                    sender.send(Message::Text(resp.to_string())).await?;
                                }
                                continue;
                            }
                            if cdp_browser::unverified_page_method(&message) {
                                sender.send(Message::Text(cdp_browser::error(&message, -32601, "CDP domain not implemented").to_string())).await?;
                                continue;
                            }
                            if let Some(error) = js_adapter.as_ref().and_then(|adapter| adapter.invalid_context_response(&message)) {
                                sender.send(Message::Text(error.to_string())).await?;
                                continue;
                            }
                            let outgoing = if let Some(adapter) = &mut js_adapter {
                                adapter.outgoing(message)
                            } else { vec![message] };
                            for message in outgoing {
                                session.send_bridge_message(&mut client, &message).await?;
                            }
                        }
                        Some(Ok(Message::Close(_))) | None => break,
                        Some(Ok(_)) => {}
                        Some(Err(error)) => return Err(error.into()),
                    }
                }
                outbound = session.next_raw_message(&mut client, timeout) => {
                    match outbound {
                        Ok(message) => {
                            if let Some(message) = session.bridge_message(&message)? {
                                let incoming = if let Some(adapter) = &mut js_adapter {
                                    adapter.incoming(message)
                                } else { vec![message] };
                                for message in incoming {
                                    let message = diagnostic_cdp_response(message);
                                    sender.send(Message::Text(serde_json::to_string(&message)?)).await?;
                                }
                            }
                        }
                        Err(error) if error.timeout_duration().is_some() && client.is_usable() => {},
                        Err(error) => return Err(error.into()),
                    }
                }
            }
        }
        Ok(())
        } => result,
    };
    let mut owner = super::tasks::session_owner::Session::Inspector {
        session,
        attached: true,
    };
    let cleanup =
        super::tasks::session_owner::cleanup(&mut client, &mut owner, CDP_PAGE_CLEANUP_TIMEOUT)
            .await;
    super::tasks::control::finish(bridge_result, cleanup)
}

enum BrowserPageOutput {
    Message {
        session_id: String,
        message: JsonValue,
    },
    Closed {
        session_id: String,
    },
}

async fn cdp_browser_ws(
    ws: WebSocketUpgrade,
    State(state): State<ServerState>,
    Extension(principal): Extension<Principal>,
    Path(browser_id): Path<String>,
) -> Result<impl IntoResponse, AppError> {
    if browser_id != "ios-cli" {
        return Err(AppError::bad_request("unknown CDP browser endpoint"));
    }
    Ok(ws
        .max_frame_size(MAX_CDP_CLIENT_MESSAGE_BYTES)
        .max_message_size(MAX_CDP_CLIENT_MESSAGE_BYTES)
        .on_upgrade(move |socket| async move {
            if let Err(error) = handle_cdp_browser_socket(socket, &state, principal.revoked()).await
            {
                let _ = crate::output::write_error_line(format_args!(
                    "cdp browser bridge closed with error: {}",
                    crate::output::error_message(&error)
                ));
            }
        }))
}

async fn handle_cdp_browser_socket(
    socket: WebSocket,
    server: &ServerState,
    stop: impl std::future::Future<Output = ()> + Send,
) -> Result<()> {
    tokio::pin!(stop);
    let (mut sender, mut receiver) = socket.split();
    let mut state = BrowserCdpState::new("ios-cli");
    let initial = tokio::select! {
        biased;
        _ = &mut stop => return Err(anyhow::Error::new(ios_core::OperationCancelled::new())
            .context("webinspector CDP browser cancelled")),
        result = load_open_pages(server) => result?,
    };
    state.refresh(&initial);
    let (page_output_tx, mut page_output_rx) = mpsc::channel(CDP_PAGE_CHANNEL);
    let mut sessions: HashMap<String, BrowserPageSession> = HashMap::new();
    // Removed children remain owned outside active work, even if revocation
    // interrupts a borrowed cleanup while joins or native expiry are pending.
    let mut retiring = BrowserPageCleanup::new(Vec::new(), CDP_PAGE_CLEANUP_TIMEOUT);
    let mut poll = tokio::time::interval(CDP_TARGET_POLL);
    poll.tick().await;
    let result = tokio::select! {
        biased;
        _ = &mut stop => Err(anyhow::Error::new(ios_core::OperationCancelled::new())
            .context("webinspector CDP browser cancelled")),
        result = async {
        loop {
            tokio::task::yield_now().await;
            tokio::select! {
                inbound = receiver.next() => {
                    match inbound {
                        Some(Ok(Message::Text(text))) => {
                            let request: JsonValue = match serde_json::from_str(text.as_ref()) {
                                Ok(request) => request,
                                Err(_) => {
                                    sender.send(Message::Text(json!({"error": {"code": -32700, "message": "Parse error"}}).to_string())).await?;
                                    continue;
                                }
                            };
                            if matches!(request.get("method").and_then(JsonValue::as_str), Some("Target.getTargets" | "Target.attachToTarget")) {
                                match load_open_pages(server).await {
                                    Ok(pages) => {
                                        let refresh = state.refresh(&pages);
                                        for id in refresh.removed_sessions {
                                            if let Some(page) = sessions.remove(&id) {
                                                retiring.push(page);
                                                report_cleanup(retiring.stop().await);
                                            }
                                        }
                                        send_cdp_messages(&mut sender, refresh.messages).await?;
                                    }
                                    Err(_) => {
                                        send_cdp_messages(&mut sender, vec![cdp_browser::error(&request, -32000, "Target discovery failed")]).await?;
                                        continue;
                                    }
                                }
                            }
                            let action = state.handle(request);
                            match action {
                                BrowserCdpAction::Send(messages) => send_cdp_messages(&mut sender, messages).await?,
                                BrowserCdpAction::Attach { request, target_id } => {
                                    let id = state.register_attachment(&target_id).expect("known target");
                                    match open_cdp_browser_page(server, &target_id, &id, page_output_tx.clone()).await {
                                        Ok(page) => {
                                            sessions.insert(id.clone(), page);
                                            send_cdp_messages(&mut sender, vec![cdp_browser::reply(&request, json!({"sessionId": id}))]).await?;
                                        }
                                        Err(_) => {
                                            state.detach(&id, false);
                                            send_cdp_messages(&mut sender, vec![cdp_browser::error(&request, -32000, "Cannot attach to target")]).await?;
                                        }
                                    }
                                }
                                BrowserCdpAction::Detach { request, session_id } => {
                                    if let Some(page) = sessions.remove(&session_id) {
                                        retiring.push(page);
                                        report_cleanup(retiring.stop().await);
                                    }
                                    state.detach(&session_id, false);
                                    send_cdp_messages(&mut sender, vec![cdp_browser::reply(&request, json!({}))]).await?;
                                }
                                BrowserCdpAction::Forward { session_id, message } => {
                                    if let Some(page) = sessions.get(&session_id) {
                                        if page.sender.send(message).await.is_err() {
                                            if let Some(page) = sessions.remove(&session_id) {
                                                retiring.push(page);
                                                report_cleanup(retiring.stop().await);
                                            }
                                            send_cdp_messages(&mut sender, state.detach(&session_id, true)).await?;
                                        }
                                    }
                                }
                            }
                            auto_attach_cdp_pages(server, &mut state, &mut sessions, &page_output_tx, &mut sender).await?;
                        }
                        Some(Ok(Message::Close(_))) | None => break,
                        Some(Ok(_)) => {},
                        Some(Err(error)) => return Err(error.into()),
                    }
                }
                output = page_output_rx.recv() => {
                    match output {
                        Some(BrowserPageOutput::Message {session_id, message}) => {
                            if let Some(message) = state.page_message(&session_id, message) {
                                send_cdp_messages(&mut sender, vec![message]).await?;
                            }
                        }
                        Some(BrowserPageOutput::Closed {session_id}) => {
                            if let Some(page) = sessions.remove(&session_id) {
                                retiring.push(page);
                                report_cleanup(retiring.stop().await);
                            }
                            send_cdp_messages(&mut sender, state.detach(&session_id, true)).await?;
                        }
                        None => break,
                    }
                }
                _ = poll.tick() => {
                    if let Ok(pages) = load_open_pages(server).await {
                        let refresh = state.refresh(&pages);
                        for id in refresh.removed_sessions {
                            if let Some(page) = sessions.remove(&id) {
                                retiring.push(page);
                                report_cleanup(retiring.stop().await);
                            }
                        }
                        send_cdp_messages(&mut sender, refresh.messages).await?;
                        auto_attach_cdp_pages(server, &mut state, &mut sessions, &page_output_tx, &mut sender).await?;
                    }
                }
            }
        }
        Ok(())
        } => result,
    };
    let (mut pages, issued) = retiring.into_remaining();
    pages.extend(sessions.into_values());
    let cleanup = super::tasks::control::finish(
        issued,
        stop_browser_pages(pages, CDP_PAGE_CLEANUP_TIMEOUT).await,
    );
    super::tasks::control::finish(result, cleanup)
}

async fn send_cdp_messages(
    sender: &mut futures_util::stream::SplitSink<WebSocket, Message>,
    messages: Vec<JsonValue>,
) -> Result<()> {
    for message in messages {
        sender.send(Message::Text(message.to_string())).await?;
    }
    Ok(())
}

async fn auto_attach_cdp_pages(
    server: &ServerState,
    state: &mut BrowserCdpState,
    sessions: &mut HashMap<String, BrowserPageSession>,
    output: &mpsc::Sender<BrowserPageOutput>,
    sender: &mut futures_util::stream::SplitSink<WebSocket, Message>,
) -> Result<()> {
    for target_id in state.unattached_targets() {
        if let Some(session_id) = state.register_attachment(&target_id) {
            if let Ok(page) =
                open_cdp_browser_page(server, &target_id, &session_id, output.clone()).await
            {
                sessions.insert(session_id.clone(), page);
                if let Some(event) = state.attached_event(&session_id) {
                    send_cdp_messages(sender, vec![event]).await?;
                }
            } else {
                state.detach(&session_id, false);
            }
        }
    }
    Ok(())
}

async fn open_cdp_browser_page(
    server: &ServerState,
    target_id: &str,
    cdp_session_id: &str,
    output: mpsc::Sender<BrowserPageOutput>,
) -> Result<BrowserPageSession> {
    let budget = Budget::new(server.timeout, "CDP browser page setup")?;
    let (guard, device, mut client, mut inspector, mut js_adapter) = budget
        .drive(
            Box::pin(async {
                let guard = lock_cdp_page(target_id, budget.remaining()?).await?;
                let InspectorConnection { device, stream } = server
                    .source
                    .connect(&server.udid, budget.remaining()?)
                    .await?;
                let mut client = WebInspectorClient::new(stream);
                let remaining = budget.cap(&mut client)?;
                client.start_preserving_errors(remaining).await?;
                let (application_id, page) =
                    discover_cdp_page(&mut client, target_id, budget).await?;
                let mut inspector = InspectorSession::new(application_id, page.id);
                let js_adapter =
                    (page.page_type == WirType::JavaScript).then(JsContextCdpAdapter::default);
                let remaining = budget.cap(&mut client)?;
                inspector
                    .attach_unpaused(
                        &mut client,
                        page.page_type != WirType::JavaScript,
                        remaining,
                    )
                    .await?;
                client.set_operation_timeout(None);
                Ok((guard, device, client, inspector, js_adapter))
            }),
            std::future::pending::<std::io::Result<()>>(),
        )
        .await?;
    let (input, mut receiver) = mpsc::channel(CDP_PAGE_CHANNEL);
    let (cancel, mut cancellation) = watch::channel(false);
    let timeout = server.timeout;
    let cdp_session_id = cdp_session_id.to_string();
    let outcome = browser_tasks::outcome();
    let task_outcome = outcome.clone();
    let task = tokio::spawn(async move {
        let _guard = guard;
        let _device = device;
        let stop = async {
            loop {
                if *cancellation.borrow() {
                    return;
                }
                if cancellation.changed().await.is_err() {
                    return;
                }
            }
        };
        tokio::pin!(stop);
        let primary = tokio::select! {
            biased;
            _ = &mut stop => Ok(()),
            result = async {
        loop {
            tokio::task::yield_now().await;
            tokio::select! {
                inbound = receiver.recv() => {
                    match inbound {
                        Some(message) => {
                            if let Some(error) = js_adapter.as_ref().and_then(|adapter| adapter.invalid_context_response(&message)) {
                                output.send(BrowserPageOutput::Message { session_id: cdp_session_id.clone(), message: error }).await?;
                                continue;
                            }
                            let messages = if let Some(adapter) = &mut js_adapter {
                                adapter.outgoing(message)
                            } else { vec![message] };
                            for message in messages {
                                inspector.send_bridge_message(&mut client, &message).await?;
                            }
                        }
                        None => break,
                    }
                }
                outbound = inspector.next_raw_message(&mut client, timeout) => {
                    match outbound {
                        Ok(message) => {
                            if let Some(message) = inspector.bridge_message(&message)? {
                                // The outer session ID is added by the browser state.
                                let messages = if let Some(adapter) = &mut js_adapter {
                                    adapter.incoming(message)
                                } else { vec![message] };
                                for message in messages {
                                    let message = diagnostic_cdp_response(message);
                                    output.send(BrowserPageOutput::Message {session_id: cdp_session_id.clone(), message}).await?;
                                }
                            }
                        },
                        Err(error) if error.timeout_duration().is_some() && client.is_usable() => {},
                        Err(error) => return Err(error.into()),
                    }
                }
            }
        }
        Ok(())
            } => result,
        };
        task_outcome
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
            .primary(primary);
        let mut owner = super::tasks::session_owner::Session::Inspector {
            session: inspector,
            attached: true,
        };
        let cleanup =
            super::tasks::session_owner::cleanup(&mut client, &mut owner, CDP_PAGE_CLEANUP_TIMEOUT)
                .await;
        task_outcome
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
            .cleanup(cleanup);
        let _ = output.try_send(BrowserPageOutput::Closed {
            session_id: cdp_session_id,
        });
    });
    Ok(BrowserPageSession {
        sender: input,
        cancel,
        task,
        outcome,
    })
}

async fn load_open_pages(server: &ServerState) -> Result<Vec<ApplicationPage>> {
    let budget = Budget::new(server.timeout, "CDP discovery")?;
    budget
        .drive(
            Box::pin(async {
                let InspectorConnection {
                    device: _device,
                    stream,
                } = server
                    .source
                    .connect(&server.udid, budget.remaining()?)
                    .await?;
                let mut client = WebInspectorClient::new(stream);
                let remaining = budget.cap(&mut client)?;
                client.start_preserving_errors(remaining).await?;
                budget.cap(&mut client)?;
                client
                    .open_application_pages(PAGE_DISCOVERY_IDLE)
                    .await
                    .map_err(Into::into)
            }),
            std::future::pending::<std::io::Result<()>>(),
        )
        .await
}

/// A selected target can attach once its exact cached page is admitted. The
/// target listing is not a request for a quiescent snapshot of every page.
async fn discover_cdp_page(
    client: &mut WebInspectorClient<ios_core::device::ServiceStream>,
    page_id: &str,
    budget: Budget,
) -> Result<(String, ios_core::webinspector::Page)> {
    budget.cap(client)?;
    client.request_connected_applications().await?;
    loop {
        let remaining = budget.cap(client)?;
        if let Ok(page) = find_page_by_id(client, page_id) {
            return Ok(page);
        }
        match client
            .next_event_with_timeout(remaining.min(PAGE_DISCOVERY_IDLE))
            .await
        {
            Ok(_) => tokio::task::yield_now().await,
            Err(error) if error.timeout_duration().is_some() && client.is_usable() => {
                budget.cap(client)?;
                client.request_connected_applications().await?;
            }
            Err(error) => return Err(error.into()),
        }
    }
}

fn cdp_target_descriptors(
    pages: &[ApplicationPage],
    host: &str,
    port: u16,
) -> Vec<CdpTargetDescriptor> {
    pages
        .iter()
        .filter(|page| is_inspectable_page_type(&page.page.page_type))
        .map(|page| CdpTargetDescriptor {
            description: String::new(),
            id: page_target_id(&page.application.id, &page.page),
            title: page.page.title.clone().unwrap_or_default(),
            target_type: if matches!(page.page.page_type, WirType::JavaScript) {
                "node".to_string()
            } else {
                "page".to_string()
            },
            url: page.page.url.clone().unwrap_or_default(),
            web_socket_debugger_url: format!(
                "ws://{host}:{port}/devtools/page/{}",
                page_target_id(&page.application.id, &page.page)
            ),
            // This bridge exposes the CDP WebSocket but does not host a
            // DevTools frontend. Do not advertise a URL that clients cannot
            // actually open; the field is optional in the CDP target schema.
            devtools_frontend_url: None,
        })
        .collect()
}

#[cfg(test)]
pub(super) mod lifecycle_tests;

#[cfg(test)]
mod setup_tests;

#[cfg(test)]
mod cleanup_tests;

#[cfg(test)]
mod lease_tests;

#[cfg(test)]
mod browser_task_tests;

#[cfg(test)]
mod browser_lease_tests;

#[cfg(test)]
mod tests {
    use super::super::workflows::find_page_in_snapshot;
    use super::*;
    use ios_core::webinspector::{Application, AutomationAvailability, Page};

    #[tokio::test]
    async fn cdp_websocket_failure_hides_device_error_data_and_keeps_routing() {
        const SECRET: &str = "synthetic-private-key-UDID-token-device-data";
        let raw = json!({"id": 17, "sessionId": "synthetic-session", "error": {
            "code": -32001, "message": SECRET, "data": {"PrivateKey": SECRET}
        }});
        assert!(raw.to_string().contains(SECRET));
        let router = Router::new().route(
            "/synthetic-reply",
            get(move |ws: WebSocketUpgrade| {
                let raw = raw.clone();
                async move {
                    ws.on_upgrade(move |mut socket| async move {
                        socket
                            .send(Message::Text(diagnostic_cdp_response(raw).to_string()))
                            .await
                            .unwrap();
                    })
                }
            }),
        );
        let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
            .await
            .unwrap();
        let port = listener.local_addr().unwrap().port();
        let server = tokio::spawn(async move {
            axum::serve(listener, router).await.unwrap();
        });
        let (mut socket, _) =
            tokio_tungstenite::connect_async(format!("ws://127.0.0.1:{port}/synthetic-reply"))
                .await
                .unwrap();
        let frame = tokio::time::timeout(Duration::from_secs(2), socket.next())
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        let raw = frame.into_text().unwrap();
        assert!(!raw.contains(SECRET));
        let reply: JsonValue = serde_json::from_str(&raw).unwrap();
        assert_eq!(reply["id"], 17);
        assert_eq!(reply["sessionId"], "synthetic-session");
        assert_eq!(reply["error"]["code"], -32001);
        assert!(reply["error"]["message"]
            .as_str()
            .unwrap()
            .contains("WebInspector protocol request failed: operation error:"));
        assert!(reply["error"].get("data").is_none());
        let result =
            json!({"id": 18, "result": {"value": SECRET, "exceptionDetails": {"text": SECRET}}});
        assert_eq!(diagnostic_cdp_response(result.clone()), result);
        server.abort();
        let _ = server.await;
    }

    #[test]
    fn cdp_target_descriptors_filter_non_inspectable_pages() {
        let pages = vec![
            ApplicationPage {
                application: Application {
                    id: "PID:42".into(),
                    bundle_identifier: "com.apple.mobilesafari".into(),
                    pid: 42,
                    name: "Safari".into(),
                    availability: AutomationAvailability::Available,
                    is_active: true,
                    is_proxy: false,
                    is_ready: true,
                    host_application_identifier: None,
                },
                page: Page {
                    id: 7,
                    listing_key: "page-7".into(),
                    page_type: WirType::WebPage,
                    title: Some("Example".into()),
                    url: Some("https://example.com".into()),
                    automation_is_paired: None,
                    automation_name: None,
                    automation_version: None,
                    automation_session_id: None,
                    automation_connection_id: None,
                },
            },
            ApplicationPage {
                application: Application {
                    id: "PID:42".into(),
                    bundle_identifier: "com.apple.mobilesafari".into(),
                    pid: 42,
                    name: "Safari".into(),
                    availability: AutomationAvailability::Available,
                    is_active: true,
                    is_proxy: false,
                    is_ready: true,
                    host_application_identifier: None,
                },
                page: Page {
                    id: 8,
                    listing_key: "page-8".into(),
                    page_type: WirType::Automation,
                    title: Some("Automation".into()),
                    url: None,
                    automation_is_paired: Some(true),
                    automation_name: Some("Safari".into()),
                    automation_version: Some("1".into()),
                    automation_session_id: Some("S".into()),
                    automation_connection_id: Some("C".into()),
                },
            },
            ApplicationPage {
                application: Application {
                    id: "PID:43".into(),
                    bundle_identifier: "com.example.jscontext".into(),
                    pid: 43,
                    name: "JSContext host".into(),
                    availability: AutomationAvailability::Available,
                    is_active: true,
                    is_proxy: false,
                    is_ready: true,
                    host_application_identifier: None,
                },
                page: Page {
                    id: 1,
                    listing_key: "js-1".into(),
                    page_type: WirType::JavaScript,
                    title: None,
                    url: None,
                    automation_is_paired: None,
                    automation_name: None,
                    automation_version: None,
                    automation_session_id: None,
                    automation_connection_id: None,
                },
            },
        ];
        let descriptors = cdp_target_descriptors(&pages, "127.0.0.1", 9222);
        assert_eq!(descriptors.len(), 2);
        assert_eq!(descriptors[0].id, "PID:42:page-7");
        assert_eq!(
            descriptors[0].web_socket_debugger_url,
            "ws://127.0.0.1:9222/devtools/page/PID:42:page-7"
        );
        assert_eq!(descriptors[0].devtools_frontend_url, None);
        assert_eq!(descriptors[1].id, "PID:43:js-1");
        assert_eq!(descriptors[1].target_type, "node");
        assert_eq!(descriptors[1].devtools_frontend_url, None);

        let json = serde_json::to_value(&descriptors).unwrap();
        assert!(json
            .as_array()
            .unwrap()
            .iter()
            .all(|target| target.get("devtoolsFrontendUrl").is_none()));
    }

    #[test]
    fn qualified_cdp_target_ids_select_duplicate_page_keys_by_application() {
        let application = |id: &str| Application {
            id: id.into(),
            bundle_identifier: format!("com.example.{id}"),
            pid: 42,
            name: id.into(),
            availability: AutomationAvailability::Available,
            is_active: true,
            is_proxy: false,
            is_ready: true,
            host_application_identifier: None,
        };
        let pages = vec![
            ApplicationPage {
                application: application("PID:42"),
                page: Page {
                    id: 1,
                    listing_key: "1".into(),
                    page_type: WirType::WebPage,
                    title: None,
                    url: None,
                    automation_is_paired: None,
                    automation_name: None,
                    automation_version: None,
                    automation_session_id: None,
                    automation_connection_id: None,
                },
            },
            ApplicationPage {
                application: application("PID:43"),
                page: Page {
                    id: 1,
                    listing_key: "1".into(),
                    page_type: WirType::WebPage,
                    title: None,
                    url: None,
                    automation_is_paired: None,
                    automation_name: None,
                    automation_version: None,
                    automation_session_id: None,
                    automation_connection_id: None,
                },
            },
        ];

        let first = cdp_target_descriptors(&pages, "127.0.0.1", 9222);
        assert_eq!(
            first
                .iter()
                .map(|target| target.id.as_str())
                .collect::<Vec<_>>(),
            ["PID:42:1", "PID:43:1"]
        );
        let (application_id, page) = find_page_in_snapshot(&pages, "PID:43:1")
            .expect("qualified target should resolve to its owning application");
        assert_eq!(application_id, "PID:43");
        assert_eq!(page.id, 1);
    }
    #[tokio::test]
    async fn dropping_a_browser_page_ownership_guard_aborts_its_child_task() {
        let lock = Arc::new(Mutex::new(()));
        let guard = lock.clone().lock_owned().await;
        let (sender, _receiver) = mpsc::channel(1);
        let (cancel, _cancellation) = watch::channel(false);
        let task = tokio::spawn(async move {
            let _guard = guard;
            std::future::pending::<()>().await;
        });
        drop(BrowserPageSession {
            sender,
            cancel,
            task,
            outcome: browser_tasks::outcome(),
        });
        let _released = tokio::time::timeout(Duration::from_secs(1), lock.lock_owned())
            .await
            .unwrap();
    }
}
