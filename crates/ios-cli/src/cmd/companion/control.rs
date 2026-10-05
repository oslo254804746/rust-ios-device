use std::{future::Future, time::Duration};

use anyhow::{Context, Result};
use ios_core::companion::{CompanionDeviceStream, CompanionError, CompanionProxyClient};
use tokio::time::{error::Elapsed, Instant};

use super::{operation_deadline, CompanionSub};

pub(super) const SETUP_BUDGET: Duration = Duration::from_secs(30);
const CLEANUP_BUDGET: Duration = Duration::from_secs(5);

pub(super) fn timeout(sub: &CompanionSub) -> Option<u64> {
    match sub {
        CompanionSub::Listen { timeout }
        | CompanionSub::Forward { timeout, .. }
        | CompanionSub::Stop { timeout, .. } => *timeout,
        CompanionSub::List | CompanionSub::Get { .. } => None,
    }
}

pub(super) fn preflight(sub: &CompanionSub) -> Result<()> {
    operation_deadline(timeout(sub)).map(|_| ())
}

fn cancelled() -> anyhow::Error {
    anyhow::Error::new(ios_core::OperationCancelled::new())
        .context("companion operation cancelled by Ctrl+C")
}

async fn elapsed(deadline: Option<Instant>) -> Elapsed {
    match deadline {
        Some(deadline) => tokio::time::timeout_at(deadline, std::future::pending::<()>())
            .await
            .expect_err("pending deadline cannot complete"),
        None => std::future::pending().await,
    }
}

pub(super) async fn drive<T>(
    operation: impl Future<Output = Result<T>>,
    signal: impl Future<Output = std::io::Result<()>>,
    deadline: Option<Instant>,
    on_stop: impl FnOnce() -> Result<T>,
) -> Result<T> {
    let mut operation = Box::pin(operation);
    let expiry = Box::pin(elapsed(deadline));
    tokio::pin!(signal);
    // Tokio's timer may need another poll after its nominal deadline. Do not
    // admit operation work in that gap; the native timer still supplies Elapsed.
    let guarded_operation = std::future::poll_fn(|cx| {
        if deadline.is_some_and(|deadline| Instant::now() >= deadline) {
            std::task::Poll::Pending
        } else {
            operation.as_mut().poll(cx)
        }
    });
    tokio::select! {
        biased;
        signal = &mut signal => {
            signal.context("failed to wait for companion Ctrl+C")?;
            on_stop()
        }
        error = expiry => Err(anyhow::Error::new(error).context("companion operation timed out")),
        result = guarded_operation => result,
    }
}

pub(super) async fn setup<T>(
    budget: Duration,
    total: Option<Instant>,
    operation: impl Future<Output = Result<T>>,
    signal: impl Future<Output = std::io::Result<()>>,
) -> Result<T> {
    if budget.is_zero() {
        return Err(crate::output::input_decision(
            "companion setup budget must be positive",
        ));
    }
    let deadline = Instant::now()
        .checked_add(budget)
        .ok_or_else(|| crate::output::input_decision("companion setup deadline is too large"))?;
    let deadline = total.map_or(deadline, |total| total.min(deadline));
    drive(operation, signal, Some(deadline), || Err(cancelled())).await
}

pub(super) async fn run<S>(
    mut client: CompanionProxyClient<S>,
    sub: CompanionSub,
    deadline: Option<Instant>,
    json: bool,
    mut emit: impl FnMut(&str) -> Result<()>,
    signal: impl Future<Output = std::io::Result<()>>,
) -> Result<()>
where
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin + Send + 'static,
{
    preflight(&sub)?;
    tokio::pin!(signal);
    match sub {
        CompanionSub::List => {
            drive(
                async {
                    let devices = client.list().await?;
                    if json {
                        emit(&serde_json::to_string_pretty(&devices)?)?;
                    } else {
                        for device in devices {
                            emit(&serde_json::to_string_pretty(&device)?)?;
                            tokio::task::yield_now().await;
                        }
                    }
                    Ok(())
                },
                signal.as_mut(),
                deadline,
                || Err(cancelled()),
            )
            .await
        }
        CompanionSub::Get { udid, key } => {
            drive(
                async {
                    let value = client.get_value(&udid, &key).await?;
                    if json {
                        emit(&serde_json::to_string_pretty(&value)?)?;
                    } else if let Some(string) = value.as_string() {
                        emit(string)?;
                    } else {
                        emit(&serde_json::to_string_pretty(&value)?)?;
                    }
                    Ok(())
                },
                signal.as_mut(),
                deadline,
                || Err(cancelled()),
            )
            .await
        }
        CompanionSub::Stop { remote_port, .. } => {
            drive(
                async {
                    let response = client.stop_forwarding_service_port(remote_port).await?;
                    if json {
                        emit(&serde_json::to_string(&plist::Value::Dictionary(response))?)?;
                    } else if let Some(command) =
                        response.get("Command").and_then(plist::Value::as_string)
                    {
                        emit(command)?;
                    } else {
                        emit(&serde_json::to_string_pretty(&plist::Value::Dictionary(
                            response,
                        ))?)?;
                    }
                    Ok(())
                },
                signal.as_mut(),
                deadline,
                || Err(cancelled()),
            )
            .await
        }
        CompanionSub::Listen { .. } => {
            let mut events = drive(
                async { Ok(client.listen_for_devices().await?) },
                signal.as_mut(),
                deadline,
                || Err(cancelled()),
            )
            .await?;
            drive(
                consume_events(&mut events, json, &mut emit),
                signal.as_mut(),
                deadline,
                || Ok(()),
            )
            .await
        }
        CompanionSub::Forward {
            remote_port,
            service_name,
            ..
        } => {
            let forwarding = drive(
                async {
                    Ok(client
                        .start_forwarding_service_port_handle(
                            remote_port,
                            service_name.as_deref(),
                            None,
                        )
                        .await?)
                },
                signal.as_mut(),
                deadline,
                || Err(cancelled()),
            )
            .await?;
            let remote_port = forwarding.remote_port();
            let service_port = forwarding.service_port();
            // Disarm at future creation, before any output or wait can fail or
            // the caller can drop this command. Cleanup starts only on await.
            let closing = Box::pin(forwarding.close_with_timeout(CLEANUP_BUDGET));
            let primary = drive(async {
                let line = if json {
                    serde_json::json!({
                        "GizmoRemotePortNumber": remote_port,
                        "CompanionProxyServicePort": service_port,
                    }).to_string()
                } else {
                    format!("Forwarded companion port {} through service port {}. Press Ctrl+C to stop.",
                        remote_port, service_port)
                };
                emit(&line)?;
                std::future::pending().await
            }, signal.as_mut(), deadline, || Ok(())).await;
            finish_forwarding(closing, client.is_usable(), primary).await
        }
    }
}

async fn consume_events<S>(
    events: &mut CompanionDeviceStream<S>,
    json: bool,
    emit: &mut impl FnMut(&str) -> Result<()>,
) -> Result<()>
where
    S: tokio::io::AsyncRead + Unpin + Send + 'static,
{
    loop {
        let event = events.next_event().await;
        tokio::task::yield_now().await;
        match event {
            Some(Ok(event)) => {
                let value = plist::Value::Dictionary(event);
                let line = if json {
                    serde_json::to_string(&value)?
                } else {
                    serde_json::to_string_pretty(&value)?
                };
                emit(&line)?;
            }
            Some(Err(error)) => return Err(error.into()),
            None => return Err(anyhow::anyhow!("companion listener closed unexpectedly")),
        }
        tokio::task::yield_now().await;
    }
}

async fn finish_forwarding(
    closing: impl Future<Output = std::result::Result<plist::Dictionary, CompanionError>>,
    usable: bool,
    primary: Result<()>,
) -> Result<()> {
    if !usable {
        drop(closing);
        return primary;
    }
    let cleanup = closing
        .await
        .context("failed to stop companion forwarding")
        .map(|_| ());
    match (primary, cleanup) {
        (Ok(()), cleanup) => cleanup,
        (Err(primary), Ok(())) => Err(primary),
        (Err(primary), Err(cleanup)) => Err(anyhow::Error::new(CompanionCleanupFailure {
            primary,
            cleanup,
        })),
    }
}

struct CompanionCleanupFailure {
    primary: anyhow::Error,
    cleanup: anyhow::Error,
}

impl std::fmt::Display for CompanionCleanupFailure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("companion operation failed; forwarding cleanup also failed")
    }
}
impl std::fmt::Debug for CompanionCleanupFailure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CompanionCleanupFailure")
            .field(
                "primary_class",
                &ios_core::classify_error(self.primary.as_ref()),
            )
            .field(
                "cleanup_class",
                &ios_core::classify_error(self.cleanup.as_ref()),
            )
            .finish()
    }
}
impl std::error::Error for CompanionCleanupFailure {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        Some(self.primary.as_ref())
    }
}

#[cfg(test)]
mod tests;
