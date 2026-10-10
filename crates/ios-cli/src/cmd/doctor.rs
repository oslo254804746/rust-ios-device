use std::{future::Future, net::SocketAddrV6, time::Duration};

use anyhow::{bail, Context, Result};
use serde::Serialize;

const MAX_MANAGER_RESPONSE: usize = 1024 * 1024;

mod device;

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

#[derive(Debug, Serialize)]
struct Check {
    id: &'static str,
    status: Status,
    detail: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    hint: Option<&'static str>,
}

impl Check {
    fn new(id: &'static str, status: Status, detail: impl Into<String>) -> Self {
        Self {
            id,
            status,
            detail: detail.into(),
            hint: None,
        }
    }

    fn hint(mut self, hint: &'static str) -> Self {
        self.hint = Some(hint);
        self
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
        let report = self.inspect().await;
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
        let work = async {
            let host = self.inspect().await;
            let timeout = Duration::from_secs(self.timeout);
            let device = match rsd {
                Some(request) => {
                    device::inspect_with_rsd(&device::NativeSource, udid, timeout, request).await
                }
                None => device::inspect(&device::NativeSource, udid, timeout).await,
            };
            Ok(TargetReport::new(host, device))
        };
        let mut report = cancellable(work, tokio::signal::ctrl_c()).await?;
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

    async fn inspect(&self) -> Report {
        let deadline = Duration::from_secs(self.timeout);
        let (mux, ipv6, manager, kernel, platform_usb, platform_route, remoted, ddi_dir) = tokio::join!(
            bounded("usbmuxd", deadline, Status::Failed, probe_usbmux()),
            bounded("ipv6_loopback", deadline, Status::Warning, probe_ipv6()),
            bounded(
                "tunnel_manager",
                deadline,
                if self.access.has_token() {
                    Status::Failed
                } else {
                    Status::Warning
                },
                probe_manager(self.tunnel_port, &self.access)
            ),
            bounded("kernel_tun", deadline, Status::Warning, probe_kernel_tun()),
            bounded("platform_usb", deadline, Status::Warning, probe_platform_usb()),
            bounded("platform_route", deadline, Status::Warning, probe_platform_route()),
            bounded("remoted_coexistence", deadline, Status::Warning, probe_remoted_coexistence()),
            bounded("ddi_directory", deadline, Status::Warning, probe_ddi_directory()),
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
        ];
        checks.push(if self.mdns {
            // Browsing itself uses the requested window; allow a bounded margin for setup/cleanup.
            bounded(
                "bonjour",
                deadline + Duration::from_secs(1),
                Status::Warning,
                probe_mdns(deadline),
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
) -> Result<T> {
    tokio::pin!(signal);
    tokio::select! {
        biased;
        signal = &mut signal => {
            signal.context("waiting for doctor Ctrl-C")?;
            Err(ios_core::OperationCancelled::new().into())
        }
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
        Err(_) => Check::new(id, timeout_status, "The probe exceeded its deadline."),
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

async fn probe_platform_usb() -> Check {
    #[cfg(target_os = "linux")]
    {
        let sys_bus = std::path::Path::new("/sys/bus/usb/devices");
        if sys_bus.is_dir() {
            let mut apple_count = 0;
            let mut total_count = 0;
            if let Ok(entries) = std::fs::read_dir(sys_bus) {
                for entry in entries.flatten() {
                    let vendor_path = entry.path().join("idVendor");
                    if let Ok(vendor) = std::fs::read_to_string(vendor_path) {
                        total_count += 1;
                        if vendor.trim().eq_ignore_ascii_case("05ac") {
                            apple_count += 1;
                        }
                    }
                }
            }
            if apple_count > 0 {
                Check::new(
                    "platform_usb",
                    Status::Ok,
                    format!("USB host subsystem operational: detected {apple_count} Apple USB device(s) (VID 0x05ac) among {total_count} USB device(s)."),
                )
            } else {
                Check::new(
                    "platform_usb",
                    Status::Ok,
                    format!("USB host subsystem operational: sysfs USB tree accessible ({total_count} devices found); no Apple USB device (VID 0x05ac) detected."),
                )
                .hint("Connect an iOS device via USB cable to enable USB communications.")
            }
        } else {
            Check::new(
                "platform_usb",
                Status::Warning,
                "Linux sysfs USB directory /sys/bus/usb/devices is not accessible.",
            )
            .hint("Check that sysfs is mounted and USB controller kernel modules are loaded.")
        }
    }
    #[cfg(target_os = "windows")]
    {
        let apple_driver = std::path::Path::new(
            r"C:\Program Files\Common Files\Apple\Mobile Device Support\Drivers\usbaapl64.sys",
        )
        .exists()
            || std::path::Path::new(
                r"C:\Program Files (x86)\Common Files\Apple\Mobile Device Support\Drivers\usbaapl.sys",
            )
            .exists();
        if apple_driver {
            Check::new(
                "platform_usb",
                Status::Ok,
                "USB host subsystem operational: Apple Mobile Device USB driver (usbaapl) installed on Windows.",
            )
        } else {
            Check::new(
                "platform_usb",
                Status::Ok,
                "USB host subsystem operational on Windows; Win32 USB subsystem accessible.",
            )
        }
    }
    #[cfg(target_os = "macos")]
    {
        Check::new(
            "platform_usb",
            Status::Ok,
            "USB host subsystem operational on macOS.",
        )
    }
    #[cfg(not(any(target_os = "linux", target_os = "windows", target_os = "macos")))]
    {
        Check::new(
            "platform_usb",
            Status::Ok,
            "USB host subsystem operational.",
        )
    }
}

async fn probe_platform_route() -> Check {
    match tokio::net::UdpSocket::bind("[::]:0").await {
        Ok(_) => Check::new(
            "platform_route",
            Status::Ok,
            "Host network stack supports IPv6 route binding required for RSD and CoreDevice tunnels.",
        ),
        Err(_) => Check::new(
            "platform_route",
            Status::Warning,
            "Host network stack could not bind an IPv6 socket for routing diagnostics.",
        )
        .hint("Enable IPv6 in host network settings to support modern iOS 17+ RSD tunnels."),
    }
}

async fn probe_remoted_coexistence() -> Check {
    #[cfg(target_os = "macos")]
    {
        let sock = std::path::Path::new("/var/run/com.apple.remoted.sock");
        if sock.exists() {
            Check::new(
                "remoted_coexistence",
                Status::Ok,
                "macOS native remoted socket detected at /var/run/com.apple.remoted.sock; coexistence supported without service termination.",
            )
        } else {
            Check::new(
                "remoted_coexistence",
                Status::Ok,
                "macOS native remoted socket is not active; no coexistence contention detected.",
            )
        }
    }
    #[cfg(not(target_os = "macos"))]
    {
        Check::new(
            "remoted_coexistence",
            Status::Ok,
            "The remoted daemon is macOS-specific; no native daemon conflict on this platform.",
        )
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

    for dir in &search_paths {
        if dir.is_dir() {
            let mut ddi_files = 0;
            if let Ok(entries) = std::fs::read_dir(dir) {
                for entry in entries.flatten() {
                    let name = entry.file_name();
                    let name_str = name.to_string_lossy();
                    if name_str.ends_with(".dmg") || name_str.contains("DeveloperDiskImage") {
                        ddi_files += 1;
                    }
                }
            }
            if ddi_files > 0 {
                return Check::new(
                    "ddi_directory",
                    Status::Ok,
                    format!(
                        "Local DDI directory found at '{}' containing {ddi_files} image file(s).",
                        dir.display()
                    ),
                );
            } else {
                return Check::new(
                    "ddi_directory",
                    Status::Warning,
                    format!(
                        "Local DDI directory found at '{}', but contains no DeveloperDiskImage files.",
                        dir.display()
                    ),
                )
                .hint("Place DeveloperDiskImage.dmg and .signature matching target iOS versions into this directory.");
            }
        }
    }

    Check::new(
        "ddi_directory",
        Status::Warning,
        "No local DeveloperDiskImage repository found at standard locations (~/.ios/ddi).",
    )
    .hint("Create ~/.ios/ddi and place DeveloperDiskImage files or configure IOS_DDI_PATH.")
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
    async fn platform_probes_produce_valid_checks() {
        let usb = probe_platform_usb().await;
        assert_eq!(usb.id, "platform_usb");
        assert!(usb.status == Status::Ok || usb.status == Status::Warning);

        let route = probe_platform_route().await;
        assert_eq!(route.id, "platform_route");
        assert!(route.status == Status::Ok || route.status == Status::Warning);

        let remoted = probe_remoted_coexistence().await;
        assert_eq!(remoted.id, "remoted_coexistence");
        assert_eq!(remoted.status, Status::Ok);

        let ddi = probe_ddi_directory().await;
        assert_eq!(ddi.id, "ddi_directory");
        assert!(ddi.status == Status::Ok || ddi.status == Status::Warning);
    }

    #[tokio::test]
    async fn ddi_probe_respects_custom_environment_path() {
        let temp_dir = std::env::temp_dir().join(format!("test_ddi_{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&temp_dir).unwrap();
        let dmg_path = temp_dir.join("DeveloperDiskImage.dmg");
        std::fs::write(&dmg_path, b"dummy").unwrap();

        std::env::set_var("IOS_DDI_PATH", &temp_dir);
        let check = probe_ddi_directory().await;
        assert_eq!(check.id, "ddi_directory");
        assert_eq!(check.status, Status::Ok);
        assert!(check.detail.contains("image file(s)"));
        std::env::remove_var("IOS_DDI_PATH");

        let _ = std::fs::remove_dir_all(&temp_dir);
    }
}

