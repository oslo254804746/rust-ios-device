//! Automatic connection episodes, independent of contested failure-table writes.
//!
//! A row holds the original coordinator and failure reservation. There is one
//! row per admitted device, no task or event queue, and no eviction that could
//! silently restart an exhausted budget. Original native owners remain in the
//! existing retirement registry; this ledger never stops or releases them.

use std::num::{NonZeroU32, NonZeroU64};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use anyhow::Result;
use ios_core::ErrorClass;
use tokio::sync::{OwnedSemaphorePermit, Semaphore};
use tokio::time::Instant;

use super::{AttemptIdentity, Ticket, MAX_FAILURE_RECORDS};

mod causes;
pub(super) use causes::{Completion, FailureReceipt, Receipt, SourceFailure};
mod storage;
pub(super) use storage::RemovedBatch;
use storage::{CauseState, OwnedRemoved, Removed, State};

const INITIAL_DELAY: Duration = Duration::from_secs(30);
const MAX_DELAY: Duration = Duration::from_secs(300);

#[cfg(test)]
mod tests;

#[derive(clap::Args, Clone, Debug)]
pub(super) struct RetryArgs {
    /// Initial delay after an automatic tunnel connection fails
    #[arg(long, default_value_t = 30_000)]
    retry_initial_delay_ms: u64,
    /// Maximum exponential delay between automatic tunnel attempts
    #[arg(long, default_value_t = 300_000)]
    retry_max_delay_ms: u64,
    /// Maximum automatic connection attempts per attachment failure episode
    #[arg(long)]
    retry_max_attempts: Option<NonZeroU32>,
    /// Total episode budget, including attempts and time between retries
    #[arg(long)]
    retry_total_ms: Option<NonZeroU64>,
}

impl RetryArgs {
    pub(super) fn validate(self) -> Result<RetryPolicy> {
        RetryPolicy::new(
            Duration::from_millis(self.retry_initial_delay_ms),
            Duration::from_millis(self.retry_max_delay_ms),
            self.retry_max_attempts,
            self.retry_total_ms
                .map(|value| Duration::from_millis(value.get())),
        )
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct RetryPolicy {
    initial_delay: Duration,
    max_delay: Duration,
    max_attempts: Option<NonZeroU32>,
    total: Option<Duration>,
}

impl Default for RetryPolicy {
    fn default() -> Self {
        Self {
            initial_delay: INITIAL_DELAY,
            max_delay: MAX_DELAY,
            max_attempts: None,
            total: None,
        }
    }
}

impl RetryPolicy {
    pub(super) fn new(
        initial_delay: Duration,
        max_delay: Duration,
        max_attempts: Option<NonZeroU32>,
        total: Option<Duration>,
    ) -> Result<Self> {
        let now = Instant::now();
        if initial_delay.is_zero()
            || max_delay < initial_delay
            || total.is_some_and(|budget| budget.is_zero())
            || now.checked_add(max_delay).is_none()
            || total.is_some_and(|budget| now.checked_add(budget).is_none())
        {
            return Err(crate::output::input_decision("invalid tunnel retry policy"));
        }
        Ok(Self {
            initial_delay,
            max_delay,
            max_attempts,
            total,
        })
    }

    pub(super) fn delay(&self, failures: u32) -> Duration {
        let mut delay = self.initial_delay;
        // Duration's representable range is reached in fewer than 128 doublings.
        for _ in 0..failures.saturating_sub(1).min(128) {
            delay = delay
                .checked_mul(2)
                .unwrap_or(self.max_delay)
                .min(self.max_delay);
            if delay == self.max_delay {
                break;
            }
        }
        delay
    }

    fn authoritative_schedule(&self) -> bool {
        // Keep the legacy default failure-table schedule, including its normal
        // bounded deferred writes. Explicit policy/budgets also retain their
        // schedule if a scanner or contested failure writer is abandoned.
        self != &Self::default()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Phase {
    Connecting,
    Backoff,
    Interrupted,
    Exhausted,
    Recovered,
    Cancelled,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Reason {
    AttemptStarted,
    ConnectionFailed,
    AttemptInterrupted,
    AttemptsExhausted,
    TotalExhausted,
    Recovered,
    IdentityChanged,
    Detached,
    Stopped,
    Cleared,
    Shutdown,
}

#[derive(Clone, Copy, Debug)]
pub(super) struct Snapshot {
    pub(super) schedule: ScheduleProvenance,
    pub(super) phase: Phase,
    pub(super) attempts: u32,
    pub(super) failures: u32,
    pub(super) first_attempt: Instant,
    pub(super) deadline: Option<Instant>,
    pub(super) next_attempt: Option<Instant>,
    pub(super) last_class: Option<ErrorClass>,
    pub(super) sequence: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum ScheduleProvenance {
    /// This retained episode owns cumulative counters and the retry clock.
    Episode,
    /// The legacy failure table owns the schedule. This transient admission
    /// snapshot does not establish cumulative counters or the next retry time.
    LegacyFailureTable,
}

struct Episode {
    identity: AttemptIdentity,
    ticket: Ticket,
    slot: super::failure_slot::Owner,
    version: Arc<()>,
    snapshot: Snapshot,
    pending_event: Option<Reason>,
}

pub(super) struct Ledger {
    policy: RetryPolicy,
    state: Mutex<State>,
    bank: Arc<Semaphore>,
    _storage: OwnedSemaphorePermit,
}

pub(super) type LedgerOwner = super::shared_value::Shared<Ledger>;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Admission {
    Ready,
    Waiting,
    Exhausted,
}

impl Ledger {
    pub(super) fn with_metadata(policy: RetryPolicy, bank: Arc<Semaphore>) -> Result<LedgerOwner> {
        let storage = super::admission::known_metadata_charge(&bank, std::mem::size_of::<Self>())?;
        Ok(LedgerOwner::new(Self {
            policy,
            state: Mutex::new(State::new(&bank)),
            bank,
            _storage: storage,
        }))
    }

    pub(super) fn policy(&self) -> &RetryPolicy {
        &self.policy
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, State> {
        self.state
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
    }

    fn same(episode: &Episode, identity: &AttemptIdentity, ticket: &Ticket) -> bool {
        episode.identity == *identity && episode.ticket.same_incarnation(ticket)
    }

    /// Called inside the current ticket's commit, before any owner/Source work.
    /// Does not retain or reset an identity merely because discovery succeeded.
    pub(super) fn admission(
        &self,
        udid: &str,
        identity: &AttemptIdentity,
        ticket: &Ticket,
    ) -> Admission {
        let mut state = self.lock();
        if state.sealed {
            return Admission::Exhausted;
        }
        let Some(row) = state
            .row_mut(udid)
            .filter(|row| Self::same(row, identity, ticket))
        else {
            return Admission::Ready;
        };
        let now = Instant::now();
        let reason = if row.snapshot.deadline.is_some_and(|at| now >= at) {
            Some(Reason::TotalExhausted)
        } else if self
            .policy
            .max_attempts
            .is_some_and(|limit| row.snapshot.attempts >= limit.get())
        {
            Some(Reason::AttemptsExhausted)
        } else {
            None
        };
        if let Some(reason) = reason {
            if row.snapshot.phase != Phase::Exhausted {
                row.snapshot.phase = Phase::Exhausted;
                row.snapshot.next_attempt = None;
                row.snapshot.sequence = row.snapshot.sequence.saturating_add(1);
                row.pending_event = Some(reason);
            }
            return Admission::Exhausted;
        }
        if self.policy.authoritative_schedule()
            && row.snapshot.next_attempt.is_some_and(|at| now < at)
        {
            Admission::Waiting
        } else {
            Admission::Ready
        }
    }

    /// Reuse the same bounded failure permit even when the failure writer never
    /// committed. It cannot be freed/reacquired to evade an episode's budget.
    pub(super) fn slot(&self, udid: &str) -> Option<super::failure_slot::Owner> {
        let state = self.lock();
        state
            .row(udid)
            .map(|row| row.slot.clone())
            .or_else(|| state.cause(udid).map(|receipt| receipt.slot()))
    }

    /// Event formatting/subscribers run after every lifecycle watch guard is
    /// released. At most one unreported transition is held per bounded row.
    pub(super) fn emit_pending(&self, udid: &str, ticket: &Ticket) {
        let event = {
            let mut state = self.lock();
            state
                .row_mut(udid)
                .filter(|row| row.ticket.same_incarnation(ticket))
                .and_then(|row| {
                    row.pending_event
                        .take()
                        .map(|reason| (row.snapshot, reason))
                })
        };
        if let Some((snapshot, reason)) = event {
            emit(udid, snapshot, reason);
        }
    }

    pub(super) fn deadline(
        &self,
        udid: &str,
        identity: &AttemptIdentity,
        ticket: &Ticket,
    ) -> Option<Instant> {
        self.lock()
            .row(udid)
            .filter(|row| Self::same(row, identity, ticket))
            .and_then(|row| row.snapshot.deadline)
    }

    pub(super) fn version(&self, udid: &str, ticket: &Ticket) -> Option<Arc<()>> {
        let state = self.lock();
        state
            .row(udid)
            .filter(|row| row.ticket.same_incarnation(ticket))
            .map(|row| Arc::clone(&row.version))
            .or_else(|| {
                state
                    .cause(udid)
                    .filter(|receipt| receipt.same_ticket(ticket))
                    .map(|receipt| receipt.version())
            })
    }

    /// An explicitly requested successful connection also ends the prior
    /// automatic failure episode, but cannot clear an independently newer row.
    pub(super) fn recovered(&self, udid: &str, ticket: &Ticket, version: &Arc<()>) {
        let mut removed = None;
        let mut cause = None;
        let _ = ticket.commit(|| {
            let mut state = self.lock();
            if state.row(udid).is_some_and(|row| {
                row.ticket.same_incarnation(ticket) && Arc::ptr_eq(&row.version, version)
            }) {
                removed = state.remove_row(udid);
            }
            if state.cause(udid).is_some_and(|receipt| {
                receipt.same_ticket(ticket)
                    && (Arc::ptr_eq(&receipt.version(), version) || removed.is_some())
            }) {
                cause = state.remove_cause(udid);
            }
            state.release_empty_backing();
            Ok(())
        });
        if let Some(node) = removed {
            emit(
                udid,
                node.row.as_ref().expect("episode node").snapshot,
                Reason::Recovered,
            );
        }
        drop(cause);
    }
}

impl LedgerOwner {
    /// Admission is synchronous immediately before the Source future is polled.
    /// Owner reservation and connection semaphore admission already succeeded.
    /// Caller holds the ticket's commit guard, never the asynchronous tables.
    pub(super) fn start<I: std::borrow::Borrow<AttemptIdentity>>(
        &self,
        udid: &str,
        identity: I,
        ticket: &Ticket,
        slot: super::failure_slot::Owner,
    ) -> Result<Attempt> {
        // Production supplies the already-admitted shared node identity. Keep
        // that owner alive until every independent copy has its own fee.
        let identity = identity.borrow();
        if self.admission(udid, identity, ticket) != Admission::Ready {
            return Err(crate::output::resource_decision(
                "tunnel retry episode is not admitted",
            ));
        }
        let now = Instant::now();
        let deadline = self
            .policy
            .total
            .map(|budget| {
                now.checked_add(budget)
                    .ok_or_else(|| crate::output::input_decision("tunnel retry deadline overflow"))
            })
            .transpose()?;
        let mut state = self.lock();
        if state.sealed {
            return Err(super::lifecycle::cancelled());
        }
        let same = state
            .row(udid)
            .is_some_and(|row| Ledger::same(row, identity, ticket));
        if !same && state.row(udid).is_none() && state.rows.len() >= MAX_FAILURE_RECORDS {
            return Err(crate::output::resource_decision(
                "tunnel retry episode capacity",
            ));
        }
        // Admit actual table backing, capsule payloads and every copied field
        // before advancing the episode or polling an irreplaceable Source.
        if state.row(udid).is_none() {
            state.rows.reserve(1)?;
        }
        state.causes.reserve(1)?;
        let version = Arc::new(());
        let slot = state
            .row(udid)
            .filter(|_| same)
            .map_or_else(|| slot, |row| row.slot.clone());
        let receipt = FailureReceipt::reserve(
            udid,
            identity,
            ticket,
            slot.clone(),
            Arc::clone(&version),
            &self.bank,
        )?;
        let cause_node = Removed::receipt(receipt.clone(), &self.bank)?;
        let replacement = if !same {
            Some(Removed::episode(
                udid,
                identity,
                |identity| Episode {
                    identity,
                    ticket: ticket.clone(),
                    slot,
                    version: Arc::clone(&version),
                    snapshot: Snapshot {
                        schedule: if self.policy.authoritative_schedule() {
                            ScheduleProvenance::Episode
                        } else {
                            ScheduleProvenance::LegacyFailureTable
                        },
                        phase: Phase::Connecting,
                        attempts: 0,
                        failures: 0,
                        first_attempt: now,
                        deadline,
                        next_attempt: None,
                        last_class: None,
                        sequence: 0,
                    },
                    pending_event: None,
                },
                &self.bank,
            )?)
        } else {
            None
        };
        let displaced = if let Some(replacement) = replacement {
            if let Some(index) = state.row_index(udid) {
                let old = state.rows.remove(index);
                state.rows.insert_reserved(index, replacement);
                Some(old)
            } else {
                state.rows.push_reserved(replacement);
                None
            }
        } else {
            None
        };
        state.causes.push_reserved(cause_node);
        let row = state.row_mut(udid).expect("admitted episode");
        row.version = version;
        row.snapshot.attempts = row.snapshot.attempts.saturating_add(1);
        row.snapshot.phase = Phase::Connecting;
        row.snapshot.next_attempt = None;
        row.snapshot.sequence = row.snapshot.sequence.saturating_add(1);
        let snapshot = row.snapshot;
        drop(state);
        Ok(Attempt {
            ledger: self.clone(),
            receipt,
            started: snapshot,
            displaced,
            complete: false,
        })
    }
}

impl Ledger {
    #[cfg(test)]
    pub(super) fn snapshot(&self, udid: &str) -> Option<Snapshot> {
        self.lock().row(udid).map(|row| row.snapshot)
    }

    fn finish(
        &self,
        attempt: &Attempt,
        success: bool,
        class: Option<ErrorClass>,
        interrupted: bool,
    ) {
        // The caller may be a future Drop; retain no watch guard while emitting
        // events or releasing old coordinator/permit ownership.
        let mut removed = None;
        let mut removed_cause = None;
        let mut event = None;
        let committed = attempt.receipt.ticket.commit(|| {
            let mut state = self.lock();
            let Some(row) = state
                .row_mut(&attempt.receipt.udid)
                .filter(|row| Arc::ptr_eq(&row.version, &attempt.receipt.version))
            else {
                return Ok(());
            };
            if success {
                removed = state.remove_row(&attempt.receipt.udid);
                if state.cause(&attempt.receipt.udid).is_some_and(|receipt| {
                    receipt.same_identity(&attempt.receipt.identity, &attempt.receipt.ticket)
                }) {
                    removed_cause = state.remove_cause(&attempt.receipt.udid);
                }
                return Ok(());
            }
            let now = Instant::now();
            let total_expired = row.snapshot.deadline.is_some_and(|at| now >= at);
            // An admitted Source future may be dropped by the outer timer. An
            // expired current episode then ends with one local budget Timeout,
            // not an invented native Elapsed or acquired Source error. A ready
            // native Err keeps its own class; early Drop stays Interrupted.
            row.snapshot.last_class = if interrupted && total_expired {
                Some(ErrorClass::Timeout)
            } else {
                class.or(row.snapshot.last_class)
            };
            if !interrupted || total_expired {
                row.snapshot.failures = row.snapshot.failures.saturating_add(1);
            }
            let reason = if total_expired {
                row.snapshot.phase = Phase::Exhausted;
                row.snapshot.next_attempt = None;
                Reason::TotalExhausted
            } else if self
                .policy
                .max_attempts
                .is_some_and(|limit| row.snapshot.attempts >= limit.get())
            {
                row.snapshot.phase = Phase::Exhausted;
                row.snapshot.next_attempt = None;
                Reason::AttemptsExhausted
            } else {
                row.snapshot.phase = if interrupted {
                    Phase::Interrupted
                } else {
                    Phase::Backoff
                };
                row.snapshot.next_attempt =
                    now.checked_add(self.policy.delay(row.snapshot.failures.max(1)));
                if interrupted {
                    Reason::AttemptInterrupted
                } else {
                    Reason::ConnectionFailed
                }
            };
            row.snapshot.sequence = row.snapshot.sequence.saturating_add(1);
            event = Some((row.snapshot, reason));
            // Default scans retain only their existing failure-table owner.
            // A failed/abandoned deferred write must still release its permit.
            if !self.policy.authoritative_schedule() {
                removed = state.remove_row(&attempt.receipt.udid);
            }
            Ok(())
        });
        if committed.is_err() {
            let mut state = self.lock();
            if state
                .row(&attempt.receipt.udid)
                .is_some_and(|row| Arc::ptr_eq(&row.version, &attempt.receipt.version))
            {
                removed = state.remove_row(&attempt.receipt.udid);
            }
            // Removal of this exact obsolete row cannot touch a fresh epoch.
            // No cancelled attempt invents a connection failure or new budget.
            event = removed.as_ref().map(|node| {
                (
                    node.row.as_ref().expect("episode node").snapshot,
                    Reason::Stopped,
                )
            });
        }
        if success && committed.is_ok() {
            if let Some(row) = &removed {
                emit(
                    &attempt.receipt.udid,
                    row.row.as_ref().expect("episode node").snapshot,
                    Reason::Recovered,
                );
            }
        }
        if let Some((snapshot, reason)) = event {
            emit(&attempt.receipt.udid, snapshot, reason);
        }
        // An interrupted/successful Source produced no original. Remove only
        // its exact pending reservation; acquired active/retired roots survive.
        let pending = {
            let mut state = self.lock();
            let pending = state.remove_pending(&attempt.receipt);
            state.release_empty_backing();
            pending
        };
        drop(pending);
        drop(removed_cause);
    }

    /// Every removed owner carries its pre-Source node and byte reservation.
    pub(super) fn remove_where(
        &self,
        predicate: impl Fn(&str, &AttemptIdentity, &Ticket) -> bool,
    ) -> RemovedBatch {
        let mut state = self.lock();
        state.select(&predicate);
        let removed = state.remove_selected();
        state.release_empty_backing();
        removed
    }

    /// Reconciliation restores the same nodes at exact original positions if
    /// synchronous predicate/removal work crosses the whole-scan deadline.
    pub(super) fn reconcile(
        &self,
        clock: &super::scan::Deadline,
        predicate: impl Fn(&str, &AttemptIdentity, &Ticket) -> bool,
    ) -> Result<RemovedBatch> {
        let mut state = self.lock();
        state.select(&predicate);
        clock.check()?;
        let removed = state.remove_selected();
        if let Err(error) = clock.check() {
            state.restore(removed);
            return Err(error);
        }
        state.release_empty_backing();
        Ok(removed)
    }

    pub(super) fn cancelled(rows: RemovedBatch, reason: Reason) {
        for removed in rows {
            if let Some(row) = removed.row.as_ref() {
                emit(&removed.udid, row.snapshot, reason);
            }
        }
    }

    pub(super) fn close(&self) {
        let removed = {
            let mut state = self.lock();
            state.sealed = true;
            state.select(&|_, _, _| true);
            let removed = state.remove_selected();
            state.release_empty_backing();
            removed
        };
        Self::cancelled(removed, Reason::Shutdown);
    }

    #[cfg(test)]
    pub(super) fn structural_bytes() -> usize {
        std::mem::size_of::<Self>()
    }

    /// First insertion uses KnownVec's two-slot actual requested backing. The
    /// copied selectors in this fixture have exactly their length as capacity.
    #[cfg(test)]
    pub(super) fn initial_attempt_bytes(udid: &str, identity: &AttemptIdentity) -> usize {
        4 * std::mem::size_of::<OwnedRemoved>()
            + 2 * std::mem::size_of::<Removed>()
            + udid.len()
            + super::candidate_input::route_length(&identity.route)
            + FailureReceipt::initial_bytes(udid, identity)
    }
    #[cfg(test)]
    pub(super) fn len(&self) -> usize {
        self.lock().rows.len()
    }
}

pub(super) struct Attempt {
    ledger: LedgerOwner,
    receipt: Receipt,
    started: Snapshot,
    displaced: Option<OwnedRemoved>,
    complete: bool,
}
impl Attempt {
    pub(super) fn source_failed(&self, original: anyhow::Error) -> anyhow::Error {
        self.ledger.capture_source(self, original)
    }
    pub(super) fn deadline(&self) -> Option<Instant> {
        self.started.deadline
    }

    pub(super) fn emit_start(&self) {
        if let Some(old) = &self.displaced {
            emit(
                &self.receipt.udid,
                old.row.as_ref().expect("episode node").snapshot,
                Reason::IdentityChanged,
            );
        }
        emit(&self.receipt.udid, self.started, Reason::AttemptStarted);
    }

    pub(super) fn finish<T>(mut self, result: &Result<T>) {
        self.ledger.finish(
            &self,
            result.is_ok(),
            result
                .as_ref()
                .err()
                .map(|error| crate::output::operation_error_class(error.as_ref())),
            false,
        );
        self.complete = true;
    }
}

impl Drop for Attempt {
    fn drop(&mut self) {
        if !self.complete {
            self.ledger.finish(self, false, None, true);
        }
    }
}

fn emit(udid: &str, mut snapshot: Snapshot, reason: Reason) {
    if reason == Reason::Recovered {
        snapshot.phase = Phase::Recovered;
    }
    if matches!(
        reason,
        Reason::IdentityChanged
            | Reason::Detached
            | Reason::Stopped
            | Reason::Cleared
            | Reason::Shutdown
    ) {
        snapshot.phase = Phase::Cancelled;
    }
    if snapshot.schedule == ScheduleProvenance::LegacyFailureTable {
        // A default attempt releases its row before deferred failure-table
        // bookkeeping. Do not invent episode counters or an uncommitted delay
        // from this one attempt's temporary snapshot.
        tracing::info!(
            device = %ios_core::redaction::diagnostic_id(udid),
            schedule_source = "legacy_failure_table",
            state = ?snapshot.phase,
            reason = ?reason,
            error_class = ?snapshot.last_class,
            "tunnel reconnect state"
        );
        return;
    }
    let now = Instant::now();
    tracing::info!(
        device = %ios_core::redaction::diagnostic_id(udid),
        schedule_source = "episode",
        state = ?snapshot.phase,
        reason = ?reason,
        attempt = snapshot.attempts,
        failures = snapshot.failures,
        sequence = snapshot.sequence,
        elapsed_ms = now.saturating_duration_since(snapshot.first_attempt).as_millis() as u64,
        remaining_ms = snapshot.deadline.map(|at| at.saturating_duration_since(now).as_millis() as u64),
        next_delay_ms = snapshot.next_attempt.map(|at| at.saturating_duration_since(now).as_millis() as u64),
        error_class = ?snapshot.last_class,
        "tunnel reconnect state"
    );
}
