use anyhow::Result;
use ios_core::MuxClient;
use ios_core::TunMode;
use ios_core::{connect, ConnectOptions, LockdownClient, PairRecord, LOCKDOWN_PORT};

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
            println!("{}", serde_json::to_string_pretty(&info)?);
        } else {
            println!("BatteryCurrentCapacity: {}", info.battery_current_capacity);
            println!("BatteryIsCharging:      {}", info.battery_is_charging);
            println!("ExternalChargeCapable:  {}", info.external_charge_capable);
            println!("ExternalConnected:      {}", info.external_connected);
            println!("FullyCharged:           {}", info.fully_charged);
            println!("GasGaugeCapability:     {}", info.gas_gauge_capability);
            println!("HasBattery:             {}", info.has_battery);
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
    let pair_record = PairRecord::load(udid)?;

    let mut mux = MuxClient::connect().await?;
    mux.read_pair_record(udid).await?;
    let stream = mux
        .connect_to_port(device.info.device_id, LOCKDOWN_PORT)
        .await?;
    let mut client = LockdownClient::connect_with_stream(stream, &pair_record).await?;

    Ok(BatteryInfo {
        battery_current_capacity: query_u64(&mut client, BATTERY_DOMAIN, "BatteryCurrentCapacity")
            .await?,
        battery_is_charging: query_bool(&mut client, BATTERY_DOMAIN, "BatteryIsCharging").await?,
        external_charge_capable: query_bool(&mut client, BATTERY_DOMAIN, "ExternalChargeCapable")
            .await?,
        external_connected: query_bool(&mut client, BATTERY_DOMAIN, "ExternalConnected").await?,
        fully_charged: query_bool(&mut client, BATTERY_DOMAIN, "FullyCharged").await?,
        gas_gauge_capability: query_bool(&mut client, BATTERY_DOMAIN, "GasGaugeCapability").await?,
        has_battery: query_bool(&mut client, BATTERY_DOMAIN, "HasBattery").await?,
    })
}

async fn query_bool(client: &mut LockdownClient, domain: &str, key: &str) -> Result<bool> {
    let value = client.get_value(Some(domain), Some(key)).await?;
    value
        .as_boolean()
        .ok_or_else(|| anyhow::anyhow!("{key} is not a boolean"))
}

async fn query_u64(client: &mut LockdownClient, domain: &str, key: &str) -> Result<u64> {
    let value = client.get_value(Some(domain), Some(key)).await?;
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
