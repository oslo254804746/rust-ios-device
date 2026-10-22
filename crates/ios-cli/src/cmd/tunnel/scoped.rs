//! Finite foreground owners reuse the manager's original native retirement registry.
use super::{
    future_storage::PaidFuture,
    ownership::Ownership,
    retirement::{CleanupClock, NativeLease, Registry, RegistryOwner},
};
use anyhow::{Context, Result};
use ios_core::{ConnectOptions, ConnectedDevice, TunMode, TunnelHandle, TunnelOwner};
use std::{future::Future, ops::Deref, sync::Arc, time::Duration};
use tokio::sync::{OwnedSemaphorePermit, Semaphore};

#[derive(Clone)]
pub(crate) struct NativeScope(InnerOwner);
type InnerOwner = super::shared_allocation::Shared<Inner>;
struct Inner {
    registry: RegistryOwner,
    connections: Arc<Semaphore>,
}
impl Drop for Inner {
    fn drop(&mut self) {
        self.registry.close_and_stop_all();
    }
}

pub(crate) struct NativeConnection<D = ConnectedDevice> {
    device: Option<D>,
    lease: Option<NativeLease>,
    _permit: OwnedSemaphorePermit,
}
impl<D> Deref for NativeConnection<D> {
    type Target = D;
    fn deref(&self) -> &D {
        self.device.as_ref().expect("live native connection")
    }
}
impl<D> Drop for NativeConnection<D> {
    fn drop(&mut self) {
        drop(self.device.take());
        drop(self.lease.take());
    }
}

trait Device {
    fn tunnel_handle(&self) -> Option<&Arc<TunnelHandle>>;
}
impl Device for ConnectedDevice {
    fn tunnel_handle(&self) -> Option<&Arc<TunnelHandle>> {
        ConnectedDevice::tunnel_handle(self)
    }
}

// Keep the complete concrete connector future in its original containing F.
// The test connector uses the same reservation/handoff path without device IO.
trait Connector: Send + Sync {
    type Device: Device + Send;

    fn connect<'a>(
        &'a self,
        udid: &'a str,
        options: ConnectOptions,
        owner: &'a dyn TunnelOwner,
    ) -> impl Future<Output = Result<Self::Device>> + Send + 'a;
}
struct NativeConnector;
impl Connector for NativeConnector {
    type Device = ConnectedDevice;

    async fn connect<'a>(
        &'a self,
        udid: &'a str,
        options: ConnectOptions,
        owner: &'a dyn TunnelOwner,
    ) -> Result<Self::Device> {
        Ok(ios_core::connect_with_owner(udid, options, owner).await?)
    }
}

impl NativeScope {
    pub(crate) fn new() -> Result<Self> {
        Self::from_registry(Registry::new()?)
    }

    fn from_registry(registry: RegistryOwner) -> Result<Self> {
        let bank = registry.byte_bank();
        let storage =
            super::admission::known_metadata_charge(&bank, InnerOwner::allocation_bytes())?;
        Ok(Self(InnerOwner::from_grant(
            Inner {
                registry,
                connections: Arc::new(Semaphore::new(4)),
            },
            &bank,
            storage,
        )?))
    }
    pub(crate) async fn connect(&self, udid: &str, mode: TunMode) -> Result<NativeConnection> {
        self.prepare_connect(udid, mode, NativeConnector)?.await
    }

    fn prepare_connect<'a, C: Connector + 'a>(
        &'a self,
        udid: &'a str,
        mode: TunMode,
        connector: C,
    ) -> Result<PaidFuture<impl Future<Output = Result<NativeConnection<C::Device>>> + Send + 'a>>
    {
        super::admission::selectors(udid, None)?;
        let permit = self
            .0
            .connections
            .clone()
            .try_acquire_owned()
            .map_err(|_| {
                crate::output::resource_decision("foreground native connection capacity")
            })?;
        PaidFuture::new(
            self.connect_admitted(udid, mode, permit, connector),
            &self.0.registry.byte_bank(),
        )
    }

    async fn connect_admitted<C: Connector>(
        &self,
        udid: &str,
        mode: TunMode,
        permit: OwnedSemaphorePermit,
        connector: C,
    ) -> Result<NativeConnection<C::Device>> {
        let ownership = Ownership::standalone(self.0.registry.clone())?;
        let device = connector
            .connect(
                udid,
                ConnectOptions {
                    tun_mode: mode,
                    pair_record_path: None,
                    skip_tunnel: false,
                },
                &ownership,
            )
            .await?;
        let lease = device
            .tunnel_handle()
            .map(|handle| ownership.take_completed(handle))
            .transpose()?;
        Ok(NativeConnection {
            device: Some(device),
            lease,
            _permit: permit,
        })
    }
    /// No new task is spawned. Cancelled shutdown keeps its original registry;
    /// another collect/close can resume unfinished joins.
    pub(crate) async fn collect(&self, budget: Duration) -> Result<()> {
        let clock = CleanupClock::new(budget)?;
        self.0
            .registry
            .drain_retired(clock)
            .await
            .map(|_| ())
            .map_err(|primary| {
                anyhow::Error::new(RetainedScopeFailure {
                    primary: primary.into(),
                    scope: self.clone(),
                })
            })
    }
    pub(crate) async fn close(&self, budget: Duration) -> Result<()> {
        self.0.registry.close_and_stop_all();
        self.collect(budget)
            .await
            .context("foreground native joins failed")
    }
}
struct RetainedScopeFailure {
    primary: anyhow::Error,
    scope: NativeScope,
}
impl std::fmt::Display for RetainedScopeFailure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("foreground original native cleanup failed")
    }
}
impl std::fmt::Debug for RetainedScopeFailure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RetainedScopeFailure")
            .field(
                "original_owners",
                &self.scope.0.registry.snapshot().occupied(),
            )
            .finish_non_exhaustive()
    }
}
impl std::error::Error for RetainedScopeFailure {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        Some(self.primary.as_ref())
    }
}

#[cfg(test)]
mod frame_tests;
#[cfg(test)]
mod root_tests;

#[cfg(test)]
mod tests {
    use super::*;
    use ios_core::{TunnelHandle, TunnelInfo, TunnelOwner, UserspaceTunDevice};
    use tokio::io::AsyncReadExt;

    #[tokio::test]
    async fn foreground_close_joins_original_active_native_and_closes_owned_tcp_peer() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let stream = tokio::net::TcpStream::connect(listener.local_addr().unwrap())
            .await
            .unwrap();
        let mut peer = listener.accept().await.unwrap().0;
        let scope = NativeScope::new().unwrap();
        let ownership = Ownership::standalone(scope.0.registry.clone()).unwrap();
        let mut reservation = ownership.reserve_owned().unwrap();
        let runtime = UserspaceTunDevice::start("fd00::2", "fd00::1", 1280, stream)
            .await
            .unwrap();
        let handle = Arc::new(TunnelHandle::new_userspace(
            "foreground-fixture".into(),
            TunnelInfo {
                server_address: "fd00::1".into(),
                server_rsd_port: 58783,
                client_address: "fd00::2".into(),
                client_mtu: 1280,
            },
            runtime,
        ));
        reservation.attach(handle.clone()).unwrap();
        reservation.complete();
        drop(reservation);
        let lease = ownership.take_completed(&handle).unwrap();
        drop(ownership);
        scope.close(Duration::from_secs(2)).await.unwrap();
        let mut unexpected = Vec::new();
        tokio::time::timeout(Duration::from_secs(2), peer.read_to_end(&mut unexpected))
            .await
            .unwrap()
            .unwrap();
        assert!(
            unexpected.is_empty(),
            "idle original native must close without manufactured traffic"
        );
        assert!(handle.shutdown_status().is_ok());
        assert_eq!(scope.0.registry.snapshot().occupied(), 0);
        drop(lease);
        assert!(
            scope
                .connect("foreground-fixture", TunMode::Userspace)
                .await
                .is_err(),
            "closed admission must fail before discovery IO"
        );
    }
}
