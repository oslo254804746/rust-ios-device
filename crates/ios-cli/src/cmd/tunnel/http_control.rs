//! Actual HTTP control work owns its original future allocation. Core stop,
//! clear and terminal Drop retain their original zero-quota control rules.
use std::future::Future;

use anyhow::Result;
use axum::response::Response;

use super::{future_storage::PaidFuture, Principal, TunnelAgentState};

async fn stop_future(
    state: &TunnelAgentState,
    principal: &Principal,
    udid: &str,
) -> std::result::Result<Result<bool>, Response> {
    // The same existing Principal guard surrounds the same core stop. Only
    // its original ScopeDenied->Response conversion is inside this owned F.
    principal
        .run(state.stop_tunnel(udid))
        .await
        .map_err(Into::into)
}

pub(super) fn prepare_stop<'a>(
    state: &'a TunnelAgentState,
    principal: &'a Principal,
    udid: &'a str,
) -> Result<PaidFuture<impl Future<Output = std::result::Result<Result<bool>, Response>> + 'a>> {
    PaidFuture::new(
        stop_future(state, principal, udid),
        &state.inner.global_metadata_bytes,
    )
}

pub(super) fn prepare_epoch(
    state: &TunnelAgentState,
) -> Result<PaidFuture<impl Future<Output = ()> + '_>> {
    // /epoch has never added a Principal cancellation guard to clear. Preserve
    // that original infallible body and its own zero-quota internal callers.
    PaidFuture::new(state.clear(), &state.inner.global_metadata_bytes)
}

#[cfg(test)]
pub(super) fn stop_bytes() -> usize {
    fn size<A, B, C, F: Future>(_create: impl FnOnce(A, B, C) -> F) -> usize {
        std::mem::size_of::<F>()
    }
    size(stop_future)
}

#[cfg(test)]
pub(super) fn epoch_bytes() -> usize {
    fn size<A, F: Future>(_create: impl FnOnce(A) -> F) -> usize {
        std::mem::size_of::<F>()
    }
    size(TunnelAgentState::clear)
}
