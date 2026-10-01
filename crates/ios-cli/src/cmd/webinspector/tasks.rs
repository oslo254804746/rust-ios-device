//! CLI one-shot and interactive task orchestration. Core owns service/session state.
use super::connection::{connect_webinspector, connect_webinspector_with_timeout};
use super::shell::{print_runtime_evaluation_human, run_js_shell_loop};
use super::workflows::{
    is_inspectable_page_type, page_matches_selector, page_type_name, remaining_webinspector_time,
    runtime_evaluate_params, runtime_evaluation_error, wait_for_application,
    wait_for_inspectable_page, webinspector_deadline,
};
use super::SESSION_CLEANUP_MAX;
use anyhow::{anyhow, Context, Result};
use ios_core::webinspector::{
    ApplicationPage, AutomationSession, InspectorSession, WebInspectorClient, RSD_SERVICE_NAME,
    SAFARI_BUNDLE_ID, SERVICE_NAME,
};
use serde::Serialize;
use serde_json::json;
use std::time::Duration;

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

pub(super) async fn run_opened_tabs(
    udid: &str,
    timeout: Duration,
    json_output: bool,
) -> Result<()> {
    let (_device, stream, _use_rsd) = connect_webinspector_with_timeout(udid, timeout).await?;
    let mut client = WebInspectorClient::new(stream);
    client.start(timeout).await?;
    let pages = client.open_application_pages(timeout).await?;
    let rows = pages.into_iter().map(opened_tab_row).collect::<Vec<_>>();

    if json_output {
        crate::output::outln!("{}", serde_json::to_string_pretty(&rows)?);
    } else if rows.is_empty() {
        crate::output::outln!("No inspectable pages reported");
    } else {
        for row in rows {
            crate::output::outln!(
                "{} [{}] page={} {} {}",
                row.application_name,
                row.bundle_identifier,
                row.page_id,
                row.title.as_deref().unwrap_or("<no title>"),
                row.url.as_deref().unwrap_or("<no url>")
            );
            crate::output::outln!("  app_id: {}", row.application_id);
            crate::output::outln!("  type: {}", row.page_type);
        }
    }
    Ok(())
}

pub(super) async fn run_eval(
    udid: &str,
    app_id: Option<String>,
    bundle_id: String,
    page_selector: &str,
    expression: &str,
    timeout: Duration,
    json_output: bool,
) -> Result<()> {
    let (_device, stream, _use_rsd) = connect_webinspector_with_timeout(udid, timeout).await?;
    let mut client = WebInspectorClient::new(stream);
    client.start(timeout).await?;
    client.open_application_pages(timeout).await?;

    let application_id = match app_id {
        Some(app_id) => app_id,
        None => client
            .application_by_bundle(&bundle_id)
            .map(|application| application.id.clone())
            .ok_or_else(|| anyhow!("bundle '{bundle_id}' is not currently inspectable"))?,
    };

    let page = client
        .application_pages(&application_id)
        .into_iter()
        .flat_map(|pages| pages.values())
        .find(|page| {
            is_inspectable_page_type(&page.page_type) && page_matches_selector(page, page_selector)
        })
        .cloned()
        .ok_or_else(|| {
            anyhow!(
                "inspectable page {page_selector} was not found under application '{application_id}'"
            )
        })?;
    let page_id = page.id;

    let mut session = InspectorSession::new(application_id.clone(), page_id);
    session.attach(&mut client, true, timeout).await?;
    let response = session
        .send_command_and_wait(
            &mut client,
            "Runtime.evaluate",
            runtime_evaluate_params(expression),
            timeout,
        )
        .await?;
    if let Some(error) = runtime_evaluation_error(&response) {
        return Err(anyhow!("webinspector evaluate error: {error}"));
    }

    if json_output {
        crate::output::outln!("{}", serde_json::to_string_pretty(&response)?);
    } else {
        print_runtime_evaluation_human(&response)?;
    }
    Ok(())
}

pub(super) async fn run_launch(
    udid: &str,
    bundle_id: &str,
    url: Option<&str>,
    timeout: Duration,
    json_output: bool,
) -> Result<()> {
    let operation = run_launch_inner(udid, bundle_id, url, timeout);
    tokio::pin!(operation);
    let result = tokio::select! {
        result = tokio::time::timeout(timeout, &mut operation) => {
            result.map_err(|_| anyhow!("webinspector launch timed out after {timeout:?}"))?
        }
        signal = tokio::signal::ctrl_c() => {
            signal.context("failed waiting for Ctrl+C")?;
            Err(anyhow!("webinspector launch cancelled"))
        }
    }?;

    if json_output {
        crate::output::outln!("{}", serde_json::to_string_pretty(&result)?);
    } else {
        crate::output::outln!("Launched {} (pid {})", result.bundle_id, result.pid);
        crate::output::outln!("  page: {}", result.page_id);
        crate::output::outln!("  url: {}", result.url.as_deref().unwrap_or("<no url>"));
        crate::output::outln!("  title: {}", result.title);
        crate::output::outln!("  session: {}", result.session_id);
        if let Some(connection_id) = result.automation_connection_id.as_deref() {
            crate::output::outln!("  automation connection: {connection_id}");
        }
        crate::output::outln!("  service: {}", result.service_name);
    }
    Ok(())
}

async fn run_launch_inner(
    udid: &str,
    bundle_id: &str,
    url: Option<&str>,
    timeout: Duration,
) -> Result<LaunchResult> {
    let deadline = webinspector_deadline(timeout)?;
    let (device, stream, uses_rsd) = connect_webinspector(udid).await?;
    let mut client = WebInspectorClient::new(stream);
    client
        .start(remaining_webinspector_time(deadline, timeout)?)
        .await?;
    client.request_application_launch(bundle_id).await?;
    let application = wait_for_application(&mut client, bundle_id, deadline, timeout).await?;
    let mut automation = AutomationSession::new(
        application.id.clone(),
        application.bundle_identifier.clone(),
    );
    automation
        .attach(&mut client, remaining_webinspector_time(deadline, timeout)?)
        .await?;
    automation.start_session(&mut client).await?;
    if let Some(url) = url.filter(|url| !url.is_empty()) {
        automation.navigate(&mut client, url).await?;
    }
    let current_url = automation.current_url(&mut client).await?;
    let title = automation.get_title(&mut client).await?;
    let page = client
        .page(&application.id, automation.page_id())
        .cloned()
        .ok_or_else(|| anyhow!("automation page {} disappeared", automation.page_id()))?;
    let result = LaunchResult {
        bundle_id: application.bundle_identifier,
        application_id: application.id,
        application_name: application.name,
        pid: application.pid,
        page_id: page.id,
        page_key: page.listing_key,
        page_type: page_type_name(&page.page_type),
        url: current_url.or(page.url),
        title,
        session_id: automation.session_id().to_string(),
        inspector_connection_id: client.connection_id().to_string(),
        automation_connection_id: page.automation_connection_id,
        service_name: if uses_rsd {
            RSD_SERVICE_NAME
        } else {
            SERVICE_NAME
        },
        uses_rsd,
    };
    // Closing the browsing context is best effort. The result has already been
    // collected, and a device disconnect should not turn a successful launch
    // into a second attempt or another launch request.
    // Cleanup must not consume the operation deadline: the launch result is
    // already complete, and disconnecting the service is also safe cleanup.
    if let Ok(remaining) = remaining_webinspector_time(deadline, timeout) {
        let _ = tokio::time::timeout(
            remaining.min(SESSION_CLEANUP_MAX),
            automation.stop_session(&mut client),
        )
        .await;
    }
    drop(device);
    Ok(result)
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
        .start(remaining_webinspector_time(deadline, timeout)?)
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
        .map_err(|_| anyhow!("webinspector Safari launch timed out after {timeout:?}"))??;
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
