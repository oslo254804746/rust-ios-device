use anyhow::Result;
use ios_core::lockdown::pair_record::{default_pair_record_path, PairRecord};
use ios_core::lockdown::supervised_pair;
use ios_core::mux::MuxClient;
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
        #[arg(long, help = "P12 password", default_value = "")]
        password: String,
        #[arg(long, help = "Target device UDID; falls back to global -u/--udid")]
        udid: Option<String>,
    },
}

impl PairCmd {
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
                println!("No saved pair credentials found.");
            } else {
                println!("{} credential(s) in {}:", creds.len(), creds_dir.display());
                for c in &creds {
                    println!(
                        "  {} (id={}, rsd_port={})",
                        c.device_address, c.host_identifier, c.rsd_port
                    );
                }
            }
            return Ok(());
        }

        // Resolve device IPv6 address
        let device_addr: std::net::Ipv6Addr = if let Some(addr) = self.address {
            addr.parse()?
        } else {
            eprintln!("Scanning for iOS 17+ devices via mDNS...");
            eprintln!("(Make sure the device is connected via USB-Ethernet or Wi-Fi)");
            let mdns_stream = ios_core::discover_mdns().await?;
            tokio::pin!(mdns_stream);

            // Take the first discovered device
            match tokio::time::timeout(
                std::time::Duration::from_secs(15),
                mdns_stream.next(),
            ).await {
                Ok(Some(dev)) => {
                    eprintln!("Found: {} ({})", dev.name, dev.ipv6);
                    dev.ipv6
                }
                Ok(None) => anyhow::bail!("mDNS stream ended without finding a device"),
                Err(_)   => anyhow::bail!("No iOS devices found via mDNS within 15 seconds.\nTip: use --address <IPv6> to specify manually."),
            }
        };

        eprintln!("Connecting to {}...", device_addr);
        eprintln!("*** Please press 'Trust' on the device when prompted ***");

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
            rsd_port: ios_core::xpc::rsd::RSD_PORT,
        };
        persisted.save(&creds_dir)?;
        let remote_pair_record = ios_core::RemotePairingRecord {
            public_key: creds.host_public_key.clone(),
            private_key: creds.host_private_key.clone(),
            remote_unlock_host_key: creds.remote_unlock_host_key.clone(),
        };
        remote_pair_record.save_for_identifier(&creds_dir, &creds.remote_identifier)?;

        println!("Pairing successful!");
        println!("  Remote ID: {}", creds.remote_identifier);
        println!("  Host ID: {}", creds.host_identifier);
        println!(
            "  Saved legacy creds: {}",
            ios_core::PersistedCredentials::path_for(&creds_dir, &device_addr.to_string())
                .display()
        );
        println!(
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
        eprintln!("Scanning for WiFi pairing services via Bonjour...");
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

        eprintln!("Found: {}", svc.instance);

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

    eprintln!("Connecting to {}...", device_addr);
    eprintln!("*** Please press 'Trust' on the device when prompted ***");

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
        rsd_port: ios_core::xpc::rsd::RSD_PORT,
    };
    persisted.save(&creds_dir)?;
    let remote_pair_record = ios_core::RemotePairingRecord {
        public_key: creds.host_public_key.clone(),
        private_key: creds.host_private_key.clone(),
        remote_unlock_host_key: creds.remote_unlock_host_key.clone(),
    };
    remote_pair_record.save_for_identifier(&creds_dir, &creds.remote_identifier)?;

    println!("WiFi pairing successful!");
    println!("  Remote ID: {}", creds.remote_identifier);
    println!("  Host ID: {}", creds.host_identifier);
    println!(
        "  Saved legacy creds: {}",
        ios_core::PersistedCredentials::path_for(&creds_dir, &device_addr.to_string()).display()
    );
    println!(
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
    let mut stream = mux2
        .connect_to_port(device_id, ios_core::lockdown::LOCKDOWN_PORT)
        .await?;

    // 4. Optionally get WiFi address before pairing (uses the same raw stream)
    //    We skip this because the stream will be consumed by the pair protocol.
    //    WiFi address can be obtained separately if needed.

    // 5. Run supervised pairing protocol
    eprintln!("Starting supervised pairing for device {}...", serial);
    let (pair_record, escrow_bag) =
        supervised_pair::pair_supervised(&mut stream, &p12_bytes, password, &buid).await?;

    // 6. Save pair record to disk
    let pair_record_path = default_pair_record_path(udid);
    supervised_pair::save_pair_record(&pair_record, &escrow_bag, None, &pair_record_path)?;

    if json {
        println!(
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
        println!("Supervised pairing successful!");
        println!("  UDID: {}", serial);
        println!("  HostID: {}", pair_record.host_id);
        println!("  SystemBUID: {}", pair_record.system_buid);
        println!("  Pair record: {}", pair_record_path.display());
        println!("  EscrowBag: {} bytes", escrow_bag.len());
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
        println!("{}", serde_json::to_string_pretty(&summary)?);
    } else {
        println!("UDID: {}", summary.udid);
        println!("Path: {}", summary.path);
        println!("HostID: {}", summary.host_id);
        println!("SystemBUID: {}", summary.system_buid);
        println!(
            "DeviceCertificateBytes: {}",
            summary.device_certificate_bytes
        );
        println!("HostCertificateBytes: {}", summary.host_certificate_bytes);
        println!("HostPrivateKeyBytes: {}", summary.host_private_key_bytes);
        println!("RootCertificateBytes: {}", summary.root_certificate_bytes);
    }

    Ok(())
}
