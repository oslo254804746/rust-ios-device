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
