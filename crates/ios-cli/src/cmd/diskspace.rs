use anyhow::Result;
use ios_core::plist_json::{self, DataEncoding, PlistJsonPolicy};
use ios_core::TunMode;
use ios_core::{connect, ConnectOptions};

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
            crate::output::outln!("{}", serde_json::to_string_pretty(&plist_to_json(&value))?);
        } else {
            print_diskspace(&value)?;
        }

        Ok(())
    }
}

fn print_diskspace(value: &plist::Value) -> Result<()> {
    let Some(dict) = value.as_dictionary() else {
        crate::output::outln!("{value:?}");
        return Ok(());
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
            crate::output::outln!("{key:<22} {bytes} ({:.2} GiB)", bytes_to_gib(bytes));
        }
    }
    Ok(())
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
    plist_json::to_json(value, PlistJsonPolicy::new(DataEncoding::HexString))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bytes_to_gib_uses_binary_units() {
        assert_eq!(bytes_to_gib(1_073_741_824), 1.0);
    }
}

#[cfg(test)]
#[test]
fn plist_json_legacy_contract() {
    crate::plist_contract::check(
        plist_to_json(&crate::plist_contract::value()),
        serde_json::json!("007fff"),
        serde_json::json!(""),
        true,
    );
}
