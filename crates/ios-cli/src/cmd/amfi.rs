use anyhow::Result;
use ios_core::TunMode;
use ios_core::{connect, ConnectOptions};

#[derive(clap::Args)]
pub struct AmfiCmd {
    #[command(subcommand)]
    sub: AmfiSub,
}

#[derive(Debug, clap::Subcommand)]
enum AmfiSub {
    /// Reveal the Developer Mode option in the device Settings UI
    RevealDeveloperMode,
    /// Request enabling Developer Mode through com.apple.amfi.lockdown
    EnableDeveloperMode,
}

impl AmfiCmd {
    pub async fn run(self, udid: Option<String>, json: bool) -> Result<()> {
        let udid = udid.ok_or_else(|| anyhow::anyhow!("--udid required for amfi"))?;

        match self.sub {
            AmfiSub::RevealDeveloperMode => reveal_developer_mode(&udid, json).await,
            AmfiSub::EnableDeveloperMode => enable_developer_mode(&udid, json).await,
        }
    }
}

async fn reveal_developer_mode(udid: &str, json: bool) -> Result<()> {
    let opts = ConnectOptions {
        tun_mode: TunMode::Userspace,
        pair_record_path: None,
        skip_tunnel: true,
    };
    let device = connect(udid, opts).await?;
    let stream = device
        .connect_service_with_timeout(
            ios_core::amfi::SERVICE_NAME,
            std::time::Duration::from_secs(30),
        )
        .await?;
    let mut client = ios_core::amfi::AmfiClient::new(stream);
    client.set_operation_timeout(Some(std::time::Duration::from_secs(30)));
    client
        .reveal_developer_mode()
        .await
        .map_err(|e| anyhow::Error::new(e).context("amfi reveal-developer-mode failed"))?;

    if json {
        crate::output::outln!(
            "{}",
            serde_json::to_string_pretty(&serde_json::json!({
                "developer_mode_revealed": true,
            }))?
        );
    } else {
        crate::output::outln!(
            "Developer Mode option revealed. Open Settings > Privacy & Security on the device."
        );
    }

    Ok(())
}

async fn enable_developer_mode(udid: &str, json: bool) -> Result<()> {
    let opts = ConnectOptions {
        tun_mode: TunMode::Userspace,
        pair_record_path: None,
        skip_tunnel: true,
    };
    let device = connect(udid, opts).await?;
    let stream = device
        .connect_service_with_timeout(
            ios_core::amfi::SERVICE_NAME,
            std::time::Duration::from_secs(30),
        )
        .await?;
    let mut client = ios_core::amfi::AmfiClient::new(stream);
    client.set_operation_timeout(Some(std::time::Duration::from_secs(30)));
    client
        .enable_developer_mode()
        .await
        .map_err(|e| anyhow::Error::new(e).context("amfi enable-developer-mode failed"))?;

    if json {
        crate::output::outln!(
            "{}",
            serde_json::to_string_pretty(&serde_json::json!({
                "developer_mode_enable_requested": true,
                "restart_required": true,
            }))?
        );
    } else {
        crate::output::outln!(
            "Developer Mode enable requested. Restart the device and confirm the prompt."
        );
    }

    Ok(())
}
