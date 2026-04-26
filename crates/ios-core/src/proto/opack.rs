/// opack encoder/decoder.
///
/// opack is a compact binary serialization format used by Apple for SRP pairing.
/// Format:
///   - Dictionary: 0xE0 | count, then count * (key, value) pairs
///   - String:     0x40 | len (if len < 0x20), then bytes; or 0x61 + u8 len + bytes
///   - Bytes:      0x70..0x7F (len 0..15), 0x80..0x8F (len 16..31),
///     or 0x91 + u8 len + bytes
///   - Integer:    0x08..0x0F (single byte values), 0x30 (i8), 0x31 (i16 le), etc.
///   - Bool:       0x01 = true, 0x02 = false
///   - Null:       0x04
#[derive(Debug, Clone, PartialEq)]
pub enum OpackValue {
    Null,
    Bool(bool),
    Int(i64),
    String(String),
    Bytes(Vec<u8>),
    Array(Vec<OpackValue>),
    Dict(Vec<(OpackValue, OpackValue)>),
}

#[derive(Debug, thiserror::Error)]
pub enum OpackError {
    #[error("unexpected end of buffer")]
    UnexpectedEof,
    #[error("unknown opack tag: 0x{0:02X}")]
    UnknownTag(u8),
    #[error("invalid UTF-8 in string")]
    InvalidUtf8,
    #[error("opack encode error: {0}")]
    Encode(String),
}

/// Encode an OpackValue into bytes.
pub fn encode(value: &OpackValue) -> Result<Vec<u8>, OpackError> {
    let mut out = Vec::new();
    encode_value(value, &mut out)?;
    Ok(out)
}

fn encode_value(value: &OpackValue, out: &mut Vec<u8>) -> Result<(), OpackError> {
    match value {
        OpackValue::Null => out.push(0x04),
        OpackValue::Bool(true) => out.push(0x01),
        OpackValue::Bool(false) => out.push(0x02),
        OpackValue::Int(n) => {
            if *n >= 0 && *n < 8 {
                out.push(0x08 + *n as u8);
            } else if *n >= i8::MIN as i64 && *n <= i8::MAX as i64 {
                out.push(0x30);
                out.push(*n as i8 as u8);
            } else if *n >= i16::MIN as i64 && *n <= i16::MAX as i64 {
                out.push(0x31);
                out.extend_from_slice(&(*n as i16).to_le_bytes());
            } else {
                out.push(0x33);
                out.extend_from_slice(&n.to_le_bytes());
            }
        }
        OpackValue::String(s) => {
            let bytes = s.as_bytes();
            if bytes.len() < 0x20 {
                out.push(0x40 | bytes.len() as u8);
            } else if bytes.len() <= 0xFF {
                out.push(0x61);
                out.push(bytes.len() as u8);
            } else {
                if bytes.len() > u16::MAX as usize {
                    return Err(OpackError::Encode(format!(
                        "string too long: {} bytes (max {})",
                        bytes.len(),
                        u16::MAX
                    )));
                }
                out.push(0x62);
                out.extend_from_slice(&(bytes.len() as u16).to_le_bytes());
            }
            out.extend_from_slice(bytes);
        }
        OpackValue::Bytes(b) => {
            let len = b.len();
            if len <= 0x0F {
                out.push(0x70 | len as u8);
            } else if len < 0x20 {
                out.push(0x80 | (len as u8 & 0x0F));
            } else if len <= 0xFF {
                out.push(0x91);
                out.push(len as u8);
            } else {
                return Err(OpackError::Encode(format!(
                    "bytes too long: {len} bytes (max {})",
                    u8::MAX
                )));
            }
            out.extend_from_slice(b);
        }
        OpackValue::Array(arr) => {
            if arr.len() > 0x0F {
                return Err(OpackError::Encode(format!(
                    "array too large: {} elements (max 15)",
                    arr.len()
                )));
            }
            out.push(0xD0 | (arr.len() as u8));
            for v in arr {
                encode_value(v, out)?;
            }
        }
        OpackValue::Dict(pairs) => {
            if pairs.len() > 0x0F {
                return Err(OpackError::Encode(format!(
                    "dict too large: {} entries (max 15)",
                    pairs.len()
                )));
            }
            out.push(0xE0 | (pairs.len() as u8));
            for (k, v) in pairs {
                encode_value(k, out)?;
                encode_value(v, out)?;
            }
        }
    }
    Ok(())
}

/// Decode an OpackValue from bytes.
pub fn decode(buf: &[u8]) -> Result<(OpackValue, usize), OpackError> {
    decode_at(buf, 0)
}

fn decode_at(buf: &[u8], pos: usize) -> Result<(OpackValue, usize), OpackError> {
    if pos >= buf.len() {
        return Err(OpackError::UnexpectedEof);
    }
    let tag = buf[pos];
    match tag {
        0x04 => Ok((OpackValue::Null, pos + 1)),
        0x01 => Ok((OpackValue::Bool(true), pos + 1)),
        0x02 => Ok((OpackValue::Bool(false), pos + 1)),
        0x08..=0x0F => Ok((OpackValue::Int((tag - 0x08) as i64), pos + 1)),
        0x30 => {
            if pos + 2 > buf.len() {
                return Err(OpackError::UnexpectedEof);
            }
            Ok((OpackValue::Int(buf[pos + 1] as i8 as i64), pos + 2))
        }
        0x31 => {
            if pos + 3 > buf.len() {
                return Err(OpackError::UnexpectedEof);
            }
            let n = i16::from_le_bytes([buf[pos + 1], buf[pos + 2]]);
            Ok((OpackValue::Int(n as i64), pos + 3))
        }
        0x33 => {
            if pos + 9 > buf.len() {
                return Err(OpackError::UnexpectedEof);
            }
            let mut arr = [0u8; 8];
            arr.copy_from_slice(&buf[pos + 1..pos + 9]);
            Ok((OpackValue::Int(i64::from_le_bytes(arr)), pos + 9))
        }
        0x40..=0x5F => {
            let len = (tag - 0x40) as usize;
            if pos + 1 + len > buf.len() {
                return Err(OpackError::UnexpectedEof);
            }
            let s = std::str::from_utf8(&buf[pos + 1..pos + 1 + len])
                .map_err(|_| OpackError::InvalidUtf8)?;
            Ok((OpackValue::String(s.to_string()), pos + 1 + len))
        }
        0x61 => {
            if pos + 2 > buf.len() {
                return Err(OpackError::UnexpectedEof);
            }
            let len = buf[pos + 1] as usize;
            if pos + 2 + len > buf.len() {
                return Err(OpackError::UnexpectedEof);
            }
            let s = std::str::from_utf8(&buf[pos + 2..pos + 2 + len])
                .map_err(|_| OpackError::InvalidUtf8)?;
            Ok((OpackValue::String(s.to_string()), pos + 2 + len))
        }
        0x62 => {
            if pos + 3 > buf.len() {
                return Err(OpackError::UnexpectedEof);
            }
            let len = u16::from_le_bytes([buf[pos + 1], buf[pos + 2]]) as usize;
            if pos + 3 + len > buf.len() {
                return Err(OpackError::UnexpectedEof);
            }
            let s = std::str::from_utf8(&buf[pos + 3..pos + 3 + len])
                .map_err(|_| OpackError::InvalidUtf8)?;
            Ok((OpackValue::String(s.to_string()), pos + 3 + len))
        }
        0x70..=0x7F => {
            let len = (tag - 0x70) as usize;
            if pos + 1 + len > buf.len() {
                return Err(OpackError::UnexpectedEof);
            }
            Ok((
                OpackValue::Bytes(buf[pos + 1..pos + 1 + len].to_vec()),
                pos + 1 + len,
            ))
        }
        0x80..=0x8F => {
            let len = 0x10 + (tag - 0x80) as usize;
            if pos + 1 + len > buf.len() {
                return Err(OpackError::UnexpectedEof);
            }
            Ok((
                OpackValue::Bytes(buf[pos + 1..pos + 1 + len].to_vec()),
                pos + 1 + len,
            ))
        }
        0x91 => {
            if pos + 2 > buf.len() {
                return Err(OpackError::UnexpectedEof);
            }
            let len = buf[pos + 1] as usize;
            if pos + 2 + len > buf.len() {
                return Err(OpackError::UnexpectedEof);
            }
            Ok((
                OpackValue::Bytes(buf[pos + 2..pos + 2 + len].to_vec()),
                pos + 2 + len,
            ))
        }
        0xD0..=0xDF => {
            let count = (tag - 0xD0) as usize;
            let mut items = Vec::with_capacity(count);
            let mut cur = pos + 1;
            for _ in 0..count {
                let (v, next) = decode_at(buf, cur)?;
                items.push(v);
                cur = next;
            }
            Ok((OpackValue::Array(items), cur))
        }
        0xE0..=0xEF => {
            let count = (tag - 0xE0) as usize;
            let mut pairs = Vec::with_capacity(count);
            let mut cur = pos + 1;
            for _ in 0..count {
                let (k, next) = decode_at(buf, cur)?;
                cur = next;
                let (v, next) = decode_at(buf, cur)?;
                cur = next;
                pairs.push((k, v));
            }
            Ok((OpackValue::Dict(pairs), cur))
        }
        _ => Err(OpackError::UnknownTag(tag)),
    }
}

#[cfg(test)]
mod tests {
    use super::{decode, encode, OpackValue};

    fn roundtrip(value: OpackValue) -> OpackValue {
        let encoded = encode(&value).expect("encode should succeed");
        let (decoded, used) = decode(&encoded).expect("decode should succeed");
        assert_eq!(used, encoded.len());
        decoded
    }

    fn sample_bytes(len: usize) -> Vec<u8> {
        (0..len).map(|i| i as u8).collect()
    }

    #[test]
    fn roundtrips_scalars() {
        assert_eq!(roundtrip(OpackValue::Null), OpackValue::Null);
        assert_eq!(roundtrip(OpackValue::Bool(true)), OpackValue::Bool(true));
        assert_eq!(roundtrip(OpackValue::Bool(false)), OpackValue::Bool(false));
        assert_eq!(roundtrip(OpackValue::Int(0)), OpackValue::Int(0));
        assert_eq!(roundtrip(OpackValue::Int(7)), OpackValue::Int(7));
        assert_eq!(roundtrip(OpackValue::Int(0xff)), OpackValue::Int(0xff));
        assert_eq!(
            roundtrip(OpackValue::Int(i64::MIN)),
            OpackValue::Int(i64::MIN)
        );
    }

    #[test]
    fn roundtrips_strings_and_bytes() {
        assert_eq!(
            roundtrip(OpackValue::String(String::new())),
            OpackValue::String(String::new())
        );
        assert_eq!(
            roundtrip(OpackValue::String("short string".into())),
            OpackValue::String("short string".into())
        );
        assert_eq!(
            roundtrip(OpackValue::String("x".repeat(32))),
            OpackValue::String("x".repeat(32))
        );
    }

    #[test]
    fn encodes_bytes_with_reference_tags() {
        let cases = [
            (0usize, vec![0x70]),
            (1, vec![0x71]),
            (15, vec![0x7F]),
            (16, vec![0x80]),
            (31, vec![0x8F]),
            (32, vec![0x91, 0x20]),
        ];

        for (len, expected_prefix) in cases {
            let bytes = sample_bytes(len);
            let mut expected = expected_prefix;
            expected.extend_from_slice(&bytes);
            assert_eq!(encode(&OpackValue::Bytes(bytes)).unwrap(), expected);
        }
    }

    #[test]
    fn decodes_bytes_with_reference_tags() {
        let cases = [
            (vec![0x70], 0usize),
            (
                {
                    let bytes = sample_bytes(1);
                    let mut encoded = vec![0x71];
                    encoded.extend_from_slice(&bytes);
                    encoded
                },
                1usize,
            ),
            (
                {
                    let bytes = sample_bytes(15);
                    let mut encoded = vec![0x7F];
                    encoded.extend_from_slice(&bytes);
                    encoded
                },
                15usize,
            ),
            (
                {
                    let bytes = sample_bytes(16);
                    let mut encoded = vec![0x80];
                    encoded.extend_from_slice(&bytes);
                    encoded
                },
                16usize,
            ),
            (
                {
                    let bytes = sample_bytes(31);
                    let mut encoded = vec![0x8F];
                    encoded.extend_from_slice(&bytes);
                    encoded
                },
                31usize,
            ),
            (
                {
                    let bytes = sample_bytes(32);
                    let mut encoded = vec![0x91, 0x20];
                    encoded.extend_from_slice(&bytes);
                    encoded
                },
                32usize,
            ),
        ];

        for (encoded, len) in cases {
            let (decoded, used) = decode(&encoded).expect("decode should succeed");
            assert_eq!(decoded, OpackValue::Bytes(sample_bytes(len)));
            assert_eq!(used, encoded.len());
        }
    }

    #[test]
    fn roundtrips_collections_with_fifteen_entries() {
        let array = OpackValue::Array((0..15).map(OpackValue::Int).collect());
        assert_eq!(roundtrip(array.clone()), array);

        let dict = OpackValue::Dict(
            (0..15)
                .map(|i| (OpackValue::String(format!("k{i}")), OpackValue::Int(i)))
                .collect(),
        );
        assert_eq!(roundtrip(dict.clone()), dict);
    }

    #[test]
    fn rejects_arrays_and_dicts_larger_than_reference_limit() {
        let array = OpackValue::Array((0..16).map(OpackValue::Int).collect());
        let err = encode(&array).unwrap_err();
        assert!(err.to_string().contains("array too large"));

        let dict = OpackValue::Dict(
            (0..16)
                .map(|i| (OpackValue::String(format!("k{i}")), OpackValue::Int(i)))
                .collect(),
        );
        let err = encode(&dict).unwrap_err();
        assert!(err.to_string().contains("dict too large"));
    }
}
