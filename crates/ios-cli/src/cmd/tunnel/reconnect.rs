//! Automatic connection episodes, independent of contested failure-table writes.
//!
//! A row holds the original coordinator and failure reservation. There is one
//! row per admitted device, no task or event queue, and no eviction that could
//! silently restart an exhausted budget. Original native owners remain in the
//! existing retirement registry; this ledger never stops or releases them.

use std::collections::HashMap;
use std::num::{NonZeroU32, NonZeroU64};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use anyhow::Result;
use ios_core::ErrorClass;
use tokio::sync::OwnedSemaphorePermit;
use tokio::time::Instant;

use super::{AttemptIdentity, Ticket, MAX_FAILURE_RECORDS};

const INITIAL_DELAY: Duration = Duration::from_secs(30);
const MAX_DELAY: Duration = Duration::from_secs(300);

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
    slot: Arc<OwnedSemaphorePermit>,
    version: Arc<()>,
    snapshot: Snapshot,
    pending_event: Option<Reason>,
}

struct State {
    sealed: bool,
    rows: HashMap<String, Episode>,
}

pub(super) struct Ledger {
    policy: RetryPolicy,
    state: Mutex<State>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Admission {
    Ready,
    Waiting,
    Exhausted,
}

impl Ledger {
    pub(super) fn new(policy: RetryPolicy) -> Arc<Self> {
        Arc::new(Self {
            policy,
            state: Mutex::new(State {
                sealed: false,
                rows: HashMap::new(),
            }),
        })
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
            .rows
            .get_mut(udid)
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
    pub(super) fn slot(&self, udid: &str) -> Option<Arc<OwnedSemaphorePermit>> {
        self.lock().rows.get(udid).map(|row| Arc::clone(&row.slot))
    }

    /// Event formatting/subscribers run after every lifecycle watch guard is
    /// released. At most one unreported transition is held per bounded row.
    pub(super) fn emit_pending(&self, udid: &str, ticket: &Ticket) {
        let event = {
            let mut state = self.lock();
            state
                .rows
                .get_mut(udid)
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
            .rows
            .get(udid)
            .filter(|row| Self::same(row, identity, ticket))
            .and_then(|row| row.snapshot.deadline)
    }

    pub(super) fn version(&self, udid: &str, ticket: &Ticket) -> Option<Arc<()>> {
        self.lock()
            .rows
            .get(udid)
            .filter(|row| row.ticket.same_incarnation(ticket))
            .map(|row| Arc::clone(&row.version))
    }

    /// An explicitly requested successful connection also ends the prior
    /// automatic failure episode, but cannot clear an independently newer row.
    pub(super) fn recovered(&self, udid: &str, ticket: &Ticket, version: &Arc<()>) {
        let mut removed = None;
        let _ = ticket.commit(|| {
            let mut state = self.lock();
            if state.rows.get(udid).is_some_and(|row| {
                row.ticket.same_incarnation(ticket) && Arc::ptr_eq(&row.version, version)
            }) {
                removed = state.rows.remove(udid);
            }
            Ok(())
        });
        if let Some(row) = removed {
            emit(udid, row.snapshot, Reason::Recovered);
        }
    }

    /// Admission is synchronous immediately before the Source future is polled.
    /// Owner reservation and connection semaphore admission already succeeded.
    /// Caller holds the ticket's commit guard, never the asynchronous tables.
    pub(super) fn start(
        self: &Arc<Self>,
        udid: &str,
        identity: AttemptIdentity,
        ticket: &Ticket,
        slot: Arc<OwnedSemaphorePermit>,
    ) -> Result<Attempt> {
        if self.admission(udid, &identity, ticket) != Admission::Ready {
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
            .rows
            .get(udid)
            .is_some_and(|row| Self::same(row, &identity, ticket));
        if !same && !state.rows.contains_key(udid) && state.rows.len() >= MAX_FAILURE_RECORDS {
            return Err(crate::output::resource_decision(
                "tunnel retry episode capacity",
            ));
        }
        let displaced = if !same {
            state.rows.insert(
                udid.to_owned(),
                Episode {
                    identity,
                    ticket: ticket.clone(),
                    slot,
                    version: Arc::new(()),
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
            )
        } else {
            None
        };
        let row = state.rows.get_mut(udid).expect("admitted episode");
        row.version = Arc::new(());
        row.snapshot.attempts = row.snapshot.attempts.saturating_add(1);
        row.snapshot.phase = Phase::Connecting;
        row.snapshot.next_attempt = None;
        row.snapshot.sequence = row.snapshot.sequence.saturating_add(1);
        let snapshot = row.snapshot;
        let version = Arc::clone(&row.version);
        drop(state);
        Ok(Attempt {
            ledger: Arc::clone(self),
            udid: udid.to_owned(),
            ticket: ticket.clone(),
            version,
            started: snapshot,
            displaced,
            complete: false,
        })
    }

    #[cfg(test)]
    pub(super) fn snapshot(&self, udid: &str) -> Option<Snapshot> {
        self.lock().rows.get(udid).map(|row| row.snapshot)
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
        let mut event = None;
        let committed = attempt.ticket.commit(|| {
            let mut state = self.lock();
            let Some(row) = state
                .rows
                .get_mut(&attempt.udid)
                .filter(|row| Arc::ptr_eq(&row.version, &attempt.version))
            else {
                return Ok(());
            };
            if success {
                removed = state.rows.remove(&attempt.udid);
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
                removed = state.rows.remove(&attempt.udid);
            }
            Ok(())
        });
        if committed.is_err() {
            let mut state = self.lock();
            if state
                .rows
                .get(&attempt.udid)
                .is_some_and(|row| Arc::ptr_eq(&row.version, &attempt.version))
            {
                removed = state.rows.remove(&attempt.udid);
            }
            // Removal of this exact obsolete row cannot touch a fresh epoch.
            // No cancelled attempt invents a connection failure or new budget.
            event = removed.as_ref().map(|row| (row.snapshot, Reason::Stopped));
        }
        if success && committed.is_ok() {
            if let Some(row) = &removed {
                emit(&attempt.udid, row.snapshot, Reason::Recovered);
            }
        }
        if let Some((snapshot, reason)) = event {
            emit(&attempt.udid, snapshot, reason);
        }
    }

    /// Return displaced rows so their permits/gates drop outside epoch borrows.
    pub(super) fn remove_where(
        &self,
        predicate: impl Fn(&str, &AttemptIdentity, &Ticket) -> bool,
    ) -> Vec<Removed> {
        let mut state = self.lock();
        let keys = state
            .rows
            .iter()
            .filter(|(udid, row)| predicate(udid, &row.identity, &row.ticket))
            .map(|(udid, _)| udid.clone())
            .collect::<Vec<_>>();
        keys.into_iter()
            .filter_map(|udid| state.rows.remove(&udid).map(|row| Removed { udid, row }))
            .collect()
    }

    /// Reconciliation shares the whole scan clock. Restore exact unobserved
    /// episodes before unlocking if synchronous filtering/removal runs late.
    pub(super) fn reconcile(
        &self,
        clock: &super::scan::Deadline,
        predicate: impl Fn(&str, &AttemptIdentity, &Ticket) -> bool,
    ) -> Result<Vec<Removed>> {
        let mut state = self.lock();
        let keys = state
            .rows
            .iter()
            .filter(|(udid, row)| predicate(udid, &row.identity, &row.ticket))
            .map(|(udid, _)| udid.clone())
            .collect::<Vec<_>>();
        clock.check()?;
        let removed = keys
            .into_iter()
            .filter_map(|udid| state.rows.remove(&udid).map(|row| Removed { udid, row }))
            .collect::<Vec<_>>();
        if let Err(error) = clock.check() {
            for removed in removed {
                state.rows.insert(removed.udid, removed.row);
            }
            return Err(error);
        }
        Ok(removed)
    }

    pub(super) fn cancelled(rows: Vec<Removed>, reason: Reason) {
        for removed in rows {
            emit(&removed.udid, removed.row.snapshot, reason);
        }
    }

    pub(super) fn close(&self) {
        let rows = {
            let mut state = self.lock();
            state.sealed = true;
            std::mem::take(&mut state.rows)
        };
        for (udid, row) in rows {
            emit(&udid, row.snapshot, Reason::Shutdown);
        }
    }

    #[cfg(test)]
    pub(super) fn len(&self) -> usize {
        self.lock().rows.len()
    }
}

pub(super) struct Removed {
    udid: String,
    row: Episode,
}

pub(super) struct Attempt {
    ledger: Arc<Ledger>,
    udid: String,
    ticket: Ticket,
    version: Arc<()>,
    started: Snapshot,
    displaced: Option<Episode>,
    complete: bool,
}

impl Attempt {
    pub(super) fn deadline(&self) -> Option<Instant> {
        self.started.deadline
    }

    pub(super) fn emit_start(&self) {
        if let Some(old) = &self.displaced {
            emit(&self.udid, old.snapshot, Reason::IdentityChanged);
        }
        emit(&self.udid, self.started, Reason::AttemptStarted);
    }

    pub(super) fn finish(mut self, result: &Result<super::TunnelRecord>) {
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
