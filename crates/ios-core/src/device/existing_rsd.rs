//! Read-only observations through a caller-selected, already established route.
//!
//! This API opens its own sockets. It never creates, owns or stops a native
//! tunnel, reads pairing material, or starts a device service. Cancelling its
//! future drops the current socket; compatibility fallbacks use fresh sockets.

use std::{
    collections::HashMap,
    error::Error,
    fmt,
    future::Future,
    io,
    net::SocketAddrV6,
    pin::Pin,
    task::{Context, Poll},
    time::Duration,
};

use futures_util::future::BoxFuture;
use tokio::io::{AsyncRead, AsyncWrite, AsyncWriteExt, ReadBuf};
use tokio::time::Instant;

use crate::{CoreError, ErrorClass, ServiceStream, XpcError};

pub(crate) const MAX_FIELD_BYTES: usize = 4096;
pub(crate) const MAX_SERVICES: usize = 4096;
pub(crate) const MAX_FEATURES: usize = 16_384;
pub(crate) const MAX_TEXT_BYTES: usize = 1024 * 1024;

/// An explicit RSD destination reachable through an existing route.
///
/// The userspace proxy is always contacted on IPv4 loopback. Its prelude cannot
/// encode an IPv6 scope ID, so proxy routes reject a nonzero scope rather than
/// silently discard it. Direct routes preserve the complete SocketAddrV6.
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct ExistingRsdRoute {
    endpoint: SocketAddrV6,
    proxy_port: Option<u16>,
}

impl ExistingRsdRoute {
    pub fn direct(endpoint: SocketAddrV6) -> Result<Self, CoreError> {
        Self::new(endpoint, None)
    }

    pub fn via_userspace_proxy(endpoint: SocketAddrV6, proxy_port: u16) -> Result<Self, CoreError> {
        Self::new(endpoint, Some(proxy_port))
    }

    fn new(endpoint: SocketAddrV6, proxy_port: Option<u16>) -> Result<Self, CoreError> {
        if endpoint.port() == 0
            || proxy_port == Some(0)
            || (proxy_port.is_some() && (endpoint.scope_id() != 0 || endpoint.flowinfo() != 0))
        {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "existing RSD route requires nonzero ports and an unscoped proxy destination",
            )
            .into());
        }
        Ok(Self {
            endpoint,
            proxy_port,
        })
    }

    /// Explicit access to the selected address; callers must redact diagnostics.
    pub fn endpoint(&self) -> SocketAddrV6 {
        self.endpoint
    }

    pub fn proxy_port(&self) -> Option<u16> {
        self.proxy_port
    }
}

impl fmt::Debug for ExistingRsdRoute {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ExistingRsdRoute")
            .field("via_userspace_proxy", &self.proxy_port.is_some())
            .field("scope_id", &self.endpoint.scope_id())
            .finish_non_exhaustive()
    }
}

/// A logical admission decision, independent of native IO or parser errors.
/// Diagnostics contain only fixed owner context and the precise category.
pub struct ExistingRsdProbeError {
    phase: &'static str,
    class: ErrorClass,
}

impl ExistingRsdProbeError {
    pub fn class(&self) -> ErrorClass {
        self.class
    }

    pub(crate) fn metadata_limit() -> XpcError {
        XpcError::Io(io::Error::other(Self {
            phase: "existing RSD metadata admission limit",
            class: ErrorClass::ResourceLimit,
        }))
    }
}

impl fmt::Display for ExistingRsdProbeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} ({})", self.phase, self.class)
    }
}

impl fmt::Debug for ExistingRsdProbeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(self, f)
    }
}

impl Error for ExistingRsdProbeError {}

/// Evidence in a successfully decoded directory, before executing any service.
/// `NotAdvertised` is valid only when identity and every service port are known.
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum ExistingRsdServiceEvidence {
    Advertised,
    NotAdvertised,
    MetadataMissing,
}

/// Strictly observed service metadata. Missing fields remain absent.
pub struct ExistingRsdService {
    port: Option<u16>,
    features: Option<Vec<String>>,
}

impl ExistingRsdService {
    pub(crate) fn new(port: Option<u16>, features: Option<Vec<String>>) -> Self {
        Self { port, features }
    }

    pub fn port(&self) -> Option<u16> {
        self.port
    }

    /// `None` is omitted metadata; `Some(&[])` is an explicit empty list.
    /// Neither form is an explicit device-side rejection of a service call.
    pub fn features(&self) -> Option<&[String]> {
        self.features.as_deref()
    }
}

impl fmt::Debug for ExistingRsdService {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ExistingRsdService")
            .field("port", &self.port)
            .field("feature_count", &self.features.as_ref().map(Vec::len))
            .finish_non_exhaustive()
    }
}

/// Checked metadata from one fresh RSD handshake.
///
/// Callers must match `udid()` against their explicit target before attributing
/// evidence to that target. Missing identity/directory/ports do not establish
/// service absence. Presence in a directory does not certify service readiness,
/// device trust, mounted DDI, platform routing, or remoted coexistence.
pub struct ExistingRsdObservation {
    udid: Option<String>,
    services: Option<HashMap<String, ExistingRsdService>>,
}

/// Bootstrap branch that actually decoded the current RSD directory.
/// This is independent of the tunnel's userspace/kernel transport mode.
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "lowercase")]
pub enum RsdHandshakeMode {
    Direct,
    Queued,
    Legacy,
    Passive,
}

/// Safe protocol selection facts from one successful checked RSD operation.
#[derive(Clone, Debug, serde::Serialize)]
pub struct RsdSelectionReceipt {
    pub handshake_mode: RsdHandshakeMode,
    pub attempted_modes: Vec<RsdHandshakeMode>,
}

mod service;
pub use service::{
    ExistingRsdCleanupFailure, ExistingRsdCleanupReceipt, ExistingRsdServiceConnection,
    ExistingRsdServiceKind, ExistingRsdServiceOwner, ExistingRsdSession,
};

impl ExistingRsdObservation {
    pub(crate) fn new(
        udid: Option<String>,
        services: Option<HashMap<String, ExistingRsdService>>,
    ) -> Self {
        Self { udid, services }
    }

    /// Exact remote identity for an explicit caller; diagnostics must redact it.
    pub fn udid(&self) -> Option<&str> {
        self.udid.as_deref()
    }

    /// Exact directory for explicit callers; service and feature names can be
    /// sensitive and must not be included in automatic diagnostics.
    pub fn services(&self) -> Option<&HashMap<String, ExistingRsdService>> {
        self.services.as_ref()
    }

    pub fn complete_service_directory(&self) -> bool {
        self.udid.is_some()
            && self
                .services
                .as_ref()
                .is_some_and(|services| services.values().all(|service| service.port.is_some()))
    }

    /// Canonical service names take precedence over `.shim.remote`, including
    /// an incomplete canonical entry. A missing port cannot authorize fallback.
    pub fn service_evidence(&self, name: &str) -> ExistingRsdServiceEvidence {
        if self.udid.is_none() {
            return ExistingRsdServiceEvidence::MetadataMissing;
        }
        let Some(services) = &self.services else {
            return ExistingRsdServiceEvidence::MetadataMissing;
        };
        let entry = services
            .get(name)
            .or_else(|| services.get(&format!("{name}.shim.remote")));
        if let Some(entry) = entry {
            if entry.port.is_some() {
                ExistingRsdServiceEvidence::Advertised
            } else {
                ExistingRsdServiceEvidence::MetadataMissing
            }
        } else if self.complete_service_directory() {
            ExistingRsdServiceEvidence::NotAdvertised
        } else {
            ExistingRsdServiceEvidence::MetadataMissing
        }
    }
}

impl fmt::Debug for ExistingRsdObservation {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ExistingRsdObservation")
            .field("identity_present", &self.udid.is_some())
            .field("service_count", &self.services.as_ref().map(HashMap::len))
            .field(
                "complete_service_directory",
                &self.complete_service_directory(),
            )
            .finish_non_exhaustive()
    }
}

/// Observe an already established RSD route with one absolute budget.
///
/// The budget covers stable host identity resolution, dial, proxy prelude,
/// bootstrap and directory reads, including all fresh compatibility fallbacks.
/// Dropping the future closes its own current socket; it never stops the
/// pre-existing tunnel/proxy. Native failures acquired in the crossing poll
/// retain precedence over expiry. Zero budget performs no identity or dial IO.
/// The logical ceilings bound newly retained metadata, not raw XPC allocation,
/// allocator/RSS, physical OS cancellation, or remote resource retirement.
pub async fn observe_existing_rsd(
    route: ExistingRsdRoute,
    total_budget: Duration,
) -> Result<ExistingRsdObservation, CoreError> {
    observe_from_source(&NativeSource, route, total_budget).await
}

/// Observe the same existing route and retain its actual successful bootstrap.
pub async fn observe_existing_rsd_with_selection(
    route: ExistingRsdRoute,
    total_budget: Duration,
) -> Result<(ExistingRsdObservation, RsdSelectionReceipt), CoreError> {
    let clock = Clock::new(total_budget)?;
    observe_selected_from_source(&NativeSource, route, &clock).await
}

/// Prepare a current catalogue under the caller's original absolute deadline.
/// Only the new catalogue/selection own exact-layout admission. The legacy H2
/// decoder's raw scratch/tree and private native/error allocations stay outside
/// this known storage contract and must not be called a global manager heap cap.
pub async fn prepare_existing_rsd_at(
    route: ExistingRsdRoute,
    deadline: Instant,
    bank: &std::sync::Arc<tokio::sync::Semaphore>,
) -> Result<std::sync::Arc<super::PreparedRsdCatalogue>, CoreError> {
    prepare_selected_from_source(&NativeSource, route, deadline, bank).await
}

/// Prepare through an explicit host identity, bypassing host identity lookup.
/// The selected socket/checked protocol and byte-owned catalogue are unchanged.
pub async fn prepare_existing_rsd_with_identity_at(
    route: ExistingRsdRoute,
    deadline: Instant,
    bank: &std::sync::Arc<tokio::sync::Semaphore>,
    identity: uuid::Uuid,
) -> Result<std::sync::Arc<super::PreparedRsdCatalogue>, CoreError> {
    struct ExplicitIdentity(uuid::Uuid);
    impl Source for ExplicitIdentity {
        fn identity(&self) -> BoxFuture<'_, Result<uuid::Uuid, XpcError>> {
            Box::pin(async { Ok(self.0) })
        }
        fn dial(&self, route: ExistingRsdRoute) -> BoxFuture<'_, Result<ServiceStream, CoreError>> {
            NativeSource.dial(route)
        }
    }
    prepare_selected_from_source(&ExplicitIdentity(identity), route, deadline, bank).await
}

async fn prepare_selected_from_source(
    source: &dyn Source,
    route: ExistingRsdRoute,
    deadline: Instant,
    bank: &std::sync::Arc<tokio::sync::Semaphore>,
) -> Result<std::sync::Arc<super::PreparedRsdCatalogue>, CoreError> {
    let clock = Clock {
        deadline,
        budget: deadline.saturating_duration_since(Instant::now()),
    };
    clock
        .run(async {
            let identity = source.identity().await?;
            clock.check()?;
            let (catalogue, receipt, fee) = crate::xpc::rsd::checked_handshake_with_parser(
                || async {
                    clock.check().map_err(connection_error)?;
                    let stream = source.dial(route).await.map_err(connection_error)?;
                    clock.check().map_err(connection_error)?;
                    let mut stream = clock.guard(stream);
                    if route.proxy_port.is_some() {
                        stream.write_all(&route.endpoint.ip().octets()).await?;
                        stream
                            .write_all(&(route.endpoint.port() as u32).to_le_bytes())
                            .await?;
                        stream.flush().await?;
                    }
                    clock.check().map_err(connection_error)?;
                    Ok(stream)
                },
                identity,
                deadline,
                Some(bank),
                |dictionary| super::PreparedRsdCatalogue::decode(dictionary, bank),
            )
            .await?;
            catalogue.finish(receipt, fee.expect("prepared selection admission"))
        })
        .await
}

trait Source: Send + Sync {
    fn identity(&self) -> BoxFuture<'_, Result<uuid::Uuid, XpcError>>;
    fn dial(&self, route: ExistingRsdRoute) -> BoxFuture<'_, Result<ServiceStream, CoreError>>;
}

struct NativeSource;

impl Source for NativeSource {
    fn identity(&self) -> BoxFuture<'_, Result<uuid::Uuid, XpcError>> {
        Box::pin(crate::xpc::rsd::default_handshake_uuid())
    }

    fn dial(&self, route: ExistingRsdRoute) -> BoxFuture<'_, Result<ServiceStream, CoreError>> {
        Box::pin(async move {
            let stream = match route.proxy_port {
                Some(port) => tokio::net::TcpStream::connect(("127.0.0.1", port)).await?,
                None => tokio::net::TcpStream::connect(route.endpoint).await?,
            };
            Ok(Box::new(stream) as ServiceStream)
        })
    }
}

fn connection_error(error: CoreError) -> XpcError {
    match error {
        CoreError::Io(error) => XpcError::Io(error),
        CoreError::Xpc(error) => error,
        other => XpcError::Io(io::Error::other(other)),
    }
}

async fn observe_from_source(
    source: &dyn Source,
    route: ExistingRsdRoute,
    total_budget: Duration,
) -> Result<ExistingRsdObservation, CoreError> {
    let clock = Clock::new(total_budget)?;
    observe_selected_from_source(source, route, &clock)
        .await
        .map(|(observation, _)| observation)
}

async fn observe_selected_from_source(
    source: &dyn Source,
    route: ExistingRsdRoute,
    clock: &Clock,
) -> Result<(ExistingRsdObservation, RsdSelectionReceipt), CoreError> {
    clock
        .run(async {
            let identity = source.identity().await?;
            clock.check()?;
            let observation = crate::xpc::rsd::checked_handshake_with_connector_observed(
                || async {
                    clock.check().map_err(connection_error)?;
                    let stream = source.dial(route).await.map_err(connection_error)?;
                    clock.check().map_err(connection_error)?;
                    let mut stream = clock.guard(stream);
                    if route.proxy_port.is_some() {
                        stream.write_all(&route.endpoint.ip().octets()).await?;
                        stream
                            .write_all(&(route.endpoint.port() as u32).to_le_bytes())
                            .await?;
                        stream.flush().await?;
                    }
                    clock.check().map_err(connection_error)?;
                    Ok(stream)
                },
                identity,
                clock.deadline,
            )
            .await?;
            Ok(observation)
        })
        .await
}

#[derive(Debug)]
struct DeadlineReached;

impl fmt::Display for DeadlineReached {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("existing RSD probe deadline reached")
    }
}

impl Error for DeadlineReached {}

fn is_deadline_reached(error: &(dyn Error + 'static)) -> bool {
    let mut current = Some(error);
    for _ in 0..32 {
        let Some(error) = current else { break };
        if error.is::<DeadlineReached>() {
            return true;
        }
        current = if let Some(error) = error.downcast_ref::<io::Error>() {
            error
                .get_ref()
                .map(|source| source as &(dyn Error + 'static))
        } else {
            error.source()
        };
    }
    false
}

struct Clock {
    deadline: Instant,
    budget: Duration,
}

impl Clock {
    fn new(budget: Duration) -> Result<Self, CoreError> {
        let deadline = Instant::now().checked_add(budget).ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::InvalidInput,
                "existing RSD deadline overflow",
            )
        })?;
        Ok(Self { deadline, budget })
    }

    fn check(&self) -> Result<(), CoreError> {
        if Instant::now() >= self.deadline {
            Err(io::Error::new(io::ErrorKind::TimedOut, DeadlineReached).into())
        } else {
            Ok(())
        }
    }

    fn guard(&self, stream: ServiceStream) -> ServiceStream {
        Box::new(DeadlineStream {
            stream,
            deadline: self.deadline,
        })
    }

    async fn run<T>(
        &self,
        operation: impl Future<Output = Result<T, CoreError>>,
    ) -> Result<T, CoreError> {
        tokio::pin!(operation);
        let guarded = std::future::poll_fn(|cx| {
            if self.check().is_err() {
                return Poll::Pending;
            }
            match operation.as_mut().poll(cx) {
                Poll::Ready(Err(error)) if is_deadline_reached(&error) => Poll::Pending,
                Poll::Ready(Ok(_)) if self.check().is_err() => Poll::Pending,
                result => result,
            }
        });
        tokio::time::timeout_at(self.deadline, guarded)
            .await
            .map_err(|source| {
                crate::error::context::deadline_io("existing RSD probe", self.budget, source)
            })?
    }
}

struct DeadlineStream {
    stream: ServiceStream,
    deadline: Instant,
}

impl AsyncRead for DeadlineStream {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        if Instant::now() >= self.deadline {
            return Poll::Pending;
        }
        let result = Pin::new(&mut self.stream).poll_read(cx, buf);
        match result {
            Poll::Ready(Ok(())) if Instant::now() >= self.deadline => Poll::Pending,
            result => result,
        }
    }
}

impl AsyncWrite for DeadlineStream {
    fn poll_write(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        bytes: &[u8],
    ) -> Poll<io::Result<usize>> {
        if Instant::now() >= self.deadline {
            return Poll::Pending;
        }
        let result = Pin::new(&mut self.stream).poll_write(cx, bytes);
        match result {
            Poll::Ready(Ok(_)) if Instant::now() >= self.deadline => Poll::Pending,
            result => result,
        }
    }

    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        if Instant::now() >= self.deadline {
            return Poll::Pending;
        }
        let result = Pin::new(&mut self.stream).poll_flush(cx);
        match result {
            Poll::Ready(Ok(())) if Instant::now() >= self.deadline => Poll::Pending,
            result => result,
        }
    }

    fn poll_shutdown(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        if Instant::now() >= self.deadline {
            Poll::Pending
        } else {
            Pin::new(&mut self.stream).poll_shutdown(cx)
        }
    }
}

#[cfg(test)]
mod tests;
