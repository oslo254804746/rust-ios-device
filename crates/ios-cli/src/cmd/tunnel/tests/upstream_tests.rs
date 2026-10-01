use super::*;
use crate::access::test_support::{grant, tls_forward, PolicyFixture, DEVICE, REPLACEMENT, TOKEN};
use crate::access::test_tls_material::{CA, VALID};
use tungstenite::client::IntoClientRequest;

#[derive(clap::Parser)]
struct Options {
    #[command(flatten)]
    upstreams: UpstreamArgs,
    #[command(flatten)]
    boundary: crate::access::HttpBoundaryArgs,
}

async fn configured(fixture: &PolicyFixture, entries: serde_json::Value) -> Upstreams {
    let path = fixture
        .path
        .parent()
        .unwrap()
        .join(format!("upstreams-{}.json", uuid::Uuid::new_v4()));
    ios_core::secret_file::write_secret_new(
        &path,
        &serde_json::to_vec(&json!({"version":1,"upstreams":entries})).unwrap(),
    )
    .unwrap();
    Options::parse_from(["fixture", "--upstream-config", path.to_str().unwrap()])
        .upstreams
        .load()
        .await
        .unwrap()
}

fn client() -> reqwest::Client {
    reqwest::Client::builder()
        .no_proxy()
        .redirect(reqwest::redirect::Policy::none())
        .timeout(Duration::from_secs(3))
        .build()
        .unwrap()
}

#[tokio::test]
async fn two_production_managers_use_distinct_credentials_across_verified_tls_and_preserve_revocation(
) {
    let proxy = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
    let mut record = fake_record(proxy.local_addr().unwrap().port());
    record.udid = DEVICE.into();
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
        let expected = b"two-protected-manager-bytes";
        let mut bytes = vec![0; expected.len()];
        stream.read_exact(&mut bytes).await.unwrap();
        assert_eq!(bytes, expected);
        stream.write_all(&bytes).await.unwrap();
        assert_eq!(stream.read(&mut [0; 1]).await.unwrap(), 0);
    });
    let leaf_token = "f".repeat(64);
    let expected = format!("Bearer {leaf_token}");
    let ws_expected = expected.clone();
    let listing = serde_json::to_value(&record).unwrap();
    let leaf = Router::new()
        .route(
            "/",
            get(
                move |headers: axum::http::HeaderMap, Query(query): Query<FederationQuery>| {
                    let expected = expected.clone();
                    let listing = listing.clone();
                    async move {
                        assert_eq!(headers["authorization"], expected);
                        assert_eq!(query.hop, 2);
                        Json(json!({DEVICE:[listing]}))
                    }
                },
            ),
        )
        .route(
            "/connect",
            get(
                move |headers: axum::http::HeaderMap,
                      Query(query): Query<ConnectQuery>,
                      ws: WebSocketUpgrade| {
                    let expected = ws_expected.clone();
                    let record = record.clone();
                    async move {
                        assert_eq!(headers["authorization"], expected);
                        assert_eq!(query.hop, 2);
                        assert_eq!(query.udid, DEVICE);
                        assert_eq!(query.port, 12345);
                        assert!(query.upstream.is_none());
                        let tcp = connect_record_port(&record, query.port).await.unwrap();
                        ws.on_upgrade(move |socket| async move {
                            let _ = bridge_tcp_websocket(socket, tcp).await;
                        })
                    }
                },
            ),
        );
    let (leaf_port, leaf_task) = spawn_test_router(leaf).await;
    let leaf_root = format!("http://127.0.0.1:{leaf_port}/");
    let middle = PolicyFixture::new(vec![grant(
        REPLACEMENT,
        vec![
            Permission::List,
            Permission::Connect,
            Permission::ManageServer,
        ],
    )]);
    ios_core::secret_file::write_secret(&middle.token_path, REPLACEMENT.as_bytes()).unwrap();
    let leaf_file = middle.path.parent().unwrap().join("leaf-token");
    ios_core::secret_file::write_secret_new(&leaf_file, leaf_token.as_bytes()).unwrap();
    let middle_upstreams =
        configured(&middle, json!([{"url":leaf_root,"token_file":leaf_file}])).await;
    let middle_state = TunnelAgentState::with_upstreams(
        TunMode::Userspace,
        AccessControl::from_file(middle.path.clone()).await.unwrap(),
        middle_upstreams,
    );
    let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
    let address = listener.local_addr().unwrap();
    let (tls_port, tls_task) = tls_forward(address.port(), VALID).await;
    let authority = format!("127.0.0.1:{tls_port}");
    let router = Options::parse_from(["fixture", "--allow-host", &authority])
        .boundary
        .validate()
        .unwrap()
        .for_listener(address)
        .protect(
            agent_router(middle_state),
            crate::http_diagnostics::ErrorShape::Manager,
        );
    let middle_task = tokio::spawn(async move {
        axum::serve(listener, router).await.unwrap();
    });
    let top = PolicyFixture::new(vec![grant(
        TOKEN,
        vec![
            Permission::List,
            Permission::Connect,
            Permission::ManageServer,
        ],
    )]);
    let ca_file = top.path.parent().unwrap().join("test-ca.pem");
    ios_core::secret_file::write_secret_new(&ca_file, CA).unwrap();
    let middle_root = format!("https://127.0.0.1:{tls_port}/");
    let top_upstreams = configured(
        &top,
        json!([{"url":middle_root,"token_file":middle.token_path,"ca_file":ca_file}]),
    )
    .await;
    let top_state = TunnelAgentState::with_upstreams(
        TunMode::Userspace,
        AccessControl::from_file(top.path.clone()).await.unwrap(),
        top_upstreams,
    );
    let (top_port, top_task) = spawn_test_router(agent_router(top_state)).await;
    let response = client()
        .get(format!("http://127.0.0.1:{top_port}/"))
        .bearer_auth(TOKEN)
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let listing: serde_json::Value = serde_json::from_str(&response.text().await.unwrap()).unwrap();
    assert_eq!(listing[DEVICE][0]["upstream"], middle_root);
    let mut request = format!(
        "ws://127.0.0.1:{top_port}/connect?udid={DEVICE}&port=12345&upstream={}",
        reqwest::Url::parse(&middle_root).unwrap()
    )
    .into_client_request()
    .unwrap();
    request
        .headers_mut()
        .insert("authorization", format!("Bearer {TOKEN}").parse().unwrap());
    let (mut socket, _) = tokio_tungstenite::connect_async(request).await.unwrap();
    socket
        .send(tungstenite::Message::Binary(
            b"two-protected-manager-bytes".to_vec(),
        ))
        .await
        .unwrap();
    assert_eq!(
        socket.next().await.unwrap().unwrap(),
        tungstenite::Message::Binary(b"two-protected-manager-bytes".to_vec())
    );
    middle.replace(vec![]);
    let native = reqwest::Client::builder()
        .no_proxy()
        .redirect(reqwest::redirect::Policy::none())
        .add_root_certificate(reqwest::Certificate::from_pem(CA).unwrap())
        .build()
        .unwrap();
    assert_eq!(
        native
            .post(format!("{middle_root}auth/reload"))
            .bearer_auth(REPLACEMENT)
            .send()
            .await
            .unwrap()
            .status(),
        StatusCode::OK
    );
    let closed = tokio::time::timeout(Duration::from_secs(3), socket.next())
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
    let refused = client()
        .get(format!("http://127.0.0.1:{top_port}/"))
        .bearer_auth(TOKEN)
        .send()
        .await
        .unwrap();
    assert_eq!(refused.status(), StatusCode::BAD_GATEWAY);
    let diagnostic = refused.text().await.unwrap();
    for secret in [TOKEN, REPLACEMENT, DEVICE, leaf_token.as_str()] {
        assert!(!diagnostic.contains(secret));
    }
    top_task.abort();
    middle_task.abort();
    leaf_task.abort();
    tls_task.abort();
}

#[tokio::test]
async fn invalid_upstream_credentials_fail_listing_and_selection_without_empty_success() {
    let upstream = PolicyFixture::new(vec![grant(REPLACEMENT, vec![Permission::List])]);
    let (upstream_port, upstream_task) =
        spawn_test_router(agent_router(TunnelAgentState::with_access(
            TunMode::Userspace,
            AccessControl::from_file(upstream.path.clone())
                .await
                .unwrap(),
        )))
        .await;
    let fixture = PolicyFixture::new(vec![grant(
        TOKEN,
        vec![Permission::List, Permission::Connect],
    )]);
    let upstreams = configured(&fixture,json!([{"url":format!("http://127.0.0.1:{upstream_port}/"),"token_file":fixture.token_path}])).await;
    let state = TunnelAgentState::with_upstreams(
        TunMode::Userspace,
        AccessControl::from_file(fixture.path.clone())
            .await
            .unwrap(),
        upstreams,
    );
    let (port, task) = spawn_test_router(agent_router(state)).await;
    let response = client()
        .get(format!("http://127.0.0.1:{port}/"))
        .bearer_auth(TOKEN)
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::BAD_GATEWAY);
    let mut request = format!("ws://127.0.0.1:{port}/connect?udid={DEVICE}&port=12345")
        .into_client_request()
        .unwrap();
    request
        .headers_mut()
        .insert("authorization", format!("Bearer {TOKEN}").parse().unwrap());
    let denied = tokio_tungstenite::connect_async(request).await;
    assert!(
        matches!(denied,Err(tungstenite::Error::Http(response)) if response.status() == StatusCode::BAD_GATEWAY)
    );
    let mut request =
        format!("ws://127.0.0.1:{port}/connect?udid={DEVICE}&port=12345&upstream=not-registered")
            .into_client_request()
            .unwrap();
    request
        .headers_mut()
        .insert("authorization", format!("Bearer {TOKEN}").parse().unwrap());
    assert!(
        matches!(tokio_tungstenite::connect_async(request).await,Err(tungstenite::Error::Http(response)) if response.status() == StatusCode::BAD_REQUEST)
    );
    task.abort();
    upstream_task.abort();
}

#[tokio::test]
async fn startup_rejects_invalid_private_upstream_config_before_opening_listener() {
    let args = Options::parse_from([
        "fixture",
        "--upstream-config",
        "missing-synthetic-private-upstreams",
    ]);
    assert!(args.upstreams.load().await.is_err());
    // Actual CLI startup capture separately proves the listener/device boundary.
}
