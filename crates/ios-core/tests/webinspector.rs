use std::time::Duration;

use ios_core::webinspector::{
    AutomationAvailability, AutomationSession, By, InspectorSession, WebInspectorClient,
    WebInspectorError, WebInspectorEvent, WirType,
};
use serde_json::json;
use tokio::io::{duplex, AsyncReadExt, AsyncWriteExt};
use tokio::time::timeout;

fn encode_plist(value: &plist::Value) -> Vec<u8> {
    let mut payload = Vec::new();
    plist::to_writer_xml(&mut payload, value).expect("plist serialization");
    let mut framed = Vec::with_capacity(payload.len() + 4);
    framed.extend_from_slice(&(payload.len() as u32).to_be_bytes());
    framed.extend_from_slice(&payload);
    framed
}

async fn read_plist_frame(stream: &mut tokio::io::DuplexStream) -> plist::Value {
    let mut len_buf = [0u8; 4];
    stream.read_exact(&mut len_buf).await.expect("frame length");
    let len = u32::from_be_bytes(len_buf) as usize;
    let mut payload = vec![0u8; len];
    stream
        .read_exact(&mut payload)
        .await
        .expect("frame payload");
    plist::from_bytes(&payload).expect("plist decode")
}

fn current_state_message_with_availability(availability: &str) -> plist::Value {
    plist::Value::Dictionary(plist::Dictionary::from_iter([
        (
            "__selector".to_string(),
            plist::Value::String("_rpc_reportCurrentState:".into()),
        ),
        (
            "__argument".to_string(),
            plist::Value::Dictionary(plist::Dictionary::from_iter([(
                "WIRAutomationAvailabilityKey".to_string(),
                plist::Value::String(availability.into()),
            )])),
        ),
    ]))
}

fn current_state_message() -> plist::Value {
    current_state_message_with_availability("WIRAutomationAvailabilityAvailable")
}

fn connected_application_list_message() -> plist::Value {
    plist::Value::Dictionary(plist::Dictionary::from_iter([
        (
            "__selector".to_string(),
            plist::Value::String("_rpc_reportConnectedApplicationList:".into()),
        ),
        (
            "__argument".to_string(),
            plist::Value::Dictionary(plist::Dictionary::from_iter([(
                "WIRApplicationDictionaryKey".to_string(),
                plist::Value::Dictionary(plist::Dictionary::from_iter([(
                    "PID:42".to_string(),
                    plist::Value::Dictionary(plist::Dictionary::from_iter([
                        (
                            "WIRApplicationIdentifierKey".to_string(),
                            plist::Value::String("PID:42".into()),
                        ),
                        (
                            "WIRApplicationBundleIdentifierKey".to_string(),
                            plist::Value::String("com.apple.mobilesafari".into()),
                        ),
                        (
                            "WIRApplicationNameKey".to_string(),
                            plist::Value::String("Safari".into()),
                        ),
                        (
                            "WIRAutomationAvailabilityKey".to_string(),
                            plist::Value::String("WIRAutomationAvailabilityAvailable".into()),
                        ),
                        (
                            "WIRIsApplicationActiveKey".to_string(),
                            plist::Value::Integer(2.into()),
                        ),
                        (
                            "WIRIsApplicationProxyKey".to_string(),
                            plist::Value::Boolean(false),
                        ),
                        (
                            "WIRIsApplicationReadyKey".to_string(),
                            plist::Value::Boolean(true),
                        ),
                    ])),
                )])),
            )])),
        ),
    ]))
}

fn listing_message() -> plist::Value {
    plist::Value::Dictionary(plist::Dictionary::from_iter([
        (
            "__selector".to_string(),
            plist::Value::String("_rpc_applicationSentListing:".into()),
        ),
        (
            "__argument".to_string(),
            plist::Value::Dictionary(plist::Dictionary::from_iter([
                (
                    "WIRApplicationIdentifierKey".to_string(),
                    plist::Value::String("PID:42".into()),
                ),
                (
                    "WIRListingKey".to_string(),
                    plist::Value::Dictionary(plist::Dictionary::from_iter([(
                        "page-1".to_string(),
                        plist::Value::Dictionary(plist::Dictionary::from_iter([
                            (
                                "WIRPageIdentifierKey".to_string(),
                                plist::Value::Integer(1.into()),
                            ),
                            (
                                "WIRTypeKey".to_string(),
                                plist::Value::String("WIRTypeWebPage".into()),
                            ),
                            (
                                "WIRTitleKey".to_string(),
                                plist::Value::String("Example".into()),
                            ),
                            (
                                "WIRURLKey".to_string(),
                                plist::Value::String("https://example.com".into()),
                            ),
                        ])),
                    )])),
                ),
            ])),
        ),
    ]))
}

fn automation_listing_message(session_id: &str, connection_id: Option<&str>) -> plist::Value {
    let mut page = plist::Dictionary::from_iter([
        (
            "WIRPageIdentifierKey".to_string(),
            plist::Value::Integer(2.into()),
        ),
        (
            "WIRTypeKey".to_string(),
            plist::Value::String("WIRTypeAutomation".into()),
        ),
        (
            "WIRAutomationTargetIsPairedKey".to_string(),
            plist::Value::Boolean(true),
        ),
        (
            "WIRAutomationTargetNameKey".to_string(),
            plist::Value::String("Safari Automation".into()),
        ),
        (
            "WIRAutomationTargetVersionKey".to_string(),
            plist::Value::String("1".into()),
        ),
        (
            "WIRSessionIdentifierKey".to_string(),
            plist::Value::String(session_id.to_string()),
        ),
    ]);
    if let Some(connection_id) = connection_id {
        page.insert(
            "WIRConnectionIdentifierKey".to_string(),
            plist::Value::String(connection_id.to_string()),
        );
    }

    plist::Value::Dictionary(plist::Dictionary::from_iter([
        (
            "__selector".to_string(),
            plist::Value::String("_rpc_applicationSentListing:".into()),
        ),
        (
            "__argument".to_string(),
            plist::Value::Dictionary(plist::Dictionary::from_iter([
                (
                    "WIRApplicationIdentifierKey".to_string(),
                    plist::Value::String("PID:42".into()),
                ),
                (
                    "WIRListingKey".to_string(),
                    plist::Value::Dictionary(plist::Dictionary::from_iter([(
                        "automation-2".to_string(),
                        plist::Value::Dictionary(page),
                    )])),
                ),
            ])),
        ),
    ]))
}

fn target_created_message() -> plist::Value {
    plist::Value::Dictionary(plist::Dictionary::from_iter([
        (
            "__selector".to_string(),
            plist::Value::String("_rpc_applicationSentData:".into()),
        ),
        (
            "__argument".to_string(),
            plist::Value::Dictionary(plist::Dictionary::from_iter([(
                "WIRMessageDataKey".to_string(),
                plist::Value::Data(
                    serde_json::to_vec(&json!({
                        "method": "Target.targetCreated",
                        "params": {
                            "targetInfo": {
                                "targetId": "target-1"
                            }
                        }
                    }))
                    .unwrap(),
                ),
            )])),
        ),
    ]))
}

fn dispatched_response_message() -> plist::Value {
    plist::Value::Dictionary(plist::Dictionary::from_iter([
        (
            "__selector".to_string(),
            plist::Value::String("_rpc_applicationSentData:".into()),
        ),
        (
            "__argument".to_string(),
            plist::Value::Dictionary(plist::Dictionary::from_iter([(
                "WIRMessageDataKey".to_string(),
                plist::Value::Data(
                    serde_json::to_vec(&json!({
                        "method": "Target.dispatchMessageFromTarget",
                        "params": {
                            "targetId": "target-1",
                            "message": "{\"id\":1,\"result\":{\"enabled\":true}}"
                        }
                    }))
                    .unwrap(),
                ),
            )])),
        ),
    ]))
}

fn dispatched_event_message() -> plist::Value {
    plist::Value::Dictionary(plist::Dictionary::from_iter([
        (
            "__selector".to_string(),
            plist::Value::String("_rpc_applicationSentData:".into()),
        ),
        (
            "__argument".to_string(),
            plist::Value::Dictionary(plist::Dictionary::from_iter([(
                "WIRMessageDataKey".to_string(),
                plist::Value::Data(
                    serde_json::to_vec(&json!({
                        "method": "Target.dispatchMessageFromTarget",
                        "params": {
                            "targetId": "target-1",
                            "message": "{\"method\":\"Console.messageAdded\",\"params\":{\"message\":{\"level\":\"log\",\"text\":\"queued\"}}}"
                        }
                    }))
                    .unwrap(),
                ),
            )])),
        ),
    ]))
}

fn target_transport_ack_message() -> plist::Value {
    plist::Value::Dictionary(plist::Dictionary::from_iter([
        (
            "__selector".to_string(),
            plist::Value::String("_rpc_applicationSentData:".into()),
        ),
        (
            "__argument".to_string(),
            plist::Value::Dictionary(plist::Dictionary::from_iter([(
                "WIRMessageDataKey".to_string(),
                plist::Value::Data(
                    serde_json::to_vec(&json!({
                        "id": 1,
                        "result": {}
                    }))
                    .unwrap(),
                ),
            )])),
        ),
    ]))
}

fn automation_command_response_message(id: u64, result: serde_json::Value) -> plist::Value {
    plist::Value::Dictionary(plist::Dictionary::from_iter([
        (
            "__selector".to_string(),
            plist::Value::String("_rpc_applicationSentData:".into()),
        ),
        (
            "__argument".to_string(),
            plist::Value::Dictionary(plist::Dictionary::from_iter([(
                "WIRMessageDataKey".to_string(),
                plist::Value::Data(
                    serde_json::to_vec(&json!({
                        "id": id,
                        "result": result
                    }))
                    .unwrap(),
                ),
            )])),
        ),
    ]))
}

#[tokio::test]
async fn start_and_open_application_pages_updates_state_and_requests_listings() {
    let (client_stream, mut server_stream) = duplex(16 * 1024);
    let task = tokio::spawn(async move {
        let mut client = WebInspectorClient::with_connection_id(client_stream, "TEST-CONNECTION");
        client.start(Duration::from_millis(100)).await.unwrap();
        let pages = client
            .open_application_pages(Duration::from_millis(100))
            .await
            .unwrap();
        (client, pages)
    });

    let report_identifier = read_plist_frame(&mut server_stream).await;
    let report_identifier = report_identifier
        .as_dictionary()
        .expect("reportIdentifier dict");
    assert_eq!(
        report_identifier
            .get("__selector")
            .and_then(plist::Value::as_string),
        Some("_rpc_reportIdentifier:")
    );
    assert_eq!(
        report_identifier
            .get("__argument")
            .and_then(plist::Value::as_dictionary)
            .and_then(|value| value.get("WIRConnectionIdentifierKey"))
            .and_then(plist::Value::as_string),
        Some("TEST-CONNECTION")
    );
    server_stream
        .write_all(&encode_plist(&current_state_message()))
        .await
        .unwrap();

    let get_connected = read_plist_frame(&mut server_stream).await;
    let get_connected = get_connected.as_dictionary().expect("getConnected dict");
    assert_eq!(
        get_connected
            .get("__selector")
            .and_then(plist::Value::as_string),
        Some("_rpc_getConnectedApplications:")
    );
    server_stream
        .write_all(&encode_plist(&connected_application_list_message()))
        .await
        .unwrap();

    let forward_listing = read_plist_frame(&mut server_stream).await;
    let forward_listing = forward_listing
        .as_dictionary()
        .expect("forwardGetListing dict");
    assert_eq!(
        forward_listing
            .get("__selector")
            .and_then(plist::Value::as_string),
        Some("_rpc_forwardGetListing:")
    );
    assert_eq!(
        forward_listing
            .get("__argument")
            .and_then(plist::Value::as_dictionary)
            .and_then(|value| value.get("WIRApplicationIdentifierKey"))
            .and_then(plist::Value::as_string),
        Some("PID:42")
    );
    server_stream
        .write_all(&encode_plist(&listing_message()))
        .await
        .unwrap();

    let (client, pages) = task.await.unwrap();
    assert_eq!(
        client.automation_availability(),
        Some(AutomationAvailability::Available)
    );
    assert_eq!(pages.len(), 1);
    assert_eq!(
        pages[0].application.bundle_identifier,
        "com.apple.mobilesafari"
    );
    assert_eq!(pages[0].page.id, 1);
    assert_eq!(pages[0].page.page_type, WirType::WebPage);
    assert_eq!(pages[0].page.title.as_deref(), Some("Example"));
}

#[tokio::test]
async fn inspector_session_wraps_target_commands_and_unwraps_nested_responses() {
    let (client_stream, mut server_stream) = duplex(16 * 1024);
    let task = tokio::spawn(async move {
        let mut client = WebInspectorClient::with_connection_id(client_stream, "TEST-CONNECTION");
        let mut session = InspectorSession::with_session_id("PID:42", 1, "TEST-SESSION");
        session
            .attach(&mut client, true, Duration::from_millis(100))
            .await
            .unwrap();
        session
            .send_command_and_wait(
                &mut client,
                "Runtime.enable",
                serde_json::Value::Object(Default::default()),
                Duration::from_millis(100),
            )
            .await
            .unwrap()
    });

    let socket_setup = read_plist_frame(&mut server_stream).await;
    let socket_setup = socket_setup.as_dictionary().expect("socket setup dict");
    assert_eq!(
        socket_setup
            .get("__selector")
            .and_then(plist::Value::as_string),
        Some("_rpc_forwardSocketSetup:")
    );
    let socket_setup_arg = socket_setup
        .get("__argument")
        .and_then(plist::Value::as_dictionary)
        .expect("socket setup arg");
    assert_eq!(
        socket_setup_arg
            .get("WIRSenderKey")
            .and_then(plist::Value::as_string),
        Some("TEST-SESSION")
    );
    server_stream
        .write_all(&encode_plist(&target_created_message()))
        .await
        .unwrap();

    let socket_data = read_plist_frame(&mut server_stream).await;
    let socket_data = socket_data.as_dictionary().expect("socket data dict");
    assert_eq!(
        socket_data
            .get("__selector")
            .and_then(plist::Value::as_string),
        Some("_rpc_forwardSocketData:")
    );
    let socket_data_arg = socket_data
        .get("__argument")
        .and_then(plist::Value::as_dictionary)
        .expect("socket data arg");
    let payload = socket_data_arg
        .get("WIRSocketDataKey")
        .and_then(plist::Value::as_data)
        .expect("socket payload");
    let payload: serde_json::Value = serde_json::from_slice(payload).expect("socket json");
    assert_eq!(payload["method"], "Target.sendMessageToTarget");
    assert_eq!(payload["params"]["targetId"], "target-1");
    let nested = payload["params"]["message"]
        .as_str()
        .expect("nested message");
    let nested: serde_json::Value = serde_json::from_str(nested).expect("nested json");
    assert_eq!(nested["id"], 1);
    assert_eq!(nested["method"], "Runtime.enable");

    server_stream
        .write_all(&encode_plist(&target_transport_ack_message()))
        .await
        .unwrap();

    server_stream
        .write_all(&encode_plist(&dispatched_response_message()))
        .await
        .unwrap();

    let response = task.await.unwrap();
    assert_eq!(response["result"]["enabled"], true);
}

#[tokio::test]
async fn inspector_session_wait_for_response_preserves_unmatched_target_events() {
    let (client_stream, mut server_stream) = duplex(16 * 1024);
    let task = tokio::spawn(async move {
        let mut client = WebInspectorClient::with_connection_id(client_stream, "TEST-CONNECTION");
        let mut session = InspectorSession::with_session_id("PID:42", 1, "TEST-SESSION");
        session
            .attach(&mut client, true, Duration::from_millis(100))
            .await
            .unwrap();
        let response = session
            .send_command_and_wait(
                &mut client,
                "Runtime.enable",
                serde_json::Value::Object(Default::default()),
                Duration::from_millis(100),
            )
            .await
            .unwrap();
        let queued = session
            .next_raw_message(&mut client, Duration::from_millis(100))
            .await
            .unwrap();
        (response, queued)
    });

    let socket_setup = read_plist_frame(&mut server_stream).await;
    let socket_setup = socket_setup.as_dictionary().expect("socket setup dict");
    assert_eq!(
        socket_setup
            .get("__selector")
            .and_then(plist::Value::as_string),
        Some("_rpc_forwardSocketSetup:")
    );
    server_stream
        .write_all(&encode_plist(&target_created_message()))
        .await
        .unwrap();

    let socket_data = read_plist_frame(&mut server_stream).await;
    let socket_data = socket_data.as_dictionary().expect("socket data dict");
    assert_eq!(
        socket_data
            .get("__selector")
            .and_then(plist::Value::as_string),
        Some("_rpc_forwardSocketData:")
    );

    server_stream
        .write_all(&encode_plist(&target_transport_ack_message()))
        .await
        .unwrap();
    server_stream
        .write_all(&encode_plist(&dispatched_event_message()))
        .await
        .unwrap();
    server_stream
        .write_all(&encode_plist(&dispatched_response_message()))
        .await
        .unwrap();

    let (response, queued) = task.await.unwrap();
    assert_eq!(response["result"]["enabled"], true);
    assert_eq!(queued["method"], "Target.dispatchMessageFromTarget");
    assert_eq!(
        queued["params"]["message"],
        "{\"method\":\"Console.messageAdded\",\"params\":{\"message\":{\"level\":\"log\",\"text\":\"queued\"}}}"
    );
}

#[tokio::test]
async fn next_event_decodes_application_sent_data_messages() {
    let (client_stream, mut server_stream) = duplex(16 * 1024);
    let task = tokio::spawn(async move {
        let mut client = WebInspectorClient::new(client_stream);
        client.next_event().await.unwrap()
    });

    server_stream
        .write_all(&encode_plist(&target_created_message()))
        .await
        .unwrap();

    match task.await.unwrap() {
        WebInspectorEvent::SocketData { message, .. } => {
            assert_eq!(message["method"], "Target.targetCreated");
        }
        other => panic!("unexpected event: {other:?}"),
    }
}

#[tokio::test]
async fn automation_session_attach_requests_session_and_waits_for_connection_identifier() {
    let (client_stream, mut server_stream) = duplex(16 * 1024);
    let task = tokio::spawn(async move {
        let mut client = WebInspectorClient::with_connection_id(client_stream, "TEST-CONNECTION");
        let mut session =
            AutomationSession::with_session_id("PID:42", "com.apple.mobilesafari", "TEST-SESSION");
        session
            .attach(&mut client, Duration::from_millis(100))
            .await
            .unwrap();
        session.page_id()
    });

    let automation_request = read_plist_frame(&mut server_stream).await;
    let automation_request = automation_request
        .as_dictionary()
        .expect("automation request dict");
    assert_eq!(
        automation_request
            .get("__selector")
            .and_then(plist::Value::as_string),
        Some("_rpc_forwardAutomationSessionRequest:")
    );
    let automation_request_arg = automation_request
        .get("__argument")
        .and_then(plist::Value::as_dictionary)
        .expect("automation request arg");
    assert_eq!(
        automation_request_arg
            .get("WIRApplicationIdentifierKey")
            .and_then(plist::Value::as_string),
        Some("PID:42")
    );

    let listing_request = read_plist_frame(&mut server_stream).await;
    let listing_request = listing_request
        .as_dictionary()
        .expect("listing request dict");
    assert_eq!(
        listing_request
            .get("__selector")
            .and_then(plist::Value::as_string),
        Some("_rpc_forwardGetListing:")
    );

    server_stream
        .write_all(&encode_plist(&automation_listing_message(
            "TEST-SESSION",
            None,
        )))
        .await
        .unwrap();

    let socket_setup = read_plist_frame(&mut server_stream).await;
    let socket_setup = socket_setup.as_dictionary().expect("socket setup dict");
    assert_eq!(
        socket_setup
            .get("__selector")
            .and_then(plist::Value::as_string),
        Some("_rpc_forwardSocketSetup:")
    );

    let second_listing_request = read_plist_frame(&mut server_stream).await;
    let second_listing_request = second_listing_request
        .as_dictionary()
        .expect("second listing request dict");
    assert_eq!(
        second_listing_request
            .get("__selector")
            .and_then(plist::Value::as_string),
        Some("_rpc_forwardGetListing:")
    );

    server_stream
        .write_all(&encode_plist(&automation_listing_message(
            "TEST-SESSION",
            Some("AUTOMATION-CONNECTION"),
        )))
        .await
        .unwrap();

    let page_id = task.await.unwrap();
    assert_eq!(page_id, 2);
}

#[tokio::test]
async fn automation_session_attach_fails_fast_when_remote_automation_is_unavailable() {
    let (client_stream, mut server_stream) = duplex(16 * 1024);
    let server = tokio::spawn(async move {
        let report_identifier = read_plist_frame(&mut server_stream).await;
        let report_identifier = report_identifier
            .as_dictionary()
            .expect("report identifier dict");
        assert_eq!(
            report_identifier
                .get("__selector")
                .and_then(plist::Value::as_string),
            Some("_rpc_reportIdentifier:")
        );

        server_stream
            .write_all(&encode_plist(&current_state_message_with_availability(
                "WIRAutomationAvailabilityNotAvailable",
            )))
            .await
            .unwrap();

        timeout(
            Duration::from_millis(100),
            read_plist_frame(&mut server_stream),
        )
        .await
    });

    let mut client = WebInspectorClient::with_connection_id(client_stream, "TEST-CONNECTION");
    client.start(Duration::from_millis(100)).await.unwrap();

    let mut session =
        AutomationSession::with_session_id("PID:42", "com.apple.mobilesafari", "TEST-SESSION");
    let err = session
        .attach(&mut client, Duration::from_millis(100))
        .await
        .expect_err("attach must fail before requesting automation when unavailable");
    assert!(matches!(
        err,
        WebInspectorError::Protocol(message) if message == "remote automation is not available"
    ));

    assert!(
        server.await.unwrap().is_err(),
        "attach should not send automation traffic after learning remote automation is unavailable"
    );
}

#[tokio::test]
async fn automation_session_find_elements_rewrites_class_name_to_css_selector() {
    let (client_stream, mut server_stream) = duplex(16 * 1024);
    let task = tokio::spawn(async move {
        let mut client = WebInspectorClient::with_connection_id(client_stream, "TEST-CONNECTION");
        let mut session =
            AutomationSession::with_page("PID:42", "com.apple.mobilesafari", "TEST-SESSION", 2);
        session
            .find_elements(&mut client, By::ClassName, "link-class", false)
            .await
            .unwrap()
    });

    let socket_data = read_plist_frame(&mut server_stream).await;
    let socket_data = socket_data.as_dictionary().expect("socket data dict");
    assert_eq!(
        socket_data
            .get("__selector")
            .and_then(plist::Value::as_string),
        Some("_rpc_forwardSocketData:")
    );
    let socket_data_arg = socket_data
        .get("__argument")
        .and_then(plist::Value::as_dictionary)
        .expect("socket data arg");
    let payload = socket_data_arg
        .get("WIRSocketDataKey")
        .and_then(plist::Value::as_data)
        .expect("socket payload");
    let payload: serde_json::Value = serde_json::from_slice(payload).expect("socket json");
    assert_eq!(payload["method"], "Automation.evaluateJavaScriptFunction");
    assert_eq!(payload["params"]["arguments"][0], "\"css selector\"");
    assert_eq!(payload["params"]["arguments"][2], "\".link-class\"");
    assert_eq!(payload["params"]["expectsImplicitCallbackArgument"], true);

    server_stream
        .write_all(&encode_plist(&automation_command_response_message(
            1,
            json!({
                "result": "[\"node-1\",\"node-2\"]"
            }),
        )))
        .await
        .unwrap();

    let nodes = task.await.unwrap();
    assert_eq!(nodes, vec![json!("node-1"), json!("node-2")]);
}
