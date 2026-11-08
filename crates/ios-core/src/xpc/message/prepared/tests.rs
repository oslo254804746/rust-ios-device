use super::*;

#[test]
fn borrowed_sending_grammar_matches_original_bytes_for_nested_utf8_nul_padding_and_all_fields() {
    let id = [0x91; 16];
    let inner = [
        ("flag", Value::Bool(false)),
        ("count", Value::Uint64(u64::MAX)),
    ];
    let fields = [
        ("unicode-中文", Value::String("value\0tail")),
        ("id", Value::Uuid(&id)),
        ("props", Value::Dictionary(&inner)),
        ("empty", Value::Dictionary(&[])),
    ];
    let original = XpcValue::Dictionary(IndexMap::from([
        (
            "unicode-中文".into(),
            XpcValue::String("value\0tail".into()),
        ),
        ("id".into(), XpcValue::Uuid(id)),
        (
            "props".into(),
            XpcValue::Dictionary(IndexMap::from([
                ("flag".into(), XpcValue::Bool(false)),
                ("count".into(), XpcValue::Uint64(u64::MAX)),
            ])),
        ),
        ("empty".into(), XpcValue::Dictionary(IndexMap::new())),
    ]));
    for message_flags in [
        flags::ALWAYS_SET,
        flags::ALWAYS_SET | flags::DATA,
        flags::INIT_HANDSHAKE | flags::ALWAYS_SET,
    ] {
        let bank = Arc::new(Semaphore::new(65536));
        let actual = encode(message_flags, 31, Some(&Value::Dictionary(&fields)), &bank).unwrap();
        let expected = encode_message(&XpcMessage {
            flags: message_flags,
            msg_id: 31,
            body: Some(original.clone()),
        })
        .unwrap();
        assert_eq!(&actual.bytes[..], &expected[..]);
        assert_eq!(actual.bytes.capacity(), expected.len());
        assert_eq!(bank.available_permits(), 65536 - expected.len());
        drop(actual);
        assert_eq!(bank.available_permits(), 65536);
    }
}

#[test]
fn exact_wire_last_byte_denial_keeps_original_borrowed_text_and_returns_grant_after_last_owner() {
    let original = String::from("original-borrowed-文本");
    let pointer = original.as_ptr();
    let expected = encode_message(&XpcMessage {
        flags: flags::ALWAYS_SET,
        msg_id: 0,
        body: Some(XpcValue::String(original.clone())),
    })
    .unwrap();
    let short = Arc::new(Semaphore::new(expected.len() - 1));
    let error = encode(
        flags::ALWAYS_SET,
        0,
        Some(&Value::String(&original)),
        &short,
    )
    .err()
    .unwrap();
    assert_eq!(
        crate::classify_error(&error),
        crate::ErrorClass::ResourceLimit
    );
    assert_eq!(short.available_permits(), expected.len() - 1);
    let bank = Arc::new(Semaphore::new(expected.len()));
    let wire = encode(flags::ALWAYS_SET, 0, Some(&Value::String(&original)), &bank).unwrap();
    assert_eq!(bank.available_permits(), 0);
    assert_eq!(&wire.bytes[..], &expected[..]);
    assert_eq!(original.as_ptr(), pointer);
    drop(wire);
    assert_eq!(bank.available_permits(), expected.len());
    assert_eq!(original.as_ptr(), pointer);
}
