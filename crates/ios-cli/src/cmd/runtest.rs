use std::collections::HashMap;
use std::future::Future;
use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::{Context, Result};
use ios_core::apps::{AppInfo, InstallationProxy};
use ios_core::device::{ConnectOptions, ConnectedDevice, ServiceStream};
use ios_core::instruments::process_control::ProcessControl;
use ios_core::testmanager::workflow::{InstalledAppInfo, TestLaunchPlan};
use ios_core::testmanager::xctestrun::{parse_xctestrun_file, TestConfiguration};
use ios_core::testmanager::TestmanagerClient;
use ios_core::MuxClient;
use ios_core::TunMode;
use uuid::Uuid;

#[derive(clap::Args)]
pub struct RunTestCmd {
    #[arg(help = "Path to the .xctestrun file")]
    pub xctestrun: PathBuf,
    #[arg(long, default_value_t = 30, help = "Startup timeout in seconds")]
    pub startup_timeout_secs: u64,
}

impl RunTestCmd {
    pub async fn run(self, udid: Option<String>) -> Result<()> {
        let udid = udid.ok_or_else(|| anyhow::anyhow!("--udid required for runtest"))?;
        eprintln!("UNTESTED: XCTest execution workflow has automated coverage, but no real-device validation in this workspace yet.");

        let device = connect_testmanager_device(&udid).await?;
        let configs = parse_xctestrun_file(&self.xctestrun)
            .with_context(|| format!("failed to parse {}", self.xctestrun.display()))?;
        let (configuration_name, plan) =
            build_plan_from_xctestrun(&device, &self.xctestrun, &configs).await?;
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
    _instruments_device: ConnectedDevice,
    _process_control: ProcessControl<ServiceStream>,
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
}

pub async fn start_test_plan(udid: &str, plan: TestLaunchPlan) -> Result<TestStartupResult> {
    Ok(start_test_plan_session(udid, plan)
        .await?
        .startup_result()
        .clone())
}

pub async fn start_test_plan_session(udid: &str, plan: TestLaunchPlan) -> Result<ActiveTestPlan> {
    let device = connect_testmanager_device(udid).await?;
    let product_version = device.product_version().await?;
    if product_version.major < 17 {
        anyhow::bail!("runtest is currently implemented only for iOS 17+ tunnel/RSD devices");
    }

    let session_stream = device
        .connect_rsd_service(ios_core::testmanager::SERVICE_NAME)
        .await
        .context("failed to connect testmanager session stream")?;
    let control_stream = device
        .connect_rsd_service(ios_core::testmanager::SERVICE_NAME)
        .await
        .context("failed to connect testmanager control stream")?;
    let mut testmanager = TestmanagerClient::connect(session_stream, control_stream)
        .await
        .map_err(|err| anyhow::anyhow!("DTX error: {err}"))?;

    let session_id = Uuid::new_v4();
    let configuration = plan.xctest_configuration(product_version.major, session_id);
    let capabilities = configuration.ide_capabilities.clone();
    testmanager
        .initiate_control_session_with_capabilities(capabilities.clone())
        .await
        .map_err(|err| anyhow::anyhow!("control session error: {err}"))?;
    testmanager
        .initiate_session_with_capabilities(session_id, capabilities)
        .await
        .map_err(|err| anyhow::anyhow!("session init error: {err}"))?;

    let (instruments_device, instruments_stream) =
        crate::cmd::instruments::connect_instruments(udid).await?;
    let mut process_control = ProcessControl::connect(instruments_stream)
        .await
        .map_err(|err| anyhow::anyhow!("process control error: {err}"))?;
    let launch_args = plan.launch_arguments();
    let launch_arg_refs: Vec<&str> = launch_args.iter().map(String::as_str).collect();
    let launch_env = plan.launch_environment(product_version.major, session_id);
    let launch_options = plan.launch_options(product_version.major);
    let pid = process_control
        .launch_with_options(
            &plan.runner.bundle_id,
            &launch_arg_refs,
            &launch_env,
            &launch_options,
        )
        .await
        .map_err(|err| anyhow::anyhow!("launch error: {err}"))?;

    let summary = testmanager
        .authorize_and_start_test_plan_with_configuration(pid, configuration)
        .await
        .map_err(|err| anyhow::anyhow!("startup handshake error: {err}"))?;

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
        _process_control: process_control,
    })
}

pub async fn connect_testmanager_device(udid: &str) -> Result<ConnectedDevice> {
    let opts = ConnectOptions {
        tun_mode: TunMode::Userspace,
        pair_record_path: None,
        skip_tunnel: false,
    };
    ios_core::connect(udid, opts)
        .await
        .context("failed to establish device tunnel for testmanager")
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

async fn build_plan_from_xctestrun(
    device: &ConnectedDevice,
    xctestrun_path: &Path,
    configs: &[TestConfiguration],
) -> Result<(String, TestLaunchPlan)> {
    let config = configs.first().ok_or_else(|| {
        anyhow::anyhow!(
            "no test configuration found in {}",
            xctestrun_path.display()
        )
    })?;
    let scheme = config.test_targets.first().ok_or_else(|| {
        anyhow::anyhow!("test configuration {:?} has no TestTargets", config.name)
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
    use std::sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    };
    use std::time::Duration;

    use clap::Parser;
    use ios_core::apps::AppInfo;
    use ios_core::testmanager::xctestrun::SchemeData;
    use plist::Value;

    use super::{infer_target_bundle_id, wait_for_ready, RunTestCmd};

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
