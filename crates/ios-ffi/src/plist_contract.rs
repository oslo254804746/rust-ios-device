//! Fixed historical plist JSON shapes, independent of conversion implementation.

use plist::{Dictionary, Value};
use serde_json::{json, Value as JsonValue};

pub(crate) fn value() -> Value {
    let data = Value::Data(vec![0, 127, 255]);
    let date = Value::Date(plist::Date::from_xml_format("2001-01-01T00:00:00Z").unwrap());
    let uid = Value::Uid(plist::Uid::new(u64::MAX));
    let nested = Value::Dictionary(Dictionary::from_iter([
        ("data".to_string(), data.clone()),
        ("date".to_string(), date.clone()),
        ("uid".to_string(), uid.clone()),
    ]));
    Value::Dictionary(Dictionary::from_iter([
        ("data".to_string(), data),
        ("date".to_string(), date),
        ("uid".to_string(), uid),
        ("signed".to_string(), Value::Integer(i64::MIN.into())),
        ("unsigned".to_string(), Value::Integer(u64::MAX.into())),
        ("finite".to_string(), Value::Real(1.25)),
        ("nan".to_string(), Value::Real(f64::NAN)),
        ("infinity".to_string(), Value::Real(f64::INFINITY)),
        (
            "negative_infinity".to_string(),
            Value::Real(f64::NEG_INFINITY),
        ),
        ("boolean".to_string(), Value::Boolean(true)),
        (
            "text".to_string(),
            Value::String("synthetic requested text".into()),
        ),
        ("empty_data".to_string(), Value::Data(Vec::new())),
        ("nested".to_string(), Value::Array(vec![nested])),
    ]))
}

pub(crate) fn check(actual: JsonValue, data: JsonValue, empty: JsonValue, metadata: bool) {
    let date = if metadata {
        json!("2001-01-01T00:00:00Z")
    } else {
        JsonValue::Null
    };
    let uid = if metadata {
        json!(18446744073709551615_u64)
    } else {
        JsonValue::Null
    };
    let expected = json!({
        "data": data, "date": date, "uid": uid,
        "signed": -9223372036854775808_i64, "unsigned": 18446744073709551615_u64,
        "finite": 1.25, "nan": null, "infinity": null, "negative_infinity": null,
        "boolean": true, "text": "synthetic requested text", "empty_data": empty,
        "nested": [{"data": data, "date": date, "uid": uid}]
    });
    assert_eq!(actual, expected);
    // Freeze the complete JSON value, including number width, after serialization.
    assert_eq!(
        serde_json::from_str::<JsonValue>(&serde_json::to_string(&actual).unwrap()).unwrap(),
        expected
    );
}

#[test]
fn native_serde_plist_json_contract() {
    let value = value();
    check(
        serde_json::to_value(&value).unwrap(),
        json!([0, 127, 255]),
        json!([]),
        true,
    );
    let rendered = serde_json::to_string_pretty(&value).unwrap();
    check(
        serde_json::from_str(&rendered).unwrap(),
        json!([0, 127, 255]),
        json!([]),
        true,
    );
}
