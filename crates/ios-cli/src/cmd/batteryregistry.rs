use anyhow::Result;
use ios_core::TunMode;
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
            .connect_service(ios_core::diagnostics::SERVICE_NAME)
            .await?;
        let battery = ios_core::diagnostics::query_battery(&mut *stream).await?;

        if json {
            crate::output::outln!("{}", serde_json::to_string_pretty(&battery)?);
        } else {
            crate::output::outln!(
                "InstantAmperage:        {}",
                display_opt(battery.instant_amperage)
            );
            crate::output::outln!(
                "Temperature:            {}",
                display_opt(battery.temperature)
            );
            crate::output::outln!("Voltage:                {}", display_opt(battery.voltage));
            crate::output::outln!(
                "IsCharging:             {}",
                display_opt(battery.is_charging)
            );
            crate::output::outln!(
                "CurrentCapacity:        {}",
                display_opt(battery.current_capacity)
            );
            crate::output::outln!(
                "DesignCapacity:         {}",
                display_opt(battery.design_capacity)
            );
            crate::output::outln!(
                "NominalChargeCapacity:  {}",
                display_opt(battery.nominal_charge_capacity)
            );
            crate::output::outln!(
                "AbsoluteCapacity:       {}",
                display_opt(battery.absolute_capacity)
            );
            crate::output::outln!(
                "AppleRawCurrentCapacity:{}",
                display_opt(battery.apple_raw_current_capacity)
            );
            crate::output::outln!(
                "AppleRawMaxCapacity:    {}",
                display_opt(battery.apple_raw_max_capacity)
            );
            crate::output::outln!(
                "CycleCount:             {}",
                display_opt(battery.cycle_count)
            );
            crate::output::outln!(
                "AtCriticalLevel:        {}",
                display_opt(battery.at_critical_level)
            );
            crate::output::outln!(
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
