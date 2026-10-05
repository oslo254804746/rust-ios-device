use super::super::lifecycle_session_tests::{bounded, request};
use super::*;
use serde_json::json;
use tokio::io::{duplex, AsyncWriteExt};

#[tokio::test]
async fn lifecycle_webinspector_session_event_cap_inherits_into_actual_automatic_listing_write() {
    let application = plist::Dictionary::from_iter([
        (
            "WIRApplicationIdentifierKey".to_owned(),
            plist::Value::String("PID:42".into()),
        ),
        (
            "WIRApplicationBundleIdentifierKey".to_owned(),
            plist::Value::String("owned-bundle".into()),
        ),
        (
            "WIRApplicationNameKey".to_owned(),
            plist::Value::String("owned".into()),
        ),
        (
            "WIRAutomationAvailabilityKey".to_owned(),
            plist::Value::String("WIRAutomationAvailabilityAvailable".into()),
        ),
        (
            "WIRIsApplicationActiveKey".to_owned(),
            plist::Value::Boolean(true),
        ),
        (
            "WIRIsApplicationProxyKey".to_owned(),
            plist::Value::Boolean(false),
        ),
        (
            "WIRIsApplicationReadyKey".to_owned(),
            plist::Value::Boolean(true),
        ),
    ]);
    let value = plist::Value::Dictionary(plist::Dictionary::from_iter([
        (
            "__selector".to_owned(),
            plist::Value::String("_rpc_reportConnectedApplicationList:".into()),
        ),
        (
            "__argument".to_owned(),
            plist::Value::Dictionary(plist::Dictionary::from_iter([(
                "WIRApplicationDictionaryKey".to_owned(),
                plist::Value::Dictionary(plist::Dictionary::from_iter([(
                    "PID:42".to_owned(),
                    plist::Value::Dictionary(application),
                )])),
            )])),
        ),
    ]));
    let (stream, mut peer) = duplex(16384);
    let mut client = WebInspectorClient::with_connection_id(stream, "owned");
    client.set_operation_timeout(Some(Duration::MAX));
    let mut body = Vec::new();
    plist::to_writer_xml(&mut body, &value).unwrap();
    peer.write_u32(body.len() as u32).await.unwrap();
    peer.write_all(&body).await.unwrap();
    let event = bounded(client.next_event_with_timeout(Duration::from_secs(1)))
        .await
        .unwrap();
    assert!(
        matches!(event, WebInspectorEvent::ConnectedApplications { ref applications } if applications.len() == 1 && applications[0].id == "PID:42")
    );
    let listing = request(&mut peer).await;
    assert_eq!(
        listing["__selector"].as_string(),
        Some("_rpc_forwardGetListing:")
    );
    assert_eq!(
        listing["__argument"].as_dictionary().unwrap()["WIRApplicationIdentifierKey"].as_string(),
        Some("PID:42")
    );
    assert!(client.is_usable());
    assert!(client.applications().contains_key("PID:42"));
    assert_eq!(
        crate::classify_error(&client.report_identifier().await.unwrap_err()),
        crate::ErrorClass::InvalidInput
    );
}

fn event(text: &str) -> WebInspectorEvent {
    WebInspectorEvent::SocketData {
        application_id: None,
        message: JsonValue::String(text.into()),
    }
}

#[tokio::test]
async fn lifecycle_webinspector_session_pending_and_skipped_share_count_limit() {
    let mut client = WebInspectorClient::with_connection_id(tokio::io::empty(), "owned");
    client
        .restore_events(
            std::iter::repeat_with(|| event("pending"))
                .take(MAX_EVENTS - 1)
                .collect(),
        )
        .unwrap();
    let mut operation = WebInspectorOperation::new(&mut client).unwrap();
    operation.commit();
    operation.preserve(event("last")).unwrap();
    let error = operation.preserve(event("over-count")).unwrap_err();
    assert_eq!(
        crate::classify_error(&error),
        crate::ErrorClass::ResourceLimit
    );
    assert!(operation.finish::<()>(Err(error)).is_err());
    assert!(!client.is_usable());
    assert_eq!(client.pending_event_bytes, 0);
    assert_eq!(client.pending_events.capacity(), 0);
}

#[tokio::test]
async fn lifecycle_webinspector_session_exact_byte_limit_releases_on_cached_pop() {
    let overhead = serde_json::to_vec(&event("")).unwrap().len();
    let exactly = event(&"a".repeat(MAX_EVENT_BYTES - overhead));
    let mut client = WebInspectorClient::with_connection_id(tokio::io::empty(), "owned");
    let mut operation = WebInspectorOperation::new(&mut client).unwrap();
    operation.preserve(exactly).unwrap();
    let error = operation
        .preserve(WebInspectorEvent::ConnectedDrivers)
        .unwrap_err();
    assert_eq!(
        crate::classify_error(&error),
        crate::ErrorClass::ResourceLimit
    );
    // Rejection occurs before retaining the extra event. Finish the accepted
    // local owner to test accounting release; this owner sent no wire command.
    operation.finish(Ok(())).unwrap();
    assert_eq!(client.pending_event_bytes, MAX_EVENT_BYTES);
    assert_eq!(client.pending_events.len(), 1);
    assert!(
        matches!(client.next_event().await.unwrap(), WebInspectorEvent::SocketData { message: JsonValue::String(ref text), .. } if text.len() == MAX_EVENT_BYTES - overhead)
    );
    assert_eq!(client.pending_event_bytes, 0);
    assert!(client.is_usable());
    client
        .restore_events(VecDeque::from([event("after-release")]))
        .unwrap();
    assert!(client.pending_event_bytes < 1024);
}

#[test]
fn lifecycle_webinspector_session_logical_bytes_include_json_escaping_and_existing_pending() {
    let escaped = event("\0\n\"\\你好");
    let encoded = serde_json::to_vec(&escaped).unwrap();
    assert_eq!(event_size(&escaped, encoded.len()).unwrap(), encoded.len());
    assert_eq!(
        crate::classify_error(&event_size(&escaped, encoded.len() - 1).unwrap_err()),
        crate::ErrorClass::ResourceLimit
    );
    let mut client = WebInspectorClient::with_connection_id(tokio::io::empty(), "owned");
    client
        .restore_events(VecDeque::from([escaped.clone()]))
        .unwrap();
    assert_eq!(client.pending_event_bytes, encoded.len());
    let overhead = serde_json::to_vec(&event("")).unwrap().len();
    let mut operation = WebInspectorOperation::new(&mut client).unwrap();
    operation.commit();
    operation
        .preserve(event(
            &"a".repeat(MAX_EVENT_BYTES - encoded.len() - overhead),
        ))
        .unwrap();
    let error = operation.preserve(event("too-many-bytes")).unwrap_err();
    assert!(operation.finish::<()>(Err(error)).is_err());
    assert_eq!(client.pending_event_bytes, 0);
    assert_eq!(client.pending_events.capacity(), 0);
}

#[tokio::test]
async fn lifecycle_webinspector_session_restoration_keeps_order_business_error_and_accounting() {
    let mut client = WebInspectorClient::with_connection_id(tokio::io::empty(), "owned");
    client
        .restore_events(VecDeque::from([event("third")]))
        .unwrap();
    let mut operation = WebInspectorOperation::new(&mut client).unwrap();
    operation.commit();
    operation.preserve(event("first")).unwrap();
    operation.preserve(event("second")).unwrap();
    let error = operation
        .finish_response::<()>(Err(WebInspectorError::Protocol(
            "owned business failure".into(),
        )))
        .unwrap_err();
    assert!(
        matches!(error, WebInspectorError::Protocol(ref message) if message == "owned business failure")
    );
    let expected_bytes: usize = ["first", "second", "third"]
        .iter()
        .map(|s| serde_json::to_vec(&event(s)).unwrap().len())
        .sum();
    assert_eq!(client.pending_event_bytes, expected_bytes);
    for text in ["first", "second", "third"] {
        assert_eq!(client.next_event().await.unwrap(), event(text));
    }
    assert_eq!(client.pending_event_bytes, 0);
    assert!(client.is_usable());
}

#[test]
fn lifecycle_webinspector_session_restore_rejection_discards_owned_queue_before_retention() {
    let mut client = WebInspectorClient::with_connection_id(tokio::io::empty(), "owned");
    client
        .restore_events(
            std::iter::repeat_with(|| event("pending"))
                .take(MAX_EVENTS)
                .collect(),
        )
        .unwrap();
    let error = client
        .restore_events(VecDeque::from([event("over")]))
        .unwrap_err();
    assert_eq!(
        crate::classify_error(&error),
        crate::ErrorClass::ResourceLimit
    );
    assert!(!client.is_usable());
    assert_eq!(client.pending_events.capacity(), 0);
    assert_eq!(client.pending_event_bytes, 0);
}

#[test]
fn lifecycle_webinspector_session_committed_owner_drop_releases_pending_and_skipped_state() {
    let mut client = WebInspectorClient::with_connection_id(tokio::io::empty(), "owned");
    client
        .restore_events(VecDeque::from([event("pending")]))
        .unwrap();
    {
        let mut operation = WebInspectorOperation::new(&mut client).unwrap();
        operation.commit();
        operation.preserve(event("skipped")).unwrap();
    }
    assert!(!client.is_usable());
    assert_eq!(client.pending_events.capacity(), 0);
    assert_eq!(client.pending_event_bytes, 0);
    client
        .restore_events(VecDeque::from([WebInspectorEvent::SocketData {
            application_id: None,
            message: json!({"id": 1}),
        }]))
        .unwrap();
    assert!(client.pending_events.is_empty());
}
