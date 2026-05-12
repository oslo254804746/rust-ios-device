use std::time::Duration;

use anyhow::Result;
use ios_core::TunMode;
use ios_core::{connect, ConnectOptions};

#[derive(clap::Args)]
pub struct RestoreCmd {
    #[command(subcommand)]
    sub: RestoreSub,
}

#[derive(clap::Subcommand)]
enum RestoreSub {
    /// Reboot the device into recovery mode
    EnterRecovery,
    /// Set delay-recovery-image for supported devices
    DelayRecoveryImage,
    /// Reboot the device via restore service
    Reboot,
    /// Query restore preflight info
    PreflightInfo,
    /// Query AP/SEP nonces from restore service
    Nonces,
    /// Query restore app parameters
    AppParameters,
    /// Set the restore language
    RestoreLang {
        /// Language identifier to request from restore service
        language: String,
    },
    /// Read restore lifecycle events without starting a destructive restore
    Events {
        /// Maximum number of lifecycle events to read
        #[arg(long)]
        count: Option<usize>,
        /// Stop waiting when no event arrives before this timeout
        #[arg(long, default_value_t = 300)]
        timeout_secs: u64,
    },
}

impl RestoreCmd {
    pub async fn run(self, udid: Option<String>, json: bool) -> Result<()> {
        let udid = udid.ok_or_else(|| anyhow::anyhow!("--udid required for restore"))?;
        let success_message = match &self.sub {
            RestoreSub::EnterRecovery => Some("Recovery request accepted."),
            RestoreSub::DelayRecoveryImage => Some("Delay recovery image request accepted."),
            RestoreSub::Reboot => Some("Reboot request accepted."),
            _ => None,
        };
        let device = connect(
            &udid,
            ConnectOptions {
                tun_mode: TunMode::Userspace,
                pair_record_path: None,
                skip_tunnel: false,
            },
        )
        .await?;

        let stream = device
            .connect_rsd_service(ios_core::restore::SERVICE_NAME)
            .await?;
        let mut client = ios_core::restore::RestoreServiceClient::connect(stream).await?;

        if let RestoreSub::Events {
            count,
            timeout_secs,
        } = self.sub
        {
            let events = read_restore_events(&mut client, count, timeout_secs).await?;
            let rendered: Vec<_> = events
                .iter()
                .map(ios_core::restore::restore_lifecycle_event_to_json)
                .collect();
            if json {
                println!("{}", serde_json::to_string_pretty(&rendered)?);
            } else {
                for event in rendered {
                    println!("{}", serde_json::to_string_pretty(&event)?);
                }
            }
            return Ok(());
        }

        let response = match self.sub {
            RestoreSub::EnterRecovery => client.enter_recovery().await?,
            RestoreSub::DelayRecoveryImage => client.delay_recovery_image().await?,
            RestoreSub::Reboot => client.reboot().await?,
            RestoreSub::PreflightInfo => client.get_preflight_info().await?,
            RestoreSub::Nonces => client.get_nonces().await?,
            RestoreSub::AppParameters => client.get_app_parameters().await?,
            RestoreSub::RestoreLang { language } => client.restore_lang(language).await?,
            RestoreSub::Events { .. } => unreachable!("events handled above"),
        };

        let rendered =
            ios_core::restore::xpc_value_to_json(&ios_core::XpcValue::Dictionary(response.clone()));
        if json {
            println!("{}", serde_json::to_string_pretty(&rendered)?);
        } else if let Some(message) = success_message {
            println!("{message}");
        } else {
            println!("{}", serde_json::to_string_pretty(&rendered)?);
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
        command: RestoreCmd,
    }

    #[test]
    fn parses_restore_enter_recovery_subcommand() {
        let parsed = TestCli::try_parse_from(["restore", "enter-recovery"]);
        assert!(parsed.is_ok(), "restore enter-recovery should parse");
    }

    #[test]
    fn parses_restore_preflight_info_subcommand() {
        let parsed = TestCli::try_parse_from(["restore", "preflight-info"]);
        assert!(parsed.is_ok(), "restore preflight-info should parse");
    }

    #[test]
    fn parses_restore_delay_recovery_image_subcommand() {
        let parsed = TestCli::try_parse_from(["restore", "delay-recovery-image"]);
        assert!(parsed.is_ok(), "restore delay-recovery-image should parse");
    }

    #[test]
    fn parses_restore_reboot_subcommand() {
        let parsed = TestCli::try_parse_from(["restore", "reboot"]);
        assert!(parsed.is_ok(), "restore reboot should parse");
    }

    #[test]
    fn parses_restore_app_parameters_subcommand() {
        let parsed = TestCli::try_parse_from(["restore", "app-parameters"]);
        assert!(parsed.is_ok(), "restore app-parameters should parse");
    }

    #[test]
    fn parses_restore_lang_subcommand() {
        let parsed = TestCli::try_parse_from(["restore", "restore-lang", "en"]);
        assert!(parsed.is_ok(), "restore restore-lang should parse");
    }

    #[test]
    fn parses_restore_events_subcommand() {
        let parsed =
            TestCli::try_parse_from(["restore", "events", "--count", "2", "--timeout-secs", "30"]);
        assert!(parsed.is_ok(), "restore events should parse");
    }
}

async fn read_restore_events<S>(
    client: &mut ios_core::restore::RestoreServiceClient<S>,
    count: Option<usize>,
    timeout_secs: u64,
) -> Result<Vec<ios_core::restore::RestoreLifecycleEvent>>
where
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin,
{
    let mut events = Vec::new();
    loop {
        if count.is_some_and(|count| events.len() >= count) {
            break;
        }

        let event = tokio::time::timeout(
            Duration::from_secs(timeout_secs),
            client.next_lifecycle_event(),
        )
        .await
        .map_err(|_| anyhow::anyhow!("timed out waiting for restore lifecycle event"))??;
        let finished = matches!(
            event,
            ios_core::restore::RestoreLifecycleEvent::Status { finished: true, .. }
        );
        events.push(event);
        if finished {
            break;
        }
    }
    Ok(events)
}
