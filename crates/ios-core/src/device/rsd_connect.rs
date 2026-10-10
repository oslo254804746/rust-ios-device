//! Optional direct and userspace-proxy RSD handshakes.

use super::rsd_observation::RsdProbeOutcome;
use super::tunnel_endpoint::{connection_error_to_xpc, TunnelEndpoint};
#[cfg(not(feature = "mdns"))]
use super::InitialRsdProbeSkipReason;
#[cfg(feature = "mdns")]
use crate::xpc::rsd::handshake as rsd_handshake;
use crate::CoreError;
#[cfg(test)]
use std::time::Duration;
#[cfg(feature = "mdns")]
use std::{net::Ipv6Addr, str::FromStr};
#[cfg(test)]
use tokio::net::TcpStream;

/// Attempt the optional RSD handshake, retaining the actual owner outcome.
#[cfg(all(feature = "tunnel", feature = "mdns"))]
pub(super) async fn attempt_rsd(server_addr: &str, rsd_port: u16) -> RsdProbeOutcome {
    let addr = match Ipv6Addr::from_str(server_addr) {
        Ok(addr) => addr,
        Err(error) => {
            return RsdProbeOutcome::Completed(Err(CoreError::Io(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                error,
            ))));
        }
    };
    match rsd_handshake(addr, rsd_port).await {
        Ok(h) => {
            tracing::info!("RSD: {} services discovered", h.services.len());
            RsdProbeOutcome::Completed(Ok(h))
        }
        Err(e) => {
            tracing::debug!(error = %crate::redaction::diagnostic_error(&e), "RSD handshake failed (may be iOS <17)");
            RsdProbeOutcome::Completed(Err(CoreError::Xpc(e)))
        }
    }
}

/// Direct RSD discovery is only available when Bonjour/mdns support is built.
/// Keep the tunnel-only feature combinations usable: userspace tunnels still
/// discover RSD through their local proxy, while kernel/tunnel-only callers can
/// observe a skipped initial probe while retaining the legacy optional cache.
#[cfg(all(feature = "tunnel", not(feature = "mdns")))]
pub(super) async fn attempt_rsd(_server_addr: &str, _rsd_port: u16) -> RsdProbeOutcome {
    tracing::debug!("RSD direct probe skipped because ios-core feature 'mdns' is disabled");
    RsdProbeOutcome::Skipped(InitialRsdProbeSkipReason::DirectProbeFeatureUnavailable)
}

/// Attempt RSD via the go-ios-compatible userspace proxy. The direct and proxy
/// routes share the same host identity, bootstrap ordering and bounded retries.
#[cfg(feature = "tunnel")]
pub(super) async fn attempt_rsd_via_proxy(
    proxy_port: u16,
    server_addr: &str,
    rsd_port: u16,
) -> RsdProbeOutcome {
    let endpoint = match TunnelEndpoint::resolve(server_addr, Some(proxy_port)) {
        Ok(endpoint) => endpoint,
        Err(error) => {
            tracing::warn!("RSD proxy endpoint is invalid");
            return RsdProbeOutcome::Completed(Err(error));
        }
    };
    let result = crate::xpc::rsd::handshake_with_connector(|| async {
        endpoint
            .connect(rsd_port)
            .await
            .map_err(connection_error_to_xpc)
    })
    .await;
    match result {
        Ok(handshake) => {
            tracing::info!(
                "RSD via proxy: {} services discovered",
                handshake.services.len()
            );
            RsdProbeOutcome::Completed(Ok(handshake))
        }
        Err(error) => {
            tracing::debug!(error = %crate::redaction::diagnostic_error(&error), "RSD via proxy failed");
            RsdProbeOutcome::Completed(Err(CoreError::Xpc(error)))
        }
    }
}

#[cfg(all(test, feature = "tunnel"))]
mod rsd_proxy_tests {
    use super::*;
    use crate::xpc::message::{decode_message, encode_message, flags, XpcMessage, XpcValue};
    #[cfg(not(feature = "mdns"))]
    use crate::InitialRsdProbeSkipReason;
    use crate::{ErrorClass, InitialRsdProbeState};
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    #[tokio::test]
    async fn owned_non_listening_proxy_retains_real_dial_error_in_failed_observation() {
        // Reserve the port without listening, preventing a released-port race.
        let socket = tokio::net::TcpSocket::new_v4().unwrap();
        socket.bind("127.0.0.1:0".parse().unwrap()).unwrap();
        let port = socket.local_addr().unwrap().port();
        let result = tokio::time::timeout(
            // Native Winsock refusal itself takes ~2s before RSD retries.
            Duration::from_secs(if cfg!(windows) { 10 } else { 2 }),
            attempt_rsd_via_proxy(port, "fd00::42", 58783),
        )
        .await
        .unwrap();
        let (directory, observation) = result.into_parts();
        assert!(directory.is_none());
        assert_eq!(observation.state(), InitialRsdProbeState::Failed);
        assert!(observation.skip_reason().is_none());
        assert_eq!(observation.error_class(), Some(ErrorClass::Io));
        let CoreError::Xpc(crate::XpcError::Io(error)) = observation.error().unwrap() else {
            panic!("typed proxy dial error lost at the owner")
        };
        assert_eq!(error.kind(), std::io::ErrorKind::ConnectionRefused);
        drop(socket);
    }

    #[tokio::test]
    async fn invalid_probe_endpoint_is_a_failed_request_without_connecting() {
        let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
            .await
            .unwrap();
        let port = listener.local_addr().unwrap().port();
        let secret = "synthetic-private-invalid-probe-endpoint";
        let (directory, observation) = attempt_rsd_via_proxy(port, secret, 58783)
            .await
            .into_parts();
        assert!(directory.is_none());
        assert_eq!(observation.state(), InitialRsdProbeState::Failed);
        assert_eq!(observation.error_class(), Some(ErrorClass::Protocol));
        assert!(!format!("{observation:?} {observation:#?}").contains(secret));
        assert!(
            tokio::time::timeout(Duration::from_millis(10), listener.accept())
                .await
                .is_err()
        );
        let (directory, direct) = attempt_rsd(secret, 58783).await.into_parts();
        assert!(directory.is_none());
        #[cfg(feature = "mdns")]
        {
            assert_eq!(direct.state(), InitialRsdProbeState::Failed);
            assert_eq!(direct.error_class(), Some(ErrorClass::InvalidInput));
            let CoreError::Io(error) = direct.error().unwrap() else {
                panic!("typed input error lost")
            };
            assert!(error.get_ref().unwrap().is::<std::net::AddrParseError>());
        }
        #[cfg(not(feature = "mdns"))]
        {
            assert_eq!(direct.state(), InitialRsdProbeState::NotAttempted);
            assert_eq!(
                direct.skip_reason(),
                Some(InitialRsdProbeSkipReason::DirectProbeFeatureUnavailable)
            );
            assert!(direct.error().is_none());
        }
    }

    async fn read_frame(stream: &mut TcpStream) -> (u8, u32, Vec<u8>) {
        let mut header = [0; 9];
        stream.read_exact(&mut header).await.unwrap();
        let length =
            (usize::from(header[0]) << 16) | (usize::from(header[1]) << 8) | usize::from(header[2]);
        assert!(length <= 4096);
        let mut payload = vec![0; length];
        stream.read_exact(&mut payload).await.unwrap();
        (
            header[3],
            u32::from_be_bytes(header[5..9].try_into().unwrap()),
            payload,
        )
    }

    #[tokio::test]
    async fn proxy_route_sends_destination_and_reuses_stable_rsd_identity() {
        let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
            .await
            .unwrap();
        let proxy_port = listener.local_addr().unwrap().port();
        let identity = crate::xpc::rsd::default_handshake_uuid().await.unwrap();
        let server = tokio::spawn(async move {
            for _ in 0..2 {
                let (mut stream, _) = listener.accept().await.unwrap();
                let mut destination = [0; 20];
                stream.read_exact(&mut destination).await.unwrap();
                assert_eq!(
                    &destination[..16],
                    &"fd00::42".parse::<std::net::Ipv6Addr>().unwrap().octets()
                );
                assert_eq!(
                    u32::from_le_bytes(destination[16..].try_into().unwrap()),
                    58783
                );
                let mut preface = [0; 24];
                stream.read_exact(&mut preface).await.unwrap();
                assert_eq!(&preface, crate::xpc::h2_raw::H2_PREFACE);
                assert_eq!(read_frame(&mut stream).await.0, 4);
                assert_eq!(read_frame(&mut stream).await.0, 8);
                stream
                    .write_all(&[0, 0, 0, 4, 0, 0, 0, 0, 0])
                    .await
                    .unwrap();
                loop {
                    let (kind, stream_id, payload) = read_frame(&mut stream).await;
                    if kind != 0 || stream_id != 1 {
                        continue;
                    }
                    let message = decode_message(bytes::Bytes::from(payload)).unwrap();
                    let Some(body) = message.body.as_ref().and_then(XpcValue::as_dict) else {
                        continue;
                    };
                    if body.get("MessageType").and_then(XpcValue::as_str) != Some("Handshake") {
                        continue;
                    }
                    assert_eq!(body["UUID"], XpcValue::Uuid(*identity.as_bytes()));
                    break;
                }
                let reply = encode_message(&XpcMessage {
                    flags: flags::ALWAYS_SET | flags::DATA,
                    msg_id: 1,
                    body: Some(XpcValue::Dictionary(indexmap::IndexMap::from([
                        ("MessageType".into(), XpcValue::String("Handshake".into())),
                        (
                            "Properties".into(),
                            XpcValue::Dictionary(indexmap::IndexMap::from([(
                                "UniqueDeviceID".into(),
                                XpcValue::String("synthetic-proxy-device".into()),
                            )])),
                        ),
                        (
                            "Services".into(),
                            XpcValue::Dictionary(indexmap::IndexMap::new()),
                        ),
                    ]))),
                })
                .unwrap();
                let length = u32::try_from(reply.len()).unwrap().to_be_bytes();
                stream
                    .write_all(&[length[1], length[2], length[3], 0, 0, 0, 0, 0, 1])
                    .await
                    .unwrap();
                stream.write_all(&reply).await.unwrap();
            }
        });
        for _ in 0..2 {
            let result = tokio::time::timeout(
                Duration::from_secs(2),
                attempt_rsd_via_proxy(proxy_port, "fd00::42", 58783),
            )
            .await
            .unwrap();
            let (directory, observation) = result.into_parts();
            assert_eq!(observation.state(), InitialRsdProbeState::Available);
            assert!(observation.error().is_none());
            let result = directory.unwrap();
            assert_eq!(result.udid, "synthetic-proxy-device");
        }
        server.await.unwrap();
    }
}
