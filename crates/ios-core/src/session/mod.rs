//! Shared bounded ownership for finite SDK operations.
//! Public entry points consume leases and return owned observations only.
//!
//! Admission is bounded to 64 leases in an explicitly shared pool and four
//! per owner, without a waiter queue. A pool is not a process-wide singleton
//! across separately linked C/Python libraries. Close seals new admission and
//! cancels active work, but retains an unpolled lease's count until real Drop.
//! It reports local release, without a remote teardown acknowledgement.
//!
//! Only the finite prepared entry below drives a public operation. Generic
//! closures and borrowed contexts remain crate-internal; successful values
//! cannot be transports. A native Ready(Err) candidate can survive an upper
//! future's Pending until cancellation/expiry. Core code which handles that
//! error before a non-IO wait must explicitly acknowledge the handling; this
//! framework cannot infer arbitrary future intent. Zero-byte reads are never
//! manufactured into native errors. Credentials, discovery, dialed sockets
//! or aliases retained by the caller are outside this operation's ownership.

mod operation;
mod owner;

pub(crate) use operation::OperationContext;
pub use operation::{OperationCancel, OperationOptions};
pub use owner::{OperationLease, SessionOwner, SharedAdmission};

use crate::{CoreError, ErrorClass};
use std::sync::Arc;
use tokio::io::{AsyncRead, AsyncWrite};

crate::redaction::error_debug!(SessionFailure);

#[derive(thiserror::Error)]
#[error("SDK session: {reason}")]
pub(crate) struct SessionFailure {
    class: ErrorClass,
    reason: &'static str,
}

impl SessionFailure {
    pub(crate) fn class(&self) -> ErrorClass {
        self.class
    }
}

fn failure(class: ErrorClass, reason: &'static str) -> CoreError {
    std::io::Error::other(SessionFailure { class, reason }).into()
}

// Core-only registration prevents a transport from escaping the lease scope.
pub(crate) trait OperationValue: Send + 'static {}
impl OperationValue for () {}
impl OperationValue for bool {}
impl OperationValue for u16 {}
impl OperationValue for u32 {}
impl OperationValue for u64 {}
impl OperationValue for String {}
impl OperationValue for Vec<u8> {}
impl OperationValue for bytes::Bytes {}

/// Finite StartService metadata observed on an existing-trust session.
///
/// The advertised service port is not dialed. This does not certify service
/// functionality or peer liveness, and does not acknowledge Apple cleanup.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ServiceObservation {
    pub port: u16,
    pub enable_service_ssl: bool,
    pub enable_session_ssl: bool,
}

impl OperationValue for ServiceObservation {}

/// Prepare a finite QueryType/StartSession/TLS/StartService observation.
///
/// The caller already owns the raw stream and pairing record, and obtains the
/// lease before preparing/enqueuing work. The lease's absolute total includes
/// time before this future is polled. The selector is explicit, nonempty, has
/// no embedded NUL, and is at most 4096 UTF-8 bytes; rejection performs no IO.
///
/// The operation never pairs, retries, enumerates, opens the advertised port,
/// or activates a tunnel. All outcomes destroy the original stream before
/// releasing admission. No raw transport or borrowed checked session escapes.
pub fn prepared_service_observation<S>(
    stream: S,
    pair_record: Arc<crate::PairRecord>,
    lease: OperationLease,
    service: String,
) -> Result<
    impl std::future::Future<Output = Result<ServiceObservation, CoreError>> + Send,
    CoreError,
>
where
    S: AsyncRead + AsyncWrite + Unpin + Send + 'static,
{
    let error = if service.is_empty() || service.as_bytes().contains(&0) {
        Some(failure(
            ErrorClass::InvalidInput,
            "invalid service selector",
        ))
    } else if service.len() > 4096 {
        Some(failure(
            ErrorClass::ResourceLimit,
            "service selector exceeds byte limit",
        ))
    } else {
        None
    };
    if let Some(error) = error {
        drop(stream);
        drop(pair_record);
        drop(lease);
        return Err(error);
    }
    Ok(lease.run(move |context| {
        Box::pin(async move {
            let mut session = crate::lockdown::start_existing_trust_session(
                context,
                stream,
                pair_record.as_ref(),
            )
            .await?;
            let enable_session_ssl = session.uses_tls();
            let info = session.start_service(&service).await?;
            Ok(ServiceObservation {
                port: info.port,
                enable_service_ssl: info.enable_service_ssl,
                enable_session_ssl,
            })
        })
    }))
}

#[cfg(test)]
mod tests;
