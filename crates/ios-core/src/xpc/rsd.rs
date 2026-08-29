//! RSD (Remote Service Discovery) client for iOS 17+.
//!
//! Protocol:
//! 1. TCP connect to [server_address]:58783
//! 2. Raw HTTP/2 handshake (preface + SETTINGS exchange)
//! 3. Read XPC handshake message on clientServer stream (stream 1)
//!    containing UDID + Services
//!
//! The device sends the handshake immediately after the H2 SETTINGS exchange;
//! do not send the usual XPC initialization sequence on the RSD port.
//!
//! Reference: go-ios/ios/rsd.go + go-ios/ios/http/http.go

use std::collections::HashMap;
#[cfg(feature = "tunnel")]
use std::collections::VecDeque;
#[cfg(feature = "tunnel")]
use std::mem::size_of;
#[cfg(all(feature = "tunnel", feature = "mdns"))]
use std::net::{Ipv6Addr, SocketAddr};

#[cfg(feature = "tunnel")]
use bytes::{Bytes, BytesMut};
#[cfg(all(feature = "tunnel", feature = "mdns"))]
use tokio::net::TcpStream;

#[cfg(feature = "tunnel")]
use crate::xpc::h2_raw::{H2Error, H2Framer, MAX_BUFFERED_BYTES_PER_STREAM, MAX_FRAME_PAYLOAD};
#[cfg(feature = "tunnel")]
use crate::xpc::message::{
    checked_xpc_body_len, decode_message, flags, xpc_body_limit_for_flags, XpcMessage, XpcValue,
};
#[cfg(feature = "tunnel")]
use crate::xpc::XpcError;

pub const RSD_PORT: u16 = 58783;

/// A discovered iOS 17+ service and its advertised capabilities.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServiceDescriptor {
    pub port: u16,
    /// Feature identifiers advertised under `Properties.Features`.
    ///
    /// An empty list means the device did not advertise capability details; it
    /// must not be interpreted as a deny-all list.
    pub features: Vec<String>,
}

impl ServiceDescriptor {
    pub fn new(port: u16) -> Self {
        Self {
            port,
            features: Vec::new(),
        }
    }

    /// Whether the service can be treated as supporting `feature`.
    ///
    /// Missing capability metadata is permissive because older devices and
    /// some transports omit `Properties.Features` entirely.
    pub fn supports_feature(&self, feature: &str) -> bool {
        self.features.is_empty() || self.features.iter().any(|item| item == feature)
    }
}

/// Result of the RSD handshake.
#[derive(Debug, Clone)]
pub struct RsdHandshake {
    pub udid: String,
    pub services: HashMap<String, ServiceDescriptor>,
}

impl RsdHandshake {
    /// Look up a service port, with automatic `.shim.remote` fallback.
    pub fn get_port(&self, service: &str) -> Option<u16> {
        if let Some(s) = self.services.get(service) {
            return Some(s.port);
        }
        let shim = format!("{service}.shim.remote");
        self.services.get(&shim).map(|s| s.port)
    }

    /// Return features explicitly advertised for an exact service name.
    pub fn get_service_features(&self, service: &str) -> Option<&[String]> {
        self.services
            .get(service)
            .map(|descriptor| descriptor.features.as_slice())
    }

    /// Return features for a service, resolving the `.shim.remote` entry used
    /// by some RSD handshakes when the canonical service name is absent.
    pub fn get_resolved_service_features(&self, service: &str) -> Option<&[String]> {
        if let Some(features) = self.get_service_features(service) {
            return Some(features);
        }
        let shim = format!("{service}.shim.remote");
        self.services
            .get(&shim)
            .map(|descriptor| descriptor.features.as_slice())
    }

    /// Check a feature against an exact service entry.
    ///
    /// Returns `None` when the service is absent. For a present service whose
    /// handshake entry has no feature list, returns `Some(true)` so missing
    /// metadata is not mistaken for an explicit device-side rejection.
    pub fn supports_feature(&self, service: &str, feature: &str) -> Option<bool> {
        self.services
            .get(service)
            .map(|descriptor| descriptor.supports_feature(feature))
    }
}

/// Perform an RSD handshake with an iOS 17+ device.
///
/// `addr` is the device's tunnel IPv6 address (from CDTunnel handshake).
#[cfg(all(feature = "tunnel", feature = "mdns"))]
pub async fn handshake(addr: Ipv6Addr, port: u16) -> Result<RsdHandshake, XpcError> {
    let sock_addr = SocketAddr::new(addr.into(), port);
    let stream = tokio::time::timeout(
        crate::tunnel::TUNNEL_CONNECT_TIMEOUT,
        TcpStream::connect(sock_addr),
    )
    .await
    .map_err(|_| {
        std::io::Error::new(
            std::io::ErrorKind::TimedOut,
            format!("RSD dial to {sock_addr} timed out"),
        )
    })??;
    tokio::time::timeout(crate::tunnel::TUNNEL_CONNECT_TIMEOUT, async {
        let mut framer = H2Framer::connect(stream)
            .await
            .map_err(|e| XpcError::Tls(format!("H2: {e}")))?;
        read_rsd_handshake(&mut framer).await
    })
    .await
    .map_err(|_| {
        std::io::Error::new(
            std::io::ErrorKind::TimedOut,
            format!("RSD handshake with {sock_addr} timed out"),
        )
    })?
}

/// Perform an RSD handshake on an already-connected H2 framer.
/// Used by ios-core's `attempt_rsd_via_proxy`.
#[cfg(feature = "tunnel")]
pub async fn handshake_on_framer<S>(framer: &mut H2Framer<S>) -> Result<RsdHandshake, XpcError>
where
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin,
{
    read_rsd_handshake(framer).await
}

/// Initialize the XPC connection using go-ios's 3-message bootstrap.
///
/// Some devices appear to withhold the RSD handshake until these stream
/// bootstrapping messages have been exchanged.
#[cfg(feature = "tunnel")]
pub async fn initialize_xpc_connection_on_framer<S>(
    framer: &mut H2Framer<S>,
) -> Result<(), XpcError>
where
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin,
{
    use crate::xpc::message::{encode_message, flags, XpcMessage, XpcValue};

    let msg1 = encode_message(&XpcMessage {
        flags: flags::ALWAYS_SET,
        msg_id: 0,
        body: Some(XpcValue::Dictionary(indexmap::IndexMap::new())),
    })?;
    framer
        .write_client_server(&msg1)
        .await
        .map_err(|e| XpcError::Tls(format!("xpc init write 1: {e}")))?;
    discard_xpc_on_client_server(framer).await?;

    let msg3 = encode_message(&XpcMessage {
        flags: flags::ALWAYS_SET | 0x200,
        msg_id: 0,
        body: None,
    })?;
    framer
        // remoted requires stream 3's HEADERS before the terminating stream-1
        // bootstrap message (the order used by pymobiledevice3).
        .open_stream(crate::xpc::h2_raw::STREAM_SERVER_CLIENT)
        .await
        .map_err(|e| XpcError::Tls(format!("xpc init open stream 3: {e}")))?;
    framer
        .write_client_server(&msg3)
        .await
        .map_err(|e| XpcError::Tls(format!("xpc init write 2: {e}")))?;
    discard_xpc_on_client_server(framer).await?;

    let msg2 = encode_message(&XpcMessage {
        flags: flags::INIT_HANDSHAKE | flags::ALWAYS_SET,
        msg_id: 0,
        body: None,
    })?;
    framer
        .write_server_client(&msg2)
        .await
        .map_err(|e| XpcError::Tls(format!("xpc init write 3: {e}")))?;
    discard_xpc_on_server_client(framer).await?;

    Ok(())
}

/// Queue the minimal RemoteXPC bootstrap used by pymobiledevice3 before it
/// reads the first RSD handshake message from stream 1.
#[cfg(feature = "tunnel")]
pub async fn queue_rsd_handshake_bootstrap_on_framer<S>(
    framer: &mut H2Framer<S>,
) -> Result<(), XpcError>
where
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin,
{
    use crate::xpc::message::{encode_message, flags, XpcMessage, XpcValue};

    let msg1 = encode_message(&XpcMessage {
        flags: flags::ALWAYS_SET,
        msg_id: 0,
        body: Some(XpcValue::Dictionary(indexmap::IndexMap::new())),
    })?;
    framer
        .write_client_server(&msg1)
        .await
        .map_err(|e| XpcError::Tls(format!("rsd bootstrap write 1: {e}")))?;

    // Open stream 3 before the terminating stream-1 bootstrap message. This
    // ordering is significant to remoted and matches RemoteXPC clients:
    // HEADERS#1, DATA#1(init), HEADERS#3, DATA#1(term), DATA#3(init).
    framer
        .open_stream(crate::xpc::h2_raw::STREAM_SERVER_CLIENT)
        .await
        .map_err(|e| XpcError::Tls(format!("rsd bootstrap open stream 3: {e}")))?;

    let msg2 = encode_message(&XpcMessage {
        flags: flags::ALWAYS_SET | 0x200,
        msg_id: 0,
        body: None,
    })?;
    framer
        .write_client_server(&msg2)
        .await
        .map_err(|e| XpcError::Tls(format!("rsd bootstrap write 2: {e}")))?;

    let msg3 = encode_message(&XpcMessage {
        flags: flags::INIT_HANDSHAKE | flags::ALWAYS_SET,
        msg_id: 0,
        body: None,
    })?;
    framer
        .write_server_client(&msg3)
        .await
        .map_err(|e| XpcError::Tls(format!("rsd bootstrap write 3: {e}")))?;

    Ok(())
}

/// Read the RSD handshake message from clientServer stream (stream 1).
///
/// The device sends the handshake immediately after the H2 connection is
/// established — no XPC initialization is needed on the RSD port.
/// go-ios reads this via `ReceiveOnClientServerStream()` (rsd.go:208).
#[cfg(feature = "tunnel")]
async fn read_rsd_handshake<S>(framer: &mut H2Framer<S>) -> Result<RsdHandshake, XpcError>
where
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin,
{
    let mut last_err = None;
    for _ in 0..6 {
        let msg = read_xpc_on_client_server(framer).await?;
        match parse_handshake_message(msg) {
            Ok(handshake) => return Ok(handshake),
            Err(err) => {
                tracing::debug!("RSD: skipping non-handshake stream-1 message: {err}");
                last_err = Some(err);
            }
        }
    }
    Err(last_err.unwrap_or_else(|| XpcError::Tls("RSD: no handshake message received".into())))
}

#[cfg(feature = "tunnel")]
async fn read_xpc_on_client_server<S>(framer: &mut H2Framer<S>) -> Result<XpcMessage, XpcError>
where
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin,
{
    let (header, body) = read_raw_xpc_on_client_server(framer).await?;
    let mut full = bytes::BytesMut::new();
    full.extend_from_slice(&header);
    full.extend_from_slice(&body);
    decode_message(full.freeze())
}

#[cfg(feature = "tunnel")]
async fn discard_xpc_on_client_server<S>(framer: &mut H2Framer<S>) -> Result<(), XpcError>
where
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin,
{
    let _ = read_raw_xpc_on_client_server(framer).await?;
    Ok(())
}

#[cfg(feature = "tunnel")]
async fn discard_xpc_on_server_client<S>(framer: &mut H2Framer<S>) -> Result<(), XpcError>
where
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin,
{
    let _ = read_raw_xpc_on_server_client(framer).await?;
    Ok(())
}

/// Read an XPC body without asking the H2 demultiplexer to retain the whole
/// message at once.  H2 intentionally caps a buffered stream at 16 MiB, while
/// an explicit FILE_TX body is allowed up to 64 MiB by the XPC layer.  Consume
/// one H2-sized chunk at a time so that limit remains useful for control
/// traffic and unknown streams while valid file/data messages can use their
/// larger, already-validated XPC limit.
#[cfg(feature = "tunnel")]
async fn read_xpc_body_in_chunks<S>(
    framer: &mut H2Framer<S>,
    stream_id: u32,
    body_len: usize,
) -> Result<Bytes, H2Error>
where
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin,
{
    // A DATA frame can overshoot read_stream's requested length by one frame.
    // Leave that much headroom so bytes left over after the 24-byte XPC header
    // cannot make an otherwise exact 16 MiB read trip the H2 buffer limit.
    const MAX_XPC_BODY_READ_CHUNK: usize = MAX_BUFFERED_BYTES_PER_STREAM - MAX_FRAME_PAYLOAD;
    let mut body = BytesMut::with_capacity(body_len.min(MAX_XPC_BODY_READ_CHUNK));
    let mut remaining = body_len;
    while remaining != 0 {
        let chunk_len = remaining.min(MAX_XPC_BODY_READ_CHUNK);
        let chunk = framer.read_stream(stream_id, chunk_len).await?;
        body.extend_from_slice(&chunk);
        remaining -= chunk_len;
    }
    Ok(body.freeze())
}

#[cfg(feature = "tunnel")]
async fn read_raw_xpc_on_client_server<S>(
    framer: &mut H2Framer<S>,
) -> Result<(Bytes, Bytes), XpcError>
where
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin,
{
    let header = framer
        .read_client_server(24)
        .await
        .map_err(|e| XpcError::Tls(format!("read header: {e}")))?;
    let declared_body_len = u64::from_le_bytes(
        header[8..16]
            .try_into()
            .map_err(|_| XpcError::Tls("bad header".into()))?,
    );
    let message_flags = u32::from_le_bytes(
        header[4..8]
            .try_into()
            .map_err(|_| XpcError::Tls("bad header flags".into()))?,
    );
    let body_len = checked_xpc_body_len(declared_body_len, xpc_body_limit_for_flags(message_flags))
        .map_err(XpcError::Tls)?;
    let body = if body_len > 0 {
        read_xpc_body_in_chunks(framer, crate::xpc::h2_raw::STREAM_CLIENT_SERVER, body_len)
            .await
            .map_err(|e| XpcError::Tls(format!("read body: {e}")))?
    } else {
        Bytes::new()
    };
    Ok((header, body))
}

#[cfg(feature = "tunnel")]
async fn read_raw_xpc_on_server_client<S>(
    framer: &mut H2Framer<S>,
) -> Result<(Bytes, Bytes), XpcError>
where
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin,
{
    let header = framer
        .read_server_client(24)
        .await
        .map_err(|e| XpcError::Tls(format!("read header: {e}")))?;
    let declared_body_len = u64::from_le_bytes(
        header[8..16]
            .try_into()
            .map_err(|_| XpcError::Tls("bad header".into()))?,
    );
    let message_flags = u32::from_le_bytes(
        header[4..8]
            .try_into()
            .map_err(|_| XpcError::Tls("bad header flags".into()))?,
    );
    let body_len = checked_xpc_body_len(declared_body_len, xpc_body_limit_for_flags(message_flags))
        .map_err(XpcError::Tls)?;
    let body = if body_len > 0 {
        read_xpc_body_in_chunks(framer, crate::xpc::h2_raw::STREAM_SERVER_CLIENT, body_len)
            .await
            .map_err(|e| XpcError::Tls(format!("read body: {e}")))?
    } else {
        Bytes::new()
    };
    Ok((header, body))
}

#[cfg(feature = "tunnel")]
fn parse_handshake_message(msg: XpcMessage) -> Result<RsdHandshake, XpcError> {
    let dict = msg
        .body
        .as_ref()
        .and_then(|b| b.as_dict())
        .ok_or_else(|| XpcError::Tls("RSD: expected XPC dict body".into()))?;
    let message_type = dict
        .get("MessageType")
        .and_then(|v| v.as_str())
        .ok_or_else(|| XpcError::Tls("RSD: missing Handshake MessageType".into()))?;
    if message_type != "Handshake" {
        return Err(XpcError::Tls(format!(
            "RSD: unexpected MessageType {message_type:?}"
        )));
    }
    // UDID
    let udid = dict
        .get("Properties")
        .and_then(|v| v.as_dict())
        .and_then(|d| d.get("UniqueDeviceID"))
        .and_then(|v| v.as_str())
        .ok_or_else(|| XpcError::Tls("RSD: missing UniqueDeviceID".into()))?
        .to_string();

    // Services
    let mut services = HashMap::new();
    match dict.get("Services") {
        Some(XpcValue::Dictionary(svc_map)) => {
            tracing::debug!(
                "RSD handshake for {} exposed {} services",
                udid,
                svc_map.len()
            );
            for (name, svc_val) in svc_map {
                if let Some(svc_dict) = svc_val.as_dict() {
                    // Port can be a String or Uint64. Reject out-of-range
                    // integers rather than truncating them into a different
                    // service port.
                    let port = svc_dict.get("Port").and_then(|p| match p {
                        XpcValue::String(s) => s.parse::<u16>().ok(),
                        XpcValue::Uint64(n) => u16::try_from(*n).ok(),
                        _ => None,
                    });
                    if let Some(port) = port {
                        let features = svc_dict
                            .get("Properties")
                            .and_then(XpcValue::as_dict)
                            .and_then(|properties| properties.get("Features"))
                            .and_then(|features| match features {
                                XpcValue::Array(features) => Some(features),
                                _ => None,
                            })
                            .map(|features| {
                                features
                                    .iter()
                                    .filter_map(XpcValue::as_str)
                                    .map(str::to_owned)
                                    .collect()
                            })
                            .unwrap_or_default();
                        services.insert(name.clone(), ServiceDescriptor { port, features });
                    } else {
                        tracing::debug!("RSD service {name:?} has an invalid or missing Port");
                    }
                }
            }
        }
        Some(other) => {
            tracing::debug!("RSD Services has unexpected type: {:?}", other);
        }
        None => {
            tracing::debug!("RSD handshake missing Services key");
        }
    }

    Ok(RsdHandshake { udid, services })
}

/// Ceiling on unmatched messages parked per stream while awaiting a reply id.
///
/// [`XpcConnection::recv_reply_on_stream`] holds on to every message that is not
/// the reply it wants so a later `recv` can still see it. A device that never
/// sends the awaited id would otherwise keep that queue growing for the lifetime
/// of the connection, so an overlong backlog is treated as a protocol fault
/// instead of being absorbed silently.
#[cfg(feature = "tunnel")]
const MAX_PENDING_MESSAGES_PER_STREAM: usize = 1024;

/// A pending XPC reply may be as large as the data-stream body limit. Keep
/// enough room for one such reply plus ordinary metadata on each stream, while
/// bounding aggregate retention when several streams are waiting concurrently.
/// These limits apply only to messages parked while matching a reply; the H2
/// framer's separate 8/64 MiB body limits continue to govern wire reads.
#[cfg(feature = "tunnel")]
const DEFAULT_PENDING_BYTES_PER_STREAM: usize = 128 * 1024 * 1024;
#[cfg(feature = "tunnel")]
const DEFAULT_PENDING_BYTES_CONNECTION: usize = 512 * 1024 * 1024;

#[cfg(feature = "tunnel")]
struct PendingMessage {
    message: XpcMessage,
    bytes: usize,
}

/// Count the allocations retained by an already-decoded XPC message.
///
/// Container capacities are used instead of lengths because decoded values
/// retain their backing allocations. The inline enum/message fields are not
/// counted recursively; vector/map slots and boxed file-transfer values are
/// counted where they introduce an allocation.
#[cfg(feature = "tunnel")]
fn add_pending_size(size: &mut usize, amount: usize, kind: &str) -> Result<(), XpcError> {
    *size = size.checked_add(amount).ok_or_else(|| {
        XpcError::Tls(format!(
            "XPC pending message size overflow while accounting {kind}"
        ))
    })?;
    Ok(())
}

#[cfg(feature = "tunnel")]
fn xpc_value_memory_size(value: &XpcValue) -> Result<usize, XpcError> {
    let mut size = 0usize;

    match value {
        XpcValue::Data(data) => add_pending_size(&mut size, data.len(), "data")?,
        XpcValue::String(string) => add_pending_size(&mut size, string.capacity(), "string")?,
        XpcValue::Array(values) => {
            add_pending_size(
                &mut size,
                values
                    .capacity()
                    .checked_mul(size_of::<XpcValue>())
                    .ok_or_else(|| {
                        XpcError::Tls(
                            "XPC pending message size overflow while accounting array slots".into(),
                        )
                    })?,
                "array slots",
            )?;
            for child in values {
                add_pending_size(&mut size, xpc_value_memory_size(child)?, "array values")?;
            }
        }
        XpcValue::Dictionary(entries) => {
            add_pending_size(
                &mut size,
                entries
                    .capacity()
                    .checked_mul(size_of::<(String, XpcValue)>())
                    .ok_or_else(|| {
                        XpcError::Tls(
                            "XPC pending message size overflow while accounting dictionary slots"
                                .into(),
                        )
                    })?,
                "dictionary slots",
            )?;
            for (key, child) in entries {
                add_pending_size(&mut size, key.capacity(), "dictionary key")?;
                add_pending_size(
                    &mut size,
                    xpc_value_memory_size(child)?,
                    "dictionary values",
                )?;
            }
        }
        XpcValue::FileTransfer { data, .. } => {
            add_pending_size(&mut size, size_of::<XpcValue>(), "file-transfer value")?;
            size = size
                .checked_add(xpc_value_memory_size(data)?)
                .ok_or_else(|| {
                    XpcError::Tls(
                        "XPC pending message size overflow while accounting file-transfer data"
                            .into(),
                    )
                })?;
        }
        XpcValue::Null
        | XpcValue::Bool(_)
        | XpcValue::Int64(_)
        | XpcValue::Uint64(_)
        | XpcValue::Double(_)
        | XpcValue::Date(_)
        | XpcValue::Uuid(_) => {}
    }

    Ok(size)
}

#[cfg(feature = "tunnel")]
fn xpc_message_memory_size(message: &XpcMessage) -> Result<usize, XpcError> {
    size_of::<XpcMessage>()
        .checked_add(
            message
                .body
                .as_ref()
                .map(xpc_value_memory_size)
                .transpose()?
                .unwrap_or_default(),
        )
        .ok_or_else(|| XpcError::Tls("XPC pending message size overflow".into()))
}

#[cfg(feature = "tunnel")]
struct PendingMessageBudget {
    per_stream_limit: usize,
    connection_limit: usize,
    per_stream_bytes: HashMap<u32, usize>,
    total_bytes: usize,
}

#[cfg(feature = "tunnel")]
impl PendingMessageBudget {
    fn new(per_stream_limit: usize, connection_limit: usize) -> Self {
        Self {
            per_stream_limit,
            connection_limit,
            per_stream_bytes: HashMap::new(),
            total_bytes: 0,
        }
    }

    fn reserve(&mut self, stream_id: u32, bytes: usize) -> Result<(), XpcError> {
        let current_stream = self.per_stream_bytes.get(&stream_id).copied().unwrap_or(0);
        let next_stream = current_stream.checked_add(bytes).ok_or_else(|| {
            XpcError::Tls(format!(
                "XPC pending bytes overflow on stream {stream_id}: current {current_stream}, incoming {bytes}"
            ))
        })?;
        let next_total = self.total_bytes.checked_add(bytes).ok_or_else(|| {
            XpcError::Tls(format!(
                "XPC pending connection bytes overflow: current {}, incoming {bytes}",
                self.total_bytes
            ))
        })?;
        if next_stream > self.per_stream_limit {
            return Err(XpcError::Tls(format!(
                "XPC pending bytes {next_stream} on stream {stream_id} exceed per-stream limit {} (current {current_stream}, incoming {bytes})",
                self.per_stream_limit
            )));
        }
        if next_total > self.connection_limit {
            return Err(XpcError::Tls(format!(
                "XPC pending connection bytes {next_total} exceed limit {} (current {}, incoming {bytes})",
                self.connection_limit, self.total_bytes
            )));
        }

        self.per_stream_bytes.insert(stream_id, next_stream);
        self.total_bytes = next_total;
        Ok(())
    }

    fn replace(
        &mut self,
        stream_id: u32,
        old_bytes: usize,
        new_bytes: usize,
    ) -> Result<(), XpcError> {
        let current_stream = self.per_stream_bytes.get(&stream_id).copied().unwrap_or(0);
        let base_stream = current_stream.checked_sub(old_bytes).ok_or_else(|| {
            XpcError::Tls(format!(
                "XPC pending byte accounting underflow on stream {stream_id}: current {current_stream}, replacing {old_bytes}"
            ))
        })?;
        let base_total = self.total_bytes.checked_sub(old_bytes).ok_or_else(|| {
            XpcError::Tls(format!(
                "XPC pending connection byte accounting underflow: current {}, replacing {old_bytes}",
                self.total_bytes
            ))
        })?;
        let next_stream = base_stream.checked_add(new_bytes).ok_or_else(|| {
            XpcError::Tls(format!(
                "XPC pending bytes overflow on stream {stream_id}: base {base_stream}, replacement {new_bytes}"
            ))
        })?;
        let next_total = base_total.checked_add(new_bytes).ok_or_else(|| {
            XpcError::Tls(format!(
                "XPC pending connection bytes overflow: base {base_total}, replacement {new_bytes}"
            ))
        })?;
        if next_stream > self.per_stream_limit {
            return Err(XpcError::Tls(format!(
                "XPC replacement pending bytes {next_stream} on stream {stream_id} exceed per-stream limit {} (old {old_bytes}, new {new_bytes})",
                self.per_stream_limit
            )));
        }
        if next_total > self.connection_limit {
            return Err(XpcError::Tls(format!(
                "XPC replacement pending connection bytes {next_total} exceed limit {} (old {old_bytes}, new {new_bytes})",
                self.connection_limit
            )));
        }

        if next_stream == 0 {
            self.per_stream_bytes.remove(&stream_id);
        } else {
            self.per_stream_bytes.insert(stream_id, next_stream);
        }
        self.total_bytes = next_total;
        Ok(())
    }

    fn release(&mut self, stream_id: u32, bytes: usize) {
        if let Some(current_stream) = self.per_stream_bytes.get(&stream_id).copied() {
            debug_assert!(current_stream >= bytes);
            let remaining = current_stream.saturating_sub(bytes);
            if remaining == 0 {
                self.per_stream_bytes.remove(&stream_id);
            } else {
                self.per_stream_bytes.insert(stream_id, remaining);
            }
        }
        debug_assert!(self.total_bytes >= bytes);
        self.total_bytes = self.total_bytes.saturating_sub(bytes);
    }

    fn clear_stream(&mut self, stream_id: u32) {
        if let Some(bytes) = self.per_stream_bytes.remove(&stream_id) {
            debug_assert!(self.total_bytes >= bytes);
            self.total_bytes = self.total_bytes.saturating_sub(bytes);
        }
    }

    fn clear(&mut self) {
        self.per_stream_bytes.clear();
        self.total_bytes = 0;
    }
}

/// A live XPC connection to an iOS 17+ service.
#[cfg(feature = "tunnel")]
pub struct XpcConnection<S> {
    framer: H2Framer<S>,
    msg_id: u64,
    pending_messages: HashMap<u32, VecDeque<PendingMessage>>,
    pending_budget: PendingMessageBudget,
}

#[cfg(feature = "tunnel")]
impl<S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin> XpcConnection<S> {
    pub fn new(framer: H2Framer<S>) -> Self {
        Self::with_pending_memory_limits(
            framer,
            DEFAULT_PENDING_BYTES_PER_STREAM,
            DEFAULT_PENDING_BYTES_CONNECTION,
        )
    }

    /// Construct a connection with explicit pending-reply memory limits.
    ///
    /// This is useful for constrained embedders and deterministic tests. A
    /// normal connection should use [`Self::new`], whose limits are sized for
    /// the XPC control/data body limits.
    pub fn with_pending_memory_limits(
        framer: H2Framer<S>,
        per_stream_limit: usize,
        connection_limit: usize,
    ) -> Self {
        Self {
            framer,
            msg_id: 1,
            pending_messages: HashMap::new(),
            pending_budget: PendingMessageBudget::new(per_stream_limit, connection_limit),
        }
    }

    /// Number of bytes currently retained by unmatched replies.
    #[allow(dead_code)]
    pub fn pending_memory_bytes_used(&self) -> usize {
        self.pending_budget.total_bytes
    }

    fn next_id(&mut self) -> u64 {
        let id = self.msg_id;
        self.msg_id += 1;
        id
    }

    /// Send a dictionary as an XPC message on the clientServer stream.
    pub async fn send(&mut self, body: XpcValue) -> Result<(), XpcError> {
        self.send_with_flags(body, 0).await.map(|_| ())
    }

    /// Send a dictionary as an XPC message on the clientServer stream with
    /// additional wrapper flags.
    pub async fn send_with_flags(
        &mut self,
        body: XpcValue,
        extra_flags: u32,
    ) -> Result<u64, XpcError> {
        let id = self.next_id();
        let msg = XpcMessage {
            flags: flags::ALWAYS_SET | flags::DATA | extra_flags,
            msg_id: id,
            body: Some(body),
        };
        let bytes = crate::xpc::message::encode_message(&msg)?;
        self.framer
            .write_client_server(&bytes)
            .await
            .map_err(|e| XpcError::Tls(e.to_string()))?;
        Ok(id)
    }

    /// Receive one XPC message from the serverClient stream.
    pub async fn recv(&mut self) -> Result<XpcMessage, XpcError> {
        self.recv_on_stream(crate::xpc::h2_raw::STREAM_SERVER_CLIENT)
            .await
    }

    /// Receive one XPC message from the clientServer stream.
    pub async fn recv_client_server(&mut self) -> Result<XpcMessage, XpcError> {
        self.recv_on_stream(crate::xpc::h2_raw::STREAM_CLIENT_SERVER)
            .await
    }

    async fn recv_on_stream(&mut self, stream_id: u32) -> Result<XpcMessage, XpcError> {
        if let Some(message) = self.pop_next_pending_message(stream_id) {
            return Ok(message);
        }
        self.recv_fresh_on_stream(stream_id).await
    }

    /// Receive the reply with a specific XPC message id from one stream.
    pub async fn recv_reply_on_stream(
        &mut self,
        stream_id: u32,
        msg_id: u64,
    ) -> Result<XpcMessage, XpcError> {
        if let Some(message) = self.take_pending_message(stream_id, msg_id) {
            return Ok(message);
        }

        loop {
            let message = self.recv_fresh_on_stream(stream_id).await?;
            if message.msg_id == msg_id {
                return Ok(message);
            }
            if let Err(err) = self.push_pending_message(stream_id, message) {
                self.clear_pending_stream(stream_id);
                return Err(err);
            }
        }
    }

    async fn recv_fresh_on_stream(&mut self, stream_id: u32) -> Result<XpcMessage, XpcError> {
        let result = self.recv_fresh_on_stream_inner(stream_id).await;
        if result.is_err() {
            self.clear_pending_stream(stream_id);
        }
        result
    }

    async fn recv_fresh_on_stream_inner(&mut self, stream_id: u32) -> Result<XpcMessage, XpcError> {
        let header = self
            .framer
            .read_stream(stream_id, 24)
            .await
            .map_err(|e| XpcError::Tls(e.to_string()))?;
        let declared_body_len = u64::from_le_bytes(
            header[8..16]
                .try_into()
                .map_err(|_| XpcError::Tls("invalid header bytes".into()))?,
        );
        let message_flags = u32::from_le_bytes(
            header[4..8]
                .try_into()
                .map_err(|_| XpcError::Tls("invalid header flags".into()))?,
        );
        let body_len =
            checked_xpc_body_len(declared_body_len, xpc_body_limit_for_flags(message_flags))
                .map_err(XpcError::Tls)?;
        let body = if body_len > 0 {
            read_xpc_body_in_chunks(&mut self.framer, stream_id, body_len)
                .await
                .map_err(|e| XpcError::Tls(e.to_string()))?
        } else {
            Bytes::new()
        };
        let mut full = bytes::BytesMut::new();
        full.extend_from_slice(&header);
        full.extend_from_slice(&body);
        decode_message(full.freeze())
    }

    fn push_pending_message(
        &mut self,
        stream_id: u32,
        message: XpcMessage,
    ) -> Result<(), XpcError> {
        let bytes = xpc_message_memory_size(&message)?;
        let replacement_index = if message.msg_id == 0 {
            None
        } else {
            self.pending_messages.get(&stream_id).and_then(|pending| {
                pending
                    .iter()
                    .position(|entry| entry.message.msg_id == message.msg_id)
            })
        };

        if let Some(index) = replacement_index {
            return self.replace_pending_message(stream_id, index, message, bytes);
        }

        let pending_len = self
            .pending_messages
            .get(&stream_id)
            .map_or(0, VecDeque::len);
        if pending_len >= MAX_PENDING_MESSAGES_PER_STREAM {
            return Err(XpcError::Tls(format!(
                "XPC: more than {MAX_PENDING_MESSAGES_PER_STREAM} unmatched messages \
                 buffered on stream {stream_id} while waiting for a reply"
            )));
        }

        self.pending_budget.reserve(stream_id, bytes)?;
        self.pending_messages
            .entry(stream_id)
            .or_default()
            .push_back(PendingMessage { message, bytes });
        Ok(())
    }

    fn replace_pending_message(
        &mut self,
        stream_id: u32,
        index: usize,
        message: XpcMessage,
        bytes: usize,
    ) -> Result<(), XpcError> {
        let old_bytes = self
            .pending_messages
            .get(&stream_id)
            .and_then(|pending| pending.get(index))
            .map(|entry| entry.bytes)
            .ok_or_else(|| {
                XpcError::Tls(format!(
                    "XPC pending message replacement index {index} missing on stream {stream_id}"
                ))
            })?;
        self.pending_budget.replace(stream_id, old_bytes, bytes)?;

        if let Some(entry) = self
            .pending_messages
            .get_mut(&stream_id)
            .and_then(|pending| pending.get_mut(index))
        {
            *entry = PendingMessage { message, bytes };
            return Ok(());
        }

        // Keep the accounting consistent even if an internal queue invariant
        // is ever violated between the two lookups above.
        let _ = self.pending_budget.replace(stream_id, bytes, old_bytes);
        Err(XpcError::Tls(format!(
            "XPC pending message replacement index {index} disappeared on stream {stream_id}"
        )))
    }

    fn pop_next_pending_message(&mut self, stream_id: u32) -> Option<XpcMessage> {
        let (message, bytes, empty) = {
            let pending = self.pending_messages.get_mut(&stream_id)?;
            let entry = pending.pop_front()?;
            (entry.message, entry.bytes, pending.is_empty())
        };
        if empty {
            self.pending_messages.remove(&stream_id);
        }
        self.pending_budget.release(stream_id, bytes);
        Some(message)
    }

    fn take_pending_message(&mut self, stream_id: u32, msg_id: u64) -> Option<XpcMessage> {
        let (message, bytes, empty) = {
            let pending = self.pending_messages.get_mut(&stream_id)?;
            let index = pending
                .iter()
                .position(|entry| entry.message.msg_id == msg_id)?;
            let entry = pending.remove(index)?;
            (entry.message, entry.bytes, pending.is_empty())
        };
        if empty {
            self.pending_messages.remove(&stream_id);
        }
        self.pending_budget.release(stream_id, bytes);
        Some(message)
    }

    fn clear_pending_stream(&mut self, stream_id: u32) {
        self.pending_messages.remove(&stream_id);
        self.pending_budget.clear_stream(stream_id);
    }
}

#[cfg(feature = "tunnel")]
impl<S> Drop for XpcConnection<S> {
    fn drop(&mut self) {
        self.pending_messages.clear();
        self.pending_budget.clear();
    }
}

#[cfg(test)]
#[cfg(feature = "tunnel")]
mod tests {
    use std::pin::Pin;
    use std::task::{Context, Poll};

    use bytes::Bytes;
    use indexmap::IndexMap;
    use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt, ReadBuf};
    use tokio::time::{timeout, Duration};

    use super::*;
    use crate::xpc::message::{encode_message, flags, XpcMessage, XpcValue};

    const FRAME_DATA: u8 = 0x00;
    const FRAME_HEADERS: u8 = 0x01;
    const FRAME_SETTINGS: u8 = 0x04;
    const FLAG_END_HEADERS: u8 = 0x04;
    const FLAG_SETTINGS_ACK: u8 = 0x01;
    const STREAM_INIT: u32 = 0;
    const STREAM_CLIENT_SERVER: u32 = 1;
    const STREAM_SERVER_CLIENT: u32 = 3;

    fn build_frame(frame_type: u8, flags: u8, stream_id: u32, payload: &[u8]) -> Vec<u8> {
        let len = payload.len();
        let mut out = Vec::with_capacity(9 + len);
        out.push(((len >> 16) & 0xFF) as u8);
        out.push(((len >> 8) & 0xFF) as u8);
        out.push((len & 0xFF) as u8);
        out.push(frame_type);
        out.push(flags);
        out.extend_from_slice(&(stream_id & 0x7FFF_FFFF).to_be_bytes());
        out.extend_from_slice(payload);
        out
    }

    fn settings_frame() -> Vec<u8> {
        build_frame(FRAME_SETTINGS, 0, STREAM_INIT, &[])
    }

    fn settings_ack_frame() -> Vec<u8> {
        build_frame(FRAME_SETTINGS, FLAG_SETTINGS_ACK, STREAM_INIT, &[])
    }

    fn headers_frame(stream_id: u32) -> Vec<u8> {
        build_frame(FRAME_HEADERS, FLAG_END_HEADERS, stream_id, &[])
    }

    fn data_frame(stream_id: u32, payload: &[u8]) -> Vec<u8> {
        build_frame(FRAME_DATA, 0, stream_id, payload)
    }

    fn pending_data_message(msg_id: u64, len: usize) -> XpcMessage {
        XpcMessage {
            flags: flags::ALWAYS_SET,
            msg_id,
            body: Some(XpcValue::Data(Bytes::from(vec![0xA5; len]))),
        }
    }

    async fn connected_framer_for_pending_test() -> H2Framer<tokio::io::DuplexStream> {
        let (client, mut server) = tokio::io::duplex(4096);
        let server_task = tokio::spawn(async move {
            let mut preface = [0u8; 24];
            server.read_exact(&mut preface).await.unwrap();
            let mut settings = [0u8; 21];
            server.read_exact(&mut settings).await.unwrap();
            let mut window_update = [0u8; 13];
            server.read_exact(&mut window_update).await.unwrap();
            server.write_all(&settings_frame()).await.unwrap();
            server.flush().await.unwrap();
            let mut ack = [0u8; 9];
            server.read_exact(&mut ack).await.unwrap();
        });

        let framer = H2Framer::connect(client).await.unwrap();
        server_task.await.unwrap();
        framer
    }

    struct ScriptedIo {
        input: Bytes,
        offset: usize,
        output: Vec<u8>,
    }

    impl AsyncRead for ScriptedIo {
        fn poll_read(
            mut self: Pin<&mut Self>,
            _cx: &mut Context<'_>,
            buf: &mut ReadBuf<'_>,
        ) -> Poll<std::io::Result<()>> {
            let available = self.input.len().saturating_sub(self.offset);
            if available == 0 {
                return Poll::Ready(Ok(()));
            }
            let count = available.min(buf.remaining());
            let end = self.offset + count;
            buf.put_slice(&self.input[self.offset..end]);
            self.offset = end;
            Poll::Ready(Ok(()))
        }
    }

    impl AsyncWrite for ScriptedIo {
        fn poll_write(
            mut self: Pin<&mut Self>,
            _cx: &mut Context<'_>,
            buf: &[u8],
        ) -> Poll<std::io::Result<usize>> {
            self.output.extend_from_slice(buf);
            Poll::Ready(Ok(buf.len()))
        }

        fn poll_flush(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
            Poll::Ready(Ok(()))
        }

        fn poll_shutdown(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
            Poll::Ready(Ok(()))
        }
    }

    fn scripted_file_transfer_wire(body_len: usize) -> Bytes {
        let mut xpc = vec![0xA5; 24 + body_len];
        xpc[4..8].copy_from_slice(&flags::FILE_TX_STREAM_RESPONSE.to_le_bytes());
        xpc[8..16].copy_from_slice(&(body_len as u64).to_le_bytes());

        let mut wire = settings_frame();
        wire.reserve(xpc.len() + (xpc.len() / MAX_FRAME_PAYLOAD + 1) * 9);
        for chunk in xpc.chunks(MAX_FRAME_PAYLOAD) {
            wire.extend_from_slice(&data_frame(STREAM_SERVER_CLIENT, chunk));
        }
        Bytes::from(wire)
    }

    #[tokio::test]
    async fn file_transfer_body_can_cross_h2_buffer_boundary_in_chunks() {
        for body_len in [
            MAX_BUFFERED_BYTES_PER_STREAM + 1,
            crate::xpc::message::XPC_DATA_STREAM_BODY_LIMIT,
        ] {
            let wire = scripted_file_transfer_wire(body_len);
            let mut framer = H2Framer::connect(ScriptedIo {
                input: wire,
                offset: 0,
                output: Vec::new(),
            })
            .await
            .unwrap();

            let (header, body) = read_raw_xpc_on_server_client(&mut framer).await.unwrap();
            let declared = u64::from_le_bytes(header[8..16].try_into().unwrap());
            let flags = u32::from_le_bytes(header[4..8].try_into().unwrap());
            let checked = checked_xpc_body_len(declared, xpc_body_limit_for_flags(flags)).unwrap();
            assert_eq!(checked, body_len);
            assert_eq!(body.len(), body_len);
            assert_eq!(body.first(), Some(&0xA5));
            assert_eq!(body.last(), Some(&0xA5));
        }
    }

    fn sample_handshake_xpc_message(message_type: Option<&str>) -> XpcMessage {
        let mut properties = IndexMap::new();
        properties.insert(
            "UniqueDeviceID".to_string(),
            XpcValue::String("00008150-00013DD00104401C".into()),
        );

        let mut service = IndexMap::new();
        service.insert("Port".to_string(), XpcValue::String("12345".into()));
        service.insert(
            "Properties".to_string(),
            XpcValue::Dictionary(IndexMap::from([(
                "Features".to_string(),
                XpcValue::Array(vec![
                    XpcValue::String("com.apple.coredevice.feature.one".into()),
                    XpcValue::String("com.apple.coredevice.feature.two".into()),
                ]),
            )])),
        );

        let mut services = IndexMap::new();
        services.insert(
            "com.apple.instruments.dtservicehub".to_string(),
            XpcValue::Dictionary(service),
        );

        let mut body = IndexMap::new();
        if let Some(message_type) = message_type {
            body.insert(
                "MessageType".to_string(),
                XpcValue::String(message_type.into()),
            );
        }
        body.insert("Properties".to_string(), XpcValue::Dictionary(properties));
        body.insert("Services".to_string(), XpcValue::Dictionary(services));

        XpcMessage {
            flags: flags::ALWAYS_SET | flags::DATA,
            msg_id: 0,
            body: Some(XpcValue::Dictionary(body)),
        }
    }

    fn sample_handshake_message() -> Bytes {
        encode_message(&sample_handshake_xpc_message(Some("Handshake")))
            .expect("synthetic RSD message should encode")
    }

    #[test]
    fn parse_handshake_message_rejects_missing_or_wrong_message_type() {
        let missing = parse_handshake_message(sample_handshake_xpc_message(None));
        assert!(missing.is_err());

        let wrong = parse_handshake_message(sample_handshake_xpc_message(Some("NotHandshake")));
        assert!(wrong.is_err());
    }

    #[test]
    fn parse_handshake_message_accepts_valid_handshake() {
        let handshake =
            parse_handshake_message(sample_handshake_xpc_message(Some("Handshake"))).unwrap();

        assert_eq!(handshake.udid, "00008150-00013DD00104401C");
        assert_eq!(
            handshake.get_port("com.apple.instruments.dtservicehub"),
            Some(12345)
        );
        assert_eq!(
            handshake
                .get_service_features("com.apple.instruments.dtservicehub")
                .unwrap(),
            [
                "com.apple.coredevice.feature.one",
                "com.apple.coredevice.feature.two"
            ]
        );
        assert_eq!(
            handshake.supports_feature(
                "com.apple.instruments.dtservicehub",
                "com.apple.coredevice.feature.two"
            ),
            Some(true)
        );
        assert_eq!(
            handshake.supports_feature(
                "com.apple.instruments.dtservicehub",
                "com.apple.coredevice.feature.missing"
            ),
            Some(false)
        );
    }

    #[test]
    fn service_descriptor_without_feature_metadata_is_permissive() {
        let descriptor = ServiceDescriptor::new(12345);
        assert!(descriptor.supports_feature("com.apple.coredevice.feature.future"));
    }

    #[test]
    fn resolved_service_features_follow_shim_fallback() {
        let mut descriptor = ServiceDescriptor::new(12345);
        descriptor.features = vec!["com.apple.coredevice.feature.streamapplist".into()];
        let handshake = RsdHandshake {
            udid: "test-udid".into(),
            services: HashMap::from([(
                "com.apple.coredevice.appservice.shim.remote".into(),
                descriptor,
            )]),
        };

        assert_eq!(
            handshake.get_resolved_service_features("com.apple.coredevice.appservice"),
            Some(["com.apple.coredevice.feature.streamapplist".to_string()].as_slice())
        );
    }

    #[test]
    fn resolved_service_features_prefer_canonical_entry() {
        let mut canonical = ServiceDescriptor::new(1111);
        canonical.features = vec!["canonical-feature".into()];
        let mut shim = ServiceDescriptor::new(2222);
        shim.features = vec!["shim-feature".into()];
        let handshake = RsdHandshake {
            udid: "test-udid".into(),
            services: HashMap::from([
                ("com.apple.coredevice.appservice".into(), canonical),
                ("com.apple.coredevice.appservice.shim.remote".into(), shim),
            ]),
        };

        assert_eq!(
            handshake.get_port("com.apple.coredevice.appservice"),
            Some(1111)
        );
        assert_eq!(
            handshake.get_resolved_service_features("com.apple.coredevice.appservice"),
            Some(["canonical-feature".to_string()].as_slice())
        );
    }

    #[test]
    fn parse_handshake_message_rejects_out_of_range_service_port() {
        let mut message = sample_handshake_xpc_message(Some("Handshake"));
        let services = message
            .body
            .as_mut()
            .and_then(|body| match body {
                XpcValue::Dictionary(body) => body.get_mut("Services"),
                _ => None,
            })
            .and_then(|services| match services {
                XpcValue::Dictionary(services) => Some(services),
                _ => None,
            })
            .unwrap();
        let service = services
            .get_mut("com.apple.instruments.dtservicehub")
            .and_then(|service| match service {
                XpcValue::Dictionary(service) => Some(service),
                _ => None,
            })
            .unwrap();
        service.insert("Port".into(), XpcValue::Uint64(u16::MAX as u64 + 1));

        let handshake = parse_handshake_message(message).unwrap();
        assert!(handshake.services.is_empty());
    }

    #[tokio::test]
    async fn handshake_on_framer_reads_stream_1_without_xpc_init() {
        let (client, mut server) = tokio::io::duplex(4096);

        let server_task = tokio::spawn(async move {
            let mut preface = [0u8; 24];
            server.read_exact(&mut preface).await.unwrap();
            assert_eq!(&preface, crate::xpc::h2_raw::H2_PREFACE);

            let mut settings = [0u8; 21];
            server.read_exact(&mut settings).await.unwrap();
            assert_eq!(settings[3], FRAME_SETTINGS);

            let mut window_update = [0u8; 13];
            server.read_exact(&mut window_update).await.unwrap();
            assert_eq!(window_update[3], 0x08);

            server.write_all(&settings_frame()).await.unwrap();
            server.flush().await.unwrap();

            let mut ack = [0u8; 9];
            server.read_exact(&mut ack).await.unwrap();
            assert_eq!(ack, settings_ack_frame().as_slice());

            // The RSD port should not receive the usual XPC init traffic.
            assert!(timeout(Duration::from_millis(100), async {
                let mut extra = [0u8; 1];
                server.read_exact(&mut extra).await
            })
            .await
            .is_err());

            server
                .write_all(&headers_frame(STREAM_CLIENT_SERVER))
                .await
                .unwrap();
            server
                .write_all(&headers_frame(STREAM_SERVER_CLIENT))
                .await
                .unwrap();
            server
                .write_all(&data_frame(
                    STREAM_CLIENT_SERVER,
                    &sample_handshake_message(),
                ))
                .await
                .unwrap();
            server.flush().await.unwrap();
        });

        let mut framer = H2Framer::connect(client).await.unwrap();
        let handshake = timeout(Duration::from_secs(1), handshake_on_framer(&mut framer))
            .await
            .expect("handshake timed out")
            .unwrap();

        assert_eq!(handshake.udid, "00008150-00013DD00104401C");
        assert_eq!(
            handshake.get_port("com.apple.instruments.dtservicehub"),
            Some(12345)
        );

        server_task.await.unwrap();
    }

    #[tokio::test]
    async fn initialize_xpc_connection_consumes_step_responses_in_reference_order() {
        let (client, mut server) = tokio::io::duplex(4096);

        let empty = encode_message(&XpcMessage {
            flags: flags::ALWAYS_SET,
            msg_id: 0,
            body: None,
        })
        .unwrap();

        let server_task = tokio::spawn(async move {
            let mut preface = [0u8; 24];
            server.read_exact(&mut preface).await.unwrap();
            assert_eq!(&preface, crate::xpc::h2_raw::H2_PREFACE);

            let mut settings = [0u8; 21];
            server.read_exact(&mut settings).await.unwrap();
            assert_eq!(settings[3], FRAME_SETTINGS);

            let mut window_update = [0u8; 13];
            server.read_exact(&mut window_update).await.unwrap();
            assert_eq!(window_update[3], 0x08);

            server.write_all(&settings_frame()).await.unwrap();
            server.flush().await.unwrap();

            let mut ack = [0u8; 9];
            server.read_exact(&mut ack).await.unwrap();
            assert_eq!(ack, settings_ack_frame().as_slice());

            let mut cs_headers = [0u8; 9];
            server.read_exact(&mut cs_headers).await.unwrap();
            assert_eq!(cs_headers, headers_frame(STREAM_CLIENT_SERVER).as_slice());

            let mut cs_msg1_header = [0u8; 9];
            server.read_exact(&mut cs_msg1_header).await.unwrap();
            assert_eq!(cs_msg1_header[3], FRAME_DATA);
            assert_eq!(
                u32::from_be_bytes([
                    cs_msg1_header[5] & 0x7F,
                    cs_msg1_header[6],
                    cs_msg1_header[7],
                    cs_msg1_header[8]
                ]),
                STREAM_CLIENT_SERVER
            );
            let msg1_len = ((cs_msg1_header[0] as usize) << 16)
                | ((cs_msg1_header[1] as usize) << 8)
                | (cs_msg1_header[2] as usize);
            let mut cs_msg1 = vec![0u8; msg1_len];
            server.read_exact(&mut cs_msg1).await.unwrap();

            server
                .write_all(&data_frame(STREAM_CLIENT_SERVER, &empty))
                .await
                .unwrap();
            server.flush().await.unwrap();

            let mut sc_headers = [0u8; 9];
            server.read_exact(&mut sc_headers).await.unwrap();
            assert_eq!(sc_headers, headers_frame(STREAM_SERVER_CLIENT).as_slice());

            let mut cs_msg3_header = [0u8; 9];
            server.read_exact(&mut cs_msg3_header).await.unwrap();
            assert_eq!(cs_msg3_header[3], FRAME_DATA);
            assert_eq!(
                u32::from_be_bytes([
                    cs_msg3_header[5] & 0x7F,
                    cs_msg3_header[6],
                    cs_msg3_header[7],
                    cs_msg3_header[8]
                ]),
                STREAM_CLIENT_SERVER
            );
            let msg3_len = ((cs_msg3_header[0] as usize) << 16)
                | ((cs_msg3_header[1] as usize) << 8)
                | (cs_msg3_header[2] as usize);
            let mut cs_msg3 = vec![0u8; msg3_len];
            server.read_exact(&mut cs_msg3).await.unwrap();

            server
                .write_all(&data_frame(STREAM_CLIENT_SERVER, &empty))
                .await
                .unwrap();
            server.flush().await.unwrap();

            let mut sc_msg2_header = [0u8; 9];
            server.read_exact(&mut sc_msg2_header).await.unwrap();
            assert_eq!(sc_msg2_header[3], FRAME_DATA);
            assert_eq!(
                u32::from_be_bytes([
                    sc_msg2_header[5] & 0x7F,
                    sc_msg2_header[6],
                    sc_msg2_header[7],
                    sc_msg2_header[8]
                ]),
                STREAM_SERVER_CLIENT
            );
            let msg2_len = ((sc_msg2_header[0] as usize) << 16)
                | ((sc_msg2_header[1] as usize) << 8)
                | (sc_msg2_header[2] as usize);
            let mut sc_msg2 = vec![0u8; msg2_len];
            server.read_exact(&mut sc_msg2).await.unwrap();

            server
                .write_all(&data_frame(STREAM_SERVER_CLIENT, &empty))
                .await
                .unwrap();
            server.flush().await.unwrap();
        });

        let mut framer = H2Framer::connect(client).await.unwrap();
        timeout(
            Duration::from_secs(1),
            initialize_xpc_connection_on_framer(&mut framer),
        )
        .await
        .expect("bootstrap timed out")
        .unwrap();

        server_task.await.unwrap();
    }

    #[tokio::test]
    async fn recv_reply_on_stream_rejects_an_unbounded_pending_backlog() {
        let (client, mut server) = tokio::io::duplex(64 * 1024);

        let server_task = tokio::spawn(async move {
            let mut preface = [0u8; 24];
            server.read_exact(&mut preface).await.unwrap();

            let mut settings = [0u8; 21];
            server.read_exact(&mut settings).await.unwrap();

            let mut window_update = [0u8; 13];
            server.read_exact(&mut window_update).await.unwrap();

            server.write_all(&settings_frame()).await.unwrap();
            server.flush().await.unwrap();

            let mut ack = [0u8; 9];
            server.read_exact(&mut ack).await.unwrap();

            server
                .write_all(&headers_frame(STREAM_SERVER_CLIENT))
                .await
                .unwrap();

            // Every reply carries an id the caller is not waiting for, so each one
            // is parked instead of returned.
            for id in 0..=MAX_PENDING_MESSAGES_PER_STREAM as u64 {
                let stray = encode_message(&XpcMessage {
                    flags: flags::ALWAYS_SET,
                    msg_id: 7 + id,
                    body: None,
                })
                .unwrap();
                server
                    .write_all(&data_frame(STREAM_SERVER_CLIENT, &stray))
                    .await
                    .unwrap();
            }
            server.flush().await.unwrap();
        });

        let framer = H2Framer::connect(client).await.unwrap();
        let mut connection = XpcConnection::new(framer);
        let err = timeout(
            Duration::from_secs(5),
            connection.recv_reply_on_stream(STREAM_SERVER_CLIENT, u64::MAX),
        )
        .await
        .expect("the cap should trip instead of blocking forever")
        .unwrap_err();

        assert!(err.to_string().contains("unmatched messages"), "{err}");
        assert_eq!(connection.pending_memory_bytes_used(), 0);

        server_task.await.unwrap();
    }

    #[test]
    fn pending_budget_rejects_overflow_without_mutating_exact_usage() {
        let message = pending_data_message(1, 8);
        let bytes = xpc_message_memory_size(&message).unwrap();
        let mut budget = PendingMessageBudget::new(bytes * 2, bytes * 2);

        budget.reserve(3, bytes).unwrap();
        budget.reserve(5, bytes).unwrap();
        assert_eq!(budget.total_bytes, bytes * 2);
        assert!(budget.reserve(3, 1).is_err());
        assert_eq!(budget.total_bytes, bytes * 2);

        budget.release(3, bytes);
        assert_eq!(budget.total_bytes, bytes);
        budget.release(5, bytes);
        assert_eq!(budget.total_bytes, 0);
    }

    #[tokio::test]
    async fn pending_messages_account_for_replacement_pop_and_stream_close() {
        let framer = connected_framer_for_pending_test().await;
        let first = pending_data_message(7, 8);
        let first_bytes = xpc_message_memory_size(&first).unwrap();
        let replacement = pending_data_message(7, 24);
        let replacement_bytes = xpc_message_memory_size(&replacement).unwrap();
        let mut connection = XpcConnection::with_pending_memory_limits(
            framer,
            replacement_bytes,
            replacement_bytes * 2,
        );

        connection.push_pending_message(3, first).unwrap();
        assert_eq!(connection.pending_memory_bytes_used(), first_bytes);
        connection
            .push_pending_message(3, replacement)
            .expect("same nonzero reply id should replace its parked value");
        assert_eq!(connection.pending_memory_bytes_used(), replacement_bytes);

        assert!(connection
            .push_pending_message(3, pending_data_message(8, 1))
            .is_err());
        assert_eq!(
            connection.pending_memory_bytes_used(),
            replacement_bytes,
            "rejected insert must not consume budget"
        );

        let popped = connection.pop_next_pending_message(3).unwrap();
        assert_eq!(popped.msg_id, 7);
        assert_eq!(connection.pending_memory_bytes_used(), 0);

        let stream_three = pending_data_message(9, 8);
        let stream_three_bytes = xpc_message_memory_size(&stream_three).unwrap();
        let stream_five = pending_data_message(10, 12);
        let stream_five_bytes = xpc_message_memory_size(&stream_five).unwrap();
        connection.push_pending_message(3, stream_three).unwrap();
        connection.push_pending_message(5, stream_five).unwrap();
        assert_eq!(
            connection.pending_memory_bytes_used(),
            stream_three_bytes + stream_five_bytes
        );

        connection.clear_pending_stream(3);
        assert_eq!(connection.pending_memory_bytes_used(), stream_five_bytes);
        connection.clear_pending_stream(5);
        assert_eq!(connection.pending_memory_bytes_used(), 0);
    }

    #[tokio::test]
    async fn queue_rsd_handshake_bootstrap_matches_pymobiledevice3_order() {
        let (client, mut server) = tokio::io::duplex(4096);

        let server_task = tokio::spawn(async move {
            let mut preface = [0u8; 24];
            server.read_exact(&mut preface).await.unwrap();
            assert_eq!(&preface, crate::xpc::h2_raw::H2_PREFACE);

            let mut settings = [0u8; 21];
            server.read_exact(&mut settings).await.unwrap();
            assert_eq!(settings[3], FRAME_SETTINGS);

            let mut window_update = [0u8; 13];
            server.read_exact(&mut window_update).await.unwrap();
            assert_eq!(window_update[3], 0x08);

            server.write_all(&settings_frame()).await.unwrap();
            server.flush().await.unwrap();

            let mut ack = [0u8; 9];
            server.read_exact(&mut ack).await.unwrap();
            assert_eq!(ack, settings_ack_frame().as_slice());

            let mut cs_headers = [0u8; 9];
            server.read_exact(&mut cs_headers).await.unwrap();
            assert_eq!(cs_headers, headers_frame(STREAM_CLIENT_SERVER).as_slice());

            let mut cs_msg1_header = [0u8; 9];
            server.read_exact(&mut cs_msg1_header).await.unwrap();
            assert_eq!(cs_msg1_header[3], FRAME_DATA);
            let cs_msg1_len = ((cs_msg1_header[0] as usize) << 16)
                | ((cs_msg1_header[1] as usize) << 8)
                | (cs_msg1_header[2] as usize);
            let mut cs_msg1 = vec![0u8; cs_msg1_len];
            server.read_exact(&mut cs_msg1).await.unwrap();
            let decoded1 = decode_message(Bytes::from(cs_msg1)).unwrap();
            assert_eq!(decoded1.flags, flags::ALWAYS_SET);
            assert_eq!(
                decoded1.body,
                Some(XpcValue::Dictionary(IndexMap::<String, XpcValue>::new()))
            );

            let mut sc_headers = [0u8; 9];
            server.read_exact(&mut sc_headers).await.unwrap();
            assert_eq!(sc_headers, headers_frame(STREAM_SERVER_CLIENT).as_slice());

            let mut cs_msg2_header = [0u8; 9];
            server.read_exact(&mut cs_msg2_header).await.unwrap();
            assert_eq!(cs_msg2_header[3], FRAME_DATA);
            let cs_msg2_len = ((cs_msg2_header[0] as usize) << 16)
                | ((cs_msg2_header[1] as usize) << 8)
                | (cs_msg2_header[2] as usize);
            let mut cs_msg2 = vec![0u8; cs_msg2_len];
            server.read_exact(&mut cs_msg2).await.unwrap();
            let decoded2 = decode_message(Bytes::from(cs_msg2)).unwrap();
            assert_eq!(decoded2.flags, flags::ALWAYS_SET | 0x200);
            assert!(decoded2.body.is_none());

            let mut sc_msg3_header = [0u8; 9];
            server.read_exact(&mut sc_msg3_header).await.unwrap();
            assert_eq!(sc_msg3_header[3], FRAME_DATA);
            let sc_msg3_len = ((sc_msg3_header[0] as usize) << 16)
                | ((sc_msg3_header[1] as usize) << 8)
                | (sc_msg3_header[2] as usize);
            let mut sc_msg3 = vec![0u8; sc_msg3_len];
            server.read_exact(&mut sc_msg3).await.unwrap();
            let decoded3 = decode_message(Bytes::from(sc_msg3)).unwrap();
            assert_eq!(decoded3.flags, flags::INIT_HANDSHAKE | flags::ALWAYS_SET);
            assert!(decoded3.body.is_none());
        });

        let mut framer = H2Framer::connect(client).await.unwrap();
        timeout(
            Duration::from_secs(1),
            queue_rsd_handshake_bootstrap_on_framer(&mut framer),
        )
        .await
        .expect("queued bootstrap timed out")
        .unwrap();

        server_task.await.unwrap();
    }
}
