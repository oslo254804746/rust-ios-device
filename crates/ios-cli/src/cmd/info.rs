use anyhow::Result;
use ios_core::{connect, ConnectOptions};
use ios_tunnel::TunMode;

const AMFI_DOMAIN: &str = "com.apple.security.mac.amfi";
const DEVELOPER_MODE_STATUS_KEY: &str = "DeveloperModeStatus";

#[derive(clap::Args)]
pub struct InfoCmd {
    #[command(subcommand)]
    sub: Option<InfoSub>,
    #[arg(long, help = "Lockdown domain to query")]
    domain: Option<String>,
    #[arg(long, help = "Lockdown key to query")]
    key: Option<String>,
}

#[derive(Debug, clap::Subcommand)]
enum InfoSub {
    /// Show display information using diagnostics relay IORegistry
    Display,
    /// Show standard lockdown information
    Lockdown(InfoLockdownArgs),
}

#[derive(Debug, Clone, clap::Args)]
struct InfoLockdownArgs {
    #[arg(long, help = "Lockdown domain to query")]
    domain: Option<String>,
    #[arg(long, help = "Lockdown key to query")]
    key: Option<String>,
}

impl InfoCmd {
    pub async fn run(self, udid: Option<String>, json: bool) -> Result<()> {
        let udid = udid.ok_or_else(|| anyhow::anyhow!("--udid required for info"))?;

        let opts = ConnectOptions {
            tun_mode: TunMode::Userspace,
            pair_record_path: None,
            skip_tunnel: true,
        };
        let device = connect(&udid, opts).await?;

        if let Some(sub) = self.sub {
            match sub {
                InfoSub::Display => {
                    let mut stream = device
                        .connect_service(ios_services::diagnostics::SERVICE_NAME)
                        .await?;
                    let value = ios_services::diagnostics::query_ioregistry(
                        &mut *stream,
                        "IOMobileFramebuffer",
                    )
                    .await?;
                    let summary = summarize_display_ioregistry(&value);
                    if json {
                        println!("{}", serde_json::to_string_pretty(&summary)?);
                    } else {
                        print_display_summary(&summary);
                    }
                    return Ok(());
                }
                InfoSub::Lockdown(args) => {
                    let value = device
                        .lockdown_get_value_in_domain(args.domain.as_deref(), args.key.as_deref())
                        .await?;
                    if json {
                        println!("{}", serde_json::to_string_pretty(&plist_to_json(&value))?);
                    } else {
                        print_value(&value);
                    }
                    return Ok(());
                }
            }
        }

        if self.domain.is_some() || self.key.is_some() {
            let value = device
                .lockdown_get_value_in_domain(self.domain.as_deref(), self.key.as_deref())
                .await?;
            if json {
                println!("{}", serde_json::to_string_pretty(&plist_to_json(&value))?);
            } else {
                print_value(&value);
            }
            return Ok(());
        }

        // Fetch all lockdown values (key=None → returns full dict)
        let all = device.lockdown_get_value(None).await?;

        let get_str = |key: &str| -> String {
            all.as_dictionary()
                .and_then(|d| d.get(key))
                .and_then(|v| v.as_string())
                .unwrap_or("N/A")
                .to_string()
        };
        let get_bool = |key: &str| -> bool {
            all.as_dictionary()
                .and_then(|d| d.get(key))
                .and_then(|v| v.as_boolean())
                .unwrap_or(false)
        };

        let developer_mode = resolve_developer_mode_status(
            all.as_dictionary(),
            device
                .lockdown_get_value_in_domain(Some(AMFI_DOMAIN), Some(DEVELOPER_MODE_STATUS_KEY))
                .await
                .ok()
                .as_ref(),
        );

        if json {
            let mut map = serde_json::Map::new();
            if let Some(dict) = all.as_dictionary() {
                for (k, v) in dict {
                    let jv = plist_to_json(v);
                    map.insert(k.clone(), jv);
                }
            }
            println!("{}", serde_json::to_string_pretty(&map)?);
        } else {
            println!("DeviceName:       {}", get_str("DeviceName"));
            println!("ProductType:      {}", get_str("ProductType"));
            println!("ProductVersion:   {}", get_str("ProductVersion"));
            println!("BuildVersion:     {}", get_str("BuildVersion"));
            println!("HardwareModel:    {}", get_str("HardwareModel"));
            println!("SerialNumber:     {}", get_str("SerialNumber"));
            println!("UniqueDeviceID:   {}", get_str("UniqueDeviceID"));
            println!("CPUArchitecture:  {}", get_str("CPUArchitecture"));
            println!("DeviceColor:      {}", get_str("DeviceColor"));
            println!("WiFiAddress:      {}", get_str("WiFiAddress"));
            println!("BluetoothAddress: {}", get_str("BluetoothAddress"));
            println!(
                "DeveloperMode:    {}",
                developer_mode.unwrap_or_else(|| get_bool("DeveloperModeStatus"))
            );
        }
        Ok(())
    }
}

fn resolve_developer_mode_status(
    all_values: Option<&plist::Dictionary>,
    amfi_value: Option<&plist::Value>,
) -> Option<bool> {
    all_values
        .and_then(|d| d.get(DEVELOPER_MODE_STATUS_KEY))
        .and_then(plist::Value::as_boolean)
        .or_else(|| amfi_value.and_then(plist_value_to_bool))
}

fn summarize_display_ioregistry(value: &plist::Value) -> serde_json::Value {
    let Some(dict) = value.as_dictionary() else {
        return serde_json::json!({
            "source": "diagnostics.ioregistry:IOMobileFramebuffer",
            "raw": plist_to_json(value),
        });
    };

    let mut obj = serde_json::Map::new();
    obj.insert(
        "source".to_string(),
        serde_json::Value::String("diagnostics.ioregistry:IOMobileFramebuffer".into()),
    );

    insert_string_field(&mut obj, "io_class", dict, "IOClass");
    insert_string_field(&mut obj, "io_name", dict, "IONameMatched");
    insert_string_field(&mut obj, "panel_id", dict, "Panel_ID");
    insert_bool_field(&mut obj, "external", dict, "external");
    insert_fixed_field(&mut obj, "ambient_brightness", dict, "AmbientBrightness");
    insert_fixed_field(&mut obj, "brightness_level", dict, "IOMFBBrightnessLevel");
    insert_fixed_field(
        &mut obj,
        "digital_dimming_level",
        dict,
        "IOMFBDigitalDimmingLevel",
    );

    if let Some(refresh) = dict.get("IOMFBDisplayRefresh") {
        obj.insert("display_refresh".to_string(), plist_to_json(refresh));
    }

    serde_json::Value::Object(obj)
}

fn insert_string_field(
    obj: &mut serde_json::Map<String, serde_json::Value>,
    output_key: &str,
    dict: &plist::Dictionary,
    plist_key: &str,
) {
    if let Some(value) = dict.get(plist_key).and_then(|v| v.as_string()) {
        obj.insert(
            output_key.to_string(),
            serde_json::Value::String(value.to_string()),
        );
    }
}

fn insert_bool_field(
    obj: &mut serde_json::Map<String, serde_json::Value>,
    output_key: &str,
    dict: &plist::Dictionary,
    plist_key: &str,
) {
    if let Some(value) = dict.get(plist_key).and_then(|v| v.as_boolean()) {
        obj.insert(output_key.to_string(), serde_json::Value::Bool(value));
    }
}

fn insert_fixed_field(
    obj: &mut serde_json::Map<String, serde_json::Value>,
    output_prefix: &str,
    dict: &plist::Dictionary,
    plist_key: &str,
) {
    if let Some(raw) = dict.get(plist_key).and_then(plist_integer_to_i64) {
        obj.insert(format!("{output_prefix}_raw"), serde_json::Value::from(raw));
        if let Some(num) = serde_json::Number::from_f64(raw as f64 / 65536.0) {
            obj.insert(output_prefix.to_string(), serde_json::Value::Number(num));
        }
    }
}

fn plist_integer_to_i64(value: &plist::Value) -> Option<i64> {
    match value {
        plist::Value::Integer(n) => n.as_signed().or_else(|| n.as_unsigned()?.try_into().ok()),
        _ => None,
    }
}

fn plist_value_to_bool(value: &plist::Value) -> Option<bool> {
    match value {
        plist::Value::Boolean(value) => Some(*value),
        plist::Value::Integer(value) => value
            .as_signed()
            .map(|value| value != 0)
            .or_else(|| value.as_unsigned().map(|value| value != 0)),
        plist::Value::String(value) => match value.to_ascii_lowercase().as_str() {
            "true" | "1" => Some(true),
            "false" | "0" => Some(false),
            _ => None,
        },
        _ => None,
    }
}

fn print_display_summary(summary: &serde_json::Value) {
    let Some(obj) = summary.as_object() else {
        println!(
            "{}",
            serde_json::to_string_pretty(summary).unwrap_or_else(|_| "null".into())
        );
        return;
    };

    print_row(obj, "Source", "source");
    print_row(obj, "IOClass", "io_class");
    print_row(obj, "IONameMatched", "io_name");
    print_row(obj, "External", "external");
    print_row(obj, "Panel_ID", "panel_id");
    print_fixed_row(obj, "AmbientBrightness", "ambient_brightness");
    print_fixed_row(obj, "BrightnessLevel", "brightness_level");
    print_fixed_row(obj, "DigitalDimmingLevel", "digital_dimming_level");
    if let Some(value) = obj.get("display_refresh") {
        let rendered = serde_json::to_string(value).unwrap_or_else(|_| value.to_string());
        println!("DisplayRefresh:  {rendered}");
    }
}

fn print_row(obj: &serde_json::Map<String, serde_json::Value>, label: &str, key: &str) {
    if let Some(value) = obj.get(key) {
        println!("{label:<16} {}", format_json_value(value));
    }
}

fn print_fixed_row(obj: &serde_json::Map<String, serde_json::Value>, label: &str, key: &str) {
    let raw_key = format!("{key}_raw");
    match (obj.get(&raw_key), obj.get(key)) {
        (Some(raw), Some(value)) => {
            println!(
                "{label:<16} {} ({})",
                format_json_value(raw),
                format_json_value(value)
            );
        }
        (Some(raw), None) => println!("{label:<16} {}", format_json_value(raw)),
        _ => {}
    }
}

fn print_value(value: &plist::Value) {
    match value {
        plist::Value::String(s) => println!("{s}"),
        plist::Value::Boolean(v) => println!("{v}"),
        plist::Value::Integer(n) => {
            if let Some(i) = n.as_signed() {
                println!("{i}");
            } else if let Some(u) = n.as_unsigned() {
                println!("{u}");
            }
        }
        plist::Value::Real(v) => println!("{v}"),
        other => println!(
            "{}",
            serde_json::to_string_pretty(&plist_to_json(other)).unwrap_or_else(|_| "null".into())
        ),
    }
}

fn format_json_value(value: &serde_json::Value) -> String {
    match value {
        serde_json::Value::Null => "null".to_string(),
        serde_json::Value::Bool(v) => v.to_string(),
        serde_json::Value::Number(v) => v.to_string(),
        serde_json::Value::String(v) => v.clone(),
        other => serde_json::to_string(other).unwrap_or_else(|_| other.to_string()),
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
        #[command(flatten)]
        command: InfoCmd,
    }

    #[test]
    fn parses_info_key_and_domain_flags() {
        let cmd = TestCli::parse_from([
            "info",
            "--domain",
            "com.apple.mobile.iTunes",
            "--key",
            "BuildVersion",
        ]);
        assert_eq!(
            cmd.command.domain.as_deref(),
            Some("com.apple.mobile.iTunes")
        );
        assert_eq!(cmd.command.key.as_deref(), Some("BuildVersion"));
    }

    #[test]
    fn parses_info_display_subcommand() {
        let cmd = TestCli::parse_from(["info", "display"]);
        assert!(matches!(cmd.command.sub, Some(InfoSub::Display)));
        assert_eq!(cmd.command.domain, None);
        assert_eq!(cmd.command.key, None);
    }

    #[test]
    fn parses_info_lockdown_subcommand() {
        let cmd = TestCli::parse_from(["info", "lockdown"]);
        assert!(matches!(cmd.command.sub, Some(InfoSub::Lockdown(_))));
        assert_eq!(cmd.command.domain, None);
        assert_eq!(cmd.command.key, None);
    }

    #[test]
    fn parses_info_lockdown_key_flag() {
        let cmd = TestCli::parse_from(["info", "lockdown", "--key", "ProductVersion"]);
        match cmd.command.sub {
            Some(InfoSub::Lockdown(args)) => {
                assert_eq!(args.key.as_deref(), Some("ProductVersion"));
                assert_eq!(args.domain, None);
            }
            other => panic!("expected lockdown subcommand, got {other:?}"),
        }
    }

    #[test]
    fn summarizes_iomobileframebuffer_display_fields() {
        let value = plist::Value::Dictionary(plist::Dictionary::from_iter([
            (
                "IOClass".to_string(),
                plist::Value::String("AppleCLCD2".into()),
            ),
            (
                "IONameMatched".to_string(),
                plist::Value::String("dispext0,t8150".into()),
            ),
            ("external".to_string(), plist::Value::Boolean(true)),
            (
                "AmbientBrightness".to_string(),
                plist::Value::Integer(65536.into()),
            ),
            (
                "IOMFBBrightnessLevel".to_string(),
                plist::Value::Integer(131072.into()),
            ),
            (
                "IOMFBDigitalDimmingLevel".to_string(),
                plist::Value::Integer(327680.into()),
            ),
            (
                "Panel_ID".to_string(),
                plist::Value::String("panel-123".into()),
            ),
        ]));

        let summary = summarize_display_ioregistry(&value);
        let obj = summary.as_object().unwrap();
        assert_eq!(obj["source"], "diagnostics.ioregistry:IOMobileFramebuffer");
        assert_eq!(obj["io_class"], "AppleCLCD2");
        assert_eq!(obj["io_name"], "dispext0,t8150");
        assert_eq!(obj["external"], true);
        assert_eq!(obj["panel_id"], "panel-123");
        assert_eq!(obj["ambient_brightness_raw"], 65536);
        assert_eq!(obj["brightness_level_raw"], 131072);
        assert_eq!(obj["digital_dimming_level_raw"], 327680);
        assert_eq!(obj["ambient_brightness"], 1.0);
        assert_eq!(obj["brightness_level"], 2.0);
        assert_eq!(obj["digital_dimming_level"], 5.0);
    }

    #[test]
    fn developer_mode_status_falls_back_to_amfi_domain_value() {
        let amfi_value = plist::Value::Boolean(true);
        let resolved = resolve_developer_mode_status(None, Some(&amfi_value));
        assert_eq!(resolved, Some(true));
    }

    #[test]
    fn developer_mode_status_prefers_global_value_when_present() {
        let all_values = plist::Dictionary::from_iter([(
            DEVELOPER_MODE_STATUS_KEY.to_string(),
            plist::Value::Boolean(false),
        )]);
        let amfi_value = plist::Value::Boolean(true);
        let resolved = resolve_developer_mode_status(Some(&all_values), Some(&amfi_value));
        assert_eq!(resolved, Some(false));
    }
}
