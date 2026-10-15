//! Original RSD wire messages from a borrowed stack grammar on banked paths.
use super::*;
use crate::xpc::message::prepared::{self, Value};
use crate::xpc::raw_storage::MutableBytes;
use std::sync::Arc;
use tokio::sync::Semaphore;

pub(super) enum Message {
    Legacy(Bytes),
    Metered(MutableBytes),
}
impl std::ops::Deref for Message {
    type Target = [u8];
    fn deref(&self) -> &[u8] {
        match self {
            Self::Legacy(bytes) => bytes,
            Self::Metered(bytes) => &bytes.bytes,
        }
    }
}
pub(super) fn control(
    bank: Option<&Arc<Semaphore>>,
    flags: u32,
    dictionary: bool,
) -> Result<Message, XpcError> {
    match bank {
        Some(bank) => Ok(Message::Metered(prepared::encode(
            flags,
            0,
            dictionary.then_some(&Value::Dictionary(&[])),
            bank,
        )?)),
        None => Ok(Message::Legacy(crate::xpc::message::encode_message(
            &XpcMessage {
                flags,
                msg_id: 0,
                body: dictionary.then(|| XpcValue::Dictionary(indexmap::IndexMap::new())),
            },
        )?)),
    }
}
pub(super) fn host(
    bank: Option<&Arc<Semaphore>>,
    identity: uuid::Uuid,
) -> Result<Message, XpcError> {
    let Some(bank) = bank else {
        return Ok(Message::Legacy(crate::xpc::message::encode_message(
            &device_handshake_message(identity),
        )?));
    };
    let properties = [
        (
            "RemoteXPCVersionFlags",
            Value::Uint64(0x0100_0000_0000_0006),
        ),
        ("SensitivePropertiesVisible", Value::Bool(true)),
    ];
    let fields = [
        ("MessageType", Value::String("Handshake")),
        ("MessagingProtocolVersion", Value::Uint64(7)),
        ("UUID", Value::Uuid(identity.as_bytes())),
        ("Properties", Value::Dictionary(&properties)),
        ("Services", Value::Dictionary(&[])),
    ];
    Ok(Message::Metered(prepared::encode(
        flags::ALWAYS_SET | flags::DATA,
        1,
        Some(&Value::Dictionary(&fields)),
        bank,
    )?))
}

#[cfg(test)]
mod tests;
