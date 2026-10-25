//! Authenticated and checked candidate, before selected-backend publication.
#[cfg(any(
    feature = "tunnel-userspace",
    all(
        feature = "tunnel-kernel",
        any(target_os = "windows", target_os = "linux", target_os = "macos")
    )
))]
use super::OwnedConnectedDevice;
use super::{ConnectedDevice, InitialRsdProbe, PreparedRsdOwner};
use crate::{CoreError, DeviceInfo, TunnelHandle};
use std::sync::Arc;
use tokio::time::Instant;

/// A candidate whose current directory was checked over its own private packet
/// engine. It has not created a native interface, address, or route. Dropping
/// this owner requests stop; the original attached handle retains all joins.
pub struct PreparedConnectedDevice {
    inner: Option<ConnectedDevice>,
    deadline: Instant,
}
impl PreparedConnectedDevice {
    pub(super) fn new(inner: ConnectedDevice, deadline: Instant) -> Self {
        Self {
            inner: Some(inner),
            deadline,
        }
    }
    fn device(&self) -> &ConnectedDevice {
        self.inner.as_ref().expect("owned prepared connection")
    }
    pub fn info(&self) -> &DeviceInfo {
        &self.device().info
    }
    pub fn tunnel_handle(&self) -> &Arc<TunnelHandle> {
        self.device()
            .tunnel
            .as_ref()
            .expect("prepared attached runtime")
    }
    pub fn requested_mode(&self) -> crate::TunMode {
        self.tunnel_handle()
            .prepared_mode()
            .expect("original selected prepared backend")
    }
    /// The original preparation clock also bounds final native/public exposure.
    pub fn deadline(&self) -> Instant {
        self.deadline
    }
    pub fn current_rsd_catalogue(&self) -> &PreparedRsdOwner {
        self.device()
            .owned_prepared_rsd
            .as_ref()
            .expect("independent checked catalogue")
    }
    pub fn initial_rsd_probe(&self) -> &InitialRsdProbe {
        &self.device().initial_rsd_probe
    }
    pub fn rsd(&self) -> Option<&crate::OwnedRsdHandshake> {
        self.device().owned_initial_rsd.as_ref()
    }
    pub fn initial_rsd_bytes_in(
        &self,
        bank: &Arc<tokio::sync::Semaphore>,
    ) -> Result<usize, CoreError> {
        match self.rsd() {
            Some(directory) if directory.belongs_to(bank) => Ok(directory.known_bytes()),
            None => Ok(0),
            Some(_) => Err(CoreError::Xpc(
                super::existing_rsd::ExistingRsdProbeError::metadata_limit(),
            )),
        }
    }
    pub fn retained_metadata_bytes(&self) -> Option<usize> {
        self.device().retained_metadata_bytes()
    }

    /// Activate the selected Kernel backend after staging publication owners.
    /// The checked directory, handle and original absolute clock stay the same.
    #[cfg(all(
        feature = "tunnel-kernel",
        any(target_os = "windows", target_os = "linux", target_os = "macos")
    ))]
    pub async fn finish_kernel_at(
        self,
        deadline: Instant,
    ) -> Result<OwnedConnectedDevice, CoreError> {
        self.finish_kernel_with(deadline, |runtime, bank, info, deadline| {
            runtime.prepare_kernel(bank, info, deadline)
        })
        .await
    }

    #[cfg(all(test, feature = "tunnel-kernel", target_os = "windows"))]
    pub(crate) async fn finish_kernel_scripted_at(
        self,
        deadline: Instant,
        script: &crate::tunnel::tun::userspace::WindowsScript,
    ) -> Result<OwnedConnectedDevice, CoreError> {
        self.finish_kernel_with(deadline, |runtime, bank, info, deadline| {
            runtime.prepare_kernel_scripted(bank, info, deadline, script)
        })
        .await
    }

    #[cfg(all(
        feature = "tunnel-kernel",
        any(target_os = "windows", target_os = "linux", target_os = "macos")
    ))]
    async fn finish_kernel_with(
        mut self,
        deadline: Instant,
        prepare: impl FnOnce(
            &crate::tunnel::tun::userspace::UserspaceTunDevice,
            Arc<tokio::sync::Semaphore>,
            &crate::tunnel::TunnelInfo,
            Instant,
        ) -> Result<
            crate::tunnel::tun::userspace::KernelRuntime,
            crate::tunnel::TunnelError,
        >,
    ) -> Result<OwnedConnectedDevice, CoreError> {
        let bank = self
            .device()
            .owned_initial_bank
            .as_ref()
            .expect("original prepared byte bank")
            .clone();
        self.tunnel_handle()
            .finish_prepared_kernel_with(bank, deadline.min(self.deadline), prepare)
            .await
            .map_err(CoreError::Tunnel)?;
        Ok(OwnedConnectedDevice::new(
            self.inner.take().expect("same prepared device"),
        ))
    }

    /// Promote the same userspace listener after the caller has staged its
    /// publication owners. The caller supplies its original absolute clock.
    #[cfg(feature = "tunnel-userspace")]
    pub async fn finish_userspace_at(
        mut self,
        deadline: Instant,
    ) -> Result<OwnedConnectedDevice, CoreError> {
        let clock = super::existing_rsd::Clock::at(deadline.min(self.deadline));
        clock
            .run(async {
                clock.check()?;
                self.tunnel_handle()
                    .finish_prepared_userspace()
                    .map_err(CoreError::Tunnel)?;
                clock.check()?;
                Ok(())
            })
            .await?;
        Ok(OwnedConnectedDevice::new(
            self.inner.take().expect("same prepared device"),
        ))
    }
}
impl Drop for PreparedConnectedDevice {
    fn drop(&mut self) {
        if let Some(handle) = self
            .inner
            .as_ref()
            .and_then(|device| device.tunnel.as_ref())
        {
            handle.request_stop();
        }
    }
}
