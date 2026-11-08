use super::*;
fn message(payload: DtxPayload) -> DtxMessage {
    DtxMessage {
        identifier: 1,
        conversation_idx: 0,
        channel_code: 1,
        expects_reply: false,
        payload,
    }
}
#[test]
fn process_archive_keeps_unsigned_sentinels_and_signed_values() {
    let values = plist::Value::Array(vec![
        plist::Value::Integer(42.into()),
        plist::Value::Integer(u64::MAX.into()),
        plist::Value::Integer((-1).into()),
    ]);
    let processes =
        plist::Value::Dictionary(plist::Dictionary::from_iter([("42".to_string(), values)]));
    let bytes = crate::archive_dict(vec![("Processes".into(), processes)]);
    let msg = checked_sysmontap_message(message(DtxPayload::Raw(bytes.into()))).unwrap();
    let sample =
        parse_process_snapshot(&msg, &["pid".into(), "sentinel".into(), "signed".into()]).unwrap();
    assert_eq!(sample.processes.len(), 1);
    assert_eq!(sample.processes[0]["pid"].as_u64(), Some(42));
    assert_eq!(sample.processes[0]["sentinel"].as_u64(), Some(u64::MAX));
    assert_eq!(sample.processes[0]["signed"].as_i64(), Some(-1));
}
#[test]
fn malformed_archive_and_remote_error_do_not_silently_wait_for_another_snapshot() {
    assert!(
        checked_sysmontap_message(message(DtxPayload::Raw(bytes::Bytes::from_static(
            b"not-an-archive"
        ))))
        .is_err()
    );
    assert!(
        checked_sysmontap_message(message(DtxPayload::Error(NSObject::String(
            "rejected".into()
        ))))
        .is_err()
    );
}
