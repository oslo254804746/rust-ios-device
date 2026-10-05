//! CLI one-shot and interactive task orchestration. Core owns service/session state.
use super::connection::connect_webinspector_with_timeout;
use super::workflows::{
    is_inspectable_page_type, page_matches_selector, page_type_name, remaining_webinspector_time,
    runtime_evaluate_params, runtime_evaluation_error, wait_for_application, webinspector_deadline,
};
use anyhow::{anyhow, Context, Result};
use ios_core::webinspector::{
    ApplicationPage, AutomationSession, InspectorSession, WebInspectorClient, RSD_SERVICE_NAME,
    SERVICE_NAME,
};
use serde::Serialize;
#[cfg(test)]
use serde_json::json;
use std::time::Duration;

mod control;
mod input;
mod interactive;
mod oneshot;
mod session_owner;
mod source;
#[cfg(test)]
mod test_peer;

#[derive(Debug, Serialize)]
struct OpenedTabRow {
    application_id: String,
    bundle_identifier: String,
    application_name: String,
    pid: u64,
    page_id: u64,
    page_key: String,
    page_type: String,
    title: Option<String>,
    url: Option<String>,
}

#[derive(Debug, Serialize)]
struct LaunchResult {
    bundle_id: String,
    application_id: String,
    application_name: String,
    pid: u64,
    page_id: u64,
    page_key: String,
    page_type: String,
    url: Option<String>,
    title: String,
    session_id: String,
    inspector_connection_id: String,
    automation_connection_id: Option<String>,
    service_name: &'static str,
    uses_rsd: bool,
}

pub(super) fn finite_duration(seconds: f64) -> Result<Duration> {
    control::finite_duration(seconds)
}

pub(super) async fn run_opened_tabs(
    udid: Option<String>,
    timeout: Duration,
    json_output: bool,
) -> Result<()> {
    oneshot::run_command(udid, oneshot::Command::Tabs, timeout, json_output).await
}

pub(super) async fn run_eval(
    udid: Option<String>,
    app_id: Option<String>,
    bundle_id: String,
    page_selector: &str,
    expression: &str,
    timeout: Duration,
    json_output: bool,
) -> Result<()> {
    oneshot::run_command(
        udid,
        oneshot::Command::Eval {
            app_id,
            bundle_id,
            page_selector,
            expression,
        },
        timeout,
        json_output,
    )
    .await
}

pub(super) async fn run_launch(
    udid: Option<String>,
    bundle_id: &str,
    url: Option<&str>,
    timeout: Duration,
    json_output: bool,
) -> Result<()> {
    oneshot::run_command(
        udid,
        oneshot::Command::Launch { bundle_id, url },
        timeout,
        json_output,
    )
    .await
}

// The original source/cancellation assertions exercise the same new driver.
#[cfg(test)]
async fn launch_result_with_signal<T>(
    operation: impl std::future::Future<Output = Result<T>>,
    signal: impl std::future::Future<Output = std::io::Result<()>>,
    timeout: Duration,
) -> Result<T> {
    control::Budget::new(timeout, "launch")?
        .drive(operation, signal)
        .await
}

#[allow(clippy::too_many_arguments)]
pub(super) async fn run_js_shell(
    udid: Option<String>,
    url: Option<&str>,
    bundle_id: Option<&str>,
    page_id: Option<&str>,
    open_safari: bool,
    continue_on_error: bool,
    timeout: Duration,
    json_output: bool,
) -> Result<()> {
    interactive::run_command(
        udid,
        interactive::Options {
            url,
            bundle_id,
            page_id,
            open_safari,
            continue_on_error,
            timeout,
            json: json_output,
        },
    )
    .await
}

fn opened_tab_row(page: ApplicationPage) -> OpenedTabRow {
    let ApplicationPage { application, page } = page;
    OpenedTabRow {
        application_id: application.id,
        bundle_identifier: application.bundle_identifier,
        application_name: application.name,
        pid: application.pid,
        page_id: page.id,
        page_key: page.listing_key,
        page_type: serde_json::to_string(&page.page_type)
            .unwrap_or_else(|_| "\"unknown\"".to_string())
            .trim_matches('"')
            .to_string(),
        title: page.title,
        url: page.url,
    }
}

#[cfg(test)]
mod tests {
    use super::super::workflows::runtime_remote_object;
    use super::*;
    use ios_core::webinspector::WirType;
    use serde_json::Value as JsonValue;

    struct OwnedPending(std::sync::Arc<std::sync::atomic::AtomicBool>);

    impl std::future::Future for OwnedPending {
        type Output = Result<()>;

        fn poll(
            self: std::pin::Pin<&mut Self>,
            _: &mut std::task::Context<'_>,
        ) -> std::task::Poll<Self::Output> {
            std::task::Poll::Pending
        }
    }

    impl Drop for OwnedPending {
        fn drop(&mut self) {
            self.0.store(true, std::sync::atomic::Ordering::SeqCst);
        }
    }

    #[tokio::test]
    async fn local_launch_cancel_keeps_context_typed_source_and_unknown_effects() {
        let dropped = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let error = {
            let operation = OwnedPending(dropped.clone());
            tokio::pin!(operation);
            launch_result_with_signal(
                &mut operation,
                std::future::ready(Ok(())),
                Duration::from_secs(1),
            )
            .await
            .unwrap_err()
        };
        assert!(dropped.load(std::sync::atomic::Ordering::SeqCst));
        assert_eq!(error.to_string(), "webinspector launch cancelled");
        assert!(error
            .chain()
            .any(|source| source.is::<ios_core::OperationCancelled>()));
        assert_eq!(
            ios_core::classify_error(error.as_ref()),
            ios_core::ErrorClass::Cancelled
        );
        let message = crate::output::operation_failure_message(
            crate::output::ErrorFormat::Json,
            "webinspector",
            &error,
        )
        .unwrap();
        let body: JsonValue = serde_json::from_str(&message).unwrap();
        assert_eq!(body["error"]["class"], "cancelled");
        assert_eq!(body["error"]["effects"], "unknown");
        assert_eq!(body["error"]["exit_code"], 1);
    }

    #[tokio::test]
    async fn failed_signal_registration_keeps_io_instead_of_claiming_cancellation() {
        let dropped = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let error = {
            let operation = OwnedPending(dropped.clone());
            tokio::pin!(operation);
            launch_result_with_signal(
                &mut operation,
                std::future::ready(Err(std::io::Error::new(
                    std::io::ErrorKind::PermissionDenied,
                    "synthetic-private-signal-error",
                ))),
                Duration::from_secs(1),
            )
            .await
            .unwrap_err()
        };
        assert!(dropped.load(std::sync::atomic::Ordering::SeqCst));
        assert_eq!(error.to_string(), "failed waiting for Ctrl+C");
        assert_eq!(
            ios_core::classify_error(error.as_ref()),
            ios_core::ErrorClass::PermissionDenied
        );
        assert!(!error
            .chain()
            .any(|source| source.is::<ios_core::OperationCancelled>()));
        let io = error
            .chain()
            .find_map(|source| source.downcast_ref::<std::io::Error>())
            .unwrap();
        assert_eq!(io.to_string(), "synthetic-private-signal-error");
    }

    #[tokio::test]
    async fn launch_timeout_still_retains_elapsed_and_releases_the_owned_future() {
        let dropped = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let error = {
            let operation = OwnedPending(dropped.clone());
            tokio::pin!(operation);
            launch_result_with_signal(
                &mut operation,
                std::future::pending::<std::io::Result<()>>(),
                Duration::from_millis(20),
            )
            .await
            .unwrap_err()
        };
        assert!(dropped.load(std::sync::atomic::Ordering::SeqCst));
        assert_eq!(
            error.to_string(),
            "webinspector launch timed out after 20ms"
        );
        assert_eq!(
            ios_core::classify_error(error.as_ref()),
            ios_core::ErrorClass::Timeout
        );
        assert!(error
            .chain()
            .any(|source| source.is::<tokio::time::error::Elapsed>()));
    }

    #[tokio::test]
    async fn completed_launch_result_is_returned_without_cancellation_or_retry() {
        assert_eq!(
            launch_result_with_signal(
                std::future::ready(Ok(7u64)),
                std::future::pending::<std::io::Result<()>>(),
                Duration::from_secs(1),
            )
            .await
            .unwrap(),
            7
        );
    }

    #[test]
    fn shell_result_and_page_type_names_are_stable() {
        assert_eq!(page_type_name(&WirType::WebPage), "web_page");
        let response = json!({"result": {"type": "string", "value": "你好"}});
        assert_eq!(
            runtime_remote_object(&response)
                .and_then(|value| value.get("value"))
                .and_then(JsonValue::as_str),
            Some("你好")
        );
    }
}
