use anyhow::Result;
use ios_core::{connect, ConnectOptions};
use ios_lockdown::pair_record::default_pair_record_path;
use ios_tunnel::TunMode;

const INTERNATIONAL_DOMAIN: &str = "com.apple.international";
const ACCESSIBILITY_DOMAIN: &str = "com.apple.Accessibility";
const WIRELESS_LOCKDOWN_DOMAIN: &str = "com.apple.mobile.wireless_lockdown";
const ASSISTIVE_TOUCH_KEY: &str = "AssistiveTouchEnabledByiTunes";
const VOICE_OVER_KEY: &str = "VoiceOverTouchEnabledByiTunes";
const ZOOM_TOUCH_KEY: &str = "ZoomTouchEnabledByiTunes";
const INVERT_DISPLAY_KEY: &str = "InvertDisplayEnabledByiTunes";
const USES_24_HOUR_CLOCK_KEY: &str = "Uses24HourClock";
const ENABLE_WIFI_CONNECTIONS_KEY: &str = "EnableWifiConnections";
const ACCESSIBILITY_FONT_SIZE_KEYS: &[&str] = &[
    "DYNAMIC_TYPE",
    "DynamicType",
    "TextSize",
    "PreferredContentSizeCategory",
    "UIContentSizeCategory",
];
const AMFI_DOMAIN: &str = "com.apple.security.mac.amfi";
const DEVELOPER_MODE_STATUS_KEY: &str = "DeveloperModeStatus";
const TIME_ZONE_KEY: &str = "TimeZone";

#[derive(clap::Args)]
pub struct LockdownCmd {
    #[command(subcommand)]
    sub: LockdownSub,
}

#[derive(Debug, clap::Subcommand)]
enum LockdownSub {
    /// Query all lockdown values
    Info,
    /// Query lockdown values by domain and key
    Get {
        #[arg(help = "Lockdown key to query", conflicts_with = "key")]
        key_arg: Option<String>,
        #[arg(long, help = "Lockdown domain to query")]
        domain: Option<String>,
        #[arg(long, help = "Lockdown key to query", conflicts_with = "key_arg")]
        key: Option<String>,
    },
    /// Set a lockdown value by domain and key
    Set {
        #[arg(help = "Value to set")]
        value: String,
        #[arg(long, help = "Lockdown domain to update")]
        domain: Option<String>,
        #[arg(long, help = "Lockdown key to update")]
        key: String,
        #[arg(
            long = "type",
            value_enum,
            default_value = "string",
            help = "Value type"
        )]
        value_type: LockdownValueType,
    },
    /// Remove a lockdown value by domain and key
    Remove {
        #[arg(long, help = "Lockdown domain to update")]
        domain: Option<String>,
        #[arg(long, help = "Lockdown key to remove")]
        key: String,
    },
    /// Save the local lockdown pair record for a device
    SavePairRecord {
        #[arg(help = "Path to write the raw pair record plist")]
        output: std::path::PathBuf,
    },
    /// Start heartbeat service exchanges
    Heartbeat {
        #[arg(
            long,
            default_value_t = 1,
            help = "Number of heartbeat exchanges to perform"
        )]
        count: usize,
        #[arg(long, help = "Timeout in seconds for each heartbeat exchange")]
        timeout: Option<u64>,
    },
    /// Start a lockdown/CoreDevice tunnel for developer services
    StartTunnel {
        #[arg(long, help = "Print only RSD host and port for shell scripts")]
        script_mode: bool,
    },
    /// Show device date from lockdown
    Date,
    /// Get or set the current device name
    DeviceName {
        #[arg(help = "New device name to set")]
        new_name: Option<String>,
    },
    /// Show current language from com.apple.international
    Language {
        #[arg(long, help = "Include locale and supported language/locale lists")]
        details: bool,
    },
    /// Get or set current locale from com.apple.international
    Locale {
        #[arg(help = "New locale to set")]
        new_locale: Option<String>,
    },
    /// Get or set AssistiveTouch visibility
    AssistiveTouch {
        #[arg(value_enum, help = "Desired AssistiveTouch state")]
        state: Option<ToggleState>,
    },
    /// Get or set VoiceOver visibility
    VoiceOver {
        #[arg(value_enum, help = "Desired VoiceOver state")]
        state: Option<ToggleState>,
    },
    /// Get or set ZoomTouch visibility
    ZoomTouch {
        #[arg(value_enum, help = "Desired ZoomTouch state")]
        state: Option<ToggleState>,
    },
    /// Get or set display inversion
    InvertDisplay {
        #[arg(value_enum, help = "Desired invert display state")]
        state: Option<ToggleState>,
    },
    /// Get or set time format
    TimeFormat {
        #[arg(value_enum, help = "Desired time format")]
        format: Option<ClockFormat>,
    },
    /// Show developer mode status from AMFI lockdown domain
    DeveloperMode,
    /// Get or set wifi connections visibility over lockdown pairing
    WifiConnections {
        #[arg(value_enum, help = "Desired wifi connections state")]
        state: Option<ToggleState>,
    },
    /// Reset the common accessibility lockdown toggles to defaults
    ResetAccessibility,
    /// Query font-size related accessibility keys exposed via lockdown
    FontSize,
    /// Show current device time zone
    TimeZone,
}

#[derive(Clone, Copy, Debug, clap::ValueEnum, Eq, PartialEq)]
enum ToggleState {
    On,
    Off,
}

#[derive(Clone, Copy, Debug, clap::ValueEnum, Eq, PartialEq)]
enum ClockFormat {
    #[value(name = "12")]
    Twelve,
    #[value(name = "24")]
    TwentyFour,
}

#[derive(Clone, Copy, Debug, clap::ValueEnum, Eq, PartialEq)]
enum LockdownValueType {
    String,
    Bool,
    Int,
    Float,
}

impl LockdownCmd {
    pub async fn run(self, udid: Option<String>, json: bool) -> Result<()> {
        let udid = udid.ok_or_else(|| anyhow::anyhow!("--udid required for lockdown"))?;

        let opts = ConnectOptions {
            tun_mode: TunMode::Userspace,
            pair_record_path: None,
            skip_tunnel: true,
        };
        let device = connect(&udid, opts).await?;

        match self.sub {
            LockdownSub::Info => {
                let value = device.lockdown_get_value_in_domain(None, None).await?;
                if json {
                    println!("{}", serde_json::to_string_pretty(&plist_to_json(&value))?);
                } else {
                    print_value(&value);
                }
            }
            LockdownSub::Get {
                key_arg,
                domain,
                key,
            } => {
                let key = key.or(key_arg);
                let value = device
                    .lockdown_get_value_in_domain(domain.as_deref(), key.as_deref())
                    .await?;
                if json {
                    println!("{}", serde_json::to_string_pretty(&plist_to_json(&value))?);
                } else {
                    print_value(&value);
                }
            }
            LockdownSub::Set {
                value,
                domain,
                key,
                value_type,
            } => {
                let parsed = parse_lockdown_value(&value, value_type)?;
                device
                    .lockdown_set_value_in_domain(domain.as_deref(), Some(&key), parsed)
                    .await?;
                let value = device
                    .lockdown_get_value_in_domain(domain.as_deref(), Some(&key))
                    .await?;
                if json {
                    println!("{}", serde_json::to_string_pretty(&plist_to_json(&value))?);
                } else {
                    print_value(&value);
                }
            }
            LockdownSub::Remove { domain, key } => {
                device
                    .lockdown_remove_value_in_domain(domain.as_deref(), Some(&key))
                    .await?;
                if json {
                    println!(
                        "{}",
                        serde_json::to_string_pretty(&serde_json::json!({
                            "removed": true,
                            "domain": domain,
                            "key": key,
                        }))?
                    );
                } else if let Some(domain) = domain {
                    println!("removed domain={domain} key={key}");
                } else {
                    println!("removed key={key}");
                }
            }
            LockdownSub::SavePairRecord { output } => {
                let source = default_pair_record_path(&udid);
                let raw = std::fs::read(&source)?;
                if let Some(parent) = output.parent() {
                    if !parent.as_os_str().is_empty() {
                        std::fs::create_dir_all(parent)?;
                    }
                }
                std::fs::write(&output, &raw)?;

                if json {
                    println!(
                        "{}",
                        serde_json::to_string_pretty(&serde_json::json!({
                            "source_path": source.display().to_string(),
                            "output_path": output.display().to_string(),
                            "bytes": raw.len(),
                        }))?
                    );
                } else {
                    println!("Source: {}", source.display());
                    println!("Output: {}", output.display());
                    println!("Bytes: {}", raw.len());
                }
            }
            LockdownSub::Heartbeat { count, timeout } => {
                crate::cmd::heartbeat::run_heartbeat(&udid, json, count, timeout).await?;
            }
            LockdownSub::StartTunnel { script_mode } => {
                crate::cmd::tunnel::run_tunnel_start(&udid, TunMode::Userspace, script_mode)
                    .await?;
            }
            LockdownSub::Date => {
                let value = device
                    .lockdown_get_value(Some("TimeIntervalSince1970"))
                    .await?;
                let timestamp = plist_value_to_f64(&value)
                    .ok_or_else(|| anyhow::anyhow!("TimeIntervalSince1970 was not numeric"))?;
                let formatted = format_unix_timestamp(timestamp)?;
                if json {
                    println!(
                        "{}",
                        serde_json::to_string_pretty(&serde_json::json!({
                            "formatted_date": formatted,
                            "time_interval_since_1970": timestamp,
                        }))?
                    );
                } else {
                    println!("{formatted}");
                }
            }
            LockdownSub::DeviceName { new_name } => {
                if let Some(new_name) = new_name {
                    device
                        .lockdown_set_value(
                            Some("DeviceName"),
                            plist::Value::String(new_name.clone()),
                        )
                        .await?;
                }
                let value = device.lockdown_get_value(Some("DeviceName")).await?;
                if json {
                    println!(
                        "{}",
                        serde_json::to_string_pretty(&serde_json::json!({
                            "device_name": plist_value_to_string(&value).unwrap_or_default(),
                        }))?
                    );
                } else {
                    print_value(&value);
                }
            }
            LockdownSub::Language { details } => {
                let config = device.lockdown_international_configuration().await?;
                if json {
                    println!("{}", serde_json::to_string_pretty(&config)?);
                } else if details {
                    print_international_configuration(&config);
                } else {
                    println!("{}", config.language);
                }
            }
            LockdownSub::Locale { new_locale } => {
                if let Some(new_locale) = new_locale {
                    device
                        .lockdown_set_value_in_domain(
                            Some(INTERNATIONAL_DOMAIN),
                            Some("Locale"),
                            plist::Value::String(new_locale.clone()),
                        )
                        .await?;
                }
                let value = device
                    .lockdown_get_value_in_domain(Some(INTERNATIONAL_DOMAIN), Some("Locale"))
                    .await?;
                if json {
                    println!(
                        "{}",
                        serde_json::to_string_pretty(&serde_json::json!({
                            "locale": plist_value_to_string(&value).unwrap_or_default(),
                        }))?
                    );
                } else {
                    print_value(&value);
                }
            }
            LockdownSub::AssistiveTouch { state } => {
                run_accessibility_toggle(&device, ASSISTIVE_TOUCH_KEY, state, json).await?;
            }
            LockdownSub::VoiceOver { state } => {
                run_accessibility_toggle(&device, VOICE_OVER_KEY, state, json).await?;
            }
            LockdownSub::ZoomTouch { state } => {
                run_accessibility_toggle(&device, ZOOM_TOUCH_KEY, state, json).await?;
            }
            LockdownSub::InvertDisplay { state } => {
                run_accessibility_toggle(&device, INVERT_DISPLAY_KEY, state, json).await?;
            }
            LockdownSub::TimeFormat { format } => {
                if let Some(format) = format {
                    let uses_24_hour_clock = matches!(format, ClockFormat::TwentyFour);
                    device
                        .lockdown_set_value(
                            Some(USES_24_HOUR_CLOCK_KEY),
                            plist::Value::Boolean(uses_24_hour_clock),
                        )
                        .await?;
                }

                let value = device
                    .lockdown_get_value(Some(USES_24_HOUR_CLOCK_KEY))
                    .await?;
                let uses_24_hour_clock = plist_value_to_bool(&value).ok_or_else(|| {
                    anyhow::anyhow!(
                        "{USES_24_HOUR_CLOCK_KEY} was not a boolean-compatible value: {value:?}"
                    )
                })?;

                if json {
                    println!(
                        "{}",
                        serde_json::to_string_pretty(&serde_json::json!({
                            USES_24_HOUR_CLOCK_KEY: uses_24_hour_clock,
                            "time_format": if uses_24_hour_clock { "24" } else { "12" },
                        }))?
                    );
                } else {
                    println!("{}", if uses_24_hour_clock { "24" } else { "12" });
                }
            }
            LockdownSub::DeveloperMode => {
                let value = device
                    .lockdown_get_value_in_domain(
                        Some(AMFI_DOMAIN),
                        Some(DEVELOPER_MODE_STATUS_KEY),
                    )
                    .await?;
                let enabled = plist_value_to_bool(&value).ok_or_else(|| {
                    anyhow::anyhow!(
                        "{DEVELOPER_MODE_STATUS_KEY} was not a boolean-compatible value: {value:?}"
                    )
                })?;

                if json {
                    println!(
                        "{}",
                        serde_json::to_string_pretty(&serde_json::json!({
                            DEVELOPER_MODE_STATUS_KEY: enabled,
                        }))?
                    );
                } else {
                    println!("{enabled}");
                }
            }
            LockdownSub::WifiConnections { state } => {
                run_wifi_connections_toggle(&device, state, json).await?;
            }
            LockdownSub::ResetAccessibility => {
                for key in [
                    ASSISTIVE_TOUCH_KEY,
                    VOICE_OVER_KEY,
                    ZOOM_TOUCH_KEY,
                    INVERT_DISPLAY_KEY,
                ] {
                    device
                        .lockdown_set_value_in_domain(
                            Some(ACCESSIBILITY_DOMAIN),
                            Some(key),
                            plist::Value::Integer(0.into()),
                        )
                        .await?;
                }
                let value = device
                    .lockdown_get_value_in_domain(Some(ACCESSIBILITY_DOMAIN), None)
                    .await?;
                if json {
                    println!("{}", serde_json::to_string_pretty(&plist_to_json(&value))?);
                } else {
                    print_value(&value);
                }
            }
            LockdownSub::FontSize => {
                let value = device
                    .lockdown_get_value_in_domain(Some(ACCESSIBILITY_DOMAIN), None)
                    .await?;
                let dict = value.as_dictionary().ok_or_else(|| {
                    anyhow::anyhow!("com.apple.Accessibility did not return a dictionary")
                })?;
                let mut out = serde_json::Map::new();
                for key in ACCESSIBILITY_FONT_SIZE_KEYS {
                    if let Some(value) = dict.get(key) {
                        out.insert((*key).to_string(), plist_to_json(value));
                    }
                }
                if out.is_empty() {
                    return Err(anyhow::anyhow!(
                        "font size is not exposed by com.apple.Accessibility on this device"
                    ));
                }

                let rendered = serde_json::Value::Object(out);
                if json {
                    println!("{}", serde_json::to_string_pretty(&rendered)?);
                } else {
                    print_value_json(&rendered);
                }
            }
            LockdownSub::TimeZone => {
                let value = device.lockdown_get_value(Some(TIME_ZONE_KEY)).await?;
                if json {
                    println!(
                        "{}",
                        serde_json::to_string_pretty(&serde_json::json!({
                            TIME_ZONE_KEY: plist_value_to_string(&value).unwrap_or_default(),
                        }))?
                    );
                } else {
                    print_value(&value);
                }
            }
        }

        Ok(())
    }
}

fn plist_value_to_f64(value: &plist::Value) -> Option<f64> {
    match value {
        plist::Value::Integer(value) => value
            .as_signed()
            .map(|value| value as f64)
            .or_else(|| value.as_unsigned().map(|value| value as f64)),
        plist::Value::Real(value) => Some(*value),
        _ => None,
    }
}

fn plist_value_to_string(value: &plist::Value) -> Option<String> {
    value.as_string().map(ToOwned::to_owned)
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

fn parse_lockdown_value(value: &str, value_type: LockdownValueType) -> Result<plist::Value> {
    match value_type {
        LockdownValueType::String => Ok(plist::Value::String(value.to_string())),
        LockdownValueType::Bool => match value.to_ascii_lowercase().as_str() {
            "true" | "1" | "on" => Ok(plist::Value::Boolean(true)),
            "false" | "0" | "off" => Ok(plist::Value::Boolean(false)),
            _ => Err(anyhow::anyhow!("invalid bool value: {value}")),
        },
        LockdownValueType::Int => {
            let parsed: i64 = value.parse()?;
            Ok(plist::Value::Integer(parsed.into()))
        }
        LockdownValueType::Float => {
            let parsed: f64 = value.parse()?;
            Ok(plist::Value::Real(parsed))
        }
    }
}

async fn run_accessibility_toggle(
    device: &ios_core::ConnectedDevice,
    key: &str,
    state: Option<ToggleState>,
    json: bool,
) -> Result<()> {
    if let Some(state) = state {
        let enabled = matches!(state, ToggleState::On);
        device
            .lockdown_set_value_in_domain(
                Some(ACCESSIBILITY_DOMAIN),
                Some(key),
                plist::Value::Integer((enabled as i64).into()),
            )
            .await?;
    }

    let value = device
        .lockdown_get_value_in_domain(Some(ACCESSIBILITY_DOMAIN), Some(key))
        .await?;
    let enabled = plist_value_to_bool(&value)
        .ok_or_else(|| anyhow::anyhow!("{key} was not a boolean-compatible value: {value:?}"))?;

    if json {
        println!(
            "{}",
            serde_json::to_string_pretty(&serde_json::json!({
                key: enabled,
            }))?
        );
    } else {
        println!("{enabled}");
    }

    Ok(())
}

async fn run_wifi_connections_toggle(
    device: &ios_core::ConnectedDevice,
    state: Option<ToggleState>,
    json: bool,
) -> Result<()> {
    if let Some(state) = state {
        let enabled = matches!(state, ToggleState::On);
        device
            .lockdown_set_value_in_domain(
                Some(WIRELESS_LOCKDOWN_DOMAIN),
                Some(ENABLE_WIFI_CONNECTIONS_KEY),
                plist::Value::Boolean(enabled),
            )
            .await?;
    }

    let value = device
        .lockdown_get_value_in_domain(
            Some(WIRELESS_LOCKDOWN_DOMAIN),
            Some(ENABLE_WIFI_CONNECTIONS_KEY),
        )
        .await?;
    let enabled = plist_value_to_bool(&value).ok_or_else(|| {
        anyhow::anyhow!(
            "{ENABLE_WIFI_CONNECTIONS_KEY} was not a boolean-compatible value: {value:?}"
        )
    })?;

    if json {
        println!(
            "{}",
            serde_json::to_string_pretty(&serde_json::json!({
                ENABLE_WIFI_CONNECTIONS_KEY: enabled,
            }))?
        );
    } else {
        println!("{enabled}");
    }

    Ok(())
}

fn format_unix_timestamp(timestamp: f64) -> Result<String> {
    let secs = timestamp.trunc() as i64;
    let nanos = ((timestamp.fract()) * 1_000_000_000f64).round() as u32;
    let datetime = chrono::DateTime::<chrono::Utc>::from_timestamp(secs, nanos)
        .ok_or_else(|| anyhow::anyhow!("invalid unix timestamp: {timestamp}"))?;
    Ok(datetime.to_rfc3339_opts(chrono::SecondsFormat::Nanos, true))
}

fn print_value(value: &plist::Value) {
    match value {
        plist::Value::String(value) => println!("{value}"),
        plist::Value::Boolean(value) => println!("{value}"),
        plist::Value::Integer(value) => {
            if let Some(signed) = value.as_signed() {
                println!("{signed}");
            } else if let Some(unsigned) = value.as_unsigned() {
                println!("{unsigned}");
            }
        }
        plist::Value::Real(value) => println!("{value}"),
        other => println!(
            "{}",
            serde_json::to_string_pretty(&plist_to_json(other)).unwrap_or_else(|_| "null".into())
        ),
    }
}

fn print_value_json(value: &serde_json::Value) {
    match value {
        serde_json::Value::String(value) => println!("{value}"),
        other => println!(
            "{}",
            serde_json::to_string_pretty(other).unwrap_or_else(|_| "null".into())
        ),
    }
}

fn print_international_configuration(config: &ios_core::InternationalConfiguration) {
    println!("Language: {}", config.language);
    println!("Locale: {}", config.locale);
    println!(
        "SupportedLanguages: {}",
        config.supported_languages.join(", ")
    );
    println!("SupportedLocales: {}", config.supported_locales.join(", "));
}

fn plist_to_json(value: &plist::Value) -> serde_json::Value {
    match value {
        plist::Value::String(value) => serde_json::Value::String(value.clone()),
        plist::Value::Boolean(value) => serde_json::Value::Bool(*value),
        plist::Value::Integer(value) => value
            .as_signed()
            .map(serde_json::Value::from)
            .or_else(|| value.as_unsigned().map(serde_json::Value::from))
            .unwrap_or(serde_json::Value::Null),
        plist::Value::Real(value) => serde_json::Number::from_f64(*value)
            .map(serde_json::Value::Number)
            .unwrap_or(serde_json::Value::Null),
        plist::Value::Data(bytes) => serde_json::Value::String(hex::encode(bytes)),
        plist::Value::Array(values) => {
            serde_json::Value::Array(values.iter().map(plist_to_json).collect())
        }
        plist::Value::Dictionary(values) => serde_json::Value::Object(
            values
                .iter()
                .map(|(key, value)| (key.clone(), plist_to_json(value)))
                .collect(),
        ),
        plist::Value::Date(value) => serde_json::Value::String(value.to_xml_format()),
        plist::Value::Uid(value) => serde_json::Value::from(value.get()),
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
        command: LockdownSub,
    }

    #[test]
    fn parses_lockdown_info_subcommand() {
        let cmd = TestCli::parse_from(["lockdown", "info"]);
        assert!(matches!(cmd.command, LockdownSub::Info));
    }

    #[test]
    fn parses_lockdown_get_key_flag() {
        let cmd = TestCli::parse_from(["lockdown", "get", "--key", "ProductVersion"]);
        match cmd.command {
            LockdownSub::Get {
                key_arg,
                domain,
                key,
            } => {
                assert_eq!(key_arg, None);
                assert_eq!(domain, None);
                assert_eq!(key.as_deref(), Some("ProductVersion"));
            }
            other => panic!("expected get subcommand, got {other:?}"),
        }
    }

    #[test]
    fn parses_lockdown_get_positional_key() {
        let cmd = TestCli::parse_from(["lockdown", "get", "DeviceName"]);
        match cmd.command {
            LockdownSub::Get { key_arg, key, .. } => {
                assert_eq!(key_arg.as_deref(), Some("DeviceName"));
                assert_eq!(key, None);
            }
            other => panic!("expected get subcommand, got {other:?}"),
        }
    }

    #[test]
    fn parses_lockdown_get_domain_flag() {
        let cmd = TestCli::parse_from([
            "lockdown",
            "get",
            "--domain",
            "com.apple.disk_usage",
            "--key",
            "TotalDataCapacity",
        ]);
        match cmd.command {
            LockdownSub::Get { domain, key, .. } => {
                assert_eq!(domain.as_deref(), Some("com.apple.disk_usage"));
                assert_eq!(key.as_deref(), Some("TotalDataCapacity"));
            }
            other => panic!("expected get subcommand, got {other:?}"),
        }
    }

    #[test]
    fn parses_lockdown_set_string_value() {
        let cmd = TestCli::parse_from([
            "lockdown",
            "set",
            "en_US",
            "--domain",
            "com.apple.international",
            "--key",
            "Locale",
        ]);
        match cmd.command {
            LockdownSub::Set {
                value,
                domain,
                key,
                value_type,
            } => {
                assert_eq!(value, "en_US");
                assert_eq!(domain.as_deref(), Some("com.apple.international"));
                assert_eq!(key, "Locale");
                assert!(matches!(value_type, LockdownValueType::String));
            }
            other => panic!("expected set subcommand, got {other:?}"),
        }
    }

    #[test]
    fn parses_lockdown_set_bool_value() {
        let cmd = TestCli::parse_from([
            "lockdown",
            "set",
            "true",
            "--domain",
            "com.apple.Accessibility",
            "--key",
            "AssistiveTouchEnabledByiTunes",
            "--type",
            "bool",
        ]);
        match cmd.command {
            LockdownSub::Set {
                value,
                domain,
                key,
                value_type,
            } => {
                assert_eq!(value, "true");
                assert_eq!(domain.as_deref(), Some("com.apple.Accessibility"));
                assert_eq!(key, "AssistiveTouchEnabledByiTunes");
                assert!(matches!(value_type, LockdownValueType::Bool));
            }
            other => panic!("expected set subcommand, got {other:?}"),
        }
    }

    #[test]
    fn parses_lockdown_remove_key() {
        let cmd = TestCli::parse_from([
            "lockdown",
            "remove",
            "--domain",
            "com.apple.mobile.wireless_lockdown",
            "--key",
            "EnableWifiConnections",
        ]);
        match cmd.command {
            LockdownSub::Remove { domain, key } => {
                assert_eq!(
                    domain.as_deref(),
                    Some("com.apple.mobile.wireless_lockdown")
                );
                assert_eq!(key, "EnableWifiConnections");
            }
            other => panic!("expected remove subcommand, got {other:?}"),
        }
    }

    #[test]
    fn parses_lockdown_save_pair_record() {
        let cmd = TestCli::parse_from([
            "lockdown",
            "save-pair-record",
            "ios-rs-tmp/exported-pair-record.plist",
        ]);
        match cmd.command {
            LockdownSub::SavePairRecord { output } => {
                assert_eq!(
                    output,
                    std::path::PathBuf::from("ios-rs-tmp/exported-pair-record.plist")
                );
            }
            other => panic!("expected save-pair-record subcommand, got {other:?}"),
        }
    }

    #[test]
    fn parses_lockdown_heartbeat_defaults() {
        let cmd = TestCli::parse_from(["lockdown", "heartbeat"]);
        match cmd.command {
            LockdownSub::Heartbeat { count, timeout } => {
                assert_eq!(count, 1);
                assert_eq!(timeout, None);
            }
            other => panic!("expected heartbeat subcommand, got {other:?}"),
        }
    }

    #[test]
    fn parses_lockdown_heartbeat_flags() {
        let cmd = TestCli::parse_from(["lockdown", "heartbeat", "--count", "2", "--timeout", "5"]);
        match cmd.command {
            LockdownSub::Heartbeat { count, timeout } => {
                assert_eq!(count, 2);
                assert_eq!(timeout, Some(5));
            }
            other => panic!("expected heartbeat subcommand, got {other:?}"),
        }
    }

    #[test]
    fn parses_lockdown_date_subcommand() {
        let cmd = TestCli::parse_from(["lockdown", "date"]);
        assert!(matches!(cmd.command, LockdownSub::Date));
    }

    #[test]
    fn parses_lockdown_device_name_subcommand() {
        let cmd = TestCli::parse_from(["lockdown", "device-name"]);
        assert!(matches!(
            cmd.command,
            LockdownSub::DeviceName { new_name: None }
        ));
    }

    #[test]
    fn parses_lockdown_device_name_with_value() {
        let cmd = TestCli::parse_from(["lockdown", "device-name", "Example Test Device"]);
        assert!(matches!(
            cmd.command,
            LockdownSub::DeviceName {
                new_name: Some(name)
            } if name == "Example Test Device"
        ));
    }

    #[test]
    fn parses_lockdown_language_subcommand() {
        let cmd = TestCli::parse_from(["lockdown", "language"]);
        assert!(matches!(
            cmd.command,
            LockdownSub::Language { details: false }
        ));
    }

    #[test]
    fn parses_lockdown_language_details_subcommand() {
        let cmd = TestCli::parse_from(["lockdown", "language", "--details"]);
        assert!(matches!(
            cmd.command,
            LockdownSub::Language { details: true }
        ));
    }

    #[test]
    fn parses_lockdown_locale_subcommand() {
        let cmd = TestCli::parse_from(["lockdown", "locale"]);
        assert!(matches!(
            cmd.command,
            LockdownSub::Locale { new_locale: None }
        ));
    }

    #[test]
    fn parses_lockdown_locale_with_value() {
        let cmd = TestCli::parse_from(["lockdown", "locale", "en_US"]);
        assert!(matches!(
            cmd.command,
            LockdownSub::Locale {
                new_locale: Some(locale)
            } if locale == "en_US"
        ));
    }

    #[test]
    fn parses_lockdown_assistive_touch_subcommand() {
        let cmd = TestCli::parse_from(["lockdown", "assistive-touch"]);
        assert!(matches!(
            cmd.command,
            LockdownSub::AssistiveTouch { state: None }
        ));
    }

    #[test]
    fn parses_lockdown_assistive_touch_on_subcommand() {
        let cmd = TestCli::parse_from(["lockdown", "assistive-touch", "on"]);
        assert!(matches!(
            cmd.command,
            LockdownSub::AssistiveTouch {
                state: Some(ToggleState::On)
            }
        ));
    }

    #[test]
    fn parses_lockdown_assistive_touch_off_subcommand() {
        let cmd = TestCli::parse_from(["lockdown", "assistive-touch", "off"]);
        assert!(matches!(
            cmd.command,
            LockdownSub::AssistiveTouch {
                state: Some(ToggleState::Off)
            }
        ));
    }

    #[test]
    fn parses_lockdown_voice_over_subcommand() {
        let cmd = TestCli::parse_from(["lockdown", "voice-over"]);
        assert!(matches!(
            cmd.command,
            LockdownSub::VoiceOver { state: None }
        ));
    }

    #[test]
    fn parses_lockdown_voice_over_on_subcommand() {
        let cmd = TestCli::parse_from(["lockdown", "voice-over", "on"]);
        assert!(matches!(
            cmd.command,
            LockdownSub::VoiceOver {
                state: Some(ToggleState::On)
            }
        ));
    }

    #[test]
    fn parses_lockdown_voice_over_off_subcommand() {
        let cmd = TestCli::parse_from(["lockdown", "voice-over", "off"]);
        assert!(matches!(
            cmd.command,
            LockdownSub::VoiceOver {
                state: Some(ToggleState::Off)
            }
        ));
    }

    #[test]
    fn parses_lockdown_zoom_touch_subcommand() {
        let cmd = TestCli::parse_from(["lockdown", "zoom-touch"]);
        assert!(matches!(
            cmd.command,
            LockdownSub::ZoomTouch { state: None }
        ));
    }

    #[test]
    fn parses_lockdown_zoom_touch_on_subcommand() {
        let cmd = TestCli::parse_from(["lockdown", "zoom-touch", "on"]);
        assert!(matches!(
            cmd.command,
            LockdownSub::ZoomTouch {
                state: Some(ToggleState::On)
            }
        ));
    }

    #[test]
    fn parses_lockdown_zoom_touch_off_subcommand() {
        let cmd = TestCli::parse_from(["lockdown", "zoom-touch", "off"]);
        assert!(matches!(
            cmd.command,
            LockdownSub::ZoomTouch {
                state: Some(ToggleState::Off)
            }
        ));
    }

    #[test]
    fn parses_lockdown_invert_display_subcommand() {
        let cmd = TestCli::parse_from(["lockdown", "invert-display"]);
        assert!(matches!(
            cmd.command,
            LockdownSub::InvertDisplay { state: None }
        ));
    }

    #[test]
    fn parses_lockdown_invert_display_on_subcommand() {
        let cmd = TestCli::parse_from(["lockdown", "invert-display", "on"]);
        assert!(matches!(
            cmd.command,
            LockdownSub::InvertDisplay {
                state: Some(ToggleState::On)
            }
        ));
    }

    #[test]
    fn parses_lockdown_invert_display_off_subcommand() {
        let cmd = TestCli::parse_from(["lockdown", "invert-display", "off"]);
        assert!(matches!(
            cmd.command,
            LockdownSub::InvertDisplay {
                state: Some(ToggleState::Off)
            }
        ));
    }

    #[test]
    fn parses_lockdown_time_format_subcommand() {
        let cmd = TestCli::parse_from(["lockdown", "time-format"]);
        assert!(matches!(
            cmd.command,
            LockdownSub::TimeFormat { format: None }
        ));
    }

    #[test]
    fn parses_lockdown_time_format_12_subcommand() {
        let cmd = TestCli::parse_from(["lockdown", "time-format", "12"]);
        assert!(matches!(
            cmd.command,
            LockdownSub::TimeFormat {
                format: Some(ClockFormat::Twelve)
            }
        ));
    }

    #[test]
    fn parses_lockdown_time_format_24_subcommand() {
        let cmd = TestCli::parse_from(["lockdown", "time-format", "24"]);
        assert!(matches!(
            cmd.command,
            LockdownSub::TimeFormat {
                format: Some(ClockFormat::TwentyFour)
            }
        ));
    }

    #[test]
    fn parses_lockdown_developer_mode_subcommand() {
        let cmd = TestCli::parse_from(["lockdown", "developer-mode"]);
        assert!(matches!(cmd.command, LockdownSub::DeveloperMode));
    }

    #[test]
    fn parses_lockdown_wifi_connections_subcommand() {
        let cmd = TestCli::parse_from(["lockdown", "wifi-connections"]);
        assert!(matches!(
            cmd.command,
            LockdownSub::WifiConnections { state: None }
        ));
    }

    #[test]
    fn parses_lockdown_wifi_connections_on_subcommand() {
        let cmd = TestCli::parse_from(["lockdown", "wifi-connections", "on"]);
        assert!(matches!(
            cmd.command,
            LockdownSub::WifiConnections {
                state: Some(ToggleState::On)
            }
        ));
    }

    #[test]
    fn parses_lockdown_wifi_connections_off_subcommand() {
        let cmd = TestCli::parse_from(["lockdown", "wifi-connections", "off"]);
        assert!(matches!(
            cmd.command,
            LockdownSub::WifiConnections {
                state: Some(ToggleState::Off)
            }
        ));
    }

    #[test]
    fn parses_lockdown_time_zone_subcommand() {
        let cmd = TestCli::parse_from(["lockdown", "time-zone"]);
        assert!(matches!(cmd.command, LockdownSub::TimeZone));
    }

    #[test]
    fn parses_lockdown_reset_accessibility_subcommand() {
        let parsed = TestCli::try_parse_from(["lockdown", "reset-accessibility"]);
        assert!(
            parsed.is_ok(),
            "lockdown reset-accessibility command should parse"
        );
    }

    #[test]
    fn parses_lockdown_font_size_subcommand() {
        let parsed = TestCli::try_parse_from(["lockdown", "font-size"]);
        assert!(parsed.is_ok(), "lockdown font-size command should parse");
    }

    #[test]
    fn formats_fractional_unix_timestamp() {
        let rendered = format_unix_timestamp(1_710_000_000.25).expect("timestamp should format");
        assert_eq!(rendered, "2024-03-09T16:00:00.250000000Z");
    }
}
