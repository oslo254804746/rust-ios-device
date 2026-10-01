//! Explicit browser gate against the production manager and owned TCP fixtures.

use super::*;
use crate::access::test_support::{grant, PolicyFixture, DEVICE, TOKEN};
use std::process::Stdio;

#[cfg(unix)]
struct OwnedProcessGroup(u32);

#[cfg(unix)]
impl Drop for OwnedProcessGroup {
    fn drop(&mut self) {
        // This process group was created solely for the owned fixture. It also
        // contains Chromium descendants when Node fails or its deadline fires.
        let _ = std::process::Command::new("kill")
            .args(["-KILL", "--", &format!("-{}", self.0)])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
    }
}

#[derive(clap::Parser)]
struct BoundaryOptions {
    #[command(flatten)]
    boundary: crate::access::HttpBoundaryArgs,
}

#[tokio::test]
#[ignore = "requires Playwright and Chromium; run the documented S02 browser gate explicitly"]
async fn browser_ticket_probe_uses_production_manager() {
    let node = std::env::var_os("IOS_BROWSER_NODE").expect("IOS_BROWSER_NODE is required");
    let chromium =
        std::env::var_os("IOS_BROWSER_EXECUTABLE").expect("IOS_BROWSER_EXECUTABLE is required");
    for expiry in [false, true] {
        browser_fixture(&node, &chromium, expiry).await;
    }
}

async fn ui() -> (String, tokio::task::JoinHandle<()>) {
    let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
    let address = listener.local_addr().unwrap();
    let router = Router::new().route(
        "/",
        get(|| async {
            (
                [("cache-control", "no-store"), ("content-type", "text/html")],
                "<!doctype html><title>Owned S02 host fixture</title>",
            )
        }),
    );
    (
        format!("http://{address}"),
        tokio::spawn(async move {
            axum::serve(listener, router).await.unwrap();
        }),
    )
}

async fn browser_fixture(node: &std::ffi::OsStr, chromium: &std::ffi::OsStr, expiry: bool) {
    let (origin, ui_task) = ui().await;
    let (unapproved_origin, other_ui_task) = ui().await;
    let proxy = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
    let proxy_port = proxy.local_addr().unwrap().port();
    let mut proxy_task = tokio::spawn(async move {
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
        let mut payload = [0; 64];
        let count = stream.read(&mut payload).await.unwrap();
        assert_eq!(&payload[..count], b"owned-browser-bytes");
        stream.write_all(&payload[..count]).await.unwrap();
        assert_eq!(stream.read(&mut payload).await.unwrap(), 0);
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
                move |headers: axum::http::HeaderMap,
                      Query(query): Query<ConnectQuery>,
                      ws: WebSocketUpgrade| async move {
                    assert!(!headers.contains_key("authorization"));
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
    let scope = grant(TOKEN, vec![Permission::Connect, Permission::ManageServer]);
    let fixture = PolicyFixture::new(vec![if expiry {
        scope.expires_after(15)
    } else {
        scope
    }]);
    let state = TunnelAgentState::with_access(
        TunMode::Userspace,
        AccessControl::from_file(fixture.path.clone())
            .await
            .unwrap(),
    );
    state
        .inner
        .upstreams
        .write()
        .await
        .insert(format!("http://127.0.0.1:{upstream_port}/"));
    let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
    let address = listener.local_addr().unwrap();
    let boundary = BoundaryOptions::parse_from(["fixture", "--allow-origin", &origin]).boundary;
    let router = boundary.validate().unwrap().for_listener(address).protect(
        agent_router(state),
        crate::http_diagnostics::ErrorShape::Manager,
    );
    let manager_task = tokio::spawn(async move {
        axum::serve(listener, router).await.unwrap();
    });
    let mut command = tokio::process::Command::new(node);
    command
        .arg(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../scripts/browser-ticket-probe.cjs"
        ))
        .env_remove("DEBUG")
        .env_remove("PWDEBUG")
        .env_remove("NODE_DEBUG")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    #[cfg(unix)]
    command.process_group(0);
    let mut child = command.spawn().expect("spawn owned browser probe");
    #[cfg(unix)]
    let _group = OwnedProcessGroup(child.id().expect("owned browser PID"));
    let config = serde_json::to_vec(&json!({
        "fixture": "s02-owned-host-fixture-v1", "base": format!("http://{address}"),
        "origin": origin, "unapproved_origin": unapproved_origin,
        "token_file": fixture.token_path, "policy_file": fixture.path,
        "executable": chromium.to_str().expect("UTF-8 browser executable path"), "expiry": expiry,
    }))
    .unwrap();
    let mut stdin = child.stdin.take().unwrap();
    stdin.write_all(&config).await.unwrap();
    drop(stdin);
    let mut stdout = child.stdout.take().unwrap().take(64 * 1024 + 1);
    let mut stderr = child.stderr.take().unwrap().take(64 * 1024 + 1);
    // Read bounded raw pipes without persisting URLs, credentials or raw errors.
    let result = tokio::time::timeout(Duration::from_secs(35), async {
        let mut out = Vec::new();
        let mut err = Vec::new();
        let (out_result, err_result, status) = tokio::join!(
            stdout.read_to_end(&mut out),
            stderr.read_to_end(&mut err),
            child.wait()
        );
        out_result.unwrap();
        err_result.unwrap();
        (out, err, status.unwrap())
    })
    .await;
    let passed = match result {
        Ok((out, err, status)) => {
            assert!(
                out.len() <= 64 * 1024 && err.len() <= 64 * 1024,
                "browser raw capture exceeded its bound"
            );
            for secret in [TOKEN, DEVICE] {
                assert!(
                    !out.windows(secret.len())
                        .any(|value| value == secret.as_bytes()),
                    "browser stdout raw leakage"
                );
                assert!(
                    !err.windows(secret.len())
                        .any(|value| value == secret.as_bytes()),
                    "browser stderr raw leakage"
                );
            }
            let opaque_secret = regex::Regex::new("[0-9a-f]{64}").unwrap();
            assert!(
                !opaque_secret.is_match(&String::from_utf8_lossy(&out)),
                "browser stdout opaque ticket leakage"
            );
            assert!(
                !opaque_secret.is_match(&String::from_utf8_lossy(&err)),
                "browser stderr opaque ticket leakage"
            );
            if status.success() {
                let report: serde_json::Value =
                    serde_json::from_slice(&out).expect("fixed browser report");
                assert_eq!(report["status"], "passed");
                assert_eq!(report["checks"], 16);
                assert!(err.is_empty());
                println!("S02 browser fixture: {}", report);
                true
            } else {
                for stage in [
                    "prerequisite",
                    "bootstrap",
                    "unapproved-origin",
                    "expiry",
                    "reload",
                ] {
                    if err == format!("browser access probe failed: {stage}\n").as_bytes() {
                        println!("owned browser failure phase: {stage}");
                    }
                }
                false
            }
        }
        Err(_) => {
            let _ = child.start_kill();
            let _ = child.wait().await;
            false
        }
    };
    let proxy_closed = tokio::time::timeout(Duration::from_secs(2), &mut proxy_task).await;
    if proxy_closed.is_err() {
        proxy_task.abort();
    }
    manager_task.abort();
    upstream_task.abort();
    ui_task.abort();
    other_ui_task.abort();
    assert!(
        passed,
        "owned browser probe failed; raw diagnostics intentionally withheld"
    );
    proxy_closed
        .expect("owned proxy closed")
        .expect("owned proxy validated bytes and EOF");
}
