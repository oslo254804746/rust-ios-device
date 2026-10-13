use std::{future::Future, net::SocketAddrV6, time::Duration};

use anyhow::{bail, Context, Result};
use serde::Serialize;

const MAX_MANAGER_RESPONSE: usize = 1024 * 1024;

#[derive(Clone)]
struct ProbeCancel(tokio::sync::watch::Sender<bool>);

impl ProbeCancel {
    fn new() -> Self {
        Self(tokio::sync::watch::channel(false).0)
    }

    async fn cancelled(&self) {
        let mut receiver = self.0.subscribe();
        while !*receiver.borrow_and_update() {
            if receiver.changed().await.is_err() {
                break;
            }
        }
    }
}

mod device;
mod host_directories;

/// Safe typed context for this command's probe interpretation. Keeping it in
/// anyhow context preserves the original native error and its downcasts.
#[derive(Debug)]
pub(crate) struct DoctorProbeClass(pub(crate) ios_core::ErrorClass);

impl std::fmt::Display for DoctorProbeClass {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "selected device probe failed ({})", self.0)
    }
}

/// Read-only host checks; an explicit --udid also probes existing device trust.
#[derive(clap::Args)]
pub struct DoctorCmd {
    /// Deadline for each probe, in seconds
    #[arg(long, default_value_t = 3, value_parser = clap::value_parser!(u64).range(1..=60))]
    timeout: u64,
    /// Also browse local Bonjour advertisements during the probe window
    #[arg(long)]
    mdns: bool,
    /// Port of the optional tunnel manager on 127.0.0.1
    #[arg(long, default_value_t = 49151, value_parser = clap::value_parser!(u16).range(1..))]
    tunnel_port: u16,
    /// Explicit already established RSD IPv6 endpoint, including its port
    #[arg(long, value_name = "[IPV6%SCOPE]:PORT")]
    rsd_address: Option<SocketAddrV6>,
    /// Existing userspace proxy port on 127.0.0.1; no tunnel is started
    #[arg(long, requires = "rsd_address", value_parser = clap::value_parser!(u16).range(1..))]
    rsd_proxy_port: Option<u16>,
    /// Observe one service advertisement without connecting to that service
    #[arg(long, requires = "rsd_address")]
    rsd_service: Option<String>,
    #[command(flatten)]
    access: crate::access::ClientAccessArgs,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
enum Status {
    Ok,
    Warning,
    Failed,
    Unknown,
}

#[derive(Serialize)]
struct Check {
    id: &'static str,
    status: Status,
    detail: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    hint: Option<&'static str>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    remoted_observations: Vec<RemotedObservation>,
    #[serde(skip_serializing_if = "Option::is_none")]
    state: Option<device::State>,
    #[serde(skip_serializing_if = "Option::is_none")]
    error_class: Option<ios_core::ErrorClass>,
    #[serde(skip)]
    error: Option<anyhow::Error>,
}

impl std::fmt::Debug for Check {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("HostCheck")
            .field("id", &self.id)
            .field("status", &self.status)
            .field("state", &self.state)
            .field("error_class", &self.error_class)
            .field("error_retained", &self.error.is_some())
            .finish_non_exhaustive()
    }
}

#[derive(Debug, Serialize)]
struct RemotedObservation {
    running: bool,
    pid: Option<u32>,
    observed_at: String,
}

impl Check {
    fn new(id: &'static str, status: Status, detail: impl Into<String>) -> Self {
        Self {
            id,
            status,
            detail: detail.into(),
            hint: None,
            remoted_observations: Vec::new(),
            state: None,
            error_class: None,
            error: None,
        }
    }

    fn hint(mut self, hint: &'static str) -> Self {
        self.hint = Some(hint);
        self
    }

    fn state(mut self, state: device::State) -> Self {
        self.state = Some(state);
        self
    }

    fn io_failure(id: &'static str, error: std::io::Error, detail: &'static str) -> Self {
        Self::failure(id, error.into(), detail)
    }

    fn failure(id: &'static str, error: anyhow::Error, detail: &'static str) -> Self {
        let class = ios_core::classify_error(error.as_ref());
        let state = match class {
            ios_core::ErrorClass::PermissionDenied => device::State::PermissionDenied,
            ios_core::ErrorClass::NotFound => device::State::MetadataMissing,
            ios_core::ErrorClass::ServiceMissing => device::State::ServiceUnavailable,
            ios_core::ErrorClass::Unsupported => device::State::Unsupported,
            ios_core::ErrorClass::Protocol | ios_core::ErrorClass::InvalidInput => {
                device::State::Failed
            }
            ios_core::ErrorClass::Timeout
            | ios_core::ErrorClass::Disconnected
            | ios_core::ErrorClass::DeviceNotFound
            | ios_core::ErrorClass::Io => device::State::Unreachable,
            _ => device::State::Failed,
        };
        let mut check = Self::new(id, Status::Warning, detail).state(state);
        check.error_class = Some(class);
        check.error = Some(error);
        check
    }
}

#[derive(Debug, Serialize)]
struct Report {
    schema_version: u8,
    scope: &'static str,
    os: &'static str,
    arch: &'static str,
    version: &'static str,
    checks: Vec<Check>,
}

impl Report {
    fn failed(&self) -> bool {
        self.checks
            .iter()
            .any(|check| check.status == Status::Failed)
    }
}

impl DoctorCmd {
    pub async fn run(self, udid: Option<String>, json: bool) -> Result<()> {
        // Validate every selector before host probes, credential reads, device
        // enumeration or route IO, including programmatically built commands.
        let rsd = self.rsd_request(udid.as_deref())?;
        if let Some(udid) = udid {
            device::validate_target(&udid)?;
            return self.run_target(&udid, json, rsd.as_ref()).await;
        }
        let cancel = ProbeCancel::new();
        let report = cancellable(
            async { Ok(self.inspect(&cancel).await) },
            tokio::signal::ctrl_c(),
            &cancel,
        )
        .await?;
        if json {
            crate::output::outln!("{}", serde_json::to_string_pretty(&report)?);
        } else {
            crate::output::outln!(
                "ios {} host diagnostics ({} / {})",
                report.version,
                report.os,
                report.arch
            );
            for check in &report.checks {
                let status = match check.status {
                    Status::Ok => "OK",
                    Status::Warning => "WARN",
                    Status::Failed => "FAIL",
                    Status::Unknown => "?",
                };
                crate::output::outln!("[{status}] {}: {}", check.id, check.detail);
                if let Some(hint) = check.hint {
                    crate::output::outln!("       {hint}");
                }
            }
        }
        if report.failed() {
            bail!("host diagnostics found a failed check");
        }
        Ok(())
    }

    fn rsd_request(&self, udid: Option<&str>) -> Result<Option<device::rsd::Request>> {
        device::rsd::Request::from_selectors(
            udid,
            self.rsd_address,
            self.rsd_proxy_port,
            self.rsd_service.as_deref(),
        )
    }

    async fn run_target(
        &self,
        udid: &str,
        json: bool,
        rsd: Option<&device::rsd::Request>,
    ) -> Result<()> {
        let cancel = ProbeCancel::new();
        let work = async {
            #[cfg(target_os = "macos")]
            let mut host = self.inspect(&cancel).await;
            #[cfg(not(target_os = "macos"))]
            let host = self.inspect(&cancel).await;
            let timeout = Duration::from_secs(self.timeout);
            let device_work = async {
                match rsd {
                    Some(request) => {
                        device::inspect_with_rsd(&device::NativeSource, udid, timeout, request)
                            .await
                    }
                    None => device::inspect(&device::NativeSource, udid, timeout).await,
                }
            };
            let device = until_cancelled(async { Ok(device_work.await) }, &cancel).await?;
            #[cfg(target_os = "macos")]
            if rsd.is_some() {
                let after = probe_remoted_coexistence(timeout, &cancel).await;
                let selected_rsd_ok = device
                    .checks
                    .iter()
                    .any(|check| check.id == "rsd_services" && check.status == Status::Ok);
                if let Some(check) = host
                    .checks
                    .iter_mut()
                    .find(|check| check.id == "remoted_coexistence")
                {
                    reconcile_remoted(check, after, selected_rsd_ok);
                }
            }
            Ok(TargetReport::new(host, device))
        };
        let mut report = cancellable(work, tokio::signal::ctrl_c(), &cancel).await?;
        if json {
            crate::output::outln!("{}", serde_json::to_string_pretty(&report)?);
        } else {
            crate::output::outln!(
                "ios {} host and device diagnostics ({} / {})",
                report.version,
                report.os,
                report.arch
            );
            for check in &report.checks {
                crate::output::outln!(
                    "[{}] {}: {}",
                    status_label(check.status),
                    check.id,
                    check.detail
                );
                if let Some(hint) = check.hint {
                    crate::output::outln!("       {hint}");
                }
            }
            for check in &report.device.checks {
                crate::output::outln!(
                    "[{}] {}: {}",
                    status_label(check.status),
                    check.id,
                    check.detail
                );
                if let Some(hint) = check.hint {
                    crate::output::outln!("       {hint}");
                }
            }
        }
        if report.failed() {
            if let Some(error) = report.device.take_first_failed_error() {
                return Err(error.context("selected device diagnostics found a failed check"));
            }
            bail!("host or selected device diagnostics found a failed check");
        }
        Ok(())
    }

    async fn inspect(&self, cancel: &ProbeCancel) -> Report {
        let deadline = Duration::from_secs(self.timeout);
        let (mux, ipv6, manager, kernel, platform_usb, platform_route, remoted, ddi_dir, pair_dir) = tokio::join!(
            bounded_cancel("usbmuxd", deadline, Status::Failed, probe_usbmux(), cancel),
            bounded_cancel(
                "ipv6_loopback",
                deadline,
                Status::Warning,
                probe_ipv6(),
                cancel
            ),
            bounded_cancel(
                "tunnel_manager",
                deadline,
                if self.access.has_token() {
                    Status::Failed
                } else {
                    Status::Warning
                },
                probe_manager(self.tunnel_port, &self.access),
                cancel
            ),
            bounded_cancel(
                "kernel_tun",
                deadline,
                Status::Warning,
                probe_kernel_tun(),
                cancel
            ),
            probe_platform_usb(deadline, cancel),
            bounded_cancel(
                "platform_route",
                deadline,
                Status::Warning,
                probe_platform_route(self.rsd_address),
                cancel
            ),
            probe_remoted_coexistence(deadline, cancel),
            bounded_cancel(
                "ddi_directory",
                deadline,
                Status::Warning,
                probe_ddi_directory(),
                cancel
            ),
            bounded_cancel(
                "pairing_directory",
                deadline,
                Status::Warning,
                host_directories::probe_pairing_directory(),
                cancel
            ),
        );
        let mut checks = vec![
            mux,
            ipv6,
            manager,
            kernel,
            Check::new(
                "userspace_tunnel",
                Status::Ok,
                "Userspace tunnel support is compiled in; establishing a device tunnel was not tested.",
            ),
            platform_usb,
            platform_route,
            remoted,
            ddi_dir,
            pair_dir,
        ];
        checks.push(if self.mdns {
            // Browsing itself uses the requested window; allow a bounded margin for setup/cleanup.
            bounded_cancel(
                "bonjour",
                deadline + Duration::from_secs(1),
                Status::Warning,
                probe_mdns(deadline),
                cancel,
            )
            .await
        } else {
            Check::new(
                "bonjour",
                Status::Unknown,
                "Bonjour browsing was not requested.",
            )
            .hint("Run ios doctor --mdns to check for local advertisements.")
        });
        checks.push(Check::new("device_readiness", Status::Unknown,
            "Device trust, Developer Mode, mounted DDI and service connectivity were not tested.")
            .hint("Use ios list, ios ddi devmode-status and ios ddi status for device checks."));
        Report {
            schema_version: 1,
            scope: "host",
            os: std::env::consts::OS,
            arch: std::env::consts::ARCH,
            version: env!("CARGO_PKG_VERSION"),
            checks,
        }
    }
}

/// Schema 2 is selected only by an explicit UDID. Host-only Report stays schema 1.
#[derive(Debug, Serialize)]
struct TargetReport {
    schema_version: u8,
    scope: &'static str,
    os: &'static str,
    arch: &'static str,
    version: &'static str,
    checks: Vec<Check>,
    device: device::Assessment,
}

impl TargetReport {
    fn new(host: Report, device: device::Assessment) -> Self {
        Self {
            schema_version: 2,
            scope: "host_and_device",
            os: host.os,
            arch: host.arch,
            version: host.version,
            checks: host.checks,
            device,
        }
    }

    fn failed(&self) -> bool {
        self.checks
            .iter()
            .any(|check| check.status == Status::Failed)
            || self.device.failed()
    }
}

fn status_label(status: Status) -> &'static str {
    match status {
        Status::Ok => "OK",
        Status::Warning => "WARN",
        Status::Failed => "FAIL",
        Status::Unknown => "?",
    }
}

async fn cancellable<T>(
    work: impl Future<Output = Result<T>>,
    signal: impl Future<Output = std::io::Result<()>>,
    cancel: &ProbeCancel,
) -> Result<T> {
    tokio::pin!(signal);
    tokio::pin!(work);
    tokio::select! {
        biased;
        signal = &mut signal => {
            cancel.0.send_replace(true);
            // Host process probes observe this request, kill their child and
            // await the original handle. Keep polling the owner until cleanup
            // completes instead of dropping it at the Ctrl-C boundary.
            let _ = work.await;
            signal.context("waiting for doctor Ctrl-C")?;
            Err(ios_core::OperationCancelled::new().into())
        }
        result = &mut work => result,
    }
}

async fn bounded_cancel(
    id: &'static str,
    deadline: Duration,
    timeout_status: Status,
    probe: impl Future<Output = Check>,
    cancel: &ProbeCancel,
) -> Check {
    tokio::select! {
        biased;
        _ = cancel.cancelled() => Check::new(id, Status::Unknown, "The probe was cancelled.").state(device::State::NotProbed),
        check = bounded(id, deadline, timeout_status, probe) => check,
    }
}

async fn until_cancelled<T>(
    work: impl Future<Output = Result<T>>,
    cancel: &ProbeCancel,
) -> Result<T> {
    tokio::select! {
        biased;
        _ = cancel.cancelled() => Err(ios_core::OperationCancelled::new().into()),
        result = work => result,
    }
}

async fn bounded(
    id: &'static str,
    deadline: Duration,
    timeout_status: Status,
    probe: impl Future<Output = Check>,
) -> Check {
    match tokio::time::timeout(deadline, probe).await {
        Ok(check) => check,
        Err(error) => {
            let mut check = Check::io_failure(
                id,
                std::io::Error::new(std::io::ErrorKind::TimedOut, error),
                "The probe exceeded its deadline.",
            );
            check.status = timeout_status;
            check
        }
    }
}

async fn probe_usbmux() -> Check {
    let mut client = match ios_core::MuxClient::connect().await {
        Ok(client) => client,
        Err(_) => {
            return Check::new(
                "usbmuxd",
                Status::Failed,
                "Cannot connect to the configured usbmuxd endpoint.",
            )
            .hint("Check usbmuxd / Apple Mobile Device Service and USBMUXD_SOCKET_ADDRESS.")
        }
    };
    match client.list_devices().await {
        Ok(devices) => {
            let usb = devices
                .iter()
                .filter(|device| device.connection_type == "USB")
                .count();
            let network = devices
                .iter()
                .filter(|device| device.connection_type == "Network")
                .count();
            let check = Check::new("usbmuxd", Status::Ok, format!(
                "Enumeration succeeded: {usb} USB, {network} network, {} other entries. Service connections were not tested.",
                devices.len() - usb - network,
            ));
            if devices.is_empty() {
                check.hint("No devices were enumerated. Connect and unlock a device to run device commands.")
            } else {
                check
            }
        }
        Err(_) => Check::new(
            "usbmuxd",
            Status::Failed,
            "The endpoint accepted a connection but device enumeration failed.",
        )
        .hint("Check that the endpoint provides the usbmuxd plist protocol."),
    }
}

async fn probe_ipv6() -> Check {
    match tokio::net::TcpListener::bind((std::net::Ipv6Addr::LOCALHOST, 0)).await {
        Ok(_) => Check::new(
            "ipv6_loopback",
            Status::Ok,
            "An IPv6 loopback TCP socket can be bound. Device routes were not tested.",
        ),
        Err(_) => Check::new(
            "ipv6_loopback",
            Status::Warning,
            "An IPv6 loopback TCP socket could not be bound.",
        )
        .hint("Check host IPv6 support before using native IPv6 tunnel endpoints."),
    }
}

async fn probe_kernel_tun() -> Check {
    #[cfg(target_os = "linux")]
    {
        use std::os::unix::fs::FileTypeExt;
        match tokio::fs::metadata("/dev/net/tun").await {
            Ok(metadata) if metadata.file_type().is_char_device() => Check::new(
                "kernel_tun", Status::Unknown,
                "The TUN character device exists; permission to create an interface was not tested.")
                .hint("Kernel tunnels require CAP_NET_ADMIN. Userspace tunnels do not create an interface."),
            _ => Check::new("kernel_tun", Status::Warning, "No TUN character device was found at /dev/net/tun.")
                .hint("Use a userspace tunnel, or enable TUN in the host/container for kernel mode."),
        }
    }
    #[cfg(not(target_os = "linux"))]
    {
        Check::new(
            "kernel_tun",
            Status::Unknown,
            "Native interface creation and its permissions were not tested on this platform.",
        )
    }
}

async fn probe_manager(port: u16, access: &crate::access::ClientAccessArgs) -> Check {
    let url =
        reqwest::Url::parse(&format!("http://127.0.0.1:{port}/")).expect("literal loopback URL");
    let authorization = match access.header(&url).await {
        Ok(header) => header,
        Err(error) => return Check::new(
            "tunnel_manager",
            Status::Failed,
            format!(
                "The private manager credential could not be read: {}",
                crate::output::error_message(&error)
            ),
        )
        .hint(
            "Provide a private --token-file with list permission for the intended device scope.",
        ),
    };
    // Bypass environment proxies and redirects: this probe must stay on loopback.
    let client = match reqwest::Client::builder()
        .no_proxy()
        .redirect(reqwest::redirect::Policy::none())
        .build()
    {
        Ok(client) => client,
        Err(_) => {
            return Check::new(
                "tunnel_manager",
                Status::Warning,
                "Cannot initialize the local HTTP probe.",
            )
        }
    };
    let mut request = client.get(url);
    if let Some(header) = authorization {
        request = request.header(reqwest::header::AUTHORIZATION, header);
    }
    let mut response = match request.send().await {
        Ok(response) if response.status().is_success() => response,
        Ok(response) if matches!(response.status().as_u16(), 401 | 403) => {
            return Check::new("tunnel_manager", if access.has_token() { Status::Failed } else { Status::Warning },
                "The manager requires a valid credential with list permission.")
                .hint("Provide a private --token-file matching the manager policy and selected device scope.");
        }
        _ => return Check::new(
            "tunnel_manager",
            Status::Warning,
            "No successful response from the optional loopback tunnel manager.",
        )
        .hint(
            "Start ios tunnel serve --auth-config PRIVATE_POLICY_FILE, or select --tunnel-port.",
        ),
    };
    let mut body = Vec::new();
    loop {
        match response.chunk().await {
            Ok(Some(chunk)) if body.len().saturating_add(chunk.len()) <= MAX_MANAGER_RESPONSE => {
                body.extend_from_slice(&chunk)
            }
            Ok(None) => break,
            _ => {
                return Check::new(
                    "tunnel_manager",
                    Status::Warning,
                    "The manager response was unreadable or exceeded 1 MiB.",
                )
            }
        }
    }
    manager_response_check(&body)
}

fn manager_response_check(body: &[u8]) -> Check {
    let count = serde_json::from_slice::<serde_json::Value>(body)
        .ok()
        .and_then(|value| {
            let groups = value.as_object()?;
            groups.values().try_fold(0usize, |count, group| {
                let entries = group.as_array()?;
                if entries.iter().all(|entry| {
                    entry
                        .get("tunnel-address")
                        .and_then(|v| v.as_str())
                        .is_some()
                        && entry
                            .get("tunnel-port")
                            .and_then(|v| v.as_u64())
                            .is_some_and(|port| (1..=65535).contains(&port))
                }) {
                    Some(count + entries.len())
                } else {
                    None
                }
            })
        });
    match count {
        Some(count) => Check::new("tunnel_manager", Status::Ok,
            format!("The local manager reports {count} tunnel(s). Endpoint reachability was not tested.")),
        None => Check::new("tunnel_manager", Status::Warning, "The local endpoint did not return a recognized tunnel list."),
    }
}

async fn probe_mdns(window: Duration) -> Check {
    let (mobdev2, remote) = tokio::join!(
        ios_core::browse_mobdev2(window),
        ios_core::browse_remotepairing(window),
    );
    match (mobdev2, remote) {
        (Ok(mobdev2), Ok(remote)) => {
            let status = if mobdev2.is_empty() && remote.is_empty() {
                Status::Unknown
            } else {
                Status::Ok
            };
            Check::new("bonjour", status, format!(
                "Observed {} mobdev2 and {} RemotePairing advertisements; this does not prove device trust. An empty result does not prove multicast is blocked.",
                mobdev2.len(), remote.len(),
            ))
        }
        _ => Check::new(
            "bonjour",
            Status::Warning,
            "At least one Bonjour browser could not complete.",
        )
        .hint("Check multicast/network-interface availability; discovery needs UDP port 5353."),
    }
}

#[cfg(any(target_os = "windows", target_os = "macos", test))]
async fn bounded_host_command(
    mut command: tokio::process::Command,
    budget: Duration,
    cancel: &ProbeCancel,
) -> Result<(std::process::ExitStatus, Vec<u8>)> {
    use tokio::io::AsyncReadExt;
    let deadline = tokio::time::Instant::now() + budget;
    anyhow::ensure!(!*cancel.0.borrow(), "host probe cancelled before startup");
    command
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .kill_on_drop(true);
    #[cfg(target_os = "windows")]
    command.creation_flags(0x08000000); // CREATE_NO_WINDOW
    let mut child = command.spawn()?;
    let mut stdout = child
        .stdout
        .take()
        .context("host probe stdout unavailable")?;
    let work = async {
        host_command_clock(deadline).await?;
        let mut bytes = Vec::new();
        let mut chunk = [0u8; 8192];
        loop {
            let read = stdout.read(&mut chunk).await?;
            host_command_clock(deadline).await?;
            if read == 0 {
                break;
            }
            anyhow::ensure!(
                bytes.len() + read <= 1024 * 1024,
                "host probe output bound exceeded"
            );
            bytes.extend_from_slice(&chunk[..read]);
        }
        let status = child.wait().await?;
        host_command_clock(deadline).await?;
        Ok::<_, anyhow::Error>((status, bytes))
    };
    // Reserve a margin inside the caller's public probe deadline so an expired
    // child can be killed and the original process handle awaited.
    let outcome = tokio::select! {
        biased;
        _ = cancel.cancelled() => Ok(Err(ios_core::OperationCancelled::new().into())),
        outcome = tokio::time::timeout_at(deadline, work) => outcome,
    };
    match outcome {
        Ok(Ok(result)) => Ok(result),
        outcome => {
            let primary = match outcome {
                Ok(Err(error)) => Err(error),
                Err(error) => Err(error.into()),
                Ok(Ok(_)) => unreachable!(),
            };
            // Never replace the read/limit/timeout cause with a secondary kill
            // error. Both kill and wait are bounded; an unconfirmed reap stays
            // a reported failure instead of a successful diagnostic.
            let kill = match child.try_wait() {
                Ok(Some(_)) => Ok(()),
                Ok(None) => child.start_kill().map_err(anyhow::Error::new),
                Err(error) => Err(error.into()),
            };
            let waited = tokio::time::timeout(Duration::from_secs(2), child.wait())
                .await
                .context("host probe child reap deadline expired")
                .and_then(|result| result.map(|_| ()).map_err(anyhow::Error::new));
            let cleanup = crate::cmd::test_workflow::combine_results(
                kill,
                waited,
                "host probe child wait failed",
            );
            crate::cmd::test_workflow::combine_results(
                primary,
                cleanup,
                "host probe process cleanup failed",
            )?;
            unreachable!()
        }
    }
}

#[cfg(any(target_os = "windows", target_os = "macos", test))]
async fn host_command_clock(deadline: tokio::time::Instant) -> Result<()> {
    if tokio::time::Instant::now() >= deadline {
        tokio::time::timeout_at(deadline, std::future::pending::<()>()).await?;
    }
    Ok(())
}

async fn probe_platform_usb(_budget: Duration, _cancel: &ProbeCancel) -> Check {
    #[cfg(target_os = "linux")]
    {
        bounded_cancel(
            "platform_usb",
            _budget,
            Status::Warning,
            async {
                match tokio::task::spawn_blocking(|| {
                    host_directories::inspect_linux_usb(std::path::Path::new(
                        "/sys/bus/usb/devices",
                    ))
                })
                .await
                {
                    Ok(check) => check,
                    Err(_) => Check::new(
                        "platform_usb",
                        Status::Unknown,
                        "The Linux USB observation worker did not complete.",
                    )
                    .state(device::State::Unreachable),
                }
            },
            _cancel,
        )
        .await
    }
    #[cfg(any(target_os = "windows", target_os = "macos"))]
    {
        let command = if cfg!(target_os = "windows") {
            let mut command = tokio::process::Command::new("powershell.exe");
            command.args(["-NoLogo", "-NoProfile", "-NonInteractive", "-Command",
                "$ErrorActionPreference='Stop'; $d=@(Get-CimInstance Win32_PnPEntity -Filter \"Present = TRUE AND PNPDeviceID LIKE 'USB%'\"); @{total=$d.Count;apple=@($d | Where-Object {$_.PNPDeviceID -match 'VID_05AC'}).Count;failed=@($d | Where-Object {$_.ConfigManagerErrorCode -ne 0}).Count} | ConvertTo-Json -Compress"]);
            command
        } else {
            let mut command = tokio::process::Command::new("/usr/sbin/ioreg");
            command.args(["-p", "IOUSB", "-l"]);
            command
        };
        match bounded_host_command(command, _budget, _cancel).await {
            Ok((status, stdout)) if status.success() => {
                if cfg!(target_os = "windows") {
                    host_directories::windows_usb_output(&stdout)
                } else {
                    host_directories::ioreg_usb_output(&stdout)
                }
            }
            Ok(_) => Check::new(
                "platform_usb",
                Status::Unknown,
                "The platform USB enumeration command exited unsuccessfully; its permission or service state is unknown.",
            ).state(device::State::Failed),
            Err(error) => Check::failure("platform_usb", error,
                "Platform USB enumeration could not complete or exceeded its output limit."),
        }
    }
    #[cfg(not(any(target_os = "linux", target_os = "windows", target_os = "macos")))]
    {
        Check::new(
            "platform_usb",
            Status::Unknown,
            "No USB enumeration adapter is available for this platform.",
        )
        .state(device::State::Unsupported)
    }
}

async fn probe_platform_route(address: Option<SocketAddrV6>) -> Check {
    let Some(address) = address else {
        return Check::new(
            "platform_route",
            Status::Unknown,
            "No existing target RSD endpoint selected for route observation.",
        )
        .state(device::State::NotProbed)
        .hint(
            "Select --udid and --rsd-address for a bounded read-only route and RSD observation.",
        );
    };
    match tokio::net::TcpStream::connect(address).await {
        Ok(stream) => match stream.local_addr() {
            Ok(local) if !local.ip().is_unspecified() => Check::new("platform_route", Status::Ok,
                "The selected existing IPv6 RSD endpoint accepted TCP over a concrete local route; RSD identity and services are checked separately.").state(device::State::Available),
            _ => Check::new("platform_route", Status::Unknown, "The selected connection did not expose a concrete local route.").state(device::State::MetadataMissing),
        },
        Err(error) => Check::io_failure("platform_route", error,
            "The selected existing RSD endpoint could not be reached; RSD identity and services remain independently assessed."),
    }
}

async fn probe_remoted_coexistence(_budget: Duration, _cancel: &ProbeCancel) -> Check {
    #[cfg(target_os = "macos")]
    {
        let mut command = tokio::process::Command::new("/bin/launchctl");
        command.args(["print", "system/com.apple.remoted"]);
        let output = bounded_host_command(command, _budget, _cancel).await;
        match output {
            Ok((status, stdout)) if status.success() => {
                remoted_output_check(&stdout)
            },
            Ok(_) => Check::new("remoted_coexistence", Status::Unknown,
                "The system remoted observation command exited unsuccessfully; no permission or service-state conclusion is available.").state(device::State::Failed),
            Err(error) => Check::failure("remoted_coexistence", error,
                "The system remoted service state could not be observed; no coexistence conclusion is available."),
        }
    }
    #[cfg(not(target_os = "macos"))]
    {
        Check::new(
            "remoted_coexistence",
            Status::Unknown,
            "macOS remoted coexistence is not applicable on this host.",
        )
        .state(device::State::Unsupported)
    }
}

#[cfg(any(target_os = "macos", test))]
fn remoted_output_check(stdout: &[u8]) -> Check {
    let text = String::from_utf8_lossy(stdout);
    let running = text.lines().any(|line| line.trim() == "state = running");
    let pid = text
        .lines()
        .find_map(|line| line.trim().strip_prefix("pid = ")?.parse::<u32>().ok())
        .filter(|pid| *pid > 0);
    let mut check = Check::new("remoted_coexistence", Status::Unknown,
        "launchctl observed the system remoted service. Coexistence requires a successful selected existing RSD observation while this service remains running; host enumeration alone does not establish it.").state(device::State::Available);
    check.remoted_observations.push(RemotedObservation {
        running: running && pid.is_some(),
        pid,
        observed_at: chrono::Utc::now().to_rfc3339(),
    });
    check
}

#[cfg(any(target_os = "macos", test))]
fn reconcile_remoted(before: &mut Check, mut after: Check, selected_rsd_ok: bool) {
    let observed = before
        .remoted_observations
        .first()
        .is_some_and(|state| state.running)
        && after
            .remoted_observations
            .first()
            .is_some_and(|state| state.running);
    before
        .remoted_observations
        .append(&mut after.remoted_observations);
    if observed && selected_rsd_ok {
        before.status = Status::Ok;
        before.detail = "The selected existing RSD directory was read successfully while launchctl observed the system remoted process running both before and after the query. This bounded observation does not prove uninterrupted liveness between samples.".into();
    } else {
        before.status = Status::Unknown;
        before.detail = "Coexistence was not established: it requires both running remoted process observations and a successful RSD directory observation identifying the selected target.".into();
    }
}

async fn probe_ddi_directory() -> Check {
    let mut search_paths = Vec::new();
    if let Ok(env_path) = std::env::var("IOS_DDI_PATH") {
        search_paths.push(std::path::PathBuf::from(env_path));
    }
    if let Some(home) = dirs_home() {
        search_paths.push(home.join(".ios").join("ddi"));
    }
    #[cfg(target_os = "macos")]
    {
        search_paths.push(std::path::PathBuf::from(
            "/Applications/Xcode.app/Contents/Developer/Platforms/iPhoneOS.platform/DeviceSupport",
        ));
    }

    match tokio::task::spawn_blocking(move || inspect_ddi_directories(&search_paths)).await {
        Ok(check) => check,
        Err(_) => Check::new(
            "ddi_directory",
            Status::Unknown,
            "The DDI directory observation worker did not complete.",
        )
        .state(device::State::Unreachable),
    }
}

fn inspect_ddi_directories(search_paths: &[std::path::PathBuf]) -> Check {
    let mut queue: std::collections::VecDeque<_> =
        search_paths.iter().cloned().map(|path| (path, 0)).collect();
    let mut regular_images = 0usize;
    let mut signed_pairs = 0usize;
    let mut inspected = 0usize;
    let mut unavailable = 0usize;
    let mut first_error = None;
    while let Some((directory, depth)) = queue.pop_front() {
        let entries = match std::fs::read_dir(&directory) {
            Ok(entries) => entries,
            Err(error) => {
                unavailable += 1;
                first_error.get_or_insert(error);
                continue;
            }
        };
        for entry in entries {
            inspected += 1;
            if inspected > 4096 {
                return Check::new("ddi_directory", Status::Unknown, "DDI directory scan reached its 4096-entry limit; no usability conclusion is available.").state(device::State::MetadataMissing);
            }
            let entry = match entry {
                Ok(entry) => entry,
                Err(error) => {
                    unavailable += 1;
                    first_error.get_or_insert(error);
                    continue;
                }
            };
            let kind = match entry.file_type() {
                Ok(kind) => kind,
                Err(error) => {
                    unavailable += 1;
                    first_error.get_or_insert(error);
                    continue;
                }
            };
            // Do not follow symlinks/reparse points outside the selected roots.
            if kind.is_dir() && depth < 2 {
                queue.push_back((entry.path(), depth + 1));
                continue;
            }
            if !kind.is_file()
                || entry.path().extension().and_then(|value| value.to_str()) != Some("dmg")
            {
                continue;
            }
            let metadata = match entry.metadata() {
                Ok(metadata) => metadata,
                Err(error) => {
                    unavailable += 1;
                    first_error.get_or_insert(error);
                    continue;
                }
            };
            if metadata.len() == 0 {
                continue;
            }
            regular_images += 1;
            let signature = entry.path().with_extension("dmg.signature");
            if std::fs::symlink_metadata(&signature)
                .is_ok_and(|meta| meta.is_file() && meta.len() > 0 && meta.len() <= 1024 * 1024)
            {
                signed_pairs += 1;
            }
        }
    }
    let detail = format!("Discovered {regular_images} nonempty regular DDI image files and {signed_pairs} nonempty signature companions; {unavailable} directory entries were unavailable. Target version/manifest matching and cryptographic acceptance remain unprobed.");
    let mut check = if let Some(error) = first_error {
        Check::io_failure(
            "ddi_directory",
            error,
            "DDI directory observation was incomplete.",
        )
    } else {
        Check::new(
            "ddi_directory",
            Status::Unknown,
            "DDI directory observation completed.",
        )
        .state(device::State::Available)
    };
    check.detail = detail;
    check.status = if regular_images == 0 {
        Status::Warning
    } else {
        Status::Unknown
    };
    check.hint("Select the target and use its image-mounter diagnostics to observe mounted-image usability; directory discovery alone is insufficient.")
}

fn dirs_home() -> Option<std::path::PathBuf> {
    std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .map(std::path::PathBuf::from)
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::Parser;

    #[test]
    #[ignore = "owned child-process fixture, invoked by host command owner tests"]
    fn host_process_fixture_child() {
        use std::io::Write;
        let Ok(port) = std::env::var("IOS_DOCTOR_PROCESS_FIXTURE_PORT") else {
            return;
        };
        let mut socket = std::net::TcpStream::connect(format!("127.0.0.1:{port}")).unwrap();
        socket.write_all(b"ready").unwrap();
        if std::env::var("IOS_DOCTOR_PROCESS_FIXTURE_MODE").as_deref() == Ok("limit") {
            let mut output = std::io::stdout().lock();
            for _ in 0..160 {
                output.write_all(&[b'x'; 8192]).unwrap();
            }
            output.flush().unwrap();
        }
        std::thread::sleep(Duration::from_secs(30));
        drop(socket);
    }

    #[tokio::test]
    async fn owned_host_command_limit_timeout_and_cancel_kill_and_reap_the_original_process() {
        use tokio::io::AsyncReadExt;
        for mode in ["limit", "timeout", "cancel"] {
            let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
                .await
                .unwrap();
            let mut command = tokio::process::Command::new(std::env::current_exe().unwrap());
            command
                .args([
                    "--ignored",
                    "--exact",
                    "cmd::doctor::tests::host_process_fixture_child",
                    "--nocapture",
                ])
                .env(
                    "IOS_DOCTOR_PROCESS_FIXTURE_PORT",
                    listener.local_addr().unwrap().port().to_string(),
                )
                .env("IOS_DOCTOR_PROCESS_FIXTURE_MODE", mode);
            let cancel = ProbeCancel::new();
            let (result, peer_close) = tokio::time::timeout(Duration::from_secs(8), async {
                tokio::join!(
                    bounded_host_command(command, Duration::from_secs(3), &cancel),
                    async {
                        let (mut peer, _) = listener.accept().await.unwrap();
                        let mut ready = [0; 5];
                        peer.read_exact(&mut ready).await.unwrap();
                        assert_eq!(&ready, b"ready");
                        if mode == "cancel" {
                            cancel.0.send_replace(true);
                        }
                        peer.read(&mut [0]).await
                    }
                )
            })
            .await
            .unwrap();
            let error = result.unwrap_err();
            match mode {
                "limit" => assert!(format!("{error:#}").contains("output bound exceeded")),
                "timeout" => assert!(error.is::<tokio::time::error::Elapsed>()),
                "cancel" => assert!(error.is::<ios_core::OperationCancelled>()),
                _ => unreachable!(),
            }
            assert!(
                error
                    .downcast_ref::<crate::cmd::test_workflow::WorkflowSecondaryFailure>()
                    .is_none(),
                "normal kill/wait must succeed"
            );
            // Forced process termination on Windows can abort Winsock with
            // ConnectionReset. Record that original result separately; never
            // manufacture an EOF or use it as native tunnel join evidence.
            let observed = match peer_close {
                Ok(0) => "eof",
                #[cfg(windows)]
                Err(error) if error.kind() == std::io::ErrorKind::ConnectionReset => {
                    "connection_reset"
                }
                other => panic!("owned child socket did not close: {other:?}"),
            };
            println!(
                "IOS_DOCTOR_OWNED_CHILD_V1 {}",
                serde_json::json!({
                    "mode": mode, "original_process_reaped": true,
                    "peer_close": observed, "strict_peer_eof": observed == "eof",
                })
            );
        }
    }

    #[test]
    fn remoted_coexistence_requires_both_running_process_samples_and_selected_rsd_identity() {
        for (before_running, after_running, selected_rsd_ok) in [
            (true, true, true),
            (false, true, true),
            (true, false, true),
            (true, true, false),
        ] {
            let output = |running| {
                if running {
                    b"state = running\npid = 42\n".as_slice()
                } else {
                    b"state = waiting\n".as_slice()
                }
            };
            let mut before = remoted_output_check(output(before_running));
            let after = remoted_output_check(output(after_running));
            reconcile_remoted(&mut before, after, selected_rsd_ok);
            assert_eq!(
                before.status == Status::Ok,
                before_running && after_running && selected_rsd_ok
            );
            assert_eq!(before.remoted_observations.len(), 2);
        }
        assert!(!remoted_output_check(b"state = running\n").remoted_observations[0].running);
    }

    fn access_options(path: Option<&std::path::Path>) -> crate::access::ClientAccessArgs {
        let mut args = vec![std::ffi::OsString::from("ios"), "doctor".into()];
        if let Some(path) = path {
            args.extend(["--token-file".into(), path.as_os_str().to_owned()]);
        }
        let crate::Commands::Doctor(command) = crate::Cli::try_parse_from(args).unwrap().command
        else {
            panic!("doctor command");
        };
        command.access
    }

    #[tokio::test]
    async fn private_manager_credentials_report_admission_and_fail_without_leaking_remote_errors() {
        use crate::access::test_support::{grant, PolicyFixture, DEVICE, REPLACEMENT, TOKEN};
        use axum::{
            http::{HeaderMap, StatusCode},
            response::IntoResponse,
            routing::get,
            Router,
        };
        let fixture = PolicyFixture::new(vec![grant(TOKEN, vec![crate::access::Permission::List])]);
        for scope_denied in [false, true] {
            let app = Router::new().route("/", get(move |headers: HeaderMap| async move {
                if !headers.get("authorization").is_some_and(|value| value.as_bytes() == format!("Bearer {TOKEN}").as_bytes()) {
                    return (StatusCode::UNAUTHORIZED, "synthetic-private-remote-error").into_response();
                }
                if scope_denied { return (StatusCode::FORBIDDEN, "synthetic-private-remote-error").into_response(); }
                axum::Json(serde_json::json!({DEVICE: [{"tunnel-address": "fd00::1234", "tunnel-port": 12345}]})).into_response()
            }));
            let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
                .await
                .unwrap();
            let port = listener.local_addr().unwrap().port();
            let server = tokio::spawn(async move {
                axum::serve(listener, app).await.unwrap();
            });
            let no_token = probe_manager(port, &access_options(None)).await;
            assert_eq!(no_token.status, Status::Warning);
            let valid = probe_manager(port, &access_options(Some(&fixture.token_path))).await;
            assert_eq!(
                valid.status,
                if scope_denied {
                    Status::Failed
                } else {
                    Status::Ok
                }
            );
            let diagnostic = serde_json::to_string(&valid).unwrap();
            for secret in [
                TOKEN,
                DEVICE,
                "fd00::1234",
                "synthetic-private-remote-error",
            ] {
                assert!(!diagnostic.contains(secret));
            }
            ios_core::secret_file::write_secret(&fixture.token_path, REPLACEMENT.as_bytes())
                .unwrap();
            let invalid = probe_manager(port, &access_options(Some(&fixture.token_path))).await;
            assert_eq!(invalid.status, Status::Failed);
            assert!(!serde_json::to_string(&invalid)
                .unwrap()
                .contains("synthetic-private-remote-error"));
            ios_core::secret_file::write_secret(&fixture.token_path, TOKEN.as_bytes()).unwrap();
            server.abort();
            let _ = server.await;
        }
        let missing = fixture.token_path.with_file_name("missing-token");
        let failed = probe_manager(1, &access_options(Some(&missing))).await;
        assert_eq!(failed.status, Status::Failed);
        assert!(failed.detail.contains("could not be read"));
    }

    #[tokio::test]
    async fn stalled_probe_becomes_report_entry() {
        let check = bounded(
            "test",
            Duration::from_millis(1),
            Status::Failed,
            std::future::pending(),
        )
        .await;
        assert_eq!(check.status, Status::Failed);
        assert_eq!(check.id, "test");
    }

    #[test]
    fn manager_output_counts_without_leaking_identity_or_addresses() {
        let check = manager_response_check(
            br#"{"secret-udid":[{"tunnel-address":"fd00::1234","tunnel-port":1234}]}"#,
        );
        assert_eq!(check.status, Status::Ok);
        let json = serde_json::to_string(&check).unwrap();
        assert!(json.contains("1 tunnel(s)"));
        assert!(!json.contains("secret-udid"));
        assert!(!json.contains("fd00"));
    }

    #[test]
    fn unrelated_or_malformed_http_service_is_not_a_healthy_manager() {
        for body in [
            b"[]".as_slice(),
            b"{\"hello\":true}",
            b"{\"device\":[{}]}",
            b"{\"device\":[{\"tunnel-address\":\"::1\",\"tunnel-port\":0}]}",
            b"invalid",
        ] {
            assert_eq!(manager_response_check(body).status, Status::Warning);
        }
        assert_eq!(manager_response_check(b"{}").status, Status::Ok);
    }

    #[test]
    fn unknown_and_warning_checks_do_not_fail_report() {
        let mut report = Report {
            schema_version: 1,
            scope: "host",
            os: "test",
            arch: "test",
            version: "test",
            checks: vec![
                Check::new("skipped", Status::Unknown, "not tested"),
                Check::new("optional", Status::Warning, "absent"),
            ],
        };
        assert!(!report.failed());
        report
            .checks
            .push(Check::new("required", Status::Failed, "failed"));
        assert!(report.failed());
    }

    #[tokio::test]
    async fn manager_probe_rejects_redirects_and_oversized_bodies() {
        use axum::{http::StatusCode, response::IntoResponse, routing::get, Router};
        for oversized in [false, true] {
            let app = Router::new().route(
                "/",
                get(move || async move {
                    if oversized {
                        vec![b'x'; MAX_MANAGER_RESPONSE + 1].into_response()
                    } else {
                        (StatusCode::FOUND, [("location", "http://127.0.0.1:1/")]).into_response()
                    }
                }),
            );
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
            let port = listener.local_addr().unwrap().port();
            let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
            let check = bounded(
                "tunnel_manager",
                Duration::from_secs(3),
                Status::Failed,
                probe_manager(port, &access_options(None)),
            )
            .await;
            server.abort();
            assert_eq!(check.status, Status::Warning);
            if oversized {
                assert!(check.detail.contains("exceeded 1 MiB"));
            } else {
                assert!(check.detail.contains("No successful response"));
            }
        }
    }

    #[tokio::test]
    async fn unselected_route_and_remoted_are_not_healthy_observations() {
        let route = probe_platform_route(None).await;
        assert_eq!(route.id, "platform_route");
        assert_eq!(route.status, Status::Unknown);

        let remoted = probe_remoted_coexistence(Duration::from_secs(3), &ProbeCancel::new()).await;
        assert_eq!(remoted.id, "remoted_coexistence");
        assert_eq!(remoted.status, Status::Unknown);
    }

    #[tokio::test]
    async fn ddi_discovery_rejects_named_directory_and_continues_later_roots() {
        let temp_dir = std::env::temp_dir().join(format!("test_ddi_{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&temp_dir).unwrap();
        let dmg_path = temp_dir.join("DeveloperDiskImage.dmg");
        std::fs::create_dir(&dmg_path).unwrap();
        let check = inspect_ddi_directories(std::slice::from_ref(&temp_dir));
        assert_eq!(check.id, "ddi_directory");
        assert_eq!(check.status, Status::Warning);
        assert!(check.detail.contains("Discovered 0 nonempty regular"));
        let later = temp_dir.join("later");
        std::fs::create_dir(&later).unwrap();
        std::fs::write(later.join("DeveloperDiskImage.dmg"), b"synthetic-image").unwrap();
        std::fs::write(
            later.join("DeveloperDiskImage.dmg.signature"),
            b"synthetic-signature",
        )
        .unwrap();
        let empty = temp_dir.join("empty");
        std::fs::create_dir(&empty).unwrap();
        let check = inspect_ddi_directories(&[empty, later]);
        assert_eq!(check.status, Status::Unknown);
        assert!(check.detail.contains("Discovered 1 nonempty regular"));
        assert!(check.detail.contains("1 nonempty signature companions"));

        let _ = std::fs::remove_dir_all(&temp_dir);
    }
}
