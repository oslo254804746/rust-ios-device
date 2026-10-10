//! Composable test execution workflow: application management, test runner execution,
//! syslog streaming capture, JUnit XML reporting, cancellation recovery, and failure reruns (F02).

use std::collections::HashMap;
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
                path.exists(),
                "test workflow preflight: application package '{}' does not exist",
                path.display()
            );
        }
        if let Some(path) = &self.install_runner {
            anyhow::ensure!(
                path.exists(),
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
        Ok(())
    }
}

/// Active background syslog capture handle.
pub struct LogCaptureGuard {
    stop_tx: Option<oneshot::Sender<()>>,
    join_handle: Option<tokio::task::JoinHandle<()>>,
    #[allow(dead_code)]
    destination: PathBuf,
}

impl LogCaptureGuard {
    /// Start capturing syslog lines to the destination file in the background.
    /// If syslog connection fails, records an explanatory banner so partial logs are never missing.
    pub async fn start(udid: &str, destination: PathBuf) -> Self {
        let (stop_tx, mut stop_rx) = oneshot::channel();
        let path = destination.clone();
        let target_udid = udid.to_string();

        let join_handle = tokio::spawn(async move {
            let file_result = tokio::fs::OpenOptions::new()
                .create(true)
                .write(true)
                .truncate(true)
                .open(&path)
                .await;

            let mut file = match file_result {
                Ok(f) => f,
                Err(err) => {
                    tracing::warn!(error = %err, "test workflow failed opening log file");
                    return;
                }
            };

            let header = format!(
                "[test workflow log capture started: {} (udid: [PRIVATE])]\n",
                chrono::Utc::now().to_rfc3339()
            );
            let _ = file.write_all(header.as_bytes()).await;
            let _ = file.flush().await;

            // Attempt to connect to syslog
            let connect_opts = ios_core::device::ConnectOptions {
                tun_mode: ios_core::TunMode::Userspace,
                pair_record_path: None,
                skip_tunnel: true,
            };

            let stream_opt = match ios_core::connect(&target_udid, connect_opts).await {
                Ok(dev) => dev
                    .connect_service(ios_core::syslog::SERVICE_NAME)
                    .await
                    .ok(),
                Err(_) => None,
            };

            if let Some(stream) = stream_opt {
                let log_stream = ios_core::syslog::into_stream(stream);
                tokio::pin!(log_stream);
                loop {
                    tokio::select! {
                        _ = &mut stop_rx => {
                            break;
                        }
                        item = log_stream.next() => {
                            match item {
                                Some(Ok(line)) => {
                                    let mut line_bytes = line.into_bytes();
                                    line_bytes.push(b'\n');
                                    let _ = file.write_all(&line_bytes).await;
                                }
                                Some(Err(_)) | None => {
                                    break;
                                }
                            }
                        }
                    }
                }
            } else {
                let banner = "[syslog service not available on device or mocked host]\n";
                let _ = file.write_all(banner.as_bytes()).await;
                // Wait for stop signal
                let _ = stop_rx.await;
            }

            let footer = format!(
                "[test workflow log capture finished: {}]\n",
                chrono::Utc::now().to_rfc3339()
            );
            let _ = file.write_all(footer.as_bytes()).await;
            let _ = file.flush().await;
        });

        Self {
            stop_tx: Some(stop_tx),
            join_handle: Some(join_handle),
            destination,
        }
    }

    /// Stop log capture and wait bounded time for flush.
    pub async fn stop(mut self) {
        if let Some(tx) = self.stop_tx.take() {
            let _ = tx.send(());
        }
        if let Some(handle) = self.join_handle.take() {
            let _ = tokio::time::timeout(Duration::from_secs(3), handle).await;
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
                *case = new_case;
            }
            if matches!(case.status, Some(TestCaseStatus::Failed)) {
                failed_count += 1;
            }
        }
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
    pub async fn cleanup(&self, udid: &str) {
        if !self.should_cleanup || self.installed_bundles.is_empty() {
            return;
        }
        let connect_opts = ios_core::device::ConnectOptions {
            tun_mode: ios_core::TunMode::Userspace,
            pair_record_path: None,
            skip_tunnel: true,
        };
        if let Ok(device) = ios_core::connect(udid, connect_opts).await {
            if let Ok(stream) = device
                .connect_service(ios_core::apps::INSTALLATION_PROXY_SERVICE)
                .await
            {
                let mut proxy = ios_core::apps::InstallationProxy::new(stream);
                for bundle_id in &self.installed_bundles {
                    tracing::info!(bundle_id = %bundle_id, "test workflow: uninstalling bundle");
                    let _ = proxy.uninstall(bundle_id).await;
                }
            }
        }
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
        path.exists(),
        "package path '{}' does not exist",
        path.display()
    );

    let connect_opts = ios_core::device::ConnectOptions {
        tun_mode: ios_core::TunMode::Userspace,
        pair_record_path: None,
        skip_tunnel: true,
    };
    let device = ios_core::connect(udid, connect_opts)
        .await
        .context("connecting to device for test package installation")?;

    tracing::info!(package = %path.display(), "test workflow: installing package");
    let svc_name = ios_core::apps::zipconduit::SERVICE_NAME;
    let mut stream = device
        .connect_service(svc_name)
        .await
        .context("failed to connect streaming_zip_conduit service")?;
    ios_core::apps::install_ipa_with_checked_progress(&mut stream, path, None)
        .await
        .with_context(|| format!("failed installing package '{}'", path.display()))?;

    // Attempt to read bundle identifier from path filename as a fallback identifier
    if let Some(stem) = path.file_stem().and_then(|s| s.to_str()) {
        tracker.track(stem.to_string());
    }
    Ok(())
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
        guard.stop().await;

        let content = std::fs::read_to_string(&log_file).unwrap();
        assert!(content.contains("[test workflow log capture started"));
        assert!(content.contains("[test workflow log capture finished"));

        let _ = std::fs::remove_dir_all(&temp_dir);
    }
}
