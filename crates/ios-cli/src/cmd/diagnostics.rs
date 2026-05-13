use std::time::Duration;

use anyhow::Result;
use ios_core::TunMode;
use ios_core::{connect, ConnectOptions};

#[derive(clap::Args)]
pub struct DiagnosticsCmd {
    #[command(subcommand)]
    sub: DiagnosticsSub,
}

#[derive(clap::Subcommand)]
enum DiagnosticsSub {
    /// Reboot the device via diagnostics_relay
    Reboot,
    /// Read battery diagnostics from diagnostics_relay
    Battery,
    /// Monitor battery diagnostics from diagnostics_relay
    BatteryMonitor {
        #[arg(long, help = "Stop after collecting this many samples")]
        count: Option<u64>,
        #[arg(
            long,
            default_value_t = 1000,
            help = "Polling interval between samples in milliseconds"
        )]
        interval_ms: u64,
    },
    /// List the full diagnostics payload from diagnostics_relay
    List,
    /// Read a named diagnostics entry from diagnostics_relay All payload
    Show {
        #[arg(help = "Diagnostics entry name (e.g. GasGauge, HDMI)")]
        name: String,
    },
    /// Read GasGauge diagnostics from diagnostics_relay
    Gasgauge,
    /// Read HDMI diagnostics from diagnostics_relay
    Hdmi,
    /// Read WiFi diagnostics from diagnostics_relay
    Wifi,
    /// Read NAND diagnostics from diagnostics_relay
    Nand,
    /// Query a raw IORegistry entry class from diagnostics_relay
    Ioregistry {
        #[arg(
            value_name = "ENTRY_CLASS",
            required_unless_present = "entry_name",
            help = "IORegistry entry class (e.g. IOPlatformExpertDevice)"
        )]
        entry_class: Option<String>,
        #[arg(long = "name", help = "IORegistry entry name (e.g. device-tree)")]
        entry_name: Option<String>,
        #[arg(long = "plane", help = "IORegistry plane (e.g. IODeviceTree)")]
        plane: Option<String>,
    },
    /// Preview iOS 17+ CoreDevice sysdiagnose capture metadata without collecting logs
    Sysdiagnose,
}

impl DiagnosticsCmd {
    pub async fn run(self, udid: Option<String>, json: bool) -> Result<()> {
        let udid = udid.ok_or_else(|| anyhow::anyhow!("--udid required for diagnostics"))?;
        let sub = self.sub;

        if matches!(sub, DiagnosticsSub::Sysdiagnose) {
            return run_coredevice_sysdiagnose_dry_run(&udid, json).await;
        }

        let opts = ConnectOptions {
            tun_mode: TunMode::Userspace,
            pair_record_path: None,
            skip_tunnel: true,
        };
        let device = connect(&udid, opts).await?;
        let mut stream = device
            .connect_service(ios_core::diagnostics::SERVICE_NAME)
            .await?;

        match sub {
            DiagnosticsSub::Reboot => {
                ios_core::diagnostics::reboot(&mut *stream).await?;
                if json {
                    println!(
                        "{}",
                        serde_json::to_string_pretty(&serde_json::json!({
                            "status": "reboot_requested",
                        }))?
                    );
                } else {
                    println!("Reboot request sent.");
                }
            }
            DiagnosticsSub::Battery => {
                let battery = ios_core::diagnostics::query_battery(&mut *stream).await?;
                if json {
                    println!("{}", serde_json::to_string_pretty(&battery)?);
                } else {
                    print_battery(&battery);
                }
            }
            DiagnosticsSub::BatteryMonitor { count, interval_ms } => {
                let limit = count.unwrap_or(u64::MAX);
                let mut collected = 0u64;

                while collected < limit {
                    let battery = ios_core::diagnostics::query_battery(&mut *stream).await?;
                    if json {
                        println!("{}", serde_json::to_string(&battery)?);
                    } else {
                        if collected > 0 {
                            println!();
                        }
                        print_battery(&battery);
                    }

                    collected += 1;
                    if collected >= limit {
                        break;
                    }
                    tokio::time::sleep(Duration::from_millis(interval_ms)).await;
                }
            }
            DiagnosticsSub::List => {
                let value = ios_core::diagnostics::query_all_values(&mut *stream).await?;
                println!("{}", serde_json::to_string_pretty(&value)?);
            }
            DiagnosticsSub::Show { name } => {
                let value = ios_core::diagnostics::query_all_values(&mut *stream).await?;
                let (resolved_name, entry) = resolve_named_diagnostics_entry(&value, &name)?;
                if json {
                    println!("{}", serde_json::to_string_pretty(&entry)?);
                } else {
                    print_named_diagnostics_entry(&resolved_name, &entry);
                }
            }
            DiagnosticsSub::Gasgauge => {
                let value = ios_core::diagnostics::query_all_values(&mut *stream).await?;
                let gas_gauge = extract_named_diagnostics_entry(&value, "GasGauge")?;
                if json {
                    println!("{}", serde_json::to_string_pretty(&gas_gauge)?);
                } else {
                    print_named_diagnostics_entry("GasGauge", &gas_gauge);
                }
            }
            DiagnosticsSub::Hdmi => {
                let value = ios_core::diagnostics::query_all_values(&mut *stream).await?;
                let hdmi = extract_named_diagnostics_entry(&value, "HDMI")?;
                if json {
                    println!("{}", serde_json::to_string_pretty(&hdmi)?);
                } else {
                    print_named_diagnostics_entry("HDMI", &hdmi);
                }
            }
            DiagnosticsSub::Wifi => {
                let value = ios_core::diagnostics::query_all_values(&mut *stream).await?;
                let wifi = extract_named_diagnostics_entry(&value, "WiFi")?;
                if json {
                    println!("{}", serde_json::to_string_pretty(&wifi)?);
                } else {
                    print_named_diagnostics_entry("WiFi", &wifi);
                }
            }
            DiagnosticsSub::Nand => {
                let value = ios_core::diagnostics::query_all_values(&mut *stream).await?;
                let nand = extract_named_diagnostics_entry(&value, "NAND")?;
                if json {
                    println!("{}", serde_json::to_string_pretty(&nand)?);
                } else {
                    print_named_diagnostics_entry("NAND", &nand);
                }
            }
            DiagnosticsSub::Ioregistry {
                entry_class,
                entry_name,
                plane,
            } => {
                let value = ios_core::diagnostics::query_ioregistry_with(
                    &mut *stream,
                    ios_core::diagnostics::IoRegistryQuery {
                        entry_class: entry_class.as_deref(),
                        entry_name: entry_name.as_deref(),
                        current_plane: plane.as_deref(),
                    },
                )
                .await?;
                println!("{}", serde_json::to_string_pretty(&value)?);
            }
            DiagnosticsSub::Sysdiagnose => unreachable!("handled before diagnostics_relay connect"),
        }

        Ok(())
    }
}

async fn run_coredevice_sysdiagnose_dry_run(udid: &str, json: bool) -> Result<()> {
    let opts = ConnectOptions {
        tun_mode: TunMode::Userspace,
        pair_record_path: None,
        skip_tunnel: false,
    };
    let device = connect(udid, opts).await?;
    let xpc = device
        .connect_xpc_service(ios_core::diagnosticsservice::SERVICE_NAME)
        .await
        .map_err(|error| {
            anyhow::anyhow!(
                "CoreDevice diagnosticsservice is unavailable for this device/session: {error}"
            )
        })?;
    let mut client =
        ios_core::diagnosticsservice::DiagnosticsServiceClient::new(xpc, udid.to_string());
    let response = client.capture_sysdiagnose(true).await?;

    if json {
        println!(
            "{}",
            serde_json::to_string_pretty(&serde_json::json!({
                "dry_run": true,
                "preferred_filename": response.preferred_filename,
                "file_size": response.file_size,
            }))?
        );
    } else {
        println!("Dry-run sysdiagnose capture:");
        println!("PreferredFilename: {}", response.preferred_filename);
        println!("ExpectedLength: {}", response.file_size);
    }

    Ok(())
}

fn print_battery(battery: &ios_core::diagnostics::BatteryDiagnostics) {
    if let Some(value) = battery.current_capacity {
        println!("CurrentCapacity: {value}");
    }
    if let Some(value) = battery.is_charging {
        println!("IsCharging: {value}");
    }
    if let Some(value) = battery.temperature {
        println!("Temperature: {value}");
    }
    if let Some(value) = battery.voltage {
        println!("Voltage: {value}");
    }
    if let Some(value) = battery.instant_amperage {
        println!("InstantAmperage: {value}");
    }
    if let Some(value) = battery.cycle_count {
        println!("CycleCount: {value}");
    }
    if let Some(value) = battery.design_capacity {
        println!("DesignCapacity: {value}");
    }
    if let Some(value) = battery.nominal_charge_capacity {
        println!("NominalChargeCapacity: {value}");
    }
    if let Some(value) = battery.absolute_capacity {
        println!("AbsoluteCapacity: {value}");
    }
    if let Some(value) = battery.apple_raw_current_capacity {
        println!("AppleRawCurrentCapacity: {value}");
    }
    if let Some(value) = battery.apple_raw_max_capacity {
        println!("AppleRawMaxCapacity: {value}");
    }
    if let Some(value) = battery.at_warn_level {
        println!("AtWarnLevel: {value}");
    }
    if let Some(value) = battery.at_critical_level {
        println!("AtCriticalLevel: {value}");
    }
}

fn extract_named_diagnostics_entry(value: &plist::Value, key: &str) -> Result<plist::Value> {
    resolve_named_diagnostics_entry(value, key).map(|(_, value)| value)
}

fn resolve_named_diagnostics_entry(
    value: &plist::Value,
    key: &str,
) -> Result<(String, plist::Value)> {
    let dict = value
        .as_dictionary()
        .ok_or_else(|| anyhow::anyhow!("diagnostics payload was not a dictionary"))?;
    if let Some(entry) = dict.get(key) {
        return Ok((key.to_string(), entry.clone()));
    }

    dict.iter()
        .find(|(name, _)| name.eq_ignore_ascii_case(key))
        .map(|(name, value)| (name.clone(), value.clone()))
        .ok_or_else(|| anyhow::anyhow!("diagnostics payload missing {key}"))
}

fn print_named_diagnostics_entry(label: &str, value: &plist::Value) {
    if let Some(dict) = value.as_dictionary() {
        println!("{label}:");
        let mut keys: Vec<_> = dict.keys().cloned().collect();
        keys.sort();
        for key in keys {
            if let Some(entry) = dict.get(&key) {
                println!("  {key}: {}", format_plist_value(entry));
            }
        }
        return;
    }

    println!("{label}: {}", format_plist_value(value));
}

fn format_plist_value(value: &plist::Value) -> String {
    match value {
        plist::Value::String(s) => s.clone(),
        plist::Value::Boolean(v) => v.to_string(),
        plist::Value::Integer(n) => n
            .as_signed()
            .map(|v| v.to_string())
            .or_else(|| n.as_unsigned().map(|v| v.to_string()))
            .unwrap_or_else(|| "0".to_string()),
        plist::Value::Real(v) => v.to_string(),
        other => serde_json::to_string(&plist_to_json(other)).unwrap_or_else(|_| "null".into()),
    }
}

fn plist_to_json(v: &plist::Value) -> serde_json::Value {
    match v {
        plist::Value::String(s) => serde_json::Value::String(s.clone()),
        plist::Value::Boolean(b) => serde_json::Value::Bool(*b),
        plist::Value::Integer(n) => {
            if let Some(i) = n.as_signed() {
                serde_json::json!(i)
            } else {
                serde_json::json!(n.as_unsigned().unwrap_or(0))
            }
        }
        plist::Value::Real(f) => serde_json::json!(f),
        plist::Value::Data(d) => serde_json::Value::String(hex::encode(d)),
        plist::Value::Array(a) => serde_json::Value::Array(a.iter().map(plist_to_json).collect()),
        plist::Value::Dictionary(d) => {
            let mut m = serde_json::Map::new();
            for (k, val) in d {
                m.insert(k.clone(), plist_to_json(val));
            }
            serde_json::Value::Object(m)
        }
        _ => serde_json::Value::Null,
    }
}

#[cfg(test)]
mod tests {
    use clap::Parser;

    use super::*;

    #[derive(Parser)]
    struct TestCli {
        #[command(subcommand)]
        command: DiagnosticsSub,
    }

    #[test]
    fn parses_battery_subcommand() {
        let cmd = TestCli::parse_from(["diagnostics", "battery"]);
        assert!(matches!(cmd.command, DiagnosticsSub::Battery));
    }

    #[test]
    fn parses_reboot_subcommand() {
        let cmd = TestCli::parse_from(["diagnostics", "reboot"]);
        assert!(matches!(cmd.command, DiagnosticsSub::Reboot));
    }

    #[test]
    fn parses_list_subcommand() {
        let cmd = TestCli::parse_from(["diagnostics", "list"]);
        assert!(matches!(cmd.command, DiagnosticsSub::List));
    }

    #[test]
    fn parses_show_subcommand() {
        let cmd = TestCli::parse_from(["diagnostics", "show", "GasGauge"]);
        match cmd.command {
            DiagnosticsSub::Show { name } => assert_eq!(name, "GasGauge"),
            _ => panic!("expected show subcommand"),
        }
    }

    #[test]
    fn parses_gasgauge_subcommand() {
        let cmd = TestCli::parse_from(["diagnostics", "gasgauge"]);
        assert!(matches!(cmd.command, DiagnosticsSub::Gasgauge));
    }

    #[test]
    fn parses_hdmi_subcommand() {
        let cmd = TestCli::parse_from(["diagnostics", "hdmi"]);
        assert!(matches!(cmd.command, DiagnosticsSub::Hdmi));
    }

    #[test]
    fn parses_wifi_subcommand() {
        let cmd = TestCli::parse_from(["diagnostics", "wifi"]);
        assert!(matches!(cmd.command, DiagnosticsSub::Wifi));
    }

    #[test]
    fn parses_battery_monitor_subcommand() {
        let cmd = TestCli::parse_from([
            "diagnostics",
            "battery-monitor",
            "--count",
            "2",
            "--interval-ms",
            "250",
        ]);
        match cmd.command {
            DiagnosticsSub::BatteryMonitor { count, interval_ms } => {
                assert_eq!(count, Some(2));
                assert_eq!(interval_ms, 250);
            }
            _ => panic!("expected battery-monitor subcommand"),
        }
    }

    #[test]
    fn parses_nand_subcommand() {
        let cmd = TestCli::parse_from(["diagnostics", "nand"]);
        assert!(matches!(cmd.command, DiagnosticsSub::Nand));
    }

    #[test]
    fn extracts_named_entry_case_insensitively() {
        let value = plist::Value::Dictionary(plist::Dictionary::from_iter([(
            "WiFi".to_string(),
            plist::Value::Dictionary(plist::Dictionary::from_iter([(
                "Status".to_string(),
                plist::Value::String("WifiInfoDeprecated".to_string()),
            )])),
        )]));

        let entry = extract_named_diagnostics_entry(&value, "wifi").expect("entry should exist");
        let dict = entry.as_dictionary().expect("entry should be a dictionary");
        assert_eq!(
            dict.get("Status").and_then(plist::Value::as_string),
            Some("WifiInfoDeprecated")
        );
    }

    #[test]
    fn resolves_named_entry_with_canonical_key() {
        let value = plist::Value::Dictionary(plist::Dictionary::from_iter([(
            "WiFi".to_string(),
            plist::Value::Dictionary(plist::Dictionary::from_iter([(
                "Status".to_string(),
                plist::Value::String("WifiInfoDeprecated".to_string()),
            )])),
        )]));

        let (name, entry) =
            resolve_named_diagnostics_entry(&value, "wifi").expect("entry should exist");
        assert_eq!(name, "WiFi");
        let dict = entry.as_dictionary().expect("entry should be a dictionary");
        assert_eq!(
            dict.get("Status").and_then(plist::Value::as_string),
            Some("WifiInfoDeprecated")
        );
    }

    #[test]
    fn parses_ioregistry_subcommand() {
        let cmd = TestCli::parse_from(["diagnostics", "ioregistry", "IOPlatformExpertDevice"]);
        match cmd.command {
            DiagnosticsSub::Ioregistry {
                entry_class,
                entry_name,
                plane,
            } => {
                assert_eq!(entry_class.as_deref(), Some("IOPlatformExpertDevice"));
                assert_eq!(entry_name, None);
                assert_eq!(plane, None);
            }
            _ => panic!("expected ioregistry subcommand"),
        }
    }

    #[test]
    fn parses_ioregistry_name_and_plane_flags() {
        let cmd = TestCli::parse_from([
            "diagnostics",
            "ioregistry",
            "--name",
            "device-tree",
            "--plane",
            "IODeviceTree",
        ]);

        match cmd.command {
            DiagnosticsSub::Ioregistry {
                entry_class,
                entry_name,
                plane,
            } => {
                assert_eq!(entry_class, None);
                assert_eq!(entry_name.as_deref(), Some("device-tree"));
                assert_eq!(plane.as_deref(), Some("IODeviceTree"));
            }
            _ => panic!("expected ioregistry subcommand"),
        }
    }

    #[test]
    fn parses_sysdiagnose_subcommand() {
        let cmd = TestCli::parse_from(["diagnostics", "sysdiagnose"]);
        assert!(matches!(cmd.command, DiagnosticsSub::Sysdiagnose));
    }
}
