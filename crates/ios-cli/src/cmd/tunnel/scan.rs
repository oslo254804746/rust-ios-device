//! Whole-scan control and bounded ownership of errors acquired before cancellation.
use std::error::Error;
use std::future::Future;
use std::sync::{Arc, Mutex as StdMutex, OnceLock};
use std::time::Duration;

use anyhow::{Context, Result};
use tokio::sync::{watch, MutexGuard, OwnedSemaphorePermit, Semaphore};
use tokio::time::Instant;

pub(super) const MAX_ABANDONED_SCANS: usize = 8;

#[cfg(test)]
#[path = "scan/capsule_tests.rs"]
mod capsule_tests;

enum Slot {
    Empty,
    Active,
    Abandoned(Arc<AbandonedScan>),
}

struct Slots {
    closed: bool,
    entries: [Slot; MAX_ABANDONED_SCANS],
    history_root: Option<Arc<HistorySnapshot>>,
}

struct HistoryRoots {
    entries: [Option<Arc<AbandonedScan>>; MAX_ABANDONED_SCANS],
    len: usize,
}

/// Immutable snapshot payload paid and created before its scan polls Source.
/// The inline roots have no separate Vec allocation. Arc/anyhow private headers
/// and the separate terminal report wrapper remain outside this body grant.
pub(super) struct HistorySnapshot {
    roots: OnceLock<HistoryRoots>,
    // The original roots drop before this application-owned body grant.
    _storage: OwnedSemaphorePermit,
}

impl HistorySnapshot {
    fn freeze(&self, slots: &[Slot; MAX_ABANDONED_SCANS]) {
        let mut roots = HistoryRoots {
            entries: std::array::from_fn(|_| None),
            len: 0,
        };
        for slot in slots {
            if let Slot::Abandoned(original) = slot {
                roots.entries[roots.len] = Some(Arc::clone(original));
                roots.len += 1;
            }
        }
        assert!(roots.len != 0, "only abandoned scans publish history");
        assert!(self.roots.set(roots).is_ok(), "scan history freezes once");
    }

    fn roots(&self) -> &HistoryRoots {
        self.roots.get().expect("history published after freeze")
    }

    pub(super) fn len(&self) -> usize {
        self.roots().len
    }

    pub(super) fn first(&self) -> Option<&Arc<AbandonedScan>> {
        self.iter().next()
    }

    pub(super) fn iter(&self) -> impl Iterator<Item = &Arc<AbandonedScan>> {
        let roots = self.roots();
        roots.entries[..roots.len]
            .iter()
            .map(|root| root.as_ref().expect("packed scan history root"))
    }
}

#[cfg(test)]
impl std::ops::Index<usize> for HistorySnapshot {
    type Output = Arc<AbandonedScan>;

    fn index(&self, index: usize) -> &Self::Output {
        let roots = self.roots();
        roots.entries[..roots.len][index]
            .as_ref()
            .expect("packed scan history root")
    }
}

pub(super) struct Outcomes {
    slots: StdMutex<Slots>,
    bank: Arc<Semaphore>,
}

fn compound_bytes() -> usize {
    std::mem::size_of::<SourceContext>()
        .max(std::mem::size_of::<super::bookkeeping::FailureContext>())
}

pub(super) fn capsule_reservation_bytes() -> usize {
    compound_bytes()
        + std::mem::size_of::<AbandonedScan>()
        + std::mem::size_of::<super::ScanCleanupFailure>()
        + std::mem::size_of::<HistorySnapshot>()
}

impl Outcomes {
    #[cfg(test)]
    pub(super) fn new() -> Arc<Self> {
        Self::with_metadata(Arc::new(Semaphore::new(
            super::MAX_GLOBAL_KNOWN_METADATA_BYTES,
        )))
    }

    pub(super) fn with_metadata(bank: Arc<Semaphore>) -> Arc<Self> {
        Arc::new(Self {
            slots: StdMutex::new(Slots {
                closed: false,
                entries: std::array::from_fn(|_| Slot::Empty),
                history_root: None,
            }),
            bank,
        })
    }

    pub(super) fn begin<'a>(self: &Arc<Self>, lock: MutexGuard<'a, ()>) -> Result<Session<'a>> {
        let mut slots = self.slots.lock().unwrap_or_else(|p| p.into_inner());
        if slots.closed {
            return Err(super::lifecycle::cancelled());
        }
        let index = slots
            .entries
            .iter()
            .position(|slot| matches!(slot, Slot::Empty))
            .ok_or_else(|| {
                crate::output::resource_decision("tunnel abandoned scan error capacity")
            })?;
        // One atomic prospective reservation, before any original Source can
        // return an error. No later quota refusal may discard that original.
        let mut storage =
            super::admission::known_metadata_charge(&self.bank, capsule_reservation_bytes())?;
        let compound = storage
            .split(compound_bytes())
            .expect("reserved compound bytes");
        let cleanup = storage
            .split(std::mem::size_of::<super::ScanCleanupFailure>())
            .expect("reserved cleanup bytes");
        let snapshot_storage = storage
            .split(std::mem::size_of::<HistorySnapshot>())
            .expect("reserved immutable history snapshot body");
        let history_snapshot = Arc::new(HistorySnapshot {
            roots: OnceLock::new(),
            _storage: snapshot_storage,
        });
        slots.entries[index] = Slot::Active;
        Ok(Session {
            outcomes: Arc::clone(self),
            index,
            acquired: StdMutex::new(Acquired::default()),
            writes: StdMutex::new(None),
            compound: StdMutex::new(Some(compound)),
            cleanup: StdMutex::new(Some(cleanup)),
            abandoned: Some(storage),
            // Separate from AbandonedScan: a snapshot must not own itself.
            history_snapshot: Some(history_snapshot),
            _lock: lock,
        })
    }

    pub(super) fn close(&self) {
        self.slots.lock().unwrap_or_else(|p| p.into_inner()).closed = true;
    }

    pub(super) fn history(&self) -> Option<Arc<HistorySnapshot>> {
        self.slots
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .history_root
            .as_ref()
            .map(Arc::clone)
    }

    pub(super) fn payload_history(
        &self,
    ) -> (
        usize,
        Result<super::metadata_vec::KnownVec<Arc<AbandonedScan>>>,
    ) {
        let slots = self.slots.lock().unwrap_or_else(|p| p.into_inner());
        let active = slots
            .entries
            .iter()
            .filter(|slot| matches!(slot, Slot::Active))
            .count();
        let history = (|| {
            let count = slots.history_root.as_ref().map_or(0, |root| root.len());
            let mut roots =
                super::metadata_vec::KnownVec::new(Arc::clone(&self.bank), MAX_ABANDONED_SCANS);
            // Admit output backing before cloning any original roots.
            roots.reserve(count)?;
            if let Some(snapshot) = &slots.history_root {
                for original in snapshot.iter() {
                    roots.push_reserved(Arc::clone(original));
                }
            }
            Ok(roots)
        })();
        (active, history)
    }
}

pub(super) struct Session<'a> {
    outcomes: Arc<Outcomes>,
    index: usize,
    acquired: StdMutex<Acquired>,
    writes: StdMutex<Option<Arc<super::bookkeeping::FailureWriteOwner>>>,
    // Stack-held prospective permits are not a claim about stack/heap size.
    // Only actually materialized app-owned payloads receive their exact slice.
    compound: StdMutex<Option<OwnedSemaphorePermit>>,
    cleanup: StdMutex<Option<OwnedSemaphorePermit>>,
    abandoned: Option<OwnedSemaphorePermit>,
    history_snapshot: Option<Arc<HistorySnapshot>>,
    // Drop records the original error before this guard admits another scanner.
    _lock: MutexGuard<'a, ()>,
}

#[derive(Clone, Copy)]
pub(super) enum Source {
    Usb,
    Paired,
    Coordinator,
    Freshness,
}

/// Fixed source slots: repeated unknown keys never extend an error chain.
#[derive(Default)]
struct Acquired {
    usb: Option<anyhow::Error>,
    paired: Option<anyhow::Error>,
    coordinator: Option<anyhow::Error>,
    freshness: Option<anyhow::Error>,
}

impl Acquired {
    fn is_empty(&self) -> bool {
        self.usb.is_none()
            && self.paired.is_none()
            && self.coordinator.is_none()
            && self.freshness.is_none()
    }

    fn retain(&mut self, source: Source, error: anyhow::Error) {
        let slot = match source {
            Source::Usb => &mut self.usb,
            Source::Paired => &mut self.paired,
            Source::Coordinator => &mut self.coordinator,
            Source::Freshness => &mut self.freshness,
        };
        if slot.is_none() {
            *slot = Some(error);
        }
    }

    fn finish(
        mut self,
        mut completion: Option<anyhow::Error>,
        storage: &StdMutex<Option<OwnedSemaphorePermit>>,
    ) -> Option<anyhow::Error> {
        let primary = self
            .usb
            .take()
            .or_else(|| self.paired.take())
            .or_else(|| self.coordinator.take())
            .or_else(|| self.freshness.take())
            .or_else(|| completion.take())?;
        if self.is_empty() {
            // Keep the existing one-source completion context and its typed
            // secondary accessor for callers already inspecting that contract.
            return Some(match completion {
                Some(error) => primary.context(super::bookkeeping::FailureContext::admitted(
                    error,
                    materialize::<super::bookkeeping::FailureContext>(storage),
                )),
                None => primary,
            });
        }
        Some(primary.context(SourceContext {
            usb: self.usb,
            paired: self.paired,
            coordinator: self.coordinator,
            freshness: self.freshness,
            completion,
            _storage: materialize::<SourceContext>(storage),
        }))
    }
}

/// The primary remains the original anyhow object. Independent causes are
/// owned by one safe typed context, without exposing their arbitrary strings.
pub(super) struct SourceContext {
    usb: Option<anyhow::Error>,
    paired: Option<anyhow::Error>,
    coordinator: Option<anyhow::Error>,
    freshness: Option<anyhow::Error>,
    completion: Option<anyhow::Error>,
    _storage: OwnedSemaphorePermit,
}

fn materialize<T>(storage: &StdMutex<Option<OwnedSemaphorePermit>>) -> OwnedSemaphorePermit {
    let mut reserved = storage
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .take()
        .expect("one terminal compound context per admitted scan");
    // The unused maximum is refunded before constructing the chosen payload.
    reserved
        .split(std::mem::size_of::<T>())
        .expect("reserved exact context payload")
}

impl SourceContext {
    pub(super) fn secondary(&self, source: Source) -> Option<&anyhow::Error> {
        match source {
            Source::Usb => self.usb.as_ref(),
            Source::Paired => self.paired.as_ref(),
            Source::Coordinator => self.coordinator.as_ref(),
            Source::Freshness => self.freshness.as_ref(),
        }
    }

    pub(super) fn completion(&self) -> Option<&anyhow::Error> {
        self.completion.as_ref()
    }
}

impl std::fmt::Display for SourceContext {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("tunnel scan has additional source or completion failures")
    }
}

impl std::fmt::Debug for SourceContext {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let class = |error: &Option<anyhow::Error>| {
            error
                .as_ref()
                .map(|error| crate::output::operation_error_class(error.as_ref()))
        };
        f.debug_struct("TunnelScanSources")
            .field("usb_class", &class(&self.usb))
            .field("paired_class", &class(&self.paired))
            .field("coordinator_class", &class(&self.coordinator))
            .field("freshness_class", &class(&self.freshness))
            .field("completion_class", &class(&self.completion))
            .finish()
    }
}

impl Session<'_> {
    pub(super) fn retain_writes(
        &self,
        owner: Arc<super::bookkeeping::FailureWriteOwner>,
    ) -> Result<()> {
        let mut slot = self.writes.lock().unwrap_or_else(|p| p.into_inner());
        anyhow::ensure!(
            slot.is_none(),
            crate::output::resource_decision("tunnel scan write owner already registered")
        );
        owner.in_scan();
        *slot = Some(owner);
        Ok(())
    }
    pub(super) fn has_primary(&self) -> bool {
        !self
            .acquired
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .is_empty()
    }
    pub(super) fn retain(&self, source: Source, error: anyhow::Error) {
        self.acquired
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .retain(source, error);
    }

    pub(super) fn take(&self) -> Option<anyhow::Error> {
        // Terminal extraction: production does not refill Source slots after
        // this point. A subsequent finish only returns its completion intact.
        self.take_acquired().finish(None, &self.compound)
    }

    pub(super) fn cleanup_storage(&self) -> OwnedSemaphorePermit {
        self.cleanup
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .take()
            .expect("one original list cleanup payload per scan")
    }

    fn take_acquired(&self) -> Acquired {
        std::mem::take(&mut *self.acquired.lock().unwrap_or_else(|p| p.into_inner()))
    }

    pub(super) fn finish(self, result: Result<Option<()>>) -> Result<()> {
        let acquired = self.take_acquired();
        let completion = match result {
            Err(error) => Some(error),
            Ok(None) if !acquired.is_empty() => Some(super::lifecycle::cancelled()),
            Ok(_) => None,
        };
        let result = match acquired.finish(completion, &self.compound) {
            Some(error) => Err(error),
            None => Ok(()),
        };
        let owner = self.writes.lock().unwrap_or_else(|p| p.into_inner()).take();
        match owner {
            Some(owner) => owner.finish_scan(result),
            None => result,
        }
    }
}

impl Drop for Session<'_> {
    fn drop(&mut self) {
        let primary = self.take();
        let owner = self.writes.lock().unwrap_or_else(|p| p.into_inner()).take();
        let primary = match owner {
            Some(owner) => owner.abandon(primary),
            None => primary,
        };
        let mut slots = self
            .outcomes
            .slots
            .lock()
            .unwrap_or_else(|p| p.into_inner());
        let displaced = match primary {
            Some(primary) => {
                slots.entries[self.index] = Slot::Abandoned(Arc::new(AbandonedScan {
                    original: primary,
                    _storage: self
                        .abandoned
                        .take()
                        .expect("reserved abandoned scan payload"),
                }));
                let snapshot = self
                    .history_snapshot
                    .take()
                    .expect("one prepaid history snapshot per admitted scan");
                snapshot.freeze(&slots.entries);
                slots.history_root.replace(snapshot)
            }
            None => {
                slots.entries[self.index] = Slot::Empty;
                None
            }
        };
        drop(slots);
        // Displaced roots and their body grant outlive the state guard.
        drop(displaced);
    }
}

pub(super) struct AbandonedScan {
    original: anyhow::Error,
    _storage: OwnedSemaphorePermit,
}

impl AbandonedScan {
    pub(super) fn original(&self) -> &anyhow::Error {
        &self.original
    }
}

impl std::fmt::Display for AbandonedScan {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("tunnel scan was abandoned after a discovery failure")
    }
}

impl std::fmt::Debug for AbandonedScan {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AbandonedTunnelScan")
            .field(
                "primary_class",
                &ios_core::classify_error(self.original().as_ref()),
            )
            .finish()
    }
}

impl Error for AbandonedScan {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        Some(self.original.as_ref())
    }
}

#[derive(Clone)]
pub(super) struct Epoch {
    pub(super) changes: watch::Receiver<Option<Arc<()>>>,
    pub(super) generation: Arc<()>,
}

impl Epoch {
    fn current(&self) -> bool {
        self.changes
            .borrow()
            .as_ref()
            .is_some_and(|epoch| Arc::ptr_eq(epoch, &self.generation))
    }

    pub(super) fn commit<T>(
        &self,
        clock: &Deadline,
        action: impl FnOnce() -> Result<T>,
    ) -> Result<T> {
        let epoch = self.changes.borrow();
        if !epoch
            .as_ref()
            .is_some_and(|epoch| Arc::ptr_eq(epoch, &self.generation))
        {
            return Err(super::lifecycle::cancelled());
        }
        clock.check()?;
        action()
    }

    // Evaluate device metadata before taking a watch guard. If the clock
    // crosses during removals, restore every unobserved original owner.
    pub(super) fn retain<T>(
        &self,
        clock: &Deadline,
        table: &mut super::publication_table::Table<T>,
        bank: &Arc<Semaphore>,
        keep: impl Fn(&str, &T) -> bool,
    ) -> Result<super::publication_retain::Removed<T>> {
        super::publication_retain::retain(self, clock, table, bank, keep)
    }
}

pub(super) struct Deadline {
    pub(super) at: Instant,
}

#[derive(Debug)]
pub(super) struct DeadlineReached;
impl std::fmt::Display for DeadlineReached {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("tunnel whole-scan deadline reached")
    }
}
impl Error for DeadlineReached {}

impl Deadline {
    pub(super) fn new(timeout: Duration) -> Result<Self> {
        let at = Instant::now()
            .checked_add(timeout)
            .ok_or_else(|| crate::output::input_decision("tunnel whole-scan deadline overflow"))?;
        Ok(Self { at })
    }

    pub(super) fn check(&self) -> Result<()> {
        if Instant::now() >= self.at {
            Err(DeadlineReached.into())
        } else {
            Ok(())
        }
    }

    pub(super) async fn run<T>(
        &self,
        epoch: &Epoch,
        work: impl Future<Output = Result<T>>,
    ) -> Result<Option<T>> {
        let mut changes = epoch.changes.clone();
        if !epoch.current() {
            return Ok(None);
        }
        tokio::pin!(work);
        tokio::select! {
            biased;
            _ = changes.changed() => Ok(None),
            expired = tokio::time::timeout_at(self.at, std::future::pending::<T>()) => {
                expired.map(Some).context("tunnel whole-scan total expired")
            }
            result = std::future::poll_fn(|cx| {
                if self.check().is_err() || !epoch.current() {
                    return std::task::Poll::Pending;
                }
                match work.as_mut().poll(cx) {
                    std::task::Poll::Ready(Err(error)) if error.is::<DeadlineReached>() => std::task::Poll::Pending,
                    std::task::Poll::Ready(Err(error)) if error.is::<ios_core::OperationCancelled>() && !epoch.current() => std::task::Poll::Pending,
                    std::task::Poll::Ready(Ok(_)) if self.check().is_err() || !epoch.current() => std::task::Poll::Pending,
                    result => result,
                }
            }) => result.map(Some),
        }
    }
}
