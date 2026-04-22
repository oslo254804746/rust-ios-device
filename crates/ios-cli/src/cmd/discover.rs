use std::time::Duration;

use anyhow::Result;

#[derive(clap::Args)]
pub struct DiscoverCmd {
    #[command(subcommand)]
    sub: DiscoverSub,
}

#[derive(clap::Subcommand)]
enum DiscoverSub {
    /// Browse for iOS 17+ devices via mDNS (_remoted._tcp)
    Mdns {
        #[arg(long, default_value = "5", help = "Discovery timeout in seconds")]
        timeout: u64,
        #[arg(short = 'j', long, help = "Output JSON")]
        json: bool,
    },
    /// Browse for mobdev2 Wi-Fi devices (_apple-mobdev2._tcp)
    Mobdev2 {
        #[arg(long, default_value = "5", help = "Discovery timeout in seconds")]
        timeout: u64,
        #[arg(short = 'j', long, help = "Output JSON")]
        json: bool,
    },
    /// Browse for remote pairing services (_remotepairing._tcp)
    Remotepairing {
        #[arg(long, default_value = "5", help = "Discovery timeout in seconds")]
        timeout: u64,
        #[arg(short = 'j', long, help = "Output JSON")]
        json: bool,
    },
}

impl DiscoverCmd {
    pub async fn run(self, json_global: bool) -> Result<()> {
        match self.sub {
            DiscoverSub::Mdns { timeout, json } => {
                let json = json || json_global;
                use tokio_stream::StreamExt;

                let stream = ios_core::discover_mdns().await?;
                tokio::pin!(stream);

                let deadline = tokio::time::Instant::now() + Duration::from_secs(timeout);
                let mut devices = Vec::new();

                loop {
                    match tokio::time::timeout_at(deadline, stream.next()).await {
                        Ok(Some(device)) => {
                            if json {
                                devices.push(serde_json::json!({
                                    "udid": device.udid,
                                    "ipv6": device.ipv6.to_string(),
                                    "rsd_port": device.rsd_port,
                                    "name": device.name,
                                }));
                            } else {
                                println!(
                                    "{:<45} [{}]:{} {}",
                                    device.udid, device.ipv6, device.rsd_port, device.name
                                );
                            }
                        }
                        Ok(None) | Err(_) => break,
                    }
                }

                if json {
                    println!("{}", serde_json::to_string_pretty(&devices)?);
                } else if devices.is_empty() {
                    println!("No mDNS devices discovered within {timeout}s.");
                }
            }
            DiscoverSub::Mobdev2 { timeout, json } => {
                let json = json || json_global;
                let services = ios_core::browse_mobdev2(Duration::from_secs(timeout)).await?;
                print_bonjour_services(&services, json)?;
            }
            DiscoverSub::Remotepairing { timeout, json } => {
                let json = json || json_global;
                let services = ios_core::browse_remotepairing(Duration::from_secs(timeout)).await?;
                print_bonjour_services(&services, json)?;
            }
        }
        Ok(())
    }
}

fn print_bonjour_services(services: &[ios_core::BonjourService], json: bool) -> Result<()> {
    if json {
        let list: Vec<_> = services
            .iter()
            .map(|s| {
                serde_json::json!({
                    "instance": s.instance,
                    "port": s.port,
                    "addresses": s.addresses,
                    "properties": s.properties,
                })
            })
            .collect();
        println!("{}", serde_json::to_string_pretty(&list)?);
    } else if services.is_empty() {
        println!("No services discovered.");
    } else {
        for s in services {
            println!(
                "{:<60} port={} addrs={}",
                s.instance,
                s.port,
                s.addresses.join(", ")
            );
            for (key, value) in &s.properties {
                println!("  {key}={value}");
            }
        }
    }
    Ok(())
}
