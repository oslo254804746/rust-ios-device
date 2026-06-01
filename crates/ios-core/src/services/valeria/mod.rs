//! Experimental Valeria / QuickTime USB video capture.

mod coremedia;
mod frame;
mod protocol;
mod session;
mod usb;

pub use frame::H264Frame;
pub use session::{CaptureSummary, ValeriaSession};
pub use usb::UsbValeriaCapture;

#[derive(Debug, thiserror::Error)]
pub enum ValeriaError {
    #[error("USB error: {0}")]
    Usb(String),
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
    #[error("protocol error: {0}")]
    Protocol(String),
    #[error("device not found: {0}")]
    DeviceNotFound(String),
    #[error("multiple candidate devices found; pass --udid")]
    MultipleDevices,
    #[error("capture stopped")]
    Stopped,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CaptureOptions {
    pub udid: Option<String>,
    pub queue_capacity: usize,
}

impl Default for CaptureOptions {
    fn default() -> Self {
        Self {
            udid: None,
            queue_capacity: 90,
        }
    }
}
