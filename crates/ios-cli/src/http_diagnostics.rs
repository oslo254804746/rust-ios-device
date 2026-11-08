//! Safe fallback replies for framework rejections before a handler runs.

use axum::extract::{Request, State};
use axum::http::{header, HeaderValue, Method};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use axum::Json;
use ios_core::ErrorClass;
use serde_json::json;

pub const ERROR_HEADER_EXPOSURE: &str =
    "X-Ios-Error-Schema-Version, X-Ios-Error-Class, X-Ios-Error-Phase, X-Ios-Error-Effects";

#[derive(Clone, Copy, Debug)]
pub enum ErrorPhase {
    Request,
    Access,
    Ticket,
    WebInspector,
    Manager,
    PolicyReload,
}

impl ErrorPhase {
    fn as_str(self) -> &'static str {
        match self {
            Self::Request => "http_request",
            Self::Access => "http_access",
            Self::Ticket => "browser_ticket",
            Self::WebInspector => "webinspector",
            Self::Manager => "tunnel_manager",
            Self::PolicyReload => "policy_reload",
        }
    }
}

fn attach_category(response: &mut Response, class: ErrorClass, phase: ErrorPhase) {
    if response.status().is_client_error() || response.status().is_server_error() {
        let headers = response.headers_mut();
        headers.insert("x-ios-error-schema-version", HeaderValue::from_static("1"));
        headers.insert(
            "x-ios-error-class",
            HeaderValue::from_static(class.as_str()),
        );
        headers.insert(
            "x-ios-error-phase",
            HeaderValue::from_static(phase.as_str()),
        );
        // HTTP failure may follow device/host effects or incomplete output.
        headers.insert("x-ios-error-effects", HeaderValue::from_static("unknown"));
    }
}

#[derive(Clone, Copy)]
pub enum ErrorShape {
    Manager,
    WebDriver,
    Cdp,
}

#[derive(Clone)]
struct ReviewedDiagnostic;

/// Mark only replies built from the reviewed, fixed-phase diagnostic helpers.
/// This stays in response extensions and is never a client-controlled header.
pub fn reviewed(response: impl IntoResponse) -> Response {
    let mut response = response.into_response();
    response.extensions_mut().insert(ReviewedDiagnostic);
    attach_category(&mut response, ErrorClass::Unknown, ErrorPhase::Request);
    response
}

/// A server-selected class/phase, without changing the existing body or status.
pub fn classified(response: impl IntoResponse, class: ErrorClass, phase: ErrorPhase) -> Response {
    let mut response = reviewed(response);
    attach_category(&mut response, class, phase);
    response
}

pub async fn rejections(State(shape): State<ErrorShape>, request: Request, next: Next) -> Response {
    let head = request.method() == Method::HEAD;
    let response = next.run(request).await;
    if !(response.status().is_client_error() || response.status().is_server_error())
        || response.extensions().get::<ReviewedDiagnostic>().is_some()
    {
        return response;
    }
    let status = response.status();
    // Drop the unreviewed body without reading or formatting any of its bytes.
    // JSON/query/path/handshake errors can otherwise echo submitted secrets.
    let correlation = uuid::Uuid::new_v4().to_string();
    let message = format!(
        "HTTP request rejected: http/{} request:{correlation}",
        status.as_u16()
    );
    let body = match shape {
        ErrorShape::Manager => json!({"message": "HTTP request rejected", "error": message}),
        ErrorShape::WebDriver => json!({"value": {"error": "invalid request", "message": message}}),
        ErrorShape::Cdp => json!({"error": {"code": -32600, "message": message}}),
    };
    let mut safe = (status, Json(body)).into_response();
    attach_category(&mut safe, ErrorClass::Unknown, ErrorPhase::Request);
    // Preserve the framework's method contract. Never copy arbitrary headers.
    if let Some(allow) = response.headers().get(header::ALLOW) {
        safe.headers_mut().insert(header::ALLOW, allow.clone());
    }
    safe.headers_mut()
        .insert(header::CACHE_CONTROL, "no-store".parse().unwrap());
    if head {
        *safe.body_mut() = axum::body::Body::empty();
    }
    safe
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::extract::{Path, Query};
    use axum::http::StatusCode;
    use axum::middleware;
    use axum::routing::{get, post};
    use axum::Router;
    use serde::Deserialize;
    use std::time::Duration;

    #[derive(Deserialize)]
    struct NumericInput {
        port: u16,
    }

    #[tokio::test]
    async fn classified_error_headers_keep_existing_status_and_body() {
        let body = json!({"message": "original fixed phase"});
        let response = classified(
            (StatusCode::GATEWAY_TIMEOUT, Json(body.clone())),
            ErrorClass::Timeout,
            ErrorPhase::Manager,
        );
        assert_eq!(response.status(), StatusCode::GATEWAY_TIMEOUT);
        assert_eq!(response.headers()["x-ios-error-schema-version"], "1");
        assert_eq!(response.headers()["x-ios-error-class"], "timeout");
        assert_eq!(response.headers()["x-ios-error-phase"], "tunnel_manager");
        assert_eq!(response.headers()["x-ios-error-effects"], "unknown");
        assert_eq!(
            serde_json::from_slice::<serde_json::Value>(
                &axum::body::to_bytes(response.into_body(), 8192)
                    .await
                    .unwrap()
            )
            .unwrap(),
            body
        );
        for status in [
            StatusCode::OK,
            StatusCode::NO_CONTENT,
            StatusCode::SWITCHING_PROTOCOLS,
        ] {
            let response = classified(status, ErrorClass::Timeout, ErrorPhase::Manager);
            assert!(!response
                .headers()
                .contains_key("x-ios-error-schema-version"));
            assert!(!response.headers().contains_key("x-ios-error-class"));
        }
    }

    #[tokio::test]
    async fn actual_framework_rejections_do_not_echo_body_query_or_path_values() {
        const SECRET: &str = "synthetic-private-credential-device-data";
        for shape in [ErrorShape::Manager, ErrorShape::WebDriver, ErrorShape::Cdp] {
            let router = Router::new()
                .route(
                    "/json",
                    post(|Json(input): Json<NumericInput>| async move { input.port.to_string() }),
                )
                .route(
                    "/query",
                    get(|Query(input): Query<NumericInput>| async move { input.port.to_string() }),
                )
                .route(
                    "/path/:port",
                    get(|Path(port): Path<u16>| async move { port.to_string() }),
                )
                .route(
                    "/reviewed",
                    get(|| async {
                        reviewed((
                            StatusCode::BAD_REQUEST,
                            Json(json!({"message": "fixed reviewed phase"})),
                        ))
                    }),
                )
                .route(
                    "/forged",
                    get(|| async {
                        (
                            StatusCode::BAD_REQUEST,
                            [
                                ("x-ios-error-class", "timeout"),
                                ("x-ios-error-phase", SECRET),
                            ],
                            SECRET,
                        )
                    }),
                )
                .layer(middleware::from_fn_with_state(shape, rejections));
            let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
                .await
                .unwrap();
            let port = listener.local_addr().unwrap().port();
            let server = tokio::spawn(async move {
                axum::serve(listener, router).await.unwrap();
            });
            let client = reqwest::Client::builder()
                .no_proxy()
                .timeout(Duration::from_secs(2))
                .build()
                .unwrap();
            let base = format!("http://127.0.0.1:{port}");
            let requests = [
                client
                    .post(format!("{base}/json"))
                    .header("content-type", "application/json")
                    .body(format!("{{\"port\":\"{SECRET}\"}}")),
                client.get(format!("{base}/query?port={SECRET}")),
                client.get(format!("{base}/path/{SECRET}")),
                client.post(format!("{base}/query")),
                client
                    .get(format!("{base}/forged"))
                    .header("x-ios-error-class", "timeout")
                    .header("x-ios-error-phase", SECRET),
            ];
            for request in requests {
                let response = request.send().await.unwrap();
                assert!(response.status().is_client_error());
                assert_eq!(response.headers()["x-ios-error-schema-version"], "1");
                assert_eq!(response.headers()["x-ios-error-class"], "unknown");
                assert_eq!(response.headers()["x-ios-error-phase"], "http_request");
                assert_eq!(response.headers()["x-ios-error-effects"], "unknown");
                assert!(!format!("{:?}", response.headers()).contains(SECRET));
                let raw = response.text().await.unwrap();
                assert!(!raw.contains(SECRET));
                assert!(raw.contains("HTTP request rejected: http/"));
                let reply: serde_json::Value = serde_json::from_str(&raw).unwrap();
                match shape {
                    ErrorShape::Manager => assert!(reply["message"].is_string()),
                    ErrorShape::WebDriver => assert_eq!(reply["value"]["error"], "invalid request"),
                    ErrorShape::Cdp => assert_eq!(reply["error"]["code"], -32600),
                }
            }
            let response = client
                .head(format!("{base}/path/{SECRET}"))
                .send()
                .await
                .unwrap();
            assert_eq!(response.status(), StatusCode::BAD_REQUEST);
            assert_eq!(response.headers()["x-ios-error-class"], "unknown");
            assert!(response.bytes().await.unwrap().is_empty());
            let response = client.post(format!("{base}/query")).send().await.unwrap();
            assert_eq!(response.status(), StatusCode::METHOD_NOT_ALLOWED);
            assert!(response.headers().contains_key(header::ALLOW));
            let raw = client
                .get(format!("{base}/reviewed"))
                .send()
                .await
                .unwrap()
                .text()
                .await
                .unwrap();
            assert!(raw.contains("fixed reviewed phase"));
            assert!(!raw.contains("HTTP request rejected"));
            server.abort();
            let _ = server.await;
        }
    }
}
