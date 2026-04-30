#[path = "../src/test_util.rs"]
mod test_util;

use test_util::MockStream;

#[tokio::test]
async fn get_pid_list_sends_pid_list_request_and_parses_payload() {
    let mut stream = MockStream::with_prefixed_plist_response(
        &[1],
        plist::Value::Dictionary(plist::Dictionary::from_iter([(
            "Payload".to_string(),
            plist::Value::Array(vec![plist::Value::Dictionary(
                plist::Dictionary::from_iter([
                    ("PID".to_string(), plist::Value::Integer(42.into())),
                    (
                        "Name".to_string(),
                        plist::Value::String("SpringBoard".into()),
                    ),
                ]),
            )]),
        )])),
    );
    let mut client = ios_core::ostrace::OsTraceClient::new(&mut stream);

    let response = client.get_pid_list().await.unwrap();
    let payload = response
        .get("Payload")
        .and_then(plist::Value::as_array)
        .expect("payload array");
    assert_eq!(payload.len(), 1);

    let len = u32::from_be_bytes(stream.written[..4].try_into().unwrap()) as usize;
    let request: plist::Dictionary = plist::from_bytes(&stream.written[4..4 + len]).unwrap();
    assert_eq!(request["Request"].as_string(), Some("PidList"));
}
