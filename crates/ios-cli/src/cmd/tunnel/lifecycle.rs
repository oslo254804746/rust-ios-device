//! Private manager coordination. No network future runs under the device table lock.

use std::future::Future;
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, Result};
use tokio::sync::{watch, Mutex, OwnedSemaphorePermit, Semaphore};
use tokio::time::Instant;

pub(super) const MAX_CONNECTS: usize = 4;
pub(super) const MAX_DEVICE_GATES: usize = 4096;

#[cfg(test)]
type LifecycleCallback = Box<dyn FnOnce(&Lifecycle) + Send>;

pub(super) struct Lifecycle {
    // One namespace/ceiling for automatic gates, manual intent and stop state.
    // Idle Automatic entries are pruned exactly as expired Weak entries were;
    // published owners/tickets and persistent desired states retain their gate.
    devices: Mutex<DeviceTable>,
    metadata_bytes: Arc<Semaphore>,
    pub(super) connects: Semaphore,
    // Some is a reusable clear epoch; None is terminal server shutdown.
    shutdown: watch::Sender<Option<Arc<()>>>,
    scan: watch::Sender<Arc<()>>,
    #[cfg(test)]
    scan_after_open: std::sync::Mutex<Option<LifecycleCallback>>,
    #[cfg(test)]
    failure_before_commit: std::sync::Mutex<Option<LifecycleCallback>>,
    #[cfg(test)]
    clear_before_cleanup: std::sync::Mutex<Option<LifecycleCallback>>,
}

struct DeviceGate {
    // The original canonical key follows this already-paid gate through tickets.
    // Late success collection borrows it instead of creating another String.
    udid: String,
    serial: Mutex<()>,
    generation: watch::Sender<Arc<()>>,
    intent: super::intent::Cell,
    // The table key and this gate share one admission lifetime. A ticket may
    // retain the gate after the table disappears; that last owner pays until
    // it releases the original gate, rather than refunding at row removal.
    _metadata: OwnedSemaphorePermit,
}

type DeviceRow = Arc<DeviceGate>;

pub(super) struct DesiredRow {
    pub(super) udid: String,
    pub(super) ticket: Ticket,
    pub(super) snapshot: super::intent::Snapshot,
    // This exact copied key can move into the manual-success writer. Keep its
    // permit with that owner, independently of this listing's backing.
    pub(super) _key_storage: OwnedSemaphorePermit,
}

#[derive(Default)]
struct DeviceTable {
    // Vec exposes its actual backing capacity. HashMap capacity alone does not
    // expose bucket/control allocation bytes, so it cannot certify that charge.
    rows: Vec<DeviceRow>,
    storage: Option<OwnedSemaphorePermit>,
}

impl DeviceTable {
    fn get(&self, udid: &str) -> Option<&Arc<DeviceGate>> {
        self.rows.iter().find(|gate| gate.udid == udid)
    }

    fn reserve_row(&mut self, bank: &Arc<Semaphore>) -> Result<()> {
        if self.rows.len() < self.rows.capacity() {
            return Ok(());
        }
        let capacity = self
            .rows
            .capacity()
            .max(1)
            .saturating_mul(2)
            .min(MAX_DEVICE_GATES);
        // Both old and replacement layouts stay charged during growth.
        // Rejection happens before allocation or moving an original gate.
        let allocation = super::metadata_vec::exact_backing(bank, capacity)?;
        let storage = allocation.storage;
        let mut replacement = allocation.values;
        replacement.append(&mut self.rows);
        self.rows = replacement;
        self.storage = Some(storage);
        Ok(())
    }
}

#[derive(Clone)]
pub(super) struct Ticket {
    gate: Arc<DeviceGate>,
    generation: Arc<()>,
    shutdown: watch::Receiver<Option<Arc<()>>>,
    shutdown_generation: Arc<()>,
    intent_version: Option<Arc<()>>,
}

#[derive(Clone, Debug)]
pub(super) struct GlobalEpoch {
    changes: watch::Receiver<Option<Arc<()>>>,
    generation: Arc<()>,
}

impl GlobalEpoch {
    fn current(&self) -> bool {
        self.changes.has_changed().is_ok()
            && self
                .changes
                .borrow()
                .as_ref()
                .is_some_and(|epoch| Arc::ptr_eq(epoch, &self.generation))
    }

    pub(super) fn check(&self) -> Result<()> {
        if self.current() {
            Ok(())
        } else {
            Err(cancelled())
        }
    }

    /// Synchronous final admission only: do not await or reenter this watch.
    pub(super) fn commit<T>(&self, action: impl FnOnce() -> Result<T>) -> Result<T> {
        let current = self.changes.borrow();
        if self.changes.has_changed().is_err()
            || !current
                .as_ref()
                .is_some_and(|epoch| Arc::ptr_eq(epoch, &self.generation))
        {
            return Err(cancelled());
        }
        action()
    }

    pub(super) async fn run<T>(&self, work: impl Future<Output = Result<T>>) -> Result<T> {
        let mut changes = self.changes.clone();
        let retired = async {
            loop {
                if !self.current() || changes.changed().await.is_err() {
                    return;
                }
            }
        };
        tokio::pin!(retired);
        tokio::pin!(work);
        std::future::poll_fn(|cx| {
            if !self.current() || retired.as_mut().poll(cx).is_ready() {
                return std::task::Poll::Ready(Err(cancelled()));
            }
            // Preserve the original result if this ready poll itself crosses
            // a global retirement. Success consumers commit before handoff.
            work.as_mut().poll(cx)
        })
        .await
    }
}

pub(super) struct ScanControl {
    changes: watch::Receiver<Arc<()>>,
    pub(super) generation: Arc<()>,
    deadline: Instant,
    pub(super) epoch: super::scan::Epoch,
}

pub(super) struct ConnectionDeadline {
    deadline: Instant,
    timeout_context: &'static str,
    whole: Option<Instant>,
    retry: std::sync::Mutex<Option<Instant>>,
}

#[derive(Debug)]
struct ConnectionDeadlineReached;

impl std::fmt::Display for ConnectionDeadlineReached {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("tunnel connection deadline reached")
    }
}

impl std::error::Error for ConnectionDeadlineReached {}

impl ConnectionDeadline {
    pub(super) fn new(timeout: Duration) -> Result<Self> {
        Self::with_context(timeout, "timed out while creating tunnel")
    }

    pub(super) fn for_scan_candidate(timeout: Duration) -> Result<Self> {
        Self::with_context(timeout, "timed out establishing scanned tunnel")
    }

    pub(super) fn for_scan_bookkeeping(timeout: Duration) -> Result<Self> {
        Self::with_context(timeout, "tunnel scan failure bookkeeping total expired")
    }

    fn with_context(timeout: Duration, timeout_context: &'static str) -> Result<Self> {
        let deadline = Instant::now()
            .checked_add(timeout)
            .ok_or_else(|| crate::output::input_decision("tunnel connection deadline overflow"))?;
        Ok(Self {
            deadline,
            timeout_context,
            whole: None,
            retry: std::sync::Mutex::new(None),
        })
    }

    pub(super) fn within_scan(mut self, whole: Option<&super::scan::Deadline>) -> Self {
        self.whole = whole.map(|whole| whole.at);
        self
    }

    /// Start an episode's clock only at actual Source admission. Every run of
    /// this candidate observes the same shorter clock, including metadata waits
    /// after Source returns and a ready poll that crosses the new deadline.
    pub(super) fn clamp_retry(&self, at: Instant) {
        let mut retry = self
            .retry
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        *retry = Some(retry.map_or(at, |old| old.min(at)));
    }

    pub(super) fn absolute_at(&self) -> Instant {
        self.whole
            .map_or_else(|| self.effective(), |whole| whole.min(self.effective()))
    }

    fn effective(&self) -> Instant {
        self.retry
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
            .map_or(self.deadline, |retry| retry.min(self.deadline))
    }

    /// A synchronous fence for side effects inside a poll. Only run converts
    /// this private decision into the independent timer's original Elapsed.
    pub(super) fn check(&self) -> Result<()> {
        if self.whole.is_some_and(|whole| Instant::now() >= whole) {
            return Err(super::scan::DeadlineReached.into());
        }
        if Instant::now() >= self.effective() {
            Err(ConnectionDeadlineReached.into())
        } else {
            Ok(())
        }
    }

    pub(super) async fn run<T>(&self, work: impl Future<Output = Result<T>>) -> Result<T> {
        tokio::pin!(work);
        let mut at = self.effective();
        let mut expiry = Box::pin(tokio::time::timeout_at(at, std::future::pending::<T>()));
        std::future::poll_fn(|cx| {
            let before = self.effective();
            if before != at {
                at = before;
                expiry = Box::pin(tokio::time::timeout_at(at, std::future::pending::<T>()));
            }
            // Poll only an independent pending timer, never a late work future.
            if let std::task::Poll::Ready(expired) = expiry.as_mut().poll(cx) {
                return std::task::Poll::Ready(expired.context(self.timeout_context));
            }
            if Instant::now() >= at {
                return std::task::Poll::Pending;
            }
            let result = work.as_mut().poll(cx);
            if let std::task::Poll::Ready(Err(error)) = &result {
                if !error.is::<ConnectionDeadlineReached>() {
                    // Preserve a native cause already returned by work even if
                    // that ready error poll crossed the cooperative clock.
                    return result;
                }
            }
            let after = self.effective();
            if after != at {
                at = after;
                expiry = Box::pin(tokio::time::timeout_at(at, std::future::pending::<T>()));
            }
            // Register a newly clamped timer before returning Pending. A Source
            // may have begun the episode while this very poll was in progress.
            if let std::task::Poll::Ready(expired) = expiry.as_mut().poll(cx) {
                return std::task::Poll::Ready(expired.context(self.timeout_context));
            }
            match result {
                std::task::Poll::Ready(Err(error)) if error.is::<ConnectionDeadlineReached>() => {
                    std::task::Poll::Pending
                }
                std::task::Poll::Ready(Ok(_)) if Instant::now() >= at => std::task::Poll::Pending,
                result => result,
            }
        })
        .await
    }
}

impl Lifecycle {
    #[cfg(test)]
    pub(super) fn new() -> Self {
        Self::with_metadata(Arc::new(Semaphore::new(
            super::MAX_GLOBAL_KNOWN_METADATA_BYTES,
        )))
    }

    pub(super) fn with_metadata(metadata_bytes: Arc<Semaphore>) -> Self {
        Self {
            devices: Mutex::new(DeviceTable::default()),
            metadata_bytes,
            connects: Semaphore::new(MAX_CONNECTS),
            shutdown: watch::channel(Some(Arc::new(()))).0,
            scan: watch::channel(Arc::new(())).0,
            #[cfg(test)]
            scan_after_open: std::sync::Mutex::new(None),
            #[cfg(test)]
            failure_before_commit: std::sync::Mutex::new(None),
            #[cfg(test)]
            clear_before_cleanup: std::sync::Mutex::new(None),
        }
    }

    pub(super) fn global_epoch(&self) -> Result<GlobalEpoch> {
        let mut changes = self.shutdown.subscribe();
        let generation = changes.borrow_and_update().clone().ok_or_else(cancelled)?;
        Ok(GlobalEpoch {
            changes,
            generation,
        })
    }

    pub(super) async fn ticket(&self, udid: &str) -> Result<Ticket> {
        // Guard every caller before retaining a key or waiting on the map.
        // Accepted key capacity and gate storage also enter the shared byte
        // bank before insertion. The selector check remains a separate bound.
        super::admission::selectors(udid, None)?;
        let mut shutdown = self.shutdown.subscribe();
        let shutdown_generation = shutdown.borrow().clone().ok_or_else(cancelled)?;
        let mut devices = tokio::select! {
            biased;
            _ = shutdown.changed() => return Err(cancelled()),
            devices = self.devices.lock() => devices,
        };
        // Closing never waits for this lock, and an old queued admission must
        // not acquire a new epoch after clear or terminal shutdown.
        if !shutdown
            .borrow()
            .as_ref()
            .is_some_and(|epoch| Arc::ptr_eq(epoch, &shutdown_generation))
        {
            return Err(cancelled());
        }
        // Clear never waits on this map. Renew old desired states on the next
        // admission, retaining the canonical gate while old tickets still own
        // it. Removing/replacing those Arcs would evade the live 4096 ceiling.
        for gate in &devices.rows {
            if !gate.intent.is_epoch(&shutdown_generation) {
                gate.intent.reset(shutdown_generation.clone());
                gate.generation.send_replace(Arc::new(()));
            }
        }
        let gate = match devices.get(udid).cloned() {
            Some(gate) => gate,
            None => {
                if devices.rows.len() >= MAX_DEVICE_GATES {
                    devices
                        .rows
                        .retain(|gate| Arc::strong_count(gate) > 1 || gate.intent.persistent());
                    anyhow::ensure!(
                        devices.rows.len() < MAX_DEVICE_GATES,
                        crate::output::resource_decision("tunnel device coordinator limit")
                    );
                }
                let mut metadata = super::admission::known_metadata_charge(
                    &self.metadata_bytes,
                    std::mem::size_of::<DeviceGate>(),
                )?;
                let key_allocation =
                    super::metadata_vec::exact_backing(&self.metadata_bytes, udid.len())?;
                metadata.merge(key_allocation.storage);
                let mut key_bytes = key_allocation.values;
                key_bytes.extend_from_slice(udid.as_bytes());
                let key = String::from_utf8(key_bytes).expect("copied UDID str is valid UTF-8");
                devices.reserve_row(&self.metadata_bytes)?;
                let gate = Arc::new(DeviceGate {
                    udid: key,
                    serial: Mutex::new(()),
                    generation: watch::channel(Arc::new(())).0,
                    intent: super::intent::Cell::new(
                        shutdown_generation.clone(),
                        Arc::clone(&self.metadata_bytes),
                    ),
                    _metadata: metadata,
                });
                devices.rows.push(gate.clone());
                gate
            }
        };
        let generation = gate.generation.borrow().clone();
        Ok(Ticket {
            gate,
            generation,
            shutdown,
            shutdown_generation,
            intent_version: None,
        })
    }

    pub(super) async fn desired(&self) -> Result<super::metadata_vec::KnownVec<DesiredRow>> {
        let mut rows =
            super::metadata_vec::KnownVec::new(Arc::clone(&self.metadata_bytes), MAX_DEVICE_GATES);
        let shutdown = self.shutdown.subscribe();
        let Some(epoch) = shutdown.borrow().clone() else {
            return Ok(rows);
        };
        let devices = self.devices.lock().await;
        for gate in &devices.rows {
            let udid = &gate.udid;
            let snapshot = gate.intent.snapshot();
            if !Arc::ptr_eq(&snapshot.epoch, &epoch)
                || matches!(&snapshot.desired, super::intent::Desired::Automatic)
            {
                continue;
            }
            rows.reserve(1)?;
            let udid = udid.clone();
            let key_storage =
                super::admission::known_metadata_charge(&self.metadata_bytes, udid.capacity())?;
            rows.push(DesiredRow {
                udid,
                ticket: Ticket {
                    gate: gate.clone(),
                    generation: gate.generation.borrow().clone(),
                    shutdown: shutdown.clone(),
                    shutdown_generation: epoch.clone(),
                    intent_version: Some(snapshot.version.clone()),
                },
                snapshot,
                _key_storage: key_storage,
            })?;
        }
        Ok(rows)
    }

    /// Read CURRENT gates and one atomic desired/pending proof after Source.
    /// Every destination was allocated before Source; no key/body is cloned here.
    pub(super) async fn fill_manual_successes(
        &self,
        prepared: &mut super::manual_success::Preflight,
    ) {
        let shutdown = self.shutdown.subscribe();
        let Some(epoch) = shutdown.borrow().clone() else {
            return;
        };
        let devices = self.devices.lock().await;
        for gate in &devices.rows {
            let Some((desired, failure, authority)) = gate.intent.manual_success(&epoch) else {
                continue;
            };
            // Do not take an authority revision borrow while intent locks are held.
            if !authority.current() {
                continue;
            }
            prepared.record(
                Ticket {
                    gate: Arc::clone(gate),
                    generation: gate.generation.borrow().clone(),
                    shutdown: shutdown.clone(),
                    shutdown_generation: Arc::clone(&epoch),
                    intent_version: Some(Arc::clone(&desired)),
                },
                desired,
                failure,
                authority,
            );
        }
    }

    #[cfg(test)]
    pub(super) async fn hold_coordinator(&self) -> impl Drop + '_ {
        self.devices.lock().await
    }

    #[cfg(test)]
    pub(super) async fn coordinator_count(&self) -> usize {
        self.devices.lock().await.rows.len()
    }

    #[cfg(test)]
    pub(super) fn initial_metadata_bytes(udid: &str) -> usize {
        2 * std::mem::size_of::<DeviceRow>() + std::mem::size_of::<DeviceGate>() + udid.len()
    }

    #[cfg(test)]
    pub(super) fn additional_metadata_bytes(udid: &str) -> usize {
        std::mem::size_of::<DeviceGate>() + udid.len()
    }

    #[cfg(test)]
    pub(super) fn on_next_scan_capture(&self, callback: impl FnOnce(&Lifecycle) + Send + 'static) {
        let mut pending = self.scan_after_open.lock().unwrap();
        assert!(pending.is_none(), "scan capture hook already installed");
        *pending = Some(Box::new(callback));
    }

    #[cfg(test)]
    pub(super) fn on_next_failure_record(
        &self,
        callback: impl FnOnce(&Lifecycle) + Send + 'static,
    ) {
        let mut pending = self.failure_before_commit.lock().unwrap();
        assert!(pending.is_none(), "failure record hook already installed");
        *pending = Some(Box::new(callback));
    }

    #[cfg(test)]
    pub(super) fn before_failure_record(&self) {
        let callback = {
            let mut pending = self.failure_before_commit.lock().unwrap();
            let callback = pending.take();
            drop(pending);
            callback
        };
        if let Some(callback) = callback {
            callback(self);
        }
    }

    #[cfg(test)]
    pub(super) fn on_next_clear_cleanup(&self, callback: impl FnOnce(&Lifecycle) + Send + 'static) {
        let mut pending = self.clear_before_cleanup.lock().unwrap();
        assert!(pending.is_none(), "clear cleanup hook already installed");
        *pending = Some(Box::new(callback));
    }

    #[cfg(test)]
    pub(super) fn before_clear_cleanup(&self) {
        let callback = self.clear_before_cleanup.lock().unwrap().take();
        if let Some(callback) = callback {
            callback(self);
        }
    }

    // The caller holds the tunnel table write guard. This fences publication
    // together with removal without waiting for a connecting device's serial lock.
    #[cfg(test)]
    pub(super) async fn stop(&self, udid: &str) {
        let _ = self.stop_owned(udid).await;
    }

    pub(super) async fn stop_owned(&self, udid: &str) -> Option<Arc<()>> {
        let gate = {
            let devices = self.devices.lock().await;
            devices.get(udid).cloned()
        };
        let previous = gate.map(|gate| {
            gate.intent.replace(super::intent::Desired::Stopped);
            gate.generation.send_replace(Arc::new(()))
        });
        self.scan.send_replace(Arc::new(()));
        previous
    }

    pub(super) fn clear(&self) {
        let _ = self.clear_owned();
    }

    pub(super) fn clear_owned(&self) -> Option<Arc<()>> {
        let mut previous = None;
        self.shutdown.send_if_modified(|epoch| {
            let Some(current) = epoch else { return false };
            previous = Some(Arc::clone(current));
            *epoch = Some(Arc::new(()));
            true
        });
        self.scan.send_replace(Arc::new(()));
        previous
    }

    pub(super) fn take_stale_failures(
        &self,
        failed: &mut super::failure_table::Table,
    ) -> super::failure_table::RemovedBatch {
        // Caller holds only the failure writer. Selection/removal is synchronous
        // and returned payloads outlive both this watch read and the table guard.
        let current = self.shutdown.borrow();
        failed.take_other_globals(current.as_ref())
    }

    pub(super) fn close(&self) {
        self.shutdown.send_replace(None);
        self.scan.send_replace(Arc::new(()));
    }

    pub(super) fn scan_control(&self, timeout: Duration) -> Result<ScanControl> {
        if self.shutdown.borrow().is_none() {
            return Err(cancelled());
        }
        let deadline = Instant::now().checked_add(timeout).ok_or_else(|| {
            crate::output::input_decision("tunnel scan discovery deadline overflow")
        })?;
        #[cfg(test)]
        {
            // Force one admission boundary on this instance, without retaining
            // any watch Ref or hook mutex while invoking close/clear.
            let callback = {
                let mut pending = self.scan_after_open.lock().unwrap();
                let callback = pending.take();
                drop(pending);
                callback
            };
            if let Some(callback) = callback {
                callback(self);
            }
        }
        let mut changes = self.scan.subscribe();
        let generation = changes.borrow_and_update().clone();
        // A completed close may already be marked seen by this subscription.
        // Terminal shutdown is monotonic, so reject it after capturing the epoch.
        let mut global = self.shutdown.subscribe();
        let global_generation = global.borrow_and_update().clone().ok_or_else(cancelled)?;
        Ok(ScanControl {
            changes,
            generation,
            deadline,
            epoch: super::scan::Epoch {
                changes: global,
                generation: global_generation,
            },
        })
    }

    pub(super) fn scan_is_current(&self, generation: &Arc<()>) -> bool {
        self.shutdown.borrow().is_some() && Arc::ptr_eq(&self.scan.borrow(), generation)
    }
}

impl ScanControl {
    pub(super) async fn drive<T>(&mut self, work: impl Future<Output = T>) -> Result<Option<T>> {
        if !Arc::ptr_eq(&self.changes.borrow(), &self.generation) {
            return Ok(None);
        }
        let deadline = self.deadline;
        tokio::pin!(work);
        tokio::select! {
            biased;
            _ = self.changes.changed() => Ok(None),
            expired = tokio::time::timeout_at(deadline, std::future::pending::<T>()) => {
                expired.map(Some).context("tunnel scan discovery total expired")
            }
            result = std::future::poll_fn(|cx| {
                // Suppress late ready locks and tickets on every poll, even
                // before the independently polled native timer becomes ready.
                if Instant::now() >= deadline {
                    std::task::Poll::Pending
                } else {
                    work.as_mut().poll(cx)
                }
            }) => Ok(Some(result)),
        }
    }
}

impl Ticket {
    /// A published transport belongs to its original device/global epochs.
    /// Desired-version changes can keep that same runtime alive (live manual
    /// shortcut); they must not revoke its existing data connections.
    pub(super) fn for_connection(&self) -> Self {
        let mut connection = self.clone();
        connection.intent_version = None;
        connection
    }

    pub(super) fn udid(&self) -> &str {
        &self.gate.udid
    }

    pub(super) fn fenced(&self) -> Self {
        let mut ticket = self.clone();
        ticket.intent_version = Some(self.gate.intent.snapshot().version);
        ticket
    }

    /// Preserve an existing fence; otherwise bind the observation to the exact
    /// captured desired version, without reading or reviving a later selector.
    pub(super) fn observe_version(&self, version: &Arc<()>) -> Self {
        let mut ticket = self.clone();
        ticket.intent_version = Some(
            self.intent_version
                .clone()
                .unwrap_or_else(|| Arc::clone(version)),
        );
        ticket
    }

    pub(super) fn desired(&self) -> super::intent::Snapshot {
        self.gate.intent.snapshot()
    }

    // Only an unfenced explicit start establishes a new desired selector. The
    // caller already owns the tunnel table and both ticket epoch guards.
    #[cfg(test)]
    pub(super) fn establish_manual(
        &self,
        route: super::intent::RouteIntent,
        authority: crate::access::DeviceAuthority,
        failure: Option<Arc<()>>,
        deadline: Option<&ConnectionDeadline>,
    ) -> Result<()> {
        self.establish_manual_with(
            || self.gate.intent.manual(route, authority.clone()),
            &authority,
            failure,
            deadline,
        )
    }

    pub(super) fn establish_manual_shared(
        &self,
        route: super::intent::RouteOwner,
        authority: crate::access::DeviceAuthority,
        failure: Option<Arc<()>>,
        deadline: Option<&ConnectionDeadline>,
    ) -> Result<()> {
        self.establish_manual_with(
            || self.gate.intent.manual_shared(route, authority.clone()),
            &authority,
            failure,
            deadline,
        )
    }

    fn establish_manual_with(
        &self,
        desired: impl FnOnce() -> Result<super::intent::Desired>,
        authority: &crate::access::DeviceAuthority,
        failure: Option<Arc<()>>,
        deadline: Option<&ConnectionDeadline>,
    ) -> Result<()> {
        debug_assert!(self.intent_version.is_none());
        self.gate.intent.checked_replace(desired, failure, || {
            if let Some(deadline) = deadline {
                deadline.check()?;
            }
            authority.check_clock()
        })?;
        Ok(())
    }

    #[cfg(test)]
    pub(super) fn on_next_manual_establish(
        &self,
        after_candidate: bool,
        callback: impl FnOnce() + Send + 'static,
    ) {
        self.gate
            .intent
            .on_next_checked_replace(after_candidate, callback);
    }

    #[cfg(test)]
    pub(super) fn hold_manual_intent(&self, pending: bool, action: impl FnOnce()) {
        self.gate.intent.hold_replace_lock(pending, action);
    }

    #[cfg(test)]
    pub(super) fn subscribe_intent(&self) -> watch::Receiver<Arc<()>> {
        self.gate.intent.subscribe()
    }

    pub(super) fn failure_version(&self) -> Option<Arc<()>> {
        self.gate.intent.failure().map(|(row, _)| row)
    }

    pub(super) fn failure_stamp(&self) -> Option<(Arc<()>, Arc<()>)> {
        self.gate.intent.failure()
    }

    pub(super) fn restore_failure_stamp(&self, stamp: Option<(Arc<()>, Arc<()>)>) {
        self.gate.intent.set_failure(stamp);
    }

    pub(super) fn clear_failure_version(&self, version: &Arc<()>) {
        let stamp = self.failure_stamp();
        if stamp
            .as_ref()
            .is_some_and(|(row, _)| Arc::ptr_eq(row, version))
        {
            self.restore_failure_stamp(None);
        }
    }

    // Called under the original failure-table writer/commit, including rollback.
    pub(super) fn remember_failure(&self, version: Option<Arc<()>>) {
        let desired = self
            .intent_version
            .clone()
            .unwrap_or_else(|| self.desired().version);
        self.gate
            .intent
            .set_failure(version.map(|version| (version, desired)));
    }

    pub(super) fn failure_is_current(&self, version: &Arc<()>) -> bool {
        let desired = self.desired();
        matches!(desired.desired, super::intent::Desired::Automatic)
            || self.gate.intent.failure().is_some_and(|(row, intent)| {
                Arc::ptr_eq(&row, version) && Arc::ptr_eq(&intent, &desired.version)
            })
    }

    pub(super) fn pending_success(&self) -> Option<(Arc<()>, super::bookkeeping::DeferredSuccess)> {
        self.gate
            .intent
            .pending_success()
            .filter(|(desired, _)| self.gate.intent.matches(desired))
            .map(|(desired, version)| (desired, super::bookkeeping::DeferredSuccess { version }))
    }

    pub(super) fn clear_success(&self, desired: &Arc<()>, failure: &Arc<()>) {
        self.gate.intent.clear_success(desired, failure);
    }

    pub(super) fn clear_success_checked(
        &self,
        desired: &Arc<()>,
        failure: &Arc<()>,
        check: impl Fn() -> Result<()>,
    ) -> Result<()> {
        self.gate
            .intent
            .clear_success_checked(desired, failure, check)
    }

    /// Compare immutable identities without borrowing either watch again from
    /// inside a commit. A ledger row holds this coordinator across scan windows.
    pub(super) fn same_incarnation(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.gate, &other.gate)
            && Arc::ptr_eq(&self.generation, &other.generation)
            && Arc::ptr_eq(&self.shutdown_generation, &other.shutdown_generation)
    }

    pub(super) fn global_epoch(&self) -> Arc<()> {
        Arc::clone(&self.shutdown_generation)
    }

    pub(super) fn belongs_to_global(&self, old: &Arc<()>) -> bool {
        Arc::ptr_eq(&self.shutdown_generation, old)
    }

    pub(super) fn belongs_to_device(&self, old: &Arc<()>) -> bool {
        Arc::ptr_eq(&self.generation, old)
    }

    pub(super) fn ownership_stamp(&self) -> super::retirement::OwnershipStamp {
        super::retirement::OwnershipStamp::new(
            Arc::clone(&self.shutdown_generation),
            Arc::clone(&self.generation),
        )
    }

    pub(super) fn is_current(&self) -> bool {
        self.intent_version
            .as_ref()
            .map_or(true, |version| self.gate.intent.matches(version))
            && Arc::ptr_eq(&self.gate.generation.borrow(), &self.generation)
            && self
                .shutdown
                .borrow()
                .as_ref()
                .is_some_and(|epoch| Arc::ptr_eq(epoch, &self.shutdown_generation))
    }

    // The caller already owns its destination table guard. Keep both epoch
    // borrows through the synchronous mutation so stop/clear/close either win
    // before admission or wait until this commit finishes. The action must not
    // await or reenter these watches, including through is_current().
    pub(super) fn commit<T>(&self, action: impl FnOnce() -> Result<T>) -> Result<T> {
        let generation = self.gate.generation.borrow();
        let shutdown = self.shutdown.borrow();
        if !Arc::ptr_eq(&generation, &self.generation)
            || !shutdown
                .as_ref()
                .is_some_and(|epoch| Arc::ptr_eq(epoch, &self.shutdown_generation))
        {
            return Err(cancelled());
        }
        let intent = self
            .intent_version
            .as_ref()
            .map(|_| self.gate.intent.lock());
        if self
            .intent_version
            .as_ref()
            .zip(intent.as_ref())
            .is_some_and(|(expected, current)| !Arc::ptr_eq(expected, &current.version))
        {
            return Err(cancelled());
        }
        let result = action();
        drop(intent);
        drop(shutdown);
        drop(generation);
        result
    }

    pub(super) async fn serial(&self) -> tokio::sync::MutexGuard<'_, ()> {
        self.gate.serial.lock().await
    }

    pub(super) async fn run<T>(&self, work: impl Future<Output = Result<T>>) -> Result<T> {
        let mut device = self.gate.generation.subscribe();
        let mut shutdown = self.shutdown.clone();
        if !self.is_current() {
            return Err(cancelled());
        }
        tokio::select! {
            biased;
            _ = device.changed() => Err(cancelled()),
            _ = shutdown.changed() => Err(cancelled()),
            _ = async {
                if let Some(version) = &self.intent_version {
                    self.gate.intent.changed(version).await;
                } else {
                    std::future::pending::<()>().await;
                }
            } => Err(cancelled()),
            result = work => result,
        }
    }
}

pub(super) fn cancelled() -> anyhow::Error {
    anyhow::Error::new(ios_core::OperationCancelled::new()).context("tunnel operation cancelled")
}

#[cfg(test)]
#[path = "lifecycle/desired_tests.rs"]
mod desired_tests;

#[cfg(test)]
#[path = "lifecycle/global_epoch_tests.rs"]
mod global_epoch_tests;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn coordinator_vector_refusal_precedes_allocation_without_changing_backing() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        let bank = Arc::new(Semaphore::new(2 * std::mem::size_of::<DeviceRow>() - 1));
        let mut table = DeviceTable::default();
        let allocated = Arc::new(AtomicUsize::new(0));
        let seen = Arc::clone(&allocated);
        super::super::metadata_vec::with_allocation_hook(
            move |_, _| {
                seen.fetch_add(1, Ordering::SeqCst);
            },
            || {
                assert!(table.reserve_row(&bank).is_err());
            },
        );
        assert_eq!(allocated.load(Ordering::SeqCst), 0);
        assert_eq!(table.rows.capacity(), 0);
        assert!(table.storage.is_none());
        assert_eq!(
            bank.available_permits(),
            2 * std::mem::size_of::<DeviceRow>() - 1
        );
    }

    #[tokio::test]
    async fn coordinator_canonical_key_is_admitted_before_its_first_copy() {
        use futures_util::FutureExt;
        use std::sync::atomic::{AtomicUsize, Ordering};
        let bytes = std::mem::size_of::<DeviceGate>();
        let bank = Arc::new(Semaphore::new(bytes));
        let lifecycle = Lifecycle::with_metadata(Arc::clone(&bank));
        let allocated = Arc::new(AtomicUsize::new(0));
        let seen = Arc::clone(&allocated);
        let result = super::super::metadata_vec::with_allocation_hook(
            move |_, _| {
                seen.fetch_add(1, Ordering::SeqCst);
            },
            || lifecycle.ticket("a").now_or_never().unwrap(),
        );
        assert!(result.is_err());
        assert_eq!(allocated.load(Ordering::SeqCst), 0);
        assert!(lifecycle.devices.lock().await.rows.is_empty());
        assert_eq!(bank.available_permits(), bytes);
    }

    fn resource(error: &anyhow::Error) {
        assert_eq!(
            crate::output::operation_error_class(error.as_ref()),
            ios_core::ErrorClass::ResourceLimit
        );
    }

    #[tokio::test]
    async fn coordinator_metadata_exact_capacity_shared_tickets_and_last_owner_release() {
        let key = "synthetic-byte-owner";
        let bytes = Lifecycle::initial_metadata_bytes(key);
        let bank = Arc::new(Semaphore::new(bytes));
        let lifecycle = Lifecycle::with_metadata(Arc::clone(&bank));
        let original = lifecycle.ticket(key).await.unwrap();
        assert_eq!(bank.available_permits(), 0);
        let repeated = lifecycle.ticket(key).await.unwrap();
        assert!(original.same_incarnation(&repeated));
        assert_eq!(
            bank.available_permits(),
            0,
            "same gate must not double charge"
        );
        resource(&lifecycle.ticket("another").await.err().unwrap());
        assert_eq!(lifecycle.coordinator_count().await, 1);
        assert!(original.is_current());
        lifecycle.clear();
        assert!(!original.is_current());
        assert_eq!(
            bank.available_permits(),
            0,
            "epoch invalidation does not free storage"
        );
        let clone = original.clone();
        drop(lifecycle);
        let backing = 2 * std::mem::size_of::<DeviceRow>();
        assert_eq!(
            bank.available_permits(),
            backing,
            "table backing freed, original gate still owned"
        );
        drop(original);
        drop(repeated);
        assert_eq!(bank.available_permits(), backing);
        drop(clone);
        assert_eq!(bank.available_permits(), bytes);
    }

    #[tokio::test]
    async fn coordinator_metadata_failed_backing_admission_rolls_back_original_storage() {
        let key = "synthetic-backing";
        let bytes = Lifecycle::initial_metadata_bytes(key) - 1;
        let bank = Arc::new(Semaphore::new(bytes));
        let lifecycle = Lifecycle::with_metadata(Arc::clone(&bank));
        resource(&lifecycle.ticket(key).await.err().unwrap());
        let devices = lifecycle.devices.lock().await;
        assert!(devices.rows.is_empty());
        assert_eq!(devices.rows.capacity(), 0);
        assert!(devices.storage.is_none());
        drop(devices);
        assert_eq!(
            bank.available_permits(),
            bytes,
            "failed key and backing charges returned"
        );
        drop(lifecycle);
        assert_eq!(bank.available_permits(), bytes);
    }

    #[tokio::test]
    async fn coordinator_metadata_rejected_growth_preserves_original_gates_and_capacity() {
        let key_bytes = std::mem::size_of::<DeviceGate>() + 1;
        let backing = 2 * std::mem::size_of::<DeviceRow>();
        // Two original rows plus a third gate fit, but old+new backing do not.
        let bytes = 3 * key_bytes + backing;
        let bank = Arc::new(Semaphore::new(bytes));
        let lifecycle = Lifecycle::with_metadata(Arc::clone(&bank));
        let first = lifecycle.ticket("a").await.unwrap();
        let second = lifecycle.ticket("b").await.unwrap();
        assert_eq!(bank.available_permits(), key_bytes);
        resource(&lifecycle.ticket("c").await.err().unwrap());
        assert_eq!(bank.available_permits(), key_bytes);
        assert_eq!(lifecycle.coordinator_count().await, 2);
        assert_eq!(lifecycle.devices.lock().await.rows.capacity(), 2);
        let same = lifecycle.ticket("a").await.unwrap();
        assert!(same.same_incarnation(&first));
        assert!(second.is_current());
        drop(lifecycle);
        assert_eq!(bank.available_permits(), key_bytes + backing);
        drop(first);
        drop(second);
        drop(same);
        assert_eq!(bank.available_permits(), bytes);
    }

    #[tokio::test]
    async fn coordinator_metadata_growth_charges_actual_vec_capacity_and_releases_old_backing() {
        let bank = Arc::new(Semaphore::new(64 * 1024));
        let lifecycle = Lifecycle::with_metadata(Arc::clone(&bank));
        let mut tickets = Vec::new();
        for key in ["a", "b", "c", "d", "e"] {
            tickets.push(lifecycle.ticket(key).await.unwrap());
            let devices = lifecycle.devices.lock().await;
            let actual = devices.rows.capacity() * std::mem::size_of::<DeviceRow>()
                + devices
                    .rows
                    .iter()
                    .map(|gate| gate.udid.capacity() + std::mem::size_of::<DeviceGate>())
                    .sum::<usize>();
            assert_eq!(bank.available_permits(), 64 * 1024 - actual);
        }
        drop(lifecycle);
        assert_eq!(
            bank.available_permits(),
            64 * 1024 - 5 * (std::mem::size_of::<DeviceGate>() + 1)
        );
        drop(tickets);
        assert_eq!(bank.available_permits(), 64 * 1024);
    }

    #[tokio::test]
    async fn global_clear_resets_desired_intent_and_invalidates_old_ticket() {
        let lifecycle = Lifecycle::new();
        let old = lifecycle.ticket("synthetic-stopped").await.unwrap();
        old.gate
            .intent
            .replace(super::super::intent::Desired::Stopped);
        let before = old.gate.intent.snapshot();
        lifecycle.clear();
        assert!(!old.is_current());
        let desired = lifecycle.desired().await.unwrap();
        assert_eq!(
            desired.len(),
            0,
            "global clear must fence old desired snapshots"
        );
        let fresh = lifecycle.ticket("synthetic-stopped").await.unwrap();
        assert!(fresh.is_current());
        let after = fresh.gate.intent.snapshot();
        assert!(matches!(
            after.desired,
            super::super::intent::Desired::Automatic
        ));
        assert!(!Arc::ptr_eq(&before.epoch, &after.epoch));
        assert!(!Arc::ptr_eq(&before.version, &after.version));
        assert!(Arc::ptr_eq(&old.gate, &fresh.gate));
        lifecycle.close();
        assert!(lifecycle.ticket("synthetic-stopped").await.is_err());
    }

    #[tokio::test]
    async fn lifecycle_tunnel_coordinator_cancel_returns_before_held_lock_release() {
        for terminal in [false, true] {
            let lifecycle = Lifecycle::new();
            let held = lifecycle.devices.lock().await;
            let mut queued = Box::pin(lifecycle.ticket("synthetic-waiting-device"));
            assert!(futures_util::poll!(&mut queued).is_pending());
            if terminal {
                lifecycle.close();
            } else {
                lifecycle.clear();
            }
            let outcome = tokio::time::timeout(Duration::from_secs(1), queued).await;
            assert!(
                held.rows.is_empty(),
                "cancelled admission created coordinator state"
            );
            drop(held);
            match outcome.expect("cancellation waited for the held coordinator lock") {
                Err(error) => {
                    assert!(error.is::<ios_core::OperationCancelled>());
                    assert_eq!(
                        ios_core::classify_error(error.as_ref()),
                        ios_core::ErrorClass::Cancelled
                    );
                }
                Ok(_) => panic!("old admission resumed after cancellation"),
            }
            if !terminal {
                assert!(lifecycle.ticket("synthetic-waiting-device").await.is_ok());
            }
        }
    }

    #[tokio::test]
    async fn lifecycle_tunnel_shutdown_admission_coordinator_wait_keeps_original_epoch() {
        for terminal in [false, true] {
            let lifecycle = Lifecycle::new();
            let held = lifecycle.devices.lock().await;
            let mut queued = Box::pin(lifecycle.ticket("synthetic-queued-device"));
            assert!(futures_util::poll!(&mut queued).is_pending());
            if terminal {
                lifecycle.close();
            } else {
                lifecycle.clear();
            }
            drop(held);
            let outcome = tokio::time::timeout(Duration::from_secs(1), queued)
                .await
                .unwrap();
            match outcome {
                Err(error) => assert!(error.is::<ios_core::OperationCancelled>()),
                Ok(_) => panic!("queued admission acquired a replacement epoch"),
            }
            assert!(lifecycle.devices.lock().await.rows.is_empty());
            if !terminal {
                assert!(lifecycle.ticket("synthetic-queued-device").await.is_ok());
            }
        }
    }
}
