use bytes::{Buf, BufMut, Bytes, BytesMut};
use indexmap::IndexMap;

/// XPC binary message magic numbers
pub const XPC_MAGIC: u32 = 0x29B00B92;
pub const XPC_BODY_MAGIC: u32 = 0x42133742;
pub const XPC_ALWAYS_SET: u32 = 0x00000001;
pub const XPC_DATA_LEN_OFFSET: u32 = 0x00000002; // always-set mask for data presence

/// XPC value types
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
}

/// A parsed XPC message frame.
#[derive(Debug, Clone)]
pub struct XpcMessage {
    pub flags: u32,
    pub msg_id: u64,
    pub body: Option<XpcValue>,
}

#[derive(Debug, thiserror::Error)]
pub enum XpcError {
    #[error("buffer too short")]
    BufferTooShort,
    #[error("bad magic: expected 0x{expected:08X}, got 0x{got:08X}")]
    BadMagic { expected: u32, got: u32 },
    #[error("unknown XPC type: 0x{0:08X}")]
    UnknownType(u32),
    #[error("invalid UTF-8 in string")]
    InvalidUtf8,
}

// XPC type tags
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

/// Encode an XPC message to bytes.
pub fn encode_xpc(msg: &XpcMessage) -> Bytes {
    let mut buf = BytesMut::new();
    buf.put_u32_le(XPC_MAGIC);
    buf.put_u32_le(msg.flags);
    buf.put_u64_le(msg.msg_id);

    if let Some(body) = &msg.body {
        let body_bytes = encode_body(body);
        buf.put_u32_le(body_bytes.len() as u32);
        buf.put_slice(&body_bytes);
    } else {
        buf.put_u32_le(0);
    }

    buf.freeze()
}

fn encode_body(value: &XpcValue) -> BytesMut {
    let mut buf = BytesMut::new();
    buf.put_u32_le(XPC_BODY_MAGIC);
    encode_value(value, &mut buf);
    buf
}

fn encode_value(value: &XpcValue, buf: &mut BytesMut) {
    match value {
        XpcValue::Null => {
            buf.put_u32_le(TYPE_NULL);
            buf.put_u32_le(0);
        }
        XpcValue::Bool(b) => {
            buf.put_u32_le(TYPE_BOOL);
            buf.put_u32_le(4);
            buf.put_u32_le(if *b { 1 } else { 0 });
            // pad to 4-byte boundary (already aligned)
        }
        XpcValue::Int64(n) => {
            buf.put_u32_le(TYPE_INT64);
            buf.put_u32_le(8);
            buf.put_i64_le(*n);
        }
        XpcValue::Uint64(n) => {
            buf.put_u32_le(TYPE_UINT64);
            buf.put_u32_le(8);
            buf.put_u64_le(*n);
        }
        XpcValue::Double(f) => {
            buf.put_u32_le(TYPE_DOUBLE);
            buf.put_u32_le(8);
            buf.put_f64_le(*f);
        }
        XpcValue::Date(n) => {
            buf.put_u32_le(TYPE_DATE);
            buf.put_u32_le(8);
            buf.put_i64_le(*n);
        }
        XpcValue::Data(d) => {
            buf.put_u32_le(TYPE_DATA);
            let padded = (d.len() + 3) & !3;
            buf.put_u32_le(d.len() as u32);
            buf.put_slice(d);
            for _ in d.len()..padded {
                buf.put_u8(0);
            }
        }
        XpcValue::String(s) => {
            buf.put_u32_le(TYPE_STRING);
            let bytes = s.as_bytes();
            let total = bytes.len() + 1; // null terminator
            let padded = (total + 3) & !3;
            buf.put_u32_le(total as u32);
            buf.put_slice(bytes);
            for _ in bytes.len()..padded {
                buf.put_u8(0);
            }
        }
        XpcValue::Uuid(u) => {
            buf.put_u32_le(TYPE_UUID);
            buf.put_u32_le(16);
            buf.put_slice(u);
        }
        XpcValue::Array(arr) => {
            buf.put_u32_le(TYPE_ARRAY);
            let len_offset = buf.len();
            buf.put_u32_le(0); // placeholder
            let start = buf.len();
            buf.put_u32_le(arr.len() as u32);
            for v in arr {
                encode_value(v, buf);
            }
            let len = (buf.len() - start) as u32;
            buf[len_offset..len_offset + 4].copy_from_slice(&len.to_le_bytes());
        }
        XpcValue::Dictionary(map) => {
            buf.put_u32_le(TYPE_DICTIONARY);
            let len_offset = buf.len();
            buf.put_u32_le(0); // placeholder
            let start = buf.len();
            buf.put_u32_le(map.len() as u32);
            for (k, v) in map {
                encode_value(&XpcValue::String(k.clone()), buf);
                encode_value(v, buf);
            }
            let len = (buf.len() - start) as u32;
            buf[len_offset..len_offset + 4].copy_from_slice(&len.to_le_bytes());
        }
    }
}

/// Decode an XPC message from a byte buffer.
pub fn decode_xpc(buf: &mut Bytes) -> Result<XpcMessage, XpcError> {
    if buf.remaining() < 16 {
        return Err(XpcError::BufferTooShort);
    }
    let magic = buf.get_u32_le();
    if magic != XPC_MAGIC {
        return Err(XpcError::BadMagic {
            expected: XPC_MAGIC,
            got: magic,
        });
    }
    let flags = buf.get_u32_le();
    let msg_id = buf.get_u64_le();

    if buf.remaining() < 4 {
        return Err(XpcError::BufferTooShort);
    }
    let body_len = buf.get_u32_le() as usize;

    let body = if body_len > 0 {
        if buf.remaining() < body_len {
            return Err(XpcError::BufferTooShort);
        }
        let body_magic = buf.get_u32_le();
        if body_magic != XPC_BODY_MAGIC {
            return Err(XpcError::BadMagic {
                expected: XPC_BODY_MAGIC,
                got: body_magic,
            });
        }
        Some(decode_value(buf)?)
    } else {
        None
    };

    Ok(XpcMessage {
        flags,
        msg_id,
        body,
    })
}

fn decode_value(buf: &mut Bytes) -> Result<XpcValue, XpcError> {
    if buf.remaining() < 8 {
        return Err(XpcError::BufferTooShort);
    }
    let type_tag = buf.get_u32_le();
    let data_len = buf.get_u32_le() as usize;

    match type_tag {
        TYPE_NULL => Ok(XpcValue::Null),
        TYPE_BOOL => {
            if buf.remaining() < 4 {
                return Err(XpcError::BufferTooShort);
            }
            let v = buf.get_u32_le();
            Ok(XpcValue::Bool(v != 0))
        }
        TYPE_INT64 => {
            if buf.remaining() < 8 {
                return Err(XpcError::BufferTooShort);
            }
            Ok(XpcValue::Int64(buf.get_i64_le()))
        }
        TYPE_UINT64 => {
            if buf.remaining() < 8 {
                return Err(XpcError::BufferTooShort);
            }
            Ok(XpcValue::Uint64(buf.get_u64_le()))
        }
        TYPE_DOUBLE => {
            if buf.remaining() < 8 {
                return Err(XpcError::BufferTooShort);
            }
            Ok(XpcValue::Double(buf.get_f64_le()))
        }
        TYPE_DATE => {
            if buf.remaining() < 8 {
                return Err(XpcError::BufferTooShort);
            }
            Ok(XpcValue::Date(buf.get_i64_le()))
        }
        TYPE_DATA => {
            let padded = (data_len + 3) & !3;
            if buf.remaining() < padded {
                return Err(XpcError::BufferTooShort);
            }
            let d = buf.copy_to_bytes(data_len);
            // skip padding bytes
            let pad = padded - data_len;
            if pad > 0 {
                buf.advance(pad);
            }
            Ok(XpcValue::Data(d))
        }
        TYPE_STRING => {
            let padded = (data_len + 3) & !3;
            if buf.remaining() < padded {
                return Err(XpcError::BufferTooShort);
            }
            let raw = buf.copy_to_bytes(data_len);
            // skip padding bytes
            let pad = padded - data_len;
            if pad > 0 {
                buf.advance(pad);
            }
            // Find null terminator
            let end = raw.iter().position(|&b| b == 0).unwrap_or(raw.len());
            let s = std::str::from_utf8(&raw[..end]).map_err(|_| XpcError::InvalidUtf8)?;
            Ok(XpcValue::String(s.to_string()))
        }
        TYPE_UUID => {
            if buf.remaining() < 16 {
                return Err(XpcError::BufferTooShort);
            }
            let mut u = [0u8; 16];
            buf.copy_to_slice(&mut u);
            Ok(XpcValue::Uuid(u))
        }
        TYPE_ARRAY => {
            if buf.remaining() < 4 {
                return Err(XpcError::BufferTooShort);
            }
            let count = buf.get_u32_le() as usize;
            let mut arr = Vec::with_capacity(count);
            for _ in 0..count {
                arr.push(decode_value(buf)?);
            }
            Ok(XpcValue::Array(arr))
        }
        TYPE_DICTIONARY => {
            if buf.remaining() < 4 {
                return Err(XpcError::BufferTooShort);
            }
            let count = buf.get_u32_le() as usize;
            let mut map = IndexMap::new();
            for _ in 0..count {
                let key = match decode_value(buf)? {
                    XpcValue::String(s) => s,
                    _ => return Err(XpcError::UnknownType(0)),
                };
                let val = decode_value(buf)?;
                map.insert(key, val);
            }
            Ok(XpcValue::Dictionary(map))
        }
        other => Err(XpcError::UnknownType(other)),
    }
}
