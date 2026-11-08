//! Same-cause ordering across the three actual staged runtime ledgers.
//!
//! Lock order is hub state -> local ledger outcomes. A local record first
//! checks binding while holding only its local lock, then releases that lock
//! before entering this order. No notification or user-owned cause Drop occurs
//! under either lock. The hub retains identities and causes, never Ledgers,
//! worker handles, a runtime owner or a task's own JoinHandle.
use std::error::Error;
use std::fmt;
use std::future::Future;
use std::marker::PhantomData;
use std::mem::{self, ManuallyDrop};
use std::ptr::NonNull;
use std::sync::atomic::{fence, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::task::Poll;

use tokio::sync::{OwnedSemaphorePermit, Semaphore};
use tokio::time::error::Elapsed;
use tokio::time::Instant;

use super::{Cause, Identity, Ledger, LedgerState, OUTCOME_SLOTS};
use crate::tunnel::TunnelError;

const PHASE_COUNT: usize = 3;

/// These correspond to the three real ledgers: original four workers, native
/// control/read/write/bridge, and the separately owned undo coordinator.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Phase {
    Original = 0,
    Native = 1,
    Undo = 2,
}
impl Phase {
    const ALL: [Self; PHASE_COUNT] = [Self::Original, Self::Native, Self::Undo];
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum RecordKind {
    Work,
    Join,
    Cleanup,
}
fn record_kind(phase: Phase, slot: usize, work: bool) -> RecordKind {
    if !work && slot >= 4 {
        RecordKind::Join
    } else if work && phase != Phase::Undo {
        RecordKind::Work
    } else {
        RecordKind::Cleanup
    }
}
#[derive(Clone)]
pub(crate) struct PhaseCause {
    phase: Phase,
    slot: usize,
    kind: RecordKind,
    original: Cause,
}
impl PhaseCause {
    pub(crate) fn phase(&self) -> Phase {
        self.phase
    }
    pub(crate) fn local_slot(&self) -> usize {
        self.slot
    }
    pub(crate) fn kind(&self) -> RecordKind {
        self.kind
    }
    pub(crate) fn original(&self) -> &(dyn Error + Send + Sync + 'static) {
        self.original.as_ref()
    }
    pub(crate) fn shared(&self) -> &Cause {
        &self.original
    }
}
impl fmt::Debug for PhaseCause {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("StagedPhaseCause")
            .field("phase", &self.phase)
            .field("local_slot", &self.slot)
            .field("kind", &self.kind)
            .field("class", &crate::classify_error(self.original()))
            .finish()
    }
}
struct State {
    first: Option<PhaseCause>,
    // Actual phase identities only. Holding a Ledger here would make a cycle
    // because each bound local Ledger retains its optional hub sink.
    bound: [Option<Identity>; PHASE_COUNT],
}
struct Body {
    state: Mutex<State>,
    // Physical backing and the actual Body's cause/identity fields die before
    // this complete original allocation's byte credit is returned.
    _storage: Option<OwnedSemaphorePermit>,
}
#[repr(C)]
struct Allocation {
    strong: AtomicUsize,
    body: ManuallyDrop<Body>,
}

/// Independently admitted complete allocation, shared without a raw/Weak escape.
pub(crate) struct PhaseOutcomes {
    pointer: NonNull<Allocation>,
    _allocation: PhantomData<Allocation>,
}
// SAFETY: the private original allocation only shares a mutex and atomic count;
// final ownership acquires all releases before dropping its original fields.
unsafe impl Send for PhaseOutcomes where Allocation: Send + Sync {}
unsafe impl Sync for PhaseOutcomes where Allocation: Send + Sync {}

impl PhaseOutcomes {
    pub(crate) fn new(bank: Option<&Arc<Semaphore>>) -> Result<Self, TunnelError> {
        let (storage, mut values) = match bank {
            Some(bank) => {
                let (storage, values) =
                    crate::device::prepared_rsd::exact_vec::<Allocation>(bank, 1)
                        .map_err(|error| TunnelError::Io(std::io::Error::other(error)))?;
                (Some(storage), values)
            }
            None => (None, Vec::with_capacity(1)),
        };
        assert!(values.is_empty() && values.capacity() == 1);
        values.push(Allocation {
            strong: AtomicUsize::new(1),
            body: ManuallyDrop::new(Body {
                state: Mutex::new(State {
                    first: None,
                    bound: std::array::from_fn(|_| None),
                }),
                _storage: storage,
            }),
        });
        let pointer = NonNull::from(&mut values[0]);
        mem::forget(values);
        Ok(Self {
            pointer,
            _allocation: PhantomData,
        })
    }

    fn allocation(&self) -> &Allocation {
        // SAFETY: a live sealed handle retains a positive original count.
        unsafe { self.pointer.as_ref() }
    }
    fn body(&self) -> &Body {
        &self.allocation().body
    }
    fn same_hub(&self, other: &Self) -> bool {
        self.pointer == other.pointer
    }

    /// Bind before the first acquired local outcome. Native workers may
    /// already exist, but the lock/recheck protocol makes a racing record either
    /// sink through this hub or make this binding refuse without losing it.
    pub(crate) fn bind(&self, ledger: &Ledger, phase: Phase) -> Result<(), BindError> {
        let mut state = self
            .body()
            .state
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        let outcomes = ledger
            .outcomes
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        if let Some(sink) = ledger.phase_sink.get() {
            return if self.same_hub(&sink.hub) && phase == sink.phase {
                Ok(())
            } else {
                Err(BindError::LedgerAlreadyBound)
            };
        }
        if outcomes.slots.iter().any(Option::is_some)
            || outcomes.additional_unretained_outcomes != 0
        {
            return Err(BindError::OutcomePrecedesBinding);
        }
        if state.bound[phase as usize]
            .as_ref()
            .is_some_and(|identity| !identity.ptr_eq(&ledger.identity))
        {
            return Err(BindError::PhaseAlreadyBound);
        }
        let unused = ledger
            .phase_sink
            .set(PhaseSink {
                hub: self.clone(),
                phase,
            })
            .err();
        if unused.is_none() {
            state.bound[phase as usize] = Some(ledger.identity.clone());
        }
        drop(outcomes);
        drop(state);
        // A rejected retained root is dropped outside both guards as well.
        let failed = unused.is_some();
        drop(unused);
        if failed {
            Err(BindError::LedgerAlreadyBound)
        } else {
            Ok(())
        }
    }

    /// Snapshot raw acquired local evidence. This never calls local result():
    /// healthy running native workers remain healthy in work_result().
    pub(crate) fn snapshot(
        &self,
        original: &Ledger,
        native: Option<&Ledger>,
        undo: Option<&Ledger>,
    ) -> PhaseSnapshot {
        let state = self
            .body()
            .state
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        let inputs = [Some(original), native, undo];
        let mut binding_valid = true;
        let mut additional_unretained_outcomes = 0u64;
        let phases = std::array::from_fn(|index| {
            let phase = Phase::ALL[index];
            let Some(ledger) = inputs[index] else {
                if state.bound[index].is_some() {
                    binding_valid = false;
                }
                return None;
            };
            let valid = ledger
                .phase_sink
                .get()
                .is_some_and(|sink| sink.phase == phase && self.same_hub(&sink.hub))
                && state.bound[index]
                    .as_ref()
                    .is_some_and(|identity| identity.ptr_eq(&ledger.identity));
            binding_valid &= valid;
            let outcomes = ledger
                .outcomes
                .lock()
                .unwrap_or_else(|error| error.into_inner());
            let records = outcomes
                .slots
                .iter()
                .enumerate()
                .fold(0u16, |mask, (slot, record)| {
                    mask | if record.is_some() { 1 << slot } else { 0 }
                });
            // Match the original launch fence-before-count observation order.
            let launch_pending = ledger.is_launch_pending();
            let pending_joins = ledger.pending.load(Ordering::Acquire);
            match additional_unretained_outcomes
                .checked_add(outcomes.additional_unretained_outcomes)
            {
                Some(total) => additional_unretained_outcomes = total,
                None => {
                    additional_unretained_outcomes = u64::MAX;
                    binding_valid = false;
                }
            }
            Some(LocalPhaseSnapshot {
                phase,
                ledger: ledger.clone(),
                records,
                pending_joins,
                launch_pending,
                accounting_valid: !ledger.invalid_count.load(Ordering::Acquire),
            })
        });
        let first = state.first.clone().or_else(|| {
            phases
                .iter()
                .flatten()
                .find_map(|phase| phase.records().next())
        });
        let location = first.as_ref().map(|cause| FirstLocation {
            phase: cause.phase,
            slot: u8::try_from(cause.slot).expect("original ten-slot index"),
            kind: cause.kind,
        });
        let first = first.map(|cause| cause.original);
        drop(state);
        PhaseSnapshot {
            first,
            location,
            phases,
            binding_valid,
            additional_unretained_outcomes,
            require_joined: false,
            elapsed: None,
        }
    }

    pub(crate) fn work_result(
        &self,
        original: &Ledger,
        native: Option<&Ledger>,
        undo: Option<&Ledger>,
    ) -> Result<(), PhaseFailure> {
        self.snapshot(original, native, undo).result(None, false)
    }
    pub(crate) fn status(
        &self,
        original: &Ledger,
        native: Option<&Ledger>,
        undo: Option<&Ledger>,
    ) -> Result<(), PhaseFailure> {
        self.snapshot(original, native, undo).result(None, true)
    }
    pub(crate) fn attempt_result(
        &self,
        original: &Ledger,
        native: Option<&Ledger>,
        undo: Option<&Ledger>,
        elapsed: Option<Elapsed>,
    ) -> Result<(), PhaseFailure> {
        self.snapshot(original, native, undo).result(elapsed, false)
    }
    pub(crate) fn result(
        &self,
        original: &Ledger,
        native: Option<&Ledger>,
        undo: Option<&Ledger>,
        elapsed: Option<Elapsed>,
    ) -> Result<(), PhaseFailure> {
        self.snapshot(original, native, undo).result(elapsed, true)
    }
}

/// One caller clock covers all actual phase joins and the already admitted
/// undo job. A cancelled waiter leaves those joins and receipts in their owners.
/// The concrete collector stays on the caller's stack; cleanup allocates no Box.
pub(crate) async fn join_until(
    deadline: Result<Instant, TunnelError>,
    phases: &PhaseOutcomes,
    original: &Ledger,
    native: &Ledger,
    undo: &Ledger,
    work: impl Future<Output = ()>,
) -> Result<(), TunnelError> {
    let deadline = deadline?;
    tokio::pin!(work);
    let guarded = std::future::poll_fn(|cx| {
        if Instant::now() >= deadline {
            return Poll::Pending;
        }
        match work.as_mut().poll(cx) {
            Poll::Ready(()) if Instant::now() >= deadline => Poll::Pending,
            result => result,
        }
    });
    let elapsed = tokio::select! {
        biased;
        result = tokio::time::timeout_at(deadline, std::future::pending::<()>()) => {
            result.err()
        }
        _ = guarded => None,
    };
    phases
        .result(original, Some(native), Some(undo), elapsed)
        .map_err(TunnelError::Phases)
}

/// The original preparation clock also covers quiescence, the FIFO flush and
/// actual native readiness. A timer does not erase work already acquired by any
/// phase, and healthy persistent packet jobs need not be joined for readiness.
pub(crate) async fn activation_until(
    deadline: Instant,
    phases: &PhaseOutcomes,
    original: &Ledger,
    native: &Ledger,
    undo: &Ledger,
    operation: impl Future<Output = Result<(), TunnelError>>,
) -> Result<(), TunnelError> {
    tokio::pin!(operation);
    let guarded = std::future::poll_fn(|cx| {
        if Instant::now() >= deadline {
            return Poll::Pending;
        }
        match operation.as_mut().poll(cx) {
            Poll::Ready(Ok(())) if Instant::now() >= deadline => Poll::Pending,
            Poll::Ready(Err(error))
                if Instant::now() >= deadline
                    && crate::classify_error(&error) == crate::ErrorClass::Cancelled =>
            {
                Poll::Pending
            }
            result => result,
        }
    });
    tokio::select! {
        biased;
        elapsed = tokio::time::timeout_at(deadline, std::future::pending::<()>()) => {
            phases.attempt_result(original, Some(native), Some(undo), elapsed.err())
                .map_err(TunnelError::Phases)
        }
        result = guarded => result,
    }
}
impl Clone for PhaseOutcomes {
    fn clone(&self) -> Self {
        if self.allocation().strong.fetch_add(1, Ordering::Relaxed) >= isize::MAX as usize {
            std::process::abort();
        }
        Self {
            pointer: self.pointer,
            _allocation: PhantomData,
        }
    }
}
impl Drop for PhaseOutcomes {
    fn drop(&mut self) {
        if self.allocation().strong.fetch_sub(1, Ordering::Release) != 1 {
            return;
        }
        fence(Ordering::Acquire);
        let pointer = self.pointer.as_ptr();
        // SAFETY: unique final ownership, no mutable/pin/Weak/raw export. Body
        // can move; free the same actual backing before its storage field dies.
        let body = unsafe { ManuallyDrop::take(&mut (*pointer).body) };
        unsafe { drop(Vec::from_raw_parts(pointer, 0, 1)) };
        #[cfg(test)]
        AFTER_FREE.with(|hook| {
            if let Some(callback) = hook.borrow_mut().as_mut() {
                callback();
            }
        });
        drop(body);
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum BindError {
    LedgerAlreadyBound,
    PhaseAlreadyBound,
    OutcomePrecedesBinding,
}
impl fmt::Display for BindError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("staged phase ledger binding is invalid or too late")
    }
}
impl Error for BindError {}

#[derive(Clone)]
pub(super) struct PhaseSink {
    hub: PhaseOutcomes,
    phase: Phase,
}
impl PhaseSink {
    pub(super) fn owns_work_report(&self, report: &PhaseFailure) -> bool {
        report.work_report() && report.snapshot.same_hub(&self.hub)
    }
    /// Called without a local outcomes guard. Hub -> local is the only nesting.
    pub(super) fn record(&self, ledger: &LedgerState, slot: usize, cause: Cause, primary: bool) {
        let mut state = self
            .hub
            .body()
            .state
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        let mut outcomes = ledger
            .outcomes
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        if slot < OUTCOME_SLOTS && outcomes.slots[slot].is_none() && state.first.is_none() {
            // This lock is the cross-phase first-record linearization point.
            // Both records share the exact original cause, never a string/copy.
            state.first = Some(PhaseCause {
                phase: self.phase,
                slot,
                kind: record_kind(self.phase, slot, primary),
                original: cause.clone(),
            });
        }
        let rejected = LedgerState::record_locked(&mut outcomes, slot, cause, primary);
        drop(outcomes);
        drop(state);
        // An overwritten/repeated input can have an arbitrary panicking Drop.
        drop(rejected);
    }
}

/// Actual evidence copied from one original local ledger, without retaining it.
pub(crate) struct LocalPhaseSnapshot {
    phase: Phase,
    ledger: Ledger,
    // The original fixed slots are append-only. This exact observed mask
    // makes later iteration immutable without allocating/copying another arena.
    records: u16,
    pending_joins: usize,
    launch_pending: bool,
    accounting_valid: bool,
}
impl LocalPhaseSnapshot {
    pub(crate) fn records(&self) -> impl Iterator<Item = PhaseCause> + '_ {
        (0..OUTCOME_SLOTS).filter_map(move |slot| {
            if self.records & (1 << slot) == 0 {
                return None;
            }
            let outcomes = self
                .ledger
                .outcomes
                .lock()
                .unwrap_or_else(|error| error.into_inner());
            let record = outcomes.slots[slot]
                .as_ref()
                .expect("immutable acquired outcome slot");
            Some(PhaseCause {
                phase: self.phase,
                slot,
                kind: record_kind(self.phase, slot, record.work),
                original: record.cause.clone(),
            })
        })
    }
    pub(crate) fn joins_complete(&self) -> bool {
        self.pending_joins == 0 && !self.launch_pending && self.accounting_valid
    }
}

/// Plain owned report value. Its caller can store it in a pre-admitted report
/// destination; constructing it does not allocate a new Box/Arc or acquire quota.
pub(crate) struct PhaseSnapshot {
    first: Option<Cause>,
    location: Option<FirstLocation>,
    phases: [Option<LocalPhaseSnapshot>; PHASE_COUNT],
    binding_valid: bool,
    // Local ledgers retain their complete original counters. The aggregate
    // caches their checked sum at this snapshot, rather than duplicating arrays.
    additional_unretained_outcomes: u64,
    require_joined: bool,
    elapsed: Option<Elapsed>,
}
#[derive(Clone, Copy)]
struct FirstLocation {
    phase: Phase,
    slot: u8,
    kind: RecordKind,
}
impl PhaseSnapshot {
    pub(crate) fn first(&self) -> Option<PhaseCause> {
        self.first
            .as_ref()
            .zip(self.location)
            .map(|(cause, location)| PhaseCause {
                phase: location.phase,
                slot: usize::from(location.slot),
                kind: location.kind,
                original: cause.clone(),
            })
    }
    pub(crate) fn phases(&self) -> impl Iterator<Item = &LocalPhaseSnapshot> {
        self.phases.iter().flatten()
    }
    pub(crate) fn same_hub(&self, hub: &PhaseOutcomes) -> bool {
        self.phases().any(|phase| {
            phase
                .ledger
                .phase_sink
                .get()
                .is_some_and(|sink| sink.hub.same_hub(hub))
        })
    }
    pub(crate) fn binding_valid(&self) -> bool {
        self.binding_valid
    }
    pub(crate) fn pending_joins(&self) -> Option<usize> {
        self.phases().try_fold(0usize, |total, phase| {
            total.checked_add(phase.pending_joins)
        })
    }
    pub(crate) fn accounting_valid(&self) -> bool {
        self.binding_valid
            && self.pending_joins().is_some()
            && self.phases().all(|phase| phase.accounting_valid)
    }
    pub(crate) fn joins_complete(&self) -> bool {
        self.accounting_valid() && self.phases().all(LocalPhaseSnapshot::joins_complete)
    }
    pub(crate) fn has_retained_outcomes(&self) -> bool {
        self.first.is_some() || self.phases().any(|phase| phase.records().next().is_some())
    }
    fn result(
        mut self,
        elapsed: Option<Elapsed>,
        require_joined: bool,
    ) -> Result<(), PhaseFailure> {
        if self.has_retained_outcomes()
            || elapsed.is_some()
            || !self.accounting_valid()
            || (require_joined && !self.joins_complete())
        {
            self.elapsed = elapsed;
            self.require_joined = require_joined;
            Err(PhaseFailure { snapshot: self })
        } else {
            Ok(())
        }
    }
}

pub struct PhaseFailure {
    snapshot: PhaseSnapshot,
}
impl PhaseFailure {
    #[cfg(test)]
    pub(crate) fn snapshot(&self) -> &PhaseSnapshot {
        &self.snapshot
    }
    pub(crate) fn primary_shared(&self) -> Option<&Cause> {
        self.snapshot.first.as_ref()
    }
    pub fn primary(&self) -> &(dyn Error + 'static) {
        if let Some(first) = self.snapshot.first.as_ref() {
            return first.as_ref();
        }
        if let Some(elapsed) = &self.snapshot.elapsed {
            return elapsed;
        }
        &INCOMPLETE
    }
    pub fn cleanup(&self) -> impl Iterator<Item = PhaseCleanup<'_>> {
        let first = self.primary_shared();
        self.snapshot
            .phases()
            .flat_map(LocalPhaseSnapshot::records)
            .filter(move |record| !first.is_some_and(|cause| Arc::ptr_eq(cause, record.shared())))
            .map(|record| PhaseCleanup::Cause(record.original))
            .chain(
                self.snapshot
                    .elapsed
                    .iter()
                    .filter(move |_| first.is_some())
                    .map(PhaseCleanup::Elapsed),
            )
    }
    pub fn work_report(&self) -> bool {
        !self.snapshot.require_joined
    }
    pub fn elapsed(&self) -> Option<&Elapsed> {
        self.snapshot.elapsed.as_ref()
    }
    pub fn pending_joins(&self) -> Option<usize> {
        self.snapshot.pending_joins()
    }
    /// All selected joins were collected, including genuine failed joins.
    /// This does not certify native undo. Also check the attached handle's
    /// `has_pending_native_cleanup()` before releasing its retirement owner.
    pub fn joins_complete(&self) -> bool {
        self.snapshot.joins_complete()
    }
    pub fn has_retained_outcomes(&self) -> bool {
        self.snapshot.has_retained_outcomes()
    }
    pub fn additional_unretained_outcomes(&self) -> u64 {
        self.snapshot.additional_unretained_outcomes
    }
}
/// One acquired original cause (same Arc), or the caller's original elapsed.
/// Reading cleanup performs only reference-count clones of stable local slots.
pub enum PhaseCleanup<'a> {
    Cause(Arc<dyn Error + Send + Sync>),
    Elapsed(&'a Elapsed),
}
impl PhaseCleanup<'_> {
    pub fn original(&self) -> &(dyn Error + 'static) {
        match self {
            Self::Cause(cause) => cause.as_ref(),
            Self::Elapsed(elapsed) => *elapsed,
        }
    }
}
impl fmt::Display for PhaseFailure {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("staged native tunnel phase collection failed")
    }
}
impl fmt::Debug for PhaseFailure {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let first = self.snapshot.first();
        f.debug_struct("StagedPhaseFailure")
            .field("primary_class", &crate::classify_error(self.primary()))
            .field("first_phase", &first.as_ref().map(PhaseCause::phase))
            .field("first_slot", &first.as_ref().map(PhaseCause::local_slot))
            .field("first_kind", &first.as_ref().map(PhaseCause::kind))
            .field("pending_joins", &self.snapshot.pending_joins())
            .field("binding_valid", &self.snapshot.binding_valid())
            .field("joins_complete", &self.snapshot.joins_complete())
            .field("work_report", &self.work_report())
            .finish()
    }
}
impl Error for PhaseFailure {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        Some(self.primary())
    }
}
#[derive(Debug)]
struct IncompletePhases;
static INCOMPLETE: IncompletePhases = IncompletePhases;
impl fmt::Display for IncompletePhases {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("staged native phase joins or accounting remain incomplete")
    }
}
impl Error for IncompletePhases {}

#[cfg(test)]
type FreeHook = std::cell::RefCell<Option<Box<dyn FnMut()>>>;
#[cfg(test)]
thread_local! { static AFTER_FREE: FreeHook = const { std::cell::RefCell::new(None) }; }

#[cfg(test)]
#[path = "phase_outcomes/tests.rs"]
mod tests;
