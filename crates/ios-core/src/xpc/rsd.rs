//! RSD (Remote Service Discovery) client for iOS 17+.
//!
//! Modern RSD announces a stable host UUID after the RemoteXPC bootstrap and
//! reads the device's service directory on stream 1. Older queued, synchronous
//! and passive bootstrap styles are retried on fresh connections.
//!
//! Reference: go-ios/ios/rsd.go + go-ios/ios/http/http.go

use std::collections::HashMap;
#[cfg(feature = "tunnel")]
use std::collections::VecDeque;
#[cfg(feature = "tunnel")]
use std::mem::size_of;
#[cfg(all(feature = "tunnel", feature = "mdns"))]
use std::net::{Ipv6Addr, SocketAddr};

#[cfg(feature = "tunnel")]
use bytes::{Bytes, BytesMut};
#[cfg(all(feature = "tunnel", feature = "mdns"))]
use tokio::net::TcpStream;

#[cfg(feature = "tunnel")]
use crate::xpc::error_mapping::{h2_error, xpc_timeout};
#[cfg(feature = "tunnel")]
use crate::xpc::h2_raw::{H2Error, H2Framer, MAX_BUFFERED_BYTES_PER_STREAM, MAX_FRAME_PAYLOAD};
#[cfg(feature = "tunnel")]
use crate::xpc::message::{
    checked_xpc_body_len, decode_message, flags, xpc_body_limit_for_flags, XpcMessage, XpcValue,
};
#[cfg(feature = "tunnel")]
use crate::xpc::XpcError;

pub const RSD_PORT: u16 = 58783;

/// Evidence in a successfully obtained RSD directory. This does not describe
/// an unattempted/failed probe, nor certify an operation on a physical device.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum RsdFeatureEvidence {
    Advertised,
    NotAdvertised,
    /// The historical descriptor has no usable capability list. An omitted
    /// list and an explicitly empty list cannot be distinguished by this type.
    MetadataUnavailable,
    ServiceMissing,
}

fn feature_evidence(features: &[String], feature: &str) -> RsdFeatureEvidence {
    if features.is_empty() {
        RsdFeatureEvidence::MetadataUnavailable
    } else if features.iter().any(|item| item == feature) {
        RsdFeatureEvidence::Advertised
    } else {
        RsdFeatureEvidence::NotAdvertised
    }
}

/// A discovered iOS 17+ service and its advertised capabilities.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServiceDescriptor {
    pub port: u16,
    /// Feature identifiers advertised under `Properties.Features`.
    ///
    /// An empty list means the device did not advertise capability details; it
    /// must not be interpreted as a deny-all list.
    pub features: Vec<String>,
}

impl ServiceDescriptor {
    pub fn new(port: u16) -> Self {
        Self {
            port,
            features: Vec::new(),
        }
    }

    /// Whether the service can be treated as supporting `feature`.
    ///
    /// Missing capability metadata is permissive because older devices and
    /// some transports omit `Properties.Features` entirely.
    pub fn supports_feature(&self, feature: &str) -> bool {
        self.features.is_empty() || self.features.iter().any(|item| item == feature)
    }

    /// Report capability evidence separately from the permissive legacy check.
    /// NotAdvertised is an observation, not a device-side unsupported reply.
    pub fn feature_evidence(&self, feature: &str) -> RsdFeatureEvidence {
        feature_evidence(&self.features, feature)
    }
}

/// Result of the RSD handshake.
#[derive(Debug, Clone)]
pub struct RsdHandshake {
    pub udid: String,
    pub services: HashMap<String, ServiceDescriptor>,
}

impl RsdHandshake {
    /// Query an exact service entry in this successful directory snapshot.
    pub fn feature_evidence(&self, service: &str, feature: &str) -> RsdFeatureEvidence {
        self.get_service_features(service)
            .map(|features| feature_evidence(features, feature))
            .unwrap_or(RsdFeatureEvidence::ServiceMissing)
    }

    /// Query with the existing canonical-then-`.shim.remote` resolution order.
    pub fn resolved_feature_evidence(&self, service: &str, feature: &str) -> RsdFeatureEvidence {
        self.get_resolved_service_features(service)
            .map(|features| feature_evidence(features, feature))
            .unwrap_or(RsdFeatureEvidence::ServiceMissing)
    }

    /// Look up a service port, with automatic `.shim.remote` fallback.
    pub fn get_port(&self, service: &str) -> Option<u16> {
        if let Some(s) = self.services.get(service) {
            return Some(s.port);
        }
        let shim = format!("{service}.shim.remote");
        self.services.get(&shim).map(|s| s.port)
    }

    /// Return features explicitly advertised for an exact service name.
    pub fn get_service_features(&self, service: &str) -> Option<&[String]> {
        self.services
            .get(service)
            .map(|descriptor| descriptor.features.as_slice())
    }

    /// Return features for a service, resolving the `.shim.remote` entry used
    /// by some RSD handshakes when the canonical service name is absent.
    pub fn get_resolved_service_features(&self, service: &str) -> Option<&[String]> {
        if let Some(features) = self.get_service_features(service) {
            return Some(features);
        }
        let shim = format!("{service}.shim.remote");
        self.services
            .get(&shim)
            .map(|descriptor| descriptor.features.as_slice())
    }

    /// Check a feature against an exact service entry.
    ///
    /// Returns `None` when the service is absent. For a present service whose
    /// handshake entry has no feature list, returns `Some(true)` so missing
    /// metadata is not mistaken for an explicit device-side rejection.
    pub fn supports_feature(&self, service: &str, feature: &str) -> Option<bool> {
        self.services
            .get(service)
            .map(|descriptor| descriptor.supports_feature(feature))
    }
}

/// Perform an RSD handshake with an iOS 17+ device.
///
/// `addr` is the device's tunnel IPv6 address (from CDTunnel handshake).
#[cfg(all(feature = "tunnel", feature = "mdns"))]
pub async fn handshake(addr: Ipv6Addr, port: u16) -> Result<RsdHandshake, XpcError> {
    let sock_addr = SocketAddr::new(addr.into(), port);
    handshake_with_connector(|| async {
        TcpStream::connect(sock_addr).await.map_err(XpcError::from)
    })
    .await
}

/// Resolve the UUID shared by RSD connections from this host.
///
/// On macOS this queries the local `remoted` identity once, with a bounded
/// subprocess. Other hosts (and macOS without remoted) use UUIDv3 derived from
/// the OS hostname. Resolve this before suspending remoted, if applicable.
#[cfg(feature = "tunnel")]
pub async fn default_handshake_uuid() -> Result<uuid::Uuid, XpcError> {
    Ok(crate::xpc::identity::default_handshake_uuid().await?)
}

#[cfg(feature = "tunnel")]
#[derive(Clone, Copy, Debug)]
enum HandshakeMode {
    Active,
    Queued,
    Legacy,
    Passive,
}

/// Both routed TCP and userspace proxy connections use the same identity and
/// retry policy. A failed/timed-out read may consume a partial frame, so each
/// fallback owns a fresh transport; no parser state is reused after cancellation.
#[cfg(feature = "tunnel")]
pub(crate) async fn handshake_with_connector<F, Fut, S>(
    connector: F,
) -> Result<RsdHandshake, XpcError>
where
    F: FnMut() -> Fut,
    Fut: std::future::Future<Output = Result<S, XpcError>>,
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin,
{
    let identity = default_handshake_uuid().await?;
    // Preserve the former direct path's dial + handshake upper bound across
    // all retries instead of multiplying it by the number of bootstrap modes.
    rsd_timeout(
        crate::tunnel::TUNNEL_CONNECT_TIMEOUT * 2,
        "RSD discovery timed out",
        handshake_attempts(connector, identity),
    )
    .await
}

#[cfg(feature = "tunnel")]
async fn handshake_attempts<F, Fut, S>(
    mut connector: F,
    identity: uuid::Uuid,
) -> Result<RsdHandshake, XpcError>
where
    F: FnMut() -> Fut,
    Fut: std::future::Future<Output = Result<S, XpcError>>,
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin,
{
    use std::time::Duration;

    let mut last_error = None;
    for mode in [
        HandshakeMode::Active,
        HandshakeMode::Queued,
        HandshakeMode::Legacy,
        HandshakeMode::Passive,
    ] {
        let stream = rsd_timeout(
            crate::tunnel::TUNNEL_CONNECT_TIMEOUT,
            "RSD dial timed out",
            connector(),
        )
        .await?;
        let attempt = async {
            let mut framer = rsd_timeout(
                Duration::from_secs(3),
                "RSD bootstrap timed out",
                bootstrap_rsd(stream, mode, identity),
            )
            .await?;
            let response_timeout = match mode {
                HandshakeMode::Active | HandshakeMode::Queued => Duration::from_secs(4),
                HandshakeMode::Legacy => Duration::from_secs(3),
                HandshakeMode::Passive => Duration::from_secs(2),
            };
            rsd_timeout(
                response_timeout,
                "RSD response timed out",
                handshake_on_framer(&mut framer),
            )
            .await
        }
        .await;
        match attempt {
            Ok(handshake) => return Ok(handshake),
            Err(error) => {
                tracing::debug!(?mode, "RSD bootstrap attempt failed");
                last_error = Some(error);
            }
        }
    }
    Err(last_error.expect("RSD attempted every bootstrap mode"))
}

#[cfg(feature = "tunnel")]
async fn bootstrap_rsd<S>(
    stream: S,
    mode: HandshakeMode,
    identity: uuid::Uuid,
) -> Result<H2Framer<S>, XpcError>
where
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin,
{
    bootstrap_rsd_with_bank(stream, mode, identity, None).await
}

#[cfg(feature = "tunnel")]
async fn bootstrap_rsd_with_bank<S>(
    stream: S,
    mode: HandshakeMode,
    identity: uuid::Uuid,
    bank: Option<&std::sync::Arc<tokio::sync::Semaphore>>,
) -> Result<H2Framer<S>, XpcError>
where
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin,
{
    let mut framer = if matches!(mode, HandshakeMode::Active) {
        let mut framer = H2Framer::begin_connect_with_bank(stream, bank)
            .await
            .map_err(|error| h2_error("RSD H2: ", error))?;
        queue_rsd_handshake_bootstrap_on_framer(&mut framer).await?;
        framer
            .finish_connect()
            .await
            .map_err(|error| h2_error("RSD H2: ", error))?;
        framer
    } else {
        H2Framer::connect_with_bank(stream, bank)
            .await
            .map_err(|error| h2_error("RSD H2: ", error))?
    };
    match mode {
        HandshakeMode::Active => send_device_handshake_on_framer(&mut framer, identity).await?,
        HandshakeMode::Queued => queue_rsd_handshake_bootstrap_on_framer(&mut framer).await?,
        HandshakeMode::Legacy => initialize_xpc_connection_on_framer(&mut framer).await?,
        HandshakeMode::Passive => {}
    }
    Ok(framer)
}

/// The caller guards every socket poll and owns one deadline across identity,
/// connector/prelude and all bootstrap modes. A decoded Handshake is final:
/// missing fields are observations; malformed fields are original protocol
/// errors, never a reason to replace them with another dial or timeout.
#[cfg(feature = "tunnel")]
pub(crate) async fn checked_handshake_with_connector_observed<F, Fut, S>(
    connector: F,
    identity: uuid::Uuid,
    deadline: tokio::time::Instant,
) -> Result<
    (
        crate::device::existing_rsd::ExistingRsdObservation,
        crate::device::existing_rsd::RsdSelectionReceipt,
    ),
    XpcError,
>
where
    F: FnMut() -> Fut,
    Fut: std::future::Future<Output = Result<S, XpcError>>,
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin,
{
    checked_handshake_with_parser(
        connector,
        identity,
        deadline,
        None,
        parse_checked_handshake_dictionary,
    )
    .await
    .map(|(directory, receipt, _)| (directory, receipt))
}

#[cfg(feature = "tunnel")]
pub(crate) async fn checked_handshake_with_parser<F, Fut, S, T, P>(
    connector: F,
    identity: uuid::Uuid,
    deadline: tokio::time::Instant,
    bank: Option<&std::sync::Arc<tokio::sync::Semaphore>>,
    mut parse: P,
) -> Result<
    (
        T,
        crate::device::existing_rsd::RsdSelectionReceipt,
        Option<tokio::sync::OwnedSemaphorePermit>,
    ),
    XpcError,
>
where
    F: FnMut() -> Fut,
    Fut: std::future::Future<Output = Result<S, XpcError>>,
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin,
    P: FnMut(&indexmap::IndexMap<String, XpcValue>) -> Result<T, XpcError>,
{
    checked_handshake_wire(connector, identity, deadline, bank, |header, body| {
        // Compatibility parser for the unbanked observation API. The prepared
        // path calls checked_handshake_prepared and never creates this tree.
        let mut full = BytesMut::new();
        full.extend_from_slice(header);
        full.extend_from_slice(body);
        let message = decode_message(full.freeze()).map_err(CheckedReadError::Retry)?;
        let Some(dictionary) = message.body.as_ref().and_then(XpcValue::as_dict) else {
            return Ok(None);
        };
        match dictionary.get("MessageType") {
            None => Ok(None),
            Some(XpcValue::String(value)) if value != "Handshake" => Ok(None),
            Some(XpcValue::String(_)) => {
                parse(dictionary).map(Some).map_err(CheckedReadError::Final)
            }
            Some(_) => Err(CheckedReadError::Final(XpcError::Tls(
                "RSD: MessageType was not a string".into(),
            ))),
        }
    })
    .await
}

#[cfg(feature = "tunnel")]
async fn checked_handshake_wire<F, Fut, S, T, P>(
    mut connector: F,
    identity: uuid::Uuid,
    deadline: tokio::time::Instant,
    bank: Option<&std::sync::Arc<tokio::sync::Semaphore>>,
    mut parse: P,
) -> Result<
    (
        T,
        crate::device::existing_rsd::RsdSelectionReceipt,
        Option<tokio::sync::OwnedSemaphorePermit>,
    ),
    XpcError,
>
where
    F: FnMut() -> Fut,
    Fut: std::future::Future<Output = Result<S, XpcError>>,
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin,
    P: FnMut(&[u8], &[u8]) -> Result<Option<T>, CheckedReadError>,
{
    use crate::device::existing_rsd::{RsdHandshakeMode, RsdSelectionReceipt};
    use std::time::Duration;
    let mut last_error = None;
    let (selection_fee, mut attempted_modes) = match bank {
        Some(bank) => {
            let (fee, values) = crate::device::prepared_rsd::exact_vec(bank, 4)?;
            (Some(fee), values)
        }
        None => (None, Vec::with_capacity(4)),
    };
    for mode in [
        HandshakeMode::Active,
        HandshakeMode::Queued,
        HandshakeMode::Legacy,
        HandshakeMode::Passive,
    ] {
        if tokio::time::Instant::now() >= deadline {
            return match last_error {
                Some(error) => Err(error),
                None => tokio::time::timeout_at(
                    deadline,
                    std::future::pending::<
                        Result<
                            (
                                T,
                                RsdSelectionReceipt,
                                Option<tokio::sync::OwnedSemaphorePermit>,
                            ),
                            XpcError,
                        >,
                    >(),
                )
                .await
                .map_err(|source| {
                    crate::xpc::native_error(
                        std::io::ErrorKind::TimedOut,
                        "existing RSD discovery deadline",
                        source,
                    )
                })?,
            };
        }
        let observed_mode = match mode {
            HandshakeMode::Active => RsdHandshakeMode::Direct,
            HandshakeMode::Queued => RsdHandshakeMode::Queued,
            HandshakeMode::Legacy => RsdHandshakeMode::Legacy,
            HandshakeMode::Passive => RsdHandshakeMode::Passive,
        };
        attempted_modes.push(observed_mode);
        let stream = connector().await?;
        let bootstrap = rsd_timeout(
            Duration::from_secs(3),
            "RSD bootstrap timed out",
            bootstrap_rsd_with_bank(stream, mode, identity, bank),
        )
        .await;
        let mut framer = match bootstrap {
            Ok(framer) => framer,
            Err(error) => {
                if crate::classify_error(&error) == crate::ErrorClass::ResourceLimit {
                    return Err(error);
                }
                if tokio::time::Instant::now() >= deadline {
                    return Err(error);
                }
                last_error = Some(error);
                continue;
            }
        };
        let response_budget = match mode {
            HandshakeMode::Active | HandshakeMode::Queued => Duration::from_secs(4),
            HandshakeMode::Legacy => Duration::from_secs(3),
            HandshakeMode::Passive => Duration::from_secs(2),
        };
        match tokio::time::timeout(
            response_budget,
            read_checked_handshake(&mut framer, &mut parse),
        )
        .await
        {
            Ok(Ok(observation)) => {
                return Ok((
                    observation,
                    RsdSelectionReceipt {
                        handshake_mode: observed_mode,
                        attempted_modes,
                    },
                    selection_fee,
                ))
            }
            Ok(Err(CheckedReadError::Final(error))) => return Err(error),
            Ok(Err(CheckedReadError::Retry(error))) => {
                if tokio::time::Instant::now() >= deadline {
                    return Err(error);
                }
                last_error = Some(error);
            }
            Err(source) => {
                let error = crate::xpc::native_error(
                    std::io::ErrorKind::TimedOut,
                    "RSD response timed out",
                    source,
                );
                if tokio::time::Instant::now() >= deadline {
                    return Err(error);
                }
                last_error = Some(error);
            }
        }
    }
    Err(last_error.expect("RSD attempted every bootstrap mode"))
}

#[cfg(feature = "tunnel")]
enum CheckedReadError {
    Retry(XpcError),
    Final(XpcError),
}

#[cfg(feature = "tunnel")]
async fn read_checked_handshake<S, T>(
    framer: &mut H2Framer<S>,
    parse: &mut impl FnMut(&[u8], &[u8]) -> Result<Option<T>, CheckedReadError>,
) -> Result<T, CheckedReadError>
where
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin,
{
    let mut operation = RsdOperation::begin(framer).map_err(CheckedReadError::Retry)?;
    let mut result = Err(CheckedReadError::Retry(XpcError::Tls(
        "RSD: no handshake message received".into(),
    )));
    for _ in 0..6 {
        let (header, body) = match read_raw_xpc_on_client_server(operation.framer).await {
            Ok(raw) => {
                operation.progressed = true;
                raw
            }
            Err(error) => {
                result = Err(checked_wire_error(error));
                break;
            }
        };
        match parse(&header, &body) {
            Ok(None) => continue,
            Ok(Some(directory)) => {
                result = Ok(directory);
                break;
            }
            Err(error) => {
                result = Err(error);
                break;
            }
        }
    }
    // The socket is fresh and owned by the probe. Keep the same logical frame
    // invalidation contract as the legacy borrowed operation on any failure.
    operation.completed = true;
    if result.is_err() && operation.progressed {
        operation.framer.invalidate();
    }
    result
}

#[cfg(feature = "tunnel")]
type CheckedHandshakeParts<'a> = (
    Option<&'a String>,
    Option<&'a indexmap::IndexMap<String, XpcValue>>,
);

#[cfg(feature = "tunnel")]
pub(crate) fn checked_handshake_parts(
    dictionary: &indexmap::IndexMap<String, XpcValue>,
) -> Result<CheckedHandshakeParts<'_>, XpcError> {
    use crate::device::existing_rsd::{
        ExistingRsdProbeError, MAX_FEATURES, MAX_FIELD_BYTES, MAX_SERVICES, MAX_TEXT_BYTES,
    };
    let protocol = |message: &'static str| XpcError::Tls(message.into());
    let mut text_bytes = 0usize;
    let mut feature_count = 0usize;
    let mut admit_text = |value: &str| -> Result<(), XpcError> {
        text_bytes = text_bytes.saturating_add(value.len());
        if value.len() > MAX_FIELD_BYTES || text_bytes > MAX_TEXT_BYTES {
            return Err(ExistingRsdProbeError::metadata_limit());
        }
        Ok(())
    };
    let properties = match dictionary.get("Properties") {
        None => None,
        Some(XpcValue::Dictionary(properties)) => Some(properties),
        Some(_) => return Err(protocol("RSD: Properties was not a dictionary")),
    };
    let udid = match properties.and_then(|properties| properties.get("UniqueDeviceID")) {
        None => None,
        Some(XpcValue::String(udid)) if !udid.is_empty() => {
            admit_text(udid)?;
            Some(udid)
        }
        Some(_) => return Err(protocol("RSD: UniqueDeviceID was not a nonempty string")),
    };
    let services = match dictionary.get("Services") {
        None => None,
        Some(XpcValue::Dictionary(services)) if services.len() <= MAX_SERVICES => Some(services),
        Some(XpcValue::Dictionary(_)) => return Err(ExistingRsdProbeError::metadata_limit()),
        Some(_) => return Err(protocol("RSD: Services was not a dictionary")),
    };
    // Admit the whole borrowed snapshot before retaining additional strings.
    // A partial/incomplete directory cannot certify a selected service missing.
    if let Some(services) = services {
        for (name, value) in services {
            admit_text(name)?;
            let service = value
                .as_dict()
                .ok_or_else(|| protocol("RSD: service entry was not a dictionary"))?;
            if let Some(value) = service.get("Port") {
                if let XpcValue::String(value) = value {
                    if value.len() > MAX_FIELD_BYTES {
                        return Err(ExistingRsdProbeError::metadata_limit());
                    }
                }
                checked_service_port(value)?;
            }
            let properties = match service.get("Properties") {
                None => None,
                Some(XpcValue::Dictionary(properties)) => Some(properties),
                Some(_) => return Err(protocol("RSD: service Properties was not a dictionary")),
            };
            if let Some(value) = properties.and_then(|properties| properties.get("Features")) {
                let XpcValue::Array(features) = value else {
                    return Err(protocol("RSD: Features was not an array"));
                };
                feature_count = feature_count.saturating_add(features.len());
                if feature_count > MAX_FEATURES {
                    return Err(ExistingRsdProbeError::metadata_limit());
                }
                for feature in features {
                    let feature = feature
                        .as_str()
                        .ok_or_else(|| protocol("RSD: feature identifier was not a string"))?;
                    admit_text(feature)?;
                }
            }
        }
    }
    Ok((udid, services))
}

#[cfg(feature = "tunnel")]
fn parse_checked_handshake_dictionary(
    dictionary: &indexmap::IndexMap<String, XpcValue>,
) -> Result<crate::device::existing_rsd::ExistingRsdObservation, XpcError> {
    use crate::device::existing_rsd::{ExistingRsdObservation, ExistingRsdService};
    let (udid, services) = checked_handshake_parts(dictionary)?;
    let services = services
        .map(|services| {
            services
                .iter()
                .map(|(name, value)| {
                    let service = value.as_dict().expect("checked service dictionary");
                    let port = service.get("Port").map(checked_service_port).transpose()?;
                    let features = service
                        .get("Properties")
                        .and_then(XpcValue::as_dict)
                        .and_then(|properties| properties.get("Features"))
                        .map(|features| {
                            let XpcValue::Array(features) = features else {
                                unreachable!("checked feature array")
                            };
                            features
                                .iter()
                                .map(|feature| {
                                    feature.as_str().expect("checked feature string").to_owned()
                                })
                                .collect()
                        });
                    Ok((name.clone(), ExistingRsdService::new(port, features)))
                })
                .collect::<Result<HashMap<_, _>, XpcError>>()
        })
        .transpose()?;
    Ok(ExistingRsdObservation::new(udid.cloned(), services))
}

#[cfg(feature = "tunnel")]
pub(crate) fn checked_service_port(value: &XpcValue) -> Result<u16, XpcError> {
    let port = match value {
        XpcValue::String(port) => port.parse::<u16>().map_err(|source| {
            XpcError::Io(crate::error::context::received_io(
                "decode RSD service Port",
                None,
                source,
            ))
        })?,
        XpcValue::Uint64(port) => u16::try_from(*port).map_err(|source| {
            XpcError::Io(crate::error::context::received_io(
                "decode RSD service Port",
                Some(*port),
                source,
            ))
        })?,
        _ => return Err(XpcError::Tls("RSD: Port had an invalid type".into())),
    };
    if port == 0 {
        Err(XpcError::Tls("RSD: service Port was zero".into()))
    } else {
        Ok(port)
    }
}

#[cfg(feature = "tunnel")]
async fn rsd_timeout<T>(
    duration: std::time::Duration,
    context: &'static str,
    operation: impl std::future::Future<Output = Result<T, XpcError>>,
) -> Result<T, XpcError> {
    xpc_timeout(duration, context, operation).await
}

// Borrowed bootstrap/read operations can consume a complete H2 write or a
// buffered XPC header before waiting with no new transport progress. Keep
// logical ownership across those gaps independently of the H2 frame guard.
#[cfg(feature = "tunnel")]
struct RsdOperation<'a, S> {
    framer: &'a mut H2Framer<S>,
    progressed: bool,
    completed: bool,
}

#[cfg(feature = "tunnel")]
impl<'a, S> RsdOperation<'a, S> {
    fn begin(framer: &'a mut H2Framer<S>) -> Result<Self, XpcError> {
        if !framer.is_usable() {
            return Err(h2_error(
                "RSD operation: ",
                H2Error::Protocol(
                    "H2 connection is unusable; reconnect before another operation".into(),
                ),
            ));
        }
        Ok(Self {
            framer,
            progressed: false,
            completed: false,
        })
    }

    fn finish<T>(mut self, result: Result<T, XpcError>) -> Result<T, XpcError> {
        self.completed = true;
        if !self.framer.is_usable() || (result.is_err() && self.progressed) {
            self.framer.invalidate();
        }
        result
    }
}

#[cfg(feature = "tunnel")]
impl<S> Drop for RsdOperation<'_, S> {
    fn drop(&mut self) {
        if !self.completed && (self.progressed || !self.framer.is_usable()) {
            self.framer.invalidate();
        }
    }
}

#[cfg(feature = "tunnel")]
fn await_rsd_operation<T>(
    operation: impl std::future::Future<Output = Result<T, XpcError>>,
) -> impl std::future::Future<Output = Result<T, XpcError>> {
    // Box before constructing the caller's await state, as for H2/XPC. The
    // future remains directly owned; no task, reader or extra timer is added.
    Box::pin(operation)
}

/// Announce this host on an RSD control connection after the XPC bootstrap.
/// Never send this device handshake to an ordinary RemoteXPC service.
#[cfg(feature = "tunnel")]
pub async fn send_device_handshake_on_framer<S>(
    framer: &mut H2Framer<S>,
    identity: uuid::Uuid,
) -> Result<(), XpcError>
where
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin,
{
    let message = crate::xpc::message::encode_message(&device_handshake_message(identity))?;
    framer
        .write_client_server(&message)
        .await
        .map_err(|err| h2_error("RSD host handshake: ", err))
}

#[cfg(feature = "tunnel")]
fn device_handshake_message(identity: uuid::Uuid) -> XpcMessage {
    use indexmap::IndexMap;

    XpcMessage {
        flags: flags::ALWAYS_SET | flags::DATA,
        // The empty bootstrap dictionary already consumed message zero.
        msg_id: 1,
        body: Some(XpcValue::Dictionary(IndexMap::from([
            ("MessageType".into(), XpcValue::String("Handshake".into())),
            ("MessagingProtocolVersion".into(), XpcValue::Uint64(7)),
            ("UUID".into(), XpcValue::Uuid(*identity.as_bytes())),
            (
                "Properties".into(),
                XpcValue::Dictionary(IndexMap::from([
                    (
                        "RemoteXPCVersionFlags".into(),
                        XpcValue::Uint64(0x0100_0000_0000_0006),
                    ),
                    ("SensitivePropertiesVisible".into(), XpcValue::Bool(true)),
                ])),
            ),
            ("Services".into(), XpcValue::Dictionary(IndexMap::new())),
        ]))),
    }
}

/// Read the device's RSD handshake from an already-connected H2 framer.
/// The caller is responsible for selecting and sending any bootstrap traffic.
/// Cancellation/failure after consuming a message requires a fresh transport;
/// an empty idle wait preserves local framing. No new deadline is installed.
#[cfg(feature = "tunnel")]
pub async fn handshake_on_framer<S>(framer: &mut H2Framer<S>) -> Result<RsdHandshake, XpcError>
where
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin,
{
    read_rsd_handshake(framer).await
}

/// Initialize the XPC connection using go-ios's 3-message bootstrap.
///
/// Some devices appear to withhold the RSD handshake until these stream
/// bootstrapping messages have been exchanged.
/// Abandoning initialization after its first send invalidates local framing,
/// including an otherwise empty response wait. Successful initialization keeps
/// the connection usable; existing caller deadlines remain in effect.
#[cfg(feature = "tunnel")]
pub async fn initialize_xpc_connection_on_framer<S>(
    framer: &mut H2Framer<S>,
) -> Result<(), XpcError>
where
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin,
{
    let mut operation = RsdOperation::begin(framer)?;
    let result = await_rsd_operation(initialize_xpc_connection_inner(
        operation.framer,
        &mut operation.progressed,
    ))
    .await;
    operation.finish(result)
}

#[cfg(feature = "tunnel")]
async fn initialize_xpc_connection_inner<S>(
    framer: &mut H2Framer<S>,
    progressed: &mut bool,
) -> Result<(), XpcError>
where
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin,
{
    use crate::xpc::message::{encode_message, flags, XpcMessage, XpcValue};

    let msg1 = encode_message(&XpcMessage {
        flags: flags::ALWAYS_SET,
        msg_id: 0,
        body: Some(XpcValue::Dictionary(indexmap::IndexMap::new())),
    })?;
    framer
        .write_client_server(&msg1)
        .await
        .map_err(|e| h2_error("xpc init write 1: ", e))?;
    *progressed = true;
    discard_xpc_on_client_server(framer).await?;

    let msg3 = encode_message(&XpcMessage {
        flags: flags::ALWAYS_SET | 0x200,
        msg_id: 0,
        body: None,
    })?;
    framer
        // remoted requires stream 3's HEADERS before the terminating stream-1
        // bootstrap message (the order used by pymobiledevice3).
        .open_stream(crate::xpc::h2_raw::STREAM_SERVER_CLIENT)
        .await
        .map_err(|e| h2_error("xpc init open stream 3: ", e))?;
    framer
        .write_client_server(&msg3)
        .await
        .map_err(|e| h2_error("xpc init write 2: ", e))?;
    discard_xpc_on_client_server(framer).await?;

    let msg2 = encode_message(&XpcMessage {
        flags: flags::INIT_HANDSHAKE | flags::ALWAYS_SET,
        msg_id: 0,
        body: None,
    })?;
    framer
        .write_server_client(&msg2)
        .await
        .map_err(|e| h2_error("xpc init write 3: ", e))?;
    discard_xpc_on_server_client(framer).await?;

    Ok(())
}

/// Queue the minimal RemoteXPC bootstrap used by pymobiledevice3 before it
/// reads the first RSD handshake message from stream 1.
/// Cancellation/failure after the first complete write invalidates the framer;
/// completing this queue operation does not acknowledge a device handshake.
#[cfg(feature = "tunnel")]
pub async fn queue_rsd_handshake_bootstrap_on_framer<S>(
    framer: &mut H2Framer<S>,
) -> Result<(), XpcError>
where
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin,
{
    let mut operation = RsdOperation::begin(framer)?;
    let result = await_rsd_operation(queue_rsd_handshake_bootstrap_inner(
        operation.framer,
        &mut operation.progressed,
    ))
    .await;
    operation.finish(result)
}

#[cfg(feature = "tunnel")]
async fn queue_rsd_handshake_bootstrap_inner<S>(
    framer: &mut H2Framer<S>,
    progressed: &mut bool,
) -> Result<(), XpcError>
where
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin,
{
    use crate::xpc::message::{encode_message, flags, XpcMessage, XpcValue};

    let msg1 = encode_message(&XpcMessage {
        flags: flags::ALWAYS_SET,
        msg_id: 0,
        body: Some(XpcValue::Dictionary(indexmap::IndexMap::new())),
    })?;
    framer
        .write_client_server(&msg1)
        .await
        .map_err(|e| h2_error("rsd bootstrap write 1: ", e))?;
    *progressed = true;

    // Open stream 3 before the terminating stream-1 bootstrap message. This
    // ordering is significant to remoted and matches RemoteXPC clients:
    // HEADERS#1, DATA#1(init), HEADERS#3, DATA#1(term), DATA#3(init).
    framer
        .open_stream(crate::xpc::h2_raw::STREAM_SERVER_CLIENT)
        .await
        .map_err(|e| h2_error("rsd bootstrap open stream 3: ", e))?;

    let msg2 = encode_message(&XpcMessage {
        flags: flags::ALWAYS_SET | 0x200,
        msg_id: 0,
        body: None,
    })?;
    framer
        .write_client_server(&msg2)
        .await
        .map_err(|e| h2_error("rsd bootstrap write 2: ", e))?;

    let msg3 = encode_message(&XpcMessage {
        flags: flags::INIT_HANDSHAKE | flags::ALWAYS_SET,
        msg_id: 0,
        body: None,
    })?;
    framer
        .write_server_client(&msg3)
        .await
        .map_err(|e| h2_error("rsd bootstrap write 3: ", e))?;

    Ok(())
}

/// Read the RSD handshake message from clientServer stream (stream 1).
///
/// The device sends the handshake immediately after the H2 connection is
/// established — no XPC initialization is needed on the RSD port.
/// go-ios reads this via `ReceiveOnClientServerStream()` (rsd.go:208).
#[cfg(feature = "tunnel")]
async fn read_rsd_handshake<S>(framer: &mut H2Framer<S>) -> Result<RsdHandshake, XpcError>
where
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin,
{
    let mut operation = RsdOperation::begin(framer)?;
    let result = await_rsd_operation(read_rsd_handshake_inner(
        operation.framer,
        &mut operation.progressed,
    ))
    .await;
    operation.finish(result)
}

#[cfg(feature = "tunnel")]
async fn read_rsd_handshake_inner<S>(
    framer: &mut H2Framer<S>,
    progressed: &mut bool,
) -> Result<RsdHandshake, XpcError>
where
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin,
{
    let mut last_err = None;
    for _ in 0..6 {
        let msg = read_xpc_on_client_server(framer, progressed).await?;
        match parse_handshake_message(msg) {
            Ok(handshake) => return Ok(handshake),
            Err(err) => {
                tracing::debug!(error = %crate::redaction::diagnostic_error(&err), "RSD: skipping non-handshake stream-1 message");
                last_err = Some(err);
            }
        }
    }
    Err(last_err.unwrap_or_else(|| XpcError::Tls("RSD: no handshake message received".into())))
}

#[cfg(feature = "tunnel")]
async fn read_xpc_on_client_server<S>(
    framer: &mut H2Framer<S>,
    progressed: &mut bool,
) -> Result<XpcMessage, XpcError>
where
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin,
{
    let (header, body) = read_raw_xpc_on_client_server(framer).await?;
    *progressed = true;
    let mut full = bytes::BytesMut::new();
    full.extend_from_slice(&header);
    full.extend_from_slice(&body);
    decode_message(full.freeze())
}

#[cfg(feature = "tunnel")]
async fn discard_xpc_on_client_server<S>(framer: &mut H2Framer<S>) -> Result<(), XpcError>
where
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin,
{
    let _ = read_raw_xpc_on_client_server(framer).await?;
    Ok(())
}

#[cfg(feature = "tunnel")]
async fn discard_xpc_on_server_client<S>(framer: &mut H2Framer<S>) -> Result<(), XpcError>
where
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin,
{
    let _ = read_raw_xpc_on_server_client(framer).await?;
    Ok(())
}

/// Read an XPC body without asking the H2 demultiplexer to retain the whole
/// message at once.  H2 intentionally caps a buffered stream at 16 MiB, while
/// an explicit FILE_TX body is allowed up to 64 MiB by the XPC layer.  Consume
/// one H2-sized chunk at a time so that limit remains useful for control
/// traffic and unknown streams while valid file/data messages can use their
/// larger, already-validated XPC limit.
#[cfg(feature = "tunnel")]
async fn read_xpc_body_in_chunks<S>(
    framer: &mut H2Framer<S>,
    stream_id: u32,
    body_len: usize,
) -> Result<Bytes, H2Error>
where
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin,
{
    if let Some(bank) = framer.raw_bank() {
        let mut body = crate::xpc::raw_storage::MutableBytes::zeroed(&bank, body_len)?;
        let mut offset = 0;
        const CHUNK: usize = MAX_BUFFERED_BYTES_PER_STREAM - MAX_FRAME_PAYLOAD;
        while offset < body_len {
            let len = (body_len - offset).min(CHUNK);
            let chunk = framer.read_stream(stream_id, len).await?;
            body.bytes[offset..offset + len].copy_from_slice(&chunk);
            offset += len;
        }
        return Ok(body.freeze());
    }
    // A DATA frame can overshoot read_stream's requested length by one frame.
    // Leave that much headroom so bytes left over after the 24-byte XPC header
    // cannot make an otherwise exact 16 MiB read trip the H2 buffer limit.
    const MAX_XPC_BODY_READ_CHUNK: usize = MAX_BUFFERED_BYTES_PER_STREAM - MAX_FRAME_PAYLOAD;
    let mut body = BytesMut::with_capacity(body_len.min(MAX_XPC_BODY_READ_CHUNK));
    let mut remaining = body_len;
    while remaining != 0 {
        let chunk_len = remaining.min(MAX_XPC_BODY_READ_CHUNK);
        let chunk = framer.read_stream(stream_id, chunk_len).await?;
        body.extend_from_slice(&chunk);
        remaining -= chunk_len;
    }
    Ok(body.freeze())
}

#[cfg(feature = "tunnel")]
async fn read_raw_xpc_on_client_server<S>(
    framer: &mut H2Framer<S>,
) -> Result<(Bytes, Bytes), XpcError>
where
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin,
{
    let mut operation = RsdOperation::begin(framer)?;
    let result = await_rsd_operation(read_raw_xpc_on_client_server_inner(
        operation.framer,
        &mut operation.progressed,
    ))
    .await;
    operation.finish(result)
}

#[cfg(feature = "tunnel")]
async fn read_raw_xpc_on_client_server_inner<S>(
    framer: &mut H2Framer<S>,
    progressed: &mut bool,
) -> Result<(Bytes, Bytes), XpcError>
where
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin,
{
    let header = framer
        .read_client_server(24)
        .await
        .map_err(|e| h2_error("read header: ", e))?;
    *progressed = true;
    let declared_body_len = u64::from_le_bytes(header[8..16].try_into().map_err(|error| {
        crate::xpc::native_error(
            std::io::ErrorKind::InvalidData,
            "decode RSD client body length",
            error,
        )
    })?);
    let message_flags = u32::from_le_bytes(header[4..8].try_into().map_err(|error| {
        crate::xpc::native_error(
            std::io::ErrorKind::InvalidData,
            "decode RSD client flags",
            error,
        )
    })?);
    let body_len = checked_xpc_body_len(declared_body_len, xpc_body_limit_for_flags(message_flags))
        .map_err(XpcError::Io)?;
    let body = if body_len > 0 {
        read_xpc_body_in_chunks(framer, crate::xpc::h2_raw::STREAM_CLIENT_SERVER, body_len)
            .await
            .map_err(|e| h2_error("read body: ", e))?
    } else {
        Bytes::new()
    };
    Ok((header, body))
}

#[cfg(feature = "tunnel")]
async fn read_raw_xpc_on_server_client<S>(
    framer: &mut H2Framer<S>,
) -> Result<(Bytes, Bytes), XpcError>
where
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin,
{
    let mut operation = RsdOperation::begin(framer)?;
    let result = await_rsd_operation(read_raw_xpc_on_server_client_inner(
        operation.framer,
        &mut operation.progressed,
    ))
    .await;
    operation.finish(result)
}

#[cfg(feature = "tunnel")]
async fn read_raw_xpc_on_server_client_inner<S>(
    framer: &mut H2Framer<S>,
    progressed: &mut bool,
) -> Result<(Bytes, Bytes), XpcError>
where
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin,
{
    let header = framer
        .read_server_client(24)
        .await
        .map_err(|e| h2_error("read header: ", e))?;
    *progressed = true;
    let declared_body_len = u64::from_le_bytes(header[8..16].try_into().map_err(|error| {
        crate::xpc::native_error(
            std::io::ErrorKind::InvalidData,
            "decode RSD server body length",
            error,
        )
    })?);
    let message_flags = u32::from_le_bytes(header[4..8].try_into().map_err(|error| {
        crate::xpc::native_error(
            std::io::ErrorKind::InvalidData,
            "decode RSD server flags",
            error,
        )
    })?);
    let body_len = checked_xpc_body_len(declared_body_len, xpc_body_limit_for_flags(message_flags))
        .map_err(XpcError::Io)?;
    let body = if body_len > 0 {
        read_xpc_body_in_chunks(framer, crate::xpc::h2_raw::STREAM_SERVER_CLIENT, body_len)
            .await
            .map_err(|e| h2_error("read body: ", e))?
    } else {
        Bytes::new()
    };
    Ok((header, body))
}

#[cfg(feature = "tunnel")]
fn parse_handshake_message(msg: XpcMessage) -> Result<RsdHandshake, XpcError> {
    let dict = msg
        .body
        .as_ref()
        .and_then(|b| b.as_dict())
        .ok_or_else(|| XpcError::Tls("RSD: expected XPC dict body".into()))?;
    let message_type = dict
        .get("MessageType")
        .and_then(|v| v.as_str())
        .ok_or_else(|| XpcError::Tls("RSD: missing Handshake MessageType".into()))?;
    if message_type != "Handshake" {
        return Err(XpcError::Tls(
            "RSD: unexpected MessageType; expected Handshake".into(),
        ));
    }
    // UDID
    let udid = dict
        .get("Properties")
        .and_then(|v| v.as_dict())
        .and_then(|d| d.get("UniqueDeviceID"))
        .and_then(|v| v.as_str())
        .ok_or_else(|| XpcError::Tls("RSD: missing UniqueDeviceID".into()))?
        .to_string();

    // Services
    let mut services = HashMap::new();
    match dict.get("Services") {
        Some(XpcValue::Dictionary(svc_map)) => {
            tracing::debug!("RSD handshake exposed {} service entries", svc_map.len());
            for (name, svc_val) in svc_map {
                if let Some(svc_dict) = svc_val.as_dict() {
                    // Port can be a String or Uint64. Reject out-of-range
                    // integers rather than truncating them into a different
                    // service port.
                    let port = svc_dict.get("Port").and_then(|p| match p {
                        XpcValue::String(s) => s.parse::<u16>().ok(),
                        XpcValue::Uint64(n) => u16::try_from(*n).ok(),
                        _ => None,
                    });
                    if let Some(port) = port {
                        let features = svc_dict
                            .get("Properties")
                            .and_then(XpcValue::as_dict)
                            .and_then(|properties| properties.get("Features"))
                            .and_then(|features| match features {
                                XpcValue::Array(features) => Some(features),
                                _ => None,
                            })
                            .map(|features| {
                                features
                                    .iter()
                                    .filter_map(XpcValue::as_str)
                                    .map(str::to_owned)
                                    .collect()
                            })
                            .unwrap_or_default();
                        services.insert(name.clone(), ServiceDescriptor { port, features });
                    } else {
                        tracing::debug!("RSD service has an invalid or missing Port");
                    }
                }
            }
        }
        Some(other) => {
            tracing::debug!(
                "RSD handshake Services has unexpected XPC type: {}",
                xpc_value_kind(other)
            );
        }
        None => {
            tracing::debug!("RSD handshake missing Services key");
        }
    }

    Ok(RsdHandshake { udid, services })
}

#[cfg(feature = "tunnel")]
fn xpc_value_kind(value: &XpcValue) -> &'static str {
    match value {
        XpcValue::Null => "null",
        XpcValue::Bool(_) => "bool",
        XpcValue::Int64(_) => "int64",
        XpcValue::Uint64(_) => "uint64",
        XpcValue::Double(_) => "double",
        XpcValue::Date(_) => "date",
        XpcValue::Data(_) => "data",
        XpcValue::String(_) => "string",
        XpcValue::Uuid(_) => "uuid",
        XpcValue::Array(_) => "array",
        XpcValue::Dictionary(_) => "dictionary",
        XpcValue::FileTransfer { .. } => "file-transfer",
    }
}

/// Ceiling on unmatched messages parked per stream while awaiting a reply id.
///
/// [`XpcConnection::recv_reply_on_stream`] holds on to every message that is not
/// the reply it wants so a later `recv` can still see it. A device that never
/// sends the awaited id would otherwise keep that queue growing for the lifetime
/// of the connection, so an overlong backlog is treated as a protocol fault
/// instead of being absorbed silently.
#[cfg(feature = "tunnel")]
const MAX_PENDING_MESSAGES_PER_STREAM: usize = 1024;

/// A pending XPC reply may be as large as the data-stream body limit. Keep
/// enough room for one such reply plus ordinary metadata on each stream, while
/// bounding aggregate retention when several streams are waiting concurrently.
/// These limits apply only to messages parked while matching a reply; the H2
/// framer's separate 8/64 MiB body limits continue to govern wire reads.
#[cfg(feature = "tunnel")]
const DEFAULT_PENDING_BYTES_PER_STREAM: usize = 128 * 1024 * 1024;
#[cfg(feature = "tunnel")]
const DEFAULT_PENDING_BYTES_CONNECTION: usize = 512 * 1024 * 1024;

#[cfg(feature = "tunnel")]
struct PendingMessage {
    message: XpcMessage,
    bytes: usize,
}

/// Count the allocations retained by an already-decoded XPC message.
///
/// Container capacities are used instead of lengths because decoded values
/// retain their backing allocations. The inline enum/message fields are not
/// counted recursively; vector/map slots and boxed file-transfer values are
/// counted where they introduce an allocation.
#[cfg(feature = "tunnel")]
fn add_pending_size(size: &mut usize, amount: usize, kind: &str) -> Result<(), XpcError> {
    *size = size.checked_add(amount).ok_or_else(|| {
        XpcError::Tls(format!(
            "XPC pending message size overflow while accounting {kind}"
        ))
    })?;
    Ok(())
}

#[cfg(feature = "tunnel")]
fn xpc_value_memory_size(value: &XpcValue) -> Result<usize, XpcError> {
    let mut size = 0usize;

    match value {
        XpcValue::Data(data) => add_pending_size(&mut size, data.len(), "data")?,
        XpcValue::String(string) => add_pending_size(&mut size, string.capacity(), "string")?,
        XpcValue::Array(values) => {
            add_pending_size(
                &mut size,
                values
                    .capacity()
                    .checked_mul(size_of::<XpcValue>())
                    .ok_or_else(|| {
                        XpcError::Tls(
                            "XPC pending message size overflow while accounting array slots".into(),
                        )
                    })?,
                "array slots",
            )?;
            for child in values {
                add_pending_size(&mut size, xpc_value_memory_size(child)?, "array values")?;
            }
        }
        XpcValue::Dictionary(entries) => {
            add_pending_size(
                &mut size,
                entries
                    .capacity()
                    .checked_mul(size_of::<(String, XpcValue)>())
                    .ok_or_else(|| {
                        XpcError::Tls(
                            "XPC pending message size overflow while accounting dictionary slots"
                                .into(),
                        )
                    })?,
                "dictionary slots",
            )?;
            for (key, child) in entries {
                add_pending_size(&mut size, key.capacity(), "dictionary key")?;
                add_pending_size(
                    &mut size,
                    xpc_value_memory_size(child)?,
                    "dictionary values",
                )?;
            }
        }
        XpcValue::FileTransfer { data, .. } => {
            add_pending_size(&mut size, size_of::<XpcValue>(), "file-transfer value")?;
            size = size
                .checked_add(xpc_value_memory_size(data)?)
                .ok_or_else(|| {
                    XpcError::Tls(
                        "XPC pending message size overflow while accounting file-transfer data"
                            .into(),
                    )
                })?;
        }
        XpcValue::Null
        | XpcValue::Bool(_)
        | XpcValue::Int64(_)
        | XpcValue::Uint64(_)
        | XpcValue::Double(_)
        | XpcValue::Date(_)
        | XpcValue::Uuid(_) => {}
    }

    Ok(size)
}

#[cfg(feature = "tunnel")]
fn xpc_message_memory_size(message: &XpcMessage) -> Result<usize, XpcError> {
    size_of::<XpcMessage>()
        .checked_add(
            message
                .body
                .as_ref()
                .map(xpc_value_memory_size)
                .transpose()?
                .unwrap_or_default(),
        )
        .ok_or_else(|| XpcError::Tls("XPC pending message size overflow".into()))
}

#[cfg(feature = "tunnel")]
struct PendingMessageBudget {
    per_stream_limit: usize,
    connection_limit: usize,
    per_stream_bytes: HashMap<u32, usize>,
    total_bytes: usize,
}

#[cfg(feature = "tunnel")]
impl PendingMessageBudget {
    fn new(per_stream_limit: usize, connection_limit: usize) -> Self {
        Self {
            per_stream_limit,
            connection_limit,
            per_stream_bytes: HashMap::new(),
            total_bytes: 0,
        }
    }

    fn reserve(&mut self, stream_id: u32, bytes: usize) -> Result<(), XpcError> {
        let current_stream = self.per_stream_bytes.get(&stream_id).copied().unwrap_or(0);
        let next_stream = current_stream.checked_add(bytes).ok_or_else(|| {
            XpcError::Tls(format!(
                "XPC pending bytes overflow on stream {stream_id}: current {current_stream}, incoming {bytes}"
            ))
        })?;
        let next_total = self.total_bytes.checked_add(bytes).ok_or_else(|| {
            XpcError::Tls(format!(
                "XPC pending connection bytes overflow: current {}, incoming {bytes}",
                self.total_bytes
            ))
        })?;
        if next_stream > self.per_stream_limit {
            return Err(XpcError::Tls(format!(
                "XPC pending bytes {next_stream} on stream {stream_id} exceed per-stream limit {} (current {current_stream}, incoming {bytes})",
                self.per_stream_limit
            )));
        }
        if next_total > self.connection_limit {
            return Err(XpcError::Tls(format!(
                "XPC pending connection bytes {next_total} exceed limit {} (current {}, incoming {bytes})",
                self.connection_limit, self.total_bytes
            )));
        }

        self.per_stream_bytes.insert(stream_id, next_stream);
        self.total_bytes = next_total;
        Ok(())
    }

    fn replace(
        &mut self,
        stream_id: u32,
        old_bytes: usize,
        new_bytes: usize,
    ) -> Result<(), XpcError> {
        let current_stream = self.per_stream_bytes.get(&stream_id).copied().unwrap_or(0);
        let base_stream = current_stream.checked_sub(old_bytes).ok_or_else(|| {
            XpcError::Tls(format!(
                "XPC pending byte accounting underflow on stream {stream_id}: current {current_stream}, replacing {old_bytes}"
            ))
        })?;
        let base_total = self.total_bytes.checked_sub(old_bytes).ok_or_else(|| {
            XpcError::Tls(format!(
                "XPC pending connection byte accounting underflow: current {}, replacing {old_bytes}",
                self.total_bytes
            ))
        })?;
        let next_stream = base_stream.checked_add(new_bytes).ok_or_else(|| {
            XpcError::Tls(format!(
                "XPC pending bytes overflow on stream {stream_id}: base {base_stream}, replacement {new_bytes}"
            ))
        })?;
        let next_total = base_total.checked_add(new_bytes).ok_or_else(|| {
            XpcError::Tls(format!(
                "XPC pending connection bytes overflow: base {base_total}, replacement {new_bytes}"
            ))
        })?;
        if next_stream > self.per_stream_limit {
            return Err(XpcError::Tls(format!(
                "XPC replacement pending bytes {next_stream} on stream {stream_id} exceed per-stream limit {} (old {old_bytes}, new {new_bytes})",
                self.per_stream_limit
            )));
        }
        if next_total > self.connection_limit {
            return Err(XpcError::Tls(format!(
                "XPC replacement pending connection bytes {next_total} exceed limit {} (old {old_bytes}, new {new_bytes})",
                self.connection_limit
            )));
        }

        if next_stream == 0 {
            self.per_stream_bytes.remove(&stream_id);
        } else {
            self.per_stream_bytes.insert(stream_id, next_stream);
        }
        self.total_bytes = next_total;
        Ok(())
    }

    fn release(&mut self, stream_id: u32, bytes: usize) {
        if let Some(current_stream) = self.per_stream_bytes.get(&stream_id).copied() {
            debug_assert!(current_stream >= bytes);
            let remaining = current_stream.saturating_sub(bytes);
            if remaining == 0 {
                self.per_stream_bytes.remove(&stream_id);
            } else {
                self.per_stream_bytes.insert(stream_id, remaining);
            }
        }
        debug_assert!(self.total_bytes >= bytes);
        self.total_bytes = self.total_bytes.saturating_sub(bytes);
    }

    fn clear_stream(&mut self, stream_id: u32) {
        if let Some(bytes) = self.per_stream_bytes.remove(&stream_id) {
            debug_assert!(self.total_bytes >= bytes);
            self.total_bytes = self.total_bytes.saturating_sub(bytes);
        }
    }

    fn clear(&mut self) {
        self.per_stream_bytes.clear();
        self.total_bytes = 0;
    }
}

/// A live XPC connection to an iOS 17+ service.
#[cfg(feature = "tunnel")]
pub struct XpcConnection<S> {
    framer: H2Framer<S>,
    msg_id: u64,
    pending_messages: HashMap<u32, VecDeque<PendingMessage>>,
    pending_budget: PendingMessageBudget,
    operation_progressed: bool,
}

#[cfg(feature = "tunnel")]
pub(crate) struct XpcOperation<'a, S> {
    connection: &'a mut XpcConnection<S>,
    deadline: Option<tokio::time::Instant>,
    completed: bool,
}

#[cfg(feature = "tunnel")]
impl<S> XpcConnection<S> {
    pub fn is_usable(&self) -> bool {
        self.framer.is_usable()
    }

    fn invalidate(&mut self) {
        self.framer.invalidate();
        self.pending_messages.clear();
        self.pending_budget.clear();
    }
}

#[cfg(feature = "tunnel")]
impl<S> XpcOperation<'_, S> {
    pub(crate) fn complete(&mut self) {
        self.completed = true;
    }

    fn finish<T>(
        mut self,
        result: Result<Result<T, XpcError>, tokio::time::error::Elapsed>,
    ) -> Result<T, XpcError> {
        match result {
            Ok(result) => {
                self.completed = true;
                if !self.connection.is_usable()
                    || (result.is_err() && self.connection.operation_progressed)
                {
                    self.connection.invalidate();
                }
                result
            }
            Err(elapsed) => Err(std::io::Error::new(std::io::ErrorKind::TimedOut, elapsed).into()),
        }
    }
}

#[cfg(feature = "tunnel")]
impl<S> Drop for XpcOperation<'_, S> {
    fn drop(&mut self) {
        if !self.completed && (self.connection.operation_progressed || !self.connection.is_usable())
        {
            self.connection.invalidate();
        }
    }
}

#[cfg(feature = "tunnel")]
fn await_xpc_operation<T>(
    deadline: Option<tokio::time::Instant>,
    operation: impl std::future::Future<Output = Result<T, XpcError>>,
) -> impl std::future::Future<Output = Result<Result<T, XpcError>, tokio::time::error::Elapsed>> {
    // Keep nested call/bootstrap futures small without detaching their owner.
    let operation = Box::pin(operation);
    async move {
        match deadline {
            Some(deadline) => tokio::time::timeout_at(deadline, operation).await,
            None => Ok(operation.await),
        }
    }
}

#[cfg(feature = "tunnel")]
impl<S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin> XpcOperation<'_, S> {
    pub(crate) async fn send_with_flags(
        &mut self,
        body: XpcValue,
        extra_flags: u32,
    ) -> Result<u64, XpcError> {
        self.connection
            .send_with_flags_inner(body, extra_flags)
            .await
    }

    pub(crate) async fn recv(&mut self) -> Result<XpcMessage, XpcError> {
        self.connection
            .recv_on_stream_inner(crate::xpc::h2_raw::STREAM_SERVER_CLIENT)
            .await
    }
}

#[cfg(feature = "tunnel")]
impl<S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin> XpcConnection<S> {
    pub fn new(framer: H2Framer<S>) -> Self {
        Self::with_pending_memory_limits(
            framer,
            DEFAULT_PENDING_BYTES_PER_STREAM,
            DEFAULT_PENDING_BYTES_CONNECTION,
        )
    }

    /// Construct a connection with explicit pending-reply memory limits.
    ///
    /// This is useful for constrained embedders and deterministic tests. A
    /// normal connection should use [`Self::new`], whose limits are sized for
    /// the XPC control/data body limits.
    pub fn with_pending_memory_limits(
        framer: H2Framer<S>,
        per_stream_limit: usize,
        connection_limit: usize,
    ) -> Self {
        Self {
            framer,
            msg_id: 1,
            pending_messages: HashMap::new(),
            pending_budget: PendingMessageBudget::new(per_stream_limit, connection_limit),
            operation_progressed: false,
        }
    }

    /// Number of bytes currently retained by unmatched replies.
    #[allow(dead_code)]
    pub fn pending_memory_bytes_used(&self) -> usize {
        self.pending_budget.total_bytes
    }

    pub(crate) fn begin_operation(
        &mut self,
        budget: Option<std::time::Duration>,
    ) -> Result<XpcOperation<'_, S>, XpcError> {
        if !self.is_usable() {
            self.invalidate();
            return Err(h2_error(
                "",
                H2Error::Protocol(
                    "XPC connection is unusable; reconnect before another operation".into(),
                ),
            ));
        }
        let deadline = match budget {
            Some(budget) if budget.is_zero() => {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::TimedOut,
                    "XPC operation budget is zero",
                )
                .into());
            }
            Some(budget) => Some(tokio::time::Instant::now().checked_add(budget).ok_or_else(
                || {
                    std::io::Error::new(
                        std::io::ErrorKind::InvalidInput,
                        "XPC operation deadline is too large",
                    )
                },
            )?),
            None => None,
        };
        self.operation_progressed = false;
        Ok(XpcOperation {
            connection: self,
            deadline,
            completed: false,
        })
    }

    fn next_id(&mut self) -> Result<u64, XpcError> {
        let id = self.msg_id;
        self.msg_id = id
            .checked_add(1)
            .ok_or_else(|| XpcError::Tls("XPC message identifier exhausted".into()))?;
        Ok(id)
    }

    /// Send a dictionary as an XPC message on the clientServer stream.
    pub async fn send(&mut self, body: XpcValue) -> Result<(), XpcError> {
        self.send_with_flags(body, 0).await.map(|_| ())
    }

    /// Send a dictionary as an XPC message on the clientServer stream with
    /// additional wrapper flags.
    pub async fn send_with_flags(
        &mut self,
        body: XpcValue,
        extra_flags: u32,
    ) -> Result<u64, XpcError> {
        let operation = self.begin_operation(None)?;
        let result = await_xpc_operation(
            operation.deadline,
            operation
                .connection
                .send_with_flags_inner(body, extra_flags),
        )
        .await;
        operation.finish(result)
    }

    async fn send_with_flags_inner(
        &mut self,
        body: XpcValue,
        extra_flags: u32,
    ) -> Result<u64, XpcError> {
        let id = self.next_id()?;
        let msg = XpcMessage {
            flags: flags::ALWAYS_SET | flags::DATA | extra_flags,
            msg_id: id,
            body: Some(body),
        };
        let bytes = crate::xpc::message::encode_message(&msg)?;
        self.framer
            .write_client_server(&bytes)
            .await
            .map_err(|e| h2_error("", e))?;
        self.operation_progressed = true;
        Ok(id)
    }

    pub(crate) async fn call_on_stream(
        &mut self,
        body: XpcValue,
        stream_id: u32,
        budget: Option<std::time::Duration>,
    ) -> Result<XpcMessage, XpcError> {
        let operation = self.begin_operation(budget)?;
        let result = await_xpc_operation(operation.deadline, async {
            let connection = &mut *operation.connection;
            let id = connection
                .send_with_flags_inner(body, flags::WANTING_REPLY)
                .await?;
            connection.recv_reply_on_stream_inner(stream_id, id).await
        })
        .await;
        operation.finish(result)
    }

    #[cfg(feature = "pasteboard")]
    pub(crate) async fn call_nonempty(&mut self, body: XpcValue) -> Result<XpcValue, XpcError> {
        let operation = self.begin_operation(None)?;
        let result = await_xpc_operation(operation.deadline, async {
            let connection = &mut *operation.connection;
            connection
                .send_with_flags_inner(body, flags::WANTING_REPLY)
                .await?;
            loop {
                let message = connection
                    .recv_on_stream_inner(crate::xpc::h2_raw::STREAM_SERVER_CLIENT)
                    .await?;
                let Some(body) = message.body else { continue };
                if matches!(&body, XpcValue::Dictionary(values) if values.is_empty()) {
                    continue;
                }
                return Ok(body);
            }
        })
        .await;
        operation.finish(result)
    }

    /// Receive one XPC message from the serverClient stream.
    pub async fn recv(&mut self) -> Result<XpcMessage, XpcError> {
        self.recv_on_stream(crate::xpc::h2_raw::STREAM_SERVER_CLIENT)
            .await
    }

    /// Receive one XPC message from the clientServer stream.
    pub async fn recv_client_server(&mut self) -> Result<XpcMessage, XpcError> {
        self.recv_on_stream(crate::xpc::h2_raw::STREAM_CLIENT_SERVER)
            .await
    }

    /// Receive one fresh XPC message from whichever stream the peer sends it
    /// on.
    ///
    /// RemoteXPC services such as InstallCoordinationProxy answer with
    /// fresh, uncorrelated messages whose stream is not part of the request
    /// contract; pymobiledevice3 consumes the next DATA frame regardless of
    /// its stream id.  Frames are dispatched into per-stream buffers, so a
    /// message is always reassembled strictly from its own stream, data
    /// already buffered on other streams is preserved, and the framer's
    /// per-stream and connection memory limits keep applying.
    pub async fn recv_any_stream(&mut self) -> Result<XpcMessage, XpcError> {
        self.recv_any_stream_with_budget(None).await
    }

    pub(crate) async fn recv_any_stream_with_budget(
        &mut self,
        budget: Option<std::time::Duration>,
    ) -> Result<XpcMessage, XpcError> {
        let operation = self.begin_operation(budget)?;
        let result = await_xpc_operation(
            operation.deadline,
            operation.connection.recv_any_stream_inner(),
        )
        .await;
        operation.finish(result)
    }

    async fn recv_any_stream_inner(&mut self) -> Result<XpcMessage, XpcError> {
        let stream_id = self
            .framer
            .buffer_next_data_stream()
            .await
            .map_err(|e| h2_error("", e))?;
        self.recv_fresh_on_stream(stream_id).await
    }

    async fn recv_on_stream(&mut self, stream_id: u32) -> Result<XpcMessage, XpcError> {
        self.recv_on_stream_with_budget(stream_id, None).await
    }

    pub(crate) async fn recv_on_stream_with_budget(
        &mut self,
        stream_id: u32,
        budget: Option<std::time::Duration>,
    ) -> Result<XpcMessage, XpcError> {
        let operation = self.begin_operation(budget)?;
        let result = await_xpc_operation(
            operation.deadline,
            operation.connection.recv_on_stream_inner(stream_id),
        )
        .await;
        operation.finish(result)
    }

    async fn recv_on_stream_inner(&mut self, stream_id: u32) -> Result<XpcMessage, XpcError> {
        if let Some(message) = self.pop_next_pending_message(stream_id) {
            return Ok(message);
        }
        self.recv_fresh_on_stream(stream_id).await
    }

    /// Receive the reply with a specific XPC message id from one stream.
    #[allow(dead_code)]
    pub async fn recv_reply_on_stream(
        &mut self,
        stream_id: u32,
        msg_id: u64,
    ) -> Result<XpcMessage, XpcError> {
        let operation = self.begin_operation(None)?;
        let result = await_xpc_operation(
            operation.deadline,
            operation
                .connection
                .recv_reply_on_stream_inner(stream_id, msg_id),
        )
        .await;
        operation.finish(result)
    }

    async fn recv_reply_on_stream_inner(
        &mut self,
        stream_id: u32,
        msg_id: u64,
    ) -> Result<XpcMessage, XpcError> {
        if let Some(message) = self.take_pending_message(stream_id, msg_id) {
            return Ok(message);
        }

        loop {
            let message = self.recv_fresh_on_stream(stream_id).await?;
            if message.msg_id == msg_id {
                return Ok(message);
            }
            if let Err(err) = self.push_pending_message(stream_id, message) {
                self.clear_pending_stream(stream_id);
                return Err(err);
            }
        }
    }

    async fn recv_fresh_on_stream(&mut self, stream_id: u32) -> Result<XpcMessage, XpcError> {
        let result = self.recv_fresh_on_stream_inner(stream_id).await;
        if result.is_err() {
            self.clear_pending_stream(stream_id);
        }
        result
    }

    async fn recv_fresh_on_stream_inner(&mut self, stream_id: u32) -> Result<XpcMessage, XpcError> {
        let header = self
            .framer
            .read_stream(stream_id, 24)
            .await
            .map_err(|e| h2_error("", e))?;
        // The header is now owned by this future even if its bytes came from
        // an earlier H2 operation. Cancellation must not lose that boundary.
        self.operation_progressed = true;
        let declared_body_len = u64::from_le_bytes(header[8..16].try_into().map_err(|error| {
            crate::xpc::native_error(
                std::io::ErrorKind::InvalidData,
                "decode XPC response body length",
                error,
            )
        })?);
        let message_flags = u32::from_le_bytes(header[4..8].try_into().map_err(|error| {
            crate::xpc::native_error(
                std::io::ErrorKind::InvalidData,
                "decode XPC response flags",
                error,
            )
        })?);
        let body_len =
            checked_xpc_body_len(declared_body_len, xpc_body_limit_for_flags(message_flags))
                .map_err(XpcError::Io)?;
        let body = if body_len > 0 {
            read_xpc_body_in_chunks(&mut self.framer, stream_id, body_len)
                .await
                .map_err(|e| h2_error("", e))?
        } else {
            Bytes::new()
        };
        let mut full = bytes::BytesMut::new();
        full.extend_from_slice(&header);
        full.extend_from_slice(&body);
        decode_message(full.freeze())
    }

    #[cfg(feature = "cryptex")]
    pub(crate) async fn send_file_transfer<R: tokio::io::AsyncRead + Unpin>(
        &mut self,
        transfer_id: u64,
        size: u64,
        reader: &mut R,
    ) -> Result<(), XpcError> {
        if !(1..=5).contains(&transfer_id) || size == 0 || size > 2 * 1024 * 1024 * 1024 {
            return Err(XpcError::Tls(
                "invalid Cryptex file transfer ID or size".into(),
            ));
        }
        let operation = self.begin_operation(None)?;
        let result = await_xpc_operation(
            operation.deadline,
            operation
                .connection
                .send_file_transfer_inner(transfer_id, size, reader),
        )
        .await;
        operation.finish(result)
    }

    #[cfg(feature = "cryptex")]
    async fn send_file_transfer_inner<R: tokio::io::AsyncRead + Unpin>(
        &mut self,
        transfer_id: u64,
        size: u64,
        reader: &mut R,
    ) -> Result<(), XpcError> {
        use tokio::io::AsyncReadExt;
        let stream_id = 5 + (transfer_id as u32 - 1) * 2;
        let preamble = crate::xpc::message::encode_message(&XpcMessage {
            flags: flags::ALWAYS_SET | flags::FILE_TX_STREAM_REQUEST,
            msg_id: transfer_id,
            body: None,
        })?;
        let framer = &mut self.framer;
        framer
            .write_stream(stream_id, &preamble)
            .await
            .map_err(|e| h2_error("", e))?;
        self.operation_progressed = true;
        let mut remaining = size;
        let mut chunk = vec![0u8; 64 * 1024];
        while remaining > 0 {
            let amount = remaining.min(chunk.len() as u64) as usize;
            reader.read_exact(&mut chunk[..amount]).await?;
            framer
                .write_stream(stream_id, &chunk[..amount])
                .await
                .map_err(|e| h2_error("", e))?;
            remaining -= amount as u64;
        }
        if reader.read(&mut chunk[..1]).await? != 0 {
            return Err(XpcError::Tls("file transfer exceeds announced size".into()));
        }
        framer
            .finish_stream(stream_id)
            .await
            .map_err(|e| h2_error("", e))
    }

    fn push_pending_message(
        &mut self,
        stream_id: u32,
        message: XpcMessage,
    ) -> Result<(), XpcError> {
        let bytes = xpc_message_memory_size(&message)?;
        let replacement_index = if message.msg_id == 0 {
            None
        } else {
            self.pending_messages.get(&stream_id).and_then(|pending| {
                pending
                    .iter()
                    .position(|entry| entry.message.msg_id == message.msg_id)
            })
        };

        if let Some(index) = replacement_index {
            return self.replace_pending_message(stream_id, index, message, bytes);
        }

        let pending_len = self
            .pending_messages
            .get(&stream_id)
            .map_or(0, VecDeque::len);
        if pending_len >= MAX_PENDING_MESSAGES_PER_STREAM {
            return Err(XpcError::Tls(format!(
                "XPC: more than {MAX_PENDING_MESSAGES_PER_STREAM} unmatched messages \
                 buffered on stream {stream_id} while waiting for a reply"
            )));
        }

        self.pending_budget.reserve(stream_id, bytes)?;
        self.pending_messages
            .entry(stream_id)
            .or_default()
            .push_back(PendingMessage { message, bytes });
        Ok(())
    }

    fn replace_pending_message(
        &mut self,
        stream_id: u32,
        index: usize,
        message: XpcMessage,
        bytes: usize,
    ) -> Result<(), XpcError> {
        let old_bytes = self
            .pending_messages
            .get(&stream_id)
            .and_then(|pending| pending.get(index))
            .map(|entry| entry.bytes)
            .ok_or_else(|| {
                XpcError::Tls(format!(
                    "XPC pending message replacement index {index} missing on stream {stream_id}"
                ))
            })?;
        self.pending_budget.replace(stream_id, old_bytes, bytes)?;

        if let Some(entry) = self
            .pending_messages
            .get_mut(&stream_id)
            .and_then(|pending| pending.get_mut(index))
        {
            *entry = PendingMessage { message, bytes };
            return Ok(());
        }

        // Keep the accounting consistent even if an internal queue invariant
        // is ever violated between the two lookups above.
        let _ = self.pending_budget.replace(stream_id, bytes, old_bytes);
        Err(XpcError::Tls(format!(
            "XPC pending message replacement index {index} disappeared on stream {stream_id}"
        )))
    }

    fn pop_next_pending_message(&mut self, stream_id: u32) -> Option<XpcMessage> {
        let (message, bytes, empty) = {
            let pending = self.pending_messages.get_mut(&stream_id)?;
            let entry = pending.pop_front()?;
            (entry.message, entry.bytes, pending.is_empty())
        };
        if empty {
            self.pending_messages.remove(&stream_id);
        }
        self.pending_budget.release(stream_id, bytes);
        Some(message)
    }

    fn take_pending_message(&mut self, stream_id: u32, msg_id: u64) -> Option<XpcMessage> {
        let (message, bytes, empty) = {
            let pending = self.pending_messages.get_mut(&stream_id)?;
            let index = pending
                .iter()
                .position(|entry| entry.message.msg_id == msg_id)?;
            let entry = pending.remove(index)?;
            (entry.message, entry.bytes, pending.is_empty())
        };
        if empty {
            self.pending_messages.remove(&stream_id);
        }
        self.pending_budget.release(stream_id, bytes);
        Some(message)
    }

    fn clear_pending_stream(&mut self, stream_id: u32) {
        self.pending_messages.remove(&stream_id);
        self.pending_budget.clear_stream(stream_id);
    }
}

#[cfg(feature = "tunnel")]
impl<S> Drop for XpcConnection<S> {
    fn drop(&mut self) {
        self.pending_messages.clear();
        self.pending_budget.clear();
    }
}

#[cfg(test)]
#[cfg(feature = "tunnel")]
mod tests {
    use std::fmt;
    use std::pin::Pin;
    use std::sync::{Arc, Mutex};
    use std::task::{Context, Poll};

    use bytes::Bytes;
    use indexmap::IndexMap;
    use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt, ReadBuf};
    use tokio::time::{timeout, Duration};

    use super::*;
    use crate::xpc::message::{encode_message, flags, XpcMessage, XpcValue};

    const FRAME_DATA: u8 = 0x00;
    const FRAME_HEADERS: u8 = 0x01;
    const FRAME_SETTINGS: u8 = 0x04;
    const FLAG_END_HEADERS: u8 = 0x04;
    const FLAG_SETTINGS_ACK: u8 = 0x01;
    const STREAM_INIT: u32 = 0;
    const STREAM_CLIENT_SERVER: u32 = 1;
    const STREAM_SERVER_CLIENT: u32 = 3;

    async fn lifecycle_rsd_framer(
        capacity: usize,
    ) -> (H2Framer<tokio::io::DuplexStream>, tokio::io::DuplexStream) {
        let (stream, mut peer) = tokio::io::duplex(capacity);
        let (framer, ()) = timeout(Duration::from_secs(1), async {
            tokio::join!(H2Framer::connect(stream), async {
                read_test_preface(&mut peer).await;
                peer.write_all(&settings_frame()).await.unwrap();
                assert_eq!(read_test_frame(&mut peer).await.1, FLAG_SETTINGS_ACK);
            })
        })
        .await
        .unwrap();
        (framer.unwrap(), peer)
    }

    fn lifecycle_rsd_assert_elapsed(error: XpcError) {
        assert_eq!(crate::classify_error(&error), crate::ErrorClass::Timeout);
        let XpcError::Io(outer) = error else {
            panic!("RSD timer lost its native error")
        };
        assert_eq!(outer.kind(), std::io::ErrorKind::TimedOut);
        assert!(std::error::Error::source(&outer)
            .unwrap()
            .is::<tokio::time::error::Elapsed>());
    }

    async fn lifecycle_rsd_assert_rejected(framer: &mut H2Framer<tokio::io::DuplexStream>) {
        assert!(!framer.is_usable());
        let results = [
            initialize_xpc_connection_on_framer(framer).await,
            queue_rsd_handshake_bootstrap_on_framer(framer).await,
            handshake_on_framer(framer).await.map(|_| ()),
            send_device_handshake_on_framer(framer, uuid::Uuid::from_bytes([0x4a; 16])).await,
            read_raw_xpc_on_client_server(framer).await.map(|_| ()),
            read_raw_xpc_on_server_client(framer).await.map(|_| ()),
        ];
        for result in results {
            assert_eq!(
                crate::classify_error(&result.unwrap_err()),
                crate::ErrorClass::Protocol
            );
        }
        assert!(framer.read_stream(5, 1).await.is_err());
    }

    #[tokio::test]
    async fn lifecycle_rsd_empty_wait_and_unpolled_bootstrap_preserve_reuse() {
        timeout(Duration::from_secs(1), async {
            let (mut framer, mut peer) = lifecycle_rsd_framer(4096).await;
            drop(initialize_xpc_connection_on_framer(&mut framer));
            drop(queue_rsd_handshake_bootstrap_on_framer(&mut framer));
            drop(handshake_on_framer(&mut framer));
            assert!(
                timeout(Duration::from_millis(5), handshake_on_framer(&mut framer))
                    .await
                    .is_err()
            );
            assert!(framer.is_usable());

            framer.set_operation_timeout(Some(Duration::ZERO));
            let error = initialize_xpc_connection_on_framer(&mut framer)
                .await
                .unwrap_err();
            let XpcError::Io(outer) = error else {
                panic!("H2 zero budget changed its error type")
            };
            assert_eq!(outer.kind(), std::io::ErrorKind::TimedOut);
            assert!(framer.is_usable());
            framer.set_operation_timeout(None);
            assert!(timeout(Duration::from_millis(5), peer.read(&mut [0]))
                .await
                .is_err());

            // An ordinary bootstrap ACK is skipped with the existing parser.
            let ack = encode_message(&XpcMessage {
                flags: flags::ALWAYS_SET,
                msg_id: 0,
                body: None,
            })
            .unwrap();
            peer.write_all(&data_frame(1, &ack)).await.unwrap();
            peer.write_all(&data_frame(1, &sample_handshake_message()))
                .await
                .unwrap();
            let handshake = handshake_on_framer(&mut framer).await.unwrap();
            assert_eq!(
                handshake.get_port("com.apple.instruments.dtservicehub"),
                Some(12345)
            );
            assert!(framer.is_usable());
        })
        .await
        .unwrap();
    }

    #[tokio::test]
    async fn lifecycle_rsd_sent_initialization_cancel_and_expiry_reject_late_response() {
        for expires in [false, true] {
            timeout(Duration::from_secs(1), async {
                let (mut framer, mut peer) = lifecycle_rsd_framer(4096).await;
                let mut initialization = Box::pin(initialize_xpc_connection_on_framer(&mut framer));
                assert!(futures_util::poll!(&mut initialization).is_pending());
                assert_eq!(read_test_frame(&mut peer).await.0, FRAME_HEADERS);
                let first = read_test_frame(&mut peer).await;
                assert_eq!((first.0, first.2), (FRAME_DATA, 1));
                let message = decode_message(Bytes::from(first.3)).unwrap();
                assert_eq!(message.msg_id, 0);
                assert_eq!(message.flags, flags::ALWAYS_SET);
                assert!(message.body.unwrap().as_dict().unwrap().is_empty());
                if expires {
                    let error = rsd_timeout(
                        Duration::from_millis(5),
                        "RSD bootstrap timed out",
                        initialization,
                    )
                    .await
                    .unwrap_err();
                    lifecycle_rsd_assert_elapsed(error);
                } else {
                    drop(initialization);
                }

                peer.write_all(&data_frame(1, &sample_handshake_message()))
                    .await
                    .unwrap();
                lifecycle_rsd_assert_rejected(&mut framer).await;
                assert!(timeout(Duration::from_millis(5), peer.read(&mut [0]))
                    .await
                    .is_err());
                drop(framer);
                assert_eq!(peer.read(&mut [0]).await.unwrap(), 0);
            })
            .await
            .unwrap();
        }
    }

    #[tokio::test]
    async fn lifecycle_rsd_buffered_wrapper_cancel_and_expiry_reject_late_body() {
        for stream_id in [1, 3] {
            for expires in [false, true] {
                timeout(Duration::from_secs(1), async {
                    let (mut framer, mut peer) = lifecycle_rsd_framer(4096).await;
                    let message = encode_message(&pending_data_message(9, 7)).unwrap();
                    peer.write_all(&data_frame(stream_id, &message[..24]))
                        .await
                        .unwrap();
                    peer.write_all(&data_frame(5, b"xy")).await.unwrap();
                    assert_eq!(framer.read_stream(5, 1).await.unwrap().as_ref(), b"x");
                    // The entire wrapper is already buffered. Its body wait
                    // will make no new transport progress before cancellation.
                    let mut read = Box::pin(async {
                        if stream_id == 1 {
                            read_raw_xpc_on_client_server(&mut framer).await
                        } else {
                            read_raw_xpc_on_server_client(&mut framer).await
                        }
                    });
                    assert!(futures_util::poll!(&mut read).is_pending());
                    if expires {
                        lifecycle_rsd_assert_elapsed(
                            rsd_timeout(Duration::from_millis(5), "RSD response timed out", read)
                                .await
                                .unwrap_err(),
                        );
                    } else {
                        drop(read);
                    }
                    peer.write_all(&data_frame(stream_id, &message[24..]))
                        .await
                        .unwrap();
                    lifecycle_rsd_assert_rejected(&mut framer).await;
                    assert!(timeout(Duration::from_millis(5), peer.read(&mut [0]))
                        .await
                        .is_err());
                    drop(framer);
                    assert_eq!(peer.read(&mut [0]).await.unwrap(), 0);
                })
                .await
                .unwrap();
            }
        }
    }

    #[tokio::test]
    async fn lifecycle_rsd_queue_cancel_after_first_write_before_next_header_invalidates() {
        timeout(Duration::from_secs(1), async {
            let message = encode_message(&XpcMessage {
                flags: flags::ALWAYS_SET,
                msg_id: 0,
                body: Some(XpcValue::Dictionary(IndexMap::new())),
            })
            .unwrap();
            let expected = [headers_frame(1), data_frame(1, &message)].concat();
            let (mut framer, mut peer) = lifecycle_rsd_framer(expected.len()).await;
            let mut queue = Box::pin(queue_rsd_handshake_bootstrap_on_framer(&mut framer));
            assert!(futures_util::poll!(&mut queue).is_pending());
            // Duplex capacity ends at the first complete DATA frame. The
            // following stream-3 HEADERS write is pending before its first byte.
            drop(queue);
            assert!(!framer.is_usable());
            let mut wire = vec![0; expected.len()];
            peer.read_exact(&mut wire).await.unwrap();
            assert_eq!(wire, expected);
            lifecycle_rsd_assert_rejected(&mut framer).await;
            drop(framer);
            assert_eq!(peer.read(&mut [0]).await.unwrap(), 0);
        })
        .await
        .unwrap();
    }

    #[tokio::test]
    async fn lifecycle_rsd_stream_reset_preserves_idle_but_invalidates_committed_initialization() {
        for committed in [false, true] {
            timeout(Duration::from_secs(1), async {
                let (mut framer, mut peer) = lifecycle_rsd_framer(4096).await;
                peer.write_all(&build_frame(3, 0, 1, &8u32.to_be_bytes()))
                    .await
                    .unwrap();
                peer.write_all(&data_frame(3, b"z")).await.unwrap();
                let error = if committed {
                    initialize_xpc_connection_on_framer(&mut framer)
                        .await
                        .unwrap_err()
                } else {
                    handshake_on_framer(&mut framer).await.unwrap_err()
                };
                assert_eq!(crate::classify_error(&error), crate::ErrorClass::Protocol);
                let XpcError::Io(outer) = error else {
                    panic!("stream reset lost its H2 source")
                };
                assert_eq!(outer.kind(), std::io::ErrorKind::InvalidData);
                assert!(matches!(
                    std::error::Error::source(&outer)
                        .unwrap()
                        .downcast_ref::<H2Error>(),
                    Some(H2Error::Protocol(_))
                ));
                assert_eq!(framer.is_usable(), !committed);
                if !committed {
                    assert_eq!(framer.read_server_client(1).await.unwrap().as_ref(), b"z");
                }
            })
            .await
            .unwrap();
        }
    }

    #[tokio::test]
    async fn lifecycle_rsd_skipped_messages_and_invalid_wrappers_keep_handshake_ownership() {
        for mode in 0..4 {
            timeout(Duration::from_secs(1), async {
                let (mut framer, mut peer) = lifecycle_rsd_framer(4096).await;
                let mut message = encode_message(&XpcMessage {
                    flags: flags::ALWAYS_SET,
                    msg_id: 0,
                    body: None,
                })
                .unwrap()
                .to_vec();
                if mode == 2 {
                    message[..4].fill(0); // Existing wrapper decoder rejects this.
                } else if mode == 3 {
                    message[8..16].copy_from_slice(&u64::MAX.to_le_bytes());
                }
                peer.write_all(&data_frame(1, &message)).await.unwrap();
                if mode < 2 {
                    let mut handshake = Box::pin(handshake_on_framer(&mut framer));
                    assert!(futures_util::poll!(&mut handshake).is_pending());
                    if mode == 1 {
                        lifecycle_rsd_assert_elapsed(
                            rsd_timeout(
                                Duration::from_millis(5),
                                "RSD response timed out",
                                handshake,
                            )
                            .await
                            .unwrap_err(),
                        );
                    } else {
                        drop(handshake);
                    }
                } else {
                    let error = handshake_on_framer(&mut framer).await.unwrap_err();
                    assert_eq!(crate::classify_error(&error), crate::ErrorClass::Protocol);
                }
                lifecycle_rsd_assert_rejected(&mut framer).await;
            })
            .await
            .unwrap();
        }
    }

    #[tokio::test]
    async fn lifecycle_rsd_successful_initialization_and_queued_bootstrap_preserve_wire_and_reuse()
    {
        for queued in [false, true] {
            timeout(Duration::from_secs(1), async {
                let (mut framer, mut peer) = lifecycle_rsd_framer(4096).await;
                let first = encode_message(&XpcMessage {
                    flags: flags::ALWAYS_SET,
                    msg_id: 0,
                    body: Some(XpcValue::Dictionary(IndexMap::new())),
                })
                .unwrap();
                let second = encode_message(&XpcMessage {
                    flags: flags::ALWAYS_SET | 0x200,
                    msg_id: 0,
                    body: None,
                })
                .unwrap();
                let third = encode_message(&XpcMessage {
                    flags: flags::INIT_HANDSHAKE | flags::ALWAYS_SET,
                    msg_id: 0,
                    body: None,
                })
                .unwrap();
                let ((), ()) = tokio::join!(
                    async {
                        if queued {
                            queue_rsd_handshake_bootstrap_on_framer(&mut framer)
                                .await
                                .unwrap();
                        } else {
                            initialize_xpc_connection_on_framer(&mut framer)
                                .await
                                .unwrap();
                        }
                        assert!(framer.is_usable());
                        let directory = handshake_on_framer(&mut framer).await.unwrap();
                        assert_eq!(
                            directory.get_port("com.apple.instruments.dtservicehub"),
                            Some(12345)
                        );
                        assert!(framer.is_usable());
                        send_device_handshake_on_framer(
                            &mut framer,
                            uuid::Uuid::from_bytes([0x4a; 16]),
                        )
                        .await
                        .unwrap();
                        assert!(framer.is_usable());
                    },
                    async {
                        assert_eq!(read_test_frame(&mut peer).await.0, FRAME_HEADERS);
                        for (index, (stream_id, message)) in [(1, first), (1, second), (3, third)]
                            .into_iter()
                            .enumerate()
                        {
                            if index == 1 {
                                let header = read_test_frame(&mut peer).await;
                                assert_eq!((header.0, header.2), (FRAME_HEADERS, 3));
                            }
                            let frame = read_test_frame(&mut peer).await;
                            assert_eq!((frame.0, frame.2), (FRAME_DATA, stream_id));
                            assert_eq!(frame.3, message);
                            if !queued {
                                // Preserve the discard reader's historical
                                // header acceptance; it does not decode magic.
                                let mut ack = vec![0xA5; 24];
                                ack[8..16].fill(0);
                                peer.write_all(&data_frame(stream_id, &ack)).await.unwrap();
                            }
                        }
                        peer.write_all(&data_frame(1, &sample_handshake_message()))
                            .await
                            .unwrap();
                        let followup = read_test_frame(&mut peer).await;
                        assert_eq!((followup.0, followup.2), (FRAME_DATA, 1));
                        assert_eq!(
                            followup.3,
                            encode_message(&device_handshake_message(uuid::Uuid::from_bytes(
                                [0x4a; 16]
                            )))
                            .unwrap()
                        );
                    }
                );
            })
            .await
            .unwrap();
        }
    }

    async fn lifecycle_xpc_connection() -> (
        XpcConnection<tokio::io::DuplexStream>,
        tokio::io::DuplexStream,
    ) {
        let (stream, mut peer) = tokio::io::duplex(4096);
        let (framer, ()) = timeout(Duration::from_secs(1), async {
            tokio::join!(H2Framer::connect(stream), async {
                read_test_preface(&mut peer).await;
                peer.write_all(&settings_frame()).await.unwrap();
                assert_eq!(read_test_frame(&mut peer).await.0, FRAME_SETTINGS);
            })
        })
        .await
        .unwrap();
        (XpcConnection::new(framer.unwrap()), peer)
    }

    fn lifecycle_xpc_seed_pending(connection: &mut XpcConnection<tokio::io::DuplexStream>) {
        for stream in [1, 5] {
            connection
                .push_pending_message(stream, pending_data_message(99, 8))
                .unwrap();
        }
        assert!(connection.pending_budget.total_bytes > 0);
    }

    #[tokio::test]
    async fn lifecycle_xpc_consumed_buffered_header_invalidates_even_without_new_wire_bytes() {
        for caller_cancel in [false, true] {
            let (mut connection, mut peer) = lifecycle_xpc_connection().await;
            let message = encode_message(&pending_data_message(1, 8)).unwrap();
            peer.write_all(&data_frame(3, &message[..24]))
                .await
                .unwrap();
            peer.write_all(&data_frame(1, b"x")).await.unwrap();
            // An earlier H2 operation buffers the complete XPC header on 3.
            assert_eq!(
                connection.framer.read_stream(1, 1).await.unwrap().as_ref(),
                b"x"
            );
            lifecycle_xpc_seed_pending(&mut connection);
            assert!(connection.is_usable());
            if caller_cancel {
                assert!(timeout(Duration::from_millis(5), connection.recv())
                    .await
                    .is_err());
            } else {
                let error = connection
                    .recv_on_stream_with_budget(3, Some(Duration::from_millis(5)))
                    .await
                    .unwrap_err();
                let XpcError::Io(io) = error else {
                    panic!("actual expiry")
                };
                assert!(io.get_ref().unwrap().is::<tokio::time::error::Elapsed>());
            }
            assert!(!connection.is_usable());
            assert!(connection.pending_messages.is_empty());
            assert_eq!(connection.pending_budget.total_bytes, 0);
            assert!(connection.pending_budget.per_stream_bytes.is_empty());
            peer.write_all(&data_frame(3, &message)).await.unwrap();
            assert!(connection.recv().await.is_err());
            assert!(connection.recv_client_server().await.is_err());
            assert!(connection.recv_reply_on_stream(5, 99).await.is_err());
            assert!(connection.recv_any_stream().await.is_err());
        }
    }

    #[tokio::test]
    async fn lifecycle_xpc_native_eof_clears_pending_before_cached_reply_can_return() {
        let (mut connection, peer) = lifecycle_xpc_connection().await;
        lifecycle_xpc_seed_pending(&mut connection);
        drop(peer);
        let error = connection.recv().await.unwrap_err();
        let XpcError::Io(io) = error else {
            panic!("native EOF")
        };
        assert_eq!(io.kind(), std::io::ErrorKind::UnexpectedEof);
        assert!(!connection.is_usable());
        assert!(connection.pending_messages.is_empty());
        assert_eq!(connection.pending_budget.total_bytes, 0);
        assert!(connection.recv_client_server().await.is_err());
    }

    #[tokio::test]
    async fn lifecycle_xpc_partial_raw_frame_cancellation_clears_upper_pending_immediately() {
        let (mut connection, mut peer) = lifecycle_xpc_connection().await;
        lifecycle_xpc_seed_pending(&mut connection);
        peer.write_all(&[0, 0, 1, 0]).await.unwrap();
        assert!(timeout(Duration::from_millis(5), connection.recv())
            .await
            .is_err());
        assert!(!connection.is_usable());
        assert!(connection.pending_messages.is_empty());
        assert_eq!(connection.pending_budget.total_bytes, 0);
        assert!(connection.recv_client_server().await.is_err());
    }

    #[tokio::test]
    async fn lifecycle_xpc_bad_wrapper_invalidates_after_header_consumption() {
        let (mut connection, mut peer) = lifecycle_xpc_connection().await;
        lifecycle_xpc_seed_pending(&mut connection);
        peer.write_all(&data_frame(3, &[0; 24])).await.unwrap();
        assert!(connection.recv().await.is_err());
        assert!(!connection.is_usable());
        assert!(connection.pending_messages.is_empty());
        assert_eq!(connection.pending_budget.total_bytes, 0);
    }

    #[tokio::test]
    async fn lifecycle_xpc_complete_stream_reset_before_message_preserves_other_pending_reply() {
        let (mut connection, mut peer) = lifecycle_xpc_connection().await;
        connection.framer.open_stream(3).await.unwrap();
        assert_eq!(read_test_frame(&mut peer).await.0, FRAME_HEADERS);
        lifecycle_xpc_seed_pending(&mut connection);
        peer.write_all(&build_frame(3, 0, 3, &8u32.to_be_bytes()))
            .await
            .unwrap();
        assert!(connection.recv().await.is_err());
        assert!(connection.is_usable());
        assert_eq!(connection.recv_client_server().await.unwrap().msg_id, 99);
        assert!(connection.pending_budget.total_bytes > 0);
    }

    #[tokio::test]
    async fn lifecycle_xpc_counter_exhaustion_rejects_without_wrap_or_wire_io() {
        let (mut connection, mut peer) = lifecycle_xpc_connection().await;
        connection.msg_id = u64::MAX;
        assert!(connection.send(XpcValue::Bool(true)).await.is_err());
        assert_eq!(connection.msg_id, u64::MAX);
        assert!(connection.is_usable());
        let mut byte = [0];
        assert!(timeout(Duration::from_millis(5), peer.read(&mut byte))
            .await
            .is_err());
    }

    async fn read_test_frame<S: AsyncRead + Unpin>(stream: &mut S) -> (u8, u8, u32, Vec<u8>) {
        let mut header = [0; 9];
        stream.read_exact(&mut header).await.unwrap();
        let length =
            (usize::from(header[0]) << 16) | (usize::from(header[1]) << 8) | usize::from(header[2]);
        assert!(length <= 4096);
        let mut data = vec![0; length];
        stream.read_exact(&mut data).await.unwrap();
        (
            header[3],
            header[4],
            u32::from_be_bytes(header[5..9].try_into().unwrap()),
            data,
        )
    }

    async fn read_test_preface<S: AsyncRead + Unpin>(stream: &mut S) {
        let mut preface = [0; 24];
        stream.read_exact(&mut preface).await.unwrap();
        assert_eq!(&preface, crate::xpc::h2_raw::H2_PREFACE);
        assert_eq!(read_test_frame(stream).await.0, FRAME_SETTINGS);
        assert_eq!(read_test_frame(stream).await.0, 0x08);
    }

    async fn read_test_bootstrap<S: AsyncRead + Unpin>(stream: &mut S) {
        for (frame_type, stream_id) in [
            (FRAME_HEADERS, 1),
            (FRAME_DATA, 1),
            (FRAME_HEADERS, 3),
            (FRAME_DATA, 1),
            (FRAME_DATA, 3),
        ] {
            let frame = read_test_frame(stream).await;
            assert_eq!((frame.0, frame.2), (frame_type, stream_id));
        }
    }

    #[test]
    fn modern_device_handshake_preserves_xpc_wire_types() {
        let identity = uuid::Uuid::parse_str("49800e74-c74b-378c-958f-d88cb0dfdfd0").unwrap();
        let encoded = encode_message(&device_handshake_message(identity)).unwrap();
        let decoded = decode_message(encoded).unwrap();
        assert_eq!(decoded.flags, flags::ALWAYS_SET | flags::DATA);
        assert_eq!(decoded.msg_id, 1);
        let body = decoded.body.unwrap();
        let body = body.as_dict().unwrap();
        assert_eq!(body["MessageType"], XpcValue::String("Handshake".into()));
        assert_eq!(body["MessagingProtocolVersion"], XpcValue::Uint64(7));
        assert_eq!(body["UUID"], XpcValue::Uuid(*identity.as_bytes()));
        let properties = body["Properties"].as_dict().unwrap();
        assert_eq!(
            properties["RemoteXPCVersionFlags"],
            XpcValue::Uint64(0x0100_0000_0000_0006)
        );
        assert_eq!(
            properties["SensitivePropertiesVisible"],
            XpcValue::Bool(true)
        );
        assert!(body["Services"].as_dict().unwrap().is_empty());
    }

    #[tokio::test]
    async fn modern_rsd_sends_bootstrap_before_peer_settings_and_announces_host() {
        let (client, mut server) = tokio::io::duplex(4096);
        let identity = uuid::Uuid::from_bytes([0x4a; 16]);
        let task = tokio::spawn(async move {
            read_test_preface(&mut server).await;
            // This peer deliberately withholds SETTINGS until stream bootstrap.
            read_test_bootstrap(&mut server).await;
            server.write_all(&settings_frame()).await.unwrap();
            let ack = read_test_frame(&mut server).await;
            assert_eq!((ack.0, ack.1), (FRAME_SETTINGS, FLAG_SETTINGS_ACK));
            let (_, _, stream, payload) = read_test_frame(&mut server).await;
            assert_eq!(stream, 1);
            let message = decode_message(Bytes::from(payload)).unwrap();
            assert_eq!(message.body, device_handshake_message(identity).body);
            server
                .write_all(&data_frame(1, &sample_handshake_message()))
                .await
                .unwrap();
        });
        let mut stream = Some(client);
        let result = timeout(
            Duration::from_secs(2),
            handshake_attempts(
                || {
                    std::future::ready(Ok(stream
                        .take()
                        .expect("modern handshake should connect once")))
                },
                identity,
            ),
        )
        .await
        .unwrap()
        .unwrap();
        assert_eq!(
            result.get_port("com.apple.instruments.dtservicehub"),
            Some(12345)
        );
        task.await.unwrap();
    }

    #[tokio::test]
    async fn rejected_modern_rsd_reconnects_with_queued_legacy_bootstrap() {
        let (rejected, rejected_peer) = tokio::io::duplex(4096);
        drop(rejected_peer);
        let (legacy, mut server) = tokio::io::duplex(4096);
        let task = tokio::spawn(async move {
            read_test_preface(&mut server).await;
            server.write_all(&settings_frame()).await.unwrap();
            assert_eq!(read_test_frame(&mut server).await.1, FLAG_SETTINGS_ACK);
            read_test_bootstrap(&mut server).await;
            server
                .write_all(&data_frame(1, &sample_handshake_message()))
                .await
                .unwrap();
        });
        let mut streams = VecDeque::from([rejected, legacy]);
        let result = timeout(
            Duration::from_secs(2),
            handshake_attempts(
                || std::future::ready(Ok(streams.pop_front().unwrap())),
                uuid::Uuid::from_bytes([0x4a; 16]),
            ),
        )
        .await
        .unwrap()
        .unwrap();
        assert!(!result.services.is_empty());
        assert!(streams.is_empty());
        task.await.unwrap();
    }

    #[tokio::test]
    async fn rejected_bootstraps_eventually_reconnect_with_passive_rsd() {
        let mut streams = VecDeque::new();
        for _ in 0..3 {
            let (client, server) = tokio::io::duplex(4096);
            drop(server);
            streams.push_back(client);
        }
        let (client, mut server) = tokio::io::duplex(4096);
        streams.push_back(client);
        let task = tokio::spawn(async move {
            read_test_preface(&mut server).await;
            server.write_all(&settings_frame()).await.unwrap();
            assert_eq!(read_test_frame(&mut server).await.1, FLAG_SETTINGS_ACK);
            server
                .write_all(&data_frame(1, &sample_handshake_message()))
                .await
                .unwrap();
        });
        timeout(
            Duration::from_secs(2),
            handshake_attempts(
                || std::future::ready(Ok(streams.pop_front().unwrap())),
                uuid::Uuid::from_bytes([0x4a; 16]),
            ),
        )
        .await
        .unwrap()
        .unwrap();
        assert!(streams.is_empty());
        task.await.unwrap();
    }

    #[tokio::test]
    async fn rsd_selection_receipt_reports_each_actual_successful_bootstrap_and_fresh_fallback() {
        use crate::device::RsdHandshakeMode;
        let modes = [
            RsdHandshakeMode::Direct,
            RsdHandshakeMode::Queued,
            RsdHandshakeMode::Legacy,
            RsdHandshakeMode::Passive,
        ];
        for (index, expected) in modes.into_iter().enumerate() {
            let mut streams = VecDeque::new();
            for _ in 0..index {
                let (client, peer) = tokio::io::duplex(4096);
                drop(peer);
                streams.push_back(client);
            }
            let (client, mut peer) = tokio::io::duplex(16_384);
            streams.push_back(client);
            let server = async {
                read_test_preface(&mut peer).await;
                if index == 0 {
                    read_test_bootstrap(&mut peer).await;
                }
                peer.write_all(&settings_frame()).await.unwrap();
                let ack = read_test_frame(&mut peer).await;
                assert_eq!((ack.0, ack.1), (FRAME_SETTINGS, FLAG_SETTINGS_ACK));
                if index == 0 {
                    let (_, _, stream, payload) = read_test_frame(&mut peer).await;
                    assert_eq!(stream, 1);
                    assert_eq!(
                        decode_message(Bytes::from(payload)).unwrap().body,
                        device_handshake_message(uuid::Uuid::from_bytes([0x4a; 16])).body
                    );
                } else if index == 1 {
                    read_test_bootstrap(&mut peer).await;
                } else if index == 2 {
                    let empty = encode_message(&XpcMessage {
                        flags: flags::ALWAYS_SET,
                        msg_id: 0,
                        body: None,
                    })
                    .unwrap();
                    for stream in [1, 1, 3] {
                        if stream == 1 {
                            let header = read_test_frame(&mut peer).await;
                            assert_eq!(header.0, FRAME_HEADERS);
                        }
                        let data = read_test_frame(&mut peer).await;
                        assert_eq!((data.0, data.2), (FRAME_DATA, stream));
                        peer.write_all(&data_frame(stream, &empty)).await.unwrap();
                    }
                }
                peer.write_all(&data_frame(1, &sample_handshake_message()))
                    .await
                    .unwrap();
                let mut tail = Vec::new();
                peer.read_to_end(&mut tail).await.unwrap();
                // Only transport flow-control frames may follow the observation.
                let mut bytes = tail.as_slice();
                while !bytes.is_empty() {
                    assert!(bytes.len() >= 9);
                    let n = ((bytes[0] as usize) << 16)
                        | ((bytes[1] as usize) << 8)
                        | bytes[2] as usize;
                    assert_eq!(bytes[3], 8);
                    assert_eq!(n, 4);
                    assert!(bytes.len() >= 9 + n);
                    bytes = &bytes[9 + n..];
                }
            };
            let (result, ()) = timeout(Duration::from_secs(3), async {
                tokio::join!(
                    checked_handshake_with_connector_observed(
                        || std::future::ready(Ok(streams
                            .pop_front()
                            .expect("unexpected bootstrap retry"))),
                        uuid::Uuid::from_bytes([0x4a; 16]),
                        tokio::time::Instant::now() + Duration::from_secs(3),
                    ),
                    server
                )
            })
            .await
            .unwrap();
            let (directory, receipt) = result.unwrap();
            assert_eq!(receipt.handshake_mode, expected);
            assert_eq!(receipt.attempted_modes, modes[..=index]);
            assert!(directory.complete_service_directory());
            assert_eq!(directory.udid(), Some("00008150-00013DD00104401C"));
            assert!(streams.is_empty());
        }
    }

    #[tokio::test]
    async fn rsd_stage_timeout_reports_timeout_without_peer_contents() {
        let error = rsd_timeout(
            Duration::from_millis(1),
            "RSD response timed out",
            std::future::pending::<Result<(), XpcError>>(),
        )
        .await
        .unwrap_err();
        assert!(
            matches!(error, XpcError::Io(ref error) if error.kind() == std::io::ErrorKind::TimedOut)
        );
        assert!(error.to_string().contains("RSD response timed out"));
        assert_eq!(crate::classify_error(&error), crate::ErrorClass::Timeout);
        let XpcError::Io(outer) = error else {
            unreachable!()
        };
        assert!(std::error::Error::source(&outer)
            .unwrap()
            .is::<tokio::time::error::Elapsed>());
    }

    struct CapturingSubscriber {
        events: Arc<Mutex<Vec<String>>>,
    }

    impl tracing::Subscriber for CapturingSubscriber {
        fn enabled(&self, _metadata: &tracing::Metadata<'_>) -> bool {
            true
        }

        fn new_span(&self, _span: &tracing::span::Attributes<'_>) -> tracing::span::Id {
            tracing::span::Id::from_u64(1)
        }

        fn record(&self, _span: &tracing::span::Id, _values: &tracing::span::Record<'_>) {}

        fn event(&self, event: &tracing::Event<'_>) {
            let mut visitor = EventVisitor::default();
            event.record(&mut visitor);
            self.events.lock().unwrap().push(visitor.output);
        }

        fn enter(&self, _span: &tracing::span::Id) {}

        fn exit(&self, _span: &tracing::span::Id) {}

        fn record_follows_from(&self, _span: &tracing::span::Id, _follows: &tracing::span::Id) {}
    }

    #[derive(Default)]
    struct EventVisitor {
        output: String,
    }

    impl tracing::field::Visit for EventVisitor {
        fn record_debug(&mut self, field: &tracing::field::Field, value: &dyn fmt::Debug) {
            use fmt::Write;

            let _ = write!(self.output, "{}={value:?};", field.name());
        }
    }

    fn handshake_body_mut(message: &mut XpcMessage) -> &mut IndexMap<String, XpcValue> {
        match message.body.as_mut() {
            Some(XpcValue::Dictionary(body)) => body,
            _ => panic!("synthetic RSD message should have a dictionary body"),
        }
    }

    fn event_log() -> (tracing::Dispatch, Arc<Mutex<Vec<String>>>) {
        let events = Arc::new(Mutex::new(Vec::new()));
        let subscriber = CapturingSubscriber {
            events: Arc::clone(&events),
        };
        (tracing::Dispatch::new(subscriber), events)
    }

    fn build_frame(frame_type: u8, flags: u8, stream_id: u32, payload: &[u8]) -> Vec<u8> {
        let len = payload.len();
        let mut out = Vec::with_capacity(9 + len);
        out.push(((len >> 16) & 0xFF) as u8);
        out.push(((len >> 8) & 0xFF) as u8);
        out.push((len & 0xFF) as u8);
        out.push(frame_type);
        out.push(flags);
        out.extend_from_slice(&(stream_id & 0x7FFF_FFFF).to_be_bytes());
        out.extend_from_slice(payload);
        out
    }

    fn settings_frame() -> Vec<u8> {
        build_frame(FRAME_SETTINGS, 0, STREAM_INIT, &[])
    }

    fn settings_ack_frame() -> Vec<u8> {
        build_frame(FRAME_SETTINGS, FLAG_SETTINGS_ACK, STREAM_INIT, &[])
    }

    fn headers_frame(stream_id: u32) -> Vec<u8> {
        build_frame(FRAME_HEADERS, FLAG_END_HEADERS, stream_id, &[])
    }

    fn data_frame(stream_id: u32, payload: &[u8]) -> Vec<u8> {
        build_frame(FRAME_DATA, 0, stream_id, payload)
    }

    fn pending_data_message(msg_id: u64, len: usize) -> XpcMessage {
        XpcMessage {
            flags: flags::ALWAYS_SET,
            msg_id,
            body: Some(XpcValue::Data(Bytes::from(vec![0xA5; len]))),
        }
    }

    async fn connected_framer_for_pending_test() -> H2Framer<tokio::io::DuplexStream> {
        let (client, mut server) = tokio::io::duplex(4096);
        let server_task = tokio::spawn(async move {
            let mut preface = [0u8; 24];
            server.read_exact(&mut preface).await.unwrap();
            let mut settings = [0u8; 21];
            server.read_exact(&mut settings).await.unwrap();
            let mut window_update = [0u8; 13];
            server.read_exact(&mut window_update).await.unwrap();
            server.write_all(&settings_frame()).await.unwrap();
            server.flush().await.unwrap();
            let mut ack = [0u8; 9];
            server.read_exact(&mut ack).await.unwrap();
        });

        let framer = H2Framer::connect(client).await.unwrap();
        server_task.await.unwrap();
        framer
    }

    struct ScriptedIo {
        input: Bytes,
        offset: usize,
        output: Vec<u8>,
    }

    impl AsyncRead for ScriptedIo {
        fn poll_read(
            mut self: Pin<&mut Self>,
            _cx: &mut Context<'_>,
            buf: &mut ReadBuf<'_>,
        ) -> Poll<std::io::Result<()>> {
            let available = self.input.len().saturating_sub(self.offset);
            if available == 0 {
                return Poll::Ready(Ok(()));
            }
            let count = available.min(buf.remaining());
            let end = self.offset + count;
            buf.put_slice(&self.input[self.offset..end]);
            self.offset = end;
            Poll::Ready(Ok(()))
        }
    }

    impl AsyncWrite for ScriptedIo {
        fn poll_write(
            mut self: Pin<&mut Self>,
            _cx: &mut Context<'_>,
            buf: &[u8],
        ) -> Poll<std::io::Result<usize>> {
            self.output.extend_from_slice(buf);
            Poll::Ready(Ok(buf.len()))
        }

        fn poll_flush(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
            Poll::Ready(Ok(()))
        }

        fn poll_shutdown(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
            Poll::Ready(Ok(()))
        }
    }

    fn scripted_file_transfer_wire(body_len: usize) -> Bytes {
        let mut xpc = vec![0xA5; 24 + body_len];
        xpc[4..8].copy_from_slice(&flags::FILE_TX_STREAM_RESPONSE.to_le_bytes());
        xpc[8..16].copy_from_slice(&(body_len as u64).to_le_bytes());

        let mut wire = settings_frame();
        wire.reserve(xpc.len() + (xpc.len() / MAX_FRAME_PAYLOAD + 1) * 9);
        for chunk in xpc.chunks(MAX_FRAME_PAYLOAD) {
            wire.extend_from_slice(&data_frame(STREAM_SERVER_CLIENT, chunk));
        }
        Bytes::from(wire)
    }

    #[tokio::test]
    async fn file_transfer_body_can_cross_h2_buffer_boundary_in_chunks() {
        for body_len in [
            MAX_BUFFERED_BYTES_PER_STREAM + 1,
            crate::xpc::message::XPC_DATA_STREAM_BODY_LIMIT,
        ] {
            let wire = scripted_file_transfer_wire(body_len);
            let mut framer = H2Framer::connect(ScriptedIo {
                input: wire,
                offset: 0,
                output: Vec::new(),
            })
            .await
            .unwrap();

            let (header, body) = read_raw_xpc_on_server_client(&mut framer).await.unwrap();
            let declared = u64::from_le_bytes(header[8..16].try_into().unwrap());
            let flags = u32::from_le_bytes(header[4..8].try_into().unwrap());
            let checked = checked_xpc_body_len(declared, xpc_body_limit_for_flags(flags)).unwrap();
            assert_eq!(checked, body_len);
            assert_eq!(body.len(), body_len);
            assert_eq!(body.first(), Some(&0xA5));
            assert_eq!(body.last(), Some(&0xA5));
        }
    }

    fn sample_handshake_xpc_message(message_type: Option<&str>) -> XpcMessage {
        let mut properties = IndexMap::new();
        properties.insert(
            "UniqueDeviceID".to_string(),
            XpcValue::String("00008150-00013DD00104401C".into()),
        );

        let mut service = IndexMap::new();
        service.insert("Port".to_string(), XpcValue::String("12345".into()));
        service.insert(
            "Properties".to_string(),
            XpcValue::Dictionary(IndexMap::from([(
                "Features".to_string(),
                XpcValue::Array(vec![
                    XpcValue::String("com.apple.coredevice.feature.one".into()),
                    XpcValue::String("com.apple.coredevice.feature.two".into()),
                ]),
            )])),
        );

        let mut services = IndexMap::new();
        services.insert(
            "com.apple.instruments.dtservicehub".to_string(),
            XpcValue::Dictionary(service),
        );

        let mut body = IndexMap::new();
        if let Some(message_type) = message_type {
            body.insert(
                "MessageType".to_string(),
                XpcValue::String(message_type.into()),
            );
        }
        body.insert("Properties".to_string(), XpcValue::Dictionary(properties));
        body.insert("Services".to_string(), XpcValue::Dictionary(services));

        XpcMessage {
            flags: flags::ALWAYS_SET | flags::DATA,
            msg_id: 0,
            body: Some(XpcValue::Dictionary(body)),
        }
    }

    fn sample_handshake_message() -> Bytes {
        encode_message(&sample_handshake_xpc_message(Some("Handshake")))
            .expect("synthetic RSD message should encode")
    }

    #[test]
    fn parse_handshake_message_rejects_missing_or_wrong_message_type() {
        let missing = parse_handshake_message(sample_handshake_xpc_message(None));
        assert!(missing.is_err());

        let wrong = parse_handshake_message(sample_handshake_xpc_message(Some("NotHandshake")));
        assert!(wrong.is_err());
    }

    #[test]
    fn parse_handshake_diagnostics_do_not_expose_device_identity_or_payload() {
        const MARKER: &str = "RSD-SYNTHETIC-SECRET-MARKER";

        let mut valid = sample_handshake_xpc_message(Some("Handshake"));
        let mut properties = handshake_body_mut(&mut valid)
            .get_mut("Properties")
            .and_then(|value| value.as_dict())
            .cloned()
            .expect("synthetic properties should be a dictionary");
        properties.insert("UniqueDeviceID".into(), XpcValue::String(MARKER.into()));
        handshake_body_mut(&mut valid)
            .insert("Properties".into(), XpcValue::Dictionary(properties));
        let services = match handshake_body_mut(&mut valid).get_mut("Services") {
            Some(XpcValue::Dictionary(services)) => services,
            _ => panic!("synthetic services should be a dictionary"),
        };
        services.insert(
            MARKER.into(),
            XpcValue::Dictionary(IndexMap::from([(
                "Port".into(),
                XpcValue::String("not-a-port".into()),
            )])),
        );

        let mut unexpected_services = sample_handshake_xpc_message(Some("Handshake"));
        handshake_body_mut(&mut unexpected_services)
            .insert("Services".into(), XpcValue::String(MARKER.into()));

        let wrong_type = sample_handshake_xpc_message(Some(MARKER));
        let (dispatch, events) = event_log();
        let result = tracing::dispatcher::with_default(&dispatch, || {
            let valid_handshake = parse_handshake_message(valid).unwrap();
            let unexpected = parse_handshake_message(unexpected_services).unwrap();
            let wrong_error = parse_handshake_message(wrong_type).unwrap_err();
            (valid_handshake, unexpected, wrong_error)
        });

        assert_eq!(result.0.udid, MARKER);
        assert_eq!(result.0.services.len(), 1);
        assert!(result.1.services.is_empty());
        assert!(!result.2.to_string().contains(MARKER));

        let output = events.lock().unwrap().join("\n");
        assert!(
            !output.contains(MARKER),
            "sensitive marker was logged: {output}"
        );
        assert!(output.contains("exposed 2 service entries"), "{output}");
        assert!(output.contains("unexpected XPC type: string"), "{output}");
        assert!(
            output.contains("RSD service has an invalid or missing Port"),
            "{output}"
        );
    }

    #[tokio::test]
    async fn read_handshake_diagnostics_do_not_propagate_untrusted_message_type() {
        const MARKER: &str = "RSD-SYNTHETIC-MESSAGE-TYPE-MARKER";

        let wrong = encode_message(&sample_handshake_xpc_message(Some(MARKER)))
            .expect("synthetic wrong-type message should encode");
        let valid = sample_handshake_message();
        let mut wire = settings_frame();
        wire.extend_from_slice(&data_frame(STREAM_CLIENT_SERVER, &wrong));
        wire.extend_from_slice(&data_frame(STREAM_CLIENT_SERVER, &valid));

        let mut framer = H2Framer::connect(ScriptedIo {
            input: Bytes::from(wire),
            offset: 0,
            output: Vec::new(),
        })
        .await
        .unwrap();

        let (dispatch, events) = event_log();
        let guard = tracing::dispatcher::set_default(&dispatch);
        let handshake = read_rsd_handshake(&mut framer).await.unwrap();
        drop(guard);

        assert_eq!(handshake.udid, "00008150-00013DD00104401C");
        let output = events.lock().unwrap().join("\n");
        assert!(
            !output.contains(MARKER),
            "sensitive marker was logged: {output}"
        );
        assert!(
            output.contains("skipping non-handshake stream-1 message"),
            "{output}"
        );
    }

    #[test]
    fn parse_handshake_message_accepts_valid_handshake() {
        let handshake =
            parse_handshake_message(sample_handshake_xpc_message(Some("Handshake"))).unwrap();

        assert_eq!(handshake.udid, "00008150-00013DD00104401C");
        assert_eq!(
            handshake.get_port("com.apple.instruments.dtservicehub"),
            Some(12345)
        );
        assert_eq!(
            handshake
                .get_service_features("com.apple.instruments.dtservicehub")
                .unwrap(),
            [
                "com.apple.coredevice.feature.one",
                "com.apple.coredevice.feature.two"
            ]
        );
        assert_eq!(
            handshake.supports_feature(
                "com.apple.instruments.dtservicehub",
                "com.apple.coredevice.feature.two"
            ),
            Some(true)
        );
        assert_eq!(
            handshake.supports_feature(
                "com.apple.instruments.dtservicehub",
                "com.apple.coredevice.feature.missing"
            ),
            Some(false)
        );
    }

    #[test]
    fn service_descriptor_without_feature_metadata_is_permissive() {
        let descriptor = ServiceDescriptor::new(12345);
        assert!(descriptor.supports_feature("com.apple.coredevice.feature.future"));
    }

    #[test]
    fn resolved_service_features_follow_shim_fallback() {
        let mut descriptor = ServiceDescriptor::new(12345);
        descriptor.features = vec!["com.apple.coredevice.feature.streamapplist".into()];
        let handshake = RsdHandshake {
            udid: "test-udid".into(),
            services: HashMap::from([(
                "com.apple.coredevice.appservice.shim.remote".into(),
                descriptor,
            )]),
        };

        assert_eq!(
            handshake.get_resolved_service_features("com.apple.coredevice.appservice"),
            Some(["com.apple.coredevice.feature.streamapplist".to_string()].as_slice())
        );
    }

    #[test]
    fn resolved_service_features_prefer_canonical_entry() {
        let mut canonical = ServiceDescriptor::new(1111);
        canonical.features = vec!["canonical-feature".into()];
        let mut shim = ServiceDescriptor::new(2222);
        shim.features = vec!["shim-feature".into()];
        let handshake = RsdHandshake {
            udid: "test-udid".into(),
            services: HashMap::from([
                ("com.apple.coredevice.appservice".into(), canonical),
                ("com.apple.coredevice.appservice.shim.remote".into(), shim),
            ]),
        };

        assert_eq!(
            handshake.get_port("com.apple.coredevice.appservice"),
            Some(1111)
        );
        assert_eq!(
            handshake.get_resolved_service_features("com.apple.coredevice.appservice"),
            Some(["canonical-feature".to_string()].as_slice())
        );
    }

    #[test]
    fn parse_handshake_message_rejects_out_of_range_service_port() {
        let mut message = sample_handshake_xpc_message(Some("Handshake"));
        let services = message
            .body
            .as_mut()
            .and_then(|body| match body {
                XpcValue::Dictionary(body) => body.get_mut("Services"),
                _ => None,
            })
            .and_then(|services| match services {
                XpcValue::Dictionary(services) => Some(services),
                _ => None,
            })
            .unwrap();
        let service = services
            .get_mut("com.apple.instruments.dtservicehub")
            .and_then(|service| match service {
                XpcValue::Dictionary(service) => Some(service),
                _ => None,
            })
            .unwrap();
        service.insert("Port".into(), XpcValue::Uint64(u16::MAX as u64 + 1));

        let handshake = parse_handshake_message(message).unwrap();
        assert!(handshake.services.is_empty());
    }

    #[tokio::test]
    async fn handshake_on_framer_reads_stream_1_without_xpc_init() {
        let (client, mut server) = tokio::io::duplex(4096);

        let server_task = tokio::spawn(async move {
            let mut preface = [0u8; 24];
            server.read_exact(&mut preface).await.unwrap();
            assert_eq!(&preface, crate::xpc::h2_raw::H2_PREFACE);

            let mut settings = [0u8; 21];
            server.read_exact(&mut settings).await.unwrap();
            assert_eq!(settings[3], FRAME_SETTINGS);

            let mut window_update = [0u8; 13];
            server.read_exact(&mut window_update).await.unwrap();
            assert_eq!(window_update[3], 0x08);

            server.write_all(&settings_frame()).await.unwrap();
            server.flush().await.unwrap();

            let mut ack = [0u8; 9];
            server.read_exact(&mut ack).await.unwrap();
            assert_eq!(ack, settings_ack_frame().as_slice());

            // The RSD port should not receive the usual XPC init traffic.
            assert!(timeout(Duration::from_millis(100), async {
                let mut extra = [0u8; 1];
                server.read_exact(&mut extra).await
            })
            .await
            .is_err());

            server
                .write_all(&headers_frame(STREAM_CLIENT_SERVER))
                .await
                .unwrap();
            server
                .write_all(&headers_frame(STREAM_SERVER_CLIENT))
                .await
                .unwrap();
            server
                .write_all(&data_frame(
                    STREAM_CLIENT_SERVER,
                    &sample_handshake_message(),
                ))
                .await
                .unwrap();
            server.flush().await.unwrap();
        });

        let mut framer = H2Framer::connect(client).await.unwrap();
        let handshake = timeout(Duration::from_secs(1), handshake_on_framer(&mut framer))
            .await
            .expect("handshake timed out")
            .unwrap();

        assert_eq!(handshake.udid, "00008150-00013DD00104401C");
        assert_eq!(
            handshake.get_port("com.apple.instruments.dtservicehub"),
            Some(12345)
        );

        server_task.await.unwrap();
    }

    #[tokio::test]
    async fn initialize_xpc_connection_consumes_step_responses_in_reference_order() {
        let (client, mut server) = tokio::io::duplex(4096);

        let empty = encode_message(&XpcMessage {
            flags: flags::ALWAYS_SET,
            msg_id: 0,
            body: None,
        })
        .unwrap();

        let server_task = tokio::spawn(async move {
            let mut preface = [0u8; 24];
            server.read_exact(&mut preface).await.unwrap();
            assert_eq!(&preface, crate::xpc::h2_raw::H2_PREFACE);

            let mut settings = [0u8; 21];
            server.read_exact(&mut settings).await.unwrap();
            assert_eq!(settings[3], FRAME_SETTINGS);

            let mut window_update = [0u8; 13];
            server.read_exact(&mut window_update).await.unwrap();
            assert_eq!(window_update[3], 0x08);

            server.write_all(&settings_frame()).await.unwrap();
            server.flush().await.unwrap();

            let mut ack = [0u8; 9];
            server.read_exact(&mut ack).await.unwrap();
            assert_eq!(ack, settings_ack_frame().as_slice());

            let mut cs_headers = [0u8; 9];
            server.read_exact(&mut cs_headers).await.unwrap();
            assert_eq!(cs_headers, headers_frame(STREAM_CLIENT_SERVER).as_slice());

            let mut cs_msg1_header = [0u8; 9];
            server.read_exact(&mut cs_msg1_header).await.unwrap();
            assert_eq!(cs_msg1_header[3], FRAME_DATA);
            assert_eq!(
                u32::from_be_bytes([
                    cs_msg1_header[5] & 0x7F,
                    cs_msg1_header[6],
                    cs_msg1_header[7],
                    cs_msg1_header[8]
                ]),
                STREAM_CLIENT_SERVER
            );
            let msg1_len = ((cs_msg1_header[0] as usize) << 16)
                | ((cs_msg1_header[1] as usize) << 8)
                | (cs_msg1_header[2] as usize);
            let mut cs_msg1 = vec![0u8; msg1_len];
            server.read_exact(&mut cs_msg1).await.unwrap();

            server
                .write_all(&data_frame(STREAM_CLIENT_SERVER, &empty))
                .await
                .unwrap();
            server.flush().await.unwrap();

            let mut sc_headers = [0u8; 9];
            server.read_exact(&mut sc_headers).await.unwrap();
            assert_eq!(sc_headers, headers_frame(STREAM_SERVER_CLIENT).as_slice());

            let mut cs_msg3_header = [0u8; 9];
            server.read_exact(&mut cs_msg3_header).await.unwrap();
            assert_eq!(cs_msg3_header[3], FRAME_DATA);
            assert_eq!(
                u32::from_be_bytes([
                    cs_msg3_header[5] & 0x7F,
                    cs_msg3_header[6],
                    cs_msg3_header[7],
                    cs_msg3_header[8]
                ]),
                STREAM_CLIENT_SERVER
            );
            let msg3_len = ((cs_msg3_header[0] as usize) << 16)
                | ((cs_msg3_header[1] as usize) << 8)
                | (cs_msg3_header[2] as usize);
            let mut cs_msg3 = vec![0u8; msg3_len];
            server.read_exact(&mut cs_msg3).await.unwrap();

            server
                .write_all(&data_frame(STREAM_CLIENT_SERVER, &empty))
                .await
                .unwrap();
            server.flush().await.unwrap();

            let mut sc_msg2_header = [0u8; 9];
            server.read_exact(&mut sc_msg2_header).await.unwrap();
            assert_eq!(sc_msg2_header[3], FRAME_DATA);
            assert_eq!(
                u32::from_be_bytes([
                    sc_msg2_header[5] & 0x7F,
                    sc_msg2_header[6],
                    sc_msg2_header[7],
                    sc_msg2_header[8]
                ]),
                STREAM_SERVER_CLIENT
            );
            let msg2_len = ((sc_msg2_header[0] as usize) << 16)
                | ((sc_msg2_header[1] as usize) << 8)
                | (sc_msg2_header[2] as usize);
            let mut sc_msg2 = vec![0u8; msg2_len];
            server.read_exact(&mut sc_msg2).await.unwrap();

            server
                .write_all(&data_frame(STREAM_SERVER_CLIENT, &empty))
                .await
                .unwrap();
            server.flush().await.unwrap();
        });

        let mut framer = H2Framer::connect(client).await.unwrap();
        timeout(
            Duration::from_secs(1),
            initialize_xpc_connection_on_framer(&mut framer),
        )
        .await
        .expect("bootstrap timed out")
        .unwrap();

        server_task.await.unwrap();
    }

    #[tokio::test]
    async fn recv_reply_on_stream_rejects_an_unbounded_pending_backlog() {
        let (client, mut server) = tokio::io::duplex(64 * 1024);

        let server_task = tokio::spawn(async move {
            let mut preface = [0u8; 24];
            server.read_exact(&mut preface).await.unwrap();

            let mut settings = [0u8; 21];
            server.read_exact(&mut settings).await.unwrap();

            let mut window_update = [0u8; 13];
            server.read_exact(&mut window_update).await.unwrap();

            server.write_all(&settings_frame()).await.unwrap();
            server.flush().await.unwrap();

            let mut ack = [0u8; 9];
            server.read_exact(&mut ack).await.unwrap();

            server
                .write_all(&headers_frame(STREAM_SERVER_CLIENT))
                .await
                .unwrap();

            // Every reply carries an id the caller is not waiting for, so each one
            // is parked instead of returned.
            for id in 0..=MAX_PENDING_MESSAGES_PER_STREAM as u64 {
                let stray = encode_message(&XpcMessage {
                    flags: flags::ALWAYS_SET,
                    msg_id: 7 + id,
                    body: None,
                })
                .unwrap();
                server
                    .write_all(&data_frame(STREAM_SERVER_CLIENT, &stray))
                    .await
                    .unwrap();
            }
            server.flush().await.unwrap();
        });

        let framer = H2Framer::connect(client).await.unwrap();
        let mut connection = XpcConnection::new(framer);
        let err = timeout(
            Duration::from_secs(5),
            connection.recv_reply_on_stream(STREAM_SERVER_CLIENT, u64::MAX),
        )
        .await
        .expect("the cap should trip instead of blocking forever")
        .unwrap_err();

        assert!(err.to_string().contains("unmatched messages"), "{err}");
        assert_eq!(connection.pending_memory_bytes_used(), 0);

        server_task.await.unwrap();
    }

    #[test]
    fn pending_budget_rejects_overflow_without_mutating_exact_usage() {
        let message = pending_data_message(1, 8);
        let bytes = xpc_message_memory_size(&message).unwrap();
        let mut budget = PendingMessageBudget::new(bytes * 2, bytes * 2);

        budget.reserve(3, bytes).unwrap();
        budget.reserve(5, bytes).unwrap();
        assert_eq!(budget.total_bytes, bytes * 2);
        assert!(budget.reserve(3, 1).is_err());
        assert_eq!(budget.total_bytes, bytes * 2);

        budget.release(3, bytes);
        assert_eq!(budget.total_bytes, bytes);
        budget.release(5, bytes);
        assert_eq!(budget.total_bytes, 0);
    }

    #[tokio::test]
    async fn pending_messages_account_for_replacement_pop_and_stream_close() {
        let framer = connected_framer_for_pending_test().await;
        let first = pending_data_message(7, 8);
        let first_bytes = xpc_message_memory_size(&first).unwrap();
        let replacement = pending_data_message(7, 24);
        let replacement_bytes = xpc_message_memory_size(&replacement).unwrap();
        let mut connection = XpcConnection::with_pending_memory_limits(
            framer,
            replacement_bytes,
            replacement_bytes * 2,
        );

        connection.push_pending_message(3, first).unwrap();
        assert_eq!(connection.pending_memory_bytes_used(), first_bytes);
        connection
            .push_pending_message(3, replacement)
            .expect("same nonzero reply id should replace its parked value");
        assert_eq!(connection.pending_memory_bytes_used(), replacement_bytes);

        assert!(connection
            .push_pending_message(3, pending_data_message(8, 1))
            .is_err());
        assert_eq!(
            connection.pending_memory_bytes_used(),
            replacement_bytes,
            "rejected insert must not consume budget"
        );

        let popped = connection.pop_next_pending_message(3).unwrap();
        assert_eq!(popped.msg_id, 7);
        assert_eq!(connection.pending_memory_bytes_used(), 0);

        let stream_three = pending_data_message(9, 8);
        let stream_three_bytes = xpc_message_memory_size(&stream_three).unwrap();
        let stream_five = pending_data_message(10, 12);
        let stream_five_bytes = xpc_message_memory_size(&stream_five).unwrap();
        connection.push_pending_message(3, stream_three).unwrap();
        connection.push_pending_message(5, stream_five).unwrap();
        assert_eq!(
            connection.pending_memory_bytes_used(),
            stream_three_bytes + stream_five_bytes
        );

        connection.clear_pending_stream(3);
        assert_eq!(connection.pending_memory_bytes_used(), stream_five_bytes);
        connection.clear_pending_stream(5);
        assert_eq!(connection.pending_memory_bytes_used(), 0);
    }

    #[tokio::test]
    async fn queue_rsd_handshake_bootstrap_matches_pymobiledevice3_order() {
        let (client, mut server) = tokio::io::duplex(4096);

        let server_task = tokio::spawn(async move {
            let mut preface = [0u8; 24];
            server.read_exact(&mut preface).await.unwrap();
            assert_eq!(&preface, crate::xpc::h2_raw::H2_PREFACE);

            let mut settings = [0u8; 21];
            server.read_exact(&mut settings).await.unwrap();
            assert_eq!(settings[3], FRAME_SETTINGS);

            let mut window_update = [0u8; 13];
            server.read_exact(&mut window_update).await.unwrap();
            assert_eq!(window_update[3], 0x08);

            server.write_all(&settings_frame()).await.unwrap();
            server.flush().await.unwrap();

            let mut ack = [0u8; 9];
            server.read_exact(&mut ack).await.unwrap();
            assert_eq!(ack, settings_ack_frame().as_slice());

            let mut cs_headers = [0u8; 9];
            server.read_exact(&mut cs_headers).await.unwrap();
            assert_eq!(cs_headers, headers_frame(STREAM_CLIENT_SERVER).as_slice());

            let mut cs_msg1_header = [0u8; 9];
            server.read_exact(&mut cs_msg1_header).await.unwrap();
            assert_eq!(cs_msg1_header[3], FRAME_DATA);
            let cs_msg1_len = ((cs_msg1_header[0] as usize) << 16)
                | ((cs_msg1_header[1] as usize) << 8)
                | (cs_msg1_header[2] as usize);
            let mut cs_msg1 = vec![0u8; cs_msg1_len];
            server.read_exact(&mut cs_msg1).await.unwrap();
            let decoded1 = decode_message(Bytes::from(cs_msg1)).unwrap();
            assert_eq!(decoded1.flags, flags::ALWAYS_SET);
            assert_eq!(
                decoded1.body,
                Some(XpcValue::Dictionary(IndexMap::<String, XpcValue>::new()))
            );

            let mut sc_headers = [0u8; 9];
            server.read_exact(&mut sc_headers).await.unwrap();
            assert_eq!(sc_headers, headers_frame(STREAM_SERVER_CLIENT).as_slice());

            let mut cs_msg2_header = [0u8; 9];
            server.read_exact(&mut cs_msg2_header).await.unwrap();
            assert_eq!(cs_msg2_header[3], FRAME_DATA);
            let cs_msg2_len = ((cs_msg2_header[0] as usize) << 16)
                | ((cs_msg2_header[1] as usize) << 8)
                | (cs_msg2_header[2] as usize);
            let mut cs_msg2 = vec![0u8; cs_msg2_len];
            server.read_exact(&mut cs_msg2).await.unwrap();
            let decoded2 = decode_message(Bytes::from(cs_msg2)).unwrap();
            assert_eq!(decoded2.flags, flags::ALWAYS_SET | 0x200);
            assert!(decoded2.body.is_none());

            let mut sc_msg3_header = [0u8; 9];
            server.read_exact(&mut sc_msg3_header).await.unwrap();
            assert_eq!(sc_msg3_header[3], FRAME_DATA);
            let sc_msg3_len = ((sc_msg3_header[0] as usize) << 16)
                | ((sc_msg3_header[1] as usize) << 8)
                | (sc_msg3_header[2] as usize);
            let mut sc_msg3 = vec![0u8; sc_msg3_len];
            server.read_exact(&mut sc_msg3).await.unwrap();
            let decoded3 = decode_message(Bytes::from(sc_msg3)).unwrap();
            assert_eq!(decoded3.flags, flags::INIT_HANDSHAKE | flags::ALWAYS_SET);
            assert!(decoded3.body.is_none());
        });

        let mut framer = H2Framer::connect(client).await.unwrap();
        timeout(
            Duration::from_secs(1),
            queue_rsd_handshake_bootstrap_on_framer(&mut framer),
        )
        .await
        .expect("queued bootstrap timed out")
        .unwrap();

        server_task.await.unwrap();
    }
}

#[cfg(feature = "tunnel")]
pub(crate) fn checked_borrowed_parts<'t, 'a>(
    dictionary: crate::xpc::message::borrowed::Dictionary<'t, 'a>,
) -> Result<
    (
        Option<&'a str>,
        Option<crate::xpc::message::borrowed::Dictionary<'t, 'a>>,
    ),
    XpcError,
> {
    use crate::device::existing_rsd::{
        ExistingRsdProbeError, MAX_FEATURES, MAX_FIELD_BYTES, MAX_SERVICES, MAX_TEXT_BYTES,
    };
    let protocol = |reason: &'static str| XpcError::Tls(reason.into());
    let mut text_bytes = 0usize;
    let mut feature_count = 0usize;
    let mut admit_text = |text: &str| -> Result<(), XpcError> {
        text_bytes = text_bytes.saturating_add(text.len());
        if text.len() > MAX_FIELD_BYTES || text_bytes > MAX_TEXT_BYTES {
            return Err(ExistingRsdProbeError::metadata_limit());
        }
        Ok(())
    };
    let properties = dictionary
        .get("Properties")
        .map(|value| {
            value
                .as_dict()
                .ok_or_else(|| protocol("RSD: Properties was not a dictionary"))
        })
        .transpose()?;
    let udid = properties
        .and_then(|properties| properties.get("UniqueDeviceID"))
        .map(|value| {
            let text = value
                .as_str()
                .filter(|text| !text.is_empty())
                .ok_or_else(|| protocol("RSD: UniqueDeviceID was not a nonempty string"))?;
            admit_text(text)?;
            Ok::<_, XpcError>(text)
        })
        .transpose()?;
    let services = dictionary
        .get("Services")
        .map(|value| {
            let services = value
                .as_dict()
                .ok_or_else(|| protocol("RSD: Services was not a dictionary"))?;
            if services.len() > MAX_SERVICES {
                return Err(ExistingRsdProbeError::metadata_limit());
            }
            Ok::<_, XpcError>(services)
        })
        .transpose()?;
    if let Some(services) = services {
        for (name, value) in services.iter() {
            admit_text(name)?;
            let descriptor = value
                .as_dict()
                .ok_or_else(|| protocol("RSD: service entry was not a dictionary"))?;
            if let Some(port) = descriptor.get("Port") {
                if port
                    .as_str()
                    .is_some_and(|text| text.len() > MAX_FIELD_BYTES)
                {
                    return Err(ExistingRsdProbeError::metadata_limit());
                }
                checked_borrowed_port(port)?;
            }
            let properties = descriptor
                .get("Properties")
                .map(|value| {
                    value
                        .as_dict()
                        .ok_or_else(|| protocol("RSD: service Properties was not a dictionary"))
                })
                .transpose()?;
            if let Some(value) = properties.and_then(|properties| properties.get("Features")) {
                let features = value
                    .as_array()
                    .ok_or_else(|| protocol("RSD: Features was not an array"))?;
                feature_count = feature_count.saturating_add(features.len());
                if feature_count > MAX_FEATURES {
                    return Err(ExistingRsdProbeError::metadata_limit());
                }
                for feature in features.iter() {
                    admit_text(
                        feature
                            .as_str()
                            .ok_or_else(|| protocol("RSD: feature identifier was not a string"))?,
                    )?;
                }
            }
        }
    }
    Ok((udid, services))
}

#[cfg(feature = "tunnel")]
pub(crate) fn checked_borrowed_port(
    value: crate::xpc::message::borrowed::Value<'_, '_>,
) -> Result<u16, XpcError> {
    let port = if let Some(text) = value.as_str() {
        text.parse::<u16>().map_err(|source| {
            XpcError::Io(crate::error::context::received_io(
                "decode RSD service Port",
                None,
                source,
            ))
        })?
    } else if let Some(port) = value.as_uint64() {
        u16::try_from(port).map_err(|source| {
            XpcError::Io(crate::error::context::received_io(
                "decode RSD service Port",
                Some(port),
                source,
            ))
        })?
    } else {
        return Err(XpcError::Tls("RSD: Port had an invalid type".into()));
    };
    if port == 0 {
        Err(XpcError::Tls("RSD: service Port was zero".into()))
    } else {
        Ok(port)
    }
}

#[cfg(feature = "tunnel")]
pub(crate) async fn checked_handshake_prepared<F, Fut, S>(
    connector: F,
    identity: uuid::Uuid,
    deadline: tokio::time::Instant,
    bank: &std::sync::Arc<tokio::sync::Semaphore>,
) -> Result<
    (
        crate::device::PreparedRsdCatalogue,
        crate::device::existing_rsd::RsdSelectionReceipt,
        Option<tokio::sync::OwnedSemaphorePermit>,
    ),
    XpcError,
>
where
    F: FnMut() -> Fut,
    Fut: std::future::Future<Output = Result<S, XpcError>>,
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin,
{
    checked_handshake_wire(connector, identity, deadline, Some(bank), |header, body| {
        let tree = crate::xpc::message::borrowed::decode(header, body, bank)
            .map_err(checked_wire_error)?;
        let Some(dictionary) = tree.as_ref().and_then(|tree| tree.root().as_dict()) else {
            return Ok(None);
        };
        match dictionary.get("MessageType") {
            None => Ok(None),
            Some(value) => match value.as_str() {
                Some("Handshake") => {
                    crate::device::PreparedRsdCatalogue::decode_borrowed(dictionary, bank)
                        .map(Some)
                        .map_err(CheckedReadError::Final)
                }
                Some(_) => Ok(None),
                None => Err(CheckedReadError::Final(XpcError::Tls(
                    "RSD: MessageType was not a string".into(),
                ))),
            },
        }
    })
    .await
}

#[cfg(feature = "tunnel")]
fn checked_wire_error(error: XpcError) -> CheckedReadError {
    if crate::classify_error(&error) == crate::ErrorClass::ResourceLimit {
        CheckedReadError::Final(error)
    } else {
        CheckedReadError::Retry(error)
    }
}

#[cfg(all(test, feature = "tunnel"))]
#[path = "rsd/raw_storage_tests.rs"]
mod raw_storage_tests;
