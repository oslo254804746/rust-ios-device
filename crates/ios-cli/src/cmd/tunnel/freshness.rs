//! Refresh an exact active row in place, without acquiring or retiring a lease.
use super::{admission, lifecycle, Ticket, TunnelAgentState};
use anyhow::Result;
use ios_core::device::PreparedRsdCatalogue;
use std::sync::{Arc, Mutex, OnceLock};
use tokio::sync::{OwnedSemaphorePermit, Semaphore};

pub(super) struct Metadata {
    pub(super) catalogue: Arc<PreparedRsdCatalogue>,
    _publication: OwnedSemaphorePermit,
    _body: OwnedSemaphorePermit,
}
impl Metadata {
    pub(super) fn prepare(
        catalogue: Arc<PreparedRsdCatalogue>,
        global: &Arc<Semaphore>,
        publication: &Arc<Semaphore>,
    ) -> Result<Arc<Self>> {
        anyhow::ensure!(
            catalogue.belongs_to(global),
            crate::output::input_decision("RSD catalogue belongs to another metadata pool")
        );
        let body = admission::known_metadata_charge(global, std::mem::size_of::<Self>())?;
        let fee = admission::known_metadata_charge(
            publication,
            catalogue
                .known_bytes()
                .checked_add(std::mem::size_of::<Self>())
                .ok_or_else(|| crate::output::resource_decision("RSD refresh size overflow"))?,
        )?;
        Ok(Arc::new(Self {
            catalogue,
            _publication: fee,
            _body: body,
        }))
    }
    pub(super) fn bank_matches(
        &self,
        publication: &Arc<Semaphore>,
        global: &Arc<Semaphore>,
    ) -> bool {
        self.catalogue.belongs_to(global)
            && Arc::ptr_eq(self._body.semaphore(), global)
            && Arc::ptr_eq(self._publication.semaphore(), publication)
    }
}

/// Pre-Source capsule. Arbitrary original/private anyhow allocations remain
/// unmeasured, while this owned body and two report payloads are byte-admitted.
pub(super) struct Receipt {
    original: OnceLock<anyhow::Error>,
    reports: Mutex<[Option<OwnedSemaphorePermit>; 2]>,
    _storage: OwnedSemaphorePermit,
}
impl Receipt {
    pub(super) fn prepare(bank: &Arc<Semaphore>) -> Result<Arc<Self>> {
        let mut storage = admission::known_metadata_charge(
            bank,
            std::mem::size_of::<Self>() + 2 * std::mem::size_of::<Failure>(),
        )?;
        let first = storage
            .split(std::mem::size_of::<Failure>())
            .expect("reserved first refresh report");
        let second = storage
            .split(std::mem::size_of::<Failure>())
            .expect("reserved second refresh report");
        Ok(Arc::new(Self {
            original: OnceLock::new(),
            reports: Mutex::new([Some(first), Some(second)]),
            _storage: storage,
        }))
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
    pub(super) fn report(self: &Arc<Self>) -> anyhow::Error {
        let mut reports = self
            .reports
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        let storage = reports
            .iter_mut()
            .find_map(Option::take)
            .expect("two admitted refresh reports maximum");
        anyhow::Error::new(Failure {
            receipt: Arc::clone(self),
            _storage: storage,
        })
    }
}
pub(super) struct Failure {
    receipt: Arc<Receipt>,
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
            (
                key.as_ptr() as usize,
                Arc::clone(&row.publication),
                proof,
                route,
            )
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
        );
        let observe = std::future::poll_fn(|cx| {
            match std::future::Future::poll(source.as_mut(), cx) {
                std::task::Poll::Ready(Err(original)) => {
                    receipt.capture(original);
                    // Persist the original in this very Ready poll; no later
                    // await/cancellation/clock or bookkeeping can discard it.
                    proof.retain_failure(Arc::clone(&receipt));
                    if let Some(session) = session {
                        session.retain(super::scan::Source::Freshness, receipt.report());
                    }
                    std::task::Poll::Ready(Err(receipt.report()))
                }
                other => other,
            }
        });
        let catalogue = observe.await?;
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
            key.as_ptr() as usize == key_buffer && Arc::ptr_eq(&publication, &current.publication),
            lifecycle::cancelled()
        );
        let row = tunnels.get_mut(udid).ok_or_else(lifecycle::cancelled)?;
        anyhow::ensure!(
            Arc::ptr_eq(&publication, &row.publication) && row._ticket.same_incarnation(ticket),
            lifecycle::cancelled()
        );
        anyhow::ensure!(
            super::retirement::same_optional(row._fresh_metadata.as_ref(), proof.previous())
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
                proof.swap_metadata(Arc::clone(&proposed), || {
                    deadline.check()?;
                    anyhow::ensure!(
                        self.inner.access.permits_device(udid),
                        lifecycle::cancelled()
                    );
                    let old_cache = row.device.swap_freshness_catalogue(
                        proof.runtime(),
                        Some(Arc::clone(&proposed.catalogue)),
                    )?;
                    let old_metadata = row._fresh_metadata.replace(Arc::clone(&proposed));
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
