//! Service access on an already connected device.

use super::lockdown_transport::connect_lockdown_port;
#[cfg(not(feature = "tunnel"))]
use super::lockdown_transport::tunnel_unavailable;
#[cfg(feature = "tunnel")]
use super::rsd_connect::{attempt_rsd, attempt_rsd_via_proxy};
#[cfg(feature = "tunnel")]
use super::service_routing::ensure_remote_xpc_service;
use super::service_routing::{
    resolve_rsd_service_details, rsd_checkin_with_timeout, ResolvedRsdService, RSD_CHECKIN_TIMEOUT,
};
use super::support::{plist_value_to_string, plist_value_to_string_vec};
#[cfg(feature = "tunnel")]
use super::tunnel_endpoint::TunnelEndpoint;
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

impl ConnectedDevice {
    /// The RSD handshake result, if available (iOS 17+ with tunnel).
    pub fn rsd(&self) -> Option<&RsdHandshake> {
        self.rsd.as_ref()
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
                .map_err(|error| CoreError::Other(format!("mobile activation: {error}")))?;
        }
        self.lockdown_set_value(
            Some(crate::services::mobileactivation::ACTIVATION_STATE_ACKNOWLEDGED_KEY),
            plist::Value::Boolean(true),
        )
        .await
    }

    /// Deactivate through mobileactivationd, falling back to legacy lockdown.
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
                .map_err(|error| CoreError::Other(format!("mobile activation: {error}")))
        }
        .await;
        if service_result.is_ok() {
            return service_result;
        }

        let service_error = service_result.expect_err("checked above");
        let mut lockdown = self.lockdown_client().await?;
        lockdown.deactivate().await.map_err(|fallback_error| {
            CoreError::Other(format!(
                "mobileactivationd deactivation failed ({service_error}); lockdown fallback failed ({fallback_error})"
            ))
        })
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
        let resolved = self.resolve_rsd_service_with_retry(service_name).await?;

        let mut stream = self.connect_tunnel_port(resolved.port).await?;
        if resolved.resolved_service_name.ends_with(".shim.remote") {
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
        self.connect_xpc_service_with_metadata(service_name)
            .await
            .map(|(client, _metadata)| client)
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
        let stream = self.connect_tunnel_port(resolved.port).await?;

        let client = XpcClient::connect_stream(stream)
            .await
            .map_err(CoreError::from)?;
        let endpoint = self.tunnel_endpoint()?;
        let port = resolved.port;
        let client = client.with_reconnector(move || async move {
            let stream = endpoint
                .connect(port)
                .await
                .map_err(|error| crate::xpc::XpcError::Tls(error.to_string()))?;
            XpcClient::connect_stream(stream).await
        });

        Ok((
            client,
            ResolvedServiceMetadata {
                resolved_service_name: resolved.resolved_service_name,
                features: resolved.features,
            },
        ))
    }

    async fn resolve_rsd_service_with_retry(
        &self,
        service_name: &str,
    ) -> Result<ResolvedRsdService, CoreError> {
        if let Some(rsd) = self.rsd.as_ref() {
            return resolve_rsd_service_details(rsd, service_name).ok_or_else(|| {
                CoreError::Unsupported(format!(
                    "service '{service_name}' not found in RSD directory"
                ))
            });
        }

        let rsd = self.resolve_rsd_with_retry().await?;
        resolve_rsd_service_details(&rsd, service_name).ok_or_else(|| {
            CoreError::Unsupported(format!(
                "service '{service_name}' not found in RSD directory"
            ))
        })
    }

    async fn resolve_rsd_with_retry(&self) -> Result<RsdHandshake, CoreError> {
        #[cfg(not(feature = "tunnel"))]
        {
            Err(tunnel_unavailable())
        }

        #[cfg(feature = "tunnel")]
        {
            const MAX_ATTEMPTS: usize = 5;

            if self.tunnel.is_none() {
                return Err(CoreError::Unsupported(
                    "RSD not available (no tunnel or iOS <17)".into(),
                ));
            }

            for attempt in 0..MAX_ATTEMPTS {
                if attempt > 0 {
                    tokio::time::sleep(std::time::Duration::from_millis(500)).await;
                }

                if let Some(rsd) = self.attempt_rsd_from_tunnel().await? {
                    return Ok(rsd);
                }

                tracing::debug!(
                    "RSD handshake attempt {}/{} failed, retrying...",
                    attempt + 1,
                    MAX_ATTEMPTS
                );
            }

            Err(CoreError::Unsupported(
                "RSD handshake failed after retries".into(),
            ))
        }
    }

    #[cfg(feature = "tunnel")]
    async fn attempt_rsd_from_tunnel(&self) -> Result<Option<RsdHandshake>, CoreError> {
        let server_addr = self
            .server_address()
            .ok_or_else(|| CoreError::Unsupported("no server address".into()))?;
        let rsd_port = self
            .rsd_port()
            .ok_or_else(|| CoreError::Unsupported("no RSD port from tunnel info".into()))?;

        Ok(match self.userspace_port() {
            Some(proxy_port) => attempt_rsd_via_proxy(proxy_port, server_addr, rsd_port).await,
            None => attempt_rsd(server_addr, rsd_port).await,
        }
        .into_directory())
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
}
