//! Command grammar and dispatch; workflows and server owners do the work.
use super::cdp::run_cdp_server;
use super::tasks::{finite_duration, run_eval, run_js_shell, run_launch, run_opened_tabs};
use super::webdriver::run_selenium_server;
use super::workflows::duration_from_secs;
use crate::access::{HttpBoundaryArgs, ServerAccessArgs};
use anyhow::{anyhow, Result};
use ios_core::webinspector::SAFARI_BUNDLE_ID;

#[derive(clap::Args)]
pub struct WebInspectorCmd {
    #[command(subcommand)]
    sub: WebInspectorSub,
}

#[derive(clap::Subcommand)]
pub(super) enum WebInspectorSub {
    OpenedTabs {
        #[arg(short = 't', long, default_value = "3.0")]
        timeout: f64,
    },
    Eval {
        expression: String,
        #[arg(long)]
        app_id: Option<String>,
        #[arg(long)]
        bundle_id: Option<String>,
        #[arg(long)]
        page_id: String,
        #[arg(short = 't', long, default_value = "3.0")]
        timeout: f64,
    },
    /// Launch an app through Remote Automation and optionally navigate it.
    Launch {
        #[arg(value_name = "URL")]
        url: Option<String>,
        #[arg(long, default_value = SAFARI_BUNDLE_ID)]
        bundle_id: String,
        #[arg(short = 't', long, default_value = "5.0")]
        timeout: f64,
    },
    /// Evaluate JavaScript line-by-line in an inspectable page.
    JsShell {
        #[arg(value_name = "URL")]
        url: Option<String>,
        #[arg(long)]
        bundle_id: Option<String>,
        #[arg(long)]
        page_id: Option<String>,
        #[arg(long)]
        open_safari: bool,
        #[arg(
            long,
            default_value_t = true,
            action = clap::ArgAction::Set,
            help = "Continue after an evaluation error (use --continue-on-error=false to stop)"
        )]
        continue_on_error: bool,
        #[arg(short = 't', long, default_value = "5.0")]
        timeout: f64,
    },
    Cdp {
        #[arg(long, default_value = "127.0.0.1")]
        host: String,
        #[arg(long, default_value_t = 9222)]
        port: u16,
        #[arg(short = 't', long, default_value = "3.0")]
        timeout: f64,
        #[command(flatten)]
        access: ServerAccessArgs,
        #[command(flatten)]
        boundary: HttpBoundaryArgs,
    },
    Selenium {
        #[arg(long, default_value = "127.0.0.1")]
        host: String,
        #[arg(long, default_value_t = 4444)]
        port: u16,
        #[arg(short = 't', long, default_value = "3.0")]
        timeout: f64,
        #[command(flatten)]
        access: ServerAccessArgs,
        #[command(flatten)]
        boundary: HttpBoundaryArgs,
    },
}

impl WebInspectorCmd {
    /// Finite commands own default selection in their total/signal scope;
    /// debug listeners still require an explicit selector before server setup.
    /// Interactive shell selection remains on its existing path for now.
    pub(crate) fn needs_default_udid(&self) -> bool {
        matches!(self.sub, WebInspectorSub::JsShell { .. })
    }

    pub async fn run(self, udid: Option<String>, json_output: bool) -> Result<()> {
        match self.sub {
            WebInspectorSub::OpenedTabs { timeout } => {
                run_opened_tabs(udid, finite_duration(timeout)?, json_output).await
            }
            WebInspectorSub::Eval {
                expression,
                app_id,
                bundle_id,
                page_id,
                timeout,
            } => {
                run_eval(
                    udid,
                    app_id,
                    bundle_id.unwrap_or_else(|| SAFARI_BUNDLE_ID.to_string()),
                    &page_id,
                    &expression,
                    finite_duration(timeout)?,
                    json_output,
                )
                .await
            }
            WebInspectorSub::Launch {
                url,
                bundle_id,
                timeout,
            } => {
                run_launch(
                    udid,
                    &bundle_id,
                    url.as_deref(),
                    finite_duration(timeout)?,
                    json_output,
                )
                .await
            }
            WebInspectorSub::JsShell {
                url,
                bundle_id,
                page_id,
                open_safari,
                continue_on_error,
                timeout,
            } => {
                let udid = udid.ok_or_else(|| anyhow!("--udid required for webinspector"))?;
                run_js_shell(
                    &udid,
                    url.as_deref(),
                    bundle_id.as_deref(),
                    page_id.as_deref(),
                    open_safari,
                    continue_on_error,
                    duration_from_secs(timeout)?,
                    json_output,
                )
                .await
            }
            WebInspectorSub::Cdp {
                host,
                port,
                timeout,
                access,
                boundary,
            } => {
                let udid = udid.ok_or_else(|| anyhow!("--udid required for webinspector"))?;
                run_cdp_server(
                    &udid,
                    host,
                    port,
                    duration_from_secs(timeout)?,
                    access,
                    boundary,
                )
                .await
            }
            WebInspectorSub::Selenium {
                host,
                port,
                timeout,
                access,
                boundary,
            } => {
                let udid = udid.ok_or_else(|| anyhow!("--udid required for webinspector"))?;
                run_selenium_server(
                    &udid,
                    host,
                    port,
                    duration_from_secs(timeout)?,
                    access,
                    boundary,
                )
                .await
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::Parser;
    #[derive(Parser)]
    struct TestCli {
        #[command(subcommand)]
        command: WebInspectorSub,
    }

    #[test]
    fn parses_opened_tabs_subcommand() {
        assert!(TestCli::try_parse_from(["webinspector", "opened-tabs", "--timeout", "2"]).is_ok());
    }

    #[test]
    fn parses_eval_subcommand() {
        assert!(TestCli::try_parse_from([
            "webinspector",
            "eval",
            "1+1",
            "--page-id",
            "7",
            "--bundle-id",
            "com.apple.mobilesafari"
        ])
        .is_ok());
    }

    #[test]
    fn parses_launch_and_js_shell_subcommands() {
        let launch = TestCli::try_parse_from([
            "webinspector",
            "launch",
            "https://example.com/路径",
            "--bundle-id",
            "com.example.app",
            "--timeout",
            "15",
        ])
        .expect("launch command should parse");
        assert!(matches!(launch.command, WebInspectorSub::Launch { .. }));

        let shell = TestCli::try_parse_from([
            "webinspector",
            "js-shell",
            "https://example.com",
            "--bundle-id",
            "com.example.app",
            "--page-id",
            "7",
            "--continue-on-error",
            "false",
        ])
        .expect("js-shell command should parse");
        let WebInspectorSub::JsShell {
            page_id,
            continue_on_error,
            ..
        } = shell.command
        else {
            panic!("expected js-shell command");
        };
        assert_eq!(page_id.as_deref(), Some("7"));
        assert!(!continue_on_error);
    }

    #[test]
    fn parses_cdp_subcommand() {
        assert!(TestCli::try_parse_from([
            "webinspector",
            "cdp",
            "--host",
            "127.0.0.1",
            "--port",
            "9222"
        ])
        .is_ok());
    }

    #[test]
    fn parses_selenium_subcommand() {
        assert!(TestCli::try_parse_from([
            "webinspector",
            "selenium",
            "--host",
            "127.0.0.1",
            "--port",
            "4444",
            "--timeout",
            "5"
        ])
        .is_ok());
    }
}
