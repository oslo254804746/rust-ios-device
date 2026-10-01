//! Device/service connection strategy for WebInspector callers.
//! Returns the connection guard, supplied byte stream and selected route.

use anyhow::{Context, Result};
use ios_core::device::{ConnectOptions, ConnectedDevice, ServiceStream};
use ios_core::webinspector::{RSD_SERVICE_NAME, SERVICE_NAME};
use ios_core::TunMode;
use std::future::Future;
use std::pin::Pin;
use std::time::Duration;

/// A production connection retains its device/tunnel guard for the whole use.
/// A synthetic source supplies an owned stream without forging device state.
pub(super) struct InspectorConnection {
    pub(super) device: Option<ConnectedDevice>,
    pub(super) stream: ServiceStream,
}

pub(super) type ConnectionFuture<'a> =
    Pin<Box<dyn Future<Output = Result<InspectorConnection>> + Send + 'a>>;

/// Rust-owned admission to a service stream. No HTTP/CLI input selects a source.
pub(super) trait InspectorSource: Send + Sync {
    fn connect<'a>(&'a self, udid: &'a str, timeout: Duration) -> ConnectionFuture<'a>;
}

pub(super) struct DeviceInspectorSource;

impl InspectorSource for DeviceInspectorSource {
    fn connect<'a>(&'a self, udid: &'a str, timeout: Duration) -> ConnectionFuture<'a> {
        Box::pin(async move {
            let (device, stream, _) = connect_webinspector_with_timeout(udid, timeout).await?;
            Ok(InspectorConnection {
                device: Some(device),
                stream,
            })
        })
    }
}

pub(super) async fn connect_webinspector(
    udid: &str,
) -> Result<(ConnectedDevice, ServiceStream, bool)> {
    let probe = ios_core::connect(
        udid,
        ConnectOptions {
            tun_mode: TunMode::Userspace,
            pair_record_path: None,
            skip_tunnel: true,
        },
    )
    .await?;
    let version = probe.product_version().await?;
    drop(probe);

    if version.major >= 17 {
        let device = ios_core::connect(
            udid,
            ConnectOptions {
                tun_mode: TunMode::Userspace,
                pair_record_path: None,
                skip_tunnel: false,
            },
        )
        .await?;
        let stream = device.connect_rsd_service(RSD_SERVICE_NAME).await?;
        return Ok((device, stream, true));
    }

    let device = ios_core::connect(
        udid,
        ConnectOptions {
            tun_mode: TunMode::Userspace,
            pair_record_path: None,
            skip_tunnel: true,
        },
    )
    .await?;
    let stream = device.connect_service(SERVICE_NAME).await?;
    Ok((device, stream, false))
}

pub(super) async fn connect_webinspector_with_timeout(
    udid: &str,
    timeout: Duration,
) -> Result<(ConnectedDevice, ServiceStream, bool)> {
    tokio::time::timeout(timeout, connect_webinspector(udid))
        .await
        .with_context(|| format!("webinspector connection timed out after {timeout:?}"))?
}
