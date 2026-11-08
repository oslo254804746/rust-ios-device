//! CLI-private bounded ownership of original native tunnel join capabilities.
//!
//! No task is spawned. Native shutdown never runs under the table,
//! lifecycle watch borrows or this registry's synchronous mutex.

use super::ControlIdentity;
use std::error::Error;
use std::future::{pending, poll_fn, Future};
use std::num::NonZeroU64;
#[cfg(test)]
use std::num::NonZeroUsize;
use std::sync::{Arc, Mutex as StdMutex, MutexGuard as StdMutexGuard, OnceLock};
use std::task::Poll;
use std::time::Duration;

use anyhow::Result;
use ios_core::{ErrorClass, TunnelError, TunnelHandle, TunnelShutdownFailure};
use tokio::sync::{Mutex, Notify, OwnedSemaphorePermit, Semaphore};
use tokio::time::{error::Elapsed, Instant};

pub(super) const MAX_NATIVE_OWNERS: usize = 4096;
pub(super) struct NativeReport {
    original: OnceLock<TunnelError>,
}

type NativeReportOwner = super::shared_allocation::Shared<NativeReport>;

impl NativeReport {
    pub(super) fn body_bytes() -> usize {
        NativeReportOwner::allocation_bytes()
    }

    fn empty(storage: OwnedSemaphorePermit) -> Result<NativeReportOwner> {
        let bank = storage.semaphore().clone();
        NativeReportOwner::from_grant(
            Self {
                original: OnceLock::new(),
            },
            &bank,
            storage,
        )
    }

    fn capture(&self, original: TunnelError) {
        assert!(
            self.original.set(original).is_ok(),
            "native history root captured once"
        );
    }

    fn error(&self) -> &TunnelError {
        self.original.get().expect("captured native history")
    }
}

// A census owns each fresh immutable native snapshot in its already-admitted
// array slot. Historical roots retain their existing independent shared owners.
pub(super) enum PayloadOutcome {
    History(NativeReportOwner),
    Current(TunnelError),
}

impl std::ops::Deref for PayloadOutcome {
    type Target = TunnelError;
    fn deref(&self) -> &TunnelError {
        match self {
            Self::History(original) => original.error(),
            Self::Current(original) => original,
        }
    }
}
impl AsRef<TunnelError> for PayloadOutcome {
    fn as_ref(&self) -> &TunnelError {
        self
    }
}

/// Only call with this original handle's directly returned shutdown error.
/// First outer typed report wins, including false: a foreign report nested in
/// the native primary must never certify the current runtime. No source text,
/// liveness bit, pending count alone or AbortHandle completion is evidence.
fn shutdown_report(error: &TunnelError) -> Option<&TunnelShutdownFailure> {
    let mut current: Option<&(dyn Error + 'static)> = Some(error);
    for _ in 0..32 {
        let Some(cause) = current else { break };
        if let Some(report) = cause.downcast_ref::<TunnelShutdownFailure>() {
            return Some(report);
        }
        current = if let Some(io) = cause.downcast_ref::<std::io::Error>() {
            io.get_ref()
                .map(|payload| payload as &(dyn Error + 'static))
                .or_else(|| cause.source())
        } else {
            cause.source()
        };
    }
    None
}

fn shutdown_joins_certified(error: &TunnelError) -> bool {
    shutdown_report(error).is_some_and(TunnelShutdownFailure::joins_complete)
}

pub(super) fn shutdown_primary_is_elapsed(error: &TunnelError) -> bool {
    shutdown_report(error).is_some_and(|report| report.primary().is::<Elapsed>())
}

/// Opaque lifecycle identities, never a UDID, endpoint or runtime identity.
#[derive(Clone)]
pub(super) struct OwnershipStamp {
    global: ControlIdentity,
    device: ControlIdentity,
}

impl OwnershipStamp {
    pub(super) fn new(global: ControlIdentity, device: ControlIdentity) -> Self {
        Self { global, device }
    }

    pub(super) fn standalone(
        registry: &RegistryOwner,
        births: &mut super::marker_identity::Cutover<2>,
    ) -> Self {
        Self::new(
            births.place(&registry.byte_bank),
            births.place(&registry.byte_bank),
        )
    }
}

type Incarnation = super::shared_allocation::Shared<()>;

#[derive(Clone)]
struct Key {
    slot: usize,
    incarnation: Incarnation,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Phase {
    Reserved,
    Inflight,
    Unpublished,
    Active,
    Retired,
}

struct Entry {
    incarnation: Incarnation,
    stamp: OwnershipStamp,
    phase: Phase,
    retired_at: Option<NonZeroU64>,
    handle: Option<Arc<TunnelHandle>>,
    // An admitted device's known metadata charge remains with the original
    // runtime until its joins are certified, even after its table row drops.
    metadata: Option<super::metadata_lease::Owner>,
    global_metadata: Option<super::metadata_lease::Owner>,
    _input: Option<super::candidate_input::Owner>,
    fresh_metadata: Option<super::freshness::MetadataOwner>,
    first_freshness: Option<super::freshness::ReceiptOwner>,
    latest_freshness: Option<super::freshness::ReceiptOwner>,
    failure_observed: bool,
}

struct State {
    sealed: bool,
    // Never reset/refund these counters, even when every slot is empty.
    issued: u64,
    retired: u64,
    // Fixed length after construction. Never push, evict or resize.
    slots: Vec<Option<Entry>>,
    first_failure: Option<NativeReportOwner>,
    first_expiry: Option<NativeReportOwner>,
    first_orphan_freshness: Option<super::freshness::ReceiptOwner>,
    additional_failed_owners: u64,
}

#[derive(Clone, Copy)]
struct Selection {
    cutoff: u64,
    count: usize,
    capacity: usize,
}

impl Selection {
    fn slots(self) -> std::ops::Range<usize> {
        0..if self.count == 0 { 0 } else { self.capacity }
    }

    fn contains(self, entry: &Entry) -> bool {
        entry.phase == Phase::Retired && entry.retired_at.is_some_and(|at| at.get() <= self.cutoff)
    }
}

impl State {
    fn retire_slot(&mut self, slot: usize) {
        let entry = self.slots[slot]
            .as_mut()
            .expect("selected occupied native slot");
        if entry.phase != Phase::Retired {
            // Each issued incarnation can retire only once. Admission rejects
            // issued overflow before Source; therefore this cannot wrap or fail
            // during infallible cleanup, even when issued == u64::MAX.
            self.retired = self
                .retired
                .checked_add(1)
                .expect("one retirement per issued incarnation");
            assert!(self.retired <= self.issued);
            entry.retired_at = NonZeroU64::new(self.retired);
            entry.phase = Phase::Retired;
        } else {
            debug_assert!(entry.retired_at.is_some());
        }
    }

    fn selection(&self) -> Selection {
        Selection {
            cutoff: self.retired,
            count: self
                .slots
                .iter()
                .flatten()
                .filter(|entry| entry.phase == Phase::Retired)
                .count(),
            capacity: self.slots.len(),
        }
    }
}

/// Snapshot is identity-free. A retired count is ownership, not native task count.
#[derive(Clone, Copy, Debug)]
pub(super) struct Snapshot {
    pub(super) sealed: bool,
    pub(super) capacity: usize,
    pub(super) reserved: usize,
    pub(super) inflight: usize,
    pub(super) unpublished: usize,
    pub(super) active: usize,
    pub(super) retired: usize,
    pub(super) native_handles: usize,
    pub(super) historical_failure_class: Option<ErrorClass>,
    pub(super) additional_failed_owners: u64,
}

impl Snapshot {
    pub(super) fn occupied(&self) -> usize {
        // Each count partitions a single fixed-capacity vector.
        self.reserved + self.inflight + self.unpublished + self.active + self.retired
    }

    pub(super) fn closed_and_empty(&self) -> bool {
        self.sealed && self.occupied() == 0
    }
}

/// One checked absolute clock for mutex waits, native joins and server cleanup.
#[derive(Clone, Copy, Debug)]
pub(super) struct CleanupClock {
    deadline: Instant,
}

impl CleanupClock {
    pub(super) fn new(budget: Duration) -> Result<Self> {
        let deadline = Instant::now().checked_add(budget).ok_or_else(|| {
            crate::output::input_decision("native retirement cleanup deadline overflow")
        })?;
        Ok(Self { deadline })
    }

    pub(super) fn deadline(&self) -> Instant {
        self.deadline
    }

    /// Callers persist phase outcomes before completing work, so a Ready poll
    /// crossing the clock cannot discard an already acquired native cause.
    pub(super) async fn run<T>(self, work: impl Future<Output = T>) -> Result<T, Elapsed> {
        tokio::pin!(work);
        let guarded = poll_fn(|cx| {
            if Instant::now() >= self.deadline() {
                return Poll::Pending;
            }
            match work.as_mut().poll(cx) {
                Poll::Ready(_) if Instant::now() >= self.deadline => Poll::Pending,
                result => result,
            }
        });
        tokio::select! {
            biased;
            expired = tokio::time::timeout_at(self.deadline, pending::<T>()) => expired,
            result = guarded => Ok(result),
        }
    }

    fn remaining(&self) -> Duration {
        self.deadline.saturating_duration_since(Instant::now())
    }
}

pub(super) struct Registry {
    state: StdMutex<State>,
    drains: Mutex<()>,
    changed: Notify,
    byte_bank: super::birth_bank::BirthBank,
    // The only two immutable historical snapshots are paid before ownership.
    // Current per-attempt snapshots live inline and never reuse these values.
    error_stock: NativeReportOwner,
    expiry_stock: NativeReportOwner,
    // Backing and application-owned payload fees outlive the fields they own.
    _slots_storage: OwnedSemaphorePermit,
    // Historical producer mirrors outlive the ACTUAL whole slots Vec and all
    // earlier Registry fields. Individual slot release never refunds them.
    origins: StdMutex<super::native_origins::Catalogue>,
}

pub(super) type RegistryOwner = super::shared_allocation::Shared<Registry>;

impl Registry {
    pub(super) fn incarnation_bytes() -> usize {
        Incarnation::allocation_bytes()
    }

    pub(super) fn body_bytes() -> usize {
        RegistryOwner::allocation_bytes()
    }
    pub(super) fn new() -> Result<RegistryOwner> {
        Self::with_metadata(&super::birth_bank::BirthBank::new(
            super::MAX_GLOBAL_KNOWN_METADATA_BYTES,
        ))
    }

    pub(super) fn with_metadata(bank: &super::birth_bank::BirthBank) -> Result<RegistryOwner> {
        Self::allocate(bank, MAX_NATIVE_OWNERS)
    }

    fn allocate(bank: &super::birth_bank::BirthBank, capacity: usize) -> Result<RegistryOwner> {
        let body_storage = super::admission::known_metadata_charge(bank, Self::body_bytes())?;
        let mut reports_storage =
            super::admission::known_metadata_charge(bank, 2 * NativeReport::body_bytes())?;
        let allocation = super::metadata_vec::exact_backing(bank, capacity)?;
        let slots_storage = allocation.storage;
        let mut slots = allocation.values;
        slots.resize_with(capacity, || None);
        let error_storage = reports_storage
            .split(NativeReport::body_bytes())
            .expect("prepaid native error root");
        let error_stock = NativeReport::empty(error_storage)?;
        let expiry_stock = NativeReport::empty(reports_storage)?;
        RegistryOwner::from_grant(
            Self {
                state: StdMutex::new(State {
                    sealed: false,
                    issued: 0,
                    retired: 0,
                    slots,
                    first_failure: None,
                    first_expiry: None,
                    first_orphan_freshness: None,
                    additional_failed_owners: 0,
                }),
                drains: Mutex::new(()),
                changed: Notify::new(),
                byte_bank: bank.clone(),
                error_stock,
                expiry_stock,
                _slots_storage: slots_storage,
                origins: StdMutex::new(super::native_origins::Catalogue::default()),
            },
            bank,
            body_storage,
        )
    }

    #[cfg(test)]
    fn small(capacity: NonZeroUsize) -> RegistryOwner {
        assert!(capacity.get() <= MAX_NATIVE_OWNERS);
        Self::allocate(
            &super::birth_bank::BirthBank::new(super::MAX_GLOBAL_KNOWN_METADATA_BYTES),
            capacity.get(),
        )
        .unwrap()
    }

    #[cfg(test)]
    pub(super) fn structural_bytes(&self) -> usize {
        let state = self.lock();
        Self::body_bytes()
            + state.slots.capacity() * std::mem::size_of::<Option<Entry>>()
            + 2 * NativeReport::body_bytes()
            + self
                .origins
                .lock()
                .unwrap_or_else(|poison| poison.into_inner())
                .backing_bytes()
    }

    fn lock(&self) -> StdMutexGuard<'_, State> {
        self.state
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
    }
}

impl RegistryOwner {
    #[cfg(test)]
    pub(super) fn rebind_empty_metadata(
        &mut self,
        bank: &super::birth_bank::BirthBank,
    ) -> Result<()> {
        let current = self
            .get_mut()
            .ok_or_else(|| crate::output::resource_decision("configure registry before sharing"))?;
        let state = current.state.get_mut().unwrap_or_else(|p| p.into_inner());
        anyhow::ensure!(
            !state.sealed
                && current
                    .origins
                    .get_mut()
                    .unwrap_or_else(|poison| poison.into_inner())
                    .is_empty()
                && state.slots.iter().all(Option::is_none)
                && state.first_failure.is_none()
                && state.first_expiry.is_none()
                && state.first_orphan_freshness.is_none()
                && current.error_stock.original.get().is_none()
                && current.expiry_stock.original.get().is_none()
                && current.error_stock.strong_count() == 1
                && current.expiry_stock.strong_count() == 1,
            "native registry bank can only change before ownership/history"
        );
        let mut replacement = super::admission::known_metadata_charge(
            bank,
            Registry::body_bytes()
                + state.slots.capacity() * std::mem::size_of::<Option<Entry>>()
                + 2 * NativeReport::body_bytes(),
        )?;
        let body = replacement
            .split(Registry::body_bytes())
            .expect("whole registry body prepaid");
        let error_body = replacement
            .split(NativeReport::body_bytes())
            .expect("whole native error root prepaid");
        let expiry_body = replacement
            .split(NativeReport::body_bytes())
            .expect("whole native expiry root prepaid");
        // All roots were checked unique/empty before the single whole admission.
        // No step below can reject. Keep OLD original grants through ALL swaps.
        let old_error = current
            .error_stock
            .replace_storage(bank, error_body)
            .expect("prevalidated native error root");
        let old_expiry = current
            .expiry_stock
            .replace_storage(bank, expiry_body)
            .expect("prevalidated native expiry root");
        let old_slots = std::mem::replace(&mut current._slots_storage, replacement);
        current.byte_bank = bank.clone();
        let old_body = self
            .replace_storage(bank, body)
            .expect("prevalidated registry root");
        drop((old_error, old_expiry, old_slots, old_body));
        Ok(())
    }
    pub(super) fn reserve(&self, stamp: OwnershipStamp) -> Result<Reservation> {
        self.reserve_with_input(stamp, None)
    }

    pub(super) fn byte_bank(&self) -> Arc<Semaphore> {
        Arc::clone(&self.byte_bank)
    }

    pub(super) fn prepare_births<const N: usize>(
        &self,
        bank: &Arc<Semaphore>,
    ) -> Result<super::marker_identity::Cutover<N>> {
        super::marker_identity::Cutover::new(&self.byte_bank, bank)
    }

    pub(super) fn reserve_with_input(
        &self,
        stamp: OwnershipStamp,
        input: Option<super::candidate_input::Owner>,
    ) -> Result<Reservation> {
        let mut state = self.lock();
        if state.sealed {
            return Err(super::lifecycle::cancelled());
        }
        let slot = state
            .slots
            .iter()
            .position(Option::is_none)
            .ok_or_else(|| {
                crate::output::resource_decision("native tunnel owner capacity exhausted")
            })?;
        let issued = state.issued.checked_add(1).ok_or_else(|| {
            crate::output::resource_decision("native tunnel incarnation limit exhausted")
        })?;
        // Admit and create the complete private identity before consuming its
        // ordinal or fixed slot. The original grant follows every stale Key
        // through actual joins/slot reuse until this allocation's last owner.
        let storage = super::admission::known_metadata_charge(
            &self.byte_bank,
            Registry::incarnation_bytes(),
        )?;
        let incarnation = Incarnation::from_grant((), &self.byte_bank, storage)?;
        let mut origins = self
            .origins
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        let prepared = origins.prepare(
            &self.byte_bank,
            self._slots_storage.num_permits(),
            [&stamp.global, &stamp.device],
        )?;
        // Every original/new grant and actual backing is ready. No failure or
        // await separates catalogue commit, ordinal consumption and slot fill.
        origins.commit(prepared);
        state.issued = issued;
        state.slots[slot] = Some(Entry {
            incarnation: incarnation.clone(),
            stamp,
            phase: Phase::Reserved,
            retired_at: None,
            handle: None,
            metadata: None,
            global_metadata: None,
            _input: input,
            fresh_metadata: None,
            first_freshness: None,
            latest_freshness: None,
            failure_observed: false,
        });
        Ok(Reservation {
            registry: self.clone(),
            key: Some(Key { slot, incarnation }),
        })
    }
}

impl Registry {
    fn entry_mut<'a>(state: &'a mut State, key: &Key) -> Option<&'a mut Entry> {
        state
            .slots
            .get_mut(key.slot)?
            .as_mut()
            .filter(|entry| entry.incarnation.ptr_eq(&key.incarnation))
    }

    fn retire_matching(&self, matches: impl Fn(&Entry) -> bool, seal: bool) {
        let selected = {
            let mut state = self.lock();
            state.sealed |= seal;
            for slot in 0..state.slots.len() {
                if state.slots[slot].as_ref().is_some_and(&matches) {
                    state.retire_slot(slot);
                }
            }
            state.selection()
        };
        // Mark all matching owners before any native call. Keep the exact
        // namespace predicate; unrelated already-retired owners stay untouched.
        self.stop_selected(selected, matches);
        self.changed.notify_waiters();
    }

    fn stop_selected(&self, selected: Selection, matches: impl Fn(&Entry) -> bool) {
        for slot in selected.slots() {
            let handle = {
                let state = self.lock();
                state.slots[slot]
                    .as_ref()
                    .filter(|entry| selected.contains(entry) && matches(entry))
                    .and_then(|entry| entry.handle.clone())
            };
            if let Some(handle) = handle {
                handle.request_stop();
            }
        }
    }

    /// Call only after lifecycle.close releases its watch writes.
    pub(super) fn close_and_stop_all(&self) {
        self.retire_matching(|_| true, true);
    }

    /// Use the exact OLD global identity returned by the reusable clear fence.
    pub(super) fn retire_global(&self, old: &ControlIdentity) {
        self.retire_matching(
            |entry| ControlIdentity::ptr_eq(&entry.stamp.global, old),
            false,
        );
    }

    /// Use the exact OLD device identity returned by targeted stop's fence.
    pub(super) fn retire_device(&self, old: &ControlIdentity) {
        self.retire_matching(
            |entry| ControlIdentity::ptr_eq(&entry.stamp.device, old),
            false,
        );
    }

    fn retire_key(&self, key: &Key, release_empty: bool) {
        let mut marker_headers = super::marker_identity::ReleaseFence::<2>::new();
        let handle = {
            let mut state = self.lock();
            let Some(entry) = Self::entry_mut(&mut state, key) else {
                return;
            };
            if entry.handle.is_none() && release_empty {
                super::marker_identity::ReleaseMarkers::keep(
                    &mut marker_headers,
                    &entry.stamp.global,
                );
                super::marker_identity::ReleaseMarkers::keep(
                    &mut marker_headers,
                    &entry.stamp.device,
                );
                state.slots[key.slot] = None;
                None
            } else {
                let handle = entry.handle.clone();
                state.retire_slot(key.slot);
                handle
            }
        };
        if let Some(handle) = handle {
            handle.request_stop();
        }
        self.changed.notify_waiters();
    }

    pub(super) fn snapshot(&self) -> Snapshot {
        let state = self.lock();
        let count = |phase| {
            state
                .slots
                .iter()
                .flatten()
                .filter(|entry| entry.phase == phase)
                .count()
        };
        Snapshot {
            sealed: state.sealed,
            capacity: state.slots.len(),
            reserved: count(Phase::Reserved),
            inflight: count(Phase::Inflight),
            unpublished: count(Phase::Unpublished),
            active: count(Phase::Active),
            retired: count(Phase::Retired),
            native_handles: state
                .slots
                .iter()
                .flatten()
                .filter(|entry| entry.handle.is_some())
                .count(),
            historical_failure_class: state
                .first_failure
                .as_ref()
                .map(|error| ios_core::classify_error(error.error())),
            additional_failed_owners: state.additional_failed_owners,
        }
    }

    pub(super) fn payload_outcomes(
        &self,
    ) -> Result<(usize, super::metadata_vec::KnownVec<PayloadOutcome>)> {
        let (mut errors, handles) = {
            let state = self.lock();
            let count = state
                .slots
                .iter()
                .flatten()
                .filter(|entry| entry.handle.is_some())
                .count();
            let history = usize::from(state.first_failure.is_some())
                + usize::from(state.first_expiry.is_some());
            let mut errors = super::metadata_vec::KnownVec::new(
                Arc::clone(&self.byte_bank),
                state.slots.len() + 2,
            );
            errors.reserve(history + count)?;
            let mut handles =
                super::metadata_vec::KnownVec::new(Arc::clone(&self.byte_bank), state.slots.len());
            handles.reserve(count)?;
            // Both actual backings are admitted before cloning original roots.
            for error in state.first_failure.iter().chain(state.first_expiry.iter()) {
                errors.push_reserved(PayloadOutcome::History(error.clone()));
            }
            for handle in state
                .slots
                .iter()
                .flatten()
                .filter_map(|entry| entry.handle.as_ref())
            {
                handles.push_reserved(Arc::clone(handle));
            }
            (errors, handles)
        };
        let mut unjoined = 0;
        for handle in handles {
            if let Err(error) = handle.shutdown_status() {
                let report = shutdown_report(&error);
                if report.map_or(true, |report| !report.joins_complete()) {
                    unjoined += 1;
                }
                if report.is_some_and(TunnelShutdownFailure::has_retained_outcomes) {
                    errors.push_reserved(PayloadOutcome::Current(error));
                }
            }
        }
        Ok((unjoined, errors))
    }

    fn selection_and_stop(&self) -> Selection {
        let selected = self.lock().selection();
        self.stop_selected(selected, |_| true);
        selected
    }

    fn selected_key(&self, selected: Selection, slot: usize) -> Option<Key> {
        let state = self.lock();
        state.slots[slot]
            .as_ref()
            .filter(|entry| selected.contains(entry))
            .map(|entry| Key {
                slot,
                incarnation: entry.incarnation.clone(),
            })
    }

    fn capture_selected(&self, selected: Selection, attempt: Option<&StdMutex<AttemptEvidence>>) {
        for slot in selected.slots() {
            let Some(key) = self.selected_key(selected, slot) else {
                continue;
            };
            let SelectedOwner::Native(handle) = self.selected_owner(&key) else {
                continue;
            };
            if let Err(error) = handle.shutdown_status() {
                if shutdown_report(&error).is_some_and(TunnelShutdownFailure::has_retained_outcomes)
                {
                    let error = self.record_failure(&key, error);
                    if let Some(attempt) = attempt {
                        attempt
                            .lock()
                            .unwrap_or_else(|p| p.into_inner())
                            .record(error);
                    }
                }
            }
        }
    }

    /// Capture acquired native causes before an outer cleanup future disappears.
    /// Existing retired ownership and original join handles remain untouched.
    pub(super) fn capture_retired_outcomes(&self) {
        let selected = self.lock().selection();
        self.capture_selected(selected, None);
    }

    fn selected_owner(&self, key: &Key) -> SelectedOwner {
        let mut state = self.lock();
        match Self::entry_mut(&mut state, key) {
            None => SelectedOwner::Released,
            Some(entry) => match &entry.handle {
                Some(handle) => SelectedOwner::Native(Arc::clone(handle)),
                None => SelectedOwner::AwaitingAttachment,
            },
        }
    }

    fn certify_completed(&self, key: &Key, handle: &Arc<TunnelHandle>) -> bool {
        // Joins certify acquisition of original results, not successful native
        // undo. In particular, retain the same owner after a cleanup-body panic.
        if handle.has_pending_native_cleanup() {
            return false;
        }
        let mut marker_headers = super::marker_identity::ReleaseFence::<2>::new();
        let mut state = self.lock();
        let same = Self::entry_mut(&mut state, key).is_some_and(|entry| {
            entry.phase == Phase::Retired
                && entry
                    .handle
                    .as_ref()
                    .is_some_and(|original| Arc::ptr_eq(original, handle))
        });
        let removed = if same {
            // Caller holds explicit native shutdown Ok or the first outer
            // shutdown report's joins_complete(). This certifies native joins,
            // not a successful/deadline-compliant attempt or remote ACK.
            // The local drain clone outlives removal, so final Arc Drop is outside.
            let entry = state.slots[key.slot]
                .as_ref()
                .expect("same retired native entry");
            super::marker_identity::ReleaseMarkers::keep(&mut marker_headers, &entry.stamp.global);
            super::marker_identity::ReleaseMarkers::keep(&mut marker_headers, &entry.stamp.device);
            state.slots[key.slot].take()
        } else {
            None
        };
        drop(state);
        // Metadata fees can wake arbitrary waiters as well as the handle's
        // destructor. Neither may run under the Registry mutex.
        drop(removed);
        self.changed.notify_waiters();
        same
    }

    fn record_failure(&self, key: &Key, error: TunnelError) -> PayloadOutcome {
        let mut state = self.lock();
        let first_for_owner = Self::entry_mut(&mut state, key).is_some_and(|entry| {
            if entry.failure_observed {
                false
            } else {
                entry.failure_observed = true;
                true
            }
        });
        if first_for_owner && state.first_failure.is_some() {
            state.additional_failed_owners = state.additional_failed_owners.saturating_add(1);
        }
        // An earlier pure shutdown expiry must not bury a subsequently acquired
        // original IO/JoinError, including a retry of this same runtime. Retain
        // at most the first expiry and first non-expiry native report. This is
        // separate from the distinct failed-owner observation count.
        let expiry = shutdown_primary_is_elapsed(&error);
        let retained = if expiry && state.first_expiry.is_none() {
            self.expiry_stock.capture(error);
            let original = self.expiry_stock.clone();
            state.first_expiry = Some(original.clone());
            if state.first_failure.is_none() {
                state.first_failure = Some(original.clone());
            }
            PayloadOutcome::History(original)
        } else if !expiry
            && state
                .first_failure
                .as_ref()
                .map_or(true, |old| shutdown_primary_is_elapsed(old.error()))
        {
            self.error_stock.capture(error);
            let original = self.error_stock.clone();
            state.first_failure = Some(original.clone());
            PayloadOutcome::History(original)
        } else {
            // Preserve this newly obtained immutable snapshot in the caller's
            // original frame, without rewriting history or allocating a root.
            PayloadOutcome::Current(error)
        };
        // This method only records evidence. Its caller may release an Err
        // owner solely with the first outer report's joins_complete() result.
        retained
    }

    /// Original bounded history remains available after a monitor releases a
    /// fully joined failed owner. A fresh drain attempt has its own clock.
    pub(super) fn historical_failure(&self) -> Option<HistoryFailure> {
        let state = self.lock();
        state.first_failure.as_ref().map(|primary| HistoryFailure {
            primary: primary.clone(),
            expiry: state
                .first_expiry
                .as_ref()
                .filter(|expiry| !expiry.ptr_eq(primary))
                .cloned(),
        })
    }

    /// Stop is synchronous before the returned future exists. Selection is a
    /// fixed at-most-capacity set of incarnations, not a growing retirement log.
    /// Caller Drop retains the original handles/slot state and native Core ledger.
    pub(super) fn drain_retired(
        &self,
        clock: CleanupClock,
    ) -> impl Future<Output = std::result::Result<DrainReport, DrainFailure>> + Send + '_ {
        let selected = self.selection_and_stop();
        let selected_count = selected.count;
        async move {
            let attempt = StdMutex::new(AttemptEvidence::default());
            let evidence = &attempt;
            let work = async {
                let _serial = self.drains.lock().await;
                let mut joined = 0;
                for slot in selected.slots() {
                    if Instant::now() >= clock.deadline {
                        return DrainReport {
                            selected: selected_count,
                            joined,
                            snapshot: self.snapshot(),
                        };
                    }
                    let Some(key) = self.selected_key(selected, slot) else {
                        continue;
                    };
                    let handle = loop {
                        // Arm before inspecting state; enable prevents a missed
                        // notify_waiters race while late attachment/release occurs.
                        let changed = self.changed.notified();
                        tokio::pin!(changed);
                        changed.as_mut().enable();
                        match self.selected_owner(&key) {
                            SelectedOwner::Released => break None,
                            SelectedOwner::Native(handle) => break Some(handle),
                            SelectedOwner::AwaitingAttachment => changed.await,
                        }
                    };
                    if let Some(handle) = handle {
                        if Instant::now() >= clock.deadline {
                            return DrainReport {
                                selected: selected_count,
                                joined,
                                snapshot: self.snapshot(),
                            };
                        }
                        // The outer absolute clock includes the native call and
                        // limits its minor relative-budget conversion rounding.
                        match handle.shutdown(clock.remaining()).await {
                            Ok(()) => {
                                if self.certify_completed(&key, &handle) {
                                    joined += 1;
                                }
                            }
                            Err(error) => {
                                let complete = shutdown_joins_certified(&error);
                                let error = self.record_failure(&key, error);
                                evidence
                                    .lock()
                                    .unwrap_or_else(|p| p.into_inner())
                                    .record(error);
                                if complete && self.certify_completed(&key, &handle) {
                                    joined += 1;
                                }
                            }
                        }
                    }
                }
                DrainReport {
                    selected: selected_count,
                    joined,
                    snapshot: self.snapshot(),
                }
            };
            tokio::pin!(work);
            let guarded = poll_fn(|cx| {
                if Instant::now() >= clock.deadline {
                    return Poll::Pending;
                }
                match work.as_mut().poll(cx) {
                    Poll::Ready(_report) if Instant::now() >= clock.deadline => Poll::Pending,
                    result => result,
                }
            });
            tokio::select! {
                biased;
                expired = tokio::time::timeout_at(clock.deadline, pending::<()>()) => {
                    let elapsed = expired.expect_err("pending retirement clock cannot complete");
                    self.capture_selected(selected, Some(&attempt));
                    Err(self.failure_snapshot(&attempt, Some(elapsed), selected_count))
                }
                report = guarded => {
                    let has_failure = {
                        let current = attempt.lock().unwrap_or_else(|p| p.into_inner());
                        current.first.is_some()
                    };
                    if has_failure {
                        Err(self.failure_snapshot(&attempt, None, selected_count))
                    } else {
                        Ok(report)
                    }
                }
            }
        }
    }

    fn failure_snapshot(
        &self,
        attempt: &StdMutex<AttemptEvidence>,
        elapsed: Option<Elapsed>,
        selected: usize,
    ) -> DrainFailure {
        let (primary, additional_failures) = {
            let mut current = attempt.lock().unwrap_or_else(|p| p.into_inner());
            (current.first.take(), current.additional)
        };
        assert!(
            primary.is_some() || elapsed.is_some(),
            "failure requires native error or actual timer"
        );
        DrainFailure {
            primary,
            elapsed,
            selected,
            additional_failures,
            snapshot: self.snapshot(),
        }
    }
}

impl Drop for Registry {
    fn drop(&mut self) {
        // Final Drop is a stop fallback, never a successful join certificate.
        let state = self.state.get_mut().unwrap_or_else(|p| p.into_inner());
        for entry in state.slots.iter().flatten() {
            if let Some(handle) = &entry.handle {
                handle.request_stop();
            }
        }
    }
}

pub(super) struct Reservation {
    registry: RegistryOwner,
    key: Option<Key>,
}

impl Reservation {
    pub(super) fn byte_bank(&self) -> Arc<Semaphore> {
        Arc::clone(&self.registry.byte_bank)
    }

    pub(super) fn start(&self) -> Result<()> {
        let key = self.key.as_ref().expect("live reservation has its key");
        let mut state = self.registry.lock();
        let entry =
            Registry::entry_mut(&mut state, key).expect("live reservation owns its fixed slot");
        if entry.phase == Phase::Retired {
            return Err(super::lifecycle::cancelled());
        }
        entry.phase = Phase::Inflight;
        Ok(())
    }

    /// Attach the exact already-obtained Arc in the same poll as Source success,
    /// before metadata, liveness, daemon or table awaits. A sealed/retired slot
    /// retains and immediately stops this late handle; it never releases capacity.
    pub(super) fn attach(mut self, handle: Arc<TunnelHandle>) -> NativeLease {
        let key = self.key.take().expect("live reservation has its key");
        let stopped = {
            let mut state = self.registry.lock();
            let sealed = state.sealed;
            let entry = Registry::entry_mut(&mut state, &key)
                .expect("live reservation owns its fixed slot");
            assert!(
                entry.handle.is_none(),
                "one reservation can attach only one original runtime"
            );
            let stopped = sealed || entry.phase == Phase::Retired;
            entry.handle = Some(Arc::clone(&handle));
            if stopped {
                state.retire_slot(key.slot);
            } else {
                entry.phase = Phase::Unpublished;
            }
            stopped
        };
        if stopped {
            handle.request_stop();
        }
        self.registry.changed.notify_waiters();
        NativeLease {
            registry: self.registry.clone(),
            key,
        }
    }
}

impl Drop for Reservation {
    fn drop(&mut self) {
        if let Some(key) = &self.key {
            self.registry.retire_key(key, true);
        }
    }
}

pub(super) struct NativeLease {
    registry: RegistryOwner,
    key: Key,
}

impl NativeLease {
    pub(super) fn retain_prepared_catalogue(
        &self,
        metadata: super::freshness::MetadataOwner,
    ) -> Result<()> {
        let mut state = self.registry.lock();
        let entry =
            Registry::entry_mut(&mut state, &self.key).ok_or_else(super::lifecycle::cancelled)?;
        anyhow::ensure!(
            entry.phase == Phase::Unpublished
                && entry.fresh_metadata.is_none()
                && entry
                    .metadata
                    .as_ref()
                    .zip(entry.global_metadata.as_ref())
                    .is_some_and(|(publication, global)| metadata
                        .bank_matches(publication.semaphore(), global.semaphore())),
            super::lifecycle::cancelled()
        );
        entry.fresh_metadata = Some(metadata);
        Ok(())
    }

    pub(super) fn retain_global_metadata(
        &self,
        metadata: super::metadata_lease::Owner,
    ) -> Result<()> {
        let mut state = self.registry.lock();
        let entry =
            Registry::entry_mut(&mut state, &self.key).ok_or_else(super::lifecycle::cancelled)?;
        anyhow::ensure!(
            entry.global_metadata.is_none(),
            crate::output::input_decision("native global metadata charge already retained")
        );
        entry.global_metadata = Some(metadata);
        Ok(())
    }

    pub(super) fn retain_metadata(&self, metadata: super::metadata_lease::Owner) -> Result<()> {
        let mut state = self.registry.lock();
        let entry =
            Registry::entry_mut(&mut state, &self.key).ok_or_else(super::lifecycle::cancelled)?;
        anyhow::ensure!(
            entry.metadata.is_none(),
            crate::output::input_decision("native metadata charge already retained")
        );
        entry.metadata = Some(metadata);
        Ok(())
    }

    pub(super) fn matches_runtime(&self, handle: &Arc<TunnelHandle>) -> bool {
        let mut state = self.registry.lock();
        Registry::entry_mut(&mut state, &self.key).is_some_and(|entry| {
            entry
                .handle
                .as_ref()
                .is_some_and(|original| Arc::ptr_eq(original, handle))
        })
    }

    /// Must be inside the caller's Ticket.commit synchronous action. This
    /// method never touches lifecycle watches, a table or the native runtime.
    pub(super) fn publish(&self, stamp: &OwnershipStamp) -> Result<()> {
        let mut state = self.registry.lock();
        if state.sealed {
            return Err(super::lifecycle::cancelled());
        }
        let Some(entry) = Registry::entry_mut(&mut state, &self.key) else {
            return Err(super::lifecycle::cancelled());
        };
        if entry.phase != Phase::Unpublished
            || !ControlIdentity::ptr_eq(&entry.stamp.global, &stamp.global)
            || !ControlIdentity::ptr_eq(&entry.stamp.device, &stamp.device)
        {
            return Err(super::lifecycle::cancelled());
        }
        entry.phase = Phase::Active;
        Ok(())
    }

    pub(super) fn retire(&self) {
        self.registry.retire_key(&self.key, false);
    }
}

impl Drop for NativeLease {
    fn drop(&mut self) {
        self.retire();
    }
}

enum SelectedOwner {
    Released,
    AwaitingAttachment,
    Native(Arc<TunnelHandle>),
}

#[derive(Default)]
struct AttemptEvidence {
    first: Option<PayloadOutcome>,
    additional: u64,
}

impl AttemptEvidence {
    fn record(&mut self, error: PayloadOutcome) {
        if self.first.is_none() {
            self.first = Some(error);
        } else {
            self.additional = self.additional.saturating_add(1);
            if !shutdown_primary_is_elapsed(&error)
                && self
                    .first
                    .as_ref()
                    .is_some_and(|first| shutdown_primary_is_elapsed(first))
            {
                self.first = Some(error);
            }
        }
    }
}

#[derive(Debug)]
pub(super) struct DrainReport {
    pub(super) selected: usize,
    pub(super) joined: usize,
    pub(super) snapshot: Snapshot,
}

/// Immutable result for this drain attempt. Historical registry evidence is
/// separate; an earlier attempt Elapsed never becomes this attempt's clock.
pub(super) struct DrainFailure {
    primary: Option<PayloadOutcome>,
    elapsed: Option<Elapsed>,
    selected: usize,
    additional_failures: u64,
    snapshot: Snapshot,
}

impl DrainFailure {
    pub(super) fn primary(&self) -> &(dyn Error + 'static) {
        match &self.primary {
            Some(native) => native.as_ref(),
            None => self
                .elapsed
                .as_ref()
                .expect("actual independent retirement timer"),
        }
    }
    #[cfg(test)]
    pub(super) fn elapsed(&self) -> Option<&Elapsed> {
        self.elapsed.as_ref()
    }
    #[cfg(test)]
    pub(super) fn snapshot(&self) -> Snapshot {
        self.snapshot
    }

    pub(super) fn primary_is_elapsed(&self) -> bool {
        self.primary().is::<Elapsed>()
            || self
                .primary()
                .downcast_ref::<TunnelError>()
                .is_some_and(shutdown_primary_is_elapsed)
    }
}

impl std::fmt::Display for DrainFailure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("native tunnel retirement failed")
    }
}

impl std::fmt::Debug for DrainFailure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("NativeRetirementFailure")
            .field("primary_class", &ios_core::classify_error(self.primary()))
            .field("elapsed", &self.elapsed.is_some())
            .field("selected", &self.selected)
            .field("additional_failures", &self.additional_failures)
            .field("ownership", &self.snapshot)
            .finish_non_exhaustive()
    }
}

impl Error for DrainFailure {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        Some(self.primary())
    }
}

pub(super) struct HistoryFailure {
    primary: NativeReportOwner,
    expiry: Option<NativeReportOwner>,
}

impl HistoryFailure {
    pub(super) fn primary(&self) -> &TunnelError {
        self.primary.error()
    }
    pub(super) fn expiry(&self) -> Option<&TunnelError> {
        self.expiry.as_ref().map(|original| original.error())
    }
}

impl std::fmt::Display for HistoryFailure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("native tunnel cleanup recorded a failure")
    }
}

impl std::fmt::Debug for HistoryFailure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("NativeTunnelHistoryFailure")
            .field("primary_class", &ios_core::classify_error(self.primary()))
            .field("earlier_expiry", &self.expiry().is_some())
            .finish()
    }
}

impl Error for HistoryFailure {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        Some(self.primary())
    }
}

#[cfg(test)]
#[path = "retirement/metadata_tests.rs"]
mod metadata_tests;
#[cfg(test)]
mod native_tests;
#[cfg(test)]
#[path = "retirement/report_body_tests.rs"]
mod report_body_tests;

#[cfg(test)]
mod origin_tests;
#[cfg(test)]
mod tests;
#[cfg(test)]
mod wake_tests;

include!("retirement_refresh.inc.rs");
