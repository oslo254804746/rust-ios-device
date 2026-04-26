use ios_core::proto::nskeyedarchiver_encode;
use ios_core::services::accessibility_audit::{
    deserialize_ax_object, AccessibilityAuditClient, FocusElement,
};
use ios_core::services::dtx::primitive_enc::{archived_object, encode_primitive_dict};
use ios_core::services::dtx::{encode_dtx, read_dtx_frame, DtxPayload, NSObject};
use plist::{Dictionary, Value};
use serde_json::json;
use tokio::io::{duplex, AsyncWriteExt};

#[tokio::test]
async fn lockdown_capabilities_requests_device_capabilities_without_publishing_capabilities() {
    let (client, mut server) = duplex(4096);
    let task = tokio::spawn(async move {
        let mut audit = AccessibilityAuditClient::new(client, 17);
        audit.capabilities().await.unwrap()
    });

    let request = read_dtx_frame(&mut server).await.unwrap();
    match &request.payload {
        DtxPayload::MethodInvocation { selector, args } => {
            assert_eq!(selector, "deviceCapabilities");
            assert!(args.is_empty());
        }
        other => panic!("unexpected deviceCapabilities request: {other:?}"),
    }

    let response = nskeyedarchiver_encode::archive_array(vec![
        Value::String("cap-one".to_string()),
        Value::String("cap-two".to_string()),
    ]);
    server
        .write_all(&encode_dtx(
            request.identifier,
            1,
            0,
            false,
            3,
            &response,
            &[],
        ))
        .await
        .unwrap();

    let capabilities = task.await.unwrap();
    assert_eq!(
        capabilities,
        vec!["cap-one".to_string(), "cap-two".to_string()]
    );
}

#[tokio::test]
async fn explicit_publish_handshake_sends_capabilities_before_requesting_device_capabilities() {
    let (client, mut server) = duplex(4096);
    let task = tokio::spawn(async move {
        let mut audit = AccessibilityAuditClient::new_with_handshake(
            client,
            17,
            ios_core::services::accessibility_audit::AccessibilityAuditHandshake::PublishCapabilities,
        );
        audit.capabilities().await.unwrap()
    });

    let publish = read_dtx_frame(&mut server).await.unwrap();
    match &publish.payload {
        DtxPayload::MethodInvocation { selector, args } => {
            assert_eq!(selector, "_notifyOfPublishedCapabilities:");
            assert_eq!(publish.channel_code, 0);
            assert_eq!(args.len(), 1);
            match &args[0] {
                NSObject::Dict(dict) => {
                    assert_eq!(
                        dict.get("com.apple.private.DTXBlockCompression"),
                        Some(&NSObject::Int(2))
                    );
                    assert_eq!(
                        dict.get("com.apple.private.DTXConnection"),
                        Some(&NSObject::Int(1))
                    );
                }
                other => panic!("unexpected publish capabilities payload: {other:?}"),
            }
        }
        other => panic!("unexpected publish frame: {other:?}"),
    }

    let flush_selector = nskeyedarchiver_encode::archive_string("hostAppStateChanged:");
    let flush_payload = nskeyedarchiver_encode::archive_dict(vec![(
        "state".to_string(),
        Value::String("ready".to_string()),
    )]);
    let flush_aux = encode_primitive_dict(&[archived_object(flush_payload.clone())]);
    server
        .write_all(&encode_dtx(50, 0, 0, true, 2, &flush_selector, &flush_aux))
        .await
        .unwrap();
    let ack1 = read_dtx_frame(&mut server).await.unwrap();
    assert!(matches!(ack1.payload, DtxPayload::Empty));

    server
        .write_all(&encode_dtx(51, 0, 0, true, 2, &flush_selector, &flush_aux))
        .await
        .unwrap();
    let ack2 = read_dtx_frame(&mut server).await.unwrap();
    assert!(matches!(ack2.payload, DtxPayload::Empty));

    let request = read_dtx_frame(&mut server).await.unwrap();
    match &request.payload {
        DtxPayload::MethodInvocation { selector, args } => {
            assert_eq!(selector, "deviceCapabilities");
            assert!(args.is_empty());
        }
        other => panic!("unexpected deviceCapabilities request: {other:?}"),
    }

    let response =
        nskeyedarchiver_encode::archive_array(vec![Value::String("cap-explicit".to_string())]);
    server
        .write_all(&encode_dtx(
            request.identifier,
            1,
            0,
            false,
            3,
            &response,
            &[],
        ))
        .await
        .unwrap();

    let capabilities = task.await.unwrap();
    assert_eq!(capabilities, vec!["cap-explicit".to_string()]);
}

#[tokio::test]
async fn rsd_capabilities_requests_device_capabilities_without_publishing_capabilities() {
    let (client, mut server) = duplex(4096);
    let task = tokio::spawn(async move {
        let mut audit = AccessibilityAuditClient::new_rsd(client, 17);
        audit.capabilities().await.unwrap()
    });

    let request = read_dtx_frame(&mut server).await.unwrap();
    match &request.payload {
        DtxPayload::MethodInvocation { selector, args } => {
            assert_eq!(selector, "deviceCapabilities");
            assert!(args.is_empty());
        }
        other => panic!("unexpected first RSD request: {other:?}"),
    }

    let response = nskeyedarchiver_encode::archive_array(vec![
        Value::String("cap-rsd-one".to_string()),
        Value::String("cap-rsd-two".to_string()),
    ]);
    server
        .write_all(&encode_dtx(
            request.identifier,
            1,
            0,
            false,
            3,
            &response,
            &[],
        ))
        .await
        .unwrap();

    let capabilities = task.await.unwrap();
    assert_eq!(
        capabilities,
        vec!["cap-rsd-one".to_string(), "cap-rsd-two".to_string()]
    );
}

#[test]
fn deserialize_ax_object_unwraps_passthrough_layers_recursively() {
    let mut inner = Dictionary::new();
    inner.insert(
        "CaptionTextValue_v1".to_string(),
        Value::Dictionary(Dictionary::from_iter([
            (
                "ObjectType".to_string(),
                Value::String("passthrough".to_string()),
            ),
            ("Value".to_string(), Value::String("Hello".to_string())),
        ])),
    );

    let value = Value::Dictionary(Dictionary::from_iter([
        (
            "ObjectType".to_string(),
            Value::String("AXAuditInspectorFocus_v1".to_string()),
        ),
        (
            "Value".to_string(),
            Value::Dictionary(Dictionary::from_iter([
                (
                    "ObjectType".to_string(),
                    Value::String("passthrough".to_string()),
                ),
                ("Value".to_string(), Value::Dictionary(inner)),
            ])),
        ),
    ]));

    let json = deserialize_ax_object(&value);
    assert_eq!(json["CaptionTextValue_v1"], "Hello");
}

#[test]
fn focus_element_parses_caption_spoken_description_and_identifier() {
    let value = json!({
        "CaptionTextValue_v1": "Play",
        "SpokenDescriptionValue_v1": "Play button",
        "ElementValue_v1": {
            "PlatformElementValue_v1": [1, 2, 3, 4, 5, 6, 7, 8, 0, 1, 2, 3, 0xAA, 0xBB, 0xCC, 0xDD]
        }
    });

    let focus = FocusElement::from_event_payload(&value).unwrap();
    assert_eq!(focus.caption.as_deref(), Some("Play"));
    assert_eq!(focus.spoken_description.as_deref(), Some("Play button"));
    assert_eq!(
        focus.platform_identifier,
        "010203040506070800010203AABBCCDD"
    );
    assert_eq!(focus.estimated_uid, "AABBCCDD-0000-0000-0102-000000000000");
}

#[test]
fn focus_element_accepts_list_payload_shape() {
    let value = json!([
        {
            "CaptionTextValue_v1": "Play",
            "SpokenDescriptionValue_v1": "Play button",
            "ElementValue_v1": {
                "PlatformElementValue_v1": [1, 2, 3, 4, 5, 6, 7, 8, 0, 1, 2, 3, 0xAA, 0xBB, 0xCC, 0xDD]
            }
        }
    ]);

    let focus = FocusElement::from_event_payload(&value).unwrap();
    assert_eq!(focus.caption.as_deref(), Some("Play"));
    assert_eq!(focus.spoken_description.as_deref(), Some("Play button"));
    assert_eq!(
        focus.platform_identifier,
        "010203040506070800010203AABBCCDD"
    );
    assert_eq!(focus.estimated_uid, "AABBCCDD-0000-0000-0102-000000000000");
}
