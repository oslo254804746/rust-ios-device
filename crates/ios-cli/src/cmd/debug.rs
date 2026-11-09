use std::{future::Future, net::Ipv6Addr, path::PathBuf, time::Duration};

use anyhow::{Context, Result};
use ios_core::device::ConnectOptions;
use ios_core::TunMode;
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpListener, TcpStream},
};

use crate::cmd::tunnel::NativeScope;

const DEBUGPROXY_SERVICE: &str = "com.apple.internal.dt.remote.debugproxy";

#[derive(clap::Args)]
pub struct DebugCmd {
    #[arg(help = "Installed bundle identifier to debug")]
    pub bundle_id: String,
    #[arg(long, help = "Local .app path for LLDB target create")]
    pub local_app: Option<PathBuf>,
}

impl DebugCmd {
    pub async fn run(self, udid: Option<String>) -> Result<()> {
        let udid = udid.ok_or_else(|| anyhow::anyhow!("--udid required for debug"))?;
        let local_target = match self.local_app.as_ref() {
            Some(path) => path
                .to_str()
                .ok_or_else(|| anyhow::anyhow!("local app path is not valid Unicode"))?
                .to_owned(),
            None => "/path/to/local/application.app".to_string(),
        };
        // Validate before any device operations and never emit multiline LLDB commands.
        let local_arg = lldb_argument(&local_target)?;
        let scope = NativeScope::new()?;
        let primary = self.run_scoped(&udid, &local_arg, &scope).await;
        let cleanup = scope.close(Duration::from_secs(10)).await;
        crate::cmd::test_workflow::combine_results(
            primary,
            cleanup,
            "debug original native cleanup failed",
        )
    }

    async fn run_scoped(self, udid: &str, local_arg: &str, scope: &NativeScope) -> Result<()> {
        let operation = async {
            let (probe, version) = tokio::time::timeout(Duration::from_secs(30), async {
                let probe = ios_core::connect(
                    udid,
                    ConnectOptions {
                        tun_mode: TunMode::Userspace,
                        pair_record_path: None,
                        skip_tunnel: true,
                    },
                )
                .await
                .context("failed to probe device version for debugserver")?;
                let version = probe.product_version().await?;
                Ok::<_, anyhow::Error>((probe, version))
            })
            .await
            .context("device version probe deadline expired")??;

            if version.major < 17 {
                return run_legacy_debug(&probe, &version, &self.bundle_id, local_arg).await;
            }
            drop(probe);

            let device = scope
                .connect(udid, TunMode::Userspace)
                .await
                .context("failed to establish device tunnel for debugserver")?;
            let rsd = match device.rsd() {
                Some(rsd) => rsd,
                None => {
                    crate::output::write_error_line(format_args!("RSD directory unavailable; cannot determine whether remote debugproxy is present."))?;
                    return Err(crate::output::precondition_decision(
                        "debug RSD directory unavailable",
                    ));
                }
            };
            let port = match require_debugproxy_port(rsd.get_port(DEBUGPROXY_SERVICE)) {
                Ok(port) => port,
                Err(error) => {
                    crate::output::write_error_line(format_args!("The device RSD directory does not expose a usable remote debugproxy. A device with this service is required."))?;
                    return Err(error);
                }
            };
            let host: Ipv6Addr = device
                .server_address()
                .ok_or_else(|| anyhow::anyhow!("missing userspace tunnel server address"))?
                .parse()
                .context("invalid userspace tunnel server address")?;
            let proxy_port = device
                .userspace_port()
                .ok_or_else(|| anyhow::anyhow!("userspace tunnel proxy unavailable"))?;
            let remote_arg = lookup_installed_app_with_device(&device, &self.bundle_id).await?;
            // Wait for the original proxy's remote-connect acknowledgement before
            // exposing a listener or accepting any LLDB business bytes.
            let remote = tokio::time::timeout(
                Duration::from_secs(30),
                connect_proxy(proxy_port, host, port),
            )
            .await
            .context("debugproxy connection deadline expired")??;
            let listener = TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0)).await?;
            let address = listener.local_addr()?;
            print_lldb_commands(local_arg, &remote_arg, address)?;
            relay_one(listener, remote).await
        };
        until_cancelled(operation, tokio::signal::ctrl_c()).await
    }
}

async fn run_legacy_debug(
    device: &ios_core::ConnectedDevice,
    version: &ios_core::debugserver::Version,
    bundle_id: &str,
    local_arg: &str,
) -> Result<()> {
    let remote_arg = tokio::time::timeout(
        Duration::from_secs(30),
        lookup_installed_app_with_device(device, bundle_id),
    )
    .await
    .context("installed application lookup deadline expired")??;
    let remote = tokio::time::timeout(
        Duration::from_secs(30),
        crate::cmd::debugserver::connect_debugserver_with_device(device, version),
    )
    .await
    .context("debugserver connection deadline expired")??;
    let listener = TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0)).await?;
    let address = listener.local_addr()?;
    print_lldb_commands(local_arg, &remote_arg, address)?;
    relay_one(listener, remote).await
}

async fn lookup_installed_app_with_device(
    device: &ios_core::ConnectedDevice,
    bundle_id: &str,
) -> Result<String> {
    let app = crate::cmd::runtest::lookup_installed_app(device, bundle_id).await?;
    python_string(&app.path)
}

fn print_lldb_commands(
    local_arg: &str,
    remote_arg: &str,
    address: std::net::SocketAddr,
) -> Result<()> {
    crate::output::outln!("platform select remote-ios");
    crate::output::outln!("target create -- {local_arg}");
    crate::output::outln!(
        "script lldb.target.module[0].SetPlatformFileSpec(lldb.SBFileSpec({remote_arg}, False))"
    );
    crate::output::outln!("process connect connect://{address}");
    crate::output::outln!("process launch");
    crate::output::write_error_line(format_args!("Debug proxy listening on {address}; keep this command running while using LLDB. Ctrl+C stops it."))?;
    Ok(())
}

async fn until_cancelled<F, S>(operation: F, signal: S) -> Result<()>
where
    F: Future<Output = Result<()>>,
    S: Future<Output = std::io::Result<()>>,
{
    tokio::pin!(operation, signal);
    tokio::select! {
        result = operation => result,
        signal = signal => match signal {
            Ok(()) => Err(anyhow::Error::new(ios_core::OperationCancelled::new())),
            Err(error) => Err(anyhow::Error::new(error).context("waiting for debug cancellation")),
        },
    }
}

fn require_debugproxy_port(port: Option<u16>) -> Result<u16> {
    match port {
        Some(port) if port != 0 => Ok(port),
        _ => Err(crate::output::precondition_decision(
            "debugproxy missing or invalid in RSD directory",
        )),
    }
}

fn lldb_argument(value: &str) -> Result<String> {
    anyhow::ensure!(
        !value.chars().any(char::is_control),
        "local app path contains a control character"
    );
    Ok(format!(
        "\"{}\"",
        value.replace('\\', "\\\\").replace('"', "\\\"")
    ))
}

fn python_string(value: &str) -> Result<String> {
    anyhow::ensure!(
        !value.chars().any(|ch| ch.is_control() || ch == '\\'),
        "device app path contains a control character or backslash"
    );
    // LLDB preprocesses raw `script` commands for backtick expressions even
    // inside Python quotes. Keep backticks out of the command text entirely.
    Ok(serde_json::to_string(value)?.replace('`', "\\u0060"))
}

async fn connect_proxy(proxy_port: u16, host: Ipv6Addr, port: u16) -> Result<TcpStream> {
    let mut stream = TcpStream::connect((std::net::Ipv4Addr::LOCALHOST, proxy_port)).await?;
    stream.write_all(&host.octets()).await?;
    stream
        .write_all(&(u32::from(port) | ios_core::USERSPACE_PROXY_CONNECT_ACK).to_le_bytes())
        .await?;
    stream.flush().await?;
    match stream.read_u8().await? {
        0 => Ok(stream),
        1 => Err(std::io::Error::new(
            std::io::ErrorKind::ConnectionAborted,
            "userspace debugproxy remote connection failed",
        )
        .into()),
        _ => Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "invalid userspace debugproxy acknowledgement",
        )
        .into()),
    }
}

async fn relay_one<S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin>(
    listener: TcpListener,
    mut remote: S,
) -> Result<()> {
    let (mut local, _) = listener.accept().await?;
    drop(listener);
    tokio::io::copy_bidirectional(&mut local, &mut remote)
        .await
        .context("debugproxy transfer failed")?;
    Ok(())
}

#[cfg(test)]
mod tests;
