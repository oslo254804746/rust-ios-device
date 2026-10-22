//! Late CURRENT manual proofs enter storage owned before any scan Source poll.
use std::sync::Arc;

use anyhow::Result;
use tokio::sync::Semaphore;

use super::bookkeeping::{DeferredSuccess, SuccessIntent, SuccessKey, WriteBatch};
use super::metadata_vec::KnownVec;
use super::{FailureVersion, Ticket, TunnelAgentState};
use crate::access::DeviceAuthority;

struct Completed {
    ticket: Ticket,
    desired: Arc<()>,
    failure: FailureVersion,
    authority: DeviceAuthority,
}

pub(super) type Owner = super::owned_box::Owner<Preflight>;

pub(super) struct Preflight {
    batch: WriteBatch,
    completed: KnownVec<Completed>,
}

impl Preflight {
    pub(super) fn new(bank: &Arc<Semaphore>) -> Result<Owner> {
        let destination = super::owned_box::Destination::<Self>::new(bank)?;
        let capacity = super::lifecycle::MAX_DEVICE_GATES;
        let batch = WriteBatch::success_only(bank, capacity)?;
        let mut completed = KnownVec::new(Arc::clone(bank), capacity);
        completed.reserve(capacity)?;
        Ok(destination.place(Self { batch, completed }))
    }

    pub(super) fn record(
        &mut self,
        ticket: Ticket,
        desired: Arc<()>,
        failure: FailureVersion,
        authority: DeviceAuthority,
    ) {
        self.completed.push_reserved(Completed {
            ticket: ticket.clone(),
            desired: Arc::clone(&desired),
            failure: failure.clone(),
            authority: authority.clone(),
        });
        self.batch.record_success_reserved(SuccessIntent {
            udid: SuccessKey::Coordinator { desired, authority },
            ticket,
            success: DeferredSuccess { version: failure },
            _input: None,
            _key_storage: None,
        });
    }

    pub(super) async fn flush(
        mut self,
        state: &TunnelAgentState,
        whole: &super::scan::Deadline,
    ) -> Result<()> {
        whole.check()?;
        state.inner.lifecycle.fill_manual_successes(&mut self).await;
        self.flush_collected(state, whole).await
    }

    async fn flush_collected(
        self,
        state: &TunnelAgentState,
        whole: &super::scan::Deadline,
    ) -> Result<()> {
        whole.check()?;
        state.flush_write_batch(self.batch, Some(whole)).await?;
        for row in self.completed {
            // A stale ticket/authority must not clear a newer or unprocessed tuple.
            whole.check()?;
            let result = row.authority.commit(row.ticket.udid(), || {
                row.ticket.commit(|| {
                    row.ticket
                        .clear_success_checked(&row.desired, &row.failure, || {
                            whole.check().and_then(|()| row.authority.check_clock())
                        })
                })
            });
            if let Err(error) = result {
                if !error.is::<ios_core::OperationCancelled>() {
                    return Err(error);
                }
            }
        }
        Ok(())
    }

    #[cfg(test)]
    pub(super) fn bytes() -> usize {
        let capacity = super::lifecycle::MAX_DEVICE_GATES;
        std::mem::size_of::<Self>()
            + WriteBatch::success_only_bytes(capacity)
            + capacity * std::mem::size_of::<Completed>()
    }

    #[cfg(test)]
    pub(super) fn selected(&self) -> usize {
        self.completed.len()
    }
}

#[cfg(test)]
mod body_tests;
#[cfg(test)]
mod tests;
