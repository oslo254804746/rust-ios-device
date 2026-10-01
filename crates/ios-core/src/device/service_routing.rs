//! RSD descriptor resolution and the lockdown-framed service check-in.
//!
//! Callers receive the exact selected descriptor; this layer does not discover
//! a device, select a connection strategy or manage the tunnel lifecycle.

use crate::error::CoreError;
use crate::lockdown::protocol::{recv_lockdown, send_lockdown};
use crate::xpc::rsd::{RsdHandshake, ServiceDescriptor};
use tokio::io::{AsyncRead, AsyncWrite};

#[derive(serde::Serialize)]
#[serde(rename_all = "PascalCase")]
struct RsdCheckinRequest {
    label: &'static str,
    protocol_version: &'static str,
    request: &'static str,
}

pub(super) const RSD_CHECKIN_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(5);

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct ResolvedRsdService {
    pub(super) resolved_service_name: String,
    pub(super) port: u16,
    pub(super) features: Vec<String>,
}

fn find_rsd_service<'a>(
    rsd: &'a RsdHandshake,
    requested_service: &str,
) -> Option<(&'a String, &'a ServiceDescriptor)> {
    if let Some(entry) = rsd.services.get_key_value(requested_service) {
        return Some(entry);
    }

    let shim_service = format!("{requested_service}.shim.remote");
    rsd.services.get_key_value(&shim_service)
}

pub(super) fn resolve_rsd_service_details(
    rsd: &RsdHandshake,
    requested_service: &str,
) -> Option<ResolvedRsdService> {
    let (resolved_service_name, descriptor) = find_rsd_service(rsd, requested_service)?;
    Some(ResolvedRsdService {
        resolved_service_name: resolved_service_name.clone(),
        port: descriptor.port,
        features: descriptor.features.clone(),
    })
}

#[cfg(test)]
fn resolve_rsd_service(rsd: &RsdHandshake, requested_service: &str) -> Option<(String, u16)> {
    find_rsd_service(rsd, requested_service)
        .map(|(resolved_service_name, descriptor)| (resolved_service_name.clone(), descriptor.port))
}

#[cfg(any(feature = "tunnel", test))]
pub(super) fn ensure_remote_xpc_service(
    requested_service: &str,
    resolved_service: &str,
) -> Result<(), CoreError> {
    if resolved_service.ends_with(".shim.remote") {
        return Err(CoreError::Unsupported(format!(
            "service '{requested_service}' resolved to lockdown shim '{resolved_service}'; it cannot be opened as RemoteXPC/XPC. Use connect_rsd_service(\"{requested_service}\") for the RSDCheckin path or request the canonical RSD service"
        )));
    }
    Ok(())
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

pub(super) async fn rsd_checkin_with_timeout<S>(
    stream: &mut S,
    timeout: std::time::Duration,
) -> Result<(), CoreError>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    tokio::time::timeout(timeout, rsd_checkin(stream))
        .await
        .map_err(|_| {
            CoreError::Io(std::io::Error::new(
                std::io::ErrorKind::TimedOut,
                format!("RSD check-in timed out after {} ms", timeout.as_millis()),
            ))
        })?
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;
    use tokio::io::duplex;

    #[test]
    fn resolve_rsd_service_reports_actual_shim_match() {
        let rsd = RsdHandshake {
            udid: "test-udid".into(),
            services: HashMap::from([(
                "com.apple.mobile.notification_proxy.shim.remote".into(),
                ServiceDescriptor::new(1234),
            )]),
        };

        let resolved = resolve_rsd_service(&rsd, "com.apple.mobile.notification_proxy")
            .expect("shim fallback should resolve");

        assert_eq!(
            resolved,
            (
                "com.apple.mobile.notification_proxy.shim.remote".into(),
                1234
            )
        );
    }

    #[test]
    fn resolve_rsd_service_prefers_canonical_entry_over_shim() {
        let rsd = RsdHandshake {
            udid: "test-udid".into(),
            services: HashMap::from([
                (
                    "com.apple.mobile.notification_proxy".into(),
                    ServiceDescriptor::new(2345),
                ),
                (
                    "com.apple.mobile.notification_proxy.shim.remote".into(),
                    ServiceDescriptor::new(6789),
                ),
            ]),
        };

        assert_eq!(
            resolve_rsd_service(&rsd, "com.apple.mobile.notification_proxy"),
            Some(("com.apple.mobile.notification_proxy".to_string(), 2345,))
        );
    }

    #[test]
    #[cfg(feature = "tunnel")]
    fn resolved_rsd_service_metadata_preserves_canonical_features() {
        let mut canonical = ServiceDescriptor::new(2345);
        canonical.features = vec!["com.apple.coredevice.feature.streamapplist".into()];
        let mut shim = ServiceDescriptor::new(6789);
        shim.features = vec!["shim-only-feature".into()];
        let rsd = RsdHandshake {
            udid: "test-udid".into(),
            services: HashMap::from([
                ("com.apple.coredevice.appservice".into(), canonical),
                ("com.apple.coredevice.appservice.shim.remote".into(), shim),
            ]),
        };

        let resolved = resolve_rsd_service_details(&rsd, "com.apple.coredevice.appservice")
            .expect("canonical appservice should resolve");

        assert_eq!(
            resolved.resolved_service_name,
            "com.apple.coredevice.appservice"
        );
        assert_eq!(resolved.port, 2345);
        assert_eq!(
            resolved.features,
            ["com.apple.coredevice.feature.streamapplist"]
        );
    }

    #[test]
    #[cfg(feature = "tunnel")]
    fn resolved_rsd_service_metadata_falls_back_to_shim_features() {
        let mut shim = ServiceDescriptor::new(6789);
        shim.features = vec!["shim-feature".into()];
        let rsd = RsdHandshake {
            udid: "test-udid".into(),
            services: HashMap::from([("com.apple.coredevice.appservice.shim.remote".into(), shim)]),
        };

        let resolved = resolve_rsd_service_details(&rsd, "com.apple.coredevice.appservice")
            .expect("shim appservice should resolve");

        assert_eq!(
            resolved.resolved_service_name,
            "com.apple.coredevice.appservice.shim.remote"
        );
        assert_eq!(resolved.port, 6789);
        assert_eq!(resolved.features, ["shim-feature"]);
    }

    #[test]
    #[cfg(feature = "tunnel")]
    fn resolved_rsd_service_metadata_keeps_empty_features_for_fallback() {
        let rsd = RsdHandshake {
            udid: "test-udid".into(),
            services: HashMap::from([(
                "com.apple.coredevice.appservice".into(),
                ServiceDescriptor::new(2345),
            )]),
        };

        let resolved = resolve_rsd_service_details(&rsd, "com.apple.coredevice.appservice")
            .expect("canonical appservice should resolve without feature metadata");

        assert_eq!(
            resolved.resolved_service_name,
            "com.apple.coredevice.appservice"
        );
        assert!(resolved.features.is_empty());
    }

    #[test]
    fn remote_xpc_route_rejects_shim_before_opening_socket() {
        let error = ensure_remote_xpc_service(
            "com.apple.mobile.notification_proxy",
            "com.apple.mobile.notification_proxy.shim.remote",
        )
        .expect_err("lockdown shims must not enter XPC/H2 bootstrap");
        let message = error.to_string();
        assert!(message.contains("lockdown shim"));
        assert!(message.contains("connect_rsd_service"));
        assert!(message.contains("RSDCheckin"));
    }

    #[test]
    fn remote_xpc_route_accepts_canonical_service() {
        ensure_remote_xpc_service(
            "com.apple.coredevice.appservice",
            "com.apple.coredevice.appservice",
        )
        .expect("canonical RemoteXPC services should be accepted");
    }

    #[tokio::test]
    async fn rsd_checkin_sends_request_and_consumes_two_responses() {
        let (mut client, mut server) = duplex(4096);
        let task = tokio::spawn(async move { rsd_checkin(&mut client).await });

        let request: plist::Value = recv_lockdown(&mut server).await.expect("request frame");
        let dict = request
            .into_dictionary()
            .expect("RSDCheckin request should be a plist dictionary");
        assert_eq!(
            dict.get("Request").and_then(plist::Value::as_string),
            Some("RSDCheckin")
        );
        assert_eq!(
            dict.get("ProtocolVersion")
                .and_then(plist::Value::as_string),
            Some("2")
        );

        send_lockdown(
            &mut server,
            &plist::Value::Dictionary(plist::Dictionary::from_iter([
                (
                    String::from("Request"),
                    plist::Value::String("RSDCheckin".into()),
                ),
                (
                    String::from("Status"),
                    plist::Value::String("Acknowledged".into()),
                ),
            ])),
        )
        .await
        .expect("checkin response");
        send_lockdown(
            &mut server,
            &plist::Value::Dictionary(plist::Dictionary::from_iter([
                (
                    String::from("Request"),
                    plist::Value::String("StartService".into()),
                ),
                (String::from("Service"), plist::Value::String("shim".into())),
            ])),
        )
        .await
        .expect("start service response");

        task.await
            .expect("join")
            .expect("rsd checkin should succeed");
    }

    #[tokio::test]
    async fn rsd_checkin_rejects_unexpected_first_response() {
        let (mut client, mut server) = duplex(4096);
        let task = tokio::spawn(async move { rsd_checkin(&mut client).await });

        let _: plist::Value = recv_lockdown(&mut server).await.expect("request frame");

        send_lockdown(
            &mut server,
            &plist::Value::Dictionary(plist::Dictionary::from_iter([(
                String::from("Request"),
                plist::Value::String("StartService".into()),
            )])),
        )
        .await
        .expect("unexpected first response");
        send_lockdown(
            &mut server,
            &plist::Value::Dictionary(plist::Dictionary::from_iter([(
                String::from("Request"),
                plist::Value::String("StartService".into()),
            )])),
        )
        .await
        .expect("second response");

        let err = task
            .await
            .expect("join")
            .expect_err("rsd checkin should reject mismatched first response");
        let rendered = err.to_string();
        assert!(rendered.contains("RSD check-in response"));
        assert!(rendered.contains("Request=RSDCheckin"));
    }

    #[tokio::test]
    async fn rsd_checkin_rejects_start_service_error() {
        let (mut client, mut server) = duplex(4096);
        let task = tokio::spawn(async move { rsd_checkin(&mut client).await });

        let _: plist::Value = recv_lockdown(&mut server).await.expect("request frame");

        send_lockdown(
            &mut server,
            &plist::Value::Dictionary(plist::Dictionary::from_iter([(
                String::from("Request"),
                plist::Value::String("RSDCheckin".into()),
            )])),
        )
        .await
        .expect("checkin response");
        send_lockdown(
            &mut server,
            &plist::Value::Dictionary(plist::Dictionary::from_iter([
                (
                    String::from("Request"),
                    plist::Value::String("StartService".into()),
                ),
                (
                    String::from("Error"),
                    plist::Value::String("ServiceProhibited".into()),
                ),
            ])),
        )
        .await
        .expect("start service error response");

        let err = task
            .await
            .expect("join")
            .expect_err("rsd checkin should surface start service errors");
        let rendered = err.to_string();
        assert!(rendered.contains("RSD start-service response"));
        assert!(rendered.contains("ServiceProhibited"));
    }

    #[tokio::test]
    async fn rsd_checkin_times_out_when_peer_stalls() {
        let (mut client, _server) = duplex(4096);

        let err = rsd_checkin_with_timeout(&mut client, std::time::Duration::from_millis(20))
            .await
            .expect_err("a peer that never answers must not block forever");

        assert!(matches!(
            err,
            CoreError::Io(ref io_error) if io_error.kind() == std::io::ErrorKind::TimedOut
        ));
        assert!(err.to_string().contains("RSD check-in timed out"));
    }
}
