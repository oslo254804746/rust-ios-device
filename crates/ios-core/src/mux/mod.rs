//! usbmuxd client for iOS device discovery and USB/network connection multiplexing.
//!
//! Provides [`MuxClient`] for connecting to usbmuxd and establishing TCP connections
//! to device services, plus [`MuxEvent`] for real-time device attach/detach notifications.

pub mod client;
pub mod connection;
pub mod listener;
pub mod protocol;

pub use client::MuxClient;
pub use listener::MuxEvent;

use crate::mux::protocol::DeviceEntryRaw;

/// A connected iOS device discovered via usbmuxd.
#[derive(Clone)]
pub struct MuxDevice {
    pub device_id: u32,
    pub serial_number: String, // UDID
    pub connection_type: String,
    pub product_id: u16,
}

impl std::fmt::Debug for MuxDevice {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MuxDevice")
            .field("device_id", &self.device_id)
            .field(
                "serial_number",
                &crate::redaction::diagnostic_id(&self.serial_number),
            )
            .field("product_id", &self.product_id)
            .finish_non_exhaustive()
    }
}

impl MuxDevice {
    pub(crate) fn from_raw(raw: DeviceEntryRaw) -> Self {
        Self {
            device_id: raw.device_id,
            serial_number: raw.properties.serial_number,
            connection_type: raw.properties.connection_type,
            product_id: raw.properties.product_id.unwrap_or(0),
        }
    }
}

/// Errors from usbmuxd operations.
#[derive(thiserror::Error)]
pub enum MuxError {
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
    #[error("protocol error: {0}")]
    Protocol(String),
    #[error("device not found: {}", crate::redaction::diagnostic_id(.0))]
    DeviceNotFound(String),
}

impl std::fmt::Debug for MuxError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Io(error) => f
                .debug_struct("MuxError::Io")
                .field("kind", &error.kind())
                .field("os_code", &error.raw_os_error())
                .finish_non_exhaustive(),
            Self::Protocol(_) => f.debug_struct("MuxError::Protocol").finish_non_exhaustive(),
            Self::DeviceNotFound(identifier) => f
                .debug_tuple("MuxError::DeviceNotFound")
                .field(&crate::redaction::diagnostic_id(identifier))
                .finish(),
        }
    }
}
