//! Lockdown protocol client with TLS session establishment, pair record management,
//! and device pairing (including supervised P12 pairing).
//!
//! Use [`LockdownClient`] to read device values, start services, and manage pairing.

pub mod client;
pub mod pair_record;
pub mod pairing;
pub mod protocol;
pub mod session;
pub mod supervised_pair;

pub use client::LockdownClient;
pub use pair_record::{PairRecord, PairRecordError};
pub use protocol::LOCKDOWN_PORT;
pub use session::CORE_DEVICE_PROXY;

/// Service info returned by StartService.
#[derive(Debug, Clone)]
pub struct ServiceInfo {
    pub port: u16,
    pub enable_service_ssl: bool,
}

/// Errors from lockdown operations.
#[derive(Debug, thiserror::Error)]
pub enum LockdownError {
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
    #[error("protocol error: {0}")]
    Protocol(String),
    #[error("pair record error: {0}")]
    PairRecord(#[from] PairRecordError),
}
