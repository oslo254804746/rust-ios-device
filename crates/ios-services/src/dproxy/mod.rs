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
    Dtx(#[from] crate::dtx::DtxError),
}

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
    xpc_preface_handled: bool,
    dtx_broken: bool,
}

impl StreamDecoder {
    pub fn new(protocol: ProxyProtocol) -> Self {
        Self {
            protocol,
            buffer: BytesMut::new(),
            xpc_streams: HashMap::new(),
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

        self.buffer.extend_from_slice(chunk);
        match self.protocol {
            ProxyProtocol::Lockdown => Ok(self.decode_lockdown(direction)),
            ProxyProtocol::Dtx => Ok(self.decode_dtx(direction)),
            ProxyProtocol::Xpc => Ok(self.decode_xpc(direction)),
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
            if self.buffer.len() < 4 + len {
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
            match crate::dtx::decode_dtx_message_from_bytes(&self.buffer) {
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

    fn decode_xpc(&mut self, direction: Direction) -> Vec<ProxyEvent> {
        let mut events = Vec::new();
        loop {
            if self.consume_xpc_preface() {
                break;
            }

            let Some((stream_id, frame_type, payload, consumed)) = try_take_h2_frame(&self.buffer)
            else {
                break;
            };
            let _ = self.buffer.split_to(consumed);
            if frame_type != 0x00 {
                continue;
            }

            let stream_buffer = self.xpc_streams.entry(stream_id).or_default();
            stream_buffer.extend_from_slice(&payload);

            loop {
                match try_take_xpc_message(stream_buffer) {
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
                        stream_buffer.clear();
                        break;
                    }
                }
            }
        }
        events
    }

    fn consume_xpc_preface(&mut self) -> bool {
        if self.xpc_preface_handled {
            return false;
        }

        let preface = ios_xpc::h2_raw::H2_PREFACE;
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

fn try_take_h2_frame(buffer: &[u8]) -> Option<(u32, u8, Vec<u8>, usize)> {
    if buffer.len() < 9 {
        return None;
    }
    let len = ((buffer[0] as usize) << 16) | ((buffer[1] as usize) << 8) | buffer[2] as usize;
    let total = 9 + len;
    if buffer.len() < total {
        return None;
    }
    let frame_type = buffer[3];
    let stream_id = u32::from_be_bytes([buffer[5] & 0x7f, buffer[6], buffer[7], buffer[8]]);
    Some((stream_id, frame_type, buffer[9..total].to_vec(), total))
}

fn try_take_xpc_message(buffer: &mut BytesMut) -> Result<Option<ios_xpc::XpcMessage>, String> {
    if buffer.len() < 24 {
        return Ok(None);
    }

    let body_len = u64::from_le_bytes(
        buffer[8..16]
            .try_into()
            .map_err(|_| "invalid XPC header".to_string())?,
    ) as usize;
    let total = 24usize
        .checked_add(body_len)
        .ok_or_else(|| "XPC message length overflow".to_string())?;
    if buffer.len() < total {
        return Ok(None);
    }

    let payload = buffer.split_to(total).freeze();
    ios_xpc::message::decode_message(payload)
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

fn summarize_dtx(message: &crate::dtx::DtxMessage) -> String {
    match &message.payload {
        crate::dtx::DtxPayload::MethodInvocation { selector, .. } => format!(
            "{}.{}{} c{} {}",
            message.identifier,
            message.conversation_idx,
            if message.expects_reply { "e" } else { "" },
            message.channel_code,
            selector
        ),
        crate::dtx::DtxPayload::Response(value) => format!(
            "{}.{} c{} response {:?}",
            message.identifier, message.conversation_idx, message.channel_code, value
        ),
        crate::dtx::DtxPayload::Notification { name, .. } => format!(
            "{}.{} c{} notify {}",
            message.identifier, message.conversation_idx, message.channel_code, name
        ),
        crate::dtx::DtxPayload::Raw(bytes) => format!(
            "{}.{} c{} raw {} bytes",
            message.identifier,
            message.conversation_idx,
            message.channel_code,
            bytes.len()
        ),
        crate::dtx::DtxPayload::RawWithAux { payload, .. } => format!(
            "{}.{} c{} raw {} bytes",
            message.identifier,
            message.conversation_idx,
            message.channel_code,
            payload.len()
        ),
        crate::dtx::DtxPayload::Empty => format!(
            "{}.{} c{} empty",
            message.identifier, message.conversation_idx, message.channel_code
        ),
    }
}

fn summarize_xpc(stream_id: u32, message: &ios_xpc::XpcMessage) -> String {
    let keys = message
        .body
        .as_ref()
        .and_then(ios_xpc::XpcValue::as_dict)
        .map(|dict| dict.keys().take(4).cloned().collect::<Vec<_>>().join(","))
        .unwrap_or_else(|| "no-body".into());
    format!(
        "stream={} msg_id={} flags=0x{:08x} keys=[{}]",
        stream_id, message.msg_id, message.flags, keys
    )
}

fn dtx_message_to_json(message: &crate::dtx::DtxMessage) -> serde_json::Value {
    let payload = match &message.payload {
        crate::dtx::DtxPayload::MethodInvocation { selector, args } => serde_json::json!({
            "type": "method",
            "selector": selector,
            "args": args.iter().map(nsobject_to_json).collect::<Vec<_>>(),
        }),
        crate::dtx::DtxPayload::Response(value) => serde_json::json!({
            "type": "response",
            "value": nsobject_to_json(value),
        }),
        crate::dtx::DtxPayload::Notification { name, object } => serde_json::json!({
            "type": "notification",
            "name": name,
            "object": nsobject_to_json(object),
        }),
        crate::dtx::DtxPayload::Raw(bytes) => serde_json::json!({
            "type": "raw",
            "bytes": hex::encode(bytes),
        }),
        crate::dtx::DtxPayload::RawWithAux { payload, aux } => serde_json::json!({
            "type": "raw_with_aux",
            "payload": hex::encode(payload),
            "aux": aux.iter().map(nsobject_to_json).collect::<Vec<_>>(),
        }),
        crate::dtx::DtxPayload::Empty => serde_json::json!({"type": "empty"}),
    };

    serde_json::json!({
        "identifier": message.identifier,
        "conversation_idx": message.conversation_idx,
        "channel_code": message.channel_code,
        "expects_reply": message.expects_reply,
        "payload": payload,
    })
}

fn nsobject_to_json(value: &crate::dtx::NSObject) -> serde_json::Value {
    match value {
        crate::dtx::NSObject::Int(value) => serde_json::Value::from(*value),
        crate::dtx::NSObject::Uint(value) => serde_json::Value::from(*value),
        crate::dtx::NSObject::Double(value) => serde_json::Number::from_f64(*value)
            .map(serde_json::Value::Number)
            .unwrap_or(serde_json::Value::Null),
        crate::dtx::NSObject::Bool(value) => serde_json::Value::Bool(*value),
        crate::dtx::NSObject::String(value) => serde_json::Value::String(value.clone()),
        crate::dtx::NSObject::Data(value) => serde_json::Value::String(hex::encode(value)),
        crate::dtx::NSObject::Array(values) => {
            serde_json::Value::Array(values.iter().map(nsobject_to_json).collect())
        }
        crate::dtx::NSObject::Dict(values) => serde_json::Value::Object(
            values
                .iter()
                .map(|(key, value)| (key.clone(), nsobject_to_json(value)))
                .collect(),
        ),
        crate::dtx::NSObject::Null => serde_json::Value::Null,
    }
}

fn xpc_value_to_json(value: &ios_xpc::XpcValue) -> serde_json::Value {
    match value {
        ios_xpc::XpcValue::Null => serde_json::Value::Null,
        ios_xpc::XpcValue::Bool(value) => serde_json::Value::Bool(*value),
        ios_xpc::XpcValue::Int64(value) => serde_json::Value::from(*value),
        ios_xpc::XpcValue::Uint64(value) => serde_json::Value::from(*value),
        ios_xpc::XpcValue::Double(value) => serde_json::Number::from_f64(*value)
            .map(serde_json::Value::Number)
            .unwrap_or(serde_json::Value::Null),
        ios_xpc::XpcValue::Date(value) => serde_json::Value::from(*value),
        ios_xpc::XpcValue::Data(bytes) => serde_json::Value::String(hex::encode(bytes)),
        ios_xpc::XpcValue::String(value) => serde_json::Value::String(value.clone()),
        ios_xpc::XpcValue::Uuid(bytes) => {
            serde_json::Value::String(uuid::Uuid::from_bytes(*bytes).to_string())
        }
        ios_xpc::XpcValue::Array(values) => {
            serde_json::Value::Array(values.iter().map(xpc_value_to_json).collect())
        }
        ios_xpc::XpcValue::Dictionary(values) => serde_json::Value::Object(
            values
                .iter()
                .map(|(key, value)| (key.clone(), xpc_value_to_json(value)))
                .collect(),
        ),
        ios_xpc::XpcValue::FileTransfer { msg_id, data } => serde_json::json!({
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
    use indexmap::IndexMap;
    use ios_xpc::XpcValue;

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
    fn dtx_decoder_reassembles_fragmented_messages() {
        let selector =
            ios_proto::nskeyedarchiver_encode::archive_string("_notifyOfPublishedCapabilities:");
        let encoded = crate::dtx::encode_dtx(1, 0, 0, true, 2, &selector, &[]);

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

        let selector = ios_proto::nskeyedarchiver_encode::archive_string("after-error");
        let encoded = crate::dtx::encode_dtx(2, 0, 0, true, 2, &selector, &[]);
        assert!(decoder
            .push(Direction::HostToDevice, &encoded)
            .unwrap()
            .is_empty());
        assert!(decoder.buffer.is_empty());
    }

    #[test]
    fn xpc_decoder_reassembles_messages_across_h2_frames() {
        let payload = ios_xpc::message::encode_message(&ios_xpc::XpcMessage {
            flags: ios_xpc::message::flags::ALWAYS_SET
                | ios_xpc::message::flags::DATA
                | ios_xpc::message::flags::REPLY,
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
    }

    #[test]
    fn xpc_decoder_skips_split_http2_client_preface() {
        let payload = ios_xpc::message::encode_message(&ios_xpc::XpcMessage {
            flags: ios_xpc::message::flags::ALWAYS_SET | ios_xpc::message::flags::DATA,
            msg_id: 9,
            body: Some(XpcValue::Dictionary(IndexMap::from([(
                "request".to_string(),
                XpcValue::String("ping".into()),
            )]))),
        })
        .unwrap();

        let mut decoder = StreamDecoder::new(ProxyProtocol::Xpc);
        let preface = ios_xpc::h2_raw::H2_PREFACE;
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
