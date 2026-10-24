//! Actual HTTP control work owns its original future allocation. Core stop,
//! clear and terminal Drop retain their original zero-quota control rules.
use std::future::Future;

use anyhow::Result;
use axum::response::Response;

use super::{future_storage::PaidFuture, marker_identity::Cutover, Principal, TunnelAgentState};

async fn stop_future(
    state: &TunnelAgentState,
    principal: &Principal,
    udid: &str,
    mut cutover: Cutover<3>,
) -> std::result::Result<Result<bool>, Response> {
    // The same existing Principal guard surrounds the same core stop. Only
    // its original ScopeDenied->Response conversion is inside this owned F.
    principal
        .run(state.stop_tunnel(udid, &mut cutover))
        .await
        .map_err(Into::into)
}

pub(super) fn prepare_stop<'a>(
    state: &'a TunnelAgentState,
    principal: &'a Principal,
    udid: &'a str,
) -> Result<PaidFuture<impl Future<Output = std::result::Result<Result<bool>, Response>> + 'a>> {
    let backing = super::metadata_vec::exact_backing(&state.inner.global_metadata_bytes, 1)?;
    let cutover = state.prepare_cutover::<3>()?;
    Ok(PaidFuture::from_backing(
        stop_future(state, principal, udid, cutover),
        backing,
    ))
}

async fn epoch_future(state: &TunnelAgentState, mut cutover: Cutover<2>) {
    state.clear(&mut cutover).await;
}

pub(super) fn prepare_epoch(
    state: &TunnelAgentState,
) -> Result<PaidFuture<impl Future<Output = ()> + '_>> {
    // /epoch has never added a Principal cancellation guard to clear. Preserve
    // that original infallible body and its own zero-quota internal callers.
    let backing = super::metadata_vec::exact_backing(&state.inner.global_metadata_bytes, 1)?;
    let cutover = state.prepare_cutover::<2>()?;
    Ok(PaidFuture::from_backing(
        epoch_future(state, cutover),
        backing,
    ))
}

#[cfg(test)]
pub(super) fn stop_bytes() -> usize {
    fn size<A, B, C, D, F: Future>(_create: impl FnOnce(A, B, C, D) -> F) -> usize {
        std::mem::size_of::<F>()
    }
    size(stop_future)
}

#[cfg(test)]
pub(super) fn epoch_bytes() -> usize {
    fn size<A, B, F: Future>(_create: impl FnOnce(A, B) -> F) -> usize {
        std::mem::size_of::<F>()
    }
    size(epoch_future)
}
