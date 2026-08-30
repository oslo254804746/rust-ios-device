//! CoreDevice display media negotiation and bounded RTP access-unit transport.
//!
//! This is the protocol layer used by Apple's `displayservice`.  It deliberately
//! exposes encoded RTP/HEVC/AAC data, rather than pretending to decode pixels or
//! audio.  The VNC/pixel decoder remains a consumer-level concern.

use std::io::Write;
use std::net::{IpAddr, SocketAddr};
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::Arc;
use std::time::Duration;

use bytes::{Bytes, BytesMut};
use flate2::{write::ZlibEncoder, Compression};
use indexmap::IndexMap;
use tokio::net::UdpSocket;
use tokio::task::JoinHandle;

use crate::device::ResolvedServiceMetadata;
use crate::services::coredevice::{
    build_request_with_action, parse_output, CoreDeviceEnvelopeMode,
};
use crate::xpc::{XpcClient, XpcError, XpcValue};

pub const SERVICE_NAME: &str = "com.apple.coredevice.displayservice";
pub const MEDIA_SUPPORT_FEATURE: &str = "com.apple.coredevice.feature.getmediasupportinfo";
pub const MEDIA_STATUS_FEATURE: &str = "com.apple.coredevice.feature.getmediastreamserverstatus";
pub const START_MEDIA_FEATURE: &str = "com.apple.coredevice.feature.startmediastream";
pub const STOP_MEDIA_FEATURE: &str = "com.apple.coredevice.feature.stopmediastream";
pub const MEDIA_SUPPORT_ACTION: &str = "com.apple.coredevice.action.mediastreamgetsupportinfo";
pub const MEDIA_STATUS_ACTION: &str = "com.apple.coredevice.action.mediastreamstatus";
pub const START_MEDIA_ACTION: &str = "com.apple.coredevice.action.mediastreamstart";
pub const STOP_MEDIA_ACTION: &str = "com.apple.coredevice.action.mediastreamstop";
pub const FEATURE_GET_MEDIA_SUPPORT_INFO: &str = MEDIA_SUPPORT_FEATURE;
pub const FEATURE_GET_MEDIA_STREAM_SERVER_STATUS: &str = MEDIA_STATUS_FEATURE;
pub const FEATURE_START_MEDIA_STREAM: &str = START_MEDIA_FEATURE;
pub const FEATURE_STOP_MEDIA_STREAM: &str = STOP_MEDIA_FEATURE;
pub const ACTION_MEDIA_STREAM_GET_SUPPORT_INFO: &str = MEDIA_SUPPORT_ACTION;
pub const ACTION_MEDIA_STREAM_STATUS: &str = MEDIA_STATUS_ACTION;
pub const ACTION_MEDIA_STREAM_START: &str = START_MEDIA_ACTION;
pub const ACTION_MEDIA_STREAM_STOP: &str = STOP_MEDIA_ACTION;
pub const CLIENT_SUPPORTED_FEATURES: u64 = 140;
/// There is no universally routable media endpoint. Callers must provide the
/// address assigned by the active CDTunnel/RSD connection rather than relying
/// on a loopback default that the device cannot reach.
pub const DEFAULT_RECEIVER_IP: &str = "";
pub const DEFAULT_ACCESS_NETWORK_TYPE: i64 = 1;
pub const DEFAULT_TRANSPORT_PROTOCOL_TYPE: i64 = 2;
pub const MAX_RTP_PACKET_BYTES: usize = 65_535;
pub const DEFAULT_MAX_ACCESS_UNIT_BYTES: usize = 16 * 1024 * 1024;
pub const DEFAULT_MAX_ACCESS_UNIT_PACKETS: usize = 4_096;

// Apple's DisplayService appends this fixed footer to the final fragment of
// some coded-slice NALs. It is not part of the Annex-B HEVC access unit; match
// the upstream receiver and strip only this exact observed suffix.
const DISPLAYSERVICE_NAL_TRAILER: &[u8] =
    b"\x04\xf0\x0a\xc0\x00\x00\x03\x00\x00\x04\xec\x0a\xb0\x03";

#[derive(Debug, thiserror::Error)]
pub enum DisplayError {
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
    #[error("xpc error: {0}")]
    Xpc(#[from] XpcError),
    #[error("display protocol error: {0}")]
    Protocol(String),
    #[error("display service requires the modern CoreDevice envelope")]
    LegacyUnsupported,
    #[error("display service does not advertise feature {0}")]
    FeatureMissing(&'static str),
    #[error("display operation timed out")]
    Timeout,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MediaKind {
    Video,
    Audio,
}

impl MediaKind {
    fn wire_name(self) -> &'static str {
        match self {
            Self::Video => "video",
            Self::Audio => "audio",
        }
    }
}

#[derive(Debug, Clone)]
pub struct MediaStreamOptions {
    /// Concrete host-side tunnel address on which the UDP receiver is bound
    /// and which is advertised to the device. Do not use loopback or an
    /// unspecified wildcard address.
    pub receiver_ip: String,
    /// Concrete device-side tunnel/RSD address used for RTCP reports.
    pub sender_ip: String,
    pub receiver_port: u16,
    pub timeout: Duration,
    pub display_id: u64,
    pub client_session_id: Option<uuid::Uuid>,
    pub allow_rtcp_feedback: bool,
    pub ltrp_enabled: bool,
    pub fec_enabled: bool,
    pub tiles_per_frame: u64,
    pub max_access_unit_bytes: usize,
    pub max_access_unit_packets: usize,
}

impl Default for MediaStreamOptions {
    fn default() -> Self {
        Self {
            // Keep the type's `Default` implementation useful for filling in
            // non-network options, but fail closed until the caller supplies
            // the two tunnel endpoints. `::1` is host-local and is not a
            // valid device media destination.
            receiver_ip: String::new(),
            sender_ip: String::new(),
            receiver_port: 0,
            timeout: Duration::from_secs(20),
            display_id: 1,
            client_session_id: None,
            allow_rtcp_feedback: false,
            ltrp_enabled: false,
            fec_enabled: true,
            tiles_per_frame: 1,
            max_access_unit_bytes: DEFAULT_MAX_ACCESS_UNIT_BYTES,
            max_access_unit_packets: DEFAULT_MAX_ACCESS_UNIT_PACKETS,
        }
    }
}

#[derive(Debug, Clone)]
pub struct MediaStreamStart {
    pub kind: MediaKind,
    pub client_session_id: uuid::Uuid,
    pub response: XpcValue,
    /// The device's RTCP source port from `connection.streamConfig.SourcePort`.
    /// This is the port a receiver report must target; it is not the local RTP
    /// receiver port advertised in the request.
    pub sender_port: Option<u16>,
    /// SSRC values as named from the device's perspective. `local_ssrc` is
    /// the host/receiver SSRC (`RemoteSSRC`), while `remote_ssrc` is the
    /// device/sender SSRC (`LocalSSRC`).
    pub local_ssrc: Option<u32>,
    pub remote_ssrc: Option<u32>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RawAccessUnit {
    pub kind: MediaKind,
    pub timestamp: u32,
    pub sequence_start: u16,
    pub sequence_end: u16,
    pub ssrc: u32,
    /// Encoded HEVC Annex-B NALs or raw AAC RTP payload bytes. Not decoded.
    pub data: Bytes,
}

pub struct DisplayServiceClient {
    client: XpcClient,
    envelope_mode: CoreDeviceEnvelopeMode,
    service_features: Option<Vec<String>>,
}

pub type DisplayService = DisplayServiceClient;

impl DisplayServiceClient {
    pub fn new(client: XpcClient) -> Self {
        Self::new_with_mode(client, CoreDeviceEnvelopeMode::Modern)
    }
    pub fn new_with_mode(client: XpcClient, mode: CoreDeviceEnvelopeMode) -> Self {
        Self {
            client,
            envelope_mode: mode,
            service_features: None,
        }
    }
    pub fn new_with_features<I, S>(client: XpcClient, features: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        Self {
            client,
            envelope_mode: CoreDeviceEnvelopeMode::Modern,
            service_features: Some(features.into_iter().map(Into::into).collect()),
        }
    }
    pub fn from_resolved_metadata(
        client: XpcClient,
        metadata: &ResolvedServiceMetadata,
    ) -> Result<Self, DisplayError> {
        if metadata.resolved_service_name.ends_with(".shim.remote") {
            return Err(DisplayError::Protocol(
                "displayservice shim requires RSDCheckin and cannot use RemoteXPC".into(),
            ));
        }
        Ok(Self::new_with_features(client, metadata.features.clone()))
    }
    pub fn supports_media(&self) -> bool {
        self.service_features.as_deref().map_or(true, |f| {
            f.is_empty() || f.iter().any(|x| x == START_MEDIA_FEATURE)
        })
    }
    fn ensure(&self, feature: &'static str) -> Result<(), DisplayError> {
        if self.envelope_mode == CoreDeviceEnvelopeMode::Legacy {
            return Err(DisplayError::LegacyUnsupported);
        }
        if self
            .service_features
            .as_deref()
            .is_some_and(|f| !f.is_empty() && !f.iter().any(|x| x == feature))
        {
            return Err(DisplayError::FeatureMissing(feature));
        }
        Ok(())
    }
    pub async fn get_media_support_info(&mut self) -> Result<XpcValue, DisplayError> {
        self.ensure(MEDIA_SUPPORT_FEATURE)?;
        self.invoke(MEDIA_SUPPORT_FEATURE, MEDIA_SUPPORT_ACTION, dict([]))
            .await
    }
    pub async fn get_media_stream_server_status(&mut self) -> Result<XpcValue, DisplayError> {
        self.ensure(MEDIA_STATUS_FEATURE)?;
        self.invoke(MEDIA_STATUS_FEATURE, MEDIA_STATUS_ACTION, dict([]))
            .await
    }
    pub async fn start_video_stream(
        &mut self,
        options: &MediaStreamOptions,
    ) -> Result<MediaStreamStart, DisplayError> {
        self.start_stream(MediaKind::Video, options).await
    }
    pub async fn start_audio_stream(
        &mut self,
        options: &MediaStreamOptions,
    ) -> Result<MediaStreamStart, DisplayError> {
        self.start_stream(MediaKind::Audio, options).await
    }
    async fn start_stream(
        &mut self,
        kind: MediaKind,
        options: &MediaStreamOptions,
    ) -> Result<MediaStreamStart, DisplayError> {
        self.ensure(START_MEDIA_FEATURE)?;
        if options.receiver_port == 0
            || options.timeout.is_zero()
            || options.timeout.as_secs() == 0
            || options.max_access_unit_bytes == 0
            || options.max_access_unit_packets == 0
        {
            return Err(DisplayError::Protocol(
                "invalid media stream options".into(),
            ));
        }
        let receiver: IpAddr = options
            .receiver_ip
            .parse()
            .map_err(|_| DisplayError::Protocol("receiver_ip is not an IP address".into()))?;
        let sender: IpAddr = options
            .sender_ip
            .parse()
            .map_err(|_| DisplayError::Protocol("sender_ip is not an IP address".into()))?;
        validate_media_endpoint(receiver, "receiver_ip")?;
        validate_media_endpoint(sender, "sender_ip")?;
        if receiver.is_ipv4() != sender.is_ipv4() {
            return Err(DisplayError::Protocol(
                "receiver_ip and sender_ip address families differ".into(),
            ));
        }
        if kind == MediaKind::Video && options.display_id > i64::MAX as u64 {
            return Err(DisplayError::Protocol(
                "display_id exceeds CoreDevice's signed integer range".into(),
            ));
        }
        let id = options.client_session_id.unwrap_or_else(uuid::Uuid::new_v4);
        let sid = rand::random::<u32>();
        let call_id = uuid::Uuid::new_v4().to_string().to_ascii_uppercase();
        let offer = build_negotiator_offer(kind, &call_id, sid, options)?;
        let input = build_start_input(kind, options, id, offer);
        let response = self
            .invoke_with_deadline(
                START_MEDIA_FEATURE,
                START_MEDIA_ACTION,
                input,
                options.timeout,
            )
            .await?;
        let connection = response
            .as_dict()
            .and_then(|d| d.get("connection"))
            .and_then(XpcValue::as_dict);
        let stream_config = connection
            .and_then(|d| d.get("streamConfig"))
            .and_then(XpcValue::as_dict);
        let sender_port = stream_config
            .map(|config| optional_u16(config, "SourcePort"))
            .transpose()?
            .flatten();
        let local_ssrc = stream_config
            .map(|config| optional_u32(config, "RemoteSSRC"))
            .transpose()?
            .flatten();
        let remote_ssrc = stream_config
            .map(|config| optional_u32(config, "LocalSSRC"))
            .transpose()?
            .flatten();
        Ok(MediaStreamStart {
            kind,
            client_session_id: id,
            response,
            sender_port,
            local_ssrc,
            remote_ssrc,
        })
    }
    pub async fn stop_media_stream(&mut self, id: uuid::Uuid) -> Result<XpcValue, DisplayError> {
        self.ensure(STOP_MEDIA_FEATURE)?;
        let result = self
            .invoke(
                STOP_MEDIA_FEATURE,
                STOP_MEDIA_ACTION,
                dict([("avcMediaStreamOptionClientSessionID", uuid_value(id))]),
            )
            .await;
        match result {
            Ok(value) => Ok(value),
            Err(DisplayError::Xpc(XpcError::Io(error)))
                if matches!(
                    error.kind(),
                    std::io::ErrorKind::UnexpectedEof
                        | std::io::ErrorKind::ConnectionReset
                        | std::io::ErrorKind::BrokenPipe
                        | std::io::ErrorKind::NotConnected
                ) =>
            {
                Ok(dict([("stopped", XpcValue::Bool(true))]))
            }
            Err(e) => Err(e),
        }
    }
    async fn invoke(
        &mut self,
        feature: &'static str,
        action: &'static str,
        input: XpcValue,
    ) -> Result<XpcValue, DisplayError> {
        self.invoke_with_deadline(feature, action, input, Duration::from_secs(60))
            .await
    }

    async fn invoke_with_deadline(
        &mut self,
        feature: &'static str,
        action: &'static str,
        input: XpcValue,
        deadline: Duration,
    ) -> Result<XpcValue, DisplayError> {
        let response = tokio::time::timeout(
            deadline,
            self.client
                .call(build_request_with_action("", feature, action, input)),
        )
        .await
        .map_err(|_| DisplayError::Timeout)??;
        parse_output(response).map_err(DisplayError::Protocol)
    }
}

pub struct MediaStreamSession {
    service: Option<DisplayServiceClient>,
    socket: Arc<UdpSocket>,
    kind: MediaKind,
    id: uuid::Uuid,
    assembler: RtpAssembler,
    sender: Option<SocketAddr>,
    local_ssrc: Option<u32>,
    remote_ssrc: Option<u32>,
    highest_sequence: Arc<AtomicU32>,
    highest_sequence_seen: Arc<AtomicBool>,
    rtcp_task: Option<JoinHandle<()>>,
    drain_task: Option<JoinHandle<()>>,
}

impl MediaStreamSession {
    pub async fn start_video(
        service: DisplayServiceClient,
        options: MediaStreamOptions,
    ) -> Result<Self, DisplayError> {
        Self::start(service, MediaKind::Video, options).await
    }
    pub async fn start_audio(
        service: DisplayServiceClient,
        options: MediaStreamOptions,
    ) -> Result<Self, DisplayError> {
        Self::start(service, MediaKind::Audio, options).await
    }
    async fn start(
        mut service: DisplayServiceClient,
        kind: MediaKind,
        options: MediaStreamOptions,
    ) -> Result<Self, DisplayError> {
        let ip: IpAddr = options
            .receiver_ip
            .parse()
            .map_err(|_| DisplayError::Protocol("receiver_ip is not an IP address".into()))?;
        validate_media_endpoint(ip, "receiver_ip")?;
        let socket = Arc::new(UdpSocket::bind(SocketAddr::new(ip, options.receiver_port)).await?);
        let port = socket.local_addr()?.port();
        let mut options = options;
        options.receiver_port = port;
        let start = match kind {
            MediaKind::Video => service.start_video_stream(&options).await?,
            MediaKind::Audio => service.start_audio_stream(&options).await?,
        };
        let sender = start.sender_port.map(|port| {
            options
                .sender_ip
                .parse()
                .map(|ip| SocketAddr::new(ip, port))
                .map_err(|_| DisplayError::Protocol("sender_ip is not an IP address".into()))
        });
        let sender = sender.transpose()?;
        let highest_sequence = Arc::new(AtomicU32::new(0));
        let highest_sequence_seen = Arc::new(AtomicBool::new(false));
        let rtcp_task = match (
            sender,
            start.sender_port,
            start.local_ssrc,
            start.remote_ssrc,
        ) {
            (Some(destination), Some(_port), Some(local_ssrc), Some(remote_ssrc)) => {
                let socket = Arc::clone(&socket);
                let highest_sequence = Arc::clone(&highest_sequence);
                let highest_sequence_seen = Arc::clone(&highest_sequence_seen);
                Some(tokio::spawn(async move {
                    let mut interval = tokio::time::interval(Duration::from_secs(1));
                    interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
                    loop {
                        interval.tick().await;
                        let highest_sequence = if highest_sequence_seen.load(Ordering::Relaxed) {
                            highest_sequence.load(Ordering::Relaxed)
                        } else {
                            0
                        };
                        let packet =
                            build_rtcp_receiver_report(local_ssrc, remote_ssrc, highest_sequence);
                        if socket.send_to(&packet, destination).await.is_err() {
                            break;
                        }
                    }
                }))
            }
            _ => None,
        };
        Ok(Self {
            service: Some(service),
            socket,
            kind,
            id: start.client_session_id,
            assembler: RtpAssembler::new(
                kind,
                options.max_access_unit_bytes,
                options.max_access_unit_packets,
            ),
            sender,
            local_ssrc: start.local_ssrc,
            remote_ssrc: start.remote_ssrc,
            highest_sequence,
            highest_sequence_seen,
            rtcp_task,
            drain_task: None,
        })
    }
    pub fn client_session_id(&self) -> uuid::Uuid {
        self.id
    }
    pub fn local_addr(&self) -> Result<SocketAddr, std::io::Error> {
        self.socket.local_addr()
    }
    /// Drain RTP/RTCP in the background when the stream is only being used as
    /// the HID authorization side channel. The task is aborted by `stop` or
    /// `Drop`; it never grows a queue or retains packet payloads.
    pub fn spawn_drain(&mut self) {
        if self.drain_task.is_some() {
            return;
        }
        let socket = Arc::clone(&self.socket);
        self.drain_task = Some(tokio::spawn(async move {
            let mut packet = vec![0u8; MAX_RTP_PACKET_BYTES];
            loop {
                if socket.recv(&mut packet).await.is_err() {
                    break;
                }
            }
        }));
    }
    pub async fn recv_access_unit(
        &mut self,
        timeout: Duration,
    ) -> Result<Option<RawAccessUnit>, DisplayError> {
        let mut packet = vec![0u8; MAX_RTP_PACKET_BYTES];
        let deadline = tokio::time::Instant::now()
            .checked_add(timeout)
            .ok_or_else(|| DisplayError::Protocol("RTP receive deadline overflow".into()))?;
        loop {
            let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
            if remaining.is_zero() {
                return Err(DisplayError::Timeout);
            }
            let n = tokio::time::timeout(remaining, self.socket.recv(&mut packet))
                .await
                .map_err(|_| DisplayError::Timeout)??;
            // RTCP packets use the same RTP version bit but their 8-bit
            // packet type maps to the reserved RTP payload-type range. A
            // minimal RTCP RR is only 8 bytes, so classify it before the
            // 12-byte RTP parser rather than terminating a valid stream.
            if is_rtcp_datagram(&packet[..n]) {
                continue;
            }
            if let Some(rtp) = RtpPacket::parse(&packet[..n])? {
                self.update_highest_sequence(rtp.sequence);
                if self.remote_ssrc.is_none() {
                    self.remote_ssrc = Some(rtp.ssrc);
                }
                if let Some(au) = self.assembler.push(rtp)? {
                    return Ok(Some(au));
                }
            }
        }
    }

    fn update_highest_sequence(&self, sequence: u16) {
        if !self.highest_sequence_seen.swap(true, Ordering::Relaxed) {
            self.highest_sequence
                .store(u32::from(sequence), Ordering::Relaxed);
            return;
        }
        let current = self.highest_sequence.load(Ordering::Relaxed);
        let candidate = advance_extended_sequence(current, sequence);
        if candidate != current {
            self.highest_sequence.store(candidate, Ordering::Relaxed);
        }
    }
    pub async fn stop(&mut self, timeout: Duration) -> Result<(), DisplayError> {
        if let Some(task) = self.rtcp_task.take() {
            task.abort();
        }
        if let Some(task) = self.drain_task.take() {
            task.abort();
        }
        let Some(mut service) = self.service.take() else {
            return Ok(());
        };
        tokio::time::timeout(timeout, service.stop_media_stream(self.id))
            .await
            .map_err(|_| DisplayError::Timeout)??;
        Ok(())
    }
    pub fn sender_addr(&self) -> Option<SocketAddr> {
        self.sender
    }
    /// Send a standards-compliant RTCP receiver report when the negotiated
    /// offer opted into feedback. It is intentionally small and bounded; the
    /// device's optional proprietary RCTL companion packets are not required
    /// for raw access-unit consumers.
    pub async fn send_receiver_report(&self) -> Result<(), DisplayError> {
        let (Some(destination), Some(local_ssrc), Some(remote_ssrc)) =
            (self.sender, self.local_ssrc, self.remote_ssrc)
        else {
            return Ok(());
        };
        let packet = build_rtcp_receiver_report(
            local_ssrc,
            remote_ssrc,
            self.highest_sequence.load(Ordering::Relaxed),
        );
        self.socket.send_to(&packet, destination).await?;
        Ok(())
    }
    /// Pixel decoding/VNC is intentionally not part of this API; callers consume raw units.
    pub fn media_kind(&self) -> MediaKind {
        self.kind
    }
}

impl Drop for MediaStreamSession {
    fn drop(&mut self) {
        if let Some(task) = self.rtcp_task.take() {
            task.abort();
        }
        if let Some(task) = self.drain_task.take() {
            task.abort();
        }
        self.service.take();
    }
}

fn build_rtcp_receiver_report(
    local_ssrc: u32,
    remote_ssrc: u32,
    highest_sequence: u32,
) -> [u8; 44] {
    let mut packet = [0u8; 44];
    // RFC 3550 receiver report with one report block. The device's
    // streamConfig names its own SSRC LocalSSRC and ours RemoteSSRC.
    packet[0] = 0x81; // V=2, RC=1
    packet[1] = 201; // PT=RR
    packet[2..4].copy_from_slice(&7u16.to_be_bytes());
    packet[4..8].copy_from_slice(&local_ssrc.to_be_bytes());
    packet[8..12].copy_from_slice(&remote_ssrc.to_be_bytes());
    // fraction/cumulative loss remain zero; this bounded receiver does not
    // maintain a loss history beyond the highest sequence number.
    packet[16..20].copy_from_slice(&highest_sequence.to_be_bytes());

    // Compound RTCP packets carry an SDES CNAME chunk. Xcode sends an empty
    // CNAME, which is sufficient for the CoreDevice media daemon and keeps
    // the report free of host-identifying data.
    packet[32] = 0x81; // V=2, one source chunk
    packet[33] = 202; // PT=SDES
    packet[34..36].copy_from_slice(&2u16.to_be_bytes());
    packet[36..40].copy_from_slice(&local_ssrc.to_be_bytes());
    packet[40] = 1; // CNAME item
    packet[41] = 0; // empty CNAME
                    // packet[42] = 0 terminates the item list; packet[43] is alignment.
    packet
}

#[derive(Debug, Clone, Copy)]
pub struct RtpPacket<'a> {
    pub marker: bool,
    pub payload_type: u8,
    pub sequence: u16,
    pub timestamp: u32,
    pub ssrc: u32,
    pub payload: &'a [u8],
}

impl<'a> RtpPacket<'a> {
    pub fn parse(packet: &'a [u8]) -> Result<Option<Self>, DisplayError> {
        if packet.len() < 4 {
            return Err(DisplayError::Protocol("RTP header truncated".into()));
        }
        if packet[0] >> 6 != 2 {
            return Err(DisplayError::Protocol("unsupported RTP version".into()));
        }
        if packet.len() < 12 {
            return Err(DisplayError::Protocol("RTP header truncated".into()));
        }
        let cc = usize::from(packet[0] & 0x0f);
        let mut header = 12usize
            .checked_add(
                cc.checked_mul(4)
                    .ok_or_else(|| DisplayError::Protocol("RTP CSRC overflow".into()))?,
            )
            .ok_or_else(|| DisplayError::Protocol("RTP header overflow".into()))?;
        if packet.len() < header {
            return Err(DisplayError::Protocol("RTP CSRC truncated".into()));
        }
        if packet[0] & 0x10 != 0 {
            if packet.len() < header + 4 {
                return Err(DisplayError::Protocol("RTP extension truncated".into()));
            }
            let words = usize::from(u16::from_be_bytes([packet[header + 2], packet[header + 3]]));
            header = header
                .checked_add(
                    4 + words
                        .checked_mul(4)
                        .ok_or_else(|| DisplayError::Protocol("RTP extension overflow".into()))?,
                )
                .ok_or_else(|| DisplayError::Protocol("RTP extension overflow".into()))?;
            if packet.len() < header {
                return Err(DisplayError::Protocol(
                    "RTP extension exceeds packet".into(),
                ));
            }
        }
        let padding = if packet[0] & 0x20 != 0 {
            let p = usize::from(*packet.last().unwrap_or(&0));
            if p == 0 || p > packet.len() - header {
                return Err(DisplayError::Protocol("invalid RTP padding".into()));
            }
            p
        } else {
            0
        };
        let end = packet.len() - padding;
        Ok(Some(Self {
            marker: packet[1] & 0x80 != 0,
            payload_type: packet[1] & 0x7f,
            sequence: u16::from_be_bytes([packet[2], packet[3]]),
            timestamp: u32::from_be_bytes([packet[4], packet[5], packet[6], packet[7]]),
            ssrc: u32::from_be_bytes([packet[8], packet[9], packet[10], packet[11]]),
            payload: &packet[header..end],
        }))
    }
}

pub struct RtpAssembler {
    kind: MediaKind,
    max_bytes: usize,
    max_packets: usize,
    last_sequence: Option<u16>,
    start_sequence: u16,
    timestamp: u32,
    ssrc: u32,
    bytes: BytesMut,
    packets: usize,
    fu: BytesMut,
    fu_sequence: Option<u16>,
    corrupt: bool,
}

fn strip_displayservice_trailer(nal: &[u8]) -> &[u8] {
    nal.strip_suffix(DISPLAYSERVICE_NAL_TRAILER).unwrap_or(nal)
}

fn is_rtcp_datagram(packet: &[u8]) -> bool {
    packet.len() >= 2 && packet[0] >> 6 == 2 && (64..=95).contains(&(packet[1] & 0x7f))
}

fn advance_extended_sequence(current: u32, sequence: u16) -> u32 {
    let current_sequence = current as u16;
    let delta = sequence.wrapping_sub(current_sequence);
    // A duplicate, an old packet, and the exactly-half-cycle ambiguous case
    // must not move the RTCP highest-sequence counter backwards or spuriously
    // into the next cycle.
    if delta == 0 || delta >= 0x8000 {
        return current;
    }
    let cycles = if sequence < current_sequence {
        (current >> 16).wrapping_add(1) & 0xffff
    } else {
        current >> 16
    };
    (cycles << 16) | u32::from(sequence)
}

impl RtpAssembler {
    pub fn new(kind: MediaKind, max_bytes: usize, max_packets: usize) -> Self {
        Self {
            kind,
            max_bytes,
            max_packets,
            last_sequence: None,
            start_sequence: 0,
            timestamp: 0,
            ssrc: 0,
            bytes: BytesMut::new(),
            packets: 0,
            fu: BytesMut::new(),
            fu_sequence: None,
            corrupt: false,
        }
    }
    fn reset(&mut self) {
        self.bytes.clear();
        self.fu.clear();
        self.fu_sequence = None;
        self.packets = 0;
        self.corrupt = false;
    }
    pub fn push(&mut self, packet: RtpPacket<'_>) -> Result<Option<RawAccessUnit>, DisplayError> {
        if let Some(last) = self.last_sequence {
            let delta = packet.sequence.wrapping_sub(last);
            if delta == 0 || delta > 0x8000 {
                return Ok(None);
            }
            if delta != 1 {
                self.fu.clear();
                self.fu_sequence = None;
                self.corrupt = true;
            }
        }
        self.last_sequence = Some(packet.sequence);
        if self.packets == 0 {
            self.start_sequence = packet.sequence;
            self.timestamp = packet.timestamp;
            self.ssrc = packet.ssrc;
        }
        let Some(packet_count) = self.packets.checked_add(1) else {
            self.reset();
            return Err(DisplayError::Protocol(
                "RTP access unit packet count overflow".into(),
            ));
        };
        self.packets = packet_count;
        if self.packets > self.max_packets {
            self.reset();
            return Err(DisplayError::Protocol(format!(
                "RTP access unit packet budget exceeded ({})",
                self.max_packets
            )));
        }

        // AAC-ELD is packetized as one access unit per RTP packet by the
        // CoreDevice stream. The pmd3 receiver passes each payload directly
        // to AudioToolbox and does not wait for the video-style marker bit.
        // Keeping that boundary also prevents a sender that omits M from
        // retaining an unbounded sequence of audio frames.
        if self.kind == MediaKind::Audio {
            if self.corrupt {
                self.reset();
                return Ok(None);
            }
            if packet.payload.is_empty() {
                self.reset();
                return Ok(None);
            }
            if packet.payload.len() > self.max_bytes {
                self.reset();
                return Err(DisplayError::Protocol(format!(
                    "RTP audio access unit byte budget exceeded ({})",
                    self.max_bytes
                )));
            }
            let au = RawAccessUnit {
                kind: self.kind,
                timestamp: self.timestamp,
                sequence_start: self.start_sequence,
                sequence_end: packet.sequence,
                ssrc: self.ssrc,
                data: Bytes::copy_from_slice(packet.payload),
            };
            self.reset();
            return Ok(Some(au));
        }

        let payload_result = if self.kind == MediaKind::Video {
            self.push_hevc(packet.payload)
        } else {
            self.push_bytes(packet.payload)
        };
        if let Err(error) = payload_result {
            self.reset();
            return Err(error);
        }
        if packet.marker {
            if self.corrupt || self.bytes.is_empty() {
                self.reset();
                return Ok(None);
            }
            let data = self.bytes.split().freeze();
            let au = RawAccessUnit {
                kind: self.kind,
                timestamp: self.timestamp,
                sequence_start: self.start_sequence,
                sequence_end: packet.sequence,
                ssrc: self.ssrc,
                data,
            };
            self.packets = 0;
            self.corrupt = false;
            self.fu.clear();
            self.fu_sequence = None;
            return Ok(Some(au));
        }
        Ok(None)
    }
    fn push_bytes(&mut self, data: &[u8]) -> Result<(), DisplayError> {
        if self
            .bytes
            .len()
            .checked_add(data.len())
            .map_or(true, |n| n > self.max_bytes)
        {
            self.reset();
            return Err(DisplayError::Protocol(format!(
                "RTP access unit byte budget exceeded ({})",
                self.max_bytes
            )));
        }
        self.bytes.extend_from_slice(data);
        Ok(())
    }
    fn push_hevc(&mut self, payload: &[u8]) -> Result<(), DisplayError> {
        if payload.len() < 2 {
            return Err(DisplayError::Protocol("HEVC RTP payload truncated".into()));
        }
        let typ = (payload[0] >> 1) & 0x3f;
        match typ {
            48 => {
                let mut p = 2;
                while p < payload.len() {
                    if p + 2 > payload.len() {
                        return Err(DisplayError::Protocol(
                            "HEVC aggregation length truncated".into(),
                        ));
                    }
                    let len = usize::from(u16::from_be_bytes([payload[p], payload[p + 1]]));
                    p += 2;
                    if p.checked_add(len).map_or(true, |e| e > payload.len()) {
                        return Err(DisplayError::Protocol(
                            "HEVC aggregation element exceeds packet".into(),
                        ));
                    }
                    self.push_nal(&payload[p..p + len])?;
                    p += len;
                }
            }
            49 => {
                if payload.len() < 3 {
                    return Err(DisplayError::Protocol(
                        "HEVC fragmentation header truncated".into(),
                    ));
                }
                let h = payload[2];
                let start = h & 0x80 != 0;
                let end = h & 0x40 != 0;
                let typ = h & 0x3f;
                if start {
                    self.fu.clear();
                    self.check_fu_capacity(2 + payload.len().saturating_sub(3))?;
                    self.fu
                        .extend_from_slice(&[(payload[0] & 0x81) | (typ << 1), payload[1]]);
                    self.fu.extend_from_slice(&payload[3..]);
                    self.fu_sequence = self.last_sequence;
                } else if self.fu_sequence.is_some() {
                    self.check_fu_capacity(payload.len().saturating_sub(3))?;
                    self.fu.extend_from_slice(&payload[3..]);
                } else {
                    self.corrupt = true;
                    return Ok(());
                }
                if end {
                    let nal = self.fu.split().freeze();
                    self.push_nal(strip_displayservice_trailer(&nal))?;
                    self.fu_sequence = None;
                }
            }
            _ => self.push_nal(strip_displayservice_trailer(payload))?,
        }
        Ok(())
    }
    fn check_fu_capacity(&self, additional: usize) -> Result<(), DisplayError> {
        let projected = self
            .bytes
            .len()
            .checked_add(4)
            .and_then(|size| size.checked_add(self.fu.len()))
            .and_then(|size| size.checked_add(additional));
        if projected.map_or(true, |size| size > self.max_bytes) {
            return Err(DisplayError::Protocol(format!(
                "HEVC fragmented NAL byte budget exceeded ({})",
                self.max_bytes
            )));
        }
        Ok(())
    }
    fn push_nal(&mut self, nal: &[u8]) -> Result<(), DisplayError> {
        if nal.is_empty() {
            return Ok(());
        }
        self.push_bytes(&[0, 0, 0, 1])?;
        self.push_bytes(nal)
    }
}

fn dict<const N: usize>(items: [(&str, XpcValue); N]) -> XpcValue {
    XpcValue::Dictionary(items.into_iter().map(|(k, v)| (k.into(), v)).collect())
}
fn uuid_value(id: uuid::Uuid) -> XpcValue {
    dict([("uuid", XpcValue::Uuid(*id.as_bytes()))])
}
fn typed_i64(value: i64) -> XpcValue {
    dict([("int", XpcValue::Int64(value))])
}
fn typed_u64(value: u64) -> XpcValue {
    dict([("uint", XpcValue::Uint64(value))])
}
fn typed_string(value: &str) -> XpcValue {
    dict([("string", XpcValue::String(value.into()))])
}

fn validate_media_endpoint(address: IpAddr, name: &str) -> Result<(), DisplayError> {
    if !address.is_ipv6() {
        return Err(DisplayError::Protocol(format!(
            "{name} must be an IPv6 address from the active CoreDevice tunnel"
        )));
    }
    if address.is_unspecified() {
        return Err(DisplayError::Protocol(format!(
            "{name} must be the explicit address assigned by the active tunnel (unspecified addresses are not device-reachable)"
        )));
    }
    if address.is_loopback() {
        return Err(DisplayError::Protocol(format!(
            "{name} must be a tunnel address; loopback {address} is host-local and not device-reachable"
        )));
    }
    Ok(())
}

fn optional_u64(
    dictionary: &IndexMap<String, XpcValue>,
    key: &str,
) -> Result<Option<u64>, DisplayError> {
    let Some(value) = dictionary.get(key) else {
        return Ok(None);
    };
    match value {
        XpcValue::Uint64(value) => Ok(Some(*value)),
        XpcValue::Int64(value) if *value >= 0 => Ok(Some(*value as u64)),
        _ => Err(DisplayError::Protocol(format!(
            "streamConfig.{key} is not a non-negative integer"
        ))),
    }
}

fn optional_u16(
    dictionary: &IndexMap<String, XpcValue>,
    key: &str,
) -> Result<Option<u16>, DisplayError> {
    let Some(value) = optional_u64(dictionary, key)? else {
        return Ok(None);
    };
    if value == 0 {
        return Ok(None);
    }
    u16::try_from(value).map(Some).map_err(|_| {
        DisplayError::Protocol(format!("streamConfig.{key} value {value} exceeds u16::MAX"))
    })
}

fn optional_u32(
    dictionary: &IndexMap<String, XpcValue>,
    key: &str,
) -> Result<Option<u32>, DisplayError> {
    let Some(value) = optional_u64(dictionary, key)? else {
        return Ok(None);
    };
    if value == 0 {
        return Ok(None);
    }
    u32::try_from(value).map(Some).map_err(|_| {
        DisplayError::Protocol(format!("streamConfig.{key} value {value} exceeds u32::MAX"))
    })
}

fn build_start_input(
    kind: MediaKind,
    o: &MediaStreamOptions,
    id: uuid::Uuid,
    offer: Bytes,
) -> XpcValue {
    let mut opts = IndexMap::from([
        (
            String::from("AVCMediaStreamNegotiatorAccessNetworkType"),
            typed_i64(DEFAULT_ACCESS_NETWORK_TYPE),
        ),
        (
            String::from("AVCMediaStreamNegotiatorTransportProtocolType"),
            typed_i64(DEFAULT_TRANSPORT_PROTOCOL_TYPE),
        ),
        (
            String::from("avcMediaStreamOptionClientSessionID"),
            uuid_value(id),
        ),
    ]);
    if kind == MediaKind::Video {
        opts.insert(
            "CoreDeviceVideoDisplayMode".into(),
            typed_string("DisplayByID"),
        );
        opts.insert(
            "VideoStreamForDisplayID".into(),
            typed_i64(o.display_id as i64),
        );
    }
    dict([
        (
            "clientSupportedFeatures",
            typed_u64(CLIENT_SUPPORTED_FEATURES),
        ),
        ("direction", XpcValue::String("output".into())),
        ("negotiatorOffer", XpcValue::Data(offer)),
        ("options", XpcValue::Dictionary(opts)),
        ("receiverIP", XpcValue::String(o.receiver_ip.clone())),
        ("receiverPort", typed_u64(u64::from(o.receiver_port))),
        ("senderIP", XpcValue::String(o.sender_ip.clone())),
        ("timeout", typed_u64(o.timeout.as_secs())),
        ("type", XpcValue::String(kind.wire_name().into())),
    ])
}

fn build_negotiator_offer(
    kind: MediaKind,
    call_id: &str,
    session_id: u32,
    o: &MediaStreamOptions,
) -> Result<Bytes, DisplayError> {
    let blob = build_media_blob(kind, session_id, o)?;
    let mut endpoint = Vec::new();
    pb_field(&mut endpoint, 1, 0);
    pb_field(&mut endpoint, 2, 1);
    pb_bytes(
        &mut endpoint,
        3,
        if kind == MediaKind::Video {
            b"Mac15,9"
        } else {
            b"Mac16,11"
        },
    );
    pb_bytes(&mut endpoint, 4, b"2205.3.1");
    pb_bytes(&mut endpoint, 5, b"25F80");
    let mut compressed = ZlibEncoder::new(Vec::new(), Compression::best());
    compressed
        .write_all(&blob)
        .map_err(|e| DisplayError::Protocol(format!("offer compression failed: {e}")))?;
    let compressed = compressed
        .finish()
        .map_err(|e| DisplayError::Protocol(format!("offer compression failed: {e}")))?;
    let mut p = plist::Dictionary::new();
    p.insert(
        "avcMediaStreamOptionRemoteEndpointInfo".into(),
        plist::Value::Data(endpoint),
    );
    p.insert(
        "avcMediaStreamNegotiatorMode".into(),
        plist::Value::Integer((if kind == MediaKind::Video { 5 } else { 6 }).into()),
    );
    p.insert(
        "avcMediaStreamNegotiatorMediaBlob".into(),
        plist::Value::Data(compressed),
    );
    p.insert(
        "avcMediaStreamOptionCallID".into(),
        plist::Value::String(call_id.into()),
    );
    let mut out = Vec::new();
    plist::to_writer_binary(&mut out, &plist::Value::Dictionary(p))
        .map_err(|e| DisplayError::Protocol(format!("offer plist failed: {e}")))?;
    Ok(Bytes::from(out))
}

fn pb_varint(mut n: u64, out: &mut Vec<u8>) {
    while n >= 0x80 {
        out.push((n as u8) | 0x80);
        n >>= 7;
    }
    out.push(n as u8);
}
fn pb_field(out: &mut Vec<u8>, field: u8, val: u64) {
    pb_varint(u64::from(field) << 3, out);
    pb_varint(val, out);
}
fn pb_field_padded(out: &mut Vec<u8>, field: u8, val: u32) {
    pb_varint(u64::from(field) << 3, out);
    let mut raw = Vec::new();
    pb_varint(u64::from(val), &mut raw);
    while raw.len() < 5 {
        let last = raw.len() - 1;
        raw[last] |= 0x80;
        raw.push(0);
    }
    out.extend_from_slice(&raw);
}
fn pb_bytes(out: &mut Vec<u8>, field: u8, data: &[u8]) {
    pb_varint((u64::from(field) << 3) | 2, out);
    pb_varint(data.len() as u64, out);
    out.extend_from_slice(data);
}
fn build_media_blob(
    kind: MediaKind,
    sid: u32,
    o: &MediaStreamOptions,
) -> Result<Vec<u8>, DisplayError> {
    let mut settings = Vec::new();
    pb_field_padded(&mut settings, 1, sid);
    if kind == MediaKind::Video {
        pb_field(&mut settings, 2, u64::from(o.allow_rtcp_feedback));
        for (pt, features, count, f4) in [
            (123u64, b"FLS;SW:1;" as &[u8], 4, 1u64),
            (100, b"FLS;VRAE:0;SW:1;" as &[u8], 2, 14),
        ] {
            let mut bank = Vec::new();
            pb_field(&mut bank, 1, pt);
            for i in 0..count {
                let mut entry = Vec::new();
                pb_field(&mut entry, 1, 1);
                pb_field(&mut entry, 2, 1 + (i % 2));
                pb_field(&mut entry, 3, 50115);
                pb_field(&mut entry, 4, 0);
                pb_bytes(&mut bank, 2, &entry);
            }
            pb_bytes(&mut bank, 3, features);
            pb_field(&mut bank, 4, f4);
            pb_bytes(&mut settings, 3, &bank);
        }
        if o.tiles_per_frame != 1 {
            pb_field(&mut settings, 6, o.tiles_per_frame);
        }
        pb_field(&mut settings, 7, u64::from(o.ltrp_enabled));
        pb_field(&mut settings, 8, 63);
        if o.fec_enabled {
            pb_field(&mut settings, 10, 1);
        }
        pb_field(&mut settings, 12, 1);
    } else {
        pb_field(&mut settings, 2, 0);
        pb_field(&mut settings, 3, 0);
        pb_field(&mut settings, 4, 24191);
        pb_field(&mut settings, 5, 0);
        pb_field(&mut settings, 6, 0);
    }
    let mut blob = Vec::new();
    pb_field(&mut blob, 1, 1);
    pb_field(&mut blob, 2, 1);
    pb_bytes(
        &mut blob,
        if kind == MediaKind::Video { 5 } else { 3 },
        &settings,
    );
    pb_bytes(&mut blob, 6, b"Viceroy 1.7.0");
    pb_field(&mut blob, 8, 0);
    let tiers: &[(u64, u64, Option<u64>)] = if kind == MediaKind::Video {
        &[
            (4074, 0, Some(16384)),
            (0, 75_000_000, Some(524288)),
            (0, 40_000_000, Some(12288)),
            (16, 4100, None),
            (0, 20_000_000, Some(98304)),
            (4, 6500, None),
            (0, 6_000_000, Some(131072)),
            (0, 100_000_000, Some(1048576)),
            (0, 60_000_000, Some(262144)),
            (1, 299, None),
        ]
    } else {
        &[
            (4074, 0, Some(16384)),
            (1, 299, None),
            (0, 60_000_000, Some(262144)),
            (4, 6500, None),
            (0, 20_000_000, Some(98304)),
            (0, 100_000_000, Some(1048576)),
            (0, 40_000_000, Some(12288)),
            (0, 6_000_000, Some(131072)),
            (16, 4100, None),
            (0, 75_000_000, Some(524288)),
        ]
    };
    for (f1, f2, f3) in tiers {
        let mut tier = Vec::new();
        pb_field(&mut tier, 1, *f1);
        pb_field(&mut tier, 2, *f2);
        if let Some(f3) = f3 {
            pb_field(&mut tier, 3, *f3);
        }
        pb_bytes(&mut blob, 9, &tier);
    }
    pb_field(
        &mut blob,
        13,
        if kind == MediaKind::Video {
            17137042128614416384
        } else {
            17137179377605574656
        },
    );
    pb_field(&mut blob, 14, 2);
    pb_field(&mut blob, 16, 0);
    pb_field(&mut blob, 18, 1);
    Ok(blob)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn pkt(seq: u16, ts: u32, marker: bool, payload: &[u8]) -> Vec<u8> {
        let mut p = vec![
            0x80,
            (if marker { 0x80 } else { 0 }) | 96,
            (seq >> 8) as u8,
            seq as u8,
            (ts >> 24) as u8,
            (ts >> 16) as u8,
            (ts >> 8) as u8,
            ts as u8,
            0,
            0,
            0,
            7,
        ];
        p.extend_from_slice(payload);
        p
    }
    #[test]
    fn rtp_header_extensions_and_padding_are_checked() {
        let mut p = pkt(1, 2, true, &[1, 2]);
        p[0] |= 0x30;
        p.splice(12..12, [0, 1, 0, 1, 9, 8, 7, 6]);
        p.push(1);
        assert_eq!(RtpPacket::parse(&p).unwrap().unwrap().payload, &[1, 2]);
        assert!(RtpPacket::parse(&p[..13]).is_err());
    }

    #[test]
    fn short_rtcp_control_packets_are_classified_before_rtp_parsing() {
        assert!(is_rtcp_datagram(&[0x80, 201, 0, 1, 0, 0, 0, 0]));
        assert!(!is_rtcp_datagram(&pkt(1, 2, false, &[1])));
        assert!(!is_rtcp_datagram(&[0x40, 201]));
    }

    #[tokio::test]
    async fn fake_udp_receiver_reassembles_fragmented_access_unit() {
        let receiver = UdpSocket::bind("127.0.0.1:0").await.unwrap();
        let sender = UdpSocket::bind("127.0.0.1:0").await.unwrap();
        let receiver_addr = receiver.local_addr().unwrap();
        let first = [0x62, 1, 0x80 | 19, 9, 8];
        let last = [0x62, 1, 0x40 | 19, 7];
        sender
            .send_to(&pkt(100, 4, false, &first), receiver_addr)
            .await
            .unwrap();
        sender
            .send_to(&pkt(101, 4, true, &last), receiver_addr)
            .await
            .unwrap();

        let mut assembler = RtpAssembler::new(MediaKind::Video, 1024, 10);
        let mut packet = [0u8; MAX_RTP_PACKET_BYTES];
        let mut unit = None;
        for _ in 0..2 {
            let (length, _) =
                tokio::time::timeout(Duration::from_secs(1), receiver.recv_from(&mut packet))
                    .await
                    .unwrap()
                    .unwrap();
            if let Some(rtp) = RtpPacket::parse(&packet[..length]).unwrap() {
                if let Some(next) = assembler.push(rtp).unwrap() {
                    unit = Some(next);
                }
            }
        }
        assert_eq!(unit.unwrap().data.as_ref(), &[0, 0, 0, 1, 0x26, 1, 9, 8, 7]);
    }

    #[test]
    fn hevc_fu_reassembles_and_wraps_sequence() {
        let mut a = RtpAssembler::new(MediaKind::Video, 1024, 10);
        let first = [0x62, 1, 0x80 | 19, 9, 8];
        let last = [0x62, 1, 0x40 | 19, 7];
        assert!(a
            .push(
                RtpPacket::parse(&pkt(65535, 4, false, &first))
                    .unwrap()
                    .unwrap()
            )
            .unwrap()
            .is_none());
        let au = a
            .push(RtpPacket::parse(&pkt(0, 4, true, &last)).unwrap().unwrap())
            .unwrap()
            .unwrap();
        assert_eq!(&au.data[..4], [0, 0, 0, 1]);
        assert_eq!(&au.data[6..], [9, 8, 7]);
    }

    #[test]
    fn extended_sequence_ignores_late_packet_from_previous_cycle() {
        assert_eq!(advance_extended_sequence(65_535, 0), 65_536);
        assert_eq!(advance_extended_sequence(65_536, 65_534), 65_536);
        assert_eq!(advance_extended_sequence(65_636, 50), 65_636);
        assert_eq!(advance_extended_sequence(u32::MAX, 0), 0);
    }

    #[test]
    fn loss_drops_video_access_unit_and_bounds() {
        let mut a = RtpAssembler::new(MediaKind::Video, 16, 3);
        assert!(a
            .push(
                RtpPacket::parse(&pkt(1, 1, false, &[0x02, 1, b'1']))
                    .unwrap()
                    .unwrap()
            )
            .unwrap()
            .is_none());
        assert!(a
            .push(
                RtpPacket::parse(&pkt(3, 1, true, &[0x02, 1, b'x']))
                    .unwrap()
                    .unwrap()
            )
            .unwrap()
            .is_none());
        assert!(a
            .push({
                let mut payload = vec![0x02, 1];
                payload.extend_from_slice(b"1234567890123456");
                RtpPacket::parse(&pkt(4, 2, true, &payload))
                    .unwrap()
                    .unwrap()
            })
            .is_err());
    }

    #[test]
    fn audio_emits_one_raw_access_unit_per_rtp_packet() {
        let mut assembler = RtpAssembler::new(MediaKind::Audio, 32, 4);
        let first = pkt(10, 99, false, b"aac-one");
        let unit = assembler
            .push(RtpPacket::parse(&first).unwrap().unwrap())
            .unwrap()
            .unwrap();
        assert_eq!(unit.kind, MediaKind::Audio);
        assert_eq!(unit.sequence_start, 10);
        assert_eq!(unit.sequence_end, 10);
        assert_eq!(&unit.data[..], b"aac-one");

        // AAC-ELD packet boundaries are access-unit boundaries; marker is not
        // required and must not cause payloads to be concatenated.
        let second = pkt(11, 99, true, b"aac-two");
        let unit = assembler
            .push(RtpPacket::parse(&second).unwrap().unwrap())
            .unwrap()
            .unwrap();
        assert_eq!(&unit.data[..], b"aac-two");
    }

    #[test]
    fn hevc_fragment_budget_includes_pending_fu_and_annex_b_prefix() {
        let mut assembler = RtpAssembler::new(MediaKind::Video, 8, 4);
        let start = [0x62, 1, 0x80 | 19, 9, 8];
        assert!(assembler
            .push(
                RtpPacket::parse(&pkt(1, 1, false, &start))
                    .unwrap()
                    .unwrap()
            )
            .unwrap()
            .is_none());

        // The pending NAL already needs 4 bytes for Annex-B and 4 bytes for
        // its reconstructed header/body. Appending two more bytes must fail
        // before retaining an over-budget fragmented payload.
        let continuation = [0x62, 1, 19, 7, 6];
        assert!(assembler
            .push(
                RtpPacket::parse(&pkt(2, 1, true, &continuation))
                    .unwrap()
                    .unwrap()
            )
            .is_err());
    }

    #[test]
    fn displayservice_footer_is_removed_from_hevc_access_units() {
        let mut assembler = RtpAssembler::new(MediaKind::Video, 128, 2);
        let mut payload = vec![0x02, 1, 9, 8];
        payload.extend_from_slice(DISPLAYSERVICE_NAL_TRAILER);
        let unit = assembler
            .push(
                RtpPacket::parse(&pkt(1, 1, true, &payload))
                    .unwrap()
                    .unwrap(),
            )
            .unwrap()
            .unwrap();
        assert_eq!(&unit.data[..], &[0, 0, 0, 1, 0x02, 1, 9, 8]);
    }
    #[test]
    fn offer_has_expected_modes_and_compression() {
        let o = MediaStreamOptions::default();
        let v = build_negotiator_offer(MediaKind::Video, "ABC", 1, &o).unwrap();
        let p = plist::Value::from_reader(std::io::Cursor::new(v)).unwrap();
        let d = p.as_dictionary().unwrap();
        assert_eq!(
            d["avcMediaStreamNegotiatorMode"].as_signed_integer(),
            Some(5)
        );
        let a = build_negotiator_offer(MediaKind::Audio, "ABC", 1, &o).unwrap();
        let p = plist::Value::from_reader(std::io::Cursor::new(a)).unwrap();
        assert_eq!(
            p.as_dictionary().unwrap()["avcMediaStreamNegotiatorMode"].as_signed_integer(),
            Some(6)
        );
    }
    #[test]
    fn start_request_matches_upstream_shape_for_video_and_audio() {
        let options = MediaStreamOptions {
            receiver_port: 1234,
            sender_ip: "fd00::2".into(),
            ..MediaStreamOptions::default()
        };
        let id = uuid::Uuid::from_u128(1);
        let video = build_start_input(MediaKind::Video, &options, id, Bytes::from_static(b"offer"));
        let video = video.as_dict().unwrap();
        assert_eq!(
            video["clientSupportedFeatures"].as_dict().unwrap()["uint"],
            XpcValue::Uint64(140)
        );
        assert_eq!(video["direction"].as_str(), Some("output"));
        assert_eq!(
            video["receiverPort"].as_dict().unwrap()["uint"],
            XpcValue::Uint64(1234)
        );
        assert_eq!(video["type"].as_str(), Some("video"));
        let opts = video["options"].as_dict().unwrap();
        assert!(opts.contains_key("CoreDeviceVideoDisplayMode"));
        assert!(opts.contains_key("VideoStreamForDisplayID"));
        assert_eq!(opts["avcMediaStreamOptionClientSessionID"], uuid_value(id));
        let audio = build_start_input(MediaKind::Audio, &options, id, Bytes::from_static(b"offer"));
        let audio = audio.as_dict().unwrap();
        assert_eq!(audio["type"].as_str(), Some("audio"));
        assert!(!audio["options"]
            .as_dict()
            .unwrap()
            .contains_key("VideoStreamForDisplayID"));
    }
    #[test]
    fn rtcp_report_is_compound_and_uses_extended_sequence() {
        let packet = build_rtcp_receiver_report(0x0102_0304, 0xa0b0_c0d0, 0x0001_002a);
        assert_eq!(packet.len(), 44);
        assert_eq!(&packet[0..8], &[0x81, 201, 0, 7, 1, 2, 3, 4]);
        assert_eq!(&packet[8..12], &[0xa0, 0xb0, 0xc0, 0xd0]);
        assert_eq!(&packet[16..20], &[0, 1, 0, 0x2a]);
        assert_eq!(&packet[32..44], &[0x81, 202, 0, 2, 1, 2, 3, 4, 1, 0, 0, 0]);
    }

    #[test]
    fn media_endpoint_defaults_and_numeric_bounds_fail_closed() {
        let default = MediaStreamOptions::default();
        assert!(default.receiver_ip.is_empty());
        assert!(default.sender_ip.is_empty());
        assert!(validate_media_endpoint("::1".parse().unwrap(), "receiver_ip").is_err());
        assert!(validate_media_endpoint("::".parse().unwrap(), "receiver_ip").is_err());
        assert!(validate_media_endpoint("192.0.2.1".parse().unwrap(), "receiver_ip").is_err());

        let config = IndexMap::from([
            ("SourcePort".into(), XpcValue::Uint64(u64::from(u16::MAX))),
            ("RemoteSSRC".into(), XpcValue::Uint64(u64::from(u32::MAX))),
        ]);
        assert_eq!(optional_u16(&config, "SourcePort").unwrap(), Some(u16::MAX));
        assert_eq!(optional_u32(&config, "RemoteSSRC").unwrap(), Some(u32::MAX));
        assert!(optional_u16(
            &IndexMap::from([(
                "SourcePort".into(),
                XpcValue::Uint64(u64::from(u16::MAX) + 1)
            )]),
            "SourcePort"
        )
        .is_err());
        assert!(optional_u32(
            &IndexMap::from([(
                "RemoteSSRC".into(),
                XpcValue::Uint64(u64::from(u32::MAX) + 1)
            )]),
            "RemoteSSRC"
        )
        .is_err());
    }
}
