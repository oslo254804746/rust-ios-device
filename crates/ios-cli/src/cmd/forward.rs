use crate::cmd::tunnel::{NativeConnection, NativeScope};
use anyhow::{Context, Result};
use ios_core::device::ServiceStream;
use ios_core::MuxClient;
use std::{sync::Arc, time::Duration};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};

mod lifecycle;

#[derive(clap::Args)]
pub struct ForwardCmd {
    #[arg(help = "Local port to listen on")]
    pub host_port: u16,
    #[arg(help = "Device port to forward to")]
    pub device_port: u16,
    #[arg(
        short = 'H',
        long,
        default_value = "127.0.0.1",
        help = "Literal loopback IP or localhost; raw TCP clients use host trust"
    )]
    pub host: String,
    #[arg(long, help = "Accept a single client connection, then exit")]
    pub once: bool,
}

impl ForwardCmd {
    pub async fn run(self, udid: Option<String>) -> Result<()> {
        let udid = udid.ok_or_else(|| anyhow::anyhow!("--udid required for forward"))?;
        run_port_forward(
            &udid,
            self.host_port,
            self.device_port,
            self.host,
            self.once,
        )
        .await
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct TunnelProxyTarget {
    server_addr: String,
    proxy_port: u16,
}

#[derive(Clone)]
struct ForwardConnector {
    device_id: u32,
    tunnel_proxy: Option<TunnelProxyTarget>,
    _native: Option<Arc<NativeConnection>>,
    setup_failure: Option<Arc<anyhow::Error>>,
    #[cfg(test)]
    direct_test_target: Option<std::net::SocketAddr>,
}

pub async fn run_port_forward(
    udid: &str,
    host_port: u16,
    device_port: u16,
    host: String,
    once: bool,
) -> Result<()> {
    let scope = NativeScope::new()?;
    let primary = Box::pin(run_port_forward_scoped(
        udid,
        host_port,
        device_port,
        &host,
        once,
        &scope,
    ))
    .await;
    let cleanup = scope.close(Duration::from_secs(10)).await;
    crate::cmd::test_workflow::combine_results(
        primary,
        cleanup,
        "forward original native cleanup failed",
    )
}

/// Borrows the caller's original native registry. Only this forward's clients
/// and connector lease are retired; the caller still owns its WDA/test sessions.
pub(crate) async fn run_port_forward_scoped(
    udid: &str,
    host_port: u16,
    device_port: u16,
    host: &str,
    once: bool,
    scope: &NativeScope,
) -> Result<()> {
    validate_forward_ports(host_port, device_port)?;
    let address = crate::access::loopback_address(host)?;
    let mut cancellation = Box::pin(tokio::signal::ctrl_c());
    let setup = Box::pin(async {
        let connector = Box::pin(resolve_forward_connector_scoped(udid, scope)).await?;
        let listener = TcpListener::bind((address, host_port)).await?;
        crate::output::write_error_line(format_args!(
            "{}",
            connector.describe(host, host_port, device_port)
        ))?;
        crate::output::write_error_line(format_args!(
            "{}",
            if once {
                "Waiting for one connection..."
            } else {
                "Press Ctrl+C to stop."
            }
        ))?;
        Ok::<_, anyhow::Error>((connector, listener))
    });
    let ready = tokio::select! {
        result = tokio::time::timeout(Duration::from_secs(30), setup) => result.context("forward setup total deadline expired").and_then(|result| result),
        signal = &mut cancellation => match signal {
            Ok(()) => Err(anyhow::Error::new(ios_core::OperationCancelled::new())),
            Err(error) => Err(anyhow::Error::new(error).context("waiting for forward setup cancellation")),
        }
    };
    let primary = match ready {
        Ok((connector, listener)) => {
            lifecycle::serve(listener, connector, device_port, once, &mut cancellation).await
        }
        Err(error) => Err(error),
    };
    let cleanup = scope.collect(Duration::from_secs(10)).await;
    crate::cmd::test_workflow::combine_results(
        primary,
        cleanup,
        "forward original retired native joins failed",
    )
}

pub(crate) fn validate_forward_ports(host_port: u16, device_port: u16) -> Result<()> {
    if host_port == 0 {
        return Err(crate::output::input_decision(
            "invalid host port: port must be at least 1",
        ));
    }
    if device_port == 0 {
        return Err(crate::output::input_decision(
            "invalid device port: port must be at least 1",
        ));
    }
    Ok(())
}

struct ForwardFallbackFailure {
    primary: Arc<anyhow::Error>,
    fallback: anyhow::Error,
}
impl std::fmt::Display for ForwardFallbackFailure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("forward preferred route and fallback both failed")
    }
}
impl std::fmt::Debug for ForwardFallbackFailure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ForwardFallbackFailure")
            .field(
                "primary_class",
                &ios_core::classify_error(self.primary.as_ref().as_ref()),
            )
            .field(
                "fallback_class",
                &ios_core::classify_error(self.fallback.as_ref()),
            )
            .finish_non_exhaustive()
    }
}
impl std::error::Error for ForwardFallbackFailure {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        Some(self.fallback.as_ref())
    }
}

async fn resolve_forward_connector_scoped(
    udid: &str,
    scope: &NativeScope,
) -> Result<ForwardConnector> {
    let mut device_id = lookup_device_id(udid).await?;
    let mut native = None;
    let mut setup_failure = None;
    let tunnel_proxy = match Box::pin(scope.connect(udid, ios_core::TunMode::Userspace)).await {
        Ok(device) => {
            device_id = device.info.device_id;
            let target = tunnel_proxy_target(device.server_address(), device.userspace_port());
            if target.is_none() {
                tracing::info!(
                    device = %ios_core::redaction::diagnostic_id(udid),
                    "forward: userspace tunnel metadata unavailable, falling back to usbmux"
                );
            }
            native = Some(Arc::new(device));
            target
        }
        Err(err) => {
            tracing::info!(device = %ios_core::redaction::diagnostic_id(udid), error = %crate::output::error_message(&err),
                "forward: tunnel unavailable; falling back to usbmux");
            setup_failure = Some(Arc::new(err));
            None
        }
    };

    Ok(ForwardConnector {
        device_id,
        tunnel_proxy,
        _native: native,
        setup_failure,
        #[cfg(test)]
        direct_test_target: None,
    })
}

async fn lookup_device_id(udid: &str) -> Result<u32> {
    let mut mux = MuxClient::connect().await?;
    let devices = mux.list_devices().await?;
    devices
        .into_iter()
        .find(|device| device.serial_number == udid)
        .map(|device| device.device_id)
        .ok_or_else(|| anyhow::Error::new(ios_core::CoreError::DeviceNotFound(udid.into())))
}

fn tunnel_proxy_target(
    server_addr: Option<&str>,
    userspace_port: Option<u16>,
) -> Option<TunnelProxyTarget> {
    match (server_addr, userspace_port) {
        (Some(server_addr), Some(proxy_port)) => Some(TunnelProxyTarget {
            server_addr: server_addr.to_string(),
            proxy_port,
        }),
        _ => None,
    }
}

impl ForwardConnector {
    fn describe(&self, host: &str, host_port: u16, device_port: u16) -> String {
        match &self.tunnel_proxy {
            Some(target) => format!(
                "Forwarding {host}:{host_port} → [{}]:{device_port}  (prefers usbmux direct; proxy 127.0.0.1:{} fallback)",
                ios_core::redaction::diagnostic_id(&target.server_addr), target.proxy_port
            ),
            None => format!(
                "Forwarding {host}:{host_port} → device:{device_port}  (via usbmux direct)"
            ),
        }
    }

    async fn connect(&self, device_port: u16) -> Result<ServiceStream> {
        // A usbmux Connect reply confirms the device port was opened. The
        // legacy userspace proxy preamble has no acknowledgement: a successful
        // local TCP dial cannot establish that a legacy port exists on the
        // tunnel address. Select the direct route before copying any data.
        #[cfg(test)]
        let direct = if let Some(address) = self.direct_test_target {
            TcpStream::connect(address)
                .await
                .map(|stream| Box::new(stream) as ServiceStream)
                .map_err(anyhow::Error::new)
        } else {
            connect_via_usbmux(self.device_id, device_port).await
        };
        #[cfg(not(test))]
        let direct = connect_via_usbmux(self.device_id, device_port).await;
        let primary = match direct {
            Ok(stream) => return Ok(stream),
            Err(error) => error,
        };
        let fallback = match &self.tunnel_proxy {
            Some(target) => connect_via_tunnel_proxy(target, device_port).await,
            None => {
                return Err(match &self.setup_failure {
                    Some(error) => anyhow::Error::new(ForwardFallbackFailure {
                        primary: error.clone(),
                        fallback: primary,
                    }),
                    None => primary,
                })
            }
        };
        match fallback {
            Ok(stream) => Ok(stream),
            Err(fallback) => Err(anyhow::Error::new(ForwardFallbackFailure {
                primary: Arc::new(primary),
                fallback,
            })),
        }
    }
}

async fn connect_via_tunnel_proxy(
    target: &TunnelProxyTarget,
    device_port: u16,
) -> Result<ServiceStream> {
    let proxy_addr = format!("127.0.0.1:{}", target.proxy_port);
    let mut proxy = TcpStream::connect(&proxy_addr).await?;
    let addr_bytes = parse_ipv6_bytes(&target.server_addr)?;
    proxy.write_all(&addr_bytes).await?;
    let port = device_port as u32 | ios_core::USERSPACE_PROXY_CONNECT_ACK;
    proxy.write_all(&port.to_le_bytes()).await?;
    proxy.flush().await?;
    match proxy.read_u8().await? {
        0 => {}
        1 => {
            return Err(std::io::Error::new(
                std::io::ErrorKind::ConnectionAborted,
                "userspace remote connect aborted before establishment",
            )
            .into())
        }
        _ => {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "invalid userspace connect acknowledgement",
            )
            .into())
        }
    }
    Ok(Box::new(proxy))
}

async fn connect_via_usbmux(device_id: u32, device_port: u16) -> Result<ServiceStream> {
    let stream = MuxClient::connect()
        .await?
        .connect_to_port(device_id, device_port)
        .await?;
    Ok(Box::new(stream))
}

fn parse_ipv6_bytes(addr: &str) -> Result<[u8; 16]> {
    let addr: std::net::Ipv6Addr = addr
        .parse()
        .map_err(|e| crate::output::input_error("invalid IPv6 address", e))?;
    Ok(addr.octets())
}

#[cfg(test)]
mod tests {
    use clap::Parser;

    use super::*;

    #[derive(Parser)]
    struct TestCli {
        #[command(flatten)]
        command: ForwardCmd,
    }

    #[tokio::test]
    async fn nonloopback_forward_fails_before_device_resolution() {
        for host in ["0.0.0.0", "::", "192.0.2.1", "example.invalid"] {
            let error = run_port_forward("synthetic-device", 1234, 62078, host.into(), true)
                .await
                .unwrap_err();
            assert!(error
                .chain()
                .any(|cause| cause.to_string().contains("loopback")));
        }
    }

    #[test]
    fn parses_forward_once_flag() {
        let cmd = TestCli::parse_from(["forward", "1234", "62078", "--once"]);
        assert_eq!(cmd.command.host_port, 1234);
        assert_eq!(cmd.command.device_port, 62078);
        assert_eq!(cmd.command.host, "127.0.0.1");
        assert!(cmd.command.once);
    }

    #[test]
    fn builds_tunnel_proxy_target_when_metadata_is_complete() {
        assert_eq!(
            tunnel_proxy_target(Some("fd00::1"), Some(60105)),
            Some(TunnelProxyTarget {
                server_addr: "fd00::1".into(),
                proxy_port: 60105,
            })
        );
    }

    #[test]
    fn skips_tunnel_proxy_target_when_metadata_is_missing() {
        assert_eq!(tunnel_proxy_target(Some("fd00::1"), None), None);
        assert_eq!(tunnel_proxy_target(None, Some(60105)), None);
    }

    #[test]
    fn validate_forward_ports_rejects_zero_values() {
        assert!(validate_forward_ports(1, 1).is_ok());
        assert!(validate_forward_ports(0, 1).is_err());
        assert!(validate_forward_ports(1, 0).is_err());
    }
}
