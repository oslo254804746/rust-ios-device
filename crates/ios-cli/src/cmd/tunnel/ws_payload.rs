//! Original outgoing Binary Vec admission through sink cancellation and errors.
use std::sync::Arc;

use anyhow::Result;
use axum::extract::ws::Message;
use futures_util::{Sink, SinkExt};
use tokio::sync::{OwnedSemaphorePermit, Semaphore};

pub(super) struct Pending {
    grant: Option<OwnedSemaphorePermit>,
}

/// Both halves may retain the original message. Their destruction must finish
/// before Pending returns its fee, including when a direction is cancelled.
pub(super) struct Endpoints<S, R> {
    sink: S,
    receiver: R,
    pending: Pending,
}

impl<S, R> Endpoints<S, R> {
    pub(super) fn new(sink: S, receiver: R) -> Self {
        Self {
            sink,
            receiver,
            pending: Pending { grant: None },
        }
    }

    pub(super) fn parts(&mut self) -> (&mut S, &mut R, &mut Pending) {
        (&mut self.sink, &mut self.receiver, &mut self.pending)
    }

    pub(super) fn finish(mut self, result: Result<()>) -> Completion {
        // A send error may contain the original Message. Put the original
        // error and fee together BEFORE destroying either split half.
        let completion = Completion {
            result,
            _storage: self.pending.grant.take(),
        };
        drop(self);
        completion
    }
}

/// Keep result first: its original typed error may own the admitted Vec.
/// Only borrowed access is exposed; no error can escape its payload owner.
pub(super) struct Completion {
    result: Result<()>,
    _storage: Option<OwnedSemaphorePermit>,
}
impl Completion {
    pub(super) fn result(&self) -> &Result<()> {
        &self.result
    }
}

/// Used with the audited axum/tungstenite sink: a successful flush has disposed
/// of the original Message Vec. The separate serialized write buffer is private.
pub(super) async fn binary<S>(
    sink: &mut S,
    pending: &mut Pending,
    bytes: &[u8],
    bank: &Arc<Semaphore>,
) -> Result<()>
where
    S: Sink<Message> + Unpin,
    S::Error: Into<anyhow::Error>,
{
    anyhow::ensure!(
        pending.grant.is_none(),
        crate::output::resource_decision("tunnel WebSocket payload already pending")
    );
    let mut allocation = super::metadata_vec::exact_backing(bank, bytes.len())?;
    allocation.values.extend_from_slice(bytes);
    pending.grant = Some(allocation.storage);
    sink.send(Message::Binary(allocation.values))
        .await
        .map_err(Into::into)?;
    pending.grant.take();
    Ok(())
}

#[cfg(test)]
mod tests;
