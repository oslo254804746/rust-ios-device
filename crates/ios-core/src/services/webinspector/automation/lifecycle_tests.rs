use super::super::lifecycle_session_tests::{bounded, payload, request, socket};
use super::*;
use tokio::io::{duplex, AsyncReadExt, AsyncWriteExt};

#[tokio::test]
async fn lifecycle_webinspector_session_automation_explicit_cap_admits_configured_overflow() {
    let (stream, mut peer) = duplex(16384);
    let mut client = WebInspectorClient::with_connection_id(stream, "owned");
    client.set_operation_timeout(Some(Duration::MAX));
    let mut session = AutomationSession::with_page("PID:42", "owned-bundle", "owned-session", 7);
    let caller = session.send_command_and_wait(
        &mut client,
        "getBrowsingContext",
        json!({}),
        Duration::from_secs(1),
    );
    let server = async {
        assert_eq!(payload(&request(&mut peer).await)["id"], 1);
        peer.write_all(&socket(
            json!({"id": 1, "result": {"handle": "owned-handle"}}),
        ))
        .await
        .unwrap();
    };
    let (result, ()) = bounded(async { tokio::join!(caller, server) }).await;
    assert_eq!(result.unwrap(), json!({"handle": "owned-handle"}));
    assert!(client.is_usable());
}

#[tokio::test]
async fn lifecycle_webinspector_session_automation_max_id_is_sent_once_and_never_wraps() {
    let (stream, mut peer) = duplex(16384);
    let mut client = WebInspectorClient::with_connection_id(stream, "owned");
    let mut session = AutomationSession::with_page("PID:42", "owned-bundle", "owned-session", 7);
    session.next_command_id = Some(u64::MAX);
    assert_eq!(
        bounded(session.send_command(&mut client, "getBrowsingContext", json!({})))
            .await
            .unwrap(),
        u64::MAX
    );
    assert_eq!(payload(&request(&mut peer).await)["id"], u64::MAX);
    let error = session
        .send_command(&mut client, "getBrowsingContext", json!({}))
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
