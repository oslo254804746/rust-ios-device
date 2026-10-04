use anyhow::Result;
use ios_core::MuxClient;
use ios_core::{default_pair_record_path, PairRecord, LOCKDOWN_PORT};
use serde::Serialize;
use tokio_stream::StreamExt;

#[derive(clap::Args)]
#[command(args_conflicts_with_subcommands = true)]
pub struct PairCmd {
    #[command(subcommand)]
    sub: Option<PairSubcommand>,
    #[arg(long, help = "Device IPv6 address (skip mDNS discovery)")]
    address: Option<String>,
    #[arg(long, help = "Custom credentials directory")]
    creds_dir: Option<String>,
    #[arg(long, help = "List saved pair credentials")]
    list: bool,
}

#[derive(Debug, clap::Subcommand)]
enum PairSubcommand {
    /// Show the local lockdown pair record summary for a device
    ShowRecord {
        #[arg(long, help = "Target device UDID; falls back to global -u/--udid")]
        udid: Option<String>,
    },
    /// Pair with a device discovered via WiFi (Remote Pairing)
    Wifi {
        #[arg(long, help = "Custom credentials directory")]
        creds_dir: Option<String>,
        #[arg(long, help = "Device IPv6 address (skip Bonjour discovery)")]
        address: Option<String>,
        #[arg(long, help = "Filter by device name")]
        name: Option<String>,
    },
    /// Pair a supervised device using a P12 supervisor certificate (no trust dialog)
    Supervised {
        #[arg(long, help = "Path to P12 supervisor certificate file")]
        p12: String,
        #[arg(
            long,
            env = "P12_PASSWORD",
            hide_env_values = true,
            default_value = "",
            help = "P12 password; prefer the P12_PASSWORD env var so it stays out of \
                    shell history and the process list"
        )]
        password: String,
        #[arg(long, help = "Target device UDID; falls back to global -u/--udid")]
        udid: Option<String>,
    },
}

impl PairCmd {
    pub(crate) fn needs_default_udid(&self) -> bool {
        matches!(
            self.sub,
            Some(PairSubcommand::ShowRecord { .. } | PairSubcommand::Supervised { .. })
        )
    }

    pub async fn run(self, default_udid: Option<String>, json: bool) -> Result<()> {
        match self.sub {
            Some(PairSubcommand::ShowRecord { udid }) => {
                let udid = udid
                    .or(default_udid)
                    .ok_or_else(|| anyhow::anyhow!("--udid required for pair show-record"))?;
                return show_pair_record(&udid, json);
            }
            Some(PairSubcommand::Wifi {
                creds_dir,
                address,
                name,
            }) => {
                return wifi_pair(creds_dir, address, name).await;
            }
            Some(PairSubcommand::Supervised {
                p12,
                password,
                udid,
            }) => {
                let udid = udid
                    .or(default_udid)
                    .ok_or_else(|| anyhow::anyhow!("--udid required for pair supervised"))?;
                return supervised_pair_cmd(&udid, &p12, &password, json).await;
            }
            None => {}
        }

        let creds_dir = self
            .creds_dir
            .map(std::path::PathBuf::from)
            .unwrap_or_else(ios_core::PersistedCredentials::default_dir);

        // List mode
        if self.list {
            let creds = ios_core::PersistedCredentials::list(&creds_dir);
            if creds.is_empty() {
                crate::output::outln!("No saved pair credentials found.");
            } else {
                crate::output::outln!("{} credential(s) in {}:", creds.len(), creds_dir.display());
                for c in &creds {
                    crate::output::outln!(
                        "  {} (id={}, rsd_port={})",
                        c.device_address,
                        c.host_identifier,
                        c.rsd_port
                    );
                }
            }
            return Ok(());
        }

        // Resolve device IPv6 address
        let device_addr: std::net::Ipv6Addr = if let Some(addr) = self.address {
            addr.parse()?
        } else {
            crate::output::write_error_line(format_args!(
                "Scanning for iOS 17+ devices via mDNS..."
            ))?;
            crate::output::write_error_line(format_args!(
                "(Make sure the device is connected via USB-Ethernet or Wi-Fi)"
            ))?;
            let mdns_stream = ios_core::discover_mdns().await?;
            tokio::pin!(mdns_stream);

            // Take the first discovered device
            match tokio::time::timeout(
                std::time::Duration::from_secs(15),
                mdns_stream.next(),
            ).await {
                Ok(Some(dev)) => {
                    crate::output::write_error_line(format_args!("Found: {} ({})", ios_core::redaction::diagnostic_id(&dev.name), ios_core::redaction::diagnostic_id(&dev.ipv6.to_string())))?;
                    dev.ipv6
                }
                Ok(None) => anyhow::bail!("mDNS stream ended without finding a device"),
                Err(_)   => anyhow::bail!("No iOS devices found via mDNS within 15 seconds.\nTip: use --address <IPv6> to specify manually."),
            }
        };

        crate::output::write_error_line(format_args!(
            "Connecting to {}...",
            ios_core::redaction::diagnostic_id(&device_addr.to_string())
        ))?;
        crate::output::write_error_line(format_args!(
            "*** Please press 'Trust' on the device when prompted ***"
        ))?;

        // Run SRP pairing
        let creds = ios_core::pair_new_device(device_addr).await?;

        // Save credentials
        let hex_pub = hex::encode(&creds.host_public_key);
        let persisted = ios_core::PersistedCredentials {
            remote_identifier: Some(creds.remote_identifier.clone()),
            host_identifier: creds.host_identifier.clone(),
            host_public_key_hex: hex_pub,
            host_private_key_hex: Some(hex::encode(&creds.host_private_key)),
            remote_unlock_host_key: creds.remote_unlock_host_key.clone(),
            device_address: device_addr.to_string(),
            rsd_port: ios_core::RSD_PORT,
        };
        persisted.save(&creds_dir)?;
        let remote_pair_record = ios_core::RemotePairingRecord {
            public_key: creds.host_public_key.clone(),
            private_key: creds.host_private_key.clone(),
            remote_unlock_host_key: creds.remote_unlock_host_key.clone(),
            peer_alt_irk: creds.peer_alt_irk.clone(),
        };
        remote_pair_record.save_for_identifier(&creds_dir, &creds.remote_identifier)?;

        crate::output::outln!("Pairing successful!");
        crate::output::outln!("  Remote ID: {}", creds.remote_identifier);
        crate::output::outln!("  Host ID: {}", creds.host_identifier);
        crate::output::outln!(
            "  Saved legacy creds: {}",
            ios_core::PersistedCredentials::path_for(&creds_dir, &device_addr.to_string())
                .display()
        );
        crate::output::outln!(
            "  Saved remote pair: {}",
            ios_core::RemotePairingRecord::path_for_identifier(
                &creds_dir,
                &creds.remote_identifier
            )
            .display()
        );

        Ok(())
    }
}

async fn wifi_pair(
    creds_dir: Option<String>,
    address: Option<String>,
    name: Option<String>,
) -> Result<()> {
    let creds_dir = creds_dir
        .map(std::path::PathBuf::from)
        .unwrap_or_else(ios_core::PersistedCredentials::default_dir);

    // Resolve device IPv6 address
    let device_addr: std::net::Ipv6Addr = if let Some(addr) = address {
        addr.parse()?
    } else {
        crate::output::write_error_line(format_args!(
            "Scanning for WiFi pairing services via Bonjour..."
        ))?;
        let mut services =
            ios_core::browse_remotepairing(std::time::Duration::from_secs(15)).await?;

        // Filter by name if provided
        if let Some(ref filter) = name {
            services.retain(|s| s.instance.contains(filter.as_str()));
        }

        let svc = services
            .into_iter()
            .next()
            .ok_or_else(|| anyhow::anyhow!("No WiFi pairing services found via Bonjour"))?;

        crate::output::write_error_line(format_args!(
            "Found: {}",
            ios_core::redaction::diagnostic_id(&svc.instance)
        ))?;

        // Extract an IPv6 address from the discovered service
        svc.addresses
            .iter()
            .filter_map(|a| a.parse::<std::net::Ipv6Addr>().ok())
            .next()
            .ok_or_else(|| {
                anyhow::anyhow!(
                    "Service '{}' has no IPv6 address (addresses: {:?})",
                    svc.instance,
                    svc.addresses
                )
            })?
    };

    crate::output::write_error_line(format_args!(
        "Connecting to {}...",
        ios_core::redaction::diagnostic_id(&device_addr.to_string())
    ))?;
    crate::output::write_error_line(format_args!(
        "*** Please press 'Trust' on the device when prompted ***"
    ))?;

    // Run SRP pairing
    let creds = ios_core::pair_new_device(device_addr).await?;

    // Save credentials
    let hex_pub = hex::encode(&creds.host_public_key);
    let persisted = ios_core::PersistedCredentials {
        remote_identifier: Some(creds.remote_identifier.clone()),
        host_identifier: creds.host_identifier.clone(),
        host_public_key_hex: hex_pub,
        host_private_key_hex: Some(hex::encode(&creds.host_private_key)),
        remote_unlock_host_key: creds.remote_unlock_host_key.clone(),
        device_address: device_addr.to_string(),
        rsd_port: ios_core::RSD_PORT,
    };
    persisted.save(&creds_dir)?;
    let remote_pair_record = ios_core::RemotePairingRecord {
        public_key: creds.host_public_key.clone(),
        private_key: creds.host_private_key.clone(),
        remote_unlock_host_key: creds.remote_unlock_host_key.clone(),
        peer_alt_irk: creds.peer_alt_irk.clone(),
    };
    remote_pair_record.save_for_identifier(&creds_dir, &creds.remote_identifier)?;

    crate::output::outln!("WiFi pairing successful!");
    crate::output::outln!("  Remote ID: {}", creds.remote_identifier);
    crate::output::outln!("  Host ID: {}", creds.host_identifier);
    crate::output::outln!(
        "  Saved legacy creds: {}",
        ios_core::PersistedCredentials::path_for(&creds_dir, &device_addr.to_string()).display()
    );
    crate::output::outln!(
        "  Saved remote pair: {}",
        ios_core::RemotePairingRecord::path_for_identifier(&creds_dir, &creds.remote_identifier)
            .display()
    );

    Ok(())
}

#[derive(Serialize)]
struct PairRecordSummary {
    udid: String,
    path: String,
    host_id: String,
    system_buid: String,
    device_certificate_bytes: usize,
    host_certificate_bytes: usize,
    host_private_key_bytes: usize,
    root_certificate_bytes: usize,
}

async fn supervised_pair_cmd(udid: &str, p12_path: &str, password: &str, json: bool) -> Result<()> {
    // 1. Read P12 file
    let p12_bytes = std::fs::read(p12_path)
        .map_err(|e| anyhow::anyhow!("failed to read P12 file '{}': {}", p12_path, e))?;

    // 2. Connect to usbmuxd and find the device
    let mut mux = MuxClient::connect().await?;
    let buid = mux.read_buid().await?;
    let devices = mux.list_devices().await?;
    let dev = devices
        .iter()
        .find(|d| d.serial_number == udid)
        .ok_or_else(|| anyhow::anyhow!("device not found: {udid}"))?;
    let device_id = dev.device_id;
    let serial = dev.serial_number.clone();

    // 3. Connect to lockdown port via usbmux (raw, no TLS)
    // Need a fresh MuxClient because connect_to_port consumes self
    let mut mux2 = MuxClient::connect().await?;
    mux2.read_pair_record(udid).await.ok(); // best-effort
    let mut stream = mux2.connect_to_port(device_id, LOCKDOWN_PORT).await?;

    // 4. Optionally get WiFi address before pairing (uses the same raw stream)
    //    We skip this because the stream will be consumed by the pair protocol.
    //    WiFi address can be obtained separately if needed.

    // 5. Run supervised pairing protocol
    crate::output::write_error_line(format_args!(
        "Starting supervised pairing for device {}...",
        ios_core::redaction::diagnostic_id(&serial)
    ))?;
    let (pair_record, escrow_bag) =
        ios_core::pair_supervised(&mut stream, &p12_bytes, password, &buid).await?;

    // 6. Save pair record to disk
    let pair_record_path = default_pair_record_path(udid);
    ios_core::save_pair_record(&pair_record, &escrow_bag, None, &pair_record_path)?;

    if json {
        crate::output::outln!(
            "{}",
            serde_json::to_string_pretty(&serde_json::json!({
                "success": true,
                "udid": serial,
                "host_id": pair_record.host_id,
                "system_buid": pair_record.system_buid,
                "pair_record_path": pair_record_path.display().to_string(),
                "escrow_bag_bytes": escrow_bag.len(),
            }))?
        );
    } else {
        crate::output::outln!("Supervised pairing successful!");
        crate::output::outln!("  UDID: {}", serial);
        crate::output::outln!("  HostID: {}", pair_record.host_id);
        crate::output::outln!("  SystemBUID: {}", pair_record.system_buid);
        crate::output::outln!("  Pair record: {}", pair_record_path.display());
        crate::output::outln!("  EscrowBag: {} bytes", escrow_bag.len());
    }

    Ok(())
}

fn show_pair_record(udid: &str, json: bool) -> Result<()> {
    let path = default_pair_record_path(udid);
    let record = PairRecord::load(udid)?;
    let summary = PairRecordSummary {
        udid: udid.to_string(),
        path: path.display().to_string(),
        host_id: record.host_id,
        system_buid: record.system_buid,
        device_certificate_bytes: record.device_certificate.len(),
        host_certificate_bytes: record.host_certificate.len(),
        host_private_key_bytes: record.host_private_key.len(),
        root_certificate_bytes: record.root_certificate.len(),
    };

    if json {
        crate::output::outln!("{}", serde_json::to_string_pretty(&summary)?);
    } else {
        crate::output::outln!("UDID: {}", summary.udid);
        crate::output::outln!("Path: {}", summary.path);
        crate::output::outln!("HostID: {}", summary.host_id);
        crate::output::outln!("SystemBUID: {}", summary.system_buid);
        crate::output::outln!(
            "DeviceCertificateBytes: {}",
            summary.device_certificate_bytes
        );
        crate::output::outln!("HostCertificateBytes: {}", summary.host_certificate_bytes);
        crate::output::outln!("HostPrivateKeyBytes: {}", summary.host_private_key_bytes);
        crate::output::outln!("RootCertificateBytes: {}", summary.root_certificate_bytes);
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    #[cfg(target_os = "linux")]
    #[test]
    fn diagnostic_sources_pair_stderr_failure_returns_before_discovery() {
        const TEST: &str =
            "cmd::pair::tests::diagnostic_sources_pair_stderr_failure_returns_before_discovery";
        if std::env::var_os("IOS_A02_PAIR_STDERR_CHILD").is_some() {
            let runtime = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .unwrap();
            let error = runtime
                .block_on(super::wifi_pair(None, None, None))
                .unwrap_err();
            let native = error.downcast_ref::<std::io::Error>().unwrap();
            assert_eq!(native.raw_os_error(), Some(28));
            assert_eq!(
                ios_core::classify_error(error.as_ref()),
                ios_core::ErrorClass::Io
            );
            println!("A02 pair stderr native ENOSPC retained before discovery");
            return;
        }
        let mut child = std::process::Command::new(std::env::current_exe().unwrap())
            .args(["--exact", TEST, "--nocapture"])
            .env("IOS_A02_PAIR_STDERR_CHILD", "1")
            .stdout(std::process::Stdio::piped())
            .stderr(
                std::fs::OpenOptions::new()
                    .write(true)
                    .open("/dev/full")
                    .unwrap(),
            )
            .spawn()
            .unwrap();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
        while child.try_wait().unwrap().is_none() {
            if std::time::Instant::now() >= deadline {
                child.kill().unwrap();
                child.wait().unwrap();
                panic!("owned diagnostic child exceeded its budget");
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        let output = child.wait_with_output().unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stdout)
        );
        assert!(String::from_utf8_lossy(&output.stdout)
            .contains("A02 pair stderr native ENOSPC retained before discovery"));
    }
}
