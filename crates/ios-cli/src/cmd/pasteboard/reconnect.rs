//! Finite Darwin watch recovery. A new attempt owns a new device and monitor.
use std::error::Error;
use std::future::Future;
use std::io;
use std::pin::Pin;
use std::sync::{Arc, Mutex};
use std::task::Poll;
use std::time::Duration;

use anyhow::Result;
use futures_util::future::BoxFuture;
use ios_core::pasteboard::{DataInclusionPolicy, PasteboardError, PasteboardMonitor, PasteboardSnapshot};
use ios_core::{CoreError, ErrorClass, ServiceStream, TunnelHandle, TunnelOwner, TunnelReservation, TunnelShutdownFailure};
use tokio::time::Instant;

#[derive(Clone, Debug, clap::Args)]
pub(super) struct WatchOptions {
    #[arg(long, conflicts_with = "autonotify", help = "Recover Darwin watch with fresh device/service discovery; emits state on stderr")]
    pub(super) reconnect: bool,
    #[arg(long, default_value_t = 3, requires = "reconnect", conflicts_with = "autonotify", help = "Maximum additional connections across this watch (0..32)")]
    pub(super) reconnect_attempts: u32,
    #[arg(long, default_value_t = 60, requires = "reconnect", conflicts_with = "autonotify", help = "Recovery seconds from the first failure, including cleanup and backoff (1..3600)")]
    pub(super) reconnect_elapsed: u64,
    #[arg(long, default_value_t = 250, requires = "reconnect", conflicts_with = "autonotify")]
    pub(super) reconnect_backoff_ms: u64,
    #[arg(long, default_value_t = 5000, requires = "reconnect", conflicts_with = "autonotify")]
    pub(super) reconnect_max_backoff_ms: u64,
    #[arg(long, default_value_t = 30, conflicts_with = "autonotify", help = "Seconds for each fresh connection, service opening and baseline (1..300)")]
    pub(super) connect_timeout: u64,
    #[arg(long, conflicts_with = "autonotify", help = "Overall watch seconds, including first connection, reads and cleanup")]
    pub(super) timeout: Option<u64>,
    #[arg(long, default_value_t = 10, conflicts_with = "autonotify", help = "Seconds to stop and join each original native owner, within remaining total/recovery time (1..60)")]
    pub(super) cleanup_timeout: u64,
}

impl Default for WatchOptions {
    fn default() -> Self {
        Self { reconnect: false, reconnect_attempts: 3, reconnect_elapsed: 60,
            reconnect_backoff_ms: 250, reconnect_max_backoff_ms: 5000,
            connect_timeout: 30, timeout: None, cleanup_timeout: 10 }
    }
}

impl WatchOptions {
    pub(super) fn validate(&self, darwin: bool) -> Result<()> {
        anyhow::ensure!(darwin || (!self.reconnect && self.timeout.is_none()
            && self.connect_timeout == 30 && self.cleanup_timeout == 10
            && self.reconnect_attempts == 3 && self.reconnect_elapsed == 60
            && self.reconnect_backoff_ms == 250 && self.reconnect_max_backoff_ms == 5000),
            crate::output::input_decision("watch recovery options require Darwin notifications"));
        anyhow::ensure!(self.reconnect_attempts <= 32,
            crate::output::resource_decision("pasteboard watch retry count limit"));
        anyhow::ensure!((1..=3600).contains(&self.reconnect_elapsed)
            && (1..=300).contains(&self.connect_timeout)
            && (1..=60).contains(&self.cleanup_timeout)
            && (1..=60_000).contains(&self.reconnect_backoff_ms)
            && (self.reconnect_backoff_ms..=60_000).contains(&self.reconnect_max_backoff_ms)
            && self.timeout.map_or(true, |seconds| (1..=604_800).contains(&seconds)),
            crate::output::input_decision("invalid pasteboard watch deadline or backoff"));
        Ok(())
    }

    fn delay(&self, retry: u32) -> Duration {
        Duration::from_millis(self.reconnect_backoff_ms.saturating_mul(1u64 << retry.min(32))
            .min(self.reconnect_max_backoff_ms))
    }
}

pub(super) fn selectors(udid: &str, name: &str, policy: DataInclusionPolicy) -> Result<()> {
    anyhow::ensure!(!udid.is_empty() && !udid.contains('\0') && !name.is_empty() && !name.contains('\0'),
        crate::output::input_decision("Darwin watch requires an exact device and pasteboard selector"));
    anyhow::ensure!(udid.len() <= 4096 && name.len() <= 4096,
        crate::output::resource_decision("pasteboard watch selector limit"));
    anyhow::ensure!(!matches!(policy, DataInclusionPolicy::Threshold(value) if value < 0),
        crate::output::input_decision("negative pasteboard data threshold"));
    Ok(())
}

pub(super) trait WatchDevice: Send + Sync {
    fn notification(&self) -> BoxFuture<'_, Result<ServiceStream>>;
    fn snapshot<'a>(&'a self, name: &'a str, policy: DataInclusionPolicy)
        -> BoxFuture<'a, std::result::Result<PasteboardSnapshot, PasteboardError>>;
}

pub(super) trait WatchSource: Send + Sync {
    fn connect<'a>(&'a self, udid: &'a str, owner: &'a dyn TunnelOwner)
        -> BoxFuture<'a, Result<Arc<dyn WatchDevice>>>;
}

pub(super) struct DeviceSource;
struct Device(ios_core::ConnectedDevice);

impl WatchSource for DeviceSource {
    fn connect<'a>(&'a self, udid: &'a str, owner: &'a dyn TunnelOwner)
        -> BoxFuture<'a, Result<Arc<dyn WatchDevice>>> {
        Box::pin(async move {
            let device = ios_core::connect_with_owner(udid, ios_core::ConnectOptions {
                tun_mode: ios_core::TunMode::Userspace, pair_record_path: None, skip_tunnel: false,
            }, owner).await?;
            anyhow::ensure!(device.info.udid == udid,
                CoreError::Protocol("pasteboard connection identity differs from explicit selector".into()));
            Ok(Arc::new(Device(device)) as Arc<dyn WatchDevice>)
        })
    }
}

impl WatchDevice for Device {
    fn notification(&self) -> BoxFuture<'_, Result<ServiceStream>> {
        Box::pin(async { Ok(self.0.connect_rsd_service(ios_core::notificationproxy::RSD_SERVICE_NAME).await?) })
    }
    fn snapshot<'a>(&'a self, name: &'a str, policy: DataInclusionPolicy)
        -> BoxFuture<'a, std::result::Result<PasteboardSnapshot, PasteboardError>> {
        Box::pin(super::read_notification_snapshot(
            self.0.connect_xpc_service(ios_core::pasteboard::SERVICE_NAME), name, policy))
    }
}

#[derive(Default)]
struct OwnerState { reserved: bool, handle: Option<Arc<TunnelHandle>> }

/// One fixed cleanup slot, retained before activation and across canceled setup.
#[derive(Default)]
pub(super) struct NativeOwner(Mutex<OwnerState>);
struct Reservation<'a> { owner: &'a NativeOwner, completed: bool }

struct CapacityCause(Box<dyn Error + Send + Sync>);
impl std::fmt::Debug for CapacityCause {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result { f.write_str("PasteboardOwnerCapacity") }
}
impl std::fmt::Display for CapacityCause {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result { f.write_str("pasteboard native owner capacity") }
}
impl Error for CapacityCause {
    fn source(&self) -> Option<&(dyn Error + 'static)> { Some(self.0.as_ref()) }
}
fn capacity() -> CoreError {
    CoreError::Io(io::Error::other(CapacityCause(
        crate::output::resource_decision("pasteboard native owner capacity")
            .into_boxed_dyn_error())))
}

impl TunnelOwner for NativeOwner {
    fn reserve(&self) -> std::result::Result<Box<dyn TunnelReservation + '_>, CoreError> {
        let mut state = self.0.lock().unwrap_or_else(|p| p.into_inner());
        if state.reserved || state.handle.is_some() { return Err(capacity()); }
        state.reserved = true;
        Ok(Box::new(Reservation { owner: self, completed: false }))
    }
}
impl TunnelReservation for Reservation<'_> {
    fn attach(&mut self, handle: Arc<TunnelHandle>) -> std::result::Result<(), CoreError> {
        let mut state = self.owner.0.lock().unwrap_or_else(|p| p.into_inner());
        if state.handle.is_some() {
            drop(state);
            handle.request_stop();
            return Err(capacity());
        }
        state.handle = Some(handle);
        Ok(())
    }
    fn complete(&mut self) { self.completed = true; }
}
impl Drop for Reservation<'_> {
    fn drop(&mut self) {
        let mut state = self.owner.0.lock().unwrap_or_else(|p| p.into_inner());
        state.reserved = false;
        let handle = (!self.completed).then(|| state.handle.clone()).flatten();
        drop(state);
        if let Some(handle) = handle { handle.request_stop(); }
    }
}
impl Drop for NativeOwner {
    fn drop(&mut self) {
        let state = self.0.get_mut().unwrap_or_else(|p| p.into_inner());
        if let Some(handle) = &state.handle { handle.request_stop(); }
    }
}

struct Cleanup { reclaimed: bool, native: Option<anyhow::Error> }

/// Only inspect the report directly returned by shutdown of this slot's exact
/// handle. Never recurse into a foreign report nested in its primary cause.
fn original_receipt(error: &ios_core::TunnelError) -> Option<&TunnelShutdownFailure> {
    let ios_core::TunnelError::Io(io) = error else { return None };
    io.get_ref()?.source()?.downcast_ref::<TunnelShutdownFailure>()
}

impl NativeOwner {
    fn retained(&self) -> Option<Arc<TunnelHandle>> {
        self.0.lock().unwrap_or_else(|p| p.into_inner()).handle.clone()
    }

    async fn cleanup(&self, budget: Duration, until: Option<Instant>) -> Cleanup {
        let own_until = Instant::now().checked_add(budget);
        let handle = self.0.lock().unwrap_or_else(|p| p.into_inner()).handle.clone();
        let Some(handle) = handle else { return Cleanup { reclaimed: true, native: None } };
        let Some(own_until) = own_until else {
            handle.request_stop();
            return Cleanup { reclaimed: false, native: Some(crate::output::input_decision("pasteboard cleanup deadline overflow")) };
        };
        let at = until.map_or(own_until, |at| at.min(own_until));
        // Charge the state-lock wait to the original absolute clocks. The
        // public native owner takes a relative budget; check the whole/recovery
        // clocks again after its receipt before admitting another attempt.
        let result = handle.shutdown(at.saturating_duration_since(Instant::now())).await;
        let reclaimed = match &result {
            Ok(()) => true,
            Err(error) => original_receipt(error).is_some_and(|receipt| {
                receipt.joins_complete() && receipt.has_retained_outcomes()
                    && !receipt.cleanup().any(|cause| ios_core::classify_error(cause) == ErrorClass::Timeout)
            }),
        };
        if reclaimed {
            let mut state = self.0.lock().unwrap_or_else(|p| p.into_inner());
            if state.handle.as_ref().is_some_and(|current| Arc::ptr_eq(current, &handle)) {
                state.handle = None;
            }
        }
        Cleanup { reclaimed, native: result.err().map(anyhow::Error::new) }
    }
}

#[derive(Clone, Debug, serde::Serialize)]
pub(super) struct StateEvent {
    pub(super) operation: &'static str,
    pub(super) state: &'static str,
    pub(super) attempt: u32,
    pub(super) error_class: Option<ErrorClass>,
    pub(super) native_reclaimed: bool,
    pub(super) backoff_ms: Option<u64>,
    pub(super) gap: Option<&'static str>,
}

pub(super) enum WatchEvent<'a> { Snapshot(&'a PasteboardSnapshot), State(StateEvent) }

pub(super) struct WatchReport {
    #[cfg_attr(not(test), allow(dead_code))]
    pub(super) failures: Vec<anyhow::Error>,
    #[cfg_attr(not(test), allow(dead_code))]
    pub(super) retries: u32,
}

struct FailureContext { earlier: Vec<anyhow::Error>, secondary: Option<anyhow::Error> }
struct OutputFailure;
impl std::fmt::Display for OutputFailure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result { f.write_str("write pasteboard watch output") }
}
struct PendingNative { _handle: Arc<TunnelHandle> }
impl std::fmt::Display for PendingNative {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result { f.write_str("pasteboard watch retains an incomplete native join owner") }
}
impl std::fmt::Debug for FailureContext {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PasteboardWatchFailures")
            .field("earlier_classes", &self.earlier.iter().map(|e| crate::output::operation_error_class(e.as_ref())).collect::<Vec<_>>())
            .field("secondary_class", &self.secondary.as_ref().map(|e| crate::output::operation_error_class(e.as_ref())))
            .finish()
    }
}
impl std::fmt::Display for FailureContext {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result { f.write_str("pasteboard watch has retained operation or cleanup failures") }
}
fn combine(primary: anyhow::Error, secondary: anyhow::Error) -> anyhow::Error {
    primary.context(FailureContext { earlier: Vec::new(), secondary: Some(secondary) })
}
fn terminal(mut earlier: Vec<anyhow::Error>, current: anyhow::Error) -> anyhow::Error {
    if earlier.is_empty() { current } else {
        // At most initial + 32 admitted retries, one fixed context per attempt.
        earlier.shrink_to_fit();
        current.context(FailureContext { earlier, secondary: None })
    }
}

fn retryable(error: &anyhow::Error) -> bool {
    if error.downcast_ref::<OutputFailure>().is_some() { return false; }
    match crate::output::operation_error_class(error.as_ref()) {
        ErrorClass::Disconnected | ErrorClass::DeviceNotFound | ErrorClass::Timeout => return true,
        ErrorClass::Io | ErrorClass::Unknown => {}
        _ => return false,
    }
    // EOF/ECONNREFUSED is transport evidence only under these protocol owners,
    // never an arbitrary host file or untyped generic IO error.
    let mut current: Option<&(dyn Error + 'static)> = Some(error.as_ref());
    let mut transport = false;
    for _ in 0..32 {
        let Some(cause) = current else { break };
        if matches!(cause.downcast_ref::<ios_core::notificationproxy::NotificationProxyError>(),
            Some(ios_core::notificationproxy::NotificationProxyError::ProxyDeath)) {
            return true;
        }
        transport |= cause.is::<ios_core::XpcError>()
            || cause.is::<ios_core::notificationproxy::NotificationProxyError>()
            || cause.is::<ios_core::MuxError>() || cause.is::<ios_core::TunnelError>();
        if let Some(io) = cause.downcast_ref::<io::Error>() {
            if transport && matches!(io.kind(), io::ErrorKind::UnexpectedEof | io::ErrorKind::ConnectionRefused) { return true; }
            current = io.get_ref().map(|inner| inner as &(dyn Error + 'static)).or_else(|| cause.source());
        } else { current = cause.source(); }
    }
    false
}

enum Controlled<T> { Ready(Result<T>), Cancel(Result<(), io::Error>), Expired(anyhow::Error) }

/// Cancellation and expired clocks fence every poll before work admission.
/// An original Err acquired by that poll remains primary even if the same poll
/// crosses a clock or observes cancellation; late success is never emitted.
async fn controlled<T, C>(work: impl Future<Output = Result<T>>, cancel: &mut Pin<Box<C>>, until: Option<Instant>) -> Controlled<T>
where C: Future<Output = Result<(), io::Error>> {
    tokio::pin!(work);
    let expiry = async {
        match until {
            Some(at) => tokio::time::timeout_at(at, std::future::pending::<()>()).await.unwrap_err(),
            None => std::future::pending().await,
        }
    };
    tokio::pin!(expiry);
    std::future::poll_fn(|cx| {
        if let Poll::Ready(signal) = cancel.as_mut().poll(cx) {
            return Poll::Ready(Controlled::Cancel(signal));
        }
        if let Poll::Ready(elapsed) = expiry.as_mut().poll(cx) {
            return Poll::Ready(Controlled::Expired(elapsed.into()));
        }
        if until.is_some_and(|at| Instant::now() >= at) { return Poll::Pending; }
        let result = work.as_mut().poll(cx);
        if matches!(&result, Poll::Ready(Err(_))) {
            return result.map(Controlled::Ready);
        }
        if let Poll::Ready(signal) = cancel.as_mut().poll(cx) {
            return Poll::Ready(Controlled::Cancel(signal));
        }
        if let Poll::Ready(elapsed) = expiry.as_mut().poll(cx) {
            return Poll::Ready(Controlled::Expired(elapsed.into()));
        }
        if until.is_some_and(|at| Instant::now() >= at) { return Poll::Pending; }
        result.map(Controlled::Ready)
    }).await
}

async fn precancel<C>(cancel: &mut Pin<Box<C>>) -> Option<Result<(), io::Error>>
where C: Future<Output = Result<(), io::Error>> {
    std::future::poll_fn(|cx| Poll::Ready(match cancel.as_mut().poll(cx) {
        Poll::Ready(signal) => Some(signal), Poll::Pending => None,
    })).await
}

type SnapshotFuture = Pin<Box<dyn Future<Output = std::result::Result<PasteboardSnapshot, PasteboardError>> + Send>>;
type Reader = Box<dyn FnMut(DataInclusionPolicy) -> SnapshotFuture + Send>;
type Monitor = PasteboardMonitor<ServiceStream, Reader>;

async fn setup(source: &dyn WatchSource, owner: &NativeOwner, udid: &str, name: &str, policy: DataInclusionPolicy, retrying: bool, retries: &mut u32) -> Result<Monitor> {
    let mut connect = source.connect(udid, owner);
    let mut admitted = false;
    let device = std::future::poll_fn(|cx| {
        if !admitted {
            admitted = true;
            if retrying { *retries += 1; }
        }
        connect.as_mut().poll(cx)
    }).await?;
    let stream = device.notification().await?;
    let name = name.to_owned();
    let read: Reader = Box::new(move |policy| {
        let device = device.clone(); let name = name.clone();
        Box::pin(async move { device.snapshot(&name, policy).await })
    });
    Ok(PasteboardMonitor::start(stream, read, policy).await?)
}

fn event(state: &'static str, attempt: u32, error: Option<&anyhow::Error>, reclaimed: bool, delay: Option<Duration>) -> StateEvent {
    StateEvent { operation: "pasteboard_watch", state, attempt,
        error_class: error.map(|e| crate::output::operation_error_class(e.as_ref())),
        native_reclaimed: reclaimed, backoff_ms: delay.map(|d| d.as_millis() as u64),
        gap: (attempt > 0).then_some("changes_during_disconnect_not_replayed") }
}

async fn elapsed_at(at: Instant) -> anyhow::Error {
    tokio::time::timeout_at(at, std::future::pending::<()>()).await.unwrap_err().into()
}

pub(super) async fn run<C, E>(source: &dyn WatchSource, udid: &str, name: &str, policy: DataInclusionPolicy,
    options: &WatchOptions, cancel: C, mut emit: E) -> Result<WatchReport>
where C: Future<Output = Result<(), io::Error>>, E: for<'a> FnMut(WatchEvent<'a>) -> Result<()> {
    options.validate(true)?;
    selectors(udid, name, policy)?;
    let start = Instant::now();
    let total = options.timeout.map(|seconds| start.checked_add(Duration::from_secs(seconds)))
        .transpose_option().ok_or_else(|| crate::output::input_decision("pasteboard watch deadline overflow"))?;
    let mut cancel = Box::pin(cancel);
    let mut failures = Vec::new();
    let mut retry_until = None;
    let mut retries = 0;
    let mut retrying = false;
    let mut last_emitted = None;
    let mut emit = |event: WatchEvent<'_>| emit(event).map_err(|error| error.context(OutputFailure));
    loop {
        if let Some(signal) = precancel(&mut cancel).await {
            if let Err(error) = signal { return Err(terminal(failures, anyhow::Error::new(error).context("waiting for Ctrl-C"))); }
            if let Err(error) = emit(WatchEvent::State(event("stopped", retries, None, true, None))) {
                return Err(terminal(failures, error));
            }
            return Ok(WatchReport { failures, retries });
        }
        let owner = NativeOwner::default();
        let connect_until = Instant::now().checked_add(Duration::from_secs(options.connect_timeout))
            .ok_or_else(|| crate::output::input_decision("pasteboard connection deadline overflow"))?;
        let connect_until = total.map_or(connect_until, |at| at.min(connect_until));
        let connect_until = retry_until.map_or(connect_until, |at: Instant| at.min(connect_until));
        if let Err(error) = emit(WatchEvent::State(event(if retrying { "reconnecting" } else { "connecting" }, retries, None, true, None))) {
            return Err(terminal(failures, error));
        }
        let outcome = match controlled(setup(source, &owner, udid, name, policy, retrying, &mut retries), &mut cancel, Some(connect_until)).await {
            Controlled::Ready(Ok(mut monitor)) => {
                let work = async {
                    if total.is_some_and(|at| Instant::now() >= at) {
                        std::future::pending::<()>().await;
                    }
                    emit(WatchEvent::State(event("connected", retries, None, false, None)))?;
                    tokio::task::yield_now().await;
                    loop {
                        let snapshot = monitor.next_change().await?;
                        if total.is_some_and(|at| Instant::now() >= at) {
                            std::future::pending::<()>().await;
                        }
                        let id = snapshot.change_id().map(|(nonce, count)| (nonce.clone(), count));
                        if id.is_some() && id == last_emitted {
                            tokio::task::yield_now().await;
                            continue;
                        }
                        emit(WatchEvent::Snapshot(&snapshot))?;
                        if let Some(id) = id { last_emitted = Some(id); }
                        tokio::task::yield_now().await;
                    }
                    #[allow(unreachable_code)]
                    Ok::<(), anyhow::Error>(())
                };
                controlled(work, &mut cancel, total).await
            }
            Controlled::Ready(Err(error)) => Controlled::Ready(Err(error)),
            Controlled::Cancel(signal) => Controlled::Cancel(signal),
            Controlled::Expired(error) => Controlled::Expired(error),
        };
        if !matches!(&outcome, Controlled::Cancel(_) | Controlled::Ready(Ok(()))) && retry_until.is_none() {
            retry_until = Some(Instant::now().checked_add(Duration::from_secs(options.reconnect_elapsed))
                .ok_or_else(|| crate::output::input_decision("pasteboard recovery deadline overflow"))?);
        }
        // No monitor, device, snapshot future or setup future remains here.
        let cancelled = matches!(&outcome, Controlled::Cancel(_));
        let cleanup_until = total.into_iter().chain((!cancelled).then_some(retry_until).flatten()).min();
        let cleanup = owner.cleanup(Duration::from_secs(options.cleanup_timeout), cleanup_until).await;
        let native_retryable = cleanup.native.as_ref().map_or(true, retryable);
        let (mut error, cancelled, timer_expired) = match outcome {
            Controlled::Ready(Err(error)) => (Some(error), false, false),
            Controlled::Ready(Ok(())) => (None, false, false),
            Controlled::Cancel(Ok(())) => (None, true, false),
            Controlled::Cancel(Err(error)) => (Some(anyhow::Error::new(error).context("waiting for Ctrl-C")), true, false),
            Controlled::Expired(error) => (Some(error), false, true),
        };
        let total_expired = total.is_some_and(|at| Instant::now() >= at);
        let recovery_expired = retry_until.is_some_and(|at| Instant::now() >= at);
        if timer_expired && (total_expired || recovery_expired) && error.is_some() {
            if let Some(previous) = failures.pop() {
                let completion = error.take().expect("completion was present");
                error = Some(combine(previous, completion));
            }
        } else if !cancelled && (total_expired || recovery_expired) {
            let at = total.into_iter().chain(retry_until).min().expect("expired completion clock");
            let completion = elapsed_at(at).await;
            error = Some(match error { Some(primary) => combine(primary, completion), None => completion });
        }
        if let Some(native) = cleanup.native {
            error = Some(match error {
                Some(operation) => combine(operation, native),
                None => native,
            });
        }
        if !cleanup.reclaimed {
            let mut error = error.expect("incomplete native cleanup has an original cause");
            if let Some(handle) = owner.retained() { error = error.context(PendingNative { _handle: handle }); }
            if let Err(output) = emit(WatchEvent::State(event("cleanup_failed", retries, Some(&error), false, None))) {
                error = combine(error, output);
            }
            return Err(terminal(failures, error));
        }
        if cancelled {
            if let Err(output) = emit(WatchEvent::State(event("stopped", retries, error.as_ref(), true, None))) {
                error = Some(match error { Some(primary) => combine(primary, output), None => output });
            }
            if let Some(error) = error { return Err(terminal(failures, error)); }
            return Ok(WatchReport { failures, retries });
        }
        let Some(error) = error else { return Ok(WatchReport { failures, retries }) };
        if let Err(output) = emit(WatchEvent::State(event("disconnected", retries, Some(&error), true, None))) {
            return Err(terminal(failures, combine(error, output)));
        }
        let until = retry_until.expect("bounded validated recovery deadline");
        if !options.reconnect || total_expired || recovery_expired || !native_retryable || !retryable(&error)
            || retries >= options.reconnect_attempts || Instant::now() >= until {
            if let Err(output) = emit(WatchEvent::State(event("exhausted", retries, Some(&error), true, None))) {
                return Err(terminal(failures, combine(error, output)));
            }
            return Err(terminal(failures, error));
        }
        let delay = options.delay(retries);
        if let Err(output) = emit(WatchEvent::State(event("backoff", retries, Some(&error), true, Some(delay)))) {
            return Err(terminal(failures, combine(error, output)));
        }
        failures.push(error);
        let until = total.map_or(until, |at| at.min(until));
        match controlled(async { tokio::time::sleep(delay).await; Ok(()) }, &mut cancel, Some(until)).await {
            Controlled::Ready(Ok(())) => retrying = true,
            Controlled::Ready(Err(error)) => return Err(terminal(failures, error)),
            Controlled::Cancel(signal) => {
                if let Err(error) = signal { return Err(terminal(failures, anyhow::Error::new(error).context("waiting for Ctrl-C"))); }
                if let Err(error) = emit(WatchEvent::State(event("stopped", retries, None, true, None))) {
                    return Err(terminal(failures, error));
                }
                return Ok(WatchReport { failures, retries });
            }
            Controlled::Expired(elapsed) => {
                let primary = failures.pop().expect("backoff retains an original failure");
                return Err(terminal(failures, combine(primary, elapsed)));
            }
        }
    }
}

// Preserve optional versus overflowing checked absolute clocks without a
// sentinel Instant or an unchecked addition.
trait OptionalClock { fn transpose_option(self) -> Option<Option<Instant>>; }
impl OptionalClock for Option<Option<Instant>> {
    fn transpose_option(self) -> Option<Option<Instant>> {
        match self { None => Some(None), Some(Some(at)) => Some(Some(at)), Some(None) => None }
    }
}

#[cfg(test)]
#[path = "reconnect_tests.rs"]
mod tests;
