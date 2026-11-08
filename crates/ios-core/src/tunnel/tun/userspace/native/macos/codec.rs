//! Darwin PF_ROUTE wire codec. XNU route.h fixes this header's ABI at 92
//! bytes; rtsock.c uses ROUNDUP32 for each sockaddr, including on 64-bit hosts.
//! The target module checks every used libc header offset at compile time.

use std::{io, net::Ipv6Addr};

pub(super) const HEADER: usize = 92;
pub(super) const RESPONSE_CAPACITY: usize = u16::MAX as usize;
pub(super) const VERSION: u8 = 5;
pub(super) const ADD: u8 = 1;
pub(super) const DELETE: u8 = 2;
pub(super) const GET: u8 = 4;
pub(super) const AF_INET6: u8 = 30;
pub(super) const AF_LINK: u8 = 18;
pub(super) const UP: u32 = 1;
pub(super) const GATEWAY: u32 = 2;
pub(super) const HOST: u32 = 4;
pub(super) const STATIC: u32 = 0x800;
pub(super) const IFSCOPE: u32 = 0x1000000;
const BOOKKEEPING: u32 = 0x40 | 0x4000000; // DONE, IFREF

pub(super) fn invalid(message: &'static str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message)
}
pub(super) fn u16_at(bytes: &[u8], at: usize) -> io::Result<u16> {
    Ok(u16::from_ne_bytes(
        bytes
            .get(at..at + 2)
            .ok_or_else(|| invalid("short Darwin u16"))?
            .try_into()
            .expect("checked length"),
    ))
}
pub(super) fn u32_at(bytes: &[u8], at: usize) -> io::Result<u32> {
    Ok(u32::from_ne_bytes(
        bytes
            .get(at..at + 4)
            .ok_or_else(|| invalid("short Darwin u32"))?
            .try_into()
            .expect("checked length"),
    ))
}

fn round32(length: usize) -> io::Result<usize> {
    length
        .checked_add(3)
        .map(|length| length & !3)
        .ok_or_else(|| invalid("Darwin sockaddr length overflow"))
}

/// sa_len is one byte in XNU sockaddr. This array's bound is the native
/// protocol field's range, not a guessed per-interface or receive arena cap.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct Address {
    bytes: [u8; u8::MAX as usize],
    length: u8,
}

impl Address {
    pub fn from_bytes(bytes: &[u8]) -> io::Result<Self> {
        if bytes.len() < 2 || bytes.len() > u8::MAX as usize || bytes[0] as usize != bytes.len() {
            return Err(invalid("invalid Darwin sockaddr size"));
        }
        let mut result = Self {
            bytes: [0; u8::MAX as usize],
            length: bytes.len() as u8,
        };
        result.bytes[..bytes.len()].copy_from_slice(bytes);
        Ok(result)
    }
    pub fn ipv6(address: Ipv6Addr, scope: u32) -> Self {
        let mut result = Self {
            bytes: [0; u8::MAX as usize],
            length: 28,
        };
        result.bytes[0] = 28;
        result.bytes[1] = AF_INET6;
        result.bytes[8..24].copy_from_slice(&address.octets());
        result.bytes[24..28].copy_from_slice(&scope.to_ne_bytes());
        result
    }
    pub fn link(index: u16) -> Self {
        let mut result = Self {
            bytes: [0; u8::MAX as usize],
            length: 8,
        };
        result.bytes[0] = 8;
        result.bytes[1] = AF_LINK;
        result.bytes[2..4].copy_from_slice(&index.to_ne_bytes());
        result
    }
    pub fn mask(prefix: u8) -> io::Result<Self> {
        if prefix > 128 {
            return Err(invalid("invalid IPv6 prefix"));
        }
        let mut octets = [0; 16];
        for (index, octet) in octets.iter_mut().enumerate() {
            let bits = prefix.saturating_sub((index * 8) as u8).min(8);
            *octet = if bits == 0 { 0 } else { u8::MAX << (8 - bits) };
        }
        let mut result = Self::ipv6(Ipv6Addr::from(octets), 0);
        result.bytes[1] = 0; // XNU masks carry AF_UNSPEC.
        Ok(result)
    }
    pub fn bytes(&self) -> &[u8] {
        &self.bytes[..usize::from(self.length)]
    }
    pub fn family(&self) -> u8 {
        self.bytes[1]
    }
    pub fn ip(&self) -> io::Result<(Ipv6Addr, u32)> {
        if self.family() != AF_INET6
            || self.length != 28
            || self.bytes[2..8].iter().any(|v| *v != 0)
        {
            return Err(invalid("unexpected IPv6 route sockaddr"));
        }
        let address =
            Ipv6Addr::from(<[u8; 16]>::try_from(&self.bytes[8..24]).expect("fixed IPv6 field"));
        Ok((
            address,
            u32::from_ne_bytes(self.bytes[24..28].try_into().expect("fixed scope field")),
        ))
    }
    pub fn link_index(&self) -> io::Result<u16> {
        if self.family() != AF_LINK || self.length < 8 {
            return Err(invalid("unexpected route interface sockaddr"));
        }
        let data_length =
            usize::from(self.bytes[5]) + usize::from(self.bytes[6]) + usize::from(self.bytes[7]);
        if data_length > usize::from(self.length) - 8 {
            return Err(invalid("short link sockaddr data"));
        }
        u16_at(self.bytes(), 2)
    }
    pub fn prefix(&self) -> io::Result<u8> {
        if !matches!(self.family(), 0 | AF_INET6) || self.length > 28 {
            return Err(invalid("unexpected IPv6 route mask"));
        }
        // XNU compacts trailing zero bytes in masks. Missing tail bytes mean
        // zero, not an absent mask. No sockaddr_in6 cast is made on that tail.
        let mut octets = [0; 16];
        if self.length > 8 {
            let end = usize::from(self.length).min(24);
            octets[..end - 8].copy_from_slice(&self.bytes[8..end]);
        }
        let mut prefix = 0u8;
        let mut zero_seen = false;
        for octet in octets {
            for bit in (0..8).rev() {
                if octet & (1 << bit) != 0 {
                    if zero_seen {
                        return Err(invalid("noncontiguous IPv6 route mask"));
                    }
                    prefix += 1;
                } else {
                    zero_seen = true;
                }
            }
        }
        Ok(prefix)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct Query {
    pub destination: Ipv6Addr,
    pub prefix: u8,
    pub index: u16,
    pub scoped: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct Row {
    pub query: Query,
    pub gateway: Address,
    pub interface_address: Option<Address>,
    pub flags: u32,
    // Locks, MTU, hopcount, expiry, pipes, ssthresh and RTT/variance are stable
    // route semantics. Packet/use counters and reserved filler are excluded.
    pub metrics: [u32; 9],
}

impl Row {
    pub fn same_owned(self, other: Self) -> bool {
        self.query == other.query
            && self.gateway == other.gateway
            && self.interface_address == other.interface_address
            && self.flags & !BOOKKEEPING == other.flags & !BOOKKEEPING
            && self.metrics == other.metrics
    }
    pub fn reachable_on(self, query: Query) -> bool {
        self.query.destination == query.destination
            && self.query.prefix == query.prefix
            && self.query.index == query.index
            && self.flags & UP != 0
            && self.flags & (GATEWAY | HOST | 0x8 | 0x1000) == 0
            && self.metrics[3] == 0
    }
}

#[derive(Clone, Copy, Debug)]
pub(super) struct Message<'a> {
    pub kind: u8,
    pub index: u16,
    pub flags: u32,
    pub pid: i32,
    pub sequence: i32,
    pub errno: i32,
    pub addrs: u32,
    bytes: &'a [u8],
}

impl<'a> Message<'a> {
    pub fn parse(bytes: &'a [u8]) -> io::Result<Self> {
        if bytes.len() < HEADER
            || usize::from(u16_at(bytes, 0)?) != bytes.len()
            || bytes[2] != VERSION
        {
            return Err(invalid("invalid Darwin routing header"));
        }
        let errno = u32_at(bytes, 24)? as i32;
        if errno < 0 {
            return Err(invalid("negative Darwin routing errno"));
        }
        Ok(Self {
            kind: bytes[3],
            index: u16_at(bytes, 4)?,
            flags: u32_at(bytes, 8)?,
            addrs: u32_at(bytes, 12)?,
            pid: u32_at(bytes, 16)? as i32,
            sequence: u32_at(bytes, 20)? as i32,
            errno,
            bytes,
        })
    }
    pub fn matches(&self, request: &Request) -> bool {
        self.pid == request.pid && self.sequence == request.sequence && self.kind == request.kind
    }
    pub fn row(self) -> io::Result<Row> {
        let addresses = self.addresses()?;
        let destination = addresses[0]
            .ok_or_else(|| invalid("route destination missing"))?
            .ip()?
            .0;
        let prefix = if self.flags & HOST != 0 {
            128
        } else {
            addresses[2]
                .ok_or_else(|| invalid("route mask missing"))?
                .prefix()?
        };
        let gateway = addresses[1].ok_or_else(|| invalid("route gateway missing"))?;
        if let Some(interface) = addresses[4] {
            if interface.link_index()? != self.index {
                return Err(invalid("route interface/index disagree"));
            }
        }
        if addresses[3].is_some() || addresses[6].is_some() || addresses[7].is_some() {
            return Err(invalid("unexpected extra route key"));
        }
        let mut metrics = [0; 9];
        for (index, value) in metrics.iter_mut().enumerate() {
            *value = u32_at(self.bytes, 36 + 4 * index)?;
        }
        Ok(Row {
            query: Query {
                destination,
                prefix,
                index: self.index,
                scoped: self.flags & IFSCOPE != 0,
            },
            gateway,
            interface_address: addresses[5],
            flags: self.flags,
            metrics,
        })
    }
    fn addresses(self) -> io::Result<[Option<Address>; 8]> {
        if self.addrs & !0xff != 0 {
            return Err(invalid("unknown Darwin route sockaddr index"));
        }
        let mut result = [None; 8];
        let mut offset = HEADER;
        for (index, address) in result.iter_mut().enumerate() {
            if self.addrs & (1 << index) == 0 {
                continue;
            }
            let length = *self
                .bytes
                .get(offset)
                .ok_or_else(|| invalid("short route sockaddr"))? as usize;
            // A zero-length mask is legal for /0. Its native occupied width
            // is four; represent it as the canonical two-byte zero mask.
            if length == 0 && index == 2 {
                *address = Some(Address::from_bytes(&[2, 0])?);
                offset = offset
                    .checked_add(4)
                    .ok_or_else(|| invalid("sockaddr offset overflow"))?;
                continue;
            }
            *address = Some(Address::from_bytes(
                self.bytes
                    .get(offset..offset + length)
                    .ok_or_else(|| invalid("truncated route sockaddr"))?,
            )?);
            offset = offset
                .checked_add(round32(length)?)
                .ok_or_else(|| invalid("sockaddr offset overflow"))?;
        }
        if offset != self.bytes.len() {
            return Err(invalid("routing message sockaddr boundary differs"));
        }
        Ok(result)
    }
    /// GET/DELETE replies contain the actual row, not a verbatim request.
    /// Require exact destination/prefix and interface scope before status can
    /// certify ownership. A negative native status still keeps its errno.
    pub fn status(self, request: &Request) -> io::Result<()> {
        if !self.matches(request) {
            return Err(invalid("uncorrelated Darwin route reply"));
        }
        let addresses = self.addresses()?;
        if addresses[0]
            .ok_or_else(|| invalid("ACK destination missing"))?
            .ip()?
            .0
            != request.query.destination
            || addresses[2]
                .ok_or_else(|| invalid("ACK mask missing"))?
                .prefix()?
                != request.query.prefix
        {
            return Err(invalid("Darwin route ACK key differs"));
        }
        if request.query.index != 0 && self.index != request.query.index {
            return Err(invalid("Darwin route ACK interface differs"));
        }
        if request.query.scoped && self.flags & IFSCOPE == 0 {
            return Err(invalid("Darwin route ACK scope differs"));
        }
        if let Some(gateway) = request.gateway {
            if addresses[1] != Some(gateway) {
                return Err(invalid("Darwin route ACK gateway differs"));
            }
        }
        if self.errno != 0 {
            return Err(io::Error::from_raw_os_error(self.errno));
        }
        Ok(())
    }
}

/// Header + IPv6 destination + maximum sa_len gateway + IPv6 mask + minimal
/// AF_LINK interface selector + IPv6 source selector, all ROUNDUP32 aligned.
const REQUEST_CAPACITY: usize = HEADER + 28 + 256 + 28 + 8 + 28;

#[derive(Clone, Copy)]
pub(super) struct Request {
    bytes: [u8; REQUEST_CAPACITY],
    length: usize,
    pub kind: u8,
    pub pid: i32,
    pub sequence: i32,
    pub query: Query,
    gateway: Option<Address>,
}

impl Request {
    pub fn new(
        kind: u8,
        pid: i32,
        sequence: i32,
        query: Query,
        gateway: Option<Address>,
        source: Option<Ipv6Addr>,
        flags: u32,
    ) -> io::Result<Self> {
        if !matches!(kind, ADD | DELETE | GET) || query.prefix > 128 || pid <= 0 || sequence <= 0 {
            return Err(invalid("invalid private routing request"));
        }
        let mut result = Self {
            bytes: [0; REQUEST_CAPACITY],
            length: HEADER,
            kind,
            pid,
            sequence,
            query,
            gateway,
        };
        result.bytes[2] = VERSION;
        result.bytes[3] = kind;
        result.bytes[4..6].copy_from_slice(&query.index.to_ne_bytes());
        result.bytes[8..12]
            .copy_from_slice(&(flags | if query.scoped { IFSCOPE } else { 0 }).to_ne_bytes());
        result.bytes[16..20].copy_from_slice(&pid.to_ne_bytes());
        result.bytes[20..24].copy_from_slice(&sequence.to_ne_bytes());
        result.address(0, Address::ipv6(query.destination, 0))?;
        if let Some(gateway) = gateway {
            result.address(1, gateway)?;
        }
        result.address(2, Address::mask(query.prefix)?)?;
        result.address(4, Address::link(query.index))?;
        if let Some(source) = source {
            result.address(5, Address::ipv6(source, 0))?;
        }
        result.bytes[..2].copy_from_slice(&(result.length as u16).to_ne_bytes());
        Ok(result)
    }
    fn address(&mut self, index: u32, address: Address) -> io::Result<()> {
        let width = round32(address.bytes().len())?;
        let end = self
            .length
            .checked_add(width)
            .ok_or_else(|| invalid("request size overflow"))?;
        if end > self.bytes.len() {
            return Err(invalid("request exceeds concrete protocol layout"));
        }
        self.bytes[self.length..self.length + address.bytes().len()]
            .copy_from_slice(address.bytes());
        self.length = end;
        let addrs = u32_at(&self.bytes, 12)? | (1 << index);
        self.bytes[12..16].copy_from_slice(&addrs.to_ne_bytes());
        Ok(())
    }
    pub fn bytes(&self) -> &[u8] {
        &self.bytes[..self.length]
    }
    pub fn same_operation(&self, other: &Self) -> bool {
        self.kind == other.kind
            && self.query == other.query
            && self.length == other.length
            && self.bytes[8..16] == other.bytes[8..16]
            && self.bytes[HEADER..self.length] == other.bytes[HEADER..other.length]
    }
}

#[cfg(test)]
#[cfg_attr(not(target_os = "macos"), path = "codec/tests.rs")]
mod tests;
