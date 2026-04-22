//! NSKeyedArchiver basic decoder.
//!
//! NSKeyedArchiver is Apple's binary plist serialization format used by DTX.
//! Reference: go-ios/ios/nskeyedarchiver/
//!
//! This is a simplified implementation that handles the common cases:
//! - NSString → String
//! - NSNumber → i64 / f64 / bool
//! - NSArray → Vec
//! - NSDictionary → HashMap
//! - nil / NSNull → null

use std::collections::HashMap;

use bytes::Bytes;

#[derive(Debug, Clone)]
pub enum ArchiveValue {
    Null,
    Bool(bool),
    Int(i64),
    Float(f64),
    String(String),
    Data(Bytes),
    Array(Vec<ArchiveValue>),
    Dict(HashMap<String, ArchiveValue>),
    Unknown(String), // class name for unhandled types
}

#[derive(Debug, thiserror::Error)]
pub enum ArchiveError {
    #[error("plist error: {0}")]
    Plist(String),
    #[error("invalid archive: {0}")]
    Invalid(String),
}

/// Decode NSKeyedArchiver binary plist data.
///
/// Returns the root object as an `ArchiveValue`.
pub fn unarchive(data: &[u8]) -> Result<ArchiveValue, ArchiveError> {
    let plist: plist::Value =
        plist::from_bytes(data).map_err(|e| ArchiveError::Plist(e.to_string()))?;

    let root_dict = plist
        .as_dictionary()
        .ok_or_else(|| ArchiveError::Invalid("top-level is not a dict".into()))?;

    // Validate $archiver key
    let archiver = root_dict
        .get("$archiver")
        .and_then(|v| v.as_string())
        .unwrap_or("");
    if archiver != "NSKeyedArchiver" {
        return Err(ArchiveError::Invalid(format!(
            "unknown archiver: {archiver}"
        )));
    }

    // Get $objects array and $top
    let objects = root_dict
        .get("$objects")
        .and_then(|v| v.as_array())
        .ok_or_else(|| ArchiveError::Invalid("missing $objects".into()))?;

    let top = root_dict
        .get("$top")
        .and_then(|v| v.as_dictionary())
        .ok_or_else(|| ArchiveError::Invalid("missing $top".into()))?;

    // Root UID
    let root_uid = top
        .get("root")
        .and_then(uid_from_plist)
        .ok_or_else(|| ArchiveError::Invalid("missing $top.root uid".into()))?;

    decode_object(objects, root_uid)
}

/// Decode a single object by its UID index into the $objects array.
fn decode_object(objects: &[plist::Value], uid: usize) -> Result<ArchiveValue, ArchiveError> {
    if uid >= objects.len() {
        return Err(ArchiveError::Invalid(format!("uid {uid} out of bounds")));
    }

    decode_value(objects, &objects[uid])
}

fn decode_value(
    objects: &[plist::Value],
    obj: &plist::Value,
) -> Result<ArchiveValue, ArchiveError> {
    // $null sentinel
    if obj.as_string() == Some("$null") {
        return Ok(ArchiveValue::Null);
    }

    // Primitive: string
    if let Some(s) = obj.as_string() {
        return Ok(ArchiveValue::String(s.to_string()));
    }
    // Primitive: integer
    if let Some(n) = obj.as_unsigned_integer() {
        return Ok(ArchiveValue::Int(n as i64));
    }
    if let Some(n) = obj.as_signed_integer() {
        return Ok(ArchiveValue::Int(n));
    }
    // Primitive: float
    if let Some(f) = obj.as_real() {
        return Ok(ArchiveValue::Float(f));
    }
    // Primitive: bool
    if let Some(b) = obj.as_boolean() {
        return Ok(ArchiveValue::Bool(b));
    }
    // Primitive: data
    if let Some(d) = obj.as_data() {
        return Ok(ArchiveValue::Data(Bytes::copy_from_slice(d)));
    }

    // Complex: keyed object dict with $class
    if let Some(dict) = obj.as_dictionary() {
        let class_uid = dict.get("$class").and_then(uid_from_plist);
        let class_name = class_uid
            .and_then(|uid| objects.get(uid))
            .and_then(|cv| cv.as_dictionary())
            .and_then(|cd| cd.get("$classname"))
            .and_then(|cn| cn.as_string())
            .unwrap_or("Unknown");

        match class_name {
            "NSNull" => {
                return Ok(ArchiveValue::Null);
            }
            "NSString" | "__NSCFString" | "__NSCFConstantString" => {
                let s = dict
                    .get("NS.string")
                    .and_then(|v| v.as_string())
                    .unwrap_or("");
                return Ok(ArchiveValue::String(s.to_string()));
            }
            "NSNumber" | "__NSCFNumber" | "__NSCFBoolean" => {
                if let Some(v) = dict.get("NS.intval") {
                    if let Some(n) = v.as_signed_integer() {
                        return Ok(ArchiveValue::Int(n));
                    }
                }
                if let Some(v) = dict.get("NS.dblval") {
                    if let Some(f) = v.as_real() {
                        return Ok(ArchiveValue::Float(f));
                    }
                }
                if let Some(v) = dict.get("NS.boolval") {
                    if let Some(b) = v.as_boolean() {
                        return Ok(ArchiveValue::Bool(b));
                    }
                }
                return Ok(ArchiveValue::Null);
            }
            "NSArray" | "NSMutableArray" | "NSSet" | "NSMutableSet" | "NSOrderedSet" => {
                let items = dict
                    .get("NS.objects")
                    .and_then(|v| v.as_array())
                    .map(|arr| arr.iter().filter_map(uid_from_plist).collect::<Vec<_>>())
                    .unwrap_or_default();
                let mut result = Vec::with_capacity(items.len());
                for uid in items {
                    result.push(decode_object(objects, uid)?);
                }
                return Ok(ArchiveValue::Array(result));
            }
            "NSDictionary" | "NSMutableDictionary" => {
                let keys = dict
                    .get("NS.keys")
                    .and_then(|v| v.as_array())
                    .map(|a| a.iter().filter_map(uid_from_plist).collect::<Vec<_>>())
                    .unwrap_or_default();
                let vals = dict
                    .get("NS.objects")
                    .and_then(|v| v.as_array())
                    .map(|a| a.iter().filter_map(uid_from_plist).collect::<Vec<_>>())
                    .unwrap_or_default();
                let mut map = HashMap::new();
                for (k_uid, v_uid) in keys.into_iter().zip(vals.into_iter()) {
                    let key_str = match decode_object(objects, k_uid) {
                        Ok(ArchiveValue::String(k)) => k,
                        Ok(ArchiveValue::Int(n)) => n.to_string(),
                        Ok(ArchiveValue::Float(f)) => f.to_string(),
                        Ok(ArchiveValue::Bool(b)) => b.to_string(),
                        _ => continue,
                    };
                    let v = decode_object(objects, v_uid)?;
                    map.insert(key_str, v);
                }
                return Ok(ArchiveValue::Dict(map));
            }
            "NSData" | "NSMutableData" => {
                if let Some(d) = dict.get("NS.data").and_then(|v| v.as_data()) {
                    return Ok(ArchiveValue::Data(Bytes::copy_from_slice(d)));
                }
            }
            // DTX tap messages: decode DTTapMessagePlist as the value
            "DTSysmonTapMessage"
            | "DTActivityTraceTapMessage"
            | "DTTapMessage"
            | "DTTapHeartbeatMessage"
            | "DTTapStatusMessage"
            | "DTKTraceTapMessage" => {
                if let Some(uid) = dict.get("DTTapMessagePlist").and_then(uid_from_plist) {
                    return decode_object(objects, uid);
                }
                return Ok(ArchiveValue::Unknown(class_name.to_string()));
            }
            other => {
                let mut map = HashMap::new();
                for (key, value) in dict {
                    if key == "$class" {
                        continue;
                    }
                    map.insert(key.clone(), decode_field_value(objects, value)?);
                }
                if !map.contains_key("ObjectType") {
                    map.insert(
                        "ObjectType".to_string(),
                        ArchiveValue::String(other.to_string()),
                    );
                }
                return Ok(ArchiveValue::Dict(map));
            }
        }
    }

    Ok(ArchiveValue::Null)
}

fn decode_field_value(
    objects: &[plist::Value],
    value: &plist::Value,
) -> Result<ArchiveValue, ArchiveError> {
    if let Some(uid) = uid_from_plist(value) {
        return decode_object(objects, uid);
    }

    match value {
        plist::Value::Array(items) => Ok(ArchiveValue::Array(
            items
                .iter()
                .map(|item| decode_field_value(objects, item))
                .collect::<Result<Vec<_>, _>>()?,
        )),
        plist::Value::Dictionary(dict) if !dict.contains_key("$class") => {
            let mut map = HashMap::new();
            for (key, value) in dict {
                map.insert(key.clone(), decode_field_value(objects, value)?);
            }
            Ok(ArchiveValue::Dict(map))
        }
        _ => decode_value(objects, value),
    }
}

fn uid_from_plist(v: &plist::Value) -> Option<usize> {
    // Binary plist stores UIDs as native Uid type
    if let plist::Value::Uid(uid) = v {
        return Some(uid.get() as usize);
    }
    // XML plist UIDs are stored as Dict {"CF$UID": integer}
    if let Some(d) = v.as_dictionary() {
        if let Some(n) = d.get("CF$UID").and_then(|u| u.as_unsigned_integer()) {
            return Some(n as usize);
        }
    }
    // Or directly as unsigned integer (legacy)
    v.as_unsigned_integer().map(|n| n as usize)
}

impl ArchiveValue {
    pub fn as_str(&self) -> Option<&str> {
        if let ArchiveValue::String(s) = self {
            Some(s)
        } else {
            None
        }
    }

    pub fn as_int(&self) -> Option<i64> {
        match self {
            ArchiveValue::Int(n) => Some(*n),
            ArchiveValue::Bool(b) => Some(*b as i64),
            _ => None,
        }
    }

    pub fn as_array(&self) -> Option<&[ArchiveValue]> {
        if let ArchiveValue::Array(a) = self {
            Some(a)
        } else {
            None
        }
    }

    pub fn as_dict(&self) -> Option<&HashMap<String, ArchiveValue>> {
        if let ArchiveValue::Dict(d) = self {
            Some(d)
        } else {
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_unarchive_null_string() {
        // $null sentinel in $objects[0]
        // A minimal NSKeyedArchiver plist containing a single NSString "hello"
        // We can't easily construct binary plists in tests without a fixture,
        // so just verify the decode_object helper works with a synthetic plist.
        let objects: Vec<plist::Value> = vec![
            plist::Value::String("$null".to_string()),
            plist::Value::String("hello".to_string()),
        ];
        let result = decode_object(&objects, 0).unwrap();
        assert!(matches!(result, ArchiveValue::Null));
        let result2 = decode_object(&objects, 1).unwrap();
        assert_eq!(result2.as_str(), Some("hello"));
    }

    #[test]
    fn test_decode_unknown_class_preserves_fields() {
        let objects = vec![
            plist::Value::String("$null".to_string()),
            plist::Value::Dictionary(plist::Dictionary::from_iter([
                (
                    "ObjectType".to_string(),
                    plist::Value::String("AXAuditElement_v1".into()),
                ),
                ("Value".to_string(), plist::Value::Uid(plist::Uid::new(3))),
                ("$class".to_string(), plist::Value::Uid(plist::Uid::new(2))),
            ])),
            plist::Value::Dictionary(plist::Dictionary::from_iter([
                (
                    "$classname".to_string(),
                    plist::Value::String("AXAuditInspectorFocus_v1".into()),
                ),
                (
                    "$classes".to_string(),
                    plist::Value::Array(vec![
                        plist::Value::String("AXAuditInspectorFocus_v1".into()),
                        plist::Value::String("NSObject".into()),
                    ]),
                ),
            ])),
            plist::Value::String("payload".to_string()),
        ];

        let decoded = decode_object(&objects, 1).unwrap();
        let map = decoded
            .as_dict()
            .expect("unknown class should decode to dict");
        assert_eq!(
            map.get("ObjectType").and_then(ArchiveValue::as_str),
            Some("AXAuditElement_v1")
        );
        assert_eq!(
            map.get("Value").and_then(ArchiveValue::as_str),
            Some("payload")
        );
    }

    #[test]
    fn test_decode_archived_nsnull_object_as_null() {
        let objects = vec![
            plist::Value::String("$null".to_string()),
            plist::Value::Dictionary(plist::Dictionary::from_iter([(
                "$class".to_string(),
                plist::Value::Uid(plist::Uid::new(2)),
            )])),
            plist::Value::Dictionary(plist::Dictionary::from_iter([
                (
                    "$classname".to_string(),
                    plist::Value::String("NSNull".into()),
                ),
                (
                    "$classes".to_string(),
                    plist::Value::Array(vec![
                        plist::Value::String("NSNull".into()),
                        plist::Value::String("NSObject".into()),
                    ]),
                ),
            ])),
        ];

        let decoded = decode_object(&objects, 1).unwrap();
        assert!(matches!(decoded, ArchiveValue::Null));
    }
}
