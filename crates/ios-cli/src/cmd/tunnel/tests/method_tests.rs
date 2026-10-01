use super::*;
use crate::access::test_support::{grant, PolicyFixture, TOKEN};

#[tokio::test]
async fn authenticated_legacy_get_is_readonly_and_post_shutdown_notifies() {
    let fixture = PolicyFixture::new(vec![grant(
        TOKEN,
        vec![
            Permission::List,
            Permission::ManageServer,
            Permission::ManageTunnels,
        ],
    )]);
    let state = TunnelAgentState::with_access(
        TunMode::Userspace,
        crate::access::AccessControl::from_file(fixture.path.clone())
            .await
            .unwrap(),
    );
    let shutdown = state.inner.shutdown.notified();
    tokio::pin!(shutdown);
    shutdown.as_mut().enable();
    let (port, server) = spawn_test_router(agent_router(state.clone())).await;
    let client = reqwest::Client::builder()
        .no_proxy()
        .timeout(Duration::from_secs(2))
        .build()
        .unwrap();
    let base = format!("http://127.0.0.1:{port}");
    for method in [reqwest::Method::GET, reqwest::Method::HEAD] {
        for path in [
            "/shutdown",
            "/start-tunnel?udid=synthetic-secret-device&connection_type=invalid-secret",
        ] {
            let response = client
                .request(method.clone(), format!("{base}{path}"))
                .bearer_auth(TOKEN)
                .send()
                .await
                .unwrap();
            assert!(matches!(
                response.status(),
                StatusCode::METHOD_NOT_ALLOWED | StatusCode::GONE
            ));
            assert_eq!(response.headers()["allow"], "POST");
            assert_eq!(response.headers()["cache-control"], "no-store");
            let original = response.text().await.unwrap();
            assert!(!original.contains("synthetic-secret"));
            assert!(!original.contains("invalid-secret"));
            if method == reqwest::Method::HEAD {
                assert!(original.is_empty());
            }
        }
    }
    assert!(
        tokio::time::timeout(Duration::from_millis(50), &mut shutdown)
            .await
            .is_err()
    );
    assert_eq!(
        client
            .get(format!("{base}/tunnels"))
            .bearer_auth(TOKEN)
            .send()
            .await
            .unwrap()
            .text()
            .await
            .unwrap(),
        "[]"
    );
    // A scoped POST must deny an ungranted device before discovery. A normal
    // GET only gave the finite migration notice and never parsed its selector.
    let response = client
        .post(format!("{base}/start-tunnel"))
        .bearer_auth(TOKEN)
        .header("content-type", "application/json")
        .body("{\"udid\":\"synthetic-other-device\",\"connection_type\":\"usb\"}")
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::FORBIDDEN);
    let response = client
        .post(format!("{base}/start-tunnel"))
        .bearer_auth(TOKEN)
        .header("content-type", "application/json")
        .body("{\"udid\":\"synthetic-secret\",\"connection_type\":\"invalid-secret\"}")
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
    assert!(!response.text().await.unwrap().contains("secret"));
    assert_eq!(
        client
            .post(format!("{base}/shutdown"))
            .bearer_auth(TOKEN)
            .send()
            .await
            .unwrap()
            .status(),
        StatusCode::OK
    );
    tokio::time::timeout(Duration::from_secs(1), shutdown)
        .await
        .unwrap();
    server.abort();
    let _ = server.await;
}

#[tokio::test]
async fn legacy_notice_has_a_fixed_end_and_never_advertises_get_mutation() {
    for (now, expected) in [
        (LEGACY_GET_NOTICE_UNTIL - 1, StatusCode::METHOD_NOT_ALLOWED),
        (LEGACY_GET_NOTICE_UNTIL, StatusCode::GONE),
        (u64::MAX, StatusCode::GONE),
    ] {
        let response = legacy_management_reply(now);
        assert_eq!(response.status(), expected);
        assert_eq!(response.headers()["allow"], "POST");
        let body = axum::body::to_bytes(response.into_body(), 1024)
            .await
            .unwrap();
        assert!(std::str::from_utf8(&body).unwrap().contains("POST"));
    }
}
