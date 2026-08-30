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

use crate::cmd::connect::{connect_lockdown_only, connect_userspace_tunnel};

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
}

impl RunTestCmd {
    pub async fn run(self, udid: Option<String>) -> Result<()> {
        let udid = udid.ok_or_else(|| anyhow::anyhow!("--udid required for runtest"))?;
        if self.junit_output.is_some() && !self.wait {
            return Err(anyhow::anyhow!(
                "--junit-output requires --wait because an early XCTest startup result is not a complete test report"
            ));
        }
        eprintln!("UNTESTED: XCTest execution workflow has automated coverage, but no real-device validation in this workspace yet.");

        let device = match connect_testmanager_device(&udid).await {
            Ok(device) => device,
            Err(error) => {
                write_junit_diagnostic(
                    self.junit_output.as_deref(),
                    &incomplete_summary(false),
                    &error,
                )?;
                return Err(error);
            }
        };
        let configs = match parse_xctestrun_file(&self.xctestrun)
            .with_context(|| format!("failed to parse {}", self.xctestrun.display()))
        {
            Ok(configs) => configs,
            Err(error) => {
                write_junit_diagnostic(
                    self.junit_output.as_deref(),
                    &incomplete_summary(false),
                    &error,
                )?;
                return Err(error);
            }
        };
        let (configuration_name, plan) = match build_plan_from_xctestrun(
            &device,
            &self.xctestrun,
            &configs,
            self.configuration.as_deref(),
            self.test_target.as_deref(),
        )
        .await
        {
            Ok(plan) => plan,
            Err(error) => {
                write_junit_diagnostic(
                    self.junit_output.as_deref(),
                    &incomplete_summary(false),
                    &error,
                )?;
                return Err(error);
            }
        };

        if self.wait {
            let startup = tokio::time::timeout(
                std::time::Duration::from_secs(self.startup_timeout_secs),
                start_test_plan_session(&udid, plan),
            )
            .await;
            let mut session = match startup {
                Err(_) => {
                    let error = anyhow::anyhow!("timed out waiting for XCTest startup");
                    write_junit_diagnostic(
                        self.junit_output.as_deref(),
                        &incomplete_summary(false),
                        &error,
                    )?;
                    return Err(error);
                }
                Ok(Err(error)) => {
                    write_junit_diagnostic(
                        self.junit_output.as_deref(),
                        &incomplete_summary(false),
                        &error,
                    )?;
                    return Err(error);
                }
                Ok(Ok(session)) => session,
            };
            let result = session.startup_result().clone();
            let result_wait = tokio::time::timeout(
                std::time::Duration::from_secs(self.result_timeout_secs),
                session.wait_for_results(),
            )
            .await;
            let summary = match result_wait {
                Err(_) => {
                    let error = anyhow::anyhow!("timed out waiting for XCTest results");
                    write_junit_diagnostic(
                        self.junit_output.as_deref(),
                        &incomplete_summary(true),
                        &error,
                    )?;
                    return Err(error);
                }
                Ok(Err(error)) => {
                    write_junit_diagnostic(
                        self.junit_output.as_deref(),
                        &incomplete_summary(true),
                        &error,
                    )?;
                    return Err(error);
                }
                Ok(Ok(summary)) => summary,
            };
            write_junit_summary(self.junit_output.as_deref(), &summary)?;

            println!(
                "{}",
                serde_json::to_string_pretty(&serde_json::json!({
                    "status": "finished",
                    "untested": true,
                    "configuration": configuration_name,
                    "runner_bundle_id": result.runner_bundle_id,
                    "target_bundle_id": result.target_bundle_id,
                    "pid": result.pid,
                    "protocol_version": result.protocol_version,
                    "minimum_version": result.minimum_version,
                    "summary": summary,
                    "note": "Result event parsing is covered offline; real XCTest devices still need validation.",
                }))?
            );
            return Ok(());
        }

        let result = tokio::time::timeout(
            std::time::Duration::from_secs(self.startup_timeout_secs),
            start_test_plan(&udid, plan),
        )
        .await
        .map_err(|_| anyhow::anyhow!("timed out waiting for XCTest startup"))??;

        println!(
            "{}",
            serde_json::to_string_pretty(&serde_json::json!({
                "status": "started",
                "untested": true,
                "configuration": configuration_name,
                "runner_bundle_id": result.runner_bundle_id,
                "target_bundle_id": result.target_bundle_id,
                "pid": result.pid,
                "protocol_version": result.protocol_version,
                "minimum_version": result.minimum_version,
                "note": "Current Rust workflow stops after _IDE_startExecutingTestPlanWithProtocolVersion and does not yet stream XCTest result events.",
            }))?
        );

        Ok(())
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
        write_junit_xml_atomic_with_diagnostic(summary, path, &error.to_string()).with_context(
            || format!("failed writing diagnostic JUnit XML to {}", path.display()),
        )?;
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
    testmanager_device: ConnectedDevice,
    _testmanager: TestmanagerClient<ServiceStream>,
    _instruments_device: Option<ConnectedDevice>,
    _runner_controller: RunnerController,
}

enum RunnerController {
    Instruments(Box<ProcessControl<ServiceStream>>),
    CoreDevice(Box<AppServiceClient>),
}

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
        let device_id = self.testmanager_device.info.device_id;
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
        let mut recorder = TestRunRecorder::default();
        loop {
            let event = self
                ._testmanager
                .recv_execution_event()
                .await
                .map_err(|err| anyhow::anyhow!("XCTest result stream error: {err}"))?;
            let finished = event.is_finished_plan();
            recorder.apply(event);
            if finished {
                return Ok(recorder.summary());
            }
        }
    }

    /// Best-effort bounded cleanup used when a direct-runner deadline or
    /// cancellation interrupts result collection.
    pub async fn terminate(&mut self) {
        terminate_process(&mut self._runner_controller, self.startup_result.pid).await;
    }
}

pub async fn start_test_plan(udid: &str, plan: TestLaunchPlan) -> Result<TestStartupResult> {
    Ok(start_test_plan_session(udid, plan)
        .await?
        .startup_result()
        .clone())
}

pub async fn start_test_plan_session(udid: &str, plan: TestLaunchPlan) -> Result<ActiveTestPlan> {
    start_test_plan_session_inner(udid, plan, None, false).await
}

/// Start a test plan with one absolute deadline covering discovery, both
/// testmanager connections, runner launch, and the startup handshake.
pub async fn start_test_plan_session_until(
    udid: &str,
    plan: TestLaunchPlan,
    deadline: Instant,
) -> Result<ActiveTestPlan> {
    start_test_plan_session_inner(udid, plan, Some(deadline), true).await
}

async fn start_test_plan_session_inner(
    udid: &str,
    plan: TestLaunchPlan,
    deadline: Option<Instant>,
    prefer_coredevice_runner: bool,
) -> Result<ActiveTestPlan> {
    let device = run_stage(
        deadline,
        "connecting to testmanagerd device",
        connect_testmanager_device(udid),
    )
    .await?;
    let product_version = run_stage(deadline, "reading the device product version", async {
        device.product_version().await.map_err(anyhow::Error::from)
    })
    .await?;
    let connect_plan = testmanager_connect_plan(product_version.major);

    let session_stream = run_stage(
        deadline,
        "connecting the testmanager session stream",
        async {
            connect_testmanager_stream(&device, connect_plan)
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
            connect_testmanager_stream(&device, connect_plan)
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
    let mut testmanager = run_stage(deadline, "requesting testmanager DTX channels", async {
        TestmanagerClient::connect(session_stream, control_stream)
            .await
            .map_err(|err| anyhow::anyhow!("DTX error: {err}"))
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
                &device,
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
                    .map_err(|err| anyhow::anyhow!("control session error: {err}"))
            },
        )
        .await?;
    }
    run_stage(deadline, "initiating the testmanager session", async {
        testmanager
            .initiate_session_with_capabilities(session_id, capabilities)
            .await
            .map(|_| ())
            .map_err(|err| anyhow::anyhow!("session init error: {err}"))
    })
    .await?;

    let launch_env = plan.launch_environment(product_version.major, session_id);
    let (instruments_device, mut runner_controller, pid) = if modern_direct {
        let (appservice, pid) = run_stage(
            deadline,
            "launching the XCTest runner through CoreDevice appservice",
            async {
                let (xpc, metadata) = device
                    .connect_xpc_service_with_metadata(ios_core::apps::APPSERVICE_SERVICE)
                    .await
                    .map_err(anyhow::Error::from)?;
                let mut appservice =
                    AppServiceClient::new_with_features(xpc, udid.to_string(), metadata.features);
                let options = LaunchApplicationOptions {
                    arguments: plan.args.clone(),
                    environment_variables: launch_env.clone().into_iter().collect(),
                    standard_io_uses_pseudoterminals: true,
                    start_stopped: false,
                    terminate_existing: true,
                    standard_io_identifiers: Default::default(),
                };
                let pid = appservice
                    .launch_application_with_options(&plan.runner.bundle_id, &options)
                    .await
                    .map_err(|error| anyhow::anyhow!("CoreDevice launch error: {error}"))?
                    .ok_or_else(|| {
                        anyhow::anyhow!("CoreDevice launch returned no PID for XCTest runner")
                    })?;
                Ok::<_, anyhow::Error>((appservice, pid))
            },
        )
        .await?;
        (
            None,
            RunnerController::CoreDevice(Box::new(appservice)),
            pid,
        )
    } else {
        let (instruments_device, instruments_stream) = run_stage(
            deadline,
            "connecting the runner launch service",
            crate::cmd::instruments::connect_instruments(udid),
        )
        .await?;
        let mut process_control = run_stage(
            deadline,
            "requesting the runner process-control channel",
            async {
                ProcessControl::connect(instruments_stream)
                    .await
                    .map_err(|err| anyhow::anyhow!("process control error: {err}"))
            },
        )
        .await?;
        let launch_args = plan.launch_arguments();
        let launch_arg_refs: Vec<&str> = launch_args.iter().map(String::as_str).collect();
        let launch_options = plan.launch_options(product_version.major);
        let pid = match run_stage(deadline, "launching the XCTest runner", async {
            process_control
                .launch_with_options(
                    &plan.runner.bundle_id,
                    &launch_arg_refs,
                    &launch_env,
                    &launch_options,
                )
                .await
                .map_err(|err| anyhow::anyhow!("launch error: {err}"))
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
        (
            Some(instruments_device),
            RunnerController::Instruments(Box::new(process_control)),
            pid,
        )
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
                    .map_err(|err| anyhow::anyhow!("control session error: {err}"))
            },
        )
        .await;
        if let Err(error) = control_result {
            // The CoreDevice launch has already returned a PID at this point;
            // do not leave a runner orphaned when the second testmanager
            // handshake fails or reaches the shared deadline.
            terminate_process(&mut runner_controller, pid).await;
            return Err(error);
        }
    }

    let summary = match run_stage(deadline, "completing the XCTest startup handshake", async {
        testmanager
            .authorize_and_start_test_plan_with_configuration(pid, configuration)
            .await
            .map_err(|err| anyhow::anyhow!("startup handshake error: {err}"))
    })
    .await
    {
        Ok(summary) => summary,
        Err(error) => {
            terminate_process(&mut runner_controller, pid).await;
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
        testmanager_device: device,
        _testmanager: testmanager,
        _instruments_device: instruments_device,
        _runner_controller: runner_controller,
    })
}

async fn terminate_process(controller: &mut RunnerController, pid: u64) {
    match controller {
        RunnerController::Instruments(process_control) => {
            let _ = tokio::time::timeout(Duration::from_secs(2), process_control.kill(pid)).await;
        }
        RunnerController::CoreDevice(appservice) => {
            let _ =
                tokio::time::timeout(Duration::from_secs(2), appservice.kill_process(pid)).await;
        }
    }
}

async fn run_stage<T, F>(deadline: Option<Instant>, operation: &str, future: F) -> Result<T>
where
    F: Future<Output = Result<T>>,
{
    let result = match deadline {
        Some(deadline) => tokio::time::timeout_at(deadline, future)
            .await
            .map_err(|_| anyhow::anyhow!("XCTest deadline expired while {operation}"))?,
        None => future.await,
    };
    result.with_context(|| format!("XCTest {operation} failed"))
}

pub async fn connect_testmanager_device(udid: &str) -> Result<ConnectedDevice> {
    let product_version = crate::cmd::connect::probe_product_version(udid).await?;
    let connect_plan = testmanager_connect_plan(product_version.major);

    let device = if connect_plan.requires_tunnel {
        connect_userspace_tunnel(udid).await
    } else {
        connect_lockdown_only(udid).await
    };

    device.with_context(|| {
        if connect_plan.requires_tunnel {
            "failed to establish device tunnel for testmanager".to_string()
        } else {
            "failed to connect device for lockdown testmanager".to_string()
        }
    })
}

async fn connect_testmanager_stream(
    device: &ConnectedDevice,
    connect_plan: TestmanagerConnectPlan,
) -> Result<ServiceStream> {
    match connect_plan.transport {
        TestmanagerTransport::RemoteServiceDiscovery => {
            device.connect_rsd_service(connect_plan.service_name).await
        }
        TestmanagerTransport::LockdownSecure | TestmanagerTransport::LockdownLegacy => {
            device.connect_service(connect_plan.service_name).await
        }
    }
    .map_err(anyhow::Error::from)
}

pub async fn lookup_installed_app(
    device: &ConnectedDevice,
    bundle_id: &str,
) -> Result<InstalledAppInfo> {
    let stream = device
        .connect_service(ios_core::apps::INSTALLATION_PROXY_SERVICE)
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
    let stream = device
        .connect_service(ios_core::afc::house_arrest::SERVICE_NAME)
        .await
        .context("failed to connect legacy House Arrest for XCTest configuration")?;
    let house_arrest = HouseArrestClient::new(stream);
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
    let stream = device
        .connect_service(ios_core::apps::INSTALLATION_PROXY_SERVICE)
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
                tracing::debug!("device port not ready yet: {err}");
                tokio::time::sleep(DEVICE_PORT_POLL_INTERVAL).await;
            }
        }
    }
}

#[cfg(test)]
mod tests {
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
}
