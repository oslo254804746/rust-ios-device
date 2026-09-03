//! DTX message encoder/decoder and connection manager.
//!
//! Reference: go-ios/ios/dtx_codec/decoder.go + encoder.go + connection.go
//!
//! Key wire format details (from encoder.go):
//! - Header: 32 bytes (magic BE + rest LE)
//! - Payload header: 16 bytes LE
//! - Aux header: 16 bytes LE (buffer_size=496, unknown=0, aux_size, unknown=0)
//! - Message type 0 = OK/Ack, type 2 = MethodInvocation, type 3 = Response/Object,
//!   type 4 = Error, type 5 = Barrier

use std::collections::{HashMap, HashSet, VecDeque};
use std::mem::size_of;

use crate::proto::dtx::DTX_MAGIC;
use crate::proto::nskeyedarchiver_encode;
use bytes::{BufMut, Bytes, BytesMut};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

use super::primitive_enc::{encode_primitive_dict, PrimArg};
use super::types::{DtxMessage, DtxPayload, NSObject};

// ── DTX message type constants ────────────────────────────────────────────────

const MAX_DTX_MESSAGE_SIZE: usize = 128 * 1024 * 1024;
const MAX_DTX_FRAGMENTS: u16 = 1024;
const MAX_DTX_HEADER_SIZE: usize = 64 * 1024;
/// Connection-wide budget for bytes retained by fragmented messages and
/// messages waiting for a synchronous caller. The old independent limits made
/// the theoretical worst case roughly 8 GiB (64 fragment accumulators × 128
/// MiB) plus another 128 GiB of queued messages (1024 × 128 MiB). 512 MiB is
/// enough for a large Instruments transfer and a few concurrent notifications,
/// while keeping an unresponsive peer well below those bounds.
const DEFAULT_DTX_MEMORY_BUDGET: usize = 512 * 1024 * 1024;
/// Bound the number of fragmented messages whose first header has arrived but
/// whose body fragments have not all been received yet. A first fragment only
/// costs a small amount of state, so without a connection-wide cap a peer could
/// keep sending first fragments forever and grow this map without bound.
const MAX_IN_FLIGHT_FRAGMENTED_MESSAGES: usize = 64;
/// A synchronous DTX call still has to consume unrelated device messages while
/// it waits for its reply. Keep that backlog finite so a peer that never sends
/// the requested reply cannot exhaust host memory.
const MAX_QUEUED_MESSAGES: usize = 1024;
/// Replies for other outstanding identifiers are retained while a synchronous
/// call waits. Empty replies have no dynamic payload bytes, so the byte budget
/// alone cannot bound the HashMap's entry/node overhead.
const MAX_PENDING_REPLIES: usize = 1024;

const MSG_OK: u32 = 0;
const MSG_UNKNOWN_TYPE_ONE: u32 = 1; // sysmontap data messages
const MSG_METHOD_INVOCATION: u32 = 2;
const MSG_RESPONSE: u32 = 3;
const MSG_ERROR: u32 = 4;
const MSG_BARRIER: u32 = 5;
const _MSG_LZ4_COMPRESSED: u32 = 0x0707; // LZ4 compressed payload

// ── Error ─────────────────────────────────────────────────────────────────────

#[derive(Debug, thiserror::Error)]
pub enum DtxError {
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
    #[error("bad magic: 0x{0:08X}")]
    BadMagic(u32),
    #[error("protocol error: {0}")]
    Protocol(String),
}

/// Bytes retained by a connection's receive-side state.
///
/// This deliberately accounts dynamic payload/selector/object bytes rather
/// than allocator metadata. Every reservation is checked and committed before
/// the corresponding Vec/Bytes allocation or message insertion, so a hostile
/// peer cannot turn the independent message/count limits into an unbounded
/// connection-wide allocation.
#[derive(Debug)]
struct DtxMemoryBudget {
    limit: usize,
    used: usize,
}

impl DtxMemoryBudget {
    fn new(limit: usize) -> Self {
        Self { limit, used: 0 }
    }

    fn try_reserve(&mut self, bytes: usize, what: &str) -> Result<(), DtxError> {
        let new_used = self.used.checked_add(bytes).ok_or_else(|| {
            DtxError::Protocol(format!(
                "DTX memory accounting overflow reserving {bytes} bytes for {what}: used={} limit={}",
                self.used, self.limit
            ))
        })?;
        if new_used > self.limit {
            return Err(DtxError::Protocol(format!(
                "DTX memory budget exceeded reserving {bytes} bytes for {what}: used={} limit={}",
                self.used, self.limit
            )));
        }
        self.used = new_used;
        Ok(())
    }

    fn release(&mut self, bytes: usize) {
        // A release is paired with a successful reservation. Saturating here
        // keeps malformed/error cleanup paths non-panicking even if a future
        // caller violates that invariant.
        self.used = self.used.saturating_sub(bytes);
    }
}

fn checked_size_add(total: &mut usize, bytes: usize, what: &str) -> Result<(), DtxError> {
    let current = *total;
    *total = current.checked_add(bytes).ok_or_else(|| {
        DtxError::Protocol(format!(
            "DTX memory size overflow while measuring {what}: current={current} additional={bytes}"
        ))
    })?;
    Ok(())
}

fn checked_size_mul(lhs: usize, rhs: usize, what: &str) -> Result<usize, DtxError> {
    lhs.checked_mul(rhs).ok_or_else(|| {
        DtxError::Protocol(format!(
            "DTX memory size overflow while measuring {what}: {lhs} * {rhs}"
        ))
    })
}

fn ns_object_memory_size(value: &NSObject) -> Result<usize, DtxError> {
    let mut size = 0;
    match value {
        NSObject::String(value) => checked_size_add(&mut size, value.capacity(), "string")?,
        NSObject::Data(value) => checked_size_add(&mut size, value.len(), "data")?,
        NSObject::Array(values) => {
            checked_size_add(
                &mut size,
                checked_size_mul(values.capacity(), size_of::<NSObject>(), "array slots")?,
                "array slots",
            )?;
            for value in values {
                checked_size_add(&mut size, ns_object_memory_size(value)?, "array value")?;
            }
        }
        NSObject::Dict(values) => {
            checked_size_add(
                &mut size,
                checked_size_mul(
                    values.capacity(),
                    size_of::<(String, NSObject)>(),
                    "dict slots",
                )?,
                "dict slots",
            )?;
            for (key, value) in values {
                checked_size_add(&mut size, key.capacity(), "dict key")?;
                checked_size_add(&mut size, ns_object_memory_size(value)?, "dict value")?;
            }
        }
        NSObject::Int(_)
        | NSObject::Uint(_)
        | NSObject::Double(_)
        | NSObject::Bool(_)
        | NSObject::Null => {}
    }
    Ok(size)
}

fn dtx_message_memory_size(message: &DtxMessage) -> Result<usize, DtxError> {
    let mut size = 0;
    match &message.payload {
        DtxPayload::MethodInvocation { selector, args } => {
            checked_size_add(&mut size, selector.capacity(), "method selector")?;
            checked_size_add(
                &mut size,
                checked_size_mul(
                    args.capacity(),
                    size_of::<NSObject>(),
                    "method argument slots",
                )?,
                "method argument slots",
            )?;
            for arg in args {
                checked_size_add(&mut size, ns_object_memory_size(arg)?, "method argument")?;
            }
        }
        DtxPayload::Response(value) => {
            checked_size_add(&mut size, ns_object_memory_size(value)?, "response")?;
        }
        DtxPayload::Notification { name, object } => {
            checked_size_add(&mut size, name.capacity(), "notification name")?;
            checked_size_add(
                &mut size,
                ns_object_memory_size(object)?,
                "notification object",
            )?;
        }
        DtxPayload::Raw(bytes) => checked_size_add(&mut size, bytes.len(), "raw payload")?,
        DtxPayload::RawWithAux { payload, aux } => {
            checked_size_add(&mut size, payload.len(), "raw payload")?;
            checked_size_add(
                &mut size,
                checked_size_mul(aux.capacity(), size_of::<NSObject>(), "auxiliary slots")?,
                "auxiliary slots",
            )?;
            for value in aux {
                checked_size_add(&mut size, ns_object_memory_size(value)?, "auxiliary value")?;
            }
        }
        DtxPayload::Empty => {}
    }
    Ok(size)
}

// ── DTX Encoder (matches go-ios encoder.go exactly) ──────────────────────────

/// Encode a full DTX message to bytes.
pub fn encode_dtx(
    identifier: u32,
    conv_idx: u32,
    channel_code: i32,
    expects_reply: bool,
    msg_type: u32,
    payload: &[u8],
    aux_bytes: &[u8],
) -> Bytes {
    let aux_len = aux_bytes.len();
    let payload_len = payload.len();

    // aux_length_with_header = aux_len + 16 (header) if aux_len > 0
    let aux_with_hdr = if aux_len > 0 { aux_len + 16 } else { 0 };
    let total_payload = aux_with_hdr + payload_len;
    let msg_len = 16 + aux_with_hdr + payload_len;

    let mut out = BytesMut::with_capacity(32 + msg_len);

    // Header (32 bytes)
    out.put_u32(DTX_MAGIC); // magic (BE)
    out.put_u32_le(32); // header_length
    out.put_u16_le(0); // fragment_index
    out.put_u16_le(1); // fragment_count
    out.put_u32_le(msg_len as u32); // message_length
    out.put_u32_le(identifier); // identifier
    out.put_u32_le(conv_idx); // conversation_index
    out.put_u32_le(channel_code as u32); // channel_code
    out.put_u32_le(if expects_reply { 1 } else { 0 }); // expects_reply

    // Payload header (16 bytes)
    out.put_u32_le(msg_type);
    out.put_u32_le(aux_with_hdr as u32);
    out.put_u32_le(total_payload as u32);
    out.put_u32_le(0); // flags

    if aux_len > 0 {
        // Aux header (16 bytes): buffer_size=496 as per go-ios writeAuxHeader
        out.put_u32_le(496);
        out.put_u32_le(0);
        out.put_u32_le(aux_len as u32);
        out.put_u32_le(0);
        out.put_slice(aux_bytes);
    }
    out.put_slice(payload);

    out.freeze()
}

/// Encode a DTX ack message (48 bytes total).
pub fn encode_ack(msg: &DtxMessage) -> Bytes {
    let mut out = BytesMut::with_capacity(48);
    let conversation_idx = msg.conversation_idx.saturating_add(1);
    // Incoming even-conversation messages are exposed to callers with their
    // logical channel code (the reader negates the wire code).  ACKs are odd
    // conversation messages, so put that channel back on the wire with the
    // same parity rule used by the reference sender.  For an incoming odd
    // conversation message the ACK conversation is even and the logical code
    // is already the wire code.  `saturating_neg` keeps malformed i32::MIN
    // input from panicking while preserving all valid channel codes exactly.
    let wire_channel_code = if conversation_idx % 2 == 0 {
        msg.channel_code
    } else {
        msg.channel_code.saturating_neg()
    };
    out.put_u32(DTX_MAGIC);
    out.put_u32_le(32);
    out.put_u16_le(0);
    out.put_u16_le(1);
    out.put_u32_le(16); // message_length = 16 (payload header only)
    out.put_u32_le(msg.identifier);
    // A malformed peer can supply the conversation index to this helper. Do
    // not let the host panic in debug builds when it happens to be u32::MAX;
    // the saturated value is still preferable to terminating the process while
    // attempting to acknowledge the frame.
    out.put_u32_le(conversation_idx);
    out.put_i32_le(wire_channel_code);
    out.put_u32_le(0); // expects_reply = false
                       // Payload header: type=0 (OK/Ack)
    out.put_u32_le(MSG_OK);
    out.put_u32_le(0);
    out.put_u32_le(0);
    out.put_u32_le(0);
    out.freeze()
}

// ── DTX frame reader ──────────────────────────────────────────────────────────

/// Raw DTX header fields parsed from the 32-byte wire header.
struct DtxHeader {
    header_len: usize,
    frag_idx: u16,
    frag_cnt: u16,
    msg_len: usize,
    identifier: u32,
    conv_idx: u32,
    channel_code: i32,
    expects_reply: bool,
}

// Safety: hdr is &[u8; 32], so all fixed-size slice conversions are infallible.
fn parse_dtx_header(hdr: &[u8; 32]) -> Result<DtxHeader, DtxError> {
    let magic = u32::from_be_bytes(hdr[0..4].try_into().unwrap());
    if magic != DTX_MAGIC {
        return Err(DtxError::BadMagic(magic));
    }
    let header_len = u32::from_le_bytes(hdr[4..8].try_into().unwrap()) as usize;
    // Both references emit a fixed 32-byte header; the upper bound only stops a
    // bogus length from driving a multi-gigabyte skip buffer.
    if !(32..=MAX_DTX_HEADER_SIZE).contains(&header_len) {
        return Err(DtxError::Protocol(format!(
            "invalid DTX header length: {header_len}"
        )));
    }
    let frag_idx = u16::from_le_bytes(hdr[8..10].try_into().unwrap());
    let frag_cnt = u16::from_le_bytes(hdr[10..12].try_into().unwrap());
    if frag_cnt == 0 {
        return Err(DtxError::Protocol("invalid DTX fragment count: 0".into()));
    }
    if frag_cnt > MAX_DTX_FRAGMENTS {
        return Err(DtxError::Protocol(format!(
            "DTX message has too many fragments: {frag_cnt} exceeds {MAX_DTX_FRAGMENTS}"
        )));
    }
    if frag_idx >= frag_cnt {
        return Err(DtxError::Protocol(format!(
            "invalid DTX fragment index {frag_idx} for count {frag_cnt}"
        )));
    }
    let msg_len = u32::from_le_bytes(hdr[12..16].try_into().unwrap()) as usize;
    if msg_len > MAX_DTX_MESSAGE_SIZE {
        return Err(DtxError::Protocol(format!(
            "DTX message length {msg_len} exceeds max {MAX_DTX_MESSAGE_SIZE}"
        )));
    }
    if frag_cnt > 1 && frag_idx == 0 && msg_len == 0 {
        return Err(DtxError::Protocol(
            "multi-fragment first header declares zero total size".into(),
        ));
    }
    Ok(DtxHeader {
        header_len,
        frag_idx,
        frag_cnt,
        msg_len,
        identifier: u32::from_le_bytes(hdr[16..20].try_into().unwrap()),
        conv_idx: u32::from_le_bytes(hdr[20..24].try_into().unwrap()),
        channel_code: i32::from_le_bytes(hdr[24..28].try_into().unwrap()),
        expects_reply: u32::from_le_bytes(hdr[28..32].try_into().unwrap()) != 0,
    })
}

async fn read_dtx_header<R: AsyncRead + Unpin>(reader: &mut R) -> Result<DtxHeader, DtxError> {
    let mut hdr = [0u8; 32];
    reader.read_exact(&mut hdr).await?;
    let header = parse_dtx_header(&hdr)?;
    if header.header_len > 32 {
        let mut extra = vec![0u8; header.header_len - 32];
        reader.read_exact(&mut extra).await?;
    }
    Ok(header)
}

fn decode_dtx_body_from_slice(h: &DtxHeader, body_slice: &[u8]) -> Result<DtxMessage, DtxError> {
    if body_slice.len() < 16 {
        return Err(DtxError::Protocol(format!(
            "DTX payload header truncated: got {} bytes, need at least 16",
            body_slice.len()
        )));
    }
    if body_slice.len() != h.msg_len {
        return Err(DtxError::Protocol(format!(
            "DTX body size mismatch: got {} bytes, header declares {}",
            body_slice.len(),
            h.msg_len
        )));
    }

    let ph = &body_slice[0..16];
    let msg_type = u32::from_le_bytes(ph[0..4].try_into().unwrap());
    let aux_len = u32::from_le_bytes(ph[4..8].try_into().unwrap()) as usize;
    let total_pay = u32::from_le_bytes(ph[8..12].try_into().unwrap()) as usize;

    if aux_len > total_pay {
        return Err(DtxError::Protocol(format!(
            "aux_len ({aux_len}) exceeds total_pay ({total_pay})"
        )));
    }
    if aux_len > 0 && aux_len < 16 {
        return Err(DtxError::Protocol(format!(
            "auxiliary section is too short for its header: {aux_len} bytes"
        )));
    }
    let pay_len = total_pay - aux_len;
    let rest = &body_slice[16..];

    // The payload header's total length describes all bytes after the payload
    // header, including the optional auxiliary header. Accepting a shorter
    // body and silently returning an empty payload hides a corrupt frame and
    // can desynchronise the next frame on a stream.
    if total_pay != rest.len() {
        return Err(DtxError::Protocol(format!(
            "DTX payload length mismatch: header declares {total_pay} bytes, body has {}",
            rest.len()
        )));
    }

    let aux_data = if aux_len > 0 {
        // `aux_len == rest.len()` was checked above, so this also verifies the
        // auxiliary header is wholly inside the declared section.
        let actual_aux = u32::from_le_bytes(rest[8..12].try_into().unwrap()) as usize;
        if actual_aux > aux_len.saturating_sub(16) {
            return Err(DtxError::Protocol(format!(
                "auxiliary data size ({actual_aux}) exceeds available space ({})",
                aux_len.saturating_sub(16)
            )));
        }
        let aux_start = 16;
        let aux_end = aux_start + actual_aux;
        if rest.len() < aux_end {
            return Err(DtxError::Protocol("aux data truncated".into()));
        }
        Some(Bytes::copy_from_slice(&rest[aux_start..aux_end]))
    } else {
        None
    };

    let pay_start = aux_len;
    let pay_end = pay_start
        .checked_add(pay_len)
        .ok_or_else(|| DtxError::Protocol("DTX payload length overflow".into()))?;
    // total_pay == rest.len() and aux_len <= total_pay establish these bounds,
    // but keep the check local to the slice operation so future changes cannot
    // turn malformed length fields into a panic.
    if pay_end > rest.len() {
        return Err(DtxError::Protocol(format!(
            "DTX payload extends beyond body: end={pay_end}, body={}",
            rest.len()
        )));
    }
    let payload_bytes = if pay_len > 0 {
        Bytes::copy_from_slice(&rest[pay_start..pay_end])
    } else {
        Bytes::new()
    };

    let payload = decode_payload(msg_type, payload_bytes, aux_data);
    Ok(DtxMessage {
        identifier: h.identifier,
        conversation_idx: h.conv_idx,
        channel_code: h.channel_code,
        expects_reply: h.expects_reply,
        payload,
    })
}

pub fn decode_dtx_message_from_bytes(data: &[u8]) -> Result<Option<(DtxMessage, usize)>, DtxError> {
    if data.len() < 32 {
        return Ok(None);
    }

    let header_bytes: &[u8; 32] = data[..32]
        .try_into()
        .map_err(|_| DtxError::Protocol("DTX header truncated".into()))?;
    let header = parse_dtx_header(header_bytes)?;
    let total_len = header
        .header_len
        .checked_add(header.msg_len)
        .ok_or_else(|| DtxError::Protocol("DTX frame length overflow".into()))?;
    if data.len() < total_len {
        return Ok(None);
    }

    let body = &data[header.header_len..total_len];
    let message = decode_dtx_body_from_slice(&header, body)?;
    Ok(Some((message, total_len)))
}

/// Read a single non-fragmented DTX message body (payload header + aux + payload).
/// `msg_len` is the number of bytes after the 32-byte header.
async fn read_dtx_body<R: AsyncRead + Unpin>(
    reader: &mut R,
    h: &DtxHeader,
    body: &[u8], // pre-read body bytes (for reassembled fragments)
) -> Result<DtxMessage, DtxError> {
    // body may be pre-supplied (reassembled) or empty (read from stream)
    let body_owned: Vec<u8>;
    let body_slice: &[u8] = if body.is_empty() && h.msg_len > 0 {
        body_owned = {
            let mut b = vec![0u8; h.msg_len];
            reader.read_exact(&mut b).await?;
            b
        };
        &body_owned
    } else {
        body
    };

    decode_dtx_body_from_slice(h, body_slice)
}

pub async fn read_dtx_frame<R: AsyncRead + Unpin>(reader: &mut R) -> Result<DtxMessage, DtxError> {
    let h = read_dtx_header(reader).await?;
    tracing::trace!(
        "read_dtx_frame: frag_idx={} frag_cnt={} msg_len={} id={}",
        h.frag_idx,
        h.frag_cnt,
        h.msg_len,
        h.identifier
    );

    // First fragment of multi-fragment message: no body, just a size announcement
    if h.frag_cnt > 1 && h.frag_idx == 0 {
        return Ok(DtxMessage {
            identifier: h.identifier,
            conversation_idx: h.conv_idx,
            channel_code: h.channel_code,
            expects_reply: h.expects_reply,
            payload: DtxPayload::Empty,
        });
    }

    if h.msg_len == 0 {
        return Ok(DtxMessage {
            identifier: h.identifier,
            conversation_idx: h.conv_idx,
            channel_code: h.channel_code,
            expects_reply: h.expects_reply,
            payload: DtxPayload::Empty,
        });
    }

    read_dtx_body(reader, &h, &[]).await
}

fn decode_payload(msg_type: u32, payload: Bytes, aux: Option<Bytes>) -> DtxPayload {
    tracing::trace!(
        "decode_payload: msg_type={msg_type} payload_len={} aux={}",
        payload.len(),
        aux.is_some()
    );
    match msg_type {
        MSG_OK => DtxPayload::Empty,
        MSG_METHOD_INVOCATION => {
            let mut args = aux
                .map(super::primitive::decode_auxiliary)
                .unwrap_or_default();
            let selector = if payload.is_empty() {
                String::new()
            } else {
                match crate::proto::nskeyedarchiver::unarchive(&payload)
                    .ok()
                    .and_then(|v| v.as_str().map(String::from))
                {
                    Some(selector) => selector,
                    None => {
                        tracing::debug!(
                            "decode_payload: method invocation payload decode failed, preserving {} raw bytes",
                            payload.len()
                        );
                        args.insert(0, NSObject::Data(payload));
                        String::new()
                    }
                }
            };
            DtxPayload::MethodInvocation { selector, args }
        }
        MSG_RESPONSE | MSG_ERROR => {
            if payload.is_empty() {
                DtxPayload::Response(NSObject::Null)
            } else {
                let obj = crate::proto::nskeyedarchiver::unarchive(&payload)
                    .map(archive_to_ns)
                    .unwrap_or(NSObject::Data(payload));
                DtxPayload::Response(obj)
            }
        }
        MSG_BARRIER => DtxPayload::Empty,
        MSG_UNKNOWN_TYPE_ONE => match aux {
            Some(aux) => DtxPayload::RawWithAux {
                payload,
                aux: super::primitive::decode_auxiliary(aux),
            },
            None => DtxPayload::Raw(payload),
        },
        _ => {
            if payload.is_empty() {
                DtxPayload::Empty
            } else {
                DtxPayload::Raw(payload)
            }
        }
    }
}

fn archive_to_ns(v: crate::proto::nskeyedarchiver::ArchiveValue) -> NSObject {
    use crate::proto::nskeyedarchiver::ArchiveValue;
    match v {
        ArchiveValue::Null => NSObject::Null,
        ArchiveValue::Bool(b) => NSObject::Bool(b),
        ArchiveValue::Int(n) => NSObject::Int(n),
        ArchiveValue::Uint(n) => NSObject::Uint(n),
        ArchiveValue::Float(f) => NSObject::Double(f),
        ArchiveValue::String(s) => NSObject::String(s),
        ArchiveValue::Data(d) => NSObject::Data(d),
        ArchiveValue::Array(a) => NSObject::Array(a.into_iter().map(archive_to_ns).collect()),
        ArchiveValue::Dict(d) => {
            NSObject::Dict(d.into_iter().map(|(k, v)| (k, archive_to_ns(v))).collect())
        }
        ArchiveValue::Unknown(s) => NSObject::String(format!("<{s}>")),
    }
}

// ── Fragment reassembly state ─────────────────────────────────────────────────

/// In-progress multi-fragment message accumulator.
struct FragmentAccum {
    /// Header fields from the first fragment (index=0).
    header: DtxHeader,
    /// Body fragments keyed by fragment index - 1.
    fragments: Vec<Option<Vec<u8>>>,
    /// Number of body fragments still expected.
    remaining: u16,
    /// Bytes buffered so far, checked before each fragment is allocated.
    received: usize,
    /// Dynamic bytes reserved in the connection budget for this accumulator.
    reserved_bytes: usize,
}

struct BufferedMessage {
    message: DtxMessage,
    reserved_bytes: usize,
}

// ── DtxConnection ─────────────────────────────────────────────────────────────

/// A managed DTX connection with channel multiplexing and method call support.
pub struct DtxConnection<S> {
    stream: S,
    /// Connection-wide message identifier counter.
    ///
    /// We intentionally keep this global (instead of per-channel) to mirror
    /// pymobiledevice3's reply-correlation model, where responses are matched
    /// centrally by identifier and non-target traffic is buffered separately.
    identifier: u32,
    channel_counter: i32,
    /// Replies buffered while another request is waiting on its own response.
    pending_replies: HashMap<u32, BufferedMessage>,
    /// Identifiers for synchronous requests that are currently awaiting a reply.
    outstanding_reply_ids: HashSet<u32>,
    /// Non-reply messages buffered while a request is synchronously awaiting its reply.
    queued_messages: VecDeque<BufferedMessage>,
    /// In-progress multi-fragment messages keyed by DTX identifier.
    fragments: HashMap<u32, FragmentAccum>,
    /// Unified receive-side retained-memory budget.
    memory_budget: DtxMemoryBudget,
}

impl<S: AsyncRead + AsyncWrite + Unpin + Send> DtxConnection<S> {
    pub fn new(stream: S) -> Self {
        Self::with_memory_limit(stream, DEFAULT_DTX_MEMORY_BUDGET)
    }

    /// Construct a connection with an explicit receive-side memory limit.
    ///
    /// This is primarily useful to embedders that already enforce a tighter
    /// process budget and to deterministic tests; `new` keeps the normal
    /// Instruments-friendly 512 MiB default.
    pub fn with_memory_limit(stream: S, memory_limit: usize) -> Self {
        // Start identifier at 5 to match go-ios global channel messageIdentifier initial value
        Self {
            stream,
            identifier: 5,
            channel_counter: 1,
            pending_replies: HashMap::new(),
            outstanding_reply_ids: HashSet::new(),
            queued_messages: VecDeque::new(),
            fragments: HashMap::new(),
            memory_budget: DtxMemoryBudget::new(memory_limit),
        }
    }

    /// Number of dynamically-sized receive-side bytes currently retained.
    pub fn memory_bytes_used(&self) -> usize {
        self.memory_budget.used
    }

    /// Configured receive-side memory limit.
    pub fn memory_limit(&self) -> usize {
        self.memory_budget.limit
    }

    fn next_id(&mut self) -> Result<u32, DtxError> {
        let id = self.identifier;
        self.identifier = self
            .identifier
            .checked_add(1)
            .ok_or_else(|| DtxError::Protocol("DTX message identifier exhausted".into()))?;
        Ok(id)
    }

    fn next_channel_code(&mut self) -> Result<i32, DtxError> {
        let code = self.channel_counter;
        self.channel_counter = self
            .channel_counter
            .checked_add(1)
            .ok_or_else(|| DtxError::Protocol("DTX channel code exhausted".into()))?;
        Ok(code)
    }

    pub async fn send_raw(&mut self, data: &[u8]) -> Result<(), DtxError> {
        self.stream.write_all(data).await?;
        self.stream.flush().await?;
        Ok(())
    }

    pub async fn send_ack(&mut self, msg: &DtxMessage) -> Result<(), DtxError> {
        self.send_raw(&encode_ack(msg)).await
    }

    fn buffer_reply(&mut self, msg: DtxMessage) -> Result<(), DtxError> {
        // Take ownership instead of cloning the reply. The message's dynamic
        // bytes are measured before it enters the map, and replacement adjusts
        // the old/new total atomically.
        let identifier = msg.identifier;
        if !self.pending_replies.contains_key(&identifier)
            && self.pending_replies.len() >= MAX_PENDING_REPLIES
        {
            return Err(DtxError::Protocol(format!(
                "DTX pending reply backlog full: {} entries exceeds {}",
                self.pending_replies.len(),
                MAX_PENDING_REPLIES
            )));
        }
        let new_conversation_idx = msg.conversation_idx;
        let reserved_bytes = dtx_message_memory_size(&msg)?;
        let old_bytes = self
            .pending_replies
            .get(&identifier)
            .map(|previous| previous.reserved_bytes)
            .unwrap_or(0);
        self.adjust_replacement_budget(old_bytes, reserved_bytes, "pending reply")?;

        if let Some(previous) = self.pending_replies.insert(
            identifier,
            BufferedMessage {
                message: msg,
                reserved_bytes,
            },
        ) {
            tracing::trace!(
                "buffer_reply: replacing pending reply id={} old_conv={} new_conv={}",
                previous.message.identifier,
                previous.message.conversation_idx,
                new_conversation_idx
            );
        }
        Ok(())
    }

    fn queue_message(&mut self, msg: DtxMessage) -> Result<(), DtxError> {
        // Queue ownership is moved after the byte check; this avoids the old
        // untracked clone path and lets pop/drop release exactly this amount.
        let reserved_bytes = dtx_message_memory_size(&msg)?;
        let dropped_bytes = if self.queued_messages.len() >= MAX_QUEUED_MESSAGES {
            self.queued_messages
                .front()
                .map(|oldest| oldest.reserved_bytes)
                .unwrap_or(0)
        } else {
            0
        };
        self.ensure_budget_after_release(dropped_bytes, reserved_bytes, "queued message")?;

        if self.queued_messages.len() >= MAX_QUEUED_MESSAGES {
            if let Some(dropped) = self.queued_messages.pop_front() {
                self.memory_budget.release(dropped.reserved_bytes);
                tracing::warn!(
                    identifier = dropped.message.identifier,
                    channel_code = dropped.message.channel_code,
                    "DTX message backlog full; dropping oldest unrelated message"
                );
            }
        }
        self.memory_budget
            .try_reserve(reserved_bytes, "queued message")?;
        self.queued_messages.push_back(BufferedMessage {
            message: msg,
            reserved_bytes,
        });
        Ok(())
    }

    fn adjust_replacement_budget(
        &mut self,
        old_bytes: usize,
        new_bytes: usize,
        what: &str,
    ) -> Result<(), DtxError> {
        let current = self.memory_budget.used.checked_sub(old_bytes).ok_or_else(|| {
            DtxError::Protocol(format!(
                "DTX memory accounting underflow replacing {what}: used={} old={old_bytes} new={new_bytes}",
                self.memory_budget.used
            ))
        })?;
        let projected = current.checked_add(new_bytes).ok_or_else(|| {
            DtxError::Protocol(format!(
                "DTX memory accounting overflow replacing {what}: used={current} old={old_bytes} new={new_bytes} limit={}",
                self.memory_budget.limit
            ))
        })?;
        if projected > self.memory_budget.limit {
            return Err(DtxError::Protocol(format!(
                "DTX memory budget exceeded replacing {what}: used={current} old={old_bytes} new={new_bytes} limit={}",
                self.memory_budget.limit
            )));
        }
        self.memory_budget.used = projected;
        Ok(())
    }

    fn ensure_budget_after_release(
        &self,
        released_bytes: usize,
        added_bytes: usize,
        what: &str,
    ) -> Result<(), DtxError> {
        let current = self.memory_budget.used.checked_sub(released_bytes).ok_or_else(|| {
            DtxError::Protocol(format!(
                "DTX memory accounting underflow before reserving {added_bytes} bytes for {what}: used={} release={released_bytes}",
                self.memory_budget.used
            ))
        })?;
        let projected = current.checked_add(added_bytes).ok_or_else(|| {
            DtxError::Protocol(format!(
                "DTX memory accounting overflow reserving {added_bytes} bytes for {what}: used={current} limit={}",
                self.memory_budget.limit
            ))
        })?;
        if projected > self.memory_budget.limit {
            return Err(DtxError::Protocol(format!(
                "DTX memory budget exceeded reserving {added_bytes} bytes for {what}: used={current} release={released_bytes} limit={}",
                self.memory_budget.limit
            )));
        }
        Ok(())
    }

    fn remove_pending_reply(&mut self, id: u32) -> Option<DtxMessage> {
        self.pending_replies.remove(&id).map(|buffered| {
            self.memory_budget.release(buffered.reserved_bytes);
            buffered.message
        })
    }

    fn pop_queued_message(&mut self) -> Option<DtxMessage> {
        self.queued_messages.pop_front().map(|buffered| {
            self.memory_budget.release(buffered.reserved_bytes);
            buffered.message
        })
    }

    fn clear_fragments(&mut self) {
        for (_, accumulator) in self.fragments.drain() {
            self.memory_budget.release(accumulator.reserved_bytes);
        }
    }

    fn is_reply_message(&self, msg: &DtxMessage) -> bool {
        msg.conversation_idx > 0 && self.outstanding_reply_ids.contains(&msg.identifier)
    }

    async fn recv_from_stream(&mut self) -> Result<DtxMessage, DtxError> {
        let result = self.recv_from_stream_inner().await;
        if result.is_err() {
            // A protocol or I/O failure makes any partially assembled frame
            // unusable. Drop all of it before returning so a caller that
            // reports the error cannot keep its budget pinned indefinitely.
            self.clear_fragments();
        }
        result
    }

    async fn recv_from_stream_inner(&mut self) -> Result<DtxMessage, DtxError> {
        loop {
            let h = read_dtx_header(&mut self.stream).await?;
            tracing::trace!(
                "recv: frag_idx={} frag_cnt={} msg_len={} id={}",
                h.frag_idx,
                h.frag_cnt,
                h.msg_len,
                h.identifier
            );

            if h.frag_cnt <= 1 {
                // Single-fragment message
                if h.msg_len == 0 {
                    return Ok(DtxMessage {
                        identifier: h.identifier,
                        conversation_idx: h.conv_idx,
                        channel_code: normalize_incoming_channel_code(h.channel_code, h.conv_idx)?,
                        expects_reply: h.expects_reply,
                        payload: DtxPayload::Empty,
                    });
                }
                // Reserve before read_dtx_body allocates its body Vec. The
                // reservation is transient because a message returned to the
                // caller is no longer retained by this connection.
                self.memory_budget
                    .try_reserve(h.msg_len, "single-fragment body")?;
                let result = read_dtx_body(&mut self.stream, &h, &[]).await;
                self.memory_budget.release(h.msg_len);
                let mut msg = result?;
                msg.channel_code =
                    normalize_incoming_channel_code(msg.channel_code, msg.conversation_idx)?;
                return Ok(msg);
            }

            if h.frag_idx == 0 {
                // First fragment: no body, just announces total size
                if self.fragments.contains_key(&h.identifier) {
                    return Err(DtxError::Protocol(format!(
                        "duplicate first fragment for id={}",
                        h.identifier
                    )));
                }
                if self.fragments.len() >= MAX_IN_FLIGHT_FRAGMENTED_MESSAGES {
                    return Err(DtxError::Protocol(format!(
                        "too many in-flight fragmented DTX messages: {} exceeds {}",
                        self.fragments.len(),
                        MAX_IN_FLIGHT_FRAGMENTED_MESSAGES
                    )));
                }
                self.fragments.insert(
                    h.identifier,
                    FragmentAccum {
                        fragments: vec![None; (h.frag_cnt - 1) as usize],
                        remaining: h.frag_cnt - 1,
                        received: 0,
                        reserved_bytes: 0,
                        header: h,
                    },
                );
                continue;
            }

            // Subsequent fragment: read msg_len bytes of body.
            //
            // Check the running total against the announced size *before*
            // allocating: each fragment may declare up to MAX_DTX_MESSAGE_SIZE
            // on its own, and there can be MAX_DTX_FRAGMENTS of them, so
            // validating only the assembled total lets a crafted stream buffer
            // orders of magnitude more than one message is allowed to be.
            let id = h.identifier;
            let slot_idx = h
                .frag_idx
                .checked_sub(1)
                .map(|idx| idx as usize)
                .ok_or_else(|| {
                    DtxError::Protocol(format!("invalid fragment index {} for id={id}", h.frag_idx))
                })?;
            {
                let accum = self.fragments.get(&id).ok_or_else(|| {
                    DtxError::Protocol(format!(
                        "fragment id={id} frag_idx={} without first fragment",
                        h.frag_idx
                    ))
                })?;
                if h.frag_cnt != accum.header.frag_cnt {
                    return Err(DtxError::Protocol(format!(
                        "fragment count mismatch for id={id}: got={} expected={}",
                        h.frag_cnt, accum.header.frag_cnt
                    )));
                }
                if h.conv_idx != accum.header.conv_idx
                    || h.channel_code != accum.header.channel_code
                    || h.expects_reply != accum.header.expects_reply
                {
                    return Err(DtxError::Protocol(format!(
                        "fragment metadata mismatch for id={id}"
                    )));
                }
                if accum.fragments.get(slot_idx).is_none() {
                    return Err(DtxError::Protocol(format!(
                        "fragment index {} out of range for id={id}",
                        h.frag_idx
                    )));
                }
                if accum.fragments[slot_idx].is_some() {
                    return Err(DtxError::Protocol(format!(
                        "duplicate fragment {} for id={id}",
                        h.frag_idx
                    )));
                }
                let announced = accum.header.msg_len;
                if h.msg_len > announced.saturating_sub(accum.received) {
                    return Err(DtxError::Protocol(format!(
                        "fragmented body for id={id} exceeds announced size {announced}"
                    )));
                }
            }

            // Reserve before allocating the fragment buffer. This is the
            // precise amount retained by this accumulator; the first fragment
            // only announces the eventual size and retains no body bytes.
            self.memory_budget.try_reserve(h.msg_len, "fragment body")?;
            let mut frag_body = vec![0u8; h.msg_len];
            if let Err(error) = self.stream.read_exact(&mut frag_body).await {
                self.memory_budget.release(h.msg_len);
                return Err(error.into());
            }

            if let Some(accum) = self.fragments.get_mut(&id) {
                let received = match accum.received.checked_add(frag_body.len()) {
                    Some(received) => received,
                    None => {
                        self.memory_budget.release(h.msg_len);
                        return Err(DtxError::Protocol(format!(
                            "fragmented body byte count overflow for id={id}"
                        )));
                    }
                };
                let reserved_bytes = match accum.reserved_bytes.checked_add(h.msg_len) {
                    Some(reserved_bytes) => reserved_bytes,
                    None => {
                        self.memory_budget.release(h.msg_len);
                        return Err(DtxError::Protocol(format!(
                            "fragment memory accounting overflow for id={id}"
                        )));
                    }
                };
                accum.received = received;
                accum.reserved_bytes = reserved_bytes;
                accum.fragments[slot_idx] = Some(frag_body);
                accum.remaining -= 1;
                if accum.remaining == 0 {
                    let accum = match self.fragments.remove(&id) {
                        Some(accum) => accum,
                        None => {
                            // The just-read fragment was accounted before its
                            // allocation; no accumulator remains to release
                            // it through clear_fragments.
                            self.memory_budget.release(h.msg_len);
                            return Err(DtxError::Protocol(format!(
                                "missing fragment accumulator for id={id}"
                            )));
                        }
                    };
                    if accum.received != accum.header.msg_len {
                        self.memory_budget.release(accum.reserved_bytes);
                        return Err(DtxError::Protocol(format!(
                            "fragmented body size mismatch for id={id}: assembled={} expected={}",
                            accum.received, accum.header.msg_len
                        )));
                    }

                    // Reassembly necessarily coexists briefly with the
                    // out-of-order fragment buffers. Account for that second
                    // copy before allocating its Vec.
                    if let Err(error) = self
                        .memory_budget
                        .try_reserve(accum.header.msg_len, "fragment reassembly body")
                    {
                        self.memory_budget.release(accum.reserved_bytes);
                        return Err(error);
                    }
                    let mut body = Vec::with_capacity(accum.header.msg_len);
                    for (index, fragment) in accum.fragments.into_iter().enumerate() {
                        let fragment = match fragment {
                            Some(fragment) => fragment,
                            None => {
                                self.memory_budget.release(accum.reserved_bytes);
                                self.memory_budget.release(accum.header.msg_len);
                                return Err(DtxError::Protocol(format!(
                                    "missing fragment {} for id={id}",
                                    index + 1
                                )));
                            }
                        };
                        body.extend_from_slice(&fragment);
                    }
                    if body.len() != accum.header.msg_len {
                        self.memory_budget.release(accum.reserved_bytes);
                        self.memory_budget.release(accum.header.msg_len);
                        return Err(DtxError::Protocol(format!(
                            "fragmented body size mismatch for id={id}: assembled={} expected={}",
                            body.len(),
                            accum.header.msg_len
                        )));
                    }
                    // All fragment Vecs have been consumed into `body`; their
                    // reservations can be released before decoding the body.
                    self.memory_budget.release(accum.reserved_bytes);
                    let result = read_dtx_body(&mut self.stream, &accum.header, &body).await;
                    drop(body);
                    self.memory_budget.release(accum.header.msg_len);
                    let mut msg = result?;
                    msg.channel_code =
                        normalize_incoming_channel_code(msg.channel_code, msg.conversation_idx)?;
                    return Ok(msg);
                }
            } else {
                // The fragment buffer was allocated and accounted before this
                // lookup. It should be impossible for the map entry to vanish
                // while borrowed, but keep the error path leak-free.
                self.memory_budget.release(h.msg_len);
                return Err(DtxError::Protocol(format!(
                    "fragment id={id} frag_idx={} without first fragment",
                    h.frag_idx
                )));
            }
        }
    }

    async fn wait_for_reply(&mut self, id: u32) -> Result<DtxMessage, DtxError> {
        if let Some(msg) = self.remove_pending_reply(id) {
            self.outstanding_reply_ids.remove(&id);
            return Ok(msg);
        }

        loop {
            let msg = self.recv_from_stream().await?;
            tracing::trace!(
                "wait_for_reply: target_id={} recv id={} conv_idx={} ch={} expects_reply={}",
                id,
                msg.identifier,
                msg.conversation_idx,
                msg.channel_code,
                msg.expects_reply
            );

            if self.is_reply_message(&msg) {
                if msg.identifier == id {
                    self.outstanding_reply_ids.remove(&id);
                    return Ok(msg);
                }
                self.buffer_reply(msg)?;
                continue;
            }

            if msg.expects_reply {
                self.send_ack(&msg).await?;
            }
            self.queue_message(msg)?;
        }
    }

    /// Receive the next fully-assembled DTX message, transparently reassembling fragments.
    pub async fn recv(&mut self) -> Result<DtxMessage, DtxError> {
        if let Some(msg) = self.pop_queued_message() {
            return Ok(msg);
        }

        loop {
            let msg = self.recv_from_stream().await?;
            if self.is_reply_message(&msg) {
                self.buffer_reply(msg)?;
                continue;
            }
            return Ok(msg);
        }
    }

    /// Request a DTX channel by service name.
    /// Returns the assigned channel code.
    pub async fn request_channel(&mut self, service_name: &str) -> Result<i32, DtxError> {
        let channel_code = self.next_channel_code()?;
        let id = self.next_id()?;

        let selector =
            nskeyedarchiver_encode::archive_string("_requestChannelWithCode:identifier:");
        let arg_name = nskeyedarchiver_encode::archive_string(service_name);

        // channel_code is passed as raw Int32 (not NSKeyedArchiver), matching go-ios AddInt32()
        let aux = encode_primitive_dict(&[
            PrimArg::Int32(channel_code),
            PrimArg::Bytes(Bytes::from(arg_name)),
        ]);

        let frame = encode_dtx(id, 0, 0, true, MSG_METHOD_INVOCATION, &selector, &aux);
        self.send_raw(&frame).await?;
        self.outstanding_reply_ids.insert(id);

        // Read reply (skip unrelated notifications)
        let result = self.wait_for_reply(id).await;
        if result.is_err() {
            self.outstanding_reply_ids.remove(&id);
        }
        let msg = result?;
        tracing::debug!(
            "request_channel recv: id={} conv_idx={} ch={} expects_reply={}",
            msg.identifier,
            msg.conversation_idx,
            msg.channel_code,
            msg.expects_reply
        );
        Ok(channel_code)
    }

    /// Call a method on a channel and wait for the response.
    pub async fn method_call(
        &mut self,
        channel_code: i32,
        selector: &str,
        args: &[PrimArg],
    ) -> Result<DtxMessage, DtxError> {
        let id = self.next_id()?;
        let sel_bytes = nskeyedarchiver_encode::archive_string(selector);
        let aux = if args.is_empty() {
            Bytes::new()
        } else {
            encode_primitive_dict(args)
        };
        let frame = encode_dtx(
            id,
            0,
            channel_code,
            true,
            MSG_METHOD_INVOCATION,
            &sel_bytes,
            &aux,
        );
        self.send_raw(&frame).await?;
        self.outstanding_reply_ids.insert(id);
        tracing::debug!("method_call '{selector}' id={id} ch={channel_code}");

        let result = self.wait_for_reply(id).await;
        if result.is_err() {
            self.outstanding_reply_ids.remove(&id);
        }
        let msg = result?;
        tracing::debug!(
            "method_call recv: id={} conv_idx={} ch={}",
            msg.identifier,
            msg.conversation_idx,
            msg.channel_code
        );
        Ok(msg)
    }

    /// Fire-and-forget method call.
    pub async fn method_call_async(
        &mut self,
        channel_code: i32,
        selector: &str,
        args: &[PrimArg],
    ) -> Result<(), DtxError> {
        let id = self.next_id()?;
        let sel_bytes = nskeyedarchiver_encode::archive_string(selector);
        let aux = if args.is_empty() {
            Bytes::new()
        } else {
            encode_primitive_dict(args)
        };
        let frame = encode_dtx(
            id,
            0,
            channel_code,
            false,
            MSG_METHOD_INVOCATION,
            &sel_bytes,
            &aux,
        );
        self.send_raw(&frame).await
    }
}

impl<S> Drop for DtxConnection<S> {
    fn drop(&mut self) {
        // The containers would release their allocations automatically. Clear
        // them explicitly as the final accounting checkpoint so the budget's
        // invariant also holds while a connection is being torn down.
        self.pending_replies.clear();
        self.queued_messages.clear();
        self.outstanding_reply_ids.clear();
        self.fragments.clear();
        self.memory_budget.used = 0;
    }
}

fn normalize_incoming_channel_code(
    channel_code: i32,
    conversation_idx: u32,
) -> Result<i32, DtxError> {
    if conversation_idx % 2 == 0 {
        channel_code.checked_neg().ok_or_else(|| {
            DtxError::Protocol(format!(
                "cannot normalize incoming channel code {channel_code} for conversation {conversation_idx}"
            ))
        })
    } else {
        Ok(channel_code)
    }
}

#[cfg(test)]
mod tests {
    use bytes::BufMut;

    use super::*;

    #[test]
    fn test_encode_dtx_layout() {
        let sel = nskeyedarchiver_encode::archive_string("test");
        let frame = encode_dtx(1, 0, 1, true, MSG_METHOD_INVOCATION, &sel, &[]);
        assert_eq!(
            u32::from_be_bytes(frame[0..4].try_into().unwrap()),
            DTX_MAGIC
        );
        assert_eq!(u32::from_le_bytes(frame[4..8].try_into().unwrap()), 32);
        assert_eq!(u32::from_le_bytes(frame[28..32].try_into().unwrap()), 1); // expects_reply
        assert_eq!(
            u32::from_le_bytes(frame[32..36].try_into().unwrap()),
            MSG_METHOD_INVOCATION
        );
    }

    #[test]
    fn test_encode_ack_length() {
        let msg = DtxMessage {
            identifier: 5,
            conversation_idx: 0,
            channel_code: 1,
            expects_reply: true,
            payload: DtxPayload::Empty,
        };
        let ack = encode_ack(&msg);
        assert_eq!(ack.len(), 48);
        assert_eq!(u32::from_le_bytes(ack[32..36].try_into().unwrap()), MSG_OK);
    }

    #[tokio::test]
    async fn test_dtx_encode_decode_roundtrip() {
        let sel = nskeyedarchiver_encode::archive_string("setConfig:");
        let frame = encode_dtx(7, 0, 2, true, MSG_METHOD_INVOCATION, &sel, &[]);
        let mut cur = std::io::Cursor::new(frame);
        let msg = read_dtx_frame(&mut cur).await.unwrap();
        assert_eq!(msg.identifier, 7);
        assert_eq!(msg.channel_code, 2);
        assert!(msg.expects_reply);
        // Selector should be recoverable
        if let DtxPayload::MethodInvocation { selector, .. } = &msg.payload {
            assert_eq!(selector, "setConfig:");
        } else {
            panic!("expected MethodInvocation");
        }
    }

    #[tokio::test]
    async fn test_data_frame_keeps_raw_payload() {
        let payload = b"trace-binary-payload";
        let frame = encode_dtx(11, 0, 4, false, MSG_UNKNOWN_TYPE_ONE, payload, &[]);
        let mut cur = std::io::Cursor::new(frame);
        let msg = read_dtx_frame(&mut cur).await.unwrap();
        match msg.payload {
            DtxPayload::Raw(bytes) => assert_eq!(bytes.as_ref(), payload),
            other => panic!("expected raw payload, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn test_data_frame_preserves_auxiliary_arguments() {
        let payload = b"trace-binary-payload";
        let aux =
            encode_primitive_dict(&[PrimArg::Bytes(Bytes::from_static(b"kperf-aux-payload"))]);
        let frame = encode_dtx(13, 0, 4, false, MSG_UNKNOWN_TYPE_ONE, payload, &aux);
        let mut cur = std::io::Cursor::new(frame);
        let msg = read_dtx_frame(&mut cur).await.unwrap();

        match msg.payload {
            DtxPayload::RawWithAux { payload: body, aux } => {
                assert_eq!(body.as_ref(), payload);
                assert!(matches!(
                    aux.first(),
                    Some(NSObject::Data(bytes)) if bytes.as_ref() == b"kperf-aux-payload"
                ));
            }
            other => panic!("expected raw payload with aux, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn test_method_invocation_preserves_raw_payload_when_selector_decode_fails() {
        let payload = b"not-a-selector";
        let frame = encode_dtx(12, 0, 4, false, MSG_METHOD_INVOCATION, payload, &[]);
        let mut cur = std::io::Cursor::new(frame);
        let msg = read_dtx_frame(&mut cur).await.unwrap();

        match msg.payload {
            DtxPayload::MethodInvocation { selector, args } => {
                assert!(selector.is_empty());
                assert!(
                    matches!(args.first(), Some(NSObject::Data(bytes)) if bytes.as_ref() == payload)
                );
            }
            other => panic!("expected method invocation, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn test_method_call_buffers_unrelated_notifications() {
        let (client, mut server) = tokio::io::duplex(4096);
        let mut conn = DtxConnection::new(client);

        let call = tokio::spawn(async move {
            conn.method_call(2, "startSampling", &[])
                .await
                .map(|reply| (conn, reply))
        });

        let outbound = read_dtx_frame(&mut server).await.unwrap();
        assert_eq!(outbound.identifier, 5);
        assert!(outbound.expects_reply);

        let notify_selector = nskeyedarchiver_encode::archive_string("note:");
        let notify = encode_dtx(77, 0, 1, true, MSG_METHOD_INVOCATION, &notify_selector, &[]);
        server.write_all(&notify).await.unwrap();

        let ack = read_dtx_frame(&mut server).await.unwrap();
        assert_eq!(ack.identifier, 77);
        assert_eq!(ack.conversation_idx, 1);

        let reply = encode_dtx(5, 1, 2, false, MSG_RESPONSE, &[], &[]);
        server.write_all(&reply).await.unwrap();

        let (mut conn, reply) = call.await.unwrap().unwrap();
        assert_eq!(reply.identifier, 5);
        assert_eq!(reply.conversation_idx, 1);

        let queued = conn.recv().await.unwrap();
        assert_eq!(queued.identifier, 77);
        assert_eq!(queued.channel_code, -1);
        assert!(queued.expects_reply);
    }

    #[tokio::test]
    async fn test_recv_normalizes_even_conversation_channel_codes() {
        let (client, mut server) = tokio::io::duplex(256);
        let mut conn = DtxConnection::new(client);

        let recv_task = tokio::spawn(async move { conn.recv().await });

        let payload = b"trace-binary-payload";
        let frame = encode_dtx(42, 0, -2, false, MSG_UNKNOWN_TYPE_ONE, payload, &[]);
        server.write_all(&frame).await.unwrap();

        let msg = recv_task.await.unwrap().unwrap();
        assert_eq!(msg.identifier, 42);
        assert_eq!(msg.conversation_idx, 0);
        assert_eq!(msg.channel_code, 2);
        match msg.payload {
            DtxPayload::Raw(bytes) => assert_eq!(bytes.as_ref(), payload),
            other => panic!("expected raw payload, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn test_recv_rejects_min_channel_code_when_normalizing() {
        let (client, mut server) = tokio::io::duplex(256);
        let mut conn = DtxConnection::new(client);
        let frame = encode_dtx(
            43,
            0,
            i32::MIN,
            false,
            MSG_UNKNOWN_TYPE_ONE,
            b"malformed-channel",
            &[],
        );
        server.write_all(&frame).await.unwrap();

        let err = conn.recv().await.unwrap_err();
        assert!(matches!(
            err,
            DtxError::Protocol(message) if message.contains("cannot normalize incoming channel code")
        ));
    }

    #[tokio::test]
    async fn test_wait_for_reply_returns_buffered_reply_immediately() {
        let (client, _server) = tokio::io::duplex(64);
        let mut conn = DtxConnection::new(client);

        conn.buffer_reply(DtxMessage {
            identifier: 9,
            conversation_idx: 1,
            channel_code: 3,
            expects_reply: false,
            payload: DtxPayload::Empty,
        })
        .unwrap();

        let reply = conn.wait_for_reply(9).await.unwrap();
        assert_eq!(reply.identifier, 9);
        assert_eq!(reply.conversation_idx, 1);
        assert_eq!(reply.channel_code, 3);
    }

    #[tokio::test]
    async fn test_recv_treats_unsolicited_conversation_message_as_live_event() {
        let (client, mut server) = tokio::io::duplex(256);
        let mut conn = DtxConnection::new(client);

        let recv_task = tokio::spawn(async move { conn.recv().await });

        let payload = b"trace-binary-payload";
        let frame = encode_dtx(42, 1, 2, false, MSG_UNKNOWN_TYPE_ONE, payload, &[]);
        server.write_all(&frame).await.unwrap();

        let msg = recv_task.await.unwrap().unwrap();
        assert_eq!(msg.identifier, 42);
        assert_eq!(msg.conversation_idx, 1);
        assert_eq!(msg.channel_code, 2);
        match msg.payload {
            DtxPayload::Raw(bytes) => assert_eq!(bytes.as_ref(), payload),
            other => panic!("expected raw payload, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn test_read_dtx_frame_skips_extended_header_bytes() {
        let payload = b"trace-binary-payload";
        let mut frame = encode_dtx(21, 0, 4, false, MSG_UNKNOWN_TYPE_ONE, payload, &[]).to_vec();

        frame[4..8].copy_from_slice(&36u32.to_le_bytes());
        frame.splice(32..32, [0xAA, 0xBB, 0xCC, 0xDD]);

        let mut cur = std::io::Cursor::new(frame);
        let msg = read_dtx_frame(&mut cur)
            .await
            .expect("extended headers should be skipped before parsing payload");

        match msg.payload {
            DtxPayload::Raw(bytes) => assert_eq!(bytes.as_ref(), payload),
            other => panic!("expected raw payload, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn test_ok_reply_decodes_as_empty_payload() {
        let frame = encode_dtx(23, 1, 4, false, MSG_OK, &[], &[]);
        let mut cur = std::io::Cursor::new(frame);
        let msg = read_dtx_frame(&mut cur).await.unwrap();
        assert!(matches!(msg.payload, DtxPayload::Empty));
    }

    #[tokio::test]
    async fn test_rejects_truncated_payload_header() {
        let frame = encode_fragment(22, 0, 1, 4, false, 8, &[0u8; 8]);
        let mut cur = std::io::Cursor::new(frame);
        let err = read_dtx_frame(&mut cur).await.unwrap_err();
        assert!(matches!(
            err,
            DtxError::Protocol(message) if message.contains("payload header truncated")
        ));
    }

    #[tokio::test]
    async fn test_rejects_payload_length_mismatch_instead_of_silently_dropping_bytes() {
        let mut body = BytesMut::with_capacity(20);
        body.put_u32_le(MSG_UNKNOWN_TYPE_ONE);
        body.put_u32_le(0);
        body.put_u32_le(0); // total payload length claims no bytes
        body.put_u32_le(0);
        body.extend_from_slice(b"extra");

        let frame = encode_fragment(24, 0, 1, 4, false, body.len(), &body);
        let mut cur = std::io::Cursor::new(frame);
        let err = read_dtx_frame(&mut cur).await.unwrap_err();
        assert!(matches!(
            err,
            DtxError::Protocol(message) if message.contains("payload length mismatch")
        ));
    }

    #[tokio::test]
    async fn test_rejects_auxiliary_length_smaller_than_auxiliary_header() {
        let mut body = BytesMut::with_capacity(24);
        body.put_u32_le(MSG_METHOD_INVOCATION);
        body.put_u32_le(8); // impossible: an auxiliary section includes a 16-byte header
        body.put_u32_le(8);
        body.put_u32_le(0);
        body.extend_from_slice(&[0u8; 8]);

        let frame = encode_fragment(25, 0, 1, 4, false, body.len(), &body);
        let mut cur = std::io::Cursor::new(frame);
        let err = read_dtx_frame(&mut cur).await.unwrap_err();
        assert!(matches!(
            err,
            DtxError::Protocol(message) if message.contains("too short for its header")
        ));
    }

    #[tokio::test]
    async fn test_error_reply_decodes_like_response_object() {
        let payload = nskeyedarchiver_encode::archive_string("selector failed");
        let frame = encode_dtx(24, 1, 4, false, MSG_ERROR, &payload, &[]);
        let mut cur = std::io::Cursor::new(frame);
        let msg = read_dtx_frame(&mut cur).await.unwrap();
        assert!(matches!(
            msg.payload,
            DtxPayload::Response(NSObject::String(ref value)) if value == "selector failed"
        ));
    }

    fn encode_fragment(
        identifier: u32,
        frag_idx: u16,
        frag_cnt: u16,
        channel_code: i32,
        expects_reply: bool,
        msg_len: usize,
        body: &[u8],
    ) -> Bytes {
        let mut out = BytesMut::with_capacity(32 + body.len());
        out.put_u32(DTX_MAGIC);
        out.put_u32_le(32);
        out.put_u16_le(frag_idx);
        out.put_u16_le(frag_cnt);
        out.put_u32_le(msg_len as u32);
        out.put_u32_le(identifier);
        out.put_u32_le(0);
        out.put_u32_le(channel_code as u32);
        out.put_u32_le(if expects_reply { 1 } else { 0 });
        out.extend_from_slice(body);
        out.freeze()
    }

    #[tokio::test]
    async fn test_recv_reassembles_out_of_order_fragments_by_index() {
        let payload = b"fragmented-trace-payload";
        let mut body = BytesMut::with_capacity(16 + payload.len());
        body.put_u32_le(MSG_UNKNOWN_TYPE_ONE);
        body.put_u32_le(0);
        body.put_u32_le(payload.len() as u32);
        body.put_u32_le(0);
        body.extend_from_slice(payload);
        let body = body.freeze();

        let split_at = 10;
        let first = encode_fragment(31, 0, 3, 4, false, body.len(), &[]);
        let second = encode_fragment(31, 1, 3, 4, false, split_at, &body[..split_at]);
        let third = encode_fragment(31, 2, 3, 4, false, body.len() - split_at, &body[split_at..]);

        let (client, mut server) = tokio::io::duplex(512);
        let mut conn = DtxConnection::new(client);

        let recv_task = tokio::spawn(async move { conn.recv_from_stream().await });
        server.write_all(&first).await.unwrap();
        server.write_all(&third).await.unwrap();
        server.write_all(&second).await.unwrap();

        let msg = recv_task
            .await
            .unwrap()
            .expect("fragment order should not affect reassembly");

        match msg.payload {
            DtxPayload::Raw(bytes) => assert_eq!(bytes.as_ref(), payload),
            other => panic!("expected raw payload, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn test_recv_rejects_duplicate_first_fragment() {
        let payload = b"fragmented-trace-payload";
        let mut body = BytesMut::with_capacity(16 + payload.len());
        body.put_u32_le(MSG_UNKNOWN_TYPE_ONE);
        body.put_u32_le(0);
        body.put_u32_le(payload.len() as u32);
        body.put_u32_le(0);
        body.extend_from_slice(payload);
        let body = body.freeze();

        let first = encode_fragment(41, 0, 2, 4, false, body.len(), &[]);
        let duplicate_first = encode_fragment(41, 0, 2, 4, false, body.len(), &[]);

        let (client, mut server) = tokio::io::duplex(512);
        let mut conn = DtxConnection::new(client);

        let recv_task = tokio::spawn(async move { conn.recv_from_stream().await });
        server.write_all(&first).await.unwrap();
        server.write_all(&duplicate_first).await.unwrap();

        let err = recv_task.await.unwrap().unwrap_err();
        assert!(matches!(
            err,
            DtxError::Protocol(message) if message.contains("duplicate first fragment")
        ));
    }

    #[tokio::test]
    async fn test_recv_rejects_fragment_without_first_fragment() {
        let payload = b"fragmented-trace-payload";
        let mut body = BytesMut::with_capacity(16 + payload.len());
        body.put_u32_le(MSG_UNKNOWN_TYPE_ONE);
        body.put_u32_le(0);
        body.put_u32_le(payload.len() as u32);
        body.put_u32_le(0);
        body.extend_from_slice(payload);
        let body = body.freeze();

        let stray = encode_fragment(43, 1, 2, 4, false, body.len(), &body);

        let (client, mut server) = tokio::io::duplex(512);
        let mut conn = DtxConnection::new(client);

        let recv_task = tokio::spawn(async move { conn.recv_from_stream().await });
        server.write_all(&stray).await.unwrap();

        let err = recv_task.await.unwrap().unwrap_err();
        assert!(matches!(
            err,
            DtxError::Protocol(message) if message.contains("without first fragment")
        ));
    }

    #[tokio::test]
    async fn test_recv_rejects_fragment_metadata_mismatch() {
        let payload = b"fragmented-trace-payload";
        let mut body = BytesMut::with_capacity(16 + payload.len());
        body.put_u32_le(MSG_UNKNOWN_TYPE_ONE);
        body.put_u32_le(0);
        body.put_u32_le(payload.len() as u32);
        body.put_u32_le(0);
        body.extend_from_slice(payload);
        let body = body.freeze();

        let split_at = 10;
        let first = encode_fragment(45, 0, 3, 4, false, body.len(), &[]);
        let bad_second = encode_fragment(45, 1, 3, 5, false, split_at, &body[..split_at]);

        let (client, mut server) = tokio::io::duplex(512);
        let mut conn = DtxConnection::new(client);

        let recv_task = tokio::spawn(async move { conn.recv_from_stream().await });
        server.write_all(&first).await.unwrap();
        server.write_all(&bad_second).await.unwrap();

        let err = recv_task.await.unwrap().unwrap_err();
        assert!(matches!(
            err,
            DtxError::Protocol(message) if message.contains("fragment metadata mismatch")
        ));
    }

    #[tokio::test]
    async fn test_recv_rejects_excessive_fragment_count_before_allocation() {
        let first = encode_fragment(72, 0, MAX_DTX_FRAGMENTS + 1, 4, false, 16, &[]);
        let mut cursor = std::io::Cursor::new(first);

        let err = match read_dtx_header(&mut cursor).await {
            Ok(_) => panic!("excessive fragment count should be rejected"),
            Err(err) => err,
        };

        assert!(matches!(
            err,
            DtxError::Protocol(message) if message.contains("too many fragments")
        ));
    }

    #[tokio::test]
    async fn test_recv_rejects_fragment_without_first_before_allocating_body() {
        let stray = encode_fragment(73, 1, 2, 4, false, MAX_DTX_MESSAGE_SIZE, &[]);
        let (client, mut server) = tokio::io::duplex(128);
        server.write_all(&stray).await.unwrap();
        let mut conn = DtxConnection::new(client);

        let err = conn.recv_from_stream().await.unwrap_err();
        assert!(matches!(
            err,
            DtxError::Protocol(message) if message.contains("without first fragment")
        ));
    }

    #[tokio::test]
    async fn test_recv_bounds_in_flight_fragmented_messages() {
        let (client, mut server) = tokio::io::duplex(4096);
        let mut conn = DtxConnection::new(client);
        for id in 0..=(MAX_IN_FLIGHT_FRAGMENTED_MESSAGES as u32) {
            let first = encode_fragment(id + 100, 0, 2, 4, false, 16, &[]);
            server.write_all(&first).await.unwrap();
        }

        let err = conn.recv_from_stream().await.unwrap_err();
        assert!(matches!(
            err,
            DtxError::Protocol(message) if message.contains("in-flight fragmented")
        ));
    }

    #[test]
    fn test_queue_message_drops_oldest_when_backlog_is_full() {
        let (client, _server) = tokio::io::duplex(64);
        let mut conn = DtxConnection::new(client);
        for identifier in 0..=(MAX_QUEUED_MESSAGES as u32) {
            conn.queue_message(DtxMessage {
                identifier,
                conversation_idx: 0,
                channel_code: 0,
                expects_reply: false,
                payload: DtxPayload::Empty,
            })
            .unwrap();
        }

        assert_eq!(conn.queued_messages.len(), MAX_QUEUED_MESSAGES);
        assert_eq!(
            conn.queued_messages
                .front()
                .map(|msg| msg.message.identifier),
            Some(1)
        );
        assert_eq!(
            conn.queued_messages
                .back()
                .map(|msg| msg.message.identifier),
            Some(MAX_QUEUED_MESSAGES as u32)
        );
    }

    fn raw_message(identifier: u32, payload: &'static [u8]) -> DtxMessage {
        DtxMessage {
            identifier,
            conversation_idx: 0,
            channel_code: 0,
            expects_reply: false,
            payload: DtxPayload::Raw(Bytes::from_static(payload)),
        }
    }

    #[test]
    fn test_memory_budget_accepts_exact_limit_and_rejects_overflow() {
        let mut budget = DtxMemoryBudget::new(usize::MAX);
        budget.used = usize::MAX - 1;
        let err = budget.try_reserve(2, "test").unwrap_err();
        assert!(matches!(
            err,
            DtxError::Protocol(message) if message.contains("accounting overflow")
        ));

        let mut budget = DtxMemoryBudget::new(8);
        budget.try_reserve(8, "test").unwrap();
        assert_eq!(budget.used, 8);
        let err = budget.try_reserve(1, "test").unwrap_err();
        assert!(matches!(
            err,
            DtxError::Protocol(message) if message.contains("budget exceeded") && message.contains("limit=8")
        ));
        budget.release(8);
        assert_eq!(budget.used, 0);
    }

    #[test]
    fn test_queued_message_budget_is_released_when_consumed() {
        let (client, _server) = tokio::io::duplex(64);
        let mut conn = DtxConnection::with_memory_limit(client, 4);

        conn.queue_message(raw_message(1, b"1234")).unwrap();
        assert_eq!(conn.memory_bytes_used(), 4);

        let err = conn.queue_message(raw_message(2, b"5")).unwrap_err();
        assert!(matches!(
            err,
            DtxError::Protocol(message) if message.contains("queued message")
        ));
        assert_eq!(conn.memory_bytes_used(), 4);

        let message = conn.pop_queued_message().unwrap();
        assert_eq!(message.identifier, 1);
        assert_eq!(conn.memory_bytes_used(), 0);
    }

    #[test]
    fn test_pending_reply_replacement_and_consumption_restore_budget() {
        let (client, _server) = tokio::io::duplex(64);
        let mut conn = DtxConnection::with_memory_limit(client, 4);

        conn.buffer_reply(raw_message(7, b"1234")).unwrap();
        assert_eq!(conn.memory_bytes_used(), 4);
        conn.buffer_reply(raw_message(7, b"12")).unwrap();
        assert_eq!(conn.memory_bytes_used(), 2);
        assert_eq!(conn.remove_pending_reply(7).unwrap().identifier, 7);
        assert_eq!(conn.memory_bytes_used(), 0);
    }

    #[test]
    fn test_pending_reply_flood_is_bounded_and_drains() {
        let (client, _server) = tokio::io::duplex(64);
        let mut conn = DtxConnection::with_memory_limit(client, 4);
        conn.buffer_reply(raw_message(9, b"1234")).unwrap();

        let err = conn.buffer_reply(raw_message(10, b"5678")).unwrap_err();
        assert!(matches!(
            err,
            DtxError::Protocol(message) if message.contains("pending reply")
        ));
        assert_eq!(conn.memory_bytes_used(), 4);
        assert_eq!(conn.remove_pending_reply(9).unwrap().identifier, 9);
        assert_eq!(conn.memory_bytes_used(), 0);
    }

    #[test]
    fn test_empty_pending_reply_flood_is_count_bounded() {
        let (client, _server) = tokio::io::duplex(64);
        let mut conn = DtxConnection::with_memory_limit(client, usize::MAX);
        for identifier in 0..MAX_PENDING_REPLIES as u32 {
            conn.buffer_reply(DtxMessage {
                identifier,
                conversation_idx: 1,
                channel_code: 0,
                expects_reply: false,
                payload: DtxPayload::Empty,
            })
            .unwrap();
        }

        let err = conn
            .buffer_reply(DtxMessage {
                identifier: MAX_PENDING_REPLIES as u32,
                conversation_idx: 1,
                channel_code: 0,
                expects_reply: false,
                payload: DtxPayload::Empty,
            })
            .unwrap_err();
        assert!(matches!(
            err,
            DtxError::Protocol(message) if message.contains("pending reply backlog full")
        ));
        assert_eq!(conn.pending_replies.len(), MAX_PENDING_REPLIES);
        assert_eq!(conn.memory_bytes_used(), 0);
    }

    #[test]
    fn test_method_selector_is_included_in_queued_budget() {
        let (client, _server) = tokio::io::duplex(64);
        let mut conn = DtxConnection::with_memory_limit(client, 3);
        let message = DtxMessage {
            identifier: 8,
            conversation_idx: 0,
            channel_code: 0,
            expects_reply: false,
            payload: DtxPayload::MethodInvocation {
                selector: "abc".to_string(),
                args: Vec::new(),
            },
        };

        conn.queue_message(message).unwrap();
        assert_eq!(conn.memory_bytes_used(), 3);
        conn.pop_queued_message().unwrap();
        assert_eq!(conn.memory_bytes_used(), 0);
    }

    fn fragmented_test_body() -> Bytes {
        let mut body = BytesMut::with_capacity(16);
        body.put_u32_le(MSG_OK);
        body.put_u32_le(0);
        body.put_u32_le(0);
        body.put_u32_le(0);
        body.freeze()
    }

    #[tokio::test]
    async fn test_fragment_reassembly_exact_budget_and_release() {
        let body = fragmented_test_body();
        let first = encode_fragment(81, 0, 3, 4, false, body.len(), &[]);
        let second = encode_fragment(81, 1, 3, 4, false, 8, &body[..8]);
        let third = encode_fragment(81, 2, 3, 4, false, 8, &body[8..]);
        let (client, mut server) = tokio::io::duplex(512);
        server.write_all(&first).await.unwrap();
        server.write_all(&second).await.unwrap();
        server.write_all(&third).await.unwrap();
        let mut conn = DtxConnection::with_memory_limit(client, body.len() * 2);

        let message = conn.recv_from_stream().await.unwrap();
        assert!(matches!(message.payload, DtxPayload::Empty));
        assert_eq!(conn.memory_bytes_used(), 0);
        assert!(conn.fragments.is_empty());
    }

    #[tokio::test]
    async fn test_fragment_budget_covers_concurrent_accumulators_and_cleans_on_overage() {
        let body = fragmented_test_body();
        let first_a = encode_fragment(82, 0, 3, 4, false, body.len(), &[]);
        let first_b = encode_fragment(83, 0, 3, 4, false, body.len(), &[]);
        let second_b = encode_fragment(83, 1, 3, 4, false, 4, &body[..4]);
        let second_a = encode_fragment(82, 1, 3, 4, false, 8, &body[..8]);
        let third_a = encode_fragment(82, 2, 3, 4, false, 8, &body[8..]);
        let (client, mut server) = tokio::io::duplex(1024);
        for frame in [first_a, first_b, second_b, second_a, third_a] {
            server.write_all(&frame).await.unwrap();
        }
        let mut conn = DtxConnection::with_memory_limit(client, 35);

        let err = conn.recv_from_stream().await.unwrap_err();
        assert!(matches!(
            err,
            DtxError::Protocol(message) if message.contains("fragment reassembly body")
        ));
        assert_eq!(conn.memory_bytes_used(), 0);
        assert!(conn.fragments.is_empty());
    }

    #[tokio::test]
    async fn test_fragment_read_error_releases_budget() {
        let body = fragmented_test_body();
        let first = encode_fragment(84, 0, 2, 4, false, body.len(), &[]);
        let truncated = encode_fragment(84, 1, 2, 4, false, 8, &body[..4]);
        let (client, mut server) = tokio::io::duplex(512);
        server.write_all(&first).await.unwrap();
        server.write_all(&truncated).await.unwrap();
        server.shutdown().await.unwrap();
        let mut conn = DtxConnection::with_memory_limit(client, 8);

        let err = conn.recv_from_stream().await.unwrap_err();
        assert!(matches!(err, DtxError::Io(_)));
        assert_eq!(conn.memory_bytes_used(), 0);
        assert!(conn.fragments.is_empty());
    }

    #[tokio::test]
    async fn test_single_body_budget_rejects_before_allocation() {
        let frame = encode_dtx(85, 0, 4, false, MSG_UNKNOWN_TYPE_ONE, b"12345", &[]);
        let body_len = frame.len() - 32;
        let (client, mut server) = tokio::io::duplex(256);
        server.write_all(&frame).await.unwrap();
        let mut conn = DtxConnection::with_memory_limit(client, body_len - 1);

        let err = conn.recv_from_stream().await.unwrap_err();
        assert!(matches!(
            err,
            DtxError::Protocol(message) if message.contains("single-fragment body")
        ));
        assert_eq!(conn.memory_bytes_used(), 0);
    }

    #[test]
    fn test_unrelated_message_flood_returns_budget_error_and_queue_can_drain() {
        let (client, _server) = tokio::io::duplex(64);
        let mut conn = DtxConnection::with_memory_limit(client, 4);
        conn.queue_message(raw_message(91, b"1234")).unwrap();

        let err = conn.queue_message(raw_message(92, b"5678")).unwrap_err();
        assert!(matches!(
            err,
            DtxError::Protocol(message) if message.contains("queued message")
        ));
        assert_eq!(conn.memory_bytes_used(), 4);
        assert_eq!(conn.pop_queued_message().unwrap().identifier, 91);
        assert_eq!(conn.memory_bytes_used(), 0);
    }

    #[test]
    fn test_ack_conversation_index_does_not_overflow() {
        let msg = DtxMessage {
            identifier: 1,
            conversation_idx: u32::MAX,
            channel_code: 0,
            expects_reply: true,
            payload: DtxPayload::Empty,
        };
        let ack = encode_ack(&msg);
        assert_eq!(
            u32::from_le_bytes(ack[20..24].try_into().unwrap()),
            u32::MAX
        );
    }

    #[test]
    fn test_ack_reverses_even_conversation_channel_normalization() {
        // A device DATA/dispatch frame on conv=0 may carry wire channel -2.
        // recv_from_stream exposes the logical channel +2, so the conv=1 ACK
        // must put -2 back on the wire.
        let wire_channel = -2;
        let logical_channel = normalize_incoming_channel_code(wire_channel, 0).unwrap();
        assert_eq!(logical_channel, 2);

        let ack = encode_ack(&DtxMessage {
            identifier: 17,
            conversation_idx: 0,
            channel_code: logical_channel,
            expects_reply: true,
            payload: DtxPayload::Empty,
        });

        assert_eq!(
            u32::from_le_bytes(ack[20..24].try_into().unwrap()),
            1,
            "ACK must advance the conversation index"
        );
        assert_eq!(
            i32::from_le_bytes(ack[24..28].try_into().unwrap()),
            wire_channel,
            "ACK must preserve the original wire channel sign"
        );
    }

    #[test]
    fn test_ack_keeps_odd_conversation_channel_unchanged() {
        // For an incoming odd-conversation frame the reader leaves the wire
        // channel untouched; its conv=2 ACK therefore does not negate it.
        let ack = encode_ack(&DtxMessage {
            identifier: 18,
            conversation_idx: 1,
            channel_code: -2,
            expects_reply: true,
            payload: DtxPayload::Empty,
        });

        assert_eq!(
            u32::from_le_bytes(ack[20..24].try_into().unwrap()),
            2,
            "ACK must advance the conversation index"
        );
        assert_eq!(i32::from_le_bytes(ack[24..28].try_into().unwrap()), -2);
    }

    #[test]
    fn test_ack_saturates_min_channel_when_negating() {
        // i32::MIN has no positive counterpart.  The malformed edge must not
        // panic in a debug build while all valid channel codes remain exact.
        let ack = encode_ack(&DtxMessage {
            identifier: 19,
            conversation_idx: 0,
            channel_code: i32::MIN,
            expects_reply: true,
            payload: DtxPayload::Empty,
        });

        assert_eq!(
            i32::from_le_bytes(ack[24..28].try_into().unwrap()),
            i32::MAX
        );
    }

    #[test]
    fn test_identifiers_and_channel_codes_fail_cleanly_at_limits() {
        let (client, _server) = tokio::io::duplex(64);
        let mut conn = DtxConnection::new(client);
        conn.identifier = u32::MAX;
        assert!(matches!(
            conn.next_id(),
            Err(DtxError::Protocol(message)) if message.contains("identifier exhausted")
        ));

        conn.channel_counter = i32::MAX;
        assert!(matches!(
            conn.next_channel_code(),
            Err(DtxError::Protocol(message)) if message.contains("channel code exhausted")
        ));
    }
}
