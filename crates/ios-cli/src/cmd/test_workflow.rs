//! Composable test execution workflow: application management, test runner execution,
//! syslog streaming capture, JUnit XML reporting, cancellation recovery, and failure reruns (F02).

use std::collections::HashMap;
use std::future::Future;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::time::Duration;

use anyhow::{Context, Result};
use ios_core::device::ConnectedDevice;
use ios_core::testmanager::results::{TestCaseStatus, TestRunOutcome, TestRunSummary};
use ios_core::testmanager::workflow::TestLaunchPlan;
use tokio::io::AsyncWriteExt;
use tokio::sync::oneshot;
use tokio::time::Instant;
use tokio_stream::StreamExt;

use crate::cmd::runtest::{
    connect_testmanager_device_scoped, incomplete_summary, start_test_plan_session_owned_until,
    write_junit_diagnostic, write_junit_summary, xctest_wait_timeout, TestStartupCleanup,
};

pub(crate) mod crash;

/// CLI options for the composable test workflow.
#[derive(clap::Args, Debug, Clone, Default, PartialEq, Eq)]
pub struct TestWorkflowArgs {
    /// Optionally install application package (.ipa / zip) before starting tests
    #[arg(long = "install-app", value_name = "APP_PATH")]
    pub install_app: Option<PathBuf>,

    /// Optionally install test runner package (.ipa / zip) before starting tests
    #[arg(long = "install-runner", value_name = "RUNNER_PATH")]
    pub install_runner: Option<PathBuf>,

    /// Stream device syslog to this file during test execution
    #[arg(long = "log-output", value_name = "LOG_PATH")]
    pub log_output: Option<PathBuf>,

    /// Collect only new crash reports for the selected runner/target into an existing directory
    #[arg(long = "crash-output", value_name = "DIRECTORY")]
    pub crash_output: Option<PathBuf>,

    /// Uninstall installed application/runner packages after test completion
    #[arg(long = "uninstall-on-finish")]
    pub uninstall_on_finish: bool,

    /// Number of times to rerun failed tests (0 to disable)
    #[arg(long = "rerun-failed", default_value_t = 0, value_name = "COUNT")]
    pub rerun_failed: usize,
}

impl TestWorkflowArgs {
    /// Preflight check for workflow arguments before touching device or starting runners.
    pub fn preflight(&self) -> Result<()> {
        if let Some(path) = &self.crash_output {
            crash::validate_destination(path)?;
        }
        anyhow::ensure!(
            self.rerun_failed <= 10,
            "test workflow permits at most 10 reruns per invocation"
        );
        if let Some(path) = &self.install_app {
            anyhow::ensure!(
                path.is_file(),
                "test workflow preflight: application package '{}' does not exist",
                path.display()
            );
        }
        if let Some(path) = &self.install_runner {
            anyhow::ensure!(
                path.is_file(),
                "test workflow preflight: test runner package '{}' does not exist",
                path.display()
            );
        }
        if let Some(path) = &self.log_output {
            if let Some(parent) = path.parent() {
                if !parent.as_os_str().is_empty() {
                    anyhow::ensure!(
                        parent.is_dir(),
                        "test workflow preflight: log output directory '{}' does not exist",
                        parent.display()
                    );
                }
            }
        }
        for path in [&self.install_app, &self.install_runner]
            .into_iter()
            .flatten()
        {
            package_bundle_id(path)?;
        }
        Ok(())
    }
}

/// Every attempt has its own immutable result. The merged summary is a view of
/// current outcomes; it cannot replace the original failures in this artifact.
#[derive(Debug, Default, serde::Serialize)]
pub(crate) struct AttemptHistory {
    attempts: Vec<TestAttempt>,
    #[serde(skip)]
    result_bytes: usize,
    terminal_error_class: Option<ios_core::ErrorClass>,
}

#[derive(Debug, serde::Serialize)]
struct TestAttempt {
    number: usize,
    selectors: Vec<String>,
    state: &'static str,
    summary: Option<TestRunSummary>,
    error_class: Option<ios_core::ErrorClass>,
}

impl AttemptHistory {
    pub(crate) fn attempt_count(&self) -> usize {
        self.attempts.len()
    }

    pub(crate) fn start(&mut self, selectors: Vec<String>) {
        self.attempts.push(TestAttempt {
            number: self.attempts.len() + 1,
            selectors,
            state: "starting",
            summary: None,
            error_class: None,
        });
    }

    pub(crate) fn complete(&mut self, summary: &TestRunSummary) -> Result<()> {
        let mut counter = HistoryBudget {
            bytes: self.result_bytes,
        };
        serde_json::to_writer(&mut counter, summary)
            .context("workflow attempt history exceeds 32 MiB")?;
        self.result_bytes = counter.bytes;
        let attempt = self
            .attempts
            .last_mut()
            .context("no active workflow attempt")?;
        attempt.state = match summary.terminal_outcome() {
            Ok(TestRunOutcome::FailedTests) => "failed_tests",
            Ok(TestRunOutcome::Passed) => "completed",
            Err(error) => {
                attempt.error_class = Some(ios_core::classify_error(&error));
                "incomplete"
            }
        };
        attempt.summary = Some(summary.clone());
        Ok(())
    }

    pub(crate) fn fail(
        &mut self,
        error: &anyhow::Error,
        partial: Option<&TestRunSummary>,
    ) -> Result<()> {
        self.terminal_error_class = Some(ios_core::classify_error(error.as_ref()));
        if self
            .attempts
            .last()
            .is_some_and(|attempt| attempt.state == "starting")
        {
            if let Some(partial) = partial {
                self.complete(partial)?;
            }
            let attempt = self.attempts.last_mut().expect("active attempt");
            attempt.state = "interrupted";
            attempt.error_class = self.terminal_error_class;
        }
        Ok(())
    }

    pub(crate) async fn write(&self, junit: Option<&Path>) -> Result<()> {
        if let Some(junit) = junit {
            let mut name = junit.as_os_str().to_os_string();
            name.push(".attempts.json");
            let path = PathBuf::from(name);
            let mut output = HistoryOutput { bytes: Vec::new() };
            serde_json::to_writer(&mut output, self)
                .context("encoding workflow attempt history")?;
            crate::cmd::file::write_local_bytes_atomic(&path, &output.bytes, true).await?;
        }
        Ok(())
    }

    pub(crate) async fn write_until(&self, junit: Option<&Path>, deadline: Instant) -> Result<()> {
        if junit.is_none() {
            return Ok(());
        }
        artifact_clock(deadline).await?;
        // A started filesystem commit is not physically cancellable. Keep the
        // original writer owner until completion, then check the same clock.
        let written = self.write(junit).await;
        combine_results(
            written,
            artifact_clock(deadline).await,
            "workflow artifact deadline expired",
        )
    }
}

async fn artifact_clock(deadline: Instant) -> Result<()> {
    if Instant::now() >= deadline {
        tokio::time::timeout_at(deadline, std::future::pending::<()>())
            .await
            .context("workflow attempt artifact deadline expired")?;
    }
    Ok(())
}

/// Reclassify once under the original artifact clock when receipt output fails.
/// Never repeat the output or leave a published sidecar silently claiming success.
pub(crate) async fn finish_output(
    outcome: Result<()>,
    output: Result<()>,
    attempts: &mut AttemptHistory,
    junit: Option<&Path>,
    artifact_deadline: Instant,
) -> Result<()> {
    if output.is_ok() {
        return outcome;
    }
    let mut outcome = combine_results(outcome, output, "XCTest final receipt output failed");
    let recorded = attempts.fail(outcome.as_ref().expect_err("failed receipt output"), None);
    outcome = combine_results(
        outcome,
        recorded,
        "workflow output failure classification failed",
    );
    let rewritten = attempts.write_until(junit, artifact_deadline).await;
    combine_results(
        outcome,
        rewritten,
        "workflow output failure sidecar update failed",
    )
}

struct HistoryBudget {
    bytes: usize,
}
impl Write for HistoryBudget {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        let size = self
            .bytes
            .checked_add(bytes.len())
            .filter(|size| *size <= 32 * 1024 * 1024)
            .ok_or_else(|| std::io::Error::other("workflow attempt history byte limit"))?;
        self.bytes = size;
        Ok(bytes.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

struct HistoryOutput {
    bytes: Vec<u8>,
}
impl Write for HistoryOutput {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        let mut budget = HistoryBudget {
            bytes: self.bytes.len(),
        };
        budget.write_all(bytes)?;
        self.bytes.extend_from_slice(bytes);
        Ok(bytes.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

/// Active background syslog capture handle.
pub struct LogCaptureGuard {
    stop_tx: Option<oneshot::Sender<()>>,
    join_handle: Option<tokio::task::JoinHandle<Result<()>>>,
    #[allow(dead_code)]
    destination: PathBuf,
}

impl LogCaptureGuard {
    /// Start capturing syslog lines to the destination file in the background.
    /// If syslog connection fails, records an explanatory banner so partial logs are never missing.
    pub async fn start(udid: &str, destination: PathBuf) -> Self {
        let target_udid = udid.to_string();
        Self::start_connecting(destination, async move {
            let connect_opts = ios_core::device::ConnectOptions {
                tun_mode: ios_core::TunMode::Userspace,
                pair_record_path: None,
                skip_tunnel: true,
            };
            let device = Box::pin(ios_core::connect(&target_udid, connect_opts)).await?;
            Ok(Box::pin(device.connect_service(ios_core::syslog::SERVICE_NAME)).await?)
        })
        .await
    }

    async fn start_connecting(
        destination: PathBuf,
        connect: impl std::future::Future<Output = Result<ios_core::device::ServiceStream>>
            + Send
            + 'static,
    ) -> Self {
        let (stop_tx, mut stop_rx) = oneshot::channel();
        let path = destination.clone();

        let join_handle = tokio::spawn(async move {
            let mut file = tokio::fs::OpenOptions::new()
                .create(true)
                .write(true)
                .truncate(true)
                .open(&path)
                .await
                .context("opening workflow log")?;

            let header = format!(
                "[test workflow log capture started: {} (udid: [PRIVATE])]\n",
                chrono::Utc::now().to_rfc3339()
            );
            file.write_all(header.as_bytes()).await?;
            file.flush().await?;

            let capture = async {
                let stream = connect.await?;
                let log_stream = ios_core::syslog::into_stream(stream);
                tokio::pin!(log_stream);
                let mut bytes_written = header.len();
                loop {
                    match log_stream.next().await {
                        Some(Ok(line)) => {
                            let mut line_bytes = line.into_bytes();
                            line_bytes.push(b'\n');
                            bytes_written = bytes_written
                                .checked_add(line_bytes.len())
                                .context("workflow log size overflow")?;
                            anyhow::ensure!(
                                bytes_written <= 16 * 1024 * 1024,
                                "workflow log output limit exceeded"
                            );
                            file.write_all(&line_bytes).await?;
                        }
                        Some(Err(error)) => {
                            return Err(anyhow::Error::new(error).context("reading workflow syslog"))
                        }
                        None => anyhow::bail!("workflow syslog ended before capture was stopped"),
                    }
                }
            };
            // Dropping the complete future closes an in-flight mux/service
            // connection too; stop is not limited to the streaming phase.
            let result: Result<()> = tokio::select! {
                _ = &mut stop_rx => Ok(()),
                result = capture => result,
            };

            let footer = format!(
                "[test workflow log capture finished: {}]\n",
                chrono::Utc::now().to_rfc3339()
            );
            let footer_result = async {
                file.write_all(footer.as_bytes()).await?;
                file.flush().await?;
                Ok(())
            }
            .await;
            combine_results(result, footer_result, "workflow log footer failed")
        });

        Self {
            stop_tx: Some(stop_tx),
            join_handle: Some(join_handle),
            destination,
        }
    }

    /// Stop log capture and wait bounded time for flush.
    pub async fn stop(mut self) -> Result<()> {
        if let Some(tx) = self.stop_tx.take() {
            let _ = tx.send(());
        }
        if let Some(mut handle) = self.join_handle.take() {
            match tokio::time::timeout(Duration::from_secs(3), &mut handle).await {
                Ok(result) => result.context("joining workflow log capture")?,
                Err(elapsed) => {
                    handle.abort();
                    let joined = handle.await;
                    anyhow::ensure!(
                        joined.is_err(),
                        "aborted workflow log task unexpectedly succeeded"
                    );
                    Err(anyhow::Error::new(elapsed).context(
                        "workflow log flush deadline expired; original task aborted and joined",
                    ))
                }
            }
        } else {
            Ok(())
        }
    }

    #[allow(dead_code)]
    pub fn destination(&self) -> &Path {
        &self.destination
    }
}

impl Drop for LogCaptureGuard {
    fn drop(&mut self) {
        if let Some(tx) = self.stop_tx.take() {
            let _ = tx.send(());
        }
        if let Some(handle) = self.join_handle.take() {
            handle.abort();
        }
    }
}

/// Extract failed test selector strings from a summary (e.g. `ClassName/methodName`).
pub fn extract_failed_selectors(summary: &TestRunSummary) -> Vec<String> {
    if summary.terminal_outcome().is_err() {
        return Vec::new();
    }
    summary
        .suites
        .iter()
        .flat_map(|suite| suite.cases.iter())
        .filter(|case| matches!(case.status, Some(TestCaseStatus::Failed)))
        .map(|case| format!("{}/{}", case.class_name, case.method_name))
        .collect()
}

/// Merge rerun test results into the primary summary, replacing prior failures with new outcomes.
pub fn merge_rerun_summary(primary: &mut TestRunSummary, rerun: TestRunSummary) -> Result<()> {
    rerun.terminal_outcome()?;
    let expected: std::collections::HashSet<_> = primary
        .suites
        .iter()
        .flat_map(|suite| &suite.cases)
        .filter(|case| matches!(case.status, Some(TestCaseStatus::Failed)))
        .map(|case| (case.class_name.clone(), case.method_name.clone()))
        .collect();
    let mut rerun_cases = HashMap::new();
    for suite in rerun.suites {
        for case in suite.cases {
            let key = (case.class_name.clone(), case.method_name.clone());
            anyhow::ensure!(
                expected.contains(&key),
                "rerun returned an unselected test case"
            );
            anyhow::ensure!(
                rerun_cases.insert(key, case).is_none(),
                "rerun returned a duplicate test case"
            );
        }
    }
    anyhow::ensure!(
        rerun_cases.len() == expected.len(),
        "rerun omitted a selected failed test case"
    );

    let mut failed_count = 0u64;
    for suite in &mut primary.suites {
        for case in &mut suite.cases {
            let key = (case.class_name.clone(), case.method_name.clone());
            if let Some(new_case) = rerun_cases.remove(&key) {
                // Preserve the initial failure in the collected run artifact
                // before replacing its current outcome with the rerun result.
                if matches!(case.status, Some(TestCaseStatus::Failed)) {
                    primary
                        .debug_logs
                        .push(format!("workflow rerun prior result: {case:?}"));
                }
                *case = new_case;
            }
            if matches!(case.status, Some(TestCaseStatus::Failed)) {
                failed_count += 1;
            }
        }
        suite.failures = Some(
            suite
                .cases
                .iter()
                .filter(|case| matches!(case.status, Some(TestCaseStatus::Failed)))
                .count() as u64,
        );
        suite.skipped = Some(
            suite
                .cases
                .iter()
                .filter(|case| matches!(case.status, Some(TestCaseStatus::Skipped)))
                .count() as u64,
        );
    }
    primary.failed_tests = failed_count;
    primary.skipped_tests = primary
        .suites
        .iter()
        .flat_map(|suite| &suite.cases)
        .filter(|case| matches!(case.status, Some(TestCaseStatus::Skipped)))
        .count() as u64;
    primary.logs.extend(rerun.logs);
    primary.debug_logs.extend(rerun.debug_logs);
    Ok(())
}

/// Add the final workflow outcome only after cleanup and artifact owners have finished.
pub(crate) fn finish_receipt(
    receipt: &mut serde_json::Value,
    summary: &TestRunSummary,
    attempts: &AttemptHistory,
    outcome: &Result<()>,
) -> Result<()> {
    receipt["summary"] = serde_json::to_value(summary)?;
    receipt["attempts"] = serde_json::to_value(attempts)?;
    receipt["outcome"] = serde_json::json!(if outcome.is_ok() { "passed" } else { "failed" });
    receipt["test_outcome"] = match summary.terminal_outcome() {
        Ok(outcome) => serde_json::to_value(outcome)?,
        Err(_) => serde_json::json!("incomplete"),
    };
    receipt["terminal_error_class"] = serde_json::to_value(
        outcome
            .as_ref()
            .err()
            .map(|error| ios_core::classify_error(error.as_ref())),
    )?;
    Ok(())
}

/// Manage on-demand app installation and tracked cleanup.
pub struct AppLifecycleTracker {
    installed_bundles: Vec<String>,
    should_cleanup: bool,
}

impl AppLifecycleTracker {
    pub fn new(should_cleanup: bool) -> Self {
        Self {
            installed_bundles: Vec::new(),
            should_cleanup,
        }
    }

    /// Track an installed bundle ID for later cleanup.
    pub fn track(&mut self, bundle_id: String) {
        if !self.installed_bundles.contains(&bundle_id) {
            self.installed_bundles.push(bundle_id);
        }
    }

    /// Perform cleanup if `--uninstall-on-finish` was specified.
    pub async fn cleanup(&self, udid: &str) -> Result<()> {
        if !self.should_cleanup || self.installed_bundles.is_empty() {
            return Ok(());
        }
        let connect_opts = ios_core::device::ConnectOptions {
            tun_mode: ios_core::TunMode::Userspace,
            pair_record_path: None,
            skip_tunnel: true,
        };
        let cleanup = async {
            let device = Box::pin(ios_core::connect(udid, connect_opts)).await?;
            let stream =
                Box::pin(device.connect_service(ios_core::apps::INSTALLATION_PROXY_SERVICE))
                    .await?;
            let mut proxy = ios_core::apps::InstallationProxy::new(stream);
            self.cleanup_connected(&mut proxy).await
        };
        tokio::time::timeout(Duration::from_secs(15), cleanup)
            .await
            .context("workflow uninstall cleanup deadline expired")?
    }

    async fn cleanup_connected<S>(
        &self,
        proxy: &mut ios_core::apps::InstallationProxy<S>,
    ) -> Result<()>
    where
        S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin + Send,
    {
        let mut result = Ok(());
        for bundle_id in self.installed_bundles.iter().rev() {
            match proxy.lookup_app(bundle_id).await {
                Ok(None) => continue, // A failed install may create no application.
                Ok(Some(_)) => {}
                Err(error) => {
                    result = combine_results(
                        result,
                        Err(error.into()),
                        "workflow ownership lookup failed",
                    );
                    continue;
                }
            }
            tracing::info!(bundle_id = %bundle_id, "test workflow: uninstalling bundle");
            let uninstall = proxy.uninstall(bundle_id).await.map_err(anyhow::Error::new);
            result = combine_results(result, uninstall, "workflow uninstall failed");
        }
        result
    }
}

/// Helper to install an application or runner package if provided.
pub async fn install_package_if_present(
    udid: &str,
    package_path: Option<&Path>,
    tracker: &mut AppLifecycleTracker,
) -> Result<()> {
    let Some(path) = package_path else {
        return Ok(());
    };
    anyhow::ensure!(
        path.is_file(),
        "package path '{}' does not exist",
        path.display()
    );
    let bundle_id = package_bundle_id(path)?;

    let connect_opts = ios_core::device::ConnectOptions {
        tun_mode: ios_core::TunMode::Userspace,
        pair_record_path: None,
        skip_tunnel: true,
    };
    let device = Box::pin(ios_core::connect(udid, connect_opts))
        .await
        .context("connecting to device for test package installation")?;

    let lookup_stream =
        Box::pin(device.connect_service(ios_core::apps::INSTALLATION_PROXY_SERVICE))
            .await
            .context("checking package ownership before installation")?;
    let mut proxy = ios_core::apps::InstallationProxy::new(lookup_stream);
    let existed = proxy.lookup_app(&bundle_id).await?.is_some();
    // Record an absent bundle before the first possible install write so a
    // partially successful installation is cleaned after an error too. An
    // existing application is never owned by this run and never uninstalled.
    if !existed {
        tracker.track(bundle_id);
    }

    tracing::info!(package = %path.display(), "test workflow: installing package");
    let svc_name = ios_core::apps::zipconduit::SERVICE_NAME;
    let mut stream = Box::pin(device.connect_service(svc_name))
        .await
        .context("failed to connect streaming_zip_conduit service")?;
    ios_core::apps::install_ipa_with_checked_progress(&mut stream, path, None)
        .await
        .with_context(|| format!("failed installing package '{}'", path.display()))?;

    Ok(())
}

/// Read the sole top-level application's identifier, never the archive filename.
pub(crate) fn package_bundle_id(path: &Path) -> Result<String> {
    let file = std::fs::File::open(path).context("opening workflow package")?;
    let mut archive = zip::ZipArchive::new(file).context("reading workflow package ZIP")?;
    anyhow::ensure!(
        archive.len() <= 100_000,
        "workflow package entry limit exceeded"
    );
    let names: Vec<String> = archive
        .file_names()
        .filter(|name| {
            let parts: Vec<_> = name.split('/').collect();
            parts.len() == 3
                && parts[0] == "Payload"
                && parts[1].ends_with(".app")
                && parts[2] == "Info.plist"
        })
        .map(str::to_owned)
        .collect();
    anyhow::ensure!(
        names.len() == 1,
        "workflow package must contain exactly one Payload application Info.plist"
    );
    let mut entry = archive
        .by_name(&names[0])
        .context("opening package Info.plist")?;
    anyhow::ensure!(
        entry.size() <= 1024 * 1024,
        "workflow package Info.plist limit exceeded"
    );
    let mut bytes = Vec::new();
    entry
        .by_ref()
        .take(1024 * 1024 + 1)
        .read_to_end(&mut bytes)?;
    anyhow::ensure!(
        bytes.len() <= 1024 * 1024,
        "workflow package Info.plist limit exceeded"
    );
    let plist = plist::Value::from_reader(std::io::Cursor::new(bytes))
        .context("decoding package Info.plist")?;
    let id = plist
        .as_dictionary()
        .and_then(|dict| dict.get("CFBundleIdentifier"))
        .and_then(plist::Value::as_string)
        .context("package lacks CFBundleIdentifier")?;
    anyhow::ensure!(
        !id.is_empty()
            && id.len() <= 255
            && id.split('.').all(|part| !part.is_empty()
                && part
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')),
        "invalid package CFBundleIdentifier"
    );
    Ok(id.to_string())
}

#[derive(Debug)]
pub(crate) struct WorkflowSecondaryFailure {
    phase: &'static str,
    pub(crate) error: anyhow::Error,
}

impl std::fmt::Display for WorkflowSecondaryFailure {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            formatter,
            "{} ({})",
            self.phase,
            ios_core::classify_error(self.error.as_ref())
        )
    }
}

pub(crate) fn combine_results(
    primary: Result<()>,
    secondary: Result<()>,
    phase: &'static str,
) -> Result<()> {
    match (primary, secondary) {
        (Ok(()), result) => result,
        (Err(error), Ok(())) => Err(error),
        (Err(error), Err(secondary)) => Err(error.context(WorkflowSecondaryFailure {
            phase,
            error: secondary,
        })),
    }
}

/// Every stage outcome, including reporting errors, passes through this owner.
pub(crate) async fn finish_workflow(
    udid: &str,
    tracker: &AppLifecycleTracker,
    logs: Option<LogCaptureGuard>,
    primary: Result<()>,
) -> Result<()> {
    let mut result = primary;
    if let Some(logs) = logs {
        result = combine_results(result, logs.stop().await, "workflow log cleanup failed");
    }
    combine_results(
        result,
        tracker.cleanup(udid).await,
        "workflow application cleanup failed",
    )
}

/// Read-only preflight on device using an active ImageMounter client:
/// checks Developer Mode and DDI readiness, returning actionable diagnostics if requirements are not met.
#[cfg(test)]
async fn preflight_device_readiness_with_client<S>(
    client: &mut ios_core::imagemounter::ImageMounterClient<S>,
    udid: &str,
) -> Result<()>
where
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin + Send,
{
    probe_device_readiness(client, udid, true, &["Developer", "Personalized"]).await
}

async fn probe_device_readiness<S>(
    client: &mut ios_core::imagemounter::ImageMounterClient<S>,
    udid: &str,
    developer_mode_required: bool,
    image_types: &[&str],
) -> Result<()>
where
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin + Send,
{
    // 1. Check Developer Mode status
    if developer_mode_required {
        match client.query_developer_mode_status_checked().await {
        Ok(Some(false)) => {
            anyhow::bail!(
                "Developer Mode is disabled on device '{udid}'. Enable it under Settings -> Privacy & Security -> Developer Mode and restart device before running tests."
            );
        }
        Ok(Some(true)) => {}
        Ok(None) => anyhow::bail!("Developer Mode status is unknown on device '{udid}'; run 'ios doctor --help' and inspect the explicit target before installing tests"),
        Err(error) => return Err(anyhow::Error::new(error).context("test workflow could not observe Developer Mode readiness")),
    }
    }

    // 2. Check DDI readiness: look up image signatures or CopyDevices
    let mut has_signatures = false;
    for image_type in image_types {
        let signatures = client
            .lookup_image_signatures_checked(image_type)
            .await
            .with_context(|| {
                format!("test workflow could not observe {image_type} image signatures")
            })?;
        has_signatures |=
            signatures.is_some_and(|values| values.iter().any(|value| !value.is_empty()));
    }
    if !has_signatures {
        let devices = client
            .copy_devices()
            .await
            .context("test workflow could not observe mounted DDI entries")?;
        let has_mounted = devices.iter().any(|entry| {
            entry
                .get("DiskImageType")
                .and_then(plist::Value::as_string)
                .is_some_and(|kind| image_types.contains(&kind))
        });
        if !has_mounted {
            anyhow::bail!(
                "Developer Disk Image (DDI) is not mounted on device '{udid}'. Mount DDI using 'ios ddi mount' before running tests."
            );
        }
    }

    Ok(())
}

/// Perform read-only preflight on the device at `udid` before modifying any state.
pub async fn preflight_device_readiness(udid: &str, deadline: Instant) -> Result<()> {
    let connect_opts = ios_core::device::ConnectOptions {
        tun_mode: ios_core::TunMode::Userspace,
        pair_record_path: None,
        skip_tunnel: true,
    };
    let connect_fut = async {
        let device = Box::pin(ios_core::connect(udid, connect_opts)).await?;
        let version = device.product_version().await?;
        let stream = Box::pin(device.connect_service(ios_core::imagemounter::SERVICE_NAME))
            .await.context("test workflow readiness service unavailable; inspect the explicit target with ios doctor")?;
        let mut client = ios_core::imagemounter::ImageMounterClient::new(stream);
        let image_type = if version.major >= 17 {
            "Personalized"
        } else {
            "Developer"
        };
        probe_device_readiness(&mut client, udid, version.major >= 16, &[image_type]).await
    };

    tokio::time::timeout_at(deadline, connect_fut)
        .await
        .context("test workflow preflight: deadline expired while checking device readiness")?
}

/// Execution options for running the full test workflow orchestration.
#[derive(Debug, Clone)]
pub struct TestWorkflowRunOptions {
    pub startup_timeout: Duration,
    pub result_timeout: Duration,
    pub json_output: bool,
    pub untested_banner: bool,
    pub untested_receipt: bool,
    pub monitored_bundle_ids: Vec<String>,
}

pub(crate) fn print_receipt(json_output: bool, value: &serde_json::Value) -> Result<()> {
    if json_output {
        crate::output::outln!("{}", serde_json::to_string_pretty(value)?);
    } else if let Some(status) = value.get("status").and_then(serde_json::Value::as_str) {
        let pid = value
            .get("pid")
            .and_then(serde_json::Value::as_u64)
            .map(|pid| format!(" (pid {pid})"))
            .unwrap_or_default();
        let status = if value.get("outcome").and_then(serde_json::Value::as_str) == Some("failed") {
            "failed"
        } else {
            status
        };
        crate::output::outln!("XCTest {status}{pid}");
    }
    Ok(())
}

/// Unified, composable orchestrator for test execution workflows (F02).
pub struct TestWorkflowEngine<'a> {
    pub udid: &'a str,
    pub workflow: &'a TestWorkflowArgs,
    pub deadline: Instant,
    pub wait: bool,
    pub junit_output: Option<&'a Path>,
}

impl<'a> TestWorkflowEngine<'a> {
    pub fn new(
        udid: &'a str,
        workflow: &'a TestWorkflowArgs,
        deadline: Instant,
        wait: bool,
        junit_output: Option<&'a Path>,
    ) -> Self {
        Self {
            udid,
            workflow,
            deadline,
            wait,
            junit_output,
        }
    }

    /// Execute read-only preflight checks (both arguments and device readiness).
    pub async fn preflight(&self) -> Result<()> {
        self.workflow.preflight()?;
        tokio::select! {
            biased;
            signal = tokio::signal::ctrl_c() => {
                signal.context("waiting for XCTest preflight cancellation")?;
                Err(ios_core::OperationCancelled::new().into())
            }
            result = preflight_device_readiness(self.udid, self.deadline) => result,
        }
    }

    /// Install explicitly requested packages, recording them in `app_tracker`.
    /// Never called during retries to avoid duplicating side effects.
    pub async fn install_explicit_packages(
        &self,
        app_tracker: &mut AppLifecycleTracker,
    ) -> Result<()> {
        if let Some(app_path) = self.workflow.install_app.as_deref() {
            tokio::time::timeout_at(
                self.deadline,
                install_package_if_present(self.udid, Some(app_path), app_tracker),
            )
            .await
            .context("XCTest deadline expired during application package installation")??;
        }
        if let Some(runner_path) = self.workflow.install_runner.as_deref() {
            tokio::time::timeout_at(
                self.deadline,
                install_package_if_present(self.udid, Some(runner_path), app_tracker),
            )
            .await
            .context("XCTest deadline expired during test runner package installation")??;
        }
        Ok(())
    }

    /// Execute the complete test workflow:
    /// preflight -> baseline crashes -> install packages -> connect & resolve plan ->
    /// start test session -> collect results -> rerun failed tests (without re-installing) ->
    /// write JUnit & attempt sidecars -> graceful PID/session shutdown -> package uninstallation -> receipt output.
    pub async fn run<F>(&self, options: TestWorkflowRunOptions, resolve_plan: F) -> Result<()>
    where
        F: for<'d> FnOnce(
                &'d ConnectedDevice,
            ) -> Pin<
                Box<dyn Future<Output = Result<(String, TestLaunchPlan)>> + Send + 'd>,
            > + Send,
    {
        if let Err(error) = self.preflight().await {
            let report_result =
                write_junit_diagnostic(self.junit_output, &incomplete_summary(false), &error);
            if let Err(report_error) = report_result {
                return combine_results(
                    Err(error),
                    Err(report_error),
                    "workflow diagnostic report failed",
                );
            }
            return Err(error);
        }

        let mut app_tracker = AppLifecycleTracker::new(self.workflow.uninstall_on_finish);
        let log_guard = if let Some(log_path) = self.workflow.log_output.clone() {
            Some(LogCaptureGuard::start(self.udid, log_path).await)
        } else {
            None
        };

        let native = crate::cmd::tunnel::NativeScope::new()?;
        let mut crashes = self.workflow.crash_output.as_deref().map(|root| {
            crash::CrashCapture::new(root, options.monitored_bundle_ids.iter().cloned())
        });

        let mut workflow_sessions = Vec::new();
        let mut startup_scopes = Vec::<TestStartupCleanup>::new();
        let mut attempts = AttemptHistory::default();
        let mut latest_summary = None;
        let mut pending_startup = None;
        let mut pending_finished = None;

        let work = Box::pin(async {
            if let Some(crashes) = &mut crashes {
                crashes.baseline(self.udid, self.deadline, &native).await?;
            }

            if let Err(error) = self.install_explicit_packages(&mut app_tracker).await {
                let report_result =
                    write_junit_diagnostic(self.junit_output, &incomplete_summary(false), &error);
                if let Err(report_error) = report_result {
                    return combine_results(
                        Err(error),
                        Err(report_error),
                        "workflow diagnostic report failed",
                    );
                }
                return Err(error);
            }

            if options.untested_banner {
                crate::output::write_error_line(format_args!(
                    "UNTESTED: XCTest execution workflow has automated coverage, but no real-device validation in this workspace yet."
                ))?;
            }

            let device = match Box::pin(connect_testmanager_device_scoped(self.udid, &native)).await
            {
                Ok(device) => device,
                Err(error) => {
                    let report_result = write_junit_diagnostic(
                        self.junit_output,
                        &incomplete_summary(false),
                        &error,
                    );
                    if let Err(report_error) = report_result {
                        return combine_results(
                            Err(error),
                            Err(report_error),
                            "workflow diagnostic report failed",
                        );
                    }
                    return Err(error);
                }
            };

            let (configuration_name, plan) = match resolve_plan(&device).await {
                Ok(plan) => plan,
                Err(error) => {
                    let report_result = write_junit_diagnostic(
                        self.junit_output,
                        &incomplete_summary(false),
                        &error,
                    );
                    if let Err(report_error) = report_result {
                        return combine_results(
                            Err(error),
                            Err(report_error),
                            "workflow diagnostic report failed",
                        );
                    }
                    return Err(error);
                }
            };

            if self.wait {
                attempts.start(plan.tests_to_run.clone());
                startup_scopes.push(TestStartupCleanup::default());
                let startup = tokio::time::timeout(
                    options.startup_timeout,
                    Box::pin(start_test_plan_session_owned_until(
                        self.udid,
                        plan.clone(),
                        self.deadline,
                        startup_scopes.last_mut().expect("startup owner admitted"),
                        &native,
                    )),
                )
                .await;
                let session = match startup {
                    Err(elapsed) => {
                        let error =
                            xctest_wait_timeout(elapsed, "timed out waiting for XCTest startup");
                        let report_result = write_junit_diagnostic(
                            self.junit_output,
                            &incomplete_summary(false),
                            &error,
                        );
                        if let Err(report_error) = report_result {
                            return combine_results(
                                Err(error),
                                Err(report_error),
                                "workflow diagnostic report failed",
                            );
                        }
                        return Err(error);
                    }
                    Ok(Err(error)) => {
                        let report_result = write_junit_diagnostic(
                            self.junit_output,
                            &incomplete_summary(false),
                            &error,
                        );
                        if let Err(report_error) = report_result {
                            return combine_results(
                                Err(error),
                                Err(report_error),
                                "workflow diagnostic report failed",
                            );
                        }
                        return Err(error);
                    }
                    Ok(Ok(session)) => session,
                };

                workflow_sessions.push(session);
                let session = workflow_sessions.last_mut().expect("stored active session");
                let result = session.startup_result().clone();
                let result_wait =
                    tokio::time::timeout(options.result_timeout, session.wait_for_results()).await;
                let mut summary = match result_wait {
                    Err(elapsed) => {
                        session.terminate().await;
                        let error =
                            xctest_wait_timeout(elapsed, "timed out waiting for XCTest results");
                        let report_result = write_junit_diagnostic(
                            self.junit_output,
                            &incomplete_summary(true),
                            &error,
                        );
                        if let Err(report_error) = report_result {
                            return combine_results(
                                Err(error),
                                Err(report_error),
                                "workflow diagnostic report failed",
                            );
                        }
                        return Err(error);
                    }
                    Ok(Err(error)) => {
                        session.terminate().await;
                        let report_result = write_junit_diagnostic(
                            self.junit_output,
                            &incomplete_summary(true),
                            &error,
                        );
                        if let Err(report_error) = report_result {
                            return combine_results(
                                Err(error),
                                Err(report_error),
                                "workflow diagnostic report failed",
                            );
                        }
                        return Err(error);
                    }
                    Ok(Ok(summary)) => summary,
                };

                let mut finished_receipt = serde_json::json!({
                    "status": "finished",
                    "configuration": configuration_name,
                    "runner_bundle_id": result.runner_bundle_id,
                    "target_bundle_id": result.target_bundle_id,
                    "pid": result.pid,
                    "protocol_version": result.protocol_version,
                    "minimum_version": result.minimum_version,
                });
                if options.untested_receipt {
                    finished_receipt["untested"] = serde_json::json!(true);
                    finished_receipt["note"] = serde_json::json!(
                        "Result event parsing is covered offline; real XCTest devices still need validation."
                    );
                }
                pending_finished = Some(finished_receipt);
                latest_summary = Some(summary.clone());
                attempts.complete(&summary)?;
                session.terminate().await;
                if let Some(error) = session.take_termination_error() {
                    return Err(error);
                }

                summary.terminal_outcome()?;

                // Rerun failed tests if requested
                if summary.failure_count() > 0 && self.workflow.rerun_failed > 0 {
                    for rerun_idx in 0..self.workflow.rerun_failed {
                        let failed_selectors = extract_failed_selectors(&summary);
                        if failed_selectors.is_empty() {
                            break;
                        }
                        tracing::info!(
                            attempt = rerun_idx + 1,
                            failed_count = failed_selectors.len(),
                            "test workflow: rerunning failed test selectors"
                        );
                        let mut rerun_plan = plan.clone();
                        attempts.start(failed_selectors.clone());
                        rerun_plan.tests_to_run = failed_selectors;
                        rerun_plan.tests_to_skip.clear();

                        let rerun_session = match tokio::time::timeout(options.startup_timeout, {
                            startup_scopes.push(TestStartupCleanup::default());
                            Box::pin(start_test_plan_session_owned_until(
                                self.udid,
                                rerun_plan,
                                self.deadline,
                                startup_scopes
                                    .last_mut()
                                    .expect("rerun startup owner admitted"),
                                &native,
                            ))
                        })
                        .await
                        {
                            Ok(Ok(s)) => s,
                            Ok(Err(error)) => return Err(error),
                            Err(elapsed) => {
                                return Err(xctest_wait_timeout(
                                    elapsed,
                                    "timed out restarting XCTest runner",
                                ))
                            }
                        };

                        workflow_sessions.push(rerun_session);
                        let rerun_session = workflow_sessions
                            .last_mut()
                            .expect("stored active rerun session");
                        let rerun_res = tokio::time::timeout(
                            options.result_timeout,
                            rerun_session.wait_for_results(),
                        )
                        .await;
                        rerun_session.terminate().await;
                        let rerun_summary = rerun_res.map_err(|elapsed| {
                            xctest_wait_timeout(elapsed, "timed out waiting for rerun results")
                        })??;
                        attempts.complete(&rerun_summary)?;
                        if let Some(error) = rerun_session.take_termination_error() {
                            return Err(error);
                        }
                        merge_rerun_summary(&mut summary, rerun_summary)?;
                        latest_summary = Some(summary.clone());
                        if summary.failure_count() == 0 {
                            break;
                        }
                    }
                }

                write_junit_summary(self.junit_output, &summary)?;

                if summary.failure_count() > 0 {
                    return Err(anyhow::anyhow!(
                        "XCTest run completed with {} failed test(s)",
                        summary.failure_count()
                    ));
                }
                return Ok(());
            }

            // Not waiting: startup only
            startup_scopes.push(TestStartupCleanup::default());
            let session = tokio::time::timeout(
                options.startup_timeout,
                Box::pin(start_test_plan_session_owned_until(
                    self.udid,
                    plan,
                    self.deadline,
                    startup_scopes.last_mut().expect("startup owner admitted"),
                    &native,
                )),
            )
            .await
            .map_err(|error| {
                anyhow::Error::new(error).context("timed out waiting for XCTest startup")
            })??;

            let result = session.startup_result().clone();
            workflow_sessions.push(session);
            let mut startup_receipt = serde_json::json!({
                "status": "started",
                "configuration": configuration_name,
                "runner_bundle_id": result.runner_bundle_id,
                "target_bundle_id": result.target_bundle_id,
                "pid": result.pid,
                "protocol_version": result.protocol_version,
                "minimum_version": result.minimum_version,
                "post_disconnect_lifetime": "unknown",
                "note": "Startup was acknowledged. Service connections are closed before this receipt; continued runner or test execution after disconnect was not observed. Use --wait to collect result events in this invocation.",
            });
            if options.untested_receipt {
                startup_receipt["untested"] = serde_json::json!(true);
            }
            pending_startup = Some(startup_receipt);

            Ok(())
        });

        let mut outcome = tokio::select! {
            result = tokio::time::timeout_at(self.deadline, work) => result
                .map_err(|error| anyhow::Error::new(error).context("XCTest workflow deadline expired"))
                .and_then(|result| result),
            signal = tokio::signal::ctrl_c() => {
                match signal {
                    Ok(()) => Err(ios_core::OperationCancelled::new().into()),
                    Err(error) => Err(anyhow::Error::new(error).context("waiting for XCTest cancellation")),
                }
            }
        };

        if let Err(error) = &outcome {
            let partial = if attempts.attempt_count() == workflow_sessions.len() {
                workflow_sessions
                    .last()
                    .map(|session| session.partial_summary())
            } else {
                None
            };
            let recorded = attempts.fail(error, partial.as_ref());
            let diagnostic = latest_summary
                .as_ref()
                .or(partial.as_ref())
                .cloned()
                .unwrap_or_else(|| incomplete_summary(false));
            latest_summary = Some(diagnostic.clone());
            let reported = write_junit_diagnostic(self.junit_output, &diagnostic, error);
            outcome = combine_results(outcome, recorded, "workflow attempt history failed");
            outcome = combine_results(outcome, reported, "workflow diagnostic report failed");
        }

        for startup in &mut startup_scopes {
            outcome = combine_results(
                outcome,
                startup.close().await,
                "XCTest interrupted startup cleanup failed",
            );
        }

        for session in &mut workflow_sessions {
            if self.wait || outcome.is_err() {
                session.terminate().await;
            } else {
                session.detach().await;
            }
            if let Some(error) = session.take_termination_error() {
                outcome = combine_results(outcome, Err(error), "workflow runner cleanup failed");
            }
        }

        let mut outcome = finish_workflow(self.udid, &app_tracker, log_guard, outcome).await;

        if let Some(crashes) = &mut crashes {
            outcome = combine_results(
                outcome,
                crashes.finish(self.udid, &native).await,
                "workflow crash collection failed",
            );
        }

        outcome = combine_results(
            outcome,
            native.close(Duration::from_secs(5)).await,
            "workflow original native shutdown failed",
        );

        let recorded = if let Err(error) = &outcome {
            let record = attempts.fail(error, None);
            let diagnostic = latest_summary
                .as_ref()
                .cloned()
                .unwrap_or_else(|| incomplete_summary(false));
            combine_results(
                record,
                write_junit_diagnostic(self.junit_output, &diagnostic, error),
                "workflow final diagnostic failed",
            )
        } else {
            Ok(())
        };
        outcome = combine_results(outcome, recorded, "workflow terminal history failed");
        if let Err(error) = &outcome {
            let recorded = attempts.fail(error, None);
            outcome = combine_results(
                outcome,
                recorded,
                "workflow final history classification failed",
            );
        }

        let artifact_deadline = Instant::now() + Duration::from_secs(5);
        let artifact = attempts
            .write_until(self.junit_output, artifact_deadline)
            .await;
        let artifact_failed = artifact.is_err();
        outcome = combine_results(outcome, artifact, "workflow attempt artifact failed");
        if artifact_failed {
            let diagnostic = latest_summary
                .as_ref()
                .cloned()
                .unwrap_or_else(|| incomplete_summary(false));
            let reported = write_junit_diagnostic(
                self.junit_output,
                &diagnostic,
                outcome.as_ref().expect_err("failed attempt artifact"),
            );
            outcome = combine_results(outcome, reported, "workflow artifact diagnostic failed");
        }
        if let Err(error) = &outcome {
            let recorded = attempts.fail(error, None);
            outcome = combine_results(
                outcome,
                recorded,
                "workflow artifact failure classification failed",
            );
        }

        if let Some(mut receipt) = pending_finished {
            let output = (|| -> Result<()> {
                finish_receipt(
                    &mut receipt,
                    latest_summary.as_ref().expect("observed finished summary"),
                    &attempts,
                    &outcome,
                )?;
                print_receipt(options.json_output, &receipt)
            })();
            return finish_output(
                outcome,
                output,
                &mut attempts,
                self.junit_output,
                artifact_deadline,
            )
            .await;
        }

        if let Some(mut receipt) = pending_startup {
            receipt["service_cleanup"] = serde_json::json!(if outcome.is_ok() {
                "complete"
            } else {
                "failed"
            });
            receipt["cleanup_error_class"] = serde_json::to_value(
                outcome
                    .as_ref()
                    .err()
                    .map(|error| ios_core::classify_error(error.as_ref())),
            )?;
            return combine_results(
                outcome,
                print_receipt(options.json_output, &receipt),
                "XCTest startup receipt output failed",
            );
        }

        outcome
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ios_core::testmanager::results::{TestCaseSummary, TestSuiteSummary};

    #[tokio::test]
    async fn review_regression_readiness_respects_legacy_and_personalized_query_profiles() {
        for (developer_mode, image_type) in [(false, "Developer"), (true, "Personalized")] {
            let (stream, mut peer) = tokio::io::duplex(4096);
            let remote = tokio::spawn(async move {
                if developer_mode {
                    let request: plist::Dictionary =
                        ios_core::recv_lockdown(&mut peer).await.unwrap();
                    assert_eq!(
                        request["Command"].as_string(),
                        Some("QueryDeveloperModeStatus")
                    );
                    ios_core::send_lockdown(
                        &mut peer,
                        &plist::Dictionary::from_iter([(
                            "DeveloperModeStatus",
                            plist::Value::Boolean(true),
                        )]),
                    )
                    .await
                    .unwrap();
                }
                let request: plist::Dictionary = ios_core::recv_lockdown(&mut peer).await.unwrap();
                assert_eq!(request["Command"].as_string(), Some("LookupImage"));
                assert_eq!(request["ImageType"].as_string(), Some(image_type));
                ios_core::send_lockdown(
                    &mut peer,
                    &plist::Dictionary::from_iter([(
                        "ImageSignature",
                        plist::Value::Array(vec![plist::Value::Data(vec![1])]),
                    )]),
                )
                .await
                .unwrap();
            });
            let mut client = ios_core::imagemounter::ImageMounterClient::new(stream);
            probe_device_readiness(&mut client, "offline-target", developer_mode, &[image_type])
                .await
                .unwrap();
            remote.await.unwrap();
        }
    }

    #[tokio::test]
    async fn review_regression_readiness_eof_preserves_cause_and_missing_status_is_not_ready() {
        for missing_status in [false, true] {
            let (stream, mut peer) = tokio::io::duplex(4096);
            let remote = tokio::spawn(async move {
                let request: plist::Dictionary = ios_core::recv_lockdown(&mut peer).await.unwrap();
                assert_eq!(
                    request["Command"].as_string(),
                    Some("QueryDeveloperModeStatus")
                );
                if missing_status {
                    ios_core::send_lockdown(&mut peer, &plist::Dictionary::new())
                        .await
                        .unwrap();
                }
            });
            let mut client = ios_core::imagemounter::ImageMounterClient::new(stream);
            let error = preflight_device_readiness_with_client(&mut client, "offline-target")
                .await
                .unwrap_err();
            if !missing_status {
                assert!(error
                    .downcast_ref::<ios_core::imagemounter::ImageMounterError>()
                    .is_some());
            }
            remote.await.unwrap();
        }
    }

    fn attempt_summary(status: TestCaseStatus) -> TestRunSummary {
        use ios_core::testmanager::results::{TestExecutionEvent, TestRunRecorder};
        let mut recorder = TestRunRecorder::default();
        recorder.apply(TestExecutionEvent::BeganPlan);
        recorder.apply(TestExecutionEvent::SuiteStarted {
            name: "attempt".into(),
            started_at: None,
        });
        recorder.apply(TestExecutionEvent::CaseStarted {
            class_name: "Cases".into(),
            method_name: "flaky".into(),
        });
        recorder.apply(TestExecutionEvent::CaseFinished {
            class_name: "Cases".into(),
            method_name: "flaky".into(),
            status,
            duration_seconds: 0.25,
        });
        recorder.apply(TestExecutionEvent::FinishedPlan);
        recorder.summary()
    }

    #[tokio::test]
    async fn attempt_history_keeps_first_failure_and_interrupted_partial_then_atomically_replaces_sidecar(
    ) {
        let path =
            std::env::temp_dir().join(format!("workflow-attempts-{}.xml", uuid::Uuid::new_v4()));
        let mut history = AttemptHistory::default();
        let mut primary = attempt_summary(TestCaseStatus::Failed);
        history.start(vec![]);
        history.complete(&primary).unwrap();
        let rerun = attempt_summary(TestCaseStatus::Passed);
        history.start(vec!["Cases/flaky".into()]);
        history.complete(&rerun).unwrap();
        merge_rerun_summary(&mut primary, rerun).unwrap();
        assert_eq!(primary.failed_tests, 0);
        assert_eq!(
            history.attempts[0].summary.as_ref().unwrap().failed_tests,
            1
        );
        assert_eq!(
            history.attempts[1].summary.as_ref().unwrap().failed_tests,
            0
        );
        history.write(Some(&path)).await.unwrap();
        let mut sidecar = path.as_os_str().to_os_string();
        sidecar.push(".attempts.json");
        let sidecar = PathBuf::from(sidecar);
        let first: serde_json::Value =
            serde_json::from_slice(&std::fs::read(&sidecar).unwrap()).unwrap();
        assert_eq!(first["attempts"][0]["state"], "failed_tests");
        assert_eq!(first["attempts"][1]["state"], "completed");
        history.start(vec!["Cases/flaky".into()]);
        let mut partial = attempt_summary(TestCaseStatus::Failed);
        partial.finished = false;
        let error = anyhow::Error::new(std::io::Error::from(std::io::ErrorKind::ConnectionReset));
        history.fail(&error, Some(&partial)).unwrap();
        history.write(Some(&path)).await.unwrap();
        let last: serde_json::Value =
            serde_json::from_slice(&std::fs::read(&sidecar).unwrap()).unwrap();
        assert_eq!(last["attempts"].as_array().unwrap().len(), 3);
        assert_eq!(last["attempts"][2]["state"], "interrupted");
        assert_eq!(last["attempts"][2]["summary"]["finished"], false);
        assert!(!last["attempts"][2]["error_class"].is_null());
        std::fs::remove_file(sidecar).unwrap();
    }

    #[test]
    fn rerun_merge_rejects_unselected_duplicate_missing_and_unfinished_evidence_without_changing_primary(
    ) {
        let initial = attempt_summary(TestCaseStatus::Failed);
        for mode in [
            "unselected",
            "duplicate",
            "missing",
            "unfinished",
            "no-begin",
            "no-status",
            "stalled",
            "unknown",
            "uncaught",
        ] {
            let mut primary = initial.clone();
            let mut rerun = attempt_summary(TestCaseStatus::Passed);
            match mode {
                "unselected" => rerun.suites[0].cases[0].method_name = "unselected".into(),
                "duplicate" => {
                    let case = rerun.suites[0].cases[0].clone();
                    rerun.suites[0].cases.push(case);
                }
                "missing" => rerun.suites[0].cases.clear(),
                "unfinished" => rerun.finished = false,
                "no-begin" => rerun.began = false,
                "no-status" => rerun.suites[0].cases[0].status = None,
                "stalled" => rerun.suites[0].cases[0].status = Some(TestCaseStatus::Stalled),
                "unknown" => {
                    rerun.suites[0].cases[0].status = Some(TestCaseStatus::Other("future".into()))
                }
                "uncaught" => rerun.suites[0].uncaught_exceptions = Some(1),
                _ => unreachable!(),
            }
            assert!(merge_rerun_summary(&mut primary, rerun).is_err());
            assert_eq!(primary, initial);
        }
    }

    #[test]
    fn incomplete_attempt_keeps_actual_terminal_evidence_and_does_not_authorize_rerun() {
        let mut summary = attempt_summary(TestCaseStatus::Failed);
        summary.suites[0].uncaught_exceptions = Some(1);
        let mut history = AttemptHistory::default();
        history.start(vec![]);
        history.complete(&summary).unwrap();
        assert_eq!(history.attempts[0].state, "incomplete");
        assert_eq!(history.attempts[0].summary.as_ref(), Some(&summary));
        assert!(history.attempts[0].error_class.is_some());
        assert!(extract_failed_selectors(&summary).is_empty());
    }

    #[test]
    fn final_receipt_keeps_test_completion_separate_from_original_cleanup_error() {
        let summary = attempt_summary(TestCaseStatus::Passed);
        let mut history = AttemptHistory::default();
        history.start(vec![]);
        history.complete(&summary).unwrap();
        let outcome = Err(anyhow::Error::new(std::io::Error::from(
            std::io::ErrorKind::ConnectionReset,
        )));
        history.fail(outcome.as_ref().unwrap_err(), None).unwrap();
        let mut receipt = serde_json::json!({"status": "finished"});
        finish_receipt(&mut receipt, &summary, &history, &outcome).unwrap();
        assert_eq!(receipt["outcome"], "failed");
        assert_eq!(receipt["test_outcome"], "passed");
        assert_eq!(receipt["attempts"]["attempts"][0]["state"], "completed");
        assert_eq!(
            receipt["terminal_error_class"],
            receipt["attempts"]["terminal_error_class"]
        );
        assert!(outcome
            .as_ref()
            .err()
            .is_some_and(|error| error.downcast_ref::<std::io::Error>().is_some()));
    }

    #[tokio::test]
    async fn failed_output_reclassifies_sidecar_once_and_keeps_original_output_error() {
        let directory =
            std::env::temp_dir().join(format!("workflow-output-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir(&directory).unwrap();
        let path = directory.join("results.xml");
        let mut history = AttemptHistory::default();
        history.start(vec![]);
        history
            .complete(&attempt_summary(TestCaseStatus::Passed))
            .unwrap();
        let deadline = Instant::now() + Duration::from_secs(5);
        history.write_until(Some(&path), deadline).await.unwrap();
        let output = Err(anyhow::Error::new(std::io::Error::from(
            std::io::ErrorKind::BrokenPipe,
        )));
        let error = finish_output(Ok(()), output, &mut history, Some(&path), deadline)
            .await
            .unwrap_err();
        assert_eq!(
            error.downcast_ref::<std::io::Error>().unwrap().kind(),
            std::io::ErrorKind::BrokenPipe
        );
        let sidecar = directory.join("results.xml.attempts.json");
        let artifact: serde_json::Value =
            serde_json::from_slice(&std::fs::read(&sidecar).unwrap()).unwrap();
        assert_eq!(artifact["attempts"][0]["state"], "completed");
        assert!(!artifact["terminal_error_class"].is_null());
        assert_eq!(artifact, serde_json::to_value(&history).unwrap());
        std::fs::remove_file(sidecar).unwrap();
        std::fs::remove_dir(directory).unwrap();
    }

    #[tokio::test]
    async fn failed_output_with_expired_shared_artifact_clock_keeps_original_error_and_prior_file()
    {
        let directory =
            std::env::temp_dir().join(format!("workflow-output-expired-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir(&directory).unwrap();
        let path = directory.join("results.xml");
        let sidecar = directory.join("results.xml.attempts.json");
        let mut history = AttemptHistory::default();
        history.start(vec![]);
        history
            .complete(&attempt_summary(TestCaseStatus::Passed))
            .unwrap();
        history.write(Some(&path)).await.unwrap();
        let before = std::fs::read(&sidecar).unwrap();
        let output = Err(anyhow::Error::new(std::io::Error::from(
            std::io::ErrorKind::BrokenPipe,
        )));
        let error = finish_output(Ok(()), output, &mut history, Some(&path), Instant::now())
            .await
            .unwrap_err();
        assert_eq!(
            error.downcast_ref::<std::io::Error>().unwrap().kind(),
            std::io::ErrorKind::BrokenPipe
        );
        assert!(error
            .downcast_ref::<WorkflowSecondaryFailure>()
            .unwrap()
            .error
            .downcast_ref::<tokio::time::error::Elapsed>()
            .is_some());
        assert_eq!(std::fs::read(&sidecar).unwrap(), before);
        assert!(history.terminal_error_class.is_some());
        assert_eq!(std::fs::read_dir(&directory).unwrap().count(), 1);
        std::fs::remove_file(sidecar).unwrap();
        std::fs::remove_dir(directory).unwrap();
    }

    #[tokio::test]
    async fn cleanup_uses_real_proxy_lookup_reverse_order_and_retains_uninstall_failure() {
        use tokio::io::AsyncReadExt;
        let (client, mut peer) = tokio::io::duplex(65_536);
        let mut tracker = AppLifecycleTracker::new(true);
        tracker.track("com.fixture.app".into());
        tracker.track("com.fixture.runner".into());
        let work = async {
            let mut proxy = ios_core::apps::InstallationProxy::new(client);
            let result = tracker.cleanup_connected(&mut proxy).await;
            drop(proxy);
            result
        };
        let serve = async {
            for bundle in ["com.fixture.runner", "com.fixture.app"] {
                let lookup: plist::Dictionary = ios_core::recv_lockdown(&mut peer).await.unwrap();
                assert_eq!(lookup["Command"].as_string(), Some("Lookup"));
                let options = lookup["ClientOptions"].as_dictionary().unwrap();
                assert_eq!(
                    options["BundleIDs"].as_array().unwrap()[0].as_string(),
                    Some(bundle)
                );
                let result = if bundle.ends_with("runner") {
                    plist::Dictionary::new()
                } else {
                    plist::Dictionary::from_iter([(
                        bundle,
                        plist::Value::Dictionary(plist::Dictionary::new()),
                    )])
                };
                ios_core::send_lockdown(
                    &mut peer,
                    &plist::Dictionary::from_iter([(
                        "LookupResult",
                        plist::Value::Dictionary(result),
                    )]),
                )
                .await
                .unwrap();
            }
            let uninstall: plist::Dictionary = ios_core::recv_lockdown(&mut peer).await.unwrap();
            assert_eq!(uninstall["Command"].as_string(), Some("Uninstall"));
            assert_eq!(
                uninstall["ApplicationIdentifier"].as_string(),
                Some("com.fixture.app")
            );
            ios_core::send_lockdown(
                &mut peer,
                &plist::Dictionary::from_iter([(
                    "Error",
                    plist::Value::String("fixture-uninstall-failure".into()),
                )]),
            )
            .await
            .unwrap();
            assert_eq!(peer.read(&mut [0]).await.unwrap(), 0);
        };
        let (result, ()) =
            tokio::time::timeout(Duration::from_secs(2), async { tokio::join!(work, serve) })
                .await
                .unwrap();
        assert!(result
            .unwrap_err()
            .is::<ios_core::apps::installation::IpError>());
    }

    #[tokio::test]
    async fn actual_syslog_parser_keeps_partial_lines_and_footer_and_reports_peer_eof() {
        use tokio::io::AsyncReadExt;
        let directory =
            std::env::temp_dir().join(format!("workflow-log-peer-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir(&directory).unwrap();
        let path = directory.join("syslog.txt");
        let (client, mut peer) = tokio::io::duplex(65_536);
        let guard = LogCaptureGuard::start_connecting(path.clone(), async move {
            Ok(Box::new(client) as ios_core::device::ServiceStream)
        })
        .await;
        peer.write_all(b"Oct 11 12:00:00 fixture[7] <Notice>: original partial log\0")
            .await
            .unwrap();
        peer.shutdown().await.unwrap();
        assert_eq!(
            tokio::time::timeout(Duration::from_secs(2), peer.read(&mut [0]))
                .await
                .unwrap()
                .unwrap(),
            0
        );
        let error = guard.stop().await.unwrap_err();
        assert_eq!(
            error.downcast_ref::<std::io::Error>().unwrap().kind(),
            std::io::ErrorKind::UnexpectedEof
        );
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(text.contains("original partial log"));
        assert!(text.contains("log capture finished"));
        std::fs::remove_file(path).unwrap();
        std::fs::remove_dir(directory).unwrap();
    }

    #[test]
    fn preflight_checks_non_existent_packages() {
        let args = TestWorkflowArgs {
            install_app: Some(PathBuf::from("non_existent_app_xyz.ipa")),
            install_runner: None,
            log_output: None,
            crash_output: None,
            uninstall_on_finish: false,
            rerun_failed: 0,
        };
        assert!(args.preflight().is_err());

        let empty_args = TestWorkflowArgs::default();
        assert!(empty_args.preflight().is_ok());
    }

    #[test]
    fn failed_selector_extraction_identifies_exact_failed_cases() {
        let summary = TestRunSummary {
            began: true,
            finished: true,
            logs_truncated: false,
            results_truncated: false,
            total_tests: 2,
            failed_tests: 1,
            skipped_tests: 0,
            logs: vec![],
            debug_logs: vec![],
            suites: vec![TestSuiteSummary {
                name: "DemoSuite".into(),
                started_at: None,
                finished_at: None,
                test_count: Some(2),
                skipped: Some(0),
                failures: Some(1),
                expected_failures: None,
                unexpected_failures: None,
                uncaught_exceptions: None,
                test_duration_seconds: None,
                total_duration_seconds: None,
                cases: vec![
                    TestCaseSummary {
                        class_name: "DemoTests".into(),
                        method_name: "testPass".into(),
                        status: Some(TestCaseStatus::Passed),
                        duration_seconds: Some(0.1),
                        failure: None,
                    },
                    TestCaseSummary {
                        class_name: "DemoTests".into(),
                        method_name: "testFail".into(),
                        status: Some(TestCaseStatus::Failed),
                        duration_seconds: Some(0.5),
                        failure: None,
                    },
                ],
            }],
        };

        let failed = extract_failed_selectors(&summary);
        assert_eq!(failed, vec!["DemoTests/testFail"]);
    }

    #[test]
    fn merge_rerun_summary_updates_failure_counts() {
        let mut primary = TestRunSummary {
            began: true,
            finished: true,
            logs_truncated: false,
            results_truncated: false,
            total_tests: 2,
            failed_tests: 1,
            skipped_tests: 0,
            logs: vec!["initial log".into()],
            debug_logs: vec![],
            suites: vec![TestSuiteSummary {
                name: "DemoSuite".into(),
                started_at: None,
                finished_at: None,
                test_count: Some(2),
                skipped: Some(0),
                failures: Some(1),
                expected_failures: None,
                unexpected_failures: None,
                uncaught_exceptions: None,
                test_duration_seconds: None,
                total_duration_seconds: None,
                cases: vec![
                    TestCaseSummary {
                        class_name: "DemoTests".into(),
                        method_name: "testPass".into(),
                        status: Some(TestCaseStatus::Passed),
                        duration_seconds: Some(0.1),
                        failure: None,
                    },
                    TestCaseSummary {
                        class_name: "DemoTests".into(),
                        method_name: "testFail".into(),
                        status: Some(TestCaseStatus::Failed),
                        duration_seconds: Some(0.5),
                        failure: None,
                    },
                ],
            }],
        };

        let rerun = TestRunSummary {
            began: true,
            finished: true,
            logs_truncated: false,
            results_truncated: false,
            total_tests: 1,
            failed_tests: 0,
            skipped_tests: 0,
            logs: vec!["rerun log".into()],
            debug_logs: vec![],
            suites: vec![TestSuiteSummary {
                name: "DemoSuite".into(),
                started_at: None,
                finished_at: None,
                test_count: Some(1),
                skipped: Some(0),
                failures: Some(0),
                expected_failures: None,
                unexpected_failures: None,
                uncaught_exceptions: None,
                test_duration_seconds: None,
                total_duration_seconds: None,
                cases: vec![TestCaseSummary {
                    class_name: "DemoTests".into(),
                    method_name: "testFail".into(),
                    status: Some(TestCaseStatus::Passed),
                    duration_seconds: Some(0.4),
                    failure: None,
                }],
            }],
        };

        merge_rerun_summary(&mut primary, rerun).unwrap();
        assert_eq!(primary.failed_tests, 0);
        assert_eq!(
            primary.suites[0].cases[1].status,
            Some(TestCaseStatus::Passed)
        );
        assert_eq!(primary.logs.len(), 2);
    }

    #[tokio::test]
    async fn log_capture_guard_writes_header_and_footer() {
        let temp_dir = std::env::temp_dir().join(format!("test_log_{}", uuid::Uuid::new_v4()));
        let _ = std::fs::create_dir_all(&temp_dir);
        let log_file = temp_dir.join("test_run.log");

        let guard = LogCaptureGuard::start_connecting(log_file.clone(), async {
            Err(
                std::io::Error::new(std::io::ErrorKind::ConnectionRefused, "offline fixture")
                    .into(),
            )
        })
        .await;
        assert_eq!(guard.destination(), log_file.as_path());
        tokio::time::sleep(Duration::from_millis(50)).await;
        let _ = guard.stop().await; // Missing synthetic target is a retained failure.

        let content = std::fs::read_to_string(&log_file).unwrap();
        assert!(content.contains("[test workflow log capture started"));
        assert!(content.contains("[test workflow log capture finished"));

        let _ = std::fs::remove_dir_all(&temp_dir);
    }

    #[tokio::test]
    async fn stopping_pending_connection_joins_original_task_and_closes_peer() {
        use tokio::io::AsyncReadExt;
        let directory =
            std::env::temp_dir().join(format!("workflow-pending-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir(&directory).unwrap();
        let path = directory.join("log.txt");
        let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
            .await
            .unwrap();
        let (client, accepted) = tokio::try_join!(
            tokio::net::TcpStream::connect(listener.local_addr().unwrap()),
            listener.accept()
        )
        .unwrap();
        let (mut peer, _) = accepted;
        let (entered, entering) = oneshot::channel();
        let guard = LogCaptureGuard::start_connecting(path.clone(), async move {
            let retained = client;
            entered.send(()).unwrap();
            std::future::pending::<()>().await;
            Ok(Box::new(retained) as ios_core::device::ServiceStream)
        })
        .await;
        tokio::time::timeout(Duration::from_secs(2), entering)
            .await
            .unwrap()
            .unwrap();
        tokio::time::timeout(Duration::from_secs(1), guard.stop())
            .await
            .unwrap()
            .unwrap();
        let mut byte = [0];
        assert_eq!(
            tokio::time::timeout(Duration::from_secs(1), peer.read(&mut byte))
                .await
                .unwrap()
                .unwrap(),
            0
        );
        assert!(std::fs::read_to_string(&path)
            .unwrap()
            .contains("log capture finished"));
        std::fs::remove_file(path).unwrap();
        std::fs::remove_dir(directory).unwrap();
    }

    #[test]
    fn archive_identifier_is_used_even_when_filename_names_another_bundle() {
        let path =
            std::env::temp_dir().join(format!("com.other.existing-{}.ipa", uuid::Uuid::new_v4()));
        let file = std::fs::File::create(&path).unwrap();
        let mut archive = zip::ZipWriter::new(file);
        archive
            .start_file(
                "Payload/Runner.app/Info.plist",
                zip::write::SimpleFileOptions::default(),
            )
            .unwrap();
        let mut dictionary = plist::Dictionary::new();
        dictionary.insert(
            "CFBundleIdentifier".into(),
            "com.synthetic.actual-runner".into(),
        );
        plist::Value::Dictionary(dictionary)
            .to_writer_xml(&mut archive)
            .unwrap();
        archive.finish().unwrap();
        assert_eq!(
            package_bundle_id(&path).unwrap(),
            "com.synthetic.actual-runner"
        );
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn secondary_cleanup_error_retains_primary_and_secondary_objects() {
        let primary = std::io::Error::from(std::io::ErrorKind::ConnectionRefused);
        let secondary = std::io::Error::from(std::io::ErrorKind::PermissionDenied);
        let error = combine_results(
            Err(primary.into()),
            Err(secondary.into()),
            "synthetic cleanup",
        )
        .unwrap_err();
        assert_eq!(
            error.downcast_ref::<std::io::Error>().unwrap().kind(),
            std::io::ErrorKind::ConnectionRefused
        );
        let cleanup = error.downcast_ref::<WorkflowSecondaryFailure>().unwrap();
        assert_eq!(
            cleanup
                .error
                .downcast_ref::<std::io::Error>()
                .unwrap()
                .kind(),
            std::io::ErrorKind::PermissionDenied
        );
    }

    #[tokio::test]
    async fn preflight_detects_disabled_developer_mode_and_returns_actionable_error() {
        let (client_stream, mut peer) = tokio::io::duplex(4096);
        let mut client = ios_core::imagemounter::ImageMounterClient::new(client_stream);

        let server = async move {
            let req: plist::Dictionary = ios_core::recv_lockdown(&mut peer).await.unwrap();
            assert_eq!(req["Command"].as_string(), Some("QueryDeveloperModeStatus"));
            let mut resp = plist::Dictionary::new();
            resp.insert("DeveloperModeStatus".into(), plist::Value::Boolean(false));
            ios_core::send_lockdown(&mut peer, &resp).await.unwrap();
        };

        let client_work = async move {
            let err = preflight_device_readiness_with_client(&mut client, "test-udid-123")
                .await
                .unwrap_err();
            assert!(err
                .to_string()
                .contains("Developer Mode is disabled on device 'test-udid-123'"));
            assert!(err
                .to_string()
                .contains("Settings -> Privacy & Security -> Developer Mode"));
        };

        tokio::join!(server, client_work);
    }

    #[tokio::test]
    async fn preflight_detects_unmounted_ddi_and_returns_actionable_error() {
        let (client_stream, mut peer) = tokio::io::duplex(4096);
        let mut client = ios_core::imagemounter::ImageMounterClient::new(client_stream);

        let server = async move {
            // 1. QueryDeveloperModeStatus -> true
            let req: plist::Dictionary = ios_core::recv_lockdown(&mut peer).await.unwrap();
            assert_eq!(req["Command"].as_string(), Some("QueryDeveloperModeStatus"));
            let mut resp = plist::Dictionary::new();
            resp.insert("DeveloperModeStatus".into(), plist::Value::Boolean(true));
            ios_core::send_lockdown(&mut peer, &resp).await.unwrap();

            // 2. LookupImage Developer -> empty
            let req: plist::Dictionary = ios_core::recv_lockdown(&mut peer).await.unwrap();
            assert_eq!(req["Command"].as_string(), Some("LookupImage"));
            let mut resp = plist::Dictionary::new();
            resp.insert("ImageSignature".into(), plist::Value::Array(vec![]));
            ios_core::send_lockdown(&mut peer, &resp).await.unwrap();

            // 3. LookupImage Personalized -> empty
            let req: plist::Dictionary = ios_core::recv_lockdown(&mut peer).await.unwrap();
            assert_eq!(req["Command"].as_string(), Some("LookupImage"));
            let mut resp = plist::Dictionary::new();
            resp.insert("ImageSignature".into(), plist::Value::Array(vec![]));
            ios_core::send_lockdown(&mut peer, &resp).await.unwrap();

            // 4. CopyDevices -> empty EntryList
            let req: plist::Dictionary = ios_core::recv_lockdown(&mut peer).await.unwrap();
            assert_eq!(req["Command"].as_string(), Some("CopyDevices"));
            let mut resp = plist::Dictionary::new();
            resp.insert("EntryList".into(), plist::Value::Array(vec![]));
            ios_core::send_lockdown(&mut peer, &resp).await.unwrap();
        };

        let client_work = async move {
            let err = preflight_device_readiness_with_client(&mut client, "test-udid-456")
                .await
                .unwrap_err();
            assert!(err
                .to_string()
                .contains("Developer Disk Image (DDI) is not mounted on device 'test-udid-456'"));
            assert!(err.to_string().contains("ios ddi mount"));
        };

        tokio::join!(server, client_work);
    }

    #[tokio::test]
    async fn preflight_succeeds_when_developer_mode_and_ddi_are_ready() {
        let (client_stream, mut peer) = tokio::io::duplex(4096);
        let mut client = ios_core::imagemounter::ImageMounterClient::new(client_stream);

        let server = async move {
            // 1. QueryDeveloperModeStatus -> true
            let req: plist::Dictionary = ios_core::recv_lockdown(&mut peer).await.unwrap();
            assert_eq!(req["Command"].as_string(), Some("QueryDeveloperModeStatus"));
            let mut resp = plist::Dictionary::new();
            resp.insert("DeveloperModeStatus".into(), plist::Value::Boolean(true));
            ios_core::send_lockdown(&mut peer, &resp).await.unwrap();

            // 2. LookupImage Developer -> has signature
            let req: plist::Dictionary = ios_core::recv_lockdown(&mut peer).await.unwrap();
            assert_eq!(req["Command"].as_string(), Some("LookupImage"));
            let mut resp = plist::Dictionary::new();
            resp.insert(
                "ImageSignature".into(),
                plist::Value::Array(vec![plist::Value::Data(vec![1, 2, 3, 4])]),
            );
            ios_core::send_lockdown(&mut peer, &resp).await.unwrap();

            // 3. LookupImage Personalized -> empty
            let req: plist::Dictionary = ios_core::recv_lockdown(&mut peer).await.unwrap();
            assert_eq!(req["Command"].as_string(), Some("LookupImage"));
            let mut resp = plist::Dictionary::new();
            resp.insert("ImageSignature".into(), plist::Value::Array(vec![]));
            ios_core::send_lockdown(&mut peer, &resp).await.unwrap();
        };

        let client_work = async move {
            let result = preflight_device_readiness_with_client(&mut client, "test-udid-789").await;
            assert!(result.is_ok());
        };

        tokio::join!(server, client_work);
    }

    #[tokio::test]
    async fn review_regression_unified_workflow_preflight_failure_writes_diagnostic_and_retains_cause(
    ) {
        let directory =
            std::env::temp_dir().join(format!("workflow-diag-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&directory).unwrap();
        let junit_path = directory.join("junit.xml");

        let args = TestWorkflowArgs {
            install_app: Some(PathBuf::from("non_existent_app_for_preflight_test.ipa")),
            ..Default::default()
        };
        let deadline = Instant::now() + Duration::from_secs(10);
        let engine =
            TestWorkflowEngine::new("dummy-udid-123", &args, deadline, true, Some(&junit_path));
        let options = TestWorkflowRunOptions {
            startup_timeout: Duration::from_secs(5),
            result_timeout: Duration::from_secs(5),
            json_output: true,
            untested_banner: false,
            untested_receipt: false,
            monitored_bundle_ids: vec![],
        };

        let result = engine
            .run(options, |_device| {
                Box::pin(async move {
                    panic!("resolve_plan should not be reached when preflight fails");
                })
            })
            .await;

        assert!(result.is_err());
        let err_msg = result.unwrap_err().to_string();
        assert!(err_msg.contains("does not exist"));

        // Diagnostic JUnit file must have been generated
        assert!(junit_path.is_file());
        let xml_content = std::fs::read_to_string(&junit_path).unwrap();
        assert!(xml_content.contains("testsuite"));
        assert!(xml_content.contains("failure"));

        std::fs::remove_file(junit_path).unwrap();
        std::fs::remove_dir(directory).unwrap();
    }

    #[tokio::test]
    async fn review_regression_unified_workflow_print_receipt_formats_correctly() {
        let mut receipt = serde_json::json!({
            "status": "finished",
            "pid": 12345,
            "outcome": "failed",
        });
        // Non-JSON output format: "XCTest failed (pid 12345)"
        assert!(print_receipt(false, &receipt).is_ok());

        receipt["outcome"] = serde_json::json!("passed");
        assert!(print_receipt(false, &receipt).is_ok());

        // JSON output format
        assert!(print_receipt(true, &receipt).is_ok());
    }
}
