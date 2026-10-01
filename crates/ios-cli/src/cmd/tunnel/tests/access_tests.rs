use super::*;
use crate::access::test_support::{grant, JsonRequest, PolicyFixture, DEVICE, REPLACEMENT, TOKEN};
use std::sync::atomic::{AtomicUsize, Ordering};
use tungstenite::client::IntoClientRequest;

fn client() -> reqwest::Client {
    reqwest::Client::builder()
        .no_proxy()
        .timeout(Duration::from_secs(3))
        .build()
        .unwrap()
}

fn websocket_request(port: u16, device: &str, target_port: u16) -> tungstenite::http::Request<()> {
    let mut url = reqwest::Url::parse(&format!("ws://127.0.0.1:{port}/connect")).unwrap();
    url.query_pairs_mut()
        .append_pair("udid", device)
        .append_pair("port", &target_port.to_string());
    let mut request = url.as_str().into_client_request().unwrap();
    let mut header: tungstenite::http::HeaderValue = format!("Bearer {TOKEN}").parse().unwrap();
    header.set_sensitive(true);
    request.headers_mut().insert("authorization", header);
    request
}

async fn authenticated_state(fixture: &PolicyFixture) -> TunnelAgentState {
    TunnelAgentState::with_access(
        TunMode::Userspace,
        AccessControl::from_file(fixture.path.clone())
            .await
            .unwrap(),
    )
}

#[tokio::test]
async fn actual_http_authentication_precedes_parsing_and_all_manager_operations() {
    let fixture = PolicyFixture::new(vec![grant(TOKEN, vec![Permission::List])]);
    let state = authenticated_state(&fixture).await;
    // List-only policy does not enter usbmux/device discovery.
    state.refresh_once().await.unwrap();
    assert!(state.is_ready().await);
    let (port, server) = spawn_test_router(agent_router(state)).await;
    let base = format!("http://127.0.0.1:{port}");
    let client = client();
    let cases = [
        (reqwest::Method::GET, "/"),
        (reqwest::Method::GET, "/tunnels"),
        (reqwest::Method::GET, "/connect?port=synthetic-secret"),
        (reqwest::Method::GET, "/tunnel/synthetic-secret"),
        (reqwest::Method::DELETE, "/tunnel/synthetic-secret"),
        (reqwest::Method::GET, "/start-tunnel?udid=synthetic-secret"),
        (reqwest::Method::POST, "/start-tunnel"),
        (reqwest::Method::GET, "/upstream"),
        (reqwest::Method::POST, "/upstream"),
        (reqwest::Method::DELETE, "/upstream"),
        (reqwest::Method::POST, "/auth/reload"),
        (reqwest::Method::GET, "/shutdown"),
        (reqwest::Method::POST, "/shutdown"),
        (reqwest::Method::GET, "/health"),
        (reqwest::Method::GET, "/ready"),
        (reqwest::Method::GET, "/not-a-route"),
    ];
    for (method, path) in cases {
        let response = client
            .request(method, format!("{base}{path}"))
            .body("synthetic-secret-invalid-json")
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
        assert_eq!(response.headers()["cache-control"], "no-store");
        assert!(response.headers().contains_key("www-authenticate"));
        let original = response.text().await.unwrap();
        assert!(original.contains("manager authentication required"));
        assert!(!original.contains("synthetic-secret"));
        assert!(!original.contains(TOKEN));
    }
    let response = client.head(format!("{base}/tunnels")).send().await.unwrap();
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    assert!(response.bytes().await.unwrap().is_empty());
    for path in ["/", "/tunnels", "/health", "/ready"] {
        let response = client
            .get(format!("{base}{path}"))
            .bearer_auth(TOKEN)
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
    }
    for (method, path) in [
        (reqwest::Method::GET, "/upstream"),
        (reqwest::Method::GET, "/shutdown"),
        (reqwest::Method::POST, "/auth/reload"),
        (reqwest::Method::DELETE, "/tunnel/synthetic-manager-device"),
    ] {
        let response = client
            .request(method, format!("{base}{path}"))
            .bearer_auth(TOKEN)
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::FORBIDDEN);
        assert!(response
            .text()
            .await
            .unwrap()
            .contains("manager scope denied"));
    }
    for method in [reqwest::Method::POST, reqwest::Method::DELETE] {
        let response = client
            .request(method, format!("{base}/upstream"))
            .bearer_auth(TOKEN)
            .header("content-type", "application/json")
            .body(r#"{"url":"http://127.0.0.1:49152/"}"#)
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::FORBIDDEN);
    }
    server.abort();
    let _ = server.await;
}

#[tokio::test]
async fn device_and_port_scopes_filter_lists_and_deny_before_upstream_contact() {
    let calls = Arc::new(AtomicUsize::new(0));
    let observed = calls.clone();
    let upstream = Router::new().route("/", get(move || {
        let observed = observed.clone();
        async move {
            observed.fetch_add(1, Ordering::SeqCst);
            Json(json!({DEVICE: [{"interface": "fake"}], "synthetic-forbidden-device": [{"interface": "other"}]}))
        }
    }));
    let (upstream_port, upstream_task) = spawn_test_router(upstream).await;
    let fixture = PolicyFixture::new(vec![grant(
        TOKEN,
        vec![
            Permission::List,
            Permission::Connect,
            Permission::ManageTunnels,
        ],
    )]);
    let state = authenticated_state(&fixture).await;
    state
        .inner
        .upstreams
        .write()
        .await
        .insert(format!("http://127.0.0.1:{upstream_port}/"));
    let (port, server) = spawn_test_router(agent_router(state)).await;
    let client = client();
    let base = format!("http://127.0.0.1:{port}");
    let response = client
        .get(format!("{base}/"))
        .bearer_auth(TOKEN)
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let original = response.text().await.unwrap();
    let listing: FederationListing = serde_json::from_str(&original).unwrap();
    assert_eq!(listing.len(), 1);
    assert!(listing.contains_key(DEVICE));
    assert!(!original.contains("synthetic-forbidden-device"));
    let initial_calls = calls.load(Ordering::SeqCst);
    assert_eq!(initial_calls, 1);
    for (device, target_port) in [
        ("synthetic-forbidden-device", 12345),
        (DEVICE, 22),
        (DEVICE, 0),
    ] {
        let failure =
            tokio_tungstenite::connect_async(websocket_request(port, device, target_port))
                .await
                .unwrap_err();
        assert!(
            matches!(failure, tungstenite::Error::Http(response) if response.status() == StatusCode::FORBIDDEN)
        );
    }
    assert_eq!(
        client
            .get(format!("{base}/tunnel/synthetic-forbidden-device"))
            .bearer_auth(TOKEN)
            .send()
            .await
            .unwrap()
            .status(),
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        client
            .post(format!("{base}/start-tunnel"))
            .bearer_auth(TOKEN)
            .header("content-type", "application/json")
            .body("{\"udid\":\"synthetic-forbidden-device\"}")
            .send()
            .await
            .unwrap()
            .status(),
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        client
            .delete(format!("{base}/tunnel/synthetic-forbidden-device"))
            .bearer_auth(TOKEN)
            .send()
            .await
            .unwrap()
            .status(),
        StatusCode::FORBIDDEN
    );
    assert_eq!(calls.load(Ordering::SeqCst), initial_calls);
    // An authorized unknown target is a real 404, never an empty success.
    assert_eq!(
        client
            .delete(format!("{base}/tunnel/{DEVICE}"))
            .bearer_auth(TOKEN)
            .send()
            .await
            .unwrap()
            .status(),
        StatusCode::NOT_FOUND
    );
    server.abort();
    upstream_task.abort();
    let _ = server.await;
    let _ = upstream_task.await;
}

#[tokio::test]
async fn actual_admin_reload_preserves_bad_policy_and_replaces_credentials() {
    let roles = vec![Permission::ManageServer, Permission::List];
    let fixture = PolicyFixture::new(vec![grant(TOKEN, roles.clone())]);
    let (port, server) = spawn_test_router(agent_router(authenticated_state(&fixture).await)).await;
    let base = format!("http://127.0.0.1:{port}");
    let client = client();
    fixture.malformed();
    let response = client
        .post(format!("{base}/auth/reload"))
        .bearer_auth(TOKEN)
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    let original = response.text().await.unwrap();
    assert!(original.contains("manager policy reload failed"));
    assert!(!original.contains("invalid synthetic policy"));
    assert_eq!(
        client
            .get(format!("{base}/tunnels"))
            .bearer_auth(TOKEN)
            .send()
            .await
            .unwrap()
            .status(),
        StatusCode::OK
    );
    fixture.replace(vec![grant(REPLACEMENT, roles)]);
    let response = client
        .post(format!("{base}/auth/reload"))
        .bearer_auth(TOKEN)
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        serde_json::from_slice::<serde_json::Value>(&response.bytes().await.unwrap()).unwrap()
            ["data"],
        true
    );
    assert_eq!(
        client
            .get(format!("{base}/tunnels"))
            .bearer_auth(TOKEN)
            .send()
            .await
            .unwrap()
            .status(),
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        client
            .get(format!("{base}/tunnels"))
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

#[tokio::test]
async fn reload_cancels_an_admitted_listing_instead_of_returning_empty_success() {
    let entered = Arc::new(Notify::new());
    let release = Arc::new(Notify::new());
    let entered_handler = entered.clone();
    let release_handler = release.clone();
    let upstream = Router::new().route(
        "/",
        get(move || {
            let entered = entered_handler.clone();
            let release = release_handler.clone();
            async move {
                entered.notify_one();
                release.notified().await;
                Json(json!({DEVICE: [{"interface": "fake"}]}))
            }
        }),
    );
    let (upstream_port, upstream_task) = spawn_test_router(upstream).await;
    let fixture = PolicyFixture::new(vec![grant(
        TOKEN,
        vec![Permission::List, Permission::ManageServer],
    )]);
    let state = authenticated_state(&fixture).await;
    state
        .inner
        .upstreams
        .write()
        .await
        .insert(format!("http://127.0.0.1:{upstream_port}/"));
    let (port, server) = spawn_test_router(agent_router(state)).await;
    let pending = tokio::spawn(async move {
        client()
            .get(format!("http://127.0.0.1:{port}/"))
            .bearer_auth(TOKEN)
            .send()
            .await
            .unwrap()
    });
    tokio::time::timeout(Duration::from_secs(2), entered.notified())
        .await
        .unwrap();
    fixture.replace(vec![]);
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
    let response = tokio::time::timeout(Duration::from_secs(2), pending)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(response.status(), StatusCode::FORBIDDEN);
    assert!(response
        .text()
        .await
        .unwrap()
        .contains("manager scope denied"));
    release.notify_one();
    server.abort();
    upstream_task.abort();
    let _ = server.await;
    let _ = upstream_task.await;
}

#[tokio::test]
async fn authenticated_websocket_preserves_bytes_and_revocation_closes_both_hops() {
    authenticated_bridge(false, false).await;
}

#[tokio::test]
async fn authenticated_websocket_expiry_closes_both_hops() {
    authenticated_bridge(true, false).await;
}

#[tokio::test]
async fn browser_ticket_bridge_preserves_bytes_and_revoke_or_expiry_closes_both_hops() {
    for expiry in [false, true] {
        authenticated_bridge(expiry, true).await;
    }
}

async fn authenticated_bridge(expiry: bool, ticket: bool) {
    let proxy = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
    let proxy_port = proxy.local_addr().unwrap().port();
    let proxy_task = tokio::spawn(async move {
        let (mut stream, _) = proxy.accept().await.unwrap();
        let mut preamble = [0; 20];
        stream.read_exact(&mut preamble).await.unwrap();
        assert_eq!(
            &preamble[..16],
            &Ipv6Addr::from_str("fd00::1234").unwrap().octets()
        );
        assert_eq!(
            u32::from_le_bytes(preamble[16..].try_into().unwrap()),
            12345
        );
        let mut bytes = [0; 64];
        let count = stream.read(&mut bytes).await.unwrap();
        stream.write_all(&bytes[..count]).await.unwrap();
        assert_eq!(stream.read(&mut bytes).await.unwrap(), 0);
    });
    let mut record = fake_record(proxy_port);
    record.udid = DEVICE.into();
    let listing = serde_json::to_value(&record).unwrap();
    let upstream = Router::new()
        .route(
            "/",
            get(move || async move { Json(json!({DEVICE: [listing]})) }),
        )
        .route(
            "/connect",
            get(
                move |Query(query): Query<ConnectQuery>, ws: WebSocketUpgrade| async move {
                    assert_eq!(query.udid, DEVICE);
                    assert_eq!(query.port, 12345);
                    assert_eq!(query.hop, 1);
                    let stream = connect_record_port(&record, query.port).await.unwrap();
                    ws.on_upgrade(move |socket| async move {
                        let _ = bridge_tcp_websocket(socket, stream).await;
                    })
                },
            ),
        );
    let (upstream_port, upstream_task) = spawn_test_router(upstream).await;
    let mut scope = grant(TOKEN, vec![Permission::Connect, Permission::ManageServer]);
    if expiry {
        scope = scope.expires_after(3);
    }
    let fixture = PolicyFixture::new(vec![scope]);
    let state = authenticated_state(&fixture).await;
    state
        .inner
        .upstreams
        .write()
        .await
        .insert(format!("http://127.0.0.1:{upstream_port}/"));
    let (port, server) = spawn_test_router(agent_router(state)).await;
    let request = if ticket {
        let origin = format!("http://127.0.0.1:{port}");
        let issued = client()
            .post(format!("{origin}/auth/ticket"))
            .bearer_auth(TOKEN)
            .json(&json!({"origin": origin,
                "target": {"kind": "tunnel", "udid": DEVICE, "port": 12345}}))
            .send()
            .await
            .unwrap();
        assert_eq!(issued.status(), StatusCode::OK);
        let body: serde_json::Value = serde_json::from_str(&issued.text().await.unwrap()).unwrap();
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
        websocket_request(port, DEVICE, 12345)
    };
    let (mut websocket, _) = tokio_tungstenite::connect_async(request).await.unwrap();
    websocket
        .send(tungstenite::Message::Binary(
            b"synthetic-device-bytes".to_vec(),
        ))
        .await
        .unwrap();
    assert_eq!(
        websocket.next().await.unwrap().unwrap(),
        tungstenite::Message::Binary(b"synthetic-device-bytes".to_vec())
    );
    if !expiry {
        fixture.replace(vec![]);
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
    let closed = tokio::time::timeout(Duration::from_secs(4), websocket.next())
        .await
        .unwrap();
    assert!(matches!(
        closed,
        None | Some(Err(_)) | Some(Ok(tungstenite::Message::Close(_)))
    ));
    tokio::time::timeout(Duration::from_secs(2), proxy_task)
        .await
        .unwrap()
        .unwrap();
    server.abort();
    upstream_task.abort();
    let _ = server.await;
    let _ = upstream_task.await;
}

#[tokio::test]
async fn credential_client_reads_private_file_without_following_redirects_or_unbounded_bodies() {
    let fixture = PolicyFixture::new(vec![grant(TOKEN, vec![Permission::List])]);
    let recipient = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
    let redirected_port = recipient.local_addr().unwrap().port();
    let upstream = Router::new()
        .route(
            "/tunnels",
            get(|| async { Json(Vec::<TunnelRecord>::new()) }),
        )
        .route(
            "/redirect",
            get(move || async move {
                (
                    StatusCode::FOUND,
                    [(
                        "location",
                        format!("http://127.0.0.1:{redirected_port}/leak"),
                    )],
                )
            }),
        )
        .route(
            "/oversized",
            get(|| async { vec![b'x'; MAX_LIST_BYTES + 1] }),
        )
        .layer(axum::middleware::from_fn(
            |request: axum::extract::Request, next: axum::middleware::Next| async move {
                assert!(request.headers()["authorization"] == format!("Bearer {TOKEN}"));
                next.run(request).await
            },
        ));
    let (port, server) = spawn_test_router(upstream).await;
    // Construct the same parsed CLI options rather than making the private path public.
    let options = || {
        let parsed = crate::Cli::try_parse_from([
            "ios",
            "tunnel",
            "list",
            "--token-file",
            fixture.token_path.to_str().unwrap(),
        ])
        .unwrap();
        match parsed.command {
            crate::Commands::Tunnel(TunnelCmd {
                sub: TunnelSub::List { access, .. },
            }) => access,
            _ => panic!("parsed manager client"),
        }
    };
    assert_eq!(
        tunnel_manager_request(
            reqwest::Method::GET,
            format!("http://127.0.0.1:{port}/tunnels"),
            options()
        )
        .await
        .unwrap(),
        "[]"
    );
    assert!(tunnel_manager_request(
        reqwest::Method::GET,
        format!("http://127.0.0.1:{port}/redirect"),
        options()
    )
    .await
    .is_err());
    assert!(
        tokio::time::timeout(Duration::from_millis(50), recipient.accept())
            .await
            .is_err()
    );
    assert!(tunnel_manager_request(
        reqwest::Method::GET,
        format!("http://127.0.0.1:{port}/oversized"),
        options()
    )
    .await
    .is_err());
    server.abort();
    let _ = server.await;
}
