//! Direct XCTest runner execution.
//!
//! This is the bundle-ID form of the go-ios testmanagerd workflow. It does
//! not consume or generate an .xctestrun file: the runner and test bundle must
//! already be installed and signed for the device.

use std::future::Future;
use std::path::PathBuf;
use std::time::Duration;

use anyhow::{Context, Result};
use ios_core::testmanager::workflow::{RunXcTestPlan, RunXcTestPlanError, TestLaunchPlan};
use tokio::time::Instant;

use crate::cmd::runtest::{
    incomplete_summary, lookup_installed_app, start_test_plan_session_until,
    write_junit_diagnostic, write_junit_summary,
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
        builder
            .build()
            .map_err(|error: RunXcTestPlanError| anyhow::anyhow!("{error}"))
    }

    pub async fn run(self, udid: Option<String>, json_output: bool) -> Result<()> {
        if self.timeout_secs == 0 {
            return Err(anyhow::anyhow!("--timeout must be greater than zero"));
        }
        if self.junit_output.is_some() && !self.wait {
            return Err(anyhow::anyhow!(
                "--junit-output requires --wait because startup alone is not a complete XCTest report"
            ));
        }

        // Build and validate every user-controlled field before opening a
        // device connection. This also ensures an invalid selector cannot
        // result in a partially-started runner.
        let direct_plan = self.build_plan()?;
        let udid = udid.ok_or_else(|| anyhow::anyhow!("--udid required for runxctest"))?;
        let deadline = Instant::now()
            .checked_add(Duration::from_secs(self.timeout_secs))
            .ok_or_else(|| anyhow::anyhow!("--timeout is too large"))?;
        let junit_output = self.junit_output.as_deref();

        let device = match until(
            deadline,
            "connecting to testmanagerd",
            crate::cmd::runtest::connect_testmanager_device(&udid),
        )
        .await
        {
            Ok(device) => device,
            Err(error) => {
                write_junit_diagnostic(junit_output, &incomplete_summary(false), &error)?;
                return Err(error);
            }
        };
        let runner = match until(
            deadline,
            "looking up the XCTest runner",
            lookup_installed_app(&device, &direct_plan.test_runner_bundle_id),
        )
        .await
        {
            Ok(runner) => runner,
            Err(error) => {
                write_junit_diagnostic(junit_output, &incomplete_summary(false), &error)?;
                return Err(error);
            }
        };
        let target = match direct_plan.bundle_id.as_deref() {
            Some(bundle_id) => match until(
                deadline,
                "looking up the application under test",
                lookup_installed_app(&device, bundle_id),
            )
            .await
            {
                Ok(target) => Some(target),
                Err(error) => {
                    write_junit_diagnostic(junit_output, &incomplete_summary(false), &error)?;
                    return Err(error);
                }
            },
            None => None,
        };
        let launch_plan: TestLaunchPlan = direct_plan.into_test_launch_plan(runner, target);
        let configuration_name = launch_plan.xctest_bundle_name.clone();

        let mut session = match start_test_plan_session_until(&udid, launch_plan, deadline).await {
            Ok(session) => session,
            Err(error) => {
                write_junit_diagnostic(junit_output, &incomplete_summary(false), &error)?;
                return Err(error);
            }
        };
        let startup = session.startup_result().clone();

        if self.wait {
            let summary = match until(
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
                    write_junit_diagnostic(junit_output, &incomplete_summary(true), &error)?;
                    return Err(error);
                }
            };
            // XCTest normally exits after the finish event, but some runner
            // bundles keep their host process alive. Release the device-side
            // process explicitly so a completed direct run cannot leak a
            // runner into the next invocation; a race with normal process
            // exit is harmless and intentionally ignored by terminate().
            session.terminate().await;
            write_junit_summary(junit_output, &summary)?;
            print_result(
                json_output,
                serde_json::json!({
                    "status": "finished",
                    "configuration": configuration_name,
                    "runner_bundle_id": startup.runner_bundle_id,
                    "target_bundle_id": startup.target_bundle_id,
                    "pid": startup.pid,
                    "protocol_version": startup.protocol_version,
                    "minimum_version": startup.minimum_version,
                    "summary": summary,
                }),
            )?;
        } else {
            print_result(
                json_output,
                serde_json::json!({
                    "status": "started",
                    "configuration": configuration_name,
                    "runner_bundle_id": startup.runner_bundle_id,
                    "target_bundle_id": startup.target_bundle_id,
                    "pid": startup.pid,
                    "protocol_version": startup.protocol_version,
                    "minimum_version": startup.minimum_version,
                    "note": "The runner was started; use --wait to collect XCTest result events.",
                }),
            )?;
        }

        Ok(())
    }
}

fn print_result(json_output: bool, value: serde_json::Value) -> Result<()> {
    if json_output {
        println!("{}", serde_json::to_string_pretty(&value)?);
    } else if let Some(status) = value.get("status").and_then(serde_json::Value::as_str) {
        let pid = value
            .get("pid")
            .and_then(serde_json::Value::as_u64)
            .map(|pid| format!(" (pid {pid})"))
            .unwrap_or_default();
        println!("XCTest {status}{pid}");
    }
    Ok(())
}

async fn until<T, F>(deadline: Instant, operation: &str, future: F) -> Result<T>
where
    F: Future<Output = Result<T>>,
{
    tokio::time::timeout_at(deadline, future)
        .await
        .map_err(|_| anyhow::anyhow!("XCTest deadline expired while {operation}"))?
        .with_context(|| format!("XCTest {operation} failed"))
}

#[cfg(test)]
mod tests {
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
}
