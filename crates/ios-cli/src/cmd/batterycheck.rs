use anyhow::Result;
use ios_core::TunMode;
use ios_core::{connect, ConnectOptions, ConnectedDevice};

const BATTERY_DOMAIN: &str = "com.apple.mobile.battery";

#[derive(Debug, Clone, serde::Serialize)]
struct BatteryInfo {
    battery_current_capacity: u64,
    battery_is_charging: bool,
    external_charge_capable: bool,
    external_connected: bool,
    fully_charged: bool,
    gas_gauge_capability: bool,
    has_battery: bool,
}

#[derive(clap::Args)]
pub struct BatterycheckCmd {}

impl BatterycheckCmd {
    pub async fn run(self, udid: Option<String>, json: bool) -> Result<()> {
        let udid = udid.ok_or_else(|| anyhow::anyhow!("--udid required for batterycheck"))?;
        let info = fetch_battery_info(&udid).await?;

        if json {
            crate::output::outln!("{}", serde_json::to_string_pretty(&info)?);
        } else {
            crate::output::outln!("BatteryCurrentCapacity: {}", info.battery_current_capacity);
            crate::output::outln!("BatteryIsCharging:      {}", info.battery_is_charging);
            crate::output::outln!("ExternalChargeCapable:  {}", info.external_charge_capable);
            crate::output::outln!("ExternalConnected:      {}", info.external_connected);
            crate::output::outln!("FullyCharged:           {}", info.fully_charged);
            crate::output::outln!("GasGaugeCapability:     {}", info.gas_gauge_capability);
            crate::output::outln!("HasBattery:             {}", info.has_battery);
        }

        Ok(())
    }
}

async fn fetch_battery_info(udid: &str) -> Result<BatteryInfo> {
    let opts = ConnectOptions {
        tun_mode: TunMode::Userspace,
        pair_record_path: None,
        skip_tunnel: true,
    };
    let device = connect(udid, opts).await?;

    Ok(BatteryInfo {
        battery_current_capacity: query_u64(&device, BATTERY_DOMAIN, "BatteryCurrentCapacity")
            .await?,
        battery_is_charging: query_bool(&device, BATTERY_DOMAIN, "BatteryIsCharging").await?,
        external_charge_capable: query_bool(&device, BATTERY_DOMAIN, "ExternalChargeCapable")
            .await?,
        external_connected: query_bool(&device, BATTERY_DOMAIN, "ExternalConnected").await?,
        fully_charged: query_bool(&device, BATTERY_DOMAIN, "FullyCharged").await?,
        gas_gauge_capability: query_bool(&device, BATTERY_DOMAIN, "GasGaugeCapability").await?,
        has_battery: query_bool(&device, BATTERY_DOMAIN, "HasBattery").await?,
    })
}

async fn query_bool(device: &ConnectedDevice, domain: &str, key: &str) -> Result<bool> {
    let value = device
        .lockdown_get_value_in_domain(Some(domain), Some(key))
        .await?;
    value
        .as_boolean()
        .ok_or_else(|| anyhow::anyhow!("{key} is not a boolean"))
}

async fn query_u64(device: &ConnectedDevice, domain: &str, key: &str) -> Result<u64> {
    let value = device
        .lockdown_get_value_in_domain(Some(domain), Some(key))
        .await?;
    value
        .as_unsigned_integer()
        .ok_or_else(|| anyhow::anyhow!("{key} is not an unsigned integer"))
}

#[cfg(test)]
mod tests {
    use clap::Parser;

    use super::*;

    #[derive(Parser)]
    struct TestCli {
        #[command(flatten)]
        command: BatterycheckCmd,
    }

    #[test]
    fn parses_batterycheck_command() {
        let _cmd = TestCli::parse_from(["batterycheck"]);
    }
}
