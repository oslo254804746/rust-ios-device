use std::time::Duration;

use anyhow::{Context, Result};
use ios_core::{ConnectedDevice, ServiceStream};

use crate::cmd::connect::connect_by_ios_major;

#[derive(clap::Args)]
pub struct CompanionCmd {
    #[command(subcommand)]
    sub: CompanionSub,
}

#[derive(clap::Subcommand)]
enum CompanionSub {
    /// List paired companion devices
    List,
    /// Query a registry value for a paired companion device
    Get { udid: String, key: String },
    /// Stream companion-device registry events until Ctrl+C or --timeout
    Listen {
        /// Stop after this many seconds, including connection and reads
        #[arg(long, value_name = "SECONDS")]
        timeout: Option<u64>,
    },
    /// Start forwarding a companion-device port until Ctrl+C or --timeout
    ///
    /// The returned service port is reachable through the device connection;
    /// this command does not create a host TCP listener.
    Forward {
        /// Port on the companion device to forward (1-65535)
        #[arg(value_name = "REMOTE_PORT")]
        remote_port: u16,
        /// Optional companion service name
        #[arg(long, value_name = "SERVICE_NAME", alias = "service")]
        service_name: Option<String>,
        /// Stop after this many seconds, including connection and stop
        #[arg(long, value_name = "SECONDS")]
        timeout: Option<u64>,
    },
    /// Stop forwarding a companion-device port by its remote port
    ///
    /// CompanionProxy has no persistent host-side forwarding ID; use the
    /// remote port supplied to `forward`.
    Stop {
        /// Port on the companion device used to identify the forwarding
        #[arg(value_name = "REMOTE_PORT")]
        remote_port: u16,
        /// Bound the connection and stop request to this many seconds
        #[arg(long, value_name = "SECONDS")]
        timeout: Option<u64>,
    },
}

mod control;

impl CompanionCmd {
    pub(crate) fn preflight(&self) -> Result<()> {
        control::preflight(&self.sub)
    }

    pub async fn run(self, udid: Option<String>, json: bool) -> Result<()> {
        self.preflight()?;
        let deadline = operation_deadline(control::timeout(&self.sub))?;
        let mut signal = Box::pin(tokio::signal::ctrl_c());
        let (device, stream) = control::setup(
            control::SETUP_BUDGET,
            deadline,
            async {
                let udid = match udid {
                    Some(udid) => udid,
                    None => {
                        let devices = ios_core::list_devices()
                            .await
                            .context("failed to list connected iOS devices for default --udid")?;
                        crate::default_udid_from_devices(&devices)
                            .ok_or_else(|| anyhow::anyhow!("--udid required for companion"))?
                    }
                };
                connect_companion(&udid, deadline).await
            },
            signal.as_mut(),
        )
        .await?;
        let client = new_client(stream, remaining_timeout(deadline));
        let result = control::run(
            client,
            self.sub,
            deadline,
            json,
            |line| {
                crate::output::write_line(format_args!("{line}"))?;
                Ok(())
            },
            signal.as_mut(),
        )
        .await;
        drop(device);
        result
    }
}

async fn connect_companion(
    udid: &str,
    deadline: Option<tokio::time::Instant>,
) -> Result<(ConnectedDevice, ServiceStream)> {
    let operation = async {
        // pymobiledevice3 selects lockdown for classic providers and the RSD
        // shim for tunnel-backed providers. ProductVersion chooses the
        // connection strategy here; the RSD service is still resolved from
        // the device-advertised directory by `connect_rsd_service`.
        let (device, version) = connect_by_ios_major(udid, |major| major >= 17)
            .await
            .with_context(|| format!("failed to connect to device {udid}"))?;
        let (service_name, uses_rsd) = companion_route(version.major);
        let stream = if uses_rsd {
            device
                .connect_rsd_service(service_name)
                .await
                .context("failed to connect to RSD companion proxy shim")?
        } else {
            device
                .connect_service(service_name)
                .await
                .context("failed to connect to companion proxy service")?
        };
        Ok::<_, anyhow::Error>((device, stream))
    };
    if let Some(deadline) = deadline {
        tokio::time::timeout_at(deadline, operation)
            .await
            .map_err(|error| anyhow::Error::new(error).context("companion connection timed out"))?
    } else {
        operation.await
    }
}

fn companion_route(ios_major: u64) -> (&'static str, bool) {
    if ios_major >= 17 {
        (ios_core::companion::RSD_SERVICE_NAME, true)
    } else {
        (ios_core::companion::SERVICE_NAME, false)
    }
}

fn new_client(
    stream: ServiceStream,
    timeout: Option<Duration>,
) -> ios_core::companion::CompanionProxyClient<ServiceStream> {
    match timeout {
        Some(duration) => ios_core::companion::CompanionProxyClient::with_timeout(stream, duration),
        None => ios_core::companion::CompanionProxyClient::new(stream),
    }
}

fn operation_deadline(timeout: Option<u64>) -> Result<Option<tokio::time::Instant>> {
    timeout
        .map(|seconds| {
            let duration = seconds_duration(seconds)?;
            tokio::time::Instant::now()
                .checked_add(duration)
                .ok_or_else(|| crate::output::input_decision("--timeout is too large"))
        })
        .transpose()
}

fn remaining_timeout(deadline: Option<tokio::time::Instant>) -> Option<Duration> {
    deadline.map(|deadline| deadline.saturating_duration_since(tokio::time::Instant::now()))
}

fn seconds_duration(seconds: u64) -> Result<Duration> {
    if seconds == 0 {
        return Err(crate::output::input_decision(
            "--timeout must be greater than zero",
        ));
    }
    let duration = Duration::from_secs(seconds);
    tokio::time::Instant::now()
        .checked_add(duration)
        .map(|_| duration)
        .ok_or_else(|| crate::output::input_decision("--timeout is too large"))
}

#[cfg(test)]
mod tests {
    use clap::Parser;

    use super::*;

    #[tokio::test]
    async fn stream_contract_companion_issued_timeout_keeps_actual_elapsed() {
        let error = control::drive(
            std::future::pending::<Result<()>>(),
            std::future::pending(),
            Some(tokio::time::Instant::now()),
            || Ok(()),
        )
        .await
        .unwrap_err();
        assert_eq!(
            ios_core::classify_error(error.as_ref()),
            ios_core::ErrorClass::Timeout
        );
        assert!(error
            .chain()
            .any(|source| source.is::<tokio::time::error::Elapsed>()));
    }

    #[derive(Parser)]
    struct TestCli {
        #[command(subcommand)]
        command: CompanionSub,
    }

    #[test]
    fn parses_companion_list_subcommand() {
        let parsed = TestCli::try_parse_from(["companion", "list"]);
        assert!(parsed.is_ok(), "companion list command should parse");
    }

    #[test]
    fn parses_companion_get_subcommand() {
        let parsed = TestCli::try_parse_from(["companion", "get", "watch-udid", "name"]);
        assert!(parsed.is_ok(), "companion get command should parse");
    }

    #[test]
    fn parses_listen_forward_and_stop_subcommands() {
        assert!(TestCli::try_parse_from(["companion", "listen", "--timeout", "3"]).is_ok());
        assert!(TestCli::try_parse_from([
            "companion",
            "forward",
            "65535",
            "--service-name",
            "com.example.watch",
            "--timeout",
            "4",
        ])
        .is_ok());
        assert!(TestCli::try_parse_from(["companion", "stop", "1"]).is_ok());
        assert!(TestCli::try_parse_from(["companion", "forward", "65536"]).is_err());
    }

    #[test]
    fn timeout_deadline_overflow_is_rejected() {
        for error in [
            seconds_duration(u64::MAX).unwrap_err(),
            operation_deadline(Some(u64::MAX)).unwrap_err(),
        ] {
            assert_eq!(
                ios_core::classify_error(error.as_ref()),
                ios_core::ErrorClass::InvalidInput
            );
            assert_eq!(error.to_string(), "--timeout is too large");
        }
        assert!(seconds_duration(0).is_err());
        assert!(operation_deadline(None).unwrap().is_none());
        assert!(operation_deadline(Some(0)).is_err());
    }

    #[test]
    fn remaining_timeout_never_extends_absolute_deadline() {
        let deadline = tokio::time::Instant::now()
            .checked_add(Duration::from_secs(1))
            .unwrap();
        let remaining = remaining_timeout(Some(deadline)).unwrap();
        assert!(remaining <= Duration::from_secs(1));
    }

    #[test]
    fn companion_route_matches_classic_and_rsd_providers() {
        assert_eq!(
            companion_route(16),
            (ios_core::companion::SERVICE_NAME, false)
        );
        assert_eq!(
            companion_route(17),
            (ios_core::companion::RSD_SERVICE_NAME, true)
        );
        assert_eq!(
            companion_route(18),
            (ios_core::companion::RSD_SERVICE_NAME, true)
        );
    }
}
