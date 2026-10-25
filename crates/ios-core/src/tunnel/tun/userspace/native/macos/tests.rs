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
    for (slot, byte) in name.iter_mut().zip(b"utun7") {
        *slot = *byte as _;
    }
    Identity { index: 7, name }
}

#[test]
fn exact_alias_ioctl_layout_matches_xnu_64_bit_types_and_numbers() {
    assert_eq!(size_of::<In6Alias>(), 128);
    assert_eq!(offset_of!(In6Alias, address), 16);
    assert_eq!(offset_of!(In6Alias, destination), 44);
    assert_eq!(offset_of!(In6Alias, mask), 72);
    assert_eq!(offset_of!(In6Alias, flags), 100);
    assert_eq!(offset_of!(In6Alias, lifetime), 104);
    assert_eq!(iow::<In6Alias>(26), 0x8080691a);
    assert_eq!(iow::<libc::ifreq>(52), 0x80206934);
    assert_eq!(iowr::<libc::ifreq>(51), 0xc0206933);
}

#[test]
fn address_readiness_requires_exact_address_prefix_lifetime_and_real_dad_flags() {
    let receipt = AddressReceipt {
        identity: identity(),
        address: plan().client,
        prefix: 64,
    };
    let mut row = AddressRow {
        address: plan().client,
        scope: 0,
        mask: mask64(),
        flags: 0,
        valid: u32::MAX,
        preferred: u32::MAX,
    };
    assert!(row.matches(receipt));
    row.flags = IN6_TENTATIVE;
    assert!(row.matches(receipt));
    for flags in [IN6_NODAD, 0x10, 0x200, 0x40, 0x80] {
        row.flags = flags;
        assert!(!row.matches(receipt));
    }
    row.flags = 0;
    row.valid = 60;
    assert!(!row.matches(receipt));
    row.valid = u32::MAX;
    row.preferred = 60;
    assert!(!row.matches(receipt));
    row.preferred = u32::MAX;
    row.mask = Ipv6Addr::from([255; 16]);
    assert!(!row.matches(receipt));
}

#[test]
fn native_control_requires_prepaid_protocol_bound_before_any_syscall() {
    let empty: [u8; 0] = [];
    assert!(matches!(
        require_response(&empty),
        Err(Error::ResponseStorage {
            required: CONTROL_RESPONSE_CAPACITY
        })
    ));
    let buffer = vec![0; CONTROL_RESPONSE_CAPACITY];
    require_response(&buffer).unwrap();
    assert_eq!(CONTROL_RESPONSE_CAPACITY, u16::MAX as usize);
}

#[test]
fn actual_native_error_remains_the_original_deep_source() {
    let error = Error::Native(io::Error::from_raw_os_error(libc::EEXIST));
    let Error::Native(original) = &error else {
        unreachable!()
    };
    let pointer = original as *const io::Error;
    let source = error.source().unwrap().downcast_ref::<io::Error>().unwrap();
    assert_eq!(source as *const io::Error, pointer);
    assert_eq!(source.raw_os_error(), Some(libc::EEXIST));
}

#[test]
fn packet_and_plan_checks_preserve_whole_ipv6_and_original_64_semantics() {
    let mut packet = [0; 40];
    packet[0] = 0x60;
    validate_packet(&packet, 1280).unwrap();
    packet[5] = 1;
    assert!(validate_packet(&packet, 1280).is_err());
    assert!(validate_packet(&packet[..39], 1280).is_err());
    plan().validate().unwrap();
    assert!(Plan {
        prefix: 128,
        ..plan()
    }
    .validate()
    .is_err());
    assert!(Plan {
        server: "fd12:3456:789a:2::1".parse().unwrap(),
        ..plan()
    }
    .validate()
    .is_err());
}
