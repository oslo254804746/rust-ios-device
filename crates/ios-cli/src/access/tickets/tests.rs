use super::*;

#[test]
fn ticket_error_metadata_matches_explicit_issuer_outcomes() {
    for shape in [ErrorShape::Manager, ErrorShape::Cdp, ErrorShape::WebDriver] {
        for (status, class, phase) in [
            (
                StatusCode::TOO_MANY_REQUESTS,
                "resource_limit",
                "browser_ticket",
            ),
            (StatusCode::SERVICE_UNAVAILABLE, "unknown", "browser_ticket"),
            (StatusCode::BAD_REQUEST, "invalid_input", "browser_ticket"),
            (StatusCode::REQUEST_TIMEOUT, "unknown", "browser_ticket"),
            (StatusCode::UNAUTHORIZED, "permission_denied", "http_access"),
            (StatusCode::FORBIDDEN, "permission_denied", "http_access"),
        ] {
            let response = error(shape, status);
            assert_eq!(response.status(), status);
            assert_eq!(response.headers()["x-ios-error-schema-version"], "1");
            assert_eq!(response.headers()["x-ios-error-class"], class);
            assert_eq!(response.headers()["x-ios-error-phase"], phase);
            assert_eq!(response.headers()["x-ios-error-effects"], "unknown");
        }
    }
}
use crate::access::test_support::{grant, JsonRequest, PolicyFixture, DEVICE, TOKEN};
use crate::access::{AccessControl, HttpBoundaryArgs};
use axum::extract::ws::WebSocketUpgrade;
use axum::routing::{get, post};
use futures_util::{SinkExt, StreamExt};
use tokio_tungstenite::tungstenite::{self, client::IntoClientRequest};

struct Server {
    base: String,
    alternate: String,
    admission: Admission,
    task: tokio::task::JoinHandle<()>,
}

impl Drop for Server {
    fn drop(&mut self) {
        self.task.abort();
    }
}

async fn spawn(fixture: &PolicyFixture) -> Server {
    let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
        .await
        .unwrap();
    let address = listener.local_addr().unwrap();
    let admission = Admission::manager(
        AccessControl::from_file(fixture.path.clone())
            .await
            .unwrap(),
    );
    let app = Router::new()
        .route(
            "/connect",
            get(
                |Extension(principal): Extension<Principal>, ws: WebSocketUpgrade| async move {
                    ws.on_upgrade(move |mut socket| async move {
                        let _ = principal
                            .run(async {
                                while let Some(Ok(message)) = socket.next().await {
                                    if socket.send(message).await.is_err() {
                                        break;
                                    }
                                }
                            })
                            .await;
                    })
                },
            ),
        )
        .route("/metadata", get(|| async { "metadata" }))
        .route(
            "/auth/reload",
            post(
                |State(access): State<AccessControl>,
                 Extension(principal): Extension<Principal>| async move {
                    if access.reload(&principal).await.is_ok() {
                        StatusCode::OK
                    } else {
                        StatusCode::FORBIDDEN
                    }
                },
            ),
        )
        .with_state(admission.access_for_test())
        .merge(admission.ticket_routes())
        .layer(axum::middleware::from_fn_with_state(
            ErrorShape::Manager,
            crate::http_diagnostics::rejections,
        ))
        .layer(axum::middleware::from_fn_with_state(
            admission.clone(),
            crate::access::admission,
        ));
    let app = HttpBoundaryArgs::default()
        .validate()
        .unwrap()
        .for_listener(address)
        .protect(app, ErrorShape::Manager);
    Server {
        base: format!("http://{address}"),
        alternate: format!("http://localhost:{}", address.port()),
        admission,
        task: tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        }),
    }
}

fn client() -> reqwest::Client {
    reqwest::Client::builder()
        .no_proxy()
        .redirect(reqwest::redirect::Policy::none())
        .timeout(Duration::from_secs(3))
        .build()
        .unwrap()
}

fn target() -> serde_json::Value {
    json!({"kind": "tunnel", "udid": DEVICE, "port": 12345})
}

async fn mint(server: &Server) -> serde_json::Value {
    let response = client()
        .post(format!("{}/auth/ticket", server.base))
        .bearer_auth(TOKEN)
        .json(&json!({"origin": server.base, "target": target()}))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(response.headers()["cache-control"], "no-store");
    let body: serde_json::Value = serde_json::from_str(&response.text().await.unwrap()).unwrap();
    assert_eq!(body["expires_in"], 30);
    assert!(!body.to_string().contains(TOKEN));
    body
}

fn request(server: &Server, path: &str, origin: Option<&str>) -> tungstenite::http::Request<()> {
    let mut request = format!("{}{}", server.base.replacen("http:", "ws:", 1), path)
        .into_client_request()
        .unwrap();
    if let Some(origin) = origin {
        request
            .headers_mut()
            .insert("origin", origin.parse().unwrap());
    }
    request
}

async fn denied(request: tungstenite::http::Request<()>) {
    match tokio_tungstenite::connect_async(request).await {
        Err(tungstenite::Error::Http(response)) => {
            assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
            assert_eq!(response.headers()["cache-control"], "no-store");
            let body = String::from_utf8(response.body().clone().unwrap_or_default()).unwrap();
            assert!(!body.contains(TOKEN));
            assert!(!body.contains(DEVICE));
        }
        _ => panic!("ticket denial did not return a safe HTTP authentication failure"),
    }
}

#[tokio::test]
async fn browser_ticket_is_atomic_single_use_and_never_a_general_http_credential() {
    let fixture = PolicyFixture::new(vec![grant(TOKEN, vec![Permission::Connect])]);
    let server = spawn(&fixture).await;
    let body = mint(&server).await;
    let path = body["websocket_path"].as_str().unwrap();
    let (first, second) = tokio::join!(
        tokio_tungstenite::connect_async(request(&server, path, Some(&server.base))),
        tokio_tungstenite::connect_async(request(&server, path, Some(&server.base)))
    );
    assert_eq!(usize::from(first.is_ok()) + usize::from(second.is_ok()), 1);
    let (mut socket, _) = first.ok().or_else(|| second.ok()).unwrap();
    socket
        .send(tungstenite::Message::Binary(
            b"browser-ticket-echo".to_vec(),
        ))
        .await
        .unwrap();
    assert_eq!(
        socket.next().await.unwrap().unwrap(),
        tungstenite::Message::Binary(b"browser-ticket-echo".to_vec())
    );
    denied(request(&server, path, Some(&server.base))).await;
    let unused = mint(&server).await;
    let response = client()
        .get(format!(
            "{}/metadata?ticket={}",
            server.base,
            unused["ticket"].as_str().unwrap()
        ))
        .header("origin", &server.base)
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    socket.close(None).await.unwrap();
}

#[tokio::test]
async fn ticket_binds_origin_target_and_rejects_ambiguous_or_missing_browser_authentication() {
    let fixture = PolicyFixture::new(vec![grant(TOKEN, vec![Permission::Connect])]);
    let server = spawn(&fixture).await;
    for change in [
        "origin",
        "port",
        "device",
        "hop",
        "interface",
        "upstream",
        "path",
        "order",
    ] {
        let body = mint(&server).await;
        let original = body["websocket_path"].as_str().unwrap();
        let altered = match change {
            "port" => original.replace("port=12345", "port=22"),
            "device" => original.replace(DEVICE, "other-device"),
            "hop" => original.replace("hop=0", "hop=1"),
            "interface" => format!("{original}&interface=other"),
            "upstream" => format!("{original}&upstream=other"),
            "path" => original.replace("/connect?", "/metadata?"),
            "order" => original.replace(
                &format!("udid={DEVICE}&port=12345"),
                &format!("port=12345&udid={DEVICE}"),
            ),
            _ => original.into(),
        };
        denied(request(
            &server,
            &altered,
            Some(if change == "origin" {
                &server.alternate
            } else {
                &server.base
            }),
        ))
        .await;
        denied(request(&server, original, Some(&server.base))).await;
    }
    let body = mint(&server).await;
    let path = body["websocket_path"].as_str().unwrap();
    denied(request(&server, path, None)).await;
    for suffix in [
        format!("&ticket={}", body["ticket"].as_str().unwrap()),
        "&%74icket=malformed".into(),
    ] {
        denied(request(
            &server,
            &format!("{path}{suffix}"),
            Some(&server.base),
        ))
        .await;
    }
    let mut ambiguous = request(&server, path, Some(&server.base));
    ambiguous
        .headers_mut()
        .insert("authorization", format!("Bearer {TOKEN}").parse().unwrap());
    denied(ambiguous).await;
    let (mut socket, _) =
        tokio_tungstenite::connect_async(request(&server, path, Some(&server.base)))
            .await
            .unwrap();
    socket.close(None).await.unwrap();
    denied(request(
        &server,
        &format!("/connect?udid={DEVICE}&port=12345&ticket={TOKEN}"),
        Some(&server.base),
    ))
    .await;
}

#[tokio::test]
async fn issuance_enforces_exact_scope_body_origin_and_bounded_inputs_before_handshake() {
    for roles in [
        vec![Permission::List],
        vec![Permission::ManageServer],
        vec![Permission::Connect],
    ] {
        let fixture = PolicyFixture::new(vec![grant(TOKEN, roles.clone())]);
        let server = spawn(&fixture).await;
        for (target, status) in [
            (
                target(),
                if roles.contains(&Permission::Connect) {
                    StatusCode::OK
                } else {
                    StatusCode::FORBIDDEN
                },
            ),
            (
                json!({"kind":"tunnel","udid":"other-device","port":12345}),
                StatusCode::FORBIDDEN,
            ),
            (
                json!({"kind":"tunnel","udid":DEVICE,"port":22}),
                StatusCode::FORBIDDEN,
            ),
            (
                json!({"kind":"tunnel","udid":DEVICE,"port":0}),
                StatusCode::BAD_REQUEST,
            ),
            (json!({"kind":"cdp_browser"}), StatusCode::BAD_REQUEST),
            (
                json!({"kind":"tunnel","udid":DEVICE,"port":12345,"unexpected":TOKEN}),
                StatusCode::UNPROCESSABLE_ENTITY,
            ),
        ] {
            let response = client()
                .post(format!("{}/auth/ticket", server.base))
                .bearer_auth(TOKEN)
                .json(&json!({"origin":server.base,"target":target}))
                .send()
                .await
                .unwrap();
            assert_eq!(response.status(), status);
            assert!(!response.text().await.unwrap().contains(TOKEN));
        }
        for (origin, request_origin) in [
            ("http://unapproved.invalid", None),
            (server.alternate.as_str(), Some(server.base.as_str())),
        ] {
            let mut request = client()
                .post(format!("{}/auth/ticket", server.base))
                .bearer_auth(TOKEN)
                .json(&json!({"origin":origin,"target":target()}));
            if let Some(origin) = request_origin {
                request = request.header("origin", origin);
            }
            assert_eq!(
                request.send().await.unwrap().status(),
                StatusCode::FORBIDDEN
            );
        }
        let response = client()
            .post(format!("{}/auth/ticket", server.base))
            .bearer_auth(TOKEN)
            .header("content-type", "application/json")
            .body("x".repeat(16 * 1024 + 1))
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::PAYLOAD_TOO_LARGE);
    }
}

#[tokio::test]
async fn cors_preflight_is_bounded_metadata_and_does_not_authenticate_actual_requests() {
    let fixture = PolicyFixture::new(vec![grant(TOKEN, vec![Permission::Connect])]);
    let server = spawn(&fixture).await;
    for (path, method, headers, origin, status) in [
        (
            "/auth/ticket",
            "POST",
            "authorization, content-type",
            server.base.as_str(),
            StatusCode::NO_CONTENT,
        ),
        (
            "/metadata",
            "GET",
            "authorization",
            server.base.as_str(),
            StatusCode::NO_CONTENT,
        ),
        (
            "/unknown",
            "POST",
            "authorization",
            server.base.as_str(),
            StatusCode::FORBIDDEN,
        ),
        (
            "/auth/ticket",
            "PATCH",
            "authorization",
            server.base.as_str(),
            StatusCode::FORBIDDEN,
        ),
        (
            "/auth/ticket",
            "POST",
            "x-unreviewed",
            server.base.as_str(),
            StatusCode::FORBIDDEN,
        ),
        (
            "/auth/ticket",
            "POST",
            "authorization",
            "http://unapproved.invalid",
            StatusCode::FORBIDDEN,
        ),
    ] {
        let response = client()
            .request(reqwest::Method::OPTIONS, format!("{}{path}", server.base))
            .header("origin", origin)
            .header("access-control-request-method", method)
            .header("access-control-request-headers", headers)
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), status);
        assert!(!response
            .headers()
            .contains_key("access-control-allow-credentials"));
        if status == StatusCode::NO_CONTENT {
            assert_eq!(response.headers()["access-control-allow-origin"], origin);
            assert_eq!(response.headers()["cache-control"], "no-store");
        }
    }
    let response = client()
        .post(format!("{}/auth/ticket", server.base))
        .header("origin", &server.base)
        .json(&json!({"origin":server.base,"target":target()}))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    assert_eq!(
        response.headers()["access-control-allow-origin"],
        server.base
    );
    assert!(server.admission.tickets.0.lock().await.is_empty());
}

#[tokio::test]
async fn pending_tickets_expire_reload_invalidates_all_and_failed_reload_preserves_them() {
    let fixture = PolicyFixture::new(vec![grant(
        TOKEN,
        vec![Permission::Connect, Permission::ManageServer],
    )]);
    let server = spawn(&fixture).await;
    let expired = mint(&server).await;
    for entry in server.admission.tickets.0.lock().await.values_mut() {
        entry.deadline = tokio::time::Instant::now();
    }
    denied(request(
        &server,
        expired["websocket_path"].as_str().unwrap(),
        Some(&server.base),
    ))
    .await;
    let preserved = mint(&server).await;
    fixture.malformed();
    assert_eq!(
        client()
            .post(format!("{}/auth/reload", server.base))
            .bearer_auth(TOKEN)
            .send()
            .await
            .unwrap()
            .status(),
        StatusCode::FORBIDDEN
    );
    let (mut socket, _) = tokio_tungstenite::connect_async(request(
        &server,
        preserved["websocket_path"].as_str().unwrap(),
        Some(&server.base),
    ))
    .await
    .unwrap();
    let invalidated = mint(&server).await;
    fixture.replace(vec![grant(
        TOKEN,
        vec![Permission::Connect, Permission::ManageServer],
    )]);
    assert_eq!(
        client()
            .post(format!("{}/auth/reload", server.base))
            .bearer_auth(TOKEN)
            .send()
            .await
            .unwrap()
            .status(),
        StatusCode::OK
    );
    denied(request(
        &server,
        invalidated["websocket_path"].as_str().unwrap(),
        Some(&server.base),
    ))
    .await;
    assert!(matches!(
        tokio::time::timeout(Duration::from_secs(2), socket.next())
            .await
            .unwrap(),
        None | Some(Err(_)) | Some(Ok(tungstenite::Message::Close(_)))
    ));
}

#[tokio::test]
async fn ticket_capacity_is_explicit_and_expired_entries_release_capacity() {
    let fixture = PolicyFixture::new(vec![grant(TOKEN, vec![Permission::Connect])]);
    let server = spawn(&fixture).await;
    for _ in 0..CAPACITY {
        mint(&server).await;
    }
    let response = client()
        .post(format!("{}/auth/ticket", server.base))
        .bearer_auth(TOKEN)
        .json(&json!({"origin":server.base,"target":target()}))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::TOO_MANY_REQUESTS);
    assert!(!response.text().await.unwrap().contains(TOKEN));
    for entry in server.admission.tickets.0.lock().await.values_mut() {
        entry.deadline = tokio::time::Instant::now();
    }
    mint(&server).await;
    assert_eq!(server.admission.tickets.0.lock().await.len(), 1);
}
