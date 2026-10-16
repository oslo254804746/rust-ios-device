use super::super::metadata_vec;
use super::*;
use std::sync::atomic::{AtomicUsize, Ordering};

#[test]
fn paid_json_matches_locked_value_for_duplicate_escaped_keys_unicode_numbers_and_extensions() {
    let input = br#"{"a":1,"\u0061":{"snow":"\u96ea\ud83e\udd80","nul":"\u0000","extension":[null,true,false,18446744073709551615,-9223372036854775808,1.25,-0.0,1e100]},"empty":"","z":[],"object":{}}"#;
    let bank = Arc::new(Semaphore::new(16384));
    let expected: serde_json::Value = serde_json::from_slice(input).unwrap();
    let paid = parse(input, &bank).unwrap();
    assert_eq!(
        serde_json::to_vec(&paid).unwrap(),
        serde_json::to_vec(&expected).unwrap()
    );
    assert_eq!(paid.object().unwrap().len(), 4);
    assert_eq!(
        paid.get("a").unwrap().get("snow").unwrap().string(),
        Some("雪🦀")
    );
    drop(paid);
    assert_eq!(bank.available_permits(), 16384);
}

#[test]
fn paid_json_text_one_byte_refusal_has_zero_destination_allocations_and_original_resource_cause() {
    let bank = Arc::new(Semaphore::new(2));
    let calls = Arc::new(AtomicUsize::new(0));
    let captured = calls.clone();
    let error = metadata_vec::with_allocation_hook(
        move |_, _| {
            captured.fetch_add(1, Ordering::SeqCst);
        },
        || parse(br#""\u96ea""#, &bank).err().unwrap(),
    );
    assert_eq!(
        crate::output::operation_error_class(error.as_ref()),
        ios_core::ErrorClass::ResourceLimit
    );
    assert!(error
        .chain()
        .any(|cause| cause.to_string() == "tunnel global known metadata byte limit exceeded"));
    assert!(!error.is::<serde_json::Error>());
    assert_eq!(calls.load(Ordering::SeqCst), 0);
    assert_eq!(bank.available_permits(), 2);
}

#[test]
fn paid_json_actual_array_backing_is_paid_before_allocation_and_partial_failure_refunds() {
    let capacity = 2 * std::mem::size_of::<Value>();
    let bank = Arc::new(Semaphore::new(capacity));
    let observed = bank.clone();
    let calls = Arc::new(AtomicUsize::new(0));
    let captured = calls.clone();
    let paid = metadata_vec::with_allocation_hook(
        move |layout, _| {
            assert_eq!(layout.size(), capacity);
            assert_eq!(observed.available_permits(), 0);
            captured.fetch_add(1, Ordering::SeqCst);
        },
        || parse(b"[null]", &bank).unwrap(),
    );
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    assert_eq!(paid.array().unwrap().len(), 1);
    drop(paid);
    assert_eq!(bank.available_permits(), capacity);
    let error = parse(b"[null,\"x\"]", &bank).err().unwrap();
    assert_eq!(
        crate::output::operation_error_class(error.as_ref()),
        ios_core::ErrorClass::ResourceLimit
    );
    assert_eq!(bank.available_permits(), capacity);
}

#[test]
fn paid_json_syntax_depth_surrogate_and_trailing_rules_match_original_decoder_and_refund() {
    let bank = Arc::new(Semaphore::new(1024 * 1024));
    let mut cases = vec![
        b"{\"a\":[1,2],\"b\":}".to_vec(),
        b"true false".to_vec(),
        br#""\ud800""#.to_vec(),
        b"1e999".to_vec(),
    ];
    for depth in [126, 127, 128, 129] {
        cases.push(format!("{}null{}", "[".repeat(depth), "]".repeat(depth)).into_bytes());
    }
    for bytes in cases {
        let expected = serde_json::from_slice::<serde_json::Value>(&bytes);
        let actual = parse(&bytes, &bank);
        assert_eq!(
            actual.is_ok(),
            expected.is_ok(),
            "depth/grammar diverged for {} bytes",
            bytes.len()
        );
        match actual {
            Ok(value) => drop(value),
            Err(error) => assert!(error.is::<serde_json::Error>()),
        }
        assert_eq!(bank.available_permits(), 1024 * 1024);
    }
}

#[test]
fn paid_json_owned_decoded_text_outlives_input_and_holds_exact_fee_until_tree_drop() {
    let bank = Arc::new(Semaphore::new(4096));
    let mut input = br#"{"identity":"\u96ea\ud83e\udd80"}"#.to_vec();
    let value = parse(&input, &bank).unwrap();
    let text = value.get("identity").unwrap().string().unwrap();
    assert!(
        text.as_ptr() < input.as_ptr() || text.as_ptr() >= input.as_ptr().wrapping_add(input.len())
    );
    let before = bank.available_permits();
    input.fill(0);
    drop(input);
    let held = bank.clone().try_acquire_many_owned(before as u32).unwrap();
    assert_eq!(value.get("identity").unwrap().string(), Some("雪🦀"));
    assert_eq!(
        serde_json::to_string(&value).unwrap(),
        "{\"identity\":\"雪🦀\"}"
    );
    drop(value);
    assert_eq!(bank.available_permits(), 4096 - before);
    drop(held);
    assert_eq!(bank.available_permits(), 4096);
}

#[test]
fn paid_json_listing_shape_refusal_releases_graph_and_empty_values_need_no_backing() {
    let bank = Arc::new(Semaphore::new(4096));
    for input in [
        b"[]".as_slice(),
        b"{\"uid\":null}",
        b"{\"uid\":{\"object\":true}}",
    ] {
        let error = Listing::parse(input, &bank).err().unwrap();
        assert!(error.is::<serde_json::Error>());
        assert_eq!(bank.available_permits(), 4096);
    }
    let empty = Arc::new(Semaphore::new(0));
    for input in [b"null".as_slice(), b"true", b"123", b"\"\"", b"[]", b"{}"] {
        drop(parse(input, &empty).unwrap());
    }
    assert_eq!(empty.available_permits(), 0);
}

#[test]
fn paid_json_reverse_object_and_late_duplicates_keep_last_values_with_in_place_sort() {
    use std::fmt::Write as _;
    let mut input = String::from("{");
    for index in (0..8192).rev() {
        write!(&mut input, "\"key-{index:05}\":{index},").unwrap();
    }
    input.push_str("\"key-04000\":{\"last\":true},\"key-04000\":[\"final\"],\"key-00000\":null}");
    let bank = Arc::new(Semaphore::new(8 * 1024 * 1024));
    let expected: serde_json::Value = serde_json::from_str(&input).unwrap();
    let paid = parse(input.as_bytes(), &bank).unwrap();
    assert_eq!(paid.object().unwrap().len(), 8192);
    assert_eq!(
        serde_json::to_vec(&paid).unwrap(),
        serde_json::to_vec(&expected).unwrap()
    );
    assert_eq!(
        paid.get("key-04000").unwrap().array().unwrap()[0].string(),
        Some("final")
    );
    drop(paid);
    assert_eq!(bank.available_permits(), 8 * 1024 * 1024);
}
