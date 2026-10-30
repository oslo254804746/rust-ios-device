//! Unit test suite for Web viewer, VNC server, and bidirectional clipboard (F05).

use std::sync::Arc;
use std::time::Duration;

use reqwest::StatusCode;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::{mpsc, oneshot};

use super::clipboard::*;
use super::media::*;
use super::vnc::*;
use super::web::*;
use crate::access::test_support::{grant, JsonRequest, PolicyFixture, DEVICE, TOKEN};
use crate::access::{AccessControl, Permission};

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "manual diagnostic only; measures unpolled viewer capture future sizes"]
async fn report_viewer_capture_future_sizes() {
    let udid = "size-probe-only";
    let native = crate::cmd::tunnel::NativeScope::new().unwrap();
    let (frames, _frame_rx) = tokio::sync::watch::channel(None::<DecodedFrame>);
    let (_stop_tx, stop_rx) = tokio::sync::watch::channel(false);
    macro_rules! report_size {
        ($label:literal, $future:expr) => {{
            let future = $future;
            eprintln!(
                "viewer-future-size {}={}",
                $label,
                std::mem::size_of_val(&future)
            );
        }};
    }
    report_size!(
        "version_probe",
        crate::cmd::connect::probe_product_version(udid)
    );
    report_size!(
        "native_connect",
        native.connect(udid, ios_core::TunMode::Userspace)
    );
    report_size!(
        "instruments_connect",
        crate::cmd::instruments::connect_instruments(udid)
    );
    report_size!(
        "legacy_capture",
        crate::cmd::screenshot::capture_fallback_scoped(udid, &native)
    );
    report_size!("capture_frame", capture_frame(udid, &native, false));
    report_size!(
        "timeout_capture_frame",
        tokio::time::timeout(
            Duration::from_secs(10),
            Box::pin(capture_frame(udid, &native, false)),
        )
    );
    report_size!(
        "produce",
        produce(
            udid.to_string(),
            frames,
            stop_rx,
            native.clone(),
            false,
            0,
            Duration::from_millis(500),
        )
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "manual diagnostic only; decodes the private PNG on a default Tokio worker"]
async fn viewer_png_decode_tokio_worker_stack_probe() {
    let path = std::env::var_os("IOS_PRIVATE_PNG_TEST")
        .map(std::path::PathBuf::from)
        .expect("set IOS_PRIVATE_PNG_TEST to the private captured PNG path");
    let data = std::fs::read(&path).expect("read IOS_PRIVATE_PNG_TEST");
    let input_bytes = data.len();
    let decoded = tokio::spawn(async move { decode_png(data.into(), 1) })
        .await
        .expect("PNG decoder task panicked");
    let frame = decoded.expect("PNG decode failed");
    eprintln!(
        "viewer-png-worker-decode input_bytes={} dimensions={}x{} output_pixel_bytes={}",
        input_bytes,
        frame.width,
        frame.height,
        frame.bgra.len(),
    );
}

#[test]
fn pre_ios17_viewer_capture_skips_coredevice_tunnel_path() {
    assert!(!coredevice_capture_supported(14));
    assert!(!coredevice_capture_supported(16));
    assert!(coredevice_capture_supported(17));
    assert!(coredevice_capture_supported(18));
}

#[test]
fn native_screenshot_above_video_unit_budget_still_decodes_with_pixel_bound() {
    let mut pixels = vec![0; 1024 * 768 * 4];
    let mut state = 1234567u32;
    for byte in &mut pixels {
        state ^= state << 13;
        state ^= state >> 17;
        state ^= state << 5;
        *byte = state as u8;
    }
    let mut data = Vec::new();
    {
        let mut encoder = png::Encoder::new(&mut data, 1024, 768);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        encoder
            .write_header()
            .unwrap()
            .write_image_data(&pixels)
            .unwrap();
    }
    assert!(data.len() > MAX_ENCODED_BYTES);
    let frame = decode_png(data.into(), 1).unwrap();
    assert_eq!((frame.width, frame.height), (1024, 768));
    assert_eq!(frame.bgra.len(), pixels.len());
    assert!(decode_png(vec![0; MAX_PNG_BYTES + 1].into(), 1).is_err());
}

fn observed_backend() -> (
    mpsc::Sender<DeviceRequest>,
    mpsc::UnboundedReceiver<VncInputAction>,
) {
    let (sender, mut receiver) = mpsc::channel::<DeviceRequest>(8);
    let (observe, observed) = mpsc::unbounded_channel();
    tokio::spawn(async move {
        while let Some(request) = receiver.recv().await {
            let value = match request.command {
                DeviceCommand::Input(action) => {
                    observe.send(action).unwrap();
                    None
                }
                DeviceCommand::ReadClipboard => Some(ClipboardPayload::Text("actual text".into())),
            };
            let _ = request.reply.send(Ok(value));
        }
    });
    (sender, observed)
}
async fn observe_web_backend(state: &WebViewerState) -> mpsc::UnboundedReceiver<VncInputAction> {
    let (sender, receiver) = observed_backend();
    *state.backend.write().await = Some(sender);
    receiver
}
async fn test_vnc_state(
    password: Option<String>,
) -> (Arc<VncServerState>, mpsc::UnboundedReceiver<VncInputAction>) {
    let principal = AccessControl::trusted_test_local()
        .authenticate(&axum::http::HeaderMap::new())
        .unwrap();
    let (backend, receiver) = observed_backend();
    let state = VncServerState::new(100, 200, password, principal, false, backend);
    state
        .update_frame(100, 200, vec![0; 100 * 200 * 4], 1)
        .await;
    (state, receiver)
}

/// Called by the real timestamped HEVC gate with its decoded frame, exercising
/// actual HTTP/RFB bytes through the same production states and handlers.
pub(super) async fn verify_decoded_video_endpoints(frame: DecodedFrame) {
    let fixture = PolicyFixture::new(vec![grant(
        TOKEN,
        vec![Permission::Debug, Permission::Connect],
    )]);
    let access = AccessControl::from_file(fixture.path.clone())
        .await
        .unwrap();
    let web = WebViewerState::new(access, DEVICE.into(), 12345, false);
    web.push_timed_frame(
        "image/png",
        frame.png.clone(),
        frame.width.into(),
        frame.height.into(),
        frame.timestamp_ms,
        frame.timestamp_clock,
    )
    .await;
    let (base, server, stop) = spawn_test_server(web).await;
    let response = test_client()
        .get(format!("{base}/frame"))
        .bearer_auth(TOKEN)
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(response.headers()["X-Timestamp-Clock"], "rtp-relative-ms");
    assert_eq!(response.bytes().await.unwrap(), frame.png);
    stop.send(()).unwrap();
    server.await.unwrap();

    let (vnc, _events) = test_vnc_state(None).await;
    vnc.update_decoded_frame(frame.clone()).await;
    assert!(Arc::ptr_eq(&vnc.frame.read().await.bgra_data, &frame.bgra));
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let state = vnc.clone();
    let server = tokio::spawn(async move {
        let (stream, peer) = listener.accept().await.unwrap();
        VncSession::new(stream, peer, state).run().await
    });
    let mut client = TcpStream::connect(address).await.unwrap();
    let mut version = [0; 12];
    client.read_exact(&mut version).await.unwrap();
    client.write_all(RFB_VERSION_3_8).await.unwrap();
    assert_eq!(client.read_u8().await.unwrap(), 1);
    assert_eq!(client.read_u8().await.unwrap(), RFB_SECURITY_NONE);
    client.write_u8(RFB_SECURITY_NONE).await.unwrap();
    assert_eq!(client.read_u32().await.unwrap(), 0);
    client.write_u8(1).await.unwrap();
    assert_eq!(client.read_u16().await.unwrap(), frame.width);
    assert_eq!(client.read_u16().await.unwrap(), frame.height);
    let mut format = [0; 16];
    client.read_exact(&mut format).await.unwrap();
    let name_len = client.read_u32().await.unwrap();
    assert!(name_len <= 128);
    let mut name = vec![0; name_len as usize];
    client.read_exact(&mut name).await.unwrap();
    client
        .write_all(&[RFB_CLIENT_FRAMEBUFFER_UPDATE_REQUEST, 0, 0, 0, 0, 0])
        .await
        .unwrap();
    client.write_u16(frame.width).await.unwrap();
    client.write_u16(frame.height).await.unwrap();
    let mut rectangle = [0; 16];
    client.read_exact(&mut rectangle).await.unwrap();
    assert_eq!(&rectangle[..4], &[RFB_SERVER_FRAMEBUFFER_UPDATE, 0, 0, 1]);
    assert_eq!(
        u16::from_be_bytes(rectangle[8..10].try_into().unwrap()),
        frame.width
    );
    let mut pixels = vec![0; frame.bgra.len()];
    client.read_exact(&mut pixels).await.unwrap();
    assert_eq!(&pixels, frame.bgra.as_ref());
    drop(client);
    vnc.stop();
    server.await.unwrap().unwrap();
}

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
    let state = WebViewerState::new(access, DEVICE.into(), 12345, true);
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
    let fixture = PolicyFixture::new(vec![grant(
        TOKEN,
        vec![Permission::Debug, Permission::Connect],
    )]);
    let access = AccessControl::from_file(fixture.path.clone())
        .await
        .unwrap();
    let state = WebViewerState::new(access, DEVICE.into(), 12345, true);
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
    let state = WebViewerState::new(access, DEVICE.into(), 12345, true);
    let (base, handle, shutdown) = spawn_test_server(state.clone()).await;
    let client = test_client();
    let mut rx = observe_web_backend(&state).await;

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
    assert_eq!(ori_resp.status(), StatusCode::NO_CONTENT);
    assert_eq!(
        rx.recv().await.unwrap(),
        VncInputAction::Rotate { direction: "left" }
    );
    let invalid = client
        .post(format!("{base}/orientation"))
        .bearer_auth(TOKEN)
        .json(&serde_json::json!({"direction":"unsupported"}))
        .send()
        .await
        .unwrap();
    assert_eq!(invalid.status(), StatusCode::BAD_REQUEST);
    assert!(
        rx.try_recv().is_err(),
        "invalid direction must not reach the device"
    );

    let _ = shutdown.send(());
    let _ = handle.await;
}

#[tokio::test]
async fn vnc_server_handshake_and_pointer_cleanup() {
    let (state, mut rx) = test_vnc_state(None).await;
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
    let (state, mut rx) = test_vnc_state(None).await;
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();

    let state_clone = state.clone();
    let server_task = tokio::spawn(async move {
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

    assert_eq!(
        rx.recv().await.unwrap(),
        VncInputAction::Key {
            usage: 0xe0,
            down: true
        }
    );
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
    let _ = server_task.await;
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

    // 7. Image payload loop suppression
    let img_payload = ClipboardPayload::ImagePng(vec![1, 2, 3, 4, 5]);
    let prev_suppressions = state.suppression_count();
    assert!(state.should_sync(&img_payload, ClipboardSource::Host));
    assert!(!state.should_sync(&img_payload, ClipboardSource::Device));
    assert_eq!(state.suppression_count(), prev_suppressions + 1);
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

#[test]
fn extended_clipboard_image_compression_round_trip() {
    let png_bytes = vec![0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a, 1, 2, 3, 4];
    let encoded_png = encode_extended_provide_image(&png_bytes, true).unwrap();
    assert_eq!(encoded_png[0], SERVER_CUT_TEXT);
    let len_bytes: [u8; 4] = encoded_png[4..8].try_into().unwrap();
    let length = i32::from_be_bytes(len_bytes);
    assert!(length < 0);
    let parsed_png = parse_extended_cut_text(&encoded_png[8..]).unwrap();
    assert_eq!(parsed_png.image_png.as_deref(), Some(png_bytes.as_slice()));
    assert_eq!(parsed_png.image_jpeg, None);
    assert_eq!(parsed_png.text, None);

    let jpeg_bytes = vec![0xff, 0xd8, 0xff, 0xe0, 0x00, 0x10, 0x4a, 0x46, 0x49, 0x46];
    let encoded_jpeg = encode_extended_provide_image(&jpeg_bytes, false).unwrap();
    assert_eq!(encoded_jpeg[0], SERVER_CUT_TEXT);
    let parsed_jpeg = parse_extended_cut_text(&encoded_jpeg[8..]).unwrap();
    assert_eq!(
        parsed_jpeg.image_jpeg.as_deref(),
        Some(jpeg_bytes.as_slice())
    );
    assert_eq!(parsed_jpeg.image_png, None);
    assert_eq!(parsed_jpeg.text, None);
}

#[test]
fn review_regression_rfb_never_labels_png_as_dib_or_jpeg_as_files() {
    let caps = encode_extended_caps();
    let flags = u32::from_be_bytes(caps[8..12].try_into().unwrap());
    assert_eq!(flags & 0xffff, FORMAT_TEXT | FORMAT_DIB);
    assert_eq!(
        caps.len(),
        20,
        "each of the two standard formats requires an unsolicited size"
    );
    for bit in [1 << 3, 1 << 4] {
        let body = (ACTION_PROVIDE | bit).to_be_bytes();
        let error = parse_rfb_extended_cut_text(&body).unwrap_err();
        assert_eq!(
            error.downcast_ref::<std::io::Error>().unwrap().kind(),
            std::io::ErrorKind::Unsupported
        );
    }
}

#[tokio::test]
async fn web_viewer_image_clipboard_endpoints() {
    let fixture = PolicyFixture::new(vec![grant(
        TOKEN,
        vec![Permission::Debug, Permission::Connect],
    )]);
    let access = AccessControl::from_file(fixture.path.clone())
        .await
        .unwrap();
    let state = WebViewerState::new(access, DEVICE.into(), 12345, true);
    let (base, handle, stop) = spawn_test_server(state.clone()).await;
    let client = test_client();

    let (sender, mut receiver) = mpsc::channel::<DeviceRequest>(4);
    *state.backend.write().await = Some(sender);

    // Mock device handler for image clipboard
    let mock_png_bytes = vec![0x89, 0x50, 0x4e, 0x47, 0xaa, 0xbb];
    let mock_png_bytes_clone = mock_png_bytes.clone();
    let backend_task = tokio::spawn(async move {
        while let Some(request) = receiver.recv().await {
            match request.command {
                DeviceCommand::ReadClipboard => {
                    let _ = request.reply.send(Ok(Some(ClipboardPayload::ImagePng(
                        mock_png_bytes_clone.clone(),
                    ))));
                }
                DeviceCommand::Input(_action) => {
                    let _ = request.reply.send(Ok(None));
                }
            }
        }
    });

    // 1. GET /clipboard returns PNG base64 and uti
    let response = client
        .get(format!("{base}/clipboard"))
        .bearer_auth(TOKEN)
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let content: serde_json::Value = serde_json::from_str(&response.text().await.unwrap()).unwrap();
    assert_eq!(content["uti"], UTI_PNG);
    let b64 = content["data_base64"].as_str().unwrap();
    let decoded = base64::Engine::decode(&base64::engine::general_purpose::STANDARD, b64).unwrap();
    assert_eq!(decoded, mock_png_bytes);

    // 2. POST /clipboard with PNG
    let post_resp = client
        .post(format!("{base}/clipboard"))
        .bearer_auth(TOKEN)
        .json(&serde_json::json!({
            "uti": UTI_PNG,
            "data_base64": b64,
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(post_resp.status(), StatusCode::OK);

    // 3. POST /clipboard with invalid base64 returns BAD_REQUEST
    let bad_b64 = client
        .post(format!("{base}/clipboard"))
        .bearer_auth(TOKEN)
        .json(&serde_json::json!({
            "uti": UTI_PNG,
            "data_base64": "!!!not-valid-base64@@@",
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(bad_b64.status(), StatusCode::BAD_REQUEST);

    let _ = stop.send(());
    let _ = handle.await;
    backend_task.abort();
}

#[test]
fn vnc_des_matches_independent_pycryptodome_vector_and_not_xor() {
    // PyCryptodome DES ECB with the RFC 6143 reversed-bit password key.
    let challenge = std::array::from_fn(|i| i as u8);
    let response = vnc_encrypt_challenge(&challenge, "password");
    assert_eq!(hex::encode(response), "b866924125c8eebb9debc1db61c538e2");
    let old_xor: Vec<_> = challenge
        .iter()
        .enumerate()
        .map(|(i, byte)| byte ^ b"password"[i % 8].reverse_bits())
        .collect();
    assert_ne!(response.as_slice(), old_xor);
}

#[tokio::test]
async fn viewer_owner_policy_checks_exact_device_port_and_requires_credential() {
    let fixture = PolicyFixture::new(vec![grant(
        TOKEN,
        vec![Permission::Debug, Permission::Connect],
    )]);
    let access = AccessControl::from_file(fixture.path.clone())
        .await
        .unwrap();
    assert!(super::owner_principal(&access, None, DEVICE, 12345).is_err());
    assert!(
        super::owner_principal(&access, Some(&fixture.token_path), "another-device", 12345)
            .is_err()
    );
    assert!(super::owner_principal(&access, Some(&fixture.token_path), DEVICE, 5900).is_err());
    assert!(super::owner_principal(&access, Some(&fixture.token_path), DEVICE, 12345).is_ok());
}

#[tokio::test]
async fn web_scope_and_device_acknowledgements_prevent_false_success() {
    let fixture = PolicyFixture::new(vec![grant(
        TOKEN,
        vec![Permission::Debug, Permission::Connect],
    )]);
    let access = AccessControl::from_file(fixture.path.clone())
        .await
        .unwrap();
    let state = WebViewerState::new(access.clone(), "another-device".into(), 12345, true);
    let (base, handle, stop) = spawn_test_server(state).await;
    let client = test_client();
    assert_eq!(
        client
            .get(format!("{base}/"))
            .bearer_auth(TOKEN)
            .send()
            .await
            .unwrap()
            .status(),
        StatusCode::FORBIDDEN
    );
    let _ = stop.send(());
    handle.await.unwrap();
    let state = WebViewerState::new(access.clone(), DEVICE.into(), 5900, true);
    let (base, handle, stop) = spawn_test_server(state).await;
    assert_eq!(
        client
            .get(format!("{base}/"))
            .bearer_auth(TOKEN)
            .send()
            .await
            .unwrap()
            .status(),
        StatusCode::FORBIDDEN
    );
    let _ = stop.send(());
    handle.await.unwrap();
    let state = WebViewerState::new(access, DEVICE.into(), 12345, true);
    let (base, handle, stop) = spawn_test_server(state.clone()).await;
    let pointer = || {
        client
            .post(format!("{base}/input/pointer"))
            .bearer_auth(TOKEN)
            .json(&serde_json::json!({"x":1,"y":1,"action":"down"}))
    };
    assert_eq!(
        pointer().send().await.unwrap().status(),
        StatusCode::SERVICE_UNAVAILABLE
    );
    let (sender, mut receiver) = mpsc::channel::<DeviceRequest>(1);
    *state.backend.write().await = Some(sender);
    let backend = tokio::spawn(async move {
        let request = receiver.recv().await.unwrap();
        request
            .reply
            .send(Err(anyhow::anyhow!("synthetic device failure")))
            .unwrap();
    });
    assert_eq!(
        pointer().send().await.unwrap().status(),
        StatusCode::SERVICE_UNAVAILABLE
    );
    backend.await.unwrap();
    let _observed = observe_web_backend(&state).await;
    let response = client
        .get(format!("{base}/clipboard"))
        .bearer_auth(TOKEN)
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let content: serde_json::Value = serde_json::from_str(&response.text().await.unwrap()).unwrap();
    assert_eq!(content["text"], "actual text");
    let _ = stop.send(());
    handle.await.unwrap();
}

#[tokio::test]
async fn vnc_stop_wakes_original_silent_handshake_and_closes_socket() {
    let (state, _observed) = test_vnc_state(None).await;
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let session_state = state.clone();
    let server = tokio::spawn(async move {
        let (stream, peer) = listener.accept().await.unwrap();
        VncSession::new(stream, peer, session_state).run().await
    });
    let mut client = TcpStream::connect(addr).await.unwrap();
    let mut version = [0; 12];
    client.read_exact(&mut version).await.unwrap();
    state.stop();
    tokio::time::timeout(Duration::from_secs(1), server)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    assert_eq!(client.read(&mut version).await.unwrap(), 0);
}

const HEVC_SAMPLE: &[u8] = include_bytes!("hevc_sample.bin");

#[test]
fn hevc_decoder_real_decoding_and_dimension_verification() {
    let probe = DecoderProbe::probe();
    if !probe.available {
        eprintln!("skipping real hevc decode test: ffmpeg unavailable in environment");
        return;
    }
    let mut decoder = HevcDecoder::new().expect("hevc decoder initialization");
    assert_eq!(decoder.probe.backend, "ffmpeg");
    assert!(!decoder.probe.version.is_empty());

    let frame = decoder
        .decode_access_unit(HEVC_SAMPLE, 12345)
        .expect("decode real hevc access unit");
    assert_eq!(frame.width, 32);
    assert_eq!(frame.height, 32);
    assert_eq!(frame.timestamp_ms, 12345);
    assert_eq!(frame.bgra.len(), 32 * 32 * 4);
    assert_eq!(&frame.png[..8], b"\x89PNG\r\n\x1a\n");
    assert_eq!(decoder.last_width, 32);
    assert_eq!(decoder.last_height, 32);
}

#[test]
fn hevc_pipeline_bounded_frame_queue_and_slow_consumer_drop() {
    let probe = DecoderProbe::probe();
    if !probe.available {
        return;
    }
    let mut pipeline = HevcPipeline::new(2).expect("hevc pipeline initialization");
    assert_eq!(pipeline.dropped_count(), 0);

    pipeline
        .push_access_unit(HEVC_SAMPLE, 100)
        .expect("frame 1");
    pipeline
        .push_access_unit(HEVC_SAMPLE, 200)
        .expect("frame 2");
    assert_eq!(pipeline.dropped_count(), 0);

    // Push 3rd frame: queue capacity is 2, oldest (frame 1) is dropped
    pipeline
        .push_access_unit(HEVC_SAMPLE, 300)
        .expect("frame 3");
    assert_eq!(pipeline.dropped_count(), 1);

    let pop1 = pipeline.pop_frame().expect("pop frame 2");
    assert_eq!(pop1.timestamp_ms, 200);
    let pop2 = pipeline.pop_frame().expect("pop frame 3");
    assert_eq!(pop2.timestamp_ms, 300);
    assert!(pipeline.pop_frame().is_none());
}

#[tokio::test]
async fn hevc_decoded_frame_updates_web_and_vnc_endpoints() {
    let probe = DecoderProbe::probe();
    if !probe.available {
        return;
    }
    let mut decoder = HevcDecoder::new().expect("hevc decoder initialization");
    let frame = decoder
        .decode_access_unit(HEVC_SAMPLE, 555)
        .expect("decode sample frame");

    // 1. Update WebViewerState
    let fixture = PolicyFixture::new(vec![grant(
        TOKEN,
        vec![Permission::Debug, Permission::Connect],
    )]);
    let access = AccessControl::from_file(fixture.path.clone())
        .await
        .unwrap();
    let web_state = WebViewerState::new(access, DEVICE.into(), 12345, true);
    web_state
        .push_frame(
            "image/png",
            frame.png.to_vec(),
            frame.width as u32,
            frame.height as u32,
            frame.timestamp_ms,
        )
        .await;

    {
        let guard = web_state.frame.read().await;
        let pushed = guard.as_ref().expect("pushed web frame");
        assert_eq!(pushed.width, 32);
        assert_eq!(pushed.height, 32);
        assert_eq!(pushed.timestamp_ms, 555);
        assert_eq!(&pushed.data[..8], b"\x89PNG\r\n\x1a\n");
    }

    // 2. Update VncServerState
    let (vnc_state, _observed) = test_vnc_state(None).await;
    vnc_state
        .update_frame(
            frame.width,
            frame.height,
            (*frame.bgra).clone(),
            frame.timestamp_ms,
        )
        .await;
    {
        let guard = vnc_state.frame.read().await;
        assert_eq!(guard.width, 32);
        assert_eq!(guard.height, 32);
        assert_eq!(guard.timestamp_ms, 555);
        assert_eq!(guard.bgra_data.len(), 32 * 32 * 4);
    }
}

#[test]
fn standard_dib_png_round_trip_conversion() {
    let mut original_png = Vec::new();
    {
        let mut encoder = png::Encoder::new(&mut original_png, 2, 2);
        encoder.set_color(png::ColorType::Rgb);
        encoder.set_depth(png::BitDepth::Eight);
        let mut writer = encoder.write_header().unwrap();
        // 2x2 image with distinct RGB pixels
        writer
            .write_image_data(&[
                255, 0, 0, // top-left: Red
                0, 255, 0, // top-right: Green
                0, 0, 255, // bottom-left: Blue
                255, 255, 0, // bottom-right: Yellow
            ])
            .unwrap();
    }

    // Convert PNG to standard DIB
    let dib = png_to_dib(&original_png).expect("convert PNG to DIB");
    // Verify DIB header
    assert!(dib.len() >= 124);
    let bi_size = u32::from_le_bytes(dib[0..4].try_into().unwrap());
    assert_eq!(bi_size, 124);
    let bi_width = i32::from_le_bytes(dib[4..8].try_into().unwrap());
    assert_eq!(bi_width, 2);
    let bi_height = i32::from_le_bytes(dib[8..12].try_into().unwrap());
    assert_eq!(bi_height, 2); // Positive indicates bottom-up
    let bi_bit_count = u16::from_le_bytes(dib[14..16].try_into().unwrap());
    assert_eq!(bi_bit_count, 24);

    // Convert DIB back to PNG
    let recovered_png = dib_to_png(&dib).expect("convert DIB back to PNG");
    assert_eq!(&recovered_png[..8], b"\x89PNG\r\n\x1a\n");

    // Decode recovered PNG and verify pixels match
    let decoder = png::Decoder::new(std::io::Cursor::new(recovered_png));
    let mut reader = decoder.read_info().unwrap();
    assert_eq!(reader.info().width, 2);
    assert_eq!(reader.info().height, 2);
    let mut pixels = vec![0u8; reader.output_buffer_size()];
    reader.next_frame(&mut pixels).unwrap();
    assert_eq!(
        &pixels[..12],
        &[255, 0, 0, 0, 255, 0, 0, 0, 255, 255, 255, 0]
    );
}

#[test]
fn rgba_dib_preserves_alpha_and_jpeg_dib_uses_decoded_pixels_for_loop_suppression() {
    let mut rgba = Vec::new();
    {
        let mut encoder = png::Encoder::new(&mut rgba, 2, 1);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        encoder
            .write_header()
            .unwrap()
            .write_image_data(&[255, 0, 0, 17, 0, 255, 0, 0])
            .unwrap();
    }
    let dib = png_to_dib(&rgba).unwrap();
    assert_eq!(u16::from_le_bytes(dib[14..16].try_into().unwrap()), 32);
    assert_eq!(u32::from_le_bytes(dib[16..20].try_into().unwrap()), 3);
    assert_eq!(&dib[124..], &[0, 0, 255, 17, 0, 255, 0, 0]);
    let decoded = super::media::decode_png(dib_to_png(&dib).unwrap().into(), 0).unwrap();
    assert_eq!(decoded.bgra.as_slice(), &[0, 0, 255, 17, 0, 255, 0, 0]);
    let jpeg = ClipboardPayload::ImageJpeg(include_bytes!("clipboard_sample.jpg").to_vec());
    let encoded = encode_rfb_provide_image(&jpeg).unwrap();
    let png = parse_rfb_extended_cut_text(&encoded[8..])
        .unwrap()
        .image_png
        .unwrap();
    let decoded = super::media::decode_png(png.clone().into(), 0).unwrap();
    assert_eq!((decoded.width, decoded.height), (16, 16));
    assert!(decoded.bgra[2] > 200 && decoded.bgra[0] < 20);
    let mut sync = ClipboardSyncState::new();
    assert!(sync.should_sync(&jpeg, ClipboardSource::Device));
    assert!(!sync.should_sync(&ClipboardPayload::ImagePng(png), ClipboardSource::Host));
}

#[test]
fn rfb_extended_clipboard_dib_full_round_trip() {
    let mut original_png = Vec::new();
    {
        let mut encoder = png::Encoder::new(&mut original_png, 1, 1);
        encoder.set_color(png::ColorType::Rgb);
        encoder.set_depth(png::BitDepth::Eight);
        let mut writer = encoder.write_header().unwrap();
        writer.write_image_data(&[128, 64, 32]).unwrap();
    }

    // Server encodes DIB cut text
    let encoded = encode_extended_provide_dib(&original_png).expect("encode provide DIB");
    assert_eq!(encoded[0], SERVER_CUT_TEXT);
    let len_bytes: [u8; 4] = encoded[4..8].try_into().unwrap();
    let length = i32::from_be_bytes(len_bytes);
    assert!(length < 0);

    // Client/server parses DIB cut text
    let parsed = parse_rfb_extended_cut_text(&encoded[8..]).expect("parse provide DIB");
    assert!(parsed.image_png.is_some());
    let recovered_png = parsed.image_png.unwrap();
    assert_eq!(&recovered_png[..8], b"\x89PNG\r\n\x1a\n");

    let decoder = png::Decoder::new(std::io::Cursor::new(recovered_png));
    let mut reader = decoder.read_info().unwrap();
    assert_eq!(reader.info().width, 1);
    assert_eq!(reader.info().height, 1);
    let mut pixels = vec![0u8; reader.output_buffer_size()];
    reader.next_frame(&mut pixels).unwrap();
    assert_eq!(&pixels[..3], &[128, 64, 32]);
}

#[tokio::test]
async fn bidirectional_clipboard_device_and_vnc_sync_with_loop_suppression() {
    let mut sync = ClipboardSyncState::new();
    let text_payload = ClipboardPayload::Text("hello device".into());
    let image_payload = ClipboardPayload::ImagePng(vec![1, 2, 3, 4]);

    // 1. Initial write from Host
    assert!(sync.should_sync(&text_payload, ClipboardSource::Host));
    assert_eq!(sync.sync_count(), 1);

    // 2. Echo from Device with identical hash is suppressed
    assert!(!sync.should_sync(&text_payload, ClipboardSource::Device));
    assert_eq!(sync.suppression_count(), 1);

    // 3. Duplicate notification from Host is suppressed
    assert!(!sync.should_sync(&text_payload, ClipboardSource::Host));

    // 4. New Image from Device is accepted
    assert!(sync.should_sync(&image_payload, ClipboardSource::Device));
    assert_eq!(sync.sync_count(), 2);

    // 5. Echo from Host with identical image is suppressed
    assert!(!sync.should_sync(&image_payload, ClipboardSource::Host));
    assert_eq!(sync.suppression_count(), 2);
}

#[test]
fn review_regression_rfb_v5_external_layout_and_invalid_height() {
    // Independently laid out per BITMAPV5HEADER, not generated by png_to_dib.
    let mut dib = vec![0u8; 132];
    dib[0..4].copy_from_slice(&124u32.to_le_bytes());
    dib[4..8].copy_from_slice(&1i32.to_le_bytes());
    dib[8..12].copy_from_slice(&2i32.to_le_bytes());
    dib[12..14].copy_from_slice(&1u16.to_le_bytes());
    dib[14..16].copy_from_slice(&24u16.to_le_bytes());
    dib[56..60].copy_from_slice(&0x73524742u32.to_le_bytes());
    // bottom row blue, top row red, DWORD row alignment.
    dib[124..132].copy_from_slice(&[255, 0, 0, 0, 0, 0, 255, 0]);
    let png = dib_to_png(&dib).unwrap();
    let frame = decode_png(bytes::Bytes::from(png), 0).unwrap();
    assert_eq!((frame.width, frame.height), (1, 2));
    assert_eq!(frame.bgra.as_slice(), &[0, 0, 255, 255, 255, 0, 0, 255]);
    dib[8..12].copy_from_slice(&i32::MIN.to_le_bytes());
    assert!(
        dib_to_png(&dib).is_err(),
        "untrusted height must return an error, never panic"
    );
}

#[tokio::test]
async fn review_regression_vnc_negotiates_v5_images_and_serves_requests() {
    let principal = AccessControl::trusted_test_local()
        .authenticate(&axum::http::HeaderMap::new())
        .unwrap();
    let mut png = Vec::new();
    {
        let mut encoder = png::Encoder::new(&mut png, 1, 1);
        encoder.set_color(png::ColorType::Rgb);
        encoder.set_depth(png::BitDepth::Eight);
        encoder
            .write_header()
            .unwrap()
            .write_image_data(&[255, 0, 0])
            .unwrap();
    }
    let (backend, mut requests) = mpsc::channel::<DeviceRequest>(8);
    let (observed, mut inputs) = mpsc::unbounded_channel();
    let backend_task = tokio::spawn(async move {
        while let Some(request) = requests.recv().await {
            let result = match request.command {
                DeviceCommand::ReadClipboard => Some(ClipboardPayload::ImagePng(png.clone())),
                DeviceCommand::Input(action) => {
                    observed.send(action).unwrap();
                    None
                }
            };
            let _ = request.reply.send(Ok(result));
        }
    });
    let state = VncServerState::new(1, 1, None, principal, true, backend);
    state.update_frame(1, 1, vec![0, 0, 255, 255], 0).await;
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let server_state = state.clone();
    let server = tokio::spawn(async move {
        let (stream, peer) = listener.accept().await.unwrap();
        VncSession::new(stream, peer, server_state).run().await
    });
    tokio::time::timeout(Duration::from_secs(5), async {
        let mut client = TcpStream::connect(address).await.unwrap();
        let mut version = [0; 12];
        client.read_exact(&mut version).await.unwrap();
        client.write_all(RFB_VERSION_3_8).await.unwrap();
        assert_eq!(client.read_u16().await.unwrap(), 0x0101);
        client.write_u8(RFB_SECURITY_NONE).await.unwrap();
        assert_eq!(client.read_u32().await.unwrap(), 0);
        client.write_u8(1).await.unwrap();
        let mut init = [0; 20];
        client.read_exact(&mut init).await.unwrap();
        let size = client.read_u32().await.unwrap() as usize;
        let mut name = vec![0; size];
        client.read_exact(&mut name).await.unwrap();
        client
            .write_all(&[RFB_CLIENT_SET_ENCODINGS, 0, 0, 1])
            .await
            .unwrap();
        client.write_i32(ENC_EXTENDED_CLIPBOARD).await.unwrap();
        assert_eq!(client.read_u8().await.unwrap(), SERVER_CUT_TEXT);
        let mut padding = [0; 3];
        client.read_exact(&mut padding).await.unwrap();
        assert_eq!(client.read_i32().await.unwrap(), -12);
        let flags = client.read_u32().await.unwrap();
        assert_eq!(flags & 0xffff, FORMAT_TEXT | FORMAT_DIB);
        assert_eq!(
            client.read_u64().await.unwrap(),
            0,
            "unsolicited limits for both formats"
        );
        // Explicit client capability; before this, server assumes text only.
        let mut caps = encode_extended_caps();
        caps[0] = CLIENT_CUT_TEXT;
        client.write_all(&caps).await.unwrap();
        client.write_all(&[CLIENT_CUT_TEXT, 0, 0, 0]).await.unwrap();
        client.write_i32(-4).await.unwrap();
        client.write_u32(ACTION_REQUEST | FORMAT_DIB).await.unwrap();
        assert_eq!(client.read_u8().await.unwrap(), SERVER_CUT_TEXT);
        client.read_exact(&mut padding).await.unwrap();
        let length = client.read_i32().await.unwrap().unsigned_abs() as usize;
        let mut body = vec![0; length];
        client.read_exact(&mut body).await.unwrap();
        // Notify is asynchronous and may precede the reply to our Request.
        // Validate that exact legal announcement, then still require Provide.
        if u32::from_be_bytes(body[..4].try_into().unwrap()) == ACTION_NOTIFY | FORMAT_DIB {
            assert_eq!(body.len(), 4, "Notify carries no image payload");
            assert_eq!(client.read_u8().await.unwrap(), SERVER_CUT_TEXT);
            client.read_exact(&mut padding).await.unwrap();
            let size = client.read_i32().await.unwrap().unsigned_abs() as usize;
            assert!(size <= MAX_IMAGE_BYTES + 4);
            body.resize(size, 0);
            client.read_exact(&mut body).await.unwrap();
        }
        assert_eq!(
            u32::from_be_bytes(body[..4].try_into().unwrap()),
            ACTION_PROVIDE | FORMAT_DIB
        );
        use std::io::{Read, Write};
        let mut data = Vec::new();
        ios_core::flate2::read::ZlibDecoder::new(&body[4..])
            .read_to_end(&mut data)
            .unwrap();
        assert_eq!(u32::from_le_bytes(data[4..8].try_into().unwrap()), 124);
        assert_eq!(&data[128..132], &[0, 0, 255, 0]);
        // Independent 1x1 DIB v5 fixture (blue), sent client -> device adapter.
        let mut dib = vec![0u8; 128];
        dib[..4].copy_from_slice(&124u32.to_le_bytes());
        dib[4..8].copy_from_slice(&1i32.to_le_bytes());
        dib[8..12].copy_from_slice(&1i32.to_le_bytes());
        dib[12..14].copy_from_slice(&1u16.to_le_bytes());
        dib[14..16].copy_from_slice(&24u16.to_le_bytes());
        dib[56..60].copy_from_slice(&0x73524742u32.to_le_bytes());
        dib[124] = 255;
        let mut encoder = ios_core::flate2::write::ZlibEncoder::new(
            Vec::new(),
            ios_core::flate2::Compression::default(),
        );
        encoder.write_all(&128u32.to_be_bytes()).unwrap();
        encoder.write_all(&dib).unwrap();
        let compressed = encoder.finish().unwrap();
        client.write_all(&[CLIENT_CUT_TEXT, 0, 0, 0]).await.unwrap();
        client
            .write_i32(-((compressed.len() + 4) as i32))
            .await
            .unwrap();
        client.write_u32(ACTION_PROVIDE | FORMAT_DIB).await.unwrap();
        client.write_all(&compressed).await.unwrap();
        let VncInputAction::ClipboardOut(ClipboardPayload::ImagePng(png)) =
            inputs.recv().await.unwrap()
        else {
            panic!("expected PNG device write")
        };
        let frame = decode_png(bytes::Bytes::from(png), 0).unwrap();
        assert_eq!(frame.bgra.as_slice(), &[255, 0, 0, 255]);
        state.stop();
        drop(client);
    })
    .await
    .unwrap();
    server.await.unwrap().unwrap();
    backend_task.abort();
    let _ = backend_task.await;
}

#[test]
fn review_regression_clipboard_echo_uses_pixels_across_rfb_reencoding() {
    let mut png = Vec::new();
    {
        let mut encoder = png::Encoder::new(&mut png, 2, 1);
        encoder.set_color(png::ColorType::Rgb);
        encoder.set_depth(png::BitDepth::Eight);
        encoder
            .add_text_chunk("fixture".into(), "metadata lost by DIB conversion".into())
            .unwrap();
        encoder
            .write_header()
            .unwrap()
            .write_image_data(&[255, 0, 0, 0, 0, 255])
            .unwrap();
    }
    let echoed = dib_to_png(&png_to_dib(&png).unwrap()).unwrap();
    assert_ne!(png, echoed, "fixture must exercise distinct PNG wire bytes");
    let mut sync = ClipboardSyncState::new();
    assert!(sync.should_sync(&ClipboardPayload::ImagePng(png), ClipboardSource::Device));
    assert!(!sync.should_sync(&ClipboardPayload::ImagePng(echoed), ClipboardSource::Host));
    assert!(sync.should_sync(
        &ClipboardPayload::Text("line1\r\nline2".into()),
        ClipboardSource::Device
    ));
    assert!(!sync.should_sync(
        &ClipboardPayload::Text("line1\nline2".into()),
        ClipboardSource::Host
    ));
}
