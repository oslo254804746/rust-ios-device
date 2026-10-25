//! Borrowed rtnetlink codec. No response row or attribute is copied/retained.

use std::io;

pub(super) const HEADER: usize = 16;
pub(super) const REQUEST: u16 = 1;
pub(super) const ACK: u16 = 4;
pub(super) const DUMP: u16 = 0x300;
pub(super) const CREATE_EXCLUSIVE: u16 = 0x600;
pub(super) const MULTI: u16 = 2;
pub(super) const DUMP_INTERRUPTED: u16 = 0x10;
pub(super) const CAPPED: u16 = 0x100;
pub(super) const ACK_TLVS: u16 = 0x200;
pub(super) const ERROR: u16 = 2;
pub(super) const DONE: u16 = 3;
pub(super) const OVERRUN: u16 = 4;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct AddressLifetimes {
    pub preferred: u32,
    pub valid: u32,
}

impl AddressLifetimes {
    pub const INFINITE: Self = Self {
        preferred: u32::MAX,
        valid: u32::MAX,
    };

    pub fn parse(bytes: &[u8]) -> io::Result<Self> {
        if bytes.len() != 16 {
            return Err(invalid("invalid IFA_CACHEINFO length"));
        }
        let result = Self {
            preferred: u32_at(bytes, 0)?,
            valid: u32_at(bytes, 4)?,
        };
        if result.preferred > result.valid {
            return Err(invalid("preferred address lifetime exceeds valid lifetime"));
        }
        Ok(result)
    }
}

/// Exact observed datagram still awaiting caller-paid receive backing. The
/// containing native owner retains this scalar state even if its bank closes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct PendingResponse {
    pub(crate) required: usize,
    pub(crate) sequence: u32,
    pub(crate) message_kind: u16,
}

#[derive(Default)]
pub(super) struct ResponseState {
    pending: Option<PendingResponse>,
}

impl ResponseState {
    pub fn check(
        &mut self,
        required: usize,
        capacity: usize,
        sequence: u32,
        message_kind: u16,
    ) -> Result<(), PendingResponse> {
        if required > capacity {
            let pending = PendingResponse {
                required,
                sequence,
                message_kind,
            };
            self.pending = Some(pending);
            return Err(pending);
        }
        Ok(())
    }

    pub fn pending(&self) -> Option<PendingResponse> {
        self.pending
    }
    pub fn consumed(&mut self) {
        self.pending = None;
    }
}

/// Consuming a loss notification is a distinct native operation. Its failure
/// must not replace the already acquired loss error. The caller retains the
/// returned additional cleanup error in its concrete pre-existing slot.
pub(super) fn consume_loss<E>(
    original: io::Error,
    consume: impl FnOnce() -> Result<(), E>,
) -> (io::Error, Option<E>) {
    (original, consume().err())
}

pub(super) fn invalid(message: &'static str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message)
}

pub(super) fn u16_at(bytes: &[u8], offset: usize) -> io::Result<u16> {
    let value = bytes
        .get(offset..offset + 2)
        .ok_or_else(|| invalid("short u16"))?;
    Ok(u16::from_ne_bytes([value[0], value[1]]))
}

pub(super) fn u32_at(bytes: &[u8], offset: usize) -> io::Result<u32> {
    let value = bytes
        .get(offset..offset + 4)
        .ok_or_else(|| invalid("short u32"))?;
    Ok(u32::from_ne_bytes([value[0], value[1], value[2], value[3]]))
}

pub(super) fn aligned(length: usize) -> io::Result<usize> {
    length
        .checked_add(3)
        .map(|v| v & !3)
        .ok_or_else(|| invalid("netlink length overflow"))
}

#[derive(Clone, Copy, Debug)]
pub(super) struct Message<'a> {
    pub kind: u16,
    pub flags: u16,
    pub sequence: u32,
    pub port: u32,
    pub payload: &'a [u8],
}

pub(super) struct Messages<'a> {
    bytes: &'a [u8],
}

impl<'a> Messages<'a> {
    pub fn new(bytes: &'a [u8]) -> Self {
        Self { bytes }
    }
}

impl<'a> Iterator for Messages<'a> {
    type Item = io::Result<Message<'a>>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.bytes.is_empty() {
            return None;
        }
        let parsed = (|| {
            let length = u32_at(self.bytes, 0)? as usize;
            if length < HEADER || length > self.bytes.len() {
                return Err(invalid("invalid netlink message length"));
            }
            let message = Message {
                kind: u16_at(self.bytes, 4)?,
                flags: u16_at(self.bytes, 6)?,
                sequence: u32_at(self.bytes, 8)?,
                port: u32_at(self.bytes, 12)?,
                payload: &self.bytes[HEADER..length],
            };
            let next = aligned(length)?;
            // The last message need not carry alignment padding.
            self.bytes = if next > self.bytes.len() {
                &[]
            } else {
                &self.bytes[next..]
            };
            Ok(message)
        })();
        if parsed.is_err() {
            self.bytes = &[];
        }
        Some(parsed)
    }
}

pub(super) struct Attributes<'a> {
    bytes: &'a [u8],
}

impl<'a> Attributes<'a> {
    pub fn new(bytes: &'a [u8]) -> Self {
        Self { bytes }
    }
}

impl<'a> Iterator for Attributes<'a> {
    type Item = io::Result<(u16, &'a [u8])>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.bytes.is_empty() {
            return None;
        }
        let parsed = (|| {
            let length = u16_at(self.bytes, 0)? as usize;
            if length < 4 || length > self.bytes.len() {
                return Err(invalid("invalid rtnetlink attribute length"));
            }
            let kind = u16_at(self.bytes, 2)? & 0x3fff;
            let value = &self.bytes[4..length];
            let next = aligned(length)?;
            self.bytes = if next > self.bytes.len() {
                &[]
            } else {
                &self.bytes[next..]
            };
            Ok((kind, value))
        })();
        if parsed.is_err() {
            self.bytes = &[];
        }
        Some(parsed)
    }
}

/// A concrete request's maximum layout is an IPv6 address request with local,
/// address and 32-bit flags attributes: 16 + 8 + 20 + 20 + 8 = 72 bytes.
/// Link and route requests use fewer fields. This is request storage, not a
/// guessed receive arena. The caller pays the complete containing value.
#[derive(Clone, Copy)]
pub(super) struct Request {
    bytes: [u8; 72],
    length: usize,
}

impl Request {
    pub fn new(kind: u16, flags: u16, sequence: u32, port: u32, body: &[u8]) -> io::Result<Self> {
        if HEADER + body.len() > 72 {
            return Err(invalid("request body exceeds its concrete layout"));
        }
        let mut result = Self {
            bytes: [0; 72],
            length: HEADER + body.len(),
        };
        result.bytes[4..6].copy_from_slice(&kind.to_ne_bytes());
        result.bytes[6..8].copy_from_slice(&(REQUEST | flags).to_ne_bytes());
        result.bytes[8..12].copy_from_slice(&sequence.to_ne_bytes());
        result.bytes[12..16].copy_from_slice(&port.to_ne_bytes());
        result.bytes[HEADER..result.length].copy_from_slice(body);
        result.finish_length();
        Ok(result)
    }

    pub fn attribute(&mut self, kind: u16, value: &[u8]) -> io::Result<()> {
        let raw = 4usize
            .checked_add(value.len())
            .ok_or_else(|| invalid("attribute overflow"))?;
        let end = self
            .length
            .checked_add(aligned(raw)?)
            .ok_or_else(|| invalid("request overflow"))?;
        if end > self.bytes.len() || raw > u16::MAX as usize {
            return Err(invalid("request attributes exceed their concrete layout"));
        }
        self.bytes[self.length..self.length + 2].copy_from_slice(&(raw as u16).to_ne_bytes());
        self.bytes[self.length + 2..self.length + 4].copy_from_slice(&kind.to_ne_bytes());
        self.bytes[self.length + 4..self.length + raw].copy_from_slice(value);
        self.length = end;
        self.finish_length();
        Ok(())
    }

    fn finish_length(&mut self) {
        self.bytes[..4].copy_from_slice(&(self.length as u32).to_ne_bytes());
    }

    pub fn bytes(&self) -> &[u8] {
        &self.bytes[..self.length]
    }

    pub fn same_operation(&self, other: &Self) -> bool {
        self.length == other.length
            && self.bytes[4..8] == other.bytes[4..8]
            && self.bytes[HEADER..self.length] == other.bytes[HEADER..other.length]
    }

    /// ACKs may cap the echoed request to its header. The exact unique request
    /// header must still match; uncapped echoes must also match every key byte.
    pub fn ack(&self, payload: &[u8], flags: u16) -> io::Result<()> {
        if payload.len() < 4 + HEADER {
            return Err(invalid("short NLMSG_ERROR"));
        }
        if payload[4..4 + HEADER] != self.bytes[..HEADER] {
            return Err(invalid("ACK does not echo the pending request header"));
        }
        let echoed_length = if flags & CAPPED != 0 {
            HEADER
        } else {
            self.length
        };
        let end = 4 + aligned(echoed_length)?;
        if payload.len() < end {
            return Err(invalid("truncated uncapped request echo"));
        }
        if payload[4..4 + echoed_length] != self.bytes[..echoed_length] {
            return Err(invalid("ACK does not echo the pending request key"));
        }
        let status = i32::from_ne_bytes(payload[..4].try_into().expect("checked status"));
        if status > 0 || status == i32::MIN {
            return Err(invalid("invalid native netlink error number"));
        }
        if status < 0 {
            return Err(io::Error::from_raw_os_error(-status));
        }
        if flags & ACK_TLVS != 0 {
            for attribute in Attributes::new(&payload[end..]) {
                attribute?;
            }
        } else if payload.len() != end {
            return Err(invalid("unflagged trailing ACK data"));
        }
        Ok(())
    }
}

#[cfg(test)]
#[cfg_attr(not(target_os = "linux"), path = "codec/tests.rs")]
mod tests;
