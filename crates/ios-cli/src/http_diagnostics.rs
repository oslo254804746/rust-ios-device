//! Safe fallback replies for framework rejections before a handler runs.

use axum::extract::{Request, State};
use axum::http::{header, Method};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde_json::json;

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
            ];
            for request in requests {
                let response = request.send().await.unwrap();
                assert!(response.status().is_client_error());
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
