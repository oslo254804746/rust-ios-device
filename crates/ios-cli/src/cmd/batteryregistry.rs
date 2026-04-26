use anyhow::Result;
use ios_core::tunnel::TunMode;
use ios_core::{connect, ConnectOptions};

#[derive(clap::Args)]
pub struct BatteryregistryCmd {}

impl BatteryregistryCmd {
    pub async fn run(self, udid: Option<String>, json: bool) -> Result<()> {
        let udid = udid.ok_or_else(|| anyhow::anyhow!("--udid required for batteryregistry"))?;

        let opts = ConnectOptions {
            tun_mode: TunMode::Userspace,
            pair_record_path: None,
            skip_tunnel: true,
        };
        let device = connect(&udid, opts).await?;
        let mut stream = device
            .connect_service(ios_core::services::diagnostics::SERVICE_NAME)
            .await?;
        let battery = ios_core::services::diagnostics::query_battery(&mut *stream).await?;

        if json {
            println!("{}", serde_json::to_string_pretty(&battery)?);
        } else {
            println!(
                "InstantAmperage:        {}",
                display_opt(battery.instant_amperage)
            );
            println!(
                "Temperature:            {}",
                display_opt(battery.temperature)
            );
            println!("Voltage:                {}", display_opt(battery.voltage));
            println!(
                "IsCharging:             {}",
                display_opt(battery.is_charging)
            );
            println!(
                "CurrentCapacity:        {}",
                display_opt(battery.current_capacity)
            );
            println!(
                "DesignCapacity:         {}",
                display_opt(battery.design_capacity)
            );
            println!(
                "NominalChargeCapacity:  {}",
                display_opt(battery.nominal_charge_capacity)
            );
            println!(
                "AbsoluteCapacity:       {}",
                display_opt(battery.absolute_capacity)
            );
            println!(
                "AppleRawCurrentCapacity:{}",
                display_opt(battery.apple_raw_current_capacity)
            );
            println!(
                "AppleRawMaxCapacity:    {}",
                display_opt(battery.apple_raw_max_capacity)
            );
            println!(
                "CycleCount:             {}",
                display_opt(battery.cycle_count)
            );
            println!(
                "AtCriticalLevel:        {}",
                display_opt(battery.at_critical_level)
            );
            println!(
                "AtWarnLevel:            {}",
                display_opt(battery.at_warn_level)
            );
        }

        Ok(())
    }
}

fn display_opt<T: std::fmt::Display>(value: Option<T>) -> String {
    value
        .map(|v| v.to_string())
        .unwrap_or_else(|| "N/A".to_string())
}

#[cfg(test)]
mod tests {
    use clap::Parser;

    use super::*;

    #[derive(Parser)]
    struct TestCli {
        #[command(flatten)]
        command: BatteryregistryCmd,
    }

    #[test]
    fn parses_batteryregistry_command() {
        let _cmd = TestCli::parse_from(["batteryregistry"]);
    }
}
