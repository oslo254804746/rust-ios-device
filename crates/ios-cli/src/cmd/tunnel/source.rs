//! Injectable private boundary around the existing device connection interfaces.

use futures_util::future::BoxFuture;
use ios_core::device::PairedMobdev2Device;
use ios_core::{ConnectOptions, MuxClient, TunMode, TunnelHandle, TunnelOwner};
use std::sync::Arc;

use super::{ManagedDevice, TunnelRecord};

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum Route {
    Usbmux,
    Usb(Option<String>),
    Wifi(Option<String>),
    Mobdev2(String),
}

pub(super) enum UsbSnapshot {
    Devices(Vec<UsbDevice>),
    ConnectError(anyhow::Error),
    ListError(anyhow::Error),
}

#[derive(Clone)]
pub(super) struct UsbDevice {
    pub(super) udid: String,
    pub(super) device_id: u32,
    pub(super) usb: bool,
}

// Match the existing Core connect selection: first USB, otherwise last match.
// Keep each UDID's first discovery position so candidate scheduling is stable.
pub(super) fn selected_devices(devices: Vec<UsbDevice>) -> Vec<UsbDevice> {
    let mut positions: std::collections::HashMap<String, usize> = std::collections::HashMap::new();
    let mut selected: Vec<UsbDevice> = Vec::new();
    for device in devices {
        if let Some(&position) = positions.get(&device.udid) {
            if !selected[position].usb {
                selected[position] = device;
            }
        } else {
            positions.insert(device.udid.clone(), selected.len());
            selected.push(device);
        }
    }
    selected
}

pub(super) trait Source: Send + Sync {
    fn connect<'a>(
        &'a self,
        udid: &'a str,
        route: Route,
        mode: TunMode,
    ) -> BoxFuture<'a, anyhow::Result<ManagedDevice>>;

    fn connect_owned<'a>(
        &'a self,
        udid: &'a str,
        route: Route,
        mode: TunMode,
        owner: &'a dyn TunnelOwner,
    ) -> BoxFuture<'a, anyhow::Result<ManagedDevice>> {
        Box::pin(async move {
            let mut reservation = owner.reserve()?;
            let device = self.connect(udid, route, mode).await?;
            if let Some(handle) = device.native_handle() {
                reservation.attach(Arc::clone(handle))?;
                reservation.complete();
            }
            Ok(device)
        })
    }
    fn usb_devices(&self) -> BoxFuture<'_, UsbSnapshot>;
    fn paired_devices(&self) -> BoxFuture<'_, anyhow::Result<Vec<PairedMobdev2Device>>>;
}

pub(super) struct NativeSource;

impl Source for NativeSource {
    fn connect_owned<'a>(
        &'a self,
        udid: &'a str,
        route: Route,
        mode: TunMode,
        owner: &'a dyn TunnelOwner,
    ) -> BoxFuture<'a, anyhow::Result<ManagedDevice>> {
        Box::pin(async move {
            let options = ConnectOptions {
                tun_mode: mode,
                pair_record_path: None,
                skip_tunnel: false,
            };
            let device = match route {
                Route::Usbmux => ios_core::connect_with_owner(udid, options, owner).await?,
                Route::Usb(host) => {
                    ios_core::connect_direct_usb_tunnel_with_owner(
                        udid,
                        host.as_deref(),
                        options,
                        owner,
                    )
                    .await?
                }
                Route::Wifi(host) => {
                    ios_core::connect_remote_pairing_tunnel_with_owner(
                        udid,
                        host.as_deref(),
                        options,
                        owner,
                    )
                    .await?
                }
                Route::Mobdev2(host) => {
                    ios_core::connect_tcp_lockdown_tunnel_with_owner(udid, &host, options, owner)
                        .await?
                }
            };
            Ok(ManagedDevice::Native(Box::new(device)))
        })
    }

    fn connect<'a>(
        &'a self,
        udid: &'a str,
        route: Route,
        mode: TunMode,
    ) -> BoxFuture<'a, anyhow::Result<ManagedDevice>> {
        Box::pin(async move {
            let options = ConnectOptions {
                tun_mode: mode,
                pair_record_path: None,
                skip_tunnel: false,
            };
            let device = match route {
                Route::Usbmux => ios_core::connect(udid, options).await?,
                Route::Usb(host) => {
                    ios_core::connect_direct_usb_tunnel(udid, host.as_deref(), options).await?
                }
                Route::Wifi(host) => {
                    ios_core::connect_remote_pairing_tunnel(udid, host.as_deref(), options).await?
                }
                Route::Mobdev2(host) => {
                    ios_core::connect_tcp_lockdown_tunnel(udid, &host, options).await?
                }
            };
            Ok(ManagedDevice::Native(Box::new(device)))
        })
    }

    fn usb_devices(&self) -> BoxFuture<'_, UsbSnapshot> {
        Box::pin(async {
            let mut mux = match MuxClient::connect().await {
                Ok(mux) => mux,
                Err(error) => return UsbSnapshot::ConnectError(error.into()),
            };
            match mux.list_devices().await {
                Ok(devices) => UsbSnapshot::Devices(
                    devices
                        .into_iter()
                        .map(|device| UsbDevice {
                            udid: device.serial_number,
                            device_id: device.device_id,
                            usb: device.connection_type.eq_ignore_ascii_case("USB"),
                        })
                        .collect(),
                ),
                Err(error) => UsbSnapshot::ListError(error.into()),
            }
        })
    }

    fn paired_devices(&self) -> BoxFuture<'_, anyhow::Result<Vec<PairedMobdev2Device>>> {
        Box::pin(async { Ok(ios_core::discover_paired_mobdev2_devices().await?) })
    }
}

// This seam is confined to the CLI's private test build. The SDK device type and
// native protocols do not gain a synthetic constructor or alternate wire path.
#[cfg(test)]
pub(super) trait FakeDevice: Send + Sync {
    fn record(&self) -> TunnelRecord;
    fn is_alive(&self) -> bool;
    fn device_id(&self) -> u32;
    fn native_handle(&self) -> Option<&Arc<TunnelHandle>> {
        None
    }
}

impl ManagedDevice {
    pub(super) fn native_handle(&self) -> Option<&Arc<TunnelHandle>> {
        match self {
            Self::Native(device) => device.tunnel_handle(),
            #[cfg(test)]
            Self::Fake(device) => device.native_handle(),
        }
    }

    pub(super) fn record_for_mode(&self, mode: TunMode) -> anyhow::Result<TunnelRecord> {
        match self {
            Self::Native(device) => super::ManagedTunnel::record_for_device(device, mode),
            #[cfg(test)]
            Self::Fake(device) => Ok(device.record()),
        }
    }

    pub(super) fn device_id(&self) -> u32 {
        match self {
            Self::Native(device) => device.info.device_id,
            #[cfg(test)]
            Self::Fake(device) => device.device_id(),
        }
    }

    pub(super) fn userspace(&self) -> bool {
        match self {
            Self::Native(device) => device.userspace_port().is_some(),
            #[cfg(test)]
            Self::Fake(device) => device.record().userspace_tun,
        }
    }

    pub(super) fn is_alive(&self) -> bool {
        match self {
            Self::Native(device) => device
                .tunnel_handle()
                .map(|tunnel| tunnel.is_alive())
                .unwrap_or(false),
            #[cfg(test)]
            Self::Fake(device) => device.is_alive(),
        }
    }
}
