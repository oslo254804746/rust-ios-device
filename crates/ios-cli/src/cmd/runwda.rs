use std::future::Future;
#[cfg(test)]
use std::pin::Pin;
use std::time::Duration;

use anyhow::{Context, Result};
use ios_core::testmanager::workflow::{InstalledAppInfo, TestLaunchPlan};
use tokio::time::Instant;

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
    #[arg(
        short = 'H',
        long,
        default_value = "127.0.0.1",
        help = "Literal loopback IP or localhost for the raw TCP proxy"
    )]
    pub host: String,
    #[arg(long, default_value_t = 30)]
    pub startup_timeout_secs: u64,
    #[arg(
        long,
        help = "Stop WDA and release its services after one forwarded connection"
    )]
    pub once: bool,
}

impl RunWdaCmd {
    pub async fn run(self, udid: Option<String>) -> Result<()> {
        let udid = udid.ok_or_else(|| anyhow::anyhow!("--udid required for runwda"))?;
        crate::access::loopback_address(&self.host)?;
        crate::cmd::forward::validate_forward_ports(self.host_port, self.device_port)?;
        anyhow::ensure!(
            self.startup_timeout_secs > 0,
            "WDA startup timeout must be greater than zero"
        );
        let deadline = Instant::now()
            .checked_add(Duration::from_secs(self.startup_timeout_secs))
            .ok_or_else(|| crate::output::input_decision("WDA startup timeout is too large"))?;
        crate::output::write_error_line(format_args!("UNTESTED: WebDriverAgent startup and port proxy have not been real-device validated in this workspace."))?;
        let native = crate::cmd::tunnel::NativeScope::new()?;
        let mut cleanup = crate::cmd::runtest::TestStartupCleanup::default();
        let mut session: Option<crate::cmd::runtest::ActiveTestPlan> = None;
        let startup = Box::pin(async {
            let device = wda_until(
                deadline,
                crate::cmd::runtest::connect_testmanager_device_scoped(&udid, &native),
            )
            .await?;
            let runner = wda_until(
                deadline,
                crate::cmd::runtest::lookup_installed_app(&device, &self.runner_bundle_id),
            )
            .await?;
            let target = match &self.target_bundle_id {
                Some(bundle) => Some(
                    wda_until(
                        deadline,
                        crate::cmd::runtest::lookup_installed_app(&device, bundle),
                    )
                    .await?,
                ),
                None => None,
            };
            let plan = TestLaunchPlan {
                xctest_bundle_name: infer_wda_bundle_name(&runner),
                runner,
                target,
                is_xctest: false,
                args: Vec::new(),
                env: std::collections::HashMap::new(),
                tests_to_run: Vec::new(),
                tests_to_skip: Vec::new(),
            };
            retain_startup_until(
                deadline,
                &mut session,
                crate::cmd::runtest::start_wda_session_owned_until(
                    &udid,
                    plan,
                    deadline,
                    &mut cleanup,
                    &native,
                ),
            )
            .await?;
            wda_until(
                deadline,
                session
                    .as_ref()
                    .expect("active WDA owner stored")
                    .wait_for_device_port_ready(self.device_port),
            )
            .await
        });
        let startup = tokio::select! {
            biased;
            signal = tokio::signal::ctrl_c() => {
                signal.context("waiting for WDA startup cancellation")
                    .and_then(|_| Err(ios_core::OperationCancelled::new().into()))
            },
            result = tokio::time::timeout_at(deadline, startup) => result
                .map_err(|elapsed| anyhow::Error::new(elapsed).context("WDA startup/readiness absolute deadline expired"))
                .and_then(|result| result),
        };
        let mut outcome = match startup {
            Ok(()) => {
                // Scoped forwarding owns its Ctrl-C and original child joins.
                // Dropping it in an outer select would bypass that cleanup.
                Box::pin(crate::cmd::forward::run_port_forward_scoped(
                    &udid,
                    self.host_port,
                    self.device_port,
                    &self.host,
                    self.once,
                    &native,
                ))
                .await
            }
            Err(error) => Err(error),
        };
        outcome = crate::cmd::test_workflow::combine_results(
            outcome,
            cleanup.close().await,
            "WDA original startup cleanup failed",
        );
        if let Some(active) = &mut session {
            active.terminate().await;
            if let Some(error) = active.take_termination_error() {
                outcome = crate::cmd::test_workflow::combine_results(
                    outcome,
                    Err(error),
                    "WDA original runner cleanup failed",
                );
            }
        }
        drop(session);
        crate::cmd::test_workflow::combine_results(
            outcome,
            native.close(Duration::from_secs(5)).await,
            "WDA original native cleanup failed",
        )
    }
}

// Store the original owner inside the guarded poll, before the clock can reject
// a late success. The caller's finally retains its confirmed PID and services.
fn retain_startup_until<'a, T: 'a, F>(
    deadline: Instant,
    owner: &'a mut Option<T>,
    future: F,
) -> impl Future<Output = Result<()>> + 'a
where
    F: Future<Output = Result<T>> + 'a,
{
    wda_until(deadline, async move {
        *owner = Some(future.await?);
        Ok(())
    })
}

fn wda_until<'a, T: 'a, F>(deadline: Instant, future: F) -> impl Future<Output = Result<T>> + 'a
where
    F: Future<Output = Result<T>> + 'a,
{
    let mut future = Box::pin(future);
    async move {
        let guarded = std::future::poll_fn(|cx| {
            if Instant::now() >= deadline {
                return std::task::Poll::Pending;
            }
            match future.as_mut().poll(cx) {
                std::task::Poll::Ready(Ok(_)) if Instant::now() >= deadline => {
                    std::task::Poll::Pending
                }
                result => result,
            }
        });
        tokio::time::timeout_at(deadline, guarded)
            .await
            .map_err(|elapsed| {
                anyhow::Error::new(elapsed).context("WDA startup phase absolute deadline expired")
            })?
    }
}

#[cfg(test)]
type ReadyFuture<'a> = Pin<Box<dyn Future<Output = Result<()>> + 'a>>;

#[cfg(test)]
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

    #[tokio::test]
    async fn invalid_bind_and_ports_fail_before_wda_device_startup() {
        for args in [
            vec!["--host", "0.0.0.0"],
            vec!["--host-port", "0"],
            vec!["--device-port", "0"],
        ] {
            let mut command = vec!["runwda", "com.synthetic.wda.xctrunner"];
            command.extend(args);
            let error = TestCli::parse_from(command)
                .command
                .run(Some("synthetic-device".into()))
                .await
                .unwrap_err();
            assert!(error.chain().any(|cause| {
                let message = cause.to_string();
                message.contains("loopback")
                    || message.contains("invalid host port")
                    || message.contains("invalid device port")
            }));
        }
    }

    #[tokio::test]
    async fn zero_and_overflow_startup_deadlines_fail_before_any_device_io() {
        for timeout in ["0", "18446744073709551615"] {
            let command = TestCli::parse_from([
                "runwda",
                "com.synthetic.wda.xctrunner",
                "--startup-timeout-secs",
                timeout,
            ]);
            let error = command
                .command
                .run(Some("owned-budget-fixture".into()))
                .await
                .unwrap_err();
            assert!(format!("{error:#}").contains("timeout"));
            assert!(!error.is::<ios_core::MuxError>());
        }
    }

    #[tokio::test]
    async fn readiness_timeout_retains_original_elapsed_and_skips_expired_phase_io() {
        use std::sync::atomic::AtomicUsize;
        let calls = AtomicUsize::new(0);
        let error = super::wda_until(tokio::time::Instant::now(), async {
            calls.fetch_add(1, Ordering::SeqCst);
            Ok::<_, anyhow::Error>(())
        })
        .await
        .unwrap_err();
        assert!(error.is::<tokio::time::error::Elapsed>());
        assert_eq!(calls.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn late_startup_success_retains_original_owner_for_finally() {
        struct Owner(Arc<AtomicBool>);
        impl Drop for Owner {
            fn drop(&mut self) {
                self.0.store(true, Ordering::SeqCst);
            }
        }
        let dropped = Arc::new(AtomicBool::new(false));
        let mut owner = None;
        let deadline = tokio::time::Instant::now() + std::time::Duration::from_millis(20);
        let error = super::retain_startup_until(deadline, &mut owner, async {
            // Cross the deadline in the same poll that confirms the owner.
            std::thread::sleep(std::time::Duration::from_millis(40));
            Ok(Owner(dropped.clone()))
        })
        .await
        .unwrap_err();
        assert!(error.is::<tokio::time::error::Elapsed>());
        assert!(owner.is_some());
        assert!(!dropped.load(Ordering::SeqCst));
        drop(owner.take());
        assert!(dropped.load(Ordering::SeqCst));
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
