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
use ios_core::TunMode;
use ios_core::{connect_with_owner, ConnectOptions, ConnectedDevice};
use serde::{Deserialize, Serialize};
use serde_json::json;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
#[cfg(test)]
use tokio::sync::Notify;
use tokio::sync::{watch, Mutex, OwnedSemaphorePermit, RwLock, Semaphore};
use tokio::time::Instant;
use tokio_tungstenite::tungstenite;

use crate::access::{
    normalize_upstream_url, AccessControl, ClientAccessArgs, Permission, Principal,
    ServerAccessArgs, UpstreamArgs, Upstreams,
};

mod admission;
mod bookkeeping;
mod candidate_index;
mod candidate_input;
mod error_payload;
mod intent;
mod lifecycle;
mod metadata_vec;
mod ownership;
mod reconnect;
mod retirement;
mod scan;
mod scoped;
pub(crate) use scoped::{NativeConnection, NativeScope};
mod server;
mod source;
use bookkeeping::{
    DeferredFailure, DeferredSuccess, FailureIntent, ScanAttemptOutcome, ScanIntent, SuccessIntent,
};
use lifecycle::{Lifecycle, Ticket};
use source::{NativeSource, Route, Source, UsbSnapshot};

const DEFAULT_AGENT_HOST: &str = "127.0.0.1";
const DEFAULT_AGENT_PORT: u16 = 49151;
const DEFAULT_SCAN_INTERVAL_MS: u64 = 2_000;
const TUNNEL_CONNECT_TIMEOUT: Duration = Duration::from_secs(15);
const TUNNEL_SCAN_TIMEOUT: Duration = Duration::from_secs(30);
const MAX_FEDERATION_HOPS: u8 = 3;
const MAX_UPSTREAMS: usize = 32;
const MAX_FAILURE_RECORDS: usize = 4096;
const MAX_PUBLISHED_METADATA_BYTES: usize = 64 * 1024 * 1024;
// All admitted owner families compete in this bank. Their separate maxima are
// private ceilings, not a promise that all maxima can be held simultaneously.
const MAX_GLOBAL_KNOWN_METADATA_BYTES: usize = 128 * 1024 * 1024;
const FAILURE_WRITE_TIMEOUT: Duration = Duration::from_secs(1);
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
        retry: reconnect::RetryArgs,
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
                retry,
                access,
                boundary,
                upstreams,
            } => {
                let retry = retry.validate()?;
                let tun_mode = if userspace {
                    TunMode::Userspace
                } else {
                    TunMode::Kernel
                };
                run_tunnel_agent(
                    &host,
                    port,
                    tun_mode,
                    (Duration::from_millis(scan_interval_ms), retry),
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
        .map_err(|_| crate::output::input_decision("tunnel manager URL cannot be a base"))?
        .push("tunnels");
    Ok(url.to_string())
}

fn manager_tunnel_url(host: &str, port: u16, udid: &str) -> Result<String> {
    let mut url = manager_base_url(host, port)?;
    url.path_segments_mut()
        .map_err(|_| crate::output::input_decision("tunnel manager URL cannot be a base"))?
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
        crate::output::write_error_line(format_args!(
            "Starting {mode} tunnel for {}...",
            ios_core::redaction::diagnostic_id(udid)
        ))?;
    } else {
        crate::output::write_error_line(format_args!("tunnel: connecting ({mode})..."))?;
    }

    let native = retirement::Registry::new();
    let _owner = StandaloneOwner(Arc::clone(&native));
    let ownership = ownership::Ownership::standalone(Arc::clone(&native))?;
    let primary = async {
        let connect_timeout = Duration::from_secs(30);
        let device =
            tokio::time::timeout(connect_timeout, connect_with_owner(udid, opts, &ownership))
                .await
                .map_err(|_| {
                    anyhow::anyhow!(
                        "tunnel connect timed out after {}s — the device may not support \
                 CoreDevice tunneling or the handshake is blocked",
                        connect_timeout.as_secs()
                    )
                })??;
        let _lease = ownership.take_completed(
            device
                .tunnel_handle()
                .ok_or_else(|| lifecycle::cancelled().context("native tunnel handle missing"))?,
        )?;
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
        Ok::<(), anyhow::Error>(())
    }
    .await;
    let clock = retirement::CleanupClock::new(server::CLEANUP_TIMEOUT);
    native.close_and_stop_all();
    drop(ownership);
    let cleanup = match clock {
        Ok(clock) => server::drain_native_terminal(&native, clock).await,
        Err(error) => Err(error),
    };
    match (primary, cleanup) {
        (Ok(()), cleanup) => cleanup,
        (Err(primary), Ok(())) => Err(primary),
        (Err(primary), Err(cleanup)) => Err(server::CleanupFailure { primary, cleanup }.into()),
    }
}

struct StandaloneOwner(Arc<retirement::Registry>);

impl Drop for StandaloneOwner {
    fn drop(&mut self) {
        self.0.close_and_stop_all();
    }
}

#[derive(Clone)]
struct TunnelAgentState {
    inner: Arc<TunnelAgentInner>,
}

#[cfg(test)]
type PublicationCallback = Box<dyn FnOnce() + Send>;

struct TunnelAgentInner {
    access: AccessControl,
    tun_mode: TunMode,
    tunnels: RwLock<HashMap<String, ManagedTunnel>>,
    upstreams: RwLock<BTreeSet<String>>,
    upstream_access: Upstreams,
    failed_attempts: RwLock<HashMap<String, FailedTunnelAttempt>>,
    retry: Arc<reconnect::Ledger>,
    failure_slots: Arc<Semaphore>,
    metadata_bytes: Arc<Semaphore>,
    global_metadata_bytes: Arc<Semaphore>,
    ready: RwLock<bool>,
    lifecycle: Lifecycle,
    scan_lock: Mutex<()>,
    scan_outcomes: Arc<scan::Outcomes>,
    scan_timeout: Duration,
    source: Arc<dyn Source>,
    connect_timeout: Duration,
    failure_write_timeout: Duration,
    shutdown: watch::Sender<bool>,
    native: Arc<retirement::Registry>,
    #[cfg(test)]
    publication_after_insert: std::sync::Mutex<Option<PublicationCallback>>,
    #[cfg(test)]
    paired_after_first_check: std::sync::Mutex<Option<PublicationCallback>>,
    #[cfg(test)]
    failure_after_insert: std::sync::Mutex<Option<PublicationCallback>>,
    #[cfg(test)]
    success_before_remove: std::sync::Mutex<Option<PublicationCallback>>,
    #[cfg(test)]
    success_after_remove: std::sync::Mutex<Option<PublicationCallback>>,
}

struct ConnectionContext<'a> {
    expected_id: Option<u32>,
    deadline: Option<&'a lifecycle::ConnectionDeadline>,
    retry: Option<(AttemptIdentity, Arc<OwnedSemaphorePermit>)>,
    manual: Option<&'a intent::ManualRequest>,
    input: Option<Arc<candidate_input::Input>>,
}

struct ManagedTunnel {
    device: ManagedDevice,
    publication: Arc<()>,
    usbmux: bool,
    native: Option<retirement::NativeLease>,
    // Actual selector established by Source, not the next caller's request.
    route_intent: intent::RouteIntent,
    // Exact observed Source route, separate from a retained rediscovery filter.
    observed_route: Route,
    _ticket: Ticket,
    // Table metadata and the original native retirement owner share one charge.
    // Only removal of both owners releases it; either may outlive the other.
    _metadata: Arc<OwnedSemaphorePermit>,
    _global_metadata: Arc<OwnedSemaphorePermit>,
    _input: Option<Arc<candidate_input::Input>>,
}

impl Drop for ManagedTunnel {
    fn drop(&mut self) {
        // The independent registry keeps the original runtime through every
        // removal path, even when the table's metadata owner is destroyed.
        drop(self.native.take());
    }
}

enum ManagedDevice {
    Native(Box<ConnectedDevice>),
    #[cfg(test)]
    Fake(Box<dyn source::FakeDevice>),
}

fn managed_metadata_bytes(
    device: &ManagedDevice,
    udid: &str,
    desired: &intent::RouteIntent,
    observed: &Route,
) -> Result<usize> {
    let overflow = || crate::output::resource_decision("tunnel retained metadata size overflow");
    let device_bytes = match device {
        ManagedDevice::Native(device) => device.retained_metadata_bytes().ok_or_else(overflow)?,
        #[cfg(test)]
        ManagedDevice::Fake(_) => std::mem::size_of::<ManagedDevice>(),
    };
    let desired_bytes = match desired {
        intent::RouteIntent::Usbmux => 0,
        intent::RouteIntent::Usb(host)
        | intent::RouteIntent::Wifi(host)
        | intent::RouteIntent::Mobdev2(host) => host.as_ref().map_or(0, String::capacity),
    };
    let observed_bytes = match observed {
        Route::Usbmux => 0,
        Route::Usb(host) | Route::Wifi(host) => host.as_ref().map_or(0, String::capacity),
        Route::Mobdev2(host) => host.capacity(),
    };
    // Include the table key, row and both distinct retained route strings.
    // Hash table backing, allocator overhead, runtime buffers and dyn Error
    // allocations remain separate; this is a known-capacity admission charge.
    device_bytes
        .checked_add(std::mem::size_of::<ManagedTunnel>())
        .and_then(|bytes| bytes.checked_add(udid.len()))
        .and_then(|bytes| bytes.checked_add(desired_bytes))
        .and_then(|bytes| bytes.checked_add(observed_bytes))
        .ok_or_else(overflow)
}

#[derive(Debug, Clone)]
struct FailedTunnelAttempt {
    // Every committed failure write gets a fresh identity, even with a paused
    // clock, a saturated fail count or a reused reservation permit.
    version: Arc<()>,
    last_attempt: Instant,
    fail_count: u32,
    identity: AttemptIdentity,
    slot: Option<Arc<OwnedSemaphorePermit>>,
}

struct ScanCleanupFailure {
    primary: anyhow::Error,
    cleanup: anyhow::Error,
}

impl std::fmt::Display for ScanCleanupFailure {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("tunnel scan failed; snapshot cleanup also failed")
    }
}

impl std::fmt::Debug for ScanCleanupFailure {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("TunnelScanCleanupFailure")
            .field(
                "primary_class",
                &ios_core::classify_error(self.primary.as_ref()),
            )
            .field(
                "cleanup_class",
                &ios_core::classify_error(self.cleanup.as_ref()),
            )
            .finish()
    }
}

impl std::error::Error for ScanCleanupFailure {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        Some(self.primary.as_ref())
    }
}

#[derive(Clone, PartialEq, Eq)]
struct AttemptIdentity {
    route: Route,
    native_id: Option<u32>,
}

impl std::fmt::Debug for AttemptIdentity {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("TunnelAttemptIdentity")
    }
}

impl FailedTunnelAttempt {
    #[cfg(test)]
    fn should_skip(&self, now: Instant) -> bool {
        now.duration_since(self.last_attempt) < failed_device_backoff(self.fail_count)
    }

    fn should_skip_with_policy(&self, now: Instant, policy: &reconnect::RetryPolicy) -> bool {
        now.duration_since(self.last_attempt) < policy.delay(self.fail_count)
    }
}

#[cfg(test)]
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

    #[cfg(test)]
    fn with_upstreams(
        tun_mode: TunMode,
        access: AccessControl,
        upstream_access: Upstreams,
    ) -> Self {
        Self::with_source(
            tun_mode,
            access,
            upstream_access,
            Arc::new(source::NoDevices),
            TUNNEL_CONNECT_TIMEOUT,
        )
    }

    #[cfg(test)]
    fn with_source(
        tun_mode: TunMode,
        access: AccessControl,
        upstream_access: Upstreams,
        source: Arc<dyn Source>,
        connect_timeout: Duration,
    ) -> Self {
        Self::with_source_and_retry(
            tun_mode,
            access,
            upstream_access,
            source,
            connect_timeout,
            reconnect::RetryPolicy::default(),
        )
    }

    fn with_source_and_retry(
        tun_mode: TunMode,
        access: AccessControl,
        upstream_access: Upstreams,
        source: Arc<dyn Source>,
        connect_timeout: Duration,
        retry: reconnect::RetryPolicy,
    ) -> Self {
        let global_metadata_bytes = Arc::new(Semaphore::new(MAX_GLOBAL_KNOWN_METADATA_BYTES));
        Self {
            inner: Arc::new(TunnelAgentInner {
                access,
                tun_mode,
                tunnels: RwLock::new(HashMap::new()),
                upstreams: RwLock::new(upstream_access.urls().cloned().collect()),
                upstream_access,
                failed_attempts: RwLock::new(HashMap::new()),
                retry: reconnect::Ledger::new(retry),
                failure_slots: Arc::new(Semaphore::new(MAX_FAILURE_RECORDS)),
                metadata_bytes: Arc::new(Semaphore::new(MAX_PUBLISHED_METADATA_BYTES)),
                global_metadata_bytes: Arc::clone(&global_metadata_bytes),
                ready: RwLock::new(false),
                lifecycle: Lifecycle::with_metadata(global_metadata_bytes),
                scan_lock: Mutex::new(()),
                scan_outcomes: scan::Outcomes::new(),
                scan_timeout: TUNNEL_SCAN_TIMEOUT,
                source,
                connect_timeout,
                failure_write_timeout: FAILURE_WRITE_TIMEOUT,
                shutdown: watch::channel(false).0,
                native: retirement::Registry::new(),
                #[cfg(test)]
                publication_after_insert: std::sync::Mutex::new(None),
                #[cfg(test)]
                paired_after_first_check: std::sync::Mutex::new(None),
                #[cfg(test)]
                failure_after_insert: std::sync::Mutex::new(None),
                #[cfg(test)]
                success_before_remove: std::sync::Mutex::new(None),
                #[cfg(test)]
                success_after_remove: std::sync::Mutex::new(None),
            }),
        }
    }

    async fn is_ready(&self) -> bool {
        *self.inner.ready.read().await
    }

    #[cfg(test)]
    fn set_global_metadata_limit(&mut self, bytes: usize) {
        let inner = Arc::get_mut(&mut self.inner).expect("configure before sharing state");
        let bank = Arc::new(Semaphore::new(bytes));
        inner.lifecycle = Lifecycle::with_metadata(Arc::clone(&bank));
        inner.global_metadata_bytes = bank;
    }

    fn notify_shutdown(&self) {
        // Fence both existing and future starts before any async table cleanup.
        self.inner.lifecycle.close();
        self.inner.retry.close();
        self.inner.scan_outcomes.close();
        self.inner.native.close_and_stop_all();
        self.inner.shutdown.send_replace(true);
    }

    async fn wait_shutdown(&self) {
        let mut shutdown = self.inner.shutdown.subscribe();
        let _ = shutdown.wait_for(|stopped| *stopped).await;
    }

    async fn drain_native(&self, budget: Duration) -> Result<()> {
        let clock = retirement::CleanupClock::new(budget)?;
        let report = self.inner.native.drain_retired(clock).await?;
        if report.selected != 0 {
            tracing::debug!(
                selected = report.selected,
                joined = report.joined,
                native_handles = report.snapshot.native_handles,
                additional_failed_owners = report.snapshot.additional_failed_owners,
                historical_failure_class = ?report.snapshot.historical_failure_class,
                "native tunnel retirement collected"
            );
        }
        debug_assert!(report.snapshot.occupied() <= report.snapshot.capacity);
        Ok(())
    }

    async fn clear(&self) {
        let mut tunnels = self.inner.tunnels.write().await;
        let old_epoch = self.inner.lifecycle.clear_owned();
        let old = std::mem::take(&mut *tunnels);
        drop(tunnels);
        if let Some(old_epoch) = old_epoch {
            self.inner.native.retire_global(&old_epoch);
            let retired = self
                .inner
                .retry
                .remove_where(|_, _, ticket| ticket.belongs_to_global(&old_epoch));
            reconnect::Ledger::cancelled(retired, reconnect::Reason::Cleared);
        }
        drop(old);
        self.inner.failed_attempts.write().await.clear();
        self.inner.retry.clear_retired_causes();
    }

    async fn list_records(&self) -> Vec<TunnelRecord> {
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

    #[cfg(test)]
    async fn remove_tunnel(&self, udid: &str) -> bool {
        self.stop_tunnel(udid).await.unwrap_or(false)
    }

    async fn stop_tunnel(&self, udid: &str) -> Result<bool> {
        admission::selectors(udid, None)?;
        // Reserve the durable stopped key before mutation. At capacity a
        // published/pending known target reuses its gate; an unknown target is
        // rejected explicitly, never evicting another stop or recovery intent.
        let ticket = self.inner.lifecycle.ticket(udid).await?;
        self.stop_tunnel_with_ticket(udid, &ticket).await
    }

    async fn stop_tunnel_with_ticket(&self, udid: &str, ticket: &Ticket) -> Result<bool> {
        let mut tunnels = self.inner.tunnels.write().await;
        anyhow::ensure!(ticket.is_current(), lifecycle::cancelled());
        let old_epoch = self.inner.lifecycle.stop_owned(udid).await;
        let alive = tunnels.get(udid).is_some_and(ManagedTunnel::is_alive);
        let old = tunnels.remove(udid);
        drop(tunnels);
        if let Some(old_epoch) = old_epoch {
            self.inner.native.retire_device(&old_epoch);
            let retired = self
                .inner
                .retry
                .remove_where(|_, _, ticket| ticket.belongs_to_device(&old_epoch));
            reconnect::Ledger::cancelled(retired, reconnect::Reason::Stopped);
        }
        drop(old);
        Ok(alive)
    }

    #[cfg(test)]
    async fn ensure_tunnel(
        &self,
        udid: &str,
        ip_filter: Option<&str>,
        requested_connection_type: Option<StartTunnelConnectionType>,
    ) -> Result<TunnelRecord> {
        self.ensure_tunnel_with_intent(udid, ip_filter, requested_connection_type, None)
            .await
    }

    async fn ensure_tunnel_with_intent(
        &self,
        udid: &str,
        ip_filter: Option<&str>,
        requested_connection_type: Option<StartTunnelConnectionType>,
        manual: Option<intent::ManualRequest>,
    ) -> Result<TunnelRecord> {
        admission::selectors(udid, ip_filter)?;
        anyhow::ensure!(
            manual.is_none() || !udid.is_empty(),
            crate::output::input_decision(
                "manual tunnel recovery requires an exact device identifier"
            )
        );
        let deadline = lifecycle::ConnectionDeadline::new(self.inner.connect_timeout)?;
        let ticket = deadline.run(self.inner.lifecycle.ticket(udid)).await?;
        ticket
            .run(deadline.run(async {
                let _serial = ticket.serial().await;
                if let Some(record) = self.get_record(udid).await {
                    if let Some(manual) = &manual {
                        let tunnels = self.inner.tunnels.write().await;
                        if let Some(current) =
                            tunnels.get(udid).filter(|current| current.is_alive())
                        {
                            // Preserve the legacy live shortcut's actual route.
                            // The requested different transport never ran Source.
                            let route = current.route_intent.clone();
                            ticket.commit(|| {
                                manual.authority.commit(udid, || {
                                    ticket.establish_manual(
                                        route,
                                        manual.authority.clone(),
                                        ticket.failure_version(),
                                        Some(&deadline),
                                    )
                                })
                            })?;
                        } else {
                            return Err(lifecycle::cancelled());
                        }
                    }
                    self.try_manual_success(udid, &ticket, &deadline);
                    return Ok(record);
                }
                let _permit = self.inner.lifecycle.connects.acquire().await?;
                let route = match requested_connection_type {
                    Some(StartTunnelConnectionType::Usb) => {
                        Route::Usb(ip_filter.map(str::to_owned))
                    }
                    Some(StartTunnelConnectionType::Wifi) => {
                        Route::Wifi(ip_filter.map(str::to_owned))
                    }
                    Some(StartTunnelConnectionType::Usbmux) | None => Route::Usbmux,
                };
                match self
                    .connect_and_store_with_context(
                        udid,
                        route,
                        &ticket,
                        ConnectionContext {
                            expected_id: None,
                            deadline: Some(&deadline),
                            retry: None,
                            manual: manual.as_ref(),
                            input: None,
                        },
                    )
                    .await
                {
                    Ok(record) => Ok(record),
                    Err(error) if requested_connection_type.is_some() => Err(error),
                    // Preserve an already acquired Source error without opening
                    // another discovery/transport after the total has expired.
                    Err(error) if deadline.check().is_err() => Err(error),
                    Err(usbmux_error) => {
                        let targets = self
                            .inner
                            .source
                            .paired_devices()
                            .await
                            .context("failed to browse paired mobdev2 devices")?;
                        let _snapshot_charge = admission::retain_paired_snapshot(
                            &self.inner.global_metadata_bytes,
                            &targets,
                        )?;
                        let Some(target) = targets.into_iter().find(|target| {
                            target.udid == udid && ip_filter.map_or(true, |ip| ip == target.host)
                        }) else {
                            return Err(usbmux_error.context("no paired mobdev2 target matched"));
                        };
                        self.connect_and_store_with_context(
                            udid,
                            Route::Mobdev2(target.host),
                            &ticket,
                            ConnectionContext {
                                expected_id: None,
                                deadline: Some(&deadline),
                                retry: None,
                                manual: manual.as_ref(),
                                input: None,
                            },
                        )
                        .await
                    }
                }
            }))
            .await
    }

    #[cfg(test)]
    async fn connect_and_store(
        &self,
        udid: &str,
        route: Route,
        ticket: &Ticket,
        expected_id: Option<u32>,
        deadline: Option<&lifecycle::ConnectionDeadline>,
    ) -> Result<TunnelRecord> {
        self.connect_and_store_with_retry(udid, route, ticket, expected_id, deadline, None)
            .await
    }

    #[cfg(test)]
    async fn connect_and_store_with_retry(
        &self,
        udid: &str,
        route: Route,
        ticket: &Ticket,
        expected_id: Option<u32>,
        deadline: Option<&lifecycle::ConnectionDeadline>,
        retry: Option<(AttemptIdentity, Arc<OwnedSemaphorePermit>)>,
    ) -> Result<TunnelRecord> {
        self.connect_and_store_with_context(
            udid,
            route,
            ticket,
            ConnectionContext {
                expected_id,
                deadline,
                retry,
                manual: None,
                input: None,
            },
        )
        .await
    }

    async fn connect_and_store_with_context(
        &self,
        udid: &str,
        route: Route,
        ticket: &Ticket,
        context: ConnectionContext<'_>,
    ) -> Result<TunnelRecord> {
        let ConnectionContext {
            expected_id,
            deadline,
            retry,
            manual,
            input,
        } = context;
        if let Some(deadline) = deadline {
            deadline.check()?;
        }
        let usbmux = matches!(route, Route::Usbmux | Route::Usb(_));
        let desired = ticket.desired();
        let authority = manual.map(|request| request.authority.clone()).or_else(|| {
            if let intent::Desired::Manual { authority, .. } = &desired.desired {
                Some(authority.clone())
            } else {
                None
            }
        });
        let route_intent = match (&manual, &desired.desired) {
            (None, intent::Desired::Manual { route, .. }) => route.clone(),
            _ => intent::RouteIntent::established(
                &route,
                manual.and_then(|request| request.host_filter.as_deref()),
            ),
        };
        let previous_failure = if manual.is_some() {
            self.inner
                .failed_attempts
                .try_read()
                .ok()
                .and_then(|failed| failed.get(udid).map(|failed| failed.version.clone()))
                .or_else(|| ticket.failure_version())
        } else {
            None
        };
        let ownership = ownership::Ownership::admitted_with_input(
            Arc::clone(&self.inner.native),
            ticket,
            input.clone(),
        )?;
        let manual_version = if retry.is_none() {
            ticket.commit(|| Ok(self.inner.retry.version(udid, ticket)))?
        } else {
            None
        };
        // Both capacity admissions succeeded. No rejected owner/permit consumes
        // a retry or starts the total clock; Source is the next polled operation.
        let attempt = retry
            .map(|(identity, slot)| {
                ticket.commit(|| {
                    if let Some(deadline) = deadline {
                        deadline.check()?;
                    }
                    self.inner.retry.start(udid, identity, ticket, slot)
                })
            })
            .transpose();
        self.inner.retry.emit_pending(udid, ticket);
        let attempt = attempt?;
        if let (Some(attempt), Some(deadline)) = (&attempt, deadline) {
            if let Some(at) = attempt.deadline() {
                deadline.clamp_retry(at);
            }
        }
        let result = async {
            if let Some(deadline) = deadline {
                deadline.check()?;
            }
            let observed_route = route.clone();
            let mut source =
                self.inner
                    .source
                    .connect_owned(udid, route, self.inner.tun_mode, &ownership);
            let mut emitted = false;
            let device = std::future::poll_fn(|cx| {
                let result = std::future::Future::poll(source.as_mut(), cx);
                if !emitted {
                    emitted = true;
                    if let Some(attempt) = &attempt {
                        attempt.emit_start();
                    }
                }
                result
            })
            .await
            .map_err(|error| {
                let error = match &input {
                    Some(input) => error.context(candidate_input::SourceInputContext {
                        input: Arc::clone(input),
                    }),
                    None => error,
                };
                match &attempt {
                    Some(attempt) => attempt.source_failed(error),
                    None => error,
                }
            })?;
            if let Some(deadline) = deadline {
                deadline.check()?;
            }
            let metadata_size =
                managed_metadata_bytes(&device, udid, &route_intent, &observed_route)?;
            let global_metadata =
                admission::known_metadata_charge(&self.inner.global_metadata_bytes, metadata_size)?;
            let metadata = self
                .inner
                .metadata_bytes
                .clone()
                .try_acquire_many_owned(u32::try_from(metadata_size).map_err(|_| {
                    crate::output::resource_decision("tunnel metadata admission overflow")
                })?)
                .map_err(|_| {
                    crate::output::resource_decision(
                        "tunnel published metadata byte limit exceeded",
                    )
                })?;
            let native = match device.native_handle() {
                Some(handle) => Some(ownership.take_completed(handle)?),
                None => {
                    #[cfg(not(test))]
                    return Err(lifecycle::cancelled().context("native tunnel handle missing"));
                    #[cfg(test)]
                    {
                        if matches!(device, ManagedDevice::Native(_)) {
                            return Err(
                                lifecycle::cancelled().context("native tunnel handle missing")
                            );
                        }
                        None
                    }
                }
            };
            let metadata = Arc::new(metadata);
            let global_metadata = Arc::new(global_metadata);
            if let Some(native) = &native {
                native.retain_metadata(Arc::clone(&metadata))?;
                native.retain_global_metadata(Arc::clone(&global_metadata))?;
            }
            if let Some(deadline) = deadline {
                deadline.check()?;
            }
            if expected_id.is_some_and(|expected| expected != device.device_id()) {
                return Err(lifecycle::cancelled().context("tunnel attachment snapshot changed"));
            }
            let record = device.record_for_mode(self.inner.tun_mode)?;
            if authority.is_some() && record.udid != udid {
                return Err(ios_core::CoreError::Protocol(
                    "tunnel Source identity differs from authorized device".into(),
                )
                .into());
            }
            if let Some(deadline) = deadline {
                deadline.check()?;
            }
            if !device.is_alive() {
                return Err(lifecycle::cancelled().context("tunnel inactive at admission"));
            }
            if usbmux {
                if let Some(deadline) = deadline {
                    deadline.check()?;
                }
                // Keep the completed device owned while the current daemon view is
                // checked, within the caller's existing permit/deadline/cancellation.
                match self.inner.source.usb_devices().await {
                    UsbSnapshot::Devices(devices) => {
                        let _snapshot_charge = admission::retain_usb_snapshot(
                            &self.inner.global_metadata_bytes,
                            &devices,
                        )?;
                        if !devices.iter().any(|attached| {
                            attached.udid == udid && attached.device_id == device.device_id()
                        }) {
                            return Err(lifecycle::cancelled()
                                .context("tunnel attachment absent before publication"));
                        }
                    }
                    UsbSnapshot::ConnectError(error) | UsbSnapshot::ListError(error) => {
                        return Err(error.context("check tunnel attachment before publication"));
                    }
                }
            }
            let mut tunnels = self.inner.tunnels.write().await;
            if !ticket.is_current() {
                return Err(lifecycle::cancelled());
            }
            anyhow::ensure!(
                self.inner.access.permits_device(udid),
                lifecycle::cancelled()
            );
            if !device.is_alive() {
                return Err(lifecycle::cancelled().context("tunnel inactive before publication"));
            }
            // Keep the acquired guard outside the action so a rejected commit
            // retires it after lifecycle watch/table guards have been released.
            let mut acquired = Some((device, native));
            let mut late_record = None;
            let publish = || {
                if let Some(deadline) = deadline {
                    deadline.check()?;
                }
                if let Some(native) = &acquired.as_ref().expect("unpublished owner").1 {
                    native.publish(ownership.stamp())?;
                }
                if let Some(deadline) = deadline {
                    deadline.check()?;
                }
                let (device, native) = acquired.take().expect("single publication");
                let old = tunnels.insert(
                    udid.to_owned(),
                    ManagedTunnel {
                        device,
                        publication: Arc::new(()),
                        usbmux,
                        native,
                        route_intent: route_intent.clone(),
                        observed_route,
                        _ticket: ticket.clone(),
                        _metadata: metadata,
                        _global_metadata: global_metadata,
                        _input: input.clone(),
                    },
                );
                #[cfg(test)]
                {
                    let callback = self.inner.publication_after_insert.lock().unwrap().take();
                    if let Some(callback) = callback {
                        callback();
                    }
                }
                let late = deadline
                    .map_or(Ok(()), |deadline| deadline.check())
                    .and_then(|()| {
                        authority
                            .as_ref()
                            .map_or(Ok(()), |authority| authority.check_clock())
                    })
                    .and_then(|()| {
                        if let Some(manual) = manual {
                            ticket.establish_manual(
                                route_intent,
                                manual.authority.clone(),
                                previous_failure,
                                deadline,
                            )?;
                        }
                        Ok(())
                    });
                if let Err(error) = late {
                    // Nothing has observed this row while the table and epoch
                    // fences are held. Restore the displaced original row and
                    // retire the late runtime only after releasing those guards.
                    late_record = tunnels.remove(udid);
                    if let Some(old) = old {
                        tunnels.insert(udid.to_owned(), old);
                    }
                    return Err(error);
                }
                Ok((record, old))
            };
            let result = ticket.commit(|| {
                if let Some(authority) = &authority {
                    authority.commit(udid, publish)
                } else {
                    publish()
                }
            });
            drop(tunnels);
            drop(late_record);
            result.map(|(record, old)| {
                drop(old);
                record
            })
        }
        .await;
        if let Some(attempt) = attempt {
            attempt.finish(&result);
        }
        if result.is_ok() {
            if let Some(version) = manual_version {
                self.inner.retry.recovered(udid, ticket, &version);
            }
            if let Some(deadline) = deadline {
                self.try_manual_success(udid, ticket, deadline);
            }
        }
        result
    }

    // Cleanup is optional after successful publication: a held writer/expired
    // metadata budget leaves one exact-version outcome on the bounded gate.
    // It never rolls back a successfully established route or spawns a task.
    fn try_manual_success(
        &self,
        udid: &str,
        ticket: &Ticket,
        deadline: &lifecycle::ConnectionDeadline,
    ) {
        let Some((desired, success)) = ticket.pending_success() else {
            return;
        };
        let fenced = ticket.fenced();
        if let Ok(mut failed) = self.inner.failed_attempts.try_write() {
            let result = self.remove_success_row(&mut failed, udid, &success, &fenced, deadline);
            drop(failed);
            if let Ok(old) = result {
                ticket.clear_success(&desired, &success.version);
                drop(old);
            }
        }
    }

    #[cfg(test)]
    async fn refresh_connect(
        &self,
        udid: &str,
        route: Route,
        ticket: &Ticket,
        expected_id: Option<u32>,
    ) {
        // Keep the original direct-attempt test seam and its immediate failure
        // write. Production scans defer that write until their window finishes.
        if let Some(failed) = self
            .refresh_connect_deferred(udid, route, ticket, expected_id)
            .await
        {
            self.record_failed_attempt(udid, failed.identity, failed.slot, ticket)
                .await;
        }
    }

    #[cfg(test)]
    async fn refresh_connect_deferred(
        &self,
        udid: &str,
        route: Route,
        ticket: &Ticket,
        expected_id: Option<u32>,
    ) -> Option<DeferredFailure> {
        match self
            .refresh_connect_with_total(udid, route, ticket, expected_id, None)
            .await
        {
            ScanAttemptOutcome::Failure(failed) => Some(failed),
            ScanAttemptOutcome::Success(success) => {
                // Keep the original direct-attempt seam's completed cleanup.
                // Production defers contested writers until the window ends.
                if let Ok(deadline) = lifecycle::ConnectionDeadline::for_scan_bookkeeping(
                    self.inner.failure_write_timeout,
                ) {
                    let _ = deadline
                        .run(async {
                            let mut failed = self.inner.failed_attempts.write().await;
                            let result = self.remove_success_row(
                                &mut failed,
                                udid,
                                &success,
                                ticket,
                                &deadline,
                            );
                            drop(failed);
                            result.map(drop)
                        })
                        .await;
                }
                None
            }
            ScanAttemptOutcome::NoChange => None,
        }
    }

    #[cfg(test)]
    async fn refresh_connect_with_total(
        &self,
        udid: &str,
        route: Route,
        ticket: &Ticket,
        expected_id: Option<u32>,
        whole: Option<&scan::Deadline>,
    ) -> ScanAttemptOutcome {
        self.refresh_connect_with_input(udid, route, ticket, expected_id, whole, None)
            .await
    }

    async fn refresh_connect_with_input(
        &self,
        udid: &str,
        route: Route,
        ticket: &Ticket,
        expected_id: Option<u32>,
        whole: Option<&scan::Deadline>,
        input: Option<Arc<candidate_input::Input>>,
    ) -> ScanAttemptOutcome {
        let identity = AttemptIdentity {
            route: route.clone(),
            native_id: expected_id,
        };
        // Keep the reservation outside the cancellable connection future until
        // its failure is recorded or the attempt releases it.
        let mut failure_slot = None;
        let mut established = false;
        let mut success = None;
        let authority = match ticket.desired().desired {
            intent::Desired::Stopped => return ScanAttemptOutcome::NoChange,
            intent::Desired::Manual { authority, .. } => Some(authority),
            intent::Desired::Automatic => None,
        };
        let work = ticket.run(async {
            let deadline =
                lifecycle::ConnectionDeadline::for_scan_candidate(self.inner.connect_timeout)?
                    .within_scan(whole);
            let (retry, until) = ticket.commit(|| {
                Ok((
                    self.inner.retry.admission(udid, &identity, ticket),
                    self.inner.retry.deadline(udid, &identity, ticket),
                ))
            })?;
            self.inner.retry.emit_pending(udid, ticket);
            if retry != reconnect::Admission::Ready {
                return Ok(());
            }
            // An existing episode also bounds serial/metadata/permit waits.
            // A first episode still starts only when Source is admitted.
            if let Some(until) = until {
                deadline.clamp_retry(until);
            }
            deadline
                .run(async {
                    let _serial = ticket.serial().await;
                    let mut tunnels = self.inner.tunnels.write().await;
                    deadline.check()?;
                    let (existing, remove_dead) = match tunnels.get(udid) {
                        Some(tunnel) if tunnel.is_alive() => {
                            let record = tunnel.record().ok();
                            deadline.check()?;
                            (record, false)
                        }
                        Some(_) => (None, true),
                        None => (None, false),
                    };
                    let removed = ticket.commit(|| {
                        deadline.check()?;
                        Ok(if remove_dead {
                            tunnels.remove(udid)
                        } else {
                            None
                        })
                    })?;
                    drop(tunnels);
                    drop(removed);
                    if existing.is_some() {
                        return Ok(());
                    }
                    let (skip, previous_version) =
                        self.inner.failed_attempts.read().await.get(udid).map_or(
                            (false, None),
                            |failed| {
                                (
                                    ticket.failure_is_current(&failed.version)
                                        && failed.identity == identity
                                        && failed.should_skip_with_policy(
                                            Instant::now(),
                                            self.inner.retry.policy(),
                                        ),
                                    Some(Arc::clone(&failed.version)),
                                )
                            },
                        );
                    if skip {
                        return Ok(());
                    }
                    let slot = self.reserve_failure_slot(udid).await?;
                    failure_slot = Some(Arc::clone(&slot));
                    let _permit = self.inner.lifecycle.connects.acquire().await?;
                    deadline.check()?;
                    self.connect_and_store_with_context(
                        udid,
                        route,
                        ticket,
                        ConnectionContext {
                            expected_id,
                            deadline: Some(&deadline),
                            retry: Some((identity.clone(), slot)),
                            manual: None,
                            input: input.clone(),
                        },
                    )
                    .await?;
                    // Publication completed within the candidate budget.
                    // Contested cleanup must not queue a writer while later
                    // candidates still need failure-table reads, or turn a
                    // successful connection into a failure when cleanup waits.
                    established = true;
                    if let Some(version) = previous_version {
                        let pending = DeferredSuccess { version };
                        if let Ok(mut failed) = self.inner.failed_attempts.try_write() {
                            let cleanup = self.remove_success_row(
                                &mut failed,
                                udid,
                                &pending,
                                ticket,
                                &deadline,
                            );
                            drop(failed);
                            match cleanup {
                                Ok(old) => drop(old),
                                Err(error) if error.is::<ios_core::OperationCancelled>() => {}
                                Err(_) => success = Some(pending),
                            }
                        } else {
                            success = Some(pending);
                        }
                    }
                    Ok(())
                })
                .await
        });
        let result = match &authority {
            Some(authority) => authority.run(work).await,
            None => work.await,
        };
        if authority
            .as_ref()
            .is_some_and(|authority| !authority.current())
        {
            // The captured desired version prevents revocation cleanup from
            // stopping a later independently authorized explicit publication.
            let _ = self.stop_tunnel_with_ticket(udid, ticket).await;
        }
        if established {
            return success.map_or(ScanAttemptOutcome::NoChange, ScanAttemptOutcome::Success);
        }
        if let Err(error) = result {
            // A lifecycle cancellation is not a connection failure and must not restart backoff.
            if ticket.is_current()
                && !error.is::<ios_core::OperationCancelled>()
                && !error.is::<scan::DeadlineReached>()
                && whole.map_or(true, |whole| whole.check().is_ok())
            {
                // A remaining episode clock can expire in a pre-Source lock or
                // semaphore wait. Publish that exhaustion without charging a
                // transport attempt or depending on the failure-table writer.
                let _ = ticket.commit(|| Ok(self.inner.retry.admission(udid, &identity, ticket)));
                self.inner.retry.emit_pending(udid, ticket);
                tracing::warn!(
                    device = %ios_core::redaction::diagnostic_id(udid),
                    error = %crate::output::error_message(&error),
                    "tunnel agent: failed to establish tunnel"
                );
                if let Some(slot) = failure_slot {
                    let receipt = error
                        .downcast_ref::<reconnect::SourceFailure>()
                        .map(reconnect::SourceFailure::receipt);
                    return ScanAttemptOutcome::Failure(DeferredFailure {
                        identity,
                        slot,
                        receipt,
                    });
                }
            }
        }
        ScanAttemptOutcome::NoChange
    }

    async fn refresh_once(&self) -> Result<()> {
        // Only scans serialize. Queries, stops and explicit starts never take this lock.
        let whole = scan::Deadline::new(self.inner.scan_timeout)?;
        let mut control = self
            .inner
            .lifecycle
            .scan_control(self.inner.connect_timeout)?;
        let epoch = control.epoch.clone();
        let lock = whole
            .run(&epoch, control.drive(self.inner.scan_lock.lock()))
            .await?;
        let Some(lock) = lock.flatten() else {
            return Ok(());
        };
        let session = self.inner.scan_outcomes.begin(lock)?;
        let result = whole
            .run(
                &epoch,
                self.refresh_scan(&mut control, &session, &whole, &epoch),
            )
            .await;
        session.finish(result)
    }

    async fn refresh_scan(
        &self,
        control: &mut lifecycle::ScanControl,
        session: &scan::Session<'_>,
        whole: &scan::Deadline,
        epoch: &scan::Epoch,
    ) -> Result<()> {
        let scan_generation = control.generation.clone();
        let Some(desired) = control.drive(self.inner.lifecycle.desired()).await? else {
            return Ok(());
        };
        let mut manual = HashMap::new();
        let mut suppressed = std::collections::HashSet::new();
        for (udid, ticket, snapshot) in desired {
            match snapshot.desired {
                intent::Desired::Manual { route, authority } if authority.current() => {
                    manual.insert(udid, (ticket, route));
                }
                intent::Desired::Manual { .. } => {
                    // Use this exact captured desired version: a concurrent
                    // authorized explicit start must survive obsolete cleanup.
                    let _ = self.stop_tunnel_with_ticket(&udid, &ticket).await;
                    suppressed.insert(udid);
                }
                intent::Desired::Stopped => {
                    suppressed.insert(udid);
                }
                intent::Desired::Automatic => {}
            }
        }
        let initial = {
            let Some(mut tunnels) = control.drive(self.inner.tunnels.write()).await? else {
                return Ok(());
            };
            epoch.retain(whole, &mut tunnels, |device, _| {
                self.inner.access.permits_device(device)
            })?;
            tunnels
                .iter()
                .map(|(udid, tunnel)| (udid.clone(), tunnel.publication.clone()))
                .collect::<HashMap<_, _>>()
        };
        if !self.inner.access.permits_any_device() {
            let Some(mut failed) = control.drive(self.inner.failed_attempts.write()).await? else {
                return Ok(());
            };
            epoch.commit(whole, || {
                failed.clear();
                Ok(())
            })?;
            drop(failed);
            let removed =
                epoch.commit(whole, || self.inner.retry.reconcile(whole, |_, _, _| true))?;
            reconnect::Ledger::cancelled(removed, reconnect::Reason::Detached);
            let Some(mut ready) = control.drive(self.inner.ready.write()).await? else {
                return Ok(());
            };
            epoch.commit(whole, || {
                *ready = true;
                Ok(())
            })?;
            return Ok(());
        }

        whole.check()?;
        let Some(snapshot) = control.drive(self.inner.source.usb_devices()).await? else {
            return Ok(());
        };
        let mut usb_complete = true;
        let devices = match snapshot {
            UsbSnapshot::Devices(devices) => {
                // Fence successful old snapshots without discarding a native
                // source error that the discovery future has already returned.
                if !self.inner.lifecycle.scan_is_current(&scan_generation) {
                    return Ok(());
                }
                devices
            }
            UsbSnapshot::ConnectError(error) => {
                if manual.is_empty() {
                    return Err(error);
                }
                usb_complete = false;
                session.retain(scan::Source::Usb, error);
                Vec::new()
            }
            UsbSnapshot::ListError(error) => {
                session.retain(scan::Source::Usb, error);
                match control.drive(self.inner.tunnels.write()).await {
                    Ok(Some(mut tunnels)) => {
                        epoch.retain(whole, &mut tunnels, |udid, tunnel| {
                            !initial
                                .get(udid)
                                .is_some_and(|old| Arc::ptr_eq(old, &tunnel.publication))
                                || manual.contains_key(udid)
                        })?;
                    }
                    Ok(None) => {}
                    Err(cleanup) => {
                        return Err(ScanCleanupFailure {
                            primary: session.take().expect("acquired list primary"),
                            cleanup,
                        }
                        .into());
                    }
                }
                if manual.is_empty() {
                    return Err(session.take().expect("acquired list primary"));
                }
                usb_complete = false;
                Vec::new()
            }
        };
        let admission = admission::retain_usb_snapshot(&self.inner.global_metadata_bytes, &devices);
        if !self.inner.lifecycle.scan_is_current(&scan_generation) {
            return Ok(());
        }
        let mut _usb_snapshot_charge = match admission {
            Ok(charge) => Some(charge),
            Err(error) => {
                if manual.is_empty() {
                    return Err(error);
                }
                usb_complete = false;
                session.retain(scan::Source::Usb, error);
                None
            }
        };
        let devices = if usb_complete { devices } else { Vec::new() };
        let attached = devices
            .iter()
            .map(|device| (device.udid.as_str(), device.device_id))
            .collect::<std::collections::HashSet<_>>();
        let Some(mut tunnels) = control.drive(self.inner.tunnels.write()).await? else {
            return Ok(());
        };
        epoch.retain(whole, &mut tunnels, |udid, tunnel| {
            tunnel.is_alive()
                && self.inner.access.permits_device(udid)
                && (!tunnel.usbmux
                    || !usb_complete
                    || attached.contains(&(udid, tunnel.device.device_id()))
                    || !initial
                        .get(udid)
                        .is_some_and(|old| Arc::ptr_eq(old, &tunnel.publication)))
        })?;
        drop(tunnels);
        drop(attached);

        // Capture every device generation before waiting for the first connection.
        // A stop while an earlier device stalls invalidates the queued ticket too.
        let mut candidates = metadata_vec::KnownVec::new(
            Arc::clone(&self.inner.global_metadata_bytes),
            MAX_FAILURE_RECORDS,
        );
        let mut seen = std::collections::HashSet::new();
        let mut candidate_budget = admission::CandidateBudget::default();
        let (selected, _usb_selection_charge) =
            source::selected_devices(devices, &self.inner.global_metadata_bytes)?;
        let attached_ids = selected
            .iter()
            .map(|device| (device.udid.as_str(), device.device_id))
            .collect::<HashMap<_, _>>();
        // Manual intent wins over ordinary discovery for the same exact key.
        // Hostless Wi-Fi is rediscovered inside NativeSource on each admitted
        // attempt; no old ConnectedDevice, RSD address or service is replayed.
        let mut pending_mobdev2 = Vec::new();
        for (udid, (ticket, route)) in &manual {
            if !ticket.is_current() {
                continue;
            }
            let (route, expected_id) = match route {
                intent::RouteIntent::Wifi(host) => (Route::Wifi(host.clone()), None),
                intent::RouteIntent::Usbmux | intent::RouteIntent::Usb(_) => {
                    let id = attached_ids.get(udid.as_str()).copied();
                    if usb_complete && id.is_none() {
                        continue;
                    }
                    let route = match route {
                        intent::RouteIntent::Usb(host) => Route::Usb(host.clone()),
                        _ => Route::Usbmux,
                    };
                    (route, id)
                }
                intent::RouteIntent::Mobdev2(host) => {
                    pending_mobdev2.push((udid.clone(), ticket.clone(), host.clone()));
                    continue;
                }
            };
            let bytes = match &route {
                Route::Usb(host) | Route::Wifi(host) => host.as_ref().map_or(0, String::len),
                _ => 0,
            };
            candidate_budget.admit(udid.len(), bytes)?;
            seen.insert(udid.clone());
            let input = candidate_input::Input::owned(
                udid.clone(),
                route,
                &self.inner.global_metadata_bytes,
            )?;
            candidates.push(candidate_index::Candidate {
                input,
                ticket: ticket.clone(),
                native_id: expected_id,
            })?;
        }
        drop(attached_ids);
        for device in selected {
            let udid = device.udid;
            if self.inner.access.permits_device(&udid)
                && !seen.contains(&udid)
                && !manual.contains_key(&udid)
                && !suppressed.contains(&udid)
            {
                candidate_budget.admit(udid.len(), 0)?;
                seen.insert(udid.clone());
                let Some(ticket) = control.drive(self.inner.lifecycle.ticket(&udid)).await? else {
                    return Ok(());
                };
                let ticket = match ticket {
                    Ok(ticket) => ticket.fenced(),
                    Err(error)
                        if crate::output::operation_error_class(error.as_ref())
                            == ios_core::ErrorClass::ResourceLimit =>
                    {
                        session.retain(scan::Source::Coordinator, error);
                        continue;
                    }
                    Err(error) => return Err(error),
                };
                let input = candidate_input::Input::usb(
                    udid,
                    _usb_snapshot_charge
                        .as_mut()
                        .expect("complete USB snapshot has charge"),
                    &self.inner.global_metadata_bytes,
                )?;
                candidates.push(candidate_index::Candidate {
                    input,
                    ticket,
                    native_id: Some(device.device_id),
                })?;
            }
        }
        if !self.inner.lifecycle.scan_is_current(&scan_generation) {
            return Ok(());
        }
        // Discover both families before connecting so a pending USB connection
        // cannot prevent a known mobdev2 candidate from entering the window.
        // Keep a discovery failure until the USB candidates have been processed,
        // preserving their publication and the original returned source error.
        whole.check()?;
        let Some(paired) = control.drive(self.inner.source.paired_devices()).await? else {
            return Ok(());
        };
        let paired = match paired {
            Ok(targets) => Some(targets),
            Err(error) => {
                session.retain(scan::Source::Paired, error);
                None
            }
        };
        if !self.inner.lifecycle.scan_is_current(&scan_generation) {
            // Discard a successful stale snapshot, but preserve the original
            // discovery error already acquired in this cancellation poll.
            return Ok(());
        }
        #[cfg(test)]
        {
            let callback = self.inner.paired_after_first_check.lock().unwrap().take();
            if let Some(callback) = callback {
                callback();
            }
        }
        // Discovery and candidate clocks remain independent sub-budgets.
        // The whole clock and global epoch also cover all later metadata waits.
        // Keep original Source Err outside the admission decision: a stale
        // Source success remains quietly discarded, even if its metadata fails
        // admission. Rejected snapshots are dropped before USB connection waits.
        // Admit complete paired metadata before a saved Mobdev2 selector uses
        // it. A failed/partial browse preserves that selector and episode.
        let mut _paired_snapshot_charge = None;
        let mut paired = paired.map(|targets| {
            let charge = match admission::retain_paired_snapshot(
                &self.inner.global_metadata_bytes,
                &targets,
            ) {
                Ok(charge) => charge,
                Err(error) => return (admission::PairedCandidates::Incomplete(error), Vec::new()),
            };
            let mut planned = Vec::new();
            for (udid, ticket, host) in pending_mobdev2 {
                if let Some(index) = targets.iter().position(|target| {
                    target.udid == udid && host.as_ref().map_or(true, |host| *host == target.host)
                }) {
                    if let Err(error) =
                        candidate_budget.admit(udid.len(), targets[index].host.len())
                    {
                        return (admission::PairedCandidates::Incomplete(error), Vec::new());
                    }
                    planned.push((udid, ticket, index));
                }
            }
            let reply = candidate_budget.paired_candidates(
                targets,
                &seen,
                |udid| {
                    self.inner.access.permits_device(udid)
                        && !suppressed.contains(udid)
                        && !manual.contains_key(udid)
                },
                &self.inner.global_metadata_bytes,
            );
            if matches!(&reply, admission::PairedCandidates::Complete { .. }) {
                _paired_snapshot_charge = Some(charge);
            }
            (reply, planned)
        });
        // Build and byte-admit the entire paired input batch before acquiring
        // any automatic paired ticket or mutating the existing USB window.
        if let Some((
            admission::PairedCandidates::Complete {
                targets, indices, ..
            },
            planned,
        )) = &paired
        {
            let prepare = (|| -> Result<_> {
                let mut inputs = metadata_vec::KnownVec::new(
                    Arc::clone(&self.inner.global_metadata_bytes),
                    MAX_FAILURE_RECORDS,
                );
                inputs.reserve(planned.len() + indices.len())?;
                for (udid, ticket, index) in planned {
                    if !seen.contains(udid) {
                        let input = candidate_input::Input::owned(
                            udid.clone(),
                            Route::Mobdev2(targets[*index].host.clone()),
                            &self.inner.global_metadata_bytes,
                        )?;
                        inputs.push((input, Some(ticket.clone())))?;
                    }
                }
                for &index in indices {
                    let target = &targets[index];
                    if self.inner.access.permits_device(&target.udid)
                        && !manual.contains_key(&target.udid)
                        && !suppressed.contains(&target.udid)
                        && !seen.contains(&target.udid)
                    {
                        let input = candidate_input::Input::owned(
                            target.udid.clone(),
                            Route::Mobdev2(target.host.clone()),
                            &self.inner.global_metadata_bytes,
                        )?;
                        inputs.push((input, None))?;
                    }
                }
                candidates.reserve(inputs.len())?;
                Ok(inputs)
            })();
            match prepare {
                Err(error) => {
                    _paired_snapshot_charge = None;
                    paired = Some((admission::PairedCandidates::Incomplete(error), Vec::new()));
                }
                Ok(inputs) => {
                    for (input, manual_ticket) in inputs {
                        if !self.inner.access.permits_device(&input.udid) {
                            continue;
                        }
                        if !seen.insert(input.udid.clone()) {
                            continue;
                        }
                        whole.check()?;
                        let ticket = match manual_ticket {
                            Some(ticket) => ticket,
                            None => match self.inner.lifecycle.ticket(&input.udid).await {
                                Ok(ticket) => ticket.fenced(),
                                Err(error)
                                    if crate::output::operation_error_class(error.as_ref())
                                        == ios_core::ErrorClass::ResourceLimit =>
                                {
                                    session.retain(scan::Source::Coordinator, error);
                                    continue;
                                }
                                Err(error) => return Err(error),
                            },
                        };
                        // Admission may have waited behind another coordinator
                        // while authorization was revoked. Rechecks can remove
                        // prepared inputs, never add an unplanned target.
                        if !self.inner.access.permits_device(&input.udid) {
                            continue;
                        }
                        candidates.push(candidate_index::Candidate {
                            input,
                            ticket,
                            native_id: None,
                        })?;
                    }
                }
            }
        }
        if !self.inner.lifecycle.scan_is_current(&scan_generation) {
            // An epoch can change after the first paired reply check too.
            // Preserve its already acquired error at this final handoff fence.
            return Ok(());
        }
        // Retain rejected current metadata before any candidate waits. A caller
        // drop must keep its original cause in the same bounded history slot.
        let paired = match paired {
            Some((admission::PairedCandidates::Incomplete(error), _)) => {
                session.retain(scan::Source::Paired, error);
                None
            }
            complete => complete.map(|(reply, _)| reply),
        };
        if let Some(admission::PairedCandidates::Complete { targets, .. }) = &paired {
            let observed = targets
                .iter()
                .map(|target| (target.udid.as_str(), target.host.as_str()))
                .collect::<std::collections::HashSet<_>>();
            let mut tunnels = self.inner.tunnels.write().await;
            epoch.retain(whole, &mut tunnels, |udid, tunnel| {
                // An older browse cannot retire a concurrent publication. A
                // failed/incomplete browse never reaches this complete view.
                !initial
                    .get(udid)
                    .is_some_and(|old| Arc::ptr_eq(old, &tunnel.publication))
                    || match &tunnel.observed_route {
                        Route::Mobdev2(host) => observed.contains(&(udid, host.as_str())),
                        _ => true,
                    }
            })?;
        }
        self.reconcile_failures(
            candidates.as_slice(),
            matches!(&paired, Some(admission::PairedCandidates::Complete { .. })),
            usb_complete,
            &manual,
            whole,
            epoch,
        )
        .await?;
        self.refresh_candidates(candidates, whole, session).await?;
        if session.has_primary() {
            return Ok(());
        }
        let mut ready = self.inner.ready.write().await;
        epoch.commit(whole, || {
            *ready = true;
            Ok(())
        })?;
        Ok(())
    }

    async fn refresh_candidates(
        &self,
        candidates: metadata_vec::KnownVec<candidate_index::Candidate>,
        whole: &scan::Deadline,
        session: &scan::Session<'_>,
    ) -> Result<()> {
        // Own these futures in the scanner itself. Dropping the scan cancels
        // every active connection and releases its serial guard/permit.
        let mut batch =
            bookkeeping::WriteBatch::new(&self.inner.global_metadata_bytes, candidates.len())?;
        session.retain_writes(batch.owner())?;
        let mut connections = futures_util::stream::iter(candidates)
            .map(|candidate| async move {
                let candidate_index::Candidate {
                    input,
                    ticket,
                    native_id,
                } = candidate;
                let route = match input.source_route() {
                    Ok(route) => route,
                    Err(error) => return Err(error),
                };
                match self
                    .refresh_connect_with_input(
                        &input.udid,
                        route,
                        &ticket,
                        native_id,
                        Some(whole),
                        Some(Arc::clone(&input)),
                    )
                    .await
                {
                    ScanAttemptOutcome::Failure(failed) => {
                        Ok(Some(ScanIntent::Failure(FailureIntent {
                            udid: input.udid.clone(),
                            ticket,
                            failed,
                            _input: Some(Arc::clone(&input)),
                        })))
                    }
                    ScanAttemptOutcome::Success(success) => {
                        Ok(Some(ScanIntent::Success(SuccessIntent {
                            udid: input.udid.clone(),
                            ticket,
                            success,
                            _input: Some(Arc::clone(&input)),
                        })))
                    }
                    ScanAttemptOutcome::NoChange => Ok(None),
                }
            })
            .buffer_unordered(lifecycle::MAX_CONNECTS);
        while let Some(outcome) = connections.next().await {
            let outcome = match outcome {
                Ok(outcome) => outcome,
                Err(error) => return batch.finish(Err(error)),
            };
            if let Some(intent) = outcome {
                if let Err(error) = batch.record(intent) {
                    return batch.finish(Err(error));
                }
            }
        }
        drop(connections);
        // Snapshot admission bounds this collection to 4096 candidates. No
        // writer for these outcomes enters Tokio's queue while later candidates
        // still need their backoff reads or connection slots. Uncontended success
        // cleanup already returned its old reservation through try_write.
        if let Err(error) = whole.check() {
            return batch.finish(Err(error));
        }
        if let Err(error) = self.flush_manual_successes(whole).await {
            return batch.finish(Err(error));
        }
        self.flush_write_batch(batch, Some(whole)).await
    }

    async fn flush_manual_successes(&self, whole: &scan::Deadline) -> Result<()> {
        whole.check()?;
        let rows = self.inner.lifecycle.desired().await;
        let mut batch =
            bookkeeping::WriteBatch::new(&self.inner.global_metadata_bytes, rows.len())?;
        let mut completed =
            metadata_vec::KnownVec::new(Arc::clone(&self.inner.global_metadata_bytes), rows.len());
        completed.reserve(rows.len())?;
        for (udid, ticket, snapshot) in rows {
            if !matches!(&snapshot.desired, intent::Desired::Manual { authority, .. } if authority.current())
            {
                continue;
            }
            if let Some((desired, success)) = ticket.pending_success() {
                completed.push((ticket.clone(), desired, success.version.clone()))?;
                batch.record(ScanIntent::Success(SuccessIntent {
                    udid,
                    ticket,
                    success,
                    _input: None,
                }))?;
            }
        }
        // A separate bounded batch keeps both collections <=4096 and queues
        // no writer until every candidate in the Source window has finished.
        self.flush_write_batch(batch, Some(whole)).await?;
        for (ticket, desired, failure) in completed {
            ticket.clear_success(&desired, &failure);
        }
        Ok(())
    }

    async fn reconcile_failures(
        &self,
        candidates: &[candidate_index::Candidate],
        paired_complete: bool,
        usb_complete: bool,
        manual: &HashMap<String, (Ticket, intent::RouteIntent)>,
        whole: &scan::Deadline,
        epoch: &scan::Epoch,
    ) -> Result<()> {
        let known = candidate_index::Index::new(candidates, &self.inner.global_metadata_bytes)?;
        let mut failed = self.inner.failed_attempts.write().await;
        epoch.retain(whole, &mut failed, |udid, failed| {
            self.inner.access.permits_device(udid)
                && match known.get(udid) {
                    Some((route, native_id)) => {
                        failed.identity.route == *route && failed.identity.native_id == native_id
                    }
                    None => {
                        (!paired_complete && matches!(failed.identity.route, Route::Mobdev2(_)))
                            || (!usb_complete
                                && matches!(failed.identity.route, Route::Usbmux | Route::Usb(_)))
                            || manual.get(udid).is_some_and(|(_, route)| {
                                matches!(route, intent::RouteIntent::Wifi(_))
                            })
                    }
                }
        })?;
        drop(failed);
        let removed = epoch.commit(whole, || {
            self.inner.retry.reconcile(whole, |udid, identity, _| {
                !self.inner.access.permits_device(udid)
                    || match known.get(udid) {
                        Some((route, native_id)) => {
                            identity.route != *route || identity.native_id != native_id
                        }
                        None => {
                            !((!paired_complete && matches!(identity.route, Route::Mobdev2(_)))
                                || (!usb_complete
                                    && matches!(identity.route, Route::Usbmux | Route::Usb(_)))
                                || manual.get(udid).is_some_and(|(_, route)| {
                                    matches!(route, intent::RouteIntent::Wifi(_))
                                }))
                        }
                    }
            })
        })?;
        reconnect::Ledger::cancelled(removed, reconnect::Reason::Detached);
        Ok(())
    }

    async fn reserve_failure_slot(&self, udid: &str) -> Result<Arc<OwnedSemaphorePermit>> {
        if let Some(slot) = self.inner.retry.slot(udid) {
            return Ok(slot);
        }
        if let Some(slot) = self
            .inner
            .failed_attempts
            .read()
            .await
            .get(udid)
            .and_then(|failed| failed.slot.clone())
        {
            return Ok(slot);
        }
        self.inner
            .failure_slots
            .clone()
            .try_acquire_owned()
            .map(Arc::new)
            .map_err(|_| crate::output::resource_decision("tunnel failure record capacity"))
    }

    #[cfg(test)]
    async fn record_failed_attempt(
        &self,
        udid: &str,
        identity: AttemptIdentity,
        slot: Arc<OwnedSemaphorePermit>,
        ticket: &Ticket,
    ) {
        let mut failed = self.inner.failed_attempts.write().await;
        // Clear/stop can run while this write lock is queued. An old outcome
        // must not resurrect metadata after its lifecycle generation ended.
        if !ticket.is_current() {
            return;
        }
        #[cfg(test)]
        self.inner.lifecycle.before_failure_record();
        // A close/clear/stop after the first check must not resurrect a row.
        // Keep the same synchronous epoch fence as successful publication.
        let _ = ticket.commit(|| {
            failed
                .entry(udid.to_string())
                .and_modify(|attempt| {
                    attempt.version = Arc::new(());
                    attempt.last_attempt = Instant::now();
                    if attempt.identity == identity {
                        attempt.fail_count = attempt.fail_count.saturating_add(1);
                    } else {
                        attempt.identity = identity.clone();
                        attempt.fail_count = 1;
                    }
                })
                .or_insert(FailedTunnelAttempt {
                    version: Arc::new(()),
                    last_attempt: Instant::now(),
                    fail_count: 1,
                    identity,
                    slot: Some(slot),
                });
            Ok(())
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
        .map_err(|_| crate::output::input_decision("cannot use upstream URL for WebSocket"))?;
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
        self.device.record_for_mode(if self.device.userspace() {
            TunMode::Userspace
        } else {
            TunMode::Kernel
        })
    }

    fn is_alive(&self) -> bool {
        self.device.is_alive()
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
    timing: (Duration, reconnect::RetryPolicy),
    access: ServerAccessArgs,
    boundary: crate::access::HttpBoundaryArgs,
    upstreams: UpstreamArgs,
) -> Result<()> {
    let (scan_interval, retry) = timing;
    anyhow::ensure!(!scan_interval.is_zero(), "scan interval must be nonzero");
    let boundary = boundary.validate()?;
    let (address, access) = access.load(host).await?;
    let upstreams = upstreams.load().await?;
    let state = TunnelAgentState::with_source_and_retry(
        tun_mode,
        access,
        upstreams,
        Arc::new(NativeSource),
        TUNNEL_CONNECT_TIMEOUT,
        retry,
    );

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

    server::serve(
        listener,
        app,
        state,
        scan_interval,
        server::CLEANUP_TIMEOUT,
        tokio::signal::ctrl_c(),
    )
    .await
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
        .route("/refresh", axum::routing::post(agent_refresh))
        .route("/epoch", axum::routing::post(agent_epoch))
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
        crate::output::operation_error_class(error),
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
    admission::selectors(&query.udid, None).map_err(|error| {
        manager_operation_error(
            StatusCode::BAD_REQUEST,
            "tunnel selector metadata limit exceeded",
            error.as_ref(),
        )
    })?;
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
    admission::selectors(&udid, None).map_err(|error| {
        manager_operation_error(
            StatusCode::BAD_REQUEST,
            "tunnel selector metadata limit exceeded",
            error.as_ref(),
        )
    })?;
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
    admission::selectors(&udid, None).map_err(|error| {
        manager_operation_error(
            StatusCode::BAD_REQUEST,
            "tunnel selector metadata limit exceeded",
            error.as_ref(),
        )
    })?;
    principal.require(Permission::ManageTunnels, Some(&udid), None)?;
    let removed = principal
        .run(state.stop_tunnel(&udid))
        .await?
        .map_err(|error| {
            manager_operation_error(
                StatusCode::BAD_GATEWAY,
                "failed to stop tunnel",
                error.as_ref(),
            )
        })?;
    if removed {
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

    // Bound selectors before the authority retains the exact target string.
    admission::selectors(&query.udid, query.ip.as_deref()).map_err(|error| {
        manager_operation_error(
            StatusCode::BAD_GATEWAY,
            "failed to create tunnel",
            error.as_ref(),
        )
    })?;
    let authority = principal.device_authority(Permission::ManageTunnels, &query.udid)?;
    let manual = intent::ManualRequest {
        authority,
        host_filter: query.ip.clone(),
    };
    principal
        .run(state.ensure_tunnel_with_intent(
            &query.udid,
            query.ip.as_deref(),
            query.connection_type,
            Some(manual),
        ))
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

async fn agent_refresh(
    State(state): State<TunnelAgentState>,
    Extension(principal): Extension<Principal>,
) -> Result<Json<serde_json::Value>, Response> {
    principal.require(Permission::ManageServer, None, None)?;
    state.refresh_once().await.map_err(|error| {
        manager_operation_error(
            StatusCode::BAD_GATEWAY,
            "tunnel agent refresh failed",
            error.as_ref(),
        )
    })?;
    Ok(Json(json!({"operation": "refresh", "data": true})))
}

async fn agent_epoch(
    State(state): State<TunnelAgentState>,
    Extension(principal): Extension<Principal>,
) -> Result<Json<serde_json::Value>, Response> {
    principal.require(Permission::ManageServer, None, None)?;
    state.clear().await;
    Ok(Json(json!({"operation": "epoch", "data": true})))
}

#[cfg(test)]
mod tests {
    use crate::access::test_support::JsonRequest;
    use clap::Parser;
    use futures_util::{SinkExt, StreamExt};

    use super::*;

    mod access_tests;
    mod browser_tests;
    mod lifecycle_tests;
    mod method_tests;
    mod native_retirement_tests;
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
            version: Arc::new(()),
            last_attempt: now - Duration::from_secs(10),
            fail_count: 2,
            identity: AttemptIdentity {
                route: Route::Usbmux,
                native_id: None,
            },
            slot: None,
        };
        assert!(failed.should_skip(now));

        let expired = FailedTunnelAttempt {
            version: Arc::new(()),
            last_attempt: now - Duration::from_secs(61),
            fail_count: 2,
            identity: AttemptIdentity {
                route: Route::Usbmux,
                native_id: None,
            },
            slot: None,
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

    #[tokio::test]
    async fn agent_refresh_and_epoch_endpoints_execute() {
        let state = TunnelAgentState::new(TunMode::Userspace);
        let (port, task) = spawn_test_router(agent_router(state)).await;
        let client = reqwest::Client::new();

        let refresh_resp = client
            .post(format!("http://127.0.0.1:{port}/refresh"))
            .send()
            .await
            .unwrap();
        assert_eq!(refresh_resp.status(), StatusCode::OK);
        let refresh_bytes = refresh_resp.bytes().await.unwrap();
        let refresh_json: serde_json::Value = serde_json::from_slice(&refresh_bytes).unwrap();
        assert_eq!(refresh_json["operation"], "refresh");
        assert_eq!(refresh_json["data"], true);

        let epoch_resp = client
            .post(format!("http://127.0.0.1:{port}/epoch"))
            .send()
            .await
            .unwrap();
        assert_eq!(epoch_resp.status(), StatusCode::OK);
        let epoch_bytes = epoch_resp.bytes().await.unwrap();
        let epoch_json: serde_json::Value = serde_json::from_slice(&epoch_bytes).unwrap();
        assert_eq!(epoch_json["operation"], "epoch");
        assert_eq!(epoch_json["data"], true);

        task.abort();
    }

    #[tokio::test]
    async fn agent_endpoints_reject_oversized_selectors() {
        let state = TunnelAgentState::new(TunMode::Userspace);
        let (port, task) = spawn_test_router(agent_router(state)).await;
        let client = reqwest::Client::new();
        let oversized = "a".repeat(4097);

        let get_resp = client
            .get(format!("http://127.0.0.1:{port}/tunnel/{oversized}"))
            .send()
            .await
            .unwrap();
        assert_eq!(get_resp.status(), StatusCode::BAD_REQUEST);

        let del_resp = client
            .delete(format!("http://127.0.0.1:{port}/tunnel/{oversized}"))
            .send()
            .await
            .unwrap();
        assert_eq!(del_resp.status(), StatusCode::BAD_REQUEST);

        task.abort();
    }
}
