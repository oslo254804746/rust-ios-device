#[cfg(feature = "mdns")]
use std::collections::HashMap;
#[cfg(feature = "tunnel")]
use std::net::Ipv6Addr;
#[cfg(feature = "tunnel")]
use std::path::Path;
#[cfg(feature = "tunnel")]
use std::str::FromStr;
use std::sync::Arc;
#[cfg(any(feature = "tunnel", feature = "mdns"))]
use std::time::Duration;
#[cfg(feature = "tunnel")]
use std::{
    pin::Pin,
    task::{Context, Poll},
    time::Instant,
};

#[cfg(feature = "mdns")]
use crate::lockdown::pair_record::default_pair_record_dir;
use crate::lockdown::pair_record::PairRecord;
#[cfg(feature = "tunnel")]
use crate::lockdown::pairing::{
    build_verify_start_tlv, build_verify_step2_tlv, HostIdentity, VerifyPairSession,
};
use crate::lockdown::protocol::{recv_lockdown, send_lockdown};
#[cfg(feature = "tunnel")]
use crate::lockdown::session::CORE_DEVICE_PROXY;
use crate::lockdown::session::{start_lockdown_session, start_service, wrap_service_tls};
use crate::lockdown::LOCKDOWN_PORT;
use crate::mux::MuxClient;
#[cfg(feature = "tunnel")]
use crate::proto::tlv::TlvBuffer;
#[cfg(feature = "tunnel-kernel")]
use crate::tunnel::forward::forward_packets;
use crate::tunnel::manager::{TunMode, TunnelHandle};
#[cfg(feature = "tunnel-kernel")]
use crate::tunnel::tun::kernel::KernelTunDevice;
#[cfg(feature = "tunnel-userspace")]
use crate::tunnel::tun::userspace::UserspaceTunDevice;
#[cfg(feature = "tunnel")]
use crate::xpc::message::XpcValue;
#[cfg(all(feature = "tunnel", feature = "mdns"))]
use crate::xpc::rsd::handshake as rsd_handshake;
use crate::xpc::rsd::{RsdHandshake, ServiceDescriptor};
#[cfg(feature = "tunnel")]
use crate::xpc::XpcClient;
#[cfg(feature = "tunnel")]
use base64::{engine::general_purpose::STANDARD as BASE64_STANDARD, Engine as _};
#[cfg(feature = "tunnel")]
use chacha20poly1305::{aead::Aead, KeyInit};
#[cfg(feature = "tunnel")]
use indexmap::IndexMap;
#[cfg(feature = "tunnel")]
use rand::RngCore;
#[cfg(feature = "tunnel")]
use tokio::io::ReadBuf;
use tokio::io::{AsyncRead, AsyncWrite};
use tokio::net::TcpStream;
#[cfg(feature = "tunnel")]
use tokio_stream::StreamExt;

#[cfg(feature = "tunnel")]
use crate::credentials::{PersistedCredentials, RemotePairingRecord};
use crate::discovery::DeviceInfo;
#[cfg(feature = "mdns")]
use crate::discovery::{
    browse_mobdev2, browse_remotepairing, mobdev2_wifi_mac, BonjourService, MdnsDevice,
};
use crate::error::CoreError;

// ── ConnectOptions ─────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Default)]
pub struct ConnectOptions {
    pub tun_mode: TunMode,
    pub pair_record_path: Option<std::path::PathBuf>,
    /// Skip tunnel; use direct lockdown (iOS <17 or service-only access).
    pub skip_tunnel: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct InternationalConfiguration {
    pub language: String,
    pub locale: String,
    pub supported_locales: Vec<String>,
    pub supported_languages: Vec<String>,
}

// ── ServiceStream ──────────────────────────────────────────────────────────────

/// A boxed bidirectional async stream returned by `connect_service()`.
pub type ServiceStream = Box<dyn ServiceStreamTrait>;

pub trait ServiceStreamTrait: AsyncRead + AsyncWrite + Unpin + Send {}
impl<T: AsyncRead + AsyncWrite + Unpin + Send> ServiceStreamTrait for T {}

#[cfg(feature = "tunnel")]
const TUNNEL_HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(5);
#[cfg(feature = "mdns")]
const MOBDEV2_DISCOVERY_TIMEOUT: Duration = Duration::from_secs(3);
#[cfg(all(feature = "tunnel", feature = "mdns"))]
const DIRECT_RSD_DISCOVERY_TIMEOUT: Duration = Duration::from_secs(3);
// Direct pairing TLV type: public key exchange (X25519 ephemeral public key)
#[cfg(feature = "tunnel")]
const DIRECT_PAIRING_TYPE_PUBLIC_KEY: u8 = 0x03;
// Direct pairing TLV type: error response from device (pairing rejected or failed)
#[cfg(feature = "tunnel")]
const DIRECT_PAIRING_TYPE_ERROR: u8 = 0x07;
#[cfg(feature = "tunnel")]
const DIRECT_CONTROL_CHANNEL_ENVELOPE_TYPE: &str = "RemotePairing.ControlChannelMessageEnvelope";
#[cfg(feature = "tunnel")]
const DIRECT_CONTROL_CHANNEL_ORIGIN: &str = "host";

// ── ConnectedDevice ────────────────────────────────────────────────────────────

pub struct ConnectedDevice {
    pub info: DeviceInfo,
    pub(crate) tunnel: Option<Arc<TunnelHandle>>,
    /// RSD service directory (only available after tunnel is up on iOS 17+)
    pub(crate) rsd: Option<RsdHandshake>,
    pair_record: Option<Arc<PairRecord>>,
    lockdown_transport: LockdownTransport,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PairedMobdev2Device {
    pub udid: String,
    pub host: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg(feature = "tunnel")]
enum TunnelConnectionTarget {
    UserspaceProxy {
        proxy_port: u16,
        remote_addr: Ipv6Addr,
    },
    DirectIpv6 {
        remote_addr: Ipv6Addr,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum LockdownTransport {
    Usbmux { device_id: u32 },
    Tcp { host: String },
}

fn should_strip_service_ssl(service_name: &str) -> bool {
    matches!(
        service_name,
        "com.apple.instruments.remoteserver" | "com.apple.accessibility.axAuditDaemon.remoteserver"
    )
}

include!("connected.rs");
include!("lockdown_transport.rs");
include!("tunnel_connect.rs");
include!("discovery_match.rs");
include!("rsd_connect.rs");
include!("support.rs");

#[cfg(test)]
include!("tests.rs");
