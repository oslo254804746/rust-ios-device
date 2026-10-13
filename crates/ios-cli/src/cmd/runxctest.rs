//! Direct XCTest runner execution.
//!
//! This is the bundle-ID form of the go-ios testmanagerd workflow. It does
//! not consume or generate an .xctestrun file: the runner and test bundle must
//! already be installed and signed for the device.

use std::future::Future;
use std::path::PathBuf;
use std::time::Duration;

use anyhow::{Context, Result};
use ios_core::testmanager::workflow::{RunXcTestPlan, TestLaunchPlan};
use tokio::time::Instant;

use crate::cmd::runtest::{
    incomplete_summary, lookup_installed_app, start_test_plan_session_owned_until,
    write_junit_diagnostic, write_junit_summary, TestStartupCleanup,
};

#[derive(clap::Args)]
pub struct RunXcTestCmd {
    /// Installed application-under-test bundle identifier (omit for unit tests).
    #[arg(long = "bundle-id", value_name = "BUNDLE_ID")]
    pub bundle_id: Option<String>,
    /// Installed XCTest runner application bundle identifier.
    #[arg(long = "test-runner-bundle-id", value_name = "BUNDLE_ID")]
    pub test_runner_bundle_id: String,
    /// Test bundle inside the runner, for example ExampleTests.xctest.
    #[arg(long = "xctest-config", value_name = "BUNDLE.xctest")]
    pub xctest_config: String,
    /// Test selector to execute; may be repeated.
    #[arg(long = "test", alias = "test-to-run", value_name = "SELECTOR")]
    pub tests_to_run: Vec<String>,
    /// Test selector to skip; may be repeated.
    #[arg(long = "test-to-skip", value_name = "SELECTOR")]
    pub tests_to_skip: Vec<String>,
    /// Test class convenience selector (mutually exclusive with --test).
    #[arg(long, value_name = "CLASS")]
    pub class: Option<String>,
    /// Test method convenience selector; requires --class.
    #[arg(long, value_name = "METHOD")]
    pub method: Option<String>,
    /// Environment entry in KEY=VALUE form; may be repeated.
    #[arg(long = "env", value_name = "KEY=VALUE")]
    pub environment: Vec<String>,
    /// Additional argument passed to the runner; may be repeated.
    #[arg(long = "arg", value_name = "ARG", allow_hyphen_values = true)]
    pub args: Vec<String>,
    /// Treat the bundle as a unit-test bundle rather than a UI-test bundle.
    #[arg(long)]
    pub xctest: bool,
    /// Use a single absolute deadline for discovery, startup, and result wait.
    #[arg(
        long = "timeout",
        visible_alias = "timeout-secs",
        default_value_t = 300,
        value_name = "SECONDS"
    )]
    pub timeout_secs: u64,
    /// Wait for XCTest result events. Without this flag, report only startup.
    #[arg(long)]
    pub wait: bool,
    /// Write the completed result (or a diagnostic on failure) as JUnit XML.
    #[arg(long, value_name = "PATH")]
    pub junit_output: Option<PathBuf>,
    /// Composable test execution workflow options (installation, logging, cleanup, rerun).
    #[command(flatten)]
    pub workflow: crate::cmd::test_workflow::TestWorkflowArgs,
}

impl RunXcTestCmd {
    pub(crate) fn build_plan(&self) -> Result<RunXcTestPlan> {
        if !self.xctest && self.bundle_id.is_none() {
            return Err(anyhow::anyhow!(
                "--bundle-id is required for UI tests; use --xctest for a unit-test bundle"
            ));
        }
        let mut builder = RunXcTestPlan::builder(&self.test_runner_bundle_id, &self.xctest_config)
            .xctest(self.xctest);
        if let Some(bundle_id) = &self.bundle_id {
            builder = builder.bundle_id(bundle_id);
        }
        if let Some(class) = &self.class {
            builder = builder.class(class);
        }
        if let Some(method) = &self.method {
            builder = builder.method(method);
        }
        for selector in &self.tests_to_run {
            builder = builder.test(selector);
        }
        for selector in &self.tests_to_skip {
            builder = builder.skip(selector);
        }
        for argument in &self.args {
            builder = builder.arg(argument);
        }
        for entry in &self.environment {
            let (key, value) = entry
                .split_once('=')
                .ok_or_else(|| anyhow::anyhow!("--env expects KEY=VALUE, got {entry:?}"))?;
            builder = builder.env(key, value);
        }
        builder.build().map_err(anyhow::Error::new)
    }

    pub async fn run(self, udid: Option<String>, json_output: bool) -> Result<()> {
        if self.timeout_secs == 0 {
            return Err(crate::output::input_decision(
                "--timeout must be greater than zero",
            ));
        }
        if self.junit_output.is_some() && !self.wait {
            return Err(anyhow::anyhow!(
                "--junit-output requires --wait because startup alone is not a complete XCTest report"
            ));
        }

        self.workflow.preflight()?;
        anyhow::ensure!(
            self.wait
                || (!self.workflow.uninstall_on_finish
                    && self.workflow.rerun_failed == 0
                    && self.workflow.crash_output.is_none()),
            "workflow cleanup and reruns require --wait"
        );
        if let Some(path) = &self.workflow.install_runner {
            anyhow::ensure!(
                crate::cmd::test_workflow::package_bundle_id(path)? == self.test_runner_bundle_id,
                "installed runner package identifier does not match the selected test runner"
            );
        }
        if let Some(path) = &self.workflow.install_app {
            anyhow::ensure!(
                Some(crate::cmd::test_workflow::package_bundle_id(path)?.as_str())
                    == self.bundle_id.as_deref(),
                "installed application package identifier does not match the selected target"
            );
        }

        // Build and validate every user-controlled field before opening a
        // device connection. This also ensures an invalid selector cannot
        // result in a partially-started runner.
        let direct_plan = self.build_plan()?;
        let udid = udid.ok_or_else(|| anyhow::anyhow!("--udid required for runxctest"))?;
        let deadline = Instant::now()
            .checked_add(Duration::from_secs(self.timeout_secs))
            .ok_or_else(|| crate::output::input_decision("--timeout is too large"))?;
        let junit_output = self.junit_output.as_deref();

        let mut app_tracker =
            crate::cmd::test_workflow::AppLifecycleTracker::new(self.workflow.uninstall_on_finish);
        let log_guard = if let Some(log_path) = self.workflow.log_output.clone() {
            Some(crate::cmd::test_workflow::LogCaptureGuard::start(&udid, log_path).await)
        } else {
            None
        };

        let native = crate::cmd::tunnel::NativeScope::new();
        let mut crashes = self.workflow.crash_output.as_deref().map(|root| {
            crate::cmd::test_workflow::crash::CrashCapture::new(
                root,
                std::iter::once(direct_plan.test_runner_bundle_id.clone())
                    .chain(direct_plan.bundle_id.clone()),
            )
        });
        let mut workflow_sessions = Vec::new();
        let mut startup_scopes = Vec::<TestStartupCleanup>::new();
        let mut attempts = crate::cmd::test_workflow::AttemptHistory::default();
        let mut latest_summary = None;
        let mut pending_startup = None;
        let mut pending_finished = None;
        let work = Box::pin(async {
            if let Some(crashes) = &mut crashes {
                crashes.baseline(&udid, deadline, &native).await?;
            }
            if let Err(error) = tokio::time::timeout_at(
                deadline,
                crate::cmd::test_workflow::install_package_if_present(
                    &udid,
                    self.workflow.install_app.as_deref(),
                    &mut app_tracker,
                ),
            )
            .await
            .context("XCTest deadline expired during package installation")?
            {
                let report_result =
                    write_junit_diagnostic(junit_output, &incomplete_summary(false), &error);
                if let Err(report_error) = report_result {
                    return crate::cmd::test_workflow::combine_results(
                        Err(error),
                        Err(report_error),
                        "workflow diagnostic report failed",
                    );
                }
                return Err(error);
            }

            if let Err(error) = tokio::time::timeout_at(
                deadline,
                crate::cmd::test_workflow::install_package_if_present(
                    &udid,
                    self.workflow.install_runner.as_deref(),
                    &mut app_tracker,
                ),
            )
            .await
            .context("XCTest deadline expired during package installation")?
            {
                let report_result =
                    write_junit_diagnostic(junit_output, &incomplete_summary(false), &error);
                if let Err(report_error) = report_result {
                    return crate::cmd::test_workflow::combine_results(
                        Err(error),
                        Err(report_error),
                        "workflow diagnostic report failed",
                    );
                }
                return Err(error);
            }

            let device = match until(
                deadline,
                "connecting to testmanagerd",
                Box::pin(crate::cmd::runtest::connect_testmanager_device_scoped(
                    &udid, &native,
                )),
            )
            .await
            {
                Ok(device) => device,
                Err(error) => {
                    let report_result =
                        write_junit_diagnostic(junit_output, &incomplete_summary(false), &error);
                    if let Err(report_error) = report_result {
                        return crate::cmd::test_workflow::combine_results(
                            Err(error),
                            Err(report_error),
                            "workflow diagnostic report failed",
                        );
                    }
                    return Err(error);
                }
            };
            let runner = match until(
                deadline,
                "looking up the XCTest runner",
                Box::pin(lookup_installed_app(
                    &device,
                    &direct_plan.test_runner_bundle_id,
                )),
            )
            .await
            {
                Ok(runner) => runner,
                Err(error) => {
                    let report_result =
                        write_junit_diagnostic(junit_output, &incomplete_summary(false), &error);
                    if let Err(report_error) = report_result {
                        return crate::cmd::test_workflow::combine_results(
                            Err(error),
                            Err(report_error),
                            "workflow diagnostic report failed",
                        );
                    }
                    return Err(error);
                }
            };
            let target = match direct_plan.bundle_id.as_deref() {
                Some(bundle_id) => match until(
                    deadline,
                    "looking up the application under test",
                    Box::pin(lookup_installed_app(&device, bundle_id)),
                )
                .await
                {
                    Ok(target) => Some(target),
                    Err(error) => {
                        let report_result = write_junit_diagnostic(
                            junit_output,
                            &incomplete_summary(false),
                            &error,
                        );
                        if let Err(report_error) = report_result {
                            return crate::cmd::test_workflow::combine_results(
                                Err(error),
                                Err(report_error),
                                "workflow diagnostic report failed",
                            );
                        }
                        return Err(error);
                    }
                },
                None => None,
            };
            let launch_plan: TestLaunchPlan = direct_plan.into_test_launch_plan(runner, target);
            let configuration_name = launch_plan.xctest_bundle_name.clone();

            attempts.start(launch_plan.tests_to_run.clone());
            startup_scopes.push(TestStartupCleanup::default());
            let session = match Box::pin(start_test_plan_session_owned_until(
                &udid,
                launch_plan.clone(),
                deadline,
                startup_scopes.last_mut().expect("startup owner admitted"),
                &native,
            ))
            .await
            {
                Ok(session) => session,
                Err(error) => {
                    let report_result =
                        write_junit_diagnostic(junit_output, &incomplete_summary(false), &error);
                    if let Err(report_error) = report_result {
                        return crate::cmd::test_workflow::combine_results(
                            Err(error),
                            Err(report_error),
                            "workflow diagnostic report failed",
                        );
                    }
                    return Err(error);
                }
            };
            workflow_sessions.push(session);
            let session = workflow_sessions
                .last_mut()
                .expect("stored active XCTest session");
            let startup = session.startup_result().clone();

            if self.wait {
                let mut summary = match until(
                    deadline,
                    "waiting for XCTest results",
                    session.wait_for_results(),
                )
                .await
                {
                    Ok(summary) => summary,
                    Err(error) => {
                        // Dropping the active DTX connections stops result
                        // delivery. The process is killed by the bounded cleanup
                        // in ActiveTestPlan when a deadline/cancellation occurs.
                        session.terminate().await;
                        let report_result =
                            write_junit_diagnostic(junit_output, &incomplete_summary(true), &error);
                        if let Err(report_error) = report_result {
                            return crate::cmd::test_workflow::combine_results(
                                Err(error),
                                Err(report_error),
                                "workflow diagnostic report failed",
                            );
                        }
                        return Err(error);
                    }
                };
                pending_finished = Some(serde_json::json!({
                    "status": "finished",
                    "configuration": configuration_name,
                    "runner_bundle_id": startup.runner_bundle_id,
                    "target_bundle_id": startup.target_bundle_id,
                    "pid": startup.pid,
                    "protocol_version": startup.protocol_version,
                    "minimum_version": startup.minimum_version,
                }));
                latest_summary = Some(summary.clone());
                attempts.complete(&summary)?;
                // XCTest normally exits after the finish event, but some runner
                // bundles keep their host process alive. Release the device-side
                // process explicitly so a completed direct run cannot leak a
                // runner into the next invocation; a race with normal process
                // exit is harmless and intentionally ignored by terminate().
                session.terminate().await;
                if let Some(error) = session.take_termination_error() {
                    return Err(error);
                }

                summary.terminal_outcome()?;
                // Rerun failed tests if requested
                if summary.failure_count() > 0 && self.workflow.rerun_failed > 0 {
                    for rerun_idx in 0..self.workflow.rerun_failed {
                        let failed_selectors =
                            crate::cmd::test_workflow::extract_failed_selectors(&summary);
                        if failed_selectors.is_empty() {
                            break;
                        }
                        tracing::info!(
                            attempt = rerun_idx + 1,
                            failed_count = failed_selectors.len(),
                            "test workflow: rerunning failed test selectors"
                        );
                        let mut rerun_plan = launch_plan.clone();
                        attempts.start(failed_selectors.clone());
                        rerun_plan.tests_to_run = failed_selectors;
                        rerun_plan.tests_to_skip.clear();

                        let rerun_session =
                            match until(deadline, "restarting test runner for failed selectors", {
                                startup_scopes.push(TestStartupCleanup::default());
                                Box::pin(start_test_plan_session_owned_until(
                                    &udid,
                                    rerun_plan,
                                    deadline,
                                    startup_scopes
                                        .last_mut()
                                        .expect("rerun startup owner admitted"),
                                    &native,
                                ))
                            })
                            .await
                            {
                                Ok(s) => s,
                                Err(error) => return Err(error),
                            };

                        workflow_sessions.push(rerun_session);
                        let rerun_session = workflow_sessions
                            .last_mut()
                            .expect("stored active rerun session");
                        let rerun_res = until(
                            deadline,
                            "waiting for rerun results",
                            rerun_session.wait_for_results(),
                        )
                        .await;
                        rerun_session.terminate().await;
                        let rerun_summary = rerun_res?;
                        attempts.complete(&rerun_summary)?;
                        if let Some(error) = rerun_session.take_termination_error() {
                            return Err(error);
                        }
                        {
                            crate::cmd::test_workflow::merge_rerun_summary(
                                &mut summary,
                                rerun_summary,
                            )?;
                            latest_summary = Some(summary.clone());
                            if summary.failure_count() == 0 {
                                break;
                            }
                        }
                    }
                }

                write_junit_summary(junit_output, &summary)?;

                if summary.failure_count() > 0 {
                    return Err(anyhow::anyhow!(
                        "XCTest run completed with {} failed test(s)",
                        summary.failure_count()
                    ));
                }
            } else {
                pending_startup = Some(serde_json::json!({
                    "status": "started",
                    "configuration": configuration_name,
                    "runner_bundle_id": startup.runner_bundle_id,
                    "target_bundle_id": startup.target_bundle_id,
                    "pid": startup.pid,
                    "protocol_version": startup.protocol_version,
                    "minimum_version": startup.minimum_version,
                    "post_disconnect_lifetime": "unknown",
                    "note": "Startup was acknowledged. Service connections are closed before this receipt; continued runner or test execution after disconnect was not observed. Use --wait to collect result events in this invocation.",
                }));
            }

            Ok(())
        });
        let mut outcome = tokio::select! {
            result = tokio::time::timeout_at(deadline, work) => result
                .map_err(|error| anyhow::Error::new(error).context("XCTest workflow deadline expired")).and_then(|result| result),
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
            let reported = write_junit_diagnostic(junit_output, &diagnostic, error);
            outcome = crate::cmd::test_workflow::combine_results(
                outcome,
                recorded,
                "workflow attempt history failed",
            );
            outcome = crate::cmd::test_workflow::combine_results(
                outcome,
                reported,
                "workflow diagnostic report failed",
            );
        }
        for startup in &mut startup_scopes {
            outcome = crate::cmd::test_workflow::combine_results(
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
                outcome = crate::cmd::test_workflow::combine_results(
                    outcome,
                    Err(error),
                    "workflow runner cleanup failed",
                );
            }
        }
        let mut outcome =
            crate::cmd::test_workflow::finish_workflow(&udid, &app_tracker, log_guard, outcome)
                .await;
        if let Some(crashes) = &mut crashes {
            outcome = crate::cmd::test_workflow::combine_results(
                outcome,
                crashes.finish(&udid, &native).await,
                "workflow crash collection failed",
            );
        }
        outcome = crate::cmd::test_workflow::combine_results(
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
            crate::cmd::test_workflow::combine_results(
                record,
                write_junit_diagnostic(junit_output, &diagnostic, error),
                "workflow final diagnostic failed",
            )
        } else {
            Ok(())
        };
        let mut outcome = crate::cmd::test_workflow::combine_results(
            outcome,
            recorded,
            "workflow terminal history failed",
        );
        if let Err(error) = &outcome {
            let recorded = attempts.fail(error, None);
            outcome = crate::cmd::test_workflow::combine_results(
                outcome,
                recorded,
                "workflow final history classification failed",
            );
        }
        let artifact_deadline = Instant::now() + Duration::from_secs(5);
        let artifact = attempts.write_until(junit_output, artifact_deadline).await;
        let artifact_failed = artifact.is_err();
        outcome = crate::cmd::test_workflow::combine_results(
            outcome,
            artifact,
            "workflow attempt artifact failed",
        );
        if artifact_failed {
            let diagnostic = latest_summary
                .as_ref()
                .cloned()
                .unwrap_or_else(|| incomplete_summary(false));
            let reported = write_junit_diagnostic(
                junit_output,
                &diagnostic,
                outcome.as_ref().expect_err("failed attempt artifact"),
            );
            outcome = crate::cmd::test_workflow::combine_results(
                outcome,
                reported,
                "workflow artifact diagnostic failed",
            );
        }
        if let Err(error) = &outcome {
            let recorded = attempts.fail(error, None);
            outcome = crate::cmd::test_workflow::combine_results(
                outcome,
                recorded,
                "workflow artifact failure classification failed",
            );
        }
        if let Some(mut receipt) = pending_finished {
            let output = (|| {
                crate::cmd::test_workflow::finish_receipt(
                    &mut receipt,
                    latest_summary.as_ref().expect("observed finished summary"),
                    &attempts,
                    &outcome,
                )?;
                print_result(json_output, receipt)
            })();
            return crate::cmd::test_workflow::finish_output(
                outcome,
                output,
                &mut attempts,
                junit_output,
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
            return crate::cmd::test_workflow::combine_results(
                outcome,
                print_result(json_output, receipt),
                "XCTest startup receipt output failed",
            );
        }
        outcome
    }
}

fn print_result(json_output: bool, value: serde_json::Value) -> Result<()> {
    if json_output {
        crate::output::outln!("{}", serde_json::to_string_pretty(&value)?);
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

fn until<'a, T: 'a, F>(
    deadline: Instant,
    operation: &'a str,
    future: F,
) -> impl Future<Output = Result<T>> + 'a
where
    F: Future<Output = Result<T>> + 'a,
{
    let future = Box::pin(future);
    async move {
        tokio::time::timeout_at(deadline, future)
            .await
            .map_err(|error| {
                anyhow::Error::new(error)
                    .context(format!("XCTest deadline expired while {operation}"))
            })?
            .with_context(|| format!("XCTest {operation} failed"))
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn workflow_future_retains_only_bounded_outer_state_before_first_poll() {
        let command = TestCli::parse_from([
            "runxctest",
            "--xctest",
            "--test-runner-bundle-id",
            "com.example.Runner",
            "--xctest-config",
            "DemoTests.xctest",
            "--wait",
        ])
        .command;
        let future = command.run(Some("owned-size-fixture".into()), true);
        assert!(
            std::mem::size_of_val(&future) <= 256 * 1024,
            "the public future embeds large startup/service futures"
        );
    }

    #[tokio::test]
    async fn owner_guards_xctest_invalid_timeout_precedes_connection() {
        for timeout in ["0", "18446744073709551615"] {
            let command = TestCli::parse_from([
                "runxctest",
                "--xctest",
                "--test-runner-bundle-id",
                "com.example.Runner",
                "--xctest-config",
                "DemoTests.xctest",
                "--timeout",
                timeout,
            ])
            .command;
            let error = command
                .run(Some("owned-private-device".to_owned()), true)
                .await
                .unwrap_err();
            assert_eq!(
                ios_core::classify_error(error.as_ref()),
                ios_core::ErrorClass::InvalidInput
            );
            let io = error.downcast_ref::<std::io::Error>().unwrap();
            assert!(io.raw_os_error().is_none());
            assert!(io.get_ref().unwrap().source().is_none());
        }
    }

    #[test]
    fn cli_native_sources_xctest_plan_keeps_original_typed_input_error() {
        let mut command = TestCli::parse_from([
            "runxctest",
            "--xctest",
            "--test-runner-bundle-id",
            "com.example.Runner",
            "--xctest-config",
            "DemoTests.xctest",
        ])
        .command;
        command.test_runner_bundle_id.clear();
        let error = command.build_plan().unwrap_err();
        assert!(matches!(
            error.downcast_ref::<ios_core::RunXcTestPlanError>(),
            Some(ios_core::RunXcTestPlanError::MissingRunnerBundleId)
        ));
        assert_eq!(
            ios_core::classify_error(error.as_ref()),
            ios_core::ErrorClass::InvalidInput
        );
    }
    #[tokio::test]
    async fn cli_native_sources_xctest_deadline_retains_actual_elapsed() {
        let error = super::until(
            tokio::time::Instant::now(),
            "owned-private-operation",
            std::future::pending::<anyhow::Result<()>>(),
        )
        .await
        .unwrap_err();
        assert_eq!(
            ios_core::classify_error(error.as_ref()),
            ios_core::ErrorClass::Timeout
        );
        assert!(error
            .chain()
            .any(|source| source.is::<tokio::time::error::Elapsed>()));
    }
    use std::path::PathBuf;
    use std::time::Duration;

    use clap::Parser;
    use tokio::time::Instant;

    use super::{until, RunXcTestCmd};

    #[derive(Parser)]
    struct TestCli {
        #[command(flatten)]
        command: RunXcTestCmd,
    }

    #[test]
    fn parses_direct_runner_options_and_aliases() {
        let parsed = TestCli::parse_from([
            "runxctest",
            "--bundle-id",
            "com.example.App",
            "--test-runner-bundle-id",
            "com.example.Runner",
            "--xctest-config",
            "DemoTests.xctest",
            "--test-to-run",
            "LoginTests/testHappyPath",
            "--env",
            "LANG=zh=CN",
            "--arg",
            "--verbose",
            "--wait",
            "--timeout",
            "42",
            "--junit-output",
            "results.xml",
        ]);
        assert_eq!(parsed.command.bundle_id.as_deref(), Some("com.example.App"));
        assert_eq!(parsed.command.test_runner_bundle_id, "com.example.Runner");
        assert_eq!(parsed.command.xctest_config, "DemoTests.xctest");
        assert_eq!(parsed.command.tests_to_run, ["LoginTests/testHappyPath"]);
        assert_eq!(parsed.command.environment, ["LANG=zh=CN"]);
        assert_eq!(parsed.command.args, ["--verbose"]);
        assert!(parsed.command.wait);
        assert_eq!(parsed.command.timeout_secs, 42);
        assert_eq!(
            parsed.command.junit_output,
            Some(PathBuf::from("results.xml"))
        );

        let class_command = TestCli::parse_from([
            "runxctest",
            "--test-runner-bundle-id",
            "com.example.Runner",
            "--xctest-config",
            "DemoTests.xctest",
            "--class",
            "OtherTests",
            "--method",
            "testUnicode",
        ]);
        assert_eq!(class_command.command.class.as_deref(), Some("OtherTests"));
        assert_eq!(class_command.command.method.as_deref(), Some("testUnicode"));
    }

    #[test]
    fn validates_selection_and_environment_before_device_access() {
        let mut cmd = TestCli::parse_from([
            "runxctest",
            "--bundle-id",
            "com.example.App",
            "--test-runner-bundle-id",
            "com.example.Runner",
            "--xctest-config",
            "DemoTests.xctest",
            "--method",
            "testOnly",
        ])
        .command;
        assert!(cmd
            .build_plan()
            .unwrap_err()
            .to_string()
            .contains("--method"));

        cmd = TestCli::parse_from([
            "runxctest",
            "--bundle-id",
            "com.example.App",
            "--test-runner-bundle-id",
            "com.example.Runner",
            "--xctest-config",
            "DemoTests.xctest",
            "--env",
            "MALFORMED",
        ])
        .command;
        assert!(cmd
            .build_plan()
            .unwrap_err()
            .to_string()
            .contains("KEY=VALUE"));

        cmd = TestCli::parse_from([
            "runxctest",
            "--test-runner-bundle-id",
            "com.example.Runner",
            "--xctest-config",
            "DemoTests.xctest",
        ])
        .command;
        assert!(cmd
            .build_plan()
            .unwrap_err()
            .to_string()
            .contains("--bundle-id"));
    }

    #[test]
    fn invalid_direct_configuration_is_rejected() {
        let cmd = TestCli::try_parse_from([
            "runxctest",
            "--test-runner-bundle-id",
            "com.example.Runner",
            "--xctest-config",
            "../DemoTests.xctest",
        ])
        .unwrap()
        .command;
        assert!(cmd.build_plan().is_err());
    }

    #[tokio::test]
    async fn until_uses_one_absolute_deadline() {
        let deadline = Instant::now() + Duration::from_millis(1);
        let error = until(deadline, "a fake stalled handshake", async {
            tokio::time::sleep(Duration::from_secs(1)).await;
            Ok::<_, anyhow::Error>(())
        })
        .await
        .unwrap_err();
        assert!(error.to_string().contains("deadline expired"));
    }

    #[test]
    fn parses_workflow_options_in_runxctest() {
        let cmd = TestCli::parse_from([
            "runxctest",
            "--xctest",
            "--test-runner-bundle-id",
            "com.example.Runner",
            "--xctest-config",
            "DemoTests.xctest",
            "--install-app",
            "app.ipa",
            "--install-runner",
            "runner.ipa",
            "--log-output",
            "device.log",
            "--uninstall-on-finish",
            "--rerun-failed",
            "2",
        ])
        .command;
        assert_eq!(cmd.workflow.install_app, Some(PathBuf::from("app.ipa")));
        assert_eq!(
            cmd.workflow.install_runner,
            Some(PathBuf::from("runner.ipa"))
        );
        assert_eq!(cmd.workflow.log_output, Some(PathBuf::from("device.log")));
        assert!(cmd.workflow.uninstall_on_finish);
        assert_eq!(cmd.workflow.rerun_failed, 2);
    }

    #[tokio::test]
    async fn preflight_rejects_missing_package_in_runxctest() {
        let cmd = TestCli::parse_from([
            "runxctest",
            "--xctest",
            "--test-runner-bundle-id",
            "com.example.Runner",
            "--xctest-config",
            "DemoTests.xctest",
            "--install-app",
            "nonexistent_package_for_test.ipa",
        ])
        .command;
        let err = cmd
            .run(Some("test-device".to_string()), true)
            .await
            .unwrap_err();
        assert!(err.to_string().contains("does not exist"));
    }
}
