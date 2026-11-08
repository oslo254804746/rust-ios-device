//! Explicit historical plist-to-JSON result policies.
//!
//! These functions convert an already constructed value. Callers retain their
//! input/frame and output budgets. Conversion does not redact or truncate data.
//! Container traversal is iterative; diagnostic redaction remains independent.

use base64::Engine;
use plist::Value;
use serde_json::{Map, Value as JsonValue};

/// Encoding used for ordinary plist Data values, including nested values.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum DataEncoding {
    NumericArray,
    HexString,
    Base64Object,
}

/// Result policy with private fields so future policy additions are compatible.
/// Select an encoding explicitly; there is no implicit default result format.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PlistJsonPolicy {
    data: DataEncoding,
    date_and_uid: bool,
    zero_integer_fallback: bool,
}

impl PlistJsonPolicy {
    /// Preserve Date as XML UTC text, Uid as an unsigned number, and unknown or
    /// non-finite values as null, using the supplied Data encoding.
    pub const fn new(data: DataEncoding) -> Self {
        Self {
            data,
            date_and_uid: true,
            zero_integer_fallback: false,
        }
    }

    /// Historical info/diagnostics commands omit Date and Uid as JSON null.
    pub const fn without_date_and_uid(mut self) -> Self {
        self.date_and_uid = false;
        self
    }

    /// Preserve a legacy zero fallback for an integer that fits neither i64 nor
    /// u64. Locked plist constructors cannot currently construct that value.
    pub const fn with_zero_integer_fallback(mut self) -> Self {
        self.zero_integer_fallback = true;
        self
    }
}

/// Convert a borrowed plist value without changing the selected result policy.
pub fn to_json(value: &Value, policy: PlistJsonPolicy) -> JsonValue {
    convert(Input::Borrowed(value), policy)
}

/// Convert an owned value, moving ordinary strings/keys instead of cloning them.
pub fn into_json(value: Value, policy: PlistJsonPolicy) -> JsonValue {
    convert(Input::Owned(value), policy)
}

enum Input<'a> {
    Borrowed(&'a Value),
    Owned(Value),
}
// A cursor stores only the current container's iterator and output. Wide
// inputs do not create another queue/value buffer for every sibling.
enum Children<'a> {
    BorrowedArray(std::slice::Iter<'a, Value>),
    OwnedArray(std::vec::IntoIter<Value>),
    BorrowedDictionary(plist::dictionary::Iter<'a>),
    OwnedDictionary(plist::dictionary::IntoIter),
}

struct Frame<'a> {
    children: Children<'a>,
    output: JsonValue,
    key: Option<String>,
}

impl<'a> Frame<'a> {
    fn next(&mut self) -> Option<Input<'a>> {
        match &mut self.children {
            Children::BorrowedArray(values) => values.next().map(Input::Borrowed),
            Children::OwnedArray(values) => values.next().map(Input::Owned),
            Children::BorrowedDictionary(values) => values.next().map(|(key, value)| {
                self.key = Some(key.clone());
                Input::Borrowed(value)
            }),
            Children::OwnedDictionary(values) => values.next().map(|(key, value)| {
                self.key = Some(key);
                Input::Owned(value)
            }),
        }
    }

    fn accept(&mut self, value: JsonValue) {
        match &mut self.output {
            JsonValue::Array(values) => values.push(value),
            JsonValue::Object(values) => {
                values.insert(self.key.take().expect("dictionary child key"), value);
            }
            _ => unreachable!("conversion frame is a container"),
        }
    }
}

fn convert(value: Input<'_>, policy: PlistJsonPolicy) -> JsonValue {
    let mut next = Some(value);
    let mut completed = None;
    let mut frames: Vec<Frame<'_>> = Vec::new();
    loop {
        if let Some(value) = next.take() {
            let container = match value {
                Input::Borrowed(Value::Array(values)) => Some((
                    Children::BorrowedArray(values.iter()),
                    JsonValue::Array(Vec::with_capacity(values.len())),
                )),
                Input::Owned(Value::Array(values)) => {
                    let length = values.len();
                    Some((
                        Children::OwnedArray(values.into_iter()),
                        JsonValue::Array(Vec::with_capacity(length)),
                    ))
                }
                Input::Borrowed(Value::Dictionary(values)) => Some((
                    Children::BorrowedDictionary(values.iter()),
                    JsonValue::Object(Map::new()),
                )),
                Input::Owned(Value::Dictionary(values)) => Some((
                    Children::OwnedDictionary(values.into_iter()),
                    JsonValue::Object(Map::new()),
                )),
                Input::Owned(Value::String(value)) => {
                    completed = Some(JsonValue::String(value));
                    None
                }
                Input::Owned(value) => {
                    completed = Some(scalar(&value, policy));
                    None
                }
                Input::Borrowed(value) => {
                    completed = Some(scalar(value, policy));
                    None
                }
            };
            if let Some((children, output)) = container {
                frames.push(Frame {
                    children,
                    output,
                    key: None,
                });
            }
        }
        if let Some(value) = completed.take() {
            match frames.last_mut() {
                Some(frame) => frame.accept(value),
                None => return value,
            }
        }
        // Each child is visited once before the next sibling. Empty containers
        // finish too. These invariants depend on typed iterators, not wire data.
        let frame = frames.last_mut().expect("unfinished conversion container");
        if let Some(value) = frame.next() {
            next = Some(value);
        } else {
            completed = Some(frames.pop().expect("conversion container").output);
        }
    }
}

fn scalar(value: &Value, policy: PlistJsonPolicy) -> JsonValue {
    match value {
        Value::String(value) => JsonValue::String(value.clone()),
        Value::Boolean(value) => JsonValue::Bool(*value),
        Value::Integer(value) => value
            .as_signed()
            .map(JsonValue::from)
            .or_else(|| value.as_unsigned().map(JsonValue::from))
            .unwrap_or_else(|| {
                if policy.zero_integer_fallback {
                    JsonValue::from(0)
                } else {
                    JsonValue::Null
                }
            }),
        Value::Real(value) => serde_json::Number::from_f64(*value)
            .map(JsonValue::Number)
            .unwrap_or(JsonValue::Null),
        Value::Data(value) => match policy.data {
            DataEncoding::NumericArray => {
                JsonValue::Array(value.iter().copied().map(JsonValue::from).collect())
            }
            DataEncoding::HexString => JsonValue::String(hex::encode(value)),
            DataEncoding::Base64Object => {
                serde_json::json!({"data_base64": base64::engine::general_purpose::STANDARD.encode(value)})
            }
        },
        Value::Date(value) if policy.date_and_uid => JsonValue::String(value.to_xml_format()),
        Value::Uid(value) if policy.date_and_uid => JsonValue::from(value.get()),
        _ => JsonValue::Null,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recorded_profiles_preserve_borrowed_and_owned_results() {
        for (policy, data, empty, metadata) in [
            (
                PlistJsonPolicy::new(DataEncoding::NumericArray),
                serde_json::json!([0, 127, 255]),
                serde_json::json!([]),
                true,
            ),
            (
                PlistJsonPolicy::new(DataEncoding::HexString),
                serde_json::json!("007fff"),
                serde_json::json!(""),
                true,
            ),
            (
                PlistJsonPolicy::new(DataEncoding::HexString)
                    .without_date_and_uid()
                    .with_zero_integer_fallback(),
                serde_json::json!("007fff"),
                serde_json::json!(""),
                false,
            ),
            (
                PlistJsonPolicy::new(DataEncoding::Base64Object),
                serde_json::json!({"data_base64":"AH//"}),
                serde_json::json!({"data_base64":""}),
                true,
            ),
        ] {
            let value = crate::plist_contract::value();
            crate::plist_contract::check(
                to_json(&value, policy),
                data.clone(),
                empty.clone(),
                metadata,
            );
            crate::plist_contract::check(into_json(value, policy), data, empty, metadata);
        }
    }

    #[test]
    fn containers_preserve_empty_shapes_and_dictionary_value_association() {
        let value = Value::Dictionary(plist::Dictionary::from_iter([
            ("last".to_string(), Value::Array(Vec::new())),
            (
                "first".to_string(),
                Value::Dictionary(plist::Dictionary::new()),
            ),
            ("middle".to_string(), Value::String("requested data".into())),
        ]));
        let expected = serde_json::json!({"last":[],"first":{},"middle":"requested data"});
        let policy = PlistJsonPolicy::new(DataEncoding::HexString);
        assert_eq!(to_json(&value, policy), expected);
        assert_eq!(into_json(value, policy), expected);
    }

    #[test]
    fn deeply_nested_values_use_an_iterative_conversion_walk() {
        // Exercise the borrowed and consuming public paths without recursive
        // cloning, serialization or dropping of this hostile depth fixture.
        for owned in [false, true] {
            let mut value = Value::String("leaf".into());
            for _ in 0..4096 {
                value = Value::Array(vec![value]);
            }
            let policy = PlistJsonPolicy::new(DataEncoding::NumericArray);
            let mut result = if owned {
                into_json(std::mem::replace(&mut value, Value::Boolean(false)), policy)
            } else {
                to_json(&value, policy)
            };
            // Unwind both values iteratively too; recursive JSON serialization/drop
            // have separate caller budgets and are not certified by this walk.
            for _ in 0..4096 {
                if !owned {
                    value = match value {
                        Value::Array(mut array) => array.pop().unwrap(),
                        _ => panic!("fixture depth"),
                    };
                }
                result = match result {
                    JsonValue::Array(mut array) => array.pop().unwrap(),
                    _ => panic!("result depth"),
                };
            }
            if !owned {
                assert_eq!(value.as_string(), Some("leaf"));
            }
            assert_eq!(result, "leaf");
        }
    }
}
