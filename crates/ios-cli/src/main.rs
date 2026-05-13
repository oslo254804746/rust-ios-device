//! ios-cli: Command-line interface for iOS device management.

mod cmd;
mod output;

use std::future::Future;
use std::pin::Pin;

use anyhow::{Context, Result};
use clap::Parser;

#[derive(clap::Parser)]
#[command(name = "ios", about = "iOS device management CLI (supports iOS 17+)")]
struct Cli {
    #[arg(
        short = 'u',
        long,
        env = "IOS_UDID",
        help = "Device UDID; defaults to the first connected device when omitted"
    )]
    udid: Option<String>,
    #[arg(long, help = "Disable JSON output, use table format")]
    no_json: bool,
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
    /// Listen for usbmux attach/detach events
    Listen(cmd::listen::ListenCmd),
    /// Access lockdown values directly
    Lockdown(cmd::lockdown::LockdownCmd),
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
    /// Start WebDriverAgent and forward its HTTP port locally
    Runwda(cmd::runwda::RunWdaCmd),
    /// Send HTTP commands to a running WebDriverAgent endpoint or device port
    Wda(cmd::wda::WdaCmd),
    /// Inspect Safari/WebView pages via WebInspector
    Webinspector(cmd::webinspector::WebInspectorCmd),
    /// List and download device symbols
    Symbols(cmd::symbols::SymbolsCmd),
}

type CommandFuture = Pin<Box<dyn Future<Output = Result<()>>>>;

fn dispatch_command(command: Commands, udid: Option<String>, no_json: bool) -> CommandFuture {
    match command {
        Commands::List(c) => Box::pin(async move { c.run(!no_json).await }),
        Commands::AccessibilityAudit(c) => Box::pin(async move { c.run(udid, !no_json).await }),
        Commands::Arbitration(c) => Box::pin(async move { c.run(udid, !no_json).await }),
        Commands::Activation(c) => Box::pin(async move { c.run(udid, !no_json).await }),
        Commands::Amfi(c) => Box::pin(async move { c.run(udid, !no_json).await }),
        Commands::Info(c) => Box::pin(async move { c.run(udid, !no_json).await }),
        Commands::Listen(c) => Box::pin(async move { c.run(!no_json).await }),
        Commands::Lockdown(c) => Box::pin(async move { c.run(udid, !no_json).await }),
        Commands::Tunnel(c) => Box::pin(async move { c.run(udid).await }),
        Commands::Pair(c) => Box::pin(async move { c.run(udid, !no_json).await }),
        Commands::Provisioning(c) => Box::pin(async move { c.run(udid, !no_json).await }),
        Commands::Profiles(c) => Box::pin(async move { c.run(udid, !no_json).await }),
        Commands::Rsd(c) => Box::pin(async move { c.run(udid, !no_json).await }),
        Commands::Apps(c) => Box::pin(async move { c.run(udid, !no_json).await }),
        Commands::Backup(c) => Box::pin(async move { c.run(udid, !no_json).await }),
        Commands::Batterycheck(c) => Box::pin(async move { c.run(udid, !no_json).await }),
        Commands::Batteryregistry(c) => Box::pin(async move { c.run(udid, !no_json).await }),
        Commands::Companion(c) => Box::pin(async move { c.run(udid, !no_json).await }),
        Commands::Crash(c) => Box::pin(async move { c.run(udid).await }),
        Commands::Debug(c) => Box::pin(async move { c.run(udid).await }),
        Commands::Debugserver(c) => Box::pin(async move { c.run(udid).await }),
        Commands::Devicestate(c) => Box::pin(async move { c.run(udid).await }),
        Commands::Diagnostics(c) => Box::pin(async move { c.run(udid, !no_json).await }),
        Commands::Discover(c) => Box::pin(async move { c.run(!no_json).await }),
        Commands::Diskspace(c) => Box::pin(async move { c.run(udid, !no_json).await }),
        Commands::Erase(c) => Box::pin(async move { c.run(udid, !no_json).await }),
        Commands::File(c) => Box::pin(async move { c.run(udid, !no_json).await }),
        Commands::FileRelay(c) => Box::pin(async move { c.run(udid).await }),
        Commands::Dproxy(c) => Box::pin(async move { c.run(udid).await }),
        Commands::Forward(c) => Box::pin(async move { c.run(udid).await }),
        Commands::Heartbeat(c) => Box::pin(async move { c.run(udid, !no_json).await }),
        Commands::Httpproxy(c) => Box::pin(async move { c.run(udid, !no_json).await }),
        Commands::Idam(c) => Box::pin(async move { c.run(udid, !no_json).await }),
        Commands::Screenshot(c) => Box::pin(async move { c.run(udid).await }),
        Commands::Springboard(c) => Box::pin(async move { c.run(udid).await }),
        Commands::Syslog(c) => Box::pin(async move { c.run(udid, !no_json).await }),
        Commands::Notify(c) => Box::pin(async move { c.run(udid).await }),
        Commands::Pcap(c) => Box::pin(async move { c.run(udid).await }),
        Commands::Instruments(c) => Box::pin(async move { c.run(udid).await }),
        Commands::Location(c) => Box::pin(async move { c.run(udid).await }),
        Commands::Memlimitoff(c) => Box::pin(async move { c.run(udid).await }),
        Commands::Mobilegestalt(c) => Box::pin(async move { c.run(udid, !no_json).await }),
        Commands::Ddi(c) => Box::pin(async move { c.run(udid, !no_json).await }),
        Commands::OsTrace(c) => Box::pin(async move { c.run(udid, !no_json).await }),
        Commands::PowerAssert(c) => Box::pin(async move { c.run(udid, !no_json).await }),
        Commands::Preboard(c) => Box::pin(async move { c.run(udid, !no_json).await }),
        Commands::Prepare(c) => Box::pin(async move { c.run(udid, !no_json).await }),
        Commands::Restore(c) => Box::pin(async move { c.run(udid, !no_json).await }),
        Commands::Runtest(c) => Box::pin(async move { c.run(udid).await }),
        Commands::Runwda(c) => Box::pin(async move { c.run(udid).await }),
        Commands::Wda(c) => Box::pin(async move { c.run(udid, !no_json).await }),
        Commands::Webinspector(c) => Box::pin(async move { c.run(udid, !no_json).await }),
        Commands::Symbols(c) => Box::pin(async move { c.run(udid, !no_json).await }),
    }
}

fn command_needs_default_udid(command: &Commands) -> bool {
    match command {
        Commands::List(_) | Commands::Listen(_) | Commands::Discover(_) => false,
        Commands::Pair(command) => command.needs_default_udid(),
        Commands::Prepare(command) => command.needs_default_udid(),
        Commands::Wda(command) => command.needs_default_udid(),
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

    let devices = ios_core::list_devices()
        .await
        .context("failed to list connected iOS devices for default --udid")?;
    default_udid_from_devices(&devices)
        .map(Some)
        .ok_or_else(|| anyhow::anyhow!("--udid required: no connected iOS devices found"))
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();

    let level = match cli.verbose {
        0 => tracing::Level::WARN,
        1 => tracing::Level::INFO,
        2 => tracing::Level::DEBUG,
        _ => tracing::Level::TRACE,
    };
    tracing_subscriber::fmt().with_max_level(level).init();

    let udid = resolve_cli_udid(cli.udid, &cli.command).await?;
    dispatch_command(cli.command, udid, cli.no_json).await
}

#[cfg(test)]
mod tests {
    use std::mem::size_of_val;

    use super::*;

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
    fn info_command_resolves_default_udid() {
        let cli = Cli::try_parse_from(["ios", "info"]).expect("info command should parse");

        assert!(command_needs_default_udid(&cli.command));
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
}
