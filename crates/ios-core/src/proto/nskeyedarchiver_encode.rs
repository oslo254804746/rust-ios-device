//! NSKeyedArchiver binary encoder.
//!
//! Encodes Rust values to NSKeyedArchiver binary plist format,
//! which is required for DTX method invocation payloads and arguments.
//!
//! Reference: go-ios/ios/nskeyedarchiver/archiver.go

use plist::{Dictionary, Uid, Value};
use uuid::Uuid;

#[derive(Debug, Clone)]
pub struct NsUrl {
    pub path: String,
}

#[derive(Debug, Clone)]
pub struct XctCapabilities {
    pub capabilities: Vec<(String, Value)>,
}

#[derive(Debug, Clone)]
pub struct XcTestConfiguration {
    pub session_identifier: Uuid,
    pub test_bundle_url: NsUrl,
    pub ide_capabilities: XctCapabilities,
    pub automation_framework_path: String,
    pub initialize_for_ui_testing: bool,
    pub report_results_to_ide: bool,
    pub tests_must_run_on_main_thread: bool,
    pub test_timeouts_enabled: bool,
    pub additional_fields: Vec<(String, Value)>,
}

/// Encode a string as NSKeyedArchiver binary plist (NSString).
pub fn archive_string(s: &str) -> Vec<u8> {
    archive_value(Value::String(s.to_string()))
}

/// Encode an integer as NSKeyedArchiver binary plist (NSNumber/int64).
pub fn archive_int(n: i64) -> Vec<u8> {
    archive_value(Value::Integer(n.into()))
}

/// Encode a float as NSKeyedArchiver binary plist (NSNumber/double).
pub fn archive_float(f: f64) -> Vec<u8> {
    archive_value(Value::Real(f))
}

/// Encode a bool as NSKeyedArchiver binary plist (NSNumber/BOOL).
pub fn archive_bool(b: bool) -> Vec<u8> {
    archive_value(Value::Boolean(b))
}

/// Encode an NSNull object.
pub fn archive_null() -> Vec<u8> {
    let mut objects = vec![Value::String("$null".to_string())];

    let mut object = Dictionary::new();
    object.insert("$class".to_string(), Value::Uid(Uid::new(2)));
    objects.push(Value::Dictionary(object));
    objects.push(class_descriptor("NSNull", &["NSNull", "NSObject"]));

    let root_doc = build_keyed_archive(Value::Uid(Uid::new(1)), objects);
    to_binary_plist(&root_doc)
}

/// Encode a byte array as NSKeyedArchiver binary plist (NSData).
pub fn archive_data(data: &[u8]) -> Vec<u8> {
    archive_value(Value::Data(data.to_vec()))
}

/// Encode an NSUUID object.
pub fn archive_uuid(uuid: Uuid) -> Vec<u8> {
    let mut objects = vec![Value::String("$null".to_string())];
    let root_uid = archive_nsuuid_into(uuid, &mut objects);
    let root_doc = build_keyed_archive(root_uid, objects);
    to_binary_plist(&root_doc)
}

/// Encode an NSURL object with a file:// relative path.
pub fn archive_nsurl(url: NsUrl) -> Vec<u8> {
    let mut objects = vec![Value::String("$null".to_string())];
    let root_uid = archive_nsurl_into(url, &mut objects);
    let root_doc = build_keyed_archive(root_uid, objects);
    to_binary_plist(&root_doc)
}

/// Encode an XCTCapabilities object with a capabilities-dictionary payload.
pub fn archive_xct_capabilities(capabilities: XctCapabilities) -> Vec<u8> {
    let mut objects = vec![Value::String("$null".to_string())];
    let root_uid = archive_xct_capabilities_into(capabilities, &mut objects);
    let root_doc = build_keyed_archive(root_uid, objects);
    to_binary_plist(&root_doc)
}

/// Encode a minimal XCTestConfiguration object suitable for testmanager startup.
pub fn archive_xctest_configuration(config: XcTestConfiguration) -> Vec<u8> {
    let mut objects = vec![Value::String("$null".to_string())];
    let root_uid = archive_xctest_configuration_into(config, &mut objects);
    let root_doc = build_keyed_archive(root_uid, objects);
    to_binary_plist(&root_doc)
}

/// Encode an array of pre-archived values as NSArray.
///
/// Each item must already be a plist-compatible `Value`.
pub fn archive_array(items: Vec<Value>) -> Vec<u8> {
    // Build $objects: [$null, NSArray_dict, item1, item2, ...]
    let count = items.len();
    let mut objects = vec![Value::String("$null".to_string())];

    // NSArray object at index 1
    let mut arr_obj = Dictionary::new();
    arr_obj.insert("$class".to_string(), Value::Uid(Uid::new(2 + count as u64)));
    let ns_objects: Vec<Value> = (0..count)
        .map(|i| Value::Uid(Uid::new((2 + i) as u64)))
        .collect();
    arr_obj.insert("NS.objects".to_string(), Value::Array(ns_objects));
    objects.push(Value::Dictionary(arr_obj));

    // Item objects
    for item in items {
        objects.push(item);
    }

    // NSArray class descriptor
    let mut class_obj = Dictionary::new();
    class_obj.insert(
        "$classname".to_string(),
        Value::String("NSArray".to_string()),
    );
    class_obj.insert(
        "$classes".to_string(),
        Value::Array(vec![
            Value::String("NSArray".to_string()),
            Value::String("NSObject".to_string()),
        ]),
    );
    objects.push(Value::Dictionary(class_obj));

    let root_doc = build_keyed_archive(Value::Uid(Uid::new(1)), objects);
    to_binary_plist(&root_doc)
}

/// Encode a dictionary as NSDictionary.
pub fn archive_dict(pairs: Vec<(String, Value)>) -> Vec<u8> {
    let mut objects: Vec<Value> = vec![Value::String("$null".to_string())];
    let root_uid = archive_dict_into(&pairs, &mut objects);
    let root_doc = build_keyed_archive(root_uid, objects);
    to_binary_plist(&root_doc)
}

/// Recursively archive a plist Value into the objects array, returning its UID.
fn archive_value_into(val: Value, objects: &mut Vec<Value>) -> Value {
    match val {
        // Primitives go directly into objects array
        Value::String(_)
        | Value::Integer(_)
        | Value::Real(_)
        | Value::Boolean(_)
        | Value::Data(_) => {
            let idx = objects.len();
            objects.push(val);
            Value::Uid(Uid::new(idx as u64))
        }
        Value::Array(items) => {
            // NSArray: {$class, NS.objects: [UIDs]}
            let item_uids: Vec<Value> = items
                .into_iter()
                .map(|v| archive_value_into(v, objects))
                .collect();

            let arr_idx = objects.len();
            let class_idx = arr_idx + 1;

            let mut arr_obj = Dictionary::new();
            arr_obj.insert("$class".to_string(), Value::Uid(Uid::new(class_idx as u64)));
            arr_obj.insert("NS.objects".to_string(), Value::Array(item_uids));
            objects.push(Value::Dictionary(arr_obj));

            let mut class_obj = Dictionary::new();
            class_obj.insert(
                "$classname".to_string(),
                Value::String("NSArray".to_string()),
            );
            class_obj.insert(
                "$classes".to_string(),
                Value::Array(vec![
                    Value::String("NSArray".to_string()),
                    Value::String("NSObject".to_string()),
                ]),
            );
            objects.push(Value::Dictionary(class_obj));

            Value::Uid(Uid::new(arr_idx as u64))
        }
        Value::Dictionary(dict) => {
            let pairs = dict.into_iter().collect::<Vec<_>>();
            archive_dict_into(&pairs, objects)
        }
        other => {
            let idx = objects.len();
            objects.push(other);
            Value::Uid(Uid::new(idx as u64))
        }
    }
}

fn archive_dict_into(pairs: &[(String, Value)], objects: &mut Vec<Value>) -> Value {
    let dict_idx = objects.len();
    // placeholder
    objects.push(Value::Boolean(false));

    let mut key_uids = Vec::new();
    let mut val_uids = Vec::new();
    for (k, v) in pairs {
        let k_uid = archive_value_into(Value::String(k.clone()), objects);
        let v_uid = archive_value_into(v.clone(), objects);
        key_uids.push(k_uid);
        val_uids.push(v_uid);
    }

    let class_idx = objects.len();
    let mut class_obj = Dictionary::new();
    class_obj.insert(
        "$classname".to_string(),
        Value::String("NSDictionary".to_string()),
    );
    class_obj.insert(
        "$classes".to_string(),
        Value::Array(vec![
            Value::String("NSDictionary".to_string()),
            Value::String("NSObject".to_string()),
        ]),
    );
    objects.push(Value::Dictionary(class_obj));

    let mut dict_obj = Dictionary::new();
    dict_obj.insert("$class".to_string(), Value::Uid(Uid::new(class_idx as u64)));
    dict_obj.insert("NS.keys".to_string(), Value::Array(key_uids));
    dict_obj.insert("NS.objects".to_string(), Value::Array(val_uids));
    objects[dict_idx] = Value::Dictionary(dict_obj);

    Value::Uid(Uid::new(dict_idx as u64))
}

fn archive_nsuuid_into(uuid: Uuid, objects: &mut Vec<Value>) -> Value {
    let object_idx = objects.len();
    let class_idx = object_idx + 1;

    let mut object = Dictionary::new();
    object.insert("$class".to_string(), Value::Uid(Uid::new(class_idx as u64)));
    object.insert(
        "NS.uuidbytes".to_string(),
        Value::Data(uuid.into_bytes().to_vec()),
    );
    objects.push(Value::Dictionary(object));

    objects.push(class_descriptor("NSUUID", &["NSUUID", "NSObject"]));
    Value::Uid(Uid::new(object_idx as u64))
}

fn archive_nsurl_into(url: NsUrl, objects: &mut Vec<Value>) -> Value {
    let object_idx = objects.len();
    let class_idx = object_idx + 1;
    let relative_idx = object_idx + 2;

    let mut object = Dictionary::new();
    object.insert("$class".to_string(), Value::Uid(Uid::new(class_idx as u64)));
    object.insert("NS.base".to_string(), Value::Uid(Uid::new(0)));
    object.insert(
        "NS.relative".to_string(),
        Value::Uid(Uid::new(relative_idx as u64)),
    );
    objects.push(Value::Dictionary(object));
    objects.push(class_descriptor("NSURL", &["NSURL", "NSObject"]));
    objects.push(Value::String(format!("file://{}", url.path)));

    Value::Uid(Uid::new(object_idx as u64))
}

fn archive_xct_capabilities_into(capabilities: XctCapabilities, objects: &mut Vec<Value>) -> Value {
    let dict_uid = archive_dict_into(&capabilities.capabilities, objects);
    let object_idx = objects.len();
    let class_idx = object_idx + 1;

    let mut object = Dictionary::new();
    object.insert("$class".to_string(), Value::Uid(Uid::new(class_idx as u64)));
    object.insert("capabilities-dictionary".to_string(), dict_uid);
    objects.push(Value::Dictionary(object));
    objects.push(class_descriptor(
        "XCTCapabilities",
        &["XCTCapabilities", "NSObject"],
    ));

    Value::Uid(Uid::new(object_idx as u64))
}

fn archive_xctest_configuration_into(
    config: XcTestConfiguration,
    objects: &mut Vec<Value>,
) -> Value {
    let session_uid = archive_nsuuid_into(config.session_identifier, objects);
    let bundle_uid = archive_nsurl_into(config.test_bundle_url, objects);
    let caps_uid = archive_xct_capabilities_into(config.ide_capabilities, objects);
    let automation_uid =
        archive_value_into(Value::String(config.automation_framework_path), objects);

    let mut object = Dictionary::new();
    object.insert("sessionIdentifier".to_string(), session_uid);
    object.insert("testBundleURL".to_string(), bundle_uid);
    object.insert("IDECapabilities".to_string(), caps_uid);
    object.insert("automationFrameworkPath".to_string(), automation_uid);
    object.insert(
        "initializeForUITesting".to_string(),
        Value::Boolean(config.initialize_for_ui_testing),
    );
    object.insert(
        "reportResultsToIDE".to_string(),
        Value::Boolean(config.report_results_to_ide),
    );
    object.insert(
        "testsMustRunOnMainThread".to_string(),
        Value::Boolean(config.tests_must_run_on_main_thread),
    );
    object.insert(
        "testTimeoutsEnabled".to_string(),
        Value::Boolean(config.test_timeouts_enabled),
    );
    for (key, value) in config.additional_fields {
        if key == "testsToRun" || key == "testsToSkip" {
            if let Value::Array(items) = &value {
                let selectors = items
                    .iter()
                    .filter_map(Value::as_string)
                    .map(ToString::to_string)
                    .collect::<Vec<_>>();
                if selectors.len() == items.len() {
                    // XCTestConfiguration uses NSSet for the legacy selector
                    // fields and an XCTTestIdentifierSet for the newer, richer
                    // representation. Sending NSArray here happens to decode
                    // in our permissive parser but is rejected by some
                    // testmanagerd releases.
                    object.insert(key.clone(), archive_nsset_into(&selectors, objects));
                    let identifier_key = if key == "testsToRun" {
                        "testIdentifiersToRun"
                    } else {
                        "testIdentifiersToSkip"
                    };
                    object.insert(
                        identifier_key.to_string(),
                        archive_xct_test_identifier_set_into(&selectors, objects),
                    );
                    continue;
                }
            }
        }
        // XCTestConfiguration stores object-valued fields as keyed references.
        // Keep scalar NSNumber values inline (as the system archive does), but
        // archive strings, data, arrays, and dictionaries before attaching them
        // to the configuration object. Inlining an NSArray/NSDictionary here
        // produces a plist that our decoder can read but NSKeyedUnarchiver on
        // device cannot resolve as an object reference.
        object.insert(key, archive_configuration_field(value, objects));
    }

    // Object-valued additional fields may have appended entries to the object
    // table, so compute these indexes only after all fields are archived.
    let object_idx = objects.len();
    let class_idx = object_idx + 1;
    object.insert("$class".to_string(), Value::Uid(Uid::new(class_idx as u64)));
    objects.push(Value::Dictionary(object));
    objects.push(class_descriptor(
        "XCTestConfiguration",
        &["XCTestConfiguration", "NSObject"],
    ));

    Value::Uid(Uid::new(object_idx as u64))
}

fn archive_nsset_into(values: &[String], objects: &mut Vec<Value>) -> Value {
    let object_idx = objects.len();
    objects.push(Value::Boolean(false));
    let class_idx = objects.len();
    objects.push(class_descriptor("NSSet", &["NSSet", "NSObject"]));

    let item_uids = values
        .iter()
        .map(|value| archive_value_into(Value::String(value.clone()), objects))
        .collect::<Vec<_>>();

    let mut object = Dictionary::new();
    object.insert("$class".to_string(), Value::Uid(Uid::new(class_idx as u64)));
    object.insert("NS.objects".to_string(), Value::Array(item_uids));
    objects[object_idx] = Value::Dictionary(object);
    Value::Uid(Uid::new(object_idx as u64))
}

fn archive_xct_test_identifier_set_into(values: &[String], objects: &mut Vec<Value>) -> Value {
    let set_idx = objects.len();
    objects.push(Value::Boolean(false));

    let array_idx = objects.len();
    objects.push(Value::Boolean(false));
    let array_class_idx = objects.len();
    objects.push(class_descriptor(
        "NSMutableArray",
        &["NSMutableArray", "NSArray", "NSObject"],
    ));

    let identifier_uids = values
        .iter()
        .map(|selector| archive_xct_test_identifier_into(selector, objects))
        .collect::<Vec<_>>();
    let mut array = Dictionary::new();
    array.insert(
        "$class".to_string(),
        Value::Uid(Uid::new(array_class_idx as u64)),
    );
    array.insert("NS.objects".to_string(), Value::Array(identifier_uids));
    objects[array_idx] = Value::Dictionary(array);

    let set_class_idx = objects.len();
    objects.push(class_descriptor(
        "XCTTestIdentifierSet",
        &["XCTTestIdentifierSet", "NSObject"],
    ));
    let mut set = Dictionary::new();
    set.insert(
        "$class".to_string(),
        Value::Uid(Uid::new(set_class_idx as u64)),
    );
    set.insert(
        "identifiers".to_string(),
        Value::Uid(Uid::new(array_idx as u64)),
    );
    objects[set_idx] = Value::Dictionary(set);
    Value::Uid(Uid::new(set_idx as u64))
}

fn archive_xct_test_identifier_into(selector: &str, objects: &mut Vec<Value>) -> Value {
    let class_idx = objects.len();
    objects.push(class_descriptor(
        "XCTTestIdentifier",
        &["XCTTestIdentifier", "NSObject"],
    ));

    let (class, method) = selector
        .split_once('/')
        .map(|(class, method)| (class, Some(method)))
        .unwrap_or((selector, None));
    // The upstream implementation intentionally ignores the optional module
    // in this representation: testmanagerd's identifier set expects class and
    // method components, while the module remains in testsToRun.
    let class = class
        .split_once('.')
        .map(|(_, class)| class)
        .unwrap_or(class);
    let components = match method {
        Some(method) => vec![
            Value::String(class.to_string()),
            Value::String(method.to_string()),
        ],
        None => vec![Value::String(class.to_string())],
    };
    let options = if method.is_some() { 2 } else { 3 };
    let components_uid = archive_value_into(Value::Array(components), objects);

    let object_idx = objects.len();
    let mut object = Dictionary::new();
    object.insert("$class".to_string(), Value::Uid(Uid::new(class_idx as u64)));
    object.insert("c".to_string(), components_uid);
    object.insert("o".to_string(), Value::Integer(options.into()));
    objects.push(Value::Dictionary(object));
    Value::Uid(Uid::new(object_idx as u64))
}

fn archive_configuration_field(value: Value, objects: &mut Vec<Value>) -> Value {
    match value {
        Value::String(_)
        | Value::Data(_)
        | Value::Date(_)
        | Value::Array(_)
        | Value::Dictionary(_) => archive_value_into(value, objects),
        // UID(0) is the NSKeyedArchiver `$null` sentinel. Preserve all existing
        // references supplied by callers instead of wrapping the UID itself in
        // another object-table entry.
        Value::Uid(_) | Value::Integer(_) | Value::Real(_) | Value::Boolean(_) => value,
        other => other,
    }
}

fn class_descriptor(classname: &str, classes: &[&str]) -> Value {
    let mut class_obj = Dictionary::new();
    class_obj.insert(
        "$classname".to_string(),
        Value::String(classname.to_string()),
    );
    class_obj.insert(
        "$classes".to_string(),
        Value::Array(
            classes
                .iter()
                .map(|name| Value::String((*name).to_string()))
                .collect(),
        ),
    );
    Value::Dictionary(class_obj)
}

// ── Internal helpers ──────────────────────────────────────────────────────────

/// Encode a simple scalar value (String, Integer, Real, Boolean, Data).
fn archive_value(val: Value) -> Vec<u8> {
    let objects = vec![Value::String("$null".to_string()), val];
    let root_doc = build_keyed_archive(Value::Uid(Uid::new(1)), objects);
    to_binary_plist(&root_doc)
}

fn build_keyed_archive(root_uid: Value, objects: Vec<Value>) -> Value {
    let mut top = Dictionary::new();
    top.insert("root".to_string(), root_uid);

    let mut doc = Dictionary::new();
    doc.insert(
        "$archiver".to_string(),
        Value::String("NSKeyedArchiver".to_string()),
    );
    doc.insert("$version".to_string(), Value::Integer(100000.into()));
    doc.insert("$top".to_string(), Value::Dictionary(top));
    doc.insert("$objects".to_string(), Value::Array(objects));
    Value::Dictionary(doc)
}

// Safety: plist::to_writer_binary into a Vec<u8> performs only in-memory writes,
// which are infallible (the only failure mode is OOM, which triggers a panic via
// the global allocator, not an Err). The unwrap is therefore safe.
fn to_binary_plist(val: &Value) -> Vec<u8> {
    let mut buf = Vec::new();
    plist::to_writer_binary(&mut buf, val).unwrap();
    buf
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::proto::nskeyedarchiver::ArchiveValue;

    fn plist_doc(data: &[u8]) -> Value {
        plist::from_bytes(data).unwrap()
    }

    fn objects(data: &[u8]) -> Vec<Value> {
        let plist = plist_doc(data);
        plist.as_dictionary().unwrap()["$objects"]
            .as_array()
            .unwrap()
            .clone()
    }

    fn root_index(data: &[u8]) -> usize {
        let plist = plist_doc(data);
        let top = plist.as_dictionary().unwrap()["$top"]
            .as_dictionary()
            .unwrap();
        match &top["root"] {
            Value::Uid(uid) => uid.get() as usize,
            other => panic!("unexpected root reference: {other:?}"),
        }
    }

    fn root_object<'a>(data: &[u8], objects: &'a [Value]) -> &'a Dictionary {
        objects[root_index(data)].as_dictionary().unwrap()
    }

    #[test]
    fn test_archive_string_is_valid_plist() {
        let data = archive_string("_requestChannelWithCode:identifier:");
        // Should start with 'bplist00'
        assert_eq!(&data[..6], b"bplist");
        // Should be decodable
        let _val: Value = plist::from_bytes(&data).unwrap();
        // Root should be recoverable via unarchive
        let recovered = crate::proto::nskeyedarchiver::unarchive(&data).unwrap();
        assert_eq!(
            recovered.as_str(),
            Some("_requestChannelWithCode:identifier:")
        );
    }

    #[test]
    fn test_archive_int() {
        let data = archive_int(42);
        let recovered = crate::proto::nskeyedarchiver::unarchive(&data).unwrap();
        assert_eq!(recovered.as_int(), Some(42));
    }

    #[test]
    fn test_archive_null_stores_nsnull_class_descriptor() {
        let data = archive_null();
        let objects = objects(&data);
        let root = root_object(&data, &objects);
        let class_ref = match &root["$class"] {
            Value::Uid(uid) => uid.get() as usize,
            _ => panic!("expected uid"),
        };
        assert_eq!(
            objects[class_ref].as_dictionary().unwrap()["$classname"].as_string(),
            Some("NSNull")
        );
    }

    #[test]
    fn test_archive_null_roundtrips_to_null() {
        let data = archive_null();
        let recovered = crate::proto::nskeyedarchiver::unarchive(&data).unwrap();
        assert!(matches!(
            recovered,
            crate::proto::nskeyedarchiver::ArchiveValue::Null
        ));
    }

    #[test]
    fn test_archive_roundtrip_nonempty() {
        let s = archive_string("com.apple.instruments.server.services.sysmontap");
        assert!(!s.is_empty());
        assert!(s.len() > 8);
    }

    #[test]
    fn test_archive_array_preserves_item_order() {
        let data = archive_array(vec![
            Value::Integer(12.into()),
            Value::Integer(34.into()),
            Value::Integer(56.into()),
        ]);
        let recovered = crate::proto::nskeyedarchiver::unarchive(&data).unwrap();
        let values = recovered.as_array().unwrap();
        assert_eq!(values[0].as_int(), Some(12));
        assert_eq!(values[1].as_int(), Some(34));
        assert_eq!(values[2].as_int(), Some(56));
    }

    #[test]
    fn test_archive_dict_roundtrips_nested_dictionary_values() {
        let nested = Dictionary::from_iter([
            (
                "inner-key".to_string(),
                Value::String("inner-value".to_string()),
            ),
            ("inner-int".to_string(), Value::Integer(7.into())),
        ]);
        let data = archive_dict(vec![(
            "outer".to_string(),
            Value::Array(vec![Value::Dictionary(nested)]),
        )]);

        let recovered = crate::proto::nskeyedarchiver::unarchive(&data).unwrap();
        let dict = recovered.as_dict().expect("root should be a dictionary");
        let outer = dict.get("outer").expect("outer key should exist");
        let outer_items = outer.as_array().expect("outer should be an array");
        let first = outer_items.first().expect("outer should contain one item");
        let nested = first
            .as_dict()
            .expect("nested dictionary should survive archiving");

        assert_eq!(
            nested.get("inner-key").and_then(|value| value.as_str()),
            Some("inner-value")
        );
        assert_eq!(
            nested.get("inner-int").and_then(|value| value.as_int()),
            Some(7)
        );
    }

    #[test]
    fn test_archive_uuid_stores_nsuuid_class_and_bytes() {
        let uuid = Uuid::parse_str("00112233-4455-6677-8899-aabbccddeeff").unwrap();
        let data = archive_uuid(uuid);
        let objects = objects(&data);
        let root = root_object(&data, &objects);
        assert_eq!(
            root["NS.uuidbytes"].as_data().unwrap(),
            &uuid.into_bytes().to_vec()
        );
        let class_ref = match &root["$class"] {
            Value::Uid(uid) => uid.get() as usize,
            _ => panic!("expected uid"),
        };
        let class = objects[class_ref].as_dictionary().unwrap();
        assert_eq!(class["$classname"].as_string(), Some("NSUUID"));
    }

    #[test]
    fn test_archive_nsurl_stores_file_relative_path() {
        let data = archive_nsurl(NsUrl {
            path: "/private/tmp/TestBundle.xctest".to_string(),
        });
        let objects = objects(&data);
        let root = root_object(&data, &objects);
        let rel_ref = match &root["NS.relative"] {
            Value::Uid(uid) => uid.get() as usize,
            _ => panic!("expected uid"),
        };
        assert_eq!(
            objects[rel_ref].as_string(),
            Some("file:///private/tmp/TestBundle.xctest")
        );
    }

    #[test]
    fn test_archive_xct_capabilities_stores_capabilities_dictionary() {
        let data = archive_xct_capabilities(XctCapabilities {
            capabilities: vec![(
                "expected failure test capability".to_string(),
                Value::Boolean(true),
            )],
        });
        let objects = objects(&data);
        let root = root_object(&data, &objects);
        let class_ref = match &root["$class"] {
            Value::Uid(uid) => uid.get() as usize,
            _ => panic!("expected uid"),
        };
        assert_eq!(
            objects[class_ref].as_dictionary().unwrap()["$classname"].as_string(),
            Some("XCTCapabilities")
        );
        let dict_ref = match &root["capabilities-dictionary"] {
            Value::Uid(uid) => uid.get() as usize,
            _ => panic!("expected uid"),
        };
        let dict = objects[dict_ref].as_dictionary().unwrap();
        assert!(dict.contains_key("NS.keys"));
        assert!(dict.contains_key("NS.objects"));
    }

    #[test]
    fn test_archive_xctest_configuration_stores_nested_testmanager_objects() {
        let data = archive_xctest_configuration(XcTestConfiguration {
            session_identifier: Uuid::parse_str("00112233-4455-6677-8899-aabbccddeeff").unwrap(),
            test_bundle_url: NsUrl {
                path: "/private/tmp/WebDriverAgentRunner.xctest".to_string(),
            },
            ide_capabilities: XctCapabilities {
                capabilities: vec![("XCTIssue capability".to_string(), Value::Boolean(true))],
            },
            automation_framework_path:
                "/System/Developer/Library/PrivateFrameworks/XCTAutomationSupport.framework"
                    .to_string(),
            initialize_for_ui_testing: true,
            report_results_to_ide: true,
            tests_must_run_on_main_thread: true,
            test_timeouts_enabled: false,
            additional_fields: Vec::new(),
        });

        let objects = objects(&data);
        let root = root_object(&data, &objects);
        let class_ref = match &root["$class"] {
            Value::Uid(uid) => uid.get() as usize,
            _ => panic!("expected uid"),
        };
        assert_eq!(
            objects[class_ref].as_dictionary().unwrap()["$classname"].as_string(),
            Some("XCTestConfiguration")
        );
        assert!(matches!(root.get("sessionIdentifier"), Some(Value::Uid(_))));
        assert!(matches!(root.get("testBundleURL"), Some(Value::Uid(_))));
        assert!(matches!(root.get("IDECapabilities"), Some(Value::Uid(_))));
        assert_eq!(root["reportResultsToIDE"].as_boolean(), Some(true));
        assert_eq!(root["testsMustRunOnMainThread"].as_boolean(), Some(true));
    }

    #[test]
    fn test_archive_xctest_configuration_archives_object_valued_additional_fields() {
        let data = archive_xctest_configuration(XcTestConfiguration {
            session_identifier: Uuid::parse_str("00112233-4455-6677-8899-aabbccddeeff").unwrap(),
            test_bundle_url: NsUrl {
                path: "/private/tmp/WebDriverAgentRunner.xctest".to_string(),
            },
            ide_capabilities: XctCapabilities {
                capabilities: Vec::new(),
            },
            automation_framework_path:
                "/System/Developer/Library/PrivateFrameworks/XCTAutomationSupport.framework"
                    .to_string(),
            initialize_for_ui_testing: true,
            report_results_to_ide: true,
            tests_must_run_on_main_thread: true,
            test_timeouts_enabled: false,
            additional_fields: vec![
                (
                    "targetApplicationPath".to_string(),
                    Value::String("/private/var/containers/Bundle/Application/App.app".into()),
                ),
                (
                    "targetApplicationArguments".to_string(),
                    Value::Array(vec![Value::String("-AppleLanguages".into())]),
                ),
                (
                    "targetApplicationEnvironment".to_string(),
                    Value::Dictionary(Dictionary::from_iter([(
                        "LANG".to_string(),
                        Value::String("en_US".into()),
                    )])),
                ),
            ],
        });

        let archived_objects = objects(&data);
        let root = root_object(&data, &archived_objects);

        let path_uid = match &root["targetApplicationPath"] {
            Value::Uid(uid) => uid.get() as usize,
            other => panic!("targetApplicationPath should be a UID, got {other:?}"),
        };
        assert_eq!(
            archived_objects[path_uid].as_string(),
            Some("/private/var/containers/Bundle/Application/App.app")
        );

        let args_uid = match &root["targetApplicationArguments"] {
            Value::Uid(uid) => uid.get() as usize,
            other => panic!("targetApplicationArguments should be a UID, got {other:?}"),
        };
        let args_object = archived_objects[args_uid]
            .as_dictionary()
            .expect("arguments should be an NSArray object");
        let arg_uid = match args_object["NS.objects"].as_array().unwrap().first() {
            Some(Value::Uid(uid)) => uid.get() as usize,
            other => panic!("array item should be a UID, got {other:?}"),
        };
        assert_eq!(
            archived_objects[arg_uid].as_string(),
            Some("-AppleLanguages")
        );

        assert!(matches!(
            root["targetApplicationEnvironment"],
            Value::Uid(_)
        ));

        let decoded = crate::proto::nskeyedarchiver::unarchive(&data).unwrap();
        let decoded = decoded
            .as_dict()
            .expect("configuration should decode as dict");
        assert_eq!(
            decoded
                .get("targetApplicationPath")
                .and_then(ArchiveValue::as_str),
            Some("/private/var/containers/Bundle/Application/App.app")
        );
        assert!(decoded
            .get("targetApplicationArguments")
            .and_then(ArchiveValue::as_array)
            .is_some());
    }

    #[test]
    fn test_archive_xctest_configuration_uses_selector_sets_and_identifiers() {
        let data = archive_xctest_configuration(XcTestConfiguration {
            session_identifier: Uuid::parse_str("00112233-4455-6677-8899-aabbccddeeff").unwrap(),
            test_bundle_url: NsUrl {
                path: "PlugIns/DemoTests.xctest".to_string(),
            },
            ide_capabilities: XctCapabilities {
                capabilities: Vec::new(),
            },
            automation_framework_path:
                "/System/Developer/Library/PrivateFrameworks/XCTAutomationSupport.framework"
                    .to_string(),
            initialize_for_ui_testing: false,
            report_results_to_ide: true,
            tests_must_run_on_main_thread: true,
            test_timeouts_enabled: false,
            additional_fields: vec![
                (
                    "testsToRun".to_string(),
                    Value::Array(vec![
                        Value::String("DemoTests.LoginTests/testHappyPath".to_string()),
                        Value::String("UnicodeTests".to_string()),
                    ]),
                ),
                (
                    "testsToSkip".to_string(),
                    Value::Array(vec![Value::String("FlakyTests/testEventually".to_string())]),
                ),
            ],
        });
        let archived_objects = objects(&data);
        let root = root_object(&data, &archived_objects);

        let class_name = |uid: &Value| {
            let class_uid = match uid {
                Value::Uid(uid) => uid.get() as usize,
                other => panic!("expected object uid, got {other:?}"),
            };
            let class_ref = match archived_objects[class_uid]
                .as_dictionary()
                .and_then(|object| object.get("$class"))
            {
                Some(Value::Uid(uid)) => uid.get() as usize,
                other => panic!("expected class uid, got {other:?}"),
            };
            archived_objects[class_ref]
                .as_dictionary()
                .and_then(|class| class.get("$classname"))
                .and_then(Value::as_string)
        };

        assert_eq!(
            class_name(&root["testsToRun"]),
            Some("NSSet"),
            "legacy selectors must use NSSet"
        );
        assert_eq!(
            class_name(&root["testIdentifiersToRun"]),
            Some("XCTTestIdentifierSet")
        );
        assert_eq!(
            class_name(&root["testIdentifiersToSkip"]),
            Some("XCTTestIdentifierSet")
        );

        let identifier_set_uid = match &root["testIdentifiersToRun"] {
            Value::Uid(uid) => uid.get() as usize,
            _ => panic!("expected identifier-set uid"),
        };
        let identifier_set = archived_objects[identifier_set_uid]
            .as_dictionary()
            .unwrap();
        let array_uid = match &identifier_set["identifiers"] {
            Value::Uid(uid) => uid.get() as usize,
            _ => panic!("expected identifier array uid"),
        };
        assert_eq!(
            class_name(&Value::Uid(Uid::new(array_uid as u64))),
            Some("NSMutableArray")
        );
        let array = archived_objects[array_uid].as_dictionary().unwrap();
        let first_uid = match array["NS.objects"].as_array().unwrap().first() {
            Some(Value::Uid(uid)) => uid.get() as usize,
            _ => panic!("expected first identifier uid"),
        };
        let first = archived_objects[first_uid].as_dictionary().unwrap();
        assert_eq!(first["o"].as_signed_integer(), Some(2));
        let components_uid = match &first["c"] {
            Value::Uid(uid) => uid.get() as usize,
            _ => panic!("expected components uid"),
        };
        let components = archived_objects[components_uid].as_dictionary().unwrap();
        let component_strings = components["NS.objects"]
            .as_array()
            .unwrap()
            .iter()
            .map(|value| match value {
                Value::Uid(uid) => archived_objects[uid.get() as usize].as_string().unwrap(),
                _ => panic!("expected component uid"),
            })
            .collect::<Vec<_>>();
        assert_eq!(component_strings, ["LoginTests", "testHappyPath"]);
    }
}
