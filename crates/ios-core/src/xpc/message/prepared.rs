//! Borrowed sending grammar for RSD; one admitted original wire allocation.
use super::*;
use crate::xpc::raw_storage::MutableBytes;
use std::sync::Arc;
use tokio::sync::Semaphore;

pub(crate) enum Value<'a> {
    Bool(bool),
    Uint64(u64),
    String(&'a str),
    Uuid(&'a [u8; 16]),
    Dictionary(&'a [(&'a str, Value<'a>)]),
}
fn add(a: usize, b: usize) -> Result<usize, XpcError> {
    a.checked_add(b)
        .ok_or_else(|| XpcError::Tls("XPC encoded length overflow".into()))
}
impl Value<'_> {
    fn len(&self) -> Result<usize, XpcError> {
        match self {
            Self::Bool(_) => Ok(8),
            Self::Uint64(_) => Ok(12),
            Self::Uuid(_) => Ok(20),
            Self::String(text) => {
                let total = add(text.len(), 1)?;
                checked_u32_len("string", total)?;
                add(8, checked_align4("string", total)?)
            }
            Self::Dictionary(values) => {
                checked_u32_len("dict count", values.len())?;
                let mut length = 4;
                for (key, value) in *values {
                    length = add(length, checked_align4("dict key", add(key.len(), 1)?)?)?;
                    length = add(length, value.len()?)?;
                }
                checked_collection_len("dict", length)?;
                add(8, length)
            }
        }
    }
    fn write(&self, writer: &mut Writer<'_>) {
        match self {
            Self::Bool(value) => {
                writer.u32(TYPE_BOOL);
                writer.u32(u32::from(*value));
            }
            Self::Uint64(value) => {
                writer.u32(TYPE_UINT64);
                writer.raw(&value.to_le_bytes());
            }
            Self::Uuid(value) => {
                writer.u32(TYPE_UUID);
                writer.raw(&value[..]);
            }
            Self::String(text) => {
                writer.u32(TYPE_STRING);
                writer.u32((text.len() + 1) as u32);
                writer.raw(text.as_bytes());
                writer.zeros(((text.len() + 1 + 3) & !3) - text.len());
            }
            Self::Dictionary(values) => {
                writer.u32(TYPE_DICTIONARY);
                let length_at = writer.offset;
                writer.u32(0);
                let start = writer.offset;
                writer.u32(values.len() as u32);
                for (key, value) in *values {
                    writer.raw(key.as_bytes());
                    writer.zeros(((key.len() + 1 + 3) & !3) - key.len());
                    value.write(writer);
                }
                let len = (writer.offset - start) as u32;
                writer.bytes[length_at..length_at + 4].copy_from_slice(&len.to_le_bytes());
            }
        }
    }
}
struct Writer<'a> {
    bytes: &'a mut [u8],
    offset: usize,
}
impl Writer<'_> {
    fn raw(&mut self, data: &[u8]) {
        self.bytes[self.offset..self.offset + data.len()].copy_from_slice(data);
        self.offset += data.len();
    }
    fn u32(&mut self, value: u32) {
        self.raw(&value.to_le_bytes());
    }
    fn zeros(&mut self, len: usize) {
        self.offset += len;
    }
}

pub(crate) fn encode(
    flags: u32,
    msg_id: u64,
    body: Option<&Value<'_>>,
    bank: &Arc<Semaphore>,
) -> Result<MutableBytes, XpcError> {
    let body_len = body
        .map(|value| add(8, value.len()?))
        .transpose()?
        .unwrap_or(0);
    let declared = checked_u64_len("body", body_len)?;
    let mut owned = MutableBytes::zeroed(bank, add(24, body_len)?)?;
    let mut writer = Writer {
        bytes: &mut owned.bytes,
        offset: 0,
    };
    writer.u32(WRAPPER_MAGIC);
    writer.u32(flags);
    writer.raw(&declared.to_le_bytes());
    writer.raw(&msg_id.to_le_bytes());
    if let Some(body) = body {
        writer.u32(OBJECT_MAGIC);
        writer.u32(BODY_VERSION);
        body.write(&mut writer);
    }
    debug_assert_eq!(writer.offset, writer.bytes.len());
    Ok(owned)
}

#[cfg(test)]
mod tests;
