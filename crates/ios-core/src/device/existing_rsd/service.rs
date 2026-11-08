//! Current directory and service sockets share one absolute observer clock.
//! No native tunnel, pairing or background task is created by this owner.
use super::*;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex as StdMutex};
use tokio::sync::{Notify, OwnedSemaphorePermit, Semaphore};

const MAX_SERVICE_CONNECTIONS: usize = 4;
const MAX_SERVICE_WIRE_BYTES: usize = 64 * 1024 * 1024;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ExistingRsdServiceKind {
    /// Canonical RemoteXPC only; a lockdown shim is rejected before dialing.
    RemoteXpc,
    /// Raw DTX stream; a resolved shim completes the original RSDCheckin.
    RawDtx,
}

pub struct ExistingRsdServiceConnection {
    pub stream: ServiceStream,
    pub resolved_service_name: String,
    pub port: u16,
    pub features: Option<Vec<String>>,
    pub close_owner: ExistingRsdServiceOwner,
}
impl fmt::Debug for ExistingRsdServiceConnection {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ExistingRsdServiceConnection")
            .field("port", &self.port)
            .field(
                "shim",
                &self.resolved_service_name.ends_with(".shim.remote"),
            )
            .field("feature_count", &self.features.as_ref().map(Vec::len))
            .finish_non_exhaustive()
    }
}

/// A fresh checked directory and all subsequent finite service operations.
/// The caller-selected route is borrowed; dropping this owner never stops it.
pub struct ExistingRsdSession {
    route: ExistingRsdRoute,
    clock: Clock,
    observation: ExistingRsdObservation,
    selection: RsdSelectionReceipt,
    connections: Arc<Semaphore>,
    sockets: Arc<ServiceRegistry>,
}
impl fmt::Debug for ExistingRsdSession {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ExistingRsdSession")
            .field("observation", &self.observation)
            .field("selection", &self.selection)
            .finish_non_exhaustive()
    }
}
impl ExistingRsdSession {
    pub async fn observe(
        route: ExistingRsdRoute,
        total_budget: Duration,
    ) -> Result<Self, CoreError> {
        Self::observe_from_source(&NativeSource, route, total_budget).await
    }

    async fn observe_from_source(
        source: &dyn Source,
        route: ExistingRsdRoute,
        total_budget: Duration,
    ) -> Result<Self, CoreError> {
        let clock = Clock::new(total_budget)?;
        let (observation, selection) = observe_selected_from_source(source, route, &clock).await?;
        Ok(Self {
            route,
            clock,
            observation,
            selection,
            connections: Arc::new(Semaphore::new(MAX_SERVICE_CONNECTIONS)),
            sockets: ServiceRegistry::new(),
        })
    }

    pub fn observation(&self) -> &ExistingRsdObservation {
        &self.observation
    }
    pub fn selection(&self) -> &RsdSelectionReceipt {
        &self.selection
    }
    pub fn remaining_budget(&self) -> Duration {
        self.clock
            .deadline
            .saturating_duration_since(Instant::now())
    }

    /// The same clock covers the actual protocol bootstrap and service query.
    /// Acquired native errors in a crossing poll retain their original cause.
    pub async fn run<T>(
        &self,
        operation: impl Future<Output = Result<T, CoreError>>,
    ) -> Result<T, CoreError> {
        self.clock.run(operation).await
    }

    /// Retire only sockets opened by this session. Cleanup has a separate
    /// absolute deadline and raw drain ceiling; it cannot resume business IO.
    pub async fn close(
        &self,
        cleanup_budget: Duration,
    ) -> Result<ExistingRsdCleanupReceipt, CoreError> {
        self.sockets.close(cleanup_budget).await
    }

    pub async fn open_service(
        &self,
        expected_udid: &str,
        requested_service: &str,
        kind: ExistingRsdServiceKind,
        max_wire_bytes: usize,
    ) -> Result<ExistingRsdServiceConnection, CoreError> {
        self.open_from_source(
            &NativeSource,
            expected_udid,
            requested_service,
            kind,
            max_wire_bytes,
        )
        .await
    }

    async fn open_from_source(
        &self,
        source: &dyn Source,
        expected_udid: &str,
        requested_service: &str,
        kind: ExistingRsdServiceKind,
        max_wire_bytes: usize,
    ) -> Result<ExistingRsdServiceConnection, CoreError> {
        self.clock
            .run(async {
                if expected_udid.is_empty()
                    || expected_udid.len() > MAX_FIELD_BYTES
                    || requested_service.is_empty()
                    || requested_service.len() > MAX_FIELD_BYTES
                    || max_wire_bytes == 0
                    || max_wire_bytes > MAX_SERVICE_WIRE_BYTES
                {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidInput,
                        "existing RSD service limits or selectors invalid",
                    )
                    .into());
                }
                if self.observation.udid() != Some(expected_udid) {
                    return Err(CoreError::Protocol(
                        "existing RSD directory identity does not match selected target".into(),
                    ));
                }
                let services = self.observation.services().ok_or_else(|| {
                    CoreError::Protocol("existing RSD directory metadata missing".into())
                })?;
                let shim = format!("{requested_service}.shim.remote");
                let (resolved_name, descriptor) = services
                    .get_key_value(requested_service)
                    .or_else(|| services.get_key_value(&shim))
                    .ok_or_else(|| {
                        if self.observation.complete_service_directory() {
                            CoreError::Xpc(XpcError::ServiceNotFound(requested_service.into()))
                        } else {
                            CoreError::Protocol(
                                "existing RSD directory cannot establish service absence".into(),
                            )
                        }
                    })?;
                let port = descriptor.port().ok_or_else(|| {
                    CoreError::Protocol(
                        "existing RSD canonical service port metadata missing".into(),
                    )
                })?;
                if kind == ExistingRsdServiceKind::RemoteXpc {
                    super::super::service_routing::ensure_remote_xpc_service(
                        requested_service,
                        resolved_name,
                    )?;
                }
                let permit = self.connections.clone().try_acquire_owned().map_err(|_| {
                    CoreError::Io(io::Error::other(ExistingRsdProbeError {
                        phase: "existing RSD service connection capacity",
                        class: ErrorClass::ResourceLimit,
                    }))
                })?;
                let reservation = self.sockets.reserve()?;
                let mut endpoint = self.route.endpoint;
                endpoint.set_port(port);
                let route = ExistingRsdRoute::new(endpoint, self.route.proxy_port)?;
                self.clock.check()?;
                let raw = tokio::select! {
                    biased;
                    result = source.dial(route) => result?,
                    _ = self.sockets.wait_closed() => return Err(cancelled()),
                };
                let owner = ExistingRsdServiceOwner(Arc::new(StdMutex::new(SocketState {
                    stream: Some(raw),
                    remaining: max_wire_bytes,
                    terminal: false,
                    write_closed: false,
                    eof_seen: false,
                    drained_bytes: 0,
                    close_error: None,
                    permit: Some(permit),
                })));
                self.sockets.attach(owner.clone());
                drop(reservation);
                if self.sockets.closed.load(Ordering::SeqCst) {
                    return Err(cancelled());
                }
                let mut stream: ServiceStream = Box::new(ServiceWireStream {
                    owner: owner.clone(),
                    deadline: self.clock.deadline,
                    registry: Some(self.sockets.clone()),
                    timer: Box::pin(tokio::time::sleep_until(self.clock.deadline)),
                });
                self.clock.check()?;
                if route.proxy_port.is_some() {
                    stream.write_all(&route.endpoint.ip().octets()).await?;
                    stream.write_all(&(port as u32).to_le_bytes()).await?;
                    stream.flush().await?;
                }
                if resolved_name.ends_with(".shim.remote") {
                    super::super::service_routing::rsd_checkin_with_timeout(
                        &mut stream,
                        self.remaining_budget()
                            .min(super::super::service_routing::RSD_CHECKIN_TIMEOUT),
                    )
                    .await?;
                }
                self.clock.check()?;
                Ok(ExistingRsdServiceConnection {
                    stream,
                    resolved_service_name: resolved_name.clone(),
                    port,
                    features: descriptor.features().map(<[String]>::to_vec),
                    close_owner: owner,
                })
            })
            .await
    }
}

const MAX_CLEANUP_DRAIN_BYTES: usize = 1024 * 1024;
fn cancelled() -> CoreError {
    io::Error::new(io::ErrorKind::Interrupted, crate::OperationCancelled::new()).into()
}
struct RegistryState {
    owners: Vec<ExistingRsdServiceOwner>,
    pending_dials: usize,
}
struct ServiceRegistry {
    closed: AtomicBool,
    state: StdMutex<RegistryState>,
    changed: Notify,
}
impl ServiceRegistry {
    fn new() -> Arc<Self> {
        Arc::new(Self {
            closed: AtomicBool::new(false),
            state: StdMutex::new(RegistryState {
                owners: Vec::new(),
                pending_dials: 0,
            }),
            changed: Notify::new(),
        })
    }
    fn reserve(self: &Arc<Self>) -> Result<DialReservation, CoreError> {
        let mut state = self.state.lock().unwrap_or_else(|p| p.into_inner());
        if self.closed.load(Ordering::SeqCst) {
            return Err(cancelled());
        }
        state.owners.retain(|owner| !owner.closed());
        state.pending_dials += 1;
        Ok(DialReservation(self.clone()))
    }
    fn attach(&self, owner: ExistingRsdServiceOwner) {
        let mut state = self.state.lock().unwrap_or_else(|p| p.into_inner());
        state.owners.push(owner);
        debug_assert!(state.owners.len() <= MAX_SERVICE_CONNECTIONS);
        drop(state);
        self.changed.notify_waiters();
    }
    async fn wait_closed(&self) {
        loop {
            let changed = self.changed.notified();
            tokio::pin!(changed);
            changed.as_mut().enable();
            if self.closed.load(Ordering::SeqCst) {
                return;
            }
            changed.await;
        }
    }
    async fn close(
        self: &Arc<Self>,
        budget: Duration,
    ) -> Result<ExistingRsdCleanupReceipt, CoreError> {
        {
            let _state = self.state.lock().unwrap_or_else(|p| p.into_inner());
            self.closed.store(true, Ordering::SeqCst);
        }
        self.changed.notify_waiters();
        let clock = match Clock::new(budget) {
            Ok(clock) => clock,
            Err(error) => return Err(retained_registry_cleanup(Arc::new(error), self.clone())),
        };
        let result = clock
            .run(async {
                loop {
                    let changed = self.changed.notified();
                    tokio::pin!(changed);
                    changed.as_mut().enable();
                    let owners = self
                        .state
                        .lock()
                        .unwrap_or_else(|p| p.into_inner())
                        .owners
                        .clone();
                    let mut first = None;
                    for owner in &owners {
                        if let Err(error) = owner.close_on_clock(&clock).await {
                            if first.is_none() {
                                first = Some(error);
                            }
                        }
                    }
                    let (pending, all_closed, current) = {
                        let state = self.state.lock().unwrap_or_else(|p| p.into_inner());
                        (
                            state.pending_dials,
                            state.owners.iter().all(|owner| owner.closed()),
                            state.owners.clone(),
                        )
                    };
                    if pending == 0 {
                        if !all_closed {
                            if let Some(error) = first {
                                return Err(error);
                            }
                            continue;
                        }
                        if let Some(error) = first {
                            return Err(error);
                        }
                        return Ok(cleanup_receipt(&current));
                    }
                    changed.await;
                }
            })
            .await;
        result.map_err(|error| retained_registry_cleanup(Arc::new(error), self.clone()))
    }
}
struct DialReservation(Arc<ServiceRegistry>);
impl Drop for DialReservation {
    fn drop(&mut self) {
        let mut state = self.0.state.lock().unwrap_or_else(|p| p.into_inner());
        state.pending_dials -= 1;
        drop(state);
        self.0.changed.notify_waiters();
    }
}
struct SocketState {
    stream: Option<ServiceStream>,
    remaining: usize,
    terminal: bool,
    write_closed: bool,
    eof_seen: bool,
    drained_bytes: usize,
    close_error: Option<Arc<CoreError>>,
    permit: Option<OwnedSemaphorePermit>,
}

/// The original raw socket remains outside a cancellable client operation.
/// Closing enables only shutdown and bounded raw drainage, never business IO.
#[derive(Clone)]
pub struct ExistingRsdServiceOwner(Arc<StdMutex<SocketState>>);
impl fmt::Debug for ExistingRsdServiceOwner {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let state = self.0.lock().unwrap_or_else(|p| p.into_inner());
        f.debug_struct("ExistingRsdServiceOwner")
            .field("closed", &state.stream.is_none())
            .field("eof_seen", &state.eof_seen)
            .field("write_closed", &state.write_closed)
            .field("drained_bytes", &state.drained_bytes)
            .finish_non_exhaustive()
    }
}
impl ExistingRsdServiceOwner {
    fn closed(&self) -> bool {
        self.0
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .stream
            .is_none()
    }
    pub async fn close(
        &self,
        cleanup_budget: Duration,
    ) -> Result<ExistingRsdCleanupReceipt, CoreError> {
        close_owners(vec![self.clone()], cleanup_budget).await
    }

    async fn close_on_clock(&self, clock: &Clock) -> Result<(), CoreError> {
        let mut scratch = [0; 8192];
        clock
            .run(std::future::poll_fn(|cx| {
                let mut state = self.0.lock().unwrap_or_else(|p| p.into_inner());
                state.terminal = true;
                if state.stream.is_none() {
                    let original = state.close_error.clone();
                    drop(state);
                    return Poll::Ready(match original {
                        Some(original) => Err(retained_cleanup(original, vec![self.clone()])),
                        None => Ok(()),
                    });
                }
                if !state.write_closed {
                    match Pin::new(state.stream.as_mut().expect("closing original socket"))
                        .poll_shutdown(cx)
                    {
                        Poll::Pending => return Poll::Pending,
                        Poll::Ready(Ok(())) => state.write_closed = true,
                        Poll::Ready(Err(error)) => {
                            let original = Arc::new(CoreError::Io(error));
                            state.close_error = Some(Arc::clone(&original));
                            let socket = state.stream.take();
                            let permit = state.permit.take();
                            drop(state);
                            drop(socket);
                            drop(permit);
                            return Poll::Ready(Err(retained_cleanup(
                                original,
                                vec![self.clone()],
                            )));
                        }
                    }
                }
                for _ in 0..16 {
                    if state.eof_seen {
                        let socket = state.stream.take();
                        let permit = state.permit.take();
                        drop(state);
                        drop(socket);
                        drop(permit);
                        return Poll::Ready(Ok(()));
                    }
                    let available = MAX_CLEANUP_DRAIN_BYTES.saturating_sub(state.drained_bytes);
                    if available == 0 {
                        return Poll::Ready(Err(CoreError::Io(io::Error::other(
                            ExistingRsdProbeError {
                                phase: "existing RSD cleanup cumulative drain limit",
                                class: ErrorClass::ResourceLimit,
                            },
                        ))));
                    }
                    let count = available.min(scratch.len());
                    let mut buffer = ReadBuf::new(&mut scratch[..count]);
                    match Pin::new(state.stream.as_mut().expect("draining original socket"))
                        .poll_read(cx, &mut buffer)
                    {
                        Poll::Pending => return Poll::Pending,
                        Poll::Ready(Err(error)) => {
                            let original = Arc::new(CoreError::Io(error));
                            state.close_error = Some(Arc::clone(&original));
                            let socket = state.stream.take();
                            let permit = state.permit.take();
                            drop(state);
                            drop(socket);
                            drop(permit);
                            return Poll::Ready(Err(retained_cleanup(
                                original,
                                vec![self.clone()],
                            )));
                        }
                        Poll::Ready(Ok(())) => {
                            let count = buffer.filled().len();
                            state.drained_bytes += count;
                            state.eof_seen = count == 0;
                        }
                    }
                }
                cx.waker().wake_by_ref();
                Poll::Pending
            }))
            .await
    }
}

#[derive(Clone, Copy, Debug, serde::Serialize)]
pub struct ExistingRsdCleanupReceipt {
    pub opened_sockets: usize,
    pub closed_sockets: usize,
    pub drained_bytes: usize,
    pub strict_eof: bool,
}

/// Independent cleanup cause and original socket owners, including on expiry.
pub struct ExistingRsdCleanupFailure {
    primary: Arc<CoreError>,
    secondary: Vec<Arc<CoreError>>,
    owners: Vec<ExistingRsdServiceOwner>,
    registry: Option<Arc<ServiceRegistry>>,
}
impl ExistingRsdCleanupFailure {
    pub fn primary(&self) -> &CoreError {
        &self.primary
    }
    pub fn secondary(&self) -> &[Arc<CoreError>] {
        &self.secondary
    }
    pub fn pending_sockets(&self) -> usize {
        if let Some(registry) = &self.registry {
            registry
                .state
                .lock()
                .unwrap_or_else(|p| p.into_inner())
                .owners
                .iter()
                .filter(|owner| !owner.closed())
                .count()
        } else {
            self.owners.iter().filter(|owner| !owner.closed()).count()
        }
    }
    pub fn pending_dials(&self) -> usize {
        self.registry.as_ref().map_or(0, |registry| {
            registry
                .state
                .lock()
                .unwrap_or_else(|p| p.into_inner())
                .pending_dials
        })
    }
    pub async fn retry(&self, budget: Duration) -> Result<ExistingRsdCleanupReceipt, CoreError> {
        if let Some(registry) = &self.registry {
            registry.close(budget).await
        } else {
            close_owners(self.owners.clone(), budget).await
        }
    }
}
impl fmt::Display for ExistingRsdCleanupFailure {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("existing RSD original socket cleanup failed")
    }
}
impl fmt::Debug for ExistingRsdCleanupFailure {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ExistingRsdCleanupFailure")
            .field("primary_class", &crate::classify_error(self.primary()))
            .field("secondary_causes", &self.secondary.len())
            .field("pending_sockets", &self.pending_sockets())
            .finish()
    }
}
impl Error for ExistingRsdCleanupFailure {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        Some(self.primary())
    }
}
fn retained_cleanup(primary: Arc<CoreError>, owners: Vec<ExistingRsdServiceOwner>) -> CoreError {
    let (primary, secondary) = original_cleanup_causes(primary, &owners);
    CoreError::Io(io::Error::other(ExistingRsdCleanupFailure {
        primary,
        secondary,
        owners,
        registry: None,
    }))
}
fn retained_registry_cleanup(primary: Arc<CoreError>, registry: Arc<ServiceRegistry>) -> CoreError {
    let owners = registry
        .state
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .owners
        .clone();
    let (primary, secondary) = original_cleanup_causes(primary, &owners);
    CoreError::Io(io::Error::other(ExistingRsdCleanupFailure {
        primary,
        secondary,
        owners: Vec::new(),
        registry: Some(registry),
    }))
}
fn original_cleanup_causes(
    completion: Arc<CoreError>,
    owners: &[ExistingRsdServiceOwner],
) -> (Arc<CoreError>, Vec<Arc<CoreError>>) {
    let mut causes = owners
        .iter()
        .filter_map(|owner| {
            owner
                .0
                .lock()
                .unwrap_or_else(|p| p.into_inner())
                .close_error
                .clone()
        })
        .collect::<Vec<_>>();
    if causes.is_empty() {
        return (completion, Vec::new());
    }
    let primary = causes.remove(0);
    if !Arc::ptr_eq(&primary, &completion) {
        causes.push(completion);
    }
    (primary, causes)
}
async fn close_owners(
    owners: Vec<ExistingRsdServiceOwner>,
    budget: Duration,
) -> Result<ExistingRsdCleanupReceipt, CoreError> {
    let clock = match Clock::new(budget) {
        Ok(clock) => clock,
        Err(error) => return Err(retained_cleanup(Arc::new(error), owners)),
    };
    let result = clock
        .run(async {
            let mut first = None;
            for owner in &owners {
                if let Err(error) = owner.close_on_clock(&clock).await {
                    if first.is_none() {
                        first = Some(error);
                    }
                }
            }
            if let Some(error) = first {
                Err(error)
            } else {
                Ok(cleanup_receipt(&owners))
            }
        })
        .await;
    result.map_err(|error| retained_cleanup(Arc::new(error), owners))
}
fn cleanup_receipt(owners: &[ExistingRsdServiceOwner]) -> ExistingRsdCleanupReceipt {
    let mut closed_sockets = 0;
    let mut drained_bytes = 0;
    let mut strict_eof = true;
    for owner in owners {
        let state = owner.0.lock().unwrap_or_else(|p| p.into_inner());
        closed_sockets += usize::from(state.stream.is_none());
        drained_bytes += state.drained_bytes;
        strict_eof &= state.eof_seen;
    }
    ExistingRsdCleanupReceipt {
        opened_sockets: owners.len(),
        closed_sockets,
        drained_bytes,
        strict_eof,
    }
}

struct ServiceWireStream {
    owner: ExistingRsdServiceOwner,
    deadline: Instant,
    timer: Pin<Box<tokio::time::Sleep>>,
    registry: Option<Arc<ServiceRegistry>>,
}
impl ServiceWireStream {
    fn expired(&mut self, cx: &mut Context<'_>) -> bool {
        std::future::Future::poll(self.timer.as_mut(), cx).is_ready()
    }
    fn expired_error(state: &mut SocketState) -> io::Error {
        state.terminal = true;
        io::Error::new(io::ErrorKind::TimedOut, DeadlineReached)
    }
    fn resource_error(state: &mut SocketState) -> io::Error {
        state.terminal = true;
        io::Error::other(ExistingRsdProbeError {
            phase: "existing RSD service cumulative wire limit",
            class: ErrorClass::ResourceLimit,
        })
    }
    fn admission(
        &mut self,
        state: &mut SocketState,
        cx: &mut Context<'_>,
    ) -> Result<(), io::Error> {
        if state.terminal
            || state.stream.is_none()
            || self
                .registry
                .as_ref()
                .is_some_and(|registry| registry.closed.load(Ordering::SeqCst))
        {
            return Err(io::Error::new(
                io::ErrorKind::NotConnected,
                "existing RSD service stream terminal",
            ));
        }
        if self.expired(cx) {
            return Err(Self::expired_error(state));
        }
        Ok(())
    }
}
impl AsyncRead for ServiceWireStream {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        let owner = self.owner.clone();
        let mut state = owner.0.lock().unwrap_or_else(|p| p.into_inner());
        if let Err(error) = self.admission(&mut state, cx) {
            return Poll::Ready(Err(error));
        }
        if buf.remaining() == 0 {
            return Poll::Ready(Ok(()));
        }
        if state.remaining == 0 {
            return Poll::Ready(Err(Self::resource_error(&mut state)));
        }
        let count = buf.remaining().min(state.remaining);
        let mut limited = ReadBuf::new(buf.initialize_unfilled_to(count));
        match Pin::new(state.stream.as_mut().expect("admitted socket")).poll_read(cx, &mut limited)
        {
            Poll::Ready(Err(error)) => {
                state.terminal = true;
                Poll::Ready(Err(error))
            }
            Poll::Ready(Ok(())) => {
                let count = limited.filled().len();
                buf.advance(count);
                state.remaining -= count;
                if count == 0 {
                    state.eof_seen = true;
                }
                if Instant::now() >= self.deadline {
                    Poll::Ready(Err(Self::expired_error(&mut state)))
                } else {
                    Poll::Ready(Ok(()))
                }
            }
            Poll::Pending => Poll::Pending,
        }
    }
}
impl AsyncWrite for ServiceWireStream {
    fn poll_write(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        bytes: &[u8],
    ) -> Poll<io::Result<usize>> {
        let owner = self.owner.clone();
        let mut state = owner.0.lock().unwrap_or_else(|p| p.into_inner());
        if let Err(error) = self.admission(&mut state, cx) {
            return Poll::Ready(Err(error));
        }
        if bytes.len() > state.remaining {
            return Poll::Ready(Err(Self::resource_error(&mut state)));
        }
        match Pin::new(state.stream.as_mut().expect("admitted socket")).poll_write(cx, bytes) {
            Poll::Ready(Err(error)) => {
                state.terminal = true;
                Poll::Ready(Err(error))
            }
            Poll::Ready(Ok(count)) => {
                state.remaining -= count;
                if Instant::now() >= self.deadline {
                    Poll::Ready(Err(Self::expired_error(&mut state)))
                } else {
                    Poll::Ready(Ok(count))
                }
            }
            Poll::Pending => Poll::Pending,
        }
    }
    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        let owner = self.owner.clone();
        let mut state = owner.0.lock().unwrap_or_else(|p| p.into_inner());
        if let Err(error) = self.admission(&mut state, cx) {
            return Poll::Ready(Err(error));
        }
        match Pin::new(state.stream.as_mut().expect("admitted socket")).poll_flush(cx) {
            Poll::Ready(Err(error)) => {
                state.terminal = true;
                Poll::Ready(Err(error))
            }
            Poll::Ready(Ok(())) if Instant::now() >= self.deadline => {
                Poll::Ready(Err(Self::expired_error(&mut state)))
            }
            result => result,
        }
    }
    fn poll_shutdown(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        // This is a close transition only. It never enables reads or writes
        // after the business deadline; session.close owns the bounded drain.
        let mut state = self.owner.0.lock().unwrap_or_else(|p| p.into_inner());
        state.terminal = true;
        if state.write_closed || state.stream.is_none() {
            return Poll::Ready(Ok(()));
        }
        match Pin::new(state.stream.as_mut().expect("closing socket")).poll_shutdown(cx) {
            Poll::Ready(Ok(())) => {
                state.write_closed = true;
                Poll::Ready(Ok(()))
            }
            result => result,
        }
    }
}

#[cfg(test)]
mod tests;
