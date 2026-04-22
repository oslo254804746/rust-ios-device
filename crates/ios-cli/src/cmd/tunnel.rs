use std::collections::HashMap;
use std::io::Write;
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, Result};
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::routing::get;
use axum::{Json, Router};
use ios_core::{
    connect, connect_direct_usb_tunnel, connect_remote_pairing_tunnel, connect_tcp_lockdown_tunnel,
    discover_paired_mobdev2_devices, ConnectOptions, ConnectedDevice,
};
use ios_mux::MuxClient;
use ios_tunnel::TunMode;
use serde::Serialize;
use serde_json::json;
use tokio::net::TcpListener;
use tokio::sync::{Mutex, Notify, RwLock};

const DEFAULT_AGENT_HOST: &str = "127.0.0.1";
const DEFAULT_AGENT_PORT: u16 = 49151;
const DEFAULT_SCAN_INTERVAL_MS: u64 = 2_000;
const TUNNEL_CONNECT_TIMEOUT: Duration = Duration::from_secs(15);

#[derive(clap::Args)]
pub struct TunnelCmd {
    #[command(subcommand)]
    sub: TunnelSub,
}

#[derive(clap::Subcommand)]
enum TunnelSub {
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
    },
    /// Stop a tunnel
    Stop,
    /// List active tunnels
    List,
}

impl TunnelCmd {
    pub async fn run(self, udid: Option<String>) -> Result<()> {
        match self.sub {
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
                )
                .await?;
            }
            TunnelSub::Stop => {
                println!(
                    "tunnel stop: not yet implemented (use the HTTP manager DELETE /tunnel/:udid or /shutdown)"
                );
            }
            TunnelSub::List => {
                println!(
                    "tunnel list: not yet implemented (use the HTTP manager GET / or /tunnels)"
                );
            }
        }
        Ok(())
    }
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
        eprintln!("Starting {mode} tunnel for {udid}...");
    }

    let device = connect(udid, opts).await?;
    let server_address = device
        .server_address()
        .ok_or_else(|| anyhow::anyhow!("tunnel started without a server address"))?;
    let rsd_port = device
        .rsd_port()
        .ok_or_else(|| anyhow::anyhow!("tunnel started without an RSD port"))?;

    if script_mode {
        println!("{server_address} {rsd_port}");
    } else {
        println!("Identifier: {udid}");
        println!("Mode: {mode}");
        println!("Protocol: tcp");
        println!("RSD Address: {server_address}");
        println!("RSD Port: {rsd_port}");
        if let Some(userspace_port) = device.userspace_port() {
            println!("Userspace Proxy: 127.0.0.1:{userspace_port}");
        }
        println!("Use the following connection option:");
        println!("--rsd {server_address} {rsd_port}");
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
    tun_mode: TunMode,
    tunnels: RwLock<HashMap<String, ManagedTunnel>>,
    ready: RwLock<bool>,
    lifecycle_lock: Mutex<()>,
    shutdown: Notify,
}

struct ManagedTunnel {
    device: ConnectedDevice,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
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

use serde::Deserialize;

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
    fn new(tun_mode: TunMode) -> Self {
        Self {
            inner: Arc::new(TunnelAgentInner {
                tun_mode,
                tunnels: RwLock::new(HashMap::new()),
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
    }

    async fn list_records(&self) -> Vec<TunnelRecord> {
        let _guard = self.inner.lifecycle_lock.lock().await;
        let mut tunnels = self.inner.tunnels.write().await;
        tunnels.retain(|_, tunnel| tunnel.is_alive());
        tunnels.values().filter_map(|t| t.record().ok()).collect()
    }

    async fn list_tunneld_view(&self) -> HashMap<String, Vec<TunnelRecord>> {
        let mut grouped = HashMap::<String, Vec<TunnelRecord>>::new();
        for record in self.list_records().await {
            grouped.entry(record.udid.clone()).or_default().push(record);
        }
        grouped
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

        let mut mux = MuxClient::connect().await?;
        let devices = mux.list_devices().await?;
        drop(mux);

        let attached_udids = devices
            .iter()
            .map(|device| device.serial_number.clone())
            .collect::<std::collections::HashSet<_>>();

        {
            let mut tunnels = self.inner.tunnels.write().await;
            tunnels.retain(|udid, tunnel| {
                tunnel.is_alive()
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
            if existing.contains(&device.serial_number) {
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
                }
                Ok(Err(err)) => {
                    tracing::warn!(
                        "tunnel agent: failed to establish tunnel for {}: {err}",
                        device.serial_number
                    );
                }
                Err(_) => {
                    tracing::warn!(
                        "tunnel agent: timed out establishing tunnel for {}",
                        device.serial_number
                    );
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
            if existing.contains(&target.udid) {
                continue;
            }

            match self
                .connect_mobdev2_tunnel_locked(&target.udid, &target.host)
                .await
            {
                Ok(_) => {}
                Err(err) => {
                    tracing::warn!(
                        "tunnel agent: failed to establish mobdev2 tunnel for {} via {}: {err}",
                        target.udid,
                        target.host
                    );
                }
            }
        }

        self.mark_ready().await;
        Ok(())
    }
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
            .tunnel
            .as_ref()
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

pub async fn run_tunnel_agent(
    host: &str,
    port: u16,
    tun_mode: TunMode,
    scan_interval: Duration,
) -> Result<()> {
    let state = TunnelAgentState::new(tun_mode);
    let app = Router::new()
        .route("/", get(agent_list_tunneld))
        .route("/health", get(agent_health))
        .route("/ready", get(agent_ready))
        .route("/shutdown", get(agent_shutdown))
        .route("/start-tunnel", get(agent_start_tunnel))
        .route("/tunnels", get(agent_list_records))
        .route(
            "/tunnel/:udid",
            get(agent_get_tunnel).delete(agent_delete_tunnel),
        )
        .with_state(state.clone());

    let listener = TcpListener::bind((host, port))
        .await
        .with_context(|| format!("failed to bind tunnel agent on {host}:{port}"))?;

    tracing::info!(
        "tunnel agent listening on http://{}:{} ({})",
        host,
        port,
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
                        tracing::warn!("tunnel agent refresh failed: {err}");
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

async fn agent_list_tunneld(
    State(state): State<TunnelAgentState>,
) -> Json<HashMap<String, Vec<TunnelRecord>>> {
    Json(state.list_tunneld_view().await)
}

async fn agent_list_records(State(state): State<TunnelAgentState>) -> Json<Vec<TunnelRecord>> {
    Json(state.list_records().await)
}

async fn agent_get_tunnel(
    State(state): State<TunnelAgentState>,
    Path(udid): Path<String>,
) -> Result<Json<TunnelRecord>, (StatusCode, Json<serde_json::Value>)> {
    state.get_record(&udid).await.map(Json).ok_or_else(|| {
        (
            StatusCode::NOT_FOUND,
            Json(json!({ "message": "tunnel not found", "udid": udid })),
        )
    })
}

async fn agent_delete_tunnel(
    State(state): State<TunnelAgentState>,
    Path(udid): Path<String>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<serde_json::Value>)> {
    if state.remove_tunnel(&udid).await {
        Ok(Json(
            json!({ "operation": "cancel", "udid": udid, "data": true }),
        ))
    } else {
        Err((
            StatusCode::NOT_FOUND,
            Json(json!({ "message": "tunnel not found", "udid": udid })),
        ))
    }
}

async fn agent_start_tunnel(
    State(state): State<TunnelAgentState>,
    Query(query): Query<StartTunnelQuery>,
) -> Result<Json<StartTunnelResponse>, (StatusCode, Json<serde_json::Value>)> {
    if let Some(message) = query.unsupported_transport_message() {
        return Err((
            StatusCode::NOT_IMPLEMENTED,
            Json(json!({
                "message": message,
                "udid": query.udid,
                "connection_type": query.requested_connection_type(),
                "ip": query.ip,
            })),
        ));
    }

    state
        .ensure_tunnel(&query.udid, query.ip.as_deref(), query.connection_type)
        .await
        .map(StartTunnelResponse::from_record)
        .map(Json)
        .map_err(|err| {
            (
                StatusCode::BAD_GATEWAY,
                Json(json!({
                    "message": "failed to create tunnel",
                    "udid": query.udid,
                    "error": err.to_string(),
                })),
            )
        })
}

async fn agent_health() -> StatusCode {
    StatusCode::OK
}

async fn agent_ready(State(state): State<TunnelAgentState>) -> StatusCode {
    if state.is_ready().await {
        StatusCode::OK
    } else {
        StatusCode::SERVICE_UNAVAILABLE
    }
}

async fn agent_shutdown(State(state): State<TunnelAgentState>) -> Json<serde_json::Value> {
    state.notify_shutdown();
    Json(json!({
        "operation": "shutdown",
        "data": true,
        "message": "Server shutting down..."
    }))
}

#[cfg(test)]
mod tests {
    use clap::Parser;

    use super::*;

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
}
