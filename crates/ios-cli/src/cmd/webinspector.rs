use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use anyhow::{anyhow, Context, Result};
use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::routing::{get, post};
use axum::{Json, Router};
use futures_util::{SinkExt, StreamExt};
use ios_core::device::{ConnectOptions, ConnectedDevice, ServiceStream};
use ios_core::webinspector::{
    ApplicationPage, AutomationSession, By, InspectorSession, Page, WebInspectorClient, WirType,
    RSD_SERVICE_NAME, SAFARI_BUNDLE_ID, SERVICE_NAME,
};
use ios_core::TunMode;
use serde::Serialize;
use serde_json::{json, Value as JsonValue};
use tokio::net::TcpListener;
use tokio::sync::Mutex;
use uuid::Uuid;

const WD_ELEMENT_KEY: &str = "element-6066-11e4-a52e-4f735466cecf";

#[derive(clap::Args)]
pub struct WebInspectorCmd {
    #[command(subcommand)]
    sub: WebInspectorSub,
}

#[derive(clap::Subcommand)]
enum WebInspectorSub {
    OpenedTabs {
        #[arg(short = 't', long, default_value = "3.0")]
        timeout: f64,
    },
    Eval {
        expression: String,
        #[arg(long)]
        app_id: Option<String>,
        #[arg(long)]
        bundle_id: Option<String>,
        #[arg(long)]
        page_id: u64,
        #[arg(short = 't', long, default_value = "3.0")]
        timeout: f64,
    },
    Cdp {
        #[arg(long, default_value = "127.0.0.1")]
        host: String,
        #[arg(long, default_value_t = 9222)]
        port: u16,
        #[arg(short = 't', long, default_value = "3.0")]
        timeout: f64,
    },
    Selenium {
        #[arg(long, default_value = "127.0.0.1")]
        host: String,
        #[arg(long, default_value_t = 4444)]
        port: u16,
        #[arg(short = 't', long, default_value = "3.0")]
        timeout: f64,
    },
}

#[derive(Debug, Serialize)]
struct OpenedTabRow {
    application_id: String,
    bundle_identifier: String,
    application_name: String,
    pid: u64,
    page_id: u64,
    page_type: String,
    title: Option<String>,
    url: Option<String>,
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
    devtools_frontend_url: String,
}

#[derive(Clone)]
struct ServerState {
    udid: String,
    timeout: Duration,
    cdp_host: String,
    cdp_port: u16,
    selenium_sessions: Arc<Mutex<HashMap<String, Arc<Mutex<SeleniumRuntime>>>>>,
}

struct SeleniumRuntime {
    _device: ConnectedDevice,
    client: WebInspectorClient<ServiceStream>,
    automation: AutomationSession,
    elements: HashMap<String, JsonValue>,
}

impl SeleniumRuntime {
    async fn stop(&mut self) -> Result<()> {
        self.automation.stop_session(&mut self.client).await?;
        Ok(())
    }

    async fn current_url(&mut self) -> Result<Option<String>> {
        self.automation
            .current_url(&mut self.client)
            .await
            .map_err(Into::into)
    }

    async fn navigate(&mut self, url: &str) -> Result<()> {
        self.automation.navigate(&mut self.client, url).await?;
        Ok(())
    }

    async fn go_back(&mut self) -> Result<()> {
        self.automation.go_back(&mut self.client).await?;
        Ok(())
    }

    async fn go_forward(&mut self) -> Result<()> {
        self.automation.go_forward(&mut self.client).await?;
        Ok(())
    }

    async fn refresh(&mut self) -> Result<()> {
        self.automation.refresh(&mut self.client).await?;
        Ok(())
    }

    async fn title(&mut self) -> Result<String> {
        self.automation
            .get_title(&mut self.client)
            .await
            .map_err(Into::into)
    }

    async fn page_source(&mut self) -> Result<String> {
        self.automation
            .get_page_source(&mut self.client)
            .await
            .map_err(Into::into)
    }

    async fn execute_script(&mut self, script: &str, args: &[JsonValue]) -> Result<JsonValue> {
        self.automation
            .execute_script(&mut self.client, script, args)
            .await
            .map_err(Into::into)
    }

    async fn screenshot_base64(&mut self) -> Result<String> {
        self.automation
            .screenshot_base64(&mut self.client)
            .await
            .map_err(Into::into)
    }

    async fn find_element(&mut self, by: By, value: &str) -> Result<Option<JsonValue>> {
        self.automation
            .find_element(&mut self.client, by, value)
            .await
            .map_err(Into::into)
    }

    async fn find_elements(&mut self, by: By, value: &str) -> Result<Vec<JsonValue>> {
        self.automation
            .find_elements(&mut self.client, by, value, false)
            .await
            .map_err(Into::into)
    }

    async fn element_text(&mut self, raw: &JsonValue) -> Result<String> {
        self.automation
            .element_text(&mut self.client, raw)
            .await
            .map_err(Into::into)
    }

    async fn element_tag_name(&mut self, raw: &JsonValue) -> Result<String> {
        self.automation
            .element_tag_name(&mut self.client, raw)
            .await
            .map_err(Into::into)
    }

    async fn click_element(&mut self, raw: &JsonValue) -> Result<()> {
        self.automation.click_element(&mut self.client, raw).await?;
        Ok(())
    }
}

#[derive(Debug)]
struct AppError {
    status: StatusCode,
    message: String,
}

impl AppError {
    fn bad_request(message: impl Into<String>) -> Self {
        Self {
            status: StatusCode::BAD_REQUEST,
            message: message.into(),
        }
    }
}

impl IntoResponse for AppError {
    fn into_response(self) -> axum::response::Response {
        (
            self.status,
            Json(json!({ "value": { "error": "unknown error", "message": self.message } })),
        )
            .into_response()
    }
}

impl From<anyhow::Error> for AppError {
    fn from(value: anyhow::Error) -> Self {
        Self {
            status: StatusCode::INTERNAL_SERVER_ERROR,
            message: value.to_string(),
        }
    }
}

impl From<axum::Error> for AppError {
    fn from(value: axum::Error) -> Self {
        Self {
            status: StatusCode::INTERNAL_SERVER_ERROR,
            message: value.to_string(),
        }
    }
}

impl From<serde_json::Error> for AppError {
    fn from(value: serde_json::Error) -> Self {
        Self {
            status: StatusCode::BAD_REQUEST,
            message: value.to_string(),
        }
    }
}

impl WebInspectorCmd {
    pub async fn run(self, udid: Option<String>, json_output: bool) -> Result<()> {
        let udid = udid.ok_or_else(|| anyhow!("--udid required for webinspector"))?;
        match self.sub {
            WebInspectorSub::OpenedTabs { timeout } => {
                run_opened_tabs(&udid, duration_from_secs(timeout), json_output).await
            }
            WebInspectorSub::Eval {
                expression,
                app_id,
                bundle_id,
                page_id,
                timeout,
            } => {
                run_eval(
                    &udid,
                    app_id,
                    bundle_id.unwrap_or_else(|| SAFARI_BUNDLE_ID.to_string()),
                    page_id,
                    &expression,
                    duration_from_secs(timeout),
                    json_output,
                )
                .await
            }
            WebInspectorSub::Cdp {
                host,
                port,
                timeout,
            } => run_cdp_server(&udid, host, port, duration_from_secs(timeout)).await,
            WebInspectorSub::Selenium {
                host,
                port,
                timeout,
            } => run_selenium_server(&udid, host, port, duration_from_secs(timeout)).await,
        }
    }
}

async fn run_opened_tabs(udid: &str, timeout: Duration, json_output: bool) -> Result<()> {
    let (_device, stream, _use_rsd) = connect_webinspector(udid).await?;
    let mut client = WebInspectorClient::new(stream);
    client.start(timeout).await?;
    let pages = client.open_application_pages(timeout).await?;
    let rows = pages.into_iter().map(opened_tab_row).collect::<Vec<_>>();

    if json_output {
        println!("{}", serde_json::to_string_pretty(&rows)?);
    } else if rows.is_empty() {
        println!("No inspectable pages reported");
    } else {
        for row in rows {
            println!(
                "{} [{}] page={} {} {}",
                row.application_name,
                row.bundle_identifier,
                row.page_id,
                row.title.as_deref().unwrap_or("<no title>"),
                row.url.as_deref().unwrap_or("<no url>")
            );
            println!("  app_id: {}", row.application_id);
            println!("  type: {}", row.page_type);
        }
    }
    Ok(())
}

async fn run_eval(
    udid: &str,
    app_id: Option<String>,
    bundle_id: String,
    page_id: u64,
    expression: &str,
    timeout: Duration,
    json_output: bool,
) -> Result<()> {
    let (_device, stream, _use_rsd) = connect_webinspector(udid).await?;
    let mut client = WebInspectorClient::new(stream);
    client.start(timeout).await?;
    client.open_application_pages(timeout).await?;

    let application_id = match app_id {
        Some(app_id) => app_id,
        None => client
            .application_by_bundle(&bundle_id)
            .map(|application| application.id.clone())
            .ok_or_else(|| anyhow!("bundle '{bundle_id}' is not currently inspectable"))?,
    };

    client.page(&application_id, page_id).ok_or_else(|| {
        anyhow!("page {page_id} was not found under application '{application_id}'")
    })?;

    let mut session = InspectorSession::new(application_id.clone(), page_id);
    session.attach(&mut client, true, timeout).await?;
    let response = session
        .send_command_and_wait(
            &mut client,
            "Runtime.evaluate",
            runtime_evaluate_params(expression),
            timeout,
        )
        .await?;

    if json_output {
        println!("{}", serde_json::to_string_pretty(&response)?);
    } else if let Some(value) = response.pointer("/result/result/value") {
        println!("{}", value.as_str().unwrap_or(&value.to_string()));
    } else if let Some(description) = response.pointer("/result/result/description") {
        println!("{description}");
    } else {
        println!("{}", serde_json::to_string_pretty(&response)?);
    }
    Ok(())
}

async fn run_cdp_server(udid: &str, host: String, port: u16, timeout: Duration) -> Result<()> {
    let state = ServerState {
        udid: udid.to_string(),
        timeout,
        cdp_host: host.clone(),
        cdp_port: port,
        selenium_sessions: Arc::new(Mutex::new(HashMap::new())),
    };
    let app = Router::new()
        .route("/json", get(cdp_targets))
        .route("/json/list", get(cdp_targets))
        .route("/json/version", get(cdp_version))
        .route("/devtools/page/:page_id", get(cdp_page_ws))
        .with_state(state);
    serve(host, port, app).await
}

async fn run_selenium_server(udid: &str, host: String, port: u16, timeout: Duration) -> Result<()> {
    let state = ServerState {
        udid: udid.to_string(),
        timeout,
        cdp_host: "127.0.0.1".to_string(),
        cdp_port: 9222,
        selenium_sessions: Arc::new(Mutex::new(HashMap::new())),
    };
    let app = Router::new()
        .route("/status", get(webdriver_status))
        .route("/session", post(webdriver_new_session))
        .route(
            "/session/:session_id",
            axum::routing::delete(webdriver_delete_session),
        )
        .route(
            "/session/:session_id/url",
            get(webdriver_get_url).post(webdriver_navigate),
        )
        .route("/session/:session_id/back", post(webdriver_back))
        .route("/session/:session_id/forward", post(webdriver_forward))
        .route("/session/:session_id/refresh", post(webdriver_refresh))
        .route("/session/:session_id/title", get(webdriver_title))
        .route("/session/:session_id/source", get(webdriver_source))
        .route(
            "/session/:session_id/execute/sync",
            post(webdriver_execute_sync),
        )
        .route("/session/:session_id/screenshot", get(webdriver_screenshot))
        .route("/session/:session_id/element", post(webdriver_find_element))
        .route(
            "/session/:session_id/elements",
            post(webdriver_find_elements),
        )
        .route(
            "/session/:session_id/element/:element_id/text",
            get(webdriver_element_text),
        )
        .route(
            "/session/:session_id/element/:element_id/name",
            get(webdriver_element_name),
        )
        .route(
            "/session/:session_id/element/:element_id/click",
            post(webdriver_element_click),
        )
        .with_state(state);
    serve(host, port, app).await
}

async fn serve(host: String, port: u16, app: Router) -> Result<()> {
    let addr: SocketAddr = format!("{host}:{port}")
        .parse()
        .with_context(|| format!("invalid listen address {host}:{port}"))?;
    let listener = TcpListener::bind(addr).await?;
    println!("Listening on http://{addr}");
    axum::serve(listener, app).await?;
    Ok(())
}

async fn cdp_targets(
    State(state): State<ServerState>,
) -> Result<Json<Vec<CdpTargetDescriptor>>, AppError> {
    let pages = load_open_pages(&state.udid, state.timeout).await?;
    Ok(Json(cdp_target_descriptors(
        &pages,
        &state.cdp_host,
        state.cdp_port,
    )))
}

async fn cdp_version(State(state): State<ServerState>) -> Json<JsonValue> {
    Json(json!({
        "Browser": "Safari",
        "Protocol-Version": "1.1",
        "User-Agent": "ios-cli",
        "V8-Version": "7.2.233",
        "WebKit-Version": "537.36",
        "webSocketDebuggerUrl": format!("ws://{}:{}/devtools/browser/ios-cli", state.cdp_host, state.cdp_port),
    }))
}

async fn cdp_page_ws(
    ws: WebSocketUpgrade,
    State(state): State<ServerState>,
    Path(page_id): Path<String>,
) -> Result<impl IntoResponse, AppError> {
    let udid = state.udid.clone();
    let timeout = state.timeout;
    Ok(ws.on_upgrade(move |socket| async move {
        if let Err(error) = handle_cdp_socket(socket, &udid, timeout, &page_id).await {
            eprintln!("cdp bridge closed with error: {error:#}");
        }
    }))
}

async fn handle_cdp_socket(
    socket: WebSocket,
    udid: &str,
    timeout: Duration,
    page_id: &str,
) -> Result<()> {
    let (_device, stream, _use_rsd) = connect_webinspector(udid).await?;
    let mut client = WebInspectorClient::new(stream);
    client.start(timeout).await?;
    client.open_application_pages(timeout).await?;
    let (application_id, page) = find_page_by_id(&client, page_id)?;
    let mut session = InspectorSession::new(application_id, page.id);
    session.attach(&mut client, true, timeout).await?;

    let (mut sender, mut receiver) = socket.split();
    loop {
        tokio::select! {
            inbound = receiver.next() => {
                match inbound {
                    Some(Ok(Message::Text(text))) => {
                        let message: JsonValue = serde_json::from_str(text.as_ref())?;
                        session.send_bridge_message(&mut client, &message).await?;
                    }
                    Some(Ok(Message::Close(_))) | None => break,
                    Some(Ok(_)) => {}
                    Some(Err(error)) => return Err(error.into()),
                }
            }
            outbound = session.next_raw_message(&mut client, timeout) => {
                let message = outbound?;
                if let Some(message) = session.bridge_message(&message)? {
                    sender.send(Message::Text(serde_json::to_string(&message)?)).await?;
                }
            }
        }
    }
    Ok(())
}

async fn webdriver_status() -> Json<JsonValue> {
    Json(
        json!({ "value": { "ready": true, "message": "ios-cli Safari automation bridge is ready" } }),
    )
}

async fn webdriver_new_session(
    State(state): State<ServerState>,
) -> Result<Json<JsonValue>, AppError> {
    let runtime = build_selenium_runtime(&state.udid, state.timeout).await?;
    let session_id = Uuid::new_v4().to_string();
    state
        .selenium_sessions
        .lock()
        .await
        .insert(session_id.clone(), Arc::new(Mutex::new(runtime)));
    Ok(Json(json!({
        "value": {
            "sessionId": session_id,
            "capabilities": {
                "browserName": "Safari",
                "platformName": "iOS",
                "acceptInsecureCerts": true
            }
        }
    })))
}

async fn webdriver_delete_session(
    State(state): State<ServerState>,
    Path(session_id): Path<String>,
) -> Result<Json<JsonValue>, AppError> {
    if let Some(runtime) = state.selenium_sessions.lock().await.remove(&session_id) {
        let mut runtime = runtime.lock().await;
        let _ = runtime.stop().await;
    }
    Ok(Json(json!({ "value": null })))
}

async fn webdriver_get_url(
    State(state): State<ServerState>,
    Path(session_id): Path<String>,
) -> Result<Json<JsonValue>, AppError> {
    let session = get_runtime(&state, &session_id).await?;
    let mut session = session.lock().await;
    let value = session.current_url().await?;
    Ok(Json(json!({ "value": value })))
}

async fn webdriver_navigate(
    State(state): State<ServerState>,
    Path(session_id): Path<String>,
    Json(body): Json<JsonValue>,
) -> Result<Json<JsonValue>, AppError> {
    let url = body
        .get("url")
        .and_then(JsonValue::as_str)
        .unwrap_or_default();
    let session = get_runtime(&state, &session_id).await?;
    let mut session = session.lock().await;
    session.navigate(url).await?;
    Ok(Json(json!({ "value": null })))
}

async fn webdriver_back(
    State(state): State<ServerState>,
    Path(session_id): Path<String>,
) -> Result<Json<JsonValue>, AppError> {
    let session = get_runtime(&state, &session_id).await?;
    let mut session = session.lock().await;
    session.go_back().await?;
    Ok(Json(json!({ "value": null })))
}

async fn webdriver_forward(
    State(state): State<ServerState>,
    Path(session_id): Path<String>,
) -> Result<Json<JsonValue>, AppError> {
    let session = get_runtime(&state, &session_id).await?;
    let mut session = session.lock().await;
    session.go_forward().await?;
    Ok(Json(json!({ "value": null })))
}

async fn webdriver_refresh(
    State(state): State<ServerState>,
    Path(session_id): Path<String>,
) -> Result<Json<JsonValue>, AppError> {
    let session = get_runtime(&state, &session_id).await?;
    let mut session = session.lock().await;
    session.refresh().await?;
    Ok(Json(json!({ "value": null })))
}

async fn webdriver_title(
    State(state): State<ServerState>,
    Path(session_id): Path<String>,
) -> Result<Json<JsonValue>, AppError> {
    let session = get_runtime(&state, &session_id).await?;
    let mut session = session.lock().await;
    let value = session.title().await?;
    Ok(Json(json!({ "value": value })))
}

async fn webdriver_source(
    State(state): State<ServerState>,
    Path(session_id): Path<String>,
) -> Result<Json<JsonValue>, AppError> {
    let session = get_runtime(&state, &session_id).await?;
    let mut session = session.lock().await;
    let value = session.page_source().await?;
    Ok(Json(json!({ "value": value })))
}

async fn webdriver_execute_sync(
    State(state): State<ServerState>,
    Path(session_id): Path<String>,
    Json(body): Json<JsonValue>,
) -> Result<Json<JsonValue>, AppError> {
    let script = body
        .get("script")
        .and_then(JsonValue::as_str)
        .unwrap_or_default();
    let args = body
        .get("args")
        .and_then(JsonValue::as_array)
        .cloned()
        .unwrap_or_default();
    let session = get_runtime(&state, &session_id).await?;
    let mut session = session.lock().await;
    let args = args
        .iter()
        .map(|value| decode_webdriver_arg(value, &session.elements))
        .collect::<Vec<_>>();
    let value = session.execute_script(script, &args).await?;
    Ok(Json(
        json!({ "value": encode_webdriver_value(value, &mut session.elements) }),
    ))
}

async fn webdriver_screenshot(
    State(state): State<ServerState>,
    Path(session_id): Path<String>,
) -> Result<Json<JsonValue>, AppError> {
    let session = get_runtime(&state, &session_id).await?;
    let mut session = session.lock().await;
    let value = session.screenshot_base64().await?;
    Ok(Json(json!({ "value": value })))
}

async fn webdriver_find_element(
    State(state): State<ServerState>,
    Path(session_id): Path<String>,
    Json(body): Json<JsonValue>,
) -> Result<Json<JsonValue>, AppError> {
    let by = parse_by(
        body.get("using")
            .and_then(JsonValue::as_str)
            .unwrap_or("css selector"),
    )?;
    let value = body
        .get("value")
        .and_then(JsonValue::as_str)
        .unwrap_or_default();
    let session = get_runtime(&state, &session_id).await?;
    let mut session = session.lock().await;
    let element = session.find_element(by, value).await?;
    let value = element
        .map(|raw| register_webdriver_element(&mut session.elements, raw))
        .unwrap_or(JsonValue::Null);
    Ok(Json(json!({ "value": value })))
}

async fn webdriver_find_elements(
    State(state): State<ServerState>,
    Path(session_id): Path<String>,
    Json(body): Json<JsonValue>,
) -> Result<Json<JsonValue>, AppError> {
    let by = parse_by(
        body.get("using")
            .and_then(JsonValue::as_str)
            .unwrap_or("css selector"),
    )?;
    let value = body
        .get("value")
        .and_then(JsonValue::as_str)
        .unwrap_or_default();
    let session = get_runtime(&state, &session_id).await?;
    let mut session = session.lock().await;
    let elements = session.find_elements(by, value).await?;
    let elements = elements
        .into_iter()
        .map(|raw| register_webdriver_element(&mut session.elements, raw))
        .collect::<Vec<_>>();
    Ok(Json(json!({ "value": elements })))
}

async fn webdriver_element_text(
    State(state): State<ServerState>,
    Path((session_id, element_id)): Path<(String, String)>,
) -> Result<Json<JsonValue>, AppError> {
    let session = get_runtime(&state, &session_id).await?;
    let mut session = session.lock().await;
    let raw = session
        .elements
        .get(&element_id)
        .cloned()
        .ok_or_else(|| AppError::bad_request("unknown element id"))?;
    let value = session.element_text(&raw).await?;
    Ok(Json(json!({ "value": value })))
}

async fn webdriver_element_name(
    State(state): State<ServerState>,
    Path((session_id, element_id)): Path<(String, String)>,
) -> Result<Json<JsonValue>, AppError> {
    let session = get_runtime(&state, &session_id).await?;
    let mut session = session.lock().await;
    let raw = session
        .elements
        .get(&element_id)
        .cloned()
        .ok_or_else(|| AppError::bad_request("unknown element id"))?;
    let value = session.element_tag_name(&raw).await?;
    Ok(Json(json!({ "value": value })))
}

async fn webdriver_element_click(
    State(state): State<ServerState>,
    Path((session_id, element_id)): Path<(String, String)>,
) -> Result<Json<JsonValue>, AppError> {
    let session = get_runtime(&state, &session_id).await?;
    let mut session = session.lock().await;
    let raw = session
        .elements
        .get(&element_id)
        .cloned()
        .ok_or_else(|| AppError::bad_request("unknown element id"))?;
    session.click_element(&raw).await?;
    Ok(Json(json!({ "value": null })))
}

async fn get_runtime(
    state: &ServerState,
    session_id: &str,
) -> Result<Arc<Mutex<SeleniumRuntime>>, AppError> {
    state
        .selenium_sessions
        .lock()
        .await
        .get(session_id)
        .cloned()
        .ok_or_else(|| AppError::bad_request("unknown webdriver session"))
}

async fn build_selenium_runtime(udid: &str, timeout: Duration) -> Result<SeleniumRuntime> {
    let (device, stream, _use_rsd) = connect_webinspector(udid).await?;
    let mut client = WebInspectorClient::new(stream);
    client.start(timeout).await?;
    client.open_application_pages(timeout).await?;

    let application = match client.application_by_bundle(SAFARI_BUNDLE_ID) {
        Some(application) => application.clone(),
        None => {
            client.request_application_launch(SAFARI_BUNDLE_ID).await?;
            client.open_application_pages(timeout).await?;
            client
                .application_by_bundle(SAFARI_BUNDLE_ID)
                .cloned()
                .ok_or_else(|| anyhow!("Safari is not currently inspectable"))?
        }
    };

    let mut automation = AutomationSession::new(
        application.id.clone(),
        application.bundle_identifier.clone(),
    );
    automation.attach(&mut client, timeout).await?;
    automation.start_session(&mut client).await?;

    Ok(SeleniumRuntime {
        _device: device,
        client,
        automation,
        elements: HashMap::new(),
    })
}

async fn load_open_pages(udid: &str, timeout: Duration) -> Result<Vec<ApplicationPage>> {
    let (_device, stream, _use_rsd) = connect_webinspector(udid).await?;
    let mut client = WebInspectorClient::new(stream);
    client.start(timeout).await?;
    client
        .open_application_pages(timeout)
        .await
        .map_err(Into::into)
}

fn cdp_target_descriptors(
    pages: &[ApplicationPage],
    host: &str,
    port: u16,
) -> Vec<CdpTargetDescriptor> {
    pages
        .iter()
        .filter(|page| matches!(page.page.page_type, WirType::Web | WirType::WebPage))
        .map(|page| CdpTargetDescriptor {
            description: String::new(),
            id: page.page.id.to_string(),
            title: page.page.title.clone().unwrap_or_default(),
            target_type: "page".to_string(),
            url: page.page.url.clone().unwrap_or_default(),
            web_socket_debugger_url: format!("ws://{host}:{port}/devtools/page/{}", page.page.id),
            devtools_frontend_url: format!(
                "/devtools/inspector.html?ws://{host}:{port}/devtools/page/{}",
                page.page.id
            ),
        })
        .collect()
}

fn find_page_by_id(
    client: &WebInspectorClient<ServiceStream>,
    page_id: &str,
) -> Result<(String, Page)> {
    for page in client.open_pages_snapshot() {
        if page.page.id.to_string() == page_id {
            return Ok((page.application.id, page.page));
        }
    }
    Err(anyhow!("inspectable page {page_id} not found"))
}

async fn connect_webinspector(udid: &str) -> Result<(ConnectedDevice, ServiceStream, bool)> {
    let probe = ios_core::connect(
        udid,
        ConnectOptions {
            tun_mode: TunMode::Userspace,
            pair_record_path: None,
            skip_tunnel: true,
        },
    )
    .await?;
    let version = probe.product_version().await?;
    drop(probe);

    if version.major >= 17 {
        let device = ios_core::connect(
            udid,
            ConnectOptions {
                tun_mode: TunMode::Userspace,
                pair_record_path: None,
                skip_tunnel: false,
            },
        )
        .await?;
        let stream = device.connect_rsd_service(RSD_SERVICE_NAME).await?;
        return Ok((device, stream, true));
    }

    let device = ios_core::connect(
        udid,
        ConnectOptions {
            tun_mode: TunMode::Userspace,
            pair_record_path: None,
            skip_tunnel: true,
        },
    )
    .await?;
    let stream = device.connect_service(SERVICE_NAME).await?;
    Ok((device, stream, false))
}

fn opened_tab_row(page: ApplicationPage) -> OpenedTabRow {
    let ApplicationPage { application, page } = page;
    OpenedTabRow {
        application_id: application.id,
        bundle_identifier: application.bundle_identifier,
        application_name: application.name,
        pid: application.pid,
        page_id: page.id,
        page_type: serde_json::to_string(&page.page_type)
            .unwrap_or_else(|_| "\"unknown\"".to_string())
            .trim_matches('"')
            .to_string(),
        title: page.title,
        url: page.url,
    }
}

fn runtime_evaluate_params(expression: &str) -> JsonValue {
    json!({
        "expression": expression,
        "objectGroup": "console",
        "includeCommandLineAPI": true,
        "doNotPauseOnExceptionsAndMuteConsole": false,
        "silent": false,
        "returnByValue": true,
        "generatePreview": true,
        "userGesture": true,
        "awaitPromise": false,
        "replMode": true,
        "allowUnsafeEvalBlockedByCSP": false,
        "uniqueContextId": "0.1"
    })
}

fn duration_from_secs(seconds: f64) -> Duration {
    Duration::from_secs_f64(seconds.max(0.1))
}

fn parse_by(value: &str) -> Result<By, AppError> {
    Ok(match value {
        "id" => By::Id,
        "xpath" => By::XPath,
        "link text" => By::LinkText,
        "partial link text" => By::PartialLinkText,
        "name" => By::Name,
        "tag name" => By::TagName,
        "class name" => By::ClassName,
        "css selector" => By::CssSelector,
        other => {
            return Err(AppError::bad_request(format!(
                "unsupported locator strategy: {other}"
            )))
        }
    })
}

fn register_webdriver_element(store: &mut HashMap<String, JsonValue>, raw: JsonValue) -> JsonValue {
    let id = Uuid::new_v4().to_string();
    store.insert(id.clone(), raw);
    json!({ WD_ELEMENT_KEY: id })
}

fn decode_webdriver_arg(value: &JsonValue, store: &HashMap<String, JsonValue>) -> JsonValue {
    match value {
        JsonValue::Object(map) => {
            if let Some(id) = map.get(WD_ELEMENT_KEY).and_then(JsonValue::as_str) {
                return store.get(id).cloned().unwrap_or(JsonValue::Null);
            }
            JsonValue::Object(
                map.iter()
                    .map(|(key, value)| (key.clone(), decode_webdriver_arg(value, store)))
                    .collect(),
            )
        }
        JsonValue::Array(values) => JsonValue::Array(
            values
                .iter()
                .map(|value| decode_webdriver_arg(value, store))
                .collect(),
        ),
        _ => value.clone(),
    }
}

fn encode_webdriver_value(value: JsonValue, store: &mut HashMap<String, JsonValue>) -> JsonValue {
    match value {
        JsonValue::Array(values) => JsonValue::Array(
            values
                .into_iter()
                .map(|value| encode_webdriver_value(value, store))
                .collect(),
        ),
        JsonValue::Object(map) if map.keys().any(|key| key.starts_with("session-node-")) => {
            register_webdriver_element(store, JsonValue::Object(map))
        }
        other => other,
    }
}

#[cfg(test)]
mod tests {
    use clap::Parser;
    use ios_core::webinspector::{Application, AutomationAvailability};

    use super::*;

    #[derive(Parser)]
    struct TestCli {
        #[command(subcommand)]
        command: WebInspectorSub,
    }

    #[test]
    fn parses_opened_tabs_subcommand() {
        assert!(TestCli::try_parse_from(["webinspector", "opened-tabs", "--timeout", "2"]).is_ok());
    }

    #[test]
    fn parses_eval_subcommand() {
        assert!(TestCli::try_parse_from([
            "webinspector",
            "eval",
            "1+1",
            "--page-id",
            "7",
            "--bundle-id",
            "com.apple.mobilesafari"
        ])
        .is_ok());
    }

    #[test]
    fn parses_cdp_subcommand() {
        assert!(TestCli::try_parse_from([
            "webinspector",
            "cdp",
            "--host",
            "127.0.0.1",
            "--port",
            "9222"
        ])
        .is_ok());
    }

    #[test]
    fn parses_selenium_subcommand() {
        assert!(TestCli::try_parse_from([
            "webinspector",
            "selenium",
            "--host",
            "127.0.0.1",
            "--port",
            "4444",
            "--timeout",
            "5"
        ])
        .is_ok());
    }

    #[test]
    fn cdp_target_descriptors_filter_non_web_pages() {
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
        ];
        let descriptors = cdp_target_descriptors(&pages, "127.0.0.1", 9222);
        assert_eq!(descriptors.len(), 1);
        assert_eq!(descriptors[0].id, "7");
        assert_eq!(
            descriptors[0].web_socket_debugger_url,
            "ws://127.0.0.1:9222/devtools/page/7"
        );
    }
}
