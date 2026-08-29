//! iOS 17+ CoreDevice appearance/accessibility and orientation controls.

use anyhow::{Context, Result};
use clap::ValueEnum;
use ios_core::configuration::{
    ColorFilterType, ConfigurationServiceClient, DeviceTextSize, UserInterfaceStyle,
};
use ios_core::orientation::{OrientationServiceClient, OrientationState, RotationDirection};
use ios_core::{connect, ConnectOptions, TunMode};

#[derive(clap::Args)]
pub struct DeviceControlCmd {
    #[command(subcommand)]
    sub: DeviceControlSub,
}

#[derive(clap::Subcommand)]
enum DeviceControlSub {
    /// Read or change appearance and accessibility settings.
    ///
    /// Set operations mutate device-wide UI state and may affect the person
    /// using the device. This requires an iOS 17+ CoreDevice tunnel.
    Configuration(ConfigurationCmd),
    /// Rotate the device by one 90-degree step.
    ///
    /// Rotation changes the physical device UI state and may interrupt an
    /// active user session. This requires an iOS 17+ CoreDevice tunnel.
    Orientation(OrientationCmd),
}

#[derive(clap::Args)]
struct ConfigurationCmd {
    #[command(subcommand)]
    sub: ConfigurationSub,
}

#[derive(clap::Subcommand)]
enum ConfigurationSub {
    /// Read one appearance/accessibility value as JSON or human-readable text.
    Get {
        #[arg(value_name = "SETTING")]
        setting: ConfigurationSetting,
    },
    /// Change one appearance/accessibility value.
    Set {
        #[arg(value_name = "SETTING")]
        setting: ConfigurationSetting,
        #[arg(
            value_name = "VALUE",
            help = "Setting value (for booleans use true or false)"
        )]
        value: String,
        #[arg(
            long,
            value_name = "TYPE",
            help = "Color filter preset, e.g. Grayscale or Protanopia"
        )]
        filter_type: Option<String>,
        #[arg(
            long,
            value_name = "FLOAT",
            help = "Color filter intensity in the inclusive range 0.0..=1.0"
        )]
        intensity: Option<f64>,
    },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, ValueEnum)]
#[value(rename_all = "kebab-case")]
enum ConfigurationSetting {
    Style,
    ColorFilter,
    TextSize,
    ReduceMotion,
    IncreaseContrast,
    ShowBorders,
    ReduceTransparency,
    LiquidGlassOpacity,
}

#[derive(clap::Args)]
struct OrientationCmd {
    /// Rotate 90 degrees counter-clockwise (left) or clockwise (right).
    #[arg(value_name = "DIRECTION", default_value = "left")]
    direction: CliRotationDirection,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, ValueEnum)]
#[value(rename_all = "kebab-case")]
enum CliRotationDirection {
    Left,
    Right,
}

impl From<CliRotationDirection> for RotationDirection {
    fn from(direction: CliRotationDirection) -> Self {
        match direction {
            CliRotationDirection::Left => Self::Left,
            CliRotationDirection::Right => Self::Right,
        }
    }
}

impl DeviceControlCmd {
    pub async fn run(self, udid: Option<String>, json_output: bool) -> Result<()> {
        let udid = udid.ok_or_else(|| anyhow::anyhow!("--udid required for device-control"))?;
        match self.sub {
            DeviceControlSub::Configuration(command) => command.run(&udid, json_output).await,
            DeviceControlSub::Orientation(command) => command.run(&udid, json_output).await,
        }
    }
}

impl ConfigurationCmd {
    async fn run(self, udid: &str, json_output: bool) -> Result<()> {
        match self.sub {
            ConfigurationSub::Get { setting } => {
                if matches!(
                    setting,
                    ConfigurationSetting::IncreaseContrast
                        | ConfigurationSetting::LiquidGlassOpacity
                ) {
                    anyhow::bail!(
                        "configuration setting '{}' has no CoreDevice getter",
                        setting_name(setting)
                    );
                }

                let (mut client, _device) = connect_configuration(udid).await?;
                let value = match setting {
                    ConfigurationSetting::Style => {
                        serde_json::json!(client.get_user_interface_style().await?.to_string())
                    }
                    ConfigurationSetting::ColorFilter => {
                        serde_json::to_value(client.get_color_filter().await?)?
                    }
                    ConfigurationSetting::TextSize => {
                        serde_json::json!(client.get_device_text_size().await?.to_string())
                    }
                    ConfigurationSetting::ReduceMotion => {
                        serde_json::json!(client.get_reduce_motion().await?)
                    }
                    ConfigurationSetting::ShowBorders => {
                        serde_json::json!(client.get_show_borders().await?)
                    }
                    ConfigurationSetting::ReduceTransparency => {
                        serde_json::json!(client.get_reduce_transparency().await?)
                    }
                    ConfigurationSetting::IncreaseContrast
                    | ConfigurationSetting::LiquidGlassOpacity => unreachable!(),
                };
                render_setting(setting, value, json_output)
            }
            ConfigurationSub::Set {
                setting,
                value,
                filter_type,
                intensity,
            } => {
                let (mut client, _device) = connect_configuration(udid).await?;
                let result_value = match setting {
                    ConfigurationSetting::Style => {
                        let style: UserInterfaceStyle =
                            value.parse().map_err(anyhow::Error::msg)?;
                        client.set_user_interface_style(style.clone()).await?;
                        serde_json::json!(style.to_string())
                    }
                    ConfigurationSetting::ColorFilter => {
                        let enabled = parse_bool(&value)?;
                        let filter_type = filter_type
                            .map(|value| value.parse::<ColorFilterType>())
                            .transpose()
                            .map_err(anyhow::Error::msg)?;
                        let filter_type_name = filter_type.as_ref().map(ToString::to_string);
                        client
                            .set_color_filter(enabled, filter_type, intensity)
                            .await?;
                        let mut result = serde_json::json!({"enabled": enabled});
                        if enabled {
                            if let Some(filter_type) = filter_type_name {
                                result["filterType"] = serde_json::Value::String(filter_type);
                            }
                            if let Some(intensity) = intensity {
                                result["intensity"] = serde_json::json!(intensity);
                            }
                        }
                        result
                    }
                    ConfigurationSetting::TextSize => {
                        let size: DeviceTextSize = value.parse().map_err(anyhow::Error::msg)?;
                        client.set_device_text_size(size.clone()).await?;
                        serde_json::json!(size.to_string())
                    }
                    ConfigurationSetting::ReduceMotion => {
                        let enabled = parse_bool(&value)?;
                        client.set_reduce_motion(enabled).await?;
                        serde_json::json!(enabled)
                    }
                    ConfigurationSetting::IncreaseContrast => {
                        let enabled = parse_bool(&value)?;
                        client.set_increase_contrast(enabled).await?;
                        serde_json::json!(enabled)
                    }
                    ConfigurationSetting::ShowBorders => {
                        let enabled = parse_bool(&value)?;
                        client.set_show_borders(enabled).await?;
                        serde_json::json!(enabled)
                    }
                    ConfigurationSetting::ReduceTransparency => {
                        let enabled = parse_bool(&value)?;
                        client.set_reduce_transparency(enabled).await?;
                        serde_json::json!(enabled)
                    }
                    ConfigurationSetting::LiquidGlassOpacity => {
                        let opacity = value.parse()?;
                        client.set_liquid_glass_opacity(opacity).await?;
                        serde_json::json!(opacity)
                    }
                };
                if json_output {
                    println!(
                        "{}",
                        serde_json::to_string_pretty(&set_operation_json(setting, result_value))?
                    );
                } else {
                    println!("Set {} to {result_value}", setting_name(setting));
                }
                Ok(())
            }
        }
    }
}

impl OrientationCmd {
    async fn run(self, udid: &str, json_output: bool) -> Result<()> {
        let direction = self.direction.into();
        let (mut client, _device) = connect_orientation(udid).await?;
        let state = client.rotate(direction).await?;
        if json_output {
            println!(
                "{}",
                serde_json::to_string_pretty(&serde_json::json!({
                    "direction": direction.to_string(),
                    "state": state,
                }))?
            );
        } else {
            print_orientation(direction, &state);
        }
        Ok(())
    }
}

async fn connect_configuration(
    udid: &str,
) -> Result<(ConfigurationServiceClient, ios_core::ConnectedDevice)> {
    let device = connect(
        udid,
        ConnectOptions {
            tun_mode: TunMode::Userspace,
            pair_record_path: None,
            skip_tunnel: false,
        },
    )
    .await
    .context("failed to establish a CoreDevice tunnel")?;
    let (xpc, metadata) = device
        .connect_xpc_service_with_metadata(ios_core::configuration::SERVICE_NAME)
        .await
        .context("configuration service is unavailable in the device RSD directory")?;
    Ok((
        ConfigurationServiceClient::new_with_features(xpc, udid, metadata.features),
        device,
    ))
}

async fn connect_orientation(
    udid: &str,
) -> Result<(OrientationServiceClient, ios_core::ConnectedDevice)> {
    let device = connect(
        udid,
        ConnectOptions {
            tun_mode: TunMode::Userspace,
            pair_record_path: None,
            skip_tunnel: false,
        },
    )
    .await
    .context("failed to establish a CoreDevice tunnel")?;
    let (xpc, metadata) = device
        .connect_xpc_service_with_metadata(ios_core::orientation::SERVICE_NAME)
        .await
        .context("orientation service is unavailable in the device RSD directory")?;
    Ok((
        OrientationServiceClient::new_with_features(xpc, metadata.features),
        device,
    ))
}

fn parse_bool(value: &str) -> Result<bool> {
    value
        .parse::<bool>()
        .map_err(|_| anyhow::anyhow!("boolean value must be 'true' or 'false', got {value:?}"))
}

fn setting_name(setting: ConfigurationSetting) -> &'static str {
    match setting {
        ConfigurationSetting::Style => "style",
        ConfigurationSetting::ColorFilter => "color-filter",
        ConfigurationSetting::TextSize => "text-size",
        ConfigurationSetting::ReduceMotion => "reduce-motion",
        ConfigurationSetting::IncreaseContrast => "increase-contrast",
        ConfigurationSetting::ShowBorders => "show-borders",
        ConfigurationSetting::ReduceTransparency => "reduce-transparency",
        ConfigurationSetting::LiquidGlassOpacity => "liquid-glass-opacity",
    }
}

fn render_setting(
    setting: ConfigurationSetting,
    value: serde_json::Value,
    json_output: bool,
) -> Result<()> {
    if json_output {
        println!(
            "{}",
            serde_json::to_string_pretty(&setting_json(setting, value))?
        );
    } else {
        println!("{}: {value}", setting_name(setting));
    }
    Ok(())
}

fn setting_json(setting: ConfigurationSetting, value: serde_json::Value) -> serde_json::Value {
    serde_json::json!({
        "setting": setting_name(setting),
        "value": value,
    })
}

fn set_operation_json(
    setting: ConfigurationSetting,
    value: serde_json::Value,
) -> serde_json::Value {
    serde_json::json!({
        "operation": "set",
        "setting": setting_name(setting),
        "value": value,
    })
}

fn print_orientation(direction: RotationDirection, state: &OrientationState) {
    println!("Rotated {}", direction);
    println!("orientation: {}", state.current_device_orientation);
    println!(
        "non-flat orientation: {}",
        state.current_device_non_flat_orientation
    );
    println!(
        "orientation locked: {}",
        state.current_device_orientation_locked
    );
}

#[cfg(test)]
mod tests {
    use clap::Parser;

    use super::*;

    #[derive(Parser)]
    struct TestCli {
        #[command(subcommand)]
        command: DeviceControlSub,
    }

    #[test]
    fn parses_configuration_get_and_set() {
        let parsed = TestCli::parse_from(["device-control", "configuration", "get", "style"]);
        assert!(matches!(
            parsed.command,
            DeviceControlSub::Configuration(ConfigurationCmd {
                sub: ConfigurationSub::Get {
                    setting: ConfigurationSetting::Style
                }
            })
        ));

        let parsed = TestCli::parse_from([
            "device-control",
            "configuration",
            "set",
            "color-filter",
            "true",
            "--filter-type",
            "Protanopia",
            "--intensity",
            "0.5",
        ]);
        assert!(matches!(
            parsed.command,
            DeviceControlSub::Configuration(ConfigurationCmd {
                sub: ConfigurationSub::Set {
                    setting: ConfigurationSetting::ColorFilter,
                    value,
                    filter_type: Some(_),
                    intensity: Some(0.5),
                }
            }) if value == "true"
        ));
    }

    #[test]
    fn parses_orientation_direction_and_defaults_left() {
        let parsed = TestCli::parse_from(["device-control", "orientation", "right"]);
        assert!(matches!(
            parsed.command,
            DeviceControlSub::Orientation(OrientationCmd {
                direction: CliRotationDirection::Right
            })
        ));

        let parsed = TestCli::parse_from(["device-control", "orientation"]);
        assert!(matches!(
            parsed.command,
            DeviceControlSub::Orientation(OrientationCmd {
                direction: CliRotationDirection::Left
            })
        ));
    }

    #[test]
    fn rejects_unknown_setting_and_direction() {
        assert!(
            TestCli::try_parse_from(["device-control", "configuration", "get", "unknown"]).is_err()
        );
        assert!(TestCli::try_parse_from(["device-control", "orientation", "up"]).is_err());
    }

    #[test]
    fn setting_names_are_stable_for_json_contract() {
        assert_eq!(setting_name(ConfigurationSetting::TextSize), "text-size");
        assert_eq!(
            setting_name(ConfigurationSetting::LiquidGlassOpacity),
            "liquid-glass-opacity"
        );
    }

    #[test]
    fn json_output_snapshots_keep_public_values_flat() {
        assert_eq!(
            setting_json(
                ConfigurationSetting::ColorFilter,
                serde_json::json!({
                    "enabled": true,
                    "filterType": "InvertColors"
                }),
            ),
            serde_json::json!({
                "setting": "color-filter",
                "value": {
                    "enabled": true,
                    "filterType": "InvertColors"
                }
            })
        );
        assert_eq!(
            set_operation_json(
                ConfigurationSetting::TextSize,
                serde_json::json!("accessibilityGigantic"),
            ),
            serde_json::json!({
                "operation": "set",
                "setting": "text-size",
                "value": "accessibilityGigantic"
            })
        );
    }
}
