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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum WaitResult {
    Cancelled,
    TimedOut,
}

impl CompanionCmd {
    pub async fn run(self, udid: Option<String>, json: bool) -> Result<()> {
        let udid = udid.ok_or_else(|| anyhow::anyhow!("--udid required for companion"))?;
        match self.sub {
            CompanionSub::List => {
                let (_device, stream) = connect_companion(&udid, None).await?;
                let mut client = ios_core::companion::CompanionProxyClient::new(stream);
                let devices = client.list().await?;
                if json {
                    println!("{}", serde_json::to_string_pretty(&devices)?);
                } else {
                    for device in devices {
                        println!("{}", serde_json::to_string_pretty(&device)?);
                    }
                }
            }
            CompanionSub::Get {
                udid: companion_udid,
                key,
            } => {
                let (_device, stream) = connect_companion(&udid, None).await?;
                let mut client = ios_core::companion::CompanionProxyClient::new(stream);
                let value = client.get_value(&companion_udid, &key).await?;
                if json {
                    println!("{}", serde_json::to_string_pretty(&value)?);
                } else if let Some(string) = value.as_string() {
                    println!("{string}");
                } else {
                    println!("{}", serde_json::to_string_pretty(&value)?);
                }
            }
            CompanionSub::Listen { timeout } => {
                let deadline = operation_deadline(timeout)?;
                let (device, stream) = connect_companion(&udid, deadline).await?;
                let client = new_client(stream, remaining_timeout(deadline));
                let mut events = client.listen_for_devices().await?;
                let termination = wait_for_events(&mut events, deadline, json).await?;
                drop(events);
                drop(device);
                finish_wait("companion listener", termination, timeout)?;
            }
            CompanionSub::Forward {
                remote_port,
                service_name,
                timeout,
            } => {
                let deadline = operation_deadline(timeout)?;
                let (device, stream) = connect_companion(&udid, deadline).await?;
                let mut client = new_client(stream, remaining_timeout(deadline));
                let mut forwarding = client
                    .start_forwarding_service_port_handle(
                        remote_port,
                        service_name.as_deref(),
                        None,
                    )
                    .await?;
                if json {
                    println!(
                        "{}",
                        serde_json::json!({
                            "GizmoRemotePortNumber": forwarding.remote_port(),
                            "CompanionProxyServicePort": forwarding.service_port(),
                        })
                    );
                } else {
                    println!(
                        "Forwarded companion port {} through service port {}. Press Ctrl+C to stop.",
                        forwarding.remote_port(),
                        forwarding.service_port()
                    );
                }
                let termination = wait_for_stop(deadline).await?;
                let stop_result: Result<(), ios_core::companion::CompanionError> =
                    if termination == WaitResult::TimedOut {
                        // There is no budget left for a protocol stop after an
                        // absolute deadline. Dropping still schedules the
                        // bounded best-effort cleanup when a runtime is alive.
                        Ok(())
                    } else {
                        let stop_timeout = remaining_timeout(deadline).unwrap_or_else(|| {
                            // The handle's configured timeout is the
                            // library default when this command has no
                            // absolute deadline.
                            forwarding.timeout()
                        });
                        forwarding.stop_with_timeout(stop_timeout).await.map(|_| ())
                    };
                drop(forwarding);
                drop(client);
                drop(device);
                stop_result.with_context(|| "failed to stop companion forwarding")?;
                finish_wait("companion forwarding", termination, timeout)?;
            }
            CompanionSub::Stop {
                remote_port,
                timeout,
            } => {
                let deadline = operation_deadline(timeout)?;
                let (device, stream) = connect_companion(&udid, deadline).await?;
                let mut client = new_client(stream, remaining_timeout(deadline));
                let response = client.stop_forwarding_service_port(remote_port).await?;
                drop(client);
                drop(device);
                if json {
                    println!(
                        "{}",
                        serde_json::to_string(&plist::Value::Dictionary(response))?
                    );
                } else if let Some(command) =
                    response.get("Command").and_then(plist::Value::as_string)
                {
                    println!("{command}");
                } else {
                    println!(
                        "{}",
                        serde_json::to_string_pretty(&plist::Value::Dictionary(response))?
                    );
                }
            }
        }

        Ok(())
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
            .map_err(|_| anyhow::anyhow!("companion connection timed out"))?
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
                .ok_or_else(|| anyhow::anyhow!("--timeout is too large"))
        })
        .transpose()
}

fn remaining_timeout(deadline: Option<tokio::time::Instant>) -> Option<Duration> {
    deadline.map(|deadline| deadline.saturating_duration_since(tokio::time::Instant::now()))
}

async fn wait_for_events<S>(
    events: &mut ios_core::companion::CompanionDeviceStream<S>,
    deadline: Option<tokio::time::Instant>,
    json: bool,
) -> Result<WaitResult>
where
    S: tokio::io::AsyncRead + Unpin + Send + 'static,
{
    let mut ctrl_c = Box::pin(tokio::signal::ctrl_c());
    loop {
        let event = if let Some(deadline) = deadline {
            tokio::select! {
                signal = ctrl_c.as_mut() => {
                    signal.context("failed to wait for Ctrl+C")?;
                    return Ok(WaitResult::Cancelled);
                }
                event = events.next_event() => event,
                _ = tokio::time::sleep_until(deadline) => return Ok(WaitResult::TimedOut),
            }
        } else {
            tokio::select! {
                signal = ctrl_c.as_mut() => {
                    signal.context("failed to wait for Ctrl+C")?;
                    return Ok(WaitResult::Cancelled);
                }
                event = events.next_event() => event,
            }
        };
        match event {
            Some(Ok(event)) => {
                let value = plist::Value::Dictionary(event);
                if json {
                    println!("{}", serde_json::to_string(&value)?);
                } else {
                    println!("{}", serde_json::to_string_pretty(&value)?);
                }
            }
            Some(Err(error)) => return Err(error.into()),
            None => return Err(anyhow::anyhow!("companion listener closed unexpectedly")),
        }
    }
}

async fn wait_for_stop(deadline: Option<tokio::time::Instant>) -> Result<WaitResult> {
    let mut ctrl_c = Box::pin(tokio::signal::ctrl_c());
    if let Some(deadline) = deadline {
        Ok(tokio::select! {
            signal = ctrl_c.as_mut() => {
                signal.context("failed to wait for Ctrl+C")?;
                WaitResult::Cancelled
            }
            _ = tokio::time::sleep_until(deadline) => WaitResult::TimedOut,
        })
    } else {
        ctrl_c.await.context("failed to wait for Ctrl+C")?;
        Ok(WaitResult::Cancelled)
    }
}

fn finish_wait(operation: &str, result: WaitResult, timeout: Option<u64>) -> Result<()> {
    match result {
        WaitResult::Cancelled => Ok(()),
        WaitResult::TimedOut => match timeout {
            Some(seconds) => Err(anyhow::anyhow!(
                "{operation} timed out after {seconds} seconds"
            )),
            None => Err(anyhow::anyhow!("{operation} timed out")),
        },
    }
}

fn seconds_duration(seconds: u64) -> Result<Duration> {
    let duration = Duration::from_secs(seconds);
    tokio::time::Instant::now()
        .checked_add(duration)
        .map(|_| duration)
        .ok_or_else(|| anyhow::anyhow!("--timeout is too large"))
}

#[cfg(test)]
mod tests {
    use clap::Parser;

    use super::*;

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
        assert!(seconds_duration(u64::MAX).is_err());
        assert!(operation_deadline(Some(u64::MAX)).is_err());
        assert!(finish_wait("test", WaitResult::TimedOut, Some(2)).is_err());
        assert!(finish_wait("test", WaitResult::Cancelled, Some(2)).is_ok());
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
