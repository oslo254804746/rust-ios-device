//! HTTP startup, shared server resources and safe error/reload responses.
use super::connection::{DeviceInspectorSource, InspectorSource};
use super::sessions::Sessions;
use super::webdriver::SeleniumRuntime;
use crate::access::{
    AccessControl, HttpBoundary, HttpBoundaryArgs, Permission, Principal, ServerAccessArgs,
};
use anyhow::Result;
use axum::extract::State;
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::{Extension, Json, Router};
use serde_json::json;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;
use tokio::net::TcpListener;

#[derive(Clone)]
pub(super) struct ServerState {
    pub(super) udid: String,
    pub(super) timeout: Duration,
    pub(super) cdp_host: String,
    pub(super) cdp_port: u16,
    pub(super) access: AccessControl,
    pub(super) selenium_sessions: Sessions<SeleniumRuntime>,
    pub(super) source: Arc<dyn InspectorSource>,
}

#[derive(Debug)]
pub(super) struct AppError {
    status: StatusCode,
    message: String,
}

impl AppError {
    pub(super) fn bad_request(message: &'static str) -> Self {
        Self {
            status: StatusCode::BAD_REQUEST,
            message: message.into(),
        }
    }
}

impl IntoResponse for AppError {
    fn into_response(self) -> axum::response::Response {
        crate::http_diagnostics::reviewed((
            self.status,
            Json(json!({ "value": { "error": "unknown error", "message": self.message } })),
        ))
    }
}

impl From<anyhow::Error> for AppError {
    fn from(value: anyhow::Error) -> Self {
        Self {
            status: StatusCode::INTERNAL_SERVER_ERROR,
            message: format!(
                "WebInspector operation failed: {}",
                crate::output::error_message(&value)
            ),
        }
    }
}

impl From<axum::Error> for AppError {
    fn from(value: axum::Error) -> Self {
        Self {
            status: StatusCode::INTERNAL_SERVER_ERROR,
            message: format!(
                "WebInspector HTTP failed: {}",
                ios_core::redaction::diagnostic_error(&value)
            ),
        }
    }
}

impl From<serde_json::Error> for AppError {
    fn from(value: serde_json::Error) -> Self {
        Self {
            status: StatusCode::BAD_REQUEST,
            message: format!(
                "WebInspector JSON decode failed: {}",
                ios_core::redaction::diagnostic_error(&value)
            ),
        }
    }
}

pub(super) async fn server_state(
    udid: &str,
    host: &str,
    port: u16,
    timeout: Duration,
    access: ServerAccessArgs,
    boundary: HttpBoundaryArgs,
) -> Result<(TcpListener, ServerState, HttpBoundary)> {
    let boundary = boundary.validate()?;
    let (address, access) = access.load(host).await?;
    let listener = TcpListener::bind((address, port)).await?;
    let bound = listener.local_addr()?;
    let state = ServerState {
        udid: udid.to_string(),
        timeout,
        cdp_host: match bound {
            SocketAddr::V4(_) => bound.ip().to_string(),
            SocketAddr::V6(_) => format!("[{}]", bound.ip()),
        },
        cdp_port: bound.port(),
        access,
        selenium_sessions: Sessions::default(),
        source: Arc::new(DeviceInspectorSource),
    };
    Ok((listener, state, boundary.for_listener(bound)))
}

pub(super) async fn serve(listener: TcpListener, app: Router) -> Result<()> {
    let addr = listener.local_addr()?;
    println!("Listening on http://{addr}");
    axum::serve(listener, app).await?;
    Ok(())
}

pub(super) async fn cdp_reload_policy(
    State(state): State<ServerState>,
    Extension(principal): Extension<Principal>,
) -> axum::response::Response {
    debug_reload_policy(state, principal, crate::http_diagnostics::ErrorShape::Cdp).await
}

pub(super) async fn webdriver_reload_policy(
    State(state): State<ServerState>,
    Extension(principal): Extension<Principal>,
) -> axum::response::Response {
    debug_reload_policy(
        state,
        principal,
        crate::http_diagnostics::ErrorShape::WebDriver,
    )
    .await
}

async fn debug_reload_policy(
    state: ServerState,
    principal: Principal,
    shape: crate::http_diagnostics::ErrorShape,
) -> axum::response::Response {
    if !principal.allows(Permission::ManageServer, None, None) {
        return crate::access::denied_for(shape, StatusCode::FORBIDDEN, false);
    }
    if let Err(error) = state.access.reload(&principal).await {
        let message = format!(
            "debug policy reload failed: {}",
            crate::output::error_message(&error)
        );
        let body = match shape {
            crate::http_diagnostics::ErrorShape::Cdp => {
                json!({"error": {"code": -32600, "message": message}})
            }
            _ => json!({"value": {"error": "invalid request", "message": message}}),
        };
        return crate::http_diagnostics::reviewed((StatusCode::BAD_REQUEST, Json(body)));
    }
    Json(json!({"operation": "reload_policy", "data": true})).into_response()
}
