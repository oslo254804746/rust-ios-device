//! Stateful CDP translations backed by real WebInspector responses.
//! Protocol evidence: pinned pymobiledevice3 cdp_target.py getResourceTree,
//! snapshotRect, in-page navigation and Network interception adapters.
//! Input is refused: in-page DOM synthesis cannot meet trusted CDP input semantics.
use super::cdp_browser::{error, reply, unverified_page_error};
use base64::{engine::general_purpose::STANDARD, Engine};
use serde_json::{json, Value};
use std::{
    collections::HashMap,
    time::{Duration, Instant},
};

const MAX_PENDING: usize = 128;
const MAX_REQUESTS: usize = 512;
const MAX_SCREENSHOT_BYTES: usize = 16 * 1024 * 1024;

#[derive(Default, Debug)]
pub struct PageCdpOutput {
    pub wire: Vec<Value>,
    pub client: Vec<Value>,
}
impl PageCdpOutput {
    fn wire(message: Value) -> Self {
        Self {
            wire: vec![message],
            client: vec![],
        }
    }
    fn client(message: Value) -> Self {
        Self {
            wire: vec![],
            client: vec![message],
        }
    }
}
#[derive(Debug)]
enum Translation {
    Pass,
    FrameTree,
    NavigateTree {
        url: String,
        frame_id: Option<String>,
    },
    Navigate {
        frame_id: String,
        url: String,
    },
    Viewport,
    Screenshot,
    FetchAdd {
        stage: String,
    },
    FetchEnable {
        stage: String,
    },
    FetchDisable,
    FetchRemove,
    FetchRollback {
        cause: Value,
    },
    PerformanceEnable,
    PerformanceStart,
    PerformanceStop,
    PerformanceDisable,
    PerformanceRollback {
        cause: Value,
    },
    FetchResume {
        request_id: String,
    },
}
#[derive(Debug)]
struct Pending {
    request: Value,
    translation: Translation,
    deadline: Instant,
}
#[derive(Debug)]
pub struct PageCdpAdapter {
    next_id: i64,
    pending: HashMap<i64, Pending>,
    timeout: Duration,
    frame_id: Option<String>,
    frame_contexts: HashMap<String, i64>,
    fetch_enabled: bool,
    fetch_stage: Option<String>,
    request_meta: HashMap<String, Value>,
    paused: HashMap<String, &'static str>,
    performance_enabled: bool,
    memory_metrics: Option<Value>,
}
impl PageCdpAdapter {
    fn performance_changing(&self) -> bool {
        self.pending.values().any(|p| {
            matches!(
                p.translation,
                Translation::PerformanceEnable
                    | Translation::PerformanceStart
                    | Translation::PerformanceStop
                    | Translation::PerformanceDisable
                    | Translation::PerformanceRollback { .. }
            )
        })
    }
    pub fn next_deadline(&self) -> Option<Instant> {
        self.pending.values().map(|pending| pending.deadline).min()
    }
    pub fn new(timeout: Duration) -> Self {
        Self {
            next_id: -1,
            pending: HashMap::new(),
            timeout,
            frame_id: None,
            frame_contexts: HashMap::new(),
            fetch_enabled: false,
            fetch_stage: None,
            request_meta: HashMap::new(),
            paused: HashMap::new(),
            performance_enabled: false,
            memory_metrics: None,
        }
    }
    fn issue(
        &mut self,
        request: Value,
        translation: Translation,
        method: &str,
        params: Value,
        deadline: Option<Instant>,
    ) -> PageCdpOutput {
        if self.pending.len() >= MAX_PENDING || self.next_id == i64::MIN {
            return PageCdpOutput::client(error(
                &request,
                -32000,
                "CDP translation budget exceeded",
            ));
        }
        let id = self.next_id;
        self.next_id -= 1;
        self.pending.insert(
            id,
            Pending {
                request,
                translation,
                deadline: deadline.unwrap_or_else(|| Instant::now() + self.timeout),
            },
        );
        PageCdpOutput::wire(json!({"id":id,"method":method,"params":params}))
    }
    pub fn outgoing(&mut self, request: Value) -> PageCdpOutput {
        let Some(method) = request
            .get("method")
            .and_then(Value::as_str)
            .map(str::to_owned)
        else {
            return PageCdpOutput::client(error(&request, -32600, "CDP request missing method"));
        };
        if request.get("id").and_then(Value::as_u64).is_none()
            || request["id"]
                .as_u64()
                .is_some_and(|id| id > i64::MAX as u64)
        {
            return PageCdpOutput::client(error(
                &request,
                -32600,
                "CDP id must be a nonnegative signed-64 integer",
            ));
        }
        if self
            .pending
            .values()
            .any(|pending| pending.request["id"] == request["id"])
        {
            return PageCdpOutput::client(error(&request, -32600, "CDP id is already pending"));
        }
        let params = request.get("params").cloned().unwrap_or_else(|| json!({}));
        if !params.is_object() {
            return PageCdpOutput::client(error(&request, -32602, "params must be an object"));
        }
        if let Some(err) = unverified_page_error(&request) {
            return PageCdpOutput::client(err);
        }
        if matches!(method.as_str(), "Fetch.enable" | "Fetch.disable")
            && self.pending.values().any(|p| {
                matches!(
                    p.translation,
                    Translation::FetchAdd { .. }
                        | Translation::FetchEnable { .. }
                        | Translation::FetchDisable
                        | Translation::FetchRemove
                        | Translation::FetchRollback { .. }
                )
            })
        {
            return PageCdpOutput::client(error(&request, -32000, "Fetch state change is pending"));
        }
        match method.as_str() {
            "HeapProfiler.collectGarbage" => {
                self.issue(request, Translation::Pass, "Heap.gc", json!({}), None)
            }
            "Performance.enable" => {
                if params
                    .get("timeDomain")
                    .is_some_and(|value| value != "timeTicks")
                {
                    return PageCdpOutput::client(error(
                        &request,
                        -32602,
                        "only device elapsed-time metrics are available",
                    ));
                }
                if self.performance_enabled {
                    return PageCdpOutput::client(reply(&request, json!({})));
                }
                if self.performance_changing() {
                    return PageCdpOutput::client(error(
                        &request,
                        -32000,
                        "Performance state change is pending",
                    ));
                }
                self.memory_metrics = None;
                self.issue(
                    request,
                    Translation::PerformanceEnable,
                    "Memory.enable",
                    json!({}),
                    None,
                )
            }
            "Performance.disable" => {
                if self.performance_changing() {
                    return PageCdpOutput::client(error(
                        &request,
                        -32000,
                        "Performance state change is pending",
                    ));
                }
                if !self.performance_enabled {
                    return PageCdpOutput::client(reply(&request, json!({})));
                }
                self.issue(
                    request,
                    Translation::PerformanceStop,
                    "Memory.stopTracking",
                    json!({}),
                    None,
                )
            }
            "Performance.getMetrics" => {
                if let Some(metrics) = self
                    .memory_metrics
                    .as_ref()
                    .filter(|_| self.performance_enabled)
                {
                    return PageCdpOutput::client(reply(&request, json!({"metrics":metrics})));
                }
                PageCdpOutput::client(error(
                    &request,
                    -32000,
                    "enable Performance and wait for a device memory sample",
                ))
            }
            "Page.enable" | "Page.disable" | "Console.enable" | "Console.disable" => {
                self.issue(request, Translation::Pass, &method, params, None)
            }
            "Page.reload" => {
                let ignore_cache = params
                    .get("ignoreCache")
                    .and_then(Value::as_bool)
                    .unwrap_or(false);
                self.issue(
                    request,
                    Translation::Pass,
                    "Page.reload",
                    json!({"ignoreCache": ignore_cache}),
                    None,
                )
            }
            "Page.setDownloadBehavior" => PageCdpOutput::client(error(
                &request,
                -32601,
                "Page.setDownloadBehavior has no verified WebInspector download manager mapping",
            )),
            "Page.getFrameTree" | "Page.getResourceTree" => self.issue(
                request,
                Translation::FrameTree,
                "Page.getResourceTree",
                params,
                None,
            ),
            "Page.navigate" => {
                let Some(url) = params
                    .get("url")
                    .and_then(Value::as_str)
                    .filter(|url| !url.is_empty() && url.len() <= 16 * 1024)
                else {
                    return PageCdpOutput::client(error(
                        &request,
                        -32602,
                        "navigate requires a bounded URL",
                    ));
                };
                if params
                    .as_object()
                    .unwrap()
                    .keys()
                    .any(|key| !["url", "frameId"].contains(&key.as_str()))
                {
                    return PageCdpOutput::client(error(
                        &request,
                        -32602,
                        "navigation options lack a verified mapping",
                    ));
                }
                if self.pending.values().any(|pending| {
                    matches!(
                        pending.translation,
                        Translation::Navigate { .. } | Translation::NavigateTree { .. }
                    )
                }) {
                    return PageCdpOutput::client(error(
                        &request,
                        -32000,
                        "a navigation is already pending",
                    ));
                }
                if params
                    .get("frameId")
                    .is_some_and(|id| id.as_str().map_or(true, |id| id.is_empty()))
                {
                    return PageCdpOutput::client(error(
                        &request,
                        -32602,
                        "frameId must be a nonempty string",
                    ));
                }
                let target_frame_id = params
                    .get("frameId")
                    .and_then(Value::as_str)
                    .map(str::to_owned);
                let url = url.to_owned();
                self.issue(
                    request,
                    Translation::NavigateTree {
                        url,
                        frame_id: target_frame_id,
                    },
                    "Page.getResourceTree",
                    json!({}),
                    None,
                )
            }
            "Page.captureScreenshot" => {
                if params
                    .get("format")
                    .and_then(Value::as_str)
                    .is_some_and(|format| format != "png")
                    || params.as_object().unwrap().keys().any(|key| {
                        !["format", "clip", "captureBeyondViewport", "fromSurface"]
                            .contains(&key.as_str())
                    })
                    || params.get("captureBeyondViewport").and_then(Value::as_bool) == Some(true)
                    || params.get("fromSurface").and_then(Value::as_bool) == Some(false)
                {
                    return PageCdpOutput::client(error(
                        &request,
                        -32602,
                        "only PNG viewport/scale-1 clip screenshots are supported",
                    ));
                }
                if let Some(clip) = params.get("clip") {
                    let Some(rect) = screenshot_rect(clip) else {
                        return PageCdpOutput::client(error(
                            &request,
                            -32602,
                            "invalid screenshot rectangle or scale",
                        ));
                    };
                    self.issue(
                        request,
                        Translation::Screenshot,
                        "Page.snapshotRect",
                        rect,
                        None,
                    )
                } else {
                    self.issue(request,Translation::Viewport,"Runtime.evaluate",json!({"expression":"({x:window.scrollX,y:window.scrollY,width:window.innerWidth,height:window.innerHeight})","returnByValue":true}),None)
                }
            }
            "Fetch.enable" => {
                if self.fetch_enabled || !self.paused.is_empty() {
                    return PageCdpOutput::client(error(
                        &request,
                        -32000,
                        "Fetch already enabled or paused requests remain",
                    ));
                }
                let patterns = params.get("patterns").and_then(Value::as_array);
                if params.get("handleAuthRequests").and_then(Value::as_bool) == Some(true)
                    || params
                        .as_object()
                        .unwrap()
                        .keys()
                        .any(|key| !["patterns", "handleAuthRequests"].contains(&key.as_str()))
                    || patterns.is_some_and(|patterns| {
                        patterns.len() != 1
                            || patterns[0].as_object().map_or(true, |p| {
                                p.keys().any(|key| {
                                    !["urlPattern", "requestStage"].contains(&key.as_str())
                                })
                            })
                    })
                {
                    return PageCdpOutput::client(error(&request,-32602,"only one wildcard interception pattern without auth/resource filter is supported"));
                }
                let pattern = patterns
                    .and_then(|p| p.first())
                    .cloned()
                    .unwrap_or_else(|| json!({}));
                if pattern
                    .get("urlPattern")
                    .and_then(Value::as_str)
                    .is_some_and(|pattern| pattern != "*")
                {
                    return PageCdpOutput::client(error(
                        &request,
                        -32602,
                        "only wildcard URL interception is supported",
                    ));
                }
                let stage = match pattern
                    .get("requestStage")
                    .and_then(Value::as_str)
                    .unwrap_or("Request")
                {
                    "Request" => "request",
                    "Response" => "response",
                    _ => {
                        return PageCdpOutput::client(error(
                            &request,
                            -32602,
                            "invalid interception stage",
                        ))
                    }
                };
                self.issue(
                    request,
                    Translation::FetchAdd {
                        stage: stage.to_owned(),
                    },
                    "Network.addInterception",
                    json!({"url":".*","stage":stage,"isRegex":true}),
                    None,
                )
            }
            "Fetch.disable" => {
                if !self.fetch_enabled {
                    return PageCdpOutput::client(reply(&request, json!({})));
                }
                self.issue(
                    request,
                    Translation::FetchDisable,
                    "Network.setInterceptionEnabled",
                    json!({"enabled":false}),
                    None,
                )
            }
            "Fetch.continueRequest" | "Fetch.failRequest" | "Fetch.fulfillRequest" => {
                let Some(request_id) = params
                    .get("requestId")
                    .and_then(Value::as_str)
                    .map(str::to_owned)
                else {
                    return PageCdpOutput::client(error(&request, -32602, "requestId is required"));
                };
                let Some(stage) = self.paused.get(&request_id).copied() else {
                    return PageCdpOutput::client(error(
                        &request,
                        -32602,
                        "requestId is not paused in this session",
                    ));
                };
                if self.pending.values().any(|pending| matches!(&pending.translation,Translation::FetchResume{request_id:id} if id==&request_id)) {
                    return PageCdpOutput::client(error(&request,-32000,"request resume is already pending"));
                }
                let (wire_method, mut wire_params) = match method.as_str() {
                    "Fetch.continueRequest" => {
                        let options = params.as_object().unwrap();
                        if options.len() == 1 {
                            ("Network.interceptContinue", json!({"stage":stage}))
                        } else {
                            if stage != "request"
                                || options.keys().any(|key| {
                                    !["requestId", "url", "method", "headers", "postData"]
                                        .contains(&key.as_str())
                                })
                            {
                                return PageCdpOutput::client(error(
                                    &request,
                                    -32602,
                                    "unsupported request override or response-stage override",
                                ));
                            }
                            let mut native = json!({});
                            for key in ["url", "method", "postData"] {
                                if let Some(value) = params.get(key) {
                                    let Some(text) = value
                                        .as_str()
                                        .filter(|text| text.len() <= MAX_SCREENSHOT_BYTES)
                                    else {
                                        return PageCdpOutput::client(error(
                                            &request,
                                            -32602,
                                            "request override must be a bounded string",
                                        ));
                                    };
                                    if key == "postData" && STANDARD.decode(text).is_err() {
                                        return PageCdpOutput::client(error(
                                            &request,
                                            -32602,
                                            "postData must be base64",
                                        ));
                                    }
                                    native[key] = value.clone();
                                }
                            }
                            if let Some(headers) = params.get("headers") {
                                let Some(headers) = header_object(Some(headers)) else {
                                    return PageCdpOutput::client(error(
                                        &request,
                                        -32602,
                                        "invalid request headers",
                                    ));
                                };
                                native["headers"] = json!(headers);
                            }
                            ("Network.interceptWithRequest", native)
                        }
                    }
                    "Fetch.failRequest" => {
                        if stage != "request" {
                            return PageCdpOutput::client(error(
                                &request,
                                -32602,
                                "failRequest is only valid in request stage",
                            ));
                        }
                        let reason = params
                            .get("errorReason")
                            .and_then(Value::as_str)
                            .unwrap_or("Failed");
                        let error_type = match reason {
                            "Failed" | "ConnectionFailed" | "ConnectionRefused"
                            | "ConnectionReset" | "AddressUnreachable" => "General",
                            "Aborted" | "ConnectionAborted" => "Cancellation",
                            "TimedOut" => "Timeout",
                            "AccessDenied" | "BlockedByClient" | "BlockedByResponse" => {
                                "AccessControl"
                            }
                            _ => {
                                return PageCdpOutput::client(error(
                                    &request,
                                    -32602,
                                    "only verified request failure reasons are supported",
                                ));
                            }
                        };
                        (
                            "Network.interceptRequestWithError",
                            json!({"errorType": error_type}),
                        )
                    }
                    _ => {
                        let Some(code) = params
                            .get("responseCode")
                            .and_then(Value::as_u64)
                            .filter(|c| (100..=599).contains(c))
                        else {
                            return PageCdpOutput::client(error(
                                &request,
                                -32602,
                                "valid responseCode is required",
                            ));
                        };
                        let headers = match header_object(params.get("responseHeaders")) {
                            Some(headers) => headers,
                            None => {
                                return PageCdpOutput::client(error(
                                    &request,
                                    -32602,
                                    "invalid or duplicate response headers",
                                ))
                            }
                        };
                        let body = params.get("body").and_then(Value::as_str).unwrap_or("");
                        if body.len() > MAX_SCREENSHOT_BYTES || STANDARD.decode(body).is_err() {
                            return PageCdpOutput::client(error(
                                &request,
                                -32602,
                                "body must be bounded base64",
                            ));
                        }
                        let mime = headers
                            .get("Content-Type")
                            .or_else(|| headers.get("content-type"))
                            .and_then(Value::as_str)
                            .unwrap_or("application/octet-stream");
                        (
                            if stage == "request" {
                                "Network.interceptRequestWithResponse"
                            } else {
                                "Network.interceptWithResponse"
                            },
                            json!({"content":body,"base64Encoded":true,"mimeType":mime,"status":code,
                            "statusText":params.get("responsePhrase").and_then(Value::as_str).unwrap_or(""),"headers":headers}),
                        )
                    }
                };
                wire_params["requestId"] = json!(request_id);
                self.issue(
                    request,
                    Translation::FetchResume { request_id },
                    wire_method,
                    wire_params,
                    None,
                )
            }
            _ => PageCdpOutput::wire(request),
        }
    }
    pub fn incoming(&mut self, message: Value) -> PageCdpOutput {
        if let Some(id) = message
            .get("id")
            .and_then(Value::as_i64)
            .filter(|id| *id < 0)
        {
            let Some(pending) = self.pending.remove(&id) else {
                return PageCdpOutput::default();
            };
            if let Some(cause) = message.get("error") {
                if let Translation::FetchEnable { stage } = &pending.translation {
                    return self.issue(
                        pending.request,
                        Translation::FetchRollback {
                            cause: cause.clone(),
                        },
                        "Network.removeInterception",
                        json!({"url":".*","stage":stage,"isRegex":true}),
                        Some(pending.deadline),
                    );
                }
                if matches!(pending.translation, Translation::PerformanceStart) {
                    return self.issue(
                        pending.request,
                        Translation::PerformanceRollback {
                            cause: cause.clone(),
                        },
                        "Memory.disable",
                        json!({}),
                        Some(pending.deadline),
                    );
                }
                if let Translation::PerformanceRollback { cause: primary }
                | Translation::FetchRollback { cause: primary } = &pending.translation
                {
                    return PageCdpOutput::client(rollback_failure(
                        &pending.request,
                        primary,
                        Some(cause),
                    ));
                }
                let mut response = json!({"id":pending.request["id"],"error":cause});
                if let Some(session) = pending.request.get("sessionId") {
                    response["sessionId"] = session.clone();
                }
                return PageCdpOutput::client(response);
            }
            let Some(result) = message.get("result") else {
                return PageCdpOutput::client(error(
                    &pending.request,
                    -32000,
                    "WebInspector omitted the command result",
                ));
            };
            return match pending.translation {
                Translation::PerformanceEnable => self.issue(
                    pending.request,
                    Translation::PerformanceStart,
                    "Memory.startTracking",
                    json!({}),
                    Some(pending.deadline),
                ),
                Translation::PerformanceStart => {
                    self.performance_enabled = true;
                    PageCdpOutput::client(reply(&pending.request, json!({})))
                }
                Translation::PerformanceStop => {
                    self.performance_enabled = false;
                    self.memory_metrics = None;
                    self.issue(
                        pending.request,
                        Translation::PerformanceDisable,
                        "Memory.disable",
                        json!({}),
                        Some(pending.deadline),
                    )
                }
                Translation::PerformanceDisable => {
                    PageCdpOutput::client(reply(&pending.request, json!({})))
                }
                Translation::PerformanceRollback { cause } => {
                    PageCdpOutput::client(rollback_failure(&pending.request, &cause, None))
                }
                Translation::Pass => PageCdpOutput::client(reply(&pending.request, result.clone())),
                Translation::FrameTree => {
                    match result.get("frameTree").filter(|tree| valid_tree(tree)) {
                        Some(tree) => {
                            self.frame_id = tree["frame"]["id"].as_str().map(str::to_owned);
                            PageCdpOutput::client(reply(
                                &pending.request,
                                json!({"frameTree": clean_frame_tree(tree)}),
                            ))
                        }
                        None => PageCdpOutput::client(error(
                            &pending.request,
                            -32000,
                            "WebInspector returned an invalid frame tree",
                        )),
                    }
                }
                Translation::NavigateTree { url, frame_id } => {
                    let Some(tree) = result.get("frameTree").filter(|tree| valid_tree(tree)) else {
                        return PageCdpOutput::client(error(
                            &pending.request,
                            -32000,
                            "navigation requires a real frame tree",
                        ));
                    };
                    let main_id = tree["frame"]["id"].as_str().unwrap().to_owned();
                    let effective_frame_id = frame_id.unwrap_or_else(|| main_id.clone());
                    let context = if effective_frame_id != main_id {
                        let context = self
                            .frame_contexts
                            .get(&effective_frame_id)
                            .copied()
                            .filter(|_| tree_contains_frame(tree, &effective_frame_id));
                        let Some(context) = context else {
                            return PageCdpOutput::client(error(
                                &pending.request,
                                -32601,
                                "subframe navigation requires a verified execution-context mapping",
                            ));
                        };
                        Some(context)
                    } else {
                        None
                    };
                    self.frame_id = Some(main_id);
                    let expression = format!(
                        "window.location.assign({})",
                        serde_json::to_string(&url).unwrap()
                    );
                    let mut params = json!({"expression": expression});
                    if let Some(context) = context {
                        params["contextId"] = json!(context);
                    }
                    self.issue(
                        pending.request,
                        Translation::Navigate {
                            frame_id: effective_frame_id,
                            url,
                        },
                        "Runtime.evaluate",
                        params,
                        Some(pending.deadline),
                    )
                }
                Translation::Navigate { frame_id, url } => {
                    if result.get("wasThrown").and_then(Value::as_bool) == Some(true)
                        || result.get("exceptionDetails").is_some()
                    {
                        PageCdpOutput::client(error(
                            &pending.request,
                            -32000,
                            "navigation expression failed",
                        ))
                    } else {
                        // Evaluation success is not a navigation commit. Keep the original obligation.
                        self.pending.insert(
                            id,
                            Pending {
                                request: pending.request,
                                translation: Translation::Navigate { frame_id, url },
                                deadline: pending.deadline,
                            },
                        );
                        PageCdpOutput::default()
                    }
                }
                Translation::Viewport => {
                    let Some(rect) = result
                        .get("result")
                        .and_then(|value| value.get("value"))
                        .and_then(screenshot_rect)
                    else {
                        return PageCdpOutput::client(error(
                            &pending.request,
                            -32000,
                            "device did not report its viewport",
                        ));
                    };
                    self.issue(
                        pending.request,
                        Translation::Screenshot,
                        "Page.snapshotRect",
                        rect,
                        Some(pending.deadline),
                    )
                }
                Translation::Screenshot => {
                    let Some(data) = result
                        .get("dataURL")
                        .and_then(Value::as_str)
                        .and_then(|data| data.strip_prefix("data:image/png;base64,"))
                    else {
                        return PageCdpOutput::client(error(
                            &pending.request,
                            -32000,
                            "WebInspector did not return a PNG data URL",
                        ));
                    };
                    if data.len() > MAX_SCREENSHOT_BYTES {
                        return PageCdpOutput::client(error(
                            &pending.request,
                            -32000,
                            "screenshot exceeds budget",
                        ));
                    }
                    match STANDARD.decode(data) {
                        Ok(bytes) if valid_png(&bytes) => {
                            PageCdpOutput::client(reply(&pending.request, json!({"data":data})))
                        }
                        _ => PageCdpOutput::client(error(
                            &pending.request,
                            -32000,
                            "invalid PNG base64 data",
                        )),
                    }
                }
                Translation::FetchAdd { stage } => self.issue(
                    pending.request,
                    Translation::FetchEnable { stage },
                    "Network.setInterceptionEnabled",
                    json!({"enabled":true}),
                    Some(pending.deadline),
                ),
                Translation::FetchEnable { stage } => {
                    self.fetch_enabled = true;
                    self.fetch_stage = Some(stage);
                    PageCdpOutput::client(reply(&pending.request, result.clone()))
                }
                Translation::FetchDisable => {
                    self.fetch_enabled = false;
                    self.paused.clear();
                    self.request_meta.clear();
                    let stage = self
                        .fetch_stage
                        .take()
                        .expect("enabled interception has a stage");
                    self.issue(
                        pending.request,
                        Translation::FetchRemove,
                        "Network.removeInterception",
                        json!({"url":".*","stage":stage,"isRegex":true}),
                        Some(pending.deadline),
                    )
                }
                Translation::FetchRemove => {
                    PageCdpOutput::client(reply(&pending.request, result.clone()))
                }
                Translation::FetchRollback { cause } => {
                    PageCdpOutput::client(rollback_failure(&pending.request, &cause, None))
                }
                Translation::FetchResume { request_id } => {
                    self.paused.remove(&request_id);
                    PageCdpOutput::client(reply(&pending.request, result.clone()))
                }
            };
        }
        let method = message.get("method").and_then(Value::as_str).unwrap_or("");
        if method == "Memory.trackingUpdate"
            && (self.performance_enabled || self.performance_changing())
        {
            if let Some(metrics) = memory_metrics(&message["params"]["event"]) {
                self.memory_metrics = Some(metrics.clone());
                return PageCdpOutput::client(json!({"method":"Performance.metrics", "params":{
                    "metrics":metrics,"title":"WebKit memory allocation sample"
                }}));
            }
        }
        if method == "Runtime.executionContextCreated" {
            let context = &message["params"]["context"];
            if context["type"] == "normal" || context["isPageContext"] == true {
                if let (Some(frame), Some(id)) =
                    (context["frameId"].as_str(), context["id"].as_i64())
                {
                    if self.frame_contexts.len() < MAX_PENDING
                        || self.frame_contexts.contains_key(frame)
                    {
                        self.frame_contexts.insert(frame.to_owned(), id);
                    }
                }
            }
        }
        if method == "Runtime.executionContextsCleared" {
            self.frame_contexts.clear();
        }
        if method == "Runtime.executionContextDestroyed" {
            if let Some(id) = message["params"]["executionContextId"].as_i64() {
                self.frame_contexts.retain(|_, context| *context != id);
            }
        }
        if method == "Console.messageAdded" {
            // WebKit ConsoleMessage has no executionContextId. Neither a guessed
            // context 1 nor host wall time is valid Runtime.consoleAPICalled evidence.
            // Preserve the native event until a proven context/time association exists.
            return PageCdpOutput::client(message);
        }
        if method == "Page.frameNavigated" || method == "Page.navigatedWithinDocument" {
            let frame = message.get("params").and_then(|params| params.get("frame"));
            let frame_id = frame
                .and_then(|frame| frame.get("id"))
                .and_then(Value::as_str)
                .or_else(|| message["params"]["frameId"].as_str());
            let url = frame
                .and_then(|frame| frame.get("url"))
                .and_then(Value::as_str)
                .or_else(|| message["params"]["url"].as_str());
            if let Some(frame) = frame {
                if frame.get("parentId").is_none() {
                    self.frame_id = frame_id.map(str::to_owned);
                }
            }
            let ids: Vec<_> = self
                .pending
                .iter()
                .filter_map(|(id, pending)| match &pending.translation {
                    Translation::Navigate {
                        frame_id: expected,
                        url: expected_url,
                    } if frame_id == Some(expected.as_str())
                        && url == Some(expected_url.as_str()) =>
                    {
                        Some(*id)
                    }
                    _ => None,
                })
                .collect();
            let mut output = PageCdpOutput::default();
            for id in ids {
                let pending = self.pending.remove(&id).unwrap();
                let mut result = json!({"frameId":frame_id.unwrap()});
                if let Some(loader) = frame
                    .and_then(|frame| frame.get("loaderId"))
                    .filter(|loader| loader.is_string())
                {
                    result["loaderId"] = loader.clone();
                }
                output.client.push(reply(&pending.request, result));
            }
            output.client.push(message);
            return output;
        }
        if method == "Page.frameDetached" {
            if let Some(frame) = message["params"]["frameId"].as_str() {
                self.frame_contexts.remove(frame);
            }
            let mut message = message;
            if let Some(params) = message.get_mut("params").and_then(Value::as_object_mut) {
                if !params.contains_key("reason") {
                    params.insert("reason".into(), json!("remove"));
                }
            }
            return PageCdpOutput::client(message);
        }
        if method == "Network.requestWillBeSent" {
            if let Some(id) = message["params"]["requestId"].as_str() {
                if self.request_meta.len() >= MAX_REQUESTS {
                    self.request_meta.clear();
                }
                self.request_meta
                    .insert(id.to_owned(), message["params"].clone());
            }
        }
        if matches!(method, "Network.loadingFinished" | "Network.loadingFailed") {
            if let Some(id) = message["params"]["requestId"].as_str() {
                self.request_meta.remove(id);
                self.paused.remove(id);
            }
        }
        if (self.fetch_enabled
            || self
                .pending
                .values()
                .any(|p| matches!(p.translation, Translation::FetchEnable { .. })))
            && matches!(
                method,
                "Network.requestIntercepted" | "Network.responseIntercepted"
            )
        {
            let params = &message["params"];
            let Some(id) = params["requestId"].as_str() else {
                return PageCdpOutput::client(message);
            };
            let Some(meta) = self.request_meta.get(id) else {
                return PageCdpOutput::client(message);
            };
            let Some(frame_id) = meta["frameId"].as_str() else {
                return PageCdpOutput::client(message);
            };
            if self.paused.len() >= MAX_REQUESTS {
                return PageCdpOutput::client(
                    json!({"method":"Inspector.detached","params":{"reason":"Fetch paused-request budget exceeded"}}),
                );
            }
            let stage = if method == "Network.requestIntercepted" {
                "request"
            } else {
                "response"
            };
            let request = if stage == "request" {
                params.get("request")
            } else {
                meta.get("request")
            };
            let Some(request) = request.filter(|request| request.is_object()) else {
                return PageCdpOutput::client(message);
            };
            let mut paused = json!({"requestId":id,"networkId":id,"frameId":frame_id,"request":request,"resourceType":meta.get("type").and_then(Value::as_str).unwrap_or("Other")});
            if stage == "response" {
                let response = &params["response"];
                paused["responseStatusCode"] = response["status"].clone();
                paused["responseStatusText"] = response["statusText"].clone();
                paused["responseHeaders"] = json!(response["headers"]
                    .as_object()
                    .map(|headers| headers
                        .iter()
                        .map(|(name, value)| json!({"name":name,"value":value}))
                        .collect::<Vec<_>>())
                    .unwrap_or_default());
            }
            self.paused.insert(id.to_owned(), stage);
            return PageCdpOutput::client(json!({"method":"Fetch.requestPaused","params":paused}));
        }
        PageCdpOutput::client(message)
    }
    pub fn expired(&mut self) -> Vec<Value> {
        let now = Instant::now();
        let ids: Vec<_> = self
            .pending
            .iter()
            .filter_map(|(id, pending)| (pending.deadline <= now).then_some(*id))
            .collect();
        ids.into_iter()
            .map(|id| {
                let pending = self.pending.remove(&id).unwrap();
                if let Translation::PerformanceRollback { cause }
                | Translation::FetchRollback { cause } = &pending.translation
                {
                    return rollback_failure(
                        &pending.request,
                        cause,
                        Some(&json!({"code":-32000,"message":"native rollback deadline elapsed"})),
                    );
                }
                error(
                    &pending.request,
                    -32000,
                    "WebInspector translation deadline elapsed",
                )
            })
            .collect()
    }
}
fn rollback_failure(request: &Value, primary: &Value, cleanup: Option<&Value>) -> Value {
    let mut response = reply(request, json!({}));
    response.as_object_mut().unwrap().remove("result");
    response["error"] = primary.clone();
    if let Some(cleanup) = cleanup {
        response["error"]["data"] =
            json!({"primaryData":primary.get("data"),"cleanupError":cleanup});
    }
    response
}
fn memory_metrics(event: &Value) -> Option<Value> {
    let timestamp = event["timestamp"]
        .as_f64()
        .filter(|n| n.is_finite() && *n >= 0.0)?;
    let categories = event["categories"].as_array()?;
    if categories.is_empty() || categories.len() > 6 {
        return None;
    }
    let mut metrics = vec![json!({"name":"Timestamp", "value":timestamp})];
    for category in categories {
        // WebKit JavaScript includes GCHeap and GCOwned allocations: neither
        // CDP JSHeapUsedSize nor JSHeapTotalSize describes it accurately.
        let name = match category["type"].as_str()? {
            "javascript" => "WebKitJavaScriptMemory",
            "jit" => "WebKitJITMemory",
            "images" => "WebKitImageMemory",
            "layers" => "WebKitLayerMemory",
            "page" => "WebKitPageMemory",
            "other" => "WebKitOtherMemory",
            _ => return None,
        };
        let size = category["size"]
            .as_f64()
            .filter(|n| n.is_finite() && *n >= 0.0)?;
        if metrics.iter().any(|metric| metric["name"] == name) {
            return None;
        }
        metrics.push(json!({"name":name,"value":size}));
    }
    Some(json!(metrics))
}

fn valid_tree(tree: &Value) -> bool {
    fn validate(tree: &Value, depth: usize) -> bool {
        depth <= 64
            && tree.get("frame").is_some_and(|frame| {
                frame["id"].as_str().is_some_and(|id| !id.is_empty()) && frame["url"].is_string()
            })
            && tree.get("childFrames").map_or(true, |frames| {
                frames.as_array().is_some_and(|frames| {
                    frames.len() <= 512 && frames.iter().all(|frame| validate(frame, depth + 1))
                })
            })
    }
    validate(tree, 0)
}
fn tree_contains_frame(tree: &Value, id: &str) -> bool {
    tree["frame"]["id"].as_str() == Some(id)
        || tree
            .get("childFrames")
            .and_then(Value::as_array)
            .is_some_and(|frames| frames.iter().any(|frame| tree_contains_frame(frame, id)))
}
fn clean_frame_tree(tree: &Value) -> Value {
    let mut cleaned = serde_json::Map::new();
    if let Some(frame) = tree.get("frame") {
        cleaned.insert("frame".into(), frame.clone());
    }
    if let Some(child_frames) = tree.get("childFrames").and_then(Value::as_array) {
        let cleaned_children: Vec<Value> = child_frames.iter().map(clean_frame_tree).collect();
        cleaned.insert("childFrames".into(), Value::Array(cleaned_children));
    }
    Value::Object(cleaned)
}
fn screenshot_rect(value: &Value) -> Option<Value> {
    if value
        .get("scale")
        .and_then(Value::as_f64)
        .is_some_and(|scale| scale != 1.0)
    {
        return None;
    }
    let x = value.get("x")?.as_f64()?;
    let y = value.get("y")?.as_f64()?;
    let width = value.get("width")?.as_f64()?;
    let height = value.get("height")?.as_f64()?;
    if ![x, y, width, height]
        .iter()
        .all(|v| v.is_finite() && *v >= 0.0 && v.fract() == 0.0)
        || width == 0.0
        || height == 0.0
        || width * height > 4_194_304.0
    {
        return None;
    }
    Some(json!({"x":x,"y":y,"width":width,"height":height,"coordinateSystem":"Page"}))
}
fn header_object(value: Option<&Value>) -> Option<Value> {
    let mut headers = serde_json::Map::new();
    if let Some(value) = value {
        let values = value.as_array()?;
        if values.len() > 128 {
            return None;
        }
        for entry in values {
            let name = entry.get("name")?.as_str()?;
            let value = entry.get("value")?.as_str()?;
            if name.is_empty()
                || name.len() + value.len() > 16 * 1024
                || headers
                    .keys()
                    .any(|existing: &String| existing.eq_ignore_ascii_case(name))
            {
                return None;
            }
            headers.insert(name.to_owned(), json!(value));
        }
    }
    Some(Value::Object(headers))
}

fn valid_png(bytes: &[u8]) -> bool {
    let mut decoder = png::Decoder::new(std::io::Cursor::new(bytes));
    decoder.set_limits(png::Limits {
        bytes: MAX_SCREENSHOT_BYTES,
    });
    let Ok(mut reader) = decoder.read_info() else {
        return false;
    };
    let info = reader.info();
    if info.width == 0 || info.height == 0 || reader.output_buffer_size() > MAX_SCREENSHOT_BYTES {
        return false;
    }
    let mut pixels = vec![0; reader.output_buffer_size()];
    reader.next_frame(&mut pixels).is_ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fetch_override_response_and_disable_remove_real_native_interception() {
        let mut adapter = PageCdpAdapter::new(Duration::from_secs(1));
        let add = adapter.outgoing(json!({"id":1,"method":"Fetch.enable","params":{"patterns":[{"requestStage":"Response"}]}}));
        let enabled = adapter.incoming(json!({"id":add.wire[0]["id"],"result":{}}));
        adapter.incoming(json!({"method":"Network.requestWillBeSent","params":{"requestId":"native","frameId":"frame","request":{"url":"https://a.test"}}}));
        let paused = adapter.incoming(json!({"method":"Network.responseIntercepted","params":{"requestId":"native","response":{"status":200,"headers":{}}}}));
        assert_eq!(
            paused.client[0]["method"], "Fetch.requestPaused",
            "native may pause before enable ACK"
        );
        adapter.incoming(json!({"id":enabled.wire[0]["id"],"result":{}}));
        let fulfilled = adapter.outgoing(json!({"id":2,"method":"Fetch.fulfillRequest","params":{"requestId":"native","responseCode":201,"body":"b2s="}}));
        assert_eq!(fulfilled.wire[0]["method"], "Network.interceptWithResponse");
        assert_eq!(fulfilled.wire[0]["params"]["content"], "b2s=");
        adapter.incoming(json!({"id":fulfilled.wire[0]["id"],"result":{}}));
        let disable = adapter.outgoing(json!({"id":3,"method":"Fetch.disable"}));
        let remove = adapter.incoming(json!({"id":disable.wire[0]["id"],"result":{}}));
        assert_eq!(remove.wire[0]["method"], "Network.removeInterception");
        assert_eq!(
            remove.wire[0]["params"],
            json!({"url":".*","stage":"response","isRegex":true})
        );
        adapter.incoming(json!({"id":remove.wire[0]["id"],"result":{}}));
        let add = adapter.outgoing(json!({"id":4,"method":"Fetch.enable"}));
        assert_eq!(add.wire[0]["method"], "Network.addInterception");
    }

    #[test]
    fn performance_metrics_use_device_seconds_and_named_webkit_bytes_and_stop_sampling() {
        let mut adapter = PageCdpAdapter::new(Duration::from_secs(1));
        let early = adapter.outgoing(json!({"id":1,"method":"Performance.getMetrics"}));
        assert_eq!(early.client[0]["error"]["code"], -32000);
        let enabled = adapter.outgoing(json!({"id":2,"method":"Performance.enable"}));
        assert_eq!(enabled.wire[0]["method"], "Memory.enable");
        let start = adapter.incoming(json!({"id":enabled.wire[0]["id"],"result":{}}));
        assert_eq!(start.wire[0]["method"], "Memory.startTracking");
        adapter.incoming(json!({"id":start.wire[0]["id"],"result":{}}));
        let sample = adapter.incoming(json!({"method":"Memory.trackingUpdate","params":{"event":{
            "timestamp":12.5,"categories":[{"type":"javascript","size":4096},{"type":"images","size":1024}]
        }}}));
        assert_eq!(sample.client[0]["method"], "Performance.metrics");
        let metrics = adapter.outgoing(json!({"id":3,"method":"Performance.getMetrics"}));
        assert_eq!(
            metrics.client[0]["result"]["metrics"],
            json!([
                {"name":"Timestamp","value":12.5},{"name":"WebKitJavaScriptMemory","value":4096.0},
                {"name":"WebKitImageMemory","value":1024.0}
            ])
        );
        assert!(
            memory_metrics(&json!({"timestamp":1,"categories":[{"type":"unknown","size":0}]}))
                .is_none()
        );
        let stop = adapter.outgoing(json!({"id":4,"method":"Performance.disable"}));
        assert_eq!(stop.wire[0]["method"], "Memory.stopTracking");
        let disable = adapter.incoming(json!({"id":stop.wire[0]["id"],"result":{}}));
        assert_eq!(disable.wire[0]["method"], "Memory.disable");
        adapter.incoming(json!({"id":disable.wire[0]["id"],"result":{}}));
        assert!(adapter
            .outgoing(json!({"id":5,"method":"Performance.getMetrics"}))
            .client[0]
            .get("error")
            .is_some());
        let gc = adapter.outgoing(json!({"id":6,"method":"HeapProfiler.collectGarbage"}));
        assert_eq!(gc.wire[0]["method"], "Heap.gc");
    }

    #[test]
    fn performance_start_failure_disables_native_domain_and_preserves_primary_error() {
        let mut adapter = PageCdpAdapter::new(Duration::from_secs(1));
        let enabled = adapter.outgoing(json!({"id":1,"method":"Performance.enable"}));
        let start = adapter.incoming(json!({"id":enabled.wire[0]["id"],"result":{}}));
        let original = json!({"code":-32000,"message":"native sampling unavailable"});
        let rollback = adapter.incoming(json!({"id":start.wire[0]["id"],"error":original}));
        assert_eq!(rollback.wire[0]["method"], "Memory.disable");
        let output = adapter.incoming(json!({"id":rollback.wire[0]["id"],"result":{}}));
        assert_eq!(output.client[0]["error"], original);
        assert!(!adapter.performance_enabled);
    }

    #[test]
    fn rollback_timeout_retains_primary_and_cleanup_observations() {
        let mut adapter = PageCdpAdapter::new(Duration::from_secs(1));
        let enable = adapter.outgoing(json!({"id":1,"method":"Performance.enable"}));
        let start = adapter.incoming(json!({"id":enable.wire[0]["id"],"result":{}}));
        let rollback=adapter.incoming(json!({"id":start.wire[0]["id"],"error":{"code":-123,"message":"original sampling error","data":{"native":1}}}));
        adapter
            .pending
            .get_mut(&rollback.wire[0]["id"].as_i64().unwrap())
            .unwrap()
            .deadline = Instant::now();
        let result = adapter.expired();
        assert_eq!(result[0]["error"]["code"], -123);
        assert_eq!(result[0]["error"]["data"]["primaryData"]["native"], 1);
        assert_eq!(result[0]["error"]["data"]["cleanupError"]["code"], -32000);
    }

    #[test]
    fn subframe_navigation_selects_native_normal_context_and_detach_removes_it() {
        let mut adapter = PageCdpAdapter::new(Duration::from_secs(1));
        adapter.incoming(json!({"method":"Runtime.executionContextCreated","params":{
            "context":{"id":101,"type":"normal","frameId":"child","name":""}
        }}));
        let tree = json!({"frame":{"id":"main","url":"https://a.test"},"childFrames":[
            {"frame":{"id":"child","parentId":"main","url":"https://b.test"}}
        ]});
        let request = adapter.outgoing(json!({"id":1,"method":"Page.navigate","params":{"url":"https://b.test/new","frameId":"child"}}));
        let sent =
            adapter.incoming(json!({"id":request.wire[0]["id"],"result":{"frameTree":tree}}));
        assert_eq!(sent.wire[0]["method"], "Runtime.evaluate");
        assert_eq!(sent.wire[0]["params"]["contextId"], 101);
        let evaluated = adapter
            .incoming(json!({"id":sent.wire[0]["id"],"result":{"result":{"type":"undefined"}}}));
        assert!(evaluated.client.is_empty());
        let wrong = adapter.incoming(json!({"method":"Page.frameNavigated","params":{"frame":{"id":"main","url":"https://b.test/new"}}}));
        assert!(wrong.client.iter().all(|m| m.get("id").is_none()));
        let committed = adapter.incoming(json!({"method":"Page.frameNavigated","params":{"frame":{"id":"child","parentId":"main","url":"https://b.test/new"}}}));
        assert_eq!(committed.client[0]["result"]["frameId"], "child");
        adapter.incoming(json!({"method":"Page.frameDetached","params":{"frameId":"child"}}));
        assert!(!adapter.frame_contexts.contains_key("child"));
    }
    fn adapter() -> PageCdpAdapter {
        PageCdpAdapter::new(Duration::from_secs(5))
    }
    fn tree() -> Value {
        json!({"frame":{"id":"device-top-31","loaderId":"loader-old","url":"https://host.test/"},
        "childFrames":[{"frame":{"id":"device-same-8","parentId":"device-top-31","loaderId":"same-loader","url":"https://host.test/sub"}},
            {"frame":{"id":"device-cross-72","parentId":"device-top-31","loaderId":"cross-loader","url":"https://another.test/"}}]})
    }
    #[test]
    fn review_regression_subframe_navigation_cannot_write_to_main_frame() {
        let mut adapter = adapter();
        for option in ["referrer", "transitionType"] {
            let mut params = json!({"url":"https://host.test/new"});
            params[option] = json!("ignored-before-review");
            let rejected =
                adapter.outgoing(json!({"id":1,"method":"Page.navigate","params":params}));
            assert!(rejected.wire.is_empty());
            assert_eq!(rejected.client[0]["error"]["code"], -32602);
        }
        let sent = adapter.outgoing(json!({"id":2,"method":"Page.navigate","params":{"url":"https://host.test/new","frameId":"child"}}));
        let rejected = adapter.incoming(json!({"id":sent.wire[0]["id"],"result":{"frameTree":{"frame":{"id":"main","url":"https://host.test/"},"childFrames":[{"frame":{"id":"child","url":"https://other.test/"}}]}}}));
        assert!(
            rejected.wire.is_empty(),
            "must not evaluate in the default main context"
        );
        assert_eq!(rejected.client[0]["error"]["code"], -32601);
    }

    #[test]
    fn frame_tree_preserves_native_iframe_ids_and_navigate_waits_for_commit() {
        let mut adapter = adapter();
        let sent = adapter.outgoing(json!({"id":7,"method":"Page.getFrameTree"}));
        assert!(sent.client.is_empty());
        assert_eq!(sent.wire[0]["method"], "Page.getResourceTree");
        let response =
            adapter.incoming(json!({"id":sent.wire[0]["id"],"result":{"frameTree":tree()}}));
        assert_eq!(response.client[0]["result"]["frameTree"], tree());
        let sent = adapter.outgoing(
            json!({"id":8,"method":"Page.navigate","params":{"url":"https://host.test/new"}}),
        );
        let evaluate =
            adapter.incoming(json!({"id":sent.wire[0]["id"],"result":{"frameTree":tree()}}));
        assert_eq!(evaluate.wire[0]["method"], "Runtime.evaluate");
        assert!(evaluate.client.is_empty());
        let accepted = adapter.incoming(json!({"id":evaluate.wire[0]["id"],"result":{}}));
        assert!(
            accepted.client.is_empty(),
            "expression acknowledgement is not a navigation commit"
        );
        let unrelated=adapter.incoming(json!({"method":"Page.frameNavigated","params":{"frame":{"id":"device-cross-72","url":"https://host.test/new"}}}));
        assert_eq!(unrelated.client.len(), 1);
        assert!(unrelated.client[0].get("id").is_none());
        let committed=adapter.incoming(json!({"method":"Page.frameNavigated","params":{"frame":{"id":"device-top-31","loaderId":"loader-new","url":"https://host.test/new"}}}));
        assert_eq!(committed.client[0]["id"], 8);
        assert_eq!(
            committed.client[0]["result"],
            json!({"frameId":"device-top-31","loaderId":"loader-new"})
        );
        assert_eq!(committed.client[1]["method"], "Page.frameNavigated");
    }
    #[test]
    fn screenshot_uses_real_viewport_then_real_png_and_refuses_jpeg() {
        let mut adapter = adapter();
        assert_eq!(
            adapter
                .outgoing(
                    json!({"id":1,"method":"Page.captureScreenshot","params":{"format":"jpeg"}})
                )
                .client[0]["error"]["code"],
            -32602
        );
        let viewport = adapter.outgoing(json!({"id":2,"method":"Page.captureScreenshot"}));
        assert!(viewport.client.is_empty());
        assert_eq!(viewport.wire[0]["method"], "Runtime.evaluate");
        let capture=adapter.incoming(json!({"id":viewport.wire[0]["id"],"result":{"result":{"type":"object","value":{"x":2,"y":3,"width":20,"height":30}}}}));
        assert_eq!(capture.wire[0]["method"], "Page.snapshotRect");
        assert_eq!(capture.wire[0]["params"]["width"], 20.0);
        let mut encoded = Vec::new();
        {
            let mut encoder = png::Encoder::new(&mut encoded, 2, 1);
            encoder.set_color(png::ColorType::Rgb);
            encoder.set_depth(png::BitDepth::Eight);
            encoder
                .write_header()
                .unwrap()
                .write_image_data(&[255, 0, 0, 0, 255, 0])
                .unwrap();
        }
        let png = STANDARD.encode(encoded);
        let response=adapter.incoming(json!({"id":capture.wire[0]["id"],"result":{"dataURL":format!("data:image/png;base64,{png}")}}));
        assert_eq!(response.client[0], json!({"id":2,"result":{"data":png}}));
        let send=adapter.outgoing(json!({"id":3,"method":"Page.captureScreenshot","params":{"clip":{"x":0,"y":0,"width":2,"height":2,"scale":1}}}));
        let failure = adapter.incoming(
            json!({"id":send.wire[0]["id"],"result":{"dataURL":"data:image/jpeg;base64,ZmFrZQ=="}}),
        );
        assert!(failure.client[0].get("error").is_some());
    }
    #[test]
    fn fetch_requires_device_acknowledgements_and_real_paused_request_ids() {
        let mut adapter = adapter();
        let add = adapter.outgoing(json!({"id":1,"method":"Fetch.enable"}));
        assert!(add.client.is_empty());
        assert_eq!(add.wire[0]["method"], "Network.addInterception");
        let enable = adapter.incoming(json!({"id":add.wire[0]["id"],"result":{}}));
        assert!(enable.client.is_empty());
        assert_eq!(enable.wire[0]["method"], "Network.setInterceptionEnabled");
        assert_eq!(
            adapter
                .incoming(json!({"id":enable.wire[0]["id"],"result":{}}))
                .client[0]["id"],
            1
        );
        assert!(adapter.outgoing(json!({"id":2,"method":"Fetch.continueRequest","params":{"requestId":"never-existed"}})).client[0].get("error").is_some());
        adapter.incoming(json!({"method":"Network.requestWillBeSent","params":{"requestId":"native-request-89","frameId":"device-cross-72","type":"Document","request":{"url":"https://another.test/","method":"GET","headers":{}}}}));
        let paused=adapter.incoming(json!({"method":"Network.requestIntercepted","params":{"requestId":"native-request-89","request":{"url":"https://another.test/","method":"GET","headers":{}}}}));
        assert_eq!(paused.client[0]["method"], "Fetch.requestPaused");
        assert_eq!(paused.client[0]["params"]["frameId"], "device-cross-72");
        let resume=adapter.outgoing(json!({"id":3,"method":"Fetch.continueRequest","params":{"requestId":"native-request-89"}}));
        assert!(resume.client.is_empty());
        assert_eq!(resume.wire[0]["method"], "Network.interceptContinue");
        let failed=adapter.incoming(json!({"id":resume.wire[0]["id"],"error":{"code":-32001,"message":"device rejected request"}}));
        assert_eq!(
            failed.client[0],
            json!({"id":3,"error":{"code":-32001,"message":"device rejected request"}})
        );
        assert!(adapter.paused.contains_key("native-request-89"));
        let retry=adapter.outgoing(json!({"id":4,"method":"Fetch.continueRequest","params":{"requestId":"native-request-89"}}));
        adapter.incoming(json!({"id":retry.wire[0]["id"],"result":{}}));
        assert!(!adapter.paused.contains_key("native-request-89"));
    }
    #[test]
    fn unsupported_input_metrics_and_expired_commands_cannot_succeed() {
        let mut adapter = adapter();
        for method in [
            "Input.dispatchMouseEvent",
            "Input.dispatchKeyEvent",
            "Memory.getDOMCounters",
        ] {
            assert_eq!(
                adapter.outgoing(json!({"id":1,"method":method})).client[0]["error"]["code"],
                -32601
            );
        }
        let sent = adapter.outgoing(json!({"id":9,"method":"Page.enable"}));
        adapter
            .pending
            .get_mut(&sent.wire[0]["id"].as_i64().unwrap())
            .unwrap()
            .deadline = Instant::now();
        assert_eq!(adapter.expired()[0]["id"], 9);
        assert!(
            adapter
                .incoming(json!({"id":sent.wire[0]["id"],"result":{}}))
                .client
                .is_empty(),
            "late responses cannot turn timeouts into success"
        );
    }

    #[test]
    fn page_reload_and_console_message_translation() {
        let mut adapter = adapter();
        let reload = adapter
            .outgoing(json!({"id": 10, "method": "Page.reload", "params": {"ignoreCache": true}}));
        assert_eq!(reload.wire[0]["method"], "Page.reload");
        assert_eq!(reload.wire[0]["params"]["ignoreCache"], true);

        let console_msg = json!({
            "method": "Console.messageAdded",
            "params": {
                "message": {
                    "source": "console-api",
                    "level": "error",
                    "text": "test console error message"
                }
            }
        });
        let output = adapter.incoming(console_msg.clone());
        assert_eq!(output.client, vec![console_msg], "native console evidence must be preserved without fabricating a Runtime context or timestamp");

        let download = adapter.outgoing(json!({"id": 11, "method": "Page.setDownloadBehavior"}));
        assert_eq!(download.client[0]["error"]["code"], -32601);
    }
}
