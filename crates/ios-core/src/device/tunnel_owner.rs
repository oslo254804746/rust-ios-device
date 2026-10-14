//! Optional caller ownership before a connection's initial RSD probe.

use std::sync::Arc;

use crate::{CoreError, TunnelHandle};

/// Reserves caller-owned cleanup capacity before each tunnel strategy attempt.
///
/// Owner-aware connection functions call this before opening the strategy's
/// transport. A connection can try several targets, so each attempt gets its
/// own reservation. Returning an error prevents that attempt's transport work.
/// The implementation must remain synchronous and avoid blocking or reentering
/// the connection. Ordinary connection functions do not invoke this interface.
pub trait TunnelOwner: Send + Sync {
    fn reserve(&self) -> Result<Box<dyn TunnelReservation + '_>, CoreError>;
}

/// Caller cleanup responsibility for one attempted tunnel runtime.
///
/// Dropping an unattached reservation releases its capacity. After attachment,
/// Drop must retain the original handle in the caller's cleanup owner and
/// request stop. It cannot await a native join. A successful `complete` transfers
/// that responsibility to the owner until the caller explicitly publishes or
/// retires the returned device. Neither method certifies native completion.
pub trait TunnelReservation: Send {
    /// Optional bank for application-owned tunnel runtime buffer capacity.
    /// Defaults to legacy unmetered construction. A manager returns its existing
    /// shared bank before workers start; permits follow each actual buffer owner.
    /// This does not certify a complete native heap or allocator RSS bound.
    fn byte_bank(&self) -> Option<Arc<tokio::sync::Semaphore>> {
        None
    }

    /// Retain this exact Arc synchronously, before the first RSD probe await.
    /// An error rejects the connection; retain an attached owner for cleanup
    /// even when rejecting it. Core also requests stop on rejection.
    fn attach(&mut self, handle: Arc<TunnelHandle>) -> Result<(), CoreError>;

    /// Transfer the attached cleanup responsibility as the connection returns.
    /// The owner must still retire it if the outer call is later cancelled or
    /// rejected before publication. This is not the publication commit itself.
    fn complete(&mut self);
}
