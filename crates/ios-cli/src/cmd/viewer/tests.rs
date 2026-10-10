//! Unit test suite for Web viewer, VNC server, and bidirectional clipboard (F05).

use std::sync::Arc;
use std::time::Duration;

use reqwest::StatusCode;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::oneshot;

use super::clipboard::*;
use super::vnc::*;
use super::web::*;
use crate::access::test_support::{grant, JsonRequest, PolicyFixture, TOKEN};
use crate::access::{AccessControl, Permission};

fn test_client() -> reqwest::Client {
    reqwest::Client::builder()
        .no_proxy()
        .timeout(Duration::from_secs(5))
        .build()
        .unwrap()
}

async fn spawn_test_server(
    state: Arc<WebViewerState>,
) -> (String, tokio::task::JoinHandle<()>, oneshot::Sender<()>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let (tx, rx) = oneshot::channel();
    let app = create_viewer_router(state);
    let handle = tokio::spawn(async move {
        axum::serve(listener, app)
            .with_graceful_shutdown(async move {
                let _ = rx.await;
            })
            .await
            .unwrap();
    });
    (format!("http://127.0.0.1:{port}"), handle, tx)
}

#[tokio::test]
async fn web_viewer_index_and_stream_lifecycle() {
    let fixture = PolicyFixture::new(vec![grant(
        TOKEN,
        vec![Permission::Debug, Permission::Connect],
    )]);
    let access = AccessControl::from_file(fixture.path.clone())
        .await
        .unwrap();
    let state = WebViewerState::new(access);
    let (base, handle, shutdown) = spawn_test_server(state.clone()).await;
    let client = test_client();

    // 1. GET / returns HTML with token
    let resp = client
        .get(format!("{base}/"))
        .bearer_auth(TOKEN)
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body = resp.text().await.unwrap();
    assert!(body.starts_with("<!DOCTYPE html>"));

    // 2. Push frame
    let fake_png = vec![0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a];
    state
        .push_frame("image/png", fake_png.clone(), 1170, 2532, 1000)
        .await;
    {
        let guard = state.frame.read().await;
        assert!(guard.is_some());
        assert_eq!(guard.as_ref().unwrap().width, 1170);
    }

    // 3. POST /stop terminates stream
    let stop_resp = client
        .post(format!("{base}/stop"))
        .bearer_auth(TOKEN)
        .send()
        .await
        .unwrap();
    assert_eq!(stop_resp.status(), StatusCode::OK);
    assert!(state.is_stopped());

    let _ = shutdown.send(());
    let _ = handle.await;
}

#[tokio::test]
async fn web_viewer_auth_rejection() {
    let fixture = PolicyFixture::new(vec![grant(TOKEN, vec![Permission::Debug])]);
    let access = AccessControl::from_file(fixture.path.clone())
        .await
        .unwrap();
    let state = WebViewerState::new(access);
    let (base, handle, shutdown) = spawn_test_server(state.clone()).await;
    let client = test_client();

    // 1. Request without token is rejected
    let resp = client.get(format!("{base}/")).send().await.unwrap();
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);

    // 2. Request with invalid token is rejected
    let bad_resp = client
        .get(format!("{base}/"))
        .bearer_auth("invalid-token-1234")
        .send()
        .await
        .unwrap();
    assert_eq!(bad_resp.status(), StatusCode::UNAUTHORIZED);

    // 3. Request with valid token is accepted
    let auth_resp = client
        .get(format!("{base}/"))
        .bearer_auth(TOKEN)
        .send()
        .await
        .unwrap();
    assert_eq!(auth_resp.status(), StatusCode::OK);

    let _ = shutdown.send(());
    let _ = handle.await;
}

#[tokio::test]
async fn web_viewer_input_endpoints_dispatch_actions() {
    let fixture = PolicyFixture::new(vec![grant(
        TOKEN,
        vec![Permission::Debug, Permission::Connect],
    )]);
    let access = AccessControl::from_file(fixture.path.clone())
        .await
        .unwrap();
    let state = WebViewerState::new(access);
    let (base, handle, shutdown) = spawn_test_server(state.clone()).await;
    let client = test_client();
    let mut rx = state.input_tx.subscribe();

    // 1. Pointer input
    let ptr_resp = client
        .post(format!("{base}/input/pointer"))
        .bearer_auth(TOKEN)
        .json(&serde_json::json!({
            "x": 100,
            "y": 200,
            "action": "down"
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(ptr_resp.status(), StatusCode::OK);

    let action = rx.recv().await.unwrap();
    assert_eq!(
        action,
        VncInputAction::Touch {
            contact: true,
            x: 100,
            y: 200
        }
    );

    // 2. Button input (Home)
    let btn_resp = client
        .post(format!("{base}/input/button"))
        .bearer_auth(TOKEN)
        .json(&serde_json::json!({
            "name": "home"
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(btn_resp.status(), StatusCode::OK);

    let down_act = rx.recv().await.unwrap();
    assert_eq!(
        down_act,
        VncInputAction::Button {
            page: BTN_HOME.0,
            code: BTN_HOME.1,
            down: true
        }
    );
    let up_act = rx.recv().await.unwrap();
    assert_eq!(
        up_act,
        VncInputAction::Button {
            page: BTN_HOME.0,
            code: BTN_HOME.1,
            down: false
        }
    );

    // 3. Orientation input
    let ori_resp = client
        .post(format!("{base}/orientation"))
        .bearer_auth(TOKEN)
        .json(&serde_json::json!({
            "direction": "left"
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(ori_resp.status(), StatusCode::OK);

    let ori_act = rx.recv().await.unwrap();
    assert_eq!(ori_act, VncInputAction::Rotate { direction: "left" });

    let _ = shutdown.send(());
    let _ = handle.await;
}

#[tokio::test]
async fn vnc_server_handshake_and_pointer_cleanup() {
    let state = VncServerState::new(100, 200, None);
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();

    let state_clone = state.clone();
    let server_task = tokio::spawn(async move {
        let (stream, peer) = listener.accept().await.unwrap();
        let session = VncSession::new(stream, peer, state_clone);
        let _ = session.run().await;
    });

    let mut client = TcpStream::connect(addr).await.unwrap();

    // 1. Read ProtocolVersion
    let mut version = [0u8; 12];
    client.read_exact(&mut version).await.unwrap();
    assert_eq!(&version, RFB_VERSION_3_8);

    // Send client version
    client.write_all(RFB_VERSION_3_8).await.unwrap();

    // 2. Read Security types
    let count = client.read_u8().await.unwrap();
    assert_eq!(count, 1);
    let sec_type = client.read_u8().await.unwrap();
    assert_eq!(sec_type, RFB_SECURITY_NONE);

    // Choose security None
    client.write_all(&[RFB_SECURITY_NONE]).await.unwrap();

    // Read SecurityResult (0 = OK)
    let sec_result = client.read_u32().await.unwrap();
    assert_eq!(sec_result, 0);

    // 3. Send ClientInit (shared=1)
    client.write_all(&[1]).await.unwrap();

    // 4. Read ServerInit
    let width = client.read_u16().await.unwrap();
    let height = client.read_u16().await.unwrap();
    assert_eq!(width, 100);
    assert_eq!(height, 200);

    let mut pixel_format = [0u8; 16];
    client.read_exact(&mut pixel_format).await.unwrap();
    let name_len = client.read_u32().await.unwrap() as usize;
    let mut name = vec![0u8; name_len];
    client.read_exact(&mut name).await.unwrap();
    assert_eq!(&name, b"iOS Screen");

    let mut rx = state.input_tx.subscribe();

    // 5. Send PointerEvent down (left click bit 0)
    // msg_type=5, button_mask=1, x=50, y=80
    client
        .write_all(&[RFB_CLIENT_POINTER_EVENT, 1, 0, 50, 0, 80])
        .await
        .unwrap();

    let act1 = rx.recv().await.unwrap();
    assert_eq!(
        act1,
        VncInputAction::Touch {
            contact: true,
            x: 50,
            y: 80
        }
    );

    // 6. Abruptly drop client -> verify disconnect cleanup releases contact!
    drop(client);
    let act_cleanup = rx.recv().await.unwrap();
    assert_eq!(
        act_cleanup,
        VncInputAction::Touch {
            contact: false,
            x: 50,
            y: 80
        }
    );

    state.stop();
    let _ = server_task.await;
}

#[tokio::test]
async fn vnc_server_hotkeys_and_buttons_mapping() {
    let state = VncServerState::new(100, 200, None);
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();

    let state_clone = state.clone();
    tokio::spawn(async move {
        let (stream, peer) = listener.accept().await.unwrap();
        let session = VncSession::new(stream, peer, state_clone);
        let _ = session.run().await;
    });

    let mut client = TcpStream::connect(addr).await.unwrap();
    let mut ver = [0u8; 12];
    client.read_exact(&mut ver).await.unwrap();
    client.write_all(RFB_VERSION_3_8).await.unwrap();
    let _ = client.read_u8().await.unwrap();
    let _ = client.read_u8().await.unwrap();
    client.write_all(&[RFB_SECURITY_NONE]).await.unwrap();
    assert_eq!(client.read_u32().await.unwrap(), 0);
    client.write_all(&[1]).await.unwrap();
    // Consume ServerInit
    let mut init_buf = vec![0u8; 4 + 16 + 4 + 10];
    client.read_exact(&mut init_buf).await.unwrap();

    let mut rx = state.input_tx.subscribe();

    // 1. Send Ctrl down (keysym 0xFFE3)
    // KeyEvent: type=4, down=1, pad=0,0, keysym=0x0000FFE3
    client
        .write_all(&[RFB_CLIENT_KEY_EVENT, 1, 0, 0, 0, 0, 0xFF, 0xE3])
        .await
        .unwrap();

    // 2. Send 'h' down (keysym 0x68) while Ctrl is held -> Home button trigger!
    client
        .write_all(&[RFB_CLIENT_KEY_EVENT, 1, 0, 0, 0, 0, 0, 0x68])
        .await
        .unwrap();

    let act_down = rx.recv().await.unwrap();
    assert_eq!(
        act_down,
        VncInputAction::Button {
            page: BTN_HOME.0,
            code: BTN_HOME.1,
            down: true
        }
    );
    let act_up = rx.recv().await.unwrap();
    assert_eq!(
        act_up,
        VncInputAction::Button {
            page: BTN_HOME.0,
            code: BTN_HOME.1,
            down: false
        }
    );

    // 3. Send Middle click (button_mask bit 1 = 2) -> Lock button trigger!
    // PointerEvent: type=5, mask=2, x=0, y=0
    client
        .write_all(&[RFB_CLIENT_POINTER_EVENT, 2, 0, 0, 0, 0])
        .await
        .unwrap();
    let lock_down = rx.recv().await.unwrap();
    assert_eq!(
        lock_down,
        VncInputAction::Button {
            page: BTN_LOCK.0,
            code: BTN_LOCK.1,
            down: true
        }
    );

    // Release middle click
    client
        .write_all(&[RFB_CLIENT_POINTER_EVENT, 0, 0, 0, 0, 0])
        .await
        .unwrap();
    let lock_up = rx.recv().await.unwrap();
    assert_eq!(
        lock_up,
        VncInputAction::Button {
            page: BTN_LOCK.0,
            code: BTN_LOCK.1,
            down: false
        }
    );

    state.stop();
}

#[tokio::test]
async fn clipboard_bounds_and_loop_suppression() {
    let mut state = ClipboardSyncState::new();

    let text_payload = ClipboardPayload::Text("Hello iOS".to_string());
    text_payload.validate_bounds().unwrap();

    // 1. First sync from Host -> allowed
    assert!(state.should_sync(&text_payload, ClipboardSource::Host));
    assert_eq!(state.sync_count(), 1);
    assert_eq!(state.suppression_count(), 0);

    // 2. Immediate echo from Device with identical text -> suppressed!
    assert!(!state.should_sync(&text_payload, ClipboardSource::Device));
    assert_eq!(state.sync_count(), 1);
    assert_eq!(state.suppression_count(), 1);

    // 3. Duplicate notification from Host -> suppressed!
    assert!(!state.should_sync(&text_payload, ClipboardSource::Host));
    assert_eq!(state.sync_count(), 1);

    // 4. Fresh text from Device -> allowed!
    let new_dev_payload = ClipboardPayload::Text("New text from iPhone".to_string());
    assert!(state.should_sync(&new_dev_payload, ClipboardSource::Device));
    assert_eq!(state.sync_count(), 2);

    // 5. Echo back from Host -> suppressed!
    assert!(!state.should_sync(&new_dev_payload, ClipboardSource::Host));
    assert_eq!(state.suppression_count(), 2);

    // 6. Test bounds violation
    let oversized_text = "x".repeat(MAX_TEXT_BYTES + 1);
    let oversized_payload = ClipboardPayload::Text(oversized_text);
    assert!(oversized_payload.validate_bounds().is_err());

    let oversized_png = vec![0u8; MAX_IMAGE_BYTES + 1];
    let oversized_img = ClipboardPayload::ImagePng(oversized_png);
    assert!(oversized_img.validate_bounds().is_err());
}

#[test]
fn extended_clipboard_compression_round_trip() {
    let original = "Testing Extended Clipboard with \r\n CRLF and special chars!";
    let encoded = encode_extended_provide_text(original).unwrap();
    assert_eq!(encoded[0], SERVER_CUT_TEXT);

    // Skip 1 byte msg_type + 3 bytes pad
    let len_bytes: [u8; 4] = encoded[4..8].try_into().unwrap();
    let length = i32::from_be_bytes(len_bytes);
    assert!(length < 0, "extended clipboard length must be negative");

    let body = &encoded[8..];
    let parsed = parse_extended_cut_text(body).unwrap();
    assert_eq!(
        parsed.text.as_deref(),
        Some("Testing Extended Clipboard with \n CRLF and special chars!")
    );
}
