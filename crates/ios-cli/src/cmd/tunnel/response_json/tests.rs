use super::super::{
    agent_router, source, Route, TunMode, TunnelAgentState, TUNNEL_CONNECT_TIMEOUT,
};
use super::*;
use futures_util::{future::BoxFuture, StreamExt};
use std::cell::Cell;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

#[tokio::test]
async fn json_wire_last_frame_slice_pays_after_body_and_handler_drop() {
    let bank = Arc::new(Semaphore::new(16384));
    let replies = Replies::new(&bank).unwrap();
    let value = serde_json::json!({"text":"escaped\n\"雪🦀", "omitted":null});
    let expected = serde_json::to_vec(&value).unwrap();
    let response = replies.json(&value).unwrap();
    assert_eq!(response.headers()[header::CONTENT_TYPE], "application/json");
    assert_eq!(
        bank.available_permits(),
        16384 - DIAGNOSTIC_CAPACITY - expected.len()
    );
    let mut body = response.into_body().into_data_stream();
    let frame = body.next().await.unwrap().unwrap();
    assert_eq!(&frame[..], expected);
    let last = frame.slice(1..2);
    drop(frame);
    drop(body);
    drop(replies);
    assert_eq!(bank.available_permits(), 16384 - expected.len());
    drop(last);
    assert_eq!(bank.available_permits(), 16384);
}

#[test]
fn json_wire_one_byte_refusal_has_no_data_allocation() {
    let value = ["雪", "escape\n", "\""];
    let size = serde_json::to_vec(&value).unwrap().len();
    let bank = Arc::new(Semaphore::new(size - 1));
    let allocated = Arc::new(AtomicUsize::new(0));
    let captured = Arc::clone(&allocated);
    super::super::metadata_vec::with_allocation_hook(
        move |_, _| {
            captured.fetch_add(1, Ordering::SeqCst);
        },
        || {
            assert!(encode(&bank, &value).is_err());
        },
    );
    assert_eq!(allocated.load(Ordering::SeqCst), 0);
    assert_eq!(bank.available_permits(), size - 1);
}

#[test]
fn json_wire_actual_data_layout_is_paid_before_allocation() {
    let bank = Arc::new(Semaphore::new(4096));
    let charged = Arc::clone(&bank);
    let seen = Arc::new(AtomicUsize::new(0));
    let captured = Arc::clone(&seen);
    let value = [1, 2, 3];
    let size = serde_json::to_vec(&value).unwrap().len();
    let data = super::super::metadata_vec::with_allocation_hook(
        move |layout, _| {
            assert_eq!(layout.size(), size);
            assert_eq!(charged.available_permits(), 4096 - size);
            captured.fetch_add(1, Ordering::SeqCst);
        },
        || encode(&bank, &value).unwrap(),
    );
    assert_eq!(seen.load(Ordering::SeqCst), 1);
    assert_eq!(data.values.capacity(), size);
    drop(data);
    assert_eq!(bank.available_permits(), 4096);
}

#[test]
fn json_wire_second_pass_cannot_grow_or_keep_a_failed_buffer_grant() {
    struct Changes(Cell<bool>);
    impl Serialize for Changes {
        fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
            serializer.serialize_str(if self.0.replace(true) {
                "would need a larger buffer"
            } else {
                "a"
            })
        }
    }
    let bank = Arc::new(Semaphore::new(4096));
    assert!(encode(&bank, &Changes(Cell::new(false))).is_err());
    assert_eq!(bank.available_permits(), 4096);
}

#[tokio::test]
async fn json_wire_prepaid_native_error_keeps_class_and_schema_at_zero_available_bytes() {
    let bank = Arc::new(Semaphore::new(DIAGNOSTIC_CAPACITY));
    let replies = Replies::new(&bank).unwrap();
    assert_eq!(bank.available_permits(), 0);
    let original = io::Error::new(io::ErrorKind::ConnectionReset, "synthetic-private-original");
    let response =
        replies.operation_error(StatusCode::BAD_GATEWAY, "tunnel TCP dial failed", &original);
    assert_eq!(response.status(), StatusCode::BAD_GATEWAY);
    assert_eq!(response.headers()["x-ios-error-class"], "disconnected");
    assert_eq!(response.headers()["x-ios-error-effects"], "unknown");
    let mut body = response.into_body().into_data_stream();
    let frame = body.next().await.unwrap().unwrap();
    drop(body);
    drop(replies);
    assert_eq!(bank.available_permits(), 0);
    let json: serde_json::Value = serde_json::from_slice(&frame).unwrap();
    assert_eq!(json["message"], "tunnel TCP dial failed");
    assert!(json["error"]
        .as_str()
        .unwrap()
        .starts_with("io/ConnectionReset error:"));
    assert!(!std::str::from_utf8(&frame)
        .unwrap()
        .contains("synthetic-private-original"));
    assert_eq!(original.kind(), io::ErrorKind::ConnectionReset);
    drop(frame);
    assert_eq!(bank.available_permits(), DIAGNOSTIC_CAPACITY);
}

#[tokio::test]
async fn json_wire_constructor_refusal_uses_static_reviewed_resource_body() {
    let bank = Arc::new(Semaphore::new(DIAGNOSTIC_CAPACITY - 1));
    let Err(response) = Replies::new(&bank) else {
        panic!("diagnostic preflight bypassed")
    };
    assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(response.headers()["x-ios-error-class"], "resource_limit");
    let bytes = axum::body::to_bytes(response.into_body(), 4096)
        .await
        .unwrap();
    assert_eq!(
        &bytes[..],
        b"{\"message\":\"manager response admission failed\"}"
    );
    assert_eq!(bank.available_permits(), DIAGNOSTIC_CAPACITY - 1);
}

#[tokio::test]
async fn json_wire_keeps_large_success_output_and_omitted_option_fields() {
    #[derive(Serialize)]
    struct Data<'a> {
        text: &'a str,
        #[serde(skip_serializing_if = "Option::is_none")]
        missing: Option<u16>,
    }
    let text = "雪".repeat(400_000);
    let data = Data {
        text: &text,
        missing: None,
    };
    let bank = Arc::new(Semaphore::new(2 * 1024 * 1024));
    let replies = Replies::new(&bank).unwrap();
    let response = replies.json(&data).unwrap();
    let bytes = axum::body::to_bytes(response.into_body(), 2 * 1024 * 1024)
        .await
        .unwrap();
    assert!(
        bytes.len() > super::super::MAX_LIST_BYTES,
        "inbound cap was silently applied to local output"
    );
    assert_eq!(&bytes[..], serde_json::to_vec(&data).unwrap());
    drop(replies);
    drop(bytes);
    assert_eq!(bank.available_permits(), 2 * 1024 * 1024);
}

#[derive(Default)]
struct NoPoll(AtomicUsize);
impl source::Source for NoPoll {
    fn connect<'a>(
        &'a self,
        _: &'a str,
        _: Route,
        _: TunMode,
    ) -> BoxFuture<'a, anyhow::Result<super::super::ManagedDevice>> {
        Box::pin(async move {
            self.0.fetch_add(1, Ordering::SeqCst);
            panic!("response refusal polled connect")
        })
    }
    fn usb_devices(&self) -> BoxFuture<'_, source::UsbSnapshot> {
        Box::pin(async {
            self.0.fetch_add(1, Ordering::SeqCst);
            panic!("response refusal polled usb")
        })
    }
    fn paired_devices(
        &self,
    ) -> BoxFuture<'_, anyhow::Result<Vec<ios_core::device::PairedMobdev2Device>>> {
        Box::pin(async {
            self.0.fetch_add(1, Ordering::SeqCst);
            panic!("response refusal polled paired")
        })
    }
}

#[tokio::test]
async fn json_wire_actual_router_denial_precedes_source_and_epoch_mutation() {
    let source = Arc::new(NoPoll::default());
    let state = TunnelAgentState::with_source(
        TunMode::Userspace,
        crate::access::AccessControl::trusted_test_local(),
        crate::access::Upstreams::default(),
        source.clone(),
        TUNNEL_CONNECT_TIMEOUT,
    );
    let old = state
        .inner
        .lifecycle
        .ticket("response-fixture")
        .await
        .unwrap();
    let bank = Arc::clone(&state.inner.global_metadata_bytes);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let (stop, stopped) = tokio::sync::oneshot::channel();
    let app = agent_router(state.clone());
    let server = tokio::spawn(async move {
        axum::serve(listener, app)
            .with_graceful_shutdown(async {
                let _ = stopped.await;
            })
            .await
            .unwrap();
    });
    let held = Arc::clone(&bank)
        .try_acquire_many_owned(bank.available_permits().try_into().unwrap())
        .unwrap();
    let client = reqwest::Client::new();
    let reply = client
        .post(format!("http://{address}/refresh"))
        .send()
        .await
        .unwrap();
    assert_eq!(reply.status(), StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(reply.headers()["x-ios-error-class"], "resource_limit");
    assert!(reply
        .text()
        .await
        .unwrap()
        .contains("manager response admission failed"));
    // Enough for the diagnostic, one byte short for the prebuilt epoch reply.
    drop(held);
    let length = serde_json::to_vec(&serde_json::json!({"operation":"epoch", "data":true}))
        .unwrap()
        .len();
    let keep = DIAGNOSTIC_CAPACITY + length - 1;
    let held = Arc::clone(&bank)
        .try_acquire_many_owned((bank.available_permits() - keep).try_into().unwrap())
        .unwrap();
    let reply = client
        .post(format!("http://{address}/epoch"))
        .send()
        .await
        .unwrap();
    assert_eq!(reply.status(), StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(reply.headers()["x-ios-error-class"], "resource_limit");
    reply.bytes().await.unwrap();
    assert!(
        old.is_current(),
        "mutation ran before the output owner was admitted"
    );
    assert_eq!(source.0.load(Ordering::SeqCst), 0);
    drop(held);
    stop.send(()).unwrap();
    tokio::time::timeout(Duration::from_secs(2), server)
        .await
        .unwrap()
        .unwrap();
}

#[tokio::test]
async fn json_wire_actual_scoped_denial_precedes_pool_admission_for_get_and_delete() {
    use crate::access::test_support::{grant, PolicyFixture, TOKEN};
    let fixture = PolicyFixture::new(vec![grant(
        TOKEN,
        vec![
            crate::access::Permission::List,
            crate::access::Permission::ManageTunnels,
        ],
    )]);
    let access = crate::access::AccessControl::from_file(fixture.path.clone())
        .await
        .unwrap();
    let source = Arc::new(NoPoll::default());
    let state = TunnelAgentState::with_source(
        TunMode::Userspace,
        access,
        crate::access::Upstreams::default(),
        source.clone(),
        TUNNEL_CONNECT_TIMEOUT,
    );
    let bank = Arc::clone(&state.inner.global_metadata_bytes);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let (stop, stopped) = tokio::sync::oneshot::channel();
    let app = agent_router(state.clone());
    let server = tokio::spawn(async move {
        axum::serve(listener, app)
            .with_graceful_shutdown(async {
                let _ = stopped.await;
            })
            .await
            .unwrap();
    });
    let held = Arc::clone(&bank)
        .try_acquire_many_owned(bank.available_permits().try_into().unwrap())
        .unwrap();
    let client = reqwest::Client::builder()
        .no_proxy()
        .timeout(Duration::from_secs(2))
        .build()
        .unwrap();
    for method in [reqwest::Method::GET, reqwest::Method::DELETE] {
        let reply = client
            .request(
                method,
                format!("http://{address}/tunnel/synthetic-forbidden-device"),
            )
            .bearer_auth(TOKEN)
            .send()
            .await
            .unwrap();
        assert_eq!(reply.status(), StatusCode::FORBIDDEN);
        assert!(reply.text().await.unwrap().contains("manager scope denied"));
        assert_eq!(bank.available_permits(), 0);
    }
    assert_eq!(source.0.load(Ordering::SeqCst), 0);
    drop(held);
    stop.send(()).unwrap();
    tokio::time::timeout(Duration::from_secs(2), server)
        .await
        .unwrap()
        .unwrap();
}

#[tokio::test]
async fn json_wire_fixed_envelopes_keep_existing_json_byte_order() {
    let bank = Arc::new(Semaphore::new(32768));
    let replies = Replies::new(&bank).unwrap();
    let responses = [
        (
            replies.operation("epoch").unwrap(),
            serde_json::json!({"operation":"epoch", "data":true}),
        ),
        (
            replies.shutdown().unwrap(),
            serde_json::json!({"operation":"shutdown", "data":true, "message":"Server shutting down..."}),
        ),
        (
            replies
                .upstream("add_upstream", "http://127.0.0.1:1/", true)
                .unwrap(),
            serde_json::json!({"operation":"add_upstream", "url":"http://127.0.0.1:1/", "data":true}),
        ),
        (
            replies.cancelled("雪").unwrap(),
            serde_json::json!({"operation":"cancel", "udid":"雪", "data":true}),
        ),
    ];
    for (response, value) in responses {
        let bytes = axum::body::to_bytes(response.into_body(), 4096)
            .await
            .unwrap();
        assert_eq!(&bytes[..], serde_json::to_vec(&value).unwrap());
    }
    drop(replies);
    assert_eq!(bank.available_permits(), 32768);
}
