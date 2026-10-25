use super::*;

fn query() -> Query {
    Query {
        destination: "fd12:3456:789a:1::".parse().unwrap(),
        prefix: 64,
        index: 7,
        scoped: true,
    }
}
fn request(kind: u8) -> Request {
    Request::new(
        kind,
        203,
        41,
        query(),
        if kind == GET {
            None
        } else {
            Some(Address::link(7))
        },
        if kind == ADD {
            Some("fd12:3456:789a:1::2".parse().unwrap())
        } else {
            None
        },
        UP | STATIC,
    )
    .unwrap()
}
fn observed(kind: u8) -> Vec<u8> {
    let mut bytes = request(ADD).bytes().to_vec();
    bytes[3] = kind;
    bytes
}

#[test]
fn native_layout_uses_32_bit_alignment_and_full_u_short_response_range() {
    assert_eq!(HEADER, 92);
    assert_eq!(RESPONSE_CAPACITY, 65535);
    assert_eq!(request(ADD).bytes().len(), 192);
    assert_eq!(round32(28).unwrap(), 28);
    assert_eq!(round32(255).unwrap(), 256);
    let bytes = observed(GET);
    let row = Message::parse(&bytes).unwrap().row().unwrap();
    assert_eq!(row.query, query());
    assert_eq!(row.gateway.link_index().unwrap(), 7);
    // The first sockaddr begins at 92, which is intentionally not aligned to
    // eight. sizeof(long) alignment would corrupt this real Darwin format.
    assert_eq!(&bytes[HEADER..HEADER + 2], &[28, AF_INET6]);
}

#[test]
fn route_ack_correlates_pid_sequence_type_interface_destination_mask_and_gateway() {
    let original = request(ADD);
    let bytes = observed(ADD);
    Message::parse(&bytes).unwrap().status(&original).unwrap();
    for offset in [3, 4, 16, 20, HEADER + 8, HEADER + 28 + 2] {
        let mut changed = bytes.clone();
        changed[offset] ^= 1;
        assert!(
            Message::parse(&changed).unwrap().status(&original).is_err(),
            "offset {offset}"
        );
    }
    let different_mask = Request::new(
        ADD,
        203,
        41,
        Query {
            prefix: 128,
            ..query()
        },
        Some(Address::link(7)),
        None,
        UP | STATIC,
    )
    .unwrap();
    assert!(Message::parse(different_mask.bytes())
        .unwrap()
        .status(&original)
        .is_err());
}

#[test]
fn exact_get_readback_exposes_the_actual_row_and_native_errno_without_string_copy() {
    let original = request(GET);
    let bytes = observed(GET);
    let message = Message::parse(&bytes).unwrap();
    message.status(&original).unwrap();
    assert!(message.row().unwrap().reachable_on(query()));
    let mut failed = original.bytes().to_vec();
    failed[24..28].copy_from_slice(&3i32.to_ne_bytes());
    let error = Message::parse(&failed)
        .unwrap()
        .status(&original)
        .unwrap_err();
    assert_eq!(error.raw_os_error(), Some(3));
    failed[24..28].copy_from_slice(&(-1i32).to_ne_bytes());
    assert!(Message::parse(&failed).is_err());
}

#[test]
fn compact_masks_are_checked_for_contiguity_and_preserve_64_reachability() {
    let full = Address::mask(64).unwrap();
    assert_eq!(full.prefix().unwrap(), 64);
    let mut compact = full.bytes()[..16].to_vec();
    compact[0] = 16;
    assert_eq!(Address::from_bytes(&compact).unwrap().prefix().unwrap(), 64);
    compact[9] = 0x7f;
    assert!(Address::from_bytes(&compact).unwrap().prefix().is_err());
    assert_eq!(Address::from_bytes(&[2, 0]).unwrap().prefix().unwrap(), 0);
    assert_eq!(Address::mask(128).unwrap().prefix().unwrap(), 128);
    assert!(Address::mask(129).is_err());
}

#[test]
fn owned_readback_ignores_only_documented_counters_and_ref_bookkeeping() {
    let bytes = observed(GET);
    let row = Message::parse(&bytes).unwrap().row().unwrap();
    let mut counters = bytes.clone();
    counters[72..76].copy_from_slice(&1234u32.to_ne_bytes()); // rmx_pksent
    counters[28..32].copy_from_slice(&29u32.to_ne_bytes()); // rtm_use
    let flags = u32_at(&counters, 8).unwrap() | BOOKKEEPING;
    counters[8..12].copy_from_slice(&flags.to_ne_bytes());
    assert!(row.same_owned(Message::parse(&counters).unwrap().row().unwrap()));
    for offset in [36, 40, 44, 48] {
        let mut changed = bytes.clone();
        changed[offset] ^= 1;
        assert!(!row.same_owned(Message::parse(&changed).unwrap().row().unwrap()));
    }
    let mut changed = row;
    changed.flags |= GATEWAY;
    assert!(!changed.reachable_on(query()));
    changed = row;
    changed.query.prefix = 128;
    assert!(!changed.reachable_on(query()));
}

#[test]
fn truncated_malformed_or_extra_route_keys_never_become_acknowledgements() {
    let bytes = observed(GET);
    for length in 0..HEADER {
        assert!(Message::parse(&bytes[..length]).is_err());
    }
    let mut short = bytes.clone();
    short[0..2].copy_from_slice(&91u16.to_ne_bytes());
    assert!(Message::parse(&short).is_err());
    let mut corrupt = bytes.clone();
    corrupt[HEADER] = 1;
    assert!(Message::parse(&corrupt).unwrap().row().is_err());
    let mut extra = bytes.clone();
    let addrs = u32_at(&extra, 12).unwrap() | 0x100;
    extra[12..16].copy_from_slice(&addrs.to_ne_bytes());
    assert!(Message::parse(&extra).unwrap().row().is_err());
    let mut bad_version = bytes;
    bad_version[2] += 1;
    assert!(Message::parse(&bad_version).is_err());
}

#[test]
fn retry_keeps_original_operation_key_while_ignoring_only_new_wire_sequence() {
    let original = request(ADD);
    let newer = Request::new(
        ADD,
        203,
        42,
        query(),
        Some(Address::link(7)),
        Some("fd12:3456:789a:1::2".parse().unwrap()),
        UP | STATIC,
    )
    .unwrap();
    assert!(original.same_operation(&newer));
    assert!(!original.same_operation(&request(DELETE)));
    let different = Request::new(
        ADD,
        203,
        43,
        query(),
        Some(Address::link(8)),
        None,
        UP | STATIC,
    )
    .unwrap();
    assert!(!original.same_operation(&different));
}

#[test]
fn protocol_sized_header_has_no_guessed_4k_receive_limit() {
    let mut bytes = vec![0; RESPONSE_CAPACITY];
    bytes[..2].copy_from_slice(&u16::MAX.to_ne_bytes());
    bytes[2] = VERSION;
    bytes[3] = GET;
    assert_eq!(
        Message::parse(&bytes).unwrap().bytes.len(),
        RESPONSE_CAPACITY
    );
    bytes.push(0);
    assert!(Message::parse(&bytes).is_err());
}
