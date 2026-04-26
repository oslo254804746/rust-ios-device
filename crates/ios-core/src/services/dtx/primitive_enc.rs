//! DTX PrimitiveDictionary encoder.
//!
//! Reference: go-ios/ios/dtx_codec/dtxprimitivedictionary.go
//!
//! Wire format per entry:
//!   key is always `[t_null]`
//!   values are encoded like go-ios `writeEntry()`:
//!   `t_uint32` => `[type][u32]`
//!   `t_int64`  => `[type][i64]`
//!   `t_bytearray` => `[type][len][bytes]`

use bytes::{BufMut, Bytes, BytesMut};

// Type codes
const T_NULL: u32 = 0x0000000A;
const T_UINT32: u32 = 0x00000003;
const T_INT64: u32 = 0x00000006;
const T_DOUBLE: u32 = 0x00000009;
const T_BYTEARRAY: u32 = 0x00000002;

/// Argument value for a DTX PrimitiveDictionary entry.
#[derive(Debug, Clone)]
pub enum PrimArg {
    /// Integer (stored as u32 / 4 bytes)
    Int32(i32),
    /// Long integer (stored as i64 / 8 bytes)
    Int64(i64),
    /// Double float
    Double(f64),
    /// Raw bytes (e.g. NSKeyedArchiver data)
    Bytes(Bytes),
}

/// Encode a list of arguments into a DTX PrimitiveDictionary byte array.
///
/// Wire format (matches go-ios ToBytes() exactly — no array header):
///   For each entry: [key_type=0x0A (4 LE)] [value_type (4 LE)] [value_data]
///   t_uint32:    [0x0A][0x03][uint32 (4 LE)]
///   t_bytearray: [0x0A][0x02][length (4 LE)][data]
pub fn encode_primitive_dict(args: &[PrimArg]) -> Bytes {
    let mut out = BytesMut::new();

    for arg in args {
        // Key: t_null (0x0A)
        out.put_u32_le(T_NULL);
        match arg {
            PrimArg::Int32(v) => {
                out.put_u32_le(T_UINT32);
                out.put_u32_le(*v as u32);
            }
            PrimArg::Int64(v) => {
                out.put_u32_le(T_INT64);
                out.put_i64_le(*v);
            }
            PrimArg::Double(v) => {
                out.put_u32_le(T_DOUBLE);
                out.put_f64_le(*v);
            }
            PrimArg::Bytes(b) => {
                out.put_u32_le(T_BYTEARRAY);
                out.put_u32_le(b.len() as u32);
                out.put_slice(b);
            }
        }
    }

    out.freeze()
}

/// Helper: wrap an NSKeyedArchiver blob as a PrimArg::Bytes entry.
pub fn archived_object(data: impl Into<Bytes>) -> PrimArg {
    PrimArg::Bytes(data.into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_empty_args() {
        let b = encode_primitive_dict(&[]);
        assert!(b.is_empty());
    }

    #[test]
    fn test_int32_arg() {
        let b = encode_primitive_dict(&[PrimArg::Int32(42)]);
        // 4 null + 4 type + 4 value = 12 bytes (no array header — matches go-ios ToBytes())
        assert_eq!(b.len(), 12);
        // Key type (T_NULL) at offset 0
        assert_eq!(u32::from_le_bytes(b[0..4].try_into().unwrap()), T_NULL);
        // Value type (T_UINT32) at offset 4
        assert_eq!(u32::from_le_bytes(b[4..8].try_into().unwrap()), T_UINT32);
        // Value at offset 8
        assert_eq!(u32::from_le_bytes(b[8..12].try_into().unwrap()), 42);
    }

    #[test]
    fn test_bytes_arg() {
        let payload = b"hello";
        let b = encode_primitive_dict(&[PrimArg::Bytes(Bytes::from_static(payload))]);
        // 4 null + 4 type + 4 size + 5 payload = 17 bytes (no array header)
        assert_eq!(b.len(), 17);
    }

    #[test]
    fn test_int64_arg_omits_length_field() {
        let b = encode_primitive_dict(&[PrimArg::Int64(36)]);
        assert_eq!(b.len(), 16);
        assert_eq!(u32::from_le_bytes(b[0..4].try_into().unwrap()), T_NULL);
        assert_eq!(u32::from_le_bytes(b[4..8].try_into().unwrap()), T_INT64);
        assert_eq!(i64::from_le_bytes(b[8..16].try_into().unwrap()), 36);
    }
}
