//! Bounded original client owners; stop joins every task rather than detaching.
use super::*;
use std::future::Future;
use tokio::{
    io::{AsyncRead, AsyncReadExt, AsyncWrite},
    sync::watch,
    task::JoinSet,
    time::Instant,
};

const MAX_CLIENTS: usize = 16;
const CONNECT_BUDGET: Duration = Duration::from_secs(30);
const CLIENT_CLEANUP_BUDGET: Duration = Duration::from_secs(5);
const JOIN_BUDGET: Duration = Duration::from_secs(10);
const MAX_DRAIN_BYTES: u64 = 1024 * 1024;

async fn stop_requested(stop: &mut watch::Receiver<bool>) {
    loop {
        if *stop.borrow_and_update() {
            return;
        }
        if stop.changed().await.is_err() {
            return;
        }
    }
}

async fn retire_socket<S: AsyncRead + AsyncWrite + Unpin + ?Sized>(
    stream: &mut S,
    deadline: Instant,
) -> Result<()> {
    let shutdown = tokio::time::timeout_at(deadline, stream.shutdown())
        .await
        .context("forward socket write-shutdown deadline expired")
        .and_then(|result| result.map_err(anyhow::Error::new));
    let drain = tokio::time::timeout_at(deadline, async {
        let bytes = stream
            .take(MAX_DRAIN_BYTES + 1)
            .read_to_end(&mut Vec::new())
            .await?;
        anyhow::ensure!(
            bytes as u64 <= MAX_DRAIN_BYTES,
            crate::output::resource_decision("forward cleanup raw-drain limit exceeded")
        );
        Ok::<(), anyhow::Error>(())
    })
    .await
    .context("forward original peer EOF deadline expired")
    .and_then(|result| result);
    crate::cmd::test_workflow::combine_results(
        shutdown,
        drain,
        "forward shutdown and strict peer EOF both failed",
    )
}

async fn client(
    mut local: TcpStream,
    connector: ForwardConnector,
    device_port: u16,
    mut stop: watch::Receiver<bool>,
) -> Result<()> {
    let mut remote = None;
    let operation = Box::pin(async {
        if *stop.borrow() {
            return Err(anyhow::Error::new(ios_core::OperationCancelled::new()));
        }
        let opened = tokio::select! {
            biased;
            result = tokio::time::timeout(CONNECT_BUDGET, connector.connect(device_port)) => result
                .context("forward client connect total deadline expired").and_then(|result| result),
            _ = stop_requested(&mut stop) => Err(anyhow::Error::new(ios_core::OperationCancelled::new())),
        }?;
        remote = Some(opened);
        tokio::select! {
            biased;
            result = tokio::io::copy_bidirectional(&mut local, remote.as_mut().expect("stored original remote")) => {
                result.map(|_| ()).map_err(anyhow::Error::new)
            }
            _ = stop_requested(&mut stop) => Err(anyhow::Error::new(ios_core::OperationCancelled::new())),
        }
    });
    let primary = operation.await;
    let deadline = Instant::now() + CLIENT_CLEANUP_BUDGET;
    let (local_cleanup, remote_cleanup) =
        tokio::join!(retire_socket(&mut local, deadline), async {
            if let Some(remote) = &mut remote {
                retire_socket(remote.as_mut(), deadline).await
            } else {
                Ok(())
            }
        });
    let cleanup = crate::cmd::test_workflow::combine_results(
        local_cleanup,
        remote_cleanup,
        "forward local and remote cleanup failed",
    );
    crate::cmd::test_workflow::combine_results(
        primary,
        cleanup,
        "forward transfer and original socket cleanup failed",
    )
}

#[derive(Default)]
struct FailureHistory {
    first: Option<anyhow::Error>,
    latest: Option<anyhow::Error>,
    count: u64,
}
impl FailureHistory {
    fn record(&mut self, error: anyhow::Error) {
        self.count = self.count.saturating_add(1);
        if self.first.is_none() {
            self.first = Some(error);
        } else {
            self.latest = Some(error);
        }
    }
    fn result(self) -> Result<()> {
        if self.first.is_none() {
            Ok(())
        } else {
            Err(anyhow::Error::new(self))
        }
    }
}
impl std::fmt::Display for FailureHistory {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{} original forward client outcomes failed; first and latest causes retained",
            self.count
        )
    }
}
impl std::fmt::Debug for FailureHistory {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ForwardFailureHistory")
            .field("count", &self.count)
            .field(
                "first_class",
                &self
                    .first
                    .as_ref()
                    .map(|error| ios_core::classify_error(error.as_ref())),
            )
            .field(
                "latest_class",
                &self
                    .latest
                    .as_ref()
                    .map(|error| ios_core::classify_error(error.as_ref())),
            )
            .finish_non_exhaustive()
    }
}
impl std::error::Error for FailureHistory {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        self.first.as_ref().map(|error| error.as_ref())
    }
}

struct ServeFailure {
    primary: anyhow::Error,
    clients: FailureHistory,
}
impl std::fmt::Display for ServeFailure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("forward stopped with retained original client failures")
    }
}
impl std::fmt::Debug for ServeFailure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ForwardServeFailure")
            .field(
                "primary_class",
                &ios_core::classify_error(self.primary.as_ref()),
            )
            .field("clients", &self.clients)
            .finish_non_exhaustive()
    }
}
impl std::error::Error for ServeFailure {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        Some(self.primary.as_ref())
    }
}

pub(super) async fn serve<S: Future<Output = std::io::Result<()>>>(
    listener: TcpListener,
    connector: ForwardConnector,
    device_port: u16,
    once: bool,
    cancellation: S,
) -> Result<()> {
    let mut cancellation = Box::pin(cancellation);
    let (stop, stopped) = watch::channel(false);
    let mut tasks = JoinSet::new();
    let mut accepted_once = false;
    let mut outcome = Ok(());
    let mut failures = FailureHistory::default();
    loop {
        if once && accepted_once && tasks.is_empty() {
            break;
        }
        tokio::select! {
            biased;
            signal = &mut cancellation => {
                outcome = match signal {
                    Ok(()) => Err(anyhow::Error::new(ios_core::OperationCancelled::new())),
                    Err(error) => Err(anyhow::Error::new(error).context("waiting for forward cancellation")),
                };
                break;
            }
            completed = tasks.join_next(), if !tasks.is_empty() => {
                let result = completed.expect("owned forward task").context("original forward task failed").and_then(|result| result);
                if result.is_err() {
                    if once {
                        outcome = result;
                        break;
                    }
                    let error = result.unwrap_err();
                    tracing::warn!(class = %ios_core::classify_error(error.as_ref()), "forward original client failed; other clients remain served");
                    failures.record(error);
                }
            }
            accepted = listener.accept(), if tasks.len() < MAX_CLIENTS && !(once && accepted_once) => {
                match accepted {
                    Ok((stream, _)) => {
                        accepted_once = true;
                        tasks.spawn(client(stream, connector.clone(), device_port, stopped.clone()));
                    }
                    Err(error) => { outcome = Err(anyhow::Error::new(error)); break; }
                }
            }
        }
    }
    drop(listener);
    let _ = stop.send(true);
    let deadline = Instant::now() + JOIN_BUDGET;
    while !tasks.is_empty() {
        match tokio::time::timeout_at(deadline, tasks.join_next()).await {
            Ok(Some(result)) => {
                let result = result
                    .context("original forward task failed")
                    .and_then(|result| result);
                if let Err(error) = result {
                    failures.record(error);
                }
            }
            Ok(None) => break,
            Err(elapsed) => {
                outcome = crate::cmd::test_workflow::combine_results(
                    outcome,
                    Err(anyhow::Error::new(elapsed)
                        .context("forward original task join deadline expired")),
                    "forward joins incomplete",
                );
                tasks.abort_all();
                // Keep the original JoinSet until every abort has actually
                // completed. No replacement task or dropped JoinHandle receipt.
                while let Some(result) = tasks.join_next().await {
                    if let Err(error) = result
                        .context("original forward task aborted after cleanup expiry")
                        .and_then(|result| result)
                    {
                        failures.record(error);
                    }
                }
                break;
            }
        }
    }
    match outcome {
        Ok(()) => failures.result(),
        Err(primary) if failures.first.is_some() => Err(anyhow::Error::new(ServeFailure {
            primary,
            clients: failures,
        })),
        Err(primary) => Err(primary),
    }
}

#[cfg(test)]
mod tests;
