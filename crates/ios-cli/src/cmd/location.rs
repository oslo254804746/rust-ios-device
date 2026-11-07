use std::path::PathBuf;

use anyhow::Result;
use tokio::fs;

const LOCATION_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(30);

enum LocationClient {
    Legacy(ios_core::simlocation::SimLocationClient<ios_core::device::ServiceStream>),
    Modern(ios_core::instruments::LocationSimulationClient<ios_core::device::ServiceStream>),
}

fn uses_dvt(major: u64) -> bool {
    major >= 17
}

async fn connect_location(udid: &str) -> Result<(ios_core::ConnectedDevice, LocationClient)> {
    let version = crate::cmd::connect::probe_product_version(udid).await?;
    if uses_dvt(version.major) {
        let (device, stream) = tokio::time::timeout(
            LOCATION_TIMEOUT,
            crate::cmd::instruments::connect_instruments(udid),
        )
        .await??;
        let client = tokio::time::timeout(
            LOCATION_TIMEOUT,
            ios_core::instruments::LocationSimulationClient::connect(stream),
        )
        .await??;
        Ok((device, LocationClient::Modern(client)))
    } else {
        let (device, stream) = crate::cmd::connect::connect_lockdown_service(
            udid,
            ios_core::simlocation::SERVICE_NAME,
            LOCATION_TIMEOUT,
        )
        .await?;
        let mut client = ios_core::simlocation::SimLocationClient::new(stream);
        client.set_operation_timeout(Some(LOCATION_TIMEOUT));
        Ok((device, LocationClient::Legacy(client)))
    }
}

fn parse_latitude(latitude: &str) -> Result<f64> {
    let value = latitude
        .parse::<f64>()
        .map_err(|e| crate::output::input_error("invalid latitude", e))?;
    if !value.is_finite() || !(-90.0..=90.0).contains(&value) {
        return Err(crate::output::input_decision(
            "latitude must be a finite value in [-90.0, 90.0]",
        ));
    }
    Ok(value)
}

fn parse_longitude(longitude: &str) -> Result<f64> {
    let value = longitude
        .parse::<f64>()
        .map_err(|e| crate::output::input_error("invalid longitude", e))?;
    if !value.is_finite() || !(-180.0..=180.0).contains(&value) {
        return Err(crate::output::input_decision(
            "longitude must be a finite value in [-180.0, 180.0]",
        ));
    }
    Ok(value)
}

impl LocationClient {
    async fn set_location(&mut self, latitude: &str, longitude: &str) -> Result<()> {
        match self {
            Self::Legacy(client) => client
                .set_location(latitude, longitude)
                .await
                .map_err(Into::into),
            Self::Modern(client) => {
                let latitude = parse_latitude(latitude)?;
                let longitude = parse_longitude(longitude)?;
                tokio::time::timeout(LOCATION_TIMEOUT, client.set_location(latitude, longitude))
                    .await??;
                Ok(())
            }
        }
    }

    async fn reset_location(&mut self) -> Result<()> {
        match self {
            Self::Legacy(client) => client.reset_location().await.map_err(Into::into),
            Self::Modern(client) => {
                tokio::time::timeout(LOCATION_TIMEOUT, client.reset_location()).await??;
                Ok(())
            }
        }
    }

    async fn replay_gpx_route(&mut self, gpx: &str) -> Result<usize> {
        match self {
            Self::Legacy(client) => Ok(client.replay_gpx_route(gpx).await?),
            Self::Modern(_) => {
                let route = ios_core::simlocation::parse_gpx_route(gpx)?;
                for point in &route {
                    if !point.delay_from_previous.is_zero() {
                        tokio::time::sleep(point.delay_from_previous).await;
                    }
                    self.set_location(&point.latitude, &point.longitude).await?;
                }
                Ok(route.len())
            }
        }
    }
}

#[derive(clap::Args)]
pub struct LocationCmd {
    #[command(subcommand)]
    sub: LocationSub,
}

#[derive(clap::Subcommand)]
enum LocationSub {
    /// Set a custom simulated location
    Set {
        #[arg(allow_hyphen_values = true)]
        latitude: String,
        #[arg(allow_hyphen_values = true)]
        longitude: String,
    },
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
                parse_latitude(&latitude)?;
                parse_longitude(&longitude)?;

                let (_device, mut client) = connect_location(&udid).await?;
                client
                    .set_location(&latitude, &longitude)
                    .await
                    .map_err(|e| e.context("simlocation set failed"))?;
                crate::output::outln!("Location simulation set to {latitude}, {longitude}");
            }
            LocationSub::Gpx { file } => {
                let gpx = fs::read_to_string(&file)
                    .await
                    .map_err(|e| anyhow::Error::new(e).context("failed to read GPX file"))?;

                let (_device, mut client) = connect_location(&udid).await?;
                let count = tokio::select! {
                    result = client.replay_gpx_route(&gpx) => result,
                    signal = tokio::signal::ctrl_c() => {
                        signal?;
                        return Err(std::io::Error::new(std::io::ErrorKind::Interrupted, "GPX replay cancelled; previously sent locations may remain active").into());
                    }
                }
                    .map_err(|e| e.context("simlocation GPX replay failed"))?;
                crate::output::outln!("Replayed {count} GPX point(s) from {}", file.display());
            }
            LocationSub::Reset => {
                let (_device, mut client) = connect_location(&udid).await?;
                client
                    .reset_location()
                    .await
                    .map_err(|e| e.context("simlocation reset failed"))?;
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

    #[test]
    fn selects_dvt_from_ios_17() {
        assert!(!uses_dvt(16));
        assert!(uses_dvt(17));
        assert!(uses_dvt(18));
    }

    #[tokio::test(start_paused = true)]
    async fn modern_gpx_replays_ordered_points_with_route_delay() {
        use ios_core::dtx::{encode_dtx, read_dtx_frame, DtxPayload, NSObject};
        use tokio::io::AsyncWriteExt;

        let (stream, mut peer) = tokio::io::duplex(8192);
        let device = tokio::spawn(async move {
            let channel = read_dtx_frame(&mut peer).await.unwrap();
            peer.write_all(&encode_dtx(
                channel.identifier,
                1,
                channel.channel_code,
                false,
                3,
                &[],
                &[],
            ))
            .await
            .unwrap();
            for (latitude, longitude) in [(1.0, 2.0), (3.0, 4.0)] {
                let request = read_dtx_frame(&mut peer).await.unwrap();
                assert!(
                    matches!(&request.payload, DtxPayload::MethodInvocation { selector, args }
                    if selector == "simulateLocationWithLatitude:longitude:"
                    && args.as_slice() == [NSObject::Double(latitude), NSObject::Double(longitude)].as_slice())
                );
                peer.write_all(&encode_dtx(
                    request.identifier,
                    1,
                    request.channel_code,
                    false,
                    3,
                    &[],
                    &[],
                ))
                .await
                .unwrap();
            }
        });
        let stream: ios_core::device::ServiceStream = Box::new(stream);
        let mut client = LocationClient::Modern(
            ios_core::instruments::LocationSimulationClient::connect(stream)
                .await
                .unwrap(),
        );
        let route = r#"<gpx><trk><trkseg><trkpt lat="1" lon="2"><time>2026-10-26T00:00:00Z</time></trkpt><trkpt lat="3" lon="4"><time>2026-10-26T00:00:02Z</time></trkpt></trkseg></trk></gpx>"#;
        let start = tokio::time::Instant::now();
        assert_eq!(client.replay_gpx_route(route).await.unwrap(), 2);
        assert_eq!(start.elapsed(), std::time::Duration::from_secs(2));
        device.await.unwrap();
    }

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
    fn parses_negative_set_coordinates_without_double_dash() {
        let cmd = TestCli::parse_from(["location", "set", "37.7749", "-122.4194"]);
        match cmd.command {
            LocationSub::Set {
                latitude,
                longitude,
            } => {
                assert_eq!(latitude, "37.7749");
                assert_eq!(longitude, "-122.4194");
            }
            LocationSub::Gpx { .. } | LocationSub::Reset => panic!("expected set subcommand"),
        }
    }

    #[tokio::test]
    async fn rejects_out_of_range_and_non_finite_coordinates_before_connection() {
        for (latitude, longitude) in [
            ("999", "0"),
            ("-90.001", "0"),
            ("0", "180.001"),
            ("0", "-180.001"),
            ("NaN", "0"),
            ("0", "inf"),
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
