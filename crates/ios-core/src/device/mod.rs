//! Device connection facade; implementation dependencies stay in their owners.

use std::sync::Arc;

use crate::discovery::DeviceInfo;
use crate::lockdown::pair_record::PairRecord;
use crate::tunnel::manager::{TunMode, TunnelHandle};
use crate::xpc::rsd::RsdHandshake;
use tokio::io::{AsyncRead, AsyncWrite};

// ── ConnectOptions ─────────────────────────────────────────────────────────────

#[derive(Clone, Default)]
pub struct ConnectOptions {
    pub tun_mode: TunMode,
    pub pair_record_path: Option<std::path::PathBuf>,
    /// Skip tunnel; use direct lockdown (iOS <17 or service-only access).
    pub skip_tunnel: bool,
}

impl std::fmt::Debug for ConnectOptions {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ConnectOptions")
            .field("tun_mode", &self.tun_mode)
            .field("pair_record_configured", &self.pair_record_path.is_some())
            .field("skip_tunnel", &self.skip_tunnel)
            .finish()
    }
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

/// Metadata for the RSD service entry used to establish an XPC connection.
///
/// The resolved name is the exact canonical or `.shim.remote` key selected by
/// the same lookup that opened the socket.  Features are copied from that
/// descriptor so callers do not need to inspect a possibly stale cached RSD
/// handshake after an on-demand handshake.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedServiceMetadata {
    pub resolved_service_name: String,
    pub features: Vec<String>,
}

// ── ConnectedDevice ────────────────────────────────────────────────────────────

pub struct ConnectedDevice {
    pub info: DeviceInfo,
    pub(crate) tunnel: Option<Arc<TunnelHandle>>,
    /// RSD service directory (only available after tunnel is up on iOS 17+)
    pub(crate) rsd: Option<RsdHandshake>,
    #[cfg(feature = "tunnel")]
    owned_initial_rsd: Option<crate::OwnedRsdHandshake>,
    #[cfg(feature = "tunnel")]
    owned_initial_bank: Option<Arc<tokio::sync::Semaphore>>,
    #[cfg(feature = "tunnel")]
    owned_prepared_rsd: Option<PreparedRsdOwner>,
    #[cfg(feature = "tunnel")]
    pub(crate) prepared_rsd: Option<std::sync::Arc<PreparedRsdCatalogue>>,
    initial_rsd_probe: InitialRsdProbe,
    pair_record: Option<Arc<PairRecord>>,
    lockdown_transport: LockdownTransport,
}

#[derive(Clone, PartialEq, Eq)]
pub struct PairedMobdev2Device {
    pub udid: String,
    pub host: String,
}

impl std::fmt::Debug for PairedMobdev2Device {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PairedMobdev2Device")
            .field("udid", &crate::redaction::diagnostic_id(&self.udid))
            .field("host", &crate::redaction::diagnostic_id(&self.host))
            .finish()
    }
}

#[derive(Clone, PartialEq, Eq)]
enum LockdownTransport {
    Usbmux { device_id: u32 },
    Tcp { host: String },
}

impl std::fmt::Debug for LockdownTransport {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Usbmux { device_id } => f
                .debug_struct("Usbmux")
                .field("device_id", device_id)
                .finish(),
            Self::Tcp { host } => f
                .debug_struct("Tcp")
                .field("host", &crate::redaction::diagnostic_id(host))
                .finish(),
        }
    }
}

#[cfg(feature = "mobileactivation")]
mod activation_error;
#[cfg(feature = "mobileactivation")]
mod deactivation_error;
#[cfg(feature = "mobileactivation")]
pub use deactivation_error::DeactivationFallbackError;
mod connected;
#[cfg(feature = "tunnel")]
mod owned_connection;
#[cfg(feature = "tunnel")]
pub use owned_connection::OwnedConnectedDevice;
mod connection;
#[cfg(feature = "tunnel")]
pub(crate) mod existing_rsd;
#[cfg(feature = "tunnel")]
pub(crate) mod prepared_rsd;
#[cfg(feature = "tunnel")]
pub use existing_rsd::{
    observe_existing_rsd, observe_existing_rsd_with_selection, ExistingRsdCleanupFailure,
    ExistingRsdCleanupReceipt, ExistingRsdObservation, ExistingRsdProbeError, ExistingRsdRoute,
    ExistingRsdService, ExistingRsdServiceConnection, ExistingRsdServiceEvidence,
    ExistingRsdServiceKind, ExistingRsdServiceOwner, ExistingRsdSession, RsdHandshakeMode,
    RsdSelectionReceipt,
};
#[cfg(feature = "tunnel")]
pub use existing_rsd::{
    prepare_existing_rsd_at, prepare_existing_rsd_with_identity_at, prepare_owned_existing_rsd_at,
    prepare_owned_existing_rsd_with_identity_at,
};
#[cfg(feature = "tunnel")]
pub use prepared_rsd::{PreparedRsdCatalogue, PreparedRsdOwner};
#[cfg(any(feature = "mdns", feature = "tunnel"))]
pub(crate) mod error_context;
mod lockdown_transport;
mod rsd_observation;
mod service_routing;
mod support;
mod tunnel_owner;

pub use tunnel_owner::{TunnelOwner, TunnelReservation, TunnelReservationOwner};

pub use rsd_observation::{InitialRsdProbe, InitialRsdProbeSkipReason, InitialRsdProbeState};

pub use connection::{
    connect, connect_direct_usb_tunnel, connect_direct_usb_tunnel_with_owner,
    connect_remote_pairing_tunnel, connect_remote_pairing_tunnel_with_owner,
    connect_tcp_lockdown_tunnel, connect_tcp_lockdown_tunnel_with_owner, connect_with_owner,
};
#[cfg(feature = "tunnel")]
pub use connection::{
    connect_direct_usb_tunnel_owned_with_owner, connect_owned_with_owner,
    connect_remote_pairing_tunnel_owned_with_owner, connect_tcp_lockdown_tunnel_owned_with_owner,
};

#[cfg(feature = "mdns")]
mod discovery_match;
#[cfg(feature = "mdns")]
pub use discovery_match::discover_paired_mobdev2_devices;
#[cfg(all(feature = "mdns", feature = "tunnel"))]
pub use discovery_match::{observe_paired_identity, PairedIdentityReceipt};

#[cfg(feature = "tunnel")]
mod rsd_connect;
#[cfg(feature = "tunnel")]
mod tunnel_activation;
#[cfg(all(feature = "tunnel", any(feature = "mdns", test)))]
mod tunnel_connect;
#[cfg(feature = "tunnel")]
mod tunnel_endpoint;

#[cfg(test)]
mod tests;
