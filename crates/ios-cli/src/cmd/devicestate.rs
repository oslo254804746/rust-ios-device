use anyhow::Result;
use comfy_table::{Cell, Table};

#[derive(clap::Args)]
pub struct DeviceStateCmd {
    #[command(subcommand)]
    sub: DeviceStateSub,
}

#[derive(clap::Subcommand)]
enum DeviceStateSub {
    /// List available condition inducer profiles
    List {
        #[arg(short = 'j', long, help = "Output JSON")]
        json: bool,
    },
    /// Enable a condition profile
    Enable {
        profile_type_id: String,
        profile_id: String,
    },
    /// Disable any active condition
    Disable,
}

impl DeviceStateCmd {
    pub async fn run(self, udid: Option<String>) -> Result<()> {
        let udid = udid.ok_or_else(|| anyhow::anyhow!("--udid required for devicestate"))?;
        let (_device, stream) = super::instruments::connect_instruments(&udid).await?;
        let mut client = ios_core::services::instruments::DeviceStateClient::connect(stream)
            .await
            .map_err(|e| anyhow::anyhow!("DTX error: {e}"))?;

        match self.sub {
            DeviceStateSub::List { json } => {
                let profiles = client.list().await.map_err(|e| anyhow::anyhow!("{e}"))?;
                if json {
                    println!("{}", serde_json::to_string_pretty(&profiles)?);
                } else {
                    let mut table = Table::new();
                    table.set_header([
                        "Type",
                        "Name",
                        "Active",
                        "Profile",
                        "ProfileName",
                        "Description",
                    ]);
                    for profile_type in profiles {
                        for profile in profile_type.profiles {
                            table.add_row([
                                Cell::new(&profile_type.identifier),
                                Cell::new(&profile_type.name),
                                Cell::new(if profile_type.is_active { "yes" } else { "no" }),
                                Cell::new(&profile.identifier),
                                Cell::new(&profile.name),
                                Cell::new(&profile.description),
                            ]);
                        }
                    }
                    println!("{table}");
                }
            }
            DeviceStateSub::Enable {
                profile_type_id,
                profile_id,
            } => {
                let enabled = client
                    .enable(&profile_type_id, &profile_id)
                    .await
                    .map_err(|e| anyhow::anyhow!("{e}"))?;
                if !enabled {
                    anyhow::bail!(
                        "device reported failure enabling profile {profile_id} for {profile_type_id}"
                    );
                }
                println!("Enabled {profile_type_id}:{profile_id}");
            }
            DeviceStateSub::Disable => {
                client.disable().await.map_err(|e| anyhow::anyhow!("{e}"))?;
                println!("Disabled active condition");
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use clap::Parser;

    use super::*;

    #[derive(Parser)]
    struct TestCli {
        #[command(subcommand)]
        command: DeviceStateSub,
    }

    #[test]
    fn parses_devicestate_list_subcommand() {
        let parsed = TestCli::try_parse_from(["devicestate", "list", "--json"]);
        assert!(parsed.is_ok(), "devicestate list command should parse");
    }

    #[test]
    fn parses_devicestate_enable_subcommand() {
        let parsed = TestCli::try_parse_from([
            "devicestate",
            "enable",
            "SlowNetworkCondition",
            "SlowNetwork3GGood",
        ]);
        assert!(parsed.is_ok(), "devicestate enable command should parse");
    }
}
