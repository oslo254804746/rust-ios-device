//! DTX message types and NSObject representation.

use bytes::Bytes;
use indexmap::IndexMap;

/// A fully decoded DTX message.
#[derive(Debug, Clone)]
pub struct DtxMessage {
    pub identifier: u32,
    pub conversation_idx: u32,
    pub channel_code: i32,
    pub expects_reply: bool,
    pub payload: DtxPayload,
}

/// DTX payload variants.
#[derive(Debug, Clone)]
pub enum DtxPayload {
    /// Method invocation: selector name + arguments.
    MethodInvocation {
        selector: String,
        args: Vec<NSObject>,
    },
    /// Response to a method invocation.
    Response(NSObject),
    /// Notification event.
    Notification { name: String, object: NSObject },
    /// Raw bytes (unparsed or binary payload).
    Raw(Bytes),
    /// Raw bytes plus decoded auxiliary arguments.
    RawWithAux { payload: Bytes, aux: Vec<NSObject> },
    /// Empty message (ack only).
    Empty,
}

/// NSObject value representation (simplified – no NSKeyedArchiver needed for most uses).
#[derive(Debug, Clone, PartialEq)]
pub enum NSObject {
    Int(i64),
    Uint(u64),
    Double(f64),
    Bool(bool),
    String(String),
    Data(Bytes),
    Array(Vec<NSObject>),
    Dict(IndexMap<String, NSObject>),
    Null,
}

impl NSObject {
    pub fn as_str(&self) -> Option<&str> {
        if let NSObject::String(s) = self {
            Some(s)
        } else {
            None
        }
    }

    pub fn as_int(&self) -> Option<i64> {
        if let NSObject::Int(n) = self {
            Some(*n)
        } else {
            None
        }
    }

    pub fn as_bool(&self) -> Option<bool> {
        if let NSObject::Bool(b) = self {
            Some(*b)
        } else {
            None
        }
    }
}

impl From<String> for NSObject {
    fn from(s: String) -> Self {
        NSObject::String(s)
    }
}

impl From<&str> for NSObject {
    fn from(s: &str) -> Self {
        NSObject::String(s.to_string())
    }
}

impl From<i64> for NSObject {
    fn from(n: i64) -> Self {
        NSObject::Int(n)
    }
}

impl From<bool> for NSObject {
    fn from(b: bool) -> Self {
        NSObject::Bool(b)
    }
}
