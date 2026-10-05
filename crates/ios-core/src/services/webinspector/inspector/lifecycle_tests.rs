use super::super::lifecycle_session_tests::{bounded, payload, request};
use super::*;
use tokio::io::{duplex, AsyncReadExt};

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
