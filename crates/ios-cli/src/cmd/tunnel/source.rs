//! Injectable private boundary around the existing device connection interfaces.

#[cfg(test)]
use futures_util::future::BoxFuture;
use ios_core::device::PairedMobdev2Device;
use ios_core::{ConnectOptions, MuxClient, TunMode, TunnelHandle, TunnelOwner};
use std::sync::Arc;
use tokio::sync::{OwnedSemaphorePermit, Semaphore};

#[cfg(test)]
use super::TunnelRecord;
use super::{candidate_input::RouteRef, ManagedDevice};

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

#[derive(Debug)]
pub(super) struct DiscoveryAdmission;
impl std::fmt::Display for DiscoveryAdmission {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("tunnel discovery Source future admission")
    }
}
impl std::error::Error for DiscoveryAdmission {}

fn discovery_future<'a, F: std::future::Future + Send + 'a>(
    future: F,
    bank: &Arc<Semaphore>,
) -> anyhow::Result<super::future_storage::Dynamic<'a, F::Output>> {
    super::future_storage::Dynamic::new(future, bank)
        .map_err(|error| error.context(DiscoveryAdmission))
}

// Match the existing Core connect selection: first USB, otherwise last match.
// Keep each UDID's first discovery position so candidate scheduling is stable.
pub(super) fn selected_devices(
    mut devices: Vec<UsbDevice>,
    bank: &Arc<Semaphore>,
) -> anyhow::Result<(Vec<UsbDevice>, OwnedSemaphorePermit)> {
    // Borrow identities via indices rather than cloning keys into an opaque
    // hash table. Sort by identity, then original position within that identity.
    let allocation = super::metadata_vec::exact_backing(bank, devices.len())?;
    let _index_storage = allocation.storage;
    let mut positions = allocation.values;
    positions.extend((0..devices.len()).map(|index| (index, index)));
    positions
        .sort_unstable_by(|(_, a), (_, b)| devices[*a].udid.cmp(&devices[*b].udid).then(a.cmp(b)));
    let mut read = 0;
    let mut write = 0;
    while read < positions.len() {
        let first = positions[read].0;
        let mut chosen = first;
        let mut end = read + 1;
        while end < positions.len() && devices[positions[end].0].udid == devices[first].udid {
            if !devices[chosen].usb {
                chosen = positions[end].0;
            }
            end += 1;
        }
        positions[write] = (first, chosen);
        write += 1;
        read = end;
    }
    positions.truncate(write);
    positions.sort_unstable_by_key(|(first, _)| *first);
    let allocation = super::metadata_vec::exact_backing(bank, write)?;
    let storage = allocation.storage;
    let mut selected = allocation.values;
    for (_, index) in positions {
        let device = &mut devices[index];
        // Move the original String buffer. The caller's raw snapshot reservation
        // continues to cover these capacities through candidate ownership.
        selected.push(UsbDevice {
            udid: std::mem::take(&mut device.udid),
            device_id: device.device_id,
            usb: device.usb,
        });
    }
    Ok((selected, storage))
}

#[cfg(test)]
mod selection_tests;

#[cfg(test)]
mod discovery_future_tests;
#[cfg(test)]
mod future_tests;

pub(super) trait Source: Send + Sync {
    #[cfg(not(test))]
    fn observes_freshness(&self) -> bool;
    #[cfg(test)]
    fn observes_freshness(&self) -> bool {
        false
    }

    #[cfg(not(test))]
    fn refresh_existing_rsd<'a>(
        &'a self,
        route: ios_core::device::ExistingRsdRoute,
        deadline: tokio::time::Instant,
        bank: &'a Arc<Semaphore>,
    ) -> anyhow::Result<
        super::future_storage::Dynamic<
            'a,
            anyhow::Result<Arc<ios_core::device::PreparedRsdCatalogue>>,
        >,
    >;
    #[cfg(test)]
    fn refresh_existing_rsd<'a>(
        &'a self,
        _route: ios_core::device::ExistingRsdRoute,
        _deadline: tokio::time::Instant,
        bank: &'a Arc<Semaphore>,
    ) -> anyhow::Result<
        super::future_storage::Dynamic<
            'a,
            anyhow::Result<Arc<ios_core::device::PreparedRsdCatalogue>>,
        >,
    > {
        super::future_storage::Dynamic::new(
            async {
                Err(crate::output::input_decision(
                    "private fixture refresh Source not implemented",
                ))
            },
            bank,
        )
    }

    #[cfg(test)]
    fn connect<'a>(
        &'a self,
        udid: &'a str,
        route: Route,
        mode: TunMode,
    ) -> BoxFuture<'a, anyhow::Result<ManagedDevice>>;

    #[cfg(not(test))]
    fn connect_owned<'a>(
        &'a self,
        udid: &'a str,
        route: RouteRef<'a>,
        mode: TunMode,
        owner: &'a dyn TunnelOwner,
        bank: &'a Arc<Semaphore>,
    ) -> anyhow::Result<super::future_storage::Dynamic<'a, anyhow::Result<ManagedDevice>>>;

    #[cfg(test)]
    fn connect_owned<'a>(
        &'a self,
        udid: &'a str,
        route: RouteRef<'a>,
        mode: TunMode,
        owner: &'a dyn TunnelOwner,
        bank: &'a Arc<tokio::sync::Semaphore>,
    ) -> anyhow::Result<
        crate::cmd::tunnel::future_storage::Dynamic<'a, anyhow::Result<ManagedDevice>>,
    > {
        crate::cmd::tunnel::future_storage::Dynamic::new(
            async move {
                let mut reservation = owner.reserve()?;
                let device = self.connect(udid, route.fixture_owned(), mode).await?;
                if let Some(handle) = device.native_handle() {
                    reservation.attach(Arc::clone(handle))?;
                    reservation.complete();
                }
                Ok(device)
            },
            bank,
        )
    }
    #[cfg(test)]
    fn usb_devices(&self) -> BoxFuture<'_, UsbSnapshot>;
    #[cfg(test)]
    fn paired_devices(&self) -> BoxFuture<'_, anyhow::Result<Vec<PairedMobdev2Device>>>;

    #[cfg(not(test))]
    fn usb_devices_owned<'a>(
        &'a self,
        bank: &'a Arc<Semaphore>,
    ) -> anyhow::Result<super::future_storage::Dynamic<'a, UsbSnapshot>>;
    #[cfg(test)]
    fn usb_devices_owned<'a>(
        &'a self,
        bank: &'a Arc<Semaphore>,
    ) -> anyhow::Result<super::future_storage::Dynamic<'a, UsbSnapshot>> {
        discovery_future(async move { self.usb_devices().await }, bank)
    }

    #[cfg(not(test))]
    fn paired_devices_owned<'a>(
        &'a self,
        bank: &'a Arc<Semaphore>,
    ) -> anyhow::Result<super::future_storage::Dynamic<'a, anyhow::Result<Vec<PairedMobdev2Device>>>>;
    #[cfg(test)]
    fn paired_devices_owned<'a>(
        &'a self,
        bank: &'a Arc<Semaphore>,
    ) -> anyhow::Result<super::future_storage::Dynamic<'a, anyhow::Result<Vec<PairedMobdev2Device>>>>
    {
        discovery_future(async move { self.paired_devices().await }, bank)
    }
}

pub(super) struct NativeSource;

// Generic HTTP/auth/epoch unit fixtures have no authority to enumerate the
// host's usbmux or mDNS devices. Protocol/native fixtures inject their own
// explicit localhost Source instead; production uses NativeSource directly.
#[cfg(test)]
pub(super) struct NoDevices;

#[cfg(test)]
impl Source for NoDevices {
    fn connect<'a>(
        &'a self,
        udid: &'a str,
        _route: Route,
        _mode: TunMode,
    ) -> BoxFuture<'a, anyhow::Result<ManagedDevice>> {
        Box::pin(async move { Err(ios_core::CoreError::DeviceNotFound(udid.to_owned()).into()) })
    }

    fn usb_devices(&self) -> BoxFuture<'_, UsbSnapshot> {
        Box::pin(async { UsbSnapshot::Devices(Vec::new()) })
    }

    fn paired_devices(&self) -> BoxFuture<'_, anyhow::Result<Vec<PairedMobdev2Device>>> {
        Box::pin(async { Ok(Vec::new()) })
    }
}

impl Source for NativeSource {
    fn observes_freshness(&self) -> bool {
        true
    }
    fn refresh_existing_rsd<'a>(
        &'a self,
        route: ios_core::device::ExistingRsdRoute,
        deadline: tokio::time::Instant,
        bank: &'a Arc<Semaphore>,
    ) -> anyhow::Result<
        super::future_storage::Dynamic<
            'a,
            anyhow::Result<Arc<ios_core::device::PreparedRsdCatalogue>>,
        >,
    > {
        super::future_storage::Dynamic::new(
            async move { Ok(ios_core::device::prepare_existing_rsd_at(route, deadline, bank).await?) },
            bank,
        )
    }

    fn connect_owned<'a>(
        &'a self,
        udid: &'a str,
        route: RouteRef<'a>,
        mode: TunMode,
        owner: &'a dyn TunnelOwner,
        bank: &'a Arc<tokio::sync::Semaphore>,
    ) -> anyhow::Result<
        crate::cmd::tunnel::future_storage::Dynamic<'a, anyhow::Result<ManagedDevice>>,
    > {
        let destination =
            super::owned_box::Destination::<ios_core::OwnedConnectedDevice>::new(bank)?;
        crate::cmd::tunnel::future_storage::Dynamic::new(
            async move {
                let options = ConnectOptions {
                    tun_mode: mode,
                    pair_record_path: None,
                    skip_tunnel: false,
                };
                let device = match route {
                    RouteRef::Usbmux => {
                        ios_core::connect_owned_with_owner(udid, options, owner, bank).await?
                    }
                    RouteRef::Usb(host) => {
                        ios_core::connect_direct_usb_tunnel_owned_with_owner(
                            udid, host, options, owner, bank,
                        )
                        .await?
                    }
                    RouteRef::Wifi(host) => {
                        ios_core::connect_remote_pairing_tunnel_owned_with_owner(
                            udid, host, options, owner, bank,
                        )
                        .await?
                    }
                    RouteRef::Mobdev2(host) => {
                        ios_core::connect_tcp_lockdown_tunnel_owned_with_owner(
                            udid, host, options, owner, bank,
                        )
                        .await?
                    }
                };
                Ok(ManagedDevice::Native(destination.place(device)))
            },
            bank,
        )
    }

    #[cfg(test)]
    fn connect<'a>(
        &'a self,
        _udid: &'a str,
        _route: Route,
        _mode: TunMode,
    ) -> BoxFuture<'a, anyhow::Result<ManagedDevice>> {
        Box::pin(async {
            Err(crate::output::input_decision(
                "NativeSource fixture requires admitted connect_owned; device IO disabled",
            ))
        })
    }

    fn usb_devices_owned<'a>(
        &'a self,
        bank: &'a Arc<Semaphore>,
    ) -> anyhow::Result<super::future_storage::Dynamic<'a, UsbSnapshot>> {
        discovery_future(
            async {
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
            },
            bank,
        )
    }

    fn paired_devices_owned<'a>(
        &'a self,
        bank: &'a Arc<Semaphore>,
    ) -> anyhow::Result<super::future_storage::Dynamic<'a, anyhow::Result<Vec<PairedMobdev2Device>>>>
    {
        discovery_future(
            async { Ok(ios_core::discover_paired_mobdev2_devices().await?) },
            bank,
        )
    }

    #[cfg(test)]
    fn usb_devices(&self) -> BoxFuture<'_, UsbSnapshot> {
        Box::pin(async {
            UsbSnapshot::ConnectError(crate::output::input_decision(
                "NativeSource fixture requires admitted usb_devices_owned; device IO disabled",
            ))
        })
    }

    #[cfg(test)]
    fn paired_devices(&self) -> BoxFuture<'_, anyhow::Result<Vec<PairedMobdev2Device>>> {
        Box::pin(async {
            Err(crate::output::input_decision(
                "NativeSource fixture requires admitted paired_devices_owned; device IO disabled",
            ))
        })
    }
}

// This seam is confined to the CLI's private test build. The SDK device type and
// native protocols do not gain a synthetic constructor or alternate wire path.
#[cfg(test)]
pub(super) trait FakeDevice: Send + Sync {
    fn freshness_route(&self) -> anyhow::Result<Option<ios_core::device::ExistingRsdRoute>> {
        Ok(None)
    }
    fn freshness_catalogue(&self) -> Option<&Arc<ios_core::device::PreparedRsdCatalogue>> {
        None
    }
    fn swap_freshness_catalogue(
        &mut self,
        _runtime: &Arc<TunnelHandle>,
        _proposed: Option<Arc<ios_core::device::PreparedRsdCatalogue>>,
    ) -> anyhow::Result<Option<Arc<ios_core::device::PreparedRsdCatalogue>>> {
        Err(super::lifecycle::cancelled())
    }

    fn record(&self) -> TunnelRecord;
    fn is_alive(&self) -> bool;
    fn device_id(&self) -> u32;
    fn native_handle(&self) -> Option<&Arc<TunnelHandle>> {
        None
    }
}

impl ManagedDevice {
    pub(super) fn freshness_route(
        &self,
    ) -> anyhow::Result<Option<ios_core::device::ExistingRsdRoute>> {
        match self {
            Self::Native(device) => {
                let address = device
                    .server_address()
                    .ok_or_else(super::lifecycle::cancelled)?
                    .parse()?;
                let endpoint = std::net::SocketAddrV6::new(
                    address,
                    device.rsd_port().ok_or_else(super::lifecycle::cancelled)?,
                    0,
                    0,
                );
                Ok(Some(match device.userspace_port() {
                    Some(proxy) => {
                        ios_core::device::ExistingRsdRoute::via_userspace_proxy(endpoint, proxy)?
                    }
                    None => ios_core::device::ExistingRsdRoute::direct(endpoint)?,
                }))
            }
            #[cfg(test)]
            Self::Fake(device) => device.freshness_route(),
        }
    }
    pub(super) fn freshness_catalogue(
        &self,
    ) -> Option<&Arc<ios_core::device::PreparedRsdCatalogue>> {
        match self {
            Self::Native(device) => device.prepared_rsd_owner(),
            #[cfg(test)]
            Self::Fake(device) => device.freshness_catalogue(),
        }
    }
    pub(super) fn swap_freshness_catalogue(
        &mut self,
        runtime: &Arc<TunnelHandle>,
        proposed: Option<Arc<ios_core::device::PreparedRsdCatalogue>>,
    ) -> anyhow::Result<Option<Arc<ios_core::device::PreparedRsdCatalogue>>> {
        match self {
            Self::Native(device) => Ok(device.swap_current_rsd_catalogue(runtime, proposed)?),
            #[cfg(test)]
            Self::Fake(device) => device.swap_freshness_catalogue(runtime, proposed),
        }
    }

    pub(super) fn native_handle(&self) -> Option<&Arc<TunnelHandle>> {
        match self {
            Self::Native(device) => device.tunnel_handle(),
            #[cfg(test)]
            Self::Fake(device) => device.native_handle(),
        }
    }

    pub(super) fn record_for_mode(
        &self,
        mode: TunMode,
        bank: &Arc<Semaphore>,
    ) -> anyhow::Result<super::PaidRecord> {
        match self {
            Self::Native(device) => {
                super::record_owner::PaidRecord::from_device(device, mode, bank)
            }
            #[cfg(test)]
            Self::Fake(device) => {
                super::record_owner::PaidRecord::from_fixture(&device.record(), bank)
            }
        }
    }

    pub(super) fn record_fields_available(&self) -> bool {
        match self {
            Self::Native(device) => {
                device.server_address().is_some() && device.rsd_port().is_some()
            }
            #[cfg(test)]
            Self::Fake(_) => true,
        }
    }

    pub(super) fn device_id(&self) -> u32 {
        match self {
            Self::Native(device) => device.info().device_id,
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
