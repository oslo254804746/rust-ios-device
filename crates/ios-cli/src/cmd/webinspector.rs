use std::collections::HashMap;
use std::io::{IsTerminal, Write as StdWrite};
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
    ApplicationPage, AutomationSession, By, InspectorSession, Page, WebInspectorClient,
    WebInspectorError, WirType, RSD_SERVICE_NAME, SAFARI_BUNDLE_ID, SERVICE_NAME,
};
use ios_core::TunMode;
use serde::Serialize;
use serde_json::{json, Value as JsonValue};
use tokio::io::{AsyncBufRead, AsyncBufReadExt, BufReader};
use tokio::net::TcpListener;
use tokio::sync::Mutex;
use tokio::time::Instant;
use uuid::Uuid;

const WD_ELEMENT_KEY: &str = "element-6066-11e4-a52e-4f735466cecf";
const PAGE_DISCOVERY_IDLE: Duration = Duration::from_millis(500);
const SESSION_CLEANUP_MAX: Duration = Duration::from_millis(100);
// A shell expression is sent as one Web Inspector message. Keep input bounded
// so an unterminated pipe line cannot consume memory indefinitely. This is
// large enough for normal scripts while remaining below the plist message cap.
const MAX_SHELL_INPUT_BYTES: usize = 1024 * 1024;

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
        page_id: String,
        #[arg(short = 't', long, default_value = "3.0")]
        timeout: f64,
    },
    /// Launch an app through Remote Automation and optionally navigate it.
    Launch {
        #[arg(value_name = "URL")]
        url: Option<String>,
        #[arg(long, default_value = SAFARI_BUNDLE_ID)]
        bundle_id: String,
        #[arg(short = 't', long, default_value = "5.0")]
        timeout: f64,
    },
    /// Evaluate JavaScript line-by-line in an inspectable page.
    JsShell {
        #[arg(value_name = "URL")]
        url: Option<String>,
        #[arg(long)]
        bundle_id: Option<String>,
        #[arg(long)]
        page_id: Option<String>,
        #[arg(long)]
        open_safari: bool,
        #[arg(
            long,
            default_value_t = true,
            action = clap::ArgAction::Set,
            help = "Continue after an evaluation error (use --continue-on-error=false to stop)"
        )]
        continue_on_error: bool,
        #[arg(short = 't', long, default_value = "5.0")]
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
    page_key: String,
    page_type: String,
    title: Option<String>,
    url: Option<String>,
}

#[derive(Debug, Serialize)]
struct LaunchResult {
    bundle_id: String,
    application_id: String,
    application_name: String,
    pid: u64,
    page_id: u64,
    page_key: String,
    page_type: String,
    url: Option<String>,
    title: String,
    session_id: String,
    inspector_connection_id: String,
    automation_connection_id: Option<String>,
    service_name: &'static str,
    uses_rsd: bool,
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
                run_opened_tabs(&udid, duration_from_secs(timeout)?, json_output).await
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
                    &page_id,
                    &expression,
                    duration_from_secs(timeout)?,
                    json_output,
                )
                .await
            }
            WebInspectorSub::Launch {
                url,
                bundle_id,
                timeout,
            } => {
                run_launch(
                    &udid,
                    &bundle_id,
                    url.as_deref(),
                    duration_from_secs(timeout)?,
                    json_output,
                )
                .await
            }
            WebInspectorSub::JsShell {
                url,
                bundle_id,
                page_id,
                open_safari,
                continue_on_error,
                timeout,
            } => {
                run_js_shell(
                    &udid,
                    url.as_deref(),
                    bundle_id.as_deref(),
                    page_id.as_deref(),
                    open_safari,
                    continue_on_error,
                    duration_from_secs(timeout)?,
                    json_output,
                )
                .await
            }
            WebInspectorSub::Cdp {
                host,
                port,
                timeout,
            } => run_cdp_server(&udid, host, port, duration_from_secs(timeout)?).await,
            WebInspectorSub::Selenium {
                host,
                port,
                timeout,
            } => run_selenium_server(&udid, host, port, duration_from_secs(timeout)?).await,
        }
    }
}

async fn run_opened_tabs(udid: &str, timeout: Duration, json_output: bool) -> Result<()> {
    let (_device, stream, _use_rsd) = connect_webinspector_with_timeout(udid, timeout).await?;
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
    page_selector: &str,
    expression: &str,
    timeout: Duration,
    json_output: bool,
) -> Result<()> {
    let (_device, stream, _use_rsd) = connect_webinspector_with_timeout(udid, timeout).await?;
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

    let page = client
        .application_pages(&application_id)
        .into_iter()
        .flat_map(|pages| pages.values())
        .find(|page| {
            is_inspectable_page_type(&page.page_type) && page_matches_selector(page, page_selector)
        })
        .cloned()
        .ok_or_else(|| {
            anyhow!(
                "inspectable page {page_selector} was not found under application '{application_id}'"
            )
        })?;
    let page_id = page.id;

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
    if let Some(error) = runtime_evaluation_error(&response) {
        return Err(anyhow!("webinspector evaluate error: {error}"));
    }

    if json_output {
        println!("{}", serde_json::to_string_pretty(&response)?);
    } else {
        print_runtime_evaluation_human(&response)?;
    }
    Ok(())
}

async fn run_launch(
    udid: &str,
    bundle_id: &str,
    url: Option<&str>,
    timeout: Duration,
    json_output: bool,
) -> Result<()> {
    let operation = run_launch_inner(udid, bundle_id, url, timeout);
    tokio::pin!(operation);
    let result = tokio::select! {
        result = tokio::time::timeout(timeout, &mut operation) => {
            result.map_err(|_| anyhow!("webinspector launch timed out after {timeout:?}"))?
        }
        signal = tokio::signal::ctrl_c() => {
            signal.context("failed waiting for Ctrl+C")?;
            Err(anyhow!("webinspector launch cancelled"))
        }
    }?;

    if json_output {
        println!("{}", serde_json::to_string_pretty(&result)?);
    } else {
        println!("Launched {} (pid {})", result.bundle_id, result.pid);
        println!("  page: {}", result.page_id);
        println!("  url: {}", result.url.as_deref().unwrap_or("<no url>"));
        println!("  title: {}", result.title);
        println!("  session: {}", result.session_id);
        if let Some(connection_id) = result.automation_connection_id.as_deref() {
            println!("  automation connection: {connection_id}");
        }
        println!("  service: {}", result.service_name);
    }
    Ok(())
}

async fn run_launch_inner(
    udid: &str,
    bundle_id: &str,
    url: Option<&str>,
    timeout: Duration,
) -> Result<LaunchResult> {
    let deadline = webinspector_deadline(timeout)?;
    let (device, stream, uses_rsd) = connect_webinspector(udid).await?;
    let mut client = WebInspectorClient::new(stream);
    client
        .start(remaining_webinspector_time(deadline, timeout)?)
        .await?;
    client.request_application_launch(bundle_id).await?;
    let application = wait_for_application(&mut client, bundle_id, deadline, timeout).await?;
    let mut automation = AutomationSession::new(
        application.id.clone(),
        application.bundle_identifier.clone(),
    );
    automation
        .attach(&mut client, remaining_webinspector_time(deadline, timeout)?)
        .await?;
    automation.start_session(&mut client).await?;
    if let Some(url) = url.filter(|url| !url.is_empty()) {
        automation.navigate(&mut client, url).await?;
    }
    let current_url = automation.current_url(&mut client).await?;
    let title = automation.get_title(&mut client).await?;
    let page = client
        .page(&application.id, automation.page_id())
        .cloned()
        .ok_or_else(|| anyhow!("automation page {} disappeared", automation.page_id()))?;
    let result = LaunchResult {
        bundle_id: application.bundle_identifier,
        application_id: application.id,
        application_name: application.name,
        pid: application.pid,
        page_id: page.id,
        page_key: page.listing_key,
        page_type: page_type_name(&page.page_type),
        url: current_url.or(page.url),
        title,
        session_id: automation.session_id().to_string(),
        inspector_connection_id: client.connection_id().to_string(),
        automation_connection_id: page.automation_connection_id,
        service_name: if uses_rsd {
            RSD_SERVICE_NAME
        } else {
            SERVICE_NAME
        },
        uses_rsd,
    };
    // Closing the browsing context is best effort. The result has already been
    // collected, and a device disconnect should not turn a successful launch
    // into a second attempt or another launch request.
    // Cleanup must not consume the operation deadline: the launch result is
    // already complete, and disconnecting the service is also safe cleanup.
    if let Ok(remaining) = remaining_webinspector_time(deadline, timeout) {
        let _ = tokio::time::timeout(
            remaining.min(SESSION_CLEANUP_MAX),
            automation.stop_session(&mut client),
        )
        .await;
    }
    drop(device);
    Ok(result)
}

#[allow(clippy::too_many_arguments)]
async fn run_js_shell(
    udid: &str,
    url: Option<&str>,
    bundle_id: Option<&str>,
    page_id: Option<&str>,
    open_safari: bool,
    continue_on_error: bool,
    timeout: Duration,
    json_output: bool,
) -> Result<()> {
    let operation = run_js_shell_inner(
        udid,
        url,
        bundle_id,
        page_id,
        open_safari,
        continue_on_error,
        timeout,
        json_output,
    );
    tokio::pin!(operation);
    let mut ctrl_c = Box::pin(tokio::signal::ctrl_c());
    tokio::select! {
        result = &mut operation => result,
        signal = &mut ctrl_c => {
            signal.context("failed waiting for Ctrl+C")?;
            Err(anyhow!("webinspector js-shell cancelled"))
        }
    }
}

#[allow(clippy::too_many_arguments)]
async fn run_js_shell_inner(
    udid: &str,
    url: Option<&str>,
    bundle_id: Option<&str>,
    page_id: Option<&str>,
    open_safari: bool,
    continue_on_error: bool,
    timeout: Duration,
    json_output: bool,
) -> Result<()> {
    let deadline = webinspector_deadline(timeout)?;
    let (device, stream, uses_rsd) = connect_webinspector_with_timeout(udid, timeout).await?;
    let mut client = WebInspectorClient::new(stream);
    client
        .start(remaining_webinspector_time(deadline, timeout)?)
        .await?;

    // Match go-ios: --open-safari starts Safari, while an explicitly supplied
    // --bundle-id still controls which page is selected. Without a filter,
    // --open-safari naturally selects Safari as its default target.
    let bundle_filter = if open_safari {
        bundle_id.or(Some(SAFARI_BUNDLE_ID))
    } else {
        bundle_id
    };
    if open_safari {
        let remaining = remaining_webinspector_time(deadline, timeout)?;
        tokio::time::timeout(
            remaining,
            client.request_application_launch(SAFARI_BUNDLE_ID),
        )
        .await
        .map_err(|_| anyhow!("webinspector Safari launch timed out after {timeout:?}"))??;
    }
    let (application_id, page) =
        wait_for_inspectable_page(&mut client, page_id, bundle_filter, deadline, timeout).await?;
    let application = client
        .applications()
        .get(&application_id)
        .cloned()
        .ok_or_else(|| anyhow!("application '{application_id}' disappeared"))?;
    let mut session = InspectorSession::new(application_id.clone(), page.id);
    session
        .attach(
            &mut client,
            true,
            remaining_webinspector_time(deadline, timeout)?,
        )
        .await?;

    if let Some(url) = url.filter(|url| !url.is_empty()) {
        let expression = format!(
            "window.location = {}",
            serde_json::to_string(url).context("failed encoding shell URL")?
        );
        let response = session
            .send_command_and_wait(
                &mut client,
                "Runtime.evaluate",
                runtime_evaluate_params(&expression),
                remaining_webinspector_time(deadline, timeout)?,
            )
            .await?;
        if let Some(error) = runtime_evaluation_error(&response) {
            return Err(anyhow!("webinspector navigation error: {error}"));
        }
    }

    let metadata = json!({
        "kind": "session",
        "bundle_id": application.bundle_identifier,
        "application_id": application.id,
        "page_id": page.id,
        "page_key": page.listing_key,
        "page_type": page_type_name(&page.page_type),
        "url": page.url,
        "title": page.title,
        "session_id": session.session_id(),
        "inspector_connection_id": client.connection_id(),
        "automation_connection_id": page.automation_connection_id,
        "service_name": if uses_rsd { RSD_SERVICE_NAME } else { SERVICE_NAME },
    });
    if json_output {
        println!("{}", serde_json::to_string(&metadata)?);
    } else {
        println!(
            "Connected to {} page={} {}",
            application.bundle_identifier,
            page.id,
            page.url.as_deref().unwrap_or("<no url>")
        );
    }

    run_js_shell_loop(
        &mut client,
        &mut session,
        timeout,
        continue_on_error,
        json_output,
    )
    .await?;
    drop(device);
    Ok(())
}

async fn run_js_shell_loop<S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin>(
    client: &mut WebInspectorClient<S>,
    session: &mut InspectorSession,
    timeout: Duration,
    continue_on_error: bool,
    json_output: bool,
) -> Result<()> {
    let interactive = std::io::stdin().is_terminal();
    let stdin = tokio::io::stdin();
    let mut reader = BufReader::new(stdin);
    let mut ctrl_c = Box::pin(tokio::signal::ctrl_c());

    loop {
        if interactive {
            print!("> ");
            std::io::stdout().flush()?;
        }
        let mut line = String::new();
        let bytes = tokio::select! {
            signal = &mut ctrl_c => {
                signal.context("failed waiting for Ctrl+C")?;
                return Ok(());
            }
            result = read_bounded_shell_line(&mut reader, &mut line) => result?
        };
        if bytes == 0 {
            if interactive {
                println!();
            }
            return Ok(());
        }

        let expression = line.trim();
        if expression.is_empty() {
            continue;
        }
        if matches!(expression, ".exit" | "exit" | "quit") {
            return Ok(());
        }

        let evaluation = session.send_command_and_wait(
            client,
            "Runtime.evaluate",
            runtime_evaluate_params(expression),
            timeout,
        );
        tokio::pin!(evaluation);
        let result = tokio::select! {
            signal = &mut ctrl_c => {
                signal.context("failed waiting for Ctrl+C")?;
                return Ok(());
            }
            result = &mut evaluation => result
        };
        match result {
            Ok(value) => {
                if let Some(error) = runtime_evaluation_error(&value) {
                    if continue_on_error {
                        print_shell_error(expression, &error, json_output)?;
                    } else {
                        return Err(anyhow!("webinspector evaluate error: {error}"));
                    }
                } else {
                    print_shell_value(expression, value, json_output)?;
                }
            }
            Err(error) if continue_on_error => {
                print_shell_error(expression, &error.to_string(), json_output)?;
            }
            Err(error) => return Err(error.into()),
        }
    }
}

async fn read_bounded_shell_line<R: AsyncBufRead + Unpin>(
    reader: &mut R,
    line: &mut String,
) -> Result<usize> {
    line.clear();
    let mut bytes = Vec::new();
    loop {
        let buffer = reader.fill_buf().await?;
        if buffer.is_empty() {
            break;
        }
        let take = buffer
            .iter()
            .position(|byte| *byte == b'\n')
            .map_or(buffer.len(), |position| position + 1);
        let Some(new_len) = bytes.len().checked_add(take) else {
            return Err(anyhow!(
                "JavaScript input line exceeds {MAX_SHELL_INPUT_BYTES} bytes"
            ));
        };
        if new_len > MAX_SHELL_INPUT_BYTES {
            return Err(anyhow!(
                "JavaScript input line exceeds {MAX_SHELL_INPUT_BYTES} bytes"
            ));
        }
        let has_newline = buffer[take - 1] == b'\n';
        bytes.extend_from_slice(&buffer[..take]);
        reader.consume(take);
        if has_newline {
            break;
        }
    }
    if bytes.is_empty() {
        return Ok(0);
    }
    *line = String::from_utf8(bytes).context("JavaScript input is not valid UTF-8")?;
    Ok(line.len())
}

fn print_shell_value(expression: &str, value: JsonValue, json_output: bool) -> Result<()> {
    if json_output {
        println!(
            "{}",
            serde_json::to_string(&json!({
                "kind": "result",
                "expression": expression,
                "result": value,
            }))?
        );
    } else {
        print_runtime_evaluation_human(&value)?;
    }
    Ok(())
}

fn print_shell_error(expression: &str, error: &str, json_output: bool) -> Result<()> {
    if json_output {
        println!(
            "{}",
            serde_json::to_string(&json!({
                "kind": "error",
                "expression": expression,
                "error": error,
            }))?
        );
    } else {
        eprintln!("{error}");
    }
    Ok(())
}

fn print_runtime_evaluation_human(value: &JsonValue) -> Result<()> {
    if let Some(value) = runtime_remote_object(value).and_then(|result| result.get("value")) {
        println!("{}", value.as_str().unwrap_or(&value.to_string()));
    } else if let Some(description) =
        runtime_remote_object(value).and_then(|result| result.get("description"))
    {
        println!("{description}");
    } else {
        println!("{}", serde_json::to_string_pretty(value)?);
    }
    Ok(())
}

/// `InspectorSession::send_command_and_wait` returns the command's `result`
/// object, while a few older/fake transports hand the extra `result` wrapper
/// through. Accept both shapes at the CLI boundary.
fn runtime_remote_object(response: &JsonValue) -> Option<&JsonValue> {
    response
        .pointer("/result/result")
        .or_else(|| response.pointer("/result"))
}

fn runtime_evaluation_error(response: &JsonValue) -> Option<String> {
    if let Some(error) = response.get("error") {
        return Some(error.to_string());
    }
    let exception = response
        .get("exceptionDetails")
        .or_else(|| response.pointer("/result/exceptionDetails"));
    if let Some(exception) = exception {
        return Some(
            exception
                .get("text")
                .and_then(JsonValue::as_str)
                .or_else(|| {
                    exception
                        .pointer("/exception/description")
                        .and_then(JsonValue::as_str)
                })
                .map(ToOwned::to_owned)
                .unwrap_or_else(|| exception.to_string()),
        );
    }
    let remote = runtime_remote_object(response)?;
    if remote.get("subtype").and_then(JsonValue::as_str) == Some("error") {
        return Some(
            remote
                .get("description")
                .and_then(JsonValue::as_str)
                .map(ToOwned::to_owned)
                .unwrap_or_else(|| remote.to_string()),
        );
    }
    None
}

fn select_inspectable_page(
    client: &WebInspectorClient<ServiceStream>,
    page_id: Option<&str>,
    bundle_id: Option<&str>,
) -> Result<(String, Page)> {
    select_inspectable_page_from_pages(&client.open_pages_snapshot(), page_id, bundle_id)
}

fn remaining_webinspector_time(deadline: Instant, timeout: Duration) -> Result<Duration> {
    let now = Instant::now();
    if now >= deadline {
        return Err(anyhow!(
            "webinspector operation timed out after {timeout:?}"
        ));
    }
    Ok(deadline.duration_since(now))
}

async fn wait_for_application(
    client: &mut WebInspectorClient<ServiceStream>,
    bundle_id: &str,
    deadline: Instant,
    timeout: Duration,
) -> Result<ios_core::webinspector::Application> {
    loop {
        if let Some(application) = client.application_by_bundle(bundle_id).cloned() {
            return Ok(application);
        }
        request_connected_applications_with_deadline(client, deadline, timeout).await?;
        loop {
            if let Some(application) = client.application_by_bundle(bundle_id).cloned() {
                return Ok(application);
            }
            let wait = remaining_webinspector_time(deadline, timeout)?.min(PAGE_DISCOVERY_IDLE);
            match client.next_event_with_timeout(wait).await {
                Ok(_) => continue,
                Err(WebInspectorError::Timeout(_)) => break,
                Err(error) => return Err(error.into()),
            }
        }
    }
}

async fn wait_for_inspectable_page(
    client: &mut WebInspectorClient<ServiceStream>,
    page_id: Option<&str>,
    bundle_id: Option<&str>,
    deadline: Instant,
    timeout: Duration,
) -> Result<(String, Page)> {
    loop {
        if let Ok(page) = select_inspectable_page(client, page_id, bundle_id) {
            return Ok(page);
        }
        request_connected_applications_with_deadline(client, deadline, timeout).await?;
        loop {
            if let Ok(page) = select_inspectable_page(client, page_id, bundle_id) {
                return Ok(page);
            }
            let wait = remaining_webinspector_time(deadline, timeout)?.min(PAGE_DISCOVERY_IDLE);
            match client.next_event_with_timeout(wait).await {
                Ok(_) => continue,
                Err(WebInspectorError::Timeout(_)) => break,
                Err(error) => return Err(error.into()),
            }
        }
    }
}

fn select_inspectable_page_from_pages(
    pages: &[ApplicationPage],
    page_id: Option<&str>,
    bundle_id: Option<&str>,
) -> Result<(String, Page)> {
    // Match go-ios's selector precedence: an explicit page key is a stable
    // identity and wins before the optional bundle filter is considered.
    if let Some(page_id) = page_id {
        if let Some(candidate) = pages.iter().find(|candidate| {
            page_matches_selector(&candidate.page, page_id)
                && is_inspectable_page_type(&candidate.page.page_type)
        }) {
            return Ok((candidate.application.id.clone(), candidate.page.clone()));
        }
    }
    let mut candidates = pages
        .iter()
        .filter(|candidate| is_inspectable_page_type(&candidate.page.page_type))
        .filter(|_| page_id.is_none())
        .filter(|candidate| {
            bundle_id.map_or(true, |bundle| {
                candidate.application.bundle_identifier == bundle
            })
        })
        .collect::<Vec<_>>();
    if candidates.is_empty() {
        let page = page_id.map(|id| format!(" page {id}")).unwrap_or_default();
        let bundle = bundle_id
            .map(|id| format!(" for bundle '{id}'"))
            .unwrap_or_default();
        return Err(anyhow!(
            "no inspectable WebInspector page found{page}{bundle}"
        ));
    }
    let candidate = candidates.remove(0);
    Ok((candidate.application.id.clone(), candidate.page.clone()))
}

async fn request_connected_applications_with_deadline(
    client: &mut WebInspectorClient<ServiceStream>,
    deadline: Instant,
    timeout: Duration,
) -> Result<()> {
    let remaining = remaining_webinspector_time(deadline, timeout)?;
    tokio::time::timeout(remaining, client.request_connected_applications())
        .await
        .map_err(|_| {
            anyhow!("webinspector application-list request timed out after {timeout:?}")
        })??;
    Ok(())
}

fn page_type_name(page_type: &WirType) -> String {
    serde_json::to_string(page_type)
        .unwrap_or_else(|_| "\"unknown\"".to_string())
        .trim_matches('"')
        .to_string()
}

fn is_inspectable_page_type(page_type: &WirType) -> bool {
    matches!(
        page_type,
        WirType::Web | WirType::WebPage | WirType::JavaScript
    )
}

fn page_matches_selector(page: &Page, selector: &str) -> bool {
    page.listing_key == selector || page.id.to_string() == selector
}

/// A page number is only unique within its owning WebInspector application.
/// Qualify the CDP target ID so JS contexts/pages from different apps cannot
/// collide; bare selectors remain accepted by the CLI for compatibility.
fn page_target_id(application_id: &str, page: &Page) -> String {
    format!("{application_id}:{}", page.listing_key)
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
    let (_device, stream, _use_rsd) = connect_webinspector_with_timeout(udid, timeout).await?;
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
    let (device, stream, _use_rsd) = connect_webinspector_with_timeout(udid, timeout).await?;
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
    let (_device, stream, _use_rsd) = connect_webinspector_with_timeout(udid, timeout).await?;
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

fn find_page_by_id(
    client: &WebInspectorClient<ServiceStream>,
    page_id: &str,
) -> Result<(String, Page)> {
    find_page_in_snapshot(&client.open_pages_snapshot(), page_id)
}

fn find_page_in_snapshot(pages: &[ApplicationPage], page_id: &str) -> Result<(String, Page)> {
    if let Some((application_id, page_key)) = page_id.rsplit_once(':') {
        if let Some(page) = pages.iter().find(|candidate| {
            candidate.application.id == application_id
                && is_inspectable_page_type(&candidate.page.page_type)
                && candidate.page.listing_key == page_key
        }) {
            return Ok((page.application.id.clone(), page.page.clone()));
        }
    }
    for page in pages {
        if is_inspectable_page_type(&page.page.page_type)
            && page_matches_selector(&page.page, page_id)
        {
            return Ok((page.application.id.clone(), page.page.clone()));
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

async fn connect_webinspector_with_timeout(
    udid: &str,
    timeout: Duration,
) -> Result<(ConnectedDevice, ServiceStream, bool)> {
    tokio::time::timeout(timeout, connect_webinspector(udid))
        .await
        .map_err(|_| anyhow!("webinspector connection timed out after {timeout:?}"))?
}

fn opened_tab_row(page: ApplicationPage) -> OpenedTabRow {
    let ApplicationPage { application, page } = page;
    OpenedTabRow {
        application_id: application.id,
        bundle_identifier: application.bundle_identifier,
        application_name: application.name,
        pid: application.pid,
        page_id: page.id,
        page_key: page.listing_key,
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

fn duration_from_secs(seconds: f64) -> Result<Duration> {
    if !seconds.is_finite() || seconds < 0.0 {
        return Err(anyhow!("--timeout must be a finite non-negative number"));
    }
    Duration::try_from_secs_f64(seconds.max(0.1)).map_err(|_| anyhow!("--timeout is too large"))
}

fn webinspector_deadline(timeout: Duration) -> Result<Instant> {
    Instant::now()
        .checked_add(timeout)
        .ok_or_else(|| anyhow!("--timeout is too large for the system clock"))
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
    fn parses_launch_and_js_shell_subcommands() {
        let launch = TestCli::try_parse_from([
            "webinspector",
            "launch",
            "https://example.com/路径",
            "--bundle-id",
            "com.example.app",
            "--timeout",
            "15",
        ])
        .expect("launch command should parse");
        assert!(matches!(launch.command, WebInspectorSub::Launch { .. }));

        let shell = TestCli::try_parse_from([
            "webinspector",
            "js-shell",
            "https://example.com",
            "--bundle-id",
            "com.example.app",
            "--page-id",
            "7",
            "--continue-on-error",
            "false",
        ])
        .expect("js-shell command should parse");
        let WebInspectorSub::JsShell {
            page_id,
            continue_on_error,
            ..
        } = shell.command
        else {
            panic!("expected js-shell command");
        };
        assert_eq!(page_id.as_deref(), Some("7"));
        assert!(!continue_on_error);
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
    fn duration_rejects_non_finite_and_overflowing_timeouts() {
        assert_eq!(duration_from_secs(0.0).unwrap(), Duration::from_millis(100));
        assert!(duration_from_secs(-1.0).is_err());
        assert!(duration_from_secs(f64::NAN).is_err());
        assert!(duration_from_secs(f64::INFINITY).is_err());
        assert!(duration_from_secs(f64::MAX).is_err());
    }

    #[test]
    fn deadline_rejects_system_clock_overflow() {
        assert!(webinspector_deadline(Duration::from_secs(u64::MAX)).is_err());
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
    fn page_selection_honors_page_and_bundle_filters() {
        let application = |id: &str, bundle: &str| Application {
            id: id.into(),
            bundle_identifier: bundle.into(),
            pid: 42,
            name: bundle.into(),
            availability: AutomationAvailability::Available,
            is_active: true,
            is_proxy: false,
            is_ready: true,
            host_application_identifier: None,
        };
        let pages = vec![
            ApplicationPage {
                application: application("PID:42", "com.example.one"),
                page: Page {
                    id: 1,
                    listing_key: "1".into(),
                    page_type: WirType::WebPage,
                    title: Some("first".into()),
                    url: Some("https://one.example".into()),
                    automation_is_paired: None,
                    automation_name: None,
                    automation_version: None,
                    automation_session_id: None,
                    automation_connection_id: None,
                },
            },
            ApplicationPage {
                application: application("PID:43", "com.example.two"),
                page: Page {
                    id: 2,
                    listing_key: "tab-two".into(),
                    page_type: WirType::JavaScript,
                    title: Some("second".into()),
                    url: Some("https://two.example".into()),
                    automation_is_paired: None,
                    automation_name: None,
                    automation_version: None,
                    automation_session_id: None,
                    automation_connection_id: None,
                },
            },
        ];
        let (application_id, page) =
            select_inspectable_page_from_pages(&pages, Some("tab-two"), Some("com.example.two"))
                .expect("matching page should be selected");
        assert_eq!(application_id, "PID:43");
        assert_eq!(page.id, 2);
        let (application_id, page) =
            select_inspectable_page_from_pages(&pages, Some("tab-two"), Some("com.example.other"))
                .expect("an explicit page id takes precedence over bundle filtering");
        assert_eq!(application_id, "PID:43");
        assert_eq!(page.id, 2);
        assert!(select_inspectable_page_from_pages(&pages, Some("99"), None).is_err());
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

    #[test]
    fn shell_result_and_page_type_names_are_stable() {
        assert_eq!(page_type_name(&WirType::WebPage), "web_page");
        let response = json!({"result": {"type": "string", "value": "你好"}});
        assert_eq!(
            runtime_remote_object(&response)
                .and_then(|value| value.get("value"))
                .and_then(JsonValue::as_str),
            Some("你好")
        );
    }

    #[test]
    fn runtime_evaluation_errors_cover_subtype_and_exception_details() {
        let subtype = json!({
            "result": {"type": "object", "subtype": "error", "description": "boom"}
        });
        assert_eq!(runtime_evaluation_error(&subtype).as_deref(), Some("boom"));

        let exception = json!({
            "result": {"type": "undefined"},
            "exceptionDetails": {"text": "syntax error"}
        });
        assert_eq!(
            runtime_evaluation_error(&exception).as_deref(),
            Some("syntax error")
        );

        let nested = json!({"result": {"result": {"value": "legacy"}}});
        assert_eq!(
            runtime_remote_object(&nested)
                .and_then(|value| value.get("value"))
                .and_then(JsonValue::as_str),
            Some("legacy")
        );
    }

    #[tokio::test]
    async fn shell_input_preserves_newline_and_eof_semantics() {
        let mut reader = BufReader::new(&b"1 + 1\n2 + 2"[..]);
        let mut line = String::new();
        assert_eq!(
            read_bounded_shell_line(&mut reader, &mut line)
                .await
                .unwrap(),
            6
        );
        assert_eq!(line, "1 + 1\n");
        assert_eq!(
            read_bounded_shell_line(&mut reader, &mut line)
                .await
                .unwrap(),
            5
        );
        assert_eq!(line, "2 + 2");
        assert_eq!(
            read_bounded_shell_line(&mut reader, &mut line)
                .await
                .unwrap(),
            0
        );
    }

    #[tokio::test]
    async fn shell_input_rejects_overlong_unterminated_line() {
        let input = vec![b'x'; MAX_SHELL_INPUT_BYTES + 1];
        let mut reader = BufReader::new(input.as_slice());
        let mut line = String::new();
        let error = read_bounded_shell_line(&mut reader, &mut line)
            .await
            .expect_err("an overlong line must be rejected before evaluation");
        assert!(error.to_string().contains("input line exceeds"));
    }
}
