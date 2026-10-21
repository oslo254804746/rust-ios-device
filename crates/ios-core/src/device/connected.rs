//! Service access on an already connected device.

use super::lockdown_transport::connect_lockdown_port;
#[cfg(not(feature = "tunnel"))]
use super::lockdown_transport::tunnel_unavailable;
#[cfg(feature = "tunnel")]
use super::rsd_connect::{attempt_rsd, attempt_rsd_via_proxy};
#[cfg(feature = "tunnel")]
use super::rsd_observation::RsdProbeOutcome;
#[cfg(feature = "tunnel")]
use super::service_routing::ensure_remote_xpc_service;
#[cfg(feature = "tunnel")]
use super::service_routing::ResolvedRsdService;
use super::service_routing::{
    rsd_checkin_with_timeout, with_rsd_service_details, RSD_CHECKIN_TIMEOUT,
};
use super::support::{plist_value_to_string, plist_value_to_string_vec};
#[cfg(feature = "tunnel")]
use super::tunnel_endpoint::{connection_error_to_xpc, TunnelEndpoint};
#[cfg(feature = "tunnel")]
use super::ResolvedServiceMetadata;
use super::{ConnectedDevice, InternationalConfiguration, ServiceStream};
use crate::error::CoreError;
use crate::lockdown::pair_record::PairRecord;
use crate::lockdown::session::{start_lockdown_session, start_service, wrap_service_tls};
use crate::lockdown::LOCKDOWN_PORT;
use crate::tunnel::manager::TunnelHandle;
use crate::xpc::rsd::RsdHandshake;
#[cfg(feature = "tunnel")]
use crate::xpc::XpcClient;
use std::sync::Arc;

fn should_strip_service_ssl(service_name: &str) -> bool {
    matches!(
        service_name,
        "com.apple.instruments.remoteserver" | "com.apple.accessibility.axAuditDaemon.remoteserver"
    )
}

#[cfg(feature = "tunnel")]
async fn retry_rsd<T, F, Fut>(mut probe: F) -> Result<T, CoreError>
where
    F: FnMut() -> Fut,
    Fut: std::future::Future<Output = Result<RsdProbeOutcome<T>, CoreError>>,
{
    const MAX_ATTEMPTS: usize = 5;
    let mut last_error = None;
    for attempt in 0..MAX_ATTEMPTS {
        if attempt > 0 {
            tokio::time::sleep(std::time::Duration::from_millis(500)).await;
        }
        // Owner preflight errors still fail immediately; completed probes use
        // the existing five-attempt schedule and retain their final failure.
        match probe().await?.into_result() {
            Ok(directory) => return Ok(directory),
            Err(error) => last_error = Some(error),
        }
        tracing::debug!(
            "RSD handshake attempt {}/{} failed, retrying...",
            attempt + 1,
            MAX_ATTEMPTS
        );
    }
    Err(last_error.expect("nonzero RSD retry count"))
}

#[cfg(feature = "tunnel")]
fn with_owned_service<T>(
    directory: &crate::OwnedRsdHandshake,
    name: &str,
    project: impl FnOnce(&str, u16, &mut dyn Iterator<Item = &str>) -> T,
) -> Result<T, CoreError> {
    let service = directory
        .get_resolved_service(name)
        .ok_or_else(|| CoreError::Xpc(crate::XpcError::ServiceNotFound(name.into())))?;
    let mut features = service.features();
    Ok(project(service.name(), service.port(), &mut features))
}

impl ConnectedDevice {
    /// Snapshot already loaded trust for the bounded read-only SDK profile.
    /// Each job rediscovers this UDID through the explicit mux endpoint. Direct
    /// network lockdown routes are not silently converted into a different route.
    pub fn existing_trust_device(
        &self,
        owner: crate::session::SessionOwner,
        endpoint: crate::session::UsbmuxEndpoint,
    ) -> Result<crate::session::ExistingTrustDevice, CoreError> {
        if !matches!(
            self.lockdown_transport,
            super::LockdownTransport::Usbmux { .. }
        ) {
            return Err(CoreError::Unsupported(
                "checked SDK service profile requires an existing usbmux route".into(),
            ));
        }
        crate::session::ExistingTrustDevice::new(
            self.info.udid.clone(),
            Arc::clone(self.pair_record()?),
            owner,
            endpoint,
        )
    }
    /// The RSD handshake result, if available (iOS 17+ with tunnel).
    pub fn rsd(&self) -> Option<&RsdHandshake> {
        self.rsd.as_ref()
    }

    /// Current admitted refresh catalogue. rsd() remains the historical initial
    /// legacy snapshot; service connection resolution prefers this current view.
    #[cfg(feature = "tunnel")]
    pub fn current_rsd_catalogue(&self) -> Option<&super::PreparedRsdCatalogue> {
        self.owned_prepared_rsd
            .as_deref()
            .or(self.prepared_rsd.as_deref())
    }

    #[cfg(feature = "tunnel")]
    pub fn prepared_rsd_owner(&self) -> Option<&Arc<super::PreparedRsdCatalogue>> {
        self.prepared_rsd.as_ref()
    }

    /// Synchronous swap for an exclusive device owner. This does not stop or
    /// join a tunnel; the caller must coordinate publication/fees/epoch fences.
    #[cfg(feature = "tunnel")]
    pub fn swap_current_rsd_catalogue(
        &mut self,
        runtime: &Arc<TunnelHandle>,
        proposed: Option<Arc<super::PreparedRsdCatalogue>>,
    ) -> Result<Option<Arc<super::PreparedRsdCatalogue>>, CoreError> {
        if !self
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
            .is_some_and(|catalogue| catalogue.udid() != self.info.udid)
        {
            return Err(CoreError::Protocol(
                "RSD refresh device identity changed".into(),
            ));
        }
        Ok(std::mem::replace(&mut self.prepared_rsd, proposed))
    }

    /// Conservatively charge owned directory/identity/credential buffer
    /// capacities for manager admission. Shared credentials are charged once
    /// per owner. Opaque native error causes and tunnel runtime buffers are
    /// separate; this is not a process RSS estimate or their byte bound.
    pub fn retained_metadata_bytes(&self) -> Option<usize> {
        let mut bytes = std::mem::size_of::<Self>();
        let mut add = |amount: usize| -> Option<()> {
            bytes = bytes.checked_add(amount)?;
            Some(())
        };
        add(self.info.udid.capacity())?;
        add(self.info.connection_type.capacity())?;
        if let super::LockdownTransport::Tcp { host } = &self.lockdown_transport {
            add(host.capacity())?;
        }
        if let Some(tunnel) = &self.tunnel {
            add(std::mem::size_of::<crate::tunnel::manager::TunnelHandle>())?;
            add(tunnel.udid.capacity())?;
            add(tunnel.info.server_address.capacity())?;
            add(tunnel.info.client_address.capacity())?;
        }
        if let Some(rsd) = &self.rsd {
            add(rsd.udid.capacity())?;
            add(rsd.services.capacity().checked_mul(
                std::mem::size_of::<(String, crate::xpc::rsd::ServiceDescriptor)>() + 16,
            )?)?;
            for (name, descriptor) in &rsd.services {
                add(name.capacity())?;
                add(descriptor
                    .features
                    .capacity()
                    .checked_mul(std::mem::size_of::<String>())?)?;
                for feature in &descriptor.features {
                    add(feature.capacity())?;
                }
            }
        }
        #[cfg(feature = "tunnel")]
        if let Some(directory) = &self.owned_initial_rsd {
            add(directory.known_bytes())?;
        }
        if let Some(pair) = &self.pair_record {
            add(std::mem::size_of::<PairRecord>())?;
            for buffer in [
                &pair.device_certificate,
                &pair.host_certificate,
                &pair.root_certificate,
                &*pair.host_private_key,
            ] {
                add(buffer.capacity())?;
            }
            add(pair.host_id.capacity())?;
            add(pair.system_buid.capacity())?;
            if let Some(mac) = &pair.wifi_mac_address {
                add(mac.capacity())?;
            }
        }
        Some(bytes)
    }

    /// Historical optional probe performed during connection establishment.
    /// Reading this performs no IO and never retries. Later on-demand service
    /// discovery does not update this initial observation or the legacy cache.
    pub fn initial_rsd_probe(&self) -> &super::InitialRsdProbe {
        &self.initial_rsd_probe
    }

    /// Take ownership of the RSD handshake, consuming it from the device.
    pub fn into_rsd(self) -> Option<RsdHandshake> {
        self.rsd
    }

    /// The tunnel handle, if a tunnel is active.
    pub fn tunnel_handle(&self) -> Option<&Arc<TunnelHandle>> {
        self.tunnel.as_ref()
    }

    pub fn server_address(&self) -> Option<&str> {
        self.tunnel.as_ref().map(|t| t.info.server_address.as_str())
    }

    /// The host-side IPv6 address assigned by the CDTunnel handshake.
    ///
    /// Device-initiated datagrams (for example CoreDevice media RTP) must be
    /// advertised with this address when a kernel tunnel is active. Loopback
    /// (`::1`) is local to the host and is not a routable device endpoint.
    pub fn client_address(&self) -> Option<&str> {
        self.tunnel.as_ref().map(|t| t.info.client_address.as_str())
    }

    pub fn userspace_port(&self) -> Option<u16> {
        self.tunnel.as_ref().and_then(|t| t.userspace_port)
    }

    pub fn rsd_port(&self) -> Option<u16> {
        self.tunnel.as_ref().map(|t| t.info.server_rsd_port)
    }

    fn pair_record(&self) -> Result<&Arc<PairRecord>, CoreError> {
        self.pair_record
            .as_ref()
            .ok_or_else(|| CoreError::Unsupported("no pair record loaded".into()))
    }

    async fn lockdown_client(&self) -> Result<crate::lockdown::LockdownClient, CoreError> {
        let pair_record = self.pair_record()?;
        let stream = connect_lockdown_port(
            &self.info.udid,
            &self.lockdown_transport,
            LOCKDOWN_PORT,
            true,
        )
        .await?;
        crate::lockdown::LockdownClient::connect_with_stream(stream, pair_record)
            .await
            .map_err(CoreError::from)
    }

    /// Open a lockdown service stream (iOS <17 or iOS 17+ services also accessible via lockdown).
    pub async fn connect_service(&self, service_name: &str) -> Result<ServiceStream, CoreError> {
        let pair_record = self.pair_record()?;
        let lockdown_stream = connect_lockdown_port(
            &self.info.udid,
            &self.lockdown_transport,
            LOCKDOWN_PORT,
            true,
        )
        .await?;

        let (_session_id, mut tls_reader, mut tls_writer) =
            start_lockdown_session(lockdown_stream, pair_record).await?;

        let (port, enable_ssl) =
            start_service(&mut tls_reader, &mut tls_writer, service_name).await?;

        let svc_stream =
            connect_lockdown_port(&self.info.udid, &self.lockdown_transport, port, false).await?;

        if enable_ssl {
            let tls = wrap_service_tls(svc_stream, pair_record).await?;
            if should_strip_service_ssl(service_name) {
                let stream = crate::lockdown::session::strip_service_tls(tls)?;
                Ok(Box::new(stream))
            } else {
                Ok(Box::new(tls))
            }
        } else {
            Ok(Box::new(svc_stream))
        }
    }

    /// Get the device's iOS version via lockdown.
    pub async fn product_version(&self) -> Result<semver::Version, CoreError> {
        let mut client = self.lockdown_client().await?;
        let ver = client.product_version().await?;
        Ok(ver)
    }

    /// Get a lockdown value by key (domain=None for global domain).
    pub async fn lockdown_get_value(&self, key: Option<&str>) -> Result<plist::Value, CoreError> {
        self.lockdown_get_value_in_domain(None, key).await
    }

    /// Get a lockdown value by optional domain and key.
    pub async fn lockdown_get_value_in_domain(
        &self,
        domain: Option<&str>,
        key: Option<&str>,
    ) -> Result<plist::Value, CoreError> {
        let mut client = self.lockdown_client().await?;
        client.get_value(domain, key).await.map_err(CoreError::from)
    }

    /// Get a value with typed device Error-field observations.
    pub async fn lockdown_get_value_preserving_errors(
        &self,
        key: Option<&str>,
    ) -> Result<plist::Value, CoreError> {
        self.lockdown_get_value_in_domain_preserving_errors(None, key)
            .await
    }

    /// Get a domain value with the preserving Lockdown response-error policy.
    /// This does not change connection establishment or guarantee safe retry.
    pub async fn lockdown_get_value_in_domain_preserving_errors(
        &self,
        domain: Option<&str>,
        key: Option<&str>,
    ) -> Result<plist::Value, CoreError> {
        let mut client = self.lockdown_client().await?;
        client
            .get_value_preserving_errors(domain, key)
            .await
            .map_err(CoreError::from)
    }

    /// Set a lockdown value by key (domain=None for global domain).
    pub async fn lockdown_set_value(
        &self,
        key: Option<&str>,
        value: plist::Value,
    ) -> Result<(), CoreError> {
        self.lockdown_set_value_in_domain(None, key, value).await
    }

    /// Set a lockdown value by optional domain and key.
    pub async fn lockdown_set_value_in_domain(
        &self,
        domain: Option<&str>,
        key: Option<&str>,
        value: plist::Value,
    ) -> Result<(), CoreError> {
        let mut client = self.lockdown_client().await?;
        client
            .set_value(domain, key, value)
            .await
            .map_err(CoreError::from)
    }

    /// Remove a lockdown value by key (domain=None for global domain).
    pub async fn lockdown_remove_value(&self, key: Option<&str>) -> Result<(), CoreError> {
        self.lockdown_remove_value_in_domain(None, key).await
    }

    /// Remove a lockdown value by optional domain and key.
    pub async fn lockdown_remove_value_in_domain(
        &self,
        domain: Option<&str>,
        key: Option<&str>,
    ) -> Result<(), CoreError> {
        let mut client = self.lockdown_client().await?;
        client
            .remove_value(domain, key)
            .await
            .map_err(CoreError::from)
    }

    /// Apply an activation record using the protocol selected by its shape.
    ///
    /// Wrapped `iphone-activation`/`device-activation` records use legacy
    /// lockdown `Activate`; modern records use the session-bound
    /// `HandleActivationInfoWithSessionRequest` mobileactivationd command.
    /// The distinction is made before opening the service so a legacy record
    /// is never sent in a modern envelope.
    #[cfg(feature = "mobileactivation")]
    pub async fn activate(
        &self,
        activation_record: &[u8],
        response_headers: &std::collections::BTreeMap<String, String>,
    ) -> Result<(), CoreError> {
        let has_legacy_wrapper =
            crate::services::mobileactivation::has_legacy_activation_wrapper(activation_record);
        if has_legacy_wrapper {
            let legacy_record =
                crate::services::mobileactivation::extract_legacy_activation_record(
                    activation_record,
                )
                .ok_or_else(|| {
                    CoreError::Protocol(
                        "legacy activation response is missing activation-record".into(),
                    )
                })?;
            let mut lockdown = self.lockdown_client().await?;
            lockdown
                .activate(legacy_record)
                .await
                .map_err(CoreError::from)?;
        } else {
            let stream = self
                .connect_service(crate::services::mobileactivation::SERVICE_NAME)
                .await?;
            let mut client = crate::services::mobileactivation::MobileActivationClient::new(stream);
            client
                .activate(activation_record, response_headers)
                .await
                .map_err(super::activation_error::service_error)?;
        }
        self.lockdown_set_value(
            Some(crate::services::mobileactivation::ACTIVATION_STATE_ACKNOWLEDGED_KEY),
            plist::Value::Boolean(true),
        )
        .await
    }

    /// Deactivate through mobileactivationd, falling back to legacy lockdown.
    ///
    /// If both attempts fail, the returned `CoreError::Io` retains a
    /// [`super::DeactivationFallbackError`] payload with borrowed causes and
    /// the final fallback's category. Success does not expose an earlier
    /// failure. Neither classification nor a retained cause authorizes retry.
    #[cfg(feature = "mobileactivation")]
    pub async fn deactivate(&self) -> Result<(), CoreError> {
        let service_result = async {
            let stream = self
                .connect_service(crate::services::mobileactivation::SERVICE_NAME)
                .await?;
            let mut client = crate::services::mobileactivation::MobileActivationClient::new(stream);
            client
                .deactivate()
                .await
                .map(|_| ())
                .map_err(super::activation_error::service_error)
        };
        super::deactivation_error::deactivate_with_fallback(service_result, || async {
            let mut lockdown = self.lockdown_client().await?;
            lockdown.deactivate().await.map_err(CoreError::from)
        })
        .await
    }

    /// Record the legacy iTunes activation marker in lockdown.
    #[cfg(feature = "mobileactivation")]
    pub async fn itunes_activate(&self) -> Result<(), CoreError> {
        self.lockdown_set_value(
            Some(crate::services::mobileactivation::ITUNES_HAS_CONNECTED_KEY),
            plist::Value::Boolean(true),
        )
        .await
    }

    /// Read language and locale metadata from `com.apple.international`.
    pub async fn lockdown_international_configuration(
        &self,
    ) -> Result<InternationalConfiguration, CoreError> {
        const INTERNATIONAL_DOMAIN: &str = "com.apple.international";

        let mut client = self.lockdown_client().await?;
        let language = client
            .get_value(Some(INTERNATIONAL_DOMAIN), Some("Language"))
            .await?;
        let locale = client
            .get_value(Some(INTERNATIONAL_DOMAIN), Some("Locale"))
            .await?;
        let supported_locales = client
            .get_value(Some(INTERNATIONAL_DOMAIN), Some("SupportedLocales"))
            .await?;
        let supported_languages = client
            .get_value(Some(INTERNATIONAL_DOMAIN), Some("SupportedLanguages"))
            .await?;

        Ok(InternationalConfiguration {
            language: plist_value_to_string(&language, "Language")?,
            locale: plist_value_to_string(&locale, "Locale")?,
            supported_locales: plist_value_to_string_vec(&supported_locales, "SupportedLocales")?,
            supported_languages: plist_value_to_string_vec(
                &supported_languages,
                "SupportedLanguages",
            )?,
        })
    }

    /// Connect to an RSD service as a raw TCP stream (no XPC/H2 framing).
    ///
    /// Suitable for DTX-based services like `com.apple.instruments.dtservicehub`
    /// and RSD lockdown shims. A resolved `.shim.remote` entry receives the
    /// required RSDCheckin exchange before the stream is returned.
    /// Supports userspace proxy and direct IPv6/kernel tunnel connections.
    /// Performs an on-demand RSD handshake if rsd is not already populated.
    pub async fn connect_rsd_service(
        &self,
        service_name: &str,
    ) -> Result<ServiceStream, CoreError> {
        let (port, shim) = self
            .project_rsd_service_with_retry(service_name, |name, port, _features| {
                (port, name.ends_with(".shim.remote"))
            })
            .await?;
        let mut stream = self.connect_tunnel_port(port).await?;
        if shim {
            rsd_checkin_with_timeout(&mut stream, RSD_CHECKIN_TIMEOUT).await?;
        }
        Ok(stream)
    }

    /// Connect to an iOS 17+ XPC service via RSD.
    ///
    /// Returns an XpcClient ready for method calls. This endpoint accepts
    /// canonical RemoteXPC services only; a `.shim.remote` fallback is
    /// rejected because it needs the raw RSDCheckin path.
    /// Performs an on-demand RSD handshake if rsd is not already populated.
    #[cfg(feature = "tunnel")]
    pub async fn connect_xpc_service(&self, service_name: &str) -> Result<XpcClient, CoreError> {
        let port = self
            .project_rsd_service_with_retry(service_name, |name, port, _features| {
                ensure_remote_xpc_service(service_name, name)?;
                Ok::<_, CoreError>(port)
            })
            .await??;
        self.connect_xpc_port(port).await
    }

    /// Connect to an XPC service and return the exact RSD descriptor used for
    /// that connection.
    ///
    /// This performs one RSD lookup (using the cached handshake when present,
    /// otherwise one on-demand handshake) and one service socket connection.
    /// The returned metadata is from that same lookup; callers must not make
    /// a second connection merely to rediscover feature capabilities.
    #[cfg(feature = "tunnel")]
    pub async fn connect_xpc_service_with_metadata(
        &self,
        service_name: &str,
    ) -> Result<(XpcClient, ResolvedServiceMetadata), CoreError> {
        let resolved = self.resolve_rsd_service_with_retry(service_name).await?;
        // RSD entries ending in ".shim.remote" are lockdown-style services
        // exposed through RSD. They require the plist RSDCheckin exchange
        // before their protocol starts; feeding the raw socket to XpcClient
        // would send an H2 preface and can reset or wedge the service. There
        // is no safe conversion from that checked-in stream to this
        // RemoteXPC-only return type, so reject before dialing and direct
        // callers to connect_rsd_service, which performs check-in.
        ensure_remote_xpc_service(service_name, &resolved.resolved_service_name)?;
        let client = self.connect_xpc_port(resolved.port).await?;
        Ok((
            client,
            ResolvedServiceMetadata {
                resolved_service_name: resolved.resolved_service_name,
                features: resolved.features,
            },
        ))
    }

    #[cfg(feature = "tunnel")]
    async fn connect_xpc_port(&self, port: u16) -> Result<XpcClient, CoreError> {
        let stream = self.connect_tunnel_port(port).await?;
        let client = XpcClient::connect_stream(stream)
            .await
            .map_err(CoreError::from)?;
        let endpoint = self.tunnel_endpoint()?;
        let client = client.with_reconnector(move || async move {
            let stream = endpoint
                .connect(port)
                .await
                .map_err(connection_error_to_xpc)?;
            XpcClient::connect_stream(stream).await
        });

        Ok(client)
    }

    #[cfg(feature = "tunnel")]
    async fn resolve_rsd_service_with_retry(
        &self,
        service_name: &str,
    ) -> Result<ResolvedRsdService, CoreError> {
        self.project_rsd_service_with_retry(service_name, |name, port, features| {
            ResolvedRsdService {
                resolved_service_name: name.into(),
                port,
                features: features.map(str::to_owned).collect(),
            }
        })
        .await
    }

    // Project while the original directory is still alive. Plain connections
    // need only the port/shim decision, so no owned name or feature copy exists.
    async fn project_rsd_service_with_retry<T>(
        &self,
        service_name: &str,
        project: impl FnOnce(&str, u16, &mut dyn Iterator<Item = &str>) -> T,
    ) -> Result<T, CoreError> {
        #[cfg(feature = "tunnel")]
        if let Some(rsd) = self.current_rsd_catalogue() {
            return rsd.with_resolved(service_name, project).ok_or_else(|| {
                CoreError::Xpc(crate::XpcError::ServiceNotFound(service_name.into()))
            });
        }
        #[cfg(feature = "tunnel")]
        if let Some(rsd) = self.owned_initial_rsd.as_ref() {
            return with_owned_service(rsd, service_name, project);
        }
        #[cfg(feature = "tunnel")]
        if let Some(bank) = self.owned_initial_bank.as_ref() {
            let rsd = retry_rsd(|| async {
                let server = self
                    .server_address()
                    .ok_or_else(|| CoreError::Unsupported("no server address".into()))?;
                let port = self
                    .rsd_port()
                    .ok_or_else(|| CoreError::Unsupported("no RSD port from tunnel info".into()))?;
                let probe = match self.userspace_port() {
                    Some(proxy) => {
                        super::rsd_connect::attempt_owned_rsd_via_proxy(proxy, server, port, bank)
                            .await
                    }
                    None => super::rsd_connect::attempt_owned_rsd(server, port, bank).await,
                };
                match probe {
                    RsdProbeOutcome::Completed(Err(error))
                        if error.class() == crate::ErrorClass::ResourceLimit =>
                    {
                        Err(error)
                    }
                    probe => Ok(probe),
                }
            })
            .await?;
            return with_owned_service(&rsd, service_name, project);
        }
        if let Some(rsd) = self.rsd.as_ref() {
            return with_rsd_service_details(rsd, service_name, project).ok_or_else(|| {
                CoreError::Xpc(crate::XpcError::ServiceNotFound(service_name.into()))
            });
        }

        let rsd = self.resolve_rsd_with_retry().await?;
        with_rsd_service_details(&rsd, service_name, project)
            .ok_or_else(|| CoreError::Xpc(crate::XpcError::ServiceNotFound(service_name.into())))
    }

    async fn resolve_rsd_with_retry(&self) -> Result<RsdHandshake, CoreError> {
        #[cfg(not(feature = "tunnel"))]
        {
            Err(tunnel_unavailable())
        }

        #[cfg(feature = "tunnel")]
        {
            if self.tunnel.is_none() {
                return Err(CoreError::Unsupported(
                    "RSD not available (no tunnel or iOS <17)".into(),
                ));
            }

            retry_rsd(|| self.attempt_rsd_from_tunnel()).await
        }
    }

    #[cfg(feature = "tunnel")]
    async fn attempt_rsd_from_tunnel(&self) -> Result<RsdProbeOutcome, CoreError> {
        let server_addr = self
            .server_address()
            .ok_or_else(|| CoreError::Unsupported("no server address".into()))?;
        let rsd_port = self
            .rsd_port()
            .ok_or_else(|| CoreError::Unsupported("no RSD port from tunnel info".into()))?;

        Ok(match self.userspace_port() {
            Some(proxy_port) => attempt_rsd_via_proxy(proxy_port, server_addr, rsd_port).await,
            None => attempt_rsd(server_addr, rsd_port).await,
        })
    }

    #[cfg(feature = "tunnel")]
    fn tunnel_endpoint(&self) -> Result<TunnelEndpoint, CoreError> {
        let server_addr = self
            .server_address()
            .ok_or_else(|| CoreError::Unsupported("no server address".into()))?;

        TunnelEndpoint::resolve(server_addr, self.userspace_port())
    }

    async fn connect_tunnel_port(&self, port: u16) -> Result<ServiceStream, CoreError> {
        #[cfg(not(feature = "tunnel"))]
        {
            let _ = port;
            Err(tunnel_unavailable())
        }

        #[cfg(feature = "tunnel")]
        {
            Ok(Box::new(self.tunnel_endpoint()?.connect(port).await?))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strip_ssl_selection_matches_legacy_dtx_services() {
        assert!(should_strip_service_ssl(
            "com.apple.accessibility.axAuditDaemon.remoteserver"
        ));
        assert!(should_strip_service_ssl(
            "com.apple.instruments.remoteserver"
        ));
        assert!(!should_strip_service_ssl(
            "com.apple.instruments.remoteserver.DVTSecureSocketProxy"
        ));
        assert!(!should_strip_service_ssl("com.apple.mobile.screenshotr"));
        assert!(!should_strip_service_ssl("com.apple.webinspector"));
    }

    #[test]
    fn parses_string_array_values_for_international_configuration() {
        let value = plist::Value::Array(vec![
            plist::Value::String("en-US".into()),
            plist::Value::String("zh-Hans".into()),
        ]);

        let parsed = plist_value_to_string_vec(&value, "SupportedLanguages")
            .expect("string array should parse");

        assert_eq!(parsed, vec!["en-US".to_string(), "zh-Hans".to_string()]);
    }

    #[test]
    fn rejects_non_string_entries_in_international_configuration_arrays() {
        let value = plist::Value::Array(vec![plist::Value::Integer(1i64.into())]);

        let err = plist_value_to_string_vec(&value, "SupportedLocales")
            .expect_err("non-string entry should fail");

        let rendered = err.to_string();
        assert!(rendered.contains("SupportedLocales"));
        assert!(rendered.contains("string"));
    }

    #[cfg(feature = "tunnel")]
    #[tokio::test]
    async fn retry_exhaustion_retains_final_typed_source_and_existing_attempt_count() {
        #[derive(thiserror::Error)]
        #[error("synthetic-private-attempt-source")]
        struct AttemptSource(usize);

        impl std::fmt::Debug for AttemptSource {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                f.write_str("AttemptSource")
            }
        }

        let mut attempts = 0;
        let error = retry_rsd(|| {
            attempts += 1;
            std::future::ready(Ok(RsdProbeOutcome::<RsdHandshake>::Completed(Err(
                CoreError::Xpc(crate::XpcError::Io(std::io::Error::new(
                    std::io::ErrorKind::TimedOut,
                    AttemptSource(attempts),
                ))),
            ))))
        })
        .await
        .unwrap_err();
        assert_eq!(attempts, 5);
        assert_eq!(error.class(), crate::ErrorClass::Timeout);
        let CoreError::Xpc(crate::XpcError::Io(source)) = &error else {
            panic!("final native failure lost at retry owner")
        };
        assert_eq!(
            source
                .get_ref()
                .unwrap()
                .downcast_ref::<AttemptSource>()
                .unwrap()
                .0,
            5
        );
        assert!(!format!("{error:?}").contains("synthetic-private"));
    }

    #[cfg(feature = "tunnel")]
    #[tokio::test]
    async fn retry_stops_at_success_and_owner_preflight_error() {
        let mut attempts = 0;
        let directory = retry_rsd(|| {
            attempts += 1;
            std::future::ready(Ok(RsdProbeOutcome::<RsdHandshake>::Completed(
                if attempts == 1 {
                    Err(CoreError::Xpc(crate::XpcError::NotConnected))
                } else {
                    Ok(RsdHandshake {
                        udid: "synthetic-private-later-directory".into(),
                        services: std::collections::HashMap::new(),
                    })
                },
            )))
        })
        .await
        .unwrap();
        assert_eq!(attempts, 2);
        assert_eq!(directory.udid, "synthetic-private-later-directory");

        attempts = 0;
        let error = retry_rsd::<RsdHandshake, _, _>(|| {
            attempts += 1;
            std::future::ready(Err(CoreError::Io(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "synthetic-private-preflight",
            ))))
        })
        .await
        .unwrap_err();
        assert_eq!(attempts, 1);
        assert_eq!(error.class(), crate::ErrorClass::InvalidInput);
    }

    #[cfg(feature = "tunnel")]
    fn owned_rsd_device(
        directory: Option<RsdHandshake>,
        proxy_port: u16,
    ) -> (ConnectedDevice, tokio::sync::watch::Receiver<()>) {
        let probe = RsdProbeOutcome::<RsdHandshake>::Completed(directory.ok_or_else(|| {
            CoreError::Io(std::io::Error::new(
                std::io::ErrorKind::TimedOut,
                "synthetic-private-initial-failure",
            ))
        }));
        let (rsd, initial_rsd_probe) = probe.into_parts();
        let (handle, receiver) = TunnelHandle::new(
            "synthetic-private-device".into(),
            crate::TunnelInfo {
                server_address: "fd00::42".into(),
                server_rsd_port: 58783,
                client_address: "fd00::43".into(),
                client_mtu: 1280,
            },
            Some(proxy_port),
        );
        (
            ConnectedDevice {
                info: crate::DeviceInfo {
                    udid: "synthetic-private-device".into(),
                    device_id: 42,
                    connection_type: "USB".into(),
                    product_id: 0,
                },
                tunnel: Some(Arc::new(handle)),
                rsd,
                #[cfg(feature = "tunnel")]
                owned_initial_rsd: None,
                #[cfg(feature = "tunnel")]
                owned_initial_bank: None,
                #[cfg(feature = "tunnel")]
                prepared_rsd: None,
                #[cfg(feature = "tunnel")]
                owned_prepared_rsd: None,
                initial_rsd_probe,
                pair_record: None,
                lockdown_transport: super::super::LockdownTransport::Usbmux { device_id: 42 },
            },
            receiver,
        )
    }

    #[cfg(feature = "tunnel")]
    #[tokio::test]
    async fn public_service_calls_retain_real_retry_failure_without_rewriting_initial_snapshot() {
        // Reserve a non-listening port; no released-port race, TUN or device IO.
        let socket = tokio::net::TcpSocket::new_v4().unwrap();
        socket.bind("127.0.0.1:0".parse().unwrap()).unwrap();
        let (device, receiver) = owned_rsd_device(None, socket.local_addr().unwrap().port());
        // Winsock's loopback SYN refusal is delivered after ~2 seconds. This
        // public retry path makes five real attempts; observe the native refusal
        // rather than replacing it with this fixture's outer timeout.
        let observation_budget = if cfg!(windows) { 20 } else { 5 };
        let error = tokio::time::timeout(
            std::time::Duration::from_secs(observation_budget),
            device.connect_rsd_service("synthetic.private.service"),
        )
        .await
        .unwrap()
        .err()
        .expect("non-listening owned proxy must fail");
        let CoreError::Xpc(crate::XpcError::Io(source)) = &error else {
            panic!("public service call discarded the retry failure")
        };
        assert_eq!(source.kind(), std::io::ErrorKind::ConnectionRefused);
        assert_eq!(error.class(), crate::ErrorClass::Io);
        assert!(device.rsd().is_none());
        assert_eq!(
            device.initial_rsd_probe().state(),
            crate::InitialRsdProbeState::Failed
        );
        assert_eq!(
            device.initial_rsd_probe().error_class(),
            Some(crate::ErrorClass::Timeout)
        );
        drop(device);
        assert!(receiver.has_changed().is_err());
    }

    #[cfg(feature = "tunnel")]
    #[tokio::test]
    async fn newly_discovered_directory_miss_retains_initial_failure_and_releases_discovery_socket()
    {
        use crate::xpc::message::{decode_message, encode_message, flags, XpcMessage, XpcValue};
        use tokio::io::{AsyncReadExt, AsyncWriteExt};

        let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
            .await
            .unwrap();
        let (device, receiver) = owned_rsd_device(None, listener.local_addr().unwrap().port());
        let peer = async {
            let mut stream = listener.accept().await.unwrap().0;
            let mut prelude = [0; 20];
            stream.read_exact(&mut prelude).await.unwrap();
            assert_eq!(
                &prelude[..16],
                &"fd00::42".parse::<std::net::Ipv6Addr>().unwrap().octets()
            );
            assert_eq!(u32::from_le_bytes(prelude[16..].try_into().unwrap()), 58783);
            let mut preface = [0; 24];
            stream.read_exact(&mut preface).await.unwrap();
            assert_eq!(&preface, b"PRI * HTTP/2.0\r\n\r\nSM\r\n\r\n");
            stream
                .write_all(&[0, 0, 0, 4, 0, 0, 0, 0, 0])
                .await
                .unwrap();
            let mut found_handshake = false;
            for _ in 0..20 {
                let mut header = [0; 9];
                stream.read_exact(&mut header).await.unwrap();
                let length = u32::from_be_bytes([0, header[0], header[1], header[2]]) as usize;
                assert!(length <= 65536);
                let mut payload = vec![0; length];
                stream.read_exact(&mut payload).await.unwrap();
                if header[3] != 0 || u32::from_be_bytes(header[5..].try_into().unwrap()) != 1 {
                    continue;
                }
                let message = decode_message(bytes::Bytes::from(payload)).unwrap();
                if message
                    .body
                    .as_ref()
                    .and_then(XpcValue::as_dict)
                    .and_then(|body| body.get("MessageType"))
                    .and_then(XpcValue::as_str)
                    == Some("Handshake")
                {
                    found_handshake = true;
                    break;
                }
            }
            assert!(found_handshake);
            let response = encode_message(&XpcMessage {
                flags: flags::ALWAYS_SET | flags::DATA,
                msg_id: 1,
                body: Some(XpcValue::Dictionary(indexmap::IndexMap::from([
                    ("MessageType".into(), XpcValue::String("Handshake".into())),
                    (
                        "Properties".into(),
                        XpcValue::Dictionary(indexmap::IndexMap::from([(
                            "UniqueDeviceID".into(),
                            XpcValue::String("synthetic-private-later-directory".into()),
                        )])),
                    ),
                    (
                        "Services".into(),
                        XpcValue::Dictionary(indexmap::IndexMap::new()),
                    ),
                ]))),
            })
            .unwrap();
            let length = u32::try_from(response.len()).unwrap().to_be_bytes();
            stream
                .write_all(&[length[1], length[2], length[3], 0, 0, 0, 0, 0, 1])
                .await
                .unwrap();
            stream.write_all(&response).await.unwrap();
            // The discovery framer must release its transport on completion.
            let mut rest = Vec::new();
            stream.take(65536).read_to_end(&mut rest).await.unwrap();
            assert!(rest.len() < 65536);
            assert!(
                tokio::time::timeout(std::time::Duration::from_millis(20), listener.accept())
                    .await
                    .is_err()
            );
        };
        let ((), error) = tokio::time::timeout(std::time::Duration::from_secs(5), async {
            tokio::join!(
                peer,
                device.connect_rsd_service("synthetic.private.missing")
            )
        })
        .await
        .unwrap();
        let error = error.err().unwrap();
        assert_eq!(error.class(), crate::ErrorClass::ServiceMissing);
        assert!(matches!(
            error,
            CoreError::Xpc(crate::XpcError::ServiceNotFound(_))
        ));
        assert!(device.rsd().is_none());
        assert_eq!(
            device.initial_rsd_probe().state(),
            crate::InitialRsdProbeState::Failed
        );
        assert_eq!(
            device.initial_rsd_probe().error_class(),
            Some(crate::ErrorClass::Timeout)
        );
        drop(device);
        assert!(receiver.has_changed().is_err());
    }

    #[cfg(feature = "tunnel")]
    #[tokio::test]
    async fn successful_directory_miss_is_typed_and_never_dials_a_service() {
        let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
            .await
            .unwrap();
        let directory = RsdHandshake {
            udid: "synthetic-private-directory".into(),
            services: std::collections::HashMap::from([
                ("owned.service".into(), crate::ServiceDescriptor::new(12345)),
                (
                    "owned.shim.shim.remote".into(),
                    crate::ServiceDescriptor::new(23456),
                ),
            ]),
        };
        let (device, receiver) =
            owned_rsd_device(Some(directory), listener.local_addr().unwrap().port());
        let resolved = device
            .resolve_rsd_service_with_retry("owned.service")
            .await
            .unwrap();
        assert_eq!(resolved.port, 12345);
        let shim = device
            .resolve_rsd_service_with_retry("owned.shim")
            .await
            .unwrap();
        assert_eq!(shim.resolved_service_name, "owned.shim.shim.remote");
        for error in [
            device
                .connect_rsd_service("synthetic.private.missing")
                .await
                .err()
                .unwrap(),
            device
                .connect_xpc_service("synthetic.private.missing")
                .await
                .err()
                .unwrap(),
        ] {
            assert_eq!(error.class(), crate::ErrorClass::ServiceMissing);
            assert!(
                matches!(error, CoreError::Xpc(crate::XpcError::ServiceNotFound(ref name)) if name == "synthetic.private.missing")
            );
            assert!(!format!("{error:?}").contains("synthetic.private"));
        }
        assert!(
            tokio::time::timeout(std::time::Duration::from_millis(20), listener.accept())
                .await
                .is_err()
        );
        assert_eq!(
            device.initial_rsd_probe().state(),
            crate::InitialRsdProbeState::Available
        );
        assert_eq!(
            device
                .rsd()
                .unwrap()
                .feature_evidence("owned.service", "missing.feature"),
            crate::RsdFeatureEvidence::MetadataUnavailable
        );
        drop(device);
        assert!(receiver.has_changed().is_err());
    }
}
