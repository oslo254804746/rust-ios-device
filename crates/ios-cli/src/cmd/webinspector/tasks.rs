//! CLI one-shot and interactive task orchestration. Core owns service/session state.
use super::connection::connect_webinspector_with_timeout;
use super::shell::run_js_shell_loop;
use super::workflows::{
    is_inspectable_page_type, page_matches_selector, page_type_name, remaining_webinspector_time,
    runtime_evaluate_params, runtime_evaluation_error, wait_for_application,
    wait_for_inspectable_page, webinspector_deadline,
};
use anyhow::{anyhow, Context, Result};
use ios_core::webinspector::{
    ApplicationPage, AutomationSession, InspectorSession, WebInspectorClient, RSD_SERVICE_NAME,
    SAFARI_BUNDLE_ID, SERVICE_NAME,
};
use serde::Serialize;
use serde_json::json;
use std::time::Duration;

mod control;
mod oneshot;

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
    udid: &str,
    url: Option<&str>,
    bundle_id: Option<&str>,
    page_id: Option<&str>,
    open_safari: bool,
    continue_on_error: bool,
    timeout: Duration,
    json_output: bool,
) -> Result<()> {
    let operation = run_js_shell_inner(
        udid,
        url,
        bundle_id,
        page_id,
        open_safari,
        continue_on_error,
        timeout,
        json_output,
    );
    tokio::pin!(operation);
    let mut ctrl_c = Box::pin(tokio::signal::ctrl_c());
    tokio::select! {
        result = &mut operation => result,
        signal = &mut ctrl_c => {
            signal.context("failed waiting for Ctrl+C")?;
            Err(anyhow!("webinspector js-shell cancelled"))
        }
    }
}

#[allow(clippy::too_many_arguments)]
async fn run_js_shell_inner(
    udid: &str,
    url: Option<&str>,
    bundle_id: Option<&str>,
    page_id: Option<&str>,
    open_safari: bool,
    continue_on_error: bool,
    timeout: Duration,
    json_output: bool,
) -> Result<()> {
    let deadline = webinspector_deadline(timeout)?;
    let (device, stream, uses_rsd) = connect_webinspector_with_timeout(udid, timeout).await?;
    let mut client = WebInspectorClient::new(stream);
    client
        .start_preserving_errors(remaining_webinspector_time(deadline, timeout)?)
        .await?;

    // Match go-ios: --open-safari starts Safari, while an explicitly supplied
    // --bundle-id still controls which page is selected. Without a filter,
    // --open-safari naturally selects Safari as its default target.
    let bundle_filter = if open_safari {
        bundle_id.or(Some(SAFARI_BUNDLE_ID))
    } else {
        bundle_id
    };
    if open_safari {
        let remaining = remaining_webinspector_time(deadline, timeout)?;
        tokio::time::timeout(
            remaining,
            client.request_application_launch(SAFARI_BUNDLE_ID),
        )
        .await
        .with_context(|| format!("webinspector Safari launch timed out after {timeout:?}"))??;
    }
    let (application_id, page) =
        wait_for_inspectable_page(&mut client, page_id, bundle_filter, deadline, timeout).await?;
    let application = client
        .applications()
        .get(&application_id)
        .cloned()
        .ok_or_else(|| anyhow!("application '{application_id}' disappeared"))?;
    let mut session = InspectorSession::new(application_id.clone(), page.id);
    session
        .attach(
            &mut client,
            true,
            remaining_webinspector_time(deadline, timeout)?,
        )
        .await?;

    if let Some(url) = url.filter(|url| !url.is_empty()) {
        let expression = format!(
            "window.location = {}",
            serde_json::to_string(url).context("failed encoding shell URL")?
        );
        let response = session
            .send_command_and_wait(
                &mut client,
                "Runtime.evaluate",
                runtime_evaluate_params(&expression),
                remaining_webinspector_time(deadline, timeout)?,
            )
            .await?;
        if let Some(error) = runtime_evaluation_error(&response) {
            return Err(anyhow!("webinspector navigation error: {error}"));
        }
    }

    let metadata = json!({
        "kind": "session",
        "bundle_id": application.bundle_identifier,
        "application_id": application.id,
        "page_id": page.id,
        "page_key": page.listing_key,
        "page_type": page_type_name(&page.page_type),
        "url": page.url,
        "title": page.title,
        "session_id": session.session_id(),
        "inspector_connection_id": client.connection_id(),
        "automation_connection_id": page.automation_connection_id,
        "service_name": if uses_rsd { RSD_SERVICE_NAME } else { SERVICE_NAME },
    });
    if json_output {
        crate::output::outln!("{}", serde_json::to_string(&metadata)?);
    } else {
        crate::output::outln!(
            "Connected to {} page={} {}",
            application.bundle_identifier,
            page.id,
            page.url.as_deref().unwrap_or("<no url>")
        );
    }

    run_js_shell_loop(
        &mut client,
        &mut session,
        timeout,
        continue_on_error,
        json_output,
    )
    .await?;
    drop(device);
    Ok(())
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
