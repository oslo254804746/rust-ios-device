use std::path::PathBuf;

use anyhow::Result;
use ios_core::mcinstall::{build_wifi_profile, McInstallClient};
use ios_core::{connect, ConnectOptions, TunMode};

#[derive(clap::Args)]
pub struct WifiCmd {
    #[command(subcommand)]
    sub: WifiSub,
}

#[derive(clap::Subcommand)]
enum WifiSub {
    /// Install a managed Wi-Fi profile through MCInstall.
    Install {
        #[arg(help = "Wi-Fi network name")]
        ssid: String,
        #[arg(
            long,
            env = "IOS_WIFI_PASSWORD",
            help = "Wi-Fi password (never printed)"
        )]
        password: String,
        #[arg(long, default_value = "WPA", help = "Apple Wi-Fi encryption type")]
        encryption_type: String,
        #[arg(
            long,
            help = "Also save the generated mobileconfig with owner-only permissions"
        )]
        profile_output: Option<PathBuf>,
        #[arg(long, help = "Replace an existing profile output file")]
        force: bool,
    },
    /// Remove the deterministic managed Wi-Fi profile for an SSID.
    Remove {
        #[arg(help = "Wi-Fi network name")]
        ssid: String,
        #[arg(long, help = "Required confirmation for profile removal")]
        force: bool,
    },
}

impl WifiCmd {
    pub async fn run(self, udid: Option<String>, json: bool) -> Result<()> {
        let udid = udid.ok_or_else(|| anyhow::anyhow!("--udid required for wifi"))?;
        if let WifiSub::Install {
            profile_output: Some(path),
            force,
            ..
        } = &self.sub
        {
            // Preflight this before opening MCInstall so a refused local
            // overwrite cannot leave a profile installed on the device.
            super::file::ensure_local_overwrite_allowed(path, *force)?;
        }
        let device = connect(
            &udid,
            ConnectOptions {
                tun_mode: TunMode::Userspace,
                pair_record_path: None,
                skip_tunnel: true,
            },
        )
        .await?;
        let activation_state = device
            .lockdown_get_value(Some("ActivationState"))
            .await?
            .as_string()
            .map(str::to_owned)
            .ok_or_else(|| anyhow::anyhow!("lockdown ActivationState was not a string"))?;
        if activation_state == "Unactivated" {
            anyhow::bail!("please activate the device first");
        }
        let stream = device
            .connect_service(ios_core::mcinstall::SERVICE_NAME)
            .await?;
        let mut client = McInstallClient::new(stream);

        match self.sub {
            WifiSub::Install {
                ssid,
                password,
                encryption_type,
                profile_output,
                force,
            } => {
                let password = zeroize::Zeroizing::new(password);
                let profile = build_wifi_profile(&ssid, password.as_str(), &encryption_type)
                    .map_err(|err| anyhow::anyhow!("invalid Wi-Fi profile: {err}"))?;
                let result = client
                    .prepare_wifi(&ssid, password.as_str(), &encryption_type)
                    .await?;
                if let Some(path) = profile_output.as_ref() {
                    super::file::write_local_bytes_atomic(path, profile.as_ref(), force).await?;
                }
                if json {
                    println!("{}", serde_json::to_string_pretty(&result)?);
                } else {
                    println!(
                        "Installed Wi-Fi profile {} (supervised: {})",
                        result.profile_identifier, result.supervised
                    );
                    if let Some(path) = profile_output {
                        println!("Profile written to {}", path.display());
                    }
                }
            }
            WifiSub::Remove { ssid, force } => {
                crate::output::require_force(
                    force,
                    "remove the Wi-Fi profile",
                    "the managed Wi-Fi profile is removed from the device",
                )?;
                let result = client.remove_wifi(&ssid).await?;
                if json {
                    println!(
                        "{}",
                        serde_json::to_string_pretty(&serde_json::json!({
                            "removed": true,
                            "profile_identifier": result.profile_identifier,
                            "supervised": result.supervised,
                        }))?
                    );
                } else {
                    println!(
                        "Removed Wi-Fi profile {} (supervised: {})",
                        result.profile_identifier, result.supervised
                    );
                }
            }
        }
        Ok(())
    }
}
