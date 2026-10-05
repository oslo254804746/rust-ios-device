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

mod retained;
#[cfg(test)]
mod timing;

#[cfg(test)]
mod lifecycle_session_tests;

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

#[derive(thiserror::Error)]
#[error("WebInspector deadline elapsed after {duration:?}")]
struct WebInspectorTimeout {
    duration: Duration,
    #[source]
    source: tokio::time::error::Elapsed,
}

crate::redaction::error_debug!(WebInspectorTimeout);

#[derive(thiserror::Error)]
#[error("WebInspector {resource} exceeded the local limit of {limit}")]
pub(crate) struct WebInspectorLimit {
    resource: &'static str,
    limit: u64,
}

crate::redaction::error_debug!(WebInspectorLimit);

fn local_limit(resource: &'static str, limit: u64) -> WebInspectorError {
    native_error(
        std::io::ErrorKind::Other,
        "retain WebInspector state",
        WebInspectorLimit { resource, limit },
    )
}

impl WebInspectorError {
    /// The budget reported by a WebInspector-issued deadline or the legacy
    /// Timeout variant. Other IO TimedOut errors do not supply this value.
    /// This identifies the issuer, not safe retry or completed cleanup.
    pub fn timeout_duration(&self) -> Option<Duration> {
        if let Self::Timeout(duration) = self {
            return Some(*duration);
        }
        let mut next: Option<&(dyn std::error::Error + 'static)> = Some(self);
        for _ in 0..32 {
            let Some(error) = next else { break };
            if let Some(error) = error.downcast_ref::<WebInspectorTimeout>() {
                return Some(error.duration);
            }
            next = error.source();
        }
        None
    }
}

pub(super) fn native_error(
    kind: std::io::ErrorKind,
    phase: &'static str,
    source: impl std::error::Error + Send + Sync + 'static,
) -> WebInspectorError {
    WebInspectorError::Io(crate::error::context::retained_io(kind, phase, source))
}

pub(super) fn native_timeout(
    duration: Duration,
    phase: &'static str,
    source: tokio::time::error::Elapsed,
) -> WebInspectorError {
    native_error(
        std::io::ErrorKind::TimedOut,
        phase,
        WebInspectorTimeout { duration, source },
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn native_discovery_timeout_budget_scope_is_typed_bounded_and_does_not_format() {
        use std::sync::{
            atomic::{AtomicUsize, Ordering},
            Arc,
        };
        let duration = Duration::from_millis(5);
        let elapsed = tokio::time::timeout(duration, std::future::pending::<()>())
            .await
            .unwrap_err();
        let error = native_timeout(duration, "owned deadline", elapsed);
        assert_eq!(error.timeout_duration(), Some(duration));
        assert!(crate::test_native::has_source::<tokio::time::error::Elapsed>(&error));
        assert_eq!(
            WebInspectorError::Timeout(duration).timeout_duration(),
            Some(duration)
        );
        let elapsed = tokio::time::timeout(duration, std::future::pending::<()>())
            .await
            .unwrap_err();
        let unissued =
            WebInspectorError::Io(std::io::Error::new(std::io::ErrorKind::TimedOut, elapsed));
        assert_eq!(crate::classify_error(&unissued), crate::ErrorClass::Timeout);
        assert_eq!(unissued.timeout_duration(), None);
        #[derive(Debug)]
        struct Cycle(Arc<AtomicUsize>);
        impl std::fmt::Display for Cycle {
            fn fmt(&self, _: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                panic!("private source must never be formatted")
            }
        }
        impl std::error::Error for Cycle {
            fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
                self.0.fetch_add(1, Ordering::SeqCst);
                Some(self)
            }
        }
        let reads = Arc::new(AtomicUsize::new(0));
        let cycle = WebInspectorError::Io(std::io::Error::other(Cycle(Arc::clone(&reads))));
        assert_eq!(cycle.timeout_duration(), None);
        assert!(reads.load(Ordering::SeqCst) > 0 && reads.load(Ordering::SeqCst) <= 32);
    }

    #[test]
    fn disabled_error_renders_clear_action() {
        let message = WebInspectorError::Disabled.to_string();
        assert!(message.contains("Web Inspector is not enabled"));
    }
}
