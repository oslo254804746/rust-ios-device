//! iOS 17+ CoreDevice appearance/accessibility and orientation controls.

use anyhow::{Context, Result};
use clap::ValueEnum;
use ios_core::configuration::{
    ColorFilterType, ConfigurationServiceClient, DeviceTextSize, UserInterfaceStyle,
};
use ios_core::display::{DisplayServiceClient, MediaKind, MediaStreamOptions, MediaStreamSession};
use ios_core::orientation::{OrientationServiceClient, OrientationState, RotationDirection};
use ios_core::{connect, ConnectOptions, TunMode};
use std::path::PathBuf;
use std::time::Duration;

const MAX_CAPTURE_OUTPUT_BYTES: usize = 256 * 1024 * 1024;

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
    /// Query or capture an encoded CoreDevice display media stream.
    Display(DisplayCmd),
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
            DeviceControlSub::Display(command) => command.run(&udid, json_output).await,
        }
    }
}

#[derive(clap::Args)]
struct DisplayCmd {
    #[command(subcommand)]
    sub: DisplaySub,
}

#[derive(clap::Subcommand)]
enum DisplaySub {
    /// Read media capabilities and active stream status.
    Status,
    /// Capture bounded, encoded HEVC RTP access units (no pixel decoding).
    Video(MediaCaptureCmd),
    /// Capture bounded, encoded AAC RTP access units (no audio decoding).
    Audio(MediaCaptureCmd),
}

#[derive(clap::Args)]
struct MediaCaptureCmd {
    /// Device/tunnel IPv6 address advertised as the sender endpoint.
    #[arg(long)]
    sender_ip: Option<String>,
    /// Host tunnel address to bind for incoming RTP. When omitted, uses the
    /// CDTunnel handshake's client address; `::1` is never guessed for a
    /// device-initiated datagram.
    #[arg(long)]
    receiver_ip: Option<String>,
    /// Stop after this many access units.
    #[arg(long, default_value_t = 1)]
    max_units: usize,
    /// Absolute capture deadline.
    #[arg(long, default_value_t = 10)]
    timeout: u64,
    /// Optional 0600 output file containing concatenated encoded access units.
    #[arg(long)]
    output: Option<PathBuf>,
}

impl DisplayCmd {
    async fn run(self, udid: &str, json_output: bool) -> Result<()> {
        match self.sub {
            DisplaySub::Status => {
                let (mut client, _device) = connect_display(udid, TunMode::Userspace).await?;
                let support = client.get_media_support_info().await?;
                let status = client.get_media_stream_server_status().await?;
                let value = serde_json::json!({"support": xpc_value_to_json(&support), "status": xpc_value_to_json(&status)});
                if json_output {
                    println!("{}", serde_json::to_string_pretty(&value)?);
                } else {
                    println!(
                        "Media support: {}\nMedia status: {}",
                        value["support"], value["status"]
                    );
                }
                Ok(())
            }
            DisplaySub::Video(options) => {
                capture_display(udid, MediaKind::Video, options, json_output).await
            }
            DisplaySub::Audio(options) => {
                capture_display(udid, MediaKind::Audio, options, json_output).await
            }
        }
    }
}

async fn connect_display(
    udid: &str,
    tun_mode: TunMode,
) -> Result<(DisplayServiceClient, ios_core::ConnectedDevice)> {
    let device = connect(
        udid,
        ConnectOptions {
            tun_mode,
            pair_record_path: None,
            skip_tunnel: false,
        },
    )
    .await
    .context("failed to establish a CoreDevice tunnel")?;
    let (xpc, metadata) = device
        .connect_xpc_service_with_metadata(ios_core::display::SERVICE_NAME)
        .await
        .context("display media service is unavailable in the device RSD directory")?;
    Ok((
        DisplayServiceClient::from_resolved_metadata(xpc, &metadata)?,
        device,
    ))
}

async fn capture_display(
    udid: &str,
    kind: MediaKind,
    capture: MediaCaptureCmd,
    json_output: bool,
) -> Result<()> {
    if capture.max_units == 0 {
        anyhow::bail!("--max-units must be greater than zero");
    }
    if capture.timeout == 0 {
        anyhow::bail!("--timeout must be greater than zero");
    }
    // The userspace tunnel currently exposes only a TCP proxy. A host kernel
    // UDP socket cannot receive packets injected into that private stack, so
    // require the kernel TUN path instead of reporting a false capture.
    let (client, device) = connect_display(udid, TunMode::Kernel).await?;
    let sender_ip = capture
        .sender_ip
        .or_else(|| device.server_address().map(str::to_owned))
        .ok_or_else(|| anyhow::anyhow!("tunnel did not provide a device server address"))?;
    let receiver_ip = capture
        .receiver_ip
        .or_else(|| device.client_address().map(str::to_owned))
        .ok_or_else(|| {
            anyhow::anyhow!(
                "tunnel did not provide a host client address; pass --receiver-ip explicitly"
            )
        })?;
    let options = MediaStreamOptions {
        receiver_ip,
        sender_ip,
        ..MediaStreamOptions::default()
    };
    let mut session = match kind {
        MediaKind::Video => MediaStreamSession::start_video(client, options).await?,
        MediaKind::Audio => MediaStreamSession::start_audio(client, options).await?,
    };
    let deadline = tokio::time::Instant::now()
        .checked_add(Duration::from_secs(capture.timeout))
        .ok_or_else(|| anyhow::anyhow!("--timeout is too large for a capture deadline"))?;
    let mut output = capture.output.map(AtomicOutput::new).transpose()?;
    let mut units = 0usize;
    let mut bytes = 0usize;
    let capture_result: Result<()> = async {
        while units < capture.max_units {
            let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
            if remaining.is_zero() {
                break;
            }
            let Some(unit) = session.recv_access_unit(remaining).await? else {
                continue;
            };
            if let Some(file) = output.as_mut() {
                file.write(&unit.data)?;
            }
            let next_bytes = bytes
                .checked_add(unit.data.len())
                .ok_or_else(|| anyhow::anyhow!("capture byte count overflow"))?;
            if next_bytes > MAX_CAPTURE_OUTPUT_BYTES {
                anyhow::bail!(
                    "capture output exceeds the {MAX_CAPTURE_OUTPUT_BYTES}-byte safety budget"
                );
            }
            bytes = next_bytes;
            units = units
                .checked_add(1)
                .ok_or_else(|| anyhow::anyhow!("capture access-unit count overflow"))?;
            if json_output {
                println!(
                    "{}",
                    serde_json::json!({"kind": if kind == MediaKind::Video {"video"} else {"audio"}, "timestamp": unit.timestamp, "sequenceStart": unit.sequence_start, "sequenceEnd": unit.sequence_end, "ssrc": unit.ssrc, "size": unit.data.len()})
                );
            }
        }
        if let Some(file) = output.take() {
            file.commit()?;
        }
        Ok(())
    }
    .await;
    let stop_result: Result<(), anyhow::Error> = session
        .stop(Duration::from_secs(5))
        .await
        .map_err(Into::into);
    if let Err(error) = capture_result {
        if let Err(stop_error) = stop_result {
            return Err(error.context(format!("also failed to stop media stream: {stop_error}")));
        }
        return Err(error);
    }
    stop_result?;
    if !json_output {
        println!(
            "Captured {units} {} access units ({bytes} bytes)",
            if kind == MediaKind::Video {
                "video"
            } else {
                "audio"
            }
        );
    }
    Ok(())
}

struct AtomicOutput {
    path: PathBuf,
    temp: PathBuf,
    file: Option<std::fs::File>,
}
impl AtomicOutput {
    fn new(path: PathBuf) -> Result<Self> {
        let mut temp = path.clone();
        temp.set_extension(format!("{}.part", std::process::id()));
        let file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temp)
            .context("failed to create private capture staging file")?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            if let Err(error) = file.set_permissions(std::fs::Permissions::from_mode(0o600)) {
                drop(file);
                let _ = std::fs::remove_file(&temp);
                return Err(error.into());
            }
        }
        Ok(Self {
            path,
            temp,
            file: Some(file),
        })
    }
    fn write(&mut self, bytes: &[u8]) -> Result<()> {
        use std::io::Write as _;
        self.file
            .as_mut()
            .ok_or_else(|| anyhow::anyhow!("capture output is already committed"))?
            .write_all(bytes)
            .map_err(Into::into)
    }
    fn commit(mut self) -> Result<()> {
        use std::io::Write as _;
        let mut file = self
            .file
            .take()
            .ok_or_else(|| anyhow::anyhow!("capture output is already committed"))?;
        file.flush()?;
        file.sync_all()?;
        drop(file);
        std::fs::rename(&self.temp, &self.path)
            .context("failed to atomically install capture output")
    }
}

fn xpc_value_to_json(value: &ios_core::XpcValue) -> serde_json::Value {
    match value {
        ios_core::XpcValue::Null => serde_json::Value::Null,
        ios_core::XpcValue::Bool(v) => (*v).into(),
        ios_core::XpcValue::Int64(v) => (*v).into(),
        ios_core::XpcValue::Uint64(v) => serde_json::json!(*v),
        ios_core::XpcValue::Double(v) => serde_json::Number::from_f64(*v)
            .map_or(serde_json::Value::Null, serde_json::Value::Number),
        ios_core::XpcValue::Date(v) => (*v).into(),
        ios_core::XpcValue::Data(v) => serde_json::json!({"dataBytes": v.len()}),
        ios_core::XpcValue::String(v) => v.clone().into(),
        ios_core::XpcValue::Uuid(v) => uuid::Uuid::from_bytes(*v).to_string().into(),
        ios_core::XpcValue::Array(v) => v.iter().map(xpc_value_to_json).collect::<Vec<_>>().into(),
        ios_core::XpcValue::Dictionary(v) => v
            .iter()
            .map(|(k, v)| (k.clone(), xpc_value_to_json(v)))
            .collect::<serde_json::Map<_, _>>()
            .into(),
        ios_core::XpcValue::FileTransfer { msg_id, data } => {
            serde_json::json!({"fileTransferMessageId": msg_id, "data": xpc_value_to_json(data)})
        }
    }
}
impl Drop for AtomicOutput {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.temp);
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
    fn parses_display_status_and_bounded_video_capture() {
        let parsed = TestCli::parse_from(["device-control", "display", "status"]);
        assert!(matches!(
            parsed.command,
            DeviceControlSub::Display(DisplayCmd {
                sub: DisplaySub::Status
            })
        ));
        let parsed = TestCli::parse_from([
            "device-control",
            "display",
            "video",
            "--max-units",
            "3",
            "--timeout",
            "5",
        ]);
        assert!(matches!(
            parsed.command,
            DeviceControlSub::Display(DisplayCmd {
                sub: DisplaySub::Video(MediaCaptureCmd {
                    max_units: 3,
                    timeout: 5,
                    ..
                })
            })
        ));
    }

    #[test]
    fn atomic_output_commits_and_cleans_abandoned_staging_file() {
        let nonce = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("clock should be after epoch")
            .as_nanos();
        let directory = std::env::temp_dir().join(format!(
            "rust-ios-device-display-output-{}-{nonce}",
            std::process::id()
        ));
        std::fs::create_dir(&directory).expect("temporary test directory should be new");

        let output_path = directory.join("capture.bin");
        let staging_path;
        {
            let mut output = AtomicOutput::new(output_path.clone()).expect("staging file");
            staging_path = output.temp.clone();
            output.write(b"encoded").expect("write staging output");
            assert!(staging_path.exists());
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                assert_eq!(
                    std::fs::metadata(&staging_path)
                        .expect("staging metadata")
                        .permissions()
                        .mode()
                        & 0o777,
                    0o600
                );
            }
            output.commit().expect("atomic commit");
        }
        assert_eq!(
            std::fs::read(&output_path).expect("committed output"),
            b"encoded"
        );
        assert!(!staging_path.exists());

        let abandoned_path = directory.join("abandoned.bin");
        let abandoned_staging;
        {
            let output = AtomicOutput::new(abandoned_path).expect("abandoned staging file");
            abandoned_staging = output.temp.clone();
            assert!(abandoned_staging.exists());
        }
        assert!(!abandoned_staging.exists());
        std::fs::remove_file(output_path).expect("remove committed test output");
        std::fs::remove_dir(directory).expect("remove temporary test directory");
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
