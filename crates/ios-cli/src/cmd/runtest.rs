use std::collections::HashMap;
use std::future::Future;
use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::{Context, Result};
use ios_core::afc::house_arrest::HouseArrestClient;
use ios_core::apps::{AppInfo, AppServiceClient, InstallationProxy, LaunchApplicationOptions};
use ios_core::archive_xctest_configuration;
use ios_core::device::{ConnectedDevice, ServiceStream};
use ios_core::instruments::process_control::ProcessControl;
use ios_core::testmanager::results::{
    write_junit_xml_atomic, write_junit_xml_atomic_with_diagnostic, TestRunRecorder, TestRunSummary,
};
use ios_core::testmanager::workflow::{InstalledAppInfo, TestLaunchPlan};
use ios_core::testmanager::xctestrun::{parse_xctestrun_file, TestConfiguration};
use ios_core::testmanager::TestmanagerClient;
use ios_core::MuxClient;
use ios_core::XctCapabilities;
use tokio::time::Instant;
use uuid::Uuid;

#[cfg(test)]
mod startup_tests;

use crate::cmd::connect::connect_lockdown_only;
pub(crate) use crate::cmd::connect::CommandDevice as OwnedTestDevice;

#[derive(clap::Args)]
pub struct RunTestCmd {
    #[arg(help = "Path to the .xctestrun file")]
    pub xctestrun: PathBuf,
    #[arg(
        long,
        help = "Select a named TestConfigurations entry from format v2 .xctestrun files"
    )]
    pub configuration: Option<String>,
    #[arg(
        long,
        help = "Select a TestTargets entry by runner bundle id or test bundle name"
    )]
    pub test_target: Option<String>,
    #[arg(long, default_value_t = 30, help = "Startup timeout in seconds")]
    pub startup_timeout_secs: u64,
    #[arg(long, help = "Wait for XCTest result events after startup")]
    pub wait: bool,
    #[arg(long, default_value_t = 300, help = "Result wait timeout in seconds")]
    pub result_timeout_secs: u64,
    #[arg(
        long,
        value_name = "PATH",
        help = "Write the completed XCTest result as atomically-written JUnit XML (requires --wait)"
    )]
    pub junit_output: Option<PathBuf>,
    #[command(flatten)]
    pub workflow: crate::cmd::test_workflow::TestWorkflowArgs,
}

impl RunTestCmd {
    pub async fn run(self, udid: Option<String>) -> Result<()> {
        let udid = udid.ok_or_else(|| anyhow::anyhow!("--udid required for runtest"))?;
        if self.junit_output.is_some() && !self.wait {
            return Err(anyhow::anyhow!(
                "--junit-output requires --wait because an early XCTest startup result is not a complete test report"
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
        let validated_configs = parse_xctestrun_file(&self.xctestrun)
            .with_context(|| format!("failed to parse {}", self.xctestrun.display()))?;
        let config = select_configuration(&validated_configs, self.configuration.as_deref())
            .context("selected test configuration does not exist")?;
        let scheme = select_test_target(config, self.test_target.as_deref())
            .context("selected test target does not exist")?;
        if let Some(path) = &self.workflow.install_runner {
            anyhow::ensure!(
                crate::cmd::test_workflow::package_bundle_id(path)?
                    == scheme.test_host_bundle_identifier,
                "installed runner package identifier does not match xctestrun selection"
            );
        }
        if let Some(path) = &self.workflow.install_app {
            let id = crate::cmd::test_workflow::package_bundle_id(path)?;
            anyhow::ensure!(infer_target_bundle_id(scheme, None).as_deref() == Some(id.as_str()),
                "installation requires an explicit matching UITargetAppBundleIdentifier in xctestrun");
        }
        anyhow::ensure!(
            self.startup_timeout_secs > 0 && (!self.wait || self.result_timeout_secs > 0),
            "XCTest startup and result deadlines must be greater than zero"
        );
        let deadline = Instant::now()
            .checked_add(Duration::from_secs(
                self.startup_timeout_secs
                    .checked_add(self.result_timeout_secs)
                    .ok_or_else(|| crate::output::input_decision("XCTest timeout is too large"))?,
            ))
            .ok_or_else(|| crate::output::input_decision("XCTest timeout is too large"))?;
        let monitored_bundle_ids = std::iter::once(scheme.test_host_bundle_identifier.clone())
            .chain(infer_target_bundle_id(scheme, None))
            .collect();

        let engine = crate::cmd::test_workflow::TestWorkflowEngine::new(
            &udid,
            &self.workflow,
            deadline,
            self.wait,
            self.junit_output.as_deref(),
        );

        let options = crate::cmd::test_workflow::TestWorkflowRunOptions {
            startup_timeout: Duration::from_secs(self.startup_timeout_secs),
            result_timeout: Duration::from_secs(self.result_timeout_secs),
            json_output: true,
            untested_banner: true,
            untested_receipt: true,
            monitored_bundle_ids,
        };

        engine
            .run(options, |device| {
                Box::pin(async move {
                    let configs = parse_xctestrun_file(&self.xctestrun)
                        .with_context(|| format!("failed to parse {}", self.xctestrun.display()))?;
                    build_plan_from_xctestrun(
                        device,
                        &self.xctestrun,
                        &configs,
                        self.configuration.as_deref(),
                        self.test_target.as_deref(),
                    )
                    .await
                })
            })
            .await
    }
}

pub(crate) fn incomplete_summary(began: bool) -> TestRunSummary {
    TestRunSummary {
        began,
        finished: false,
        total_tests: 0,
        failed_tests: 0,
        skipped_tests: 0,
        logs: Vec::new(),
        debug_logs: Vec::new(),
        suites: Vec::new(),
        logs_truncated: false,
        results_truncated: false,
    }
}

pub(crate) fn write_junit_summary(path: Option<&Path>, summary: &TestRunSummary) -> Result<()> {
    if let Some(path) = path {
        write_junit_xml_atomic(summary, path)
            .with_context(|| format!("failed writing JUnit XML to {}", path.display()))?;
    }
    Ok(())
}

pub(crate) fn write_junit_diagnostic(
    path: Option<&Path>,
    summary: &TestRunSummary,
    error: &anyhow::Error,
) -> Result<()> {
    if let Some(path) = path {
        write_junit_xml_atomic_with_diagnostic(summary, path, &crate::output::error_message(error))
            .with_context(|| {
                format!("failed writing diagnostic JUnit XML to {}", path.display())
            })?;
    }
    Ok(())
}

#[derive(Debug, Clone)]
pub struct TestStartupResult {
    pub runner_bundle_id: String,
    pub target_bundle_id: Option<String>,
    pub pid: u64,
    pub protocol_version: u64,
    pub minimum_version: u64,
}

pub struct ActiveTestPlan {
    startup_result: TestStartupResult,
    testmanager_device: Option<OwnedTestDevice>,
    _testmanager: TestmanagerClient<ServiceStream>,
    _instruments_device: Option<OwnedTestDevice>,
    _runner_controller: RunnerController,
    result_recorder: TestRunRecorder,
    termination_attempted: bool,
    kill_attempted: bool,
    termination_error: Option<anyhow::Error>,
    native: crate::cmd::tunnel::NativeScope,
}

enum RunnerController {
    Instruments(Box<ProcessControl<ServiceStream>>),
    CoreDevice(Box<AppServiceClient>),
}

#[derive(Default)]
pub(crate) struct TestStartupCleanup {
    testmanager: Option<TestmanagerClient<ServiceStream>>,
    controller: Option<RunnerController>,
    pid: Option<u64>,
    device: Option<OwnedTestDevice>,
    instruments_device: Option<OwnedTestDevice>,
    pending_runner: Option<(String, Option<Vec<u64>>)>,
    launch_submitted: bool,
}
impl TestStartupCleanup {
    pub(crate) async fn close(&mut self) -> Result<()> {
        let mut result = Ok(());
        let interrupted = self.pending_runner.is_some()
            && matches!(&self.controller,
            Some(RunnerController::CoreDevice(controller)) if !controller.is_usable());
        if self.pid.is_none() && (self.launch_submitted || interrupted) {
            if let (
                Some(RunnerController::CoreDevice(controller)),
                Some((bundle, Some(previous))),
            ) = (&mut self.controller, self.pending_runner.take())
            {
                result = tokio::time::timeout(Duration::from_secs(2), async {
                    controller.reconnect().await?;
                    let processes = controller.list_processes().await?;
                    let candidates: Vec<_> = processes
                        .iter()
                        .filter(|process| {
                            process.bundle_id.as_deref() == Some(bundle.as_str())
                                && !previous.contains(&process.pid)
                        })
                        .collect();
                    if candidates.len() != 1 {
                        return Err(anyhow::Error::new(RunnerLaunchUnconfirmed));
                    }
                    controller.kill_process(candidates[0].pid).await?;
                    Ok(())
                })
                .await
                .map_err(anyhow::Error::new)
                .and_then(|result| result)
                .context(RunnerLaunchUnconfirmed);
            } else {
                result = Err(anyhow::Error::new(RunnerLaunchUnconfirmed));
            }
        }
        if let (Some(controller), Some(pid)) = (&mut self.controller, self.pid.take()) {
            result = terminate_process(controller, pid).await;
        }
        if let Some(client) = &mut self.testmanager {
            result = crate::cmd::test_workflow::combine_results(
                result,
                client
                    .close(Duration::from_secs(2))
                    .await
                    .map_err(anyhow::Error::new),
                "XCTest startup testmanager close failed",
            );
        }
        if let Some(RunnerController::Instruments(controller)) = &mut self.controller {
            result = crate::cmd::test_workflow::combine_results(
                result,
                controller
                    .close(Duration::from_secs(2))
                    .await
                    .map_err(anyhow::Error::new),
                "XCTest startup process-control close failed",
            );
        }
        self.testmanager.take();
        self.controller.take();
        self.instruments_device.take();
        self.device.take();
        self.pending_runner = None;
        self.launch_submitted = false;
        result
    }
}

#[derive(Debug)]
pub struct RunnerLaunchUnconfirmed;
impl std::fmt::Display for RunnerLaunchUnconfirmed {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("XCTest launch was submitted but its PID could not be reconciled; a runner may remain active on the selected device")
    }
}
impl std::error::Error for RunnerLaunchUnconfirmed {}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum TestmanagerTransport {
    RemoteServiceDiscovery,
    LockdownSecure,
    LockdownLegacy,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct TestmanagerConnectPlan {
    pub transport: TestmanagerTransport,
    pub service_name: &'static str,
    pub requires_tunnel: bool,
}

pub(crate) fn testmanager_connect_plan(product_major_version: u64) -> TestmanagerConnectPlan {
    if product_major_version >= 17 {
        TestmanagerConnectPlan {
            transport: TestmanagerTransport::RemoteServiceDiscovery,
            service_name: ios_core::testmanager::SERVICE_IOS17,
            requires_tunnel: true,
        }
    } else if product_major_version >= 14 {
        TestmanagerConnectPlan {
            transport: TestmanagerTransport::LockdownSecure,
            service_name: ios_core::testmanager::SERVICE_IOS14,
            requires_tunnel: false,
        }
    } else {
        TestmanagerConnectPlan {
            transport: TestmanagerTransport::LockdownLegacy,
            service_name: ios_core::testmanager::SERVICE_LEGACY,
            requires_tunnel: false,
        }
    }
}

impl ActiveTestPlan {
    pub fn startup_result(&self) -> &TestStartupResult {
        &self.startup_result
    }

    pub async fn wait_for_device_port_ready(&self, device_port: u16) -> Result<()> {
        let device_id = self
            .testmanager_device
            .as_ref()
            .context("XCTest session service connections are closed")?
            .info
            .device_id;
        wait_for_ready(|| async move {
            let stream = MuxClient::connect()
                .await?
                .connect_to_port(device_id, device_port)
                .await?;
            drop(stream);
            Ok(())
        })
        .await
    }

    pub async fn wait_for_results(&mut self) -> Result<TestRunSummary> {
        loop {
            let event = self
                ._testmanager
                .recv_execution_event()
                .await
                .map_err(|err| anyhow::Error::new(err).context("XCTest result stream error"))?;
            let finished = event.is_finished_plan();
            self.result_recorder.apply(event);
            if finished {
                return Ok(self.result_recorder.summary());
            }
        }
    }

    pub(crate) fn partial_summary(&self) -> TestRunSummary {
        self.result_recorder.summary()
    }

    /// Best-effort bounded cleanup used when a direct-runner deadline or
    /// cancellation interrupts result collection.
    pub async fn terminate(&mut self) {
        if !self.termination_attempted {
            if !self.kill_attempted {
                let killed =
                    terminate_process(&mut self._runner_controller, self.startup_result.pid).await;
                self.kill_attempted = true;
                self.termination_error = killed.err();
            }
            let mut result = self.termination_error.take().map_or(Ok(()), Err);
            result = crate::cmd::test_workflow::combine_results(
                result,
                self._testmanager
                    .close(Duration::from_secs(2))
                    .await
                    .map_err(anyhow::Error::new),
                "XCTest testmanager close failed",
            );
            if let RunnerController::Instruments(controller) = &mut self._runner_controller {
                result = crate::cmd::test_workflow::combine_results(
                    result,
                    controller
                        .close(Duration::from_secs(2))
                        .await
                        .map_err(anyhow::Error::new),
                    "XCTest process-control close failed",
                );
            }
            self._instruments_device.take();
            self.testmanager_device.take();
            result = crate::cmd::test_workflow::combine_results(
                result,
                self.native.collect(Duration::from_secs(5)).await,
                "XCTest original native retirement failed",
            );
            self.termination_attempted = true;
            self.termination_error = result.err();
        }
    }

    pub(crate) async fn detach(&mut self) {
        self.kill_attempted = true;
        self.terminate().await;
    }

    pub(crate) fn take_termination_error(&mut self) -> Option<anyhow::Error> {
        self.termination_error.take()
    }
}

// Retained composable helper; CLI paths use the caller-retained owned variant.
#[allow(dead_code)]
pub async fn start_test_plan(udid: &str, plan: TestLaunchPlan) -> Result<TestStartupResult> {
    let mut session = start_test_plan_session(udid, plan).await?;
    let receipt = session.startup_result().clone();
    let native = session.native.clone();
    session.detach().await;
    let result = session.take_termination_error().map_or(Ok(()), Err);
    drop(session);
    crate::cmd::test_workflow::combine_results(
        result,
        native.close(Duration::from_secs(5)).await,
        "XCTest startup receipt native cleanup failed",
    )?;
    // This receipt records startup ACK only. Continued remote execution after
    // the original service connections close has not been observed here.
    Ok(receipt)
}

#[allow(dead_code)] // Used by the retained local-owner helper above.
pub async fn start_test_plan_session(udid: &str, plan: TestLaunchPlan) -> Result<ActiveTestPlan> {
    let native = crate::cmd::tunnel::NativeScope::new()?;
    let mut cleanup = TestStartupCleanup::default();
    let result = start_test_plan_session_inner(udid, plan, None, true, &mut cleanup, &native).await;
    let result = complete_startup(result, &mut cleanup).await;
    match result {
        Ok(session) => Ok(session),
        Err(error) => Err(crate::cmd::test_workflow::combine_results(
            Err(error),
            native.close(Duration::from_secs(5)).await,
            "XCTest original native cleanup failed",
        )
        .expect_err("startup failed")),
    }
}

/// The caller retains the startup cleanup owner outside its cancellable work.
pub(crate) async fn start_test_plan_session_owned_until(
    udid: &str,
    plan: TestLaunchPlan,
    deadline: Instant,
    cleanup: &mut TestStartupCleanup,
    native: &crate::cmd::tunnel::NativeScope,
) -> Result<ActiveTestPlan> {
    start_test_plan_session_inner(udid, plan, Some(deadline), true, cleanup, native).await
}

/// WDA preserves its original Instruments launch flow, while sharing the
/// caller's absolute startup clock and owners outside cancellable work.
pub(crate) async fn start_wda_session_owned_until(
    udid: &str,
    plan: TestLaunchPlan,
    deadline: Instant,
    cleanup: &mut TestStartupCleanup,
    native: &crate::cmd::tunnel::NativeScope,
) -> Result<ActiveTestPlan> {
    start_test_plan_session_inner(udid, plan, Some(deadline), false, cleanup, native).await
}

#[allow(dead_code)] // Cleanup for the retained local-owner helper above.
async fn complete_startup(
    result: Result<ActiveTestPlan>,
    cleanup: &mut TestStartupCleanup,
) -> Result<ActiveTestPlan> {
    match result {
        Ok(session) => Ok(session),
        Err(error) => {
            let cleanup_result = cleanup.close().await;
            match crate::cmd::test_workflow::combine_results(
                Err(error),
                cleanup_result,
                "XCTest startup cleanup failed",
            ) {
                Err(error) => Err(error),
                Ok(()) => unreachable!(),
            }
        }
    }
}

async fn start_test_plan_session_inner(
    udid: &str,
    plan: TestLaunchPlan,
    deadline: Option<Instant>,
    prefer_coredevice_runner: bool,
    cleanup: &mut TestStartupCleanup,
    native: &crate::cmd::tunnel::NativeScope,
) -> Result<ActiveTestPlan> {
    cleanup.device = Some(
        run_stage(
            deadline,
            "connecting to testmanagerd device",
            Box::pin(connect_testmanager_device_scoped(udid, native)),
        )
        .await?,
    );
    let device = cleanup
        .device
        .as_ref()
        .expect("startup retains device native owner");
    let product_version = run_stage(deadline, "reading the device product version", async {
        device.product_version().await.map_err(anyhow::Error::from)
    })
    .await?;
    let connect_plan = testmanager_connect_plan(product_version.major);

    let session_stream = run_stage(
        deadline,
        "connecting the testmanager session stream",
        async {
            connect_testmanager_stream(device, connect_plan)
                .await
                .with_context(|| {
                    format!(
                        "failed to connect testmanager session stream via {}",
                        connect_plan.service_name
                    )
                })
        },
    )
    .await?;
    let control_stream = run_stage(
        deadline,
        "connecting the testmanager control stream",
        async {
            connect_testmanager_stream(device, connect_plan)
                .await
                .with_context(|| {
                    format!(
                        "failed to connect testmanager control stream via {}",
                        connect_plan.service_name
                    )
                })
        },
    )
    .await?;
    cleanup.testmanager = Some(TestmanagerClient::from_streams(
        session_stream,
        control_stream,
    ));
    let testmanager = cleanup
        .testmanager
        .as_mut()
        .expect("original streams retained");
    run_stage(deadline, "requesting testmanager DTX channels", async {
        testmanager
            .initialize_channels()
            .await
            .map_err(|err| anyhow::Error::new(err).context("DTX error"))
    })
    .await?;

    let session_id = Uuid::new_v4();
    let configuration = plan.xctest_configuration(product_version.major, session_id);
    let capabilities = configuration.ide_capabilities.clone();
    let modern_direct = prefer_coredevice_runner && product_version.major >= 17;
    if product_version.major < 14 {
        if plan.runner.container.is_none() {
            return Err(anyhow::anyhow!(
                "XCTest runner has no data-container path; iOS {} requires a device-side xctest configuration file",
                product_version.major
            ));
        }
        let configuration_for_device = configuration.clone();
        run_stage(
            deadline,
            "uploading the legacy XCTest configuration",
            upload_legacy_xctest_configuration(
                device,
                &plan.runner.bundle_id,
                session_id,
                &configuration_for_device,
            ),
        )
        .await?;
    }
    if !modern_direct {
        run_stage(
            deadline,
            "initiating the testmanager control session",
            async {
                testmanager
                    .initiate_control_session_with_capabilities(capabilities.clone())
                    .await
                    .map(|_| ())
                    .map_err(|err| anyhow::Error::new(err).context("control session error"))
            },
        )
        .await?;
    }
    run_stage(deadline, "initiating the testmanager session", async {
        testmanager
            .initiate_session_with_capabilities(session_id, capabilities)
            .await
            .map(|_| ())
            .map_err(|err| anyhow::Error::new(err).context("session init error"))
    })
    .await?;

    let launch_env = plan.launch_environment(product_version.major, session_id);
    let pid = if modern_direct {
        let pid = run_stage(
            deadline,
            "launching the XCTest runner through CoreDevice appservice",
            async {
                let (xpc, metadata) = Box::pin(
                    device.connect_xpc_service_with_metadata(ios_core::apps::APPSERVICE_SERVICE),
                )
                .await
                .map_err(anyhow::Error::from)?;
                cleanup.controller = Some(RunnerController::CoreDevice(Box::new(
                    AppServiceClient::new_with_features(xpc, udid.to_string(), metadata.features),
                )));
                let Some(RunnerController::CoreDevice(appservice)) = &mut cleanup.controller else {
                    unreachable!()
                };
                let options = LaunchApplicationOptions {
                    arguments: plan.args.clone(),
                    environment_variables: launch_env.clone().into_iter().collect(),
                    standard_io_uses_pseudoterminals: true,
                    start_stopped: false,
                    terminate_existing: true,
                    standard_io_identifiers: Default::default(),
                };
                let previous = if appservice.supports_process_listing() {
                    Some(
                        appservice
                            .list_processes()
                            .await?
                            .into_iter()
                            .filter(|process| {
                                process.bundle_id.as_deref() == Some(plan.runner.bundle_id.as_str())
                            })
                            .map(|process| process.pid)
                            .collect(),
                    )
                } else {
                    None
                };
                cleanup.pending_runner = Some((plan.runner.bundle_id.clone(), previous));
                let pid = appservice
                    .launch_application_observed(
                        &plan.runner.bundle_id,
                        &options,
                        &mut cleanup.launch_submitted,
                    )
                    .await
                    .map_err(|error| anyhow::Error::new(error).context("CoreDevice launch error"))?
                    .ok_or_else(|| {
                        anyhow::anyhow!("CoreDevice launch returned no PID for XCTest runner")
                    })?;
                cleanup.pid = Some(pid);
                cleanup.pending_runner = None;
                Ok::<_, anyhow::Error>(pid)
            },
        )
        .await?;
        pid
    } else {
        let (instruments_device, instruments_stream) = run_stage(
            deadline,
            "connecting the runner launch service",
            Box::pin(connect_instruments_scoped(udid, native)),
        )
        .await?;
        cleanup.instruments_device = Some(instruments_device);
        cleanup.controller = Some(RunnerController::Instruments(Box::new(
            ProcessControl::from_stream(instruments_stream),
        )));
        let Some(RunnerController::Instruments(process_control)) = &mut cleanup.controller else {
            unreachable!()
        };
        run_stage(
            deadline,
            "requesting the runner process-control channel",
            async {
                process_control
                    .initialize_channel()
                    .await
                    .map_err(|err| anyhow::Error::new(err).context("process control error"))
            },
        )
        .await?;
        let launch_args = plan.launch_arguments();
        let launch_arg_refs: Vec<&str> = launch_args.iter().map(String::as_str).collect();
        let launch_options = plan.launch_options(product_version.major);
        let pid = match run_stage(deadline, "launching the XCTest runner", async {
            let pid = process_control
                .launch_with_options(
                    &plan.runner.bundle_id,
                    &launch_arg_refs,
                    &launch_env,
                    &launch_options,
                )
                .await
                .map_err(|err| anyhow::Error::new(err).context("launch error"))?;
            cleanup.pid = Some(pid);
            Ok::<_, anyhow::Error>(pid)
        })
        .await
        {
            Ok(pid) => pid,
            Err(error) => {
                // A launch request that times out has no reliable PID to
                // kill. The DTX connection is dropped here; once a PID is
                // known, later startup failures use bounded cleanup.
                return Err(error);
            }
        };
        pid
    };

    if modern_direct {
        // The iOS 17+ workflow initializes the control connection after the
        // runner launch and uses an empty capability dictionary, matching the
        // CoreDevice/testmanagerd handshake used by go-ios. Keeping this
        // after launch is important: testmanagerd may otherwise authorize a
        // stale runner instance before the DDI launch has completed.
        let control_result = run_stage(
            deadline,
            "initiating the testmanager control session",
            async {
                testmanager
                    .initiate_control_session_with_capabilities(XctCapabilities {
                        capabilities: Vec::new(),
                    })
                    .await
                    .map(|_| ())
                    .map_err(|err| anyhow::Error::new(err).context("control session error"))
            },
        )
        .await;
        // The separately retained startup owner handles the confirmed PID even
        // when this control handshake fails or the caller cancels the future.
        control_result?;
    }

    let summary = match run_stage(deadline, "completing the XCTest startup handshake", async {
        testmanager
            .authorize_and_start_test_plan_with_configuration(pid, configuration)
            .await
            .map_err(|err| anyhow::Error::new(err).context("startup handshake error"))
    })
    .await
    {
        Ok(summary) => summary,
        Err(error) => {
            return Err(error);
        }
    };

    Ok(ActiveTestPlan {
        startup_result: TestStartupResult {
            runner_bundle_id: plan.runner.bundle_id.clone(),
            target_bundle_id: plan.target.as_ref().map(|target| target.bundle_id.clone()),
            pid,
            protocol_version: summary.protocol_version,
            minimum_version: summary.minimum_version,
        },
        testmanager_device: cleanup.device.take(),
        _testmanager: cleanup
            .testmanager
            .take()
            .expect("startup retains testmanager"),
        _instruments_device: cleanup.instruments_device.take(),
        _runner_controller: cleanup
            .controller
            .take()
            .expect("startup retains runner control"),
        result_recorder: TestRunRecorder::default(),
        termination_attempted: false,
        kill_attempted: false,
        termination_error: None,
        native: native.clone(),
    })
}

async fn terminate_process(controller: &mut RunnerController, pid: u64) -> Result<()> {
    match controller {
        RunnerController::Instruments(process_control) => {
            tokio::time::timeout(Duration::from_secs(2), process_control.kill(pid))
                .await
                .context("XCTest runner kill deadline expired")?
                .map_err(anyhow::Error::new)
        }
        RunnerController::CoreDevice(appservice) => {
            tokio::time::timeout(Duration::from_secs(2), appservice.kill_process(pid))
                .await
                .context("XCTest runner kill deadline expired")?
                .map_err(anyhow::Error::new)
        }
    }
}

fn run_stage<'a, T: 'a, F>(
    deadline: Option<Instant>,
    operation: &'a str,
    future: F,
) -> impl Future<Output = Result<T>> + 'a
where
    F: Future<Output = Result<T>> + 'a,
{
    let future = Box::pin(future);
    async move {
        let result = match deadline {
            Some(deadline) => tokio::time::timeout_at(deadline, future)
                .await
                .map_err(|error| {
                    anyhow::Error::new(error)
                        .context(format!("XCTest deadline expired while {operation}"))
                })?,
            None => future.await,
        };
        result.with_context(|| format!("XCTest {operation} failed"))
    }
}

pub(crate) async fn connect_testmanager_device_scoped(
    udid: &str,
    native: &crate::cmd::tunnel::NativeScope,
) -> Result<OwnedTestDevice> {
    let version = Box::pin(crate::cmd::connect::probe_product_version(udid)).await?;
    if version.major >= 17 {
        Ok(OwnedTestDevice::Scoped(Box::new(
            Box::pin(native.connect(udid, ios_core::TunMode::Userspace)).await?,
        )))
    } else {
        Ok(OwnedTestDevice::Plain(Box::new(
            Box::pin(connect_lockdown_only(udid)).await?,
        )))
    }
}

async fn connect_instruments_scoped(
    udid: &str,
    native: &crate::cmd::tunnel::NativeScope,
) -> Result<(OwnedTestDevice, ServiceStream)> {
    let device = Box::pin(connect_testmanager_device_scoped(udid, native)).await?;
    let version = device.product_version().await?;
    let stream = if version.major >= 17 {
        device
            .connect_rsd_service(ios_core::instruments::SERVICE_IOS17)
            .await?
    } else {
        device
            .connect_service(if version.major >= 14 {
                ios_core::instruments::SERVICE_IOS14
            } else {
                ios_core::instruments::SERVICE_LEGACY
            })
            .await?
    };
    Ok((device, stream))
}

async fn connect_testmanager_stream(
    device: &ConnectedDevice,
    connect_plan: TestmanagerConnectPlan,
) -> Result<ServiceStream> {
    match connect_plan.transport {
        TestmanagerTransport::RemoteServiceDiscovery => {
            Box::pin(device.connect_rsd_service(connect_plan.service_name)).await
        }
        TestmanagerTransport::LockdownSecure | TestmanagerTransport::LockdownLegacy => {
            Box::pin(device.connect_service(connect_plan.service_name)).await
        }
    }
    .map_err(anyhow::Error::from)
}

pub async fn lookup_installed_app(
    device: &ConnectedDevice,
    bundle_id: &str,
) -> Result<InstalledAppInfo> {
    let stream = Box::pin(device.connect_service(ios_core::apps::INSTALLATION_PROXY_SERVICE))
        .await
        .context("failed to connect installation_proxy")?;
    let mut proxy = InstallationProxy::new(stream);
    let attrs = ["CFBundleExecutable", "Container", "Path"];
    let app = proxy
        .lookup_app_with_attributes(bundle_id, &attrs)
        .await?
        .ok_or_else(|| anyhow::anyhow!("app not found: {bundle_id}"))?;

    Ok(InstalledAppInfo {
        bundle_id: app.bundle_id,
        path: app.path,
        executable: plist_string(&app.extra, "CFBundleExecutable")
            .ok_or_else(|| anyhow::anyhow!("missing CFBundleExecutable for {bundle_id}"))?,
        container: plist_string(&app.extra, "Container"),
    })
}

async fn upload_legacy_xctest_configuration(
    device: &ConnectedDevice,
    runner_bundle_id: &str,
    session_identifier: Uuid,
    configuration: &ios_core::XcTestConfiguration,
) -> Result<()> {
    let stream = Box::pin(device.connect_service(ios_core::afc::house_arrest::SERVICE_NAME))
        .await
        .context("failed to connect legacy House Arrest for XCTest configuration")?;
    let house_arrest = HouseArrestClient::new(stream)
        .with_operation_timeout(Some(std::time::Duration::from_secs(30)));
    let mut container = house_arrest
        .vend_container(runner_bundle_id)
        .await
        .context("failed to vend XCTest runner container")?;
    let relative_path = format!("tmp/{}.xctestconfiguration", session_identifier);
    let bytes = archive_xctest_configuration(configuration.clone());
    container
        .write_file(&relative_path, &bytes)
        .await
        .with_context(|| format!("failed to write {relative_path} in XCTest runner container"))?;
    Ok(())
}

async fn build_plan_from_xctestrun(
    device: &ConnectedDevice,
    xctestrun_path: &Path,
    configs: &[TestConfiguration],
    configuration_name: Option<&str>,
    test_target: Option<&str>,
) -> Result<(String, TestLaunchPlan)> {
    let config = select_configuration(configs, configuration_name).ok_or_else(|| {
        anyhow::anyhow!(
            "test configuration {:?} not found in {}",
            configuration_name,
            xctestrun_path.display()
        )
    })?;
    let scheme = select_test_target(config, test_target).ok_or_else(|| {
        anyhow::anyhow!(
            "test target {:?} not found in configuration {:?}",
            test_target,
            config.name
        )
    })?;

    let runner = lookup_installed_app(device, &scheme.test_host_bundle_identifier).await?;
    let target = match infer_target_bundle_id(scheme, None) {
        Some(bundle_id) => Some(lookup_installed_app(device, &bundle_id).await?),
        None if scheme.is_ui_test_bundle && !scheme.ui_target_app_path.is_empty() => {
            match infer_target_bundle_id(
                scheme,
                Some(&list_installed_apps_for_target_inference(device).await?),
            ) {
                Some(bundle_id) => Some(lookup_installed_app(device, &bundle_id).await?),
                None => None,
            }
        }
        None => None,
    };
    let plan = TestLaunchPlan::from_scheme(scheme, runner, target);
    Ok((config.name.clone(), plan))
}

fn select_configuration<'a>(
    configs: &'a [TestConfiguration],
    name: Option<&str>,
) -> Option<&'a TestConfiguration> {
    match name {
        Some(name) => configs.iter().find(|config| config.name == name),
        None => configs.first(),
    }
}

fn select_test_target<'a>(
    config: &'a TestConfiguration,
    target: Option<&str>,
) -> Option<&'a ios_core::testmanager::xctestrun::SchemeData> {
    match target {
        Some(target) => config
            .test_targets
            .iter()
            .find(|scheme| scheme_matches_target(scheme, target)),
        None => config.test_targets.first(),
    }
}

fn scheme_matches_target(
    scheme: &ios_core::testmanager::xctestrun::SchemeData,
    target: &str,
) -> bool {
    scheme.test_host_bundle_identifier == target
        || test_bundle_name_from_path(&scheme.test_bundle_path).as_deref() == Some(target)
        || scheme.test_bundle_path == target
}

fn test_bundle_name_from_path(path: &str) -> Option<String> {
    Path::new(path)
        .file_name()
        .and_then(|name| name.to_str())
        .map(|name| name.trim_end_matches(".xctest").to_string())
        .filter(|name| !name.is_empty())
}

pub(crate) fn plist_string(values: &HashMap<String, plist::Value>, key: &str) -> Option<String> {
    values
        .get(key)
        .and_then(plist::Value::as_string)
        .map(ToString::to_string)
}

async fn list_installed_apps_for_target_inference(
    device: &ConnectedDevice,
) -> Result<Vec<AppInfo>> {
    let stream = Box::pin(device.connect_service(ios_core::apps::INSTALLATION_PROXY_SERVICE))
        .await
        .context("failed to connect installation_proxy for target app inference")?;
    let mut proxy = InstallationProxy::new(stream);
    proxy
        .list_user_apps_with_attributes(&[
            "CFBundleName",
            "CFBundleExecutable",
            "Path",
            "Container",
        ])
        .await
        .map_err(anyhow::Error::from)
}

fn infer_target_bundle_id(
    scheme: &ios_core::testmanager::xctestrun::SchemeData,
    installed_apps: Option<&[AppInfo]>,
) -> Option<String> {
    if !scheme.is_ui_test_bundle {
        return None;
    }
    scheme
        .ui_target_app_environment_variables
        .get("UITargetAppBundleIdentifier")
        .and_then(plist::Value::as_string)
        .map(ToString::to_string)
        .or_else(|| infer_target_bundle_id_from_path(&scheme.ui_target_app_path, installed_apps?))
}

fn infer_target_bundle_id_from_path(
    ui_target_app_path: &str,
    installed_apps: &[AppInfo],
) -> Option<String> {
    let target_name = app_name_from_path(ui_target_app_path)?;
    installed_apps
        .iter()
        .find(|app| {
            plist_string(&app.extra, "CFBundleName").as_deref() == Some(target_name.as_str())
                || app_name_from_path(&app.path).as_deref() == Some(target_name.as_str())
        })
        .map(|app| app.bundle_id.clone())
}

fn app_name_from_path(path: &str) -> Option<String> {
    Path::new(path)
        .file_name()
        .and_then(|name| name.to_str())
        .map(|name| name.trim_end_matches(".app").to_string())
        .filter(|name| !name.is_empty())
}

const DEVICE_PORT_POLL_INTERVAL: Duration = Duration::from_millis(500);

async fn wait_for_ready<P, PFut>(mut probe: P) -> Result<()>
where
    P: FnMut() -> PFut,
    PFut: Future<Output = Result<()>>,
{
    loop {
        match probe().await {
            Ok(()) => return Ok(()),
            Err(err) => {
                tracing::debug!(error = %crate::output::error_message(&err), "device port not ready yet");
                tokio::time::sleep(DEVICE_PORT_POLL_INTERVAL).await;
            }
        }
    }
}

pub(crate) fn xctest_wait_timeout(
    elapsed: tokio::time::error::Elapsed,
    phase: &'static str,
) -> anyhow::Error {
    anyhow::Error::new(elapsed).context(phase)
}

#[cfg(test)]
mod tests {
    #[tokio::test]
    async fn submitted_launch_without_pid_cannot_report_successful_cleanup() {
        let mut cleanup = super::TestStartupCleanup {
            launch_submitted: true,
            ..Default::default()
        };
        let error = cleanup.close().await.unwrap_err();
        assert!(error
            .downcast_ref::<super::RunnerLaunchUnconfirmed>()
            .is_some());
        assert!(cleanup.close().await.is_ok(), "ownership is consumed once");
    }
    #[tokio::test]
    async fn stream_contract_xctest_waits_preserve_actual_elapsed() {
        for phase in [
            "timed out waiting for XCTest startup",
            "timed out waiting for XCTest results",
        ] {
            let elapsed =
                tokio::time::timeout(std::time::Duration::ZERO, std::future::pending::<()>())
                    .await
                    .unwrap_err();
            let error = super::xctest_wait_timeout(elapsed, phase);
            assert_eq!(
                ios_core::classify_error(error.as_ref()),
                ios_core::ErrorClass::Timeout
            );
            assert!(error
                .chain()
                .any(|cause| cause.is::<tokio::time::error::Elapsed>()));
            assert_eq!(error.to_string(), phase);
        }
    }

    #[tokio::test]
    async fn cli_native_sources_testmanager_stage_keeps_actual_elapsed() {
        let error = super::run_stage(
            Some(tokio::time::Instant::now()),
            "owned stalled stage",
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
    use std::collections::HashMap;
    use std::path::PathBuf;
    use std::sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    };
    use std::time::Duration;

    use clap::Parser;
    use ios_core::apps::AppInfo;
    use ios_core::testmanager::xctestrun::{SchemeData, TestConfiguration};
    use plist::Value;

    use super::{
        infer_target_bundle_id, select_test_target, testmanager_connect_plan, wait_for_ready,
        RunTestCmd, TestmanagerTransport,
    };

    #[derive(Parser)]
    struct TestCli {
        #[command(flatten)]
        command: RunTestCmd,
    }

    #[test]
    fn parses_runtest_command() {
        let cmd =
            TestCli::parse_from(["runtest", "Tests.xctestrun", "--startup-timeout-secs", "45"]);
        assert_eq!(
            cmd.command.xctestrun,
            std::path::PathBuf::from("Tests.xctestrun")
        );
        assert_eq!(cmd.command.startup_timeout_secs, 45);
    }

    #[test]
    fn parses_runtest_selection_and_wait_options() {
        let cmd = TestCli::parse_from([
            "runtest",
            "Tests.xctestrun",
            "--configuration",
            "UITests",
            "--test-target",
            "LoginTarget",
            "--wait",
            "--result-timeout-secs",
            "120",
            "--junit-output",
            "results.xml",
        ]);

        assert_eq!(cmd.command.configuration.as_deref(), Some("UITests"));
        assert_eq!(cmd.command.test_target.as_deref(), Some("LoginTarget"));
        assert!(cmd.command.wait);
        assert_eq!(cmd.command.result_timeout_secs, 120);
        assert_eq!(cmd.command.junit_output, Some(PathBuf::from("results.xml")));
    }

    #[test]
    fn testmanager_transport_plan_matches_ios_generation() {
        let ios17 = testmanager_connect_plan(17);
        assert_eq!(
            ios17.transport,
            TestmanagerTransport::RemoteServiceDiscovery
        );
        assert_eq!(ios17.service_name, ios_core::testmanager::SERVICE_IOS17);
        assert!(ios17.requires_tunnel);

        let ios14 = testmanager_connect_plan(14);
        assert_eq!(ios14.transport, TestmanagerTransport::LockdownSecure);
        assert_eq!(ios14.service_name, ios_core::testmanager::SERVICE_IOS14);
        assert!(!ios14.requires_tunnel);

        let ios13 = testmanager_connect_plan(13);
        assert_eq!(ios13.transport, TestmanagerTransport::LockdownLegacy);
        assert_eq!(ios13.service_name, ios_core::testmanager::SERVICE_LEGACY);
        assert!(!ios13.requires_tunnel);
    }

    fn ui_scheme() -> SchemeData {
        SchemeData {
            test_host_bundle_identifier: "com.example.Runner".to_string(),
            test_bundle_path: "DemoAppUITests.xctest".to_string(),
            skip_test_identifiers: Vec::new(),
            only_test_identifiers: Vec::new(),
            is_ui_test_bundle: true,
            command_line_arguments: Vec::new(),
            environment_variables: HashMap::new(),
            testing_environment_variables: HashMap::new(),
            ui_target_app_environment_variables: HashMap::new(),
            ui_target_app_command_line_arguments: Vec::new(),
            ui_target_app_path: "__TESTROOT__/Debug-iphoneos/DemoApp.app".to_string(),
        }
    }

    #[test]
    fn infer_target_bundle_id_prefers_explicit_bundle_identifier() {
        let mut scheme = ui_scheme();
        scheme.ui_target_app_environment_variables.insert(
            "UITargetAppBundleIdentifier".to_string(),
            Value::String("com.example.explicit".to_string()),
        );

        let apps = vec![AppInfo {
            bundle_id: "com.example.from-path".to_string(),
            display_name: String::new(),
            version: String::new(),
            app_type: String::new(),
            path: "/private/var/containers/Bundle/Application/XYZ/DemoApp.app".to_string(),
            extra: HashMap::from([(
                "CFBundleName".to_string(),
                Value::String("DemoApp".to_string()),
            )]),
        }];

        assert_eq!(
            infer_target_bundle_id(&scheme, Some(&apps)).as_deref(),
            Some("com.example.explicit")
        );
    }

    #[test]
    fn select_test_target_matches_runner_bundle_or_xctest_name() {
        let config = TestConfiguration {
            name: "UITests".to_string(),
            test_targets: vec![
                SchemeData {
                    test_host_bundle_identifier: "com.example.FirstRunner".to_string(),
                    test_bundle_path: "FirstTests.xctest".to_string(),
                    skip_test_identifiers: Vec::new(),
                    only_test_identifiers: Vec::new(),
                    is_ui_test_bundle: true,
                    command_line_arguments: Vec::new(),
                    environment_variables: HashMap::new(),
                    testing_environment_variables: HashMap::new(),
                    ui_target_app_environment_variables: HashMap::new(),
                    ui_target_app_command_line_arguments: Vec::new(),
                    ui_target_app_path: String::new(),
                },
                SchemeData {
                    test_host_bundle_identifier: "com.example.SecondRunner".to_string(),
                    test_bundle_path: "__TESTROOT__/SecondTests.xctest".to_string(),
                    skip_test_identifiers: Vec::new(),
                    only_test_identifiers: Vec::new(),
                    is_ui_test_bundle: true,
                    command_line_arguments: Vec::new(),
                    environment_variables: HashMap::new(),
                    testing_environment_variables: HashMap::new(),
                    ui_target_app_environment_variables: HashMap::new(),
                    ui_target_app_command_line_arguments: Vec::new(),
                    ui_target_app_path: String::new(),
                },
            ],
        };

        assert_eq!(
            select_test_target(&config, Some("com.example.SecondRunner"))
                .unwrap()
                .test_host_bundle_identifier,
            "com.example.SecondRunner"
        );
        assert_eq!(
            select_test_target(&config, Some("SecondTests"))
                .unwrap()
                .test_host_bundle_identifier,
            "com.example.SecondRunner"
        );
    }

    #[test]
    fn infer_target_bundle_id_falls_back_to_ui_target_app_path() {
        let scheme = ui_scheme();
        let apps = vec![AppInfo {
            bundle_id: "com.example.demo".to_string(),
            display_name: String::new(),
            version: String::new(),
            app_type: String::new(),
            path: "/private/var/containers/Bundle/Application/XYZ/DemoApp.app".to_string(),
            extra: HashMap::from([(
                "CFBundleName".to_string(),
                Value::String("DemoApp".to_string()),
            )]),
        }];

        assert_eq!(
            infer_target_bundle_id(&scheme, Some(&apps)).as_deref(),
            Some("com.example.demo")
        );
    }

    #[test]
    fn infer_target_bundle_id_returns_none_when_ui_target_path_does_not_match() {
        let scheme = ui_scheme();
        let apps = vec![AppInfo {
            bundle_id: "com.example.other".to_string(),
            display_name: String::new(),
            version: String::new(),
            app_type: String::new(),
            path: "/private/var/containers/Bundle/Application/XYZ/Other.app".to_string(),
            extra: HashMap::from([(
                "CFBundleName".to_string(),
                Value::String("Other".to_string()),
            )]),
        }];

        assert_eq!(infer_target_bundle_id(&scheme, Some(&apps)), None);
    }

    #[tokio::test]
    async fn wait_for_ready_retries_until_probe_succeeds() {
        let attempts = Arc::new(AtomicUsize::new(0));
        let attempts_for_probe = attempts.clone();

        let wait = tokio::spawn(async move {
            wait_for_ready(|| {
                let attempts = attempts_for_probe.clone();
                async move {
                    let current = attempts.fetch_add(1, Ordering::SeqCst);
                    if current < 2 {
                        anyhow::bail!("still starting")
                    }
                    Ok(())
                }
            })
            .await
        });

        tokio::task::yield_now().await;
        tokio::time::sleep(Duration::from_millis(1100)).await;

        wait.await.unwrap().unwrap();
        assert_eq!(attempts.load(Ordering::SeqCst), 3);
    }

    #[test]
    fn parses_runtest_workflow_options() {
        let parsed = TestCli::parse_from([
            "runtest",
            "Tests.xctestrun",
            "--install-app",
            "test_app.ipa",
            "--install-runner",
            "test_runner.ipa",
            "--log-output",
            "syslog.log",
            "--uninstall-on-finish",
            "--rerun-failed",
            "3",
        ]);
        assert_eq!(
            parsed.command.workflow.install_app,
            Some(PathBuf::from("test_app.ipa"))
        );
        assert_eq!(
            parsed.command.workflow.install_runner,
            Some(PathBuf::from("test_runner.ipa"))
        );
        assert_eq!(
            parsed.command.workflow.log_output,
            Some(PathBuf::from("syslog.log"))
        );
        assert!(parsed.command.workflow.uninstall_on_finish);
        assert_eq!(parsed.command.workflow.rerun_failed, 3);
    }
}
