//! Persistent tunnel tasks, native outcomes and retryable explicit joins.

use std::error::Error;
use std::future::{pending, Future};
use std::pin::Pin;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex as StdMutex};
use std::task::Poll;
use std::time::Duration;

use tokio::sync::OwnedSemaphorePermit;
use tokio::sync::{Mutex, MutexGuard};
use tokio::task::{AbortHandle, JoinError, JoinHandle, JoinSet};
use tokio::time::{error::Elapsed, Instant};

use super::TunnelError;

#[path = "owner/native_identity.rs"]
mod native_identity;
use native_identity::Identity;
#[path = "owner/ledger_root.rs"]
mod ledger_root;
pub(crate) use ledger_root::Ledger;
#[path = "owner/child_root.rs"]
mod child_root;
pub(crate) use child_root::SharedChildTaskSet;

type Cause = Arc<dyn Error + Send + Sync>;
const OUTCOME_SLOTS: usize = 10;
// 0..4: original parent work errors; 4..8: parent JoinErrors;
// 8: forwarding reader JoinError; 9: first proxy client JoinError.

struct Recorded {
    cause: Cause,
    sequence: u64,
}

struct Outcomes {
    slots: [Option<Recorded>; OUTCOME_SLOTS],
    primary: Option<usize>,
    next_sequence: u64,
    additional_unretained_outcomes: u64,
}

// One slot per retained outcome, plus this attempt's possible native expiry.
// These are actual inline report fields, not an estimated unused heap reserve.
struct Cleanup {
    values: [Option<Cause>; OUTCOME_SLOTS + 1],
    len: usize,
}
impl Cleanup {
    fn new() -> Self {
        Self {
            values: std::array::from_fn(|_| None),
            len: 0,
        }
    }
    fn push(&mut self, cause: Cause) {
        assert!(self.len < self.values.len());
        self.values[self.len] = Some(cause);
        self.len += 1;
    }
    fn iter(&self) -> impl Iterator<Item = &Cause> {
        self.values[..self.len].iter().flatten()
    }
}
struct CleanupClasses<'a>(&'a Cleanup);
impl std::fmt::Debug for CleanupClasses<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_list()
            .entries(
                self.0
                    .iter()
                    .map(|cause| crate::classify_error(cause.as_ref())),
            )
            .finish()
    }
}

pub(crate) struct LedgerState {
    identity: Identity,
    outcomes: StdMutex<Outcomes>,
    pending: AtomicUsize,
    invalid_count: AtomicBool,
    #[cfg(feature = "tunnel-userspace")]
    runtime_stopped: AtomicBool,
    _body_storage: Option<OwnedSemaphorePermit>,
}

impl LedgerState {
    fn new(storage: Option<OwnedSemaphorePermit>, identity: Identity) -> Self {
        Self {
            identity,
            outcomes: StdMutex::new(Outcomes {
                slots: std::array::from_fn(|_| None),
                primary: None,
                next_sequence: 0,
                additional_unretained_outcomes: 0,
            }),
            pending: AtomicUsize::new(0),
            invalid_count: AtomicBool::new(false),
            #[cfg(feature = "tunnel-userspace")]
            runtime_stopped: AtomicBool::new(false),
            _body_storage: storage,
        }
    }

    #[cfg(feature = "tunnel-userspace")]
    pub(crate) fn is_runtime_stopped(&self) -> bool {
        self.runtime_stopped.load(Ordering::Acquire)
    }

    #[cfg(feature = "tunnel-userspace")]
    pub(crate) fn request_runtime_stop(&self) {
        self.runtime_stopped.store(true, Ordering::Release);
    }

    // These hooks count actual retained join slots, including child groups once
    // integration registers them. Completion must call joined exactly once.
    pub(crate) fn registered(&self) {
        if self
            .pending
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |value| {
                value.checked_add(1)
            })
            .is_err()
        {
            self.invalid_count.store(true, Ordering::Release);
        }
    }

    pub(crate) fn joined(&self) {
        if self
            .pending
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |value| {
                value.checked_sub(1)
            })
            .is_err()
        {
            self.invalid_count.store(true, Ordering::Release);
        }
    }

    pub(crate) fn record_work(&self, role: usize, cause: Cause) {
        self.record(role, cause, true);
    }

    pub(crate) fn record_cleanup(&self, slot: usize, cause: Cause) {
        self.record(slot, cause, false);
    }

    fn is_own_work_report(&self, error: &TunnelError) -> bool {
        let TunnelError::Io(error) = error else {
            return false;
        };
        let Some(context) = error
            .get_ref()
            .and_then(|source| source.downcast_ref::<crate::error::context::RetainedCause>())
        else {
            return false;
        };
        let Some(report) = context
            .source()
            .and_then(|source| source.downcast_ref::<TunnelShutdownFailure>())
        else {
            return false;
        };
        report.work_report && self.identity.ptr_eq(&report.identity)
    }

    fn record(&self, slot: usize, cause: Cause, primary: bool) {
        let mut outcomes = self
            .outcomes
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        if slot >= OUTCOME_SLOTS || outcomes.slots[slot].is_some() {
            outcomes.additional_unretained_outcomes =
                outcomes.additional_unretained_outcomes.saturating_add(1);
            return;
        }
        let sequence = outcomes.next_sequence;
        outcomes.next_sequence = outcomes.next_sequence.saturating_add(1);
        outcomes.slots[slot] = Some(Recorded { cause, sequence });
        if primary && outcomes.primary.is_none() {
            outcomes.primary = Some(slot);
        }
    }

    pub(crate) fn result(&self, elapsed: Option<Elapsed>) -> Result<(), TunnelError> {
        self.snapshot(elapsed, true)
    }

    // A running parent cannot join its own handle; this typed task result
    // reports acquired failures without certifying whole-runtime completion.
    #[cfg(any(feature = "tunnel-kernel", feature = "tunnel-userspace"))]
    pub(crate) fn work_result(&self) -> Result<(), TunnelError> {
        self.snapshot(None, false)
    }

    fn snapshot(&self, elapsed: Option<Elapsed>, require_joined: bool) -> Result<(), TunnelError> {
        let outcomes = self
            .outcomes
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        let primary_slot = outcomes.primary.or_else(|| {
            outcomes
                .slots
                .iter()
                .enumerate()
                .filter_map(|(slot, recorded)| {
                    recorded.as_ref().map(|value| (slot, value.sequence))
                })
                .min_by_key(|(_, sequence)| *sequence)
                .map(|(slot, _)| slot)
        });
        let elapsed = elapsed.map(|value| Arc::new(value) as Cause);
        let primary = primary_slot
            .and_then(|slot| outcomes.slots[slot].as_ref())
            .map(|recorded| Arc::clone(&recorded.cause))
            .or_else(|| elapsed.clone());
        let pending_joins = self.pending.load(Ordering::Acquire);
        let primary = primary.or_else(|| {
            ((require_joined && pending_joins != 0) || self.invalid_count.load(Ordering::Acquire))
                .then(|| Arc::new(IncompleteShutdown) as Cause)
        });
        let Some(primary) = primary else {
            return Ok(());
        };
        let mut cleanup = Cleanup::new();
        for cause in outcomes
            .slots
            .iter()
            .enumerate()
            .filter(|(slot, _)| Some(*slot) != primary_slot)
            .filter_map(|(_, recorded)| recorded.as_ref().map(|value| Arc::clone(&value.cause)))
        {
            cleanup.push(cause);
        }
        if primary_slot.is_some() {
            if let Some(elapsed) = elapsed {
                cleanup.push(elapsed);
            }
        }
        let failure = TunnelShutdownFailure {
            identity: self.identity.clone(),
            work_report: !require_joined,
            primary,
            cleanup,
            pending_joins,
            join_accounting_valid: !self.invalid_count.load(Ordering::Acquire),
            retained_outcomes: outcomes.slots.iter().any(Option::is_some),
            additional_unretained_outcomes: outcomes.additional_unretained_outcomes,
        };
        drop(outcomes);
        Err(TunnelError::Io(crate::error::context::retained_io(
            std::io::ErrorKind::Other,
            "join native tunnel workers",
            failure,
        )))
    }
}

/// Immutable evidence from one native shutdown attempt, with bounded retention.
///
/// The primary cause drives classification and `Error::source`; cleanup causes
/// include additional retained failures and this attempt's native timeout.
/// Retrying produces a new snapshot and preserves the earlier one. Debug and
/// Display expose fixed context and categories; inspectors retain native causes.
pub struct TunnelShutdownFailure {
    identity: Identity,
    work_report: bool,
    primary: Cause,
    cleanup: Cleanup,
    pending_joins: usize,
    join_accounting_valid: bool,
    retained_outcomes: bool,
    additional_unretained_outcomes: u64,
}

impl TunnelShutdownFailure {
    /// Original primary work or join failure, or this attempt's native timeout.
    pub fn primary(&self) -> &(dyn Error + 'static) {
        self.primary.as_ref()
    }
    /// Additional retained native causes, including cleanup expiry.
    pub fn cleanup(&self) -> impl Iterator<Item = &(dyn Error + 'static)> {
        self.cleanup
            .iter()
            .map(|cause| cause.as_ref() as &(dyn Error + 'static))
    }
    /// Original native join slots not yet collected when this snapshot was made.
    pub fn pending_joins(&self) -> usize {
        self.pending_joins
    }
    /// Whether all owned native joins were collected with valid accounting.
    /// This does not certify an error-free attempt, its deadline, or a peer ACK.
    pub fn joins_complete(&self) -> bool {
        self.pending_joins == 0 && self.join_accounting_valid
    }

    /// Whether this snapshot includes an acquired original work/join failure.
    /// Pure attempt expiry or incomplete accounting alone does not set this.
    pub fn has_retained_outcomes(&self) -> bool {
        self.retained_outcomes
    }

    /// Repeated observations omitted by fixed per-role retention, not a count
    /// of distinct original failures. The counter saturates on overflow.
    pub fn additional_unretained_outcomes(&self) -> u64 {
        self.additional_unretained_outcomes
    }
}

impl std::fmt::Display for TunnelShutdownFailure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("native tunnel shutdown failed")
    }
}

impl std::fmt::Debug for TunnelShutdownFailure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TunnelShutdownFailure")
            .field("primary_class", &crate::classify_error(self.primary()))
            .field("cleanup_classes", &CleanupClasses(&self.cleanup))
            .field("pending_joins", &self.pending_joins)
            .field("joins_complete", &self.joins_complete())
            .field(
                "additional_unretained_outcomes",
                &self.additional_unretained_outcomes,
            )
            .finish()
    }
}

impl Error for TunnelShutdownFailure {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        Some(self.primary())
    }
}

struct IncompleteShutdown;

impl std::fmt::Display for IncompleteShutdown {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("native tunnel joins remain incomplete")
    }
}
impl std::fmt::Debug for IncompleteShutdown {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("IncompleteTunnelShutdown")
    }
}
impl Error for IncompleteShutdown {}

enum Completion {
    Normal,
    // Retained native outcomes intentionally survive repeated joins, even when
    // expected cancellation is quiet. Tests may inspect these named fields.
    WorkFailed { _cause: Arc<TunnelError> },
    RequestedAbort { _cause: Arc<JoinError> },
    UnexpectedJoin { _cause: Arc<JoinError> },
}

#[derive(Clone, Copy)]
pub(crate) enum ParentRole {
    ForwardOrReader = 0,
    #[cfg(feature = "tunnel-userspace")]
    Writer = 1,
    #[cfg(feature = "tunnel-userspace")]
    Smoltcp = 2,
    #[cfg(feature = "tunnel-userspace")]
    Listener = 3,
}

struct JoinState {
    handle: Option<JoinHandle<Result<(), TunnelError>>>,
    completion: Option<Completion>,
}

pub(crate) struct OwnedTask {
    abort: AbortHandle,
    requested_abort: AtomicBool,
    join: Mutex<JoinState>,
    role: usize,
    ledger: Ledger,
}

impl OwnedTask {
    pub(crate) fn new(
        handle: JoinHandle<Result<(), TunnelError>>,
        role: ParentRole,
        ledger: Ledger,
    ) -> Self {
        ledger.registered();
        Self {
            abort: handle.abort_handle(),
            requested_abort: AtomicBool::new(false),
            join: Mutex::new(JoinState {
                handle: Some(handle),
                completion: None,
            }),
            role: role as usize,
            ledger,
        }
    }

    #[cfg(any(feature = "tunnel-userspace", test))]
    pub(crate) fn is_finished(&self) -> bool {
        self.abort.is_finished()
    }

    #[cfg(any(feature = "tunnel-userspace", test))]
    pub(crate) fn abort(&self) {
        // Preserve an already completed unexpected cancellation. Tokio cannot
        // attribute racing aborts; the flag records our request while unfinished.
        if !self.abort.is_finished() {
            self.requested_abort.store(true, Ordering::Release);
        }
        self.abort.abort();
    }

    pub(crate) async fn join(&self) {
        let mut state = self.join.lock().await;
        if state.completion.is_some() {
            return;
        }
        let Some(handle) = state.handle.as_mut() else {
            return;
        };
        // The handle stays in state across this cancellation-safe borrow.
        let result = handle.await;
        let completion = match result {
            Ok(Ok(())) => Completion::Normal,
            Ok(Err(error)) => {
                let error = Arc::new(error);
                // A direct work report from this exact ledger describes causes
                // already retained before cleanup. Keep the typed completion
                // without promoting a child failure or recording it twice.
                // Foreign/full/wrapped reports remain ordinary work errors.
                if !self.ledger.is_own_work_report(&error) {
                    self.ledger.record_work(self.role, error.clone());
                }
                Completion::WorkFailed { _cause: error }
            }
            Err(error) if error.is_cancelled() && self.requested_abort.load(Ordering::Acquire) => {
                Completion::RequestedAbort {
                    _cause: Arc::new(error),
                }
            }
            Err(error) => {
                let error = Arc::new(error);
                self.ledger.record_cleanup(self.role + 4, error.clone());
                Completion::UnexpectedJoin { _cause: error }
            }
        };
        // No await after obtaining Ready until the original outcome is durable.
        state.completion = Some(completion);
        state.handle = None;
        self.ledger.joined();
    }
}

impl Drop for OwnedTask {
    fn drop(&mut self) {
        // Final owner Drop can abort; only explicit joins certify completion.
        self.abort.abort();
    }
}

pub(crate) fn deadline(budget: Duration) -> Result<Instant, TunnelError> {
    Instant::now().checked_add(budget).ok_or_else(|| {
        std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "native tunnel shutdown deadline is too large",
        )
        .into()
    })
}

pub(crate) async fn join_until(
    deadline: Result<Instant, TunnelError>,
    ledger: &Ledger,
    work: impl Future<Output = ()>,
) -> Result<(), TunnelError> {
    let deadline = deadline?;
    let mut work = Box::pin(work);
    let guarded = std::future::poll_fn(|cx| {
        if Instant::now() >= deadline {
            return Poll::Pending;
        }
        match Pin::as_mut(&mut work).poll(cx) {
            // Work may finish synchronously after crossing the deadline. Its
            // native outcomes are already recorded, but only the real timer
            // can complete this attempt. Never repoll the completed future.
            Poll::Ready(()) if Instant::now() >= deadline => Poll::Pending,
            result => result,
        }
    });
    tokio::select! {
        biased;
        elapsed = tokio::time::timeout_at(deadline, pending::<()>()) => {
            match elapsed {
                Err(elapsed) => ledger.result(Some(elapsed)),
                Ok(()) => ledger.result(None),
            }
        }
        _ = guarded => ledger.result(None),
    }
}

/// Existing child JoinSet kept by the runtime as well as by its parent driver.
/// Dedicated child-set mutex; never a manager/global-table lock.
pub(crate) struct ChildState {
    set: Mutex<JoinSet<()>>,
    ledger: Ledger,
    cleanup_slot: usize,
    requested_abort: AtomicBool,
    first_expected_abort: StdMutex<Option<Arc<JoinError>>>,
}

impl ChildState {
    fn new(ledger: Ledger, cleanup_slot: ChildRole) -> Self {
        Self {
            set: Mutex::new(JoinSet::new()),
            ledger,
            cleanup_slot: cleanup_slot as usize,
            requested_abort: AtomicBool::new(false),
            first_expected_abort: StdMutex::new(None),
        }
    }

    pub(crate) async fn lock(&self) -> ChildGuard<'_> {
        ChildGuard {
            set: self.set.lock().await,
            owner: self,
        }
    }

    pub(crate) async fn drain(&self) {
        let mut guard = self.lock().await;
        guard.abort_all();
        while guard.join_next().await.is_some() {}
    }
}

#[derive(Clone, Copy)]
pub(crate) enum ChildRole {
    #[cfg(any(feature = "tunnel-kernel", test))]
    ForwardReader = 8,
    #[cfg(feature = "tunnel-userspace")]
    ProxyClient = 9,
}

pub(crate) struct ChildGuard<'a> {
    set: MutexGuard<'a, JoinSet<()>>,
    owner: &'a ChildState,
}

impl ChildGuard<'_> {
    #[cfg(any(feature = "tunnel-userspace", test))]
    pub(crate) fn is_empty(&self) -> bool {
        self.set.is_empty()
    }

    pub(crate) fn spawn(&mut self, future: impl Future<Output = ()> + Send + 'static) {
        self.set.spawn(future);
        self.owner.ledger.registered();
    }

    pub(crate) fn abort_all(&mut self) {
        // Reap completed native outcomes before tagging the remaining tasks as
        // locally aborted, preserving both their causes and join accounting.
        while let Some(joined) = self.set.try_join_next() {
            let _ = self.record_joined(joined);
        }
        self.owner.requested_abort.store(true, Ordering::Release);
        self.set.abort_all();
    }

    pub(crate) async fn join_next(&mut self) -> Option<Result<(), Arc<JoinError>>> {
        let joined = self.set.join_next().await?;
        Some(self.record_joined(joined))
    }

    fn record_joined(&mut self, joined: Result<(), JoinError>) -> Result<(), Arc<JoinError>> {
        let result = match joined {
            Ok(()) => Ok(()),
            Err(error) => {
                let error = Arc::new(error);
                if error.is_cancelled() && self.owner.requested_abort.load(Ordering::Acquire) {
                    let mut saved = self
                        .owner
                        .first_expected_abort
                        .lock()
                        .unwrap_or_else(|error| error.into_inner());
                    if saved.is_none() {
                        *saved = Some(Arc::clone(&error));
                    }
                } else {
                    self.owner
                        .ledger
                        .record_cleanup(self.owner.cleanup_slot, error.clone());
                }
                Err(error)
            }
        };
        // The native outcome must be durable before completion is published.
        self.owner.ledger.joined();
        result
    }
}

impl Drop for ChildGuard<'_> {
    fn drop(&mut self) {
        // Parent unwind/abort releases this mutex guard, but the runtime still
        // owns the set and its original child handles for the next explicit join.
        self.abort_all();
    }
}

#[cfg(test)]
mod tests;

#[cfg(all(test, feature = "tunnel-userspace"))]
#[path = "owner/storage_tests.rs"]
mod storage_tests;
