//! Original upgrade/bridge captures and concrete future backing admitted before
//! target lookup or dial. Hyper/task/transport private heaps remain separate.
use anyhow::Result;
use axum::extract::ws::WebSocket;
use std::{
    future::Future,
    sync::{Arc, Mutex},
};
use tokio::{net::TcpStream, sync::Semaphore};
use tokio_tungstenite::{MaybeTlsStream, WebSocketStream};

use super::{
    future_storage::PaidFuture,
    metadata_vec::{from_grant, AdmittedVec},
    shared_weak_allocation::Strong,
};

pub(super) struct Input<T>(Strong<Mutex<Option<T>>, ()>);
impl<T> Clone for Input<T> {
    fn clone(&self) -> Self {
        Self(self.0.clone())
    }
}
impl<T> Input<T> {
    fn take(self) -> T {
        let mut value = self.0.lock().unwrap_or_else(|error| error.into_inner());
        value
            .take()
            .expect("original upgrade callback filled its input")
    }
}

pub(super) struct Prepared<T, F> {
    input: Input<T>,
    backing: AdmittedVec<F>,
}
pub(super) fn prepare<T, C, F: Future>(
    bank: &Arc<Semaphore>,
    _create: impl FnOnce(Input<T>, C) -> F,
) -> Result<Prepared<T, F>> {
    let input_bytes = Strong::<Mutex<Option<T>>, ()>::body_bytes();
    let total = input_bytes
        .checked_add(std::mem::size_of::<F>())
        .ok_or_else(|| crate::output::resource_decision("tunnel upgrade owner size overflow"))?;
    // One original grant precedes BOTH actual backings. No capture or future
    // is manufactured/polled just to discover its compiled concrete layout.
    let mut storage = super::admission::known_metadata_charge(bank, total)?;
    let input_storage = storage
        .split(input_bytes)
        .expect("whole upgrade owner grant");
    let input = Input(Strong::new(Mutex::new(None), bank, input_storage, ())?);
    let backing = from_grant(bank, 1, storage)?;
    Ok(Prepared { input, backing })
}
impl<T, F: Future> Prepared<T, F> {
    pub(super) fn complete<C>(
        self,
        context: C,
        create: impl FnOnce(Input<T>, C) -> F,
    ) -> impl FnOnce(T) -> PaidFuture<F> {
        let Self { input, backing } = self;
        // The exact original captures enter the SAME already allocated F.
        // The callback owns this paid F even while Hyper has not invoked it.
        let future = PaidFuture::from_backing(create(input.clone(), context), backing);
        move |value| {
            {
                let mut slot = input.0.lock().unwrap_or_else(|error| error.into_inner());
                assert!(slot.is_none(), "one original upgrade input");
                *slot = Some(value);
            }
            // Axum invokes FnOnce before polling this future. No allocation or
            // admission follows acquisition of the upgraded socket.
            future
        }
    }
}

pub(super) struct Local {
    pub(super) ticket: super::Ticket,
    pub(super) publication: super::publication::LifeOwner,
    pub(super) stream: TcpStream,
    pub(super) payload_bytes: Arc<Semaphore>,
    pub(super) principal: crate::access::Principal,
}
pub(super) async fn local(input: Input<WebSocket>, context: Local) {
    let socket = input.take();
    let Local {
        ticket,
        publication,
        stream,
        payload_bytes,
        principal,
    } = context;
    tokio::select! {
        result = publication.run(ticket.run(async { Ok(super::bridge_tcp_websocket(socket, stream, &payload_bytes).await) })) => {
            match result {
                Ok(completion) => {
                    if let Err(err) = completion.result() {
                        tracing::debug!(error = %crate::output::error_message(err), "tunnel agent: local WebSocket bridge ended");
                    }
                }
                Err(err) => tracing::debug!(error = %crate::output::error_message(&err), "tunnel agent: local WebSocket lifetime ended"),
            }
        }
        _ = principal.revoked() => tracing::debug!("tunnel agent: local WebSocket authorization revoked"),
    }
}

pub(super) struct Remote {
    pub(super) epoch: super::lifecycle::GlobalEpoch,
    pub(super) remote: WebSocketStream<MaybeTlsStream<TcpStream>>,
    pub(super) principal: crate::access::Principal,
}
pub(super) async fn remote(input: Input<WebSocket>, context: Remote) {
    let socket = input.take();
    let Remote {
        epoch,
        remote,
        principal,
    } = context;
    tokio::select! {
        result = epoch.run(super::bridge_websockets(socket, remote)) => {
            if let Err(err) = result {
                tracing::debug!(error = %crate::output::error_message(&err), "tunnel agent: remote WebSocket bridge ended");
            }
        }
        _ = principal.revoked() => tracing::debug!("tunnel agent: remote WebSocket authorization revoked"),
    }
}

#[cfg(test)]
mod tests;
