//! Unified high-level API for iOS device interaction.
//!
//! This crate ties together device discovery, pairing, tunneling, and service access
//! into a single ergonomic API. It is the recommended entry point for library consumers.
//!
//! Key types:
//! - [`device::IosDevice`] — connected device handle with service access
//! - [`discovery`] — USB and network device discovery
//!
//! Internal transport modules are not part of the public API. Use top-level
//! re-exports for supported types:
//!
//! ```
//! use ios_core::{pair_new_device, PairedCredentials, PairingTransportError};
//! use ios_core::{default_pair_record_path, LockdownClient, LockdownError, PairRecord};
//! use ios_core::{archive_string, NsUrl, XcTestConfiguration, XctCapabilities};
//! use ios_core::{TunMode, TunnelError, TunnelHandle, TunnelInfo, TunnelManager};
//! ```
//!
//! ```compile_fail
//! use ios_core::proto::nskeyedarchiver_encode::archive_string;
//! ```
//!
//! ```compile_fail
//! use ios_core::lockdown::PairRecord;
//! ```
//!
//! ```compile_fail
//! use ios_core::pairing_transport::UNTRUSTED_SERVICE_NAME;
//! ```
//!
//! ```compile_fail
//! use ios_core::tunnel::TunMode;
//! ```
//!
//! ```compile_fail
//! use ios_core::xpc::RsdHandshake;
//! ```

pub mod credentials;
pub mod device;
pub mod discovery;
pub mod error;
pub(crate) mod lockdown;
pub(crate) mod mux;
#[cfg(all(feature = "tunnel", feature = "mdns"))]
pub(crate) mod pairing_transport;
pub(crate) mod proto;
#[cfg(feature = "tunnel")]
pub(crate) mod psk_tls;
pub mod services;
#[cfg(test)]
pub(crate) mod test_util;
pub(crate) mod tunnel;
pub(crate) mod xpc;

pub use credentials::{PersistedCredentials, RemotePairingRecord};
#[cfg(feature = "mdns")]
pub use device::discover_paired_mobdev2_devices;
pub use device::{
    connect_direct_usb_tunnel, connect_remote_pairing_tunnel, connect_tcp_lockdown_tunnel,
    ConnectOptions, ConnectedDevice, InternationalConfiguration, PairedMobdev2Device,
    ServiceStream,
};
#[cfg(feature = "mdns")]
pub use discovery::{browse_mobdev2, browse_remotepairing, BonjourService, MdnsDevice};
pub use discovery::{DeviceEvent, DeviceInfo};
pub use error::CoreError;
pub use lockdown::{
    default_pair_record_path, handshake_only_service_tls, recv_lockdown, send_lockdown,
    start_lockdown_session, start_service, strip_service_tls, wrap_service_tls, GetValueRequest,
    GetValueResponse, LockdownClient, LockdownError, PairRecord, PairRecordError, QueryTypeRequest,
    QueryTypeResponse, RemoveValueRequest, ServiceInfo, SetValueRequest, StartServiceRequest,
    StartServiceResponse, StartSessionRequest, StartSessionResponse, StopSessionRequest,
    ValueOperationResponse, CORE_DEVICE_PROXY, LOCKDOWN_PORT,
};
#[cfg(feature = "supervised-pair")]
pub use lockdown::{pair_supervised, save_pair_record, FullPairRecord};
pub use mux::MuxClient;
#[cfg(all(feature = "tunnel", feature = "mdns"))]
pub use pairing_transport::{pair_new_device, PairedCredentials, PairingTransportError};
pub use proto::nskeyedarchiver_encode::{
    archive_array, archive_bool, archive_data, archive_dict, archive_float, archive_int,
    archive_nsurl, archive_null, archive_string, archive_uuid, archive_xct_capabilities,
    archive_xctest_configuration, NsUrl, XcTestConfiguration, XctCapabilities,
};
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
#[cfg(feature = "diagnosticsservice")]
pub use services::diagnosticsservice;
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
pub use tunnel::{TunMode, TunnelError, TunnelHandle, TunnelInfo, TunnelManager};
#[cfg(feature = "tunnel")]
pub use xpc::client::XpcClient;
pub use xpc::message::flags as xpc_message_flags;
pub use xpc::message::{
    decode_message as decode_xpc_message, encode_message as encode_xpc_message, XpcMessage,
    XpcValue,
};
pub use xpc::rsd::{RsdHandshake, ServiceDescriptor, RSD_PORT};
pub use xpc::XpcError;

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
/// Use [`connect`] to establish a session and inspect the RSD service list.
#[cfg(feature = "mdns")]
pub async fn discover_mdns() -> Result<impl futures_core::Stream<Item = MdnsDevice>, CoreError> {
    discovery::discover_mdns().await
}
