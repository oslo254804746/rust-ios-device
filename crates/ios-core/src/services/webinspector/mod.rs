use tokio::time::Duration;

pub mod cdp_browser;

mod model;
mod wire;
pub use model::{
    Application, ApplicationPage, AutomationAvailability, Page, WebInspectorEvent, WirType,
};

mod client;
pub use client::WebInspectorClient;

mod inspector;
pub use inspector::InspectorSession;
mod automation;
pub use automation::{AutomationSession, By};

mod timing;

pub const SERVICE_NAME: &str = "com.apple.webinspector";
pub const RSD_SERVICE_NAME: &str = "com.apple.webinspector.shim.remote";
pub const SAFARI_BUNDLE_ID: &str = "com.apple.mobilesafari";

service_error!(
    WebInspectorError,
    between {
    #[error("JSON error: {0}")]
    Json(#[from] serde_json::Error),
    #[error("Web Inspector is not enabled on the device")]
    Disabled,
    },
    after {
    #[error("timed out waiting for webinspector response after {0:?}")]
    Timeout(Duration),
    },
);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn disabled_error_renders_clear_action() {
        let message = WebInspectorError::Disabled.to_string();
        assert!(message.contains("Web Inspector is not enabled"));
    }
}
