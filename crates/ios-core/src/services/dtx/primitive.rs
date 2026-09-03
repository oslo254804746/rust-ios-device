//! DTX Primitive Dictionary decoder.
//!
//! The DTX auxiliary section uses a compact binary format (not plist / NSKeyedArchiver).
//! Reference: go-ios/ios/dtx_codec/dtxprimitivedictionary.go

use bytes::{Buf, Bytes};

use super::types::NSObject;

const PRIMITIVE_NULL: u32 = 0x0000000A;
const PRIMITIVE_STRING: u32 = 0x00000001;
const PRIMITIVE_INT: u32 = 0x00000003;
const PRIMITIVE_LONG: u32 = 0x00000006;
const PRIMITIVE_DOUBLE: u32 = 0x00000009;
const PRIMITIVE_BYTES: u32 = 0x00000002;

/// Decode DTX auxiliary primitive array.
pub fn decode_auxiliary(mut data: Bytes) -> Vec<NSObject> {
    let mut result = Vec::new();

    while data.remaining() >= 4 {
        let (key_type, _, remaining) = match read_entry(data) {
            Some(entry) => entry,
            None => break,
        };
        data = remaining;

        if key_type != PRIMITIVE_NULL || data.remaining() < 4 {
            break;
        }

        let (value_type, value, remaining) = match read_entry(data) {
            Some(entry) => entry,
            None => break,
        };
        data = remaining;

        result.push(entry_to_nsobject(value_type, value));
    }

    result
}

fn read_entry(mut data: Bytes) -> Option<(u32, Bytes, Bytes)> {
    if data.remaining() < 4 {
        return None;
    }

    let type_tag = data.get_u32_le();
    match type_tag {
        PRIMITIVE_NULL => Some((type_tag, Bytes::new(), data)),
        PRIMITIVE_INT => {
            if data.remaining() < 4 {
                return None;
            }
            Some((type_tag, Bytes::copy_from_slice(&data.split_to(4)), data))
        }
        PRIMITIVE_LONG | PRIMITIVE_DOUBLE => {
            if data.remaining() < 8 {
                return None;
            }
            Some((type_tag, Bytes::copy_from_slice(&data.split_to(8)), data))
        }
        // Both are length-prefixed; go-ios `hasLength()` covers t_string and t_bytearray.
        PRIMITIVE_STRING | PRIMITIVE_BYTES => {
            if data.remaining() < 4 {
                return None;
            }
            let len = data.get_u32_le() as usize;
            if data.remaining() < len {
                return None;
            }
            Some((type_tag, Bytes::copy_from_slice(&data.split_to(len)), data))
        }
        _ => None,
    }
}

fn entry_to_nsobject(type_tag: u32, value: Bytes) -> NSObject {
    match type_tag {
        PRIMITIVE_STRING => NSObject::String(String::from_utf8_lossy(&value).into_owned()),
        PRIMITIVE_INT if value.len() >= 4 => {
            NSObject::Int(i32::from_le_bytes(value[..4].try_into().unwrap()) as i64)
        }
        PRIMITIVE_LONG if value.len() >= 8 => {
            NSObject::Int(i64::from_le_bytes(value[..8].try_into().unwrap()))
        }
        PRIMITIVE_DOUBLE if value.len() >= 8 => {
            NSObject::Double(f64::from_le_bytes(value[..8].try_into().unwrap()))
        }
        PRIMITIVE_BYTES => match crate::proto::nskeyedarchiver::unarchive(&value) {
            Ok(v) => archive_to_ns(v),
            Err(_) => NSObject::Data(value),
        },
        _ => NSObject::Null,
    }
}

fn archive_to_ns(val: crate::proto::nskeyedarchiver::ArchiveValue) -> NSObject {
    use crate::proto::nskeyedarchiver::ArchiveValue;
    match val {
        ArchiveValue::Null => NSObject::Null,
        ArchiveValue::Bool(b) => NSObject::Bool(b),
        ArchiveValue::Int(n) => NSObject::Int(n),
        ArchiveValue::Uint(n) => NSObject::Uint(n),
        ArchiveValue::Float(f) => NSObject::Double(f),
        ArchiveValue::String(s) => NSObject::String(s),
        ArchiveValue::Data(d) => NSObject::Data(d),
        ArchiveValue::Array(arr) => NSObject::Array(arr.into_iter().map(archive_to_ns).collect()),
        ArchiveValue::Dict(map) => NSObject::Dict(
            map.into_iter()
                .map(|(k, v)| (k, archive_to_ns(v)))
                .collect(),
        ),
        ArchiveValue::Unknown(s) => NSObject::String(format!("<{s}>")),
    }
}

#[cfg(test)]
mod tests {
    use bytes::Bytes;

    use super::*;
    use crate::services::dtx::primitive_enc::{encode_primitive_dict, PrimArg};

    #[test]
    fn decode_auxiliary_roundtrips_encoded_int64() {
        let encoded = encode_primitive_dict(&[PrimArg::Int64(36)]);
        let decoded = decode_auxiliary(encoded);
        assert_eq!(decoded, vec![NSObject::Int(36)]);
    }

    /// Devices emit the length-prefixed string primitive (`t_string = 0x01`); our encoder
    /// never does, so this has to be built by hand.
    fn encode_string_entry(s: &str) -> Vec<u8> {
        let mut out = Vec::new();
        out.extend_from_slice(&PRIMITIVE_NULL.to_le_bytes());
        out.extend_from_slice(&PRIMITIVE_STRING.to_le_bytes());
        out.extend_from_slice(&(s.len() as u32).to_le_bytes());
        out.extend_from_slice(s.as_bytes());
        out
    }

    #[test]
    fn decode_auxiliary_decodes_string_primitive() {
        let decoded = decode_auxiliary(Bytes::from(encode_string_entry("com.example.app")));
        assert_eq!(
            decoded,
            vec![NSObject::String("com.example.app".to_string())]
        );
    }

    #[test]
    fn decode_auxiliary_keeps_arguments_after_a_string() {
        let mut encoded = encode_string_entry("selector-arg");
        encoded.extend_from_slice(&encode_primitive_dict(&[PrimArg::Int64(7)]));

        let decoded = decode_auxiliary(Bytes::from(encoded));
        assert_eq!(
            decoded,
            vec![
                NSObject::String("selector-arg".to_string()),
                NSObject::Int(7)
            ]
        );
    }

    #[test]
    fn decode_auxiliary_roundtrips_encoded_archived_bytes() {
        let archived = crate::proto::nskeyedarchiver_encode::archive_string("hello");
        let encoded = encode_primitive_dict(&[PrimArg::Bytes(Bytes::from(archived))]);
        let decoded = decode_auxiliary(encoded);
        assert_eq!(decoded, vec![NSObject::String("hello".to_string())]);
    }

    #[test]
    fn decode_auxiliary_preserves_unsigned_archived_values() {
        let archived =
            crate::proto::nskeyedarchiver_encode::archive_array(vec![plist::Value::Integer(
                plist::Integer::from(u64::MAX),
            )]);
        let encoded = encode_primitive_dict(&[PrimArg::Bytes(Bytes::from(archived))]);
        let decoded = decode_auxiliary(encoded);
        assert_eq!(
            decoded,
            vec![NSObject::Array(vec![NSObject::Uint(u64::MAX)])]
        );
    }
}
