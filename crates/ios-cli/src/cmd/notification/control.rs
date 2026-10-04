use std::{future::Future, time::Duration};

use anyhow::{Context, Result};
use ios_core::notificationproxy::{
    NotificationProxyClient, NotificationProxyError, NotificationProxyEvent,
};
use tokio::time::{error::Elapsed, Instant};

use super::{
    finish_notification_session, next_stream_notification, render_notification_output,
    NotificationSub,
};

pub(super) const SETUP_BUDGET: Duration = Duration::from_secs(30);
const SEND_BUDGET: Duration = Duration::from_secs(30);

fn checked_deadline(now: Instant, budget: Duration) -> Result<Instant> {
    if budget.is_zero() {
        return Err(crate::output::input_decision(
            "notification timeout must be greater than zero",
        ));
    }
    now.checked_add(budget)
        .ok_or_else(|| crate::output::input_decision("notification deadline is too large"))
}

pub(super) fn preflight(sub: &NotificationSub) -> Result<()> {
    let timeout = match sub {
        NotificationSub::Post { .. } => None,
        NotificationSub::Roundtrip { timeout, .. }
        | NotificationSub::Observe { timeout, .. }
        | NotificationSub::Wait { timeout, .. }
        | NotificationSub::Springboard { timeout, .. } => Some(*timeout),
        NotificationSub::Stream { timeout, .. } => *timeout,
    };
    if let Some(timeout) = timeout {
        checked_deadline(Instant::now(), Duration::from_secs(timeout))?;
    }
    Ok(())
}

fn total_budget(sub: &NotificationSub) -> Option<Duration> {
    match sub {
        NotificationSub::Roundtrip { timeout, .. }
        | NotificationSub::Wait { timeout, .. }
        | NotificationSub::Springboard { timeout, .. } => Some(Duration::from_secs(*timeout)),
        NotificationSub::Stream { timeout, .. } => timeout.map(Duration::from_secs),
        NotificationSub::Post { .. } | NotificationSub::Observe { .. } => None,
    }
}

async fn deadline_elapsed(deadline: Option<Instant>) -> Elapsed {
    match deadline {
        Some(deadline) => tokio::time::timeout_at(deadline, std::future::pending::<()>())
            .await
            .expect_err("pending deadline future cannot complete"),
        None => std::future::pending().await,
    }
}

fn cancelled() -> anyhow::Error {
    anyhow::Error::new(ios_core::OperationCancelled::new())
        .context("notification operation cancelled by Ctrl+C")
}

pub(super) async fn setup<T>(
    budget: Duration,
    operation: impl Future<Output = Result<T>>,
    signal: impl Future<Output = std::io::Result<()>>,
) -> Result<T> {
    let deadline = checked_deadline(Instant::now(), budget)?;
    let operation = Box::pin(operation);
    tokio::pin!(signal);
    let expiry = Box::pin(deadline_elapsed(Some(deadline)));
    tokio::select! {
        biased;
        signal = &mut signal => {
            signal.context("failed to wait for notification Ctrl+C")?;
            Err(cancelled())
        }
        elapsed = expiry => Err(anyhow::Error::new(elapsed).context("notification setup timed out")),
        result = operation => result,
    }
}

async fn send_stage<T>(budget: Duration, operation: impl Future<Output = Result<T>>) -> Result<T> {
    let deadline = checked_deadline(Instant::now(), budget)?;
    let operation = Box::pin(operation);
    tokio::time::timeout_at(deadline, operation)
        .await
        .context("notification send timed out")?
}

pub(super) async fn capture<S>(
    client: &mut NotificationProxyClient<S>,
    sub: NotificationSub,
    emit: impl FnMut(&str) -> Result<()>,
    signal: impl Future<Output = std::io::Result<()>>,
) -> Result<()>
where
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin,
{
    preflight(&sub)?;
    let deadline = total_budget(&sub)
        .map(|budget| checked_deadline(Instant::now(), budget))
        .transpose()?;
    let operation = controlled_at(client, sub, emit, signal, deadline).await;
    finish_notification_session(client, operation).await
}

async fn controlled_at<S>(
    client: &mut NotificationProxyClient<S>,
    sub: NotificationSub,
    emit: impl FnMut(&str) -> Result<()>,
    signal: impl Future<Output = std::io::Result<()>>,
    deadline: Option<Instant>,
) -> Result<()>
where
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin,
{
    let graceful_signal = matches!(
        sub,
        NotificationSub::Stream { .. } | NotificationSub::Observe { .. }
    );
    let graceful_deadline = matches!(sub, NotificationSub::Stream { .. });
    let operation = Box::pin(consume(client, sub, emit));
    tokio::pin!(signal);
    let expiry = Box::pin(deadline_elapsed(deadline));
    tokio::select! {
        biased;
        signal = &mut signal => {
            signal.context("failed to wait for notification Ctrl+C")?;
            if graceful_signal { Ok(()) } else { Err(cancelled()) }
        }
        elapsed = expiry => {
            if graceful_deadline { Ok(()) }
            else { Err(anyhow::Error::new(elapsed).context("notification operation timed out")) }
        }
        result = operation => result,
    }
}

async fn consume<S>(
    client: &mut NotificationProxyClient<S>,
    sub: NotificationSub,
    mut emit: impl FnMut(&str) -> Result<()>,
) -> Result<()>
where
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin,
{
    match sub {
        NotificationSub::Post { notification } => {
            send_stage(SEND_BUDGET, async { Ok(client.post(&notification).await?) }).await?;
            emit(&format!("Posted notification: {notification}"))?;
        }
        NotificationSub::Roundtrip {
            notification,
            timeout,
            json,
        } => {
            send_stage(SEND_BUDGET, async {
                client.observe(&notification).await?;
                client.post(&notification).await?;
                Ok(())
            })
            .await?;
            client
                .wait_for(&notification, Duration::from_secs(timeout))
                .await?;
            emit(&render_notification_output(&notification, json, None)?)?;
        }
        NotificationSub::Observe {
            notification,
            count,
            timeout,
            json,
        } => {
            send_stage(SEND_BUDGET, async {
                Ok(client.observe(&notification).await?)
            })
            .await?;
            let limit = count.unwrap_or(u64::MAX);
            let timeout = Duration::from_secs(timeout);
            let mut seen = 0u64;
            while seen < limit {
                let event = client.next_event(timeout).await?;
                // A completed frame can yield without abandoning framing. The
                // outer signal/deadline owner gets a turn before output and the
                // next read, even when this transport is continuously ready.
                tokio::task::yield_now().await;
                match event {
                    NotificationProxyEvent::Notification(name) => {
                        if name == notification {
                            seen += 1;
                            emit(&render_notification_output(&name, json, Some(seen))?)?;
                        }
                    }
                    NotificationProxyEvent::ProxyDeath => {
                        return Err(anyhow::Error::new(NotificationProxyError::ProxyDeath)
                            .context("notification proxy closed before notification arrived"));
                    }
                }
                tokio::task::yield_now().await;
            }
        }
        NotificationSub::Wait {
            notification,
            timeout,
            json,
        } => {
            send_stage(SEND_BUDGET, async {
                Ok(client.observe(&notification).await?)
            })
            .await?;
            client
                .wait_for(&notification, Duration::from_secs(timeout))
                .await?;
            emit(&render_notification_output(&notification, json, None)?)?;
        }
        NotificationSub::Stream {
            notifications,
            count,
            json,
            ..
        } => {
            send_stage(SEND_BUDGET, async {
                for notification in &notifications {
                    client.observe(notification).await?;
                }
                Ok(())
            })
            .await?;
            let mut received = 0u64;
            loop {
                let name = next_stream_notification(client, None).await?;
                tokio::task::yield_now().await;
                received += 1;
                emit(&render_notification_output(&name, json, Some(received))?)?;
                if count.is_some_and(|limit| received >= limit) {
                    break;
                }
                tokio::task::yield_now().await;
            }
        }
        NotificationSub::Springboard { timeout, json } => {
            send_stage(SEND_BUDGET, async {
                Ok(client
                    .observe(ios_core::notificationproxy::SPRINGBOARD_FINISHED_STARTUP)
                    .await?)
            })
            .await?;
            client
                .wait_for_springboard(Duration::from_secs(timeout))
                .await?;
            emit(&render_notification_output(
                ios_core::notificationproxy::SPRINGBOARD_FINISHED_STARTUP,
                json,
                None,
            )?)?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests;
