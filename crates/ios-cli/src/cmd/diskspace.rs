use anyhow::Result;
use ios_core::{connect, ConnectOptions};
use ios_tunnel::TunMode;

const DISK_USAGE_DOMAIN: &str = "com.apple.disk_usage";

#[derive(clap::Args)]
pub struct DiskspaceCmd;

impl DiskspaceCmd {
    pub async fn run(self, udid: Option<String>, json: bool) -> Result<()> {
        let udid = udid.ok_or_else(|| anyhow::anyhow!("--udid required for diskspace"))?;
        let device = connect(
            &udid,
            ConnectOptions {
                tun_mode: TunMode::Userspace,
                pair_record_path: None,
                skip_tunnel: true,
            },
        )
        .await?;
        let value = device
            .lockdown_get_value_in_domain(Some(DISK_USAGE_DOMAIN), None)
            .await?;

        if json {
            println!("{}", serde_json::to_string_pretty(&plist_to_json(&value))?);
        } else {
            print_diskspace(&value);
        }

        Ok(())
    }
}

fn print_diskspace(value: &plist::Value) {
    let Some(dict) = value.as_dictionary() else {
        println!("{value:?}");
        return;
    };

    let keys = [
        "TotalDiskCapacity",
        "TotalDataCapacity",
        "TotalDataAvailable",
        "AmountDataAvailable",
        "AmountRestoreAvailable",
        "TotalSystemCapacity",
        "TotalSystemAvailable",
    ];

    for key in keys {
        if let Some(bytes) = dict.get(key).and_then(plist_value_to_u64) {
            println!("{key:<22} {bytes} ({:.2} GiB)", bytes_to_gib(bytes));
        }
    }
}

fn plist_value_to_u64(value: &plist::Value) -> Option<u64> {
    match value {
        plist::Value::Integer(value) => value
            .as_unsigned()
            .or_else(|| value.as_signed().map(|v| v as u64)),
        _ => None,
    }
}

fn bytes_to_gib(bytes: u64) -> f64 {
    bytes as f64 / 1024f64 / 1024f64 / 1024f64
}

fn plist_to_json(value: &plist::Value) -> serde_json::Value {
    match value {
        plist::Value::Array(items) => {
            serde_json::Value::Array(items.iter().map(plist_to_json).collect())
        }
        plist::Value::Boolean(v) => serde_json::Value::Bool(*v),
        plist::Value::Data(bytes) => serde_json::Value::String(hex::encode(bytes)),
        plist::Value::Date(date) => serde_json::Value::String(date.to_xml_format()),
        plist::Value::Dictionary(dict) => serde_json::Value::Object(
            dict.iter()
                .map(|(k, v)| (k.clone(), plist_to_json(v)))
                .collect(),
        ),
        plist::Value::Integer(n) => n
            .as_signed()
            .map(serde_json::Value::from)
            .or_else(|| n.as_unsigned().map(serde_json::Value::from))
            .unwrap_or(serde_json::Value::Null),
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
    use super::*;

    #[test]
    fn bytes_to_gib_uses_binary_units() {
        assert_eq!(bytes_to_gib(1_073_741_824), 1.0);
    }
}
