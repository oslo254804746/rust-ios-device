//! Exact private TCP flows and creation-owned IPv6 fragment association.
//!
//! RFC 8200 section 4.5: source/destination/ID associate fragments; headers
//! before Fragment and non-first Next Header may differ. Only offset zero
//! supplies the canonical upper-layer identity. The plaintext HAO/RH2/SRH/AH
//! interpretation below does not establish isolation after ESP, IP-in-IP,
//! unknown routing/option semantics, or other native transformations.
use super::super::owned_buffer::OwnedBuffer;
use super::{FlowLease, PrivateConnector};
use crate::XpcError;
use std::{mem, net::Ipv6Addr, sync::Arc, time::Duration};
use tokio::{
    sync::{OwnedSemaphorePermit, Semaphore},
    time::Instant,
};

// The protocol's actual reassembly lifetime, not a caller operation budget.
const REASSEMBLY_LIFETIME: Duration = Duration::from_secs(60);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::tunnel::tun::userspace) enum Direction {
    ToHost,
    ToDevice,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::tunnel::tun::userspace) enum DiscardReason {
    PrivateFlow,
    Malformed,
    Overlap,
    CompletedDatagram,
}

#[derive(Debug)]
pub(in crate::tunnel::tun::userspace) enum FilterOutput {
    Forward(OwnedBuffer),
    Pending,
    Ready,
    Discard(DiscardReason),
}

#[derive(Debug)]
pub(in crate::tunnel::tun::userspace) struct FilterFailure {
    pub(in crate::tunnel::tun::userspace) error: XpcError,
    pub(in crate::tunnel::tun::userspace) packet: OwnedBuffer,
}

#[derive(Clone, Copy, PartialEq, Eq)]
struct Key {
    direction: Direction,
    source: Ipv6Addr,
    destination: Ipv6Addr,
    id: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct TcpTuple {
    source: u16,
    destination: u16,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Transport {
    Tcp(TcpTuple),
    NonTcp,
    // The original owner remains intact, but forwarding this protocol is NOT
    // evidence of private isolation after native decryption/decapsulation.
    Opaque(u8),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Canonical {
    source: Ipv6Addr,
    destination: Ipv6Addr,
    transport: Transport,
    opaque: Option<u8>,
}

#[derive(Clone, Copy)]
struct Projection {
    source: Ipv6Addr,
    destination: Ipv6Addr,
    home: bool,
    fragment: bool,
    authentication: bool,
    opaque: Option<u8>,
}
impl Projection {
    fn canonical(self, transport: Transport) -> Canonical {
        Canonical {
            source: self.source,
            destination: self.destination,
            transport,
            opaque: self.opaque,
        }
    }
}

#[derive(Clone, Copy)]
struct Fragment {
    key: Key,
    start: usize,
    end: usize,
    more: bool,
    first: Option<(Canonical, usize)>,
}

enum Parsed {
    Whole(Canonical),
    Fragment(Fragment),
    Malformed,
}

enum Status {
    Assembling,
    Complete,
    Invalid(DiscardReason),
}
struct Datagram {
    key: Key,
    expires: Instant,
    total: Option<usize>,
    first: Option<(Canonical, usize)>,
    status: Status,
}
struct Packet {
    value: OwnedBuffer,
    fragment: Fragment,
    ready: bool,
}

/// Both old and replacement actual arrays remain paid during growth. There is
/// no entry ceiling independent of the real original shared byte bank.
struct PaidVec<T> {
    values: Vec<T>,
    storage: Option<OwnedSemaphorePermit>,
    bank: Arc<Semaphore>,
}
struct Growth<T> {
    values: Vec<T>,
    storage: OwnedSemaphorePermit,
}
impl<T> PaidVec<T> {
    fn new(bank: Arc<Semaphore>) -> Self {
        Self {
            values: Vec::new(),
            storage: None,
            bank,
        }
    }
    fn growth(&self) -> Result<Option<Growth<T>>, XpcError> {
        if self.values.len() < self.values.capacity() {
            return Ok(None);
        }
        let capacity = self
            .values
            .capacity()
            .checked_mul(2)
            .map(|n| n.max(1))
            .ok_or_else(|| {
                XpcError::Io(std::io::Error::other(
                    super::super::owned_buffer::BufferAdmission::SizeOverflow,
                ))
            })?;
        let (storage, values) = crate::device::prepared_rsd::exact_vec(&self.bank, capacity)?;
        Ok(Some(Growth { values, storage }))
    }
    fn install(&mut self, growth: Option<Growth<T>>) {
        if let Some(Growth {
            mut values,
            storage,
        }) = growth
        {
            values.append(&mut self.values);
            drop(mem::replace(&mut self.values, values));
            self.storage = Some(storage);
        }
    }
    fn push(&mut self, value: T) {
        assert!(self.values.len() < self.values.capacity());
        self.values.push(value);
    }
}

/// Inline in the actual banked bridge future. It retains the same descriptor
/// registry, paid key/packet arrays, and every original queued packet owner.
pub(in crate::tunnel::tun::userspace) struct PrivateFilter {
    connector: PrivateConnector,
    client: Ipv6Addr,
    datagrams: PaidVec<Datagram>,
    packets: PaidVec<Packet>,
    // Declared last: pending packet/backing destruction precedes descriptor
    // retirement when this is the final actual filter/native lease.
    _lease: FlowLease,
}
impl PrivateFilter {
    pub(super) fn new(
        connector: PrivateConnector,
        lease: FlowLease,
        client: Ipv6Addr,
        bank: Arc<Semaphore>,
    ) -> Self {
        Self {
            connector,
            client,
            datagrams: PaidVec::new(bank.clone()),
            packets: PaidVec::new(bank),
            _lease: lease,
        }
    }

    pub(in crate::tunnel::tun::userspace) fn next_expiry(&self) -> Option<Instant> {
        self.datagrams.values.iter().map(|row| row.expires).min()
    }

    /// Native event loops select this absolute time alongside their existing
    /// deadline. Further fragments never refresh the first arrival's expiry.
    pub(in crate::tunnel::tun::userspace) fn expire(&mut self) -> usize {
        let now = Instant::now();
        let mut expired = 0;
        for index in (0..self.datagrams.values.len()).rev() {
            if self.datagrams.values[index].expires <= now {
                let key = self.datagrams.values.remove(index).key;
                self.drop_pending(key);
                expired += 1;
            }
        }
        expired
    }

    pub(in crate::tunnel::tun::userspace) fn pop_ready(
        &mut self,
    ) -> Option<(Direction, OwnedBuffer)> {
        let index = self.packets.values.iter().position(|packet| packet.ready)?;
        let packet = self.packets.values.remove(index);
        Some((packet.fragment.key.direction, packet.value))
    }

    pub(in crate::tunnel::tun::userspace) fn ingest(
        &mut self,
        direction: Direction,
        packet: OwnedBuffer,
    ) -> Result<FilterOutput, FilterFailure> {
        let arrived = Instant::now();
        self.expire();
        let fragment = match parse(direction, &packet) {
            Parsed::Malformed => return Ok(FilterOutput::Discard(DiscardReason::Malformed)),
            Parsed::Whole(canonical) => {
                return if self.private(direction, canonical) {
                    Ok(FilterOutput::Discard(DiscardReason::PrivateFlow))
                } else {
                    Ok(FilterOutput::Forward(packet))
                };
            }
            Parsed::Fragment(fragment) => fragment,
        };
        let key = fragment.key;
        let existing = self.datagrams.values.iter().position(|row| row.key == key);
        if let Some(index) = existing {
            match self.datagrams.values[index].status {
                Status::Complete => {
                    return Ok(FilterOutput::Discard(DiscardReason::CompletedDatagram))
                }
                Status::Invalid(reason) => return Ok(FilterOutput::Discard(reason)),
                Status::Assembling => {}
            }
            let overlap = self
                .packets
                .values
                .iter()
                .filter(|old| !old.ready && old.fragment.key == key)
                .any(|old| fragment.start < old.fragment.end && old.fragment.start < fragment.end);
            if overlap {
                self.datagrams.values[index].status = Status::Invalid(DiscardReason::Overlap);
                self.drop_pending(key);
                return Ok(FilterOutput::Discard(DiscardReason::Overlap));
            }
        }
        // Stage both actual empty replacements before mutating either owner.
        // Failure returns the SAME incoming packet and rolls back these grants.
        let key_growth = if existing.is_none() {
            match self.datagrams.growth() {
                Ok(growth) => growth,
                Err(error) => return Err(FilterFailure { error, packet }),
            }
        } else {
            None
        };
        let packet_growth = match self.packets.growth() {
            Ok(growth) => growth,
            Err(error) => return Err(FilterFailure { error, packet }),
        };
        let expires = if existing.is_none() {
            match arrived.checked_add(REASSEMBLY_LIFETIME) {
                Some(expires) => Some(expires),
                None => {
                    return Err(FilterFailure {
                        error: XpcError::Io(std::io::Error::other(
                            super::super::owned_buffer::BufferAdmission::SizeOverflow,
                        )),
                        packet,
                    })
                }
            }
        } else {
            None
        };
        self.datagrams.install(key_growth);
        self.packets.install(packet_growth);
        let index = existing.unwrap_or_else(|| {
            self.datagrams.push(Datagram {
                key,
                expires: expires.expect("first arrival expiry"),
                total: None,
                first: None,
                status: Status::Assembling,
            });
            self.datagrams.values.len() - 1
        });
        let row = &mut self.datagrams.values[index];
        if let Some(first) = fragment.first {
            row.first = Some(first);
        }
        let inconsistent_end =
            !fragment.more && row.total.is_some_and(|total| total != fragment.end);
        if !fragment.more {
            row.total = Some(fragment.end);
        }
        let excessive =
            row.total.is_some_and(|total| {
                fragment.end > total
                    || self.packets.values.iter().any(|old| {
                        !old.ready && old.fragment.key == key && old.fragment.end > total
                    })
                    || row.first.is_some_and(|(_, prefix)| {
                        total
                            .checked_add(prefix)
                            .map_or(true, |bytes| bytes > u16::MAX as usize)
                    })
            });
        if inconsistent_end || excessive {
            row.status = Status::Invalid(DiscardReason::Malformed);
            self.drop_pending(key);
            return Ok(FilterOutput::Discard(DiscardReason::Malformed));
        }
        self.packets.push(Packet {
            value: packet,
            fragment,
            ready: false,
        });
        let complete = if let Some(total) = self.datagrams.values[index].total {
            let mut cursor = 0;
            while cursor < total {
                let next = self.packets.values.iter().find(|part| {
                    !part.ready
                        && part.fragment.key == key
                        && part.fragment.start == cursor
                        && part.fragment.end > cursor
                });
                let Some(next) = next else { break };
                cursor = next.fragment.end;
            }
            cursor == total && self.datagrams.values[index].first.is_some()
        } else {
            false
        };
        if !complete {
            return Ok(FilterOutput::Pending);
        }
        let (canonical, _) = self.datagrams.values[index]
            .first
            .expect("complete offset zero header");
        self.datagrams.values[index].status = Status::Complete;
        if self.private(direction, canonical) {
            self.drop_pending(key);
            Ok(FilterOutput::Discard(DiscardReason::PrivateFlow))
        } else {
            for packet in &mut self.packets.values {
                if !packet.ready && packet.fragment.key == key {
                    packet.ready = true;
                }
            }
            Ok(FilterOutput::Ready)
        }
    }

    fn drop_pending(&mut self, key: Key) {
        for index in (0..self.packets.values.len()).rev() {
            if !self.packets.values[index].ready && self.packets.values[index].fragment.key == key {
                drop(self.packets.values.remove(index));
            }
        }
    }

    fn private(&self, direction: Direction, canonical: Canonical) -> bool {
        if let Some(protocol) = canonical.opaque {
            // Preserve the existing conservative comparison of any visible TCP
            // tuple, without certifying an unknown native transformation.
            tracing::trace!(
                protocol,
                "private filter endpoint projection remains unverified"
            );
        }
        let tcp = match canonical.transport {
            Transport::Tcp(tcp) => tcp,
            Transport::NonTcp => return false,
            Transport::Opaque(protocol) => {
                tracing::trace!(
                    protocol,
                    "private filter native transformation boundary remains unverified"
                );
                return false;
            }
        };
        let state = self
            .connector
            .0
            .state
            .lock()
            .unwrap_or_else(|p| p.into_inner());
        state.entries.as_ref().is_some_and(|entries| {
            entries.values.iter().flatten().any(|flow| match direction {
                Direction::ToHost => {
                    canonical.destination == self.client
                        && canonical.source == *flow.endpoint.ip()
                        && tcp.source == flow.endpoint.port()
                        && tcp.destination == flow.source_port
                }
                Direction::ToDevice => {
                    canonical.source == self.client
                        && canonical.destination == *flow.endpoint.ip()
                        && tcp.source == flow.source_port
                        && tcp.destination == flow.endpoint.port()
                }
            })
        })
    }
}

fn parse(direction: Direction, packet: &[u8]) -> Parsed {
    if packet.len() < 40
        || packet[0] >> 4 != 6
        || 40 + usize::from(u16::from_be_bytes([packet[4], packet[5]])) != packet.len()
    {
        return Parsed::Malformed;
    }
    let mut projection = Projection {
        source: Ipv6Addr::from(<[u8; 16]>::try_from(&packet[8..24]).unwrap()),
        destination: Ipv6Addr::from(<[u8; 16]>::try_from(&packet[24..40]).unwrap()),
        home: false,
        fragment: false,
        authentication: false,
        opaque: None,
    };
    let mut cursor = 40;
    let mut next = packet[6];
    let mut removed_atomic = 0;
    loop {
        match next {
            44 => {
                let Some(header) = packet.get(cursor..cursor + 8) else {
                    return Parsed::Malformed;
                };
                let bits = u16::from_be_bytes([header[2], header[3]]);
                // RFC 8200 reserved fields are ignored by receivers.
                let start = usize::from(bits & 0xfff8);
                let more = bits & 1 != 0;
                let prefix = cursor - 40 - removed_atomic;
                let following = header[0];
                let id = u32::from_be_bytes(header[4..8].try_into().unwrap());
                cursor += 8;
                let data = packet.len() - cursor;
                let Some(end) = start.checked_add(data) else {
                    return Parsed::Malformed;
                };
                if end > u16::MAX as usize || (more && data % 8 != 0) {
                    return Parsed::Malformed;
                }
                projection.fragment = true;
                if start == 0 && !more {
                    // Logical removal, never an association with queued packets.
                    // Repeated atomic headers consume input rather than a cap.
                    removed_atomic += 8;
                    next = following;
                    continue;
                }
                // Pre-fragment semantics determine association; offset zero may
                // contain later Routing headers that change its final endpoint.
                let key = Key {
                    direction,
                    source: projection.source,
                    destination: projection.destination,
                    id,
                };
                let first = if start == 0 {
                    match transport(packet, cursor, following, projection) {
                        Some(canonical) => Some((canonical, prefix)),
                        None => return Parsed::Malformed,
                    }
                } else {
                    None
                };
                return Parsed::Fragment(Fragment {
                    key,
                    start,
                    end,
                    more,
                    first,
                });
            }
            0 | 43 | 60 | 51 | 135 => {
                if next == 0 && cursor != 40 {
                    return Parsed::Malformed;
                }
                let Some((following, end)) = extension(packet, cursor, next, &mut projection)
                else {
                    return Parsed::Malformed;
                };
                next = following;
                cursor = end;
            }
            _ => {
                return match transport(packet, cursor, next, projection) {
                    Some(canonical) => Parsed::Whole(canonical),
                    None => Parsed::Malformed,
                }
            }
        }
    }
}

fn extension(
    packet: &[u8],
    cursor: usize,
    next: u8,
    projection: &mut Projection,
) -> Option<(u8, usize)> {
    let header = packet.get(cursor..cursor.checked_add(2)?)?;
    let length = if next == 51 {
        (usize::from(header[1]) + 2).checked_mul(4)?
    } else {
        (usize::from(header[1]) + 1).checked_mul(8)?
    };
    let end = cursor.checked_add(length)?;
    let bytes = packet.get(cursor..end)?;
    match next {
        0 | 60 => options(bytes, next, projection)?,
        43 => {
            if projection.home {
                // RFC 6275 requires HAO after all Routing headers in its layer.
                return None;
            }
            match routing(bytes)? {
                RoutingProjection::Unchanged => {}
                RoutingProjection::Home(home) => projection.destination = home,
                RoutingProjection::Forward {
                    next,
                    final_destination,
                } => {
                    // This is an eventual endpoint after the RFC 8754 SID's
                    // routing operation, not a claim of local TCP delivery.
                    tracing::trace!(
                        ?next,
                        ?final_destination,
                        "possible SRH forwarding endpoints"
                    );
                    projection.destination = final_destination;
                }
                RoutingProjection::Opaque => projection.opaque = Some(43),
            }
        }
        51 => {
            // RFC 4302: mandatory SPI/Sequence, IPv6 64-bit alignment. AH is
            // visible plaintext; its authenticity remains a native SA concern.
            if length < 12 || length % 8 != 0 {
                return None;
            }
            projection.authentication = true;
        }
        135 => projection.opaque = Some(135),
        _ => return None,
    }
    Some((header[0], end))
}

fn options(bytes: &[u8], kind: u8, projection: &mut Projection) -> Option<()> {
    let mut cursor = 2;
    while cursor < bytes.len() {
        let option = bytes[cursor];
        if option == 0 {
            cursor += 1;
            continue;
        }
        let length = usize::from(*bytes.get(cursor.checked_add(1)?)?);
        let end = cursor.checked_add(2)?.checked_add(length)?;
        let value = bytes.get(cursor + 2..end)?;
        if option == 0xc9 {
            // RFC 6275 §§6.3/9.3.1. A valid native binding is required to
            // accept this identity; the filter does not invent such a binding.
            if kind != 60
                || length != 16
                || cursor % 8 != 6
                || projection.home
                || projection.fragment
                || projection.authentication
            {
                return None;
            }
            let home = Ipv6Addr::from(<[u8; 16]>::try_from(value).ok()?);
            if home.is_unspecified() || home.is_multicast() || home.is_loopback() {
                return None;
            }
            projection.source = home;
            projection.home = true;
        }
        // Padding and other options keep the owner unchanged. Semantics of
        // unknown options recognized by a native implementation remain outside
        // this plaintext HAO contract; action bits alone do not prove isolation.
        cursor = end;
    }
    Some(())
}

#[derive(Debug, PartialEq, Eq)]
enum RoutingProjection {
    Unchanged,
    Home(Ipv6Addr),
    Forward {
        next: Ipv6Addr,
        final_destination: Ipv6Addr,
    },
    Opaque,
}

fn routing(bytes: &[u8]) -> Option<RoutingProjection> {
    let fixed = bytes.get(..8)?;
    let segments = usize::from(fixed[3]);
    if segments == 0 {
        // Even an unknown type is ignored here (RFC 8200 §4.4). In
        // particular, stale SRH addresses must not overwrite the current DA.
        return Some(RoutingProjection::Unchanged);
    }
    match fixed[2] {
        2 => {
            if bytes.len() != 24 || segments != 1 {
                return None;
            }
            let home = Ipv6Addr::from(<[u8; 16]>::try_from(bytes.get(8..24)?).ok()?);
            if home.is_unspecified() || home.is_multicast() || home.is_loopback() {
                return None;
            }
            Some(RoutingProjection::Home(home))
        }
        4 => {
            let count = usize::from(fixed[4]).checked_add(1)?;
            let segment_end = 8usize.checked_add(count.checked_mul(16)?)?;
            if segment_end > bytes.len() || segments > count {
                return None;
            }
            let mut cursor = segment_end;
            while cursor < bytes.len() {
                let kind = bytes[cursor];
                if kind == 0 {
                    cursor += 1;
                    continue;
                }
                let length = usize::from(*bytes.get(cursor.checked_add(1)?)?);
                let end = cursor.checked_add(2)?.checked_add(length)?;
                bytes.get(cursor..end)?;
                if kind == 4 && length > 5 {
                    return None;
                }
                cursor = end;
            }
            let next_start = 8usize.checked_add((segments - 1).checked_mul(16)?)?;
            let next =
                Ipv6Addr::from(<[u8; 16]>::try_from(bytes.get(next_start..next_start + 16)?).ok()?);
            let final_destination = Ipv6Addr::from(<[u8; 16]>::try_from(bytes.get(8..24)?).ok()?);
            Some(RoutingProjection::Forward {
                next,
                final_destination,
            })
        }
        _ => Some(RoutingProjection::Opaque),
    }
}

fn transport(
    packet: &[u8],
    mut cursor: usize,
    mut next: u8,
    mut projection: Projection,
) -> Option<Canonical> {
    loop {
        match next {
            6 => {
                let header = packet.get(cursor..cursor.checked_add(20)?)?;
                let length = usize::from(header[12] >> 4) * 4;
                if length < 20 || cursor.checked_add(length)? > packet.len() {
                    return None;
                }
                return Some(projection.canonical(Transport::Tcp(TcpTuple {
                    source: u16::from_be_bytes([header[0], header[1]]),
                    destination: u16::from_be_bytes([header[2], header[3]]),
                })));
            }
            43 | 60 | 51 | 135 => {
                let (following, end) = extension(packet, cursor, next, &mut projection)?;
                next = following;
                cursor = end;
            }
            44 => {
                let header = packet.get(cursor..cursor.checked_add(8)?)?;
                let bits = u16::from_be_bytes([header[2], header[3]]);
                if bits & 0xfff9 != 0 {
                    // A second ordinary fragment association is not handled by
                    // this single-layer model; preserve the existing rejection.
                    return None;
                }
                projection.fragment = true;
                next = header[0];
                cursor += 8;
            }
            41 => {
                // RFC 2473: IPv6 Encapsulation in IPv6.
                // Inspect the inner IPv6 packet to ensure private flows nested
                // inside outer encapsulation cannot bypass endpoint isolation.
                let inner = packet.get(cursor..)?;
                if inner.len() < 40 || inner[0] >> 4 != 6 {
                    return None;
                }
                let inner_len = usize::from(u16::from_be_bytes([inner[4], inner[5]]));
                if 40 + inner_len > inner.len() {
                    return None;
                }
                projection = Projection {
                    source: Ipv6Addr::from(<[u8; 16]>::try_from(&inner[8..24]).ok()?),
                    destination: Ipv6Addr::from(<[u8; 16]>::try_from(&inner[24..40]).ok()?),
                    home: false,
                    fragment: false,
                    authentication: false,
                    opaque: None,
                };
                next = inner[6];
                cursor += 40;
            }
            4 => {
                // RFC 2473: IPv4 Encapsulation in IPv6.
                let inner = packet.get(cursor..)?;
                if inner.len() < 20 || inner[0] >> 4 != 4 {
                    return None;
                }
                let ihl = usize::from(inner[0] & 0x0f) * 4;
                let total_len = usize::from(u16::from_be_bytes([inner[2], inner[3]]));
                if ihl < 20 || total_len > inner.len() || ihl > total_len {
                    return None;
                }
                let inner_proto = inner[9];
                if inner_proto == 41 {
                    cursor += ihl;
                    next = 41;
                    continue;
                }
                return Some(projection.canonical(Transport::NonTcp));
            }
            0 => return None,
            17 | 58 | 59 => return Some(projection.canonical(Transport::NonTcp)),
            // Explicit unresolved boundary, including ESP, encapsulated IP,
            // Mobility and native protocols not interpreted by this filter.
            _ => return Some(projection.canonical(Transport::Opaque(next))),
        }
    }
}

#[cfg(test)]
mod tests;
