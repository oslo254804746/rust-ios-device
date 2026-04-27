//! Compatibility shim – codec module kept for API compatibility.
//! The real XPC encode/decode is now in crate::xpc::message.

pub use crate::xpc::message::{decode_message, encode_message, XpcMessage, XpcValue};
