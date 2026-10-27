//! Owned WebInspector JSContext peer and actual-client process supervision.
//! Every CDP response still comes from the production protected Rust router.

use super::*;
use std::path::PathBuf;
use std::process::{ExitStatus, Stdio};
use tokio::sync::{oneshot, watch, Notify};

pub(super) const CLIENT_BUDGET: Duration = Duration::from_secs(20);
pub(super) const CLEANUP_BUDGET: Duration = Duration::from_secs(5);
const MAX_RECORDS: usize = 512;
const MAX_TRACE_BYTES: usize = 256 * 1024;
const MAX_STDOUT_BYTES: usize = 512 * 1024;
const MAX_STDERR_BYTES: usize = 64 * 1024;
const MAX_STREAMS: usize = 32;

#[derive(Default)]
pub(super) struct Evidence {
    records: Vec<JsonValue>,
    bytes: usize,
}

pub(super) type SharedEvidence = Arc<Mutex<Evidence>>;

pub(super) fn record(evidence: &SharedEvidence, event: JsonValue) -> Result<()> {
    let bytes = serde_json::to_vec(&event)?.len();
    let mut evidence = evidence.lock().unwrap();
    if evidence.records.len() >= MAX_RECORDS
        || bytes > MAX_TRACE_BYTES.saturating_sub(evidence.bytes)
    {
        return Err(anyhow!("owned client evidence limit"));
    }
    evidence.bytes += bytes;
    evidence.records.push(event);
    Ok(())
}

pub(super) fn snapshot(evidence: &SharedEvidence) -> JsonValue {
    let evidence = evidence.lock().unwrap();
    json!({"records": evidence.records, "bytes": evidence.bytes})
}

#[derive(Clone, Copy)]
pub(super) struct JsTarget {
    pub(super) page: u64,
    pub(super) listing: &'static str,
    pub(super) url: &'static str,
    pub(super) is_web: bool,
}

pub(super) struct JsSource {
    pub(super) evidence: SharedEvidence,
    pub(super) runtime_seen: Arc<Notify>,
    target: JsTarget,
    block_runtime: bool,
    closure: Arc<ClosureState>,
    calls: Mutex<usize>,
    tasks: Mutex<Vec<JoinHandle<Result<PeerReceipt>>>>,
}

#[derive(Clone, Copy)]
pub(super) enum NativeClosure {
    Page,
    Application,
    DiscoveryPage,
    DiscoveryApplication,
    DiscoveryRace,
}

struct ClosureState {
    mode: Option<NativeClosure>,
    closed: std::sync::atomic::AtomicBool,
}

pub(super) struct PeerReceipt {
    pub(super) stream: usize,
    pub(super) setups: usize,
    pub(super) runtime_enables: usize,
    pub(super) console_enables: usize,
    pub(super) evaluations: usize,
    pub(super) teardowns: usize,
    pub(super) eof: bool,
}

impl JsSource {
    pub(super) fn new(target: JsTarget, block_runtime: bool) -> Arc<Self> {
        Arc::new(Self {
            evidence: SharedEvidence::default(),
            runtime_seen: Arc::new(Notify::new()),
            target,
            block_runtime,
            closure: Arc::new(ClosureState {
                mode: None,
                closed: std::sync::atomic::AtomicBool::new(false),
            }),
            calls: Mutex::new(0),
            tasks: Mutex::default(),
        })
    }

    pub(super) fn with_closure(target: JsTarget, closure: NativeClosure) -> Arc<Self> {
        let mut source = Self::new(target, false);
        Arc::get_mut(&mut source).unwrap().closure = Arc::new(ClosureState {
            mode: Some(closure),
            closed: std::sync::atomic::AtomicBool::new(false),
        });
        source
    }

    pub(super) fn calls(&self) -> usize {
        *self.calls.lock().unwrap()
    }

    /// Join original peers; EOF is recorded only by an actual zero-byte read.
    pub(super) async fn finish(&self) -> Result<Vec<PeerReceipt>> {
        let tasks = std::mem::take(&mut *self.tasks.lock().unwrap());
        let mut receipts = Vec::new();
        let mut failure = None;
        for mut task in tasks {
            let result = match tokio::time::timeout(CLEANUP_BUDGET, &mut task).await {
                Ok(joined) => joined
                    .map_err(anyhow::Error::from)
                    .and_then(|result| result),
                Err(error) => {
                    task.abort();
                    let _ = task.await;
                    Err(anyhow::Error::new(error).context("owned WebInspector peer join"))
                }
            };
            match result {
                Ok(receipt) => {
                    if let Err(error) = record(
                        &self.evidence,
                        json!({"event": "peer_joined", "stream": receipt.stream}),
                    ) {
                        if failure.is_none() {
                            failure = Some(error);
                        }
                    }
                    receipts.push(receipt);
                }
                Err(error) => {
                    if failure.is_none() {
                        failure = Some(error);
                    }
                }
            }
        }
        match failure {
            Some(error) => Err(error),
            None => Ok(receipts),
        }
    }
}

/// Axum's upgrade task owns its cloned production ServerState independently
/// of the listener. Wait for that owner to release the source before draining
/// peer handles, so detach/auto-attach cannot create an unjoined late stream.
pub(super) async fn source_owners_released(source: &Arc<JsSource>) -> Result<()> {
    tokio::time::timeout(CLEANUP_BUDGET, async {
        while Arc::strong_count(source) != 1 {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await?;
    record(&source.evidence, json!({"event": "source_owners_released"}))
}

impl InspectorSource for JsSource {
    fn connect<'a>(&'a self, udid: &'a str, _timeout: Duration) -> ConnectionFuture<'a> {
        Box::pin(async move {
            if udid != DEVICE {
                return Err(anyhow!("owned JSContext source target mismatch"));
            }
            let id = {
                let mut calls = self.calls.lock().unwrap();
                if *calls >= MAX_STREAMS {
                    return Err(anyhow!("owned JSContext stream limit"));
                }
                *calls += 1;
                *calls
            };
            record(
                &self.evidence,
                json!({"event": "source_connected", "stream": id}),
            )?;
            let (stream, peer) = tokio::io::duplex(64 * 1024);
            self.tasks.lock().unwrap().push(tokio::spawn(js_peer(
                peer,
                id,
                self.target,
                self.block_runtime,
                self.closure.clone(),
                self.runtime_seen.clone(),
                self.evidence.clone(),
            )));
            Ok(InspectorConnection {
                device: None,
                stream: Box::new(stream),
            })
        })
    }
}

impl Drop for JsSource {
    fn drop(&mut self) {
        // Explicit finish owns joins. This fallback cannot count as EOF proof.
        for task in self.tasks.get_mut().unwrap().drain(..) {
            task.abort();
        }
    }
}

async fn native_send(
    stream: &mut DuplexStream,
    id: usize,
    evidence: &SharedEvidence,
    message: Value,
) -> Result<()> {
    record(
        evidence,
        json!({"event": "plist_send", "stream": id, "message": serde_json::to_value(&message)?}),
    )?;
    write_frame(stream, message).await
}

async fn runtime_send(
    stream: &mut DuplexStream,
    id: usize,
    evidence: &SharedEvidence,
    message: JsonValue,
) -> Result<()> {
    record(
        evidence,
        json!({"event": "runtime_send", "stream": id, "message": message}),
    )?;
    let mut argument = strings(&[("WIRApplicationIdentifierKey", "PID:42")]);
    argument.insert(
        "WIRMessageDataKey".into(),
        Value::Data(serde_json::to_vec(&message)?),
    );
    native_send(
        stream,
        id,
        evidence,
        envelope("_rpc_applicationSentData:", argument),
    )
    .await
}

async fn js_peer(
    mut stream: DuplexStream,
    id: usize,
    target: JsTarget,
    block_runtime: bool,
    closure_state: Arc<ClosureState>,
    runtime_seen: Arc<Notify>,
    evidence: SharedEvidence,
) -> Result<PeerReceipt> {
    let closure = closure_state.mode;
    let mut navigated_frames = std::collections::HashMap::<String, String>::new();
    let mut receipt = PeerReceipt {
        stream: id,
        setups: 0,
        runtime_enables: 0,
        console_enables: 0,
        evaluations: 0,
        teardowns: 0,
        eof: false,
    };
    while let Some(message) = read_frame(&mut stream).await? {
        record(
            &evidence,
            json!({"event": "plist_receive", "stream": id, "message": serde_json::to_value(&message)?}),
        )?;
        let selector = message
            .get("__selector")
            .and_then(Value::as_string)
            .ok_or_else(|| anyhow!("owned JSContext selector absent"))?;
        let argument = message
            .get("__argument")
            .and_then(Value::as_dictionary)
            .ok_or_else(|| anyhow!("owned JSContext argument absent"))?;
        match selector {
            "_rpc_reportIdentifier:" => {
                if argument
                    .get("WIRConnectionIdentifierKey")
                    .and_then(Value::as_string)
                    .is_none()
                {
                    return Err(anyhow!("owned JSContext connection identifier absent"));
                }
                native_send(
                    &mut stream,
                    id,
                    &evidence,
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
                let mut app = strings(&[
                    ("WIRApplicationIdentifierKey", "PID:42"),
                    (
                        "WIRApplicationBundleIdentifierKey",
                        "example.owned.jscontext",
                    ),
                    ("WIRApplicationNameKey", "Owned JSContext"),
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
                    app.insert(key.into(), Value::Boolean(value));
                }
                let apps = if matches!(closure, Some(NativeClosure::DiscoveryApplication))
                    && closure_state
                        .closed
                        .load(std::sync::atomic::Ordering::SeqCst)
                {
                    Dictionary::new()
                } else {
                    Dictionary::from_iter([("PID:42", Value::Dictionary(app))])
                };
                native_send(
                    &mut stream,
                    id,
                    &evidence,
                    envelope(
                        "_rpc_reportConnectedApplicationList:",
                        Dictionary::from_iter([(
                            "WIRApplicationDictionaryKey",
                            Value::Dictionary(apps),
                        )]),
                    ),
                )
                .await?;
            }
            "_rpc_forwardGetListing:" => {
                if argument
                    .get("WIRApplicationIdentifierKey")
                    .and_then(Value::as_string)
                    != Some("PID:42")
                {
                    return Err(anyhow!("owned JSContext listing application mismatch"));
                }
                let page_type = if target.is_web {
                    "WIRTypeWeb"
                } else {
                    "WIRTypeJavaScript"
                };
                let title = if target.is_web {
                    "Owned WebPage"
                } else {
                    "Owned JSContext"
                };
                let mut page = strings(&[
                    ("WIRTypeKey", page_type),
                    ("WIRTitleKey", title),
                    ("WIRURLKey", target.url),
                ]);
                page.insert(
                    "WIRPageIdentifierKey".into(),
                    Value::Integer(target.page.into()),
                );
                let mut listing = strings(&[("WIRApplicationIdentifierKey", "PID:42")]);
                listing.insert(
                    "WIRListingKey".into(),
                    Value::Dictionary(
                        if matches!(
                            closure,
                            Some(NativeClosure::DiscoveryPage | NativeClosure::DiscoveryRace)
                        ) && closure_state
                            .closed
                            .load(std::sync::atomic::Ordering::SeqCst)
                        {
                            Dictionary::new()
                        } else {
                            Dictionary::from_iter([(target.listing, Value::Dictionary(page))])
                        },
                    ),
                );
                native_send(
                    &mut stream,
                    id,
                    &evidence,
                    envelope("_rpc_applicationSentListing:", listing),
                )
                .await?;
            }
            "_rpc_forwardSocketSetup:" => {
                if argument
                    .get("WIRAutomaticallyPause")
                    .and_then(Value::as_boolean)
                    != Some(false)
                    || argument
                        .get("WIRPageIdentifierKey")
                        .and_then(Value::as_unsigned_integer)
                        != Some(target.page)
                    || receipt.setups != 0
                {
                    return Err(anyhow!("owned JSContext setup mismatch"));
                }
                receipt.setups += 1;
                if target.is_web {
                    runtime_send(
                        &mut stream,
                        id,
                        &evidence,
                        json!({
                            "method": "Target.targetCreated",
                            "params": {
                                "targetInfo": {
                                    "targetId": format!("target-{}", target.page),
                                    "type": "page",
                                    "title": "Owned WebPage",
                                    "url": target.url,
                                    "isProvisional": false
                                }
                            }
                        }),
                    )
                    .await?;
                }
            }
            "_rpc_forwardSocketData:" => {
                if receipt.setups != 1 || receipt.teardowns != 0 {
                    return Err(anyhow!("owned JSContext socket data outside attachment"));
                }
                let bytes = argument
                    .get("WIRSocketDataKey")
                    .and_then(Value::as_data)
                    .ok_or_else(|| anyhow!("owned JSContext socket data absent"))?;
                let wire: JsonValue = serde_json::from_slice(bytes)?;
                record(
                    &evidence,
                    json!({"event": "runtime_receive", "stream": id, "message": wire}),
                )?;
                let nested = wire["method"] == "Target.sendMessageToTarget";
                let (command, target_id) = if nested {
                    let cmd_str = wire["params"]["message"]
                        .as_str()
                        .ok_or_else(|| anyhow!("missing inner message"))?;
                    let cmd: JsonValue = serde_json::from_str(cmd_str)?;
                    let tid = wire["params"]["targetId"]
                        .as_str()
                        .unwrap_or("")
                        .to_string();
                    (cmd, Some(tid))
                } else {
                    (wire.clone(), None)
                };
                let cmd_id = &command["id"];
                let reply = match command["method"].as_str() {
                    Some("Runtime.enable") => {
                        receipt.runtime_enables += 1;
                        runtime_seen.notify_one();
                        if block_runtime {
                            continue;
                        }
                        if target.is_web {
                            let evt = json!({
                                "method": "Runtime.executionContextCreated",
                                "params": {
                                    "context": {
                                        "id": 1,
                                        "type": "normal",
                                        "frameId": "1.1",
                                        "name": "",
                                    }
                                }
                            });
                            let dispatch = if nested {
                                json!({
                                    "method": "Target.dispatchMessageFromTarget",
                                    "params": {
                                        "targetId": target_id.as_deref().unwrap_or(""),
                                        "message": serde_json::to_string(&evt)?
                                    }
                                })
                            } else {
                                evt
                            };
                            runtime_send(&mut stream, id, &evidence, dispatch).await?;
                            for (context, frame) in [(101, "1.3"), (102, "1.5")] {
                                let evt = json!({"method":"Runtime.executionContextCreated", "params":{
                                    "context":{"id":context,"type":"normal","frameId":frame,"name":""}
                                }});
                                let dispatch = if nested {
                                    json!({"method":"Target.dispatchMessageFromTarget","params":{
                                        "targetId":target_id.as_deref().unwrap_or(""), "message":serde_json::to_string(&evt)?
                                    }})
                                } else {
                                    evt
                                };
                                runtime_send(&mut stream, id, &evidence, dispatch).await?;
                            }
                        }
                        json!({"id": cmd_id, "result": {}})
                    }
                    Some("Console.enable") => {
                        receipt.console_enables += 1;
                        json!({"id": cmd_id, "result": {}})
                    }
                    Some(
                        "Memory.enable" | "Memory.disable" | "Memory.stopTracking" | "Heap.gc",
                    ) => json!({"id":cmd_id,"result":{}}),
                    Some("Memory.startTracking") => {
                        let evt = json!({"method":"Memory.trackingUpdate","params":{"event":{
                            "timestamp":12.5,"categories":[{"type":"javascript","size":4096},{"type":"images","size":1024}]
                        }}});
                        let dispatch = if nested {
                            json!({"method":"Target.dispatchMessageFromTarget","params":{
                                "targetId":target_id.as_deref().unwrap_or(""),"message":serde_json::to_string(&evt)?
                            }})
                        } else {
                            evt
                        };
                        runtime_send(&mut stream, id, &evidence, dispatch).await?;
                        json!({"id":cmd_id,"result":{}})
                    }
                    Some("Runtime.runIfWaitingForDebugger") => {
                        json!({"id": cmd_id, "result": {}})
                    }
                    Some("Page.enable" | "Page.disable" | "Page.reload") => {
                        json!({"id": cmd_id, "result": {}})
                    }
                    Some("Page.getResourceTree") => {
                        let mut reply = json!({
                            "id": cmd_id,
                            "result": {
                                "frameTree": {
                                    "frame": {
                                        "id": "1.1",
                                        "loaderId": "1.2",
                                        "url": target.url,
                                        "securityOrigin": "https://example.invalid"
                                    },
                                    "resources": [],
                                    "childFrames": [
                                        {
                                            "frame": {
                                                "id": "1.3",
                                                "parentId": "1.1",
                                                "loaderId": "1.4",
                                                "url": "https://example.invalid/child",
                                                "securityOrigin": "https://example.invalid"
                                            },
                                            "resources": [],
                                            "childFrames": []
                                        },
                                        {"frame":{"id":"1.5","parentId":"1.1","loaderId":"1.6",
                                            "url":"https://cross.invalid/child","securityOrigin":"https://cross.invalid"},
                                            "resources":[],"childFrames":[]}
                                    ]
                                }
                            }
                        });
                        if let Some(url) = navigated_frames.get("1.1") {
                            reply["result"]["frameTree"]["frame"]["url"] = json!(url);
                            reply["result"]["frameTree"]["frame"]["loaderId"] = json!("new-loader");
                        }
                        for child in reply["result"]["frameTree"]["childFrames"]
                            .as_array_mut()
                            .unwrap()
                        {
                            if let Some(url) =
                                navigated_frames.get(child["frame"]["id"].as_str().unwrap())
                            {
                                child["frame"]["url"] = json!(url);
                                child["frame"]["loaderId"] = json!("new-loader");
                            }
                        }
                        reply
                    }
                    Some("Page.snapshotRect") => {
                        json!({
                            "id": cmd_id,
                            "result": {
                                "dataURL": "data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mNk+M9QDwADhgGAWjR9awAAAABJRU5ErkJggg=="
                            }
                        })
                    }
                    Some(
                        "Network.addInterception"
                        | "Network.removeInterception"
                        | "Network.setInterceptionEnabled"
                        | "Network.interceptContinue",
                    ) => {
                        json!({"id": cmd_id, "result": {}})
                    }
                    Some(
                        "Network.interceptRequestWithError"
                        | "Network.interceptRequestWithResponse"
                        | "Network.interceptWithResponse"
                        | "Network.interceptWithRequest",
                    ) => {
                        match command["method"].as_str().unwrap() {
                            "Network.interceptWithRequest" => {
                                assert_eq!(command["params"]["method"], "POST");
                                assert_eq!(command["params"]["postData"], "b2s=");
                                assert_eq!(command["params"]["headers"]["X-Fixture"], "request");
                            }
                            "Network.interceptRequestWithError" => {
                                assert_eq!(command["params"]["errorType"], "Cancellation")
                            }
                            _ => {
                                assert_eq!(command["params"]["status"], 201);
                                assert_eq!(command["params"]["content"], "b2s=");
                                assert_eq!(command["params"]["base64Encoded"], true);
                            }
                        }
                        json!({"id": cmd_id, "result": {}})
                    }
                    Some("Runtime.evaluate") => {
                        if command["params"]
                            .get("contextId")
                            .and_then(JsonValue::as_u64)
                            .is_some_and(|id| ![1, 101, 102,201,202].contains(&id))
                        {
                            json!({"id": cmd_id, "error": {"code": -32602, "message": "Cannot find context with specified id"}})
                        } else if let Some(kind) = command["params"]["expression"].as_str().and_then(|s| s.strip_prefix("fixture:fetch:")) {
                            let request_id = format!("native-{kind}");
                            let request = json!({"url":"https://example.invalid/resource","method":"GET","headers":{}});
                            let metadata = json!({"method":"Network.requestWillBeSent","params":{
                                "requestId":request_id,"frameId":"1.1","type":"Fetch","request":request
                            }});
                            let paused = if kind == "response" || kind == "cancelled" {
                                json!({"method":"Network.responseIntercepted","params":{"requestId":request_id,
                                    "response":{"status":200,"statusText":"OK","headers":{"Content-Type":"text/plain"}}
                                }})
                            } else {
                                json!({"method":"Network.requestIntercepted","params":{"requestId":request_id,"request":request}})
                            };
                            for evt in [metadata,paused] {
                                let dispatch = if nested {
                                    json!({"method":"Target.dispatchMessageFromTarget","params":{
                                        "targetId":target_id.as_deref().unwrap_or(""),"message":serde_json::to_string(&evt)?
                                    }})
                                } else { evt };
                                runtime_send(&mut stream, id, &evidence, dispatch).await?;
                            }
                            json!({"id":cmd_id,"result":{"result":{"type":"undefined"}}})
                        } else if let Some(url) = command["params"]["expression"].as_str()
                            .and_then(|s| s.strip_prefix("window.location.assign("))
                            .and_then(|s| s.strip_suffix(')')) {
                            let url: String = serde_json::from_str(url)?;
                            let frame = match command["params"]["contextId"].as_u64() {
                                Some(101) => "1.3", Some(102) => "1.5", _ => "1.1",
                            };
                            navigated_frames.insert(frame.to_owned(),url.clone());
                            let evt = json!({"method":"Page.frameNavigated","params":{
                                "frame":{"id":frame,"parentId":"1.1","url":url,"loaderId":"new-loader",
                                    "securityOrigin":if frame=="1.5" {"https://cross.invalid"} else {"https://example.invalid"}}
                            }});
                            let dispatch = if nested {
                                json!({"method":"Target.dispatchMessageFromTarget","params":{
                                    "targetId":target_id.as_deref().unwrap_or(""),"message":serde_json::to_string(&evt)?
                                }})
                            } else { evt };
                            runtime_send(&mut stream, id, &evidence, dispatch).await?;
                            let context_id=if frame=="1.3" {201} else if frame=="1.5" {202} else {1};
                            let evt=json!({"method":"Runtime.executionContextCreated","params":{"context":{
                                "id":context_id,"type":"normal","name":"","frameId":frame
                            }}});
                            let dispatch=if nested {
                                json!({"method":"Target.dispatchMessageFromTarget","params":{
                                    "targetId":target_id.as_deref().unwrap_or(""),"message":serde_json::to_string(&evt)?
                                }})
                            } else {evt};
                            runtime_send(&mut stream,id,&evidence,dispatch).await?;
                            json!({"id":cmd_id,"result":{"result":{"type":"undefined"}}})
                        } else if command["params"]["expression"] == "fixture:console-log" {
                            receipt.evaluations += 1;
                            let evt = json!({
                                "method": "Console.messageAdded",
                                "params": {
                                    "message": {
                                        "source": "console-api",
                                        "level": "info",
                                        "text": "test console log from native",
                                        "type": "log",
                                        "timestamp": 12.345
                                    }
                                }
                            });
                            let dispatch = if nested {
                                json!({
                                    "method": "Target.dispatchMessageFromTarget",
                                    "params": {
                                        "targetId": target_id.as_deref().unwrap_or(""),
                                        "message": serde_json::to_string(&evt)?
                                    }
                                })
                            } else {
                                evt
                            };
                            runtime_send(&mut stream, id, &evidence, dispatch).await?;
                            json!({"id": cmd_id, "result": {"result": {"type": "string", "value": "logged"}}})
                        } else if command["params"]["expression"]
                            == "({x:window.scrollX,y:window.scrollY,width:window.innerWidth,height:window.innerHeight})"
                        {
                            json!({
                                "id": cmd_id,
                                "result": {
                                    "result": {
                                        "type": "object",
                                        "value": {
                                            "x": 0,
                                            "y": 0,
                                            "width": 800,
                                            "height": 600
                                        }
                                    }
                                }
                            })
                        } else if command["params"]["expression"] == "6*7"
                            || (closure.is_some()
                                && matches!(
                                    command["params"]["expression"].as_str(),
                                    Some("fixture:unrelated" | "fixture:native-close")
                                ))
                        {
                            receipt.evaluations += 1;
                            json!({"id": cmd_id, "result": {"result": {"type": "number", "value": 42}}})
                        } else {
                            return Err(anyhow!("owned JSContext evaluate translation mismatch"));
                        }
                    }
                    // Puppeteer asks an auto-attached session to attach children.
                    // This native JSContext has no Target domain: preserve rejection.
                    Some("Target.setAutoAttach") => {
                        json!({"id": cmd_id, "error": {"code": -32601, "message": "Target domain absent in owned JSContext"}})
                    }
                    _ => {
                        return Err(anyhow!(
                            "unexpected owned JSContext Runtime request: {}",
                            command["method"]
                        ))
                    }
                };
                if command["params"]["expression"] == "fixture:unrelated" {
                    // Unrelated application closure and a complete selected-app
                    // listing retaining the numeric page must not close it.
                    native_send(
                        &mut stream,
                        id,
                        &evidence,
                        envelope(
                            "_rpc_applicationDisconnected:",
                            strings(&[("WIRApplicationIdentifierKey", "PID:99")]),
                        ),
                    )
                    .await?;
                    native_send(
                        &mut stream,
                        id,
                        &evidence,
                        native_listing("PID:99", target.page, "unrelated-key"),
                    )
                    .await?;
                    native_send(
                        &mut stream,
                        id,
                        &evidence,
                        native_listing("PID:42", target.page, "renamed-listing-key"),
                    )
                    .await?;
                    let mut other = strings(&[("WIRApplicationIdentifierKey", "PID:99")]);
                    other.insert("WIRMessageDataKey".into(), Value::Data(serde_json::to_vec(&json!({"id": wire["id"], "result": {"result": {"type": "number", "value": -99}}}))?));
                    native_send(
                        &mut stream,
                        id,
                        &evidence,
                        envelope("_rpc_applicationSentData:", other),
                    )
                    .await?;
                }
                if nested {
                    runtime_send(
                        &mut stream,
                        id,
                        &evidence,
                        json!({"id": wire["id"], "result": {}}),
                    )
                    .await?;
                    runtime_send(
                        &mut stream,
                        id,
                        &evidence,
                        json!({
                            "method": "Target.dispatchMessageFromTarget",
                            "params": {
                                "targetId": target_id.as_deref().unwrap_or(""),
                                "message": serde_json::to_string(&reply)?
                            }
                        }),
                    )
                    .await?;
                } else {
                    runtime_send(&mut stream, id, &evidence, reply).await?;
                }
                if command["params"]["expression"] == "fixture:native-close" {
                    closure_state
                        .closed
                        .store(true, std::sync::atomic::Ordering::SeqCst);
                    if matches!(
                        closure,
                        Some(NativeClosure::DiscoveryPage | NativeClosure::DiscoveryApplication)
                    ) {
                        // Only a fresh, independent discovery socket observes closure.
                        continue;
                    }
                    let close =
                        match closure.ok_or_else(|| anyhow!("native closure mode absent"))? {
                            NativeClosure::Page | NativeClosure::DiscoveryRace => {
                                native_listing("PID:42", target.page + 1, "surviving-sibling")
                            }
                            NativeClosure::Application => envelope(
                                "_rpc_applicationDisconnected:",
                                strings(&[("WIRApplicationIdentifierKey", "PID:42")]),
                            ),
                            NativeClosure::DiscoveryPage | NativeClosure::DiscoveryApplication => {
                                unreachable!("independent discovery only")
                            }
                        };
                    native_send(&mut stream, id, &evidence, close).await?;
                }
            }
            "_rpc_forwardDidClose:" => {
                if receipt.setups != 1 || receipt.teardowns != 0 {
                    return Err(anyhow!("owned JSContext teardown mismatch"));
                }
                receipt.teardowns += 1;
            }
            _ => return Err(anyhow!("unexpected owned JSContext plist request")),
        }
    }
    receipt.eof = true;
    record(&evidence, json!({"event": "peer_eof", "stream": id}))?;
    Ok(receipt)
}

fn native_listing(application_id: &str, page_id: u64, listing_key: &str) -> Value {
    let mut page = strings(&[("WIRTypeKey", "WIRTypeJavaScript")]);
    page.insert(
        "WIRPageIdentifierKey".into(),
        Value::Integer(page_id.into()),
    );
    let mut argument = strings(&[("WIRApplicationIdentifierKey", application_id)]);
    argument.insert(
        "WIRListingKey".into(),
        Value::Dictionary(Dictionary::from_iter([(
            listing_key,
            Value::Dictionary(page),
        )])),
    );
    envelope("_rpc_applicationSentListing:", argument)
}

pub(super) struct OwnedServer {
    task: Option<JoinHandle<std::io::Result<()>>>,
    shutdown: Option<oneshot::Sender<()>>,
}

impl OwnedServer {
    pub(super) async fn stop(mut self) -> Result<()> {
        if let Some(shutdown) = self.shutdown.take() {
            let _ = shutdown.send(());
        }
        let mut task = self.task.take().unwrap();
        match tokio::time::timeout(CLEANUP_BUDGET, &mut task).await {
            Ok(joined) => joined?.map_err(anyhow::Error::from),
            Err(error) => {
                task.abort();
                let _ = task.await;
                Err(anyhow::Error::new(error).context("owned CDP server shutdown"))
            }
        }
    }
}

impl Drop for OwnedServer {
    fn drop(&mut self) {
        if let Some(shutdown) = self.shutdown.take() {
            let _ = shutdown.send(());
        }
        if let Some(task) = &self.task {
            task.abort();
        }
    }
}

pub(super) async fn start_js(source: Arc<JsSource>) -> (u16, OwnedServer, PolicyFixture) {
    let policy = PolicyFixture::new(vec![grant(TOKEN, vec![Permission::Debug])]);
    let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
    let bound = listener.local_addr().unwrap();
    let state = ServerState {
        udid: DEVICE.into(),
        timeout: Duration::from_secs(2),
        cdp_host: "127.0.0.1".into(),
        cdp_port: bound.port(),
        access: AccessControl::from_file(policy.path.clone()).await.unwrap(),
        selenium_sessions: Sessions::default(),
        source,
    };
    let router = HttpBoundaryArgs::default()
        .validate()
        .unwrap()
        .for_listener(bound)
        .protect(cdp_router(state), crate::http_diagnostics::ErrorShape::Cdp);
    let (shutdown, closed) = oneshot::channel();
    let task = tokio::spawn(async move {
        axum::serve(listener, router)
            .with_graceful_shutdown(async move {
                let _ = closed.await;
            })
            .await
    });
    (
        bound.port(),
        OwnedServer {
            task: Some(task),
            shutdown: Some(shutdown),
        },
        policy,
    )
}

pub(super) struct Capture {
    pub(super) status: ExitStatus,
    pub(super) stopped: bool,
    pub(super) stdout: Vec<u8>,
    pub(super) stderr: Vec<u8>,
}

#[derive(Clone)]
pub(super) struct ChildReceipt {
    pub(super) stopped: bool,
    pub(super) reaped: bool,
    pub(super) pipes_joined: bool,
}

pub(super) struct OwnedClient {
    task: Option<JoinHandle<Result<Capture>>>,
    stop: Option<oneshot::Sender<()>>,
    pub(super) receipt: watch::Receiver<Option<ChildReceipt>>,
}

impl OwnedClient {
    pub(super) async fn finish(mut self) -> Result<Capture> {
        // Keep the handle in self while awaiting: cancelling this future drops
        // the owner and asks the detached supervisor to kill, wait and join.
        let result = self.task.as_mut().unwrap().await?;
        self.task.take();
        self.stop.take();
        result
    }
}

impl Drop for OwnedClient {
    fn drop(&mut self) {
        if let Some(stop) = self.stop.take() {
            let _ = stop.send(());
        }
        // Never abort the supervisor: it owns the child until actual wait.
    }
}

async fn capture_pipe<R: tokio::io::AsyncRead + Unpin>(pipe: R, limit: usize) -> Result<Vec<u8>> {
    let mut bytes = Vec::new();
    pipe.take(u64::try_from(limit)? + 1)
        .read_to_end(&mut bytes)
        .await?;
    if bytes.len() > limit {
        return Err(anyhow!("owned client pipe limit"));
    }
    Ok(bytes)
}

async fn join_pipe(mut task: JoinHandle<Result<Vec<u8>>>) -> Result<Vec<u8>> {
    match tokio::time::timeout(CLEANUP_BUDGET, &mut task).await {
        Ok(joined) => joined?,
        Err(error) => {
            task.abort();
            let _ = task.await;
            Err(anyhow::Error::new(error).context("owned client pipe join"))
        }
    }
}

pub(super) fn launch_client(
    client: &str,
    port: u16,
    target: JsTarget,
    expectation: &str,
    evidence: SharedEvidence,
) -> Result<OwnedClient> {
    let directory = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../scripts/cdp-clients");
    let node = std::env::var_os("IOS_CDP_CLIENT_NODE").unwrap_or_else(|| "node".into());
    let mut command = tokio::process::Command::new(node);
    command
        .arg(directory.join("rust-bridge.mjs"))
        .arg(client)
        .arg(format!("http://127.0.0.1:{port}"))
        .arg(target.url)
        .arg(expectation)
        .env("IOS_CDP_FIXTURE_TOKEN", TOKEN)
        .env_remove("DEBUG")
        .env_remove("PWDEBUG")
        .env_remove("NODE_DEBUG")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    let mut child = command.spawn()?;
    let stdout = child.stdout.take();
    let stderr = child.stderr.take();
    let (stop, mut cancelled) = oneshot::channel();
    let (completed, receipt) = watch::channel(None);
    let task = tokio::spawn(async move {
        let (stdout, stderr) = match (stdout, stderr) {
            (Some(stdout), Some(stderr)) => (stdout, stderr),
            _ => {
                let _ = child.start_kill();
                let reaped = child.wait().await.is_ok();
                let _ = completed.send(Some(ChildReceipt {
                    stopped: true,
                    reaped,
                    pipes_joined: false,
                }));
                return Err(anyhow!("owned client capture pipe absent"));
            }
        };
        let out = tokio::spawn(capture_pipe(stdout, MAX_STDOUT_BYTES));
        let err = tokio::spawn(capture_pipe(stderr, MAX_STDERR_BYTES));
        let _ = record(&evidence, json!({"event": "child_started"}));
        let (status, stopped) = tokio::select! {
            result = child.wait() => (result, false),
            _ = &mut cancelled => {
                let _ = record(&evidence, json!({"event": "child_stop_requested", "reason": "owner_drop"}));
                let _ = child.start_kill();
                (child.wait().await, true)
            }
            _ = tokio::time::sleep(CLIENT_BUDGET) => {
                let _ = record(&evidence, json!({"event": "child_stop_requested", "reason": "total"}));
                let _ = child.start_kill();
                (child.wait().await, true)
            }
        };
        let reaped = status.is_ok();
        let _ = record(
            &evidence,
            json!({"event": "child_reaped", "reaped": reaped, "stopped": stopped}),
        );
        let (stdout, stderr) = tokio::join!(join_pipe(out), join_pipe(err));
        let pipes_joined = stdout.is_ok() && stderr.is_ok();
        let _ = record(
            &evidence,
            json!({"event": "child_pipes_joined", "complete": pipes_joined}),
        );
        let _ = completed.send(Some(ChildReceipt {
            stopped,
            reaped,
            pipes_joined,
        }));
        Ok(Capture {
            status: status?,
            stopped,
            stdout: stdout?,
            stderr: stderr?,
        })
    });
    Ok(OwnedClient {
        task: Some(task),
        stop: Some(stop),
        receipt,
    })
}
