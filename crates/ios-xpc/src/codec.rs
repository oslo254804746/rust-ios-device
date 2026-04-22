//! Compatibility shim – codec module kept for API compatibility.
//! The real XPC encode/decode is now in crate::message.

pub use crate::message::{decode_message, encode_message, XpcMessage, XpcValue};
