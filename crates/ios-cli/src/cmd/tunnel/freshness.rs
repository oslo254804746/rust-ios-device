//! Refresh an exact active row in place, without acquiring or retiring a lease.
use super::{admission, lifecycle, Ticket, TunnelAgentState};
use anyhow::Result;
use ios_core::device::PreparedRsdOwner;
use std::sync::{Arc, Mutex, OnceLock};
use tokio::sync::{OwnedSemaphorePermit, Semaphore};

pub(super) struct Metadata {
    pub(super) catalogue: PreparedRsdOwner,
    _publication_fields: OwnedSemaphorePermit,
}

pub(super) type MetadataOwner = super::shared_dual_body::Strong<Metadata>;

impl Metadata {
    pub(super) fn prepare(
        catalogue: PreparedRsdOwner,
        global: &Arc<Semaphore>,
        publication: &Arc<Semaphore>,
    ) -> Result<MetadataOwner> {
        anyhow::ensure!(
            catalogue.belongs_to(global),
            crate::output::input_decision("RSD catalogue belongs to another metadata pool")
        );
        let body = admission::known_metadata_charge(global, MetadataOwner::body_bytes())?;
        let mut fee = admission::known_metadata_charge(
            publication,
            catalogue
                .known_bytes()
                .checked_add(MetadataOwner::body_bytes())
                .ok_or_else(|| crate::output::resource_decision("RSD refresh size overflow"))?,
        )?;
        let publication_body = fee
            .split(MetadataOwner::body_bytes())
            .expect("preadmitted refresh publication body");
        MetadataOwner::new(
            Self {
                catalogue,
                _publication_fields: fee,
            },
            global,
            publication,
            body,
            publication_body,
        )
    }
}

impl MetadataOwner {
    pub(super) fn bank_matches(
        &self,
        publication: &Arc<Semaphore>,
        global: &Arc<Semaphore>,
    ) -> bool {
        self.catalogue.belongs_to(global)
            && self.banks_match(global, publication)
            && Arc::ptr_eq(self._publication_fields.semaphore(), publication)
    }
}

/// Pre-Source capsule with two actual empty report payload destinations.
/// Original/private anyhow allocations and the outer Failure wrapper layout
/// remain separate boundaries; this root and the independent payload Boxes
/// retain their original grants through each actual allocation's destruction.
pub(super) struct Receipt {
    original: OnceLock<anyhow::Error>,
    reports: Mutex<[Option<super::owned_box::Destination<ReportPayload>>; 2]>,
}

pub(super) type ReceiptOwner = super::shared_allocation::Shared<Receipt>;

#[cfg(test)]
#[path = "freshness/receipt_owner_tests.rs"]
mod receipt_owner_tests;

impl Receipt {
    pub(super) fn body_bytes() -> usize {
        ReceiptOwner::allocation_bytes()
    }
    pub(super) fn reservation_bytes() -> usize {
        Self::body_bytes() + 2 * std::mem::size_of::<ReportPayload>()
    }
    pub(super) fn prepare(bank: &Arc<Semaphore>) -> Result<ReceiptOwner> {
        let storage = admission::known_metadata_charge(bank, Self::reservation_bytes())?;
        Self::from_grant(bank, storage)
    }
    fn from_grant(
        bank: &Arc<Semaphore>,
        mut storage: OwnedSemaphorePermit,
    ) -> Result<ReceiptOwner> {
        anyhow::ensure!(
            Arc::ptr_eq(storage.semaphore(), bank)
                && storage.num_permits() == Self::reservation_bytes(),
            crate::output::resource_decision("tunnel freshness capsule grant mismatch")
        );
        let first = storage
            .split(std::mem::size_of::<ReportPayload>())
            .expect("reserved first refresh report payload");
        let second = storage
            .split(std::mem::size_of::<ReportPayload>())
            .expect("reserved second refresh report payload");
        // All three original grants exist before the first actual allocation.
        // Empty destinations contain no ReceiptOwner and cannot form a cycle.
        let first = super::owned_box::Destination::from_grant(bank, first)?;
        let second = super::owned_box::Destination::from_grant(bank, second)?;
        ReceiptOwner::from_grant(
            Self {
                original: OnceLock::new(),
                reports: Mutex::new([Some(first), Some(second)]),
            },
            bank,
            storage,
        )
    }
    pub(super) fn original(&self) -> &anyhow::Error {
        self.original.get().expect("captured refresh Source error")
    }
    fn capture(&self, error: anyhow::Error) {
        assert!(
            self.original.set(error).is_ok(),
            "one Source original per capsule"
        );
    }
}

impl ReceiptOwner {
    pub(super) fn report(&self) -> anyhow::Error {
        let mut reports = self
            .reports
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        let destination = reports
            .iter_mut()
            .find_map(Option::take)
            .expect("two admitted refresh reports maximum");
        drop(reports);
        anyhow::Error::new(Failure {
            payload: destination.place(ReportPayload {
                receipt: self.clone(),
            }),
        })
    }
}
struct ReportPayload {
    receipt: ReceiptOwner,
}
pub(super) struct Failure {
    payload: super::owned_box::Owner<ReportPayload>,
}
impl std::fmt::Display for Failure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("current RSD refresh failed")
    }
}
impl std::fmt::Debug for Failure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RsdRefreshFailure")
            .field(
                "class",
                &crate::output::operation_error_class(self.payload.receipt.original().as_ref()),
            )
            .finish()
    }
}
impl std::error::Error for Failure {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        Some(self.payload.receipt.original().as_ref())
    }
}

impl Failure {
    pub(super) fn original(&self) -> &anyhow::Error {
        self.payload.receipt.original()
    }
}

#[cfg(test)]
thread_local! {
    static AFTER_SWAP: std::cell::RefCell<Option<Box<dyn FnOnce()>>> = std::cell::RefCell::new(None);
}
#[cfg(test)]
pub(super) fn on_after_swap(callback: impl FnOnce() + 'static) {
    AFTER_SWAP.with(|hook| *hook.borrow_mut() = Some(Box::new(callback)));
}

#[derive(Clone)]
pub(super) struct OldRow {
    pub(super) key_buffer: usize,
    pub(super) publication: super::publication::LifeOwner,
    pub(super) registry: super::retirement::RefreshProof,
    pub(super) observed_ticket: Ticket,
}

impl OldRow {
    pub(super) fn matches_locked(
        &self,
        key: &str,
        row: &super::ManagedTunnel,
        ticket: &Ticket,
    ) -> bool {
        key.as_ptr() as usize == self.key_buffer
            && self.publication.ptr_eq(&row.publication)
            && row._ticket.same_incarnation(ticket)
            && row._ticket.same_incarnation(&self.observed_ticket)
            && row.device.native_handle().is_some_and(|handle| {
                self.registry.matches_row(
                    handle,
                    &row._metadata,
                    &row._global_metadata,
                    row._fresh_metadata.as_ref(),
                )
            })
            && super::retirement::same_optional(
                row.device.freshness_catalogue(),
                row._fresh_metadata
                    .as_ref()
                    .map(|metadata| &metadata.catalogue),
            )
    }
}

pub(super) struct RecoveryInput {
    pub(super) old: OldRow,
    pub(super) probe_failure: anyhow::Error,
    pub(super) candidate_failure: super::owned_box::Destination<anyhow::Error>,
}

/// A typed context keeps the probe's original anyhow downcasts/source chain
/// primary, while exposing the entire independent candidate failure explicitly.
pub(super) struct CandidateFailure {
    candidate: super::owned_box::Owner<anyhow::Error>,
}
impl CandidateFailure {
    pub(super) fn attach(
        primary: anyhow::Error,
        candidate: anyhow::Error,
        destination: super::owned_box::Destination<anyhow::Error>,
    ) -> anyhow::Error {
        primary.context(Self {
            candidate: destination.place(candidate),
        })
    }
    #[cfg(test)]
    pub(super) fn candidate(&self) -> &anyhow::Error {
        &self.candidate
    }
}
impl std::fmt::Display for CandidateFailure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "replacement candidate failed: {}", &*self.candidate)
    }
}
impl std::fmt::Debug for CandidateFailure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ReplacementCandidateFailure")
            .finish_non_exhaustive()
    }
}

pub(super) enum RefreshAttempt {
    Current,
    ProbeFailed(RecoveryInput),
    Ended(OldRow),
}

struct RecoveryPending {
    old: OldRow,
    candidate_failure: super::owned_box::Destination<anyhow::Error>,
    receipt: ReceiptOwner,
}

fn invoke<T>(work: impl FnOnce() -> Result<T>) -> Result<T> {
    work()
}

impl TunnelAgentState {
    pub(super) async fn try_refresh_existing(
        &self,
        udid: &str,
        ticket: &Ticket,
        deadline: &lifecycle::ConnectionDeadline,
        session: Option<&super::scan::Session<'_>>,
        manual_authority: Option<&crate::access::DeviceAuthority>,
    ) -> Result<RefreshAttempt> {
        if !self.inner.source.observes_freshness() {
            return Ok(RefreshAttempt::Current);
        }
        let snapshot = ticket.desired();
        let observation = ticket.observe_version(&snapshot.version);
        let authority = match (manual_authority, snapshot.desired) {
            (Some(authority), _) => Some(authority.clone()),
            (None, super::intent::Desired::Manual { authority, .. }) => Some(authority),
            (None, super::intent::Desired::Stopped) => return Err(lifecycle::cancelled()),
            (None, super::intent::Desired::Automatic) => None,
        };
        let mut recovery = None;
        let result = {
            let mut work = std::pin::pin!(observation.run(deadline.run(
                self.try_refresh_existing_inner(
                    udid,
                    &observation,
                    deadline,
                    session,
                    authority.as_ref(),
                    &mut recovery,
                )
            )));
            match &authority {
                Some(authority) => authority.run(work.as_mut()).await,
                None => work.as_mut().await,
            }
        };
        match (result, recovery) {
            (Err(error), Some(pending))
                if error
                    .downcast_ref::<Failure>()
                    .is_some_and(|failure| failure.payload.receipt.ptr_eq(&pending.receipt)) =>
            {
                Ok(RefreshAttempt::ProbeFailed(RecoveryInput {
                    old: pending.old,
                    probe_failure: error,
                    candidate_failure: pending.candidate_failure,
                }))
            }
            (result, _) => result,
        }
    }

    async fn try_refresh_existing_inner(
        &self,
        udid: &str,
        ticket: &Ticket,
        deadline: &lifecycle::ConnectionDeadline,
        session: Option<&super::scan::Session<'_>>,
        authority: Option<&crate::access::DeviceAuthority>,
        recovery: &mut Option<RecoveryPending>,
    ) -> Result<RefreshAttempt> {
        anyhow::ensure!(
            self.inner.access.permits_device(udid),
            lifecycle::cancelled()
        );
        let (key_buffer, publication, proof, route, alive) = {
            let tunnels = self.inner.tunnels.read().await;
            deadline.check()?;
            let (key, row) = tunnels
                .get_key_value(udid)
                .ok_or_else(lifecycle::cancelled)?;
            let Some(route) = row.device.freshness_route()? else {
                return Ok(RefreshAttempt::Current);
            };
            let runtime = row
                .device
                .native_handle()
                .ok_or_else(lifecycle::cancelled)?;
            let native = row.native.as_ref().ok_or_else(lifecycle::cancelled)?;
            anyhow::ensure!(
                super::retirement::same_optional(
                    row.device.freshness_catalogue(),
                    row._fresh_metadata
                        .as_ref()
                        .map(|metadata| &metadata.catalogue)
                ),
                lifecycle::cancelled()
            );
            let is_alive = runtime.is_alive();
            let capture = || {
                ticket.commit(|| {
                    deadline.check()?;
                    native.capture_existing(
                        ticket.ownership_stamp(),
                        runtime,
                        &row._metadata,
                        &row._global_metadata,
                        row._fresh_metadata.as_ref(),
                        &self.inner.global_metadata_bytes,
                    )
                })
            };
            let proof = match authority {
                Some(authority) => authority.commit(udid, capture)?,
                None => capture()?,
            };
            (
                key.as_ptr() as usize,
                row.publication.clone(),
                proof,
                route,
                is_alive,
            )
        };
        let old = OldRow {
            key_buffer,
            publication,
            registry: proof,
            observed_ticket: ticket.clone(),
        };
        if !alive {
            return Ok(RefreshAttempt::Ended(old));
        }
        // Allocate the cause capsule and two output owners before any Source poll.
        let candidate_failure =
            super::owned_box::Destination::new(&self.inner.global_metadata_bytes)?;
        let receipt = Receipt::prepare(&self.inner.global_metadata_bytes)?;
        let permit = self.inner.lifecycle.connects.acquire().await?;
        deadline.check()?;
        let admit = || {
            ticket.commit(|| {
                deadline.check()?;
                old.registry.check()
            })
        };
        let still_alive = match authority {
            Some(authority) => authority.commit(udid, admit),
            None => admit(),
        };
        if let Err(cause) = still_alive {
            drop(permit);
            deadline.check()?;
            if let Some(authority) = authority {
                authority.check_clock()?;
            }
            ticket.commit(|| old.registry.check_replacement())?;
            if old.registry.runtime().is_alive() {
                return Err(cause);
            }
            return Ok(RefreshAttempt::Ended(old));
        }
        let mut source = self.inner.source.refresh_existing_rsd(
            route,
            deadline.absolute_at(),
            &self.inner.global_metadata_bytes,
        )?;
        let observe = std::future::poll_fn(|cx| {
            match std::future::Future::poll(std::pin::Pin::new(&mut source), cx) {
                std::task::Poll::Ready(Err(original)) => {
                    receipt.capture(original);
                    // Persist the original in this very Ready poll; no later
                    // await/cancellation/clock or bookkeeping can discard it.
                    old.registry.retain_failure(receipt.clone());
                    if let Some(session) = session {
                        session.retain(super::scan::Source::Freshness, receipt.report());
                    }
                    std::task::Poll::Ready(Err(receipt.report()))
                }
                other => other,
            }
        });
        let result = observe.await;
        // The independent catalogue owns its own storage. Destroy this original
        // Source frame and release the permit before admitting new candidates or output metadata.
        drop(source);
        drop(permit);
        let catalogue = match result {
            Ok(catalogue) => catalogue,
            Err(probe_failure) => {
                // Preserve Ready(Err) through the original deadline/authority
                // wrappers. Encoding this as Ok(ProbeFailed) here would let a
                // crossed clock erase the already acquired native cause.
                *recovery = Some(RecoveryPending {
                    old,
                    candidate_failure,
                    receipt,
                });
                return Err(probe_failure);
            }
        };
        deadline.check()?;
        anyhow::ensure!(
            catalogue.udid() == udid,
            ios_core::CoreError::Protocol("RSD current identity differs from publication".into())
        );
        let proposed = Metadata::prepare(
            catalogue,
            &self.inner.global_metadata_bytes,
            &self.inner.metadata_bytes,
        )?;
        let mut tunnels = self.inner.tunnels.write().await;
        deadline.check()?;
        let (key, current) = tunnels
            .get_key_value(udid)
            .ok_or_else(lifecycle::cancelled)?;
        anyhow::ensure!(
            old.matches_locked(key, current, ticket),
            lifecycle::cancelled()
        );
        let row = tunnels.get_mut(udid).ok_or_else(lifecycle::cancelled)?;
        let swap = || {
            ticket.commit(|| {
                old.registry.swap_metadata(proposed.clone(), || {
                    deadline.check()?;
                    anyhow::ensure!(
                        self.inner.access.permits_device(udid),
                        lifecycle::cancelled()
                    );
                    let old_cache = row.device.swap_freshness_catalogue(
                        old.registry.runtime(),
                        Some(proposed.catalogue.clone()),
                    )?;
                    let old_metadata = row._fresh_metadata.replace(proposed.clone());
                    #[cfg(test)]
                    AFTER_SWAP.with(|hook| {
                        if let Some(hook) = hook.borrow_mut().take() {
                            hook();
                        }
                    });
                    let late = deadline.check().and_then(|()| {
                        anyhow::ensure!(
                            self.inner.access.permits_device(udid),
                            lifecycle::cancelled()
                        );
                        anyhow::ensure!(
                            row.device
                                .native_handle()
                                .is_some_and(|runtime| runtime.is_alive()),
                            lifecycle::cancelled()
                        );
                        if let Some(authority) = authority {
                            authority.check_clock()?;
                        }
                        Ok(())
                    });
                    if let Err(cause) = late {
                        // Exact originals return before any commit is observable. This
                        // cache swap is validated against the immutable same runtime.
                        let refused_cache = row
                            .device
                            .swap_freshness_catalogue(old.registry.runtime(), old_cache)?;
                        row._fresh_metadata = old_metadata;
                        drop(refused_cache); // proposed still owns this buffer/fees
                        return Err(cause);
                    }
                    Ok((old_cache, old_metadata))
                })
            })
        };
        // Preserve the existing authority -> ticket -> native registry lock order.
        let result = match authority {
            Some(authority) => authority.commit(udid, swap),
            None => invoke(swap),
        };
        drop(tunnels);
        // Every displaced cache/fee/error owner drops after all commit locks.
        result.map(|displaced| {
            drop(displaced);
            RefreshAttempt::Current
        })
    }

    /// Caller owns the original device serial guard. Nested cancellation/clock
    /// guards include the permit wait; no new deadline is started here.
    #[cfg(test)]
    pub(super) async fn refresh_alive_rsd(
        &self,
        udid: &str,
        ticket: &Ticket,
        deadline: &lifecycle::ConnectionDeadline,
        session: Option<&super::scan::Session<'_>>,
        manual_authority: Option<&crate::access::DeviceAuthority>,
    ) -> Result<()> {
        match self
            .try_refresh_existing(udid, ticket, deadline, session, manual_authority)
            .await?
        {
            RefreshAttempt::Current => Ok(()),
            RefreshAttempt::ProbeFailed(recovery) => Err(recovery.probe_failure),
            RefreshAttempt::Ended(_) => Err(lifecycle::cancelled()),
        }
    }
}
