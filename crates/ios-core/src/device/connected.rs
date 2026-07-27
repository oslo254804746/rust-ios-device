impl ConnectedDevice {
    /// The RSD handshake result, if available (iOS 17+ with tunnel).
    pub fn rsd(&self) -> Option<&RsdHandshake> {
        self.rsd.as_ref()
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
    /// Suitable for DTX-based services like `com.apple.instruments.dtservicehub`.
    /// Supports userspace proxy and direct IPv6/kernel tunnel connections.
    /// Performs an on-demand RSD handshake if rsd is not already populated.
    pub async fn connect_rsd_service(
        &self,
        service_name: &str,
    ) -> Result<ServiceStream, CoreError> {
        let (resolved_service_name, port) =
            self.resolve_rsd_service_with_retry(service_name).await?;

        let mut stream = self.connect_tunnel_port(port).await?;
        if resolved_service_name.ends_with(".shim.remote") {
            rsd_checkin(&mut stream).await?;
        }
        Ok(stream)
    }

    /// Connect to an iOS 17+ XPC service via RSD.
    ///
    /// Returns an XpcClient ready for method calls.
    /// Performs an on-demand RSD handshake if rsd is not already populated.
    #[cfg(feature = "tunnel")]
    pub async fn connect_xpc_service(&self, service_name: &str) -> Result<XpcClient, CoreError> {
        let (_resolved_service_name, port) =
            self.resolve_rsd_service_with_retry(service_name).await?;
        let stream = self.connect_tunnel_port(port).await?;

        XpcClient::connect_stream(stream)
            .await
            .map_err(CoreError::from)
    }

    async fn resolve_rsd_service_with_retry(
        &self,
        service_name: &str,
    ) -> Result<(String, u16), CoreError> {
        if let Some(rsd) = self.rsd.as_ref() {
            return resolve_rsd_service(rsd, service_name).ok_or_else(|| {
                CoreError::Unsupported(format!(
                    "service '{service_name}' not found in RSD directory"
                ))
            });
        }

        let rsd = self.resolve_rsd_with_retry().await?;
        resolve_rsd_service(&rsd, service_name).ok_or_else(|| {
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

#[derive(serde::Serialize)]
#[serde(rename_all = "PascalCase")]
struct RsdCheckinRequest {
    label: &'static str,
    protocol_version: &'static str,
    request: &'static str,
}

fn resolve_rsd_service(rsd: &RsdHandshake, requested_service: &str) -> Option<(String, u16)> {
    if let Some(ServiceDescriptor { port }) = rsd.services.get(requested_service) {
        return Some((requested_service.to_string(), *port));
    }

    let shim_service = format!("{requested_service}.shim.remote");
    rsd.services
        .get(&shim_service)
        .map(|ServiceDescriptor { port }| (shim_service, *port))
}

fn validate_rsd_checkin_response(
    response: plist::Value,
    expected_request: &str,
    context: &str,
) -> Result<(), CoreError> {
    let response = response.as_dictionary().ok_or_else(|| {
        CoreError::Protocol(format!(
            "{context} expected plist dictionary response, got {:?}",
            response
        ))
    })?;

    let actual_request = response
        .get("Request")
        .and_then(plist::Value::as_string)
        .ok_or_else(|| {
            CoreError::Protocol(format!(
                "{context} missing Request field in response: {:?}",
                response
            ))
        })?;

    if actual_request != expected_request {
        return Err(CoreError::Protocol(format!(
            "{context} expected Request={expected_request}, got {actual_request}"
        )));
    }

    if let Some(error) = response.get("Error") {
        return Err(CoreError::Protocol(format!(
            "{context} failed with Error={:?}",
            error
        )));
    }

    Ok(())
}

async fn rsd_checkin<S>(stream: &mut S) -> Result<(), CoreError>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    send_lockdown(
        stream,
        &RsdCheckinRequest {
            label: "ios-rs",
            protocol_version: "2",
            request: "RSDCheckin",
        },
    )
    .await?;

    let checkin_response: plist::Value = recv_lockdown(stream).await?;
    validate_rsd_checkin_response(checkin_response, "RSDCheckin", "RSD check-in response")?;

    let start_service_response: plist::Value = recv_lockdown(stream).await?;
    validate_rsd_checkin_response(
        start_service_response,
        "StartService",
        "RSD start-service response",
    )?;
    Ok(())
}

// ── connect() ─────────────────────────────────────────────────────────────────
