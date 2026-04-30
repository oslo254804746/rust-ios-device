use anyhow::Result;
use ios_core::TunMode;
use ios_core::{connect, ConnectOptions};

#[derive(clap::Args)]
pub struct IdamCmd {
    #[command(subcommand)]
    sub: IdamSub,
}

#[derive(clap::Subcommand)]
enum IdamSub {
    /// Query the current IDAM configuration
    Get,
    /// Set the IDAM configuration state
    Set { state: IdamState },
}

#[derive(Clone, Debug, clap::ValueEnum)]
enum IdamState {
    On,
    Off,
}

impl IdamCmd {
    pub async fn run(self, udid: Option<String>, json: bool) -> Result<()> {
        let udid = udid.ok_or_else(|| anyhow::anyhow!("--udid required for idam"))?;
        let device = connect(
            &udid,
            ConnectOptions {
                tun_mode: TunMode::Userspace,
                pair_record_path: None,
                skip_tunnel: true,
            },
        )
        .await?;
        let stream = device.connect_service(ios_core::idam::SERVICE_NAME).await?;
        let mut client = ios_core::idam::IdamClient::new(stream);

        match self.sub {
            IdamSub::Get => {
                let value = client.configuration_inquiry().await?;
                print_plist_value(&value, json)?;
            }
            IdamSub::Set { state } => {
                let value = client
                    .set_configuration(matches!(state, IdamState::On))
                    .await?;
                print_plist_value(&value, json)?;
            }
        }

        Ok(())
    }
}

fn print_plist_value(value: &plist::Value, json: bool) -> Result<()> {
    if json {
        println!("{}", serde_json::to_string_pretty(value)?);
    } else if let Some(text) = value.as_string() {
        println!("{text}");
    } else {
        println!("{}", serde_json::to_string_pretty(value)?);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use clap::Parser;

    use super::IdamCmd;

    #[derive(Parser)]
    struct TestCli {
        #[command(flatten)]
        command: IdamCmd,
    }

    #[test]
    fn parses_idam_get_subcommand() {
        let parsed = TestCli::try_parse_from(["idam", "get"]);
        assert!(parsed.is_ok(), "idam get command should parse");
    }

    #[test]
    fn parses_idam_set_subcommand() {
        let parsed = TestCli::try_parse_from(["idam", "set", "on"]);
        assert!(parsed.is_ok(), "idam set command should parse");
    }
}
