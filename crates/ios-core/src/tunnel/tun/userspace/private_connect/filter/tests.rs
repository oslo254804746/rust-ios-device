use super::super::{exclusive_socket, Flow};
use super::*;
use socket2::Domain;
use std::net::{Ipv4Addr, SocketAddr};

const TOTAL: usize = 1024 * 1024;
fn registry(bank: &Arc<Semaphore>) -> (PrivateConnector, u16) {
    let registry = PrivateConnector::prepare(1, 1, true, bank).unwrap();
    let loopback = exclusive_socket(Domain::IPV4).unwrap();
    loopback
        .bind(&SocketAddr::from((Ipv4Addr::LOCALHOST, 0)).into())
        .unwrap();
    let peer = loopback.local_addr().unwrap().as_socket().unwrap();
    let reservation = exclusive_socket(Domain::IPV6).unwrap();
    reservation.set_only_v6(true).unwrap();
    reservation
        .bind(&SocketAddr::from((Ipv6Addr::UNSPECIFIED, 0)).into())
        .unwrap();
    let port = reservation
        .local_addr()
        .unwrap()
        .as_socket()
        .unwrap()
        .port();
    registry
        .0
        .state
        .lock()
        .unwrap()
        .entries
        .as_mut()
        .unwrap()
        .values[0] = Some(Flow {
        peer,
        endpoint: "[fd00::1]:58783".parse().unwrap(),
        source_port: port,
        claimed: true,
        loopback,
        _kernel_port: Some(reservation),
    });
    (registry, port)
}
fn packet(bank: &Arc<Semaphore>, direction: Direction, next: u8, payload: &[u8]) -> OwnedBuffer {
    let mut packet = OwnedBuffer::zeroed(40 + payload.len(), Some(bank)).unwrap();
    packet[0] = 0x60;
    packet[4..6].copy_from_slice(&(payload.len() as u16).to_be_bytes());
    packet[6] = next;
    let client: Ipv6Addr = "fd00::2".parse().unwrap();
    let server: Ipv6Addr = "fd00::1".parse().unwrap();
    let (source, destination) = match direction {
        Direction::ToHost => (server, client),
        Direction::ToDevice => (client, server),
    };
    packet[8..24].copy_from_slice(&source.octets());
    packet[24..40].copy_from_slice(&destination.octets());
    packet[40..].copy_from_slice(payload);
    packet
}
fn tcp(direction: Direction, source_port: u16) -> Vec<u8> {
    let (source, destination) = match direction {
        Direction::ToHost => (58783, source_port),
        Direction::ToDevice => (source_port, 58783),
    };
    let mut bytes = vec![0; 32];
    bytes[..2].copy_from_slice(&source.to_be_bytes());
    bytes[2..4].copy_from_slice(&destination.to_be_bytes());
    bytes[12] = 0x50;
    bytes
}
fn fragment(
    bank: &Arc<Semaphore>,
    direction: Direction,
    id: u32,
    start: u16,
    more: bool,
    next: u8,
    bytes: &[u8],
) -> OwnedBuffer {
    let mut payload = vec![next, 0];
    payload.extend_from_slice(&(start | u16::from(more)).to_be_bytes());
    payload.extend_from_slice(&id.to_be_bytes());
    payload.extend_from_slice(bytes);
    packet(bank, direction, 44, &payload)
}

#[test]
fn exact_registry_flows_match_both_directions_through_checked_extension_headers() {
    let bank = Arc::new(Semaphore::new(TOTAL));
    let (registry, port) = registry(&bank);
    let mut filter = registry.filter("fd00::2".parse().unwrap(), &bank).unwrap();
    for direction in [Direction::ToHost, Direction::ToDevice] {
        let tcp = tcp(direction, port);
        let mut extensions = vec![60, 0, 0, 0, 0, 0, 0, 0, 6, 0, 0, 0, 0, 0, 0, 0];
        extensions.extend_from_slice(&tcp);
        assert!(matches!(
            filter
                .ingest(direction, packet(&bank, direction, 0, &extensions))
                .unwrap(),
            FilterOutput::Discard(DiscardReason::PrivateFlow)
        ));
        let unrelated = packet(
            &bank,
            direction,
            6,
            &self::tcp(direction, port.wrapping_add(1)),
        );
        let pointer = unrelated.as_ptr();
        let FilterOutput::Forward(unrelated) = filter.ingest(direction, unrelated).unwrap() else {
            panic!("unrelated TCP flow was discarded")
        };
        assert_eq!(unrelated.as_ptr(), pointer);
    }
    drop((filter, registry));
    assert_eq!(bank.available_permits(), TOTAL);
}

#[test]
fn out_of_order_private_fragments_keep_real_buffers_until_complete_tuple_is_known() {
    let bank = Arc::new(Semaphore::new(TOTAL));
    let (registry, port) = registry(&bank);
    let mut filter = registry.filter("fd00::2".parse().unwrap(), &bank).unwrap();
    let tcp = tcp(Direction::ToHost, port);
    let last = fragment(&bank, Direction::ToHost, 91, 24, false, 255, &tcp[24..]);
    let pointer = last.as_ptr();
    assert!(matches!(
        filter.ingest(Direction::ToHost, last).unwrap(),
        FilterOutput::Pending
    ));
    assert_eq!(filter.packets.values[0].value.as_ptr(), pointer);
    assert!(filter.pop_ready().is_none());
    assert!(matches!(
        filter
            .ingest(
                Direction::ToHost,
                fragment(&bank, Direction::ToHost, 91, 0, true, 6, &tcp[..24])
            )
            .unwrap(),
        FilterOutput::Discard(DiscardReason::PrivateFlow)
    ));
    assert!(filter.packets.values.is_empty());
    drop((filter, registry));
    assert_eq!(bank.available_permits(), TOTAL);
}

#[test]
fn unrelated_fragments_forward_the_same_complete_original_set_and_atomic_is_independent() {
    let bank = Arc::new(Semaphore::new(TOTAL));
    let (registry, port) = registry(&bank);
    let mut filter = registry.filter("fd00::2".parse().unwrap(), &bank).unwrap();
    let data = tcp(Direction::ToHost, port.wrapping_add(1));
    let last = fragment(&bank, Direction::ToHost, 92, 24, false, 59, &data[24..]);
    let first = fragment(&bank, Direction::ToHost, 92, 0, true, 6, &data[..24]);
    let pointers = [last.as_ptr(), first.as_ptr()];
    assert!(matches!(
        filter.ingest(Direction::ToHost, last).unwrap(),
        FilterOutput::Pending
    ));
    // Same-ID atomic packet does not consume or destroy the queued fragment.
    let atom = fragment(&bank, Direction::ToHost, 92, 0, false, 6, &data);
    assert!(matches!(
        filter.ingest(Direction::ToHost, atom).unwrap(),
        FilterOutput::Forward(_)
    ));
    assert_eq!(filter.packets.values.len(), 1);
    assert!(matches!(
        filter.ingest(Direction::ToHost, first).unwrap(),
        FilterOutput::Ready
    ));
    for pointer in pointers {
        let (direction, original) = filter.pop_ready().unwrap();
        assert_eq!(direction, Direction::ToHost);
        assert_eq!(original.as_ptr(), pointer);
    }
    assert!(filter.pop_ready().is_none());
    drop((filter, registry));
    assert_eq!(bank.available_permits(), TOTAL);
}

#[test]
fn overlap_discards_the_original_whole_group_instead_of_forwarding_unresolved_parts() {
    let bank = Arc::new(Semaphore::new(TOTAL));
    let (registry, port) = registry(&bank);
    let mut filter = registry.filter("fd00::2".parse().unwrap(), &bank).unwrap();
    let data = tcp(Direction::ToHost, port.wrapping_add(1));
    assert!(matches!(
        filter
            .ingest(
                Direction::ToHost,
                fragment(&bank, Direction::ToHost, 93, 0, true, 6, &data[..24])
            )
            .unwrap(),
        FilterOutput::Pending
    ));
    assert!(matches!(
        filter
            .ingest(
                Direction::ToHost,
                fragment(&bank, Direction::ToHost, 93, 16, false, 6, &data[16..])
            )
            .unwrap(),
        FilterOutput::Discard(DiscardReason::Overlap)
    ));
    assert!(filter.packets.values.is_empty());
    assert!(filter.pop_ready().is_none());
    drop((filter, registry));
    assert_eq!(bank.available_permits(), TOTAL);
}

#[tokio::test(start_paused = true)]
async fn fragment_lifetime_starts_at_first_arrival_and_is_never_refreshed() {
    let bank = Arc::new(Semaphore::new(TOTAL));
    let (registry, port) = registry(&bank);
    let mut filter = registry.filter("fd00::2".parse().unwrap(), &bank).unwrap();
    let data = tcp(Direction::ToHost, port);
    let origin = Instant::now();
    assert!(matches!(
        filter
            .ingest(
                Direction::ToHost,
                fragment(&bank, Direction::ToHost, 94, 24, true, 255, &data[24..])
            )
            .unwrap(),
        FilterOutput::Pending
    ));
    tokio::time::advance(Duration::from_secs(59)).await;
    assert!(matches!(
        filter
            .ingest(
                Direction::ToHost,
                fragment(&bank, Direction::ToHost, 94, 0, true, 6, &data[..24])
            )
            .unwrap(),
        FilterOutput::Pending
    ));
    assert_eq!(filter.next_expiry(), Some(origin + Duration::from_secs(60)));
    tokio::time::advance(Duration::from_secs(1)).await;
    assert_eq!(filter.expire(), 1);
    assert!(filter.packets.values.is_empty());
    drop((filter, registry));
    assert_eq!(bank.available_permits(), TOTAL);
}

#[test]
fn closed_zero_growth_refusal_returns_same_packet_and_retirement_waits_for_real_leases() {
    let bank = Arc::new(Semaphore::new(TOTAL));
    let (registry, port) = registry(&bank);
    let mut filter = registry.filter("fd00::2".parse().unwrap(), &bank).unwrap();
    let native_lease = registry.reserve_flows().unwrap();
    let data = tcp(Direction::ToHost, port);
    let fragment = fragment(&bank, Direction::ToHost, 95, 24, false, 6, &data[24..]);
    let pointer = fragment.as_ptr();
    let held = bank
        .clone()
        .try_acquire_many_owned(bank.available_permits() as u32)
        .unwrap();
    bank.close();
    let failed = filter.ingest(Direction::ToHost, fragment).unwrap_err();
    assert_eq!(
        crate::classify_error(&failed.error),
        crate::ErrorClass::ResourceLimit
    );
    assert_eq!(failed.packet.as_ptr(), pointer);
    assert!(filter.packets.values.is_empty());
    registry.close();
    drop(filter);
    assert!(registry.0.state.lock().unwrap().entries.is_some());
    let conflicting = exclusive_socket(Domain::IPV6).unwrap();
    conflicting.set_only_v6(true).unwrap();
    assert_eq!(
        conflicting
            .bind(&SocketAddr::from((Ipv6Addr::UNSPECIFIED, port)).into())
            .unwrap_err()
            .kind(),
        std::io::ErrorKind::AddrInUse
    );
    drop(native_lease);
    assert!(registry.0.state.lock().unwrap().entries.is_none());
    drop((failed, registry, held));
    assert_eq!(bank.available_permits(), TOTAL);
}

fn addresses(direction: Direction) -> (Ipv6Addr, Ipv6Addr) {
    let client = "fd00::2".parse().unwrap();
    let server = "fd00::1".parse().unwrap();
    match direction {
        Direction::ToHost => (server, client),
        Direction::ToDevice => (client, server),
    }
}

fn wire_packet(
    bank: &Arc<Semaphore>,
    direction: Direction,
    next: u8,
    payload: &[u8],
    source: Ipv6Addr,
    destination: Ipv6Addr,
) -> OwnedBuffer {
    let mut value = packet(bank, direction, next, payload);
    value[7] = 64;
    value[8..24].copy_from_slice(&source.octets());
    value[24..40].copy_from_slice(&destination.octets());
    value
}

fn tcp_sum(source: Ipv6Addr, destination: Ipv6Addr, bytes: &[u8]) -> u16 {
    let mut sum = 6u32 + bytes.len() as u32;
    for address in [source, destination] {
        for word in address.octets().chunks_exact(2) {
            sum += u32::from(u16::from_be_bytes([word[0], word[1]]));
        }
    }
    for word in bytes.chunks(2) {
        sum += u32::from(u16::from_be_bytes([word[0], *word.get(1).unwrap_or(&0)]));
    }
    while sum >> 16 != 0 {
        sum = (sum & 0xffff) + (sum >> 16);
    }
    !(sum as u16)
}

fn wire_tcp(direction: Direction, port: u16, source: Ipv6Addr, destination: Ipv6Addr) -> Vec<u8> {
    let mut bytes = tcp(direction, port);
    bytes[4..8].copy_from_slice(&1u32.to_be_bytes());
    bytes[8..12].copy_from_slice(&2u32.to_be_bytes());
    bytes[13] = 0x10;
    bytes[14..16].copy_from_slice(&4096u16.to_be_bytes());
    let checksum = tcp_sum(source, destination, &bytes);
    bytes[16..18].copy_from_slice(&checksum.to_be_bytes());
    assert_eq!(tcp_sum(source, destination, &bytes), 0);
    bytes
}

fn home(next: u8, address: Ipv6Addr) -> Vec<u8> {
    // The actual option begins at 8n+6 within this Destination header.
    let mut bytes = vec![next, 2, 1, 2, 0, 0, 0xc9, 16];
    bytes.extend_from_slice(&address.octets());
    bytes
}

fn rh2(next: u8, segments_left: u8, address: Ipv6Addr) -> Vec<u8> {
    let mut bytes = vec![next, 2, 2, segments_left, 0, 0, 0, 0];
    bytes.extend_from_slice(&address.octets());
    bytes
}

fn srh(next: u8, segments_left: u8, segments: &[Ipv6Addr]) -> Vec<u8> {
    assert!(!segments.is_empty());
    let length = 8 + segments.len() * 16;
    let mut bytes = vec![
        next,
        (length / 8 - 1) as u8,
        4,
        segments_left,
        (segments.len() - 1) as u8,
        0,
        0,
        0,
    ];
    for address in segments {
        bytes.extend_from_slice(&address.octets());
    }
    bytes
}

fn ah(next: u8, length: usize) -> Vec<u8> {
    let mut bytes = vec![0; length];
    bytes[0] = next;
    bytes[1] = (length / 4 - 2) as u8;
    bytes[4..8].copy_from_slice(&1u32.to_be_bytes());
    if length >= 12 {
        bytes[8..12].copy_from_slice(&1u32.to_be_bytes());
    }
    bytes
}

fn whole(direction: Direction, packet: &[u8]) -> Canonical {
    let Parsed::Whole(canonical) = parse(direction, packet) else {
        panic!("expected structurally complete plaintext header chain")
    };
    canonical
}

fn assert_forward(filter: &mut PrivateFilter, direction: Direction, packet: OwnedBuffer) {
    let pointer = packet.as_ptr();
    let bytes = packet.to_vec();
    let FilterOutput::Forward(original) = filter.ingest(direction, packet).unwrap() else {
        panic!("valid unrelated packet was excluded")
    };
    assert_eq!(original.as_ptr(), pointer);
    assert_eq!(&original[..], bytes.as_slice());
}

#[test]
fn home_address_uses_canonical_source_in_both_directions_without_rewriting_public_owner() {
    let bank = Arc::new(Semaphore::new(TOTAL));
    let (registry, port) = registry(&bank);
    let mut filter = registry.filter("fd00::2".parse().unwrap(), &bank).unwrap();
    let care_of = "fd00::9".parse().unwrap();
    let unrelated = "fd00::8".parse().unwrap();
    for direction in [Direction::ToHost, Direction::ToDevice] {
        let (source, destination) = addresses(direction);
        let mut payload = home(6, source);
        payload.extend_from_slice(&wire_tcp(direction, port, source, destination));
        let private = wire_packet(&bank, direction, 60, &payload, care_of, destination);
        assert_eq!(whole(direction, &private).source, source);
        // Native acceptance additionally needs the actual Mobile IPv6 binding;
        // this verifies canonical filtering, not a fabricated native delivery.
        assert!(matches!(
            filter.ingest(direction, private).unwrap(),
            FilterOutput::Discard(DiscardReason::PrivateFlow)
        ));

        let mut payload = home(6, unrelated);
        payload.extend_from_slice(&wire_tcp(direction, port, unrelated, destination));
        let public = wire_packet(&bank, direction, 60, &payload, source, destination);
        assert_eq!(whole(direction, &public).source, unrelated);
        assert_forward(&mut filter, direction, public);
    }
    drop((filter, registry));
    assert_eq!(bank.available_permits(), TOTAL);
}

#[test]
fn routing_active_endpoint_and_zero_segments_are_distinct_in_both_directions() {
    let bank = Arc::new(Semaphore::new(TOTAL));
    let (registry, port) = registry(&bank);
    let mut filter = registry.filter("fd00::2".parse().unwrap(), &bank).unwrap();
    let care_of = "fd00::9".parse().unwrap();
    let unrelated = "fd00::8".parse().unwrap();
    let middle = "fd00::7".parse().unwrap();
    for direction in [Direction::ToHost, Direction::ToDevice] {
        let (source, destination) = addresses(direction);
        for mut header in [
            rh2(6, 1, destination),
            srh(6, 1, &[destination, middle]),
            srh(6, 2, &[destination, middle]),
        ] {
            header.extend_from_slice(&wire_tcp(direction, port, source, destination));
            let private = wire_packet(&bank, direction, 43, &header, source, care_of);
            assert_eq!(whole(direction, &private).destination, destination);
            assert!(matches!(
                filter.ingest(direction, private).unwrap(),
                FilterOutput::Discard(DiscardReason::PrivateFlow)
            ));
        }
        for mut header in [rh2(6, 1, unrelated), srh(6, 1, &[unrelated, middle])] {
            header.extend_from_slice(&wire_tcp(direction, port, source, unrelated));
            let public = wire_packet(&bank, direction, 43, &header, source, destination);
            assert_eq!(whole(direction, &public).destination, unrelated);
            assert_forward(&mut filter, direction, public);
        }
        for mut header in [
            rh2(6, 0, destination),
            srh(6, 0, &[destination, middle]),
            vec![6, 0, 253, 0, 0, 0, 0, 0],
        ] {
            header.extend_from_slice(&wire_tcp(direction, port, source, unrelated));
            let public = wire_packet(&bank, direction, 43, &header, source, unrelated);
            assert_eq!(whole(direction, &public).destination, unrelated);
            assert_forward(&mut filter, direction, public);
        }
        let mut header = srh(6, 0, &[unrelated, middle]);
        header.extend_from_slice(&wire_tcp(direction, port, source, destination));
        assert!(matches!(
            filter
                .ingest(
                    direction,
                    wire_packet(&bank, direction, 43, &header, source, destination)
                )
                .unwrap(),
            FilterOutput::Discard(DiscardReason::PrivateFlow)
        ));
    }
    let route = srh(6, 2, &[unrelated, middle]);
    assert_eq!(
        routing(&route),
        Some(RoutingProjection::Forward {
            next: middle,
            final_destination: unrelated
        })
    );
    drop((filter, registry));
    assert_eq!(bank.available_permits(), TOTAL);
}

#[test]
fn ordered_routing_then_home_then_ah_uses_the_supported_final_plaintext_identity() {
    let bank = Arc::new(Semaphore::new(TOTAL));
    let (registry, port) = registry(&bank);
    let mut filter = registry.filter("fd00::2".parse().unwrap(), &bank).unwrap();
    let (source, destination) = addresses(Direction::ToHost);
    let middle = "fd00::7".parse().unwrap();
    let care_of = "fd00::9".parse().unwrap();
    let mut payload = rh2(43, 1, middle);
    payload.extend_from_slice(&srh(60, 1, &[destination]));
    payload.extend_from_slice(&home(51, source));
    payload.extend_from_slice(&ah(6, 24));
    payload.extend_from_slice(&wire_tcp(Direction::ToHost, port, source, destination));
    let private = wire_packet(&bank, Direction::ToHost, 43, &payload, care_of, care_of);
    let identity = whole(Direction::ToHost, &private);
    assert_eq!(identity.source, source);
    assert_eq!(identity.destination, destination);
    assert!(matches!(
        filter.ingest(Direction::ToHost, private).unwrap(),
        FilterOutput::Discard(DiscardReason::PrivateFlow)
    ));
    drop((filter, registry));
    assert_eq!(bank.available_permits(), TOTAL);
}

#[test]
fn offset_zero_canonical_endpoint_is_independent_of_its_fragment_association_key() {
    let bank = Arc::new(Semaphore::new(TOTAL));
    let (registry, port) = registry(&bank);
    let mut filter = registry.filter("fd00::2".parse().unwrap(), &bank).unwrap();
    let (source, client) = addresses(Direction::ToHost);
    let unrelated = "fd00::8".parse().unwrap();
    let middle = "fd00::7".parse().unwrap();
    for (id, canonical_destination, key_destination, private) in [
        (501, client, unrelated, true),
        (502, unrelated, client, false),
    ] {
        let mut data = srh(6, 1, &[canonical_destination, middle]);
        data.extend_from_slice(&wire_tcp(
            Direction::ToHost,
            port,
            source,
            canonical_destination,
        ));
        let mut last = fragment(&bank, Direction::ToHost, id, 64, false, 255, &data[64..]);
        last[24..40].copy_from_slice(&key_destination.octets());
        last[7] = 64;
        let last_pointer = last.as_ptr();
        assert!(matches!(
            filter.ingest(Direction::ToHost, last).unwrap(),
            FilterOutput::Pending
        ));
        assert_eq!(
            filter.packets.values.last().unwrap().value.as_ptr(),
            last_pointer
        );
        let mut first = fragment(&bank, Direction::ToHost, id, 0, true, 43, &data[..64]);
        first[24..40].copy_from_slice(&key_destination.octets());
        first[7] = 64;
        let Parsed::Fragment(parsed) = parse(Direction::ToHost, &first) else {
            panic!("expected first fragment")
        };
        assert_eq!(parsed.key.destination, key_destination);
        assert_eq!(parsed.first.unwrap().0.destination, canonical_destination);
        let first_pointer = first.as_ptr();
        let result = filter.ingest(Direction::ToHost, first).unwrap();
        if private {
            assert!(matches!(
                result,
                FilterOutput::Discard(DiscardReason::PrivateFlow)
            ));
            assert!(filter.pop_ready().is_none());
        } else {
            assert!(matches!(result, FilterOutput::Ready));
            for pointer in [last_pointer, first_pointer] {
                let (direction, original) = filter.pop_ready().unwrap();
                assert_eq!(direction, Direction::ToHost);
                assert_eq!(original.as_ptr(), pointer);
            }
            assert!(filter.pop_ready().is_none());
        }
    }
    drop((filter, registry));
    assert_eq!(bank.available_permits(), TOTAL);
}

#[test]
fn pre_fragment_home_processing_associates_with_canonical_source_before_offset_zero() {
    let bank = Arc::new(Semaphore::new(TOTAL));
    let (registry, port) = registry(&bank);
    let mut filter = registry.filter("fd00::2".parse().unwrap(), &bank).unwrap();
    let (source, destination) = addresses(Direction::ToHost);
    let data = wire_tcp(Direction::ToHost, port, source, destination);
    let last_part = fragment(&bank, Direction::ToHost, 503, 24, false, 255, &data[24..]);
    let mut payload = home(44, source);
    payload.extend_from_slice(&last_part[40..]);
    drop(last_part);
    let last = wire_packet(
        &bank,
        Direction::ToHost,
        60,
        &payload,
        "fd00::9".parse().unwrap(),
        destination,
    );
    assert!(matches!(
        filter.ingest(Direction::ToHost, last).unwrap(),
        FilterOutput::Pending
    ));
    assert_eq!(filter.datagrams.values[0].key.source, source);
    // Per-fragment header content can differ; the plain offset-zero source
    // already equals the home identity used when the tail reached reassembly.
    let first = fragment(&bank, Direction::ToHost, 503, 0, true, 6, &data[..24]);
    assert!(matches!(
        filter.ingest(Direction::ToHost, first).unwrap(),
        FilterOutput::Discard(DiscardReason::PrivateFlow)
    ));
    assert!(filter.packets.values.is_empty());
    drop((filter, registry));
    assert_eq!(bank.available_permits(), TOTAL);
}

#[test]
fn malformed_home_options_and_invalid_position_never_become_public_tcp() {
    let bank = Arc::new(Semaphore::new(TOTAL));
    let (registry, port) = registry(&bank);
    let mut filter = registry.filter("fd00::2".parse().unwrap(), &bank).unwrap();
    let (source, destination) = addresses(Direction::ToHost);
    let tcp = wire_tcp(Direction::ToHost, port, source, destination);
    let mut cases = Vec::new();
    for length in [15, 17] {
        let mut header = home(6, source);
        header[7] = length;
        header.extend_from_slice(&tcp);
        cases.push((60, header));
    }
    let mut misaligned = vec![6, 2, 0xc9, 16];
    misaligned.extend_from_slice(&source.octets());
    misaligned.extend_from_slice(&[0; 4]);
    misaligned.extend_from_slice(&tcp);
    cases.push((60, misaligned));
    for second in [source, "fd00::8".parse().unwrap()] {
        let mut duplicate = home(60, source);
        duplicate.extend_from_slice(&home(6, second));
        duplicate.extend_from_slice(&tcp);
        cases.push((60, duplicate));
    }
    let mut after_ah = ah(60, 24);
    after_ah.extend_from_slice(&home(6, source));
    after_ah.extend_from_slice(&tcp);
    cases.push((51, after_ah));
    let mut before_route = home(43, source);
    before_route.extend_from_slice(&rh2(6, 1, destination));
    before_route.extend_from_slice(&tcp);
    cases.push((60, before_route));
    let mut after_atomic = vec![60, 0, 0, 0, 0, 0, 0, 1];
    after_atomic.extend_from_slice(&home(6, source));
    after_atomic.extend_from_slice(&tcp);
    cases.push((44, after_atomic));
    let mut wrong_kind = home(6, source);
    wrong_kind.extend_from_slice(&tcp);
    cases.push((0, wrong_kind));
    for (next, bytes) in cases {
        let malformed = wire_packet(&bank, Direction::ToHost, next, &bytes, source, destination);
        assert!(matches!(
            parse(Direction::ToHost, &malformed),
            Parsed::Malformed
        ));
        assert!(matches!(
            filter.ingest(Direction::ToHost, malformed).unwrap(),
            FilterOutput::Discard(DiscardReason::Malformed)
        ));
    }
    // Unknown skip-action option with checked extent is preserved, as is PadN.
    let mut benign = vec![6, 0, 0x1e, 2, 3, 4, 1, 0];
    let public_source = "fd00::8".parse().unwrap();
    benign.extend_from_slice(&wire_tcp(
        Direction::ToHost,
        port,
        public_source,
        destination,
    ));
    assert_forward(
        &mut filter,
        Direction::ToHost,
        wire_packet(
            &bank,
            Direction::ToHost,
            60,
            &benign,
            public_source,
            destination,
        ),
    );
    drop((filter, registry));
    assert_eq!(bank.available_permits(), TOTAL);
}

#[test]
fn srh_bounds_and_ah_fixed_header_alignment_are_checked_before_tcp() {
    let bank = Arc::new(Semaphore::new(TOTAL));
    let (registry, port) = registry(&bank);
    let mut filter = registry.filter("fd00::2".parse().unwrap(), &bank).unwrap();
    let (source, destination) = addresses(Direction::ToHost);
    let middle = "fd00::7".parse().unwrap();
    let tcp = wire_tcp(Direction::ToHost, port, source, destination);
    let mut cases = Vec::new();
    let mut last_entry = srh(6, 1, &[destination, middle]);
    last_entry[4] = 2;
    cases.push((43, last_entry));
    cases.push((43, srh(6, 3, &[destination, middle])));
    let mut tlv = srh(6, 1, &[destination]);
    tlv[1] += 1;
    tlv.extend_from_slice(&[5, 20, 0, 0, 0, 0, 0, 0]);
    cases.push((43, tlv));
    for length in [8, 12] {
        cases.push((51, ah(6, length)));
    }
    for (next, mut header) in cases {
        header.extend_from_slice(&tcp);
        assert!(matches!(
            filter
                .ingest(
                    Direction::ToHost,
                    wire_packet(&bank, Direction::ToHost, next, &header, source, destination)
                )
                .unwrap(),
            FilterOutput::Discard(DiscardReason::Malformed)
        ));
    }
    for direction in [Direction::ToHost, Direction::ToDevice] {
        let (source, destination) = addresses(direction);
        let mut header = ah(6, 24);
        header.extend_from_slice(&wire_tcp(direction, port, source, destination));
        assert!(matches!(
            filter
                .ingest(
                    direction,
                    wire_packet(&bank, direction, 51, &header, source, destination)
                )
                .unwrap(),
            FilterOutput::Discard(DiscardReason::PrivateFlow)
        ));
        let public_source = "fd00::8".parse().unwrap();
        let mut header = ah(6, 24);
        header.extend_from_slice(&wire_tcp(direction, port, public_source, destination));
        assert_forward(
            &mut filter,
            direction,
            wire_packet(&bank, direction, 51, &header, public_source, destination),
        );
    }
    drop((filter, registry));
    assert_eq!(bank.available_permits(), TOTAL);
}

#[test]
fn repeated_atomics_and_atomic_before_ordinary_keep_original_association_independent() {
    let bank = Arc::new(Semaphore::new(TOTAL));
    let (registry, port) = registry(&bank);
    let mut filter = registry.filter("fd00::2".parse().unwrap(), &bank).unwrap();
    let (source, client) = addresses(Direction::ToHost);
    let unrelated = "fd00::8".parse().unwrap();
    for (id, destination, private) in [(504, client, true), (505, unrelated, false)] {
        let data = wire_tcp(Direction::ToHost, port, source, destination);
        let mut repeated = vec![44, 0, 0, 0, 0, 0, 0, 11, 6, 0, 0, 0, 0, 0, 0, 12];
        repeated.extend_from_slice(&data);
        let atom = wire_packet(&bank, Direction::ToHost, 44, &repeated, source, destination);
        if private {
            assert!(matches!(
                filter.ingest(Direction::ToHost, atom).unwrap(),
                FilterOutput::Discard(DiscardReason::PrivateFlow)
            ));
        } else {
            assert_forward(&mut filter, Direction::ToHost, atom);
        }

        let mut pointers = Vec::new();
        for (start, more, part) in [(24, false, &data[24..]), (0, true, &data[..24])] {
            let inner = fragment(&bank, Direction::ToHost, id, start, more, 6, part);
            let mut bytes = vec![44, 0, 0, 0, 0, 0, 0, 13];
            bytes.extend_from_slice(&inner[40..]);
            drop(inner);
            let value = wire_packet(&bank, Direction::ToHost, 44, &bytes, source, destination);
            pointers.push(value.as_ptr());
            if start == 0 {
                let Parsed::Fragment(parsed) = parse(Direction::ToHost, &value) else {
                    panic!("ordinary fragment")
                };
                assert_eq!(parsed.first.unwrap().1, 0);
            }
            let result = filter.ingest(Direction::ToHost, value).unwrap();
            if start != 0 {
                assert!(matches!(result, FilterOutput::Pending));
            } else if private {
                assert!(matches!(
                    result,
                    FilterOutput::Discard(DiscardReason::PrivateFlow)
                ));
            } else {
                assert!(matches!(result, FilterOutput::Ready));
            }
        }
        if !private {
            for pointer in pointers {
                assert_eq!(filter.pop_ready().unwrap().1.as_ptr(), pointer);
            }
        }
        assert!(filter.pop_ready().is_none());
    }
    drop((filter, registry));
    assert_eq!(bank.available_permits(), TOTAL);
}

#[test]
fn canonical_first_fragment_growth_failure_retains_both_actual_owners_and_prior_state() {
    let bank = Arc::new(Semaphore::new(TOTAL));
    let (registry, port) = registry(&bank);
    let mut filter = registry.filter("fd00::2".parse().unwrap(), &bank).unwrap();
    let (source, destination) = addresses(Direction::ToHost);
    let mut data = srh(6, 1, &[destination]);
    data.extend_from_slice(&wire_tcp(Direction::ToHost, port, source, destination));
    let last = fragment(&bank, Direction::ToHost, 506, 48, false, 255, &data[48..]);
    let last_pointer = last.as_ptr();
    assert!(matches!(
        filter.ingest(Direction::ToHost, last).unwrap(),
        FilterOutput::Pending
    ));
    let first = fragment(&bank, Direction::ToHost, 506, 0, true, 43, &data[..48]);
    let first_pointer = first.as_ptr();
    let held = bank
        .clone()
        .try_acquire_many_owned(bank.available_permits() as u32)
        .unwrap();
    bank.close();
    let failed = filter.ingest(Direction::ToHost, first).unwrap_err();
    assert_eq!(
        crate::classify_error(&failed.error),
        crate::ErrorClass::ResourceLimit
    );
    assert_eq!(failed.packet.as_ptr(), first_pointer);
    assert_eq!(filter.packets.values[0].value.as_ptr(), last_pointer);
    assert!(filter.datagrams.values[0].first.is_none());
    assert!(filter.pop_ready().is_none());
    drop((failed, filter, registry, held));
    assert_eq!(bank.available_permits(), TOTAL);
}

#[test]
fn opaque_native_transformation_boundary_is_explicit_and_remains_unaccepted() {
    let bank = Arc::new(Semaphore::new(TOTAL));
    let (registry, port) = registry(&bank);
    let mut filter = registry.filter("fd00::2".parse().unwrap(), &bank).unwrap();
    let (source, destination) = addresses(Direction::ToHost);
    let data = wire_tcp(Direction::ToHost, port, source, destination);
    let inner = wire_packet(&bank, Direction::ToHost, 6, &data, source, destination);
    assert!(matches!(
        whole(Direction::ToHost, &inner).transport,
        Transport::Tcp(_)
    ));
    let outer_source = "fd00::7".parse().unwrap();
    let outer_destination = "fd00::8".parse().unwrap();
    let outer = wire_packet(
        &bank,
        Direction::ToHost,
        41,
        &inner,
        outer_source,
        outer_destination,
    );
    // Inner private TCP tuple encapsulated inside IP-in-IP (Next Header 41) is
    // inspected and isolated; it must NOT be forwarded.
    assert!(matches!(
        whole(Direction::ToHost, &outer).transport,
        Transport::Tcp(_)
    ));
    assert!(matches!(
        filter.ingest(Direction::ToHost, outer).unwrap(),
        FilterOutput::Discard(DiscardReason::PrivateFlow)
    ));
    drop(inner);

    // Unrelated IP-in-IP is forwarded intact with original owner preserved:
    let (unrelated_src, unrelated_dst) = (
        "fd00::9".parse::<Ipv6Addr>().unwrap(),
        "fd00::10".parse::<Ipv6Addr>().unwrap(),
    );
    let unrelated_data = wire_tcp(Direction::ToHost, port, unrelated_src, unrelated_dst);
    let unrelated_inner = wire_packet(
        &bank,
        Direction::ToHost,
        6,
        &unrelated_data,
        unrelated_src,
        unrelated_dst,
    );
    let outer_unrelated = wire_packet(
        &bank,
        Direction::ToHost,
        41,
        &unrelated_inner,
        outer_source,
        outer_destination,
    );
    assert_forward(&mut filter, Direction::ToHost, outer_unrelated);
    drop(unrelated_inner);

    for protocol in [50, 253] {
        let value = wire_packet(
            &bank,
            Direction::ToHost,
            protocol,
            &[0; 32],
            source,
            destination,
        );
        assert_eq!(
            whole(Direction::ToHost, &value).transport,
            Transport::Opaque(protocol)
        );
        assert_forward(&mut filter, Direction::ToHost, value);
    }
    let mut unknown_route = vec![6, 0, 253, 1, 0, 0, 0, 0];
    unknown_route.extend_from_slice(&data);
    let value = wire_packet(
        &bank,
        Direction::ToHost,
        43,
        &unknown_route,
        outer_source,
        destination,
    );
    assert_eq!(whole(Direction::ToHost, &value).opaque, Some(43));
    assert!(matches!(
        whole(Direction::ToHost, &value).transport,
        Transport::Tcp(_)
    ));
    assert_forward(&mut filter, Direction::ToHost, value);
    // The previously excluded visible raw private tuple remains excluded; an
    // unknown active route does not turn that conservative comparison off.
    let value = wire_packet(
        &bank,
        Direction::ToHost,
        43,
        &unknown_route,
        source,
        destination,
    );
    assert!(matches!(
        filter.ingest(Direction::ToHost, value).unwrap(),
        FilterOutput::Discard(DiscardReason::PrivateFlow)
    ));
    drop((filter, registry));
    assert_eq!(bank.available_permits(), TOTAL);
}
