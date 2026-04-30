use std::collections::HashMap;
use std::net::Ipv6Addr;
use std::path::Path;
use std::pin::Pin;
use std::str::FromStr;
use std::sync::Arc;
use std::task::{Context, Poll};
use std::time::{Duration, Instant};

use crate::lockdown::pair_record::{default_pair_record_dir, PairRecord};
use crate::lockdown::pairing::{
    build_verify_start_tlv, build_verify_step2_tlv, HostIdentity, VerifyPairSession,
};
use crate::lockdown::protocol::{recv_lockdown, send_lockdown};
use crate::lockdown::session::{
    start_lockdown_session, start_service, wrap_service_tls, CORE_DEVICE_PROXY,
};
use crate::lockdown::LOCKDOWN_PORT;
use crate::mux::MuxClient;
use crate::proto::tlv::TlvBuffer;
use crate::tunnel::{
    forward::forward_packets,
    manager::{TunMode, TunnelHandle},
    tun::{kernel::KernelTunDevice, userspace::UserspaceTunDevice},
};
use crate::xpc::{
    message::XpcValue,
    rsd::{handshake as rsd_handshake, RsdHandshake, ServiceDescriptor},
    XpcClient,
};
use base64::{engine::general_purpose::STANDARD as BASE64_STANDARD, Engine as _};
use chacha20poly1305::{aead::Aead, KeyInit};
use indexmap::IndexMap;
use rand::RngCore;
use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};
use tokio::net::TcpStream;
use tokio_stream::StreamExt;

use crate::credentials::{PersistedCredentials, RemotePairingRecord};
use crate::discovery::{
    browse_mobdev2, browse_remotepairing, mobdev2_wifi_mac, BonjourService, DeviceInfo, MdnsDevice,
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

const TUNNEL_HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(5);
const MOBDEV2_DISCOVERY_TIMEOUT: Duration = Duration::from_secs(3);
const DIRECT_RSD_DISCOVERY_TIMEOUT: Duration = Duration::from_secs(3);
// Direct pairing TLV type: public key exchange (X25519 ephemeral public key)
const DIRECT_PAIRING_TYPE_PUBLIC_KEY: u8 = 0x03;
// Direct pairing TLV type: error response from device (pairing rejected or failed)
const DIRECT_PAIRING_TYPE_ERROR: u8 = 0x07;
const DIRECT_CONTROL_CHANNEL_ENVELOPE_TYPE: &str = "RemotePairing.ControlChannelMessageEnvelope";
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

impl ConnectedDevice {
    /// The RSD handshake result, if available (iOS 17+ with tunnel).
    pub fn rsd(&self) -> Option<&RsdHandshake> {
        self.rsd.as_ref()
    }

    /// Take ownership of the RSD handshake, consuming it from the device.
    pub fn into_rsd(self) -> Option<RsdHandshake> {
        self.rsd
    }

    /// The tunnel handle, if a tunnel is active.
    pub fn tunnel_handle(&self) -> Option<&Arc<TunnelHandle>> {
        self.tunnel.as_ref()
    }

    pub fn server_address(&self) -> Option<&str> {
        self.tunnel.as_ref().map(|t| t.info.server_address.as_str())
    }

    pub fn userspace_port(&self) -> Option<u16> {
        self.tunnel.as_ref().and_then(|t| t.userspace_port)
    }

    pub fn rsd_port(&self) -> Option<u16> {
        self.tunnel.as_ref().map(|t| t.info.server_rsd_port)
    }

    fn pair_record(&self) -> Result<&Arc<PairRecord>, CoreError> {
        self.pair_record
            .as_ref()
            .ok_or_else(|| CoreError::Unsupported("no pair record loaded".into()))
    }

    async fn lockdown_client(&self) -> Result<crate::lockdown::LockdownClient, CoreError> {
        let pair_record = self.pair_record()?;
        let stream = connect_lockdown_port(
            &self.info.udid,
            &self.lockdown_transport,
            LOCKDOWN_PORT,
            true,
        )
        .await?;
        crate::lockdown::LockdownClient::connect_with_stream(stream, pair_record)
            .await
            .map_err(CoreError::from)
    }

    /// Open a lockdown service stream (iOS <17 or iOS 17+ services also accessible via lockdown).
    pub async fn connect_service(&self, service_name: &str) -> Result<ServiceStream, CoreError> {
        let pair_record = self.pair_record()?;
        let lockdown_stream = connect_lockdown_port(
            &self.info.udid,
            &self.lockdown_transport,
            LOCKDOWN_PORT,
            true,
        )
        .await?;

        let (_session_id, mut tls_reader, mut tls_writer) =
            start_lockdown_session(lockdown_stream, pair_record).await?;

        let (port, enable_ssl) =
            start_service(&mut tls_reader, &mut tls_writer, service_name).await?;

        let svc_stream =
            connect_lockdown_port(&self.info.udid, &self.lockdown_transport, port, false).await?;

        if enable_ssl {
            let tls = wrap_service_tls(svc_stream, pair_record)
                .await
                .map_err(|e| CoreError::Other(e.to_string()))?;
            if should_strip_service_ssl(service_name) {
                let stream = crate::lockdown::session::strip_service_tls(tls)
                    .map_err(|e| CoreError::Other(e.to_string()))?;
                Ok(Box::new(stream))
            } else {
                Ok(Box::new(tls))
            }
        } else {
            Ok(Box::new(svc_stream))
        }
    }

    /// Get the device's iOS version via lockdown.
    pub async fn product_version(&self) -> Result<semver::Version, CoreError> {
        let mut client = self.lockdown_client().await?;
        let ver = client.product_version().await?;
        Ok(ver)
    }

    /// Get a lockdown value by key (domain=None for global domain).
    pub async fn lockdown_get_value(&self, key: Option<&str>) -> Result<plist::Value, CoreError> {
        self.lockdown_get_value_in_domain(None, key).await
    }

    /// Get a lockdown value by optional domain and key.
    pub async fn lockdown_get_value_in_domain(
        &self,
        domain: Option<&str>,
        key: Option<&str>,
    ) -> Result<plist::Value, CoreError> {
        let mut client = self.lockdown_client().await?;
        client
            .get_value(domain, key)
            .await
            .map_err(|e| CoreError::Other(e.to_string()))
    }

    /// Set a lockdown value by key (domain=None for global domain).
    pub async fn lockdown_set_value(
        &self,
        key: Option<&str>,
        value: plist::Value,
    ) -> Result<(), CoreError> {
        self.lockdown_set_value_in_domain(None, key, value).await
    }

    /// Set a lockdown value by optional domain and key.
    pub async fn lockdown_set_value_in_domain(
        &self,
        domain: Option<&str>,
        key: Option<&str>,
        value: plist::Value,
    ) -> Result<(), CoreError> {
        let mut client = self.lockdown_client().await?;
        client
            .set_value(domain, key, value)
            .await
            .map_err(|e| CoreError::Other(e.to_string()))
    }

    /// Remove a lockdown value by key (domain=None for global domain).
    pub async fn lockdown_remove_value(&self, key: Option<&str>) -> Result<(), CoreError> {
        self.lockdown_remove_value_in_domain(None, key).await
    }

    /// Remove a lockdown value by optional domain and key.
    pub async fn lockdown_remove_value_in_domain(
        &self,
        domain: Option<&str>,
        key: Option<&str>,
    ) -> Result<(), CoreError> {
        let mut client = self.lockdown_client().await?;
        client
            .remove_value(domain, key)
            .await
            .map_err(|e| CoreError::Other(e.to_string()))
    }

    /// Read language and locale metadata from `com.apple.international`.
    pub async fn lockdown_international_configuration(
        &self,
    ) -> Result<InternationalConfiguration, CoreError> {
        const INTERNATIONAL_DOMAIN: &str = "com.apple.international";

        let mut client = self.lockdown_client().await?;
        let language = client
            .get_value(Some(INTERNATIONAL_DOMAIN), Some("Language"))
            .await
            .map_err(|e| CoreError::Other(e.to_string()))?;
        let locale = client
            .get_value(Some(INTERNATIONAL_DOMAIN), Some("Locale"))
            .await
            .map_err(|e| CoreError::Other(e.to_string()))?;
        let supported_locales = client
            .get_value(Some(INTERNATIONAL_DOMAIN), Some("SupportedLocales"))
            .await
            .map_err(|e| CoreError::Other(e.to_string()))?;
        let supported_languages = client
            .get_value(Some(INTERNATIONAL_DOMAIN), Some("SupportedLanguages"))
            .await
            .map_err(|e| CoreError::Other(e.to_string()))?;

        Ok(InternationalConfiguration {
            language: plist_value_to_string(&language, "Language")?,
            locale: plist_value_to_string(&locale, "Locale")?,
            supported_locales: plist_value_to_string_vec(&supported_locales, "SupportedLocales")?,
            supported_languages: plist_value_to_string_vec(
                &supported_languages,
                "SupportedLanguages",
            )?,
        })
    }

    /// Connect to an RSD service as a raw TCP stream (no XPC/H2 framing).
    ///
    /// Suitable for DTX-based services like `com.apple.instruments.dtservicehub`.
    /// Supports userspace proxy and direct IPv6/kernel tunnel connections.
    /// Performs an on-demand RSD handshake if rsd is not already populated.
    pub async fn connect_rsd_service(
        &self,
        service_name: &str,
    ) -> Result<ServiceStream, CoreError> {
        let (resolved_service_name, port) =
            self.resolve_rsd_service_with_retry(service_name).await?;

        let mut stream = self.connect_tunnel_port(port).await?;
        if resolved_service_name.ends_with(".shim.remote") {
            rsd_checkin(&mut stream).await?;
        }
        Ok(stream)
    }

    /// Connect to an iOS 17+ XPC service via RSD.
    ///
    /// Returns an XpcClient ready for method calls.
    /// Performs an on-demand RSD handshake if rsd is not already populated.
    pub async fn connect_xpc_service(&self, service_name: &str) -> Result<XpcClient, CoreError> {
        let (_resolved_service_name, port) =
            self.resolve_rsd_service_with_retry(service_name).await?;
        let stream = self.connect_tunnel_port(port).await?;

        XpcClient::connect_stream(stream)
            .await
            .map_err(|e| CoreError::Other(e.to_string()))
    }

    async fn resolve_rsd_service_with_retry(
        &self,
        service_name: &str,
    ) -> Result<(String, u16), CoreError> {
        if let Some(rsd) = self.rsd.as_ref() {
            return resolve_rsd_service(rsd, service_name).ok_or_else(|| {
                CoreError::Unsupported(format!(
                    "service '{service_name}' not found in RSD directory"
                ))
            });
        }

        let rsd = self.resolve_rsd_with_retry().await?;
        resolve_rsd_service(&rsd, service_name).ok_or_else(|| {
            CoreError::Unsupported(format!(
                "service '{service_name}' not found in RSD directory"
            ))
        })
    }

    async fn resolve_rsd_with_retry(&self) -> Result<RsdHandshake, CoreError> {
        const MAX_ATTEMPTS: usize = 5;

        if self.tunnel.is_none() {
            return Err(CoreError::Unsupported(
                "RSD not available (no tunnel or iOS <17)".into(),
            ));
        }

        for attempt in 0..MAX_ATTEMPTS {
            if attempt > 0 {
                tokio::time::sleep(std::time::Duration::from_millis(500)).await;
            }

            if let Some(rsd) = self.attempt_rsd_from_tunnel().await? {
                return Ok(rsd);
            }

            tracing::debug!(
                "RSD handshake attempt {}/{} failed, retrying...",
                attempt + 1,
                MAX_ATTEMPTS
            );
        }

        Err(CoreError::Unsupported(
            "RSD handshake failed after retries".into(),
        ))
    }

    async fn attempt_rsd_from_tunnel(&self) -> Result<Option<RsdHandshake>, CoreError> {
        let server_addr = self
            .server_address()
            .ok_or_else(|| CoreError::Unsupported("no server address".into()))?;
        let rsd_port = self
            .rsd_port()
            .ok_or_else(|| CoreError::Unsupported("no RSD port from tunnel info".into()))?;

        Ok(match self.userspace_port() {
            Some(proxy_port) => attempt_rsd_via_proxy(proxy_port, server_addr, rsd_port).await,
            None => attempt_rsd(server_addr, rsd_port).await,
        })
    }

    fn tunnel_connection_target(&self) -> Result<TunnelConnectionTarget, CoreError> {
        let server_addr = self
            .server_address()
            .ok_or_else(|| CoreError::Unsupported("no server address".into()))?;

        resolve_tunnel_connection_target(server_addr, self.userspace_port())
    }

    async fn connect_tunnel_port(&self, port: u16) -> Result<ServiceStream, CoreError> {
        use tokio::io::AsyncWriteExt;
        use tokio::net::TcpStream;

        match self.tunnel_connection_target()? {
            TunnelConnectionTarget::UserspaceProxy {
                proxy_port,
                remote_addr,
            } => {
                let mut proxy = TcpStream::connect(format!("127.0.0.1:{proxy_port}")).await?;
                proxy.write_all(&remote_addr.octets()).await?;
                proxy.write_all(&(port as u32).to_le_bytes()).await?;
                Ok(Box::new(proxy))
            }
            TunnelConnectionTarget::DirectIpv6 { remote_addr } => {
                let addr =
                    std::net::SocketAddr::V6(std::net::SocketAddrV6::new(remote_addr, port, 0, 0));
                Ok(Box::new(TcpStream::connect(addr).await?))
            }
        }
    }
}

#[derive(serde::Serialize)]
#[serde(rename_all = "PascalCase")]
struct RsdCheckinRequest {
    label: &'static str,
    protocol_version: &'static str,
    request: &'static str,
}

fn resolve_rsd_service(rsd: &RsdHandshake, requested_service: &str) -> Option<(String, u16)> {
    if let Some(ServiceDescriptor { port }) = rsd.services.get(requested_service) {
        return Some((requested_service.to_string(), *port));
    }

    let shim_service = format!("{requested_service}.shim.remote");
    rsd.services
        .get(&shim_service)
        .map(|ServiceDescriptor { port }| (shim_service, *port))
}

fn resolve_tunnel_connection_target(
    server_addr: &str,
    userspace_port: Option<u16>,
) -> Result<TunnelConnectionTarget, CoreError> {
    let remote_addr = Ipv6Addr::from_str(server_addr)
        .map_err(|e| CoreError::Other(format!("invalid IPv6 addr: {e}")))?;

    Ok(match userspace_port {
        Some(proxy_port) => TunnelConnectionTarget::UserspaceProxy {
            proxy_port,
            remote_addr,
        },
        None => TunnelConnectionTarget::DirectIpv6 { remote_addr },
    })
}

fn validate_rsd_checkin_response(
    response: plist::Value,
    expected_request: &str,
    context: &str,
) -> Result<(), CoreError> {
    let response = response.as_dictionary().ok_or_else(|| {
        CoreError::Other(format!(
            "{context} expected plist dictionary response, got {:?}",
            response
        ))
    })?;

    let actual_request = response
        .get("Request")
        .and_then(plist::Value::as_string)
        .ok_or_else(|| {
            CoreError::Other(format!(
                "{context} missing Request field in response: {:?}",
                response
            ))
        })?;

    if actual_request != expected_request {
        return Err(CoreError::Other(format!(
            "{context} expected Request={expected_request}, got {actual_request}"
        )));
    }

    if let Some(error) = response.get("Error") {
        return Err(CoreError::Other(format!(
            "{context} failed with Error={:?}",
            error
        )));
    }

    Ok(())
}

async fn rsd_checkin<S>(stream: &mut S) -> Result<(), CoreError>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    send_lockdown(
        stream,
        &RsdCheckinRequest {
            label: "ios-rs",
            protocol_version: "2",
            request: "RSDCheckin",
        },
    )
    .await
    .map_err(|e| CoreError::Other(e.to_string()))?;

    let checkin_response: plist::Value = recv_lockdown(stream)
        .await
        .map_err(|e| CoreError::Other(e.to_string()))?;
    validate_rsd_checkin_response(checkin_response, "RSDCheckin", "RSD check-in response")?;

    let start_service_response: plist::Value = recv_lockdown(stream)
        .await
        .map_err(|e| CoreError::Other(e.to_string()))?;
    validate_rsd_checkin_response(
        start_service_response,
        "StartService",
        "RSD start-service response",
    )?;
    Ok(())
}

// ── connect() ─────────────────────────────────────────────────────────────────

pub async fn connect(udid: &str, opts: ConnectOptions) -> Result<ConnectedDevice, CoreError> {
    let mut mux = MuxClient::connect().await?;
    let devices = mux.list_devices().await?;
    let dev = select_mux_device(devices, udid)
        .ok_or_else(|| CoreError::DeviceNotFound(udid.to_string()))?;

    let info = DeviceInfo {
        udid: dev.serial_number.clone(),
        device_id: dev.device_id,
        connection_type: dev.connection_type.clone(),
        product_id: dev.product_id,
    };

    let pair_record = load_pair_record(udid, opts.pair_record_path.as_deref())?;
    connect_via_lockdown_transport(
        info,
        pair_record,
        LockdownTransport::Usbmux {
            device_id: dev.device_id,
        },
        opts,
    )
    .await
}

pub async fn connect_direct_usb_tunnel(
    udid: &str,
    rsd_ip: Option<&str>,
    opts: ConnectOptions,
) -> Result<ConnectedDevice, CoreError> {
    let mut mux = MuxClient::connect().await?;
    let devices = mux.list_devices().await?;
    let dev = select_mux_device(devices, udid)
        .ok_or_else(|| CoreError::DeviceNotFound(udid.to_string()))?;
    let pair_record = try_load_pair_record(udid, opts.pair_record_path.as_deref());
    let info = DeviceInfo {
        udid: dev.serial_number.clone(),
        device_id: dev.device_id,
        connection_type: dev.connection_type.clone(),
        product_id: dev.product_id,
    };
    let lockdown_transport = LockdownTransport::Usbmux {
        device_id: dev.device_id,
    };

    if opts.skip_tunnel {
        let pair_record =
            require_pair_record(pair_record, udid, "direct USB lockdown access requires")?;
        return Ok(ConnectedDevice {
            info,
            tunnel: None,
            rsd: None,
            pair_record: Some(pair_record),
            lockdown_transport,
        });
    }

    let targets = discover_direct_rsd_targets(udid, rsd_ip).await?;
    if targets.is_empty() {
        return Err(CoreError::Unsupported(format!(
            "no _remoted target matched udid={udid} ip={rsd_ip:?}"
        )));
    }

    let mut last_error = None;
    for target in targets {
        match connect_via_direct_rsd_target(
            info.clone(),
            pair_record.clone(),
            lockdown_transport.clone(),
            opts.clone(),
            target,
        )
        .await
        {
            Ok(device) => return Ok(device),
            Err(err) => last_error = Some(err),
        }
    }

    Err(last_error.unwrap_or_else(|| {
        CoreError::Unsupported(format!(
            "no direct RSD target produced a tunnel for udid={udid}"
        ))
    }))
}

pub async fn connect_remote_pairing_tunnel(
    udid: &str,
    host: Option<&str>,
    opts: ConnectOptions,
) -> Result<ConnectedDevice, CoreError> {
    let pair_record = try_load_pair_record(udid, opts.pair_record_path.as_deref());
    let info = DeviceInfo {
        udid: udid.to_string(),
        device_id: 0,
        connection_type: "Network".into(),
        product_id: 0,
    };

    if opts.skip_tunnel {
        let pair_record =
            require_pair_record(pair_record, udid, "remote pairing lockdown access requires")?;
        return Ok(ConnectedDevice {
            info,
            tunnel: None,
            rsd: None,
            pair_record: Some(pair_record),
            lockdown_transport: LockdownTransport::Tcp {
                host: host.unwrap_or_default().to_string(),
            },
        });
    }

    let targets = discover_remote_pairing_targets(udid, host).await?;
    if targets.is_empty() {
        return Err(CoreError::Unsupported(format!(
            "no _remotepairing target matched udid={udid} host={host:?}"
        )));
    }

    let mut last_error = None;
    for (remote_host, port) in targets {
        match connect_via_remote_pairing_target(
            info.clone(),
            pair_record.clone(),
            opts.clone(),
            udid,
            &remote_host,
            port,
        )
        .await
        {
            Ok(device) => return Ok(device),
            Err(err) => last_error = Some(err),
        }
    }

    Err(last_error.unwrap_or_else(|| {
        CoreError::Unsupported(format!(
            "no remote pairing target produced a tunnel for udid={udid}"
        ))
    }))
}

pub async fn connect_tcp_lockdown_tunnel(
    udid: &str,
    host: &str,
    opts: ConnectOptions,
) -> Result<ConnectedDevice, CoreError> {
    let pair_record = load_pair_record(udid, opts.pair_record_path.as_deref())?;
    let info = DeviceInfo {
        udid: udid.to_string(),
        device_id: 0,
        connection_type: "Network".into(),
        product_id: 0,
    };
    connect_via_lockdown_transport(
        info,
        pair_record,
        LockdownTransport::Tcp {
            host: host.to_string(),
        },
        opts,
    )
    .await
}

pub async fn discover_paired_mobdev2_devices() -> Result<Vec<PairedMobdev2Device>, CoreError> {
    let wifi_mac_to_udid = tokio::task::spawn_blocking(load_wifi_mac_pairings)
        .await
        .map_err(|e| CoreError::Other(format!("join error: {e}")))??;
    let services = browse_mobdev2(MOBDEV2_DISCOVERY_TIMEOUT).await?;
    Ok(match_paired_mobdev2_targets(&services, &wifi_mac_to_udid))
}

fn select_mux_device(
    devices: Vec<crate::mux::MuxDevice>,
    udid: &str,
) -> Option<crate::mux::MuxDevice> {
    let mut fallback = None;

    for device in devices {
        if device.serial_number != udid {
            continue;
        }

        let is_usb = device.connection_type.eq_ignore_ascii_case("USB");
        fallback = Some(device);

        if is_usb {
            return fallback;
        }
    }

    fallback
}

fn load_pair_record(
    udid: &str,
    pair_record_path: Option<&std::path::Path>,
) -> Result<Arc<PairRecord>, CoreError> {
    Ok(Arc::new(if let Some(path) = pair_record_path {
        PairRecord::load_from_path(path, udid)?
    } else {
        PairRecord::load(udid)?
    }))
}

fn try_load_pair_record(
    udid: &str,
    pair_record_path: Option<&std::path::Path>,
) -> Option<Arc<PairRecord>> {
    load_pair_record(udid, pair_record_path).ok()
}

fn require_pair_record(
    pair_record: Option<Arc<PairRecord>>,
    udid: &str,
    context: &str,
) -> Result<Arc<PairRecord>, CoreError> {
    pair_record.ok_or_else(|| {
        CoreError::Unsupported(format!("{context} a lockdown pair record for {udid}"))
    })
}

async fn connect_lockdown_port(
    udid: &str,
    transport: &LockdownTransport,
    port: u16,
    read_pair_record: bool,
) -> Result<ServiceStream, CoreError> {
    match transport {
        LockdownTransport::Usbmux { device_id } => {
            let mut mux = MuxClient::connect().await?;
            if read_pair_record {
                mux.read_pair_record(udid).await?;
            }
            let stream = mux.connect_to_port(*device_id, port).await?;
            Ok(Box::new(stream))
        }
        LockdownTransport::Tcp { host, .. } => {
            let stream = TcpStream::connect((host.as_str(), port)).await?;
            Ok(Box::new(stream))
        }
    }
}

async fn connect_via_lockdown_transport(
    info: DeviceInfo,
    pair_record: Arc<PairRecord>,
    lockdown_transport: LockdownTransport,
    opts: ConnectOptions,
) -> Result<ConnectedDevice, CoreError> {
    if opts.skip_tunnel {
        return Ok(ConnectedDevice {
            info,
            tunnel: None,
            rsd: None,
            pair_record: Some(pair_record),
            lockdown_transport,
        });
    }

    let lockdown_stream =
        connect_lockdown_port(&info.udid, &lockdown_transport, LOCKDOWN_PORT, true).await?;

    tracing::info!("tunnel connect: starting lockdown session");
    let (_session_id, mut tls_reader, mut tls_writer) =
        start_lockdown_session(lockdown_stream, &pair_record).await?;
    tracing::info!("tunnel connect: lockdown session established");

    tracing::info!("tunnel connect: requesting CoreDeviceProxy");
    let (service_port, enable_service_ssl) =
        start_service(&mut tls_reader, &mut tls_writer, CORE_DEVICE_PROXY).await?;
    tracing::info!(
        "tunnel connect: CoreDeviceProxy started on port {service_port} (ssl={enable_service_ssl})"
    );

    let proxy_stream_raw =
        connect_lockdown_port(&info.udid, &lockdown_transport, service_port, false).await?;

    let mut proxy_stream = if enable_service_ssl {
        tracing::info!("tunnel connect: wrapping CoreDeviceProxy with TLS");
        ProxyStream::Tls(Box::new(
            wrap_service_tls(proxy_stream_raw, &pair_record)
                .await
                .map_err(|e| CoreError::Other(e.to_string()))?,
        ))
    } else {
        tracing::info!("tunnel connect: CoreDeviceProxy is plaintext");
        ProxyStream::Plain(proxy_stream_raw)
    };
    tracing::info!("tunnel connect: CoreDeviceProxy stream ready");

    tracing::info!(
        "tunnel connect: exchanging CDTunnel parameters (timeout={} ms)",
        TUNNEL_HANDSHAKE_TIMEOUT.as_millis()
    );
    let tunnel_info = crate::tunnel::handshake::exchange_tunnel_parameters_with_timeout(
        &mut proxy_stream,
        TUNNEL_HANDSHAKE_TIMEOUT,
    )
    .await
    .map_err(CoreError::Tunnel)?;
    tracing::info!("tunnel connect: CDTunnel parameters received");
    tracing::info!(
        "tunnel_info: server={} rsd_port={} client={} mtu={}",
        tunnel_info.server_address,
        tunnel_info.server_rsd_port,
        tunnel_info.client_address,
        tunnel_info.client_mtu
    );

    match opts.tun_mode {
        TunMode::Kernel => {
            let (handle, cancel_rx) =
                TunnelHandle::new(info.udid.clone(), tunnel_info.clone(), None);
            let tun = KernelTunDevice::create(&tunnel_info.client_address, tunnel_info.client_mtu)
                .await
                .map_err(CoreError::Tunnel)?;
            let mtu = tunnel_info.client_mtu;
            tokio::spawn(async move {
                if let Err(e) = forward_packets(proxy_stream, tun, mtu, cancel_rx).await {
                    tracing::error!("kernel TUN forward: {e}");
                }
            });
            let rsd = attempt_rsd(&tunnel_info.server_address, tunnel_info.server_rsd_port).await;
            Ok(ConnectedDevice {
                info,
                tunnel: Some(Arc::new(handle)),
                rsd,
                pair_record: Some(pair_record),
                lockdown_transport,
            })
        }
        TunMode::Userspace => {
            let userspace = UserspaceTunDevice::start(
                &tunnel_info.client_address,
                &tunnel_info.server_address,
                tunnel_info.client_mtu,
                proxy_stream,
            )
            .await
            .map_err(CoreError::Tunnel)?;

            let proxy_port = userspace.local_port;
            let handle =
                TunnelHandle::new_userspace(info.udid.clone(), tunnel_info.clone(), userspace);
            let rsd = attempt_rsd_via_proxy(
                proxy_port,
                &tunnel_info.server_address,
                tunnel_info.server_rsd_port,
            )
            .await;
            Ok(ConnectedDevice {
                info,
                tunnel: Some(Arc::new(handle)),
                rsd,
                pair_record: Some(pair_record),
                lockdown_transport,
            })
        }
    }
}

struct GuardedTunnelStream<G> {
    stream: tokio_openssl::SslStream<TcpStream>,
    _guard: G,
}

impl<G> Unpin for GuardedTunnelStream<G> {}

impl<G> AsyncRead for GuardedTunnelStream<G> {
    fn poll_read(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<std::io::Result<()>> {
        Pin::new(&mut self.get_mut().stream).poll_read(cx, buf)
    }
}

impl<G> AsyncWrite for GuardedTunnelStream<G> {
    fn poll_write(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<std::io::Result<usize>> {
        Pin::new(&mut self.get_mut().stream).poll_write(cx, buf)
    }

    fn poll_flush(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        Pin::new(&mut self.get_mut().stream).poll_flush(cx)
    }

    fn poll_shutdown(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        Pin::new(&mut self.get_mut().stream).poll_shutdown(cx)
    }
}

struct LoadedRemotePairingCredentials {
    host_identity: HostIdentity,
}

struct RemotePairingControlChannel {
    stream: TcpStream,
}

impl RemotePairingControlChannel {
    async fn connect(host: &str, port: u16) -> Result<Self, CoreError> {
        Ok(Self {
            stream: TcpStream::connect((host, port)).await?,
        })
    }

    async fn send(&mut self, payload: &serde_json::Value) -> Result<(), CoreError> {
        use tokio::io::AsyncWriteExt;

        let body = serde_json::to_vec(payload)
            .map_err(|e| CoreError::Other(format!("remote pairing JSON encode failed: {e}")))?;
        if body.len() > u16::MAX as usize {
            return Err(CoreError::Other(format!(
                "remote pairing payload too large: {} bytes",
                body.len()
            )));
        }

        self.stream.write_all(b"RPPairing").await?;
        self.stream
            .write_all(&(body.len() as u16).to_be_bytes())
            .await?;
        self.stream.write_all(&body).await?;
        self.stream.flush().await?;
        Ok(())
    }

    async fn recv(&mut self) -> Result<serde_json::Value, CoreError> {
        use tokio::io::AsyncReadExt;

        let mut magic = [0u8; 9];
        self.stream.read_exact(&mut magic).await?;
        if &magic != b"RPPairing" {
            return Err(CoreError::Other(format!(
                "invalid RPPairing magic: {magic:?}"
            )));
        }

        let mut length = [0u8; 2];
        self.stream.read_exact(&mut length).await?;
        let body_len = u16::from_be_bytes(length) as usize;
        let mut body = vec![0u8; body_len];
        self.stream.read_exact(&mut body).await?;
        serde_json::from_slice(&body)
            .map_err(|e| CoreError::Other(format!("remote pairing JSON decode failed: {e}")))
    }
}

async fn discover_direct_rsd_targets(
    udid: &str,
    ip_filter: Option<&str>,
) -> Result<Vec<MdnsDevice>, CoreError> {
    let stream = crate::discovery::discover_mdns().await?;
    tokio::pin!(stream);

    let deadline = Instant::now() + DIRECT_RSD_DISCOVERY_TIMEOUT;
    let mut targets = Vec::new();
    let mut seen = std::collections::HashSet::new();

    loop {
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            break;
        }

        match tokio::time::timeout(remaining, stream.next()).await {
            Ok(Some(device)) => {
                let ip = device.ipv6.to_string();
                if ip_filter.map(|filter| filter != ip).unwrap_or(false) {
                    continue;
                }

                let key = (device.ipv6, device.rsd_port);
                if !seen.insert(key) {
                    continue;
                }

                targets.push(device);
            }
            Ok(None) | Err(_) => break,
        }
    }

    targets.sort_by_key(|device| {
        if device.udid == udid {
            0
        } else if device.udid.is_empty() {
            1
        } else {
            2
        }
    });
    Ok(targets)
}

async fn discover_remote_pairing_targets(
    udid: &str,
    host_filter: Option<&str>,
) -> Result<Vec<(String, u16)>, CoreError> {
    let services = browse_remotepairing(MOBDEV2_DISCOVERY_TIMEOUT).await?;
    let mut targets = Vec::new();
    let mut seen = std::collections::HashSet::new();

    for service in services {
        let Some(host) = preferred_lockdown_address(&service.addresses) else {
            continue;
        };
        if host_filter.map(|filter| filter != host).unwrap_or(false) {
            continue;
        }

        let key = (host.to_string(), service.port);
        if seen.insert(key.clone()) {
            targets.push(key);
        }
    }

    if targets.is_empty() {
        return Err(CoreError::Unsupported(format!(
            "no browse_remotepairing target matched udid={udid} host={host_filter:?}"
        )));
    }

    Ok(targets)
}

async fn connect_via_direct_rsd_target(
    info: DeviceInfo,
    pair_record: Option<Arc<PairRecord>>,
    lockdown_transport: LockdownTransport,
    opts: ConnectOptions,
    target: MdnsDevice,
) -> Result<ConnectedDevice, CoreError> {
    let rsd = rsd_handshake(target.ipv6, target.rsd_port)
        .await
        .map_err(|e| CoreError::Other(format!("direct RSD handshake failed: {e}")))?;
    if rsd.udid != info.udid {
        return Err(CoreError::Other(format!(
            "direct RSD target {} resolved to unexpected udid {}",
            target.ipv6, rsd.udid
        )));
    }

    let service_port = rsd
        .get_port(crate::pairing_transport::UNTRUSTED_SERVICE_NAME)
        .ok_or_else(|| {
            CoreError::Unsupported(format!(
                "direct RSD target {} does not expose {}",
                target.ipv6,
                crate::pairing_transport::UNTRUSTED_SERVICE_NAME
            ))
        })?;
    let mut direct_stream = establish_direct_tunnel_stream(target.ipv6, service_port).await?;

    let tunnel_info = crate::tunnel::handshake::exchange_tunnel_parameters_with_timeout(
        &mut direct_stream,
        TUNNEL_HANDSHAKE_TIMEOUT,
    )
    .await
    .map_err(CoreError::Tunnel)?;

    match opts.tun_mode {
        TunMode::Kernel => {
            let (handle, cancel_rx) =
                TunnelHandle::new(info.udid.clone(), tunnel_info.clone(), None);
            let tun = KernelTunDevice::create(&tunnel_info.client_address, tunnel_info.client_mtu)
                .await
                .map_err(CoreError::Tunnel)?;
            let mtu = tunnel_info.client_mtu;
            tokio::spawn(async move {
                if let Err(err) = forward_packets(direct_stream, tun, mtu, cancel_rx).await {
                    tracing::error!("direct kernel TUN forward: {err}");
                }
            });
            let rsd = attempt_rsd(&tunnel_info.server_address, tunnel_info.server_rsd_port).await;
            Ok(ConnectedDevice {
                info,
                tunnel: Some(Arc::new(handle)),
                rsd,
                pair_record,
                lockdown_transport,
            })
        }
        TunMode::Userspace => {
            let userspace = UserspaceTunDevice::start(
                &tunnel_info.client_address,
                &tunnel_info.server_address,
                tunnel_info.client_mtu,
                direct_stream,
            )
            .await
            .map_err(CoreError::Tunnel)?;

            let proxy_port = userspace.local_port;
            let handle =
                TunnelHandle::new_userspace(info.udid.clone(), tunnel_info.clone(), userspace);
            let rsd = attempt_rsd_via_proxy(
                proxy_port,
                &tunnel_info.server_address,
                tunnel_info.server_rsd_port,
            )
            .await;
            Ok(ConnectedDevice {
                info,
                tunnel: Some(Arc::new(handle)),
                rsd,
                pair_record,
                lockdown_transport,
            })
        }
    }
}

async fn connect_via_remote_pairing_target(
    info: DeviceInfo,
    pair_record: Option<Arc<PairRecord>>,
    opts: ConnectOptions,
    remote_identifier: &str,
    host: &str,
    port: u16,
) -> Result<ConnectedDevice, CoreError> {
    let mut remote_stream =
        establish_remote_pairing_tunnel_stream(remote_identifier, host, port).await?;

    let tunnel_info = crate::tunnel::handshake::exchange_tunnel_parameters_with_timeout(
        &mut remote_stream,
        TUNNEL_HANDSHAKE_TIMEOUT,
    )
    .await
    .map_err(CoreError::Tunnel)?;

    match opts.tun_mode {
        TunMode::Kernel => {
            let (handle, cancel_rx) =
                TunnelHandle::new(info.udid.clone(), tunnel_info.clone(), None);
            let tun = KernelTunDevice::create(&tunnel_info.client_address, tunnel_info.client_mtu)
                .await
                .map_err(CoreError::Tunnel)?;
            let mtu = tunnel_info.client_mtu;
            tokio::spawn(async move {
                if let Err(err) = forward_packets(remote_stream, tun, mtu, cancel_rx).await {
                    tracing::error!("remote pairing kernel TUN forward: {err}");
                }
            });
            let rsd = attempt_rsd(&tunnel_info.server_address, tunnel_info.server_rsd_port).await;
            Ok(ConnectedDevice {
                info,
                tunnel: Some(Arc::new(handle)),
                rsd,
                pair_record,
                lockdown_transport: LockdownTransport::Tcp {
                    host: host.to_string(),
                },
            })
        }
        TunMode::Userspace => {
            let userspace = UserspaceTunDevice::start(
                &tunnel_info.client_address,
                &tunnel_info.server_address,
                tunnel_info.client_mtu,
                remote_stream,
            )
            .await
            .map_err(CoreError::Tunnel)?;

            let proxy_port = userspace.local_port;
            let handle =
                TunnelHandle::new_userspace(info.udid.clone(), tunnel_info.clone(), userspace);
            let rsd = attempt_rsd_via_proxy(
                proxy_port,
                &tunnel_info.server_address,
                tunnel_info.server_rsd_port,
            )
            .await;
            Ok(ConnectedDevice {
                info,
                tunnel: Some(Arc::new(handle)),
                rsd,
                pair_record,
                lockdown_transport: LockdownTransport::Tcp {
                    host: host.to_string(),
                },
            })
        }
    }
}

async fn establish_direct_tunnel_stream(
    rsd_addr: Ipv6Addr,
    service_port: u16,
) -> Result<GuardedTunnelStream<XpcClient>, CoreError> {
    let mut client = XpcClient::connect(rsd_addr, service_port)
        .await
        .map_err(|e| CoreError::Other(format!("direct tunnelservice connect failed: {e}")))?;
    let mut sequence_number = 0u64;

    client
        .send(build_direct_handshake_request(sequence_number))
        .await
        .map_err(|e| CoreError::Other(format!("direct handshake request failed: {e}")))?;
    sequence_number += 1;

    let handshake = client
        .recv()
        .await
        .map_err(|e| CoreError::Other(format!("direct handshake response failed: {e}")))?;
    let remote_identifier = extract_direct_remote_identifier(
        handshake
            .body
            .as_ref()
            .ok_or_else(|| CoreError::Other("direct handshake response missing body".into()))?,
    )?;

    let loaded = {
        let id = remote_identifier.clone();
        tokio::task::spawn_blocking(move || load_remote_pairing_credentials(&id))
            .await
            .map_err(|e| CoreError::Other(format!("spawn_blocking join error: {e}")))?
    }?;

    let mut our_secret = [0u8; 32];
    rand::thread_rng().fill_bytes(&mut our_secret);
    let static_secret = x25519_dalek::StaticSecret::from(our_secret);
    let our_public = x25519_dalek::PublicKey::from(&static_secret).to_bytes();

    client
        .send(build_direct_pairing_event(
            &build_verify_start_tlv(&our_public),
            "verifyManualPairing",
            true,
            None,
            sequence_number,
        ))
        .await
        .map_err(|e| CoreError::Other(format!("verifyManualPairing start failed: {e}")))?;
    sequence_number += 1;

    let verify_start = client
        .recv()
        .await
        .map_err(|e| CoreError::Other(format!("verifyManualPairing start response failed: {e}")))?;
    let verify_start_tlv = extract_direct_pairing_tlv(
        verify_start
            .body
            .as_ref()
            .ok_or_else(|| CoreError::Other("verifyManualPairing start missing body".into()))?,
    )?;
    let verify_start_fields = TlvBuffer::decode(&verify_start_tlv);
    if let Some(error) = verify_start_fields.get(&DIRECT_PAIRING_TYPE_ERROR) {
        send_pair_verify_failed(&mut client, sequence_number).await?;
        return Err(CoreError::Other(format!(
            "verifyManualPairing start rejected: {error:?}"
        )));
    }

    let device_public: [u8; 32] = verify_start_fields
        .get(&DIRECT_PAIRING_TYPE_PUBLIC_KEY)
        .ok_or_else(|| {
            CoreError::Other("verifyManualPairing start missing device public key".into())
        })?
        .as_ref()
        .try_into()
        .map_err(|_| {
            CoreError::Other("verifyManualPairing device public key must be 32 bytes".into())
        })?;

    let verify_session = build_verify_step2_tlv(
        our_secret,
        &our_public,
        &device_public,
        &loaded.host_identity,
    )
    .map_err(|e| CoreError::Other(format!("verifyManualPairing finish build failed: {e}")))?;

    client
        .send(build_direct_pairing_event(
            &verify_session.tlv,
            "verifyManualPairing",
            false,
            None,
            sequence_number,
        ))
        .await
        .map_err(|e| CoreError::Other(format!("verifyManualPairing finish failed: {e}")))?;
    sequence_number += 1;

    let verify_finish = client.recv().await.map_err(|e| {
        CoreError::Other(format!("verifyManualPairing finish response failed: {e}"))
    })?;
    let verify_finish_tlv = extract_direct_pairing_tlv(
        verify_finish
            .body
            .as_ref()
            .ok_or_else(|| CoreError::Other("verifyManualPairing finish missing body".into()))?,
    )?;
    let verify_finish_fields = TlvBuffer::decode(&verify_finish_tlv);
    if let Some(error) = verify_finish_fields.get(&DIRECT_PAIRING_TYPE_ERROR) {
        send_pair_verify_failed(&mut client, sequence_number).await?;
        return Err(CoreError::Other(format!(
            "verifyManualPairing finish rejected: {error:?}"
        )));
    }

    let listener_port =
        create_direct_tcp_listener(&mut client, &verify_session, sequence_number).await?;
    let stream = crate::psk_tls::connect_psk_tls(
        &rsd_addr.to_string(),
        listener_port,
        &verify_session.encryption_key,
    )
    .await
    .map_err(|e| CoreError::Other(format!("direct TLS-PSK listener connect failed: {e}")))?;

    Ok(GuardedTunnelStream {
        stream,
        _guard: client,
    })
}

async fn establish_remote_pairing_tunnel_stream(
    remote_identifier: &str,
    host: &str,
    port: u16,
) -> Result<GuardedTunnelStream<RemotePairingControlChannel>, CoreError> {
    let loaded = {
        let id = remote_identifier.to_owned();
        tokio::task::spawn_blocking(move || load_remote_pairing_credentials(&id))
            .await
            .map_err(|e| CoreError::Other(format!("spawn_blocking join error: {e}")))?
    }?;
    let mut control = RemotePairingControlChannel::connect(host, port).await?;
    let mut sequence_number = 0u64;

    control
        .send(&build_remote_pairing_handshake_request(sequence_number))
        .await?;
    sequence_number += 1;
    let _handshake = control.recv().await?;

    let mut our_secret = [0u8; 32];
    rand::thread_rng().fill_bytes(&mut our_secret);
    let static_secret = x25519_dalek::StaticSecret::from(our_secret);
    let our_public = x25519_dalek::PublicKey::from(&static_secret).to_bytes();

    control
        .send(&build_remote_pairing_pairing_event(
            &build_verify_start_tlv(&our_public),
            "verifyManualPairing",
            true,
            None,
            sequence_number,
        ))
        .await?;
    sequence_number += 1;

    let verify_start = control.recv().await?;
    let verify_start_tlv = extract_remote_pairing_tlv(&verify_start)?;
    let verify_start_fields = TlvBuffer::decode(&verify_start_tlv);
    if let Some(error) = verify_start_fields.get(&DIRECT_PAIRING_TYPE_ERROR) {
        control
            .send(&build_remote_pairing_pair_verify_failed_event(
                sequence_number,
            ))
            .await?;
        return Err(CoreError::Other(format!(
            "remote pairing verify start rejected: {error:?}"
        )));
    }

    let device_public: [u8; 32] = verify_start_fields
        .get(&DIRECT_PAIRING_TYPE_PUBLIC_KEY)
        .ok_or_else(|| {
            CoreError::Other("remote pairing verify start missing device public key".into())
        })?
        .as_ref()
        .try_into()
        .map_err(|_| {
            CoreError::Other("remote pairing device public key must be 32 bytes".into())
        })?;

    let verify_session = build_verify_step2_tlv(
        our_secret,
        &our_public,
        &device_public,
        &loaded.host_identity,
    )
    .map_err(|e| CoreError::Other(format!("remote pairing verify finish build failed: {e}")))?;

    control
        .send(&build_remote_pairing_pairing_event(
            &verify_session.tlv,
            "verifyManualPairing",
            false,
            None,
            sequence_number,
        ))
        .await?;
    sequence_number += 1;

    let verify_finish = control.recv().await?;
    let verify_finish_tlv = extract_remote_pairing_tlv(&verify_finish)?;
    let verify_finish_fields = TlvBuffer::decode(&verify_finish_tlv);
    if let Some(error) = verify_finish_fields.get(&DIRECT_PAIRING_TYPE_ERROR) {
        control
            .send(&build_remote_pairing_pair_verify_failed_event(
                sequence_number,
            ))
            .await?;
        return Err(CoreError::Other(format!(
            "remote pairing verify finish rejected: {error:?}"
        )));
    }

    let listener_port =
        create_remote_pairing_tcp_listener(&mut control, &verify_session, sequence_number).await?;
    let stream =
        crate::psk_tls::connect_psk_tls(host, listener_port, &verify_session.encryption_key)
            .await
            .map_err(|e| {
                CoreError::Other(format!(
                    "remote pairing TLS-PSK listener connect failed: {e}"
                ))
            })?;

    Ok(GuardedTunnelStream {
        stream,
        _guard: control,
    })
}

async fn send_pair_verify_failed(
    client: &mut XpcClient,
    sequence_number: u64,
) -> Result<(), CoreError> {
    client
        .send(build_direct_pair_verify_failed_event(sequence_number))
        .await
        .map_err(|e| CoreError::Other(format!("pairVerifyFailed send failed: {e}")))
}

fn load_remote_pairing_credentials(
    remote_identifier: &str,
) -> Result<LoadedRemotePairingCredentials, CoreError> {
    load_remote_pairing_credentials_from_dirs(
        remote_identifier,
        &PersistedCredentials::default_dir(),
        &PersistedCredentials::pymobiledevice3_dir(),
        &current_hostname(),
    )
}

fn load_remote_pairing_credentials_from_dirs(
    remote_identifier: &str,
    ios_rs_dir: &Path,
    pymobiledevice3_dir: &Path,
    hostname: &str,
) -> Result<LoadedRemotePairingCredentials, CoreError> {
    if let Some(remote_pair_record) =
        RemotePairingRecord::load_for_identifier(ios_rs_dir, remote_identifier)
    {
        if let Some(persisted) = find_persisted_host_identity(ios_rs_dir, remote_identifier) {
            return load_ios_rs_remote_pairing_credentials(
                remote_identifier,
                remote_pair_record,
                persisted,
            );
        }
    }

    if let Some(remote_pair_record) =
        RemotePairingRecord::load_for_identifier(pymobiledevice3_dir, remote_identifier)
    {
        return load_pymobiledevice3_remote_pairing_credentials(
            remote_identifier,
            hostname,
            remote_pair_record,
            pymobiledevice3_dir,
        );
    }

    if RemotePairingRecord::load_for_identifier(ios_rs_dir, remote_identifier).is_some() {
        return Err(CoreError::Unsupported(format!(
            "missing persisted host identity for remote identifier {remote_identifier}"
        )));
    }

    Err(CoreError::Unsupported(format!(
        "missing remote pairing record for {remote_identifier} in {} or {}",
        ios_rs_dir.display(),
        pymobiledevice3_dir.display()
    )))
}

fn find_persisted_host_identity(
    creds_dir: &Path,
    remote_identifier: &str,
) -> Option<PersistedCredentials> {
    PersistedCredentials::list(creds_dir)
        .into_iter()
        .find(|creds| creds.remote_identifier.as_deref() == Some(remote_identifier))
}

fn load_ios_rs_remote_pairing_credentials(
    remote_identifier: &str,
    remote_pair_record: RemotePairingRecord,
    persisted: PersistedCredentials,
) -> Result<LoadedRemotePairingCredentials, CoreError> {
    let host_private_key = remote_pair_record.private_key.clone();
    let host_identity =
        HostIdentity::from_private_key_bytes(persisted.host_identifier, &host_private_key)
            .map_err(|e| CoreError::Other(format!("invalid persisted host identity: {e}")))?;

    if host_identity.public_key_bytes() != remote_pair_record.public_key {
        return Err(CoreError::Other(format!(
            "persisted host key mismatch for remote identifier {remote_identifier}"
        )));
    }

    if let Some(host_private_key_hex) = persisted.host_private_key_hex {
        let persisted_private_key = hex::decode(host_private_key_hex)
            .map_err(|e| CoreError::Other(format!("invalid host private key hex: {e}")))?;
        if persisted_private_key != remote_pair_record.private_key {
            return Err(CoreError::Other(format!(
                "persisted host private key mismatch for remote identifier {remote_identifier}"
            )));
        }
    }

    Ok(LoadedRemotePairingCredentials { host_identity })
}

fn load_pymobiledevice3_remote_pairing_credentials(
    remote_identifier: &str,
    hostname: &str,
    remote_pair_record: RemotePairingRecord,
    creds_dir: &Path,
) -> Result<LoadedRemotePairingCredentials, CoreError> {
    let host_identifier = pymobiledevice3_host_identifier(hostname);
    let host_identity =
        HostIdentity::from_private_key_bytes(host_identifier, &remote_pair_record.private_key)
            .map_err(|e| {
                CoreError::Other(format!(
                    "invalid pymobiledevice3 remote pairing identity for {remote_identifier}: {e}"
                ))
            })?;

    if host_identity.public_key_bytes() != remote_pair_record.public_key {
        return Err(CoreError::Other(format!(
            "pymobiledevice3 host key mismatch for remote identifier {remote_identifier} in {}",
            creds_dir.display()
        )));
    }

    Ok(LoadedRemotePairingCredentials { host_identity })
}

fn current_hostname() -> String {
    std::env::var_os("COMPUTERNAME")
        .or_else(|| std::env::var_os("HOSTNAME"))
        .unwrap_or_default()
        .to_string_lossy()
        .into_owned()
}

fn pymobiledevice3_host_identifier(hostname: &str) -> String {
    const NAMESPACE_DNS: [u8; 16] = [
        0x6b, 0xa7, 0xb8, 0x10, 0x9d, 0xad, 0x11, 0xd1, 0x80, 0xb4, 0x00, 0xc0, 0x4f, 0xd4, 0x30,
        0xc8,
    ];

    let mut input = Vec::with_capacity(NAMESPACE_DNS.len() + hostname.len());
    input.extend_from_slice(&NAMESPACE_DNS);
    input.extend_from_slice(hostname.as_bytes());

    let mut bytes = md5::compute(&input).0.to_vec();
    bytes[6] = (bytes[6] & 0x0f) | 0x30;
    bytes[8] = (bytes[8] & 0x3f) | 0x80;

    format!(
        "{:02x}{:02x}{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}{:02x}{:02x}{:02x}{:02x}",
        bytes[0],
        bytes[1],
        bytes[2],
        bytes[3],
        bytes[4],
        bytes[5],
        bytes[6],
        bytes[7],
        bytes[8],
        bytes[9],
        bytes[10],
        bytes[11],
        bytes[12],
        bytes[13],
        bytes[14],
        bytes[15]
    )
    .to_uppercase()
}

fn build_direct_handshake_request(sequence_number: u64) -> XpcValue {
    build_direct_control_envelope(
        xpc_dict(&[(
            "plain",
            xpc_dict(&[(
                "_0",
                xpc_dict(&[(
                    "request",
                    xpc_dict(&[(
                        "_0",
                        xpc_dict(&[(
                            "handshake",
                            xpc_dict(&[(
                                "_0",
                                xpc_dict(&[
                                    (
                                        "hostOptions",
                                        xpc_dict(&[("attemptPairVerify", XpcValue::Bool(true))]),
                                    ),
                                    ("wireProtocolVersion", XpcValue::Int64(19)),
                                ]),
                            )]),
                        )]),
                    )]),
                )]),
            )]),
        )]),
        sequence_number,
    )
}

fn build_direct_pairing_event(
    tlv_data: &[u8],
    kind: &str,
    start_new_session: bool,
    sending_host: Option<&str>,
    sequence_number: u64,
) -> XpcValue {
    let mut pairs = vec![
        (
            "data",
            XpcValue::Data(bytes::Bytes::copy_from_slice(tlv_data)),
        ),
        ("kind", XpcValue::String(kind.to_string())),
        ("startNewSession", XpcValue::Bool(start_new_session)),
    ];
    if let Some(host) = sending_host {
        pairs.push(("sendingHost", XpcValue::String(host.to_string())));
    }

    build_direct_control_envelope(
        xpc_dict(&[(
            "plain",
            xpc_dict(&[(
                "_0",
                xpc_dict(&[(
                    "event",
                    xpc_dict(&[(
                        "_0",
                        xpc_dict(&[("pairingData", xpc_dict(&[("_0", xpc_dict(&pairs))]))]),
                    )]),
                )]),
            )]),
        )]),
        sequence_number,
    )
}

fn build_direct_pair_verify_failed_event(sequence_number: u64) -> XpcValue {
    build_direct_control_envelope(
        xpc_dict(&[(
            "plain",
            xpc_dict(&[(
                "_0",
                xpc_dict(&[(
                    "event",
                    xpc_dict(&[("_0", xpc_dict(&[("pairVerifyFailed", xpc_dict(&[]))]))]),
                )]),
            )]),
        )]),
        sequence_number,
    )
}

fn build_direct_control_envelope(message: XpcValue, sequence_number: u64) -> XpcValue {
    xpc_dict(&[
        (
            "mangledTypeName",
            XpcValue::String(DIRECT_CONTROL_CHANNEL_ENVELOPE_TYPE.to_string()),
        ),
        (
            "value",
            xpc_dict(&[
                ("message", message),
                (
                    "originatedBy",
                    XpcValue::String(DIRECT_CONTROL_CHANNEL_ORIGIN.to_string()),
                ),
                ("sequenceNumber", XpcValue::Uint64(sequence_number)),
            ]),
        ),
    ])
}

async fn create_direct_tcp_listener(
    client: &mut XpcClient,
    session: &VerifyPairSession,
    sequence_number: u64,
) -> Result<u16, CoreError> {
    let nonce = make_direct_encrypted_nonce(0);
    let request = serde_json::json!({
        "request": {
            "_0": {
                "createListener": {
                    "key": BASE64_STANDARD.encode(session.encryption_key),
                    "peerConnectionsInfo": [{
                        "owningPID": std::process::id(),
                        "owningProcessName": "CoreDeviceService",
                    }],
                    "transportProtocolType": "tcp",
                }
            }
        }
    });
    let client_cipher = chacha20poly1305::ChaCha20Poly1305::new((&session.client_key).into());
    let encrypted = client_cipher
        .encrypt((&nonce).into(), request.to_string().as_bytes())
        .map_err(|e| CoreError::Other(format!("createListener encrypt failed: {e}")))?;

    client
        .send(build_direct_control_envelope(
            xpc_dict(&[(
                "streamEncrypted",
                xpc_dict(&[("_0", XpcValue::Data(bytes::Bytes::from(encrypted)))]),
            )]),
            sequence_number,
        ))
        .await
        .map_err(|e| CoreError::Other(format!("createListener request failed: {e}")))?;

    let response = client
        .recv()
        .await
        .map_err(|e| CoreError::Other(format!("createListener response failed: {e}")))?;
    let encrypted_response = extract_direct_stream_encrypted(
        response
            .body
            .as_ref()
            .ok_or_else(|| CoreError::Other("createListener response missing body".into()))?,
    )?;
    let server_cipher = chacha20poly1305::ChaCha20Poly1305::new((&session.server_key).into());
    let plaintext = server_cipher
        .decrypt((&nonce).into(), encrypted_response.as_ref())
        .map_err(|e| CoreError::Other(format!("createListener decrypt failed: {e}")))?;
    let response: serde_json::Value = serde_json::from_slice(&plaintext)
        .map_err(|e| CoreError::Other(format!("invalid createListener JSON: {e}")))?;
    let response_body = response
        .get("response")
        .and_then(|value| value.get("_1"))
        .ok_or_else(|| CoreError::Other("createListener response missing response._1".into()))?;

    if let Some(message) = extract_direct_error_extended_message(response_body) {
        return Err(CoreError::Other(format!(
            "createListener returned errorExtended: {message}"
        )));
    }

    let port = response_body
        .get("createListener")
        .and_then(|value| value.get("port"))
        .and_then(serde_json::Value::as_u64)
        .ok_or_else(|| CoreError::Other("createListener response missing port".into()))?;
    u16::try_from(port)
        .ok()
        .filter(|port| *port != 0)
        .ok_or_else(|| CoreError::Other(format!("invalid createListener port {port}")))
}

async fn create_remote_pairing_tcp_listener(
    control: &mut RemotePairingControlChannel,
    session: &VerifyPairSession,
    sequence_number: u64,
) -> Result<u16, CoreError> {
    let nonce = make_direct_encrypted_nonce(0);
    let request = serde_json::json!({
        "request": {
            "_0": {
                "createListener": {
                    "key": BASE64_STANDARD.encode(session.encryption_key),
                    "peerConnectionsInfo": [{
                        "owningPID": std::process::id(),
                        "owningProcessName": "CoreDeviceService",
                    }],
                    "transportProtocolType": "tcp",
                }
            }
        }
    });
    let client_cipher = chacha20poly1305::ChaCha20Poly1305::new((&session.client_key).into());
    let encrypted = client_cipher
        .encrypt((&nonce).into(), request.to_string().as_bytes())
        .map_err(|e| {
            CoreError::Other(format!("remote pairing createListener encrypt failed: {e}"))
        })?;

    control
        .send(&serde_json::json!({
            "message": {
                "streamEncrypted": {
                    "_0": BASE64_STANDARD.encode(encrypted),
                }
            },
            "originatedBy": DIRECT_CONTROL_CHANNEL_ORIGIN,
            "sequenceNumber": sequence_number,
        }))
        .await?;

    let response = control.recv().await?;
    let encrypted_response = extract_remote_pairing_stream_encrypted(&response)?;
    let server_cipher = chacha20poly1305::ChaCha20Poly1305::new((&session.server_key).into());
    let plaintext = server_cipher
        .decrypt((&nonce).into(), encrypted_response.as_ref())
        .map_err(|e| {
            CoreError::Other(format!("remote pairing createListener decrypt failed: {e}"))
        })?;
    let response: serde_json::Value = serde_json::from_slice(&plaintext).map_err(|e| {
        CoreError::Other(format!("invalid remote pairing createListener JSON: {e}"))
    })?;
    let response_body = response
        .get("response")
        .and_then(|value| value.get("_1"))
        .ok_or_else(|| {
            CoreError::Other("remote pairing createListener response missing response._1".into())
        })?;

    if let Some(message) = extract_direct_error_extended_message(response_body) {
        return Err(CoreError::Other(format!(
            "remote pairing createListener returned errorExtended: {message}"
        )));
    }

    let port = response_body
        .get("createListener")
        .and_then(|value| value.get("port"))
        .and_then(serde_json::Value::as_u64)
        .ok_or_else(|| {
            CoreError::Other("remote pairing createListener response missing port".into())
        })?;
    u16::try_from(port)
        .ok()
        .filter(|port| *port != 0)
        .ok_or_else(|| {
            CoreError::Other(format!("invalid remote pairing createListener port {port}"))
        })
}

fn xpc_dict(pairs: &[(&str, XpcValue)]) -> XpcValue {
    let mut map = IndexMap::new();
    for (key, value) in pairs {
        map.insert((*key).to_string(), value.clone());
    }
    XpcValue::Dictionary(map)
}

fn extract_direct_remote_identifier(body: &XpcValue) -> Result<String, CoreError> {
    direct_plain_message(body)?
        .get("response")
        .and_then(XpcValue::as_dict)
        .and_then(|response| response.get("_1"))
        .and_then(XpcValue::as_dict)
        .and_then(|response| response.get("handshake"))
        .and_then(XpcValue::as_dict)
        .and_then(|handshake| handshake.get("_0"))
        .and_then(XpcValue::as_dict)
        .and_then(|handshake| handshake.get("peerDeviceInfo"))
        .and_then(XpcValue::as_dict)
        .and_then(|peer| peer.get("identifier"))
        .and_then(XpcValue::as_str)
        .map(ToOwned::to_owned)
        .ok_or_else(|| CoreError::Other("handshake missing peerDeviceInfo.identifier".into()))
}

fn build_remote_pairing_handshake_request(sequence_number: u64) -> serde_json::Value {
    serde_json::json!({
        "message": {
            "plain": {
                "_0": {
                    "request": {
                        "_0": {
                            "handshake": {
                                "_0": {
                                    "hostOptions": {
                                        "attemptPairVerify": true,
                                    },
                                    "wireProtocolVersion": 19,
                                }
                            }
                        }
                    }
                }
            }
        },
        "originatedBy": DIRECT_CONTROL_CHANNEL_ORIGIN,
        "sequenceNumber": sequence_number,
    })
}

fn build_remote_pairing_pairing_event(
    tlv_data: &[u8],
    kind: &str,
    start_new_session: bool,
    sending_host: Option<&str>,
    sequence_number: u64,
) -> serde_json::Value {
    let mut body = serde_json::Map::new();
    body.insert(
        "data".into(),
        serde_json::Value::String(BASE64_STANDARD.encode(tlv_data)),
    );
    body.insert("kind".into(), serde_json::Value::String(kind.to_string()));
    body.insert(
        "startNewSession".into(),
        serde_json::Value::Bool(start_new_session),
    );
    if let Some(host) = sending_host {
        body.insert(
            "sendingHost".into(),
            serde_json::Value::String(host.to_string()),
        );
    }

    serde_json::json!({
        "message": {
            "plain": {
                "_0": {
                    "event": {
                        "_0": {
                            "pairingData": {
                                "_0": serde_json::Value::Object(body),
                            }
                        }
                    }
                }
            }
        },
        "originatedBy": DIRECT_CONTROL_CHANNEL_ORIGIN,
        "sequenceNumber": sequence_number,
    })
}

fn build_remote_pairing_pair_verify_failed_event(sequence_number: u64) -> serde_json::Value {
    serde_json::json!({
        "message": {
            "plain": {
                "_0": {
                    "event": {
                        "_0": {
                            "pairVerifyFailed": {}
                        }
                    }
                }
            }
        },
        "originatedBy": DIRECT_CONTROL_CHANNEL_ORIGIN,
        "sequenceNumber": sequence_number,
    })
}

fn extract_direct_pairing_tlv(body: &XpcValue) -> Result<Vec<u8>, CoreError> {
    let event = direct_plain_message(body)?
        .get("event")
        .and_then(XpcValue::as_dict)
        .and_then(|event| event.get("_0"))
        .and_then(XpcValue::as_dict)
        .ok_or_else(|| CoreError::Other("pairing response missing event._0".into()))?;

    if let Some(message) = event
        .get("pairingRejectedWithError")
        .and_then(extract_direct_rejection_message)
    {
        return Err(CoreError::Other(format!("pairing rejected: {message}")));
    }

    event
        .get("pairingData")
        .and_then(XpcValue::as_dict)
        .and_then(|pairing| pairing.get("_0"))
        .and_then(XpcValue::as_dict)
        .and_then(|pairing| pairing.get("data"))
        .and_then(|value| match value {
            XpcValue::Data(bytes) => Some(bytes.to_vec()),
            _ => None,
        })
        .ok_or_else(|| CoreError::Other("pairing response missing pairingData._0.data".into()))
}

fn extract_remote_pairing_tlv(body: &serde_json::Value) -> Result<Vec<u8>, CoreError> {
    let event = body
        .get("message")
        .and_then(|value| value.get("plain"))
        .and_then(|value| value.get("_0"))
        .and_then(|value| value.get("event"))
        .and_then(|value| value.get("_0"))
        .ok_or_else(|| {
            CoreError::Other("remote pairing response missing message.plain._0.event._0".into())
        })?;

    if let Some(message) = event
        .get("pairingRejectedWithError")
        .and_then(extract_remote_pairing_rejection_message)
    {
        return Err(CoreError::Other(format!(
            "remote pairing rejected: {message}"
        )));
    }

    let data = event
        .get("pairingData")
        .and_then(|value| value.get("_0"))
        .and_then(|value| value.get("data"))
        .and_then(serde_json::Value::as_str)
        .ok_or_else(|| {
            CoreError::Other("remote pairing response missing pairingData._0.data".into())
        })?;
    BASE64_STANDARD
        .decode(data)
        .map_err(|e| CoreError::Other(format!("invalid remote pairing TLV base64: {e}")))
}

fn extract_direct_stream_encrypted(body: &XpcValue) -> Result<Vec<u8>, CoreError> {
    direct_control_value(body)?
        .get("message")
        .and_then(XpcValue::as_dict)
        .and_then(|message| message.get("streamEncrypted"))
        .and_then(XpcValue::as_dict)
        .and_then(|encrypted| encrypted.get("_0"))
        .and_then(|value| match value {
            XpcValue::Data(bytes) => Some(bytes.to_vec()),
            _ => None,
        })
        .ok_or_else(|| {
            CoreError::Other("encrypted response missing message.streamEncrypted._0".into())
        })
}

fn extract_remote_pairing_stream_encrypted(body: &serde_json::Value) -> Result<Vec<u8>, CoreError> {
    let encoded = body
        .get("message")
        .and_then(|value| value.get("streamEncrypted"))
        .and_then(|value| value.get("_0"))
        .and_then(serde_json::Value::as_str)
        .ok_or_else(|| {
            CoreError::Other(
                "remote pairing encrypted response missing message.streamEncrypted._0".into(),
            )
        })?;
    BASE64_STANDARD.decode(encoded).map_err(|e| {
        CoreError::Other(format!(
            "invalid remote pairing encrypted payload base64: {e}"
        ))
    })
}

fn direct_control_value(body: &XpcValue) -> Result<&IndexMap<String, XpcValue>, CoreError> {
    let envelope = body.as_dict().ok_or_else(|| {
        CoreError::Other("direct control message body must be a dictionary".into())
    })?;
    let mangled_type = envelope
        .get("mangledTypeName")
        .and_then(XpcValue::as_str)
        .ok_or_else(|| CoreError::Other("direct control message missing mangledTypeName".into()))?;
    if mangled_type != DIRECT_CONTROL_CHANNEL_ENVELOPE_TYPE {
        return Err(CoreError::Other(format!(
            "unexpected direct control channel type {mangled_type}"
        )));
    }
    envelope
        .get("value")
        .and_then(XpcValue::as_dict)
        .ok_or_else(|| CoreError::Other("direct control message missing value".into()))
}

fn direct_plain_message(body: &XpcValue) -> Result<&IndexMap<String, XpcValue>, CoreError> {
    direct_control_value(body)?
        .get("message")
        .and_then(XpcValue::as_dict)
        .and_then(|message| message.get("plain"))
        .and_then(XpcValue::as_dict)
        .and_then(|plain| plain.get("_0"))
        .and_then(XpcValue::as_dict)
        .ok_or_else(|| CoreError::Other("direct control message missing message.plain._0".into()))
}

fn extract_direct_rejection_message(value: &XpcValue) -> Option<String> {
    value
        .as_dict()
        .and_then(|wrapped| wrapped.get("wrappedError"))
        .and_then(XpcValue::as_dict)
        .and_then(|wrapped| wrapped.get("userInfo"))
        .and_then(XpcValue::as_dict)
        .and_then(|user_info| user_info.get("NSLocalizedDescription"))
        .and_then(XpcValue::as_str)
        .map(ToOwned::to_owned)
}

fn extract_remote_pairing_rejection_message(value: &serde_json::Value) -> Option<String> {
    value
        .get("wrappedError")
        .and_then(|wrapped| wrapped.get("userInfo"))
        .and_then(|user_info| user_info.get("NSLocalizedDescription"))
        .and_then(serde_json::Value::as_str)
        .map(ToOwned::to_owned)
}

fn extract_direct_error_extended_message(value: &serde_json::Value) -> Option<String> {
    value
        .get("errorExtended")
        .and_then(|value| value.get("_0"))
        .and_then(|value| value.get("userInfo"))
        .and_then(|value| value.get("NSLocalizedDescription"))
        .and_then(serde_json::Value::as_str)
        .map(ToOwned::to_owned)
}

fn make_direct_encrypted_nonce(sequence_number: u64) -> [u8; 12] {
    let mut nonce = [0u8; 12];
    nonce[..8].copy_from_slice(&sequence_number.to_le_bytes());
    nonce
}

fn load_wifi_mac_pairings() -> Result<HashMap<String, String>, CoreError> {
    let mut wifi_mac_to_udid = HashMap::new();
    let pair_record_dir = default_pair_record_dir();

    for entry in std::fs::read_dir(pair_record_dir)? {
        let entry = entry?;
        let path = entry.path();
        if !path.is_file() || path.extension().and_then(|ext| ext.to_str()) != Some("plist") {
            continue;
        }

        let Some(udid) = path.file_stem().and_then(|stem| stem.to_str()) else {
            continue;
        };
        if udid.starts_with("remote_") {
            continue;
        }

        let record = PairRecord::load_from_path(&path, udid)?;
        let Some(mac) = record.wifi_mac_address else {
            continue;
        };
        wifi_mac_to_udid.insert(mac.to_ascii_lowercase(), udid.to_string());
    }

    Ok(wifi_mac_to_udid)
}

fn match_paired_mobdev2_targets(
    services: &[BonjourService],
    wifi_mac_to_udid: &HashMap<String, String>,
) -> Vec<PairedMobdev2Device> {
    let mut targets = Vec::new();
    let mut seen = std::collections::HashSet::<(String, String)>::new();

    for service in services {
        let Some(mac) = mobdev2_wifi_mac(&service.instance) else {
            continue;
        };
        let Some(udid) = wifi_mac_to_udid.get(&mac.to_ascii_lowercase()) else {
            continue;
        };
        let Some(host) = preferred_lockdown_address(&service.addresses) else {
            continue;
        };

        let key = (udid.clone(), host.to_string());
        if seen.insert(key.clone()) {
            targets.push(PairedMobdev2Device {
                udid: key.0,
                host: key.1,
            });
        }
    }

    targets
}

fn preferred_lockdown_address(addresses: &[String]) -> Option<&str> {
    addresses
        .iter()
        .find(|address| address.parse::<std::net::Ipv4Addr>().is_ok())
        .map(String::as_str)
        .or_else(|| {
            addresses
                .iter()
                .find(|address| {
                    !address.contains('%') && !address.to_ascii_lowercase().starts_with("fe80:")
                })
                .map(String::as_str)
        })
        .or_else(|| addresses.first().map(String::as_str))
}

/// Attempt RSD handshake; returns None on failure (e.g. iOS <17).
async fn attempt_rsd(server_addr: &str, rsd_port: u16) -> Option<RsdHandshake> {
    let addr = Ipv6Addr::from_str(server_addr).ok()?;
    match rsd_handshake(addr, rsd_port).await {
        Ok(h) => {
            tracing::info!(
                "RSD: {} services discovered for {}",
                h.services.len(),
                h.udid
            );
            Some(h)
        }
        Err(e) => {
            tracing::debug!("RSD handshake failed (may be iOS <17): {e}");
            None
        }
    }
}

/// Attempt RSD via go-ios-compatible userspace proxy.
async fn attempt_rsd_via_proxy(
    proxy_port: u16,
    server_addr: &str,
    rsd_port: u16,
) -> Option<RsdHandshake> {
    tracing::info!(
        "RSD via proxy: probing [{server_addr}]:{rsd_port} through proxy port {proxy_port}"
    );

    let mut framer = match open_rsd_proxy_framer(proxy_port, server_addr, rsd_port).await {
        Some(framer) => framer,
        None => return None,
    };

    match tokio::time::timeout(
        Duration::from_secs(3),
        crate::xpc::rsd::queue_rsd_handshake_bootstrap_on_framer(&mut framer),
    )
    .await
    {
        Ok(Ok(())) => match tokio::time::timeout(
            Duration::from_secs(4),
            crate::xpc::rsd::handshake_on_framer(&mut framer),
        )
        .await
        {
            Ok(Ok(handshake)) => {
                tracing::info!(
                    "RSD via proxy: queued bootstrap succeeded with {} services for {}",
                    handshake.services.len(),
                    handshake.udid
                );
                return Some(handshake);
            }
            Ok(Err(e)) => {
                tracing::warn!(
                    "RSD via proxy: queued bootstrap handshake failed: {e}; trying legacy bootstrap"
                );
            }
            Err(_) => {
                tracing::warn!(
                    "RSD via proxy: queued bootstrap handshake timed out; trying legacy bootstrap"
                );
            }
        },
        Ok(Err(e)) => {
            tracing::warn!("RSD via proxy: queued bootstrap failed: {e}; trying legacy bootstrap");
        }
        Err(_) => {
            tracing::warn!("RSD via proxy: queued bootstrap timed out; trying legacy bootstrap");
        }
    }

    let mut framer = match open_rsd_proxy_framer(proxy_port, server_addr, rsd_port).await {
        Some(framer) => framer,
        None => return None,
    };

    match tokio::time::timeout(
        Duration::from_secs(3),
        crate::xpc::rsd::initialize_xpc_connection_on_framer(&mut framer),
    )
    .await
    {
        Ok(Ok(())) => match tokio::time::timeout(
            Duration::from_secs(3),
            crate::xpc::rsd::handshake_on_framer(&mut framer),
        )
        .await
        {
            Ok(Ok(h)) => {
                tracing::info!(
                    "RSD via proxy: legacy bootstrap succeeded with {} services for {}",
                    h.services.len(),
                    h.udid
                );
                Some(h)
            }
            Ok(Err(e)) => {
                tracing::warn!(
                    "RSD handshake via proxy after legacy bootstrap: {e}; trying passive fallback"
                );
                match tokio::time::timeout(
                    Duration::from_secs(2),
                    crate::xpc::rsd::handshake_on_framer(&mut framer),
                )
                .await
                {
                    Ok(Ok(h)) => {
                        tracing::info!(
                            "RSD via proxy (passive fallback): {} services for {}",
                            h.services.len(),
                            h.udid
                        );
                        Some(h)
                    }
                    Ok(Err(e)) => {
                        tracing::warn!("RSD passive fallback failed: {e}");
                        None
                    }
                    Err(_) => {
                        tracing::warn!("RSD passive fallback timed out");
                        None
                    }
                }
            }
            Err(_) => {
                tracing::warn!("RSD handshake via proxy timed out after legacy bootstrap");
                None
            }
        },
        Ok(Err(e)) => {
            tracing::warn!("RSD legacy bootstrap failed: {e}; trying passive fallback");
            match tokio::time::timeout(
                Duration::from_secs(2),
                crate::xpc::rsd::handshake_on_framer(&mut framer),
            )
            .await
            {
                Ok(Ok(h)) => {
                    tracing::info!(
                        "RSD via proxy (passive fallback): {} services for {}",
                        h.services.len(),
                        h.udid
                    );
                    Some(h)
                }
                Ok(Err(e)) => {
                    tracing::warn!("RSD passive fallback failed: {e}");
                    None
                }
                Err(_) => {
                    tracing::warn!("RSD passive fallback timed out");
                    None
                }
            }
        }
        Err(_) => {
            tracing::warn!("RSD legacy bootstrap timed out; trying passive fallback");
            match tokio::time::timeout(
                Duration::from_secs(2),
                crate::xpc::rsd::handshake_on_framer(&mut framer),
            )
            .await
            {
                Ok(Ok(h)) => {
                    tracing::info!(
                        "RSD via proxy (passive fallback): {} services for {}",
                        h.services.len(),
                        h.udid
                    );
                    Some(h)
                }
                Ok(Err(e)) => {
                    tracing::warn!("RSD passive fallback failed: {e}");
                    None
                }
                Err(_) => {
                    tracing::warn!("RSD passive fallback timed out");
                    None
                }
            }
        }
    }
}

async fn open_rsd_proxy_framer(
    proxy_port: u16,
    server_addr: &str,
    rsd_port: u16,
) -> Option<crate::xpc::h2_raw::H2Framer<tokio::net::TcpStream>> {
    use tokio::io::AsyncWriteExt;
    use tokio::net::TcpStream;

    tracing::info!("RSD via proxy: connecting to 127.0.0.1:{proxy_port}");
    let mut proxy = match TcpStream::connect(format!("127.0.0.1:{proxy_port}")).await {
        Ok(stream) => {
            tracing::info!("RSD via proxy: connected to proxy");
            stream
        }
        Err(e) => {
            tracing::warn!("RSD proxy connect failed: {e}");
            return None;
        }
    };

    let addr_bytes = match Ipv6Addr::from_str(server_addr) {
        Ok(addr) => addr.octets(),
        Err(e) => {
            tracing::warn!("RSD bad server addr '{server_addr}': {e}");
            return None;
        }
    };

    if let Err(e) = proxy.write_all(&addr_bytes).await {
        tracing::warn!("RSD write addr: {e}");
        return None;
    }
    if let Err(e) = proxy.write_all(&(rsd_port as u32).to_le_bytes()).await {
        tracing::warn!("RSD write port: {e}");
        return None;
    }
    if let Err(e) = proxy.flush().await {
        tracing::warn!("RSD flush header: {e}");
        return None;
    }

    tracing::info!(
        "RSD via proxy: connecting to [{server_addr}]:{rsd_port} through proxy port {proxy_port}"
    );
    tracing::info!("RSD via proxy: starting H2 framer connect");
    match crate::xpc::h2_raw::H2Framer::connect(proxy).await {
        Ok(framer) => {
            tracing::info!("RSD via proxy: H2 framer connected");
            Some(framer)
        }
        Err(e) => {
            tracing::warn!("RSD H2 framer: {e}");
            None
        }
    }
}

// ── ProxyStream ───────────────────────────────────────────────────────────────

pub(crate) enum ProxyStream {
    Plain(ServiceStream),
    Tls(Box<tokio_rustls::client::TlsStream<ServiceStream>>),
}

impl Unpin for ProxyStream {}

impl AsyncRead for ProxyStream {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<std::io::Result<()>> {
        match &mut *self {
            ProxyStream::Plain(s) => Pin::new(s).poll_read(cx, buf),
            ProxyStream::Tls(s) => Pin::new(s).poll_read(cx, buf),
        }
    }
}

impl AsyncWrite for ProxyStream {
    fn poll_write(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<std::io::Result<usize>> {
        match &mut *self {
            ProxyStream::Plain(s) => Pin::new(s).poll_write(cx, buf),
            ProxyStream::Tls(s) => Pin::new(s).poll_write(cx, buf),
        }
    }
    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        match &mut *self {
            ProxyStream::Plain(s) => Pin::new(s).poll_flush(cx),
            ProxyStream::Tls(s) => Pin::new(s).poll_flush(cx),
        }
    }
    fn poll_shutdown(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        match &mut *self {
            ProxyStream::Plain(s) => Pin::new(s).poll_shutdown(cx),
            ProxyStream::Tls(s) => Pin::new(s).poll_shutdown(cx),
        }
    }
}

fn plist_value_to_string(value: &plist::Value, field: &str) -> Result<String, CoreError> {
    value
        .as_string()
        .map(ToOwned::to_owned)
        .ok_or_else(|| CoreError::Other(format!("{field} expected string value, got {:?}", value)))
}

fn plist_value_to_string_vec(value: &plist::Value, field: &str) -> Result<Vec<String>, CoreError> {
    let values = value.as_array().ok_or_else(|| {
        CoreError::Other(format!(
            "{field} expected string array value, got {:?}",
            value
        ))
    })?;

    values
        .iter()
        .map(|item| {
            item.as_string().map(ToOwned::to_owned).ok_or_else(|| {
                CoreError::Other(format!("{field} expected string entries, got {:?}", item))
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::path::PathBuf;

    use tokio::io::duplex;

    use super::*;

    fn temp_test_dir(label: &str) -> PathBuf {
        let unique = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("system time should be after unix epoch")
            .as_nanos();
        std::env::temp_dir().join(format!("ios_core_device_{label}_{unique}"))
    }

    fn make_remote_pair_record(identity: &HostIdentity) -> RemotePairingRecord {
        RemotePairingRecord {
            public_key: identity.public_key_bytes(),
            private_key: identity.private_key_bytes(),
            remote_unlock_host_key: None,
        }
    }

    #[test]
    fn try_load_pair_record_returns_none_for_missing_pair_record() {
        let missing_dir = temp_test_dir("missing_pair_record");

        let loaded = try_load_pair_record("missing-udid", Some(&missing_dir));

        assert!(loaded.is_none());

        let _ = std::fs::remove_dir_all(missing_dir);
    }

    #[test]
    fn require_pair_record_rejects_missing_lockdown_pair_record() {
        let err = require_pair_record(None, "test-udid", "remote pairing lockdown access requires")
            .expect_err("missing pair record should fail");

        assert!(err
            .to_string()
            .contains("remote pairing lockdown access requires"));
        assert!(err.to_string().contains("test-udid"));
    }

    #[test]
    fn load_remote_pairing_credentials_accepts_legacy_ios_rs_without_private_key_hex() {
        let base_dir = temp_test_dir("legacy_ios_rs");
        let ios_rs_dir = base_dir.join("ios-rs");
        let pymobiledevice3_dir = base_dir.join(".pymobiledevice3");
        let remote_identifier = "test-remote";
        let identity = HostIdentity::generate();

        make_remote_pair_record(&identity)
            .save_for_identifier(&ios_rs_dir, remote_identifier)
            .unwrap();
        PersistedCredentials {
            remote_identifier: Some(remote_identifier.into()),
            host_identifier: identity.identifier.clone(),
            host_public_key_hex: hex::encode(identity.public_key_bytes()),
            host_private_key_hex: None,
            remote_unlock_host_key: None,
            device_address: "fd00::1".into(),
            rsd_port: 58783,
        }
        .save(&ios_rs_dir)
        .unwrap();

        let loaded = load_remote_pairing_credentials_from_dirs(
            remote_identifier,
            &ios_rs_dir,
            &pymobiledevice3_dir,
            "unused-hostname",
        )
        .expect("legacy ios-rs credentials should load from remote pair record");

        assert_eq!(loaded.host_identity.identifier, identity.identifier);
        assert_eq!(
            loaded.host_identity.public_key_bytes(),
            identity.public_key_bytes()
        );

        let _ = std::fs::remove_dir_all(base_dir);
    }

    #[test]
    fn load_remote_pairing_credentials_prefers_ios_rs_over_pymobiledevice3() {
        let base_dir = temp_test_dir("prefers_ios_rs");
        let ios_rs_dir = base_dir.join("ios-rs");
        let pymobiledevice3_dir = base_dir.join(".pymobiledevice3");
        let remote_identifier = "test-remote";
        let ios_rs_identity = HostIdentity::generate();
        let fallback_identity = HostIdentity::from_private_key_bytes(
            pymobiledevice3_host_identifier("example-host"),
            &[0x44; 32],
        )
        .unwrap();

        make_remote_pair_record(&ios_rs_identity)
            .save_for_identifier(&ios_rs_dir, remote_identifier)
            .unwrap();
        PersistedCredentials {
            remote_identifier: Some(remote_identifier.into()),
            host_identifier: ios_rs_identity.identifier.clone(),
            host_public_key_hex: hex::encode(ios_rs_identity.public_key_bytes()),
            host_private_key_hex: Some(hex::encode(ios_rs_identity.private_key_bytes())),
            remote_unlock_host_key: None,
            device_address: "fd00::1".into(),
            rsd_port: 58783,
        }
        .save(&ios_rs_dir)
        .unwrap();
        make_remote_pair_record(&fallback_identity)
            .save_for_identifier(&pymobiledevice3_dir, remote_identifier)
            .unwrap();

        let loaded = load_remote_pairing_credentials_from_dirs(
            remote_identifier,
            &ios_rs_dir,
            &pymobiledevice3_dir,
            "example-host",
        )
        .expect("ios-rs credentials should take precedence");

        assert_eq!(loaded.host_identity.identifier, ios_rs_identity.identifier);
        assert_eq!(
            loaded.host_identity.public_key_bytes(),
            ios_rs_identity.public_key_bytes()
        );

        let _ = std::fs::remove_dir_all(base_dir);
    }

    #[test]
    fn load_remote_pairing_credentials_falls_back_to_pymobiledevice3_remote_record() {
        let base_dir = temp_test_dir("pymobiledevice3_fallback");
        let ios_rs_dir = base_dir.join("ios-rs");
        let pymobiledevice3_dir = base_dir.join(".pymobiledevice3");
        let remote_identifier = "test-remote";
        let hostname = "example-host";
        let expected_identity = HostIdentity::from_private_key_bytes(
            pymobiledevice3_host_identifier(hostname),
            &[0x22; 32],
        )
        .unwrap();

        make_remote_pair_record(&expected_identity)
            .save_for_identifier(&pymobiledevice3_dir, remote_identifier)
            .unwrap();

        let loaded = load_remote_pairing_credentials_from_dirs(
            remote_identifier,
            &ios_rs_dir,
            &pymobiledevice3_dir,
            hostname,
        )
        .expect("pymobiledevice3 remote record should be usable as fallback");

        assert_eq!(
            loaded.host_identity.identifier,
            pymobiledevice3_host_identifier(hostname)
        );
        assert_eq!(
            loaded.host_identity.public_key_bytes(),
            expected_identity.public_key_bytes()
        );

        let _ = std::fs::remove_dir_all(base_dir);
    }

    #[test]
    fn direct_handshake_request_carries_attempt_pair_verify() {
        let request = build_direct_handshake_request(7);
        let envelope = request.as_dict().expect("envelope dict");
        assert_eq!(
            envelope.get("mangledTypeName").and_then(XpcValue::as_str),
            Some(DIRECT_CONTROL_CHANNEL_ENVELOPE_TYPE)
        );

        let handshake = envelope
            .get("value")
            .and_then(XpcValue::as_dict)
            .and_then(|value| value.get("message"))
            .and_then(XpcValue::as_dict)
            .and_then(|message| message.get("plain"))
            .and_then(XpcValue::as_dict)
            .and_then(|plain| plain.get("_0"))
            .and_then(XpcValue::as_dict)
            .and_then(|plain| plain.get("request"))
            .and_then(XpcValue::as_dict)
            .and_then(|request| request.get("_0"))
            .and_then(XpcValue::as_dict)
            .and_then(|request| request.get("handshake"))
            .and_then(XpcValue::as_dict)
            .and_then(|handshake| handshake.get("_0"))
            .and_then(XpcValue::as_dict)
            .expect("handshake dict");

        assert_eq!(
            handshake
                .get("hostOptions")
                .and_then(XpcValue::as_dict)
                .and_then(|options| options.get("attemptPairVerify")),
            Some(&XpcValue::Bool(true))
        );
        assert_eq!(
            handshake.get("wireProtocolVersion"),
            Some(&XpcValue::Int64(19))
        );
    }

    #[test]
    fn remote_pairing_handshake_request_starts_at_plain_message_root() {
        let request = build_remote_pairing_handshake_request(0);
        assert_eq!(request["originatedBy"], "host");
        assert_eq!(request["sequenceNumber"], 0);
        assert_eq!(
            request["message"]["plain"]["_0"]["request"]["_0"]["handshake"]["_0"]["hostOptions"]
                ["attemptPairVerify"],
            true
        );
        assert_eq!(
            request["message"]["plain"]["_0"]["request"]["_0"]["handshake"]["_0"]
                ["wireProtocolVersion"],
            19
        );
    }

    #[test]
    fn extract_direct_remote_identifier_reads_peer_device_info() {
        let body = build_direct_control_envelope(
            xpc_dict(&[(
                "plain",
                xpc_dict(&[(
                    "_0",
                    xpc_dict(&[(
                        "response",
                        xpc_dict(&[(
                            "_1",
                            xpc_dict(&[(
                                "handshake",
                                xpc_dict(&[(
                                    "_0",
                                    xpc_dict(&[(
                                        "peerDeviceInfo",
                                        xpc_dict(&[(
                                            "identifier",
                                            XpcValue::String("test-remote".into()),
                                        )]),
                                    )]),
                                )]),
                            )]),
                        )]),
                    )]),
                )]),
            )]),
            1,
        );

        let identifier = extract_direct_remote_identifier(&body).expect("identifier should parse");
        assert_eq!(identifier, "test-remote");
    }

    #[test]
    fn extract_direct_pairing_tlv_surfaces_rejection_message() {
        let body = build_direct_control_envelope(
            xpc_dict(&[(
                "plain",
                xpc_dict(&[(
                    "_0",
                    xpc_dict(&[(
                        "event",
                        xpc_dict(&[(
                            "_0",
                            xpc_dict(&[(
                                "pairingRejectedWithError",
                                xpc_dict(&[(
                                    "wrappedError",
                                    xpc_dict(&[(
                                        "userInfo",
                                        xpc_dict(&[(
                                            "NSLocalizedDescription",
                                            XpcValue::String("Trust denied".into()),
                                        )]),
                                    )]),
                                )]),
                            )]),
                        )]),
                    )]),
                )]),
            )]),
            2,
        );

        let err = extract_direct_pairing_tlv(&body).expect_err("rejection should error");
        assert!(err.to_string().contains("Trust denied"));
    }

    #[test]
    fn extract_remote_pairing_tlv_decodes_base64_payload() {
        let body = serde_json::json!({
            "message": {
                "plain": {
                    "_0": {
                        "event": {
                            "_0": {
                                "pairingData": {
                                    "_0": {
                                        "data": BASE64_STANDARD.encode([0x01, 0x02, 0x03]),
                                        "kind": "verifyManualPairing",
                                        "startNewSession": true
                                    }
                                }
                            }
                        }
                    }
                }
            }
        });

        let tlv = extract_remote_pairing_tlv(&body).expect("payload should decode");
        assert_eq!(tlv, vec![0x01, 0x02, 0x03]);
    }

    #[test]
    fn extract_remote_pairing_tlv_surfaces_rejection_message() {
        let body = serde_json::json!({
            "message": {
                "plain": {
                    "_0": {
                        "event": {
                            "_0": {
                                "pairingRejectedWithError": {
                                    "wrappedError": {
                                        "userInfo": {
                                            "NSLocalizedDescription": "Pair denied"
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
        });

        let err = extract_remote_pairing_tlv(&body).expect_err("rejection should error");
        assert!(err.to_string().contains("Pair denied"));
    }

    #[test]
    fn make_direct_encrypted_nonce_uses_little_endian_sequence() {
        let nonce = make_direct_encrypted_nonce(0x0102_0304_0506_0708);
        assert_eq!(
            nonce,
            [0x08, 0x07, 0x06, 0x05, 0x04, 0x03, 0x02, 0x01, 0, 0, 0, 0]
        );
    }

    #[test]
    fn select_mux_device_prefers_usb_when_multiple_transports_match() {
        let selected = select_mux_device(
            vec![
                crate::mux::MuxDevice {
                    device_id: 7,
                    serial_number: "test-udid".into(),
                    connection_type: "Network".into(),
                    product_id: 0,
                },
                crate::mux::MuxDevice {
                    device_id: 8,
                    serial_number: "test-udid".into(),
                    connection_type: "USB".into(),
                    product_id: 0,
                },
            ],
            "test-udid",
        )
        .expect("matching device should be selected");

        assert_eq!(selected.device_id, 8);
        assert_eq!(selected.connection_type, "USB");
    }

    #[test]
    fn select_mux_device_falls_back_to_non_usb_match() {
        let selected = select_mux_device(
            vec![crate::mux::MuxDevice {
                device_id: 9,
                serial_number: "test-udid".into(),
                connection_type: "Network".into(),
                product_id: 0,
            }],
            "test-udid",
        )
        .expect("network-only match should still be selected");

        assert_eq!(selected.device_id, 9);
        assert_eq!(selected.connection_type, "Network");
    }

    #[test]
    fn strip_ssl_selection_matches_legacy_dtx_services() {
        assert!(should_strip_service_ssl(
            "com.apple.accessibility.axAuditDaemon.remoteserver"
        ));
        assert!(should_strip_service_ssl(
            "com.apple.instruments.remoteserver"
        ));
        assert!(!should_strip_service_ssl(
            "com.apple.instruments.remoteserver.DVTSecureSocketProxy"
        ));
        assert!(!should_strip_service_ssl("com.apple.mobile.screenshotr"));
        assert!(!should_strip_service_ssl("com.apple.webinspector"));
    }

    #[test]
    fn parses_string_array_values_for_international_configuration() {
        let value = plist::Value::Array(vec![
            plist::Value::String("en-US".into()),
            plist::Value::String("zh-Hans".into()),
        ]);

        let parsed = plist_value_to_string_vec(&value, "SupportedLanguages")
            .expect("string array should parse");

        assert_eq!(parsed, vec!["en-US".to_string(), "zh-Hans".to_string()]);
    }

    #[test]
    fn rejects_non_string_entries_in_international_configuration_arrays() {
        let value = plist::Value::Array(vec![plist::Value::Integer(1i64.into())]);

        let err = plist_value_to_string_vec(&value, "SupportedLocales")
            .expect_err("non-string entry should fail");

        let rendered = err.to_string();
        assert!(rendered.contains("SupportedLocales"));
        assert!(rendered.contains("string"));
    }

    #[test]
    fn resolve_rsd_service_reports_actual_shim_match() {
        let rsd = RsdHandshake {
            udid: "test-udid".into(),
            services: HashMap::from([(
                "com.apple.mobile.notification_proxy.shim.remote".into(),
                ServiceDescriptor { port: 1234 },
            )]),
        };

        let resolved = resolve_rsd_service(&rsd, "com.apple.mobile.notification_proxy")
            .expect("shim fallback should resolve");

        assert_eq!(
            resolved,
            (
                "com.apple.mobile.notification_proxy.shim.remote".into(),
                1234
            )
        );
    }

    #[test]
    fn resolve_tunnel_connection_target_uses_userspace_proxy_when_available() {
        let target =
            resolve_tunnel_connection_target("fd00::1", Some(60105)).expect("valid proxy target");

        assert_eq!(
            target,
            TunnelConnectionTarget::UserspaceProxy {
                proxy_port: 60105,
                remote_addr: Ipv6Addr::from_str("fd00::1").expect("valid IPv6"),
            }
        );
    }

    #[test]
    fn resolve_tunnel_connection_target_falls_back_to_direct_ipv6() {
        let target =
            resolve_tunnel_connection_target("fd00::2", None).expect("valid direct target");

        assert_eq!(
            target,
            TunnelConnectionTarget::DirectIpv6 {
                remote_addr: Ipv6Addr::from_str("fd00::2").expect("valid IPv6"),
            }
        );
    }

    #[test]
    fn resolve_tunnel_connection_target_rejects_invalid_ipv6() {
        let err = resolve_tunnel_connection_target("not-an-ipv6", Some(60105))
            .expect_err("invalid IPv6 should fail");

        assert!(err.to_string().contains("invalid IPv6 addr"));
    }

    #[test]
    fn preferred_lockdown_address_prefers_ipv4() {
        let addresses = vec![
            "fe80::1%Ethernet".to_string(),
            "192.168.31.247".to_string(),
            "fd00::1".to_string(),
        ];

        assert_eq!(
            preferred_lockdown_address(&addresses),
            Some("192.168.31.247")
        );
    }

    #[test]
    fn match_paired_mobdev2_targets_uses_wifi_mac_and_dedupes() {
        let services = vec![
            BonjourService {
                instance: "34:10:be:1b:a6:4c@fe80::1._apple-mobdev2._tcp.local.".into(),
                port: 32498,
                addresses: vec!["192.168.31.247".into()],
                properties: HashMap::new(),
            },
            BonjourService {
                instance: "34:10:be:1b:a6:4c@fe80::1._apple-mobdev2._tcp.local.".into(),
                port: 32498,
                addresses: vec!["192.168.31.247".into()],
                properties: HashMap::new(),
            },
        ];
        let wifi_mac_to_udid =
            HashMap::from([("34:10:be:1b:a6:4c".to_string(), "test-udid".to_string())]);

        let targets = match_paired_mobdev2_targets(&services, &wifi_mac_to_udid);

        assert_eq!(
            targets,
            vec![PairedMobdev2Device {
                udid: "test-udid".into(),
                host: "192.168.31.247".into(),
            }]
        );
    }

    #[tokio::test]
    async fn rsd_checkin_sends_request_and_consumes_two_responses() {
        let (mut client, mut server) = duplex(4096);
        let task = tokio::spawn(async move { rsd_checkin(&mut client).await });

        let request: plist::Value = recv_lockdown(&mut server).await.expect("request frame");
        let dict = request
            .into_dictionary()
            .expect("RSDCheckin request should be a plist dictionary");
        assert_eq!(
            dict.get("Request").and_then(plist::Value::as_string),
            Some("RSDCheckin")
        );
        assert_eq!(
            dict.get("ProtocolVersion")
                .and_then(plist::Value::as_string),
            Some("2")
        );

        send_lockdown(
            &mut server,
            &plist::Value::Dictionary(plist::Dictionary::from_iter([
                (
                    String::from("Request"),
                    plist::Value::String("RSDCheckin".into()),
                ),
                (
                    String::from("Status"),
                    plist::Value::String("Acknowledged".into()),
                ),
            ])),
        )
        .await
        .expect("checkin response");
        send_lockdown(
            &mut server,
            &plist::Value::Dictionary(plist::Dictionary::from_iter([
                (
                    String::from("Request"),
                    plist::Value::String("StartService".into()),
                ),
                (String::from("Service"), plist::Value::String("shim".into())),
            ])),
        )
        .await
        .expect("start service response");

        task.await
            .expect("join")
            .expect("rsd checkin should succeed");
    }

    #[tokio::test]
    async fn rsd_checkin_rejects_unexpected_first_response() {
        let (mut client, mut server) = duplex(4096);
        let task = tokio::spawn(async move { rsd_checkin(&mut client).await });

        let _: plist::Value = recv_lockdown(&mut server).await.expect("request frame");

        send_lockdown(
            &mut server,
            &plist::Value::Dictionary(plist::Dictionary::from_iter([(
                String::from("Request"),
                plist::Value::String("StartService".into()),
            )])),
        )
        .await
        .expect("unexpected first response");
        send_lockdown(
            &mut server,
            &plist::Value::Dictionary(plist::Dictionary::from_iter([(
                String::from("Request"),
                plist::Value::String("StartService".into()),
            )])),
        )
        .await
        .expect("second response");

        let err = task
            .await
            .expect("join")
            .expect_err("rsd checkin should reject mismatched first response");
        let rendered = err.to_string();
        assert!(rendered.contains("RSD check-in response"));
        assert!(rendered.contains("Request=RSDCheckin"));
    }

    #[tokio::test]
    async fn rsd_checkin_rejects_start_service_error() {
        let (mut client, mut server) = duplex(4096);
        let task = tokio::spawn(async move { rsd_checkin(&mut client).await });

        let _: plist::Value = recv_lockdown(&mut server).await.expect("request frame");

        send_lockdown(
            &mut server,
            &plist::Value::Dictionary(plist::Dictionary::from_iter([(
                String::from("Request"),
                plist::Value::String("RSDCheckin".into()),
            )])),
        )
        .await
        .expect("checkin response");
        send_lockdown(
            &mut server,
            &plist::Value::Dictionary(plist::Dictionary::from_iter([
                (
                    String::from("Request"),
                    plist::Value::String("StartService".into()),
                ),
                (
                    String::from("Error"),
                    plist::Value::String("ServiceProhibited".into()),
                ),
            ])),
        )
        .await
        .expect("start service error response");

        let err = task
            .await
            .expect("join")
            .expect_err("rsd checkin should surface start service errors");
        let rendered = err.to_string();
        assert!(rendered.contains("RSD start-service response"));
        assert!(rendered.contains("ServiceProhibited"));
    }
}
