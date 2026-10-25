//! The Source's checked private candidate stays inert until publication is staged.
use std::{
    future::Future,
    pin::Pin,
    sync::Arc,
    task::{Context, Poll},
};

use anyhow::Result;
use ios_core::{OwnedConnectedDevice, PreparedConnectedDevice, TunMode, TunnelHandle};
use tokio::sync::Semaphore;

use super::{
    future_storage::Dynamic, lifecycle::ConnectionDeadline, owned_box, ManagedDevice, PaidRecord,
};

pub(super) enum SourceFuture<'a> {
    Native(Dynamic<'a, Result<Candidate>>),
    // Existing unit Sources explicitly remain unverified fixtures. They cannot
    // manufacture a PreparedConnectedDevice or a checked catalogue.
    #[cfg(test)]
    Fixture(Dynamic<'a, Result<ManagedDevice>>),
}

impl Future for SourceFuture<'_> {
    type Output = Result<Candidate>;
    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        match self.get_mut() {
            Self::Native(future) => Pin::new(future).poll(cx),
            #[cfg(test)]
            Self::Fixture(future) => Pin::new(future)
                .poll(cx)
                .map(|result| result.map(Candidate::Fixture)),
        }
    }
}

pub(super) enum Candidate {
    Native {
        device: owned_box::Owner<PreparedConnectedDevice>,
        output: owned_box::Destination<OwnedConnectedDevice>,
    },
    #[cfg(test)]
    Fixture(ManagedDevice),
}

impl Candidate {
    pub(super) fn retained_metadata_bytes(&self) -> Result<usize> {
        match self {
            Self::Native { device, .. } => device.retained_metadata_bytes().ok_or_else(|| {
                crate::output::resource_decision("tunnel retained metadata size overflow")
            }),
            #[cfg(test)]
            Self::Fixture(ManagedDevice::Native(device)) => {
                device.retained_metadata_bytes().ok_or_else(|| {
                    crate::output::resource_decision("tunnel retained metadata size overflow")
                })
            }
            #[cfg(test)]
            Self::Fixture(ManagedDevice::Fake(device)) => Ok(std::mem::size_of_val(&**device)),
        }
    }

    // The final output's real empty Box is already paid, as is initial RSD.
    // The temporary candidate Box is a separate, live reservation until finish.
    pub(super) fn owned_body_bytes(&self, bank: &Arc<Semaphore>) -> Result<usize> {
        match self {
            Self::Native { device, output } => output
                .bytes_in(bank)?
                .checked_add(device.initial_rsd_bytes_in(bank)?)
                .ok_or_else(|| {
                    crate::output::resource_decision(
                        "tunnel independently owned directory size overflow",
                    )
                }),
            #[cfg(test)]
            Self::Fixture(device) => device.owned_body_bytes(bank),
        }
    }

    pub(super) fn native_handle(&self) -> Option<&Arc<TunnelHandle>> {
        match self {
            Self::Native { device, .. } => Some(device.tunnel_handle()),
            #[cfg(test)]
            Self::Fixture(device) => device.native_handle(),
        }
    }

    pub(super) fn device_id(&self) -> u32 {
        match self {
            Self::Native { device, .. } => device.info().device_id,
            #[cfg(test)]
            Self::Fixture(device) => device.device_id(),
        }
    }

    pub(super) fn record(&self, mode: TunMode, bank: &Arc<Semaphore>) -> Result<PaidRecord> {
        match self {
            Self::Native { device, .. } => PaidRecord::from_prepared(device, mode, bank),
            #[cfg(test)]
            Self::Fixture(device) => device.record_for_mode(mode, bank),
        }
    }

    pub(super) fn stage_catalogue(
        &self,
        global: &Arc<Semaphore>,
        publication: &Arc<Semaphore>,
    ) -> Result<Option<super::freshness::MetadataOwner>> {
        match self {
            Self::Native { device, .. } => Ok(Some(super::freshness::Metadata::prepare(
                device.current_rsd_catalogue().clone(),
                global,
                publication,
            )?)),
            #[cfg(test)]
            Self::Fixture(_) => Ok(None),
        }
    }

    pub(super) async fn finish(
        self,
        deadline: Option<&ConnectionDeadline>,
    ) -> Result<ManagedDevice> {
        match self {
            Self::Native { device, output } => {
                let at = deadline.map_or(device.deadline(), ConnectionDeadline::absolute_at);
                let device = device.into_inner();
                let completed = match device.requested_mode() {
                    TunMode::Userspace => device.finish_userspace_at(at).await?,
                    TunMode::Kernel => device.finish_kernel_at(at).await?,
                };
                Ok(ManagedDevice::Native(output.place(completed)))
            }
            #[cfg(test)]
            Self::Fixture(device) => Ok(device),
        }
    }
}
