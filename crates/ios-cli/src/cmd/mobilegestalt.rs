use std::io::Write;

use anyhow::Result;
use ios_core::TunMode;
use ios_core::{connect, ConnectOptions};

const COREDEVICE_DEVICEINFO_SERVICE: &str = "com.apple.coredevice.deviceinfo";

#[derive(clap::Args)]
pub struct MobileGestaltCmd {
    #[arg(required = true, help = "One or more MobileGestalt keys to query")]
    keys: Vec<String>,
    #[arg(long, help = "Output XML plist instead of pretty JSON")]
    plist: bool,
}

impl MobileGestaltCmd {
    pub async fn run(self, udid: Option<String>, json: bool) -> Result<()> {
        let udid = udid.ok_or_else(|| anyhow::anyhow!("--udid required for mobilegestalt"))?;

        let opts = ConnectOptions {
            tun_mode: TunMode::Userspace,
            pair_record_path: None,
            skip_tunnel: true,
        };
        let device = connect(&udid, opts).await?;
        let mut stream = device
            .connect_service(ios_core::diagnostics::SERVICE_NAME)
            .await?;

        let key_refs: Vec<&str> = self.keys.iter().map(String::as_str).collect();
        let value = match ios_core::diagnostics::query_mobile_gestalt(&mut *stream, &key_refs).await
        {
            Ok(value) => value,
            Err(ios_core::diagnostics::DiagnosticsError::Deprecated(message)) => {
                let rsd_state = match connect(
                    &udid,
                    ConnectOptions {
                        tun_mode: TunMode::Userspace,
                        pair_record_path: None,
                        skip_tunnel: false,
                    },
                )
                .await
                {
                    Ok(device) => describe_deviceinfo_service(device.rsd()),
                    Err(err) => {
                        format!("failed to inspect RSD services for CoreDevice fallback: {err}")
                    }
                };
                return Err(anyhow::anyhow!("{message}; {rsd_state}"));
            }
            Err(err) => return Err(err.into()),
        };

        if self.plist {
            let mut stdout = std::io::stdout().lock();
            plist::to_writer_xml(&mut stdout, &value)?;
            writeln!(&mut stdout)?;
        } else if json {
            println!("{}", serde_json::to_string_pretty(&value)?);
        } else {
            for (key, rendered) in mobilegestalt_detail_lines(&value) {
                println!("{key}: {rendered}");
            }
        }

        Ok(())
    }
}

fn describe_deviceinfo_service(rsd: Option<&ios_core::RsdHandshake>) -> String {
    let Some(rsd) = rsd else {
        return "RSD is not available in this session, so CoreDevice mobilegestalt fallback cannot be attempted".to_string();
    };

    if rsd.services.contains_key(COREDEVICE_DEVICEINFO_SERVICE) {
        return format!(
            "{COREDEVICE_DEVICEINFO_SERVICE} is exposed in this RSD session, but a CoreDevice mobilegestalt fallback is not implemented yet"
        );
    }

    let shim = format!("{COREDEVICE_DEVICEINFO_SERVICE}.shim.remote");
    if rsd.services.contains_key(&shim) {
        return format!(
            "{shim} is exposed in this RSD session, but a CoreDevice mobilegestalt fallback is not implemented yet"
        );
    }

    format!("current RSD session does not expose {COREDEVICE_DEVICEINFO_SERVICE}")
}

fn mobilegestalt_detail_lines(value: &plist::Value) -> Vec<(String, String)> {
    match value {
        plist::Value::Dictionary(dict) => {
            let mut lines: Vec<_> = dict
                .iter()
                .map(|(key, value)| (key.clone(), format_plist_value(value)))
                .collect();
            lines.sort_by(|a, b| a.0.cmp(&b.0));
            lines
        }
        other => vec![("Value".to_string(), format_plist_value(other))],
    }
}

fn format_plist_value(value: &plist::Value) -> String {
    match value {
        plist::Value::String(v) => v.clone(),
        plist::Value::Boolean(v) => v.to_string(),
        plist::Value::Integer(v) => {
            if let Some(i) = v.as_signed() {
                i.to_string()
            } else if let Some(u) = v.as_unsigned() {
                u.to_string()
            } else {
                "null".to_string()
            }
        }
        plist::Value::Real(v) => v.to_string(),
        other => serde_json::to_string(&other).unwrap_or_else(|_| "null".to_string()),
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use clap::Parser;
    use ios_core::{RsdHandshake, ServiceDescriptor};

    use super::*;

    #[derive(Parser)]
    struct TestCli {
        #[command(flatten)]
        command: MobileGestaltCmd,
    }

    #[test]
    fn parses_mobilegestalt_keys_and_plist_flag() {
        let cmd = TestCli::parse_from([
            "mobilegestalt",
            "ProductVersion",
            "MainScreenCanvasSizes",
            "--plist",
        ]);
        assert_eq!(
            cmd.command.keys,
            vec![
                "ProductVersion".to_string(),
                "MainScreenCanvasSizes".to_string()
            ]
        );
        assert!(cmd.command.plist);
    }

    #[test]
    fn detail_lines_render_scalar_values() {
        let value = plist::Value::Dictionary(plist::Dictionary::from_iter([(
            "ProductVersion".to_string(),
            plist::Value::String("26.0".into()),
        )]));

        assert_eq!(
            mobilegestalt_detail_lines(&value),
            vec![("ProductVersion".to_string(), "26.0".to_string())]
        );
    }

    #[test]
    fn detail_lines_render_nested_values_as_json() {
        let value = plist::Value::Dictionary(plist::Dictionary::from_iter([(
            "MainScreenCanvasSizes".to_string(),
            plist::Value::Array(vec![plist::Value::Dictionary(
                plist::Dictionary::from_iter([
                    ("Height".to_string(), plist::Value::Integer(2796.into())),
                    ("Width".to_string(), plist::Value::Integer(1290.into())),
                ]),
            )]),
        )]));

        let lines = mobilegestalt_detail_lines(&value);
        assert_eq!(lines.len(), 1);
        assert_eq!(lines[0].0, "MainScreenCanvasSizes");
        assert_eq!(lines[0].1, r#"[{"Height":2796,"Width":1290}]"#);
    }

    #[test]
    fn describe_deviceinfo_service_reports_missing_service() {
        let rsd = RsdHandshake {
            udid: "test".into(),
            services: HashMap::from([(
                "com.apple.coredevice.appservice".into(),
                ServiceDescriptor { port: 1234 },
            )]),
        };

        assert_eq!(
            describe_deviceinfo_service(Some(&rsd)),
            "current RSD session does not expose com.apple.coredevice.deviceinfo"
        );
    }

    #[test]
    fn describe_deviceinfo_service_reports_shim_exposure() {
        let rsd = RsdHandshake {
            udid: "test".into(),
            services: HashMap::from([(
                "com.apple.coredevice.deviceinfo.shim.remote".into(),
                ServiceDescriptor { port: 1234 },
            )]),
        };

        assert_eq!(
            describe_deviceinfo_service(Some(&rsd)),
            "com.apple.coredevice.deviceinfo.shim.remote is exposed in this RSD session, but a CoreDevice mobilegestalt fallback is not implemented yet"
        );
    }
}
