//! One total spans default selection, connection, session work and output.
use std::{future::Future, time::Duration};

use super::super::shell::runtime_evaluation_human;
use super::{
    control::{self, Budget},
    *,
};
use ios_core::device::ServiceStream;

use super::session_owner::{cap, cleanup, Session, CLEANUP_BUDGET};
use super::source::{Connection, DeviceSource, Source};

pub(super) enum Command<'a> {
    Tabs,
    Eval {
        app_id: Option<String>,
        bundle_id: String,
        page_selector: &'a str,
        expression: &'a str,
    },
    Launch {
        bundle_id: &'a str,
        url: Option<&'a str>,
    },
}

impl Command<'_> {
    fn phase(&self) -> &'static str {
        match self {
            Self::Tabs => "opened-tabs",
            Self::Eval { .. } => "eval",
            Self::Launch { .. } => "launch",
        }
    }
}

pub(super) async fn run_command(
    udid: Option<String>,
    command: Command<'_>,
    timeout: Duration,
    json: bool,
) -> Result<()> {
    let budget = Budget::new(timeout, command.phase())?;
    let signal = tokio::signal::ctrl_c();
    run(
        &DeviceSource,
        udid,
        command,
        budget,
        json,
        |line| {
            crate::output::write_line(format_args!("{line}"))?;
            Ok(())
        },
        signal,
        CLEANUP_BUDGET,
    )
    .await
}

#[allow(clippy::too_many_arguments)]
async fn run(
    source: &impl Source,
    udid: Option<String>,
    command: Command<'_>,
    budget: Budget,
    json: bool,
    mut emit: impl FnMut(&str) -> Result<()>,
    signal: impl Future<Output = std::io::Result<()>>,
    cleanup_budget: Duration,
) -> Result<()> {
    tokio::pin!(signal);
    let udid = budget
        .drive(
            Box::pin(async {
                budget.remaining()?;
                match udid {
                    Some(udid) => Ok(udid),
                    None => {
                        let devices = source
                            .list_devices()
                            .await
                            .context("failed to list connected iOS devices for default --udid")?;
                        budget.remaining()?;
                        crate::default_udid_from_devices(&devices).ok_or_else(|| {
                            anyhow!("--udid required: no connected iOS devices found")
                        })
                    }
                }
            }),
            signal.as_mut(),
        )
        .await?;
    let Connection {
        device,
        stream,
        uses_rsd,
    } = budget
        .drive(
            Box::pin(async { source.connect(&udid, budget).await }),
            signal.as_mut(),
        )
        .await?;
    let mut client = WebInspectorClient::new(stream);
    let mut session = Session::None;
    let primary = budget
        .drive(
            Box::pin(execute(
                &mut client,
                &mut session,
                &command,
                budget,
                json,
                uses_rsd,
                &mut emit,
            )),
            signal.as_mut(),
        )
        .await;
    // The supplied stream and real device guard remain owned through cleanup.
    let cleanup = cleanup(&mut client, &mut session, cleanup_budget).await;
    drop(client);
    if device.is_some() {
        tokio::time::sleep(Duration::from_millis(250)).await;
    }
    drop(device);
    control::finish(primary, cleanup)
}

async fn line(
    budget: Budget,
    emit: &mut impl FnMut(&str) -> Result<()>,
    value: &str,
) -> Result<()> {
    tokio::task::yield_now().await;
    budget.remaining()?;
    emit(value)?;
    // Give the one pinned signal and native timer a turn between ready output.
    tokio::task::yield_now().await;
    Ok(())
}

#[allow(clippy::too_many_arguments)]
async fn execute(
    client: &mut WebInspectorClient<ServiceStream>,
    owner: &mut Session,
    command: &Command<'_>,
    budget: Budget,
    json: bool,
    uses_rsd: bool,
    emit: &mut impl FnMut(&str) -> Result<()>,
) -> Result<()> {
    let remaining = cap(client, budget)?;
    client.start_preserving_errors(remaining).await?;
    match command {
        Command::Tabs => {
            cap(client, budget)?;
            // Quiescence has its own idle cap; the absolute frontend total
            // remains active, including a peer that never becomes idle.
            let pages = client
                .open_application_pages(super::super::PAGE_DISCOVERY_IDLE)
                .await?;
            budget.remaining()?;
            let rows = pages.into_iter().map(opened_tab_row).collect::<Vec<_>>();
            if json {
                line(budget, emit, &serde_json::to_string_pretty(&rows)?).await?;
            } else if rows.is_empty() {
                line(budget, emit, "No inspectable pages reported").await?;
            } else {
                for row in rows {
                    line(
                        budget,
                        emit,
                        &format!(
                            "{} [{}] page={} {} {}",
                            row.application_name,
                            row.bundle_identifier,
                            row.page_id,
                            row.title.as_deref().unwrap_or("<no title>"),
                            row.url.as_deref().unwrap_or("<no url>")
                        ),
                    )
                    .await?;
                    line(budget, emit, &format!("  app_id: {}", row.application_id)).await?;
                    line(budget, emit, &format!("  type: {}", row.page_type)).await?;
                }
            }
        }
        Command::Eval {
            app_id,
            bundle_id,
            page_selector,
            expression,
        } => {
            cap(client, budget)?;
            client
                .open_application_pages(super::super::PAGE_DISCOVERY_IDLE)
                .await?;
            budget.remaining()?;
            let application_id = match app_id {
                Some(app_id) => app_id.clone(),
                None => client
                    .application_by_bundle(bundle_id)
                    .map(|application| application.id.clone())
                    .ok_or_else(|| anyhow!("bundle '{bundle_id}' is not currently inspectable"))?,
            };
            let page = client.application_pages(&application_id).into_iter()
                .flat_map(|pages| pages.values())
                .find(|page| is_inspectable_page_type(&page.page_type)
                    && page_matches_selector(page, page_selector))
                .cloned().ok_or_else(|| anyhow!(
                    "inspectable page {page_selector} was not found under application '{application_id}'"))?;
            *owner = Session::Inspector {
                session: InspectorSession::new(application_id, page.id),
                attached: false,
            };
            let Session::Inspector { session, attached } = owner else {
                unreachable!()
            };
            let remaining = cap(client, budget)?;
            session.attach(client, true, remaining).await?;
            *attached = true;
            let remaining = cap(client, budget)?;
            let response = session
                .send_command_and_wait(
                    client,
                    "Runtime.evaluate",
                    runtime_evaluate_params(expression),
                    remaining,
                )
                .await?;
            if let Some(error) = runtime_evaluation_error(&response) {
                return Err(anyhow!("webinspector evaluate error: {error}"));
            }
            let rendered = if json {
                serde_json::to_string_pretty(&response)?
            } else {
                runtime_evaluation_human(&response)?
            };
            line(budget, emit, &rendered).await?;
        }
        Command::Launch { bundle_id, url } => {
            cap(client, budget)?;
            client.request_application_launch(bundle_id).await?;
            cap(client, budget)?;
            let application =
                wait_for_application(client, bundle_id, budget.deadline, budget.duration).await?;
            *owner = Session::Automation {
                session: AutomationSession::new(
                    application.id.clone(),
                    application.bundle_identifier.clone(),
                ),
                attached: false,
                application_id: application.id.clone(),
            };
            let Session::Automation {
                session, attached, ..
            } = owner
            else {
                unreachable!()
            };
            let remaining = cap(client, budget)?;
            session.attach(client, remaining).await?;
            *attached = true;
            cap(client, budget)?;
            session.start_session(client).await?;
            if let Some(url) = url.filter(|url| !url.is_empty()) {
                cap(client, budget)?;
                session.navigate(client, url).await?;
            }
            cap(client, budget)?;
            let current_url = session.current_url(client).await?;
            cap(client, budget)?;
            let title = session.get_title(client).await?;
            budget.remaining()?;
            let page = client
                .page(&application.id, session.page_id())
                .cloned()
                .ok_or_else(|| anyhow!("automation page {} disappeared", session.page_id()))?;
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
                session_id: session.session_id().to_string(),
                inspector_connection_id: client.connection_id().to_string(),
                automation_connection_id: page.automation_connection_id,
                service_name: if uses_rsd {
                    RSD_SERVICE_NAME
                } else {
                    SERVICE_NAME
                },
                uses_rsd,
            };
            if json {
                line(budget, emit, &serde_json::to_string_pretty(&result)?).await?;
            } else {
                line(
                    budget,
                    emit,
                    &format!("Launched {} (pid {})", result.bundle_id, result.pid),
                )
                .await?;
                line(budget, emit, &format!("  page: {}", result.page_id)).await?;
                line(
                    budget,
                    emit,
                    &format!("  url: {}", result.url.as_deref().unwrap_or("<no url>")),
                )
                .await?;
                line(budget, emit, &format!("  title: {}", result.title)).await?;
                line(budget, emit, &format!("  session: {}", result.session_id)).await?;
                if let Some(id) = result.automation_connection_id {
                    line(budget, emit, &format!("  automation connection: {id}")).await?;
                }
                line(budget, emit, &format!("  service: {}", result.service_name)).await?;
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests;
