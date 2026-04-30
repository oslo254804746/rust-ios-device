//! Unified high-level API for iOS device interaction.
//!
//! This crate ties together device discovery, pairing, tunneling, and service access
//! into a single ergonomic API. It is the recommended entry point for library consumers.
//!
//! Key types:
//! - [`device::IosDevice`] — connected device handle with service access
//! - [`discovery`] — USB and network device discovery

pub mod credentials;
pub mod device;
pub mod discovery;
pub mod error;
pub mod lockdown;
pub(crate) mod mux;
pub mod pairing_transport;
pub mod proto;
pub(crate) mod psk_tls;
pub mod services;
pub mod tunnel;
pub mod xpc;

pub use credentials::{PersistedCredentials, RemotePairingRecord};
pub use device::{
    connect_direct_usb_tunnel, connect_remote_pairing_tunnel, connect_tcp_lockdown_tunnel,
    discover_paired_mobdev2_devices, ConnectOptions, ConnectedDevice, InternationalConfiguration,
    PairedMobdev2Device, ServiceStream,
};
pub use discovery::{
    browse_mobdev2, browse_remotepairing, BonjourService, DeviceEvent, DeviceInfo, MdnsDevice,
};
pub use error::CoreError;
pub use lockdown::{LockdownClient, LOCKDOWN_PORT};
pub use mux::MuxClient;
pub use pairing_transport::{pair_new_device, PairedCredentials, UNTRUSTED_SERVICE_NAME};
#[cfg(feature = "accessibility_audit")]
pub use services::accessibility_audit;
#[cfg(feature = "afc")]
pub use services::afc;
#[cfg(feature = "amfi")]
pub use services::amfi;
#[cfg(feature = "apps")]
pub use services::apps;
#[cfg(feature = "arbitration")]
pub use services::arbitration;
#[cfg(feature = "companion")]
pub use services::companion;
#[cfg(feature = "crashreport")]
pub use services::crashreport;
#[cfg(feature = "debugserver")]
pub use services::debugserver;
#[cfg(feature = "deviceinfo")]
pub use services::deviceinfo;
#[cfg(feature = "diagnostics")]
pub use services::diagnostics;
#[cfg(feature = "dproxy")]
pub use services::dproxy;
#[cfg(feature = "dtx")]
pub use services::dtx;
#[cfg(feature = "fetchsymbols")]
pub use services::fetchsymbols;
#[cfg(feature = "file_relay")]
pub use services::file_relay;
#[cfg(feature = "fileservice")]
pub use services::fileservice;
#[cfg(feature = "heartbeat")]
pub use services::heartbeat;
#[cfg(feature = "idam")]
pub use services::idam;
#[cfg(feature = "imagemounter")]
pub use services::imagemounter;
#[cfg(feature = "instruments")]
pub use services::instruments;
#[cfg(feature = "mcinstall")]
pub use services::mcinstall;
#[cfg(feature = "misagent")]
pub use services::misagent;
#[cfg(feature = "mobileactivation")]
pub use services::mobileactivation;
#[cfg(feature = "notificationproxy")]
pub use services::notificationproxy;
#[cfg(feature = "ostrace")]
pub use services::ostrace;
#[cfg(feature = "pcap")]
pub use services::pcap;
#[cfg(feature = "power_assertion")]
pub use services::power_assertion;
#[cfg(feature = "preboard")]
pub use services::preboard;
#[cfg(feature = "prepare")]
pub use services::prepare;
#[cfg(feature = "restore")]
pub use services::restore;
#[cfg(feature = "screenshot")]
pub use services::screenshot;
#[cfg(feature = "springboard")]
pub use services::springboard;
#[cfg(feature = "syslog")]
pub use services::syslog;
#[cfg(feature = "testmanager")]
pub use services::testmanager;
#[cfg(feature = "webinspector")]
pub use services::webinspector;
pub use services::{backup2, device_link, simlocation};
pub use tunnel::TunMode;
pub use xpc::{RsdHandshake, ServiceDescriptor, XpcMessage, XpcValue};

/// List all currently connected iOS devices (via usbmuxd).
pub async fn list_devices() -> Result<Vec<DeviceInfo>, CoreError> {
    discovery::list_devices().await
}

/// Watch for usbmux attach/detach events through the reusable ios-core discovery layer.
pub async fn watch_devices(
) -> Result<impl futures_core::Stream<Item = Result<DeviceEvent, CoreError>>, CoreError> {
    discovery::watch_devices().await
}

/// Connect to an iOS device by UDID and optionally establish a CDTunnel.
pub async fn connect(udid: &str, opts: ConnectOptions) -> Result<ConnectedDevice, CoreError> {
    device::connect(udid, opts).await
}

/// Discover iOS 17+ devices on the local network via mDNS.
///
/// Returns a stream of devices with their IPv6 address and RSD port.
/// Use [`xpc::rsd::handshake`] to get the full service list.
pub async fn discover_mdns() -> Result<impl futures_core::Stream<Item = MdnsDevice>, CoreError> {
    discovery::discover_mdns().await
}
