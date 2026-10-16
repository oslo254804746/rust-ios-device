//! Keep serde's structure grammar while supplying the actual string scratch.
use std::cell::RefCell;
use std::sync::Arc;

use anyhow::Result;
use serde_json::de::{Read, SliceRead};
use tokio::sync::Semaphore;

use super::super::metadata_vec::{exact_backing, AdmittedVec};
use super::Text;

pub(super) struct Syntax(AdmittedVec<u8>);

impl Syntax {
    pub(super) fn new(bytes: &[u8], bank: &Arc<Semaphore>) -> Result<Option<Self>> {
        if !bytes.contains(&b'\\') {
            return Ok(None);
        }
        let capacity = bytes.len().checked_add(1).ok_or_else(|| {
            crate::output::resource_decision("upstream JSON syntax layout overflow")
        })?;
        let mut allocation = exact_backing(bank, capacity)?;
        allocation.values.extend_from_slice(bytes);
        let mut inside = false;
        let mut escaped = false;
        for byte in &mut allocation.values {
            if !inside {
                inside = *byte == b'"';
            } else if escaped {
                *byte = b'x';
                escaped = false;
            } else {
                match *byte {
                    b'"' => inside = false,
                    b'\\' => {
                        *byte = b'x';
                        escaped = true;
                    }
                    _ => *byte = b'x',
                }
            }
        }
        if inside {
            // Let the requested string visitor return the original reader's
            // EOF/escape error, including any earlier invalid escape. This
            // byte is never exposed as decoded user data.
            allocation.values.push(b'"');
        }
        Ok(Some(Self(allocation)))
    }

    pub(super) fn as_slice(&self) -> &[u8] {
        &self.0.values
    }
}

pub(super) struct Decoder<'a> {
    original: &'a [u8],
    syntax: &'a [u8],
    reader: RefCell<SliceRead<'a>>,
}

impl<'a> Decoder<'a> {
    pub(super) fn new(original: &'a [u8], syntax: &'a [u8]) -> Self {
        Self {
            original,
            syntax,
            reader: RefCell::new(SliceRead::new(original)),
        }
    }

    pub(super) fn copy(&self, placeholder: &str, bank: &Arc<Semaphore>) -> Result<Text> {
        // The syntax copy contains no backslashes in strings, so the locked
        // slice deserializer borrows these exact bytes without private scratch.
        let start = (placeholder.as_ptr() as usize)
            .checked_sub(self.syntax.as_ptr() as usize)
            .expect("JSON placeholder must borrow syntax");
        let end = start
            .checked_add(placeholder.len())
            .filter(|end| *end <= self.original.len())
            .expect("JSON placeholder must identify original content");
        assert!(start > 0 && self.syntax[start - 1] == b'"');
        let raw = &self.original[start..end];
        // Decoding any JSON escape uses no more bytes than its original wire
        // content. Preallocate that actual Vec in full, including malformed
        // tokens, so the original parser's extend/push never grows it.
        let capacity = if raw.contains(&b'\\') { raw.len() } else { 0 };
        let allocation = exact_backing(bank, capacity)?;
        let _storage = allocation.storage;
        let mut scratch = allocation.values;
        let mut reader = self.reader.borrow_mut();
        assert!(reader.byte_offset() <= start);
        while reader.byte_offset() < start {
            reader.next()?;
        }
        // Use the FULL original input to preserve global line/column and the
        // original serde_json::Error. Visit tokens only when serde requests
        // them, preserving earlier structure/number/depth errors.
        let decoded = reader.parse_str(&mut scratch)?;
        let text = Text::copy(&decoded, bank)?;
        debug_assert_eq!(scratch.capacity(), capacity);
        Ok(text)
    }
}
