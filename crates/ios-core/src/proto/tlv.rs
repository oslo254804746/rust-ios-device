use std::collections::HashMap;

use bytes::Bytes;

/// TLV (Tag-Length-Value) buffer encoder/decoder.
pub struct TlvBuffer(Vec<u8>);

impl TlvBuffer {
    pub fn new() -> Self {
        Self(Vec::new())
    }

    pub fn push_u8(&mut self, tag: u8, value: u8) {
        self.0.push(tag);
        self.0.push(1);
        self.0.push(value);
    }

    /// Push a byte slice. Values longer than 255 bytes are split into
    /// 255-byte chunks with the same tag (Apple TLV8 convention).
    pub fn push_bytes(&mut self, tag: u8, value: &[u8]) {
        if value.is_empty() {
            self.0.push(tag);
            self.0.push(0);
            return;
        }
        for chunk in value.chunks(255) {
            self.0.push(tag);
            self.0.push(chunk.len() as u8);
            self.0.extend_from_slice(chunk);
        }
    }

    pub fn into_bytes(self) -> Vec<u8> {
        self.0
    }

    /// Decode a flat TLV buffer into a map of tag → coalesced value bytes.
    ///
    /// Consecutive items with the same tag are concatenated (Apple TLV8 coalescing).
    pub fn decode(buf: &[u8]) -> HashMap<u8, Bytes> {
        let mut map: HashMap<u8, bytes::BytesMut> = HashMap::new();
        let mut last_tag: Option<u8> = None;
        let mut i = 0;
        while i + 2 <= buf.len() {
            let tag = buf[i];
            let len = buf[i + 1] as usize;
            i += 2;
            if i + len > buf.len() {
                break;
            }
            let chunk = &buf[i..i + len];
            i += len;
            // Coalesce same-tag consecutive chunks
            if last_tag == Some(tag) {
                map.entry(tag).or_default().extend_from_slice(chunk);
            } else {
                map.entry(tag).or_default().extend_from_slice(chunk);
                last_tag = Some(tag);
            }
        }
        map.into_iter().map(|(k, v)| (k, v.freeze())).collect()
    }
}

impl Default for TlvBuffer {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip_u8() {
        let mut buf = TlvBuffer::new();
        buf.push_u8(0x01, 42);
        let bytes = buf.into_bytes();
        let map = TlvBuffer::decode(&bytes);
        assert_eq!(map[&0x01].as_ref(), &[42]);
    }

    #[test]
    fn roundtrip_bytes() {
        let mut buf = TlvBuffer::new();
        buf.push_bytes(0x02, b"hello");
        let bytes = buf.into_bytes();
        let map = TlvBuffer::decode(&bytes);
        assert_eq!(map[&0x02].as_ref(), b"hello");
    }
}
