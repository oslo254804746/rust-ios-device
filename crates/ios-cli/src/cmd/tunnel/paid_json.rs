//! Actual owned JSON destinations. Decoder scratch and HTTP internals are separate.
use std::cell::RefCell;
use std::fmt;
use std::sync::Arc;

use anyhow::{Context as _, Result};
use serde::de::{DeserializeSeed, MapAccess, SeqAccess, Visitor};
use serde::ser::{SerializeMap, SerializeSeq};
use serde::{Serialize, Serializer};
use tokio::sync::{OwnedSemaphorePermit, Semaphore};

use super::metadata_vec::{exact_backing, KnownVec};

pub(super) struct Text {
    data: String,
    _storage: OwnedSemaphorePermit,
}
impl Text {
    pub(super) fn copy(value: &str, bank: &Arc<Semaphore>) -> Result<Self> {
        let allocation = exact_backing(bank, value.len())?;
        let storage = allocation.storage;
        let mut bytes = allocation.values;
        bytes.extend_from_slice(value.as_bytes());
        Ok(Self {
            data: String::from_utf8(bytes).expect("copied str remains UTF-8"),
            _storage: storage,
        })
    }
    pub(super) fn as_str(&self) -> &str {
        &self.data
    }
}
impl Serialize for Text {
    fn serialize<S: Serializer>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error> {
        serializer.serialize_str(self.as_str())
    }
}
impl std::ops::Deref for Text {
    type Target = str;
    fn deref(&self) -> &str {
        self.as_str()
    }
}

pub(super) enum Value {
    Null,
    Bool(bool),
    Number(serde_json::Number),
    Text(Text),
    Array(KnownVec<Value>),
    Object(KnownVec<Entry>),
}

// Arrival ordinal preserves last-value semantics through an in-place sort.
pub(super) type Entry = (Text, Value, usize);
impl Value {
    pub(super) fn object(&self) -> Option<&[Entry]> {
        match self {
            Self::Object(values) => Some(values.as_slice()),
            _ => None,
        }
    }
    pub(super) fn array(&self) -> Option<&[Value]> {
        match self {
            Self::Array(values) => Some(values.as_slice()),
            _ => None,
        }
    }
    pub(super) fn string(&self) -> Option<&str> {
        match self {
            Self::Text(value) => Some(value.as_str()),
            _ => None,
        }
    }
    pub(super) fn get(&self, key: &str) -> Option<&Value> {
        let values = self.object()?;
        values
            .binary_search_by(|(text, _, _)| text.as_str().cmp(key))
            .ok()
            .map(|index| &values[index].1)
    }
}
impl Serialize for Value {
    fn serialize<S: Serializer>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error> {
        match self {
            Self::Null => serializer.serialize_unit(),
            Self::Bool(value) => serializer.serialize_bool(*value),
            Self::Number(value) => value.serialize(serializer),
            Self::Text(value) => value.serialize(serializer),
            Self::Array(values) => {
                let mut output = serializer.serialize_seq(Some(values.len()))?;
                for value in values.as_slice() {
                    output.serialize_element(value)?;
                }
                output.end()
            }
            Self::Object(values) => {
                let mut output = serializer.serialize_map(Some(values.len()))?;
                for (key, value, _) in values.as_slice() {
                    output.serialize_entry(key, value)?;
                }
                output.end()
            }
        }
    }
}

struct Context {
    bank: Arc<Semaphore>,
    failure: RefCell<Option<anyhow::Error>>,
}
impl Context {
    fn failed<E: serde::de::Error>(&self, error: anyhow::Error) -> E {
        let mut failure = self.failure.borrow_mut();
        if failure.is_none() {
            *failure = Some(error);
        }
        E::custom("upstream JSON destination admission failed")
    }
}

struct Seed<'a>(&'a Context);
impl<'de> DeserializeSeed<'de> for Seed<'_> {
    type Value = Value;
    fn deserialize<D: serde::Deserializer<'de>>(
        self,
        deserializer: D,
    ) -> std::result::Result<Value, D::Error> {
        deserializer.deserialize_any(self)
    }
}
impl<'de> Visitor<'de> for Seed<'_> {
    type Value = Value;
    fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("a JSON value")
    }
    fn visit_unit<E: serde::de::Error>(self) -> std::result::Result<Value, E> {
        Ok(Value::Null)
    }
    fn visit_bool<E: serde::de::Error>(self, value: bool) -> std::result::Result<Value, E> {
        Ok(Value::Bool(value))
    }
    fn visit_i64<E: serde::de::Error>(self, value: i64) -> std::result::Result<Value, E> {
        Ok(Value::Number(value.into()))
    }
    fn visit_u64<E: serde::de::Error>(self, value: u64) -> std::result::Result<Value, E> {
        Ok(Value::Number(value.into()))
    }
    fn visit_f64<E: serde::de::Error>(self, value: f64) -> std::result::Result<Value, E> {
        Ok(Value::Number(
            serde_json::Number::from_f64(value)
                .ok_or_else(|| E::custom("nonfinite JSON number"))?,
        ))
    }
    fn visit_str<E: serde::de::Error>(self, value: &str) -> std::result::Result<Value, E> {
        Text::copy(value, &self.0.bank)
            .map(Value::Text)
            .map_err(|error| self.0.failed(error))
    }
    fn visit_seq<A: SeqAccess<'de>>(self, mut input: A) -> std::result::Result<Value, A::Error> {
        let mut values = KnownVec::new(self.0.bank.clone(), usize::MAX);
        while input
            .next_element_seed(Element {
                context: self.0,
                values: &mut values,
            })?
            .is_some()
        {}
        Ok(Value::Array(values))
    }
    fn visit_map<A: MapAccess<'de>>(self, mut input: A) -> std::result::Result<Value, A::Error> {
        let mut values: KnownVec<Entry> = KnownVec::new(self.0.bank.clone(), usize::MAX);
        while let Some(key) = input.next_key_seed(Key(self.0))? {
            values
                .reserve(1)
                .map_err(|error| self.0.failed::<A::Error>(error))?;
            let value = input.next_value_seed(Seed(self.0))?;
            let arrival = values.len();
            values.push_reserved((key, value, arrival));
        }
        values.as_mut_slice().sort_unstable_by(|left, right| {
            left.0
                .as_str()
                .cmp(right.0.as_str())
                .then(left.2.cmp(&right.2))
        });
        values.dedup_by(|later, earlier| {
            if later.0.as_str() == earlier.0.as_str() {
                // Keep the first key and last complete value, matching Value.
                earlier.1 = std::mem::replace(&mut later.1, Value::Null);
                true
            } else {
                false
            }
        });
        Ok(Value::Object(values))
    }
}

struct Element<'a> {
    context: &'a Context,
    values: &'a mut KnownVec<Value>,
}
impl<'de> DeserializeSeed<'de> for Element<'_> {
    type Value = ();
    fn deserialize<D: serde::Deserializer<'de>>(
        self,
        deserializer: D,
    ) -> std::result::Result<(), D::Error> {
        self.values
            .reserve(1)
            .map_err(|error| self.context.failed::<D::Error>(error))?;
        let value = Seed(self.context).deserialize(deserializer)?;
        self.values.push_reserved(value);
        Ok(())
    }
}
struct Key<'a>(&'a Context);
impl<'de> DeserializeSeed<'de> for Key<'_> {
    type Value = Text;
    fn deserialize<D: serde::Deserializer<'de>>(
        self,
        deserializer: D,
    ) -> std::result::Result<Text, D::Error> {
        deserializer.deserialize_str(self)
    }
}
impl<'de> Visitor<'de> for Key<'_> {
    type Value = Text;
    fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("a JSON object key")
    }
    fn visit_str<E: serde::de::Error>(self, value: &str) -> std::result::Result<Text, E> {
        Text::copy(value, &self.0.bank).map_err(|error| self.0.failed(error))
    }
}

pub(super) fn parse(bytes: &[u8], bank: &Arc<Semaphore>) -> Result<Value> {
    let context = Context {
        bank: bank.clone(),
        failure: RefCell::new(None),
    };
    let mut deserializer = serde_json::Deserializer::from_slice(bytes);
    let result = Seed(&context).deserialize(&mut deserializer);
    match result {
        Ok(value) => {
            deserializer
                .end()
                .context("invalid upstream listing JSON")?;
            Ok(value)
        }
        Err(error) => match context.failure.into_inner() {
            Some(original) => Err(original.context("upstream JSON destination admission failed")),
            None => Err(anyhow::Error::new(error).context("invalid upstream listing JSON")),
        },
    }
}

pub(super) struct Listing(Value);
impl Listing {
    pub(super) fn parse(bytes: &[u8], bank: &Arc<Semaphore>) -> Result<Self> {
        let value = parse(bytes, bank)?;
        if !value.object().is_some_and(|groups| {
            groups
                .iter()
                .all(|(_, entries, _)| entries.array().is_some())
        }) {
            return Err(
                anyhow::Error::new(<serde_json::Error as serde::de::Error>::custom(
                    "expected an object of JSON arrays",
                ))
                .context("invalid upstream tunnel listing"),
            );
        }
        Ok(Self(value))
    }
    pub(super) fn groups(&self) -> &[Entry] {
        self.0.object().expect("validated listing object")
    }
    pub(super) fn entries(&self, uid: &str) -> Option<&[Value]> {
        self.0.get(uid)?.array()
    }
    pub(super) fn retain(&mut self, mut keep: impl FnMut(&str) -> bool) {
        if let Value::Object(values) = &mut self.0 {
            values.retain(|(key, _, _)| keep(key.as_str()));
        }
    }
}

#[cfg(test)]
mod tests;
