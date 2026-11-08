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

crate::redaction::error_debug!(ArchiveError);

use std::collections::HashMap;

use bytes::Bytes;

#[derive(Debug, Clone)]
pub enum ArchiveValue {
    Null,
    Bool(bool),
    Int(i64),
    Uint(u64),
    Float(f64),
    String(String),
    Data(Bytes),
    Array(Vec<ArchiveValue>),
    Dict(HashMap<String, ArchiveValue>),
    Unknown(String), // class name for unhandled types
}

#[derive(thiserror::Error)]
pub enum ArchiveError {
    #[error("plist error: {0}")]
    Plist(#[source] plist::Error),
    #[error("invalid archive: {0}")]
    Invalid(String),
    #[error("invalid archive integer: {0}")]
    Integer(#[source] std::num::TryFromIntError),
    #[error("archive nesting exceeds {0} levels (cycle or hostile input?)")]
    TooDeep(usize),
}

/// Maximum object nesting followed when resolving `$objects` UIDs.
///
/// `$objects` is a flat pool addressed by UID, so nothing stops an entry from
/// referring back to itself. Bounding the depth turns what would be unbounded
/// recursion into an error.
const MAX_ARCHIVE_DEPTH: usize = 64;

/// Decode NSKeyedArchiver binary plist data.
///
/// Returns the root object as an `ArchiveValue`.
pub fn unarchive(data: &[u8]) -> Result<ArchiveValue, ArchiveError> {
    let plist: plist::Value = plist::from_bytes(data).map_err(ArchiveError::Plist)?;

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

    decode_object(objects, root_uid, 0)
}

/// Decode a single object by its UID index into the $objects array.
fn decode_object(
    objects: &[plist::Value],
    uid: usize,
    depth: usize,
) -> Result<ArchiveValue, ArchiveError> {
    if depth > MAX_ARCHIVE_DEPTH {
        return Err(ArchiveError::TooDeep(MAX_ARCHIVE_DEPTH));
    }
    if uid >= objects.len() {
        return Err(ArchiveError::Invalid(format!("uid {uid} out of bounds")));
    }

    decode_value(objects, &objects[uid], depth)
}

fn decode_value(
    objects: &[plist::Value],
    obj: &plist::Value,
    depth: usize,
) -> Result<ArchiveValue, ArchiveError> {
    // $null sentinel
    if obj.as_string() == Some("$null") {
        return Ok(ArchiveValue::Null);
    }

    // Primitive: string
    if let Some(s) = obj.as_string() {
        return Ok(ArchiveValue::String(s.to_string()));
    }
    // Sysmontap uses unsigned sentinels such as u64::MAX. Preserve those
    // exactly instead of rejecting the entire process snapshot or wrapping.
    if let Some(n) = obj.as_signed_integer() {
        return Ok(ArchiveValue::Int(n));
    }
    if let Some(n) = obj.as_unsigned_integer() {
        return Ok(ArchiveValue::Uint(n));
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

    // A keyed archive object table normally contains the object itself, but
    // archives produced by some plist writers can contain an extra UID
    // indirection. Resolve it here as well as in object fields so a valid
    // reference cannot be silently turned into `Null`.
    if matches!(obj, plist::Value::Uid(_)) {
        let uid = uid_from_plist(obj).ok_or_else(|| {
            ArchiveError::Invalid("UID does not fit in the host index type".into())
        })?;
        return decode_object(objects, uid, depth + 1);
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
                    if let Some(n) = v.as_unsigned_integer() {
                        return Ok(ArchiveValue::Uint(n));
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
                    result.push(decode_object(objects, uid, depth + 1)?);
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
                for (k_uid, v_uid) in keys.into_iter().zip(vals) {
                    let key_str = match decode_object(objects, k_uid, depth + 1) {
                        Ok(ArchiveValue::String(k)) => k,
                        Ok(ArchiveValue::Int(n)) => n.to_string(),
                        Ok(ArchiveValue::Uint(n)) => n.to_string(),
                        Ok(ArchiveValue::Float(f)) => f.to_string(),
                        Ok(ArchiveValue::Bool(b)) => b.to_string(),
                        _ => continue,
                    };
                    let v = decode_object(objects, v_uid, depth + 1)?;
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
                    return decode_object(objects, uid, depth + 1);
                }
                return Ok(ArchiveValue::Unknown(class_name.to_string()));
            }
            other => {
                let mut map = HashMap::new();
                for (key, value) in dict {
                    if key == "$class" {
                        continue;
                    }
                    // MBFile encodes scalar metadata directly as plist integers;
                    // only its actual UID values are object-table references.
                    // Other archived classes retain their legacy integer-UID
                    // behavior in decode_field_value.
                    let decoded = if other == "MBFile" && matches!(value, plist::Value::Integer(_))
                    {
                        decode_value(objects, value, depth + 1)?
                    } else {
                        decode_field_value(objects, value, depth + 1)?
                    };
                    map.insert(key.clone(), decoded);
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
    depth: usize,
) -> Result<ArchiveValue, ArchiveError> {
    if depth > MAX_ARCHIVE_DEPTH {
        return Err(ArchiveError::TooDeep(MAX_ARCHIVE_DEPTH));
    }
    if let Some(uid) = uid_from_plist(value) {
        return decode_object(objects, uid, depth + 1);
    }

    match value {
        plist::Value::Array(items) => Ok(ArchiveValue::Array(
            items
                .iter()
                .map(|item| decode_field_value(objects, item, depth + 1))
                .collect::<Result<Vec<_>, _>>()?,
        )),
        plist::Value::Dictionary(dict) if !dict.contains_key("$class") => {
            let mut map = HashMap::new();
            for (key, value) in dict {
                map.insert(key.clone(), decode_field_value(objects, value, depth + 1)?);
            }
            Ok(ArchiveValue::Dict(map))
        }
        _ => decode_value(objects, value, depth),
    }
}

fn uid_from_plist(v: &plist::Value) -> Option<usize> {
    // Binary plist stores UIDs as native Uid type
    if let plist::Value::Uid(uid) = v {
        return usize::try_from(uid.get()).ok();
    }
    // XML plist UIDs are stored as Dict {"CF$UID": integer}
    if let Some(d) = v.as_dictionary() {
        if let Some(n) = d.get("CF$UID").and_then(|u| u.as_unsigned_integer()) {
            return usize::try_from(n).ok();
        }
    }
    // Or directly as unsigned integer (legacy)
    v.as_unsigned_integer()
        .and_then(|n| usize::try_from(n).ok())
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
    fn mbfile_archive_keeps_direct_integers_and_resolves_real_uids() {
        fn archive(relative_path_uid: u64) -> Vec<u8> {
            let root = plist::Value::Dictionary(plist::Dictionary::from_iter([
                ("$class", plist::Value::Uid(plist::Uid::new(2))),
                (
                    "RelativePath",
                    plist::Value::Uid(plist::Uid::new(relative_path_uid)),
                ),
                (
                    "LastModified",
                    plist::Value::Integer(1_788_671_599u64.into()),
                ),
                ("Mode", plist::Value::Integer(0o100600u64.into())),
                ("Size", plist::Value::Integer(3u64.into())),
                ("GroupID", plist::Value::Integer(0u64.into())),
            ]));
            let class = plist::Value::Dictionary(plist::Dictionary::from_iter([
                ("$classname", plist::Value::String("MBFile".into())),
                (
                    "$classes",
                    plist::Value::Array(vec!["MBFile".into(), "NSObject".into()]),
                ),
            ]));
            let archive = plist::Value::Dictionary(plist::Dictionary::from_iter([
                ("$archiver", plist::Value::String("NSKeyedArchiver".into())),
                (
                    "$objects",
                    plist::Value::Array(vec!["$null".into(), root, class, "folder/file".into()]),
                ),
                (
                    "$top",
                    plist::Value::Dictionary(plist::Dictionary::from_iter([(
                        "root",
                        plist::Value::Uid(plist::Uid::new(1)),
                    )])),
                ),
            ]));
            let mut bytes = Vec::new();
            plist::to_writer_binary(&mut bytes, &archive).unwrap();
            bytes
        }

        let value = unarchive(&archive(3)).unwrap();
        let fields = value.as_dict().unwrap();
        assert_eq!(
            fields.get("LastModified").and_then(ArchiveValue::as_int),
            Some(1_788_671_599)
        );
        assert_eq!(
            fields.get("Mode").and_then(ArchiveValue::as_int),
            Some(0o100600)
        );
        assert_eq!(fields.get("Size").and_then(ArchiveValue::as_int), Some(3));
        assert_eq!(
            fields.get("GroupID").and_then(ArchiveValue::as_int),
            Some(0)
        );
        assert_eq!(
            fields.get("RelativePath").and_then(ArchiveValue::as_str),
            Some("folder/file")
        );
        let error = unarchive(&archive(99)).unwrap_err();
        assert!(
            matches!(error, ArchiveError::Invalid(message) if message.contains("out of bounds"))
        );
    }

    #[test]
    fn actual_binary_archive_preserves_unsigned_sentinel() {
        let mut bytes = Vec::new();
        plist::to_writer_binary(
            &mut bytes,
            &plist::Value::Dictionary(plist::Dictionary::from_iter([
                ("$archiver", plist::Value::String("NSKeyedArchiver".into())),
                (
                    "$objects",
                    plist::Value::Array(vec![plist::Value::Integer(u64::MAX.into())]),
                ),
                (
                    "$top",
                    plist::Value::Dictionary(plist::Dictionary::from_iter([(
                        "root",
                        plist::Value::Uid(plist::Uid::new(0)),
                    )])),
                ),
            ])),
        )
        .unwrap();
        assert!(matches!(
            unarchive(&bytes).unwrap(),
            ArchiveValue::Uint(u64::MAX)
        ));
    }

    #[test]
    fn actual_unarchive_plist_failure_retains_native_source_and_safe_debug() {
        use std::error::Error;
        let error = unarchive(b"<plist><dict>owned-private").unwrap_err();
        assert!(matches!(error, ArchiveError::Plist(_)));
        assert!(error.source().unwrap().is::<plist::Error>());
        assert_eq!(crate::classify_error(&error), crate::ErrorClass::Protocol);
        assert!(!format!("{error:?}").contains("owned-private"));
    }

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
        let result = decode_object(&objects, 0, 0).unwrap();
        assert!(matches!(result, ArchiveValue::Null));
        let result2 = decode_object(&objects, 1, 0).unwrap();
        assert_eq!(result2.as_str(), Some("hello"));
    }

    #[test]
    fn test_decode_unsigned_integer_primitives() {
        let objects = vec![
            plist::Value::Integer(plist::Integer::from(i64::MAX as u64)),
            plist::Value::Integer(plist::Integer::from(i64::MAX as u64 + 1)),
        ];

        assert_eq!(
            decode_object(&objects, 0, 0).unwrap().as_int(),
            Some(i64::MAX)
        );

        assert!(
            matches!(decode_object(&objects, 1, 0).unwrap(), ArchiveValue::Uint(n) if n == i64::MAX as u64 + 1)
        );
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

        let decoded = decode_object(&objects, 1, 0).unwrap();
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

        let decoded = decode_object(&objects, 1, 0).unwrap();
        assert!(matches!(decoded, ArchiveValue::Null));
    }

    #[test]
    fn test_decode_uid_alias_object() {
        let objects = vec![
            plist::Value::String("$null".to_string()),
            plist::Value::Uid(plist::Uid::new(2)),
            plist::Value::String("aliased payload".to_string()),
        ];

        let decoded = decode_object(&objects, 1, 0).unwrap();
        assert_eq!(decoded.as_str(), Some("aliased payload"));
    }

    #[test]
    fn test_decode_uid_alias_out_of_bounds_is_an_error() {
        let objects = vec![
            plist::Value::String("$null".to_string()),
            plist::Value::Uid(plist::Uid::new(u64::MAX)),
        ];

        let err = decode_object(&objects, 1, 0).unwrap_err();
        assert!(
            matches!(err, ArchiveError::Invalid(message) if message.contains("out of bounds") || message.contains("does not fit"))
        );
    }

    #[test]
    fn test_decode_xcuitest_nullable_fields_as_null() {
        let objects = vec![
            plist::Value::String("$null".to_string()),
            plist::Value::Dictionary(plist::Dictionary::from_iter([
                ("$class".to_string(), plist::Value::Uid(plist::Uid::new(2))),
                (
                    "compact-description".to_string(),
                    plist::Value::Uid(plist::Uid::new(3)),
                ),
                // iOS can encode an absent source-code context as UID(0).
                (
                    "source-code-context".to_string(),
                    plist::Value::Uid(plist::Uid::new(0)),
                ),
                // userInfo is intentionally omitted on attachments from some
                // OS versions; the generic decoder must tolerate that too.
            ])),
            plist::Value::Dictionary(plist::Dictionary::from_iter([
                (
                    "$classname".to_string(),
                    plist::Value::String("XCTIssue".into()),
                ),
                (
                    "$classes".to_string(),
                    plist::Value::Array(vec![
                        plist::Value::String("XCTIssue".into()),
                        plist::Value::String("NSObject".into()),
                    ]),
                ),
            ])),
            plist::Value::String("no source context".into()),
        ];

        let decoded = decode_object(&objects, 1, 0).unwrap();
        let fields = decoded.as_dict().expect("XCTIssue should decode as a dict");
        assert!(matches!(
            fields.get("source-code-context"),
            Some(ArchiveValue::Null)
        ));
        assert!(!fields.contains_key("userInfo"));
        assert_eq!(
            fields
                .get("compact-description")
                .and_then(ArchiveValue::as_str),
            Some("no source context")
        );
    }

    #[cfg(target_pointer_width = "32")]
    #[test]
    fn test_uid_conversion_does_not_truncate_on_32_bit_hosts() {
        let value = plist::Value::Uid(plist::Uid::new(u32::MAX as u64 + 1));
        assert_eq!(uid_from_plist(&value), None);
    }
}
