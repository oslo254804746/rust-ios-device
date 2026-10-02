use std::collections::{BTreeSet, HashMap};
use std::io::Write;
use std::net::{Ipv6Addr, SocketAddr, SocketAddrV6};
use std::str::FromStr;
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, Result};
use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::{Extension, Path, Query, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use axum::{Json, Router};
use futures_util::{SinkExt, StreamExt};
use ios_core::MuxClient;
use ios_core::TunMode;
use ios_core::{
    connect, connect_direct_usb_tunnel, connect_remote_pairing_tunnel, connect_tcp_lockdown_tunnel,
    discover_paired_mobdev2_devices, ConnectOptions, ConnectedDevice,
};
use serde::{Deserialize, Serialize};
use serde_json::json;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::{Mutex, Notify, RwLock};
use tokio::time::Instant;
use tokio_tungstenite::tungstenite;

use crate::access::{
    normalize_upstream_url, AccessControl, ClientAccessArgs, Permission, Principal,
    ServerAccessArgs, UpstreamArgs, Upstreams,
};

const DEFAULT_AGENT_HOST: &str = "127.0.0.1";
const DEFAULT_AGENT_PORT: u16 = 49151;
const DEFAULT_SCAN_INTERVAL_MS: u64 = 2_000;
const TUNNEL_CONNECT_TIMEOUT: Duration = Duration::from_secs(15);
const MAX_FEDERATION_HOPS: u8 = 3;
const MAX_UPSTREAMS: usize = 32;
const MAX_LIST_BYTES: usize = 1024 * 1024;
const BRIDGE_BUFFER_SIZE: usize = 16 * 1024;
const MAX_WS_MESSAGE_BYTES: usize = 64 * 1024;
const BRIDGE_CLOSE_TIMEOUT: Duration = Duration::from_secs(1);

#[derive(clap::Args)]
pub struct TunnelCmd {
    #[command(subcommand)]
    sub: TunnelSub,
}

#[derive(clap::Subcommand)]
enum TunnelSub {
    /// Create or check private manager credentials without contacting a device
    Auth(crate::access::AuthCmd),
    /// Start a tunnel for a device
    Start {
        #[arg(long, help = "Use userspace smoltcp TUN (no root required)")]
        userspace: bool,
        #[arg(long, help = "Print only RSD host and port for shell scripts")]
        script_mode: bool,
    },
    /// Run a local HTTP tunnel manager that auto-creates tunnels for connected devices
    Serve {
        #[arg(long, help = "Use userspace smoltcp TUN (no root required)")]
        userspace: bool,
        #[arg(long, default_value = DEFAULT_AGENT_HOST)]
        host: String,
        #[arg(long, default_value_t = DEFAULT_AGENT_PORT)]
        port: u16,
        #[arg(long, default_value_t = DEFAULT_SCAN_INTERVAL_MS)]
        scan_interval_ms: u64,
        #[command(flatten)]
        access: ServerAccessArgs,
        #[command(flatten)]
        boundary: crate::access::HttpBoundaryArgs,
        #[command(flatten)]
        upstreams: UpstreamArgs,
    },
    /// Stop a tunnel
    Stop {
        #[arg(long, default_value = DEFAULT_AGENT_HOST)]
        host: String,
        #[arg(long, default_value_t = DEFAULT_AGENT_PORT)]
        port: u16,
        #[command(flatten)]
        access: ClientAccessArgs,
    },
    /// List active tunnels
    List {
        #[arg(long, default_value = DEFAULT_AGENT_HOST)]
        host: String,
        #[arg(long, default_value_t = DEFAULT_AGENT_PORT)]
        port: u16,
        #[command(flatten)]
        access: ClientAccessArgs,
    },
}

impl TunnelCmd {
    /// `serve` auto-creates tunnels as devices appear and `list` only talks to
    /// the HTTP manager, so neither needs a device attached at startup.
    pub(crate) fn needs_default_udid(&self) -> bool {
        matches!(self.sub, TunnelSub::Start { .. } | TunnelSub::Stop { .. })
    }

    pub async fn run(self, udid: Option<String>) -> Result<()> {
        match self.sub {
            TunnelSub::Auth(command) => command.run().await?,
            TunnelSub::Start {
                userspace,
                script_mode,
            } => {
                let udid =
                    udid.ok_or_else(|| anyhow::anyhow!("--udid required for tunnel start"))?;
                let tun_mode = if userspace {
                    TunMode::Userspace
                } else {
                    TunMode::Kernel
                };
                run_tunnel_start(&udid, tun_mode, script_mode).await?;
            }
            TunnelSub::Serve {
                userspace,
                host,
                port,
                scan_interval_ms,
                access,
                boundary,
                upstreams,
            } => {
                let tun_mode = if userspace {
                    TunMode::Userspace
                } else {
                    TunMode::Kernel
                };
                run_tunnel_agent(
                    &host,
                    port,
                    tun_mode,
                    Duration::from_millis(scan_interval_ms),
                    access,
                    boundary,
                    upstreams,
                )
                .await?;
            }
            TunnelSub::Stop { host, port, access } => {
                let udid =
                    udid.ok_or_else(|| anyhow::anyhow!("--udid required for tunnel stop"))?;
                run_tunnel_stop(&host, port, &udid, access).await?;
            }
            TunnelSub::List { host, port, access } => {
                run_tunnel_list(&host, port, access).await?;
            }
        }
        Ok(())
    }
}

fn manager_base_url(host: &str, port: u16) -> Result<reqwest::Url> {
    let host = if host.eq_ignore_ascii_case("localhost") {
        "127.0.0.1".to_string()
    } else if let Ok(address) = host.parse::<Ipv6Addr>() {
        format!("[{address}]")
    } else {
        host.to_string()
    };
    reqwest::Url::parse(&format!("http://{host}:{port}"))
        .with_context(|| format!("invalid tunnel manager address {host}:{port}"))
}

fn manager_tunnels_url(host: &str, port: u16) -> Result<String> {
    let mut url = manager_base_url(host, port)?;
    url.path_segments_mut()
        .map_err(|_| anyhow::anyhow!("tunnel manager URL cannot be a base"))?
        .push("tunnels");
    Ok(url.to_string())
}

fn manager_tunnel_url(host: &str, port: u16, udid: &str) -> Result<String> {
    let mut url = manager_base_url(host, port)?;
    url.path_segments_mut()
        .map_err(|_| anyhow::anyhow!("tunnel manager URL cannot be a base"))?
        .extend(["tunnel", udid]);
    Ok(url.to_string())
}

fn parse_tunnel_list_response(body: &str) -> Result<Vec<TunnelRecord>> {
    serde_json::from_str(body).context("failed to parse tunnel manager list response")
}

fn parse_tunnel_stop_response(body: &str) -> Result<TunnelStopResponse> {
    serde_json::from_str(body).context("failed to parse tunnel manager stop response")
}

async fn run_tunnel_list(host: &str, port: u16, access: ClientAccessArgs) -> Result<()> {
    let url = manager_tunnels_url(host, port)?;
    let body = tunnel_manager_request(reqwest::Method::GET, url, access).await?;
    let records = parse_tunnel_list_response(&body)?;

    crate::output::outln!("{}", serde_json::to_string_pretty(&records)?);
    Ok(())
}

async fn run_tunnel_stop(
    host: &str,
    port: u16,
    udid: &str,
    access: ClientAccessArgs,
) -> Result<()> {
    let url = manager_tunnel_url(host, port, udid)?;
    let body = tunnel_manager_request(reqwest::Method::DELETE, url, access).await?;
    let response = parse_tunnel_stop_response(&body)?;

    crate::output::outln!("{}", serde_json::to_string_pretty(&response)?);
    Ok(())
}

async fn tunnel_manager_request(
    method: reqwest::Method,
    url: String,
    access: ClientAccessArgs,
) -> Result<String> {
    let url = reqwest::Url::parse(&url).context("invalid tunnel manager URL")?;
    let header = access.header(&url).await?;
    let mut request = reqwest::Client::builder()
        .no_proxy()
        .redirect(reqwest::redirect::Policy::none())
        .timeout(Duration::from_secs(5))
        .build()?
        .request(method, url);
    if let Some(header) = header {
        request = request.header(reqwest::header::AUTHORIZATION, header);
    }
    let mut response = request
        .send()
        .await
        .context("failed to connect to tunnel manager")?;

    let status = response.status();
    if !status.is_success() {
        // Never consume or include arbitrary remote failure payloads.
        anyhow::bail!("tunnel manager request rejected: http/{}", status.as_u16());
    }
    let mut body = Vec::new();
    while let Some(chunk) = response
        .chunk()
        .await
        .context("failed to read tunnel manager response")?
    {
        anyhow::ensure!(
            chunk.len() <= MAX_LIST_BYTES.saturating_sub(body.len()),
            "tunnel manager response limit exceeded"
        );
        body.extend_from_slice(&chunk);
    }
    String::from_utf8(body).context("tunnel manager response must be UTF-8")
}

pub async fn run_tunnel_start(udid: &str, tun_mode: TunMode, script_mode: bool) -> Result<()> {
    let opts = ConnectOptions {
        tun_mode,
        pair_record_path: None,
        skip_tunnel: false,
    };
    let mode = match tun_mode {
        TunMode::Userspace => "userspace",
        TunMode::Kernel => "kernel",
    };

    if !script_mode {
        eprintln!(
            "Starting {mode} tunnel for {}...",
            ios_core::redaction::diagnostic_id(udid)
        );
    } else {
        eprintln!("tunnel: connecting ({mode})...");
    }

    let connect_timeout = Duration::from_secs(30);
    let device = tokio::time::timeout(connect_timeout, connect(udid, opts))
        .await
        .map_err(|_| {
            anyhow::anyhow!(
                "tunnel connect timed out after {}s — the device may not support \
                 CoreDevice tunneling or the handshake is blocked",
                connect_timeout.as_secs()
            )
        })??;
    let server_address = device
        .server_address()
        .ok_or_else(|| anyhow::anyhow!("tunnel started without a server address"))?;
    let rsd_port = device
        .rsd_port()
        .ok_or_else(|| anyhow::anyhow!("tunnel started without an RSD port"))?;

    if script_mode {
        crate::output::outln!("{server_address} {rsd_port}");
    } else {
        crate::output::outln!("Identifier: {udid}");
        crate::output::outln!("Mode: {mode}");
        crate::output::outln!("Protocol: tcp");
        crate::output::outln!("RSD Address: {server_address}");
        crate::output::outln!("RSD Port: {rsd_port}");
        if let Some(userspace_port) = device.userspace_port() {
            crate::output::outln!("Userspace Proxy: 127.0.0.1:{userspace_port}");
        }
        crate::output::outln!("Use the following connection option:");
        crate::output::outln!("--rsd {server_address} {rsd_port}");
    }
    std::io::stdout().flush()?;

    tokio::signal::ctrl_c().await?;
    Ok(())
}

#[derive(Clone)]
struct TunnelAgentState {
    inner: Arc<TunnelAgentInner>,
}

struct TunnelAgentInner {
    access: AccessControl,
    tun_mode: TunMode,
    tunnels: RwLock<HashMap<String, ManagedTunnel>>,
    upstreams: RwLock<BTreeSet<String>>,
    upstream_access: Upstreams,
    failed_attempts: RwLock<HashMap<String, FailedTunnelAttempt>>,
    ready: RwLock<bool>,
    lifecycle_lock: Mutex<()>,
    shutdown: Notify,
}

struct ManagedTunnel {
    device: ConnectedDevice,
}

#[derive(Debug, Clone)]
struct FailedTunnelAttempt {
    last_attempt: Instant,
    fail_count: u32,
}

impl FailedTunnelAttempt {
    fn should_skip(&self, now: Instant) -> bool {
        now.duration_since(self.last_attempt) < failed_device_backoff(self.fail_count)
    }
}

fn failed_device_backoff(fail_count: u32) -> Duration {
    let shift = fail_count.saturating_sub(1).min(4);
    Duration::from_secs((30 * (1u64 << shift)).min(300))
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
struct TunnelRecord {
    udid: String,
    interface: String,
    #[serde(rename = "connectionType")]
    connection_type: String,
    mode: String,
    address: String,
    #[serde(rename = "rsdPort")]
    rsd_port: u16,
    #[serde(rename = "tunnel-address")]
    tunnel_address: String,
    #[serde(rename = "tunnel-port")]
    tunnel_port: u16,
    #[serde(rename = "userspaceTun")]
    userspace_tun: bool,
    #[serde(rename = "userspaceTunHost", skip_serializing_if = "Option::is_none")]
    userspace_tun_host: Option<String>,
    #[serde(rename = "userspaceTunPort", skip_serializing_if = "Option::is_none")]
    userspace_tun_port: Option<u16>,
    #[serde(rename = "userspace-port", skip_serializing_if = "Option::is_none")]
    userspace_port: Option<u16>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
struct TunnelStopResponse {
    operation: String,
    #[serde(default)]
    udid: Option<String>,
    data: bool,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
struct StartTunnelResponse {
    interface: String,
    address: String,
    port: u16,
}

#[derive(Debug, Clone, Copy, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
enum StartTunnelConnectionType {
    Usbmux,
    Usb,
    Wifi,
}

#[derive(Debug, Deserialize)]
struct StartTunnelQuery {
    udid: String,
    ip: Option<String>,
    connection_type: Option<StartTunnelConnectionType>,
}

/// The `/connect` extension relays raw TCP bytes as binary WebSocket frames.
/// `port` is the device-side TCP port; `hop` counts manager-to-manager relays.
#[derive(Debug, Deserialize)]
struct ConnectQuery {
    udid: String,
    port: u16,
    interface: Option<String>,
    upstream: Option<String>,
    #[serde(default)]
    hop: u8,
}

#[derive(Debug, Default, Deserialize)]
struct FederationQuery {
    #[serde(default)]
    hop: u8,
}

#[derive(Debug, Deserialize)]
struct UpstreamBody {
    url: String,
}

type FederationListing = HashMap<String, Vec<serde_json::Value>>;

#[derive(Debug, Clone)]
struct RemoteTarget {
    upstream: String,
    interface: Option<String>,
    next_upstream: Option<String>,
}

impl StartTunnelQuery {
    fn unsupported_transport_message(&self) -> Option<&'static str> {
        match self.connection_type {
            Some(StartTunnelConnectionType::Usbmux)
            | Some(StartTunnelConnectionType::Usb)
            | Some(StartTunnelConnectionType::Wifi)
            | None => None,
        }
    }

    fn requested_connection_type(&self) -> Option<&'static str> {
        match self.connection_type {
            Some(StartTunnelConnectionType::Usbmux) => Some("usbmux"),
            Some(StartTunnelConnectionType::Usb) => Some("usb"),
            Some(StartTunnelConnectionType::Wifi) => Some("wifi"),
            None => None,
        }
    }
}

impl TunnelAgentState {
    #[cfg(test)]
    fn new(tun_mode: TunMode) -> Self {
        Self::with_access(tun_mode, AccessControl::trusted_test_local())
    }

    #[cfg(test)]
    fn with_access(tun_mode: TunMode, access: AccessControl) -> Self {
        Self::with_upstreams(tun_mode, access, Upstreams::default())
    }

    fn with_upstreams(
        tun_mode: TunMode,
        access: AccessControl,
        upstream_access: Upstreams,
    ) -> Self {
        Self {
            inner: Arc::new(TunnelAgentInner {
                access,
                tun_mode,
                tunnels: RwLock::new(HashMap::new()),
                upstreams: RwLock::new(upstream_access.urls().cloned().collect()),
                upstream_access,
                failed_attempts: RwLock::new(HashMap::new()),
                ready: RwLock::new(false),
                lifecycle_lock: Mutex::new(()),
                shutdown: Notify::new(),
            }),
        }
    }

    async fn is_ready(&self) -> bool {
        *self.inner.ready.read().await
    }

    async fn mark_ready(&self) {
        *self.inner.ready.write().await = true;
    }

    fn notify_shutdown(&self) {
        self.inner.shutdown.notify_waiters();
    }

    async fn clear(&self) {
        self.inner.tunnels.write().await.clear();
        self.inner.failed_attempts.write().await.clear();
    }

    async fn list_records(&self) -> Vec<TunnelRecord> {
        let _guard = self.inner.lifecycle_lock.lock().await;
        let mut tunnels = self.inner.tunnels.write().await;
        tunnels.retain(|_, tunnel| tunnel.is_alive());
        tunnels.values().filter_map(|t| t.record().ok()).collect()
    }

    async fn list_tunneld_view(&self) -> FederationListing {
        let mut grouped = FederationListing::new();
        for record in self.list_records().await {
            if let Ok(value) = serde_json::to_value(&record) {
                grouped.entry(record.udid.clone()).or_default().push(value);
            }
        }
        grouped
    }

    async fn list_federated(&self, hop: u8) -> Result<FederationListing> {
        let mut grouped = self.list_tunneld_view().await;
        if hop >= MAX_FEDERATION_HOPS {
            return Ok(grouped);
        }

        let upstreams = self
            .inner
            .upstreams
            .read()
            .await
            .iter()
            .cloned()
            .collect::<Vec<_>>();
        let results = futures_util::future::join_all(upstreams.iter().map(|upstream| {
            fetch_upstream_listing(&self.inner.upstream_access, upstream, hop + 1)
        }))
        .await;
        for (upstream, result) in upstreams.into_iter().zip(results) {
            let listing = result.context("federation upstream failed")?;
            for (udid, entries) in listing {
                for mut entry in entries {
                    if let Some(object) = entry.as_object_mut() {
                        object.insert("upstream".into(), json!(upstream));
                        grouped.entry(udid.clone()).or_default().push(entry);
                    }
                }
            }
        }
        Ok(grouped)
    }

    async fn remote_target(&self, query: &ConnectQuery) -> Result<Option<RemoteTarget>> {
        if query.hop >= MAX_FEDERATION_HOPS {
            return Ok(None);
        }
        let upstreams = self
            .inner
            .upstreams
            .read()
            .await
            .iter()
            .cloned()
            .collect::<Vec<_>>();
        if let Some(requested) = &query.upstream {
            anyhow::ensure!(upstreams.contains(requested), "upstream is not registered");
        }
        let candidates = upstreams
            .into_iter()
            .filter(|upstream| {
                query
                    .upstream
                    .as_ref()
                    .map_or(true, |wanted| wanted == upstream)
            })
            .collect::<Vec<_>>();
        let results = futures_util::future::join_all(candidates.iter().map(|upstream| {
            fetch_upstream_listing(&self.inner.upstream_access, upstream, query.hop + 1)
        }))
        .await;
        for (upstream, result) in candidates.into_iter().zip(results) {
            let listing = result.context("target upstream failed")?;
            if let Some(entries) = listing.get(&query.udid) {
                if let Some(entry) = entries.iter().find(|entry| {
                    query.interface.as_ref().map_or(true, |interface| {
                        entry.get("interface").and_then(serde_json::Value::as_str)
                            == Some(interface.as_str())
                    })
                }) {
                    return Ok(Some(RemoteTarget {
                        upstream,
                        interface: entry
                            .get("interface")
                            .and_then(serde_json::Value::as_str)
                            .map(str::to_owned),
                        next_upstream: entry
                            .get("upstream")
                            .and_then(serde_json::Value::as_str)
                            .map(str::to_owned),
                    }));
                }
            }
        }
        Ok(None)
    }

    async fn get_record(&self, udid: &str) -> Option<TunnelRecord> {
        let _guard = self.inner.lifecycle_lock.lock().await;
        let mut tunnels = self.inner.tunnels.write().await;
        match tunnels.get(udid) {
            Some(tunnel) if tunnel.is_alive() => tunnel.record().ok(),
            Some(_) => {
                tunnels.remove(udid);
                None
            }
            None => None,
        }
    }

    async fn remove_tunnel(&self, udid: &str) -> bool {
        let _guard = self.inner.lifecycle_lock.lock().await;
        let mut tunnels = self.inner.tunnels.write().await;
        match tunnels.get(udid) {
            Some(tunnel) if tunnel.is_alive() => {
                tunnels.remove(udid);
                true
            }
            Some(_) => {
                tunnels.remove(udid);
                false
            }
            None => false,
        }
    }

    async fn ensure_tunnel(
        &self,
        udid: &str,
        ip_filter: Option<&str>,
        requested_connection_type: Option<StartTunnelConnectionType>,
    ) -> Result<TunnelRecord> {
        let _guard = self.inner.lifecycle_lock.lock().await;

        if let Some(existing) = self.inner.tunnels.read().await.get(udid) {
            if existing.is_alive() {
                return existing.record();
            }
        }
        self.inner.tunnels.write().await.remove(udid);

        match requested_connection_type {
            Some(StartTunnelConnectionType::Usb) => {
                return self.connect_direct_usb_tunnel_locked(udid, ip_filter).await;
            }
            Some(StartTunnelConnectionType::Wifi) => {
                return self
                    .connect_remote_pairing_tunnel_locked(udid, ip_filter)
                    .await;
            }
            Some(StartTunnelConnectionType::Usbmux) => {
                return self.connect_usbmux_tunnel_locked(udid).await;
            }
            None => {}
        }

        match self.connect_usbmux_tunnel_locked(udid).await {
            Ok(record) => Ok(record),
            Err(usbmux_err) => {
                let mobdev2_targets = discover_paired_mobdev2_devices()
                    .await
                    .context("failed to browse paired mobdev2 devices")?;
                let target = mobdev2_targets
                    .into_iter()
                    .find(|target| {
                        target.udid == udid
                            && ip_filter
                                .map(|ip| ip == target.host)
                                .unwrap_or(true)
                    })
                    .ok_or_else(|| {
                        anyhow::anyhow!(
                            "usbmux failed ({usbmux_err}); no paired mobdev2 target matched udid={udid} ip={ip_filter:?}"
                        )
                    })?;
                self.connect_mobdev2_tunnel_locked(&target.udid, &target.host)
                    .await
            }
        }
    }

    async fn connect_usbmux_tunnel_locked(&self, udid: &str) -> Result<TunnelRecord> {
        let device = tokio::time::timeout(
            TUNNEL_CONNECT_TIMEOUT,
            connect(
                udid,
                ConnectOptions {
                    tun_mode: self.inner.tun_mode,
                    pair_record_path: None,
                    skip_tunnel: false,
                },
            ),
        )
        .await
        .context("timed out while creating usbmux tunnel")??;

        self.store_tunnel_locked(udid, device).await
    }

    async fn connect_direct_usb_tunnel_locked(
        &self,
        udid: &str,
        ip_filter: Option<&str>,
    ) -> Result<TunnelRecord> {
        let device = tokio::time::timeout(
            TUNNEL_CONNECT_TIMEOUT,
            connect_direct_usb_tunnel(
                udid,
                ip_filter,
                ConnectOptions {
                    tun_mode: self.inner.tun_mode,
                    pair_record_path: None,
                    skip_tunnel: false,
                },
            ),
        )
        .await
        .with_context(|| format!("timed out while creating direct usb tunnel for {udid}"))??;

        self.store_tunnel_locked(udid, device).await
    }

    async fn connect_mobdev2_tunnel_locked(&self, udid: &str, host: &str) -> Result<TunnelRecord> {
        let device = tokio::time::timeout(
            TUNNEL_CONNECT_TIMEOUT,
            connect_tcp_lockdown_tunnel(
                udid,
                host,
                ConnectOptions {
                    tun_mode: self.inner.tun_mode,
                    pair_record_path: None,
                    skip_tunnel: false,
                },
            ),
        )
        .await
        .with_context(|| {
            format!("timed out while creating mobdev2 tunnel for {udid} via {host}")
        })??;

        self.store_tunnel_locked(udid, device).await
    }

    async fn connect_remote_pairing_tunnel_locked(
        &self,
        udid: &str,
        host: Option<&str>,
    ) -> Result<TunnelRecord> {
        let device = tokio::time::timeout(
            TUNNEL_CONNECT_TIMEOUT,
            connect_remote_pairing_tunnel(
                udid,
                host,
                ConnectOptions {
                    tun_mode: self.inner.tun_mode,
                    pair_record_path: None,
                    skip_tunnel: false,
                },
            ),
        )
        .await
        .with_context(|| format!("timed out while creating remote pairing tunnel for {udid}"))??;

        self.store_tunnel_locked(udid, device).await
    }

    async fn store_tunnel_locked(
        &self,
        udid: &str,
        device: ConnectedDevice,
    ) -> Result<TunnelRecord> {
        let record = ManagedTunnel::record_for_device(&device, self.inner.tun_mode)?;
        self.inner
            .tunnels
            .write()
            .await
            .insert(udid.to_string(), ManagedTunnel { device });
        Ok(record)
    }

    async fn refresh_once(&self) -> Result<()> {
        let _guard = self.inner.lifecycle_lock.lock().await;

        self.inner
            .tunnels
            .write()
            .await
            .retain(|device, _| self.inner.access.permits_device(device));
        if !self.inner.access.permits_any_device() {
            self.mark_ready().await;
            return Ok(());
        }

        let mut mux = MuxClient::connect().await?;
        let devices = match mux.list_devices().await {
            Ok(devices) => devices,
            Err(err) => {
                self.inner.tunnels.write().await.clear();
                return Err(err.into());
            }
        };
        drop(mux);

        let attached_udids = devices
            .iter()
            .map(|device| device.serial_number.clone())
            .collect::<std::collections::HashSet<_>>();

        {
            let mut tunnels = self.inner.tunnels.write().await;
            tunnels.retain(|udid, tunnel| {
                tunnel.is_alive()
                    && self.inner.access.permits_device(udid)
                    && (tunnel.device.info.device_id == 0 || attached_udids.contains(udid))
            });
        }

        let existing = self
            .inner
            .tunnels
            .read()
            .await
            .keys()
            .cloned()
            .collect::<std::collections::HashSet<_>>();

        for device in devices {
            if !self.inner.access.permits_device(&device.serial_number) {
                continue;
            }
            if existing.contains(&device.serial_number) {
                continue;
            }
            if self
                .inner
                .failed_attempts
                .read()
                .await
                .get(&device.serial_number)
                .is_some_and(|failed| failed.should_skip(Instant::now()))
            {
                continue;
            }

            match tokio::time::timeout(
                TUNNEL_CONNECT_TIMEOUT,
                connect(
                    &device.serial_number,
                    ConnectOptions {
                        tun_mode: self.inner.tun_mode,
                        pair_record_path: None,
                        skip_tunnel: false,
                    },
                ),
            )
            .await
            {
                Ok(Ok(connected)) => {
                    self.inner.tunnels.write().await.insert(
                        device.serial_number.clone(),
                        ManagedTunnel { device: connected },
                    );
                    self.inner
                        .failed_attempts
                        .write()
                        .await
                        .remove(&device.serial_number);
                }
                Ok(Err(err)) => {
                    tracing::warn!(
                        device = %ios_core::redaction::diagnostic_id(&device.serial_number),
                        error = %ios_core::redaction::diagnostic_error(&err),
                        "tunnel agent: failed to establish tunnel"
                    );
                    self.record_failed_attempt(&device.serial_number).await;
                }
                Err(_) => {
                    tracing::warn!(
                        "tunnel agent: timed out establishing tunnel for {}",
                        ios_core::redaction::diagnostic_id(&device.serial_number)
                    );
                    self.record_failed_attempt(&device.serial_number).await;
                }
            }
        }

        let existing = self
            .inner
            .tunnels
            .read()
            .await
            .keys()
            .cloned()
            .collect::<std::collections::HashSet<_>>();

        for target in discover_paired_mobdev2_devices().await? {
            if !self.inner.access.permits_device(&target.udid) {
                continue;
            }
            if existing.contains(&target.udid) {
                continue;
            }
            if self
                .inner
                .failed_attempts
                .read()
                .await
                .get(&target.udid)
                .is_some_and(|failed| failed.should_skip(Instant::now()))
            {
                continue;
            }

            match self
                .connect_mobdev2_tunnel_locked(&target.udid, &target.host)
                .await
            {
                Ok(_) => {
                    self.inner
                        .failed_attempts
                        .write()
                        .await
                        .remove(&target.udid);
                }
                Err(err) => {
                    tracing::warn!(
                        device = %ios_core::redaction::diagnostic_id(&target.udid),
                        host = %ios_core::redaction::diagnostic_id(&target.host),
                        error = %crate::output::error_message(&err),
                        "tunnel agent: failed to establish mobdev2 tunnel"
                    );
                    self.record_failed_attempt(&target.udid).await;
                }
            }
        }

        self.mark_ready().await;
        Ok(())
    }

    async fn record_failed_attempt(&self, udid: &str) {
        let mut failed = self.inner.failed_attempts.write().await;
        failed
            .entry(udid.to_string())
            .and_modify(|attempt| {
                attempt.last_attempt = Instant::now();
                attempt.fail_count = attempt.fail_count.saturating_add(1);
            })
            .or_insert(FailedTunnelAttempt {
                last_attempt: Instant::now(),
                fail_count: 1,
            });
    }
}

async fn fetch_upstream_listing(
    access: &Upstreams,
    upstream: &str,
    hop: u8,
) -> Result<FederationListing> {
    let listing = access
        .client(upstream)?
        .listing(hop, MAX_LIST_BYTES)
        .await?;
    serde_json::from_value(listing).context("invalid upstream tunnel listing")
}

fn remote_connect_url(target: &RemoteTarget, query: &ConnectQuery) -> Result<reqwest::Url> {
    let mut url = reqwest::Url::parse(&target.upstream)?;
    let scheme = if url.scheme() == "https" { "wss" } else { "ws" };
    url.set_scheme(scheme)
        .map_err(|_| anyhow::anyhow!("cannot use upstream URL for WebSocket"))?;
    url.set_path("/connect");
    url.query_pairs_mut()
        .append_pair("udid", &query.udid)
        .append_pair("port", &query.port.to_string())
        .append_pair("hop", &(query.hop + 1).to_string());
    if let Some(interface) = &target.interface {
        url.query_pairs_mut().append_pair("interface", interface);
    }
    if let Some(next_upstream) = &target.next_upstream {
        url.query_pairs_mut().append_pair("upstream", next_upstream);
    }
    Ok(url)
}

async fn connect_record_port(record: &TunnelRecord, port: u16) -> Result<TcpStream> {
    let ipv6 = Ipv6Addr::from_str(&record.address).context("invalid tunnel IPv6 address")?;
    if let Some(proxy_port) = record.userspace_port {
        let host = record.userspace_tun_host.as_deref().unwrap_or("127.0.0.1");
        let mut stream = TcpStream::connect((host, proxy_port)).await?;
        stream.write_all(&ipv6.octets()).await?;
        stream.write_all(&(port as u32).to_le_bytes()).await?;
        stream.flush().await?;
        Ok(stream)
    } else {
        Ok(TcpStream::connect(SocketAddr::V6(SocketAddrV6::new(ipv6, port, 0, 0))).await?)
    }
}

async fn bridge_tcp_websocket<S>(socket: WebSocket, stream: S) -> Result<()>
where
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin,
{
    let (mut ws_tx, ws_rx) = socket.split();
    let (mut tcp_rx, tcp_tx) = tokio::io::split(stream);
    let inbound = websocket_to_tcp(ws_rx, tcp_tx);
    let outbound = async {
        let mut buffer = [0u8; BRIDGE_BUFFER_SIZE];
        loop {
            let count = tcp_rx.read(&mut buffer).await?;
            if count == 0 {
                let _ =
                    tokio::time::timeout(BRIDGE_CLOSE_TIMEOUT, ws_tx.send(Message::Close(None)))
                        .await;
                return Ok::<(), anyhow::Error>(());
            }
            ws_tx
                .send(Message::Binary(buffer[..count].to_vec()))
                .await?;
        }
    };
    tokio::select! { result = inbound => result, result = outbound => result }
}

async fn websocket_to_tcp<St, W>(mut rx: St, mut tx: W) -> Result<()>
where
    St: futures_util::Stream<Item = Result<Message, axum::Error>> + Unpin,
    W: tokio::io::AsyncWrite + Unpin,
{
    while let Some(message) = rx.next().await {
        match message? {
            Message::Binary(bytes) => tx.write_all(&bytes).await?,
            Message::Close(_) => return Ok(()),
            Message::Ping(_) | Message::Pong(_) => {}
            Message::Text(_) => anyhow::bail!("TCP bridge accepts binary WebSocket messages only"),
        }
    }
    Ok(())
}

async fn bridge_websockets<S>(
    socket: WebSocket,
    upstream: tokio_tungstenite::WebSocketStream<S>,
) -> Result<()>
where
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin,
{
    let (mut client_tx, mut client_rx) = socket.split();
    let (mut upstream_tx, mut upstream_rx) = upstream.split();
    let to_upstream = async {
        while let Some(message) = client_rx.next().await {
            let message = match message {
                Ok(message) => message,
                Err(error) => {
                    let _ = tokio::time::timeout(
                        BRIDGE_CLOSE_TIMEOUT,
                        upstream_tx.send(tungstenite::Message::Close(None)),
                    )
                    .await;
                    return Err(error.into());
                }
            };
            match message {
                Message::Binary(bytes) => {
                    upstream_tx
                        .send(tungstenite::Message::Binary(bytes))
                        .await?
                }
                Message::Close(_) => {
                    let _ = tokio::time::timeout(
                        BRIDGE_CLOSE_TIMEOUT,
                        upstream_tx.send(tungstenite::Message::Close(None)),
                    )
                    .await;
                    return Ok::<(), anyhow::Error>(());
                }
                Message::Ping(_) | Message::Pong(_) => {}
                Message::Text(_) => {
                    let _ = tokio::time::timeout(
                        BRIDGE_CLOSE_TIMEOUT,
                        upstream_tx.send(tungstenite::Message::Close(None)),
                    )
                    .await;
                    anyhow::bail!("TCP bridge accepts binary WebSocket messages only")
                }
            }
        }
        let _ = tokio::time::timeout(
            BRIDGE_CLOSE_TIMEOUT,
            upstream_tx.send(tungstenite::Message::Close(None)),
        )
        .await;
        Ok(())
    };
    let to_client = async {
        while let Some(message) = upstream_rx.next().await {
            let message = match message {
                Ok(message) => message,
                Err(error) => {
                    let _ = tokio::time::timeout(
                        BRIDGE_CLOSE_TIMEOUT,
                        client_tx.send(Message::Close(None)),
                    )
                    .await;
                    return Err(error.into());
                }
            };
            match message {
                tungstenite::Message::Binary(bytes) => {
                    client_tx.send(Message::Binary(bytes)).await?
                }
                tungstenite::Message::Close(_) => {
                    let _ = tokio::time::timeout(
                        BRIDGE_CLOSE_TIMEOUT,
                        client_tx.send(Message::Close(None)),
                    )
                    .await;
                    return Ok::<(), anyhow::Error>(());
                }
                tungstenite::Message::Ping(_) | tungstenite::Message::Pong(_) => {}
                tungstenite::Message::Text(_) => {
                    let _ = tokio::time::timeout(
                        BRIDGE_CLOSE_TIMEOUT,
                        client_tx.send(Message::Close(None)),
                    )
                    .await;
                    anyhow::bail!("upstream TCP bridge sent a text WebSocket message")
                }
                tungstenite::Message::Frame(_) => {}
            }
        }
        let _ =
            tokio::time::timeout(BRIDGE_CLOSE_TIMEOUT, client_tx.send(Message::Close(None))).await;
        Ok(())
    };
    tokio::select! { result = to_upstream => result, result = to_client => result }
}

impl ManagedTunnel {
    fn interface_name_for_device(device: &ConnectedDevice) -> String {
        if device.info.device_id == 0 {
            format!("mobdev2-{}", device.info.udid)
        } else {
            format!("usbmux-{}", device.info.udid)
        }
    }

    fn record(&self) -> Result<TunnelRecord> {
        Self::record_for_device(
            &self.device,
            if self.device.userspace_port().is_some() {
                TunMode::Userspace
            } else {
                TunMode::Kernel
            },
        )
    }

    fn is_alive(&self) -> bool {
        self.device
            .tunnel_handle()
            .map(|tunnel| tunnel.is_alive())
            .unwrap_or(false)
    }

    fn record_for_device(device: &ConnectedDevice, tun_mode: TunMode) -> Result<TunnelRecord> {
        let server_address = device
            .server_address()
            .ok_or_else(|| anyhow::anyhow!("tunnel missing server address"))?
            .to_string();
        let rsd_port = device
            .rsd_port()
            .ok_or_else(|| anyhow::anyhow!("tunnel missing RSD port"))?;
        let userspace_port = device.userspace_port();

        Ok(TunnelRecord {
            udid: device.info.udid.clone(),
            interface: Self::interface_name_for_device(device),
            connection_type: device.info.connection_type.clone(),
            mode: match tun_mode {
                TunMode::Userspace => "userspace",
                TunMode::Kernel => "kernel",
            }
            .to_string(),
            address: server_address.clone(),
            rsd_port,
            tunnel_address: server_address,
            tunnel_port: rsd_port,
            userspace_tun: userspace_port.is_some(),
            userspace_tun_host: userspace_port.map(|_| "127.0.0.1".to_string()),
            userspace_tun_port: userspace_port,
            userspace_port,
        })
    }
}

impl StartTunnelResponse {
    fn from_record(record: TunnelRecord) -> Self {
        Self {
            interface: record.interface,
            address: record.address,
            port: record.rsd_port,
        }
    }
}

async fn run_tunnel_agent(
    host: &str,
    port: u16,
    tun_mode: TunMode,
    scan_interval: Duration,
    access: ServerAccessArgs,
    boundary: crate::access::HttpBoundaryArgs,
    upstreams: UpstreamArgs,
) -> Result<()> {
    anyhow::ensure!(!scan_interval.is_zero(), "scan interval must be nonzero");
    let boundary = boundary.validate()?;
    let (address, access) = access.load(host).await?;
    let upstreams = upstreams.load().await?;
    let state = TunnelAgentState::with_upstreams(tun_mode, access, upstreams);

    let listener = TcpListener::bind((address, port))
        .await
        .with_context(|| format!("failed to bind tunnel agent on {host}:{port}"))?;
    let bound = listener.local_addr()?;
    let app = boundary.for_listener(bound).protect(
        agent_router(state.clone()),
        crate::http_diagnostics::ErrorShape::Manager,
    );

    tracing::info!(
        "tunnel agent listening on http://{} ({})",
        bound,
        match tun_mode {
            TunMode::Userspace => "userspace",
            TunMode::Kernel => "kernel",
        }
    );

    let monitor_state = state.clone();
    let monitor = tokio::spawn(async move {
        let mut interval = tokio::time::interval(scan_interval);
        interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

        loop {
            tokio::select! {
                _ = monitor_state.inner.shutdown.notified() => break,
                _ = interval.tick() => {
                    if let Err(err) = monitor_state.refresh_once().await {
                        tracing::warn!(error = %crate::output::error_message(&err), "tunnel agent refresh failed");
                    }
                }
            }
        }
    });

    let shutdown_state = state.clone();
    axum::serve(listener, app)
        .with_graceful_shutdown(async move {
            shutdown_state.inner.shutdown.notified().await;
        })
        .await?;

    monitor.abort();
    let _ = monitor.await;
    state.clear().await;
    Ok(())
}

fn agent_router(state: TunnelAgentState) -> Router {
    let access = state.inner.access.clone();
    let admission = crate::access::Admission::manager(access);
    Router::new()
        .route("/", get(agent_list_tunneld))
        .route("/connect", get(agent_connect))
        .route(
            "/upstream",
            get(agent_list_upstreams)
                .post(agent_add_upstream)
                .delete(agent_remove_upstream),
        )
        .route("/health", get(agent_health))
        .route("/ready", get(agent_ready))
        .route(
            "/shutdown",
            axum::routing::post(agent_shutdown).get(legacy_shutdown_get),
        )
        .route("/auth/reload", axum::routing::post(agent_reload_policy))
        .route(
            "/start-tunnel",
            axum::routing::post(agent_start_tunnel).get(legacy_start_get),
        )
        .route("/tunnels", get(agent_list_records))
        .route(
            "/tunnel/:udid",
            get(agent_get_tunnel).delete(agent_delete_tunnel),
        )
        .with_state(state)
        .merge(admission.ticket_routes())
        .layer(axum::middleware::from_fn_with_state(
            crate::http_diagnostics::ErrorShape::Manager,
            crate::http_diagnostics::rejections,
        ))
        .layer(axum::middleware::from_fn_with_state(
            admission,
            crate::access::admission,
        ))
}

async fn agent_list_tunneld(
    State(state): State<TunnelAgentState>,
    Extension(principal): Extension<Principal>,
    Query(query): Query<FederationQuery>,
) -> Result<Json<FederationListing>, Response> {
    principal.require_permission(Permission::List)?;
    if query.hop > MAX_FEDERATION_HOPS {
        return Err(manager_classified_error(
            StatusCode::LOOP_DETECTED,
            "federation hop budget exceeded",
            ios_core::ErrorClass::ResourceLimit,
        ));
    }
    let mut records = principal
        .run(state.list_federated(query.hop))
        .await?
        .map_err(|error| {
            manager_operation_error(
                StatusCode::BAD_GATEWAY,
                "federation listing failed",
                error.as_ref(),
            )
        })?;
    principal.require_permission(Permission::List)?;
    records.retain(|device, _| principal.allows(Permission::List, Some(device), None));
    Ok(Json(records))
}

fn manager_classified_error(
    status: StatusCode,
    message: &'static str,
    class: ios_core::ErrorClass,
) -> Response {
    crate::http_diagnostics::classified(
        (status, Json(json!({ "message": message }))),
        class,
        crate::http_diagnostics::ErrorPhase::Manager,
    )
}

fn manager_operation_error(
    status: StatusCode,
    phase: &'static str,
    error: &(dyn std::error::Error + 'static),
) -> Response {
    crate::http_diagnostics::classified(
        (
            status,
            Json(json!({
                "message": phase,
                "error": ios_core::redaction::diagnostic_error(error).to_string(),
            })),
        ),
        ios_core::classify_error(error),
        crate::http_diagnostics::ErrorPhase::Manager,
    )
}

fn bounded_websocket_upgrade(ws: WebSocketUpgrade) -> WebSocketUpgrade {
    ws.max_frame_size(MAX_WS_MESSAGE_BYTES)
        .max_message_size(MAX_WS_MESSAGE_BYTES)
}

async fn agent_list_upstreams(
    State(state): State<TunnelAgentState>,
    Extension(principal): Extension<Principal>,
) -> Result<Json<Vec<String>>, Response> {
    principal.require(Permission::ManageServer, None, None)?;
    Ok(Json(
        state.inner.upstreams.read().await.iter().cloned().collect(),
    ))
}

async fn agent_add_upstream(
    State(state): State<TunnelAgentState>,
    Extension(principal): Extension<Principal>,
    Json(body): Json<UpstreamBody>,
) -> Result<Json<serde_json::Value>, Response> {
    principal.require(Permission::ManageServer, None, None)?;
    let url = normalize_upstream_url(&body.url).map_err(|err| {
        manager_operation_error(
            StatusCode::BAD_REQUEST,
            "invalid upstream URL",
            err.as_ref(),
        )
    })?;
    let mut upstreams = state.inner.upstreams.write().await;
    if !upstreams.contains(&url) && upstreams.len() >= MAX_UPSTREAMS {
        return Err(manager_classified_error(
            StatusCode::BAD_REQUEST,
            "upstream limit reached",
            ios_core::ErrorClass::ResourceLimit,
        ));
    }
    upstreams.insert(url.clone());
    Ok(Json(
        json!({ "operation": "add_upstream", "url": url, "data": true }),
    ))
}

async fn agent_remove_upstream(
    State(state): State<TunnelAgentState>,
    Extension(principal): Extension<Principal>,
    Json(body): Json<UpstreamBody>,
) -> Result<Json<serde_json::Value>, Response> {
    principal.require(Permission::ManageServer, None, None)?;
    let url = normalize_upstream_url(&body.url).map_err(|err| {
        manager_operation_error(
            StatusCode::BAD_REQUEST,
            "invalid upstream URL",
            err.as_ref(),
        )
    })?;
    let removed = state.inner.upstreams.write().await.remove(&url);
    Ok(Json(
        json!({ "operation": "remove_upstream", "url": url, "data": removed }),
    ))
}

async fn agent_connect(
    State(state): State<TunnelAgentState>,
    Extension(principal): Extension<Principal>,
    Query(query): Query<ConnectQuery>,
    ws: WebSocketUpgrade,
) -> Result<Response, Response> {
    principal.require(Permission::Connect, Some(&query.udid), Some(query.port))?;
    principal
        .run(agent_connect_authorized(
            state,
            query,
            ws,
            principal.clone(),
        ))
        .await?
}

async fn agent_connect_authorized(
    state: TunnelAgentState,
    query: ConnectQuery,
    ws: WebSocketUpgrade,
    principal: Principal,
) -> Result<Response, Response> {
    if query.hop > MAX_FEDERATION_HOPS {
        return Err(manager_classified_error(
            StatusCode::LOOP_DETECTED,
            "federation hop budget exceeded",
            ios_core::ErrorClass::ResourceLimit,
        ));
    }
    if query.udid.is_empty() || query.port == 0 {
        return Err(manager_classified_error(
            StatusCode::BAD_REQUEST,
            "udid and nonzero port are required",
            ios_core::ErrorClass::InvalidInput,
        ));
    }
    let local = if query.upstream.is_none() {
        state.get_record(&query.udid).await.filter(|record| {
            query
                .interface
                .as_ref()
                .map_or(true, |wanted| wanted == &record.interface)
        })
    } else {
        None
    };
    if let Some(record) = local {
        let stream = tokio::time::timeout(
            TUNNEL_CONNECT_TIMEOUT,
            connect_record_port(&record, query.port),
        )
        .await
        .map_err(|error| {
            manager_classified_error(
                StatusCode::GATEWAY_TIMEOUT,
                "tunnel TCP dial timed out",
                ios_core::classify_error(&error),
            )
        })?
        .map_err(|err| {
            manager_operation_error(
                StatusCode::BAD_GATEWAY,
                "tunnel TCP dial failed",
                err.as_ref(),
            )
        })?;
        return Ok(bounded_websocket_upgrade(ws)
            .on_upgrade(move |socket| async move {
                tokio::select! {
                    result = bridge_tcp_websocket(socket, stream) => {
                        if let Err(err) = result {
                            tracing::debug!(error = %crate::output::error_message(&err), "tunnel agent: local WebSocket bridge ended");
                        }
                    }
                    _ = principal.revoked() => tracing::debug!("tunnel agent: local WebSocket authorization revoked"),
                }
            })
            .into_response());
    }
    if query.hop >= MAX_FEDERATION_HOPS {
        return Err(manager_classified_error(
            StatusCode::LOOP_DETECTED,
            "federation hop budget exceeded",
            ios_core::ErrorClass::ResourceLimit,
        ));
    }
    if let Some(upstream) = &query.upstream {
        if !state.inner.upstreams.read().await.contains(upstream) {
            return Err(manager_classified_error(
                StatusCode::BAD_REQUEST,
                "upstream is not registered",
                ios_core::ErrorClass::NotFound,
            ));
        }
    }
    let target = state
        .remote_target(&query)
        .await
        .map_err(|err| {
            manager_operation_error(
                StatusCode::BAD_GATEWAY,
                "target selection failed",
                err.as_ref(),
            )
        })?
        .ok_or_else(|| {
            manager_classified_error(
                StatusCode::NOT_FOUND,
                "tunnel target not found",
                ios_core::ErrorClass::NotFound,
            )
        })?;
    let url = remote_connect_url(&target, &query).map_err(|err| {
        manager_operation_error(
            StatusCode::BAD_REQUEST,
            "upstream WebSocket URL failed",
            err.as_ref(),
        )
    })?;
    let client = state
        .inner
        .upstream_access
        .client(&target.upstream)
        .map_err(|error| {
            manager_operation_error(
                StatusCode::BAD_GATEWAY,
                "upstream transport configuration failed",
                error.as_ref(),
            )
        })?;
    let request = client.websocket_request(&url).map_err(|error| {
        manager_operation_error(
            StatusCode::BAD_GATEWAY,
            "upstream authentication configuration failed",
            error.as_ref(),
        )
    })?;
    let (remote, _) = tokio::time::timeout(
        TUNNEL_CONNECT_TIMEOUT,
        tokio_tungstenite::connect_async_tls_with_config(
            request,
            Some(tungstenite::protocol::WebSocketConfig {
                max_message_size: Some(MAX_WS_MESSAGE_BYTES),
                max_frame_size: Some(MAX_WS_MESSAGE_BYTES),
                ..Default::default()
            }),
            false,
            Some(client.connector()),
        ),
    )
    .await
    .map_err(|error| {
        manager_classified_error(
            StatusCode::GATEWAY_TIMEOUT,
            "upstream WebSocket dial timed out",
            ios_core::classify_error(&error),
        )
    })?
    .map_err(|err| {
        manager_operation_error(
            StatusCode::BAD_GATEWAY,
            "upstream WebSocket dial failed",
            &err,
        )
    })?;
    Ok(bounded_websocket_upgrade(ws)
        .on_upgrade(move |socket| async move {
            tokio::select! {
                result = bridge_websockets(socket, remote) => {
                    if let Err(err) = result {
                        tracing::debug!(error = %crate::output::error_message(&err), "tunnel agent: remote WebSocket bridge ended");
                    }
                }
                _ = principal.revoked() => tracing::debug!("tunnel agent: remote WebSocket authorization revoked"),
            }
        })
        .into_response())
}

async fn agent_list_records(
    State(state): State<TunnelAgentState>,
    Extension(principal): Extension<Principal>,
) -> Result<Json<Vec<TunnelRecord>>, Response> {
    principal.require_permission(Permission::List)?;
    let records = principal.run(state.list_records()).await?;
    principal.require_permission(Permission::List)?;
    Ok(Json(
        records
            .into_iter()
            .filter(|record| principal.allows(Permission::List, Some(&record.udid), None))
            .collect(),
    ))
}

async fn agent_get_tunnel(
    State(state): State<TunnelAgentState>,
    Extension(principal): Extension<Principal>,
    Path(udid): Path<String>,
) -> Result<Json<TunnelRecord>, Response> {
    principal.require(Permission::List, Some(&udid), None)?;
    let record = principal.run(state.get_record(&udid)).await?;
    principal.require(Permission::List, Some(&udid), None)?;
    record.map(Json).ok_or_else(|| {
        crate::http_diagnostics::classified((
            StatusCode::NOT_FOUND,
            Json(json!({ "message": "tunnel not found", "device": ios_core::redaction::diagnostic_id(&udid).to_string() })),
        ), ios_core::ErrorClass::NotFound, crate::http_diagnostics::ErrorPhase::Manager)
    })
}

async fn agent_delete_tunnel(
    State(state): State<TunnelAgentState>,
    Extension(principal): Extension<Principal>,
    Path(udid): Path<String>,
) -> Result<Json<serde_json::Value>, Response> {
    principal.require(Permission::ManageTunnels, Some(&udid), None)?;
    if principal.run(state.remove_tunnel(&udid)).await? {
        Ok(Json(
            json!({ "operation": "cancel", "udid": udid, "data": true }),
        ))
    } else {
        Err(crate::http_diagnostics::classified(
            (
                StatusCode::NOT_FOUND,
                Json(
                    json!({ "message": "tunnel not found", "device": ios_core::redaction::diagnostic_id(&udid).to_string() }),
                ),
            ),
            ios_core::ErrorClass::NotFound,
            crate::http_diagnostics::ErrorPhase::Manager,
        ))
    }
}

async fn agent_start_tunnel(
    State(state): State<TunnelAgentState>,
    Extension(principal): Extension<Principal>,
    Json(query): Json<StartTunnelQuery>,
) -> Result<Json<StartTunnelResponse>, Response> {
    principal.require(Permission::ManageTunnels, Some(&query.udid), None)?;
    if let Some(message) = query.unsupported_transport_message() {
        return Err(crate::http_diagnostics::classified(
            (
                StatusCode::NOT_IMPLEMENTED,
                Json(json!({
                    "message": message,
                    "device": ios_core::redaction::diagnostic_id(&query.udid).to_string(),
                    "connection_type": query.requested_connection_type(),
                })),
            ),
            ios_core::ErrorClass::Unsupported,
            crate::http_diagnostics::ErrorPhase::Manager,
        ));
    }

    principal
        .run(state.ensure_tunnel(&query.udid, query.ip.as_deref(), query.connection_type))
        .await?
        .map(StartTunnelResponse::from_record)
        .map(Json)
        .map_err(|err| {
            manager_operation_error(
                StatusCode::BAD_GATEWAY,
                "failed to create tunnel",
                err.as_ref(),
            )
        })
}

async fn agent_health() -> StatusCode {
    StatusCode::OK
}

// GET management never mutates state. Keep a bounded migration notice until
// 2026-11-01 00:00 UTC, then retain an authenticated 410 tombstone.
const LEGACY_GET_NOTICE_UNTIL: u64 = 1_793_491_200;

fn legacy_management_reply(now: u64) -> Response {
    let (status, message) = if now < LEGACY_GET_NOTICE_UNTIL {
        (
            StatusCode::METHOD_NOT_ALLOWED,
            "management requires POST; GET notice ends 2026-11-01",
        )
    } else {
        (StatusCode::GONE, "legacy GET management removed; use POST")
    };
    let mut response = manager_classified_error(status, message, ios_core::ErrorClass::Unsupported);
    response
        .headers_mut()
        .insert(axum::http::header::ALLOW, "POST".parse().unwrap());
    response
}

async fn legacy_shutdown_get(
    Extension(principal): Extension<Principal>,
) -> Result<Response, Response> {
    principal.require(Permission::ManageServer, None, None)?;
    Ok(legacy_management_reply(legacy_notice_time()))
}

async fn legacy_start_get(
    Extension(principal): Extension<Principal>,
) -> Result<Response, Response> {
    principal.require_permission(Permission::ManageTunnels)?;
    Ok(legacy_management_reply(legacy_notice_time()))
}

fn legacy_notice_time() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(LEGACY_GET_NOTICE_UNTIL, |duration| duration.as_secs())
}

async fn agent_ready(State(state): State<TunnelAgentState>) -> StatusCode {
    if state.is_ready().await {
        StatusCode::OK
    } else {
        StatusCode::SERVICE_UNAVAILABLE
    }
}

async fn agent_shutdown(
    State(state): State<TunnelAgentState>,
    Extension(principal): Extension<Principal>,
) -> Result<Json<serde_json::Value>, Response> {
    principal.require(Permission::ManageServer, None, None)?;
    state.notify_shutdown();
    Ok(Json(json!({
        "operation": "shutdown",
        "data": true,
        "message": "Server shutting down..."
    })))
}

async fn agent_reload_policy(
    State(state): State<TunnelAgentState>,
    Extension(principal): Extension<Principal>,
) -> Result<Json<serde_json::Value>, Response> {
    principal.require(Permission::ManageServer, None, None)?;
    state
        .inner
        .access
        .reload(&principal)
        .await
        .map_err(|error| {
            manager_operation_error(
                StatusCode::BAD_REQUEST,
                "manager policy reload failed",
                error.as_ref(),
            )
        })?;
    Ok(Json(json!({"operation": "reload_policy", "data": true})))
}

#[cfg(test)]
mod tests {
    use crate::access::test_support::JsonRequest;
    use clap::Parser;
    use futures_util::{SinkExt, StreamExt};

    use super::*;

    mod access_tests;
    mod browser_tests;
    mod method_tests;
    mod upstream_tests;

    #[tokio::test]
    async fn manager_http_categories_keep_native_and_message_only_bodies() {
        let secret = "synthetic-private-manager-native-source";
        let error = std::io::Error::new(std::io::ErrorKind::PermissionDenied, secret);
        let response =
            manager_operation_error(StatusCode::BAD_GATEWAY, "original operation phase", &error);
        assert_eq!(response.status(), StatusCode::BAD_GATEWAY);
        assert_eq!(response.headers()["x-ios-error-class"], "permission_denied");
        assert_eq!(response.headers()["x-ios-error-phase"], "tunnel_manager");
        let raw = axum::body::to_bytes(response.into_body(), 8192)
            .await
            .unwrap();
        assert!(!std::str::from_utf8(&raw).unwrap().contains(secret));
        assert_eq!(
            serde_json::from_slice::<serde_json::Value>(&raw).unwrap(),
            json!({"message": "original operation phase", "error": ios_core::redaction::diagnostic_error(&error).to_string()})
        );

        let elapsed = tokio::time::timeout(Duration::from_millis(5), std::future::pending::<()>())
            .await
            .unwrap_err();
        let response = manager_classified_error(
            StatusCode::GATEWAY_TIMEOUT,
            "original timeout phase",
            ios_core::classify_error(&elapsed),
        );
        assert_eq!(response.status(), StatusCode::GATEWAY_TIMEOUT);
        assert_eq!(response.headers()["x-ios-error-class"], "timeout");
        assert_eq!(response.headers()["x-ios-error-effects"], "unknown");
        let raw = axum::body::to_bytes(response.into_body(), 8192)
            .await
            .unwrap();
        assert_eq!(
            serde_json::from_slice::<serde_json::Value>(&raw).unwrap(),
            json!({"message": "original timeout phase"})
        );
        let response = manager_classified_error(
            StatusCode::GATEWAY_TIMEOUT,
            "opaque timeout permission denied",
            ios_core::ErrorClass::Unknown,
        );
        assert_eq!(
            response.headers()["x-ios-error-class"],
            "unknown",
            "status/prose is not native typed evidence"
        );
    }

    #[tokio::test]
    async fn actual_manager_routes_classify_named_local_failures_before_device_work() {
        const SECRET: &str = "synthetic-private-manager-selector";
        let state = TunnelAgentState::new(TunMode::Userspace);
        let (port, server) = spawn_test_router(agent_router(state.clone())).await;
        let client = reqwest::Client::builder()
            .no_proxy()
            .timeout(Duration::from_secs(2))
            .build()
            .unwrap();
        let base = format!("http://127.0.0.1:{port}");
        for (method, path, status, class) in [
            (
                reqwest::Method::GET,
                "/?hop=4".to_string(),
                StatusCode::LOOP_DETECTED,
                "resource_limit",
            ),
            (
                reqwest::Method::GET,
                format!("/tunnel/{SECRET}"),
                StatusCode::NOT_FOUND,
                "not_found",
            ),
            (
                reqwest::Method::DELETE,
                format!("/tunnel/{SECRET}"),
                StatusCode::NOT_FOUND,
                "not_found",
            ),
        ] {
            let response = client
                .request(method, format!("{base}{path}"))
                .send()
                .await
                .unwrap();
            assert_eq!(response.status(), status);
            assert_eq!(response.headers()["x-ios-error-schema-version"], "1");
            assert_eq!(response.headers()["x-ios-error-class"], class);
            assert_eq!(response.headers()["x-ios-error-phase"], "tunnel_manager");
            assert_eq!(response.headers()["x-ios-error-effects"], "unknown");
            assert!(!response.text().await.unwrap().contains(SECRET));
        }
        for (query, status, class) in [
            (
                format!("udid={SECRET}&port=0"),
                StatusCode::BAD_REQUEST,
                "invalid_input",
            ),
            (
                format!("udid={SECRET}&port=12345&hop=4"),
                StatusCode::LOOP_DETECTED,
                "resource_limit",
            ),
            (
                format!(
                    "udid={SECRET}&port=12345&upstream=http%3A%2F%2Funregistered.invalid%3A1%2F"
                ),
                StatusCode::BAD_REQUEST,
                "not_found",
            ),
            (
                format!("udid={SECRET}&port=12345"),
                StatusCode::NOT_FOUND,
                "not_found",
            ),
        ] {
            let failure = tokio::time::timeout(
                Duration::from_secs(2),
                tokio_tungstenite::connect_async(format!("ws://127.0.0.1:{port}/connect?{query}")),
            )
            .await
            .unwrap()
            .unwrap_err();
            match failure {
                tungstenite::Error::Http(response) => {
                    assert_eq!(response.status(), status);
                    assert_eq!(response.headers()["x-ios-error-class"], class);
                    assert_eq!(response.headers()["x-ios-error-phase"], "tunnel_manager");
                    assert_eq!(response.headers()["x-ios-error-effects"], "unknown");
                    assert!(!response.body().as_ref().is_some_and(|body| body
                        .windows(SECRET.len())
                        .any(|value| value == SECRET.as_bytes())));
                }
                other => panic!(
                    "unexpected owned WebSocket failure: {}",
                    ios_core::redaction::diagnostic_error(&other)
                ),
            }
        }
        state.inner.upstreams.write().await.extend(
            (0..MAX_UPSTREAMS).map(|index| format!("https://owned-fixture-{index}.invalid:1/")),
        );
        let response = client
            .post(format!("{base}/upstream"))
            .json(&json!({"url": "https://unregistered.invalid:1/"}))
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        assert_eq!(response.headers()["x-ios-error-class"], "resource_limit");
        assert_eq!(state.inner.upstreams.read().await.len(), MAX_UPSTREAMS);
        assert!(state.list_records().await.is_empty());
        server.abort();
        let _ = server.await;
    }

    fn fake_record(proxy_port: u16) -> TunnelRecord {
        TunnelRecord {
            udid: "test-udid".into(),
            interface: "fake-interface".into(),
            connection_type: "USB".into(),
            mode: "userspace".into(),
            address: "fd00::1234".into(),
            rsd_port: 58783,
            tunnel_address: "fd00::1234".into(),
            tunnel_port: 58783,
            userspace_tun: true,
            userspace_tun_host: Some("127.0.0.1".into()),
            userspace_tun_port: Some(proxy_port),
            userspace_port: Some(proxy_port),
        }
    }

    async fn spawn_test_router(app: Router) -> (u16, tokio::task::JoinHandle<()>) {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let app = crate::access::HttpBoundaryArgs::default()
            .validate()
            .unwrap()
            .for_listener(listener.local_addr().unwrap())
            .protect(app, crate::http_diagnostics::ErrorShape::Manager);
        let task = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        (port, task)
    }

    #[tokio::test]
    async fn manager_http_errors_hide_raw_inputs_and_source_chains() {
        const SECRET: &str = "synthetic-private-key-UDID-token-device-data";
        let state = TunnelAgentState::new(TunMode::Userspace);
        let router = agent_router(state).route(
            "/synthetic-failure",
            get(|| async {
                let error = anyhow::Error::new(std::io::Error::new(
                    std::io::ErrorKind::PermissionDenied,
                    SECRET,
                ))
                .context(format!("https://user:{SECRET}@example.invalid/{SECRET}"));
                assert!(format!("{error:#}").contains(SECRET));
                manager_operation_error(
                    StatusCode::BAD_GATEWAY,
                    "synthetic dial failed",
                    error.as_ref(),
                )
            }),
        );
        let (port, server) = spawn_test_router(router).await;
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(2))
            .build()
            .unwrap();
        let base = format!("http://127.0.0.1:{port}");
        for path in [format!("/tunnel/{SECRET}"), "/synthetic-failure".into()] {
            let response = client.get(format!("{base}{path}")).send().await.unwrap();
            assert!(response.status().is_client_error() || response.status().is_server_error());
            let raw = response.text().await.unwrap();
            assert!(!raw.contains(SECRET));
            assert!(!raw.contains("https://"));
            let json: serde_json::Value = serde_json::from_str(&raw).unwrap();
            assert!(json["message"].is_string());
            if path == "/synthetic-failure" {
                assert!(json["error"]
                    .as_str()
                    .unwrap()
                    .contains("io/PermissionDenied error:"));
            } else {
                assert!(json["device"].as_str().unwrap().starts_with("id:"));
            }
        }
        let response = client
            .post(format!("{base}/upstream"))
            .header("content-type", "application/json")
            .body(
                serde_json::to_vec(&json!({"url": format!("http://user:{SECRET}@127.0.0.1/")}))
                    .unwrap(),
            )
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        assert!(!response.text().await.unwrap().contains(SECRET));
        server.abort();
        let _ = server.await;
    }

    async fn spawn_fake_proxy() -> (u16, tokio::task::JoinHandle<()>) {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let task = tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.unwrap();
            let mut prelude = [0u8; 20];
            stream.read_exact(&mut prelude).await.unwrap();
            assert_eq!(
                &prelude[..16],
                &Ipv6Addr::from_str("fd00::1234").unwrap().octets()
            );
            assert_eq!(u32::from_le_bytes(prelude[16..].try_into().unwrap()), 12345);
            let mut buffer = [0u8; 64];
            let count = stream.read(&mut buffer).await.unwrap();
            stream.write_all(&buffer[..count]).await.unwrap();
        });
        (port, task)
    }

    #[test]
    fn parses_tunnel_start_script_mode_command() {
        let parsed =
            crate::Cli::try_parse_from(["ios", "tunnel", "start", "--userspace", "--script-mode"]);
        assert!(
            parsed.is_ok(),
            "tunnel start --script-mode command should parse"
        );
    }

    #[test]
    fn parses_tunnel_serve_command() {
        let parsed = crate::Cli::try_parse_from([
            "ios",
            "tunnel",
            "serve",
            "--userspace",
            "--port",
            "49151",
        ]);
        assert!(parsed.is_ok(), "tunnel serve command should parse");
    }

    #[test]
    fn parses_tunnel_list_manager_options() {
        let parsed = crate::Cli::try_parse_from([
            "ios",
            "tunnel",
            "list",
            "--host",
            "127.0.0.1",
            "--port",
            "49151",
        ]);
        assert!(parsed.is_ok(), "tunnel list manager options should parse");
    }

    #[test]
    fn parses_tunnel_stop_with_global_udid_and_manager_options() {
        let parsed = crate::Cli::try_parse_from([
            "ios",
            "--udid",
            "00008110-001234567890801E",
            "tunnel",
            "stop",
            "--host",
            "localhost",
            "--port",
            "49152",
        ]);
        assert!(
            parsed.is_ok(),
            "tunnel stop should accept global UDID and manager options"
        );
    }

    #[test]
    fn manager_client_builds_list_url() {
        assert_eq!(
            manager_tunnels_url("127.0.0.1", 49151).unwrap(),
            "http://127.0.0.1:49151/tunnels"
        );
    }

    #[test]
    fn manager_client_builds_stop_url_with_encoded_udid() {
        assert_eq!(
            manager_tunnel_url("127.0.0.1", 49151, "abc/def 123").unwrap(),
            "http://127.0.0.1:49151/tunnel/abc%2Fdef%20123"
        );
    }

    #[test]
    fn manager_client_parses_tunnel_list_response() {
        let records = parse_tunnel_list_response(
            r#"[
                {
                    "udid": "test-udid",
                    "interface": "usbmux-test-udid",
                    "connectionType": "USB",
                    "mode": "userspace",
                    "address": "fd00::1",
                    "rsdPort": 58783,
                    "tunnel-address": "fd00::1",
                    "tunnel-port": 58783,
                    "userspaceTun": true,
                    "userspaceTunHost": "127.0.0.1",
                    "userspaceTunPort": 49152,
                    "userspace-port": 49152
                }
            ]"#,
        )
        .unwrap();

        assert_eq!(records.len(), 1);
        assert_eq!(records[0].udid, "test-udid");
        assert_eq!(records[0].rsd_port, 58783);
        assert_eq!(records[0].userspace_port, Some(49152));
    }

    #[test]
    fn manager_client_parses_stop_response() {
        let response =
            parse_tunnel_stop_response(r#"{"operation":"cancel","udid":"test-udid","data":true}"#)
                .unwrap();

        assert_eq!(response.operation, "cancel");
        assert_eq!(response.udid.as_deref(), Some("test-udid"));
        assert!(response.data);
    }

    #[test]
    fn tunnel_record_includes_both_tunneld_and_go_ios_fields() {
        let record = TunnelRecord {
            udid: "test-udid".into(),
            interface: "usbmux-test-udid".into(),
            connection_type: "USB".into(),
            mode: "userspace".into(),
            address: "fd00::1".into(),
            rsd_port: 58783,
            tunnel_address: "fd00::1".into(),
            tunnel_port: 58783,
            userspace_tun: true,
            userspace_tun_host: Some("127.0.0.1".into()),
            userspace_tun_port: Some(49152),
            userspace_port: Some(49152),
        };

        let value = serde_json::to_value(record).unwrap();
        assert_eq!(value["tunnel-address"], "fd00::1");
        assert_eq!(value["tunnel-port"], 58783);
        assert_eq!(value["userspaceTun"], true);
        assert_eq!(value["userspaceTunHost"], "127.0.0.1");
        assert_eq!(value["userspaceTunPort"], 49152);
        assert_eq!(value["userspace-port"], 49152);
    }

    #[tokio::test]
    async fn deleting_unknown_tunnel_returns_false() {
        let state = TunnelAgentState::new(TunMode::Userspace);
        assert!(!state.remove_tunnel("missing-udid").await);
    }

    #[test]
    fn start_tunnel_query_accepts_usbmux_connection_type() {
        let query: StartTunnelQuery =
            serde_urlencoded::from_str("udid=test-udid&connection_type=usbmux")
                .expect("usbmux query should deserialize");

        assert_eq!(query.udid, "test-udid");
        assert_eq!(
            query.connection_type,
            Some(StartTunnelConnectionType::Usbmux)
        );
        assert_eq!(query.unsupported_transport_message(), None);
    }

    #[test]
    fn start_tunnel_query_accepts_usb_transport() {
        let query: StartTunnelQuery =
            serde_urlencoded::from_str("udid=test-udid&connection_type=usb&ip=fd00::1")
                .expect("usb query should deserialize");

        assert_eq!(query.ip.as_deref(), Some("fd00::1"));
        assert_eq!(query.unsupported_transport_message(), None);
    }

    #[test]
    fn start_tunnel_query_accepts_wifi_transport() {
        let query: StartTunnelQuery =
            serde_urlencoded::from_str("udid=test-udid&connection_type=wifi&ip=192.168.31.247")
                .expect("wifi query should deserialize");

        assert_eq!(query.ip.as_deref(), Some("192.168.31.247"));
        assert_eq!(query.connection_type, Some(StartTunnelConnectionType::Wifi));
        assert_eq!(query.unsupported_transport_message(), None);
    }

    #[test]
    fn failed_device_backoff_is_exponential_and_capped() {
        assert_eq!(failed_device_backoff(0), Duration::from_secs(30));
        assert_eq!(failed_device_backoff(1), Duration::from_secs(30));
        assert_eq!(failed_device_backoff(2), Duration::from_secs(60));
        assert_eq!(failed_device_backoff(3), Duration::from_secs(120));
        assert_eq!(failed_device_backoff(99), Duration::from_secs(300));
    }

    #[test]
    fn failed_tunnel_attempt_skip_honors_backoff_window() {
        let now = Instant::now();
        let failed = FailedTunnelAttempt {
            last_attempt: now - Duration::from_secs(10),
            fail_count: 2,
        };
        assert!(failed.should_skip(now));

        let expired = FailedTunnelAttempt {
            last_attempt: now - Duration::from_secs(61),
            fail_count: 2,
        };
        assert!(!expired.should_skip(now));
    }

    #[test]
    fn start_tunnel_response_matches_tunneld_shape() {
        let response = StartTunnelResponse::from_record(TunnelRecord {
            udid: "test-udid".into(),
            interface: "usbmux-test-udid".into(),
            connection_type: "USB".into(),
            mode: "userspace".into(),
            address: "fd00::1".into(),
            rsd_port: 58783,
            tunnel_address: "fd00::1".into(),
            tunnel_port: 58783,
            userspace_tun: true,
            userspace_tun_host: Some("127.0.0.1".into()),
            userspace_tun_port: Some(49152),
            userspace_port: Some(49152),
        });

        let value = serde_json::to_value(response).unwrap();
        assert_eq!(value["interface"], "usbmux-test-udid");
        assert_eq!(value["address"], "fd00::1");
        assert_eq!(value["port"], 58783);
        assert!(value.get("rsdPort").is_none());
    }

    #[tokio::test]
    async fn websocket_bridge_relays_tcp_bytes_and_closes_on_tcp_eof() {
        let (proxy_port, proxy_task) = spawn_fake_proxy().await;
        let record = fake_record(proxy_port);
        let app = Router::new().route(
            "/connect",
            get(move |ws: WebSocketUpgrade| async move {
                let stream = connect_record_port(&record, 12345).await.unwrap();
                ws.on_upgrade(move |socket| async move {
                    bridge_tcp_websocket(socket, stream).await.unwrap();
                })
            }),
        );
        let (port, server) = spawn_test_router(app).await;
        let (mut client, _) =
            tokio_tungstenite::connect_async(format!("ws://127.0.0.1:{port}/connect"))
                .await
                .unwrap();
        client
            .send(tungstenite::Message::Binary(b"hello".to_vec()))
            .await
            .unwrap();
        assert_eq!(
            client.next().await.unwrap().unwrap(),
            tungstenite::Message::Binary(b"hello".to_vec())
        );
        assert!(matches!(
            client.next().await.unwrap().unwrap(),
            tungstenite::Message::Close(_)
        ));
        proxy_task.await.unwrap();
        server.abort();
    }

    #[tokio::test]
    async fn websocket_close_drops_tcp_connection() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let tcp_port = listener.local_addr().unwrap().port();
        let app = Router::new().route(
            "/connect",
            get(move |ws: WebSocketUpgrade| async move {
                let stream = TcpStream::connect(("127.0.0.1", tcp_port)).await.unwrap();
                ws.on_upgrade(move |socket| async move {
                    bridge_tcp_websocket(socket, stream).await.unwrap();
                })
            }),
        );
        let (port, server) = spawn_test_router(app).await;
        let (mut client, _) =
            tokio_tungstenite::connect_async(format!("ws://127.0.0.1:{port}/connect"))
                .await
                .unwrap();
        let (mut tcp, _) = listener.accept().await.unwrap();
        client.close(None).await.unwrap();
        let mut byte = [0u8; 1];
        let eof = tokio::time::timeout(Duration::from_secs(2), tcp.read(&mut byte))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(eof, 0);
        server.abort();
    }

    #[tokio::test]
    async fn websocket_bridge_rejects_oversized_frame() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let tcp_port = listener.local_addr().unwrap().port();
        let tcp_task = tokio::spawn(async move {
            let (_stream, _) = listener.accept().await.unwrap();
            std::future::pending::<()>().await;
        });
        let app = Router::new().route(
            "/connect",
            get(move |ws: WebSocketUpgrade| async move {
                let stream = TcpStream::connect(("127.0.0.1", tcp_port)).await.unwrap();
                bounded_websocket_upgrade(ws).on_upgrade(move |socket| async move {
                    let _ = bridge_tcp_websocket(socket, stream).await;
                })
            }),
        );
        let (port, server) = spawn_test_router(app).await;
        let (mut client, _) =
            tokio_tungstenite::connect_async(format!("ws://127.0.0.1:{port}/connect"))
                .await
                .unwrap();
        let _ = client
            .send(tungstenite::Message::Binary(vec![
                0;
                MAX_WS_MESSAGE_BYTES + 1
            ]))
            .await;
        let result = tokio::time::timeout(Duration::from_secs(2), client.next())
            .await
            .unwrap();
        assert!(matches!(
            result,
            None | Some(Err(_)) | Some(Ok(tungstenite::Message::Close(_)))
        ));
        tcp_task.abort();
        server.abort();
    }

    #[tokio::test]
    async fn websocket_to_tcp_applies_backpressure_before_reading_next_frame() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        let reads = Arc::new(AtomicUsize::new(0));
        let observed = reads.clone();
        let messages = futures_util::stream::unfold(0usize, move |index| {
            let observed = observed.clone();
            async move {
                if index == 2 {
                    return None;
                }
                observed.fetch_add(1, Ordering::SeqCst);
                Some((
                    Ok::<_, axum::Error>(Message::Binary(vec![index as u8; 64])),
                    index + 1,
                ))
            }
        });
        let (writer, mut reader) = tokio::io::duplex(1);
        let task = tokio::spawn(websocket_to_tcp(Box::pin(messages), writer));
        tokio::time::timeout(Duration::from_secs(1), async {
            while reads.load(Ordering::SeqCst) == 0 {
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        assert_eq!(reads.load(Ordering::SeqCst), 1);
        let mut first = [0u8; 64];
        reader.read_exact(&mut first).await.unwrap();
        assert_eq!(first, [0; 64]);
        let mut second = [0u8; 64];
        reader.read_exact(&mut second).await.unwrap();
        assert_eq!(second, [1; 64]);
        task.await.unwrap().unwrap();
    }

    #[tokio::test]
    async fn federated_websocket_targets_remote_tunnel_and_preserves_bytes() {
        let (proxy_port, proxy_task) = spawn_fake_proxy().await;
        let record = fake_record(proxy_port);
        let listing_record = serde_json::to_value(&record).unwrap();
        let upstream = Router::new()
            .route(
                "/",
                get(move || async move { Json(json!({ "test-udid": [listing_record] })) }),
            )
            .route(
                "/connect",
                get(
                    move |Query(query): Query<ConnectQuery>, ws: WebSocketUpgrade| async move {
                        assert_eq!(query.udid, "test-udid");
                        assert_eq!(query.port, 12345);
                        assert_eq!(query.hop, 1);
                        let stream = connect_record_port(&record, query.port).await.unwrap();
                        ws.on_upgrade(move |socket| async move {
                            bridge_tcp_websocket(socket, stream).await.unwrap();
                        })
                    },
                ),
            );
        let (upstream_port, upstream_task) = spawn_test_router(upstream).await;
        let state = TunnelAgentState::new(TunMode::Userspace);
        let upstream_url = format!("http://127.0.0.1:{upstream_port}/");
        state
            .inner
            .upstreams
            .write()
            .await
            .insert(upstream_url.clone());
        let (port, agent_task) = spawn_test_router(agent_router(state)).await;
        let listing_bytes = reqwest::get(format!("http://127.0.0.1:{port}/"))
            .await
            .unwrap()
            .bytes()
            .await
            .unwrap();
        let listing: serde_json::Value = serde_json::from_slice(&listing_bytes).unwrap();
        assert_eq!(listing["test-udid"][0]["upstream"], upstream_url);
        let missing = tokio_tungstenite::connect_async(format!(
            "ws://127.0.0.1:{port}/connect?udid=test-udid&port=12345&interface=other"
        ))
        .await
        .unwrap_err();
        assert!(
            matches!(missing, tungstenite::Error::Http(response) if response.status() == StatusCode::NOT_FOUND)
        );
        let (mut client, _) = tokio_tungstenite::connect_async(format!(
            "ws://127.0.0.1:{port}/connect?udid=test-udid&port=12345&interface=fake-interface"
        ))
        .await
        .unwrap();
        client
            .send(tungstenite::Message::Binary(b"remote".to_vec()))
            .await
            .unwrap();
        assert_eq!(
            client.next().await.unwrap().unwrap(),
            tungstenite::Message::Binary(b"remote".to_vec())
        );
        proxy_task.await.unwrap();
        assert!(matches!(
            tokio::time::timeout(Duration::from_secs(2), client.next())
                .await
                .unwrap(),
            Some(Ok(tungstenite::Message::Close(_)))
        ));
        agent_task.abort();
        upstream_task.abort();
    }

    #[tokio::test]
    async fn nested_federation_preserves_selected_upstream_and_hop() {
        let (proxy_port, proxy_task) = spawn_fake_proxy().await;
        let record = fake_record(proxy_port);
        let listing_record = serde_json::to_value(&record).unwrap();
        let leaf = Router::new()
            .route(
                "/",
                get(move || async move { Json(json!({"test-udid": [listing_record]})) }),
            )
            .route(
                "/connect",
                get(
                    move |Query(query): Query<ConnectQuery>, ws: WebSocketUpgrade| async move {
                        assert_eq!(query.hop, 2);
                        assert_eq!(query.interface.as_deref(), Some("fake-interface"));
                        let stream = connect_record_port(&record, query.port).await.unwrap();
                        ws.on_upgrade(move |socket| async move {
                            bridge_tcp_websocket(socket, stream).await.unwrap();
                        })
                    },
                ),
            );
        let (leaf_port, leaf_task) = spawn_test_router(leaf).await;
        let middle = TunnelAgentState::new(TunMode::Userspace);
        middle
            .inner
            .upstreams
            .write()
            .await
            .insert(format!("http://127.0.0.1:{leaf_port}/"));
        let (middle_port, middle_task) = spawn_test_router(agent_router(middle)).await;
        let root = TunnelAgentState::new(TunMode::Userspace);
        root.inner
            .upstreams
            .write()
            .await
            .insert(format!("http://127.0.0.1:{middle_port}/"));
        let (root_port, root_task) = spawn_test_router(agent_router(root)).await;
        let (mut client, _) = tokio_tungstenite::connect_async(format!(
            "ws://127.0.0.1:{root_port}/connect?udid=test-udid&port=12345&interface=fake-interface"
        ))
        .await
        .unwrap();
        client
            .send(tungstenite::Message::Binary(b"nested".to_vec()))
            .await
            .unwrap();
        assert_eq!(
            client.next().await.unwrap().unwrap(),
            tungstenite::Message::Binary(b"nested".to_vec())
        );
        proxy_task.await.unwrap();
        assert!(matches!(
            tokio::time::timeout(Duration::from_secs(2), client.next())
                .await
                .unwrap(),
            Some(Ok(tungstenite::Message::Close(_)))
        ));
        root_task.abort();
        middle_task.abort();
        leaf_task.abort();
    }

    #[tokio::test]
    async fn cyclic_federation_stops_at_hop_budget() {
        let first = TunnelAgentState::new(TunMode::Userspace);
        let second = TunnelAgentState::new(TunMode::Userspace);
        let (first_port, first_task) = spawn_test_router(agent_router(first.clone())).await;
        let (second_port, second_task) = spawn_test_router(agent_router(second.clone())).await;
        first
            .inner
            .upstreams
            .write()
            .await
            .insert(format!("http://127.0.0.1:{second_port}/"));
        second
            .inner
            .upstreams
            .write()
            .await
            .insert(format!("http://127.0.0.1:{first_port}/"));
        let response = tokio::time::timeout(
            Duration::from_secs(2),
            reqwest::get(format!("http://127.0.0.1:{first_port}/?hop=0")),
        )
        .await
        .unwrap()
        .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let listing_bytes = response.bytes().await.unwrap();
        let listing: FederationListing = serde_json::from_slice(&listing_bytes).unwrap();
        assert!(listing.is_empty());
        let over_limit = reqwest::get(format!("http://127.0.0.1:{first_port}/?hop=4"))
            .await
            .unwrap();
        assert_eq!(over_limit.status(), StatusCode::LOOP_DETECTED);
        let exhausted = tokio_tungstenite::connect_async(format!(
            "ws://127.0.0.1:{first_port}/connect?udid=missing&port=12345&hop=3"
        ))
        .await
        .unwrap_err();
        assert!(
            matches!(exhausted, tungstenite::Error::Http(response) if response.status() == StatusCode::LOOP_DETECTED)
        );
        let connect_over_limit = tokio_tungstenite::connect_async(format!(
            "ws://127.0.0.1:{first_port}/connect?udid=missing&port=12345&hop=4"
        ))
        .await
        .unwrap_err();
        assert!(
            matches!(connect_over_limit, tungstenite::Error::Http(response) if response.status() == StatusCode::LOOP_DETECTED)
        );
        first_task.abort();
        second_task.abort();
    }

    #[test]
    fn upstream_url_rejects_credentials_and_non_root_paths() {
        assert!(normalize_upstream_url("http://user:pass@localhost:49151/").is_err());
        assert!(normalize_upstream_url("http://localhost:49151/other").is_err());
        assert!(normalize_upstream_url("http://localhost:49151").is_err());
        assert_eq!(
            normalize_upstream_url("http://127.0.0.1:49151").unwrap(),
            "http://127.0.0.1:49151/"
        );
    }

    #[tokio::test]
    async fn upstream_registration_adds_lists_and_removes() {
        let state = TunnelAgentState::new(TunMode::Userspace);
        let (port, task) = spawn_test_router(agent_router(state)).await;
        let client = reqwest::Client::new();
        let endpoint = format!("http://127.0.0.1:{port}/upstream");
        let body = json!({ "url": "http://127.0.0.1:5555" }).to_string();
        let added = client
            .post(&endpoint)
            .header("content-type", "application/json")
            .body(body.clone())
            .send()
            .await
            .unwrap();
        assert_eq!(added.status(), StatusCode::OK);
        let listed = client
            .get(&endpoint)
            .send()
            .await
            .unwrap()
            .bytes()
            .await
            .unwrap();
        let urls: Vec<String> = serde_json::from_slice(&listed).unwrap();
        assert_eq!(urls, vec!["http://127.0.0.1:5555/"]);
        let deleted = client
            .delete(&endpoint)
            .header("content-type", "application/json")
            .body(body)
            .send()
            .await
            .unwrap();
        assert_eq!(deleted.status(), StatusCode::OK);
        let listed = client
            .get(&endpoint)
            .send()
            .await
            .unwrap()
            .bytes()
            .await
            .unwrap();
        let urls: Vec<String> = serde_json::from_slice(&listed).unwrap();
        assert!(urls.is_empty());
        task.abort();
    }
}
