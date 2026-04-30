use anyhow::Result;
use ios_core::TunMode;
use ios_core::{connect, ConnectOptions};

#[derive(clap::Args)]
pub struct OsTraceCmd {
    #[command(subcommand)]
    sub: OsTraceSub,
}

#[derive(clap::Subcommand)]
enum OsTraceSub {
    /// Show the process list reported by os_trace_relay
    Ps,
}

impl OsTraceCmd {
    pub async fn run(self, udid: Option<String>, json: bool) -> Result<()> {
        let udid = udid.ok_or_else(|| anyhow::anyhow!("--udid required for os-trace"))?;
        let device = connect(
            &udid,
            ConnectOptions {
                tun_mode: TunMode::Userspace,
                pair_record_path: None,
                skip_tunnel: true,
            },
        )
        .await?;
        let stream = device
            .connect_service(ios_core::ostrace::SERVICE_NAME)
            .await?;
        let mut client = ios_core::ostrace::OsTraceClient::new(stream);

        match self.sub {
            OsTraceSub::Ps => {
                let response = client.get_pid_list().await?;
                if json {
                    println!(
                        "{}",
                        serde_json::to_string_pretty(&plist_to_json(&plist::Value::Dictionary(
                            response.clone(),
                        )))?
                    );
                } else {
                    print_pid_table(&response);
                }
            }
        }

        Ok(())
    }
}

fn print_pid_table(response: &plist::Dictionary) {
    let Some(payload) = response.get("Payload").and_then(plist::Value::as_array) else {
        println!(
            "{}",
            serde_json::to_string_pretty(response).unwrap_or_default()
        );
        return;
    };

    println!("{:<8} NAME", "PID");
    println!("{}", "-".repeat(48));
    for item in payload {
        let Some(dict) = item.as_dictionary() else {
            continue;
        };
        let pid = dict
            .get("PID")
            .and_then(plist::Value::as_unsigned_integer)
            .or_else(|| dict.get("Pid").and_then(plist::Value::as_unsigned_integer))
            .or_else(|| dict.get("pid").and_then(plist::Value::as_unsigned_integer))
            .unwrap_or_default();
        let name = dict
            .get("Name")
            .and_then(plist::Value::as_string)
            .or_else(|| dict.get("ProcessName").and_then(plist::Value::as_string))
            .or_else(|| dict.get("name").and_then(plist::Value::as_string))
            .unwrap_or("");
        println!("{:<8} {}", pid, name);
    }
}

fn plist_to_json(value: &plist::Value) -> serde_json::Value {
    match value {
        plist::Value::Array(values) => {
            serde_json::Value::Array(values.iter().map(plist_to_json).collect())
        }
        plist::Value::Dictionary(dict) => serde_json::Value::Object(
            dict.iter()
                .map(|(key, value)| (key.clone(), plist_to_json(value)))
                .collect(),
        ),
        plist::Value::Boolean(value) => serde_json::Value::Bool(*value),
        plist::Value::Data(bytes) => serde_json::Value::String(hex::encode(bytes)),
        plist::Value::Date(value) => serde_json::Value::String(value.to_xml_format()),
        plist::Value::Integer(value) => value
            .as_signed()
            .map(serde_json::Value::from)
            .or_else(|| value.as_unsigned().map(serde_json::Value::from))
            .unwrap_or(serde_json::Value::Null),
        plist::Value::Real(value) => serde_json::json!(value),
        plist::Value::String(value) => serde_json::Value::String(value.clone()),
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
        command: OsTraceSub,
    }

    #[test]
    fn parses_os_trace_ps_subcommand() {
        let parsed = TestCli::try_parse_from(["os-trace", "ps"]);
        assert!(parsed.is_ok(), "os-trace ps should parse");
    }
}
