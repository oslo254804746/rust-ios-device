use super::*;

fn plan() -> Plan {
    Plan {
        client: "fd12:3456:789a:1::2".parse().unwrap(),
        server: "fd12:3456:789a:1::1".parse().unwrap(),
        mtu: 1280,
        prefix: 64,
    }
}

fn identity() -> Identity {
    let mut name = [0; libc::IFNAMSIZ];
    name[..4].copy_from_slice(&[b't' as _, b'u' as _, b'n' as _, b'9' as _]);
    Identity { index: 9, name }
}

fn address(flags: u32, prefix: u8) -> Vec<u8> {
    let mut body = [libc::AF_INET6 as u8, prefix, 0, 0, 0, 0, 0, 0];
    body[4..8].copy_from_slice(&identity().index.to_ne_bytes());
    let mut request = Request::new(NEW_ADDRESS, 0, 3, 17, &body).unwrap();
    request.attribute(1, &plan().client.octets()).unwrap();
    request.attribute(2, &plan().client.octets()).unwrap();
    request.attribute(8, &flags.to_ne_bytes()).unwrap();
    let mut bytes = request.bytes().to_vec();
    bytes.extend_from_slice(&20u16.to_ne_bytes());
    bytes.extend_from_slice(&6u16.to_ne_bytes());
    bytes.extend_from_slice(&u32::MAX.to_ne_bytes());
    bytes.extend_from_slice(&u32::MAX.to_ne_bytes());
    bytes.extend_from_slice(&0u32.to_ne_bytes());
    bytes.extend_from_slice(&0u32.to_ne_bytes());
    let length = bytes.len() as u32;
    bytes[..4].copy_from_slice(&length.to_ne_bytes());
    bytes
}

#[test]
fn actual_address_rows_do_not_confuse_nodad_or_another_prefix_with_readiness() {
    let receipt = AddressReceipt {
        identity: identity(),
        address: plan().client,
        prefix: 64,
        flags: ADDRESS_NO_PREFIX_ROUTE,
        lifetimes: codec::AddressLifetimes::INFINITE,
        derived_local: RouteReceipt {
            identity: identity(),
            destination: plan().client,
            prefix: 128,
            table: 255,
            priority: 0,
            protocol: 2,
            scope: 0,
            kind: 2,
        },
    };
    for flags in [
        ADDRESS_NO_PREFIX_ROUTE | ADDRESS_PERMANENT,
        ADDRESS_NO_PREFIX_ROUTE | ADDRESS_PERMANENT | ADDRESS_TENTATIVE,
        ADDRESS_NO_PREFIX_ROUTE | ADDRESS_PERMANENT | ADDRESS_DAD_FAILED,
    ] {
        let request = address(flags, 64);
        let row = AddressRow::parse(&request[codec::HEADER..])
            .unwrap()
            .unwrap();
        assert!(row.matches(receipt));
        assert_eq!(row.flags, flags);
    }
    for flags in [
        0,
        ADDRESS_NO_PREFIX_ROUTE,
        ADDRESS_NO_PREFIX_ROUTE | ADDRESS_PERMANENT | 2,
        ADDRESS_NO_PREFIX_ROUTE | ADDRESS_PERMANENT | 4,
        ADDRESS_NO_PREFIX_ROUTE | ADDRESS_PERMANENT | 0x20,
    ] {
        let request = address(flags, 64);
        assert!(!AddressRow::parse(&request[codec::HEADER..])
            .unwrap()
            .unwrap()
            .matches(receipt));
    }
    let request = address(ADDRESS_NO_PREFIX_ROUTE | ADDRESS_PERMANENT, 128);
    assert!(!AddressRow::parse(&request[codec::HEADER..])
        .unwrap()
        .unwrap()
        .matches(receipt));
}

#[test]
fn route_readback_checks_full_owned_key_and_preserves_64_bit_prefix_reachability() {
    let receipt = RouteReceipt {
        identity: identity(),
        destination: plan().network(),
        prefix: 64,
        table: ROUTE_TABLE,
        priority: ROUTE_PRIORITY,
        protocol: ROUTE_PROTOCOL_STATIC,
        scope: 0,
        kind: 1,
    };
    let mut request = Request::new(NEW_ROUTE, 0, 7, 17, &route_body(receipt)).unwrap();
    request.attribute(1, &receipt.destination.octets()).unwrap();
    request
        .attribute(4, &receipt.identity.index.to_ne_bytes())
        .unwrap();
    request
        .attribute(6, &receipt.priority.to_ne_bytes())
        .unwrap();
    let row = RouteRow::parse(&request.bytes()[codec::HEADER..])
        .unwrap()
        .unwrap();
    assert!(row.exact(receipt));
    for changed in [
        RouteReceipt {
            prefix: 128,
            ..receipt
        },
        RouteReceipt {
            priority: 7,
            ..receipt
        },
        RouteReceipt {
            table: 255,
            ..receipt
        },
        RouteReceipt {
            protocol: 3,
            ..receipt
        },
        RouteReceipt {
            identity: Identity {
                index: 10,
                ..identity()
            },
            ..receipt
        },
    ] {
        assert!(!row.exact(changed));
    }
    let mut gateway = row;
    gateway.has_gateway = true;
    assert!(!gateway.exact(receipt));
    let mut multipath = row;
    multipath.extra_key = true;
    assert!(!multipath.exact(receipt));
}

#[test]
fn original_native_error_object_is_the_deep_source() {
    let error = Error::Native(io::Error::from_raw_os_error(libc::EEXIST));
    let Error::Native(original) = &error else {
        unreachable!()
    };
    let original_ptr = original as *const io::Error;
    let source = error.source().unwrap().downcast_ref::<io::Error>().unwrap();
    assert_eq!(source as *const io::Error, original_ptr);
    assert_eq!(source.raw_os_error(), Some(libc::EEXIST));
}

#[test]
fn packet_validation_requires_complete_whole_ipv6_packet_and_extra_byte_detection() {
    let mut packet = [0; 40];
    packet[0] = 0x60;
    validate_packet(&packet, 1280).unwrap();
    packet[5] = 1;
    assert!(validate_packet(&packet, 1280).is_err());
    assert!(validate_packet(&packet[..39], 1280).is_err());
    let mut oversized = [0; 1281];
    oversized[0] = 0x60;
    oversized[4..6].copy_from_slice(&1241u16.to_be_bytes());
    assert!(validate_packet(&oversized, 1280).is_err());
}

#[test]
fn plan_refuses_server_only_or_different_network_substitution() {
    let current = plan();
    current.validate().unwrap();
    assert!(Plan {
        prefix: 128,
        ..current
    }
    .validate()
    .is_err());
    assert!(Plan {
        server: "fd12:3456:789a:2::1".parse().unwrap(),
        ..current
    }
    .validate()
    .is_err());
}

#[test]
fn address_exact_readback_refuses_finite_missing_or_duplicate_cacheinfo() {
    let receipt = AddressReceipt {
        identity: identity(),
        address: plan().client,
        prefix: 64,
        flags: ADDRESS_NO_PREFIX_ROUTE,
        lifetimes: codec::AddressLifetimes::INFINITE,
        derived_local: RouteReceipt {
            identity: identity(),
            destination: plan().client,
            prefix: 128,
            table: 255,
            priority: 0,
            protocol: 2,
            scope: 0,
            kind: 2,
        },
    };
    let complete = address(ADDRESS_NO_PREFIX_ROUTE | ADDRESS_PERMANENT, 64);
    assert!(AddressRow::parse(&complete[codec::HEADER..])
        .unwrap()
        .unwrap()
        .matches(receipt));
    let mut finite = complete.clone();
    finite[76..80].copy_from_slice(&60u32.to_ne_bytes());
    finite[80..84].copy_from_slice(&120u32.to_ne_bytes());
    assert!(!AddressRow::parse(&finite[codec::HEADER..])
        .unwrap()
        .unwrap()
        .matches(receipt));
    assert!(!AddressRow::parse(&complete[codec::HEADER..72])
        .unwrap()
        .unwrap()
        .matches(receipt));
    let mut duplicate = complete.clone();
    duplicate.extend_from_slice(&complete[72..]);
    assert!(AddressRow::parse(&duplicate[codec::HEADER..]).is_err());
    let mut short = complete.clone();
    short[72..74].copy_from_slice(&19u16.to_ne_bytes());
    assert!(AddressRow::parse(&short[codec::HEADER..]).is_err());
}

#[test]
fn pending_mtu_recovery_parses_exact_scalar_and_refuses_ambiguous_request() {
    let mut body = [0; 16];
    body[4..8].copy_from_slice(&identity().index.to_ne_bytes());
    let mut request = Request::new(NEW_LINK, codec::ACK, 91, 73, &body).unwrap();
    request
        .attribute(4, &u32::from(plan().mtu).to_ne_bytes())
        .unwrap();
    assert_eq!(request_mtu(&request).unwrap(), Some(plan().mtu));
    request.attribute(4, &1400u32.to_ne_bytes()).unwrap();
    assert!(request_mtu(&request).is_err());
    let empty = Request::new(NEW_LINK, codec::ACK, 92, 73, &body).unwrap();
    assert_eq!(request_mtu(&empty).unwrap(), None);
    let mut oversized = Request::new(NEW_LINK, codec::ACK, 93, 73, &body).unwrap();
    oversized.attribute(4, &u32::MAX.to_ne_bytes()).unwrap();
    assert!(request_mtu(&oversized).is_err());
}
