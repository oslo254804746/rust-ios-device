use std::collections::HashMap;
use std::fs::{self, File};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use bytes::BytesMut;
use serde::Serialize;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProxyProtocol {
    Lockdown,
    Dtx,
    Xpc,
    Binary,
}

impl ProxyProtocol {
    fn as_str(self) -> &'static str {
        match self {
            ProxyProtocol::Lockdown => "lockdown",
            ProxyProtocol::Dtx => "dtx",
            ProxyProtocol::Xpc => "xpc",
            ProxyProtocol::Binary => "binary",
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub enum Direction {
    HostToDevice,
    DeviceToHost,
}

impl Direction {
    fn as_str(self) -> &'static str {
        match self {
            Direction::HostToDevice => "host->device",
            Direction::DeviceToHost => "device->host",
        }
    }

    fn file_name(self) -> &'static str {
        match self {
            Direction::HostToDevice => "host-to-device.bin",
            Direction::DeviceToHost => "device-to-host.bin",
        }
    }
}

#[derive(Debug, Serialize)]
pub struct ProxyEvent {
    pub timestamp_ms: u128,
    pub direction: String,
    pub protocol: String,
    pub summary: String,
    pub decoded: serde_json::Value,
}

#[derive(Debug, thiserror::Error)]
pub enum DproxyError {
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
    #[error("serde error: {0}")]
    Serde(#[from] serde_json::Error),
    #[error("DTX decode error: {0}")]
    Dtx(#[from] crate::services::dtx::DtxError),
    #[error("protocol resource limit: {0}")]
    Protocol(String),
}

const MAX_DECODER_BUFFER: usize = crate::xpc::message::XPC_PENDING_BUFFER_LIMIT;
const MAX_XPC_STREAM_BUFFER: usize = crate::xpc::message::XPC_PENDING_BUFFER_LIMIT;
const MAX_XPC_TOTAL_BUFFER: usize = MAX_XPC_STREAM_BUFFER * 2;

pub struct ProxyRecorder {
    output_dir: PathBuf,
    events: File,
    host_to_device_raw: File,
    device_to_host_raw: File,
    host_to_device_decoder: StreamDecoder,
    device_to_host_decoder: StreamDecoder,
}

impl ProxyRecorder {
    pub fn new(output_dir: impl AsRef<Path>, protocol: ProxyProtocol) -> Result<Self, DproxyError> {
        let output_dir = output_dir.as_ref().to_path_buf();
        fs::create_dir_all(&output_dir)?;

        Ok(Self {
            events: File::create(output_dir.join("events.ndjson"))?,
            host_to_device_raw: File::create(output_dir.join(Direction::HostToDevice.file_name()))?,
            device_to_host_raw: File::create(output_dir.join(Direction::DeviceToHost.file_name()))?,
            host_to_device_decoder: StreamDecoder::new(protocol),
            device_to_host_decoder: StreamDecoder::new(protocol),
            output_dir,
        })
    }

    pub fn output_dir(&self) -> &Path {
        &self.output_dir
    }

    pub fn record_chunk(&mut self, direction: Direction, chunk: &[u8]) -> Result<(), DproxyError> {
        if chunk.is_empty() {
            return Ok(());
        }

        let events = match direction {
            Direction::HostToDevice => {
                self.host_to_device_raw.write_all(chunk)?;
                self.host_to_device_decoder.push(direction, chunk)?
            }
            Direction::DeviceToHost => {
                self.device_to_host_raw.write_all(chunk)?;
                self.device_to_host_decoder.push(direction, chunk)?
            }
        };

        self.write_events(events)
    }

    pub fn record_meta_event(
        &mut self,
        direction: Direction,
        protocol: &str,
        summary: impl Into<String>,
        decoded: serde_json::Value,
    ) -> Result<(), DproxyError> {
        self.write_events(vec![ProxyEvent {
            timestamp_ms: now_ms(),
            direction: direction.as_str().to_string(),
            protocol: protocol.to_string(),
            summary: summary.into(),
            decoded,
        }])
    }

    fn write_events(&mut self, events: Vec<ProxyEvent>) -> Result<(), DproxyError> {
        for event in events {
            serde_json::to_writer(&mut self.events, &event)?;
            self.events.write_all(b"\n")?;
            eprintln!("[{}] {} {}", event.protocol, event.direction, event.summary);
        }
        self.events.flush()?;
        Ok(())
    }
}

pub async fn proxy_bidirectional<L, R>(
    local: L,
    remote: R,
    recorder: ProxyRecorder,
) -> Result<(), DproxyError>
where
    L: AsyncRead + AsyncWrite + Unpin,
    R: AsyncRead + AsyncWrite + Unpin,
{
    let recorder = std::sync::Arc::new(tokio::sync::Mutex::new(recorder));
    let (local_reader, local_writer) = tokio::io::split(local);
    let (remote_reader, remote_writer) = tokio::io::split(remote);

    tokio::try_join!(
        pump(
            local_reader,
            remote_writer,
            Direction::HostToDevice,
            recorder.clone()
        ),
        pump(
            remote_reader,
            local_writer,
            Direction::DeviceToHost,
            recorder
        ),
    )?;

    Ok(())
}

async fn pump<R, W>(
    mut reader: R,
    mut writer: W,
    direction: Direction,
    recorder: std::sync::Arc<tokio::sync::Mutex<ProxyRecorder>>,
) -> Result<(), DproxyError>
where
    R: AsyncRead + Unpin,
    W: AsyncWrite + Unpin,
{
    let mut buf = [0u8; 16 * 1024];
    loop {
        let read = reader.read(&mut buf).await?;
        if read == 0 {
            writer.shutdown().await?;
            return Ok(());
        }

        {
            let mut recorder = recorder.lock().await;
            recorder.record_chunk(direction, &buf[..read])?;
        }

        writer.write_all(&buf[..read]).await?;
        writer.flush().await?;
    }
}

pub struct StreamDecoder {
    protocol: ProxyProtocol,
    buffer: BytesMut,
    xpc_streams: HashMap<u32, BytesMut>,
    xpc_buffered_bytes: usize,
    xpc_preface_handled: bool,
    dtx_broken: bool,
}

impl StreamDecoder {
    pub fn new(protocol: ProxyProtocol) -> Self {
        Self {
            protocol,
            buffer: BytesMut::new(),
            xpc_streams: HashMap::new(),
            xpc_buffered_bytes: 0,
            xpc_preface_handled: false,
            dtx_broken: false,
        }
    }

    pub fn push(
        &mut self,
        direction: Direction,
        chunk: &[u8],
    ) -> Result<Vec<ProxyEvent>, DproxyError> {
        if self.protocol == ProxyProtocol::Dtx && self.dtx_broken {
            return Ok(Vec::new());
        }

        let new_len = self.buffer.len().checked_add(chunk.len()).ok_or_else(|| {
            DproxyError::Protocol(format!(
                "decoder buffer length overflow: current {}, incoming {}",
                self.buffer.len(),
                chunk.len()
            ))
        })?;
        if new_len > MAX_DECODER_BUFFER {
            return Err(DproxyError::Protocol(format!(
                "decoder buffer length {new_len} exceeds limit {MAX_DECODER_BUFFER}"
            )));
        }
        self.buffer.extend_from_slice(chunk);
        match self.protocol {
            ProxyProtocol::Lockdown => Ok(self.decode_lockdown(direction)),
            ProxyProtocol::Dtx => Ok(self.decode_dtx(direction)),
            ProxyProtocol::Xpc => self.decode_xpc(direction),
            ProxyProtocol::Binary => Ok(Vec::new()),
        }
    }

    fn decode_lockdown(&mut self, direction: Direction) -> Vec<ProxyEvent> {
        let mut events = Vec::new();
        loop {
            if self.buffer.len() < 4 {
                break;
            }
            // Safety: self.buffer.len() >= 4 is checked above, so [..4] is exactly 4 bytes
            // and try_into::<[u8; 4]>() is infallible.
            let len = u32::from_be_bytes(self.buffer[..4].try_into().unwrap()) as usize;
            let frame_len = match 4usize.checked_add(len) {
                Some(frame_len) if frame_len <= MAX_DECODER_BUFFER => frame_len,
                _ => {
                    events.push(decoder_error_event(
                        direction,
                        self.protocol,
                        format!(
                            "lockdown frame length {len} exceeds decoder limit {MAX_DECODER_BUFFER}"
                        ),
                    ));
                    self.buffer.clear();
                    break;
                }
            };
            if self.buffer.len() < frame_len {
                break;
            }

            let _ = self.buffer.split_to(4);
            let payload = self.buffer.split_to(len).freeze();
            let decoded = plist::from_bytes::<plist::Value>(&payload)
                .map(plist_to_json)
                .unwrap_or_else(|_| serde_json::json!({"raw": hex::encode(payload)}));
            events.push(ProxyEvent {
                timestamp_ms: now_ms(),
                direction: direction.as_str().to_string(),
                protocol: self.protocol.as_str().to_string(),
                summary: summarize_lockdown(&decoded),
                decoded,
            });
        }
        events
    }

    fn decode_dtx(&mut self, direction: Direction) -> Vec<ProxyEvent> {
        let mut events = Vec::new();
        loop {
            match crate::services::dtx::decode_dtx_message_from_bytes(&self.buffer) {
                Ok(Some((message, consumed))) => {
                    let _ = self.buffer.split_to(consumed);
                    let decoded = dtx_message_to_json(&message);
                    events.push(ProxyEvent {
                        timestamp_ms: now_ms(),
                        direction: direction.as_str().to_string(),
                        protocol: self.protocol.as_str().to_string(),
                        summary: summarize_dtx(&message),
                        decoded,
                    });
                }
                Ok(None) => break,
                Err(err) => {
                    events.push(decoder_error_event(
                        direction,
                        self.protocol,
                        format!("DTX decode error: {err}"),
                    ));
                    self.buffer.clear();
                    self.dtx_broken = true;
                    break;
                }
            }
        }
        events
    }

    fn decode_xpc(&mut self, direction: Direction) -> Result<Vec<ProxyEvent>, DproxyError> {
        let mut events = Vec::new();
        loop {
            if self.consume_xpc_preface() {
                break;
            }

            let Some((stream_id, frame_type, frame_flags, payload, consumed)) =
                try_take_h2_frame(&self.buffer).map_err(DproxyError::Protocol)?
            else {
                break;
            };
            let _ = self.buffer.split_to(consumed);
            if frame_type != 0x00 {
                continue;
            }

            self.append_xpc_stream_data(stream_id, &payload)?;

            loop {
                let (message_result, consumed) = {
                    let Some(stream_buffer) = self.xpc_streams.get_mut(&stream_id) else {
                        return Err(DproxyError::Protocol(format!(
                            "XPC stream {stream_id} buffer disappeared while decoding"
                        )));
                    };
                    let before = stream_buffer.len();
                    let result = try_take_xpc_message(stream_buffer);
                    (result, before.saturating_sub(stream_buffer.len()))
                };
                self.xpc_buffered_bytes = self.xpc_buffered_bytes.saturating_sub(consumed);
                match message_result {
                    Ok(Some(message)) => {
                        let decoded = message
                            .body
                            .as_ref()
                            .map(xpc_value_to_json)
                            .unwrap_or(serde_json::Value::Null);
                        events.push(ProxyEvent {
                            timestamp_ms: now_ms(),
                            direction: direction.as_str().to_string(),
                            protocol: self.protocol.as_str().to_string(),
                            summary: summarize_xpc(stream_id, &message),
                            decoded,
                        });
                    }
                    Ok(None) => break,
                    Err(err) => {
                        events.push(decoder_error_event(direction, self.protocol, err));
                        self.clear_xpc_stream(stream_id);
                        break;
                    }
                }
            }
            if self
                .xpc_streams
                .get(&stream_id)
                .is_some_and(BytesMut::is_empty)
            {
                self.xpc_streams.remove(&stream_id);
            }
            if frame_flags & FLAG_END_STREAM != 0 && self.xpc_streams.contains_key(&stream_id) {
                events.push(decoder_error_event(
                    direction,
                    self.protocol,
                    format!("XPC stream {stream_id} ended with an incomplete message"),
                ));
                self.clear_xpc_stream(stream_id);
            }
        }
        Ok(events)
    }

    fn unknown_xpc_stream_count(&self) -> usize {
        self.xpc_streams
            .keys()
            .filter(|stream_id| {
                **stream_id != crate::xpc::h2_raw::STREAM_CLIENT_SERVER
                    && **stream_id != crate::xpc::h2_raw::STREAM_SERVER_CLIENT
            })
            .count()
    }

    fn clear_xpc_stream(&mut self, stream_id: u32) {
        if let Some(buffer) = self.xpc_streams.remove(&stream_id) {
            self.xpc_buffered_bytes = self.xpc_buffered_bytes.saturating_sub(buffer.len());
        }
    }

    fn append_xpc_stream_data(
        &mut self,
        stream_id: u32,
        payload: &[u8],
    ) -> Result<(), DproxyError> {
        if payload.is_empty() {
            return Ok(());
        }
        let is_primary = matches!(
            stream_id,
            crate::xpc::h2_raw::STREAM_CLIENT_SERVER | crate::xpc::h2_raw::STREAM_SERVER_CLIENT
        );
        if !is_primary
            && !self.xpc_streams.contains_key(&stream_id)
            && self.unknown_xpc_stream_count() >= crate::xpc::h2_raw::MAX_UNKNOWN_BUFFERED_STREAMS
        {
            return Err(DproxyError::Protocol(format!(
                "too many unknown XPC streams with buffered data: limit {}",
                crate::xpc::h2_raw::MAX_UNKNOWN_BUFFERED_STREAMS
            )));
        }

        let current = self.xpc_streams.get(&stream_id).map_or(0, BytesMut::len);
        let requested = current.checked_add(payload.len()).ok_or_else(|| {
            self.clear_xpc_stream(stream_id);
            DproxyError::Protocol(format!(
                "XPC stream {stream_id} buffer length overflow: current {current}, incoming {}",
                payload.len()
            ))
        })?;
        if requested > MAX_XPC_STREAM_BUFFER {
            self.clear_xpc_stream(stream_id);
            return Err(DproxyError::Protocol(format!(
                "XPC stream {stream_id} buffer {requested} exceeds limit {MAX_XPC_STREAM_BUFFER}"
            )));
        }

        let total = self
            .xpc_buffered_bytes
            .checked_add(payload.len())
            .ok_or_else(|| {
                self.clear_xpc_stream(stream_id);
                DproxyError::Protocol(format!(
                    "XPC buffered byte count overflow: current {}, incoming {}",
                    self.xpc_buffered_bytes,
                    payload.len()
                ))
            })?;
        if total > MAX_XPC_TOTAL_BUFFER {
            self.clear_xpc_stream(stream_id);
            return Err(DproxyError::Protocol(format!(
                "XPC buffered bytes {total} exceed connection limit {MAX_XPC_TOTAL_BUFFER}"
            )));
        }

        self.xpc_streams
            .entry(stream_id)
            .or_default()
            .extend_from_slice(payload);
        self.xpc_buffered_bytes = total;
        Ok(())
    }

    fn consume_xpc_preface(&mut self) -> bool {
        if self.xpc_preface_handled {
            return false;
        }

        let preface = crate::xpc::h2_raw::H2_PREFACE;
        if self.buffer.len() < preface.len() {
            if preface.starts_with(self.buffer.as_ref()) {
                return true;
            }
            self.xpc_preface_handled = true;
            return false;
        }

        if self.buffer.starts_with(preface) {
            let _ = self.buffer.split_to(preface.len());
        }
        self.xpc_preface_handled = true;
        false
    }
}

fn decoder_error_event(
    direction: Direction,
    protocol: ProxyProtocol,
    summary: impl Into<String>,
) -> ProxyEvent {
    ProxyEvent {
        timestamp_ms: now_ms(),
        direction: direction.as_str().to_string(),
        protocol: protocol.as_str().to_string(),
        summary: summary.into(),
        decoded: serde_json::Value::Null,
    }
}

const FLAG_END_STREAM: u8 = 0x01;

type DecodedH2Frame = (u32, u8, u8, Vec<u8>, usize);

fn try_take_h2_frame(buffer: &[u8]) -> Result<Option<DecodedH2Frame>, String> {
    if buffer.len() < 9 {
        return Ok(None);
    }
    let len = ((buffer[0] as usize) << 16) | ((buffer[1] as usize) << 8) | buffer[2] as usize;
    if len > crate::xpc::h2_raw::MAX_FRAME_PAYLOAD {
        return Err(format!(
            "H2 frame payload {len} exceeds max frame size {}",
            crate::xpc::h2_raw::MAX_FRAME_PAYLOAD
        ));
    }
    let total = 9 + len;
    if buffer.len() < total {
        return Ok(None);
    }
    let frame_type = buffer[3];
    let frame_flags = buffer[4];
    let stream_id = u32::from_be_bytes([buffer[5] & 0x7f, buffer[6], buffer[7], buffer[8]]);
    Ok(Some((
        stream_id,
        frame_type,
        frame_flags,
        buffer[9..total].to_vec(),
        total,
    )))
}

fn try_take_xpc_message(buffer: &mut BytesMut) -> Result<Option<crate::xpc::XpcMessage>, String> {
    if buffer.len() < 24 {
        return Ok(None);
    }

    let declared_body_len = u64::from_le_bytes(
        buffer[8..16]
            .try_into()
            .map_err(|_| "invalid XPC header".to_string())?,
    );
    let message_flags = u32::from_le_bytes(
        buffer[4..8]
            .try_into()
            .map_err(|_| "invalid XPC flags".to_string())?,
    );
    let body_len = crate::xpc::message::checked_xpc_body_len(
        declared_body_len,
        crate::xpc::message::xpc_body_limit_for_flags(message_flags),
    )?;
    let total = 24usize
        .checked_add(body_len)
        .ok_or_else(|| "XPC message length overflow".to_string())?;
    if buffer.len() < total {
        return Ok(None);
    }

    let payload = buffer.split_to(total).freeze();
    crate::xpc::message::decode_message(payload)
        .map(Some)
        .map_err(|err| err.to_string())
}

fn summarize_lockdown(decoded: &serde_json::Value) -> String {
    decoded
        .get("Request")
        .or_else(|| decoded.get("Error"))
        .or_else(|| decoded.get("Type"))
        .map(|value| value.to_string().trim_matches('"').to_string())
        .unwrap_or_else(|| "lockdown frame".into())
}

fn summarize_dtx(message: &crate::services::dtx::DtxMessage) -> String {
    match &message.payload {
        crate::services::dtx::DtxPayload::MethodInvocation { selector, .. } => format!(
            "{}.{}{} c{} {}",
            message.identifier,
            message.conversation_idx,
            if message.expects_reply { "e" } else { "" },
            message.channel_code,
            selector
        ),
        crate::services::dtx::DtxPayload::Response(value) => format!(
            "{}.{} c{} response {:?}",
            message.identifier, message.conversation_idx, message.channel_code, value
        ),
        crate::services::dtx::DtxPayload::Notification { name, .. } => format!(
            "{}.{} c{} notify {}",
            message.identifier, message.conversation_idx, message.channel_code, name
        ),
        crate::services::dtx::DtxPayload::Raw(bytes) => format!(
            "{}.{} c{} raw {} bytes",
            message.identifier,
            message.conversation_idx,
            message.channel_code,
            bytes.len()
        ),
        crate::services::dtx::DtxPayload::RawWithAux { payload, .. } => format!(
            "{}.{} c{} raw {} bytes",
            message.identifier,
            message.conversation_idx,
            message.channel_code,
            payload.len()
        ),
        crate::services::dtx::DtxPayload::Empty => format!(
            "{}.{} c{} empty",
            message.identifier, message.conversation_idx, message.channel_code
        ),
    }
}

fn summarize_xpc(stream_id: u32, message: &crate::xpc::XpcMessage) -> String {
    let keys = message
        .body
        .as_ref()
        .and_then(crate::xpc::XpcValue::as_dict)
        .map(|dict| dict.keys().take(4).cloned().collect::<Vec<_>>().join(","))
        .unwrap_or_else(|| "no-body".into());
    format!(
        "stream={} msg_id={} flags=0x{:08x} keys=[{}]",
        stream_id, message.msg_id, message.flags, keys
    )
}

fn dtx_message_to_json(message: &crate::services::dtx::DtxMessage) -> serde_json::Value {
    let payload = match &message.payload {
        crate::services::dtx::DtxPayload::MethodInvocation { selector, args } => {
            serde_json::json!({
                "type": "method",
                "selector": selector,
                "args": args.iter().map(nsobject_to_json).collect::<Vec<_>>(),
            })
        }
        crate::services::dtx::DtxPayload::Response(value) => serde_json::json!({
            "type": "response",
            "value": nsobject_to_json(value),
        }),
        crate::services::dtx::DtxPayload::Notification { name, object } => serde_json::json!({
            "type": "notification",
            "name": name,
            "object": nsobject_to_json(object),
        }),
        crate::services::dtx::DtxPayload::Raw(bytes) => serde_json::json!({
            "type": "raw",
            "bytes": hex::encode(bytes),
        }),
        crate::services::dtx::DtxPayload::RawWithAux { payload, aux } => serde_json::json!({
            "type": "raw_with_aux",
            "payload": hex::encode(payload),
            "aux": aux.iter().map(nsobject_to_json).collect::<Vec<_>>(),
        }),
        crate::services::dtx::DtxPayload::Empty => serde_json::json!({"type": "empty"}),
    };

    serde_json::json!({
        "identifier": message.identifier,
        "conversation_idx": message.conversation_idx,
        "channel_code": message.channel_code,
        "expects_reply": message.expects_reply,
        "payload": payload,
    })
}

fn nsobject_to_json(value: &crate::services::dtx::NSObject) -> serde_json::Value {
    match value {
        crate::services::dtx::NSObject::Int(value) => serde_json::Value::from(*value),
        crate::services::dtx::NSObject::Uint(value) => serde_json::Value::from(*value),
        crate::services::dtx::NSObject::Double(value) => serde_json::Number::from_f64(*value)
            .map(serde_json::Value::Number)
            .unwrap_or(serde_json::Value::Null),
        crate::services::dtx::NSObject::Bool(value) => serde_json::Value::Bool(*value),
        crate::services::dtx::NSObject::String(value) => serde_json::Value::String(value.clone()),
        crate::services::dtx::NSObject::Data(value) => {
            serde_json::Value::String(hex::encode(value))
        }
        crate::services::dtx::NSObject::Array(values) => {
            serde_json::Value::Array(values.iter().map(nsobject_to_json).collect())
        }
        crate::services::dtx::NSObject::Dict(values) => serde_json::Value::Object(
            values
                .iter()
                .map(|(key, value)| (key.clone(), nsobject_to_json(value)))
                .collect(),
        ),
        crate::services::dtx::NSObject::Null => serde_json::Value::Null,
    }
}

fn xpc_value_to_json(value: &crate::xpc::XpcValue) -> serde_json::Value {
    match value {
        crate::xpc::XpcValue::Null => serde_json::Value::Null,
        crate::xpc::XpcValue::Bool(value) => serde_json::Value::Bool(*value),
        crate::xpc::XpcValue::Int64(value) => serde_json::Value::from(*value),
        crate::xpc::XpcValue::Uint64(value) => serde_json::Value::from(*value),
        crate::xpc::XpcValue::Double(value) => serde_json::Number::from_f64(*value)
            .map(serde_json::Value::Number)
            .unwrap_or(serde_json::Value::Null),
        crate::xpc::XpcValue::Date(value) => serde_json::Value::from(*value),
        crate::xpc::XpcValue::Data(bytes) => serde_json::Value::String(hex::encode(bytes)),
        crate::xpc::XpcValue::String(value) => serde_json::Value::String(value.clone()),
        crate::xpc::XpcValue::Uuid(bytes) => {
            serde_json::Value::String(uuid::Uuid::from_bytes(*bytes).to_string())
        }
        crate::xpc::XpcValue::Array(values) => {
            serde_json::Value::Array(values.iter().map(xpc_value_to_json).collect())
        }
        crate::xpc::XpcValue::Dictionary(values) => serde_json::Value::Object(
            values
                .iter()
                .map(|(key, value)| (key.clone(), xpc_value_to_json(value)))
                .collect(),
        ),
        crate::xpc::XpcValue::FileTransfer { msg_id, data } => serde_json::json!({
            "msg_id": msg_id,
            "data": xpc_value_to_json(data),
        }),
    }
}

fn plist_to_json(value: plist::Value) -> serde_json::Value {
    match value {
        plist::Value::String(value) => serde_json::Value::String(value),
        plist::Value::Boolean(value) => serde_json::Value::Bool(value),
        plist::Value::Integer(value) => value
            .as_signed()
            .map(serde_json::Value::from)
            .or_else(|| value.as_unsigned().map(serde_json::Value::from))
            .unwrap_or(serde_json::Value::Null),
        plist::Value::Real(value) => serde_json::Number::from_f64(value)
            .map(serde_json::Value::Number)
            .unwrap_or(serde_json::Value::Null),
        plist::Value::Data(bytes) => serde_json::Value::String(hex::encode(bytes)),
        plist::Value::Array(values) => {
            serde_json::Value::Array(values.into_iter().map(plist_to_json).collect())
        }
        plist::Value::Dictionary(values) => serde_json::Value::Object(
            values
                .into_iter()
                .map(|(key, value)| (key, plist_to_json(value)))
                .collect(),
        ),
        plist::Value::Date(value) => serde_json::Value::String(value.to_xml_format()),
        plist::Value::Uid(value) => serde_json::Value::from(value.get()),
        _ => serde_json::Value::Null,
    }
}

fn now_ms() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|value| value.as_millis())
        .unwrap_or(0)
}

#[cfg(test)]
fn build_h2_frame(stream_id: u32, frame_type: u8, payload: &[u8]) -> Vec<u8> {
    let len = payload.len();
    let mut frame = Vec::with_capacity(9 + len);
    frame.push(((len >> 16) & 0xff) as u8);
    frame.push(((len >> 8) & 0xff) as u8);
    frame.push((len & 0xff) as u8);
    frame.push(frame_type);
    frame.push(0);
    frame.extend_from_slice(&(stream_id & 0x7fff_ffff).to_be_bytes());
    frame.extend_from_slice(payload);
    frame
}

#[cfg(test)]
fn build_h2_data_frame(stream_id: u32, payload: &[u8]) -> Vec<u8> {
    build_h2_frame(stream_id, 0x00, payload)
}

#[cfg(test)]
mod tests {
    use crate::xpc::XpcValue;
    use indexmap::IndexMap;

    use super::*;

    #[test]
    fn lockdown_decoder_extracts_complete_frames() {
        let mut payload = Vec::new();
        let plist = plist::Value::Dictionary(plist::Dictionary::from_iter([(
            "Request".to_string(),
            plist::Value::String("QueryType".into()),
        )]));
        plist::to_writer_xml(&mut payload, &plist).unwrap();

        let mut framed = Vec::new();
        framed.extend_from_slice(&(payload.len() as u32).to_be_bytes());
        framed.extend_from_slice(&payload);

        let mut decoder = StreamDecoder::new(ProxyProtocol::Lockdown);
        let events = decoder.push(Direction::HostToDevice, &framed).unwrap();

        assert_eq!(events.len(), 1);
        assert_eq!(events[0].protocol, "lockdown");
        assert_eq!(events[0].decoded["Request"], "QueryType");
    }

    #[test]
    fn lockdown_decoder_rejects_length_overflow_without_waiting() {
        let mut decoder = StreamDecoder::new(ProxyProtocol::Lockdown);
        let events = decoder
            .push(Direction::HostToDevice, &u32::MAX.to_be_bytes())
            .unwrap();

        assert_eq!(events.len(), 1);
        assert!(events[0].summary.contains("lockdown frame length"));
        assert!(decoder.buffer.is_empty());
    }

    #[test]
    fn dtx_decoder_reassembles_fragmented_messages() {
        let selector =
            crate::proto::nskeyedarchiver_encode::archive_string("_notifyOfPublishedCapabilities:");
        let encoded = crate::services::dtx::encode_dtx(1, 0, 0, true, 2, &selector, &[]);

        let mut decoder = StreamDecoder::new(ProxyProtocol::Dtx);
        assert!(decoder
            .push(Direction::HostToDevice, &encoded[..10])
            .unwrap()
            .is_empty());
        let events = decoder
            .push(Direction::HostToDevice, &encoded[10..])
            .unwrap();

        assert_eq!(events.len(), 1);
        assert!(events[0]
            .summary
            .contains("_notifyOfPublishedCapabilities:"));
    }

    #[test]
    fn dtx_decoder_reports_errors_without_aborting_recording() {
        let mut decoder = StreamDecoder::new(ProxyProtocol::Dtx);

        let events = decoder.push(Direction::HostToDevice, &[0u8; 32]).unwrap();
        assert_eq!(events.len(), 1);
        assert!(events[0].summary.contains("DTX decode error: bad magic"));
        assert!(decoder.dtx_broken);
        assert!(decoder.buffer.is_empty());

        let selector = crate::proto::nskeyedarchiver_encode::archive_string("after-error");
        let encoded = crate::services::dtx::encode_dtx(2, 0, 0, true, 2, &selector, &[]);
        assert!(decoder
            .push(Direction::HostToDevice, &encoded)
            .unwrap()
            .is_empty());
        assert!(decoder.buffer.is_empty());
    }

    #[test]
    fn xpc_decoder_reassembles_messages_across_h2_frames() {
        let payload = crate::xpc::message::encode_message(&crate::xpc::XpcMessage {
            flags: crate::xpc::message::flags::ALWAYS_SET
                | crate::xpc::message::flags::DATA
                | crate::xpc::message::flags::REPLY,
            msg_id: 7,
            body: Some(XpcValue::Dictionary(IndexMap::from([(
                "result".to_string(),
                XpcValue::String("success".into()),
            )]))),
        })
        .unwrap();

        let first = build_h2_data_frame(3, &payload[..12]);
        let second = build_h2_data_frame(3, &payload[12..]);

        let mut decoder = StreamDecoder::new(ProxyProtocol::Xpc);
        assert!(decoder
            .push(Direction::DeviceToHost, &first)
            .unwrap()
            .is_empty());
        let events = decoder.push(Direction::DeviceToHost, &second).unwrap();

        assert_eq!(events.len(), 1);
        assert_eq!(events[0].protocol, "xpc");
        assert_eq!(events[0].decoded["result"], "success");
        assert_eq!(decoder.xpc_buffered_bytes, 0);
        assert!(decoder.xpc_streams.is_empty());
    }

    #[test]
    fn xpc_decoder_clears_partial_message_at_end_stream() {
        let payload = crate::xpc::message::encode_message(&crate::xpc::XpcMessage {
            flags: crate::xpc::message::flags::ALWAYS_SET | crate::xpc::message::flags::DATA,
            msg_id: 7,
            body: Some(XpcValue::Dictionary(IndexMap::new())),
        })
        .unwrap();
        let mut frame = build_h2_frame(3, 0x00, &payload[..payload.len() - 1]);
        frame[4] = FLAG_END_STREAM;

        let mut decoder = StreamDecoder::new(ProxyProtocol::Xpc);
        let events = decoder.push(Direction::DeviceToHost, &frame).unwrap();

        assert_eq!(events.len(), 1);
        assert!(events[0].summary.contains("incomplete message"));
        assert_eq!(decoder.xpc_buffered_bytes, 0);
        assert!(decoder.xpc_streams.is_empty());
    }

    #[test]
    fn xpc_decoder_rejects_u64_max_body_before_allocating() {
        let mut message = Vec::with_capacity(24);
        message.extend_from_slice(&crate::xpc::message::WRAPPER_MAGIC.to_le_bytes());
        message.extend_from_slice(&crate::xpc::message::flags::ALWAYS_SET.to_le_bytes());
        message.extend_from_slice(&u64::MAX.to_le_bytes());
        message.extend_from_slice(&1u64.to_le_bytes());

        let mut decoder = StreamDecoder::new(ProxyProtocol::Xpc);
        let events = decoder
            .push(Direction::DeviceToHost, &build_h2_data_frame(3, &message))
            .unwrap();

        assert_eq!(events.len(), 1);
        assert!(events[0].summary.contains(&u64::MAX.to_string()));
        assert_eq!(decoder.xpc_buffered_bytes, 0);
        assert!(decoder.xpc_streams.is_empty());
    }

    #[test]
    fn xpc_decoder_rejects_unknown_stream_flood_with_bounded_error() {
        let mut bytes = Vec::new();
        for stream_id in 100..100 + crate::xpc::h2_raw::MAX_UNKNOWN_BUFFERED_STREAMS as u32 + 1 {
            bytes.extend_from_slice(&build_h2_data_frame(stream_id, &[stream_id as u8]));
        }

        let mut decoder = StreamDecoder::new(ProxyProtocol::Xpc);
        let error = decoder.push(Direction::DeviceToHost, &bytes).unwrap_err();

        assert!(error.to_string().contains("too many unknown XPC streams"));
        assert_eq!(
            decoder.xpc_streams.len(),
            crate::xpc::h2_raw::MAX_UNKNOWN_BUFFERED_STREAMS
        );
        assert_eq!(
            decoder.xpc_buffered_bytes,
            crate::xpc::h2_raw::MAX_UNKNOWN_BUFFERED_STREAMS
        );
    }

    #[test]
    fn xpc_decoder_rejects_oversized_h2_frame_before_copying_payload() {
        let length = crate::xpc::h2_raw::MAX_FRAME_PAYLOAD + 1;
        let mut frame = vec![0u8; 9];
        frame[0] = ((length >> 16) & 0xff) as u8;
        frame[1] = ((length >> 8) & 0xff) as u8;
        frame[2] = (length & 0xff) as u8;
        frame[3] = 0;
        frame[5..9].copy_from_slice(&3u32.to_be_bytes());

        let error = try_take_h2_frame(&frame).unwrap_err();
        assert!(error.contains("exceeds max frame size"));
    }

    #[test]
    fn xpc_decoder_skips_split_http2_client_preface() {
        let payload = crate::xpc::message::encode_message(&crate::xpc::XpcMessage {
            flags: crate::xpc::message::flags::ALWAYS_SET | crate::xpc::message::flags::DATA,
            msg_id: 9,
            body: Some(XpcValue::Dictionary(IndexMap::from([(
                "request".to_string(),
                XpcValue::String("ping".into()),
            )]))),
        })
        .unwrap();

        let mut decoder = StreamDecoder::new(ProxyProtocol::Xpc);
        let preface = crate::xpc::h2_raw::H2_PREFACE;
        let split_at = 10;
        assert!(decoder
            .push(Direction::HostToDevice, &preface[..split_at])
            .unwrap()
            .is_empty());

        let mut second = preface[split_at..].to_vec();
        second.extend_from_slice(&build_h2_frame(0, 0x04, &[]));
        second.extend_from_slice(&build_h2_data_frame(1, &payload));

        let events = decoder.push(Direction::HostToDevice, &second).unwrap();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].decoded["request"], "ping");
    }
}
