use super::*;
use crate::access::test_support::{grant, JsonRequest, PolicyFixture, DEVICE, REPLACEMENT, TOKEN};
use clap::Parser;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio_tungstenite::tungstenite::{self, client::IntoClientRequest};

#[derive(Parser)]
struct Arguments {
    #[command(subcommand)]
    command: WebInspectorSub,
}

fn client() -> reqwest::Client {
    reqwest::Client::builder()
        .no_proxy()
        .redirect(reqwest::redirect::Policy::none())
        .timeout(Duration::from_secs(4))
        .build()
        .unwrap()
}

async fn response_json(response: reqwest::Response) -> JsonValue {
    serde_json::from_str(&response.text().await.unwrap()).unwrap()
}

async fn spawn(
    fixture: &PolicyFixture,
    device: &str,
    cdp: bool,
) -> (u16, tokio::task::JoinHandle<()>) {
    let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let state = ServerState {
        udid: device.into(),
        timeout: Duration::from_secs(30),
        cdp_host: "127.0.0.1".into(),
        cdp_port: port,
        access: AccessControl::from_file(fixture.path.clone())
            .await
            .unwrap(),
        selenium_sessions: Sessions::default(),
    };
    let router = if cdp {
        cdp_router(state)
    } else {
        selenium_router(state)
    };
    let router = HttpBoundaryArgs::default()
        .validate()
        .unwrap()
        .for_listener(listener.local_addr().unwrap())
        .protect(
            router,
            if cdp {
                crate::http_diagnostics::ErrorShape::Cdp
            } else {
                crate::http_diagnostics::ErrorShape::WebDriver
            },
        );
    (
        port,
        tokio::spawn(async move {
            axum::serve(listener, router).await.unwrap();
        }),
    )
}

fn check_shape(body: &JsonValue, cdp: bool) {
    if cdp {
        assert_eq!(body["error"]["code"], -32600);
        assert!(body["error"]["message"].is_string());
    } else {
        assert_eq!(body["value"]["error"], "invalid request");
        assert!(body["value"]["message"].is_string());
    }
}

#[tokio::test]
async fn startup_rejects_implicit_trust_and_remote_hosts_before_binding_or_device_access() {
    for command in ["cdp", "selenium"] {
        for extra in [
            vec![],
            vec!["--host", "0.0.0.0", "--local-unauthenticated"],
            vec!["--auth-config", "nonexistent-synthetic-debug-policy"],
        ] {
            let mut arguments = vec!["webinspector", command, "--port", "0"];
            arguments.extend(extra);
            let args = Arguments::try_parse_from(arguments).unwrap();
            let (host, port, access) = match args.command {
                WebInspectorSub::Cdp {
                    host, port, access, ..
                }
                | WebInspectorSub::Selenium {
                    host, port, access, ..
                } => (host, port, access),
                _ => unreachable!(),
            };
            assert!(server_state(
                DEVICE,
                &host,
                port,
                Duration::from_secs(1),
                access,
                HttpBoundaryArgs::default()
            )
            .await
            .is_err());
        }
        assert!(Arguments::try_parse_from([
            "webinspector",
            command,
            "--local-unauthenticated",
            "--auth-config",
            "synthetic-policy"
        ])
        .is_err());
    }
}

#[tokio::test]
async fn local_startup_uses_the_actual_ephemeral_and_ipv6_endpoint_in_metadata() {
    for host in ["localhost", "::1"] {
        let args = Arguments::try_parse_from([
            "webinspector",
            "cdp",
            "--host",
            host,
            "--port",
            "0",
            "--local-unauthenticated",
        ])
        .unwrap();
        let WebInspectorSub::Cdp {
            host, port, access, ..
        } = args.command
        else {
            unreachable!()
        };
        let (listener, state, _) = server_state(
            DEVICE,
            &host,
            port,
            Duration::from_secs(1),
            access,
            HttpBoundaryArgs::default(),
        )
        .await
        .unwrap();
        let bound = listener.local_addr().unwrap();
        assert_ne!(bound.port(), 0);
        let Json(metadata) = cdp_version(State(state)).await;
        assert_eq!(
            metadata["webSocketDebuggerUrl"],
            format!("ws://{bound}/devtools/browser/ios-cli")
        );
    }
}

#[tokio::test]
async fn actual_debug_routers_authenticate_before_parsing_paths_bodies_or_handshakes() {
    for cdp in [true, false] {
        let fixture = PolicyFixture::new(vec![grant(TOKEN, vec![Permission::Debug])]);
        let (port, server) = spawn(&fixture, DEVICE, cdp).await;
        let base = format!("http://127.0.0.1:{port}");
        let client = client();
        let paths = if cdp {
            vec![
                "/json",
                "/json/list",
                "/json/version",
                "/devtools/page/synthetic-secret",
                "/devtools/browser/synthetic-secret",
                "/auth/reload",
                "/not-a-route",
            ]
        } else {
            vec![
                "/status",
                "/session",
                "/session/synthetic-secret/url",
                "/session/synthetic-secret/element/synthetic-secret/click",
                "/auth/reload",
                "/not-a-route",
            ]
        };
        for path in paths {
            for method in [
                reqwest::Method::GET,
                reqwest::Method::POST,
                reqwest::Method::DELETE,
            ] {
                let response = client
                    .request(method, format!("{base}{path}"))
                    .header("content-type", "application/json")
                    .body("synthetic-secret-bad-json")
                    .send()
                    .await
                    .unwrap();
                assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
                assert_eq!(response.headers()["cache-control"], "no-store");
                assert!(response.headers().contains_key("www-authenticate"));
                let original = response.text().await.unwrap();
                for secret in [TOKEN, DEVICE, "synthetic-secret"] {
                    assert!(!original.contains(secret));
                }
                check_shape(&serde_json::from_str(&original).unwrap(), cdp);
            }
        }
        let metadata = if cdp { "/json/version" } else { "/status" };
        for credential in ["short", REPLACEMENT] {
            assert_eq!(
                client
                    .get(format!("{base}{metadata}"))
                    .bearer_auth(credential)
                    .send()
                    .await
                    .unwrap()
                    .status(),
                StatusCode::UNAUTHORIZED
            );
        }
        let mut headers = reqwest::header::HeaderMap::new();
        headers.append("authorization", format!("Bearer {TOKEN}").parse().unwrap());
        headers.append("authorization", format!("Bearer {TOKEN}").parse().unwrap());
        assert_eq!(
            client
                .get(format!("{base}{metadata}"))
                .headers(headers)
                .send()
                .await
                .unwrap()
                .status(),
            StatusCode::UNAUTHORIZED
        );
        let response = client
            .head(format!("{base}{metadata}"))
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
        assert!(response.bytes().await.unwrap().is_empty());
        let response = client
            .get(format!("{base}{metadata}"))
            .bearer_auth(TOKEN)
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let body = response_json(response).await;
        if cdp {
            assert_eq!(
                body["webSocketDebuggerUrl"],
                format!("ws://127.0.0.1:{port}/devtools/browser/ios-cli")
            );
        } else {
            assert_eq!(body["value"]["ready"], true);
        }
        assert_eq!(
            client
                .post(format!("{base}/auth/reload"))
                .bearer_auth(TOKEN)
                .send()
                .await
                .unwrap()
                .status(),
            StatusCode::FORBIDDEN
        );
        server.abort();
        let _ = server.await;
    }
}

#[tokio::test]
async fn roles_and_device_scope_deny_before_any_device_operation_or_extractor() {
    for cdp in [true, false] {
        for (roles, device) in [
            (
                vec![
                    Permission::List,
                    Permission::Connect,
                    Permission::ManageServer,
                ],
                DEVICE,
            ),
            (vec![Permission::Debug], "synthetic-other-device"),
        ] {
            let fixture = PolicyFixture::new(vec![grant(TOKEN, roles)]);
            let (port, server) = spawn(&fixture, device, cdp).await;
            for path in if cdp {
                vec!["/json/list", "/json/version", "/devtools/page/not-a-page"]
            } else {
                vec!["/status", "/session", "/session/not-a-session/url"]
            } {
                for method in [reqwest::Method::GET, reqwest::Method::POST] {
                    let response = client()
                        .request(method, format!("http://127.0.0.1:{port}{path}"))
                        .bearer_auth(TOKEN)
                        .header("content-type", "application/json")
                        .body("bad-secret-json")
                        .send()
                        .await
                        .unwrap();
                    assert_eq!(response.status(), StatusCode::FORBIDDEN);
                    let body = response_json(response).await;
                    check_shape(&body, cdp);
                }
            }
            server.abort();
            let _ = server.await;
        }
    }
}

#[tokio::test]
async fn reload_preserves_a_failed_policy_and_replaces_debug_credentials_atomically() {
    for cdp in [true, false] {
        let fixture = PolicyFixture::new(vec![grant(
            TOKEN,
            vec![Permission::Debug, Permission::ManageServer],
        )]);
        let (port, server) = spawn(&fixture, DEVICE, cdp).await;
        let base = format!("http://127.0.0.1:{port}");
        let metadata = if cdp { "/json/version" } else { "/status" };
        fixture.malformed();
        let response = client()
            .post(format!("{base}/auth/reload"))
            .bearer_auth(TOKEN)
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        let body = response_json(response).await;
        check_shape(&body, cdp);
        assert!(!body.to_string().contains(fixture.path.to_str().unwrap()));
        assert_eq!(
            client()
                .get(format!("{base}{metadata}"))
                .bearer_auth(TOKEN)
                .send()
                .await
                .unwrap()
                .status(),
            StatusCode::OK
        );
        fixture.replace(vec![grant(
            REPLACEMENT,
            vec![Permission::Debug, Permission::ManageServer],
        )]);
        let response = client()
            .post(format!("{base}/auth/reload"))
            .bearer_auth(TOKEN)
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(response_json(response).await["data"], true);
        assert_eq!(
            client()
                .get(format!("{base}{metadata}"))
                .bearer_auth(TOKEN)
                .send()
                .await
                .unwrap()
                .status(),
            StatusCode::UNAUTHORIZED
        );
        assert_eq!(
            client()
                .get(format!("{base}{metadata}"))
                .bearer_auth(REPLACEMENT)
                .send()
                .await
                .unwrap()
                .status(),
            StatusCode::OK
        );
        server.abort();
        let _ = server.await;
    }
}

#[tokio::test]
async fn admitted_page_websocket_revokes_while_waiting_for_the_device_page_lock() {
    page_websocket_revokes(false).await;
}

#[tokio::test]
async fn browser_page_ticket_revokes_without_contacting_a_device() {
    page_websocket_revokes(true).await;
}

async fn page_websocket_revokes(ticket: bool) {
    for expire in [false, true] {
        let scoped_grant = grant(TOKEN, vec![Permission::Debug, Permission::ManageServer]);
        let fixture = PolicyFixture::new(vec![if expire {
            scoped_grant.expires_after(3)
        } else {
            scoped_grant
        }]);
        let page = format!("synthetic-blocked-page-{}", Uuid::new_v4());
        let _guard = lock_cdp_page(&page, Duration::from_secs(1)).await.unwrap();
        let (port, server) = spawn(&fixture, DEVICE, true).await;
        let url = format!("ws://127.0.0.1:{port}/devtools/page/{page}");
        match tokio_tungstenite::connect_async(&url).await.unwrap_err() {
            tungstenite::Error::Http(response) => {
                assert_eq!(response.status(), StatusCode::UNAUTHORIZED)
            }
            other => panic!(
                "unexpected WebSocket failure category: {}",
                ios_core::redaction::diagnostic_error(&other)
            ),
        }
        let mut request = if ticket {
            let origin = format!("http://127.0.0.1:{port}");
            let issued = client()
                .post(format!("{origin}/auth/ticket"))
                .bearer_auth(TOKEN)
                .json(&json!({"origin": origin,
                    "target": {"kind": "cdp_page", "page_id": page}}))
                .send()
                .await
                .unwrap();
            assert_eq!(issued.status(), StatusCode::OK);
            let body: JsonValue = response_json(issued).await;
            let mut request = format!(
                "ws://127.0.0.1:{port}{}",
                body["websocket_path"].as_str().unwrap()
            )
            .into_client_request()
            .unwrap();
            request
                .headers_mut()
                .insert("origin", origin.parse().unwrap());
            request
        } else {
            let mut request = url.into_client_request().unwrap();
            request
                .headers_mut()
                .insert("authorization", format!("Bearer {TOKEN}").parse().unwrap());
            request
        };
        // Mark native credential sensitivity even in test-owned fixtures.
        if let Some(value) = request.headers_mut().get_mut("authorization") {
            value.set_sensitive(true);
        }
        let (mut socket, response) = tokio_tungstenite::connect_async(request).await.unwrap();
        assert_eq!(response.status(), StatusCode::SWITCHING_PROTOCOLS);
        if !expire {
            fixture.replace(vec![grant(
                REPLACEMENT,
                vec![Permission::Debug, Permission::ManageServer],
            )]);
            assert_eq!(
                client()
                    .post(format!("http://127.0.0.1:{port}/auth/reload"))
                    .bearer_auth(TOKEN)
                    .send()
                    .await
                    .unwrap()
                    .status(),
                StatusCode::OK
            );
        }
        let result = tokio::time::timeout(Duration::from_secs(4), socket.next())
            .await
            .unwrap();
        assert!(matches!(
            result,
            None | Some(Ok(tungstenite::Message::Close(_))) | Some(Err(_))
        ));
        // The held page lock prevented every device connection. This is host
        // admission/revocation evidence, not a CDP/Apple workflow simulation.
        server.abort();
        let _ = server.await;
    }
}

#[tokio::test]
async fn reload_cancels_pending_webdriver_body_extraction_with_a_safe_forbidden_reply() {
    let fixture = PolicyFixture::new(vec![grant(
        TOKEN,
        vec![Permission::Debug, Permission::ManageServer],
    )]);
    let (port, server) = spawn(&fixture, DEVICE, false).await;
    let mut connection = tokio::net::TcpStream::connect(("127.0.0.1", port))
        .await
        .unwrap();
    connection.write_all(format!("POST /session/synthetic-session/url HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nAuthorization: Bearer {TOKEN}\r\nContent-Type: application/json\r\nContent-Length: 100\r\n\r\n{{").as_bytes()).await.unwrap();
    // Confirm the request is waiting rather than already rejected.
    let mut bytes = [0; 4096];
    assert!(
        tokio::time::timeout(Duration::from_millis(50), connection.read(&mut bytes))
            .await
            .is_err()
    );
    fixture.replace(vec![grant(
        REPLACEMENT,
        vec![Permission::Debug, Permission::ManageServer],
    )]);
    assert_eq!(
        client()
            .post(format!("http://127.0.0.1:{port}/auth/reload"))
            .bearer_auth(TOKEN)
            .send()
            .await
            .unwrap()
            .status(),
        StatusCode::OK
    );
    let count = tokio::time::timeout(Duration::from_secs(2), connection.read(&mut bytes))
        .await
        .unwrap()
        .unwrap();
    let original = std::str::from_utf8(&bytes[..count]).unwrap();
    assert!(original.starts_with("HTTP/1.1 403"));
    assert!(!original.contains(TOKEN));
    assert!(!original.contains("synthetic-session"));
    server.abort();
    let _ = server.await;
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
    });
    let _released = tokio::time::timeout(Duration::from_secs(1), lock.lock_owned())
        .await
        .unwrap();
}
