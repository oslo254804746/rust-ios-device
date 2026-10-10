//! Composable test execution workflow: application management, test runner execution,
//! syslog streaming capture, JUnit XML reporting, cancellation recovery, and failure reruns (F02).

use std::collections::HashMap;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::{Context, Result};
use ios_core::testmanager::results::{TestCaseStatus, TestRunSummary};
use tokio::io::AsyncWriteExt;
use tokio::sync::oneshot;
use tokio_stream::StreamExt;

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
            let device = ios_core::connect(&target_udid, connect_opts).await?;
            Ok(device
                .connect_service(ios_core::syslog::SERVICE_NAME)
                .await?)
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
    summary
        .suites
        .iter()
        .flat_map(|suite| suite.cases.iter())
        .filter(|case| matches!(case.status, Some(TestCaseStatus::Failed)))
        .map(|case| format!("{}/{}", case.class_name, case.method_name))
        .collect()
}

/// Merge rerun test results into the primary summary, replacing prior failures with new outcomes.
pub fn merge_rerun_summary(primary: &mut TestRunSummary, rerun: TestRunSummary) {
    let mut rerun_cases = HashMap::new();
    for suite in rerun.suites {
        for case in suite.cases {
            rerun_cases.insert((case.class_name.clone(), case.method_name.clone()), case);
        }
    }

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
    }
    primary.failed_tests = failed_count;
    primary.logs.extend(rerun.logs);
    primary.debug_logs.extend(rerun.debug_logs);
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
            let device = ios_core::connect(udid, connect_opts).await?;
            let stream = device
                .connect_service(ios_core::apps::INSTALLATION_PROXY_SERVICE)
                .await?;
            let mut proxy = ios_core::apps::InstallationProxy::new(stream);
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
        };
        tokio::time::timeout(Duration::from_secs(15), cleanup)
            .await
            .context("workflow uninstall cleanup deadline expired")?
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
    let device = ios_core::connect(udid, connect_opts)
        .await
        .context("connecting to device for test package installation")?;

    let lookup_stream = device
        .connect_service(ios_core::apps::INSTALLATION_PROXY_SERVICE)
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
    let mut stream = device
        .connect_service(svc_name)
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

#[cfg(test)]
mod tests {
    use super::*;
    use ios_core::testmanager::results::{TestCaseSummary, TestSuiteSummary};

    #[test]
    fn preflight_checks_non_existent_packages() {
        let args = TestWorkflowArgs {
            install_app: Some(PathBuf::from("non_existent_app_xyz.ipa")),
            install_runner: None,
            log_output: None,
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

        merge_rerun_summary(&mut primary, rerun);
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

        let guard = LogCaptureGuard::start("mock-udid", log_file.clone()).await;
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
}
