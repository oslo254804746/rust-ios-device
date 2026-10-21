//! Whole-scan control and bounded ownership of errors acquired before cancellation.
use std::error::Error;
use std::future::Future;
use std::sync::{Arc, Mutex as StdMutex, OnceLock};
use std::time::Duration;

use anyhow::{Context, Result};
use tokio::sync::{watch, MutexGuard, Semaphore};
use tokio::time::Instant;

pub(super) const MAX_ABANDONED_SCANS: usize = 8;

pub(super) type AbandonedOwner = super::shared_allocation::Shared<AbandonedScan>;
pub(super) type HistoryOwner = super::shared_allocation::Shared<HistorySnapshot>;
pub(super) type OutcomesOwner = super::shared_allocation::Shared<Outcomes>;

#[cfg(test)]
#[path = "scan/capsule_tests.rs"]
mod capsule_tests;
#[cfg(test)]
#[path = "scan/history_owner_tests.rs"]
mod history_owner_tests;
#[cfg(test)]
#[path = "scan/init_owner_tests.rs"]
mod init_owner_tests;

enum Slot {
    Empty,
    Active,
    Abandoned(AbandonedOwner),
}

struct Slots {
    closed: bool,
    entries: [Slot; MAX_ABANDONED_SCANS],
    history_root: Option<HistoryOwner>,
}

struct HistoryRoots {
    entries: [Option<AbandonedOwner>; MAX_ABANDONED_SCANS],
    len: usize,
}

/// Immutable snapshot payload paid and created before its scan polls Source.
/// The inline roots have no separate Vec allocation. The full declared shared
/// layout is paid; opaque anyhow/context and terminal wrappers stay separate.
pub(super) struct HistorySnapshot {
    roots: OnceLock<HistoryRoots>,
}

impl HistorySnapshot {
    pub(super) fn body_bytes() -> usize {
        HistoryOwner::allocation_bytes()
    }
    fn freeze(&self, slots: &[Slot; MAX_ABANDONED_SCANS]) {
        let mut roots = HistoryRoots {
            entries: std::array::from_fn(|_| None),
            len: 0,
        };
        for slot in slots {
            if let Slot::Abandoned(original) = slot {
                roots.entries[roots.len] = Some(original.clone());
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

    pub(super) fn first(&self) -> Option<&AbandonedOwner> {
        self.iter().next()
    }

    pub(super) fn iter(&self) -> impl Iterator<Item = &AbandonedOwner> {
        let roots = self.roots();
        roots.entries[..roots.len]
            .iter()
            .map(|root| root.as_ref().expect("packed scan history root"))
    }
}

#[cfg(test)]
impl std::ops::Index<usize> for HistorySnapshot {
    type Output = AbandonedOwner;

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
    std::mem::size_of::<SourcePayload>() + std::mem::size_of::<super::bookkeeping::FailurePayload>()
}

pub(super) fn capsule_reservation_bytes() -> usize {
    compound_bytes()
        + AbandonedScan::body_bytes()
        + std::mem::size_of::<super::ScanCleanupPayload>()
        + HistorySnapshot::body_bytes()
}

impl Outcomes {
    #[cfg(test)]
    pub(super) fn new() -> OutcomesOwner {
        Self::with_metadata(Arc::new(Semaphore::new(
            super::MAX_GLOBAL_KNOWN_METADATA_BYTES,
        )))
        .unwrap()
    }

    pub(super) fn body_bytes() -> usize {
        OutcomesOwner::allocation_bytes()
    }

    pub(super) fn with_metadata(bank: Arc<Semaphore>) -> Result<OutcomesOwner> {
        let storage = super::admission::known_metadata_charge(&bank, Self::body_bytes())?;
        OutcomesOwner::from_grant(
            Self {
                slots: StdMutex::new(Slots {
                    closed: false,
                    entries: std::array::from_fn(|_| Slot::Empty),
                    history_root: None,
                }),
                bank: bank.clone(),
            },
            &bank,
            storage,
        )
    }
}

// A stable crate-local specialized implementation keeps the admitted original
// owner available for Session; it does not expose a raw Arc receiver/adapter.
impl OutcomesOwner {
    pub(super) fn begin<'a>(&self, lock: MutexGuard<'a, ()>) -> Result<Session<'a>> {
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
        let source_storage = storage
            .split(std::mem::size_of::<SourcePayload>())
            .expect("reserved source payload bytes");
        let failure_storage = storage
            .split(std::mem::size_of::<super::bookkeeping::FailurePayload>())
            .expect("reserved completion payload bytes");
        let cleanup = storage
            .split(std::mem::size_of::<super::ScanCleanupPayload>())
            .expect("reserved cleanup bytes");
        // Every branch owns its actual distinct empty allocation before Source.
        // Choosing a context later moves values into it without new backing.
        let compound = ContextDestinations {
            source: super::owned_box::Destination::from_grant(&self.bank, source_storage)?,
            failure: super::owned_box::Destination::from_grant(&self.bank, failure_storage)?,
        };
        let cleanup = super::owned_box::Destination::from_grant(&self.bank, cleanup)?;
        let snapshot_storage = storage
            .split(HistorySnapshot::body_bytes())
            .expect("reserved immutable history snapshot body");
        let history_snapshot = HistoryOwner::from_grant(
            HistorySnapshot {
                roots: OnceLock::new(),
            },
            &self.bank,
            snapshot_storage,
        )?;
        // Allocate the actual error cell BEFORE Source. Cancellation/failure
        // later only fills this one cell; it never allocates a replacement root.
        let abandoned = AbandonedOwner::from_grant(
            AbandonedScan {
                original: OnceLock::new(),
            },
            &self.bank,
            storage,
        )?;
        slots.entries[index] = Slot::Active;
        Ok(Session {
            outcomes: self.clone(),
            index,
            acquired: StdMutex::new(Acquired::default()),
            writes: StdMutex::new(None),
            compound: StdMutex::new(Some(compound)),
            cleanup: StdMutex::new(Some(cleanup)),
            abandoned: Some(abandoned),
            // Separate from AbandonedScan: a snapshot must not own itself.
            history_snapshot: Some(history_snapshot),
            _lock: lock,
        })
    }
}

impl Outcomes {
    pub(super) fn close(&self) {
        self.slots.lock().unwrap_or_else(|p| p.into_inner()).closed = true;
    }

    pub(super) fn history(&self) -> Option<HistoryOwner> {
        self.slots
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .history_root
            .as_ref()
            .cloned()
    }

    pub(super) fn payload_history(
        &self,
    ) -> (usize, Result<super::metadata_vec::KnownVec<AbandonedOwner>>) {
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
                    roots.push_reserved(original.clone());
                }
            }
            Ok(roots)
        })();
        (active, history)
    }
}

pub(super) struct Session<'a> {
    outcomes: OutcomesOwner,
    index: usize,
    acquired: StdMutex<Acquired>,
    writes: StdMutex<Option<super::bookkeeping::WriteOwner>>,
    compound: StdMutex<Option<ContextDestinations>>,
    cleanup: StdMutex<Option<super::owned_box::Destination<super::ScanCleanupPayload>>>,
    abandoned: Option<AbandonedOwner>,
    history_snapshot: Option<HistoryOwner>,
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
        destinations: &StdMutex<Option<ContextDestinations>>,
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
                    take_destinations(destinations).failure,
                )),
                None => primary,
            });
        }
        Some(primary.context(SourceContext {
            payload: take_destinations(destinations).source.place(SourcePayload {
                usb: self.usb,
                paired: self.paired,
                coordinator: self.coordinator,
                freshness: self.freshness,
                completion,
            }),
        }))
    }
}

// Each branch owns a distinct original allocation and its complete grant.
// The unused branch is physically freed before its own credit returns.
struct ContextDestinations {
    source: super::owned_box::Destination<SourcePayload>,
    failure: super::owned_box::Destination<super::bookkeeping::FailurePayload>,
}

pub(super) struct SourcePayload {
    usb: Option<anyhow::Error>,
    paired: Option<anyhow::Error>,
    coordinator: Option<anyhow::Error>,
    freshness: Option<anyhow::Error>,
    completion: Option<anyhow::Error>,
}

/// The primary remains the original anyhow object. Independent causes use the
/// original pre-Source payload Box; the opaque outer context is still separate.
pub(super) struct SourceContext {
    payload: super::owned_box::Owner<SourcePayload>,
}

fn take_destinations(destinations: &StdMutex<Option<ContextDestinations>>) -> ContextDestinations {
    destinations
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .take()
        .expect("one terminal compound context per admitted scan")
}

impl SourceContext {
    pub(super) fn secondary(&self, source: Source) -> Option<&anyhow::Error> {
        match source {
            Source::Usb => self.payload.usb.as_ref(),
            Source::Paired => self.payload.paired.as_ref(),
            Source::Coordinator => self.payload.coordinator.as_ref(),
            Source::Freshness => self.payload.freshness.as_ref(),
        }
    }

    pub(super) fn completion(&self) -> Option<&anyhow::Error> {
        self.payload.completion.as_ref()
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
            .field("usb_class", &class(&self.payload.usb))
            .field("paired_class", &class(&self.payload.paired))
            .field("coordinator_class", &class(&self.payload.coordinator))
            .field("freshness_class", &class(&self.payload.freshness))
            .field("completion_class", &class(&self.payload.completion))
            .finish()
    }
}

impl Session<'_> {
    pub(super) fn retain_writes(&self, owner: super::bookkeeping::WriteOwner) -> Result<()> {
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

    pub(super) fn cleanup_destination(
        &self,
    ) -> super::owned_box::Destination<super::ScanCleanupPayload> {
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
                let abandoned = self
                    .abandoned
                    .take()
                    .expect("preallocated abandoned scan root");
                assert!(
                    abandoned.original.set(primary).is_ok(),
                    "one original error per abandoned scan"
                );
                slots.entries[self.index] = Slot::Abandoned(abandoned);
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
    original: OnceLock<anyhow::Error>,
}

impl AbandonedScan {
    pub(super) fn body_bytes() -> usize {
        AbandonedOwner::allocation_bytes()
    }
    pub(super) fn original(&self) -> &anyhow::Error {
        self.original
            .get()
            .expect("published abandoned scan has original error")
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
        Some(self.original().as_ref())
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
