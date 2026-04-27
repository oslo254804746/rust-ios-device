use std::future::Future;
use std::pin::Pin;
use std::time::Duration;

use anyhow::Result;
use ios_core::testmanager::workflow::{InstalledAppInfo, TestLaunchPlan};

#[derive(clap::Args)]
pub struct RunWdaCmd {
    #[arg(help = "Installed WebDriverAgent runner bundle identifier")]
    pub runner_bundle_id: String,
    #[arg(long, help = "Target app bundle identifier for UI testing")]
    pub target_bundle_id: Option<String>,
    #[arg(long, default_value_t = 8100)]
    pub host_port: u16,
    #[arg(long, default_value_t = 8100)]
    pub device_port: u16,
    #[arg(short = 'H', long, default_value = "127.0.0.1")]
    pub host: String,
    #[arg(long, default_value_t = 30)]
    pub startup_timeout_secs: u64,
}

impl RunWdaCmd {
    pub async fn run(self, udid: Option<String>) -> Result<()> {
        let udid = udid.ok_or_else(|| anyhow::anyhow!("--udid required for runwda"))?;
        eprintln!("UNTESTED: WebDriverAgent startup and port proxy are implemented from reference flows, but have not been real-device validated in this workspace.");

        let device = crate::cmd::runtest::connect_testmanager_device(&udid).await?;
        let runner =
            crate::cmd::runtest::lookup_installed_app(&device, &self.runner_bundle_id).await?;
        let target = match &self.target_bundle_id {
            Some(bundle_id) => {
                Some(crate::cmd::runtest::lookup_installed_app(&device, bundle_id).await?)
            }
            None => None,
        };
        let xctest_bundle_name = infer_wda_bundle_name(&runner);
        let plan = TestLaunchPlan {
            runner,
            target,
            xctest_bundle_name,
            is_xctest: false,
            args: Vec::new(),
            env: std::collections::HashMap::new(),
            tests_to_run: Vec::new(),
            tests_to_skip: Vec::new(),
        };

        let device_port = self.device_port;
        let startup_timeout = Duration::from_secs(self.startup_timeout_secs);
        let _session = tokio::time::timeout(startup_timeout, async {
            let session = crate::cmd::runtest::start_test_plan_session(&udid, plan).await?;
            keep_session_alive_until_ready(session, |session| {
                Box::pin(session.wait_for_device_port_ready(device_port))
            })
            .await
        })
        .await
        .map_err(|_| {
            anyhow::anyhow!(
                "timed out waiting for WebDriverAgent to listen on device port {device_port}"
            )
        })??;

        crate::cmd::forward::run_port_forward(&udid, self.host_port, device_port, self.host, false)
            .await
    }
}

type ReadyFuture<'a> = Pin<Box<dyn Future<Output = Result<()>> + 'a>>;

async fn keep_session_alive_until_ready<Session>(
    session: Session,
    wait_for_ready: impl for<'a> FnOnce(&'a Session) -> ReadyFuture<'a>,
) -> Result<Session> {
    wait_for_ready(&session).await?;
    Ok(session)
}

fn infer_wda_bundle_name(runner: &InstalledAppInfo) -> String {
    runner
        .executable
        .strip_suffix("-Runner")
        .map(|prefix| format!("{prefix}.xctest"))
        .unwrap_or_else(|| "WebDriverAgentRunner.xctest".to_string())
}

#[cfg(test)]
mod tests {
    use std::sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    };

    use clap::Parser;

    use super::{keep_session_alive_until_ready, RunWdaCmd};

    #[derive(Parser)]
    struct TestCli {
        #[command(flatten)]
        command: RunWdaCmd,
    }

    #[test]
    fn parses_runwda_command() {
        let cmd = TestCli::parse_from([
            "runwda",
            "com.facebook.WebDriverAgentRunner.xctrunner",
            "--target-bundle-id",
            "com.example.Aut",
            "--host-port",
            "18100",
        ]);
        assert_eq!(
            cmd.command.runner_bundle_id,
            "com.facebook.WebDriverAgentRunner.xctrunner"
        );
        assert_eq!(
            cmd.command.target_bundle_id.as_deref(),
            Some("com.example.Aut")
        );
        assert_eq!(cmd.command.host_port, 18100);
    }

    #[tokio::test]
    async fn keep_session_alive_until_ready_holds_session_until_wait_completes() {
        struct DropSpy {
            dropped: Arc<AtomicBool>,
        }

        impl Drop for DropSpy {
            fn drop(&mut self) {
                self.dropped.store(true, Ordering::SeqCst);
            }
        }

        let dropped = Arc::new(AtomicBool::new(false));
        let session = DropSpy {
            dropped: dropped.clone(),
        };

        let session = keep_session_alive_until_ready(session, |session| {
            let dropped = dropped.clone();
            Box::pin(async move {
                let _ = std::ptr::from_ref(session);
                assert!(!dropped.load(Ordering::SeqCst));
                tokio::task::yield_now().await;
                assert!(!dropped.load(Ordering::SeqCst));
                Ok(())
            })
        })
        .await
        .unwrap();

        assert!(!dropped.load(Ordering::SeqCst));
        drop(session);
        assert!(dropped.load(Ordering::SeqCst));
    }
}
