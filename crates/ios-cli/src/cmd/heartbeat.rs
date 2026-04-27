use std::time::Duration;

use anyhow::Result;
use ios_core::tunnel::TunMode;
use ios_core::{connect, ConnectOptions};

#[derive(clap::Args)]
pub struct HeartbeatCmd {
    #[arg(
        long,
        default_value_t = 1,
        help = "Number of heartbeat exchanges to perform"
    )]
    count: usize,
    #[arg(long, help = "Timeout in seconds for each heartbeat exchange")]
    timeout: Option<u64>,
}

impl HeartbeatCmd {
    pub async fn run(self, udid: Option<String>, json: bool) -> Result<()> {
        let udid = udid.ok_or_else(|| anyhow::anyhow!("--udid required for heartbeat"))?;
        run_heartbeat(&udid, json, self.count, self.timeout).await
    }
}

pub(crate) async fn run_heartbeat(
    udid: &str,
    json: bool,
    count: usize,
    timeout: Option<u64>,
) -> Result<()> {
    let opts = ConnectOptions {
        tun_mode: TunMode::Userspace,
        pair_record_path: None,
        skip_tunnel: true,
    };
    let device = connect(udid, opts).await?;
    let stream = device
        .connect_service(ios_core::heartbeat::SERVICE_NAME)
        .await?;
    let mut client = ios_core::heartbeat::HeartbeatClient::new(stream);

    let mut messages = Vec::with_capacity(count);
    for _ in 0..count {
        let message = match timeout {
            Some(timeout) => tokio::time::timeout(Duration::from_secs(timeout), client.ping())
                .await
                .map_err(|_| anyhow::anyhow!("timed out waiting for heartbeat"))??,
            None => client.ping().await?,
        };
        messages.push(message);
    }

    if json {
        let list: Vec<_> = messages.iter().map(plist_to_json).collect();
        println!("{}", serde_json::to_string_pretty(&list)?);
    } else {
        for (idx, message) in messages.iter().enumerate() {
            println!("Heartbeat {}", idx + 1);
            println!("{}", serde_json::to_string_pretty(&plist_to_json(message))?);
        }
    }

    Ok(())
}

fn plist_to_json(value: &plist::Value) -> serde_json::Value {
    match value {
        plist::Value::Array(items) => {
            serde_json::Value::Array(items.iter().map(plist_to_json).collect())
        }
        plist::Value::Boolean(v) => serde_json::Value::Bool(*v),
        plist::Value::Data(bytes) => serde_json::Value::Array(
            bytes
                .iter()
                .map(|byte| serde_json::Value::from(*byte))
                .collect(),
        ),
        plist::Value::Date(date) => serde_json::Value::String(date.to_xml_format()),
        plist::Value::Dictionary(dict) => serde_json::Value::Object(
            dict.iter()
                .map(|(k, v)| (k.clone(), plist_to_json(v)))
                .collect(),
        ),
        plist::Value::Integer(n) => {
            if let Some(i) = n.as_signed() {
                serde_json::Value::from(i)
            } else if let Some(u) = n.as_unsigned() {
                serde_json::Value::from(u)
            } else {
                serde_json::Value::Null
            }
        }
        plist::Value::Real(v) => serde_json::Number::from_f64(*v)
            .map(serde_json::Value::Number)
            .unwrap_or(serde_json::Value::Null),
        plist::Value::String(s) => serde_json::Value::String(s.clone()),
        plist::Value::Uid(uid) => serde_json::Value::from(uid.get()),
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
        command: HeartbeatCmd,
    }

    #[test]
    fn parses_heartbeat_count_flag() {
        let cmd = TestCli::parse_from(["heartbeat", "--count", "3"]);
        assert_eq!(cmd.command.count, 3);
    }

    #[test]
    fn parses_heartbeat_timeout_flag() {
        let cmd = TestCli::parse_from(["heartbeat", "--timeout", "5"]);
        assert_eq!(cmd.command.timeout, Some(5));
    }
}
