//! ios-cli: Command-line interface for iOS device management.
#![allow(
    clippy::duplicate_mod,
    clippy::result_large_err,
    clippy::unnecessary_unwrap,
    clippy::useless_borrows_in_formatting
)]

mod access;
mod cmd;
mod http_diagnostics;
mod logging;
mod output;

pub use cmd::runtest::RunnerLaunchUnconfirmed;
/// The same orchestrator used by `runtest` and `runxctest`, available to Rust
/// callers with an explicit device, plan resolver, deadline and artifact paths.
pub use cmd::test_workflow::{TestWorkflowArgs, TestWorkflowEngine, TestWorkflowRunOptions};

use std::future::Future;
use std::pin::Pin;
use std::process::ExitCode;

use anyhow::{Context, Result};
use clap::{CommandFactory, FromArgMatches};

#[derive(clap::Parser)]
#[command(
    name = "ios",
    version,
    about = "iOS device management CLI (supports iOS 17+)"
)]
struct Cli {
    #[arg(
        short = 'u',
        long,
        env = "IOS_UDID",
        hide_env_values = true,
        help = "Device UDID; finite test workflows and exact observers require an explicit selection"
    )]
    udid: Option<String>,
    #[arg(long, help = "Disable JSON output, use table format")]
    no_json: bool,
    #[arg(
        long,
        global = true,
        value_enum,
        default_value_t = output::ErrorFormat::Human,
        help = "Failure diagnostics on stderr: human (default) or JSON"
    )]
    error_format: output::ErrorFormat,
    #[arg(short = 'v', action = clap::ArgAction::Count, help = "Increase verbosity")]
    verbose: u8,
    #[command(subcommand)]
    command: Commands,
}

#[derive(clap::Subcommand)]
enum Commands {
    /// List connected devices (USB/Network)
    List(cmd::list::ListCmd),
    /// Accessibility audit and focus inspection over axAudit DTX
    AccessibilityAudit(cmd::accessibility_audit::AccessibilityAuditCmd),
    /// Device arbitration helpers
    Arbitration(cmd::arbitration::ArbitrationCmd),
    /// Activation state helpers
    Activation(cmd::activation::ActivationCmd),
    /// AMFI developer mode helpers
    Amfi(cmd::amfi::AmfiCmd),
    /// Show device information
    Info(cmd::info::InfoCmd),
    /// Discover the device's IPv4/IPv6 addresses from pcapd packets
    Ip(cmd::ip::IpCmd),
    /// Listen for usbmux attach/detach events
    Listen(cmd::listen::ListenCmd),
    /// Access lockdown values directly
    Lockdown(cmd::lockdown::LockdownCmd),
    /// Supervised MCInstall passcode and security operations
    Mdm(cmd::mdm::MdmCmd),
    /// Establish a CDTunnel to the device (iOS 17+ required)
    Tunnel(cmd::tunnel::TunnelCmd),
    /// Pair a new (untrusted) device via SRP
    Pair(cmd::pair::PairCmd),
    /// Inspect installed provisioning profiles
    Provisioning(cmd::provisioning::ProvisioningCmd),
    /// Inspect installed configuration profiles
    Profiles(cmd::profiles::ProfilesCmd),
    /// Inspect Remote Service Discovery (RSD) services
    Rsd(cmd::rsd::RsdCmd),
    /// App management (list, install, uninstall)
    Apps(cmd::apps::AppsCmd),
    /// MobileBackup2 service helpers
    Backup(cmd::backup::BackupCmd),
    /// Capture Bluetooth HCI traffic via BTPacketLogger
    Btlogger(cmd::btlogger::BtLoggerCmd),
    /// Read battery status via lockdown
    Batterycheck(cmd::batterycheck::BatterycheckCmd),
    /// Read detailed battery IORegistry values via diagnostics relay
    Batteryregistry(cmd::batteryregistry::BatteryregistryCmd),
    /// Inspect paired companion devices
    Companion(cmd::companion::CompanionCmd),
    /// Crash report inspection
    Crash(cmd::crash::CrashCmd),
    /// Print LLDB commands for attaching to the remote debugproxy/debugserver workflow
    Debug(cmd::debug::DebugCmd),
    /// Debugserver transport helpers
    Debugserver(cmd::debugserver::DebugserverCmd),
    /// Induce developer device conditions
    Devicestate(cmd::devicestate::DeviceStateCmd),
    /// Diagnostics relay helpers
    Diagnostics(cmd::diagnostics::DiagnosticsCmd),
    /// Discover iOS devices via mDNS/Bonjour
    Discover(cmd::discover::DiscoverCmd),
    /// Diagnose host connectivity and tunnel prerequisites without pairing a device
    Doctor(cmd::doctor::DoctorCmd),
    /// Disk usage from lockdown com.apple.disk_usage
    Diskspace(cmd::diskspace::DiskspaceCmd),
    /// Erase the device via MCInstall
    Erase(cmd::erase::EraseCmd),
    /// File operations via AFC
    File(cmd::file::FileCmd),
    /// Collect a file relay archive from the device
    FileRelay(cmd::file_relay::FileRelayCmd),
    /// Record and decode proxied service traffic
    Dproxy(cmd::dproxy::DproxyCmd),
    /// Forward a local TCP port to a device service port
    Forward(cmd::forward::ForwardCmd),
    /// Exercise the lockdown heartbeat service
    Heartbeat(cmd::heartbeat::HeartbeatCmd),
    /// Inject button, touchscreen, or keyboard input through CoreDevice HID
    Hid(cmd::hid::HidCmd),
    /// Manage the device's supervised global HTTP proxy profile
    Httpproxy(cmd::httpproxy::HttpProxyCmd),
    /// Query or change IDAM configuration
    Idam(cmd::idam::IdamCmd),
    /// Capture a screenshot
    Screenshot(cmd::screenshot::ScreenshotCmd),
    /// Inspect SpringBoard state
    Springboard(cmd::springboard::SpringboardCmd),
    /// Stream device syslog
    Syslog(cmd::syslog::SyslogCmd),
    /// Wait for notification-proxy events
    Notify(cmd::notification::NotificationCmd),
    /// Read or write the iOS 17+ CoreDevice pasteboard
    Pasteboard(cmd::pasteboard::PasteboardCmd),
    /// Capture device network traffic from pcapd
    Pcap(cmd::pcap::PcapCmd),
    /// Performance instruments (CPU, memory, process control)
    Instruments(cmd::instruments::InstrumentsCmd),
    /// Simulate a device location
    Location(cmd::location::LocationCmd),
    /// Disable jetsam memory limits for a process through Instruments
    Memlimitoff(cmd::memlimitoff::MemlimitoffCmd),
    /// Query MobileGestalt keys over diagnostics relay
    Mobilegestalt(cmd::mobilegestalt::MobileGestaltCmd),
    /// Developer Disk Image management (status, mount)
    Ddi(cmd::ddi::DdiCmd),
    /// iOS 17+ CoreDevice configuration and orientation controls
    DeviceControl(cmd::device_control::DeviceControlCmd),
    /// Show the os_trace_relay process list
    OsTrace(cmd::os_trace::OsTraceCmd),
    /// Create a device power assertion
    PowerAssert(cmd::power_assert::PowerAssertCmd),
    /// Interact with preboard stashbag operations
    Preboard(cmd::preboard::PreboardCmd),
    /// Prepare a supervised device or generate supervision certificates
    Prepare(cmd::prepare::PrepareCmd),
    /// Restore-mode helpers exposed through restore service
    Restore(cmd::restore::RestoreCmd),
    /// Start an XCTest runner from a .xctestrun file
    Runtest(cmd::runtest::RunTestCmd),
    /// Start an installed XCTest runner directly by bundle identifier
    Runxctest(cmd::runxctest::RunXcTestCmd),
    /// Start WebDriverAgent and forward its HTTP port locally
    Runwda(cmd::runwda::RunWdaCmd),
    /// Send HTTP commands to a running WebDriverAgent endpoint or device port
    Wda(cmd::wda::WdaCmd),
    /// Inspect Safari/WebView pages via WebInspector
    Webinspector(cmd::webinspector::WebInspectorCmd),
    /// Install or remove a managed Wi-Fi profile
    Wifi(cmd::wifi::WifiCmd),
    /// List and download device symbols
    Symbols(cmd::symbols::SymbolsCmd),
    /// Decoded Web viewer, VNC server, and bidirectional clipboard
    Viewer(cmd::viewer::ViewerCmd),
}

type CommandFuture = Pin<Box<dyn Future<Output = Result<()>>>>;

#[inline(never)]
fn box_command<F>(f: impl FnOnce() -> F) -> CommandFuture
where
    F: Future<Output = Result<()>> + 'static,
{
    Box::pin(f())
}

fn dispatch_command(command: Commands, udid: Option<String>, no_json: bool) -> CommandFuture {
    match command {
        Commands::List(c) => box_command(move || c.run(!no_json)),
        Commands::AccessibilityAudit(c) => box_command(move || c.run(udid, !no_json)),
        Commands::Arbitration(c) => box_command(move || c.run(udid, !no_json)),
        Commands::Activation(c) => box_command(move || c.run(udid, !no_json)),
        Commands::Amfi(c) => box_command(move || c.run(udid, !no_json)),
        Commands::Info(c) => box_command(move || c.run(udid, !no_json)),
        Commands::Ip(c) => box_command(move || c.run(udid, !no_json)),
        Commands::Listen(c) => box_command(move || c.run(!no_json)),
        Commands::Lockdown(c) => box_command(move || c.run(udid, !no_json)),
        Commands::Mdm(c) => box_command(move || c.run(udid, !no_json)),
        Commands::Tunnel(c) => box_command(move || c.run(udid)),
        Commands::Pair(c) => box_command(move || c.run(udid, !no_json)),
        Commands::Provisioning(c) => box_command(move || c.run(udid, !no_json)),
        Commands::Profiles(c) => box_command(move || c.run(udid, !no_json)),
        Commands::Rsd(c) => box_command(move || c.run(udid, !no_json)),
        Commands::Apps(c) => box_command(move || c.run(udid, !no_json)),
        Commands::Backup(c) => box_command(move || c.run(udid, !no_json)),
        Commands::Btlogger(c) => box_command(move || c.run(udid, !no_json)),
        Commands::Batterycheck(c) => box_command(move || c.run(udid, !no_json)),
        Commands::Batteryregistry(c) => box_command(move || c.run(udid, !no_json)),
        Commands::Companion(c) => box_command(move || c.run(udid, !no_json)),
        Commands::Crash(c) => box_command(move || c.run(udid)),
        Commands::Debug(c) => box_command(move || c.run(udid)),
        Commands::Debugserver(c) => box_command(move || c.run(udid)),
        Commands::Devicestate(c) => box_command(move || c.run(udid)),
        Commands::Diagnostics(c) => box_command(move || c.run(udid, !no_json)),
        Commands::Discover(c) => box_command(move || c.run_with_target(udid, !no_json)),
        Commands::Doctor(c) => box_command(move || c.run(udid, !no_json)),
        Commands::Diskspace(c) => box_command(move || c.run(udid, !no_json)),
        Commands::Erase(c) => box_command(move || c.run(udid, !no_json)),
        Commands::File(c) => box_command(move || c.run(udid, !no_json)),
        Commands::FileRelay(c) => box_command(move || c.run(udid)),
        Commands::Dproxy(c) => box_command(move || c.run(udid)),
        Commands::Forward(c) => box_command(move || c.run(udid)),
        Commands::Heartbeat(c) => box_command(move || c.run(udid, !no_json)),
        Commands::Hid(c) => box_command(move || c.run(udid, !no_json)),
        Commands::Httpproxy(c) => box_command(move || c.run(udid, !no_json)),
        Commands::Idam(c) => box_command(move || c.run(udid, !no_json)),
        Commands::Screenshot(c) => box_command(move || c.run(udid)),
        Commands::Springboard(c) => box_command(move || c.run(udid)),
        Commands::Syslog(c) => box_command(move || c.run(udid, !no_json)),
        Commands::Notify(c) => box_command(move || c.run(udid)),
        Commands::Pasteboard(c) => box_command(move || c.run(udid, !no_json)),
        Commands::Pcap(c) => box_command(move || c.run(udid)),
        Commands::Instruments(c) => box_command(move || c.run(udid)),
        Commands::Location(c) => box_command(move || c.run(udid)),
        Commands::Memlimitoff(c) => box_command(move || c.run(udid)),
        Commands::Mobilegestalt(c) => box_command(move || c.run(udid, !no_json)),
        Commands::Ddi(c) => box_command(move || c.run(udid, !no_json)),
        Commands::DeviceControl(c) => box_command(move || c.run(udid, !no_json)),
        Commands::OsTrace(c) => box_command(move || c.run(udid, !no_json)),
        Commands::PowerAssert(c) => box_command(move || c.run(udid, !no_json)),
        Commands::Preboard(c) => box_command(move || c.run(udid, !no_json)),
        Commands::Prepare(c) => box_command(move || c.run(udid, !no_json)),
        Commands::Restore(c) => box_command(move || c.run(udid, !no_json)),
        Commands::Runtest(c) => box_command(move || c.run(udid)),
        Commands::Runxctest(c) => box_command(move || c.run(udid, !no_json)),
        Commands::Runwda(c) => box_command(move || c.run(udid)),
        Commands::Wda(c) => box_command(move || c.run(udid, !no_json)),
        Commands::Webinspector(c) => box_command(move || c.run(udid, !no_json)),
        Commands::Wifi(c) => box_command(move || c.run(udid, !no_json)),
        Commands::Symbols(c) => box_command(move || c.run(udid, !no_json)),
        Commands::Viewer(c) => box_command(move || c.run(udid)),
    }
}

fn command_needs_default_udid(command: &Commands) -> bool {
    match command {
        Commands::List(_) | Commands::Listen(_) | Commands::Discover(_) | Commands::Doctor(_) => {
            false
        }
        // Discovery participates in these commands' setup budget and signal.
        Commands::Notify(_) | Commands::Companion(_) => false,
        Commands::Runtest(_) | Commands::Runxctest(_) | Commands::Runwda(_) => false,
        Commands::Pair(command) => command.needs_default_udid(),
        Commands::Prepare(command) => command.needs_default_udid(),
        Commands::Wda(command) => command.needs_default_udid(),
        Commands::Tunnel(command) => command.needs_default_udid(),
        Commands::Backup(command) => command.needs_default_udid(),
        Commands::Webinspector(command) => command.needs_default_udid(),
        Commands::Rsd(command) => command.needs_default_udid(),
        Commands::Apps(command) => command.needs_default_udid(),
        Commands::Hid(command) => command.needs_default_udid(),
        _ => true,
    }
}

fn default_udid_from_devices(devices: &[ios_core::DeviceInfo]) -> Option<String> {
    devices.first().map(|device| device.udid.clone())
}

async fn resolve_cli_udid(udid: Option<String>, command: &Commands) -> Result<Option<String>> {
    if udid.is_some() || !command_needs_default_udid(command) {
        return Ok(udid);
    }

    let devices =
        tokio::time::timeout(std::time::Duration::from_secs(30), ios_core::list_devices())
            .await
            .context("default device discovery deadline")?
            .context("failed to list connected iOS devices for default --udid")?;
    default_udid_from_devices(&devices)
        .map(Some)
        .ok_or_else(|| anyhow::anyhow!("--udid required: no connected iOS devices found"))
}

/// Recover only parser-recognized format selection before an argument error.
/// Never infer a format by scanning positional values, contexts or error text.
fn error_format_from_failed_args(args: &[std::ffi::OsString]) -> output::ErrorFormat {
    // Recovery must not exit early for help/version before returning the
    // recognized format. The primary parser still owns normal help output.
    fn recovery_command(command: clap::Command) -> clap::Command {
        command
            .disable_help_flag(true)
            .disable_version_flag(true)
            .disable_help_subcommand(true)
            .mut_subcommands(recovery_command)
    }
    recovery_command(Cli::command())
        .ignore_errors(true)
        .try_get_matches_from(args)
        .ok()
        .and_then(|matches| {
            matches
                .get_one::<output::ErrorFormat>("error_format")
                .copied()
        })
        .unwrap_or_default()
}

#[tokio::main]
pub async fn run_cli() -> ExitCode {
    let args: Vec<std::ffi::OsString> = std::env::args_os().collect();
    let matches = match Cli::command().try_get_matches_from(&args) {
        Ok(matches) => matches,
        Err(error) => match error.kind() {
            clap::error::ErrorKind::DisplayHelp | clap::error::ErrorKind::DisplayVersion => {
                if let Err(error) = error.print() {
                    let error = anyhow::Error::new(error);
                    output::report_failure(output::operation_failure_message(
                        error_format_from_failed_args(&args),
                        "help/version output",
                        &error,
                    ));
                    return ExitCode::FAILURE;
                }
                return ExitCode::SUCCESS;
            }
            _ => {
                output::report_failure(output::argument_failure_message(
                    error_format_from_failed_args(&args),
                    &error,
                ));
                return ExitCode::from(2);
            }
        },
    };
    // Only the parser's canonical command name becomes diagnostic context.
    let phase = matches.subcommand_name().unwrap_or("dispatch").to_owned();
    let error_format = matches
        .get_one::<output::ErrorFormat>("error_format")
        .copied()
        .unwrap_or_default();
    let cli = match Cli::from_arg_matches(&matches) {
        Ok(cli) => cli,
        Err(error) => {
            output::report_failure(output::argument_failure_message(error_format, &error));
            return ExitCode::from(2);
        }
    };
    let error_format = cli.error_format;

    let level = match cli.verbose {
        0 => tracing::Level::WARN,
        1 => tracing::Level::INFO,
        2 => tracing::Level::DEBUG,
        _ => tracing::Level::TRACE,
    };
    logging::init(level);

    let result = async {
        if let Commands::Notify(notification) = &cli.command {
            notification.preflight()?;
        }
        if let Commands::Companion(companion) = &cli.command {
            companion.preflight()?;
        }
        let udid = resolve_cli_udid(cli.udid, &cli.command).await?;
        dispatch_command(cli.command, udid, cli.no_json).await
    }
    .await;

    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            output::report_failure(output::operation_failure_message(
                error_format,
                &phase,
                &error,
            ));
            ExitCode::FAILURE
        }
    }
}

#[cfg(test)]
mod tests {
    use clap::Parser;
    use std::mem::size_of_val;

    use super::*;

    #[test]
    fn error_format_recovery_uses_recognized_options_and_keeps_legacy_default() {
        for (args, expected) in [
            (vec!["ios", "unknown-command"], output::ErrorFormat::Human),
            (
                vec!["ios", "--error-format=json", "unknown-command"],
                output::ErrorFormat::Json,
            ),
            (
                vec!["ios", "--error-format", "json"],
                output::ErrorFormat::Json,
            ),
            (
                vec!["ios", "--error-format=json", "--help"],
                output::ErrorFormat::Json,
            ),
            (
                vec!["ios", "--error-format=json", "--version"],
                output::ErrorFormat::Json,
            ),
            (
                vec!["ios", "backup", "--error-format=json", "--help"],
                output::ErrorFormat::Json,
            ),
            (
                vec![
                    "ios",
                    "--error-format",
                    "json",
                    "forward",
                    "private-invalid-port",
                    "62078",
                ],
                output::ErrorFormat::Json,
            ),
            (
                vec!["ios", "--error-format=private-invalid-format", "list"],
                output::ErrorFormat::Human,
            ),
            (
                vec!["ios", "--", "--error-format=json"],
                output::ErrorFormat::Human,
            ),
        ] {
            assert!(Cli::try_parse_from(&args).is_err());
            let args: Vec<std::ffi::OsString> = args.into_iter().map(Into::into).collect();
            assert_eq!(error_format_from_failed_args(&args), expected);
        }
        let cli = Cli::try_parse_from(["ios", "list", "--error-format", "json"]).unwrap();
        assert_eq!(cli.error_format, output::ErrorFormat::Json);
    }

    #[test]
    fn parses_lockdown_get_command() {
        let parsed = Cli::try_parse_from(["ios", "lockdown", "get", "--key", "ProductVersion"]);
        assert!(parsed.is_ok(), "lockdown get command should parse");
    }

    #[test]
    fn parses_lockdown_info_command() {
        let parsed = Cli::try_parse_from(["ios", "lockdown", "info"]);
        assert!(parsed.is_ok(), "lockdown info command should parse");
    }

    #[test]
    fn parses_lockdown_date_command() {
        let parsed = Cli::try_parse_from(["ios", "lockdown", "date"]);
        assert!(parsed.is_ok(), "lockdown date command should parse");
    }

    #[test]
    fn parses_lockdown_device_name_command() {
        let parsed = Cli::try_parse_from(["ios", "lockdown", "device-name"]);
        assert!(parsed.is_ok(), "lockdown device-name command should parse");
    }

    #[test]
    fn parses_lockdown_remove_command() {
        let parsed = Cli::try_parse_from([
            "ios",
            "lockdown",
            "remove",
            "--domain",
            "com.apple.mobile.wireless_lockdown",
            "--key",
            "EnableWifiConnections",
        ]);
        assert!(parsed.is_ok(), "lockdown remove command should parse");
    }

    #[test]
    fn parses_lockdown_save_pair_record_command() {
        let parsed = Cli::try_parse_from([
            "ios",
            "lockdown",
            "save-pair-record",
            "ios-rs-tmp/exported-pair-record.plist",
        ]);
        assert!(
            parsed.is_ok(),
            "lockdown save-pair-record command should parse"
        );
    }

    #[test]
    fn parses_lockdown_heartbeat_command() {
        let parsed = Cli::try_parse_from(["ios", "lockdown", "heartbeat"]);
        assert!(parsed.is_ok(), "lockdown heartbeat command should parse");
    }

    #[test]
    fn parses_lockdown_language_command() {
        let parsed = Cli::try_parse_from(["ios", "lockdown", "language"]);
        assert!(parsed.is_ok(), "lockdown language command should parse");
    }

    #[test]
    fn parses_lockdown_locale_command() {
        let parsed = Cli::try_parse_from(["ios", "lockdown", "locale"]);
        assert!(parsed.is_ok(), "lockdown locale command should parse");
    }

    #[test]
    fn parses_lockdown_voice_over_command() {
        let parsed = Cli::try_parse_from(["ios", "lockdown", "voice-over"]);
        assert!(parsed.is_ok(), "lockdown voice-over command should parse");
    }

    #[test]
    fn parses_lockdown_zoom_touch_command() {
        let parsed = Cli::try_parse_from(["ios", "lockdown", "zoom-touch"]);
        assert!(parsed.is_ok(), "lockdown zoom-touch command should parse");
    }

    #[test]
    fn parses_lockdown_invert_display_command() {
        let parsed = Cli::try_parse_from(["ios", "lockdown", "invert-display"]);
        assert!(
            parsed.is_ok(),
            "lockdown invert-display command should parse"
        );
    }

    #[test]
    fn parses_lockdown_time_format_command() {
        let parsed = Cli::try_parse_from(["ios", "lockdown", "time-format"]);
        assert!(parsed.is_ok(), "lockdown time-format command should parse");
    }

    #[test]
    fn parses_lockdown_developer_mode_command() {
        let parsed = Cli::try_parse_from(["ios", "lockdown", "developer-mode"]);
        assert!(
            parsed.is_ok(),
            "lockdown developer-mode command should parse"
        );
    }

    #[test]
    fn parses_amfi_enable_developer_mode_command() {
        let parsed = Cli::try_parse_from(["ios", "amfi", "enable-developer-mode"]);
        assert!(
            parsed.is_ok(),
            "amfi enable-developer-mode command should parse"
        );
    }

    #[test]
    fn parses_amfi_reveal_developer_mode_command() {
        let parsed = Cli::try_parse_from(["ios", "amfi", "reveal-developer-mode"]);
        assert!(
            parsed.is_ok(),
            "amfi reveal-developer-mode command should parse"
        );
    }

    #[test]
    fn parses_lockdown_wifi_connections_command() {
        let parsed = Cli::try_parse_from(["ios", "lockdown", "wifi-connections"]);
        assert!(
            parsed.is_ok(),
            "lockdown wifi-connections command should parse"
        );
    }

    #[test]
    fn parses_lockdown_start_tunnel_command() {
        let parsed = Cli::try_parse_from(["ios", "lockdown", "start-tunnel", "--script-mode"]);
        assert!(parsed.is_ok(), "lockdown start-tunnel command should parse");
    }

    #[test]
    fn parses_lockdown_time_zone_command() {
        let parsed = Cli::try_parse_from(["ios", "lockdown", "time-zone"]);
        assert!(parsed.is_ok(), "lockdown time-zone command should parse");
    }

    #[test]
    fn parses_listen_command() {
        let parsed = Cli::try_parse_from(["ios", "listen"]);
        assert!(parsed.is_ok(), "listen command should parse");
    }

    #[test]
    fn parses_pair_show_record_command() {
        let parsed = Cli::try_parse_from(["ios", "pair", "show-record"]);
        assert!(parsed.is_ok(), "pair show-record command should parse");
    }

    #[test]
    fn parses_pair_show_record_with_udid_command() {
        let parsed = Cli::try_parse_from([
            "ios",
            "pair",
            "show-record",
            "--udid",
            "00008150-000A584C0E62401C",
        ]);
        assert!(
            parsed.is_ok(),
            "pair show-record --udid <udid> command should parse"
        );
    }

    #[test]
    fn parses_backup_version_command() {
        let parsed = Cli::try_parse_from(["ios", "backup", "version"]);
        assert!(parsed.is_ok(), "backup version command should parse");
    }

    #[test]
    fn parses_backup_encryption_status_command() {
        let parsed = Cli::try_parse_from(["ios", "backup", "encryption-status"]);
        assert!(
            parsed.is_ok(),
            "backup encryption-status command should parse"
        );
    }

    #[test]
    fn parses_arbitration_command() {
        let parsed = Cli::try_parse_from(["ios", "arbitration", "version"]);
        assert!(parsed.is_ok(), "arbitration command should parse");
    }

    #[test]
    fn parses_companion_command() {
        let parsed = Cli::try_parse_from(["ios", "companion", "list"]);
        assert!(parsed.is_ok(), "companion command should parse");
    }

    #[test]
    fn parses_diskspace_command() {
        let parsed = Cli::try_parse_from(["ios", "diskspace"]);
        assert!(parsed.is_ok(), "diskspace command should parse");
    }

    #[test]
    fn parses_file_relay_command() {
        let parsed = Cli::try_parse_from(["ios", "file-relay", "Network"]);
        assert!(parsed.is_ok(), "file-relay command should parse");
    }

    #[test]
    fn parses_httpproxy_command() {
        let parsed = Cli::try_parse_from([
            "ios",
            "httpproxy",
            "set",
            "proxy.example.com",
            "8080",
            "--p12",
            "identity.p12",
        ]);
        assert!(parsed.is_ok(), "httpproxy command should parse");
    }

    #[test]
    fn parses_devicestate_command() {
        let parsed = Cli::try_parse_from(["ios", "devicestate", "list"]);
        assert!(parsed.is_ok(), "devicestate command should parse");
    }

    #[test]
    fn parses_erase_force_command() {
        let parsed = Cli::try_parse_from(["ios", "erase", "--force"]);
        assert!(parsed.is_ok(), "erase --force command should parse");
    }

    #[test]
    fn parses_idam_command() {
        let parsed = Cli::try_parse_from(["ios", "idam", "get"]);
        assert!(parsed.is_ok(), "idam command should parse");
    }

    #[test]
    fn parses_memlimitoff_command() {
        let parsed = Cli::try_parse_from(["ios", "memlimitoff", "123"]);
        assert!(parsed.is_ok(), "memlimitoff command should parse");
    }

    #[test]
    fn parses_memlimitoff_process_name_command() {
        let parsed = Cli::try_parse_from(["ios", "memlimitoff", "--process", "SpringBoard"]);
        assert!(parsed.is_ok(), "memlimitoff --process command should parse");
    }

    #[test]
    fn parses_wifi_and_ip_commands() {
        assert!(Cli::try_parse_from([
            "ios",
            "wifi",
            "install",
            "Office & Guest",
            "--password",
            "secret",
            "--profile-output",
            "profile.mobileconfig",
        ])
        .is_ok());
        assert!(Cli::try_parse_from(["ios", "wifi", "remove", "Office", "--force"]).is_ok());
        assert!(Cli::try_parse_from(["ios", "ip", "--max-packets", "10"]).is_ok());
    }

    #[test]
    fn parses_power_assert_command() {
        let parsed = Cli::try_parse_from(["ios", "power-assert", "--timeout", "10"]);
        assert!(parsed.is_ok(), "power-assert command should parse");
    }

    #[test]
    fn parses_preboard_command() {
        let parsed = Cli::try_parse_from(["ios", "preboard", "create"]);
        assert!(parsed.is_ok(), "preboard command should parse");
    }

    #[test]
    fn parses_symbols_command() {
        let parsed = Cli::try_parse_from(["ios", "symbols", "list"]);
        assert!(parsed.is_ok(), "symbols command should parse");
    }

    #[test]
    fn parses_os_trace_command() {
        let parsed = Cli::try_parse_from(["ios", "os-trace", "ps"]);
        assert!(parsed.is_ok(), "os-trace command should parse");
    }

    #[test]
    fn parses_prepare_create_cert_command() {
        let parsed =
            Cli::try_parse_from(["ios", "prepare", "create-cert", "ios-rs-tmp/supervision"]);
        assert!(parsed.is_ok(), "prepare create-cert command should parse");
    }

    #[test]
    fn parses_prepare_command() {
        let parsed =
            Cli::try_parse_from(["ios", "prepare", "--cert-der", "ios-rs-tmp/supervision.der"]);
        assert!(parsed.is_ok(), "prepare command should parse");
    }

    #[test]
    fn parses_accessibility_audit_capabilities_command() {
        let parsed = Cli::try_parse_from(["ios", "accessibility-audit", "capabilities"]);
        assert!(
            parsed.is_ok(),
            "accessibility-audit capabilities command should parse"
        );
    }

    #[test]
    fn parses_accessibility_audit_list_items_command() {
        let parsed =
            Cli::try_parse_from(["ios", "accessibility-audit", "list-items", "--limit", "10"]);
        assert!(
            parsed.is_ok(),
            "accessibility-audit list-items command should parse"
        );
    }

    #[test]
    fn parses_restore_command() {
        let parsed = Cli::try_parse_from(["ios", "restore", "enter-recovery"]);
        assert!(parsed.is_ok(), "restore command should parse");
    }

    #[test]
    fn parses_wda_status_command() {
        let parsed = Cli::try_parse_from([
            "ios",
            "--udid",
            "00008101-000A5CCC2E90001E",
            "wda",
            "--device-port",
            "8100",
            "status",
            "--base-url",
            "http://127.0.0.1:8100",
        ]);
        assert!(parsed.is_ok(), "wda status command should parse");
    }

    #[test]
    fn parses_dproxy_command() {
        let parsed = Cli::try_parse_from([
            "ios",
            "dproxy",
            "service",
            "com.apple.instruments.dtservicehub",
            "--listen-port",
            "9100",
        ]);
        assert!(parsed.is_ok(), "dproxy command should parse");
    }

    #[test]
    fn parses_forward_command() {
        let parsed = Cli::try_parse_from(["ios", "forward", "1234", "62078", "--once"]);
        assert!(parsed.is_ok(), "forward command should parse");
    }

    #[test]
    fn dispatch_future_fits_within_windows_main_thread_budget() {
        let future = dispatch_command(Commands::List(cmd::list::ListCmd {}), None, false);
        assert!(
            size_of_val(&future) <= 16 * 1024,
            "dispatch future too large for main-thread stack: {} bytes",
            size_of_val(&future)
        );
    }

    #[test]
    fn parses_tunnel_start_script_mode_command() {
        let parsed =
            Cli::try_parse_from(["ios", "tunnel", "start", "--userspace", "--script-mode"]);
        assert!(
            parsed.is_ok(),
            "tunnel start --script-mode command should parse"
        );
    }

    #[test]
    fn dispatch_file_future_fits_within_windows_main_thread_budget() {
        let command = Commands::File(cmd::file::FileCmd::test_ls_command(false));
        let future = dispatch_command(command, Some("test-udid".into()), false);
        assert!(
            size_of_val(&future) <= 16 * 1024,
            "file dispatch future too large for main-thread stack: {} bytes",
            size_of_val(&future)
        );
    }

    #[test]
    fn application_command_state_fits_windows_stack_budget() {
        let cli = Cli::try_parse_from(["ios", "apps", "list", "--app-type", "all"]).unwrap();
        let Commands::Apps(command) = cli.command else {
            unreachable!()
        };
        let operation = command.run(Some("host-fixture".into()), true);
        println!(
            "application state {} connected state {}",
            size_of_val(&operation),
            cmd::apps::AppsCmd::connected_async_state_bytes()
        );
        assert!(
            size_of_val(&operation) <= 128 * 1024,
            "application command async state is {} bytes before boxing",
            size_of_val(&operation)
        );
        assert!(
            cmd::apps::AppsCmd::connected_async_state_bytes() <= 128 * 1024,
            "connected application command embeds oversized service state"
        );
    }

    #[test]
    fn default_udid_uses_first_listed_device() {
        let devices = vec![
            ios_core::DeviceInfo {
                udid: "first-udid".into(),
                device_id: 7,
                connection_type: "Network".into(),
                product_id: 0,
            },
            ios_core::DeviceInfo {
                udid: "second-udid".into(),
                device_id: 8,
                connection_type: "USB".into(),
                product_id: 0,
            },
        ];

        assert_eq!(
            default_udid_from_devices(&devices),
            Some("first-udid".into())
        );
    }

    #[test]
    fn list_command_does_not_resolve_default_udid() {
        let cli = Cli::try_parse_from(["ios", "list"]).expect("list command should parse");

        assert!(!command_needs_default_udid(&cli.command));
    }

    #[test]
    fn springboard_request_budget_is_bounded_and_available_after_subcommand() {
        for args in [
            vec!["ios", "springboard", "orientation"],
            vec![
                "ios",
                "springboard",
                "--request-timeout-ms",
                "25",
                "orientation",
            ],
            vec![
                "ios",
                "springboard",
                "orientation",
                "--request-timeout-ms",
                "25",
            ],
        ] {
            assert!(Cli::try_parse_from(args).is_ok());
        }
        assert!(Cli::try_parse_from([
            "ios",
            "springboard",
            "orientation",
            "--request-timeout-ms",
            "0",
        ])
        .is_err());
    }

    #[test]
    fn doctor_needs_no_device_and_rejects_unbounded_probe_windows() {
        let cli = Cli::try_parse_from(["ios", "doctor", "--mdns", "--timeout", "1"]).unwrap();
        assert!(!command_needs_default_udid(&cli.command));
        for timeout in ["0", "61"] {
            assert!(Cli::try_parse_from(["ios", "doctor", "--timeout", timeout]).is_err());
        }
        assert!(Cli::try_parse_from(["ios", "doctor", "--tunnel-port", "0"]).is_err());
    }

    #[test]
    fn info_command_resolves_default_udid() {
        let cli = Cli::try_parse_from(["ios", "info"]).expect("info command should parse");

        assert!(command_needs_default_udid(&cli.command));
    }

    #[tokio::test]
    async fn exact_observers_do_not_implicitly_discover_a_default_device() {
        for arguments in [
            vec![
                "ios",
                "apps",
                "install-record",
                "com.example.App",
                "--observe-zero-budget-recovery",
                "--rsd-address",
                "[::1]:12345",
            ],
            vec![
                "ios",
                "hid",
                "--confirm",
                "observe-stop",
                "--expected-status-sha256",
                "0000000000000000000000000000000000000000000000000000000000000000",
                "--confirm-stop-all",
            ],
            vec!["ios", "runtest", "fixture.xctestrun"],
            vec![
                "ios",
                "runxctest",
                "--test-runner-bundle-id",
                "com.example.Runner",
                "--xctest-config",
                "Fixture.xctest",
                "--xctest",
            ],
            vec!["ios", "runwda", "com.example.Runner"],
        ] {
            let cli = Cli::try_parse_from(arguments).unwrap();
            assert!(!command_needs_default_udid(&cli.command));
            assert!(resolve_cli_udid(None, &cli.command)
                .await
                .unwrap()
                .is_none());
        }
        let legacy = Cli::try_parse_from(["ios", "apps", "list"]).unwrap();
        assert!(command_needs_default_udid(&legacy.command));
    }

    #[test]
    fn pair_show_record_resolves_default_udid_but_pair_list_does_not() {
        let show_record =
            Cli::try_parse_from(["ios", "pair", "show-record"]).expect("show-record should parse");
        assert!(command_needs_default_udid(&show_record.command));

        let list =
            Cli::try_parse_from(["ios", "pair", "--list"]).expect("pair --list should parse");
        assert!(!command_needs_default_udid(&list.command));
    }

    #[test]
    fn prepare_apply_resolves_default_udid_but_create_cert_does_not() {
        let apply = Cli::try_parse_from(["ios", "prepare"]).expect("prepare should parse");
        assert!(command_needs_default_udid(&apply.command));

        let create_cert =
            Cli::try_parse_from(["ios", "prepare", "create-cert", "ios-rs-tmp/supervision"])
                .expect("prepare create-cert should parse");
        assert!(!command_needs_default_udid(&create_cert.command));
    }

    #[test]
    fn wda_device_port_resolves_default_udid_but_http_mode_does_not() {
        let device_port = Cli::try_parse_from(["ios", "wda", "--device-port", "8100", "status"])
            .expect("wda device-port status should parse");
        assert!(command_needs_default_udid(&device_port.command));

        let http = Cli::try_parse_from(["ios", "wda", "status"]).expect("wda status should parse");
        assert!(!command_needs_default_udid(&http.command));
    }

    #[tokio::test]
    async fn debug_listeners_require_a_selector_without_global_device_discovery() {
        for command in ["cdp", "selenium"] {
            let cli = Cli::try_parse_from(["ios", "webinspector", command]).unwrap();
            assert!(!command_needs_default_udid(&cli.command));
            assert!(resolve_cli_udid(None, &cli.command)
                .await
                .unwrap()
                .is_none());
        }
        let tabs = Cli::try_parse_from(["ios", "webinspector", "opened-tabs"]).unwrap();
        // Finite WebInspector commands now own default selection under their
        // total budget/signal; main must not do an unbounded discovery first.
        assert!(!command_needs_default_udid(&tabs.command));
    }

    #[test]
    fn tunnel_start_and_stop_resolve_default_udid_but_serve_and_list_do_not() {
        let start =
            Cli::try_parse_from(["ios", "tunnel", "start"]).expect("tunnel start should parse");
        assert!(command_needs_default_udid(&start.command));

        let stop =
            Cli::try_parse_from(["ios", "tunnel", "stop"]).expect("tunnel stop should parse");
        assert!(command_needs_default_udid(&stop.command));

        // The manager daemon must come up with no device attached.
        let serve =
            Cli::try_parse_from(["ios", "tunnel", "serve"]).expect("tunnel serve should parse");
        assert!(!command_needs_default_udid(&serve.command));

        let list =
            Cli::try_parse_from(["ios", "tunnel", "list"]).expect("tunnel list should parse");
        assert!(!command_needs_default_udid(&list.command));
    }
}

#[cfg(test)]
mod plist_contract;
