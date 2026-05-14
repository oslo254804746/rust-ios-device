//! XPC message encoder/decoder for the Apple XPC binary protocol.
//!
//! Reference: go-ios/ios/xpc/encoding.go
//!
//! Message layout (all little-endian):
//!   magic    [4] = 0x29B00B92
//!   flags    [4]
//!   body_len [8]  (0 if no body)
//!   msg_id   [8]
//!   [if body_len > 0]:
//!     body_magic   [4] = 0x42133742
//!     body_version [4] = 0x00000005
//!     XPC value encoding

use bytes::{Buf, BufMut, Bytes, BytesMut};
use indexmap::IndexMap;

use crate::xpc::XpcError;

pub const WRAPPER_MAGIC: u32 = 0x29B00B92;
pub const OBJECT_MAGIC: u32 = 0x42133742;
pub const BODY_VERSION: u32 = 0x00000005;

/// XPC message flags.
pub mod flags {
    pub const ALWAYS_SET: u32 = 0x00000001;
    pub const DATA: u32 = 0x00000100;
    pub const DATA_PRESENT: u32 = DATA;
    pub const HEARTBEAT_REQUEST: u32 = 0x00010000;
    pub const WANTING_REPLY: u32 = HEARTBEAT_REQUEST;
    pub const HEARTBEAT_REPLY: u32 = 0x00020000;
    pub const REPLY: u32 = HEARTBEAT_REPLY;
    pub const FILE_OPEN: u32 = 0x00100000;
    pub const FILE_TX_STREAM_REQUEST: u32 = FILE_OPEN;
    pub const FILE_TX_STREAM_RESPONSE: u32 = 0x00200000;
    pub const INIT_HANDSHAKE: u32 = 0x00400000;
}

/// An XPC message.
#[derive(Debug, Clone)]
pub struct XpcMessage {
    pub flags: u32,
    pub msg_id: u64,
    /// None when body_len == 0
    pub body: Option<XpcValue>,
}

/// XPC value variants (matches go-ios encoding.go type constants).
#[derive(Debug, Clone, PartialEq)]
pub enum XpcValue {
    Null,
    Bool(bool),
    Int64(i64),
    Uint64(u64),
    Double(f64),
    Date(i64),
    Data(Bytes),
    String(String),
    Uuid([u8; 16]),
    Array(Vec<XpcValue>),
    Dictionary(IndexMap<String, XpcValue>),
    FileTransfer { msg_id: u64, data: Box<XpcValue> },
}

impl XpcValue {
    pub fn as_str(&self) -> Option<&str> {
        if let XpcValue::String(s) = self {
            Some(s)
        } else {
            None
        }
    }
    pub fn as_dict(&self) -> Option<&IndexMap<String, XpcValue>> {
        if let XpcValue::Dictionary(d) = self {
            Some(d)
        } else {
            None
        }
    }
    pub fn as_uint64(&self) -> Option<u64> {
        if let XpcValue::Uint64(n) = self {
            Some(*n)
        } else {
            None
        }
    }

    pub fn as_file_transfer(&self) -> Option<(u64, &XpcValue)> {
        if let XpcValue::FileTransfer { msg_id, data } = self {
            Some((*msg_id, data.as_ref()))
        } else {
            None
        }
    }
}

// ── Type codes ─────────────────────────────────────────────────────────────────

const TYPE_NULL: u32 = 0x00001000;
const TYPE_BOOL: u32 = 0x00002000;
const TYPE_INT64: u32 = 0x00003000;
const TYPE_UINT64: u32 = 0x00004000;
const TYPE_DOUBLE: u32 = 0x00005000;
const TYPE_DATE: u32 = 0x00007000;
const TYPE_DATA: u32 = 0x00008000;
const TYPE_STRING: u32 = 0x00009000;
const TYPE_UUID: u32 = 0x0000A000;
const TYPE_ARRAY: u32 = 0x0000E000;
const TYPE_DICTIONARY: u32 = 0x0000F000;
const TYPE_FILE_TRANSFER: u32 = 0x0001A000;

// ── Encode ────────────────────────────────────────────────────────────────────

/// Encode an XPC message to bytes.
pub fn encode_message(msg: &XpcMessage) -> Result<Bytes, XpcError> {
    let mut body_buf = BytesMut::new();
    if let Some(body) = &msg.body {
        body_buf.put_u32_le(OBJECT_MAGIC);
        body_buf.put_u32_le(BODY_VERSION);
        encode_value(body, &mut body_buf)?;
    }

    let mut out = BytesMut::new();
    out.put_u32_le(WRAPPER_MAGIC);
    out.put_u32_le(msg.flags);
    out.put_u64_le(checked_u64_len("body", body_buf.len())?);
    out.put_u64_le(msg.msg_id);
    out.extend_from_slice(&body_buf);
    Ok(out.freeze())
}

fn encode_value(val: &XpcValue, out: &mut BytesMut) -> Result<(), XpcError> {
    match val {
        XpcValue::Null => {
            out.put_u32_le(TYPE_NULL);
        }
        XpcValue::Bool(b) => {
            out.put_u32_le(TYPE_BOOL);
            out.put_u8(if *b { 1 } else { 0 });
            out.put_u8(0);
            out.put_u8(0);
            out.put_u8(0);
        }
        XpcValue::Int64(n) => {
            out.put_u32_le(TYPE_INT64);
            out.put_i64_le(*n);
        }
        XpcValue::Uint64(n) => {
            out.put_u32_le(TYPE_UINT64);
            out.put_u64_le(*n);
        }
        XpcValue::Double(f) => {
            out.put_u32_le(TYPE_DOUBLE);
            out.put_f64_le(*f);
        }
        XpcValue::Date(n) => {
            out.put_u32_le(TYPE_DATE);
            out.put_i64_le(*n);
        }
        XpcValue::Data(d) => {
            out.put_u32_le(TYPE_DATA);
            out.put_u32_le(checked_u32_len("data", d.len())?);
            out.put_slice(d);
            let padded = checked_align4("data", d.len())?;
            for _ in d.len()..padded {
                out.put_u8(0);
            }
        }
        XpcValue::String(s) => {
            out.put_u32_le(TYPE_STRING);
            let raw = s.as_bytes();
            let total = raw
                .len()
                .checked_add(1)
                .ok_or_else(|| XpcError::Tls("XPC string length overflow".to_string()))?;
            out.put_u32_le(checked_u32_len("string", total)?);
            out.put_slice(raw);
            let padded = checked_align4("string", total)?;
            for _ in raw.len()..padded {
                out.put_u8(0);
            }
        }
        XpcValue::Uuid(u) => {
            out.put_u32_le(TYPE_UUID);
            out.put_slice(u); // no length field — matches Go wire format
        }
        XpcValue::Array(arr) => {
            out.put_u32_le(TYPE_ARRAY);
            let len_pos = out.len();
            out.put_u32_le(0); // placeholder
            let start = out.len();
            out.put_u32_le(checked_u32_len("array count", arr.len())?);
            for v in arr {
                encode_value(v, out)?;
            }
            let len_usize = out.len() - start;
            let len = checked_collection_len("array", len_usize)?;
            out[len_pos..len_pos + 4].copy_from_slice(&len.to_le_bytes());
        }
        XpcValue::Dictionary(map) => {
            out.put_u32_le(TYPE_DICTIONARY);
            let len_pos = out.len();
            out.put_u32_le(0); // placeholder
            let start = out.len();
            out.put_u32_le(checked_u32_len("dict count", map.len())?);
            for (k, v) in map {
                encode_dict_key(k, out)?;
                encode_value(v, out)?;
            }
            let len_usize = out.len() - start;
            let len = checked_collection_len("dict", len_usize)?;
            out[len_pos..len_pos + 4].copy_from_slice(&len.to_le_bytes());
        }
        XpcValue::FileTransfer { msg_id, data } => {
            out.put_u32_le(TYPE_FILE_TRANSFER);
            out.put_u64_le(*msg_id);
            encode_value(data, out)?;
        }
    }
    Ok(())
}

fn align4(n: usize) -> usize {
    (n + 3) & !3
}

fn checked_collection_len(kind: &str, len: usize) -> Result<u32, XpcError> {
    u32::try_from(len)
        .map_err(|_| XpcError::Tls(format!("XPC {kind} encoded size exceeds u32::MAX: {len}")))
}

fn checked_u32_len(kind: &str, len: usize) -> Result<u32, XpcError> {
    u32::try_from(len)
        .map_err(|_| XpcError::Tls(format!("XPC {kind} length exceeds u32::MAX: {len}")))
}

fn checked_u64_len(kind: &str, len: usize) -> Result<u64, XpcError> {
    u64::try_from(len)
        .map_err(|_| XpcError::Tls(format!("XPC {kind} length exceeds u64::MAX: {len}")))
}

fn checked_align4(kind: &str, len: usize) -> Result<usize, XpcError> {
    len.checked_add(3)
        .map(|value| value & !3)
        .ok_or_else(|| XpcError::Tls(format!("XPC {kind} padded length overflow: {len}")))
}

fn encode_dict_key(key: &str, out: &mut BytesMut) -> Result<(), XpcError> {
    let raw = key.as_bytes();
    out.put_slice(raw);
    out.put_u8(0);
    let total = raw
        .len()
        .checked_add(1)
        .ok_or_else(|| XpcError::Tls("XPC dict key length overflow".to_string()))?;
    let padded = checked_align4("dict key", total)?;
    for _ in total..padded {
        out.put_u8(0);
    }
    Ok(())
}

fn decode_dict_key(buf: &mut Bytes) -> Result<String, XpcError> {
    let nul_pos = buf
        .iter()
        .position(|&b| b == 0)
        .ok_or_else(|| XpcError::Tls("XPC: unterminated dictionary key".into()))?;
    let raw = buf.copy_to_bytes(nul_pos);
    if buf.remaining() < 1 {
        return Err(XpcError::Tls("XPC: dict key terminator truncated".into()));
    }
    buf.advance(1); // NUL terminator
    let total = nul_pos + 1;
    let padded = align4(total);
    let pad = padded - total;
    if buf.remaining() < pad {
        return Err(XpcError::Tls("XPC: dict key padding truncated".into()));
    }
    if pad > 0 {
        buf.advance(pad);
    }
    let s = std::str::from_utf8(&raw)
        .map_err(|_| XpcError::Tls("XPC: invalid UTF-8 in dict key".into()))?;
    Ok(s.to_string())
}

// ── Decode ────────────────────────────────────────────────────────────────────

/// Decode an XPC message from a byte buffer.
pub fn decode_message(mut buf: Bytes) -> Result<XpcMessage, XpcError> {
    if buf.remaining() < 4 {
        return Err(XpcError::Tls("XPC: buffer too short for magic".into()));
    }
    let magic = buf.get_u32_le();
    if magic != WRAPPER_MAGIC {
        return Err(XpcError::Tls(format!("XPC: bad magic 0x{magic:08X}")));
    }
    if buf.remaining() < 20 {
        return Err(XpcError::Tls("XPC: buffer too short for header".into()));
    }
    let flags = buf.get_u32_le();
    let body_len = buf.get_u64_le() as usize;
    let msg_id = buf.get_u64_le();

    let body = if body_len > 0 {
        if buf.remaining() < body_len {
            return Err(XpcError::Tls("XPC: body truncated".into()));
        }
        let mut body_buf = buf.copy_to_bytes(body_len);
        // Body header is object magic followed by protocol version.
        if body_buf.remaining() >= 8 {
            let obj_magic = body_buf.get_u32_le();
            if obj_magic != OBJECT_MAGIC {
                return Err(XpcError::Tls(format!(
                    "XPC: bad object magic 0x{obj_magic:08X}"
                )));
            }
            let version = body_buf.get_u32_le();
            if version != BODY_VERSION {
                return Err(XpcError::Tls(format!(
                    "XPC: bad body version 0x{version:08X}"
                )));
            }
            Some(decode_value(&mut body_buf)?)
        } else {
            None
        }
    } else {
        None
    };

    Ok(XpcMessage {
        flags,
        msg_id,
        body,
    })
}

/// Incrementally reassembles complete XPC messages from DATA frame payloads.
#[derive(Debug, Default)]
pub(crate) struct XpcMessageBuffer {
    pending: BytesMut,
}

impl XpcMessageBuffer {
    pub(crate) fn new() -> Self {
        Self {
            pending: BytesMut::new(),
        }
    }

    pub(crate) fn push(&mut self, bytes: &[u8]) {
        self.pending.extend_from_slice(bytes);
    }

    pub(crate) fn try_next(&mut self) -> Result<Option<XpcMessage>, XpcError> {
        if self.pending.len() < 24 {
            return Ok(None);
        }

        let body_len = u64::from_le_bytes(
            self.pending[8..16]
                .try_into()
                .map_err(|_| XpcError::Tls("XPC: invalid wrapper header".into()))?,
        ) as usize;
        let total_len = 24usize
            .checked_add(body_len)
            .ok_or_else(|| XpcError::Tls("XPC: message length overflow".into()))?;
        if self.pending.len() < total_len {
            return Ok(None);
        }

        let payload = self.pending.split_to(total_len).freeze();
        decode_message(payload).map(Some)
    }
}

fn decode_value(buf: &mut Bytes) -> Result<XpcValue, XpcError> {
    if buf.remaining() < 4 {
        return Err(XpcError::Tls("XPC: value too short".into()));
    }
    let type_tag = buf.get_u32_le();

    match type_tag {
        TYPE_NULL => Ok(XpcValue::Null),
        TYPE_BOOL => {
            if buf.remaining() < 4 {
                return Err(XpcError::Tls("XPC: bool truncated".into()));
            }
            let value = buf.get_u8() != 0;
            buf.advance(3);
            Ok(XpcValue::Bool(value))
        }
        TYPE_INT64 => {
            if buf.remaining() < 8 {
                return Err(XpcError::Tls("XPC: i64 truncated".into()));
            }
            Ok(XpcValue::Int64(buf.get_i64_le()))
        }
        TYPE_UINT64 => {
            if buf.remaining() < 8 {
                return Err(XpcError::Tls("XPC: u64 truncated".into()));
            }
            Ok(XpcValue::Uint64(buf.get_u64_le()))
        }
        TYPE_DOUBLE => {
            if buf.remaining() < 8 {
                return Err(XpcError::Tls("XPC: f64 truncated".into()));
            }
            Ok(XpcValue::Double(buf.get_f64_le()))
        }
        TYPE_DATE => {
            if buf.remaining() < 8 {
                return Err(XpcError::Tls("XPC: date truncated".into()));
            }
            Ok(XpcValue::Date(buf.get_i64_le()))
        }
        TYPE_DATA => {
            if buf.remaining() < 4 {
                return Err(XpcError::Tls("XPC: data length truncated".into()));
            }
            let data_len = buf.get_u32_le() as usize;
            let padded = align4(data_len);
            if buf.remaining() < padded {
                return Err(XpcError::Tls("XPC: data truncated".into()));
            }
            let data = buf.copy_to_bytes(data_len);
            let pad = padded - data_len;
            if pad > 0 {
                buf.advance(pad);
            }
            Ok(XpcValue::Data(data))
        }
        TYPE_STRING => {
            if buf.remaining() < 4 {
                return Err(XpcError::Tls("XPC: string length truncated".into()));
            }
            let data_len = buf.get_u32_le() as usize;
            let padded = align4(data_len);
            if buf.remaining() < padded {
                return Err(XpcError::Tls("XPC: string truncated".into()));
            }
            let raw = buf.copy_to_bytes(data_len);
            let pad = padded - data_len;
            if pad > 0 {
                buf.advance(pad);
            }
            let end = raw.iter().position(|&b| b == 0).unwrap_or(raw.len());
            let s = std::str::from_utf8(&raw[..end])
                .map_err(|_| XpcError::Tls("XPC: invalid UTF-8 in string".into()))?;
            Ok(XpcValue::String(s.to_string()))
        }
        TYPE_UUID => {
            if buf.remaining() < 16 {
                return Err(XpcError::Tls("XPC: uuid truncated".into()));
            }
            let mut u = [0u8; 16];
            buf.copy_to_slice(&mut u);
            Ok(XpcValue::Uuid(u))
        }
        TYPE_ARRAY => {
            if buf.remaining() < 8 {
                return Err(XpcError::Tls("XPC: array header truncated".into()));
            }
            let _data_len = buf.get_u32_le() as usize;
            if buf.remaining() < 4 {
                return Err(XpcError::Tls("XPC: array count truncated".into()));
            }
            let count = buf.get_u32_le() as usize;
            const MAX_XPC_COLLECTION_SIZE: usize = 65536;
            if count > MAX_XPC_COLLECTION_SIZE {
                return Err(XpcError::Tls(format!("XPC collection too large: {count}")));
            }
            let mut arr = Vec::with_capacity(count.min(256));
            for _ in 0..count {
                arr.push(decode_value(buf)?);
            }
            Ok(XpcValue::Array(arr))
        }
        TYPE_DICTIONARY => {
            if buf.remaining() < 8 {
                return Err(XpcError::Tls("XPC: dict header truncated".into()));
            }
            let _data_len = buf.get_u32_le() as usize;
            if buf.remaining() < 4 {
                return Err(XpcError::Tls("XPC: dict count truncated".into()));
            }
            let count = buf.get_u32_le() as usize;
            const MAX_XPC_COLLECTION_SIZE: usize = 65536;
            if count > MAX_XPC_COLLECTION_SIZE {
                return Err(XpcError::Tls(format!("XPC collection too large: {count}")));
            }
            let mut map = IndexMap::with_capacity(count.min(256));
            for _ in 0..count {
                let key = decode_dict_key(buf)?;
                let val = decode_value(buf)?;
                map.insert(key, val);
            }
            Ok(XpcValue::Dictionary(map))
        }
        TYPE_FILE_TRANSFER => {
            if buf.remaining() < 8 {
                return Err(XpcError::Tls("XPC: file transfer truncated".into()));
            }
            let msg_id = buf.get_u64_le();
            let data = decode_value(buf)?;
            Ok(XpcValue::FileTransfer {
                msg_id,
                data: Box::new(data),
            })
        }
        other => Err(XpcError::Tls(format!("XPC: unknown type 0x{other:08X}"))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn roundtrip(val: XpcValue) -> XpcValue {
        let msg = XpcMessage {
            flags: flags::ALWAYS_SET | flags::DATA,
            msg_id: 1,
            body: Some(val),
        };
        let bytes = encode_message(&msg).unwrap();
        decode_message(bytes).unwrap().body.unwrap()
    }

    #[test]
    fn test_xpc_string_roundtrip() {
        let v = roundtrip(XpcValue::String("hello".into()));
        assert_eq!(v.as_str(), Some("hello"));
    }

    #[test]
    fn test_xpc_uint64_roundtrip() {
        let v = roundtrip(XpcValue::Uint64(12345678));
        assert_eq!(v.as_uint64(), Some(12345678));
    }

    #[test]
    fn test_xpc_dict_roundtrip() {
        let mut map = IndexMap::new();
        map.insert("key1".to_string(), XpcValue::String("val1".into()));
        map.insert("key2".to_string(), XpcValue::Uint64(99));
        let v = roundtrip(XpcValue::Dictionary(map));
        let d = v.as_dict().unwrap();
        assert_eq!(d["key1"].as_str(), Some("val1"));
        assert_eq!(d["key2"].as_uint64(), Some(99));
    }

    #[test]
    fn test_xpc_no_body() {
        let msg = XpcMessage {
            flags: flags::ALWAYS_SET,
            msg_id: 7,
            body: None,
        };
        let bytes = encode_message(&msg).unwrap();
        let decoded = decode_message(bytes).unwrap();
        assert_eq!(decoded.msg_id, 7);
        assert!(decoded.body.is_none());
    }

    #[test]
    fn test_xpc_file_transfer_roundtrip() {
        let v = roundtrip(XpcValue::FileTransfer {
            msg_id: 9,
            data: Box::new(XpcValue::Dictionary(IndexMap::from([(
                "s".to_string(),
                XpcValue::Uint64(4096),
            )]))),
        });

        let (msg_id, data) = v.as_file_transfer().unwrap();
        assert_eq!(msg_id, 9);
        assert_eq!(
            data.as_dict()
                .and_then(|dict| dict.get("s"))
                .and_then(XpcValue::as_uint64),
            Some(4096)
        );
    }

    #[test]
    fn collection_length_rejects_values_above_u32_max() {
        let err = checked_collection_len("array", u32::MAX as usize + 1).unwrap_err();
        assert!(err
            .to_string()
            .contains("array encoded size exceeds u32::MAX"));
    }

    #[test]
    fn checked_xpc_u32_len_rejects_values_above_u32_max() {
        let err = checked_u32_len("data", u32::MAX as usize + 1).unwrap_err();
        assert!(err.to_string().contains("data length exceeds u32::MAX"));
    }

    #[test]
    fn message_buffer_reassembles_fragmented_messages() {
        let msg1 = XpcMessage {
            flags: flags::ALWAYS_SET | flags::DATA,
            msg_id: 1,
            body: Some(XpcValue::String("one".into())),
        };
        let msg2 = XpcMessage {
            flags: flags::ALWAYS_SET | flags::DATA,
            msg_id: 2,
            body: Some(XpcValue::String("two".into())),
        };
        let bytes1 = encode_message(&msg1).unwrap();
        let bytes2 = encode_message(&msg2).unwrap();
        let mut buffer = XpcMessageBuffer::new();

        buffer.push(&bytes1[..10]);
        assert!(buffer.try_next().unwrap().is_none());

        buffer.push(&bytes1[10..]);
        buffer.push(&bytes2);

        assert_eq!(buffer.try_next().unwrap().unwrap().msg_id, 1);
        assert_eq!(buffer.try_next().unwrap().unwrap().msg_id, 2);
        assert!(buffer.try_next().unwrap().is_none());
    }
}
