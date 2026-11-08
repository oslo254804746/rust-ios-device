//! Actual Rust HTTP/WebSocket adapters over owned synthetic plist streams.
//! These fixtures use the existing core WebInspector protocol tests, not a device.

mod client_fixtures;
mod client_tests;

use std::sync::{Arc, Mutex};
use std::time::Duration;

use anyhow::{anyhow, Result};
use axum::http::StatusCode;
use futures_util::{SinkExt, StreamExt};
use plist::{Dictionary, Value};
use serde_json::{json, Value as JsonValue};
use tokio::io::{AsyncReadExt, AsyncWriteExt, DuplexStream};
use tokio::net::TcpListener;
use tokio::task::JoinHandle;
use tokio_tungstenite::tungstenite::{client::IntoClientRequest, Message};

use super::cdp::cdp_router;
use super::connection::{ConnectionFuture, InspectorConnection, InspectorSource};
use super::http::ServerState;
use super::sessions::Sessions;
use super::webdriver::selenium_router;
use crate::access::test_support::{grant, PolicyFixture, DEVICE, TOKEN};
use crate::access::{AccessControl, HttpBoundaryArgs, Permission};

const SECRET: &str = "synthetic-private-source-failure";
const BUDGET: Duration = Duration::from_secs(3);
const TARGET: &str = "PID:42:page-1";

#[derive(Default, Debug)]
struct Trace {
    identifiers: usize,
    discovery: usize,
    listings: usize,
    setups: usize,
    evaluations: usize,
    failures: usize,
    teardowns: usize,
}

#[derive(Default)]
struct SyntheticSource {
    reject: bool,
    close_at_startup: bool,
    page_id: Option<u64>,
    calls: Mutex<Vec<String>>,
    tasks: Mutex<Vec<JoinHandle<Result<Trace>>>>,
}

impl InspectorSource for SyntheticSource {
    fn connect<'a>(&'a self, udid: &'a str, _timeout: Duration) -> ConnectionFuture<'a> {
        Box::pin(async move {
            self.calls.lock().unwrap().push(udid.to_owned());
            if self.reject {
                return Err(
                    std::io::Error::new(std::io::ErrorKind::PermissionDenied, SECRET).into(),
                );
            }
            let (stream, peer) = tokio::io::duplex(64 * 1024);
            if self.close_at_startup {
                drop(peer);
            } else {
                self.tasks
                    .lock()
                    .unwrap()
                    .push(tokio::spawn(fake_service(peer, self.page_id.unwrap_or(1))));
            }
            Ok(InspectorConnection {
                device: None,
                stream: Box::new(stream),
            })
        })
    }
}

impl SyntheticSource {
    fn calls(&self) -> usize {
        let calls = self.calls.lock().unwrap();
        assert!(calls.iter().all(|udid| udid == DEVICE));
        calls.len()
    }

    async fn finish(&self) -> Vec<Trace> {
        let tasks = std::mem::take(&mut *self.tasks.lock().unwrap());
        let mut traces = Vec::new();
        for mut task in tasks {
            match tokio::time::timeout(BUDGET, &mut task).await {
                Ok(result) => traces.push(result.unwrap().unwrap()),
                Err(_) => {
                    task.abort();
                    panic!("synthetic service did not release its stream");
                }
            }
        }
        traces
    }
}

impl Drop for SyntheticSource {
    fn drop(&mut self) {
        for task in self.tasks.get_mut().unwrap().drain(..) {
            task.abort();
        }
    }
}

struct ServerTask(JoinHandle<()>);
impl Drop for ServerTask {
    fn drop(&mut self) {
        self.0.abort();
    }
}

async fn start(source: Arc<SyntheticSource>, cdp: bool) -> (u16, ServerTask, PolicyFixture) {
    let policy = PolicyFixture::new(vec![grant(TOKEN, vec![Permission::Debug])]);
    let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
    let bound = listener.local_addr().unwrap();
    let state = ServerState {
        udid: DEVICE.into(),
        timeout: Duration::from_secs(1),
        cdp_host: "127.0.0.1".to_string(),
        cdp_port: bound.port(),
        access: AccessControl::from_file(policy.path.clone()).await.unwrap(),
        selenium_sessions: Sessions::default(),
        source,
    };
    let shape = if cdp {
        crate::http_diagnostics::ErrorShape::Cdp
    } else {
        crate::http_diagnostics::ErrorShape::WebDriver
    };
    let router = if cdp {
        cdp_router(state)
    } else {
        selenium_router(state)
    };
    let router = HttpBoundaryArgs::default()
        .validate()
        .unwrap()
        .for_listener(bound)
        .protect(router, shape);
    let task = tokio::spawn(async move {
        axum::serve(listener, router).await.unwrap();
    });
    (bound.port(), ServerTask(task), policy)
}

fn client() -> reqwest::Client {
    reqwest::Client::builder()
        .no_proxy()
        .timeout(BUDGET)
        .build()
        .unwrap()
}

#[tokio::test]
async fn injected_source_is_called_only_after_admission_and_failures_are_safe() {
    for cdp in [true, false] {
        let source = Arc::new(SyntheticSource {
            reject: true,
            close_at_startup: false,
            page_id: None,
            calls: Mutex::default(),
            tasks: Mutex::default(),
        });
        let (port, _server, _policy) = start(source.clone(), cdp).await;
        let url = format!(
            "http://127.0.0.1:{port}/{}",
            if cdp { "json/list" } else { "session" }
        );
        let client = client();
        let request = || {
            if cdp {
                client.get(&url)
            } else {
                client.post(&url)
            }
        };
        let denied = request().send().await.unwrap();
        assert_eq!(denied.status(), StatusCode::UNAUTHORIZED);
        assert_eq!(denied.headers()["x-ios-error-class"], "permission_denied");
        assert_eq!(denied.headers()["x-ios-error-phase"], "http_access");
        assert_eq!(source.calls(), 0);
        let failed = request().bearer_auth(TOKEN).send().await.unwrap();
        assert_eq!(failed.status(), StatusCode::INTERNAL_SERVER_ERROR);
        assert_eq!(failed.headers()["x-ios-error-schema-version"], "1");
        assert_eq!(failed.headers()["x-ios-error-class"], "permission_denied");
        assert_eq!(failed.headers()["x-ios-error-phase"], "webinspector");
        assert_eq!(failed.headers()["x-ios-error-effects"], "unknown");
        let raw = failed.text().await.unwrap();
        assert!(!raw.contains(SECRET));
        assert!(!raw.contains(DEVICE));
        let body: JsonValue = serde_json::from_str(&raw).unwrap();
        assert_eq!(body["value"]["error"], "unknown error");
        assert!(body["value"]["message"]
            .as_str()
            .unwrap()
            .contains("io/PermissionDenied"));
        assert_eq!(source.calls(), 1);
        assert!(source.finish().await.is_empty());
    }
}

#[tokio::test]
async fn actual_debug_routers_preserve_startup_io_and_existing_error_response_shapes() {
    for cdp in [true, false] {
        let source = Arc::new(SyntheticSource {
            reject: false,
            close_at_startup: true,
            page_id: None,
            calls: Mutex::default(),
            tasks: Mutex::default(),
        });
        let (port, _server, _policy) = start(source.clone(), cdp).await;
        let url = format!(
            "http://127.0.0.1:{port}/{}",
            if cdp { "json/list" } else { "session" }
        );
        let client = client();
        let request = if cdp {
            client.get(&url)
        } else {
            client.post(&url)
        };
        let response = request.bearer_auth(TOKEN).send().await.unwrap();
        assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
        assert_eq!(response.headers()["x-ios-error-schema-version"], "1");
        assert_eq!(response.headers()["x-ios-error-class"], "disconnected");
        assert_eq!(response.headers()["x-ios-error-phase"], "webinspector");
        assert_eq!(response.headers()["x-ios-error-effects"], "unknown");
        let raw = response.text().await.unwrap();
        assert!(!raw.contains(DEVICE));
        assert!(!raw.contains(SECRET));
        let body: JsonValue = serde_json::from_str(&raw).unwrap();
        assert_eq!(body["value"]["error"], "unknown error");
        assert!(body["value"]["message"]
            .as_str()
            .unwrap()
            .contains("io/BrokenPipe"));
        assert_eq!(source.calls(), 1);
        assert!(source.finish().await.is_empty());
    }
}

type Socket =
    tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;

async fn socket(port: u16, path: &str) -> Socket {
    let mut request = format!("ws://127.0.0.1:{port}/{path}")
        .into_client_request()
        .unwrap();
    request
        .headers_mut()
        .insert("authorization", format!("Bearer {TOKEN}").parse().unwrap());
    tokio::time::timeout(BUDGET, tokio_tungstenite::connect_async(request))
        .await
        .unwrap()
        .unwrap()
        .0
}

async fn exchange(socket: &mut Socket, request: JsonValue) -> JsonValue {
    let id = request["id"].clone();
    socket
        .send(Message::Text(request.to_string()))
        .await
        .unwrap();
    tokio::time::timeout(BUDGET, async {
        // Bounded event skipping: Target notifications can precede a reply.
        for _ in 0..16 {
            let frame = socket.next().await.unwrap().unwrap().into_text().unwrap();
            let reply: JsonValue = serde_json::from_str(&frame).unwrap();
            if reply["id"] == id {
                return reply;
            }
        }
        panic!("CDP request did not receive its reply");
    })
    .await
    .unwrap()
}

#[tokio::test]
async fn injected_service_runs_real_page_bridge_runtime_and_teardown() {
    let source = Arc::new(SyntheticSource::default());
    let (port, _server, _policy) = start(source.clone(), true).await;
    let response = client()
        .get(format!("http://127.0.0.1:{port}/json/list"))
        .bearer_auth(TOKEN)
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let pages: JsonValue = serde_json::from_str(&response.text().await.unwrap()).unwrap();
    assert_eq!(pages.as_array().unwrap().len(), 1);
    assert_eq!(pages[0]["id"], TARGET);
    assert_eq!(
        pages[0]["webSocketDebuggerUrl"],
        format!("ws://127.0.0.1:{port}/devtools/page/{TARGET}")
    );
    let mut socket = socket(port, &format!("devtools/page/{TARGET}")).await;
    let reply = exchange(
        &mut socket,
        json!({"id": 91, "method": "Runtime.evaluate", "params": {"expression": "6*7"}}),
    )
    .await;
    assert_eq!(
        reply,
        json!({"id": 91, "result": {"result": {"type": "number", "value": 42}}})
    );
    let failure = exchange(
        &mut socket,
        json!({"id": 92, "method": "Runtime.syntheticFailure", "params": {}}),
    )
    .await;
    assert_eq!(failure["id"], 92);
    assert_eq!(failure["error"]["code"], -32001);
    assert!(!failure.to_string().contains(SECRET));
    socket.close(None).await.unwrap();
    drop(socket);
    assert_eq!(source.calls(), 2);
    let traces = source.finish().await;
    assert_eq!(traces.len(), 2);
    assert!(traces
        .iter()
        .all(|trace| trace.identifiers == 1 && trace.discovery == 1 && trace.listings == 1));
    assert_eq!(traces.iter().map(|t| t.setups).sum::<usize>(), 1);
    assert_eq!(traces.iter().map(|t| t.evaluations).sum::<usize>(), 1);
    assert_eq!(traces.iter().map(|t| t.failures).sum::<usize>(), 1);
    assert_eq!(traces.iter().map(|t| t.teardowns).sum::<usize>(), 1);
}

#[tokio::test]
async fn injected_service_runs_real_browser_bridge_flattened_session_and_detach() {
    // The page and browser fixtures run in parallel. Distinct synthetic pages
    // keep them from contending on the production process-wide page lock.
    let target = "PID:42:page-2";
    let source = Arc::new(SyntheticSource {
        reject: false,
        close_at_startup: false,
        page_id: Some(2),
        calls: Mutex::default(),
        tasks: Mutex::default(),
    });
    let (port, _server, _policy) = start(source.clone(), true).await;
    let mut socket = socket(port, "devtools/browser/ios-cli").await;
    let targets = exchange(
        &mut socket,
        json!({"id": 1, "method": "Target.getTargets", "params": {}}),
    )
    .await;
    assert_eq!(
        targets["result"]["targetInfos"].as_array().unwrap().len(),
        1
    );
    assert_eq!(targets["result"]["targetInfos"][0]["targetId"], target);
    let attached = exchange(&mut socket, json!({"id": 2, "method": "Target.attachToTarget", "params": {"targetId": target, "flatten": true}})).await;
    let session = attached["result"]["sessionId"].as_str().unwrap();
    let reply = exchange(&mut socket, json!({"id": 3, "sessionId": session, "method": "Runtime.evaluate", "params": {"expression": "6*7"}})).await;
    assert_eq!(
        reply,
        json!({"id": 3, "sessionId": session, "result": {"result": {"type": "number", "value": 42}}})
    );
    let detached = exchange(
        &mut socket,
        json!({"id": 4, "method": "Target.detachFromTarget", "params": {"sessionId": session}}),
    )
    .await;
    assert_eq!(detached, json!({"id": 4, "result": {}}));
    socket.close(None).await.unwrap();
    drop(socket);
    let traces = source.finish().await;
    assert_eq!(source.calls(), traces.len());
    assert!(traces.len() >= 4);
    assert!(traces
        .iter()
        .all(|trace| trace.identifiers == 1 && trace.discovery == 1 && trace.listings == 1));
    assert_eq!(traces.iter().map(|t| t.setups).sum::<usize>(), 1);
    assert_eq!(traces.iter().map(|t| t.evaluations).sum::<usize>(), 1);
    assert_eq!(traces.iter().map(|t| t.teardowns).sum::<usize>(), 1);
}

pub(super) fn envelope(selector: &str, argument: Dictionary) -> Value {
    Value::Dictionary(Dictionary::from_iter([
        ("__selector".to_string(), Value::String(selector.into())),
        ("__argument".to_string(), Value::Dictionary(argument)),
    ]))
}

pub(super) fn strings(entries: &[(&str, &str)]) -> Dictionary {
    entries
        .iter()
        .map(|(k, v)| ((*k).to_string(), Value::String((*v).into())))
        .collect()
}

pub(super) async fn write_frame(stream: &mut DuplexStream, value: Value) -> Result<()> {
    let mut bytes = Vec::new();
    plist::to_writer_xml(&mut bytes, &value)?;
    stream
        .write_all(&u32::try_from(bytes.len())?.to_be_bytes())
        .await?;
    stream.write_all(&bytes).await?;
    Ok(())
}

pub(super) async fn read_frame(stream: &mut DuplexStream) -> Result<Option<Dictionary>> {
    let mut first = [0];
    if stream.read(&mut first).await? == 0 {
        return Ok(None);
    }
    let mut length = [first[0], 0, 0, 0];
    stream.read_exact(&mut length[1..]).await?;
    let length = u32::from_be_bytes(length) as usize;
    if length > 1024 * 1024 {
        return Err(anyhow!("oversized synthetic service request"));
    }
    let mut bytes = vec![0; length];
    stream.read_exact(&mut bytes).await?;
    Ok(Some(plist::from_bytes(&bytes)?))
}

pub(super) async fn send_json(stream: &mut DuplexStream, message: JsonValue) -> Result<()> {
    let mut argument = strings(&[("WIRApplicationIdentifierKey", "PID:42")]);
    argument.insert(
        "WIRMessageDataKey".to_string(),
        Value::Data(serde_json::to_vec(&message)?),
    );
    write_frame(stream, envelope("_rpc_applicationSentData:", argument)).await
}

async fn fake_service(mut stream: DuplexStream, page_id: u64) -> Result<Trace> {
    let mut trace = Trace::default();
    while let Some(message) = read_frame(&mut stream).await? {
        let selector = message
            .get("__selector")
            .and_then(Value::as_string)
            .ok_or_else(|| anyhow!("missing fixture selector"))?;
        let argument = message
            .get("__argument")
            .and_then(Value::as_dictionary)
            .ok_or_else(|| anyhow!("missing fixture argument"))?;
        match selector {
            "_rpc_reportIdentifier:" => {
                trace.identifiers += 1;
                assert!(argument
                    .get("WIRConnectionIdentifierKey")
                    .and_then(Value::as_string)
                    .is_some());
                write_frame(
                    &mut stream,
                    envelope(
                        "_rpc_reportCurrentState:",
                        strings(&[(
                            "WIRAutomationAvailabilityKey",
                            "WIRAutomationAvailabilityAvailable",
                        )]),
                    ),
                )
                .await?;
            }
            "_rpc_getConnectedApplications:" => {
                trace.discovery += 1;
                let mut app = strings(&[
                    ("WIRApplicationIdentifierKey", "PID:42"),
                    (
                        "WIRApplicationBundleIdentifierKey",
                        "com.apple.mobilesafari",
                    ),
                    ("WIRApplicationNameKey", "Safari"),
                    (
                        "WIRAutomationAvailabilityKey",
                        "WIRAutomationAvailabilityAvailable",
                    ),
                ]);
                for (key, value) in [
                    ("WIRIsApplicationActiveKey", true),
                    ("WIRIsApplicationProxyKey", false),
                    ("WIRIsApplicationReadyKey", true),
                ] {
                    app.insert(key.to_string(), Value::Boolean(value));
                }
                let apps = Dictionary::from_iter([("PID:42".to_string(), Value::Dictionary(app))]);
                write_frame(
                    &mut stream,
                    envelope(
                        "_rpc_reportConnectedApplicationList:",
                        Dictionary::from_iter([(
                            "WIRApplicationDictionaryKey".to_string(),
                            Value::Dictionary(apps),
                        )]),
                    ),
                )
                .await?;
            }
            "_rpc_forwardGetListing:" => {
                trace.listings += 1;
                assert_eq!(
                    argument
                        .get("WIRApplicationIdentifierKey")
                        .and_then(Value::as_string),
                    Some("PID:42")
                );
                let mut page = strings(&[
                    ("WIRTypeKey", "WIRTypeWebPage"),
                    ("WIRTitleKey", "Synthetic page"),
                    ("WIRURLKey", "https://example.invalid/"),
                ]);
                page.insert(
                    "WIRPageIdentifierKey".to_string(),
                    Value::Integer(page_id.into()),
                );
                let mut listing = strings(&[("WIRApplicationIdentifierKey", "PID:42")]);
                listing.insert(
                    "WIRListingKey".to_string(),
                    Value::Dictionary(Dictionary::from_iter([(
                        format!("page-{page_id}"),
                        Value::Dictionary(page),
                    )])),
                );
                write_frame(
                    &mut stream,
                    envelope("_rpc_applicationSentListing:", listing),
                )
                .await?;
            }
            "_rpc_forwardSocketSetup:" => {
                trace.setups += 1;
                assert_eq!(
                    argument
                        .get("WIRAutomaticallyPause")
                        .and_then(Value::as_boolean),
                    Some(false)
                );
                assert_eq!(
                    argument
                        .get("WIRPageIdentifierKey")
                        .and_then(Value::as_unsigned_integer),
                    Some(page_id)
                );
                send_json(&mut stream, json!({"method": "Target.targetCreated", "params": {"targetInfo": {"targetId": "synthetic-web-target"}}})).await?;
            }
            "_rpc_forwardSocketData:" => {
                let bytes = argument
                    .get("WIRSocketDataKey")
                    .and_then(Value::as_data)
                    .ok_or_else(|| anyhow!("missing fixture socket bytes"))?;
                let wire: JsonValue = serde_json::from_slice(bytes)?;
                assert_eq!(wire["method"], "Target.sendMessageToTarget");
                assert_eq!(wire["params"]["targetId"], "synthetic-web-target");
                let nested: JsonValue =
                    serde_json::from_str(wire["params"]["message"].as_str().unwrap())?;
                let reply = match nested["method"].as_str() {
                    Some("Runtime.evaluate") => {
                        trace.evaluations += 1;
                        assert_eq!(nested["params"]["expression"], "6*7");
                        json!({"id": nested["id"], "result": {"result": {"type": "number", "value": 42}}})
                    }
                    Some("Runtime.syntheticFailure") => {
                        trace.failures += 1;
                        json!({"id": nested["id"], "error": {"code": -32001, "message": SECRET, "data": SECRET}})
                    }
                    _ => return Err(anyhow!("unexpected synthetic Runtime request")),
                };
                send_json(&mut stream, json!({"id": wire["id"], "result": {}})).await?;
                send_json(&mut stream, json!({"method": "Target.dispatchMessageFromTarget", "params": {"targetId": "synthetic-web-target", "message": reply.to_string()}})).await?;
            }
            "_rpc_forwardDidClose:" => {
                trace.teardowns += 1;
            }
            _ => return Err(anyhow!("unexpected synthetic service request")),
        }
    }
    Ok(trace)
}
