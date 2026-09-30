use std::{future::Future, time::Duration};

use anyhow::{bail, Result};
use serde::Serialize;

const MAX_MANAGER_RESPONSE: usize = 1024 * 1024;

/// Read-only host checks. Device trust, Developer Mode and DDI are not probed.
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
    pub async fn run(self, json: bool) -> Result<()> {
        let report = self.inspect().await;
        if json {
            println!("{}", serde_json::to_string_pretty(&report)?);
        } else {
            println!(
                "ios {} host diagnostics ({} / {})",
                report.version, report.os, report.arch
            );
            for check in &report.checks {
                let status = match check.status {
                    Status::Ok => "OK",
                    Status::Warning => "WARN",
                    Status::Failed => "FAIL",
                    Status::Unknown => "?",
                };
                println!("[{status}] {}: {}", check.id, check.detail);
                if let Some(hint) = check.hint {
                    println!("       {hint}");
                }
            }
        }
        if report.failed() {
            bail!("host diagnostics found a failed check");
        }
        Ok(())
    }

    async fn inspect(&self) -> Report {
        let deadline = Duration::from_secs(self.timeout);
        let (mux, ipv6, manager, kernel) = tokio::join!(
            bounded("usbmuxd", deadline, Status::Failed, probe_usbmux()),
            bounded("ipv6_loopback", deadline, Status::Warning, probe_ipv6()),
            bounded(
                "tunnel_manager",
                deadline,
                Status::Warning,
                probe_manager(self.tunnel_port)
            ),
            bounded("kernel_tun", deadline, Status::Warning, probe_kernel_tun()),
        );
        let mut checks = vec![mux, ipv6, manager, kernel,
            Check::new("userspace_tunnel", Status::Ok,
                "Userspace tunnel support is compiled in; establishing a device tunnel was not tested."),
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

async fn probe_manager(port: u16) -> Check {
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
    let mut response = match client.get(format!("http://127.0.0.1:{port}/")).send().await {
        Ok(response) if response.status().is_success() => response,
        _ => {
            return Check::new(
                "tunnel_manager",
                Status::Warning,
                "No successful response from the optional loopback tunnel manager.",
            )
            .hint("Start ios tunnel serve if shared tunnels are needed, or select --tunnel-port.")
        }
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

#[cfg(test)]
mod tests {
    use super::*;

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
                probe_manager(port),
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
}
