use crate::lockdown::pair_record::PairRecordError;

/// Aggregated error type for ios-core operations.
#[derive(Debug, thiserror::Error)]
pub enum CoreError {
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
    #[error("usbmuxd error: {0}")]
    Mux(#[from] crate::mux::MuxError),
    #[error("lockdown error: {0}")]
    Lockdown(#[from] crate::lockdown::LockdownError),
    #[error("pair record error: {0}")]
    PairRecord(#[from] PairRecordError),
    #[error("tunnel error: {0}")]
    Tunnel(#[from] crate::tunnel::TunnelError),
    #[error("device not found: {0}")]
    DeviceNotFound(String),
    #[error("operation not supported: {0}")]
    Unsupported(String),
    #[error("{0}")]
    Other(String),
}
