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
pub mod pairing_transport;
pub mod psk_tls;

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
pub use pairing_transport::{pair_new_device, PairedCredentials, UNTRUSTED_SERVICE_NAME};

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
/// Use `ios_xpc::rsd::handshake(ipv6, rsd_port)` to get the full service list.
pub async fn discover_mdns() -> Result<impl futures_core::Stream<Item = MdnsDevice>, CoreError> {
    discovery::discover_mdns().await
}
