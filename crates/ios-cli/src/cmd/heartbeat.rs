use ios_core::plist_json::{self, DataEncoding, PlistJsonPolicy};
use std::time::Duration;

use anyhow::Result;
use ios_core::TunMode;
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
    if let Some(t) = timeout {
        client.set_idle_timeout(Some(Duration::from_secs(t)));
        client.set_operation_timeout(Some(Duration::from_secs(t)));
    } else {
        client.set_idle_timeout(None);
        client.set_operation_timeout(Some(Duration::from_secs(30)));
    }

    let mut messages = Vec::with_capacity(count);
    for _ in 0..count {
        let message = client.ping().await?;
        messages.push(message);
    }

    if json {
        let list: Vec<_> = messages.iter().map(plist_to_json).collect();
        crate::output::outln!("{}", serde_json::to_string_pretty(&list)?);
    } else {
        for (idx, message) in messages.iter().enumerate() {
            crate::output::outln!("Heartbeat {}", idx + 1);
            crate::output::outln!("{}", serde_json::to_string_pretty(&plist_to_json(message))?);
        }
    }

    Ok(())
}

fn plist_to_json(value: &plist::Value) -> serde_json::Value {
    plist_json::to_json(value, PlistJsonPolicy::new(DataEncoding::NumericArray))
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

#[cfg(test)]
#[test]
fn plist_json_legacy_contract() {
    crate::plist_contract::check(
        plist_to_json(&crate::plist_contract::value()),
        serde_json::json!([0, 127, 255]),
        serde_json::json!([]),
        true,
    );
}
