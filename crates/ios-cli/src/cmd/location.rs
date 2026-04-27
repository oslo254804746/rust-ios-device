use std::path::PathBuf;

use anyhow::Result;
use ios_core::tunnel::TunMode;
use ios_core::{connect, ConnectOptions};
use tokio::fs;

#[derive(clap::Args)]
pub struct LocationCmd {
    #[command(subcommand)]
    sub: LocationSub,
}

#[derive(clap::Subcommand)]
enum LocationSub {
    /// Set a custom simulated location
    Set { latitude: String, longitude: String },
    /// Replay a GPX track file against the device
    Gpx { file: PathBuf },
    /// Reset location simulation back to the device's GPS
    Reset,
}

impl LocationCmd {
    pub async fn run(self, udid: Option<String>) -> Result<()> {
        let udid = udid.ok_or_else(|| anyhow::anyhow!("--udid required for location"))?;

        match self.sub {
            LocationSub::Set {
                latitude,
                longitude,
            } => {
                latitude
                    .parse::<f64>()
                    .map_err(|e| anyhow::anyhow!("invalid latitude '{latitude}': {e}"))?;
                longitude
                    .parse::<f64>()
                    .map_err(|e| anyhow::anyhow!("invalid longitude '{longitude}': {e}"))?;

                let opts = ConnectOptions {
                    tun_mode: TunMode::Userspace,
                    pair_record_path: None,
                    skip_tunnel: true,
                };
                let device = connect(&udid, opts).await?;
                let mut stream = device
                    .connect_service(ios_core::simlocation::SERVICE_NAME)
                    .await?;
                ios_core::simlocation::set_location(&mut *stream, &latitude, &longitude)
                    .await
                    .map_err(|e| anyhow::anyhow!("simlocation set failed: {e}"))?;
                println!("Location simulation set to {latitude}, {longitude}");
            }
            LocationSub::Gpx { file } => {
                let gpx = fs::read_to_string(&file).await.map_err(|e| {
                    anyhow::anyhow!("failed to read GPX file '{}': {e}", file.display())
                })?;

                let opts = ConnectOptions {
                    tun_mode: TunMode::Userspace,
                    pair_record_path: None,
                    skip_tunnel: true,
                };
                let device = connect(&udid, opts).await?;
                let mut stream = device
                    .connect_service(ios_core::simlocation::SERVICE_NAME)
                    .await?;
                let count = ios_core::simlocation::replay_gpx_route(&mut *stream, &gpx)
                    .await
                    .map_err(|e| anyhow::anyhow!("simlocation GPX replay failed: {e}"))?;
                println!("Replayed {count} GPX point(s) from {}", file.display());
            }
            LocationSub::Reset => {
                let opts = ConnectOptions {
                    tun_mode: TunMode::Userspace,
                    pair_record_path: None,
                    skip_tunnel: true,
                };
                let device = connect(&udid, opts).await?;
                let mut stream = device
                    .connect_service(ios_core::simlocation::SERVICE_NAME)
                    .await?;
                ios_core::simlocation::reset_location(&mut *stream)
                    .await
                    .map_err(|e| anyhow::anyhow!("simlocation reset failed: {e}"))?;
                println!("Location simulation reset.");
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
        command: LocationSub,
    }

    #[test]
    fn parses_set_subcommand() {
        let cmd = TestCli::parse_from(["location", "set", "48.856614", "2.3522219"]);
        match cmd.command {
            LocationSub::Set {
                latitude,
                longitude,
            } => {
                assert_eq!(latitude, "48.856614");
                assert_eq!(longitude, "2.3522219");
            }
            LocationSub::Gpx { .. } | LocationSub::Reset => panic!("expected set subcommand"),
        }
    }

    #[test]
    fn parses_reset_subcommand() {
        let cmd = TestCli::parse_from(["location", "reset"]);
        assert!(matches!(cmd.command, LocationSub::Reset));
    }

    #[test]
    fn parses_gpx_subcommand() {
        let cmd = TestCli::parse_from(["location", "gpx", "route.gpx"]);
        match cmd.command {
            LocationSub::Gpx { file } => assert_eq!(file, PathBuf::from("route.gpx")),
            _ => panic!("expected gpx subcommand"),
        }
    }
}
