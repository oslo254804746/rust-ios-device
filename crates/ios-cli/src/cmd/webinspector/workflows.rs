//! Reusable selection, discovery deadlines and runtime value interpretation.
//! Supplied client/snapshots only; no device dialing or HTTP ownership.
use super::diagnostics::diagnostic_cdp_response;
use super::PAGE_DISCOVERY_IDLE;
use anyhow::{anyhow, Result};
use ios_core::device::ServiceStream;
use ios_core::webinspector::{
    ApplicationPage, Page, WebInspectorClient, WebInspectorError, WirType,
};
use serde_json::{json, Value as JsonValue};
use std::time::Duration;
use tokio::time::Instant;

/// `InspectorSession::send_command_and_wait` returns the command's `result`
/// object, while a few older/fake transports hand the extra `result` wrapper
/// through. Accept both shapes at the CLI boundary.
pub(super) fn runtime_remote_object(response: &JsonValue) -> Option<&JsonValue> {
    response
        .pointer("/result/result")
        .or_else(|| response.pointer("/result"))
}

pub(super) fn runtime_evaluation_error(response: &JsonValue) -> Option<String> {
    if response.get("error").is_some() {
        return diagnostic_cdp_response(response.clone())
            .pointer("/error/message")
            .and_then(JsonValue::as_str)
            .map(ToOwned::to_owned);
    }
    let exception = response
        .get("exceptionDetails")
        .or_else(|| response.pointer("/result/exceptionDetails"));
    if let Some(exception) = exception {
        return Some(
            exception
                .get("text")
                .and_then(JsonValue::as_str)
                .or_else(|| {
                    exception
                        .pointer("/exception/description")
                        .and_then(JsonValue::as_str)
                })
                .map(ToOwned::to_owned)
                .unwrap_or_else(|| exception.to_string()),
        );
    }
    let remote = runtime_remote_object(response)?;
    if remote.get("subtype").and_then(JsonValue::as_str) == Some("error") {
        return Some(
            remote
                .get("description")
                .and_then(JsonValue::as_str)
                .map(ToOwned::to_owned)
                .unwrap_or_else(|| remote.to_string()),
        );
    }
    None
}

fn select_inspectable_page(
    client: &WebInspectorClient<ServiceStream>,
    page_id: Option<&str>,
    bundle_id: Option<&str>,
) -> Result<(String, Page)> {
    select_inspectable_page_from_pages(&client.open_pages_snapshot(), page_id, bundle_id)
}

pub(super) fn remaining_webinspector_time(
    deadline: Instant,
    timeout: Duration,
) -> Result<Duration> {
    let now = Instant::now();
    if now >= deadline {
        return Err(anyhow!(
            "webinspector operation timed out after {timeout:?}"
        ));
    }
    Ok(deadline.duration_since(now))
}

pub(super) async fn wait_for_application(
    client: &mut WebInspectorClient<ServiceStream>,
    bundle_id: &str,
    deadline: Instant,
    timeout: Duration,
) -> Result<ios_core::webinspector::Application> {
    loop {
        if let Some(application) = client.application_by_bundle(bundle_id).cloned() {
            return Ok(application);
        }
        request_connected_applications_with_deadline(client, deadline, timeout).await?;
        loop {
            if let Some(application) = client.application_by_bundle(bundle_id).cloned() {
                return Ok(application);
            }
            let wait = remaining_webinspector_time(deadline, timeout)?.min(PAGE_DISCOVERY_IDLE);
            match client.next_event_with_timeout(wait).await {
                Ok(_) => continue,
                Err(WebInspectorError::Timeout(_)) => break,
                Err(error) => return Err(error.into()),
            }
        }
    }
}

pub(super) async fn wait_for_inspectable_page(
    client: &mut WebInspectorClient<ServiceStream>,
    page_id: Option<&str>,
    bundle_id: Option<&str>,
    deadline: Instant,
    timeout: Duration,
) -> Result<(String, Page)> {
    loop {
        if let Ok(page) = select_inspectable_page(client, page_id, bundle_id) {
            return Ok(page);
        }
        request_connected_applications_with_deadline(client, deadline, timeout).await?;
        loop {
            if let Ok(page) = select_inspectable_page(client, page_id, bundle_id) {
                return Ok(page);
            }
            let wait = remaining_webinspector_time(deadline, timeout)?.min(PAGE_DISCOVERY_IDLE);
            match client.next_event_with_timeout(wait).await {
                Ok(_) => continue,
                Err(WebInspectorError::Timeout(_)) => break,
                Err(error) => return Err(error.into()),
            }
        }
    }
}

fn select_inspectable_page_from_pages(
    pages: &[ApplicationPage],
    page_id: Option<&str>,
    bundle_id: Option<&str>,
) -> Result<(String, Page)> {
    // Match go-ios's selector precedence: an explicit page key is a stable
    // identity and wins before the optional bundle filter is considered.
    if let Some(page_id) = page_id {
        if let Some(candidate) = pages.iter().find(|candidate| {
            page_matches_selector(&candidate.page, page_id)
                && is_inspectable_page_type(&candidate.page.page_type)
        }) {
            return Ok((candidate.application.id.clone(), candidate.page.clone()));
        }
    }
    let mut candidates = pages
        .iter()
        .filter(|candidate| is_inspectable_page_type(&candidate.page.page_type))
        .filter(|_| page_id.is_none())
        .filter(|candidate| {
            bundle_id.map_or(true, |bundle| {
                candidate.application.bundle_identifier == bundle
            })
        })
        .collect::<Vec<_>>();
    if candidates.is_empty() {
        let page = page_id.map(|id| format!(" page {id}")).unwrap_or_default();
        let bundle = bundle_id
            .map(|id| format!(" for bundle '{id}'"))
            .unwrap_or_default();
        return Err(anyhow!(
            "no inspectable WebInspector page found{page}{bundle}"
        ));
    }
    let candidate = candidates.remove(0);
    Ok((candidate.application.id.clone(), candidate.page.clone()))
}

async fn request_connected_applications_with_deadline(
    client: &mut WebInspectorClient<ServiceStream>,
    deadline: Instant,
    timeout: Duration,
) -> Result<()> {
    let remaining = remaining_webinspector_time(deadline, timeout)?;
    tokio::time::timeout(remaining, client.request_connected_applications())
        .await
        .map_err(|_| {
            anyhow!("webinspector application-list request timed out after {timeout:?}")
        })??;
    Ok(())
}

pub(super) fn page_type_name(page_type: &WirType) -> String {
    serde_json::to_string(page_type)
        .unwrap_or_else(|_| "\"unknown\"".to_string())
        .trim_matches('"')
        .to_string()
}

pub(super) fn is_inspectable_page_type(page_type: &WirType) -> bool {
    matches!(
        page_type,
        WirType::Web | WirType::WebPage | WirType::JavaScript
    )
}

pub(super) fn page_matches_selector(page: &Page, selector: &str) -> bool {
    page.listing_key == selector || page.id.to_string() == selector
}

/// A page number is only unique within its owning WebInspector application.
/// Qualify the CDP target ID so JS contexts/pages from different apps cannot
/// collide; bare selectors remain accepted by the CLI for compatibility.
pub(super) fn page_target_id(application_id: &str, page: &Page) -> String {
    format!("{application_id}:{}", page.listing_key)
}

pub(super) fn find_page_by_id(
    client: &WebInspectorClient<ServiceStream>,
    page_id: &str,
) -> Result<(String, Page)> {
    find_page_in_snapshot(&client.open_pages_snapshot(), page_id)
}

pub(super) fn find_page_in_snapshot(
    pages: &[ApplicationPage],
    page_id: &str,
) -> Result<(String, Page)> {
    if let Some((application_id, page_key)) = page_id.rsplit_once(':') {
        if let Some(page) = pages.iter().find(|candidate| {
            candidate.application.id == application_id
                && is_inspectable_page_type(&candidate.page.page_type)
                && candidate.page.listing_key == page_key
        }) {
            return Ok((page.application.id.clone(), page.page.clone()));
        }
    }
    for page in pages {
        if is_inspectable_page_type(&page.page.page_type)
            && page_matches_selector(&page.page, page_id)
        {
            return Ok((page.application.id.clone(), page.page.clone()));
        }
    }
    Err(anyhow!("inspectable page {page_id} not found"))
}

pub(super) fn runtime_evaluate_params(expression: &str) -> JsonValue {
    json!({
        "expression": expression,
        "objectGroup": "console",
        "includeCommandLineAPI": true,
        "doNotPauseOnExceptionsAndMuteConsole": false,
        "silent": false,
        "returnByValue": true,
        "generatePreview": true,
        "userGesture": true,
        "awaitPromise": false,
        "replMode": true,
        "allowUnsafeEvalBlockedByCSP": false,
        "uniqueContextId": "0.1"
    })
}

pub(super) fn duration_from_secs(seconds: f64) -> Result<Duration> {
    if !seconds.is_finite() || seconds < 0.0 {
        return Err(anyhow!("--timeout must be a finite non-negative number"));
    }
    Duration::try_from_secs_f64(seconds.max(0.1)).map_err(|_| anyhow!("--timeout is too large"))
}

pub(super) fn webinspector_deadline(timeout: Duration) -> Result<Instant> {
    Instant::now()
        .checked_add(timeout)
        .ok_or_else(|| anyhow!("--timeout is too large for the system clock"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use ios_core::webinspector::{Application, AutomationAvailability};

    #[test]
    fn page_selection_honors_page_and_bundle_filters() {
        let application = |id: &str, bundle: &str| Application {
            id: id.into(),
            bundle_identifier: bundle.into(),
            pid: 42,
            name: bundle.into(),
            availability: AutomationAvailability::Available,
            is_active: true,
            is_proxy: false,
            is_ready: true,
            host_application_identifier: None,
        };
        let pages = vec![
            ApplicationPage {
                application: application("PID:42", "com.example.one"),
                page: Page {
                    id: 1,
                    listing_key: "1".into(),
                    page_type: WirType::WebPage,
                    title: Some("first".into()),
                    url: Some("https://one.example".into()),
                    automation_is_paired: None,
                    automation_name: None,
                    automation_version: None,
                    automation_session_id: None,
                    automation_connection_id: None,
                },
            },
            ApplicationPage {
                application: application("PID:43", "com.example.two"),
                page: Page {
                    id: 2,
                    listing_key: "tab-two".into(),
                    page_type: WirType::JavaScript,
                    title: Some("second".into()),
                    url: Some("https://two.example".into()),
                    automation_is_paired: None,
                    automation_name: None,
                    automation_version: None,
                    automation_session_id: None,
                    automation_connection_id: None,
                },
            },
        ];
        let (application_id, page) =
            select_inspectable_page_from_pages(&pages, Some("tab-two"), Some("com.example.two"))
                .expect("matching page should be selected");
        assert_eq!(application_id, "PID:43");
        assert_eq!(page.id, 2);
        let (application_id, page) =
            select_inspectable_page_from_pages(&pages, Some("tab-two"), Some("com.example.other"))
                .expect("an explicit page id takes precedence over bundle filtering");
        assert_eq!(application_id, "PID:43");
        assert_eq!(page.id, 2);
        assert!(select_inspectable_page_from_pages(&pages, Some("99"), None).is_err());
    }
    #[test]
    fn shell_protocol_failure_is_safe_while_requested_js_exception_keeps_its_value() {
        let secret = "synthetic-private-credential-device-data";
        let response =
            json!({"id": 1, "error": {"code": -32000, "message": secret, "data": secret}});
        assert!(response.to_string().contains(secret));
        let message = runtime_evaluation_error(&response).unwrap();
        assert!(!message.contains(secret));
        assert!(message.contains("WebInspector protocol request failed:"));
        let requested = json!({"result": {"exceptionDetails": {"text": secret}}});
        assert_eq!(
            runtime_evaluation_error(&requested).as_deref(),
            Some(secret)
        );
    }

    #[test]
    fn duration_rejects_non_finite_and_overflowing_timeouts() {
        assert_eq!(duration_from_secs(0.0).unwrap(), Duration::from_millis(100));
        assert!(duration_from_secs(-1.0).is_err());
        assert!(duration_from_secs(f64::NAN).is_err());
        assert!(duration_from_secs(f64::INFINITY).is_err());
        assert!(duration_from_secs(f64::MAX).is_err());
    }

    #[test]
    fn deadline_rejects_system_clock_overflow() {
        assert!(webinspector_deadline(Duration::from_secs(u64::MAX)).is_err());
    }

    #[test]
    fn runtime_evaluation_errors_cover_subtype_and_exception_details() {
        let subtype = json!({
            "result": {"type": "object", "subtype": "error", "description": "boom"}
        });
        assert_eq!(runtime_evaluation_error(&subtype).as_deref(), Some("boom"));

        let exception = json!({
            "result": {"type": "undefined"},
            "exceptionDetails": {"text": "syntax error"}
        });
        assert_eq!(
            runtime_evaluation_error(&exception).as_deref(),
            Some("syntax error")
        );

        let nested = json!({"result": {"result": {"value": "legacy"}}});
        assert_eq!(
            runtime_remote_object(&nested)
                .and_then(|value| value.get("value"))
                .and_then(JsonValue::as_str),
            Some("legacy")
        );
    }
}
