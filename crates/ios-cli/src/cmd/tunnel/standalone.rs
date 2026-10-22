//! Complete foreground CLI start future, prepared before any Source work.
use std::{future::Future, io::Write, sync::Arc, time::Duration};

use anyhow::Result;
use ios_core::{ConnectOptions, ConnectedDevice, CoreError, TunnelHandle, TunnelOwner};

use super::{future_storage::PaidFuture, lifecycle, ownership, retirement, server};

pub(super) trait Device: Send {
    fn tunnel_handle(&self) -> Option<&Arc<TunnelHandle>>;
    fn server_address(&self) -> Option<&str>;
    fn rsd_port(&self) -> Option<u16>;
    fn userspace_port(&self) -> Option<u16>;
}
impl Device for ConnectedDevice {
    fn tunnel_handle(&self) -> Option<&Arc<TunnelHandle>> {
        ConnectedDevice::tunnel_handle(self)
    }
    fn server_address(&self) -> Option<&str> {
        ConnectedDevice::server_address(self)
    }
    fn rsd_port(&self) -> Option<u16> {
        ConnectedDevice::rsd_port(self)
    }
    fn userspace_port(&self) -> Option<u16> {
        ConnectedDevice::userspace_port(self)
    }
}

pub(super) trait Connector: Send + Sync {
    type Device: Device;

    fn connect<'a>(
        &'a self,
        udid: &'a str,
        options: ConnectOptions,
        owner: &'a dyn TunnelOwner,
    ) -> impl Future<Output = std::result::Result<Self::Device, CoreError>> + Send + 'a;
}

pub(super) struct NativeConnector;
impl Connector for NativeConnector {
    type Device = ConnectedDevice;

    fn connect<'a>(
        &'a self,
        udid: &'a str,
        options: ConnectOptions,
        owner: &'a dyn TunnelOwner,
    ) -> impl Future<Output = std::result::Result<Self::Device, CoreError>> + Send + 'a {
        ios_core::connect_with_owner(udid, options, owner)
    }
}

pub(super) trait Reporter: Send {
    fn ready(
        &self,
        udid: &str,
        mode: &str,
        script_mode: bool,
        device: &impl Device,
        server_address: &str,
        rsd_port: u16,
    ) -> std::io::Result<()>;
}

pub(super) struct NativeReporter;
impl Reporter for NativeReporter {
    fn ready(
        &self,
        udid: &str,
        mode: &str,
        script_mode: bool,
        device: &impl Device,
        server_address: &str,
        rsd_port: u16,
    ) -> std::io::Result<()> {
        if script_mode {
            crate::output::outln!("{server_address} {rsd_port}");
        } else {
            crate::output::outln!("Identifier: {udid}");
            crate::output::outln!("Mode: {mode}");
            crate::output::outln!("Protocol: tcp");
            crate::output::outln!("RSD Address: {server_address}");
            crate::output::outln!("RSD Port: {rsd_port}");
            if let Some(userspace_port) = device.userspace_port() {
                crate::output::outln!("Userspace Proxy: 127.0.0.1:{userspace_port}");
            }
            crate::output::outln!("Use the following connection option:");
            crate::output::outln!("--rsd {server_address} {rsd_port}");
        }
        std::io::stdout().flush()
    }
}

pub(super) struct Request<'a> {
    pub(super) udid: &'a str,
    pub(super) opts: ConnectOptions,
    pub(super) mode: &'static str,
    pub(super) script_mode: bool,
}

pub(super) fn prepare<'a, C, R, S>(
    native: retirement::RegistryOwner,
    request: Request<'a>,
    connector: C,
    reporter: R,
    signal: S,
) -> Result<PaidFuture<impl Future<Output = Result<()>> + 'a>>
where
    C: Connector + 'a,
    R: Reporter + 'a,
    S: Future<Output = std::io::Result<()>> + 'a,
{
    let bank = native.byte_bank();
    PaidFuture::new(
        run_admitted(native, request, connector, reporter, signal),
        &bank,
    )
}

async fn run_admitted<C, R, S>(
    native: retirement::RegistryOwner,
    request: Request<'_>,
    connector: C,
    reporter: R,
    signal: S,
) -> Result<()>
where
    C: Connector,
    R: Reporter,
    S: Future<Output = std::io::Result<()>>,
{
    let Request {
        udid,
        opts,
        mode,
        script_mode,
    } = request;
    let _owner = StandaloneOwner(native.clone());
    let ownership = ownership::Ownership::standalone(native.clone())?;
    let primary = async {
        let connect_timeout = Duration::from_secs(30);
        let device =
            tokio::time::timeout(connect_timeout, connector.connect(udid, opts, &ownership))
                .await
                .map_err(|_| {
                    anyhow::anyhow!(
                        "tunnel connect timed out after {}s — the device may not support \
                 CoreDevice tunneling or the handshake is blocked",
                        connect_timeout.as_secs()
                    )
                })??;
        let _lease = ownership.take_completed(
            device
                .tunnel_handle()
                .ok_or_else(|| lifecycle::cancelled().context("native tunnel handle missing"))?,
        )?;
        let server_address = device
            .server_address()
            .ok_or_else(|| anyhow::anyhow!("tunnel started without a server address"))?;
        let rsd_port = device
            .rsd_port()
            .ok_or_else(|| anyhow::anyhow!("tunnel started without an RSD port"))?;

        reporter.ready(udid, mode, script_mode, &device, server_address, rsd_port)?;

        signal.await?;
        Ok::<(), anyhow::Error>(())
    }
    .await;
    let clock = retirement::CleanupClock::new(server::CLEANUP_TIMEOUT);
    native.close_and_stop_all();
    drop(ownership);
    let cleanup = match clock {
        Ok(clock) => server::drain_native_terminal(&native, clock).await,
        Err(error) => Err(error),
    };
    match (primary, cleanup) {
        (Ok(()), cleanup) => cleanup,
        (Err(primary), Ok(())) => Err(primary),
        (Err(primary), Err(cleanup)) => Err(server::CleanupFailure { primary, cleanup }.into()),
    }
}

struct StandaloneOwner(retirement::RegistryOwner);

impl Drop for StandaloneOwner {
    fn drop(&mut self) {
        self.0.close_and_stop_all();
    }
}

#[cfg(test)]
mod tests;
