//! WebDriver routes, automation lifetime and element representation.
//! Protocol execution remains in the core AutomationSession.
use super::connection::InspectorConnection;
use super::http::{serve, server_state, webdriver_reload_policy, AppError, ServerState};
use super::tasks::control::Budget;
use super::SESSION_CLEANUP_MAX;
use crate::access::{Admission, HttpBoundaryArgs, Principal, ServerAccessArgs};
use anyhow::{anyhow, Context, Result};
use axum::extract::{Path, State};
use axum::routing::{get, post};
use axum::{Extension, Json, Router};
use ios_core::device::{ConnectedDevice, ServiceStream};
use ios_core::webinspector::{AutomationSession, By, WebInspectorClient, SAFARI_BUNDLE_ID};
use serde_json::{json, Value as JsonValue};
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::Mutex;
use uuid::Uuid;

const WD_ELEMENT_KEY: &str = "element-6066-11e4-a52e-4f735466cecf";

#[cfg(test)]
mod lifecycle_tests;

#[cfg(test)]
mod cleanup_tests;

#[cfg(test)]
mod delete_tests;

#[cfg(test)]
mod revocation_tests;

pub(super) struct SeleniumRuntime {
    _device: Option<ConnectedDevice>,
    client: WebInspectorClient<ServiceStream>,
    automation: AutomationSession,
    application_id: String,
    cleanup_armed: bool,
    elements: HashMap<String, JsonValue>,
}

impl SeleniumRuntime {
    #[cfg(test)]
    async fn stop(&mut self) -> Result<()> {
        if !self.cleanup_armed {
            return Ok(());
        }
        // Once cleanup starts, neither a failure nor caller cancellation may
        // authorize a second close against an uncertain session.
        self.cleanup_armed = false;
        super::tasks::session_owner::cleanup_automation(
            &mut self.client,
            &mut self.automation,
            &self.application_id,
            SESSION_CLEANUP_MAX,
        )
        .await
    }

    async fn stop_until(&mut self, budget: Budget) -> Result<()> {
        if !self.cleanup_armed {
            return Ok(());
        }
        self.cleanup_armed = false;
        if !self.client.is_usable() {
            return Ok(());
        }
        super::tasks::session_owner::cleanup_automation_with_budget(
            &mut self.client,
            &mut self.automation,
            &self.application_id,
            budget,
        )
        .await
        .context("webinspector session cleanup failed")
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

pub(super) async fn run_selenium_server(
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
        boundary.protect(
            selenium_router(state),
            crate::http_diagnostics::ErrorShape::WebDriver,
        ),
    )
    .await
}

pub(super) fn selenium_router(state: ServerState) -> Router {
    let admission = Admission::debug(
        state.access.clone(),
        crate::http_diagnostics::ErrorShape::WebDriver,
        state.udid.clone(),
    );
    let sessions = state.selenium_sessions.clone();
    Router::new()
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
        .route("/auth/reload", post(webdriver_reload_policy))
        .with_state(state)
        .layer(axum::middleware::from_fn_with_state(
            crate::http_diagnostics::ErrorShape::WebDriver,
            crate::http_diagnostics::rejections,
        ))
        .layer(axum::middleware::from_fn_with_state(
            sessions,
            super::sessions::admission::<SeleniumRuntime>,
        ))
        .layer(axum::middleware::from_fn_with_state(
            admission,
            crate::access::admission,
        ))
}

async fn webdriver_status() -> Json<JsonValue> {
    Json(
        json!({ "value": { "ready": true, "message": "ios-cli Safari automation bridge is ready" } }),
    )
}

async fn webdriver_new_session(
    State(state): State<ServerState>,
    Extension(principal): Extension<Principal>,
) -> Result<Json<JsonValue>, AppError> {
    let runtime = build_selenium_runtime(&state).await?;
    let session_id = Uuid::new_v4().to_string();
    state
        .selenium_sessions
        .insert_with_budget(
            session_id.clone(),
            runtime,
            principal,
            stop_selenium_runtime_until,
        )
        .await;
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
    let budget = Budget::new(SESSION_CLEANUP_MAX, "WebDriver deletion")?;
    let runtime = budget
        .drive(
            async { Ok(state.selenium_sessions.remove(&session_id).await) },
            std::future::pending::<std::io::Result<()>>(),
        )
        .await?;
    if let Some(runtime) = runtime {
        stop_selenium_runtime_until(runtime, budget).await?;
    }
    Ok(Json(json!({ "value": null })))
}

async fn stop_selenium_runtime_until(
    runtime: Arc<Mutex<SeleniumRuntime>>,
    budget: Budget,
) -> Result<()> {
    let mut runtime = budget
        .drive(
            async { Ok(runtime.lock().await) },
            std::future::pending::<std::io::Result<()>>(),
        )
        .await?;
    // The cleanup stages keep native primary and secondary causes. An outer
    // timeout around their combined result could discard an already known
    // business failure while the forwarding close is pending.
    runtime.stop_until(budget).await
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
        .get(session_id)
        .await
        .ok_or_else(|| AppError::bad_request("unknown webdriver session"))
}

async fn build_selenium_runtime(server: &ServerState) -> Result<SeleniumRuntime> {
    let budget = Budget::new(server.timeout, "WebDriver setup")?;
    budget
        .drive(
            Box::pin(async {
                let InspectorConnection { device, stream } = server
                    .source
                    .connect(&server.udid, budget.remaining()?)
                    .await?;
                let mut client = WebInspectorClient::new(stream);
                let remaining = budget.cap(&mut client)?;
                client.start_preserving_errors(remaining).await?;
                budget.cap(&mut client)?;
                client
                    .open_application_pages(super::PAGE_DISCOVERY_IDLE)
                    .await?;

                let application = match client.application_by_bundle(SAFARI_BUNDLE_ID) {
                    Some(application) => application.clone(),
                    None => {
                        budget.cap(&mut client)?;
                        client.request_application_launch(SAFARI_BUNDLE_ID).await?;
                        budget.cap(&mut client)?;
                        client
                            .open_application_pages(super::PAGE_DISCOVERY_IDLE)
                            .await?;
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
                let remaining = budget.cap(&mut client)?;
                automation.attach(&mut client, remaining).await?;
                budget.cap(&mut client)?;
                automation.start_session(&mut client).await?;
                client.set_operation_timeout(None);

                Ok(SeleniumRuntime {
                    _device: device,
                    client,
                    automation,
                    application_id: application.id,
                    cleanup_armed: true,
                    elements: HashMap::new(),
                })
            }),
            std::future::pending::<std::io::Result<()>>(),
        )
        .await
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
        _ => return Err(AppError::bad_request("unsupported locator strategy")),
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
    use super::*;
    use axum::http::StatusCode;
    use axum::response::IntoResponse;

    #[tokio::test]
    async fn webdriver_http_error_hides_device_data_and_error_sources() {
        const SECRET: &str = "synthetic-private-key-UDID-token-device-data";
        let router = Router::new().route(
            "/synthetic-failure",
            get(|| async {
                let error = anyhow::Error::new(std::io::Error::new(
                    std::io::ErrorKind::PermissionDenied,
                    SECRET,
                ))
                .context(format!("https://user:{SECRET}@example.invalid/{SECRET}"));
                assert!(format!("{error:#}").contains(SECRET));
                AppError::from(error).into_response()
            }),
        );
        let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
            .await
            .unwrap();
        let port = listener.local_addr().unwrap().port();
        let server = tokio::spawn(async move {
            axum::serve(listener, router).await.unwrap();
        });
        let response = reqwest::Client::builder()
            .timeout(Duration::from_secs(2))
            .build()
            .unwrap()
            .get(format!("http://127.0.0.1:{port}/synthetic-failure"))
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
        let raw = response.text().await.unwrap();
        assert!(!raw.contains(SECRET));
        assert!(!raw.contains("https://"));
        let json: JsonValue = serde_json::from_str(&raw).unwrap();
        assert_eq!(json["value"]["error"], "unknown error");
        assert!(json["value"]["message"]
            .as_str()
            .unwrap()
            .contains("WebInspector operation failed: io/PermissionDenied error:"));
        server.abort();
        let _ = server.await;
        let error = parse_by(SECRET).unwrap_err().into_response();
        let raw = axum::body::to_bytes(error.into_body(), 1024).await.unwrap();
        assert!(!std::str::from_utf8(&raw).unwrap().contains(SECRET));
    }
}
