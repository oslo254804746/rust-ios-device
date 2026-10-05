use super::*;
use serde_json::{json, Value as JsonValue};
use std::{
    future::Future,
    pin::Pin,
    sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    },
    task::{Context, Poll},
};
use tokio::{
    io::{duplex, AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt, DuplexStream, ReadBuf},
    time::{timeout, Duration, Sleep},
};

pub(super) const SHORT: Duration = Duration::from_millis(20);
const BOUND: Duration = Duration::from_secs(3);

pub(super) async fn bounded<T>(operation: impl Future<Output = T>) -> T {
    timeout(BOUND, operation)
        .await
        .expect("owned session fixture stalled")
}

pub(super) fn socket(message: JsonValue) -> Vec<u8> {
    frame(
        "_rpc_applicationSentData:",
        plist::Dictionary::from_iter([
            ("WIRApplicationIdentifierKey".to_owned(), "PID:42".into()),
            (
                "WIRMessageDataKey".to_owned(),
                plist::Value::Data(serde_json::to_vec(&message).unwrap()),
            ),
        ]),
    )
}

fn frame(selector: &str, argument: plist::Dictionary) -> Vec<u8> {
    let value = plist::Value::Dictionary(plist::Dictionary::from_iter([
        ("__selector".to_owned(), selector.into()),
        ("__argument".to_owned(), plist::Value::Dictionary(argument)),
    ]));
    let mut body = Vec::new();
    plist::to_writer_xml(&mut body, &value).unwrap();
    let mut bytes = (body.len() as u32).to_be_bytes().to_vec();
    bytes.extend(body);
    bytes
}

pub(super) async fn request(peer: &mut DuplexStream) -> plist::Dictionary {
    let size = bounded(peer.read_u32()).await.unwrap() as usize;
    assert!(size < 16384);
    let mut bytes = vec![0; size];
    bounded(peer.read_exact(&mut bytes)).await.unwrap();
    plist::from_bytes(&bytes).unwrap()
}

pub(super) fn payload(request: &plist::Dictionary) -> JsonValue {
    serde_json::from_slice(
        request["__argument"].as_dictionary().unwrap()["WIRSocketDataKey"]
            .as_data()
            .unwrap(),
    )
    .unwrap()
}

fn reply(id: u64) -> Vec<u8> {
    socket(json!({"id": id, "result": {"result": "\"ok\""}}))
}

fn inspector() -> InspectorSession {
    InspectorSession::with_session_id("PID:42", 7, "owned-session")
}

fn automation() -> AutomationSession {
    AutomationSession::with_page("PID:42", "owned-bundle", "owned-session", 7)
}

async fn command<S: AsyncRead + AsyncWrite + Unpin>(
    automatic: bool,
    inspector: &mut InspectorSession,
    automation: &mut AutomationSession,
    client: &mut WebInspectorClient<S>,
    budget: Duration,
) -> Result<JsonValue, WebInspectorError> {
    if automatic {
        automation.get_title(client).await.map(JsonValue::String)
    } else {
        inspector
            .send_command_and_wait(client, "Runtime.enable", json!({}), budget)
            .await
    }
}

pub(super) fn timeout_source(error: &WebInspectorError, budget: Duration) {
    assert_eq!(error.timeout_duration(), Some(budget));
    assert_eq!(crate::classify_error(error), crate::ErrorClass::Timeout);
    assert!(crate::test_native::has_source::<tokio::time::error::Elapsed>(error));
}

async fn rejected<S: AsyncRead + AsyncWrite + Unpin>(client: &mut WebInspectorClient<S>) {
    assert!(!client.is_usable());
    assert!(client.automation_availability().is_none());
    assert!(client.applications().is_empty());
    assert!(client.application_pages("PID:42").is_none());
    client.set_operation_timeout(None);
    let mut another = inspector();
    assert!(matches!(
        another
            .send_command(client, "Runtime.enable", json!({}))
            .await,
        Err(WebInspectorError::Protocol(_))
    ));
    assert!(matches!(
        another.next_raw_message(client, SHORT).await,
        Err(WebInspectorError::Protocol(_))
    ));
    assert!(matches!(
        client.next_event().await,
        Err(WebInspectorError::Protocol(_))
    ));
}

struct DelayedWrite {
    inner: DuplexStream,
    gate: Option<Pin<Box<Sleep>>>,
    delay: Duration,
    written: Arc<AtomicUsize>,
}

impl AsyncRead for DelayedWrite {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<std::io::Result<()>> {
        Pin::new(&mut self.inner).poll_read(cx, buf)
    }
}
impl AsyncWrite for DelayedWrite {
    fn poll_write(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        bytes: &[u8],
    ) -> Poll<std::io::Result<usize>> {
        // Start the delay at the first actual write poll, so fixture setup or
        // scheduler delay cannot shorten the exercised blocked-send stage.
        let delay = self.delay;
        let gate = self
            .gate
            .get_or_insert_with(|| Box::pin(tokio::time::sleep(delay)));
        if gate.as_mut().poll(cx).is_pending() {
            return Poll::Pending;
        }
        let result = Pin::new(&mut self.inner).poll_write(cx, bytes);
        if let Poll::Ready(Ok(size)) = result {
            self.written.fetch_add(size, Ordering::SeqCst);
        }
        result
    }
    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        Pin::new(&mut self.inner).poll_flush(cx)
    }
    fn poll_shutdown(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        Pin::new(&mut self.inner).poll_shutdown(cx)
    }
}

#[tokio::test]
async fn lifecycle_webinspector_session_one_deadline_covers_actual_send_and_late_reply() {
    for automatic in [false, true] {
        let budget = Duration::from_millis(200);
        let stage = Duration::from_millis(125);
        let (stream, mut peer) = duplex(16384);
        let written = Arc::new(AtomicUsize::new(0));
        let stream = DelayedWrite {
            inner: stream,
            gate: None,
            delay: stage,
            written: Arc::clone(&written),
        };
        let mut client = WebInspectorClient::with_connection_id(stream, "owned");
        client.set_operation_timeout(Some(budget));
        let mut inspector = inspector();
        let mut automation = automation();
        let caller = async {
            let error = command(
                automatic,
                &mut inspector,
                &mut automation,
                &mut client,
                budget,
            )
            .await
            .unwrap_err();
            timeout_source(&error, budget);
        };
        let server = async {
            let request = request(&mut peer).await;
            let payload = payload(&request);
            assert_eq!(payload["id"], 1);
            assert_eq!(
                payload["method"],
                if automatic {
                    "Automation.evaluateJavaScriptFunction"
                } else {
                    "Runtime.enable"
                }
            );
            tokio::time::sleep(stage).await;
            peer.write_all(&reply(1)).await.unwrap();
        };
        bounded(async {
            tokio::join!(caller, server);
        })
        .await;
        assert!(written.load(Ordering::SeqCst) > 0);
        rejected(&mut client).await;
        let before = written.load(Ordering::SeqCst);
        assert!(matches!(
            client.report_identifier().await,
            Err(WebInspectorError::Protocol(_))
        ));
        assert_eq!(written.load(Ordering::SeqCst), before);
    }
}

#[tokio::test]
async fn lifecycle_webinspector_session_cancel_after_command_and_unrelated_frame_rejects_late_reply(
) {
    for automatic in [false, true] {
        let (stream, mut peer) = duplex(16384);
        let mut client = WebInspectorClient::with_connection_id(stream, "owned");
        client.set_operation_timeout(Some(Duration::from_secs(1)));
        let mut inspector = inspector();
        let mut automation = automation();
        {
            let operation = command(
                automatic,
                &mut inspector,
                &mut automation,
                &mut client,
                Duration::from_secs(1),
            );
            tokio::pin!(operation);
            let server = async {
                assert_eq!(payload(&request(&mut peer).await)["id"], 1);
                peer.write_all(&socket(
                    json!({"method": "Console.messageAdded", "params": {"queued": true}}),
                ))
                .await
                .unwrap();
                tokio::time::sleep(SHORT).await;
            };
            tokio::select! {
                biased;
                result = &mut operation => panic!("unexpected operation completion: {result:?}"),
                () = bounded(server) => {},
            }
        }
        peer.write_all(&reply(1)).await.unwrap();
        rejected(&mut client).await;
        drop(client);
        let mut remaining = Vec::new();
        bounded(peer.read_to_end(&mut remaining)).await.unwrap();
        assert!(remaining.is_empty(), "no follow-up command may be written");
    }
}

#[tokio::test]
async fn lifecycle_webinspector_session_expiry_after_unrelated_frame_retains_original_budget() {
    for automatic in [false, true] {
        let (stream, mut peer) = duplex(16384);
        let mut client = WebInspectorClient::with_connection_id(stream, "owned");
        client.set_operation_timeout(Some(SHORT));
        let mut inspector = inspector();
        let mut automation = automation();
        let caller = async {
            command(
                automatic,
                &mut inspector,
                &mut automation,
                &mut client,
                Duration::from_secs(1),
            )
            .await
            .unwrap_err()
        };
        let server = async {
            request(&mut peer).await;
            peer.write_all(&socket(json!({"method": "Console.messageAdded"})))
                .await
                .unwrap();
        };
        let (error, ()) = bounded(async { tokio::join!(caller, server) }).await;
        timeout_source(&error, SHORT);
        peer.write_all(&reply(1)).await.unwrap();
        rejected(&mut client).await;
    }
}

#[tokio::test]
async fn lifecycle_webinspector_session_business_errors_complete_and_restore_event_order() {
    for automatic in [false, true] {
        let (stream, mut peer) = duplex(16384);
        let mut client = WebInspectorClient::with_connection_id(stream, "owned");
        client.set_operation_timeout(Some(Duration::from_secs(1)));
        let mut inspector = inspector();
        let mut automation = automation();
        let caller = command(
            automatic,
            &mut inspector,
            &mut automation,
            &mut client,
            Duration::from_secs(1),
        );
        let business = json!({"id": 1, "error": {"code": -32000, "message": "owned-business"}});
        let server = async {
            request(&mut peer).await;
            for message in [
                json!({"method": "first"}),
                json!({"method": "second"}),
                business.clone(),
            ] {
                peer.write_all(&socket(message)).await.unwrap();
            }
        };
        let (result, ()) = bounded(async { tokio::join!(caller, server) }).await;
        if automatic {
            assert!(
                matches!(result, Err(WebInspectorError::Protocol(ref message)) if message == "automation command failed: {\"code\":-32000,\"message\":\"owned-business\"}")
            );
        } else {
            assert_eq!(result.unwrap(), business);
        }
        assert!(client.is_usable());
        for expected in ["first", "second"] {
            let event = bounded(client.next_event()).await.unwrap();
            assert!(
                matches!(event, WebInspectorEvent::SocketData { message, .. } if message["method"] == expected)
            );
        }
        let caller = command(
            automatic,
            &mut inspector,
            &mut automation,
            &mut client,
            Duration::from_secs(1),
        );
        let server = async {
            assert_eq!(payload(&request(&mut peer).await)["id"], 2);
            peer.write_all(&reply(2)).await.unwrap();
        };
        let (result, ()) = bounded(async { tokio::join!(caller, server) }).await;
        assert!(result.is_ok());
        assert!(client.is_usable());
    }
}

#[tokio::test]
async fn lifecycle_webinspector_session_malformed_unmatched_nested_response_invalidates() {
    let (stream, mut peer) = duplex(16384);
    let mut client = WebInspectorClient::with_connection_id(stream, "owned");
    let mut inspector = inspector();
    let caller = inspector.send_command_and_wait(
        &mut client,
        "Runtime.enable",
        json!({}),
        Duration::from_secs(1),
    );
    let server = async {
        request(&mut peer).await;
        peer.write_all(&socket(json!({"method": "unrelated"})))
            .await
            .unwrap();
        peer.write_all(&socket(json!({"method": "Target.dispatchMessageFromTarget", "params": {"message": "{owned-invalid-json"}}))).await.unwrap();
    };
    let (result, ()) = bounded(async { tokio::join!(caller, server) }).await;
    let error = result.unwrap_err();
    assert!(crate::test_native::has_source::<serde_json::Error>(&error));
    assert_eq!(crate::classify_error(&error), crate::ErrorClass::Protocol);
    rejected(&mut client).await;
}

#[tokio::test]
async fn lifecycle_webinspector_session_count_limit_bounds_actual_unmatched_wire_events() {
    for automatic in [false, true] {
        let (stream, mut peer) = duplex(1024 * 1024);
        let mut client = WebInspectorClient::with_connection_id(stream, "owned");
        client.set_operation_timeout(Some(Duration::from_secs(2)));
        let mut inspector = inspector();
        let mut automation = automation();
        let caller = command(
            automatic,
            &mut inspector,
            &mut automation,
            &mut client,
            Duration::from_secs(2),
        );
        let server = async {
            request(&mut peer).await;
            for index in 0..=super::retained::MAX_EVENTS {
                peer.write_all(&socket(
                    json!({"method": "unrelated", "params": {"index": index}}),
                ))
                .await
                .unwrap();
            }
            peer.write_all(&reply(1)).await.unwrap();
        };
        let (result, ()) = bounded(async { tokio::join!(caller, server) }).await;
        let error = result.unwrap_err();
        assert_eq!(
            crate::classify_error(&error),
            crate::ErrorClass::ResourceLimit
        );
        assert!(crate::test_native::has_source::<super::WebInspectorLimit>(
            &error
        ));
        assert!(!format!("{error:?}").contains("owned"));
        rejected(&mut client).await;
    }
}

#[tokio::test]
async fn lifecycle_webinspector_session_zero_and_overflow_reject_before_cached_or_wire_work() {
    for automatic in [false, true] {
        for budget in [Duration::ZERO, Duration::MAX] {
            let (stream, mut peer) = duplex(16384);
            let mut client = WebInspectorClient::with_connection_id(stream, "owned");
            client.set_operation_timeout(Some(budget));
            peer.write_all(&reply(1)).await.unwrap();
            let mut inspector = inspector();
            let mut automation = automation();
            let error = if automatic {
                automation.attach(&mut client, budget).await.unwrap_err()
            } else {
                command(false, &mut inspector, &mut automation, &mut client, budget)
                    .await
                    .unwrap_err()
            };
            assert!(client.is_usable());
            if budget.is_zero() {
                assert!(matches!(error, WebInspectorError::Timeout(value) if value.is_zero()));
            } else {
                assert_eq!(
                    crate::classify_error(&error),
                    crate::ErrorClass::InvalidInput
                );
            }
            client.set_operation_timeout(None);
            let event = bounded(client.next_event()).await.unwrap();
            assert!(
                matches!(event, WebInspectorEvent::SocketData { message, .. } if message["id"] == 1)
            );
            drop(client);
            let mut remaining = Vec::new();
            bounded(peer.read_to_end(&mut remaining)).await.unwrap();
            assert!(remaining.is_empty());
        }
    }
}

#[tokio::test]
async fn lifecycle_webinspector_session_unpolled_call_and_empty_write_cancellation_preserve_client()
{
    for automatic in [false, true] {
        let (stream, _peer) = duplex(16384);
        let written = Arc::new(AtomicUsize::new(0));
        let stream = DelayedWrite {
            inner: stream,
            gate: None,
            delay: Duration::from_secs(1),
            written: Arc::clone(&written),
        };
        let mut client = WebInspectorClient::with_connection_id(stream, "owned");
        let mut inspector = inspector();
        let mut automation = automation();
        drop(command(
            automatic,
            &mut inspector,
            &mut automation,
            &mut client,
            SHORT,
        ));
        assert!(client.is_usable());
        {
            let operation = command(
                automatic,
                &mut inspector,
                &mut automation,
                &mut client,
                Duration::from_secs(1),
            );
            tokio::pin!(operation);
            tokio::select! { biased; result = &mut operation => panic!("unexpected {result:?}"), () = tokio::time::sleep(SHORT) => {} }
        }
        assert!(client.is_usable());
        assert_eq!(written.load(Ordering::SeqCst), 0);
    }
}

#[tokio::test]
async fn lifecycle_webinspector_session_target_parse_failure_after_setup_retains_protocol_error() {
    let (stream, mut peer) = duplex(16384);
    let mut client = WebInspectorClient::with_connection_id(stream, "owned");
    let mut inspector = inspector();
    let caller = inspector.attach(&mut client, true, Duration::from_secs(1));
    let server = async {
        request(&mut peer).await;
        peer.write_all(&socket(json!({"method": "unrelated"})))
            .await
            .unwrap();
        peer.write_all(&socket(
            json!({"method": "Target.targetCreated", "params": {}}),
        ))
        .await
        .unwrap();
    };
    let (result, ()) = bounded(async { tokio::join!(caller, server) }).await;
    assert!(
        matches!(result, Err(WebInspectorError::Protocol(ref message)) if message == "Target.targetCreated missing params.targetInfo.targetId")
    );
    rejected(&mut client).await;
}

#[tokio::test]
async fn lifecycle_webinspector_session_inspector_cap_inherits_through_send_and_idle_read() {
    let (stream, mut peer) = duplex(16384);
    let mut client = WebInspectorClient::with_connection_id(stream, "owned");
    client.set_operation_timeout(Some(Duration::MAX));
    let mut inspector = inspector();
    let caller = inspector.send_command_and_wait(
        &mut client,
        "Runtime.enable",
        json!({}),
        Duration::from_secs(1),
    );
    let server = async {
        request(&mut peer).await;
        peer.write_all(&reply(1)).await.unwrap();
    };
    let (result, ()) = bounded(async { tokio::join!(caller, server) }).await;
    assert_eq!(result.unwrap()["id"], 1);
    assert!(client.is_usable());
    let error = inspector
        .next_raw_message(&mut client, SHORT)
        .await
        .unwrap_err();
    timeout_source(&error, SHORT);
    assert!(client.is_usable());
    // Scope exit restores the configured MAX policy for independent sends.
    assert_eq!(
        crate::classify_error(&client.report_identifier().await.unwrap_err()),
        crate::ErrorClass::InvalidInput
    );
    client.set_operation_timeout(None);
    peer.write_all(&reply(2)).await.unwrap();
    assert_eq!(
        bounded(inspector.next_raw_message(&mut client, SHORT))
            .await
            .unwrap()["id"],
        2
    );
}

#[tokio::test]
async fn lifecycle_webinspector_session_startup_cap_inherits_through_identifier_and_response() {
    let (stream, mut peer) = duplex(16384);
    let mut client = WebInspectorClient::with_connection_id(stream, "owned");
    client.set_operation_timeout(Some(Duration::MAX));
    let caller = client.start_preserving_errors(Duration::from_secs(1));
    let server = async {
        assert_eq!(
            request(&mut peer).await["__selector"].as_string(),
            Some("_rpc_reportIdentifier:")
        );
        peer.write_all(&frame(
            "_rpc_reportCurrentState:",
            plist::Dictionary::from_iter([(
                "WIRAutomationAvailabilityKey".to_owned(),
                plist::Value::String("WIRAutomationAvailabilityAvailable".into()),
            )]),
        ))
        .await
        .unwrap();
    };
    let (result, ()) = bounded(async { tokio::join!(caller, server) }).await;
    result.unwrap();
    assert_eq!(
        client.automation_availability(),
        Some(AutomationAvailability::Available)
    );
    assert!(client.is_usable());
    assert_eq!(
        crate::classify_error(&client.report_identifier().await.unwrap_err()),
        crate::ErrorClass::InvalidInput
    );
}

#[tokio::test]
async fn lifecycle_webinspector_session_inspector_attach_cancel_after_setup_rejects_late_target() {
    for unpaused in [false, true] {
        let (stream, mut peer) = duplex(16384);
        let mut client = WebInspectorClient::with_connection_id(stream, "owned");
        let mut inspector = inspector();
        {
            let operation = async {
                if unpaused {
                    inspector
                        .attach_unpaused(&mut client, true, Duration::from_secs(1))
                        .await
                } else {
                    inspector
                        .attach(&mut client, true, Duration::from_secs(1))
                        .await
                }
            };
            tokio::pin!(operation);
            let server = async {
                let request = request(&mut peer).await;
                assert_eq!(
                    request["__selector"].as_string(),
                    Some("_rpc_forwardSocketSetup:")
                );
                let args = request["__argument"].as_dictionary().unwrap();
                assert_eq!(
                    args.get("WIRAutomaticallyPause")
                        .and_then(plist::Value::as_boolean),
                    unpaused.then_some(false)
                );
                peer.write_all(&socket(json!({"method": "unrelated"})))
                    .await
                    .unwrap();
                tokio::time::sleep(SHORT).await;
            };
            tokio::select! { biased; result = &mut operation => panic!("unexpected {result:?}"), () = bounded(server) => {} }
        }
        peer.write_all(&socket(json!({"method": "Target.targetCreated", "params": {"targetInfo": {"targetId": "late"}}}))).await.unwrap();
        assert!(inspector.target_id().is_none());
        rejected(&mut client).await;
    }
}

#[tokio::test]
async fn lifecycle_webinspector_session_automation_attach_cancel_after_request_invalidates() {
    let (stream, mut peer) = duplex(16384);
    let mut client = WebInspectorClient::with_connection_id(stream, "owned");
    let mut automation =
        AutomationSession::with_session_id("PID:42", "owned-bundle", "owned-session");
    {
        let operation = automation.attach(&mut client, Duration::from_secs(1));
        tokio::pin!(operation);
        let server = async {
            assert_eq!(
                request(&mut peer).await["__selector"].as_string(),
                Some("_rpc_forwardAutomationSessionRequest:")
            );
            assert_eq!(
                request(&mut peer).await["__selector"].as_string(),
                Some("_rpc_forwardGetListing:")
            );
            tokio::time::sleep(SHORT).await;
        };
        tokio::select! { biased; result = &mut operation => panic!("unexpected {result:?}"), () = bounded(server) => {} }
    }
    assert_eq!(automation.page_id(), 0);
    rejected(&mut client).await;
}

#[tokio::test]
async fn lifecycle_webinspector_session_due_deadline_does_not_poll_ready_work() {
    let polled = Arc::new(AtomicUsize::new(0));
    let count = Arc::clone(&polled);
    let mut client = WebInspectorClient::with_connection_id(tokio::io::empty(), "owned");
    let deadline = client.deadline(Some(SHORT)).unwrap();
    tokio::time::sleep(SHORT + Duration::from_millis(5)).await;
    let error = super::client::await_operation(deadline, "owned due session", async move {
        count.fetch_add(1, Ordering::SeqCst);
        Ok::<_, WebInspectorError>(())
    })
    .await
    .unwrap_err();
    timeout_source(&error, SHORT);
    assert_eq!(polled.load(Ordering::SeqCst), 0);
    client.set_operation_timeout(None);
    assert!(client.is_usable());
}
