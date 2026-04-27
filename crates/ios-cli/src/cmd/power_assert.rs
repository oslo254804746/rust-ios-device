use anyhow::Result;
use ios_core::tunnel::TunMode;
use ios_core::{connect, ConnectOptions};

#[derive(clap::Args)]
pub struct PowerAssertCmd {
    #[arg(long, default_value = "60", help = "Assertion lifetime in seconds")]
    timeout: u64,
    #[arg(
        long,
        default_value = "PreventUserIdleSystemSleep",
        help = "Assertion type"
    )]
    assertion_type: String,
    #[arg(
        long,
        default_value = "ios-cli power assertion",
        help = "Assertion name"
    )]
    name: String,
    #[arg(long, help = "Optional assertion detail string")]
    details: Option<String>,
}

impl PowerAssertCmd {
    pub async fn run(self, udid: Option<String>, json: bool) -> Result<()> {
        let udid = udid.ok_or_else(|| anyhow::anyhow!("--udid required for power-assert"))?;
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
            .connect_service(ios_core::power_assertion::SERVICE_NAME)
            .await?;
        let mut client = ios_core::power_assertion::PowerAssertionClient::new(stream);
        let response = client
            .create_assertion(
                &self.assertion_type,
                &self.name,
                self.timeout as f64,
                self.details.as_deref(),
            )
            .await?;

        if json {
            println!(
                "{}",
                serde_json::to_string_pretty(&plist::Value::Dictionary(response))?
            );
        } else {
            println!(
                "Created power assertion '{}' for {}s. Press Ctrl+C to release early.",
                self.name, self.timeout
            );
        }

        tokio::select! {
            _ = tokio::signal::ctrl_c() => {}
            _ = tokio::time::sleep(std::time::Duration::from_secs(self.timeout)) => {}
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
        #[command(flatten)]
        command: PowerAssertCmd,
    }

    #[test]
    fn parses_power_assert_flags() {
        let parsed = TestCli::try_parse_from([
            "power-assert",
            "--timeout",
            "10",
            "--assertion-type",
            "PreventUserIdleSystemSleep",
            "--name",
            "hold-awake",
        ]);
        assert!(parsed.is_ok(), "power-assert flags should parse");
    }
}
