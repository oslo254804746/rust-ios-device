use std::collections::VecDeque;

use bytes::Bytes;
use tokio::io::{AsyncRead, AsyncWrite};

use super::tap::{TapClient, TapMessage};
use crate::dtx::codec::DtxError;

#[derive(Debug, Clone, PartialEq)]
pub enum ActivityTraceValue {
    Bytes(Bytes),
    Null,
    Struct(Vec<ActivityTraceValue>),
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct ActivityTraceEntry {
    pub process: u32,
    pub thread: u32,
    pub subsystem: String,
    pub category: String,
    pub message_type: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub event_type: Option<String>,
    pub sender_image_path: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub format_string: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    pub rendered_message: String,
}

#[derive(Debug, Clone)]
struct ActivityTraceTable {
    unknown0: Bytes,
    unknown2: Bytes,
    name: String,
    columns: Vec<String>,
}

#[derive(Default)]
pub struct ActivityTraceDecoder {
    stack: Vec<ActivityTraceValue>,
    generation: u32,
    background: u32,
    tables: Vec<ActivityTraceTable>,
}

pub struct ActivityTraceClient<S> {
    tap: TapClient<S>,
    decoder: ActivityTraceDecoder,
    pending: VecDeque<ActivityTraceEntry>,
    pid_filter: Option<u32>,
}

impl ActivityTraceDecoder {
    pub fn decode_message(&mut self, message: &[u8]) -> Result<Vec<ActivityTraceEntry>, DtxError> {
        if message.is_empty() || message.starts_with(b"bplist") {
            return Ok(vec![]);
        }

        let mut entries = Vec::new();
        let mut cursor = 0usize;
        while cursor + 2 <= message.len() {
            let word = read_word(message, &mut cursor)?;
            let opcode = word >> 8;
            let result = match opcode {
                CMD_TABLE_RESET => {
                    self.handle_table_reset();
                    None
                }
                CMD_SENTINEL => {
                    self.stack.push(ActivityTraceValue::Null);
                    None
                }
                CMD_STRUCT => {
                    self.handle_struct((word & 0xFF) as u8)?;
                    None
                }
                CMD_DEFINE_TABLE => {
                    self.handle_define_table()?;
                    None
                }
                CMD_DEBUG => {
                    self.handle_debug(word)?;
                    None
                }
                CMD_COPY => {
                    self.handle_copy(word)?;
                    None
                }
                CMD_END_ROW => self.handle_end_row((word & 0xFF) as usize)?,
                CMD_PLACEHOLDER_COUNT => {
                    self.handle_placeholder_count((word & 0xFF) as usize);
                    None
                }
                CMD_CONVERT_MACH_CONTINUOUS => None,
                _ => {
                    self.stack.push(handle_push(message, &mut cursor, word)?);
                    None
                }
            };

            if let Some(entry) = result {
                entries.push(entry);
            }
        }

        Ok(entries)
    }

    fn handle_table_reset(&mut self) {
        self.generation += 1;
        self.background = 0;
        self.stack.clear();
    }

    fn handle_struct(&mut self, distance: u8) -> Result<(), DtxError> {
        if distance == 0xFF {
            return Err(DtxError::Protocol(
                "activity trace long struct is not implemented".into(),
            ));
        }
        let distance = usize::from(distance);
        let item = ActivityTraceValue::Struct(self.pop_n(distance)?);
        self.stack.push(item);
        Ok(())
    }

    fn handle_define_table(&mut self) -> Result<(), DtxError> {
        let items = self.pop_n(4)?;
        let [unknown0, unknown2, name, columns] = items.try_into().map_err(|_| {
            DtxError::Protocol("activity trace table definition did not contain four items".into())
        })?;

        let name = decode_trace_string(&name).unwrap_or_default();
        let columns = match columns {
            ActivityTraceValue::Struct(items) => items
                .into_iter()
                .filter_map(|item| decode_trace_string(&item))
                .collect(),
            _ => {
                return Err(DtxError::Protocol(
                    "activity trace table columns were not a struct".into(),
                ))
            }
        };

        self.tables.push(ActivityTraceTable {
            unknown0: into_bytes(unknown0),
            unknown2: into_bytes(unknown2),
            name,
            columns,
        });
        Ok(())
    }

    fn handle_debug(&mut self, word: u16) -> Result<(), DtxError> {
        let debug_id = word & 0xFF;
        let Some(item) = self.stack.last() else {
            return Err(DtxError::Protocol(format!(
                "activity trace debug opcode {debug_id:#x} with empty stack"
            )));
        };
        let reference = trace_value_to_u64(item)? as usize;
        if reference != self.stack.len().saturating_sub(1) {
            return Err(DtxError::Protocol(format!(
                "activity trace debug reference mismatch: got {reference}, expected {}",
                self.stack.len().saturating_sub(1)
            )));
        }
        self.stack.pop();
        Ok(())
    }

    fn handle_copy(&mut self, word: u16) -> Result<(), DtxError> {
        let distance = usize::from((word & 0xFF) as u8);
        if distance != 0xFF {
            let index = self.stack.len().checked_sub(distance + 1).ok_or_else(|| {
                DtxError::Protocol(format!(
                    "activity trace copy distance {distance} exceeds stack size {}",
                    self.stack.len()
                ))
            })?;
            let item = self.stack[index].clone();
            self.stack.push(item);
            return Ok(());
        }

        let item = self.stack.pop().ok_or_else(|| {
            DtxError::Protocol("activity trace long copy missing reference".into())
        })?;
        let reference = trace_value_to_u64(&item)?.checked_sub(1).ok_or_else(|| {
            DtxError::Protocol("activity trace long copy reference underflow".into())
        })? as usize;
        let cloned = self.stack.get(reference).cloned().ok_or_else(|| {
            DtxError::Protocol(format!(
                "activity trace long copy reference {reference} out of bounds"
            ))
        })?;
        self.stack.push(cloned);
        Ok(())
    }

    fn handle_end_row(
        &mut self,
        generation: usize,
    ) -> Result<Option<ActivityTraceEntry>, DtxError> {
        let table = self
            .tables
            .get(generation)
            .ok_or_else(|| {
                DtxError::Protocol(format!(
                    "activity trace row references missing table generation {generation}"
                ))
            })?
            .clone();
        let _ = (&table.unknown0, &table.unknown2, &table.name);
        let row = self.pop_n(table.columns.len())?;
        let fields: std::collections::HashMap<_, _> =
            table.columns.iter().cloned().zip(row).collect();

        let Some(message_value) = fields.get("message") else {
            return Ok(None);
        };

        let process = fields
            .get("process")
            .and_then(|value| trace_value_to_u32(value).ok())
            .unwrap_or(0);
        let thread = fields
            .get("thread")
            .and_then(|value| trace_value_to_u32(value).ok())
            .unwrap_or(0);
        let subsystem = fields
            .get("subsystem")
            .and_then(decode_trace_string)
            .unwrap_or_default();
        let category = fields
            .get("category")
            .and_then(decode_trace_string)
            .unwrap_or_default();
        let message_type = fields
            .get("message_type")
            .and_then(decode_trace_string)
            .or_else(|| fields.get("event_type").and_then(decode_trace_string))
            .unwrap_or_else(|| "unknown".to_string());
        let event_type = fields.get("event_type").and_then(decode_trace_string);
        let sender_image_path = fields
            .get("sender_image_path")
            .and_then(decode_trace_string)
            .unwrap_or_default();
        let format_string = fields.get("format_string").and_then(decode_trace_string);
        let name = fields.get("name").and_then(decode_trace_string);
        let rendered_message = match message_value {
            ActivityTraceValue::Struct(items) => decode_message_format(items),
            ActivityTraceValue::Null => name.clone().unwrap_or_default(),
            _ => name.clone().unwrap_or_default(),
        };

        Ok(Some(ActivityTraceEntry {
            process,
            thread,
            subsystem,
            category,
            message_type,
            event_type,
            sender_image_path,
            format_string,
            name,
            rendered_message,
        }))
    }

    fn handle_placeholder_count(&mut self, count: usize) {
        if count == 0 {
            return;
        }
        let new_len = self.stack.len().saturating_sub(count);
        self.stack.truncate(new_len);
    }

    fn pop_n(&mut self, count: usize) -> Result<Vec<ActivityTraceValue>, DtxError> {
        if self.stack.len() < count {
            return Err(DtxError::Protocol(format!(
                "activity trace stack underflow: need {count}, have {}",
                self.stack.len()
            )));
        }
        Ok(self.stack.split_off(self.stack.len() - count))
    }
}

impl<S: AsyncRead + AsyncWrite + Unpin + Send> ActivityTraceClient<S> {
    pub async fn connect(stream: S, pid: Option<u32>, enable_har: bool) -> Result<Self, DtxError> {
        let tap = TapClient::connect(
            stream,
            super::ACTIVITY_TRACE_TAP_SVC,
            activity_trace_config(pid, enable_har),
        )
        .await?;
        Ok(Self {
            tap,
            decoder: ActivityTraceDecoder::default(),
            pending: VecDeque::new(),
            pid_filter: pid,
        })
    }

    pub async fn next_entry(&mut self) -> Result<ActivityTraceEntry, DtxError> {
        loop {
            if let Some(entry) = self.pending.pop_front() {
                if self.pid_filter.map_or(true, |pid| pid == entry.process) {
                    return Ok(entry);
                }
            }

            match self.tap.next_message().await? {
                TapMessage::Data(bytes) => match self.decoder.decode_message(&bytes) {
                    Ok(entries) => self.pending.extend(entries),
                    Err(error) => {
                        tracing::debug!("skipping undecodable activity trace frame: {error}");
                    }
                },
                TapMessage::Plist(_) => {}
            }
        }
    }

    pub async fn stop(&mut self) -> Result<(), DtxError> {
        self.tap.stop().await
    }
}

pub fn decode_message_format(message: &[ActivityTraceValue]) -> String {
    let mut output = String::new();
    for item in message {
        let ActivityTraceValue::Struct(parts) = item else {
            output.push_str(&format_trace_value(item));
            continue;
        };
        if parts.len() != 2 {
            output.push_str(&format_trace_value(item));
            continue;
        }

        let Some(type_name) = decode_trace_string(&parts[0]) else {
            output.push_str(&format_trace_value(item));
            continue;
        };
        let normalized = if type_name == "address" {
            "uint64-hex".to_string()
        } else {
            type_name
        };

        match normalized.as_str() {
            "narrative-text" | "string" => match decode_trace_string(&parts[1]) {
                Some(value) => output.push_str(&value),
                None if matches!(parts[1], ActivityTraceValue::Null) => output.push_str("<None>"),
                None => output.push_str(&format_trace_value(&parts[1])),
            },
            "private" => output.push_str("<private>"),
            ty if ty.starts_with("uint64") => {
                let value = trace_value_to_u64(&parts[1]).unwrap_or_default();
                if ty.contains("hex") {
                    let rendered = if ty.contains("lowercase") {
                        format!("{value:x}")
                    } else {
                        format!("{value:X}")
                    };
                    output.push_str(&rendered);
                } else {
                    output.push_str(&value.to_string());
                }
            }
            ty if ty.contains("decimal") => {
                output.push_str(
                    &trace_value_to_u64(&parts[1])
                        .unwrap_or_default()
                        .to_string(),
                );
            }
            "data" | "uuid" => {
                if let Some(bytes) = flatten_data_value(&parts[1]) {
                    output.push_str(&hex_string(&bytes));
                }
            }
            _ => output.push_str(&format_trace_value(&parts[1])),
        }
    }
    output
}

const CMD_DEFINE_TABLE: u16 = 1;
const CMD_END_ROW: u16 = 2;
const CMD_CONVERT_MACH_CONTINUOUS: u16 = 5;
const CMD_TABLE_RESET: u16 = 0x64;
const CMD_COPY: u16 = 0x65;
const CMD_SENTINEL: u16 = 0x68;
const CMD_STRUCT: u16 = 0x69;
const CMD_PLACEHOLDER_COUNT: u16 = 0x6A;
const CMD_DEBUG: u16 = 0x6B;

fn activity_trace_config(pid: Option<u32>, enable_har: bool) -> Vec<(String, plist::Value)> {
    let _ = pid;
    vec![
        ("bm".to_string(), plist::Value::Integer(0.into())),
        (
            "combineDataScope".to_string(),
            plist::Value::Integer(0.into()),
        ),
        (
            "machTimebaseDenom".to_string(),
            plist::Value::Integer(3.into()),
        ),
        (
            "machTimebaseNumer".to_string(),
            plist::Value::Integer(125.into()),
        ),
        (
            "onlySignposts".to_string(),
            plist::Value::Integer(0.into()),
        ),
        (
            "pidToInjectCombineDYLIB".to_string(),
            plist::Value::String("-1".to_string()),
        ),
        (
            "predicate".to_string(),
            plist::Value::String(
                "(messageType == info OR messageType == debug OR messageType == default OR messageType == error OR messageType == fault)"
                    .to_string(),
            ),
        ),
        (
            "signpostsAndLogs".to_string(),
            plist::Value::Integer(1.into()),
        ),
        (
            "trackPidToExecNameMapping".to_string(),
            plist::Value::Boolean(true),
        ),
        (
            "enableHTTPArchiveLogging".to_string(),
            plist::Value::Boolean(enable_har),
        ),
        (
            "targetPID".to_string(),
            plist::Value::Integer((-3).into()),
        ),
        (
            "trackExpiredPIDs".to_string(),
            plist::Value::Integer(1.into()),
        ),
        ("ur".to_string(), plist::Value::Integer(500.into())),
    ]
}

fn read_word(message: &[u8], cursor: &mut usize) -> Result<u16, DtxError> {
    if *cursor + 2 > message.len() {
        return Err(DtxError::Protocol(
            "activity trace message ended mid-word".into(),
        ));
    }
    let word = u16::from_le_bytes([message[*cursor], message[*cursor + 1]]);
    *cursor += 2;
    Ok(word)
}

fn handle_push(
    message: &[u8],
    cursor: &mut usize,
    mut word: u16,
) -> Result<ActivityTraceValue, DtxError> {
    let class = word >> 14;
    if class != 0b10 && class != 0b11 {
        return Err(DtxError::Protocol(format!(
            "invalid activity trace push word {word:#06x}"
        )));
    }

    let mut chunks = Vec::new();
    loop {
        chunks.push(word & 0x3FFF);
        if word >> 14 == 0b11 {
            break;
        }
        word = read_word(message, cursor)?;
    }

    let bit_count = chunks.len() * 14;
    let padding = 8 - (bit_count % 8);
    let total_bits = bit_count + padding;
    let mut bytes = vec![0u8; total_bits / 8];
    let mut bit_pos = 0usize;
    for chunk in chunks {
        for shift in (0..14).rev() {
            let bit = ((chunk >> shift) & 1) as u8;
            if bit != 0 {
                let byte_index = bit_pos / 8;
                let bit_index = 7 - (bit_pos % 8);
                bytes[byte_index] |= bit << bit_index;
            }
            bit_pos += 1;
        }
    }

    Ok(ActivityTraceValue::Bytes(Bytes::from(bytes)))
}

fn decode_trace_string(value: &ActivityTraceValue) -> Option<String> {
    let bytes = match value {
        ActivityTraceValue::Bytes(bytes) => bytes.as_ref(),
        ActivityTraceValue::Struct(items) => {
            return items.first().and_then(decode_trace_string);
        }
        _ => return None,
    };
    let trimmed = bytes.split(|byte| *byte == 0).next().unwrap_or_default();
    Some(String::from_utf8_lossy(trimmed).into_owned())
}

fn trace_value_to_u32(value: &ActivityTraceValue) -> Result<u32, DtxError> {
    let bytes = trace_value_leaf_bytes(value)
        .ok_or_else(|| DtxError::Protocol("activity trace expected byte payload for u32".into()))?;
    let mut buf = [0u8; 4];
    let count = bytes.len().min(4);
    buf[..count].copy_from_slice(&bytes[..count]);
    Ok(u32::from_le_bytes(buf))
}

fn trace_value_to_u64(value: &ActivityTraceValue) -> Result<u64, DtxError> {
    let bytes = trace_value_leaf_bytes(value)
        .ok_or_else(|| DtxError::Protocol("activity trace expected byte payload for u64".into()))?;
    let mut buf = [0u8; 8];
    let count = bytes.len().min(8);
    buf[..count].copy_from_slice(&bytes[..count]);
    Ok(u64::from_le_bytes(buf))
}

fn trace_value_leaf_bytes(value: &ActivityTraceValue) -> Option<&[u8]> {
    match value {
        ActivityTraceValue::Bytes(bytes) => Some(bytes),
        ActivityTraceValue::Struct(items) => items.first().and_then(trace_value_leaf_bytes),
        _ => None,
    }
}

fn flatten_data_value(value: &ActivityTraceValue) -> Option<Vec<u8>> {
    match value {
        ActivityTraceValue::Bytes(bytes) => Some(strip_one_trailing_null(bytes).to_vec()),
        ActivityTraceValue::Struct(items) => {
            let mut out = Vec::new();
            for item in items {
                out.extend(flatten_data_value(item)?);
            }
            Some(out)
        }
        ActivityTraceValue::Null => None,
    }
}

fn format_trace_value(value: &ActivityTraceValue) -> String {
    match value {
        ActivityTraceValue::Bytes(bytes) => {
            let trimmed = bytes.split(|byte| *byte == 0).next().unwrap_or_default();
            String::from_utf8_lossy(trimmed).into_owned()
        }
        ActivityTraceValue::Null => "<None>".to_string(),
        ActivityTraceValue::Struct(items) => format!("{items:?}"),
    }
}

fn hex_string(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn strip_one_trailing_null(bytes: &[u8]) -> &[u8] {
    if bytes.last() == Some(&0) {
        &bytes[..bytes.len() - 1]
    } else {
        bytes
    }
}

fn into_bytes(value: ActivityTraceValue) -> Bytes {
    match value {
        ActivityTraceValue::Bytes(bytes) => bytes,
        ActivityTraceValue::Null => Bytes::new(),
        ActivityTraceValue::Struct(_) => Bytes::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const CMD_DEFINE_TABLE: u16 = 1;
    const CMD_END_ROW: u16 = 2;
    const CMD_TABLE_RESET: u16 = 0x64;
    const CMD_STRUCT: u16 = 0x69;

    #[test]
    fn decodes_message_format() {
        let message = vec![ActivityTraceValue::Struct(vec![
            ActivityTraceValue::Bytes(Bytes::from_static(b"string\0\0")),
            ActivityTraceValue::Bytes(Bytes::from_static(b"hello\0\0\0")),
        ])];

        assert_eq!(decode_message_format(&message), "hello");
    }

    #[test]
    fn decodes_activity_trace_row() {
        let mut decoder = ActivityTraceDecoder::default();
        let payload = build_trace_message();
        let entries = decoder.decode_message(&payload).unwrap();
        assert_eq!(entries.len(), 1);

        let entry = &entries[0];
        assert_eq!(entry.process, 42);
        assert_eq!(entry.thread, 7);
        assert_eq!(entry.subsystem, "sub");
        assert_eq!(entry.category, "cat");
        assert_eq!(entry.message_type, "info");
        assert_eq!(entry.sender_image_path, "app");
        assert_eq!(entry.rendered_message, "hello");
    }

    fn build_trace_message() -> Vec<u8> {
        let mut words = Vec::new();

        words.push(opcode(CMD_TABLE_RESET, 0));

        words.extend(push_bytes(&[0]));
        words.extend(push_bytes(&[0]));
        words.extend(push_string("logs"));

        for column in [
            "process",
            "thread",
            "subsystem",
            "category",
            "message_type",
            "sender_image_path",
            "message",
        ] {
            words.extend(push_string(column));
        }
        words.push(opcode(CMD_STRUCT, 7));
        words.push(opcode(CMD_DEFINE_TABLE, 0));

        words.extend(push_u32(42));
        words.push(opcode(CMD_STRUCT, 1));
        words.extend(push_u32(7));
        words.push(opcode(CMD_STRUCT, 1));
        words.extend(push_string("sub"));
        words.extend(push_string("cat"));
        words.extend(push_string("info"));
        words.extend(push_string("app"));
        words.extend(push_string("string"));
        words.extend(push_string("hello"));
        words.push(opcode(CMD_STRUCT, 2));
        words.push(opcode(CMD_STRUCT, 1));
        words.push(opcode(CMD_END_ROW, 0));

        words
            .into_iter()
            .flat_map(u16::to_le_bytes)
            .collect::<Vec<_>>()
    }

    fn push_string(value: &str) -> Vec<u16> {
        let mut bytes = value.as_bytes().to_vec();
        bytes.push(0);
        push_bytes(&bytes)
    }

    fn push_u32(value: u32) -> Vec<u16> {
        let mut bytes = vec![0u8; 4];
        bytes[..4].copy_from_slice(&value.to_le_bytes());
        push_bytes(&bytes)
    }

    fn push_bytes(bytes: &[u8]) -> Vec<u16> {
        let group_count = bytes.len().div_ceil(7).max(1);
        let total_data_bytes = group_count * 7;
        let total_words = group_count * 4;
        let mut data = vec![0u8; total_data_bytes];
        data[..bytes.len()].copy_from_slice(bytes);

        let mut bit_pos = 0usize;
        let mut words = Vec::with_capacity(total_words);
        for index in 0..total_words {
            let mut chunk = 0u16;
            for _ in 0..14 {
                let bit = (data[bit_pos / 8] >> (7 - (bit_pos % 8))) & 1;
                chunk = (chunk << 1) | u16::from(bit);
                bit_pos += 1;
            }
            words.push(if index + 1 == total_words {
                terminal_word(chunk)
            } else {
                continuation_word(chunk)
            });
        }
        words
    }

    fn continuation_word(value: u16) -> u16 {
        (0b10 << 14) | (value & 0x3fff)
    }

    fn terminal_word(value: u16) -> u16 {
        (0b11 << 14) | (value & 0x3fff)
    }

    fn opcode(opcode: u16, arg: u8) -> u16 {
        (opcode << 8) | u16::from(arg)
    }
}
