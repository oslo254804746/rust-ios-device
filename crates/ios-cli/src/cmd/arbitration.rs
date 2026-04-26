use anyhow::Result;
use ios_core::tunnel::TunMode;
use ios_core::{connect, ConnectOptions};

#[derive(clap::Args)]
pub struct ArbitrationCmd {
    #[command(subcommand)]
    sub: ArbitrationSub,
}

#[derive(clap::Subcommand)]
enum ArbitrationSub {
    /// Acquire the device for this host
    CheckIn {
        #[arg(long, help = "Override hostname sent to the arbitration service")]
        hostname: Option<String>,
        #[arg(
            long,
            help = "Force a check-in if another host already owns the device"
        )]
        force: bool,
    },
    /// Release the device
    CheckOut,
    /// Query arbitration service version info
    Version,
}

impl ArbitrationCmd {
    pub async fn run(self, udid: Option<String>, json: bool) -> Result<()> {
        let udid = udid.ok_or_else(|| anyhow::anyhow!("--udid required for arbitration"))?;
        let stream = connect_arbitration(&udid).await?;
        let mut client = ios_core::services::arbitration::ArbitrationClient::new(stream);

        match self.sub {
            ArbitrationSub::CheckIn { hostname, force } => {
                let hostname = hostname
                    .or_else(|| std::env::var("COMPUTERNAME").ok())
                    .unwrap_or_else(|| "ios-cli".to_string());
                client.check_in(&hostname, force).await?;
                if json {
                    println!(
                        "{}",
                        serde_json::json!({
                            "result": "success",
                            "command": if force { "force-check-in" } else { "check-in" },
                            "hostname": hostname,
                        })
                    );
                } else {
                    println!("Checked in device for host {hostname}");
                }
            }
            ArbitrationSub::CheckOut => {
                client.check_out().await?;
                if json {
                    println!(
                        "{}",
                        serde_json::json!({ "result": "success", "command": "check-out" })
                    );
                } else {
                    println!("Checked out device");
                }
            }
            ArbitrationSub::Version => {
                let version = client.version().await?;
                println!(
                    "{}",
                    serde_json::to_string_pretty(&plist::Value::Dictionary(version))?
                );
            }
        }
        Ok(())
    }
}

async fn connect_arbitration(udid: &str) -> Result<ios_core::device::ServiceStream> {
    let probe = connect(
        udid,
        ConnectOptions {
            tun_mode: TunMode::Userspace,
            pair_record_path: None,
            skip_tunnel: true,
        },
    )
    .await?;
    let version = probe.product_version().await?;
    drop(probe);

    if should_use_tunnel_for_arbitration(version.major) {
        let device = connect(
            udid,
            ConnectOptions {
                tun_mode: TunMode::Userspace,
                pair_record_path: None,
                skip_tunnel: false,
            },
        )
        .await?;
        tokio::time::sleep(std::time::Duration::from_secs(2)).await;
        device
            .connect_rsd_service(ios_core::services::arbitration::SERVICE_NAME)
            .await
            .map_err(|err| {
                anyhow::anyhow!(
                    "arbitration tunnel fallback reached RSD, but '{}' is not exposed there: {err}",
                    ios_core::services::arbitration::SERVICE_NAME
                )
            })
    } else {
        let device = connect(
            udid,
            ConnectOptions {
                tun_mode: TunMode::Userspace,
                pair_record_path: None,
                skip_tunnel: true,
            },
        )
        .await?;
        device
            .connect_service(ios_core::services::arbitration::SERVICE_NAME)
            .await
            .map_err(Into::into)
    }
}

fn should_use_tunnel_for_arbitration(major_version: u64) -> bool {
    major_version >= 17
}

#[cfg(test)]
mod tests {
    use clap::Parser;

    use super::*;

    #[derive(Parser)]
    struct TestCli {
        #[command(subcommand)]
        command: ArbitrationSub,
    }

    #[test]
    fn parses_arbitration_check_in_subcommand() {
        let parsed = TestCli::try_parse_from(["arbitration", "check-in", "--hostname", "builder"]);
        assert!(parsed.is_ok(), "arbitration check-in command should parse");
    }

    #[test]
    fn parses_arbitration_check_out_subcommand() {
        let parsed = TestCli::try_parse_from(["arbitration", "check-out"]);
        assert!(parsed.is_ok(), "arbitration check-out command should parse");
    }

    #[test]
    fn arbitration_uses_tunnel_on_ios_17_and_newer() {
        assert!(should_use_tunnel_for_arbitration(17));
        assert!(should_use_tunnel_for_arbitration(18));
        assert!(!should_use_tunnel_for_arbitration(16));
    }
}
