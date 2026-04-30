use anyhow::{Context, Result};
use ios_core::device::ServiceStream;
use ios_core::MuxClient;
use tokio::io::{self, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};

#[derive(clap::Args)]
pub struct ForwardCmd {
    #[arg(help = "Local port to listen on")]
    pub host_port: u16,
    #[arg(help = "Device port to forward to")]
    pub device_port: u16,
    #[arg(short = 'H', long, default_value = "127.0.0.1")]
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

#[derive(Debug, Clone)]
struct ForwardConnector {
    device_id: u32,
    tunnel_proxy: Option<TunnelProxyTarget>,
}

pub async fn run_port_forward(
    udid: &str,
    host_port: u16,
    device_port: u16,
    host: String,
    once: bool,
) -> Result<()> {
    let connector = resolve_forward_connector(udid).await?;
    let listen_addr = format!("{}:{}", host, host_port);
    let listener = TcpListener::bind(&listen_addr).await?;
    eprintln!("{}", connector.describe(&host, host_port, device_port));
    if once {
        eprintln!("Waiting for one connection...");
    } else {
        eprintln!("Press Ctrl+C to stop.");
    }

    loop {
        let (client, peer) = listener.accept().await?;
        tracing::debug!("forward: connection from {peer}");

        let connector = connector.clone();
        let forward_task = tokio::spawn(async move {
            match forward_client_connection(connector, device_port, client).await {
                Ok(()) => Ok::<(), anyhow::Error>(()),
                Err(err) => {
                    tracing::warn!("forward: connection from {peer} failed: {err}");
                    Err(err)
                }
            }
        });

        if once {
            forward_task.await??;
            break;
        }
    }

    Ok(())
}

async fn forward_client_connection(
    connector: ForwardConnector,
    device_port: u16,
    client: TcpStream,
) -> Result<()> {
    let mut remote = connector
        .connect(device_port)
        .await
        .with_context(|| format!("failed to connect device port {device_port}"))?;
    let mut client = client;
    io::copy_bidirectional(&mut client, &mut remote).await?;
    Ok(())
}

async fn resolve_forward_connector(udid: &str) -> Result<ForwardConnector> {
    let device_id = lookup_device_id(udid).await?;
    let opts = ios_core::device::ConnectOptions {
        tun_mode: ios_core::TunMode::Userspace,
        pair_record_path: None,
        skip_tunnel: false,
    };

    let tunnel_proxy = match ios_core::connect(udid, opts).await {
        Ok(device) => match tunnel_proxy_target(device.server_address(), device.userspace_port()) {
            Some(target) => Some(target),
            None => {
                tracing::info!(
                    "forward: userspace tunnel metadata unavailable for {udid}, falling back to usbmux"
                );
                None
            }
        },
        Err(err) => {
            tracing::info!("forward: tunnel unavailable for {udid}: {err}; falling back to usbmux");
            None
        }
    };

    Ok(ForwardConnector {
        device_id,
        tunnel_proxy,
    })
}

async fn lookup_device_id(udid: &str) -> Result<u32> {
    let mut mux = MuxClient::connect().await?;
    let devices = mux.list_devices().await?;
    devices
        .into_iter()
        .find(|device| device.serial_number == udid)
        .map(|device| device.device_id)
        .ok_or_else(|| anyhow::anyhow!("device not found: {udid}"))
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
                "Forwarding {host}:{host_port} → [{}]:{device_port}  (prefers proxy 127.0.0.1:{}; usbmux fallback enabled)",
                target.server_addr, target.proxy_port
            ),
            None => format!(
                "Forwarding {host}:{host_port} → device:{device_port}  (via usbmux direct)"
            ),
        }
    }

    async fn connect(&self, device_port: u16) -> Result<ServiceStream> {
        if let Some(target) = &self.tunnel_proxy {
            match connect_via_tunnel_proxy(target, device_port).await {
                Ok(stream) => return Ok(stream),
                Err(err) => {
                    tracing::warn!(
                        "forward: tunnel proxy connect to [{}]:{} via 127.0.0.1:{} failed: {}; falling back to usbmux",
                        target.server_addr,
                        device_port,
                        target.proxy_port,
                        err
                    );
                }
            }
        }

        connect_via_usbmux(self.device_id, device_port).await
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
    proxy.write_all(&(device_port as u32).to_le_bytes()).await?;
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
        .map_err(|e| anyhow::anyhow!("invalid IPv6 address '{addr}': {e}"))?;
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
}
