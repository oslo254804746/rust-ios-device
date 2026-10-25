//! Exact private TCP flows and creation-owned IPv6 fragment association.
//!
//! RFC 8200 section 4.5: source/destination/ID associate fragments; headers
//! before Fragment and non-first Next Header may differ. Only offset zero
//! supplies the real upper-layer protocol. No unresolved packet reaches OS IO.
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

#[derive(Clone, Copy)]
struct TcpTuple {
    source: u16,
    destination: u16,
}

#[derive(Clone, Copy)]
enum Transport {
    Tcp(TcpTuple),
    Other,
}

#[derive(Clone, Copy)]
struct Fragment {
    key: Key,
    start: usize,
    end: usize,
    more: bool,
    first: Option<(Transport, usize)>,
}

enum Parsed {
    Whole {
        source: Ipv6Addr,
        destination: Ipv6Addr,
        transport: Transport,
    },
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
    first: Option<(Transport, usize)>,
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
        self.expire();
        let fragment = match parse(direction, &packet) {
            Parsed::Malformed => return Ok(FilterOutput::Discard(DiscardReason::Malformed)),
            Parsed::Whole {
                source,
                destination,
                transport,
            } => {
                return if self.private(direction, source, destination, transport) {
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
            match Instant::now().checked_add(REASSEMBLY_LIFETIME) {
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
        let (transport, _) = self.datagrams.values[index]
            .first
            .expect("complete offset zero header");
        self.datagrams.values[index].status = Status::Complete;
        if self.private(direction, key.source, key.destination, transport) {
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

    fn private(
        &self,
        direction: Direction,
        source: Ipv6Addr,
        destination: Ipv6Addr,
        transport: Transport,
    ) -> bool {
        let Transport::Tcp(tcp) = transport else {
            return false;
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
                    destination == self.client
                        && source == *flow.endpoint.ip()
                        && tcp.source == flow.endpoint.port()
                        && tcp.destination == flow.source_port
                }
                Direction::ToDevice => {
                    source == self.client
                        && destination == *flow.endpoint.ip()
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
    let source = Ipv6Addr::from(<[u8; 16]>::try_from(&packet[8..24]).expect("checked IPv6 source"));
    let destination =
        Ipv6Addr::from(<[u8; 16]>::try_from(&packet[24..40]).expect("checked IPv6 destination"));
    let mut cursor = 40;
    let mut next = packet[6];
    let mut atomic = false;
    loop {
        match next {
            44 => {
                if atomic || packet.len() < cursor + 8 {
                    return Parsed::Malformed;
                }
                let bits = u16::from_be_bytes([packet[cursor + 2], packet[cursor + 3]]);
                // RFC 8200 reserved fields are ignored by receivers.
                let start = usize::from(bits & 0xfff8);
                let more = bits & 1 != 0;
                let prefix = cursor - 40;
                let following = packet[cursor];
                let id = u32::from_be_bytes(packet[cursor + 4..cursor + 8].try_into().unwrap());
                cursor += 8;
                let data = packet.len() - cursor;
                let Some(end) = start.checked_add(data) else {
                    return Parsed::Malformed;
                };
                if end > u16::MAX as usize || (more && data % 8 != 0) {
                    return Parsed::Malformed;
                }
                if start == 0 && !more {
                    atomic = true;
                    next = following;
                    continue;
                }
                let first = if start == 0 {
                    match transport(packet, cursor, following) {
                        Some(transport) => Some((transport, prefix)),
                        None => return Parsed::Malformed,
                    }
                } else {
                    None
                };
                return Parsed::Fragment(Fragment {
                    key: Key {
                        direction,
                        source,
                        destination,
                        id,
                    },
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
                let Some((following, end)) = extension(packet, cursor, next) else {
                    return Parsed::Malformed;
                };
                next = following;
                cursor = end;
            }
            _ => {
                return match transport(packet, cursor, next) {
                    Some(transport) => Parsed::Whole {
                        source,
                        destination,
                        transport,
                    },
                    None => Parsed::Malformed,
                }
            }
        }
    }
}

fn extension(packet: &[u8], cursor: usize, next: u8) -> Option<(u8, usize)> {
    let header = packet.get(cursor..cursor.checked_add(2)?)?;
    let length = if next == 51 {
        (usize::from(header[1]) + 2).checked_mul(4)?
    } else {
        (usize::from(header[1]) + 1).checked_mul(8)?
    };
    let end = cursor.checked_add(length)?;
    (end <= packet.len()).then_some((header[0], end))
}

fn transport(packet: &[u8], mut cursor: usize, mut next: u8) -> Option<Transport> {
    loop {
        match next {
            6 => {
                let header = packet.get(cursor..cursor.checked_add(20)?)?;
                let length = usize::from(header[12] >> 4) * 4;
                if length < 20 || cursor.checked_add(length)? > packet.len() {
                    return None;
                }
                return Some(Transport::Tcp(TcpTuple {
                    source: u16::from_be_bytes([header[0], header[1]]),
                    destination: u16::from_be_bytes([header[2], header[3]]),
                }));
            }
            43 | 60 | 51 | 135 => {
                let (following, end) = extension(packet, cursor, next)?;
                next = following;
                cursor = end;
            }
            0 | 44 => return None,
            _ => return Some(Transport::Other),
        }
    }
}

#[cfg(test)]
mod tests;
