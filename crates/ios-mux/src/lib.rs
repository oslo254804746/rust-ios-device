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

use crate::protocol::DeviceEntryRaw;

/// A connected iOS device discovered via usbmuxd.
#[derive(Debug, Clone)]
pub struct MuxDevice {
    pub device_id: u32,
    pub serial_number: String, // UDID
    pub connection_type: String,
    pub product_id: u16,
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
#[derive(Debug, thiserror::Error)]
pub enum MuxError {
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
    #[error("protocol error: {0}")]
    Protocol(String),
    #[error("device not found: {0}")]
    DeviceNotFound(String),
}
