//! One setup total, cancellable idle input, per-expression totals and owned cleanup.
use std::{
    cell::Cell,
    future::Future,
    io::{IsTerminal, Write},
    pin::Pin,
    time::Duration,
};

use anyhow::{anyhow, Context, Result};
use ios_core::{
    device::ServiceStream,
    webinspector::{
        InspectorSession, WebInspectorClient, RSD_SERVICE_NAME, SAFARI_BUNDLE_ID, SERVICE_NAME,
    },
};
use serde_json::json;
use tokio::io::{AsyncRead, BufReader};

use super::super::{
    shell::{read_bounded_shell_line, runtime_evaluation_human},
    workflows::{
        page_type_name, runtime_evaluate_params, runtime_evaluation_error,
        wait_for_inspectable_page,
    },
};
use super::{
    control::{self, Budget},
    input::Input,
    session_owner::{cap, cleanup, Session, CLEANUP_BUDGET},
    source::{Connection, DeviceSource, Source},
};

pub(super) struct Options<'a> {
    pub(super) url: Option<&'a str>,
    pub(super) bundle_id: Option<&'a str>,
    pub(super) page_id: Option<&'a str>,
    pub(super) open_safari: bool,
    pub(super) continue_on_error: bool,
    pub(super) timeout: Duration,
    pub(super) json: bool,
}

enum Emission<'a> {
    OutLine(&'a str),
    ErrorLine(&'a str),
    Prompt,
}

pub(super) async fn run_command(udid: Option<String>, options: Options<'_>) -> Result<()> {
    run(
        &DeviceSource,
        udid,
        &options,
        std::io::stdin().is_terminal(),
        |item| {
            match item {
                Emission::OutLine(value) => crate::output::write_line(format_args!("{value}"))?,
                Emission::ErrorLine(value) => {
                    crate::output::write_error_line(format_args!("{value}"))?
                }
                Emission::Prompt => {
                    crate::output::write_formatted(format_args!("> "))?;
                    std::io::stdout().flush()?;
                }
            }
            Ok(())
        },
        tokio::signal::ctrl_c(),
        Input::open,
        Input::finish,
        CLEANUP_BUDGET,
    )
    .await
}

#[allow(clippy::too_many_arguments)]
async fn run<R: AsyncRead + Unpin>(
    source: &impl Source,
    udid: Option<String>,
    options: &Options<'_>,
    interactive: bool,
    mut emit: impl FnMut(Emission<'_>) -> Result<()>,
    signal: impl Future<Output = std::io::Result<()>>,
    open: impl FnOnce() -> std::io::Result<R>,
    finish: impl FnOnce(R) -> std::io::Result<()>,
    cleanup_budget: Duration,
) -> Result<()> {
    // Admission happens before source polling or changing stdin status flags.
    let setup = Budget::new(options.timeout, "js-shell setup")?;
    tokio::pin!(signal);
    let fired = Cell::new(false);
    let signal = std::future::poll_fn(|cx| {
        let result = signal.as_mut().poll(cx);
        if matches!(&result, std::task::Poll::Ready(Ok(()))) {
            fired.set(true);
        }
        result
    });
    tokio::pin!(signal);
    let input = setup.drive(async { Ok(open()?) }, signal.as_mut()).await?;
    let mut reader = BufReader::new(input);
    let primary = run_owned(
        source,
        udid,
        options,
        setup,
        interactive,
        &mut emit,
        signal.as_mut(),
        &fired,
        &mut reader,
        cleanup_budget,
    )
    .await;
    let restored = finish(reader.into_inner()).context("webinspector stdin cleanup failed");
    control::finish(primary, restored)
}

#[allow(clippy::too_many_arguments)]
async fn run_owned<R: AsyncRead + Unpin, F: Future<Output = std::io::Result<()>>>(
    source: &impl Source,
    udid: Option<String>,
    options: &Options<'_>,
    setup: Budget,
    interactive: bool,
    emit: &mut impl FnMut(Emission<'_>) -> Result<()>,
    mut signal: Pin<&mut F>,
    fired: &Cell<bool>,
    reader: &mut BufReader<R>,
    cleanup_budget: Duration,
) -> Result<()> {
    let udid = setup
        .drive(
            async {
                setup.remaining()?;
                match udid {
                    Some(udid) => Ok(udid),
                    None => {
                        let devices = source
                            .list_devices()
                            .await
                            .context("failed to list connected iOS devices for default --udid")?;
                        setup.remaining()?;
                        crate::default_udid_from_devices(&devices).ok_or_else(|| {
                            anyhow!("--udid required: no connected iOS devices found")
                        })
                    }
                }
            },
            signal.as_mut(),
        )
        .await?;
    let Connection {
        device,
        stream,
        uses_rsd,
    } = setup
        .drive(source.connect(&udid, setup), signal.as_mut())
        .await?;
    let mut client = WebInspectorClient::new(stream);
    let mut owner = Session::None;
    let initialized = setup
        .drive(
            Box::pin(initialize(
                &mut client,
                &mut owner,
                options,
                setup,
                uses_rsd,
                emit,
            )),
            signal.as_mut(),
        )
        .await;
    let primary = match initialized {
        Err(error) => Err(error),
        Ok(()) => {
            let active = active(
                &mut client,
                &mut owner,
                options,
                interactive,
                reader,
                emit,
                signal.as_mut(),
            )
            .await;
            // Only this directly owned successful signal is a normal active-shell
            // stop. Registration failures and native operation errors stay errors.
            if fired.get() {
                Ok(())
            } else {
                active
            }
        }
    };
    let cleanup = cleanup(&mut client, &mut owner, cleanup_budget).await;
    drop(client);
    drop(device);
    control::finish(primary, cleanup)
}

async fn emit_budgeted(
    budget: Budget,
    emit: &mut impl FnMut(Emission<'_>) -> Result<()>,
    item: Emission<'_>,
) -> Result<()> {
    tokio::task::yield_now().await;
    budget.remaining()?;
    emit(item)?;
    tokio::task::yield_now().await;
    Ok(())
}

async fn initialize(
    client: &mut WebInspectorClient<ServiceStream>,
    owner: &mut Session,
    options: &Options<'_>,
    budget: Budget,
    uses_rsd: bool,
    emit: &mut impl FnMut(Emission<'_>) -> Result<()>,
) -> Result<()> {
    let remaining = cap(client, budget)?;
    client.start_preserving_errors(remaining).await?;
    let bundle = if options.open_safari {
        options.bundle_id.or(Some(SAFARI_BUNDLE_ID))
    } else {
        options.bundle_id
    };
    if options.open_safari {
        cap(client, budget)?;
        client.request_application_launch(SAFARI_BUNDLE_ID).await?;
    }
    cap(client, budget)?;
    let (application_id, page) = wait_for_inspectable_page(
        client,
        options.page_id,
        bundle,
        budget.deadline,
        budget.duration,
    )
    .await?;
    let application = client
        .applications()
        .get(&application_id)
        .cloned()
        .ok_or_else(|| anyhow!("application '{application_id}' disappeared"))?;
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
    if let Some(url) = options.url.filter(|url| !url.is_empty()) {
        let expression = format!(
            "window.location = {}",
            serde_json::to_string(url).context("failed encoding shell URL")?
        );
        let remaining = cap(client, budget)?;
        let response = session
            .send_command_and_wait(
                client,
                "Runtime.evaluate",
                runtime_evaluate_params(&expression),
                remaining,
            )
            .await?;
        if let Some(error) = runtime_evaluation_error(&response) {
            return Err(anyhow!("webinspector navigation error: {error}"));
        }
    }
    let metadata = json!({
        "kind": "session", "bundle_id": application.bundle_identifier,
        "application_id": application.id, "page_id": page.id, "page_key": page.listing_key,
        "page_type": page_type_name(&page.page_type), "url": page.url, "title": page.title,
        "session_id": session.session_id(), "inspector_connection_id": client.connection_id(),
        "automation_connection_id": page.automation_connection_id,
        "service_name": if uses_rsd { RSD_SERVICE_NAME } else { SERVICE_NAME },
    });
    let value = if options.json {
        serde_json::to_string(&metadata)?
    } else {
        format!(
            "Connected to {} page={} {}",
            application.bundle_identifier,
            page.id,
            page.url.as_deref().unwrap_or("<no url>")
        )
    };
    emit_budgeted(budget, emit, Emission::OutLine(&value)).await
}

#[allow(clippy::too_many_arguments)]
async fn active<R: AsyncRead + Unpin, F: Future<Output = std::io::Result<()>>>(
    client: &mut WebInspectorClient<ServiceStream>,
    owner: &mut Session,
    options: &Options<'_>,
    interactive: bool,
    reader: &mut BufReader<R>,
    emit: &mut impl FnMut(Emission<'_>) -> Result<()>,
    mut signal: Pin<&mut F>,
) -> Result<()> {
    loop {
        if interactive && !options.json {
            let budget = Budget::new(options.timeout, "js-shell prompt")?;
            budget
                .drive(
                    emit_budgeted(budget, emit, Emission::Prompt),
                    signal.as_mut(),
                )
                .await?;
        }
        let mut line = String::new();
        let count = tokio::select! {
            biased;
            signal = signal.as_mut() => {
                signal.context("failed waiting for Ctrl+C")?;
                return Ok(());
            }
            result = read_bounded_shell_line(reader, &mut line) => result?,
        };
        // A pipe can continuously supply ready blank lines. Give the directly
        // owned signal a turn even when no expression or output budget starts.
        tokio::task::yield_now().await;
        if count == 0 {
            if interactive && !options.json {
                let budget = Budget::new(options.timeout, "js-shell output")?;
                budget
                    .drive(
                        emit_budgeted(budget, emit, Emission::OutLine("")),
                        signal.as_mut(),
                    )
                    .await?;
            }
            return Ok(());
        }
        let expression = line.trim();
        if expression.is_empty() {
            continue;
        }
        if matches!(expression, ".exit" | "exit" | "quit") {
            return Ok(());
        }
        let budget = Budget::new(options.timeout, "js-shell evaluate")?;
        budget
            .drive(
                Box::pin(evaluate(client, owner, options, expression, budget, emit)),
                signal.as_mut(),
            )
            .await?;
    }
}

async fn evaluate(
    client: &mut WebInspectorClient<ServiceStream>,
    owner: &mut Session,
    options: &Options<'_>,
    expression: &str,
    budget: Budget,
    emit: &mut impl FnMut(Emission<'_>) -> Result<()>,
) -> Result<()> {
    let Session::Inspector {
        session,
        attached: true,
    } = owner
    else {
        return Err(anyhow!("webinspector shell session is not attached"));
    };
    let remaining = cap(client, budget)?;
    // Only a completed response may continue. Wire/codec/deadline/I/O failures
    // propagate even when --continue-on-error is true, preserving their source.
    let response = session
        .send_command_and_wait(
            client,
            "Runtime.evaluate",
            runtime_evaluate_params(expression),
            remaining,
        )
        .await?;
    if let Some(error) = runtime_evaluation_error(&response) {
        if !options.continue_on_error {
            return Err(anyhow!("webinspector evaluate error: {error}"));
        }
        if options.json {
            let value = serde_json::to_string(
                &json!({ "kind": "error", "expression": expression, "error": error }),
            )?;
            emit_budgeted(budget, emit, Emission::OutLine(&value)).await
        } else {
            emit_budgeted(budget, emit, Emission::ErrorLine(&error)).await
        }
    } else {
        let value = if options.json {
            serde_json::to_string(&json!({
            "kind": "result", "expression": expression, "result": response }))?
        } else {
            runtime_evaluation_human(&response)?
        };
        emit_budgeted(budget, emit, Emission::OutLine(&value)).await
    }
}

#[cfg(test)]
mod tests;
