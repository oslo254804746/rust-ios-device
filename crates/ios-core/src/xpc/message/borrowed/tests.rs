use super::*;
use std::error::Error;

fn wire(value: XpcValue) -> Bytes {
    encode_message(&XpcMessage {
        flags: flags::ALWAYS_SET,
        msg_id: 0,
        body: Some(value),
    })
    .unwrap()
}

fn native_utf8(mut error: &(dyn Error + 'static)) -> bool {
    for _ in 0..32 {
        if error.is::<std::str::Utf8Error>() {
            return true;
        }
        let next = if let Some(io) = error.downcast_ref::<io::Error>() {
            io.get_ref().map(|source| source as &(dyn Error + 'static))
        } else {
            error.source()
        };
        match next {
            Some(next) => error = next,
            None => break,
        }
    }
    false
}

#[test]
fn string_borrows_original_wire_and_arena_owns_exact_requested_layouts() {
    let bytes = wire(XpcValue::String("borrowed-private".into()));
    let bank = Arc::new(Semaphore::new(128 * 1024 * 1024));
    let before = bank.available_permits();
    let arena = decode(&bytes[..24], &bytes[24..], &bank).unwrap().unwrap();
    let text = arena.root().as_str().unwrap();
    assert_eq!(text, "borrowed-private");
    assert!(
        text.as_ptr() >= bytes.as_ptr() && text.as_ptr() < bytes.as_ptr().wrapping_add(bytes.len())
    );
    let bytes_charged = arena.nodes.capacity() * std::mem::size_of::<Node<'_>>()
        + arena.edges.capacity() * std::mem::size_of::<Edge<'_>>();
    assert_eq!(bank.available_permits(), before - bytes_charged);
    drop(arena);
    assert_eq!(bank.available_permits(), before);
}

#[test]
fn duplicate_keys_keep_first_order_and_last_value_without_indexmap_storage() {
    let mut body = Vec::new();
    body.extend_from_slice(&OBJECT_MAGIC.to_le_bytes());
    body.extend_from_slice(&BODY_VERSION.to_le_bytes());
    body.extend_from_slice(&TYPE_DICTIONARY.to_le_bytes());
    body.extend_from_slice(&0u32.to_le_bytes()); // original decoder ignores container length
    body.extend_from_slice(&3u32.to_le_bytes());
    for (key, value) in [(b"a\0\0\0", 1u64), (b"b\0\0\0", 2), (b"a\0\0\0", 3)] {
        body.extend_from_slice(key);
        body.extend_from_slice(&TYPE_UINT64.to_le_bytes());
        body.extend_from_slice(&value.to_le_bytes());
    }
    let mut header = wire(XpcValue::Null)[..24].to_vec();
    header[8..16].copy_from_slice(&(body.len() as u64).to_le_bytes());
    let bank = Arc::new(Semaphore::new(4096));
    let arena = decode(&header, &body, &bank).unwrap().unwrap();
    let dictionary = arena.root().as_dict().unwrap();
    assert_eq!(dictionary.len(), 2);
    let actual: Vec<_> = dictionary
        .iter()
        .map(|(key, value)| (key, value.as_uint64().unwrap()))
        .collect();
    assert_eq!(actual, [("a", 3), ("b", 2)]);
}

#[test]
fn native_utf8_validation_precedes_arena_allocation_and_keeps_source() {
    let mut bytes = wire(XpcValue::String("x".into())).to_vec();
    bytes[40] = 0xff;
    let bank = Arc::new(Semaphore::new(0));
    let error = match decode(&bytes[..24], &bytes[24..], &bank) {
        Ok(_) => panic!("invalid UTF-8 accepted"),
        Err(error) => error,
    };
    assert!(native_utf8(&error));
    assert_eq!(bank.available_permits(), 0);
}

#[test]
fn arena_admission_failure_releases_earlier_vector_grant() {
    let bytes = wire(XpcValue::Array(vec![XpcValue::Null; 16]));
    let bank = Arc::new(Semaphore::new(17 * std::mem::size_of::<Node<'_>>()));
    let before = bank.available_permits();
    let error = match decode(&bytes[..24], &bytes[24..], &bank) {
        Ok(_) => panic!("edge allocation should be denied"),
        Err(error) => error,
    };
    assert_eq!(
        crate::classify_error(&error),
        crate::ErrorClass::ResourceLimit
    );
    assert_eq!(bank.available_permits(), before);
}

#[test]
fn borrowed_grammar_matches_original_for_nested_values_every_truncation_and_wire_mutations() {
    fn compare(value: Value<'_, '_>, original: &XpcValue) {
        assert_eq!(value.as_str(), original.as_str());
        assert_eq!(
            value.as_uint64(),
            match original {
                XpcValue::Uint64(n) => Some(*n),
                _ => None,
            }
        );
        match original {
            XpcValue::Dictionary(original) => {
                let actual = value.as_dict().unwrap();
                assert_eq!(actual.len(), original.len());
                for ((key, child), (expected_key, expected)) in actual.iter().zip(original) {
                    assert_eq!(key, expected_key);
                    compare(child, expected);
                }
            }
            XpcValue::Array(original) => {
                let actual = value.as_array().unwrap();
                assert_eq!(actual.len(), original.len());
                for (child, expected) in actual.iter().zip(original) {
                    compare(child, expected);
                }
            }
            _ => {
                assert!(value.as_dict().is_none());
                assert!(value.as_array().is_none());
            }
        }
    }
    let bytes = wire(XpcValue::Dictionary(IndexMap::from([(
        "all".into(),
        XpcValue::Array(vec![
            XpcValue::Null,
            XpcValue::Bool(true),
            XpcValue::Int64(-1),
            XpcValue::Uint64(u64::MAX),
            XpcValue::Double(1.5),
            XpcValue::Date(-23),
            XpcValue::Data(Bytes::from_static(b"data")),
            XpcValue::String("utf8-中文\0tail".into()),
            XpcValue::Uuid([8; 16]),
            XpcValue::FileTransfer {
                msg_id: 5,
                data: Box::new(XpcValue::Null),
            },
            XpcValue::Dictionary(IndexMap::from([(
                "nested".into(),
                XpcValue::String("value".into()),
            )])),
        ]),
    )])));
    let bank = Arc::new(Semaphore::new(1024 * 1024));
    let mut cases = vec![bytes.to_vec()];
    for end in 24..bytes.len() {
        cases.push(bytes[..end].to_vec());
    }
    for offset in 0..bytes.len() {
        for mask in [0x01, 0xff] {
            let mut case = bytes.to_vec();
            case[offset] ^= mask;
            cases.push(case);
        }
    }
    // Original ignored trailing bytes are accepted by both paths.
    let mut trailing = bytes.to_vec();
    trailing.extend_from_slice(&[0xff; 5]);
    cases.push(trailing);
    for case in cases {
        let original = decode_message(Bytes::copy_from_slice(&case));
        let actual = decode(&case[..24], &case[24..], &bank);
        assert_eq!(actual.is_ok(), original.is_ok(), "wire {case:?}");
        match (actual, original) {
            (Ok(actual), Ok(original)) => match (actual, original.body) {
                (Some(tree), Some(value)) => compare(tree.root(), &value),
                (None, None) => (),
                _ => panic!("body presence differs"),
            },
            (Err(actual), Err(original)) => {
                assert_eq!(
                    crate::classify_error(&actual),
                    crate::classify_error(&original)
                );
                assert_eq!(native_utf8(&actual), native_utf8(&original));
            }
            _ => unreachable!(),
        }
        assert_eq!(bank.available_permits(), 1024 * 1024);
    }
}
