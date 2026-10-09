//! Lockdown protocol client with TLS session establishment, pair record management,
//! and device pairing (including supervised P12 pairing).
//!
//! Use [`LockdownClient`] to read device values, start services, and manage pairing.

mod checked_session;
pub mod client;
mod error_context;
mod response_error;
crate::redaction::error_debug!(LockdownError);
pub(crate) mod pair_record;
#[cfg(feature = "tunnel")]
pub mod pairing;
pub mod protocol;
pub(crate) mod session;
#[cfg(feature = "supervised-pair")]
pub(crate) mod supervised_pair;

pub(crate) use checked_session::start_existing_trust_session;
pub use client::LockdownClient;
pub use pair_record::{default_pair_record_path, PairRecord, PairRecordError};
pub use protocol::{
    recv_lockdown, send_lockdown, GetValueRequest, GetValueResponse, QueryTypeRequest,
    QueryTypeResponse, RemoveValueRequest, SetValueRequest, StartServiceRequest,
    StartServiceResponse, StartSessionRequest, StartSessionResponse, StopSessionRequest,
    ValueOperationResponse, LOCKDOWN_PORT,
};
pub use response_error::LockdownResponseError;
pub use session::{
    handshake_only_service_tls, start_lockdown_session, start_service,
    start_service_preserving_errors, strip_service_tls, wrap_service_tls, CORE_DEVICE_PROXY,
};
#[cfg(feature = "supervised-pair")]
pub use supervised_pair::{pair_supervised, save_pair_record, FullPairRecord};

/// Service info returned by StartService.
#[derive(Debug, Clone)]
pub struct ServiceInfo {
    pub port: u16,
    pub enable_service_ssl: bool,
}

/// Errors from lockdown operations.
#[derive(thiserror::Error)]
pub enum LockdownError {
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
    #[error("protocol error: {0}")]
    Protocol(String),
    #[error("pair record error: {0}")]
    PairRecord(#[from] PairRecordError),
}
