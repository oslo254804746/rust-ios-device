use std::path::PathBuf;

use anyhow::Result;
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
                    .map_err(|e| crate::output::input_error("invalid latitude", e))?;
                longitude
                    .parse::<f64>()
                    .map_err(|e| crate::output::input_error("invalid longitude", e))?;

                let (_device, stream) = crate::cmd::connect::connect_lockdown_service(
                    &udid,
                    ios_core::simlocation::SERVICE_NAME,
                    std::time::Duration::from_secs(30),
                )
                .await?;
                let mut client = ios_core::simlocation::SimLocationClient::new(stream);
                client.set_operation_timeout(Some(std::time::Duration::from_secs(30)));
                client
                    .set_location(&latitude, &longitude)
                    .await
                    .map_err(|e| anyhow::Error::new(e).context("simlocation set failed"))?;
                crate::output::outln!("Location simulation set to {latitude}, {longitude}");
            }
            LocationSub::Gpx { file } => {
                let gpx = fs::read_to_string(&file)
                    .await
                    .map_err(|e| anyhow::Error::new(e).context("failed to read GPX file"))?;

                let (_device, stream) = crate::cmd::connect::connect_lockdown_service(
                    &udid,
                    ios_core::simlocation::SERVICE_NAME,
                    std::time::Duration::from_secs(30),
                )
                .await?;
                let mut client = ios_core::simlocation::SimLocationClient::new(stream);
                client.set_operation_timeout(Some(std::time::Duration::from_secs(30)));
                let count = tokio::select! {
                    result = client.replay_gpx_route(&gpx) => result,
                    signal = tokio::signal::ctrl_c() => {
                        signal?;
                        return Err(std::io::Error::new(std::io::ErrorKind::Interrupted, "GPX replay cancelled; previously sent locations may remain active").into());
                    }
                }
                    .map_err(|e| anyhow::Error::new(e).context("simlocation GPX replay failed"))?;
                crate::output::outln!("Replayed {count} GPX point(s) from {}", file.display());
            }
            LocationSub::Reset => {
                let (_device, stream) = crate::cmd::connect::connect_lockdown_service(
                    &udid,
                    ios_core::simlocation::SERVICE_NAME,
                    std::time::Duration::from_secs(30),
                )
                .await?;
                let mut client = ios_core::simlocation::SimLocationClient::new(stream);
                client.set_operation_timeout(Some(std::time::Duration::from_secs(30)));
                client
                    .reset_location()
                    .await
                    .map_err(|e| anyhow::Error::new(e).context("simlocation reset failed"))?;
                crate::output::outln!("Location simulation reset.");
            }
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use clap::Parser;

    use super::*;

    #[tokio::test]
    async fn cli_native_sources_location_input_and_file_fail_before_connection() {
        for (latitude, longitude) in [
            ("owned-private-latitude", "1"),
            ("1", "owned-private-longitude"),
        ] {
            let command = LocationCmd {
                sub: LocationSub::Set {
                    latitude: latitude.to_owned(),
                    longitude: longitude.to_owned(),
                },
            };
            let error = command
                .run(Some("owned-private-device".to_owned()))
                .await
                .unwrap_err();
            assert_eq!(
                ios_core::classify_error(error.as_ref()),
                ios_core::ErrorClass::InvalidInput
            );
            assert!(error
                .chain()
                .any(|source| source.is::<std::num::ParseFloatError>()));
            let json = crate::output::operation_failure_message(
                crate::output::ErrorFormat::Json,
                "location",
                &error,
            )
            .unwrap();
            assert!(!json.contains("owned-private"));
        }
        let command = LocationCmd {
            sub: LocationSub::Gpx {
                file: PathBuf::from(""),
            },
        };
        let error = command
            .run(Some("owned-private-device".to_owned()))
            .await
            .unwrap_err();
        let native = error.downcast_ref::<std::io::Error>().unwrap();
        assert_eq!(native.kind(), std::io::ErrorKind::NotFound);
        assert!(native.raw_os_error().is_some());
        assert_eq!(
            ios_core::classify_error(error.as_ref()),
            ios_core::ErrorClass::NotFound
        );
    }

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
