use anyhow::Result;
use ios_core::{connect, ConnectOptions};
use ios_tunnel::TunMode;

#[derive(clap::Args)]
pub struct CompanionCmd {
    #[command(subcommand)]
    sub: CompanionSub,
}

#[derive(clap::Subcommand)]
enum CompanionSub {
    /// List paired companion devices
    List,
    /// Query a registry value for a paired companion device
    Get { udid: String, key: String },
}

impl CompanionCmd {
    pub async fn run(self, udid: Option<String>, json: bool) -> Result<()> {
        let udid = udid.ok_or_else(|| anyhow::anyhow!("--udid required for companion"))?;
        let device = connect(
            &udid,
            ConnectOptions {
                tun_mode: TunMode::Userspace,
                pair_record_path: None,
                skip_tunnel: true,
            },
        )
        .await?;
        let stream = device
            .connect_service(ios_services::companion::SERVICE_NAME)
            .await?;
        let mut client = ios_services::companion::CompanionProxyClient::new(stream);

        match self.sub {
            CompanionSub::List => {
                let devices = client.list().await?;
                if json {
                    println!("{}", serde_json::to_string_pretty(&devices)?);
                } else {
                    for device in devices {
                        println!("{}", serde_json::to_string_pretty(&device)?);
                    }
                }
            }
            CompanionSub::Get { udid, key } => {
                let value = client.get_value(&udid, &key).await?;
                if json {
                    println!("{}", serde_json::to_string_pretty(&value)?);
                } else if let Some(string) = value.as_string() {
                    println!("{string}");
                } else {
                    println!("{}", serde_json::to_string_pretty(&value)?);
                }
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
        command: CompanionSub,
    }

    #[test]
    fn parses_companion_list_subcommand() {
        let parsed = TestCli::try_parse_from(["companion", "list"]);
        assert!(parsed.is_ok(), "companion list command should parse");
    }

    #[test]
    fn parses_companion_get_subcommand() {
        let parsed = TestCli::try_parse_from(["companion", "get", "watch-udid", "name"]);
        assert!(parsed.is_ok(), "companion get command should parse");
    }
}
