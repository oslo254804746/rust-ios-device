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
#[cfg(feature = "mdns")]
use std::net::{Ipv6Addr, SocketAddr};

#[cfg(feature = "tunnel")]
use bytes::Bytes;
#[cfg(feature = "mdns")]
use tokio::net::TcpStream;

#[cfg(feature = "tunnel")]
use crate::xpc::h2_raw::H2Framer;
#[cfg(feature = "tunnel")]
use crate::xpc::message::{decode_message, flags, XpcMessage, XpcValue};
#[cfg(any(feature = "tunnel", feature = "mdns"))]
use crate::xpc::XpcError;

pub const RSD_PORT: u16 = 58783;

/// A discovered iOS 17+ service.
#[derive(Debug, Clone)]
pub struct ServiceDescriptor {
    pub port: u16,
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
}

/// Perform an RSD handshake with an iOS 17+ device.
///
/// `addr` is the device's tunnel IPv6 address (from CDTunnel handshake).
#[cfg(feature = "mdns")]
pub async fn handshake(addr: Ipv6Addr, port: u16) -> Result<RsdHandshake, XpcError> {
    let sock_addr = SocketAddr::new(addr.into(), port);
    let stream = TcpStream::connect(sock_addr).await?;
    let mut framer = H2Framer::connect(stream)
        .await
        .map_err(|e| XpcError::Tls(format!("H2: {e}")))?;

    read_rsd_handshake(&mut framer).await
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

    let msg2 = encode_message(&XpcMessage {
        flags: flags::INIT_HANDSHAKE | flags::ALWAYS_SET,
        msg_id: 0,
        body: None,
    })?;
    framer
        .write_server_client(&msg2)
        .await
        .map_err(|e| XpcError::Tls(format!("xpc init write 2: {e}")))?;
    discard_xpc_on_server_client(framer).await?;

    let msg3 = encode_message(&XpcMessage {
        flags: flags::ALWAYS_SET | 0x200,
        msg_id: 0,
        body: None,
    })?;
    framer
        .write_client_server(&msg3)
        .await
        .map_err(|e| XpcError::Tls(format!("xpc init write 3: {e}")))?;
    discard_xpc_on_client_server(framer).await?;

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
    let body_len = u64::from_le_bytes(
        header[8..16]
            .try_into()
            .map_err(|_| XpcError::Tls("bad header".into()))?,
    ) as usize;
    let body = if body_len > 0 {
        framer
            .read_client_server(body_len)
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
    let body_len = u64::from_le_bytes(
        header[8..16]
            .try_into()
            .map_err(|_| XpcError::Tls("bad header".into()))?,
    ) as usize;
    let body = if body_len > 0 {
        framer
            .read_server_client(body_len)
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
                    // Port can be a String or Uint64
                    let port = svc_dict.get("Port").and_then(|p| match p {
                        XpcValue::String(s) => s.parse::<u16>().ok(),
                        XpcValue::Uint64(n) => Some(*n as u16),
                        _ => None,
                    });
                    if let Some(port) = port {
                        services.insert(name.clone(), ServiceDescriptor { port });
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

/// A live XPC connection to an iOS 17+ service.
#[cfg(feature = "tunnel")]
pub struct XpcConnection<S> {
    framer: H2Framer<S>,
    msg_id: u64,
}

#[cfg(feature = "tunnel")]
impl<S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin> XpcConnection<S> {
    pub fn new(framer: H2Framer<S>) -> Self {
        Self { framer, msg_id: 1 }
    }

    fn next_id(&mut self) -> u64 {
        let id = self.msg_id;
        self.msg_id += 1;
        id
    }

    /// Send a dictionary as an XPC message on the clientServer stream.
    pub async fn send(&mut self, body: XpcValue) -> Result<(), XpcError> {
        self.send_with_flags(body, 0).await
    }

    /// Send a dictionary as an XPC message on the clientServer stream with
    /// additional wrapper flags.
    pub async fn send_with_flags(
        &mut self,
        body: XpcValue,
        extra_flags: u32,
    ) -> Result<(), XpcError> {
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
            .map_err(|e| XpcError::Tls(e.to_string()))
    }

    /// Receive one XPC message from the serverClient stream.
    pub async fn recv(&mut self) -> Result<XpcMessage, XpcError> {
        let header = self
            .framer
            .read_server_client(24)
            .await
            .map_err(|e| XpcError::Tls(e.to_string()))?;
        let body_len = u64::from_le_bytes(
            header[8..16]
                .try_into()
                .map_err(|_| XpcError::Tls("invalid header bytes".into()))?,
        ) as usize;
        let body = if body_len > 0 {
            self.framer
                .read_server_client(body_len)
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
}

#[cfg(test)]
#[cfg(feature = "tunnel")]
mod tests {
    use bytes::Bytes;
    use indexmap::IndexMap;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
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

    fn sample_handshake_xpc_message(message_type: Option<&str>) -> XpcMessage {
        let mut properties = IndexMap::new();
        properties.insert(
            "UniqueDeviceID".to_string(),
            XpcValue::String("00008150-00013DD00104401C".into()),
        );

        let mut service = IndexMap::new();
        service.insert("Port".to_string(), XpcValue::String("12345".into()));

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

            let mut sc_headers = [0u8; 9];
            server.read_exact(&mut sc_headers).await.unwrap();
            assert_eq!(sc_headers, headers_frame(STREAM_SERVER_CLIENT).as_slice());

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
