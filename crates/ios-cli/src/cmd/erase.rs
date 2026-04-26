use anyhow::Result;
use ios_core::tunnel::TunMode;
use ios_core::{connect, ConnectOptions};

#[derive(clap::Args)]
pub struct EraseCmd {
    #[arg(long, help = "Required confirmation for destructive erase")]
    force: bool,
}

impl EraseCmd {
    pub async fn run(self, udid: Option<String>, json: bool) -> Result<()> {
        if !self.force {
            return Err(anyhow::anyhow!("refusing to erase device without --force"));
        }

        let udid = udid.ok_or_else(|| anyhow::anyhow!("--udid required for erase"))?;
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
            .connect_service(ios_core::services::mcinstall::SERVICE_NAME)
            .await?;
        let mut client = ios_core::services::mcinstall::McInstallClient::new(stream);
        let _ = client.flush().await;
        client.erase_device(true, false).await?;

        if json {
            println!(
                "{}",
                serde_json::to_string_pretty(&serde_json::json!({
                    "erased": true,
                    "preserve_data_plan": true,
                    "disallow_proximity_setup": false,
                }))?
            );
        } else {
            println!("Erase request sent. The device should reboot shortly.");
        }

        Ok(())
    }
}
