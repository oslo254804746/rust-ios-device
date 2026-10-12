use anyhow::Result;
use ios_core::device::{ExistingRsdRoute, ExistingRsdSession};
use std::{net::SocketAddrV6, time::Duration};

pub(super) async fn run(
    udid: &str,
    address: SocketAddrV6,
    proxy_port: Option<u16>,
    timeout: u64,
    json: bool,
) -> Result<()> {
    anyhow::ensure!(
        !udid.is_empty() && udid.len() <= 4096 && (1..=60).contains(&timeout),
        crate::output::input_decision(
            "RSD observer requires selected target and timeout 1..60 seconds"
        )
    );
    let route = match proxy_port {
        Some(port) => ExistingRsdRoute::via_userspace_proxy(address, port)?,
        None => ExistingRsdRoute::direct(address)?,
    };
    let session = ExistingRsdSession::observe(route, Duration::from_secs(timeout)).await?;
    let primary = (|| -> Result<serde_json::Value> {
        let observation = session.observation();
        anyhow::ensure!(
            observation.udid() == Some(udid),
            ios_core::CoreError::Protocol(
                "current RSD identity does not match selected target".into()
            )
        );
        anyhow::ensure!(
            observation.complete_service_directory(),
            ios_core::CoreError::Protocol("current RSD catalogue metadata incomplete".into())
        );
        let mut services = observation.services().expect("complete catalogue").iter()
            .map(|(name, service)| serde_json::json!({"name":name,"port":service.port(),"features":service.features()}))
            .collect::<Vec<_>>();
        services.sort_by(|left, right| left["name"].as_str().cmp(&right["name"].as_str()));
        Ok(serde_json::json!({
            "schema_version":1,"selected_udid":udid,"observed_udid":observation.udid(),
            "handshake_mode":session.selection().handshake_mode,
            "attempted_modes":session.selection().attempted_modes,
            "transport_scope":"existing-rsd-socket","services":services,
            "directory_socket_released":true,"directory_release_evidence":"local-raii",
            "cleanup_complete":true,
        }))
    })();
    // The RSD framer/socket is already dropped before the session is returned.
    // This session opened no service or native tunnel; close seals admission.
    let cleanup = session.close(Duration::from_secs(2)).await;
    let value = match (primary, cleanup) {
        (Ok(value), Ok(_)) => value,
        (Err(primary), Ok(_)) => return Err(primary),
        (Ok(_), Err(cleanup)) => return Err(cleanup.into()),
        (Err(primary), Err(cleanup)) => return Err(ObservationFailure { primary, cleanup }.into()),
    };
    if json {
        crate::output::outln!("{}", serde_json::to_string_pretty(&value)?);
    } else {
        crate::output::outln!(
            "RSD bootstrap: {}",
            value["handshake_mode"].as_str().unwrap_or("unknown")
        );
        for service in value["services"].as_array().expect("observer service list") {
            crate::output::outln!(
                "{} {}",
                service["name"].as_str().unwrap_or(""),
                service["port"]
            );
        }
    }
    Ok(())
}

struct ObservationFailure {
    primary: anyhow::Error,
    cleanup: ios_core::CoreError,
}
impl std::fmt::Display for ObservationFailure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("RSD observation and cleanup failed")
    }
}
impl std::fmt::Debug for ObservationFailure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RsdObservationFailure")
            .field(
                "primary_class",
                &ios_core::classify_error(self.primary.as_ref()),
            )
            .field("cleanup_class", &self.cleanup.class())
            .finish()
    }
}
impl std::error::Error for ObservationFailure {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        Some(self.primary.as_ref())
    }
}
