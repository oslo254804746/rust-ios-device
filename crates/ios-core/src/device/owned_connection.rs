//! Additive connection output with immutable initial RSD ownership.
use super::{ConnectedDevice, InternationalConfiguration, ResolvedServiceMetadata, ServiceStream};
use crate::{CoreError, DeviceInfo, OwnedRsdHandshake, TunnelHandle, XpcClient};
use std::sync::Arc;

/// A connection whose initial RSD storage and independent exports retain the
/// original byte grants. Legacy connection entrypoints keep ConnectedDevice.
/// Existing service/lockdown output allocations are separate SDK boundaries;
/// this type does not certify complete SDK heap or process RSS accounting.
pub struct OwnedConnectedDevice {
    inner: ConnectedDevice,
}
impl OwnedConnectedDevice {
    pub(super) fn new(inner: ConnectedDevice) -> Self {
        Self { inner }
    }
    pub fn info(&self) -> &DeviceInfo {
        &self.inner.info
    }
    pub fn rsd(&self) -> Option<&OwnedRsdHandshake> {
        self.inner.owned_initial_rsd.as_ref()
    }
    pub fn into_rsd(self) -> Option<OwnedRsdHandshake> {
        self.inner.owned_initial_rsd
    }
    /// Original independently admitted directory bytes, for avoiding duplicate
    /// global admission while still counting the published metadata ceiling.
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
    pub fn existing_trust_device(
        &self,
        owner: crate::session::SessionOwner,
        endpoint: crate::session::UsbmuxEndpoint,
    ) -> Result<crate::session::ExistingTrustDevice, CoreError> {
        self.inner.existing_trust_device(owner, endpoint)
    }
    pub fn current_rsd_catalogue(&self) -> Option<&super::PreparedRsdCatalogue> {
        self.inner.current_rsd_catalogue()
    }
    pub fn prepared_rsd_owner(&self) -> Option<&super::PreparedRsdOwner> {
        self.inner.owned_prepared_rsd.as_ref()
    }
    pub fn swap_current_rsd_catalogue(
        &mut self,
        runtime: &Arc<TunnelHandle>,
        proposed: Option<super::PreparedRsdOwner>,
    ) -> Result<Option<super::PreparedRsdOwner>, CoreError> {
        if !self
            .inner
            .tunnel
            .as_ref()
            .is_some_and(|original| Arc::ptr_eq(original, runtime))
        {
            return Err(CoreError::Protocol(
                "RSD refresh runtime identity changed".into(),
            ));
        }
        if proposed
            .as_ref()
            .is_some_and(|catalogue| catalogue.udid() != self.inner.info.udid)
        {
            return Err(CoreError::Protocol(
                "RSD refresh device identity changed".into(),
            ));
        }
        Ok(std::mem::replace(
            &mut self.inner.owned_prepared_rsd,
            proposed,
        ))
    }
    pub fn retained_metadata_bytes(&self) -> Option<usize> {
        self.inner.retained_metadata_bytes()
    }
    pub fn initial_rsd_probe(&self) -> &super::InitialRsdProbe {
        self.inner.initial_rsd_probe()
    }
    pub fn tunnel_handle(&self) -> Option<&Arc<TunnelHandle>> {
        self.inner.tunnel_handle()
    }
    pub fn server_address(&self) -> Option<&str> {
        self.inner.server_address()
    }
    pub fn client_address(&self) -> Option<&str> {
        self.inner.client_address()
    }
    pub fn userspace_port(&self) -> Option<u16> {
        self.inner.userspace_port()
    }
    pub fn rsd_port(&self) -> Option<u16> {
        self.inner.rsd_port()
    }
    pub async fn connect_service(&self, service_name: &str) -> Result<ServiceStream, CoreError> {
        self.inner.connect_service(service_name).await
    }
    pub async fn product_version(&self) -> Result<semver::Version, CoreError> {
        self.inner.product_version().await
    }
    pub async fn lockdown_get_value(&self, key: Option<&str>) -> Result<plist::Value, CoreError> {
        self.inner.lockdown_get_value(key).await
    }
    pub async fn lockdown_get_value_in_domain(
        &self,
        domain: Option<&str>,
        key: Option<&str>,
    ) -> Result<plist::Value, CoreError> {
        self.inner.lockdown_get_value_in_domain(domain, key).await
    }
    pub async fn lockdown_get_value_preserving_errors(
        &self,
        key: Option<&str>,
    ) -> Result<plist::Value, CoreError> {
        self.inner.lockdown_get_value_preserving_errors(key).await
    }
    pub async fn lockdown_get_value_in_domain_preserving_errors(
        &self,
        domain: Option<&str>,
        key: Option<&str>,
    ) -> Result<plist::Value, CoreError> {
        self.inner
            .lockdown_get_value_in_domain_preserving_errors(domain, key)
            .await
    }
    pub async fn lockdown_set_value(
        &self,
        key: Option<&str>,
        value: plist::Value,
    ) -> Result<(), CoreError> {
        self.inner.lockdown_set_value(key, value).await
    }
    pub async fn lockdown_set_value_in_domain(
        &self,
        domain: Option<&str>,
        key: Option<&str>,
        value: plist::Value,
    ) -> Result<(), CoreError> {
        self.inner
            .lockdown_set_value_in_domain(domain, key, value)
            .await
    }
    pub async fn lockdown_remove_value(&self, key: Option<&str>) -> Result<(), CoreError> {
        self.inner.lockdown_remove_value(key).await
    }
    pub async fn lockdown_remove_value_in_domain(
        &self,
        domain: Option<&str>,
        key: Option<&str>,
    ) -> Result<(), CoreError> {
        self.inner
            .lockdown_remove_value_in_domain(domain, key)
            .await
    }
    #[cfg(feature = "mobileactivation")]
    pub async fn activate(
        &self,
        activation_record: &[u8],
        response_headers: &std::collections::BTreeMap<String, String>,
    ) -> Result<(), CoreError> {
        self.inner
            .activate(activation_record, response_headers)
            .await
    }
    #[cfg(feature = "mobileactivation")]
    pub async fn deactivate(&self) -> Result<(), CoreError> {
        self.inner.deactivate().await
    }
    #[cfg(feature = "mobileactivation")]
    pub async fn itunes_activate(&self) -> Result<(), CoreError> {
        self.inner.itunes_activate().await
    }
    pub async fn lockdown_international_configuration(
        &self,
    ) -> Result<InternationalConfiguration, CoreError> {
        self.inner.lockdown_international_configuration().await
    }
    pub async fn connect_rsd_service(
        &self,
        service_name: &str,
    ) -> Result<ServiceStream, CoreError> {
        self.inner.connect_rsd_service(service_name).await
    }
    pub async fn connect_xpc_service(&self, service_name: &str) -> Result<XpcClient, CoreError> {
        self.inner.connect_xpc_service(service_name).await
    }
    pub async fn connect_xpc_service_with_metadata(
        &self,
        service_name: &str,
    ) -> Result<(XpcClient, ResolvedServiceMetadata), CoreError> {
        self.inner
            .connect_xpc_service_with_metadata(service_name)
            .await
    }
}
