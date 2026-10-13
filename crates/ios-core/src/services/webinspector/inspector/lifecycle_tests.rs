use super::super::lifecycle_session_tests::{bounded, payload, request};
use super::*;
use tokio::io::{duplex, AsyncReadExt};

async fn native(peer: &mut tokio::io::DuplexStream, selector: &str, argument: plist::Dictionary) {
    let value = plist::Value::Dictionary(plist::Dictionary::from_iter([
        ("__selector", plist::Value::String(selector.into())),
        ("__argument", plist::Value::Dictionary(argument)),
    ]));
    bounded(super::super::wire::send_plist(peer, &value))
        .await
        .unwrap();
}

fn listing(application: &str, page: Option<u64>) -> plist::Dictionary {
    let pages = page
        .map(|id| {
            plist::Dictionary::from_iter([(
                "renamed-key",
                plist::Value::Dictionary(plist::Dictionary::from_iter([
                    (
                        "WIRTypeKey",
                        plist::Value::String("WIRTypeJavaScript".into()),
                    ),
                    ("WIRPageIdentifierKey", plist::Value::Integer(id.into())),
                ])),
            )])
        })
        .unwrap_or_default();
    plist::Dictionary::from_iter([
        (
            "WIRApplicationIdentifierKey",
            plist::Value::String(application.into()),
        ),
        ("WIRListingKey", plist::Value::Dictionary(pages)),
    ])
}

#[tokio::test]
async fn native_inspector_cancel_at_retained_event_boundary_restores_original_event_and_reuses_stream(
) {
    use std::future::Future;
    let (stream, mut peer) = duplex(65536);
    let mut client = WebInspectorClient::new(stream);
    let mut session = InspectorSession::new("PID:42", 7);
    native(
        &mut peer,
        "_rpc_applicationDisconnected:",
        plist::Dictionary::from_iter([(
            "WIRApplicationIdentifierKey",
            plist::Value::String("PID:99".into()),
        )]),
    )
    .await;
    let mut pending = Box::pin(session.next_session_event(&mut client, Duration::from_secs(1)));
    std::future::poll_fn(|cx| {
        assert!(pending.as_mut().poll(cx).is_pending());
        std::task::Poll::Ready(())
    })
    .await;
    drop(pending);
    assert!(
        client.is_usable(),
        "complete retained event has no pending wire obligation"
    );
    assert!(
        matches!(client.next_event().await.unwrap(), WebInspectorEvent::ApplicationDisconnected { application_id } if application_id == "PID:99")
    );
    native(
        &mut peer,
        "_rpc_applicationSentListing:",
        listing("PID:42", None),
    )
    .await;
    assert!(matches!(
        session
            .next_session_event(&mut client, Duration::from_secs(1))
            .await
            .unwrap(),
        InspectorSessionEvent::Closed
    ));
    assert!(client.is_usable());
}

#[tokio::test]
async fn native_inspector_closure_matches_application_and_numeric_page_retaining_unrelated_events()
{
    let (stream, mut peer) = duplex(65536);
    let mut client = WebInspectorClient::new(stream);
    let mut session = InspectorSession::new("PID:42", 7);
    native(
        &mut peer,
        "_rpc_applicationSentListing:",
        listing("PID:99", None),
    )
    .await;
    native(
        &mut peer,
        "_rpc_applicationDisconnected:",
        plist::Dictionary::from_iter([(
            "WIRApplicationIdentifierKey",
            plist::Value::String("PID:99".into()),
        )]),
    )
    .await;
    native(
        &mut peer,
        "_rpc_applicationSentListing:",
        listing("PID:42", Some(7)),
    )
    .await;
    native(
        &mut peer,
        "_rpc_applicationSentData:",
        plist::Dictionary::from_iter([
            (
                "WIRApplicationIdentifierKey",
                plist::Value::String("PID:99".into()),
            ),
            (
                "WIRMessageDataKey",
                plist::Value::Data(serde_json::to_vec(&json!({"id": 99, "result": {}})).unwrap()),
            ),
        ]),
    )
    .await;
    native(
        &mut peer,
        "_rpc_applicationSentData:",
        plist::Dictionary::from_iter([
            (
                "WIRApplicationIdentifierKey",
                plist::Value::String("PID:42".into()),
            ),
            (
                "WIRMessageDataKey",
                plist::Value::Data(serde_json::to_vec(&json!({"id": 1, "result": {}})).unwrap()),
            ),
        ]),
    )
    .await;
    assert_eq!(
        bounded(session.next_session_event(&mut client, Duration::from_secs(1)))
            .await
            .unwrap(),
        InspectorSessionEvent::Message(json!({"id": 1, "result": {}}))
    );
    assert!(
        matches!(client.next_event().await.unwrap(), WebInspectorEvent::Listing { application_id, .. } if application_id == "PID:99")
    );
    assert!(
        matches!(client.next_event().await.unwrap(), WebInspectorEvent::ApplicationDisconnected { application_id } if application_id == "PID:99")
    );
    assert!(
        matches!(client.next_event().await.unwrap(), WebInspectorEvent::Listing { application_id, pages } if application_id == "PID:42" && pages[0].id == 7)
    );
    assert!(
        matches!(client.next_event().await.unwrap(), WebInspectorEvent::SocketData { application_id, message } if application_id.as_deref() == Some("PID:99") && message["id"] == 99)
    );
    // A complete listing can retain siblings and still positively close page 7.
    native(
        &mut peer,
        "_rpc_applicationSentListing:",
        listing("PID:42", Some(8)),
    )
    .await;
    assert_eq!(
        session
            .next_session_event(&mut client, Duration::from_secs(1))
            .await
            .unwrap(),
        InspectorSessionEvent::Closed
    );
    assert!(client.is_usable());
    assert_eq!(client.page("PID:42", 8).unwrap().id, 8);
}

#[tokio::test]
async fn native_inspector_application_closure_and_raw_api_are_distinct_from_eof_or_incomplete_listing(
) {
    for raw in [false, true] {
        let (stream, mut peer) = duplex(16384);
        let mut client = WebInspectorClient::new(stream);
        let mut session = InspectorSession::new("PID:42", 7);
        native(
            &mut peer,
            "_rpc_applicationDisconnected:",
            plist::Dictionary::from_iter([(
                "WIRApplicationIdentifierKey",
                plist::Value::String("PID:42".into()),
            )]),
        )
        .await;
        if raw {
            assert!(matches!(
                session
                    .next_raw_message(&mut client, Duration::from_secs(1))
                    .await,
                Err(WebInspectorError::Protocol(_))
            ));
        } else {
            assert_eq!(
                session
                    .next_session_event(&mut client, Duration::from_secs(1))
                    .await
                    .unwrap(),
                InspectorSessionEvent::Closed
            );
        }
        assert!(client.is_usable());
    }
    let (stream, mut peer) = duplex(16384);
    let mut client = WebInspectorClient::new(stream);
    let mut session = InspectorSession::new("PID:42", 7);
    native(
        &mut peer,
        "_rpc_applicationSentListing:",
        plist::Dictionary::from_iter([(
            "WIRApplicationIdentifierKey",
            plist::Value::String("PID:42".into()),
        )]),
    )
    .await;
    assert!(matches!(
        session
            .next_session_event(&mut client, Duration::from_secs(1))
            .await,
        Err(WebInspectorError::Protocol(_))
    ));
    assert!(!client.is_usable());
    let (stream, peer) = duplex(16384);
    let mut client = WebInspectorClient::new(stream);
    drop(peer);
    let error = session
        .next_session_event(&mut client, Duration::from_secs(1))
        .await
        .unwrap_err();
    assert!(
        matches!(error, WebInspectorError::Io(ref source) if source.kind() == std::io::ErrorKind::UnexpectedEof)
    );
    assert!(!client.is_usable());
}

#[tokio::test]
async fn native_inspector_unrelated_idle_preserves_events_but_partial_read_keeps_original_failure()
{
    use tokio::io::AsyncWriteExt;
    let (stream, mut peer) = duplex(16384);
    let mut client = WebInspectorClient::new(stream);
    let mut session = InspectorSession::new("PID:42", 7);
    native(
        &mut peer,
        "_rpc_applicationSentListing:",
        listing("PID:42", Some(7)),
    )
    .await;
    let error = session
        .next_session_event(&mut client, Duration::from_millis(10))
        .await
        .unwrap_err();
    assert!(error.timeout_duration().is_some());
    assert!(client.is_usable());
    assert!(
        matches!(client.next_event().await.unwrap(), WebInspectorEvent::Listing { pages, .. } if pages[0].id == 7)
    );
    // EOF after an accepted frame prefix must not become positive native close.
    peer.write_all(&[0, 0]).await.unwrap();
    drop(peer);
    let error = session
        .next_session_event(&mut client, Duration::from_secs(1))
        .await
        .unwrap_err();
    assert!(
        matches!(error, WebInspectorError::Io(ref source) if source.kind() == std::io::ErrorKind::UnexpectedEof)
    );
    assert!(!client.is_usable());
}

#[tokio::test]
async fn native_inspector_unrelated_event_flood_hits_retention_limit_without_closure() {
    let (stream, mut peer) = duplex(65536);
    let mut client = WebInspectorClient::new(stream);
    let mut session = InspectorSession::new("PID:42", 7);
    let writer = tokio::spawn(async move {
        for _ in 0..=super::super::retained::MAX_EVENTS {
            native(
                &mut peer,
                "_rpc_applicationSentListing:",
                listing("PID:99", None),
            )
            .await;
        }
    });
    let error = bounded(session.next_session_event(&mut client, Duration::from_secs(2)))
        .await
        .unwrap_err();
    assert_eq!(
        crate::classify_error(&error),
        crate::ErrorClass::ResourceLimit
    );
    assert!(!client.is_usable());
    drop(client);
    bounded(writer).await.unwrap();
}

#[tokio::test]
async fn lifecycle_webinspector_session_inspector_max_ids_are_sent_once_and_never_wrap() {
    for targeted in [false, true] {
        let (stream, mut peer) = duplex(16384);
        let mut client = WebInspectorClient::with_connection_id(stream, "owned");
        let mut session = InspectorSession::with_session_id("PID:42", 7, "owned-session");
        session.next_command_id = Some(u64::MAX);
        session.next_transport_id = Some(u64::MAX);
        if targeted {
            session.target_id = Some("owned-target".into());
        }
        assert_eq!(
            bounded(session.send_command(&mut client, "Runtime.enable", json!({})))
                .await
                .unwrap(),
            u64::MAX
        );
        let sent = payload(&request(&mut peer).await);
        assert_eq!(sent["id"], u64::MAX);
        if targeted {
            assert_eq!(sent["method"], "Target.sendMessageToTarget");
            let nested: JsonValue =
                serde_json::from_str(sent["params"]["message"].as_str().unwrap()).unwrap();
            assert_eq!(nested["id"], u64::MAX);
            let error = session
                .send_bridge_message(&mut client, &json!({"id": 1}))
                .await
                .unwrap_err();
            assert_eq!(
                crate::classify_error(&error),
                crate::ErrorClass::ResourceLimit
            );
        }
        let error = session
            .send_command(&mut client, "Runtime.enable", json!({}))
            .await
            .unwrap_err();
        assert_eq!(
            crate::classify_error(&error),
            crate::ErrorClass::ResourceLimit
        );
        assert!(client.is_usable());
        drop(client);
        let mut extra = Vec::new();
        bounded(peer.read_to_end(&mut extra)).await.unwrap();
        assert!(extra.is_empty());
    }
}

#[tokio::test]
async fn lifecycle_webinspector_session_transport_exhaustion_does_not_consume_command_id() {
    let (stream, mut peer) = duplex(16384);
    let mut client = WebInspectorClient::with_connection_id(stream, "owned");
    let mut session = InspectorSession::with_session_id("PID:42", 7, "owned-session");
    session.target_id = Some("owned-target".into());
    session.next_transport_id = None;
    let error = session
        .send_command(&mut client, "Runtime.enable", json!({}))
        .await
        .unwrap_err();
    assert_eq!(
        crate::classify_error(&error),
        crate::ErrorClass::ResourceLimit
    );
    assert_eq!(session.next_command_id, Some(1));
    session.target_id = None;
    assert_eq!(
        session
            .send_command(&mut client, "Runtime.enable", json!({}))
            .await
            .unwrap(),
        1
    );
    assert_eq!(payload(&request(&mut peer).await)["id"], 1);
    assert!(client.is_usable());
}
