use anyhow::{Context, Result};
use ios_core::{ConnectOptions, ConnectedDevice, TunMode};

pub fn userspace_options(skip_tunnel: bool) -> ConnectOptions {
    ConnectOptions {
        tun_mode: TunMode::Userspace,
        pair_record_path: None,
        skip_tunnel,
    }
}

pub async fn connect_lockdown_only(udid: &str) -> Result<ConnectedDevice> {
    ios_core::connect(udid, userspace_options(true))
        .await
        .with_context(|| format!("failed to connect {udid} through lockdown"))
}

pub async fn connect_userspace_tunnel(udid: &str) -> Result<ConnectedDevice> {
    ios_core::connect(udid, userspace_options(false))
        .await
        .with_context(|| format!("failed to establish userspace tunnel for {udid}"))
}

pub async fn probe_product_version(udid: &str) -> Result<semver::Version> {
    let device = connect_lockdown_only(udid)
        .await
        .context("failed to connect device for product version probe")?;
    device.product_version().await.map_err(Into::into)
}

pub async fn connect_by_ios_major(
    udid: &str,
    requires_tunnel: impl FnOnce(u64) -> bool,
) -> Result<(ConnectedDevice, semver::Version)> {
    let version = probe_product_version(udid).await?;
    let device = if requires_tunnel(version.major) {
        connect_userspace_tunnel(udid).await?
    } else {
        connect_lockdown_only(udid).await?
    };
    Ok((device, version))
}
