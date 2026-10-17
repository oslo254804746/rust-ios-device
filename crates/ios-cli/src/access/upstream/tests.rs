use super::*;
use crate::access::test_support::{grant, PolicyFixture, TOKEN};
use crate::access::Permission;
use axum::http::StatusCode;
use axum::routing::get;
use axum::{Json, Router};
use serde_json::json;
use std::sync::atomic::{AtomicUsize, Ordering};

#[test]
fn upstream_client_root_moves_original_string_and_clones_share_its_last_owner() {
    let root = String::from("https://original-client-root.invalid/");
    let pointer = root.as_ptr();
    let client = UpstreamClient::new(root, None, None).unwrap();
    assert_eq!(client.root.as_str().as_ptr(), pointer);
    let cloned = client.clone();
    assert!(Arc::ptr_eq(&client.root, &cloned.root));
    drop(client);
    assert_eq!(cloned.root.as_str().as_ptr(), pointer);
    assert_eq!(
        cloned.root.as_str(),
        "https://original-client-root.invalid/"
    );
}

#[test]
fn upstream_client_root_exact_lookup_keeps_original_limit_after_idna_expansion() {
    // Locked url2.5.4/idna1.0.3 use DnsLength::Ignore. This fixed fixture
    // expands under the original raw-input length limit without network IO.
    let raw = format!("https://{}invalid/", "é.".repeat(300));
    assert!(raw.len() < MAX_URL_BYTES);
    let expanded = normalize_upstream_url(&raw).unwrap();
    assert!(expanded.len() > MAX_URL_BYTES);
    let fixture = PolicyFixture::new(vec![grant(TOKEN, vec![Permission::List])]);
    let registry = Upstreams::read(&fixture_file(
        &fixture,
        json!({
            "version":1,"upstreams":[{"url":raw,"token_file":fixture.token_path}]
        }),
    ))
    .unwrap();
    assert!(registry.0.contains_key(&expanded));
    assert_eq!(
        registry.client(&expanded).unwrap_err().to_string(),
        "upstream URL limit exceeded"
    );
    // Preserve the original raw-alias behavior too; only the supplied raw
    // URL length was limited, not the post-normalization representation.
    let alias = registry.client(&raw).unwrap();
    assert!(Arc::ptr_eq(
        &alias.root,
        &registry.0.get(&expanded).unwrap().root
    ));
}

#[test]
fn upstream_client_root_canonical_lookup_and_alias_keep_original_client_after_registry_drop() {
    let root = "https://expected.invalid/";
    let fixture = PolicyFixture::new(vec![grant(TOKEN, vec![Permission::List])]);
    let registry = Upstreams::read(&fixture_file(
        &fixture,
        json!({
            "version":1,"upstreams":[{"url":root,"token_file":fixture.token_path}]
        }),
    ))
    .unwrap();
    let original = registry.0.get(root).unwrap();
    let pointer = original.root.as_str().as_ptr();
    let canonical = registry.client(root).unwrap();
    let alias = registry.client("https://EXPECTED.invalid:443").unwrap();
    let second_registry = registry.clone();
    let next = second_registry.client(root).unwrap();
    assert!(Arc::ptr_eq(&canonical.root, &original.root));
    assert!(Arc::ptr_eq(&canonical.root, &alias.root));
    assert!(Arc::ptr_eq(&canonical.root, &next.root));
    assert!(Arc::ptr_eq(
        canonical.token.as_ref().unwrap(),
        alias.token.as_ref().unwrap()
    ));
    for invalid in [
        "https://expected.invalid/?token=synthetic",
        "https://expected.invalid/other",
        "https://user:secret@expected.invalid/",
        "http://expected.invalid/",
    ] {
        assert!(registry.client(invalid).is_err());
    }
    drop(registry);
    drop(second_registry);
    drop(canonical);
    drop(next);
    drop(fixture);
    assert_eq!(alias.root.as_str().as_ptr(), pointer);
    let header = alias
        .header(
            &reqwest::Url::parse("wss://expected.invalid/connect").unwrap(),
            ordinary_header,
        )
        .unwrap()
        .unwrap();
    assert_eq!(header, format!("Bearer {TOKEN}"));
    assert!(header.is_sensitive());
    assert!(!format!("{alias:?}").contains(root));
    assert!(!format!("{alias:?}").contains(TOKEN));
}

#[test]
fn upstream_header_builder_runs_after_origin_checks_and_retains_original_typed_error_pointer() {
    let client = UpstreamClient::new(
        "https://expected.invalid/".into(),
        Some(ClientToken(zeroize::Zeroizing::new(TOKEN.to_owned()))),
        None,
    )
    .unwrap();
    let calls = AtomicUsize::new(0);
    let mismatch = reqwest::Url::parse("https://different.invalid/connect").unwrap();
    let error = client
        .header(&mismatch, |_| {
            calls.fetch_add(1, Ordering::SeqCst);
            unreachable!("mismatched origin must not invoke header builder")
        })
        .unwrap_err();
    assert_eq!(calls.load(Ordering::SeqCst), 0);
    assert!(error.to_string().contains("target mismatch"));
    let url = reqwest::Url::parse("wss://expected.invalid/connect?hop=1").unwrap();
    let original = anyhow::Error::new(std::io::Error::new(
        std::io::ErrorKind::PermissionDenied,
        "original-builder-error",
    ));
    let pointer = original.downcast_ref::<std::io::Error>().unwrap() as *const _ as usize;
    let error = client.header(&url, |_| Err(original)).unwrap_err();
    assert_eq!(
        error.downcast_ref::<std::io::Error>().unwrap() as *const _ as usize,
        pointer
    );
    let anonymous = UpstreamClient::new("https://expected.invalid/".into(), None, None).unwrap();
    assert!(anonymous
        .header(&url, |_| unreachable!("no token needs no payload"))
        .unwrap()
        .is_none());
}

#[test]
fn upstream_normalization_rejects_plaintext_dns_remote_credentials_and_unsafe_urls() {
    for input in [
        "http://localhost:8000",
        "http://192.0.2.1",
        "http://example.invalid",
        "ftp://127.0.0.1",
        "http://127.0.0.1:0",
        "https://user:secret@example.invalid",
        "https://example.invalid/path",
        "https://example.invalid?token=secret",
        "https://example.invalid#secret",
    ] {
        assert!(normalize_upstream_url(input).is_err());
    }
    assert_eq!(
        normalize_upstream_url("https://Example.invalid:443").unwrap(),
        "https://example.invalid/"
    );
    assert_eq!(
        normalize_upstream_url("http://[::1]:8000").unwrap(),
        "http://[::1]:8000/"
    );
}

fn fixture_file(fixture: &PolicyFixture, value: serde_json::Value) -> PathBuf {
    let path = fixture
        .path
        .parent()
        .unwrap()
        .join(format!("upstreams-{}.json", uuid::Uuid::new_v4()));
    ios_core::secret_file::write_secret_new(&path, &serde_json::to_vec(&value).unwrap()).unwrap();
    path
}

#[test]
fn private_upstream_config_is_strict_bounded_and_debug_omits_credentials_urls_and_paths() {
    let fixture = PolicyFixture::new(vec![grant(TOKEN, vec![Permission::List])]);
    let configured =
        json!({"url":"https://synthetic-private-host.invalid/", "token_file":fixture.token_path});
    for invalid in [
        json!({"version":2,"upstreams":[]}),
        json!({"version":1,"upstreams":[],"unexpected":TOKEN}),
        json!({"version":1,"upstreams":vec![configured.clone();MAX_UPSTREAMS + 1]}),
        json!({"version":1,"upstreams":[configured.clone(),configured.clone()]}),
        json!({"version":1,"upstreams":[{"url":"https://example.invalid/"}]}),
        json!({"version":1,"upstreams":[{"url":"http://192.0.2.1/","token_file":fixture.token_path}]}),
        json!({"version":1,"upstreams":[{"url":"https://example.invalid/","token_file":"missing-private-token"}]}),
    ] {
        assert!(Upstreams::read(&fixture_file(&fixture, invalid)).is_err());
    }
    let valid = Upstreams::read(&fixture_file(
        &fixture,
        json!({"version":1,"upstreams":[configured]}),
    ))
    .unwrap();
    let client = valid
        .client("https://synthetic-private-host.invalid/")
        .unwrap();
    for rendered in [format!("{client:?}"), format!("{client:#?}")] {
        for value in [
            TOKEN,
            "synthetic-private-host",
            fixture.path.to_str().unwrap(),
        ] {
            assert!(!rendered.contains(value));
        }
    }
    let path = fixture_file(&fixture, json!({"version":1,"upstreams":[]}));
    ios_core::secret_file::write_secret(&path, &vec![b'x'; MAX_CONFIG_BYTES as usize + 1]).unwrap();
    assert!(Upstreams::read(&path).is_err());
}

#[test]
fn credentials_are_bound_to_one_normalized_authority_and_transport() {
    let fixture = PolicyFixture::new(vec![grant(TOKEN, vec![Permission::List])]);
    let config = fixture_file(
        &fixture,
        json!({"version":1,"upstreams":[{
        "url":"https://example.invalid:8443/","token_file":"client-token"}]}),
    );
    let clients = Upstreams::read(&config).unwrap();
    let configured = clients.client("https://example.invalid:8443/").unwrap();
    for url in [
        "https://example.invalid:8443/?hop=1",
        "wss://example.invalid:8443/connect?udid=owned&port=12345&hop=1",
    ] {
        let header = configured
            .header(&reqwest::Url::parse(url).unwrap(), ordinary_header)
            .unwrap()
            .unwrap();
        assert!(header.is_sensitive());
        assert_eq!(header, format!("Bearer {TOKEN}"));
    }
    for url in [
        "https://other.invalid:8443/",
        "https://example.invalid:8444/",
        "ws://example.invalid:8443/connect",
        "http://example.invalid:8443/",
        "https://user:secret@example.invalid:8443/",
    ] {
        assert!(configured
            .header(&reqwest::Url::parse(url).unwrap(), ordinary_header)
            .is_err());
    }
    assert!(clients
        .client("https://other.invalid/")
        .unwrap()
        .header(
            &reqwest::Url::parse("https://other.invalid/").unwrap(),
            ordinary_header
        )
        .unwrap()
        .is_none());
}

async fn spawn(router: Router) -> (String, tokio::task::JoinHandle<()>) {
    let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
        .await
        .unwrap();
    let base = format!("http://{}/", listener.local_addr().unwrap());
    (
        base,
        tokio::spawn(async move {
            axum::serve(listener, router).await.unwrap();
        }),
    )
}

#[tokio::test]
async fn http_credentials_are_per_upstream_and_redirects_never_receive_them() {
    let redirected = Arc::new(AtomicUsize::new(0));
    let observed = redirected.clone();
    let (destination, other) = spawn(Router::new().route(
        "/",
        get(move || {
            let observed = observed.clone();
            async move {
                observed.fetch_add(1, Ordering::SeqCst);
                Json(json!({}))
            }
        }),
    ))
    .await;
    let next = destination.clone();
    let ws_next = destination.clone();
    let (source, server) = spawn(
        Router::new()
            .route(
                "/",
                get(move |headers: axum::http::HeaderMap| {
                    let next = next.clone();
                    async move {
                        assert_eq!(headers["authorization"], format!("Bearer {TOKEN}"));
                        (StatusCode::TEMPORARY_REDIRECT, [("location", next)], "")
                    }
                }),
            )
            .route(
                "/connect",
                get(move |headers: axum::http::HeaderMap| {
                    let next = ws_next.clone();
                    async move {
                        assert_eq!(headers["authorization"], format!("Bearer {TOKEN}"));
                        (StatusCode::TEMPORARY_REDIRECT, [("location", next)], "")
                    }
                }),
            ),
    )
    .await;
    let fixture = PolicyFixture::new(vec![grant(TOKEN, vec![Permission::List])]);
    let clients = Upstreams::read(&fixture_file(
        &fixture,
        json!({"version":1,"upstreams":[{"url":source,"token_file":fixture.token_path}]}),
    ))
    .unwrap();
    assert!(clients
        .client(&source)
        .unwrap()
        .listing(1, 1024)
        .await
        .is_err());
    assert_eq!(redirected.load(Ordering::SeqCst), 0);
    let client = clients.client(&source).unwrap();
    let url =
        reqwest::Url::parse(&format!("{}connect", source.replacen("http:", "ws:", 1))).unwrap();
    let request = client.websocket_request(&url, ordinary_header).unwrap();
    let refused = tokio_tungstenite::connect_async_tls_with_config(
        request,
        None,
        false,
        Some(client.connector()),
    )
    .await;
    assert!(
        matches!(refused, Err(tokio_tungstenite::tungstenite::Error::Http(response)) if response.status() == StatusCode::TEMPORARY_REDIRECT)
    );
    assert_eq!(redirected.load(Ordering::SeqCst), 0);
    assert_eq!(
        clients
            .client(&destination)
            .unwrap()
            .listing(1, 1024)
            .await
            .unwrap(),
        json!({})
    );
    assert_eq!(redirected.load(Ordering::SeqCst), 1);
    server.abort();
    other.abort();
}

#[tokio::test]
async fn rejected_credentials_and_oversized_listings_are_explicit_errors() {
    for status in [
        StatusCode::UNAUTHORIZED,
        StatusCode::FORBIDDEN,
        StatusCode::BAD_GATEWAY,
    ] {
        let (url, server) = spawn(Router::new().route(
            "/",
            get(move || async move { (status, "synthetic-secret-body") }),
        ))
        .await;
        let failure = Upstreams::default()
            .client(&url)
            .unwrap()
            .listing(1, 1024)
            .await
            .unwrap_err();
        assert!(!format!("{failure:#}").contains("synthetic-secret-body"));
        server.abort();
    }
    let (url, server) = spawn(Router::new().route("/", get(|| async { "x".repeat(1025) }))).await;
    assert!(Upstreams::default()
        .client(&url)
        .unwrap()
        .listing(1, 1024)
        .await
        .is_err());
    server.abort();
}

#[tokio::test]
async fn https_and_wss_verify_ca_expiry_and_hostname_before_sending_http_credentials() {
    use crate::access::test_support::tls_forward;
    use crate::access::test_tls_material::{CA, EXPIRED, VALID, WRONG_NAME};
    use axum::extract::ws::WebSocketUpgrade;
    use futures_util::{SinkExt, StreamExt};
    let requests = Arc::new(AtomicUsize::new(0));
    let observed = requests.clone();
    let fixture = PolicyFixture::new(vec![grant(TOKEN, vec![Permission::List])]);
    let ca = fixture.path.parent().unwrap().join("test-ca.pem");
    ios_core::secret_file::write_secret_new(&ca, CA).unwrap();
    let router = Router::new()
        .route(
            "/",
            get(move |headers: axum::http::HeaderMap| {
                let observed = observed.clone();
                async move {
                    observed.fetch_add(1, Ordering::SeqCst);
                    assert_eq!(headers["authorization"], format!("Bearer {TOKEN}"));
                    Json(json!({}))
                }
            }),
        )
        .route(
            "/connect",
            get(
                |headers: axum::http::HeaderMap, ws: WebSocketUpgrade| async move {
                    assert_eq!(headers["authorization"], format!("Bearer {TOKEN}"));
                    ws.on_upgrade(|mut socket| async move {
                        if let Some(Ok(message)) = socket.next().await {
                            socket.send(message).await.unwrap();
                        }
                    })
                },
            ),
        );
    let (http, router_task) = spawn(router).await;
    let port = reqwest::Url::parse(&http).unwrap().port().unwrap();
    for (certificate, trusted, passed) in [
        (VALID, true, true),
        (VALID, false, false),
        (WRONG_NAME, true, false),
        (EXPIRED, true, false),
    ] {
        let (tls_port, tls_task) = tls_forward(port, certificate).await;
        let root = format!("https://127.0.0.1:{tls_port}/");
        let mut configured = json!({"url":root,"token_file":fixture.token_path});
        if trusted {
            configured["ca_file"] = json!(ca);
        }
        let clients = Upstreams::read(&fixture_file(
            &fixture,
            json!({"version":1,"upstreams":[configured]}),
        ))
        .unwrap();
        let client = clients.client(&root).unwrap();
        let before = requests.load(Ordering::SeqCst);
        assert_eq!(client.listing(1, 1024).await.is_ok(), passed);
        assert_eq!(
            requests.load(Ordering::SeqCst),
            before + usize::from(passed)
        );
        let url = reqwest::Url::parse(&format!("wss://127.0.0.1:{tls_port}/connect")).unwrap();
        let request = client.websocket_request(&url, ordinary_header).unwrap();
        let connection = tokio_tungstenite::connect_async_tls_with_config(
            request,
            None,
            false,
            Some(client.connector()),
        )
        .await;
        assert_eq!(connection.is_ok(), passed);
        if let Ok((mut socket, _)) = connection {
            use tokio_tungstenite::tungstenite::Message;
            socket
                .send(Message::Binary(b"verified-wss-bytes".to_vec()))
                .await
                .unwrap();
            assert_eq!(
                socket.next().await.unwrap().unwrap(),
                Message::Binary(b"verified-wss-bytes".to_vec())
            );
        }
        tls_task.abort();
        let _ = tls_task.await;
    }
    let (first, first_task) = tls_forward(port, VALID).await;
    let (second, second_task) = tls_forward(port, VALID).await;
    let first_root = format!("https://127.0.0.1:{first}/");
    let second_root = format!("https://127.0.0.1:{second}/");
    let isolated = Upstreams::read(&fixture_file(
        &fixture,
        json!({"version":1,"upstreams":[
            {"url":first_root,"token_file":fixture.token_path,"ca_file":ca},
            {"url":second_root,"token_file":fixture.token_path}
        ]}),
    ))
    .unwrap();
    assert!(isolated
        .client(&first_root)
        .unwrap()
        .listing(1, 1024)
        .await
        .is_ok());
    let before = requests.load(Ordering::SeqCst);
    assert!(isolated
        .client(&second_root)
        .unwrap()
        .listing(1, 1024)
        .await
        .is_err());
    assert_eq!(requests.load(Ordering::SeqCst), before);
    first_task.abort();
    second_task.abort();
    router_task.abort();
}
