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

    let object_idx = objects.len();
    let class_idx = object_idx + 1;

    let mut object = Dictionary::new();
    object.insert("$class".to_string(), Value::Uid(Uid::new(class_idx as u64)));
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
        object.insert(key, value);
    }
    objects.push(Value::Dictionary(object));
    objects.push(class_descriptor(
        "XCTestConfiguration",
        &["XCTestConfiguration", "NSObject"],
    ));

    Value::Uid(Uid::new(object_idx as u64))
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
}
