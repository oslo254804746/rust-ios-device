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
    let mut stream = device.connect_service(ios_core::amfi::SERVICE_NAME).await?;
    ios_core::amfi::reveal_developer_mode(&mut stream)
        .await
        .map_err(|e| anyhow::anyhow!("amfi reveal-developer-mode failed: {e}"))?;

    if json {
        println!(
            "{}",
            serde_json::to_string_pretty(&serde_json::json!({
                "developer_mode_revealed": true,
            }))?
        );
    } else {
        println!(
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
    let mut stream = device.connect_service(ios_core::amfi::SERVICE_NAME).await?;
    ios_core::amfi::enable_developer_mode(&mut stream)
        .await
        .map_err(|e| anyhow::anyhow!("amfi enable-developer-mode failed: {e}"))?;

    if json {
        println!(
            "{}",
            serde_json::to_string_pretty(&serde_json::json!({
                "developer_mode_enable_requested": true,
                "restart_required": true,
            }))?
        );
    } else {
        println!("Developer Mode enable requested. Restart the device and confirm the prompt.");
    }

    Ok(())
}
