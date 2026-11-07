//! Explicit Node client gates over the production Rust CDP bridge.
//! A passing Playwright rejection fixture proves a gap, not a client workflow.

use super::client_fixtures::*;
use super::*;

const PUPPETEER: JsTarget = JsTarget {
    page: 1041,
    listing: "client-js-puppeteer",
    url: "https://example.invalid/owned-client-puppeteer",
    is_web: false,
};
const PLAYWRIGHT: JsTarget = JsTarget {
    page: 1042,
    listing: "client-js-playwright",
    url: "https://example.invalid/owned-client-playwright",
    is_web: false,
};
const ABORTED: JsTarget = JsTarget {
    page: 1043,
    listing: "client-js-aborted",
    url: "https://example.invalid/owned-client-aborted",
    is_web: false,
};
const CONSOLE_TARGET: JsTarget = JsTarget {
    page: 1051,
    listing: "client-js-console",
    url: "https://example.invalid/owned-client-console",
    is_web: false,
};
const WEB_TARGET: JsTarget = JsTarget {
    page: 1052,
    listing: "client-web-page",
    url: "https://example.invalid/owned-client-web",
    is_web: true,
};

async fn denied_without_source_io(port: u16, source: &JsSource) -> Result<()> {
    let response = client()
        .get(format!("http://127.0.0.1:{port}/json/list"))
        .send()
        .await?;
    if response.status() != StatusCode::UNAUTHORIZED || source.calls() != 0 {
        return Err(anyhow!(
            "actual client fixture bypassed production admission"
        ));
    }
    Ok(())
}

/// All cleanup results are collected before inspecting client behavior. In
/// particular a child failure cannot skip original peer joins or EOF checks.
async fn gate(
    client_name: &str,
    target: JsTarget,
    expectation: &str,
) -> (Capture, Vec<PeerReceipt>, JsonValue) {
    gate_with_closure(client_name, target, expectation, None).await
}

async fn gate_with_closure(
    client_name: &str,
    target: JsTarget,
    expectation: &str,
    closure: Option<NativeClosure>,
) -> (Capture, Vec<PeerReceipt>, JsonValue) {
    let source = match closure {
        Some(closure) => JsSource::with_closure(target, closure),
        None => JsSource::new(target, false),
    };
    let (port, server, _policy) = start_js(source.clone()).await;
    let captured = async {
        denied_without_source_io(port, &source).await?;
        let child = launch_client(
            client_name,
            port,
            target,
            expectation,
            source.evidence.clone(),
        )?;
        child.finish().await
    }
    .await;
    let stopped = server.stop().await;
    let released = source_owners_released(&source).await;
    let joined = source.finish().await;
    let evidence = snapshot(&source.evidence);
    safe_bytes(&serde_json::to_vec(&evidence).unwrap());
    if let Ok(capture) = &captured {
        safe_bytes(&capture.stdout);
        safe_bytes(&capture.stderr);
        let machine_report = serde_json::from_slice::<JsonValue>(&capture.stdout).ok();
        if machine_report.is_none() {
            println!(
                "F03 invalid bounded client report: {}",
                json!({"stdout": String::from_utf8_lossy(&capture.stdout),
                    "stderr": String::from_utf8_lossy(&capture.stderr)})
            );
        }
        println!(
            "F03 owned-client cleanup evidence: {}",
            json!({"client": machine_report, "native": evidence})
        );
    }
    // Cleanup/EOF failures take priority over a behavioral assertion.
    stopped.expect("owned production CDP listener joined");
    released.expect("production upgraded browser owner released its source");
    let receipts = joined.expect("original JSContext peers joined without abort");
    assert!(
        !receipts.is_empty(),
        "actual client did not reach admitted Rust source"
    );
    assert_eq!(source.calls(), receipts.len());
    assert!(
        receipts.iter().all(|receipt| receipt.eof),
        "actual peer EOF required"
    );
    assert!(
        receipts
            .iter()
            .all(|receipt| receipt.setups == receipt.teardowns),
        "every native setup requires teardown before EOF"
    );
    let captured = captured.expect("actual Node client exited and both bounded pipes joined");
    (captured, receipts, evidence)
}

fn safe_bytes(bytes: &[u8]) {
    for private in [TOKEN, DEVICE] {
        assert!(
            !bytes
                .windows(private.len())
                .any(|part| part == private.as_bytes()),
            "actual client evidence leaked private admission data"
        );
    }
}

fn report(capture: &Capture, client_name: &str, version: &str, expectation: &str) -> JsonValue {
    assert!(!capture.stopped, "actual client exceeded its total budget");
    assert!(
        capture.status.success(),
        "actual client contract failed; inspect bounded safe machine evidence"
    );
    assert!(
        capture.stderr.is_empty(),
        "unexpected actual client stderr; raw diagnostics withheld"
    );
    let report: JsonValue = serde_json::from_slice(&capture.stdout)
        .expect("actual client emits one bounded machine report");
    assert_eq!(report["schema_version"], 1);
    assert_eq!(report["client"], client_name);
    assert_eq!(report["version"], version);
    assert_eq!(report["expectation"], expectation);
    assert_eq!(report["fulfilled"], true);
    assert!(report.get("cleanup_failures").is_none());
    assert!(report["trace"].as_array().unwrap().len() <= 512);
    assert!(report["trace_bytes"].as_u64().unwrap() <= 256 * 1024);
    report
}

fn at(trace: &[JsonValue], direction: &str, method: &str) -> usize {
    trace
        .iter()
        .position(|item| item["direction"] == direction && item["message"]["method"] == method)
        .expect("actual client wire method absent")
}

fn response_after(trace: &[JsonValue], request: usize) -> &JsonValue {
    let message = &trace[request]["message"];
    trace
        .iter()
        .skip(request + 1)
        .find(|item| {
            item["direction"] == "receive"
                && item["message"]["id"] == message["id"]
                && item["message"].get("sessionId") == message.get("sessionId")
        })
        .map(|item| &item["message"])
        .expect("actual client correlated reply absent")
}

fn assert_native_order(evidence: &JsonValue, receipts: &[PeerReceipt]) {
    let records = evidence["records"].as_array().unwrap();
    for receipt in receipts {
        let eof = records
            .iter()
            .position(|item| item["event"] == "peer_eof" && item["stream"] == receipt.stream)
            .expect("native EOF trace absent");
        let joined = records
            .iter()
            .position(|item| item["event"] == "peer_joined" && item["stream"] == receipt.stream)
            .expect("original task join trace absent");
        assert!(eof < joined);
        if receipt.setups != 0 {
            let setup = records
                .iter()
                .position(|item| {
                    item["stream"] == receipt.stream
                        && item["event"] == "plist_receive"
                        && item["message"]["__selector"] == "_rpc_forwardSocketSetup:"
                })
                .unwrap();
            let teardown = records
                .iter()
                .position(|item| {
                    item["stream"] == receipt.stream
                        && item["event"] == "plist_receive"
                        && item["message"]["__selector"] == "_rpc_forwardDidClose:"
                })
                .unwrap();
            assert!(setup < teardown && teardown < eof);
        }
    }
    let reaped = records
        .iter()
        .position(|item| item["event"] == "child_reaped")
        .unwrap();
    let pipes = records
        .iter()
        .position(|item| item["event"] == "child_pipes_joined")
        .unwrap();
    assert!(reaped < pipes);
    assert_eq!(records[reaped]["reaped"], true);
    assert_eq!(records[pipes]["complete"], true);
}

#[tokio::test]
#[ignore = "requires npm ci in scripts/cdp-clients and Node; run the explicit F03 client gate"]
async fn pinned_puppeteer_uses_rust_jscontext_runtime_workflow() {
    let (capture, receipts, evidence) = gate("puppeteer", PUPPETEER, "runtime-workflow").await;
    let report = report(&capture, "puppeteer", "25.12.0", "runtime-workflow");
    assert_eq!(report["status"], "runtime-workflow");
    assert_eq!(
        report["steps"],
        json!([
            "initialized",
            "discovered",
            "attached",
            "context-selected",
            "evaluated",
            "detached",
            "disconnected"
        ])
    );
    assert_eq!(report["evaluated"], 42);
    assert_eq!(report["contexts"], json!([1]));
    assert_eq!(report["unknown"]["method"], "Runtime.evaluate");
    assert_eq!(report["unknown"]["stage"], "unknown-context");
    let trace = report["trace"].as_array().unwrap();
    let discover = at(trace, "send", "Target.setDiscoverTargets");
    let attach = at(trace, "send", "Target.attachToTarget");
    let enable = at(trace, "send", "Runtime.enable");
    let context = at(trace, "receive", "Runtime.executionContextCreated");
    let evaluate = at(trace, "send", "Runtime.evaluate");
    let detach = at(trace, "send", "Target.detachFromTarget");
    assert!(
        discover < attach
            && attach < enable
            && enable < context
            && context < evaluate
            && evaluate < detach
    );
    let context_id = trace[context]["message"]["params"]["context"]["id"]
        .as_u64()
        .unwrap();
    assert_eq!(
        trace[evaluate]["message"]["params"]["contextId"],
        context_id
    );
    assert_eq!(
        response_after(trace, evaluate)["result"]["result"]["value"],
        42
    );
    let evaluations: Vec<_> = trace
        .iter()
        .enumerate()
        .filter(|(_, item)| {
            item["direction"] == "send" && item["message"]["method"] == "Runtime.evaluate"
        })
        .map(|(index, _)| index)
        .collect();
    assert_eq!(evaluations.len(), 2);
    assert_eq!(
        trace[evaluations[1]]["message"]["params"]["contextId"],
        context_id + 1
    );
    assert_eq!(
        response_after(trace, evaluations[1])["error"]["code"],
        -32602
    );
    assert_eq!(response_after(trace, detach)["result"], json!({}));
    assert_eq!(
        receipts
            .iter()
            .map(|receipt| receipt.runtime_enables)
            .sum::<usize>(),
        1
    );
    assert_eq!(
        receipts
            .iter()
            .map(|receipt| receipt.console_enables)
            .sum::<usize>(),
        1
    );
    // Unknown context is rejected by the production adapter before native IO.
    assert_eq!(
        receipts
            .iter()
            .map(|receipt| receipt.evaluations)
            .sum::<usize>(),
        1
    );
    assert_native_order(&evidence, &receipts);
}

#[tokio::test]
#[ignore = "requires npm ci in scripts/cdp-clients and Node; this gate records the F03 Playwright gap"]
async fn pinned_playwright_records_download_behavior_initialization_rejection() {
    let (capture, receipts, evidence) =
        gate("playwright", PLAYWRIGHT, "initialization-rejection").await;
    let report = report(&capture, "playwright", "1.63.0", "initialization-rejection");
    assert_eq!(report["status"], "initialization-rejection");
    assert_eq!(report["contexts"], 1);

    let trace = report["trace"].as_array().unwrap();
    let download = at(trace, "send", "Browser.setDownloadBehavior");
    assert_eq!(response_after(trace, download)["result"], json!({}));

    assert_eq!(
        receipts
            .iter()
            .map(|receipt| receipt.evaluations)
            .sum::<usize>(),
        0
    );
    assert_native_order(&evidence, &receipts);
}

#[tokio::test]
#[ignore = "requires npm ci in scripts/cdp-clients and Node; actual child abort/EOF cleanup gate"]
async fn aborted_actual_client_waiter_reaps_child_and_joins_original_jscontext_peer() {
    let source = JsSource::new(ABORTED, true);
    let (port, server, _policy) = start_js(source.clone()).await;
    let admission = denied_without_source_io(port, &source).await;
    let launched = admission.and_then(|()| {
        launch_client(
            "puppeteer",
            port,
            ABORTED,
            "runtime-workflow",
            source.evidence.clone(),
        )
    });
    let result = match launched {
        Ok(child) => {
            let mut receipt = child.receipt.clone();
            let waiter = tokio::spawn(child.finish());
            let observed =
                tokio::time::timeout(CLIENT_BUDGET, source.runtime_seen.notified()).await;
            waiter.abort();
            let joined = waiter.await;
            let cleanup = tokio::time::timeout(CLEANUP_BUDGET, async {
                loop {
                    if let Some(value) = receipt.borrow().clone() {
                        return Ok::<_, anyhow::Error>(value);
                    }
                    receipt.changed().await.map_err(anyhow::Error::from)?;
                }
            })
            .await;
            Ok((observed, joined, cleanup))
        }
        Err(error) => Err(error),
    };
    let stopped = server.stop().await;
    let released = source_owners_released(&source).await;
    let peers = source.finish().await;
    stopped.expect("abort fixture production listener joined");
    released.expect("aborted client production upgraded owner released its source");
    let peers = peers.expect("abort fixture original native peers joined");
    assert_eq!(source.calls(), peers.len());
    assert!(!peers.is_empty());
    assert!(peers
        .iter()
        .all(|peer| peer.eof && peer.setups == peer.teardowns));
    let (observed, joined, cleanup) = result.expect("actual client launch");
    observed.expect("actual Runtime.enable consumed before cancelling waiter");
    let cancelled = match joined {
        Err(error) => error,
        Ok(result) => {
            drop(result);
            panic!("actual client waiter finished before requested abort");
        }
    };
    assert!(cancelled.is_cancelled());
    let receipt = cleanup
        .expect("owned child cleanup supervisor completed")
        .expect("owned child cleanup receipt");
    assert!(
        receipt.stopped && receipt.reaped && receipt.pipes_joined,
        "Drop requires actual child wait and bounded pipe joins"
    );
    let evidence = snapshot(&source.evidence);
    safe_bytes(&serde_json::to_vec(&evidence).unwrap());
    assert_native_order(&evidence, &peers);
    println!("F03 actual-client abort evidence: {}", evidence);
}

async fn native_closure_gate(client_name: &str, page: u64, closure: NativeClosure) {
    let target = JsTarget {
        page,
        listing: "client-native-closure",
        url: "https://example.invalid/owned-native-closure",
        is_web: false,
    };
    let expectation = match closure {
        NativeClosure::Page => "native-page-closure",
        NativeClosure::Application => "native-application-closure",
        NativeClosure::DiscoveryPage => "native-discovery-page-closure",
        NativeClosure::DiscoveryApplication => "native-discovery-application-closure",
        NativeClosure::DiscoveryRace => "native-discovery-race-closure",
    };
    let (capture, receipts, evidence) =
        gate_with_closure(client_name, target, expectation, Some(closure)).await;
    let version = if client_name == "puppeteer" {
        "25.12.0"
    } else {
        "1.63.0"
    };
    let report = report(&capture, client_name, version, expectation);
    assert_eq!(report["contexts"], json!([1]));
    assert_eq!(report["evaluated"], 42);
    assert_eq!(report["destroyed"]["executionContextId"], 1);
    assert_eq!(
        report["destroyed"]["executionContextUniqueId"],
        "jscontext.1"
    );
    let trace = report["trace"].as_array().unwrap();
    // Decode real standard transport envelopes only for correlation assertions;
    // the report retains every original pinned-client packet unchanged.
    let decoded: Vec<_> = trace
        .iter()
        .map(|item| {
            let mut item = item.clone();
            if matches!(
                item["message"]["method"].as_str(),
                Some("Target.sendMessageToTarget" | "Target.receivedMessageFromTarget")
            ) {
                let mut nested: JsonValue =
                    serde_json::from_str(item["message"]["params"]["message"].as_str().unwrap())
                        .unwrap();
                nested["sessionId"] = item["message"]["params"]["sessionId"].clone();
                item["message"] = nested;
            }
            item
        })
        .collect();
    let created = at(&decoded, "receive", "Runtime.executionContextCreated");
    let destroyed = at(&decoded, "receive", "Runtime.executionContextDestroyed");
    let detached = decoded
        .iter()
        .enumerate()
        .skip(destroyed + 1)
        .find(|(_, item)| {
            item["direction"] == "receive"
                && item["message"]["method"] == "Target.detachedFromTarget"
        })
        .map(|(index, _)| index)
        .expect("native detach absent after context destruction");
    assert!(created < destroyed && destroyed < detached);
    assert_eq!(
        decoded[destroyed]["message"]["sessionId"],
        decoded[created]["message"]["sessionId"]
    );
    assert_eq!(
        decoded[detached]["message"]["params"]["sessionId"],
        decoded[destroyed]["message"]["sessionId"]
    );
    assert_eq!(
        decoded
            .iter()
            .filter(|item| item["direction"] == "receive"
                && item["message"]["method"] == "Runtime.executionContextDestroyed")
            .count(),
        1
    );
    let evaluations: Vec<_> = decoded
        .iter()
        .enumerate()
        .filter(|(_, item)| {
            item["direction"] == "send" && item["message"]["method"] == "Runtime.evaluate"
        })
        .map(|(index, _)| index)
        .collect();
    assert_eq!(evaluations.len(), 4);
    assert_eq!(
        response_after(&decoded, evaluations[2])["result"]["result"]["value"],
        42
    );
    assert_eq!(
        response_after(&decoded, evaluations[1])["error"]["code"],
        -32602
    );
    assert_eq!(
        receipts
            .iter()
            .map(|receipt| receipt.evaluations)
            .sum::<usize>(),
        3
    );
    assert_eq!(
        receipts
            .iter()
            .map(|receipt| receipt.runtime_enables)
            .sum::<usize>(),
        1
    );
    assert_eq!(
        receipts
            .iter()
            .map(|receipt| receipt.console_enables)
            .sum::<usize>(),
        1
    );
    let records = evidence["records"].as_array().unwrap();
    let selected = receipts
        .iter()
        .find(|receipt| receipt.evaluations == 3)
        .unwrap();
    let independent = matches!(
        closure,
        NativeClosure::DiscoveryPage | NativeClosure::DiscoveryApplication
    );
    let closing_selector = match closure {
        NativeClosure::Page | NativeClosure::DiscoveryRace | NativeClosure::DiscoveryPage => {
            "_rpc_applicationSentListing:"
        }
        NativeClosure::Application => "_rpc_applicationDisconnected:",
        NativeClosure::DiscoveryApplication => "_rpc_reportConnectedApplicationList:",
    };
    let close = records
        .iter()
        .enumerate()
        .filter(|(_, item)| {
            (if independent {
                item["stream"] != selected.stream
            } else {
                item["stream"] == selected.stream
            }) && item["event"] == "plist_send"
                && item["message"]["__selector"] == closing_selector
                && if matches!(closure, NativeClosure::DiscoveryApplication) {
                    item["message"]["__argument"]["WIRApplicationDictionaryKey"] == json!({})
                } else if independent {
                    item["message"]["__argument"]["WIRApplicationIdentifierKey"] == "PID:42"
                        && item["message"]["__argument"]["WIRListingKey"] == json!({})
                } else {
                    item["message"]["__argument"]["WIRApplicationIdentifierKey"] == "PID:42"
                }
        })
        .map(|(index, _)| index)
        .next()
        .unwrap();
    let teardown = records
        .iter()
        .position(|item| {
            item["stream"] == selected.stream
                && item["event"] == "plist_receive"
                && item["message"]["__selector"] == "_rpc_forwardDidClose:"
        })
        .unwrap();
    assert!(close < teardown);
    if client_name == "playwright" {
        assert_eq!(report["unknown_session_rejected"], true);
        assert!(!trace
            .iter()
            .any(|item| item["message"]["method"] == "Browser.setDownloadBehavior"));
        assert!(trace.iter().any(
            |item| item["direction"] == "receive" && item["message"]["error"]["code"] == -32001
        ));
    }
    assert_native_order(&evidence, &receipts);
}

#[tokio::test]
#[ignore = "requires cached pinned Node clients; explicit F04 native closure gate"]
async fn pinned_puppeteer_native_page_closure_destroys_context_before_flat_detach() {
    native_closure_gate("puppeteer", 1044, NativeClosure::Page).await;
}

#[tokio::test]
#[ignore = "requires cached pinned Node clients; explicit F04 native closure gate"]
async fn pinned_puppeteer_native_application_closure_destroys_context_before_flat_detach() {
    native_closure_gate("puppeteer", 1045, NativeClosure::Application).await;
}

#[tokio::test]
#[ignore = "requires cached pinned Node clients; narrow public noDefaults/nonflat F04 gate"]
async fn pinned_playwright_native_page_closure_destroys_context_before_nonflat_detach() {
    native_closure_gate("playwright", 1046, NativeClosure::Page).await;
}

#[tokio::test]
#[ignore = "requires cached pinned Node clients; narrow public noDefaults/nonflat F04 gate"]
async fn pinned_playwright_native_application_closure_destroys_context_before_nonflat_detach() {
    native_closure_gate("playwright", 1047, NativeClosure::Application).await;
}

#[tokio::test]
#[ignore = "requires cached pinned Node clients; independent discovery closure"]
async fn pinned_puppeteer_native_discovery_page_closure_delivers_context_before_detach() {
    native_closure_gate("puppeteer", 1048, NativeClosure::DiscoveryPage).await;
}

#[tokio::test]
#[ignore = "requires cached pinned Node clients; independent discovery closure"]
async fn pinned_playwright_native_discovery_application_closure_delivers_context_before_detach() {
    native_closure_gate("playwright", 1049, NativeClosure::DiscoveryApplication).await;
}

#[tokio::test]
#[ignore = "requires cached pinned Node clients; selected/discovery closure race"]
async fn pinned_puppeteer_native_discovery_race_closure_destroys_context_once() {
    native_closure_gate("puppeteer", 1050, NativeClosure::DiscoveryRace).await;
}

#[tokio::test]
#[ignore = "requires cached pinned Node clients; explicit F04 console workflow gate"]
async fn pinned_puppeteer_jscontext_console_workflow() {
    let (capture, receipts, evidence) = gate("puppeteer", CONSOLE_TARGET, "console-workflow").await;
    let report = report(&capture, "puppeteer", "25.12.0", "console-workflow");
    assert_eq!(report["status"], "console-workflow");
    assert_eq!(report["console_event"]["type"], "log");
    assert_eq!(
        report["console_event"]["args"][0]["value"],
        "test console log from native"
    );
    assert_eq!(report["console_event"]["executionContextId"], 1);
    assert_eq!(report["console_event"]["timestamp"], 12345);
    assert_native_order(&evidence, &receipts);
}

#[tokio::test]
#[ignore = "requires cached pinned Node clients; explicit F04 five domain workflow gate"]
async fn pinned_puppeteer_five_domain_workflow() {
    let (capture, receipts, evidence) = gate("puppeteer", WEB_TARGET, "five-domain-workflow").await;
    let report = report(&capture, "puppeteer", "25.12.0", "five-domain-workflow");
    assert_eq!(report["status"], "five-domain-workflow");
    assert_eq!(
        report["steps"],
        json!([
            "initialized",
            "discovered",
            "attached",
            "context-selected",
            "evaluated",
            "frame-tree-verified",
            "screenshot-verified",
            "input-verified",
            "input-rejected",
            "fetch-enabled",
            "fetch-disabled",
            "download-verified",
            "download-rejected",
            "perf-rejected",
            "mem-rejected",
            "detached",
            "disconnected"
        ])
    );
    assert_native_order(&evidence, &receipts);
}

#[tokio::test]
#[ignore = "requires cached pinned Node clients; explicit F04 native iframe routing gate"]
async fn pinned_puppeteer_same_and_cross_origin_iframe_navigation() {
    let (capture, receipts, evidence) = gate("puppeteer", WEB_TARGET, "iframe-workflow").await;
    let report = report(&capture, "puppeteer", "25.12.0", "iframe-workflow");
    assert_eq!(report["commits"][0]["id"], "1.3");
    assert_eq!(report["commits"][1]["id"], "1.5");
    assert_native_order(&evidence, &receipts);
}

#[tokio::test]
#[ignore = "requires cached pinned Node clients; explicit F04 native world lifecycle gate"]
async fn pinned_puppeteer_native_world_lifecycle_and_retired_tree() {
    let (capture, receipts, evidence) = gate("puppeteer", WEB_TARGET, "world-workflow").await;
    let report = report(&capture, "puppeteer", "25.12.0", "world-workflow");
    for id in [101, 301, 302] {
        assert!(report["destroyedContexts"]
            .as_array()
            .unwrap()
            .contains(&json!(id)));
    }
    assert!(!report["contexts"].as_array().unwrap().contains(&json!(303)));
    let native: Vec<(String, JsonValue)> = evidence["records"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|record| record["event"] == "runtime_send" || record["event"] == "runtime_receive")
        .map(|record| {
            let event = record["event"].as_str().unwrap().to_owned();
            let message = &record["message"];
            let inner = if matches!(
                message["method"].as_str(),
                Some("Target.sendMessageToTarget" | "Target.dispatchMessageFromTarget")
            ) {
                serde_json::from_str(message["params"]["message"].as_str().unwrap()).unwrap()
            } else {
                message.clone()
            };
            (event, inner)
        })
        .collect();
    for (id, kind) in [(301, "user"), (302, "internal"), (303, "user")] {
        let context = native
            .iter()
            .find(|(event, message)| {
                event == "runtime_send"
                    && message["method"] == "Runtime.executionContextCreated"
                    && message["params"]["context"]["id"] == id
            })
            .unwrap();
        assert_eq!(context.1["params"]["context"]["type"], kind);
        assert!(context.1["params"]["context"].get("origin").is_none());
        assert!(context.1["params"]["context"].get("auxData").is_none());
    }
    let detach = native
        .iter()
        .position(|(event, message)| {
            event == "runtime_send"
                && message["method"] == "Page.frameDetached"
                && message["params"]["frameId"] == "retired"
        })
        .unwrap();
    let tree_request = native
        .iter()
        .enumerate()
        .skip(detach + 1)
        .find(|(_, (event, message))| {
            event == "runtime_receive" && message["method"] == "Page.getResourceTree"
        })
        .unwrap();
    assert!(native
        .iter()
        .skip(tree_request.0 + 1)
        .any(|(event, message)| event == "runtime_send"
            && message["id"] == tree_request.1 .1["id"]
            && message["result"]["frameTree"].is_object()));
    let commands: Vec<JsonValue> = evidence["records"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|record| record["event"] == "runtime_receive")
        .map(|record| {
            let message = &record["message"];
            if message["method"] == "Target.sendMessageToTarget" {
                serde_json::from_str(message["params"]["message"].as_str().unwrap()).unwrap()
            } else {
                message.clone()
            }
        })
        .collect();
    for id in [301, 302] {
        assert!(commands
            .iter()
            .any(|command| command["method"] == "Runtime.evaluate"
                && command["params"]["contextId"] == id));
    }
    assert!(commands
        .iter()
        .any(|command| command["method"] == "Runtime.evaluate"
            && command["params"]["contextId"] == 101
            && command["params"]["expression"]
                .as_str()
                .is_some_and(|value| value.starts_with("window.location.assign("))));
    assert_native_order(&evidence, &receipts);
}

#[tokio::test]
#[ignore = "requires cached pinned Node clients; explicit F04 native memory metrics gate"]
async fn pinned_puppeteer_device_memory_metrics_and_sampling_lifecycle() {
    let (capture, receipts, evidence) = gate("puppeteer", WEB_TARGET, "metrics-workflow").await;
    let report = report(&capture, "puppeteer", "25.12.0", "metrics-workflow");
    assert_eq!(report["metrics"][0]["value"], 12.5);
    assert_native_order(&evidence, &receipts);
}

#[tokio::test]
#[ignore = "requires cached pinned Node clients; explicit native Fetch lifecycle gate"]
async fn pinned_puppeteer_fetch_continue_fulfill_fail_response_and_pending_disable() {
    let (capture, receipts, evidence) = gate("puppeteer", WEB_TARGET, "fetch-workflow").await;
    report(&capture, "puppeteer", "25.12.0", "fetch-workflow");
    assert_native_order(&evidence, &receipts);
}
