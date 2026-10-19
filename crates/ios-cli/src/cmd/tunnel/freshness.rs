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

/// Pre-Source capsule. Arbitrary original/private anyhow allocations remain
/// unmeasured, while this owned body and two report payloads are byte-admitted.
pub(super) struct Receipt {
    original: OnceLock<anyhow::Error>,
    reports: Mutex<[Option<OwnedSemaphorePermit>; 2]>,
}

pub(super) type ReceiptOwner = super::shared_allocation::Shared<Receipt>;

#[cfg(test)]
#[path = "freshness/receipt_owner_tests.rs"]
mod receipt_owner_tests;

impl Receipt {
    pub(super) fn body_bytes() -> usize {
        ReceiptOwner::allocation_bytes()
    }
    pub(super) fn prepare(bank: &Arc<Semaphore>) -> Result<ReceiptOwner> {
        let mut storage = admission::known_metadata_charge(
            bank,
            Self::body_bytes() + 2 * std::mem::size_of::<Failure>(),
        )?;
        let first = storage
            .split(std::mem::size_of::<Failure>())
            .expect("reserved first refresh report");
        let second = storage
            .split(std::mem::size_of::<Failure>())
            .expect("reserved second refresh report");
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
        let storage = reports
            .iter_mut()
            .find_map(Option::take)
            .expect("two admitted refresh reports maximum");
        anyhow::Error::new(Failure {
            receipt: self.clone(),
            _storage: storage,
        })
    }
}
pub(super) struct Failure {
    receipt: ReceiptOwner,
    _storage: OwnedSemaphorePermit,
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
                &crate::output::operation_error_class(self.receipt.original().as_ref()),
            )
            .finish()
    }
}
impl std::error::Error for Failure {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        Some(self.receipt.original().as_ref())
    }
}

impl Failure {
    pub(super) fn original(&self) -> &anyhow::Error {
        self.receipt.original()
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

fn invoke<T>(work: impl FnOnce() -> Result<T>) -> Result<T> {
    work()
}

impl TunnelAgentState {
    /// Caller owns the original device serial guard. Nested cancellation/clock
    /// guards include the permit wait; no new deadline is started here.
    pub(super) async fn refresh_alive_rsd(
        &self,
        udid: &str,
        ticket: &Ticket,
        deadline: &lifecycle::ConnectionDeadline,
        session: Option<&super::scan::Session<'_>>,
        manual_authority: Option<&crate::access::DeviceAuthority>,
    ) -> Result<()> {
        if !self.inner.source.observes_freshness() {
            return Ok(());
        }
        let snapshot = ticket.desired();
        let observation = ticket.observe_version(&snapshot.version);
        let authority = match (manual_authority, snapshot.desired) {
            (Some(authority), _) => Some(authority.clone()),
            (None, super::intent::Desired::Manual { authority, .. }) => Some(authority),
            (None, super::intent::Desired::Stopped) => return Err(lifecycle::cancelled()),
            (None, super::intent::Desired::Automatic) => None,
        };
        let work = observation.run(deadline.run(self.refresh_alive_rsd_inner(
            udid,
            &observation,
            deadline,
            session,
            authority.as_ref(),
        )));
        match &authority {
            Some(authority) => authority.run(work).await,
            None => work.await,
        }
    }

    async fn refresh_alive_rsd_inner(
        &self,
        udid: &str,
        ticket: &Ticket,
        deadline: &lifecycle::ConnectionDeadline,
        session: Option<&super::scan::Session<'_>>,
        authority: Option<&crate::access::DeviceAuthority>,
    ) -> Result<()> {
        anyhow::ensure!(
            self.inner.access.permits_device(udid),
            lifecycle::cancelled()
        );
        let (key_buffer, publication, proof, route) = {
            let tunnels = self.inner.tunnels.read().await;
            deadline.check()?;
            let (key, row) = tunnels
                .get_key_value(udid)
                .ok_or_else(lifecycle::cancelled)?;
            let Some(route) = row.device.freshness_route()? else {
                return Ok(());
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
            let capture = || {
                ticket.commit(|| {
                    deadline.check()?;
                    native.capture_refresh(
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
            (key.as_ptr() as usize, row.publication.clone(), proof, route)
        };
        // Allocate the cause capsule and two output owners before any Source poll.
        let receipt = Receipt::prepare(&self.inner.global_metadata_bytes)?;
        let _permit = self.inner.lifecycle.connects.acquire().await?;
        deadline.check()?;
        let admit = || {
            ticket.commit(|| {
                deadline.check()?;
                proof.check()
            })
        };
        match authority {
            Some(authority) => authority.commit(udid, admit)?,
            None => admit()?,
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
                    proof.retain_failure(receipt.clone());
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
        // Source frame before admitting the output/publication metadata.
        drop(source);
        let catalogue = result?;
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
            key.as_ptr() as usize == key_buffer && publication.ptr_eq(&current.publication),
            lifecycle::cancelled()
        );
        let row = tunnels.get_mut(udid).ok_or_else(lifecycle::cancelled)?;
        anyhow::ensure!(
            publication.ptr_eq(&row.publication) && row._ticket.same_incarnation(ticket),
            lifecycle::cancelled()
        );
        anyhow::ensure!(
            super::retirement::same_metadata(row._fresh_metadata.as_ref(), proof.previous())
                && super::retirement::same_optional(
                    row.device.freshness_catalogue(),
                    row._fresh_metadata
                        .as_ref()
                        .map(|metadata| &metadata.catalogue)
                ),
            lifecycle::cancelled()
        );
        let swap = || {
            ticket.commit(|| {
                proof.swap_metadata(proposed.clone(), || {
                    deadline.check()?;
                    anyhow::ensure!(
                        self.inner.access.permits_device(udid),
                        lifecycle::cancelled()
                    );
                    let old_cache = row.device.swap_freshness_catalogue(
                        proof.runtime(),
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
                            .swap_freshness_catalogue(proof.runtime(), old_cache)?;
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
        result.map(drop)
    }
}
