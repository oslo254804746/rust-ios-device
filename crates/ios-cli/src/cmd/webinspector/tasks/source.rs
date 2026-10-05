//! Real device admission or an owned synthetic service source, selected in Rust.
use std::{future::Future, pin::Pin};

use ios_core::device::{ConnectedDevice, ServiceStream};

use super::{connect_webinspector_with_timeout, control::Budget, Result};

pub(super) type SourceFuture<'a, T> = Pin<Box<dyn Future<Output = Result<T>> + Send + 'a>>;

pub(super) struct Connection {
    pub(super) device: Option<ConnectedDevice>,
    pub(super) stream: ServiceStream,
    pub(super) uses_rsd: bool,
}

pub(super) trait Source: Send + Sync {
    fn list_devices(&self) -> SourceFuture<'_, Vec<ios_core::DeviceInfo>>;
    fn connect<'a>(&'a self, udid: &'a str, budget: Budget) -> SourceFuture<'a, Connection>;
}

pub(super) struct DeviceSource;

impl Source for DeviceSource {
    fn list_devices(&self) -> SourceFuture<'_, Vec<ios_core::DeviceInfo>> {
        Box::pin(async { Ok(ios_core::list_devices().await?) })
    }

    fn connect<'a>(&'a self, udid: &'a str, budget: Budget) -> SourceFuture<'a, Connection> {
        Box::pin(async move {
            let (device, stream, uses_rsd) =
                connect_webinspector_with_timeout(udid, budget.remaining()?).await?;
            Ok(Connection {
                device: Some(device),
                stream,
                uses_rsd,
            })
        })
    }
}
