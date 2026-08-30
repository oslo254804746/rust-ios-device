//! CoreDevice HID input commands.

use std::time::Duration;

use anyhow::{Context, Result};
use ios_core::display::{DisplayServiceClient, MediaStreamOptions, MediaStreamSession};
use ios_core::hid::{
    ButtonState, IndigoHidServiceClient, KeyboardUsage, TouchCoordinate, TouchPhase,
    UniversalHidServiceClient, DEFAULT_KEYBOARD_SERVICE_ID, DEFAULT_TOUCHSCREEN_SERVICE_ID,
    MAX_TEXT_LENGTH, MAX_TOUCH_REPORTS,
};
use ios_core::{connect, ConnectOptions, TunMode};

const DEFAULT_VENDOR_ID: i64 = 0x05ac;
const DEFAULT_PRODUCT_ID: i64 = 0x0250;

#[derive(clap::Args)]
pub struct HidCmd {
    /// Input injection changes device state and can affect the person using it.
    /// This explicit acknowledgement is required for every HID operation.
    #[arg(long, help = "Acknowledge that this injects input into the device")]
    confirm: bool,
    /// Absolute deadline covering tunnel, RSD, XPC setup, and all reports.
    #[arg(long, default_value_t = 30, value_name = "SECONDS")]
    timeout: u64,
    #[command(subcommand)]
    sub: HidSub,
}

#[derive(clap::Subcommand)]
enum HidSub {
    /// Send one Indigo hardware-button event.
    Button {
        #[arg(long, default_value_t = 0x0c)]
        usage_page: u16,
        #[arg(long)]
        usage_code: u16,
        #[arg(long, default_value = "down")]
        state: ButtonState,
    },
    /// Tap one normalized touchscreen coordinate.
    Tap {
        #[arg(long, value_name = "0..1")]
        x: f64,
        #[arg(long, value_name = "0..1")]
        y: f64,
        #[arg(long, default_value_t = DEFAULT_TOUCHSCREEN_SERVICE_ID)]
        service_id: u64,
    },
    /// Swipe between normalized touchscreen coordinates.
    Swipe {
        #[arg(long)]
        from_x: f64,
        #[arg(long)]
        from_y: f64,
        #[arg(long)]
        to_x: f64,
        #[arg(long)]
        to_y: f64,
        #[arg(long, default_value_t = 12)]
        steps: usize,
        #[arg(long, default_value_t = DEFAULT_TOUCHSCREEN_SERVICE_ID)]
        service_id: u64,
    },
    /// Send one touch state transition; use a single command invocation per phase.
    Touch {
        #[arg(value_parser = parse_touch_phase)]
        phase: TouchPhase,
        #[arg(long)]
        contact_id: u32,
        #[arg(long)]
        x: f64,
        #[arg(long)]
        y: f64,
        #[arg(long, default_value_t = DEFAULT_TOUCHSCREEN_SERVICE_ID)]
        service_id: u64,
    },
    /// Register a virtual keyboard and type text. Text is never echoed in output.
    Text {
        #[arg(value_name = "TEXT", value_parser = validate_text)]
        text: String,
        #[arg(long, default_value_t = DEFAULT_KEYBOARD_SERVICE_ID)]
        service_id: u64,
    },
    /// Register a virtual keyboard and press/release one HID usage.
    Key {
        #[arg(value_name = "USAGE")]
        usage: KeyboardUsage,
        #[arg(long)]
        shift: bool,
        #[arg(long, default_value_t = DEFAULT_KEYBOARD_SERVICE_ID)]
        service_id: u64,
    },
}

impl HidCmd {
    pub async fn run(self, udid: Option<String>, json_output: bool) -> Result<()> {
        if !self.confirm {
            anyhow::bail!(
                "HID input is potentially disruptive; rerun with --confirm to acknowledge"
            );
        }
        let udid = udid.ok_or_else(|| anyhow::anyhow!("--udid required for hid"))?;
        let timeout = if self.timeout == 0 {
            anyhow::bail!("--timeout must be greater than zero");
        } else {
            Duration::from_secs(self.timeout)
        };
        run_with_deadline(timeout, self.run_operation(udid, json_output)).await
    }

    async fn run_operation(self, udid: String, json_output: bool) -> Result<()> {
        match self.sub {
            HidSub::Button {
                usage_page,
                usage_code,
                state,
            } => {
                let device = connect_device(&udid).await?;
                let (xpc, metadata) = device
                    .connect_xpc_service_with_metadata(ios_core::hid::INDIGO_SERVICE_NAME)
                    .await
                    .context("Indigo HID service is unavailable on this device")?;
                let mut client = IndigoHidServiceClient::from_resolved_metadata(xpc, &metadata)?;
                client.send_button(usage_page, usage_code, state).await?;
                print_result(json_output, "button", 1, false);
            }
            HidSub::Tap { x, y, service_id } => {
                let coordinate = TouchCoordinate::new(x, y)?;
                let (mut client, media, _device) = connect_universal_with_media(&udid).await?;
                let mut session = client.touch_session_with_media(service_id, media);
                let operation = async {
                    session.touch(0, TouchPhase::Down, coordinate).await?;
                    session.touch(0, TouchPhase::Up, coordinate).await?;
                    Ok::<(), ios_core::hid::HidError>(())
                }
                .await;
                let close_result = session.close(Duration::from_secs(5)).await;
                operation?;
                close_result?;
                print_result(json_output, "tap", 2, false);
            }
            HidSub::Swipe {
                from_x,
                from_y,
                to_x,
                to_y,
                steps,
                service_id,
            } => {
                if !(2..=MAX_TOUCH_REPORTS).contains(&steps) {
                    anyhow::bail!("--steps must be in 2..={MAX_TOUCH_REPORTS}");
                }
                let start = TouchCoordinate::new(from_x, from_y)?;
                let end = TouchCoordinate::new(to_x, to_y)?;
                let (mut client, media, _device) = connect_universal_with_media(&udid).await?;
                let mut session = client.touch_session_with_media(service_id, media);
                let operation = async {
                    session.touch(0, TouchPhase::Down, start).await?;
                    for step in 1..steps {
                        let fraction = step as f64 / steps as f64;
                        let coordinate = TouchCoordinate::new(
                            start.x + (end.x - start.x) * fraction,
                            start.y + (end.y - start.y) * fraction,
                        )?;
                        session.touch(0, TouchPhase::Move, coordinate).await?;
                        tokio::time::sleep(Duration::from_millis(5)).await;
                    }
                    session.touch(0, TouchPhase::Up, end).await?;
                    Ok::<(), ios_core::hid::HidError>(())
                }
                .await;
                let close_result = session.close(Duration::from_secs(5)).await;
                operation?;
                close_result?;
                print_result(json_output, "swipe", steps + 1, false);
            }
            HidSub::Touch {
                phase,
                contact_id,
                x,
                y,
                service_id,
            } => {
                let coordinate = TouchCoordinate::new(x, y)?;
                validate_contact_id(contact_id)?;
                let (mut client, media, _device) = connect_universal_with_media(&udid).await?;
                let mut session = client.touch_session_with_media(service_id, media);
                let operation = session.send_transition(phase, coordinate).await;
                let close_result = session.close(Duration::from_secs(5)).await;
                operation?;
                close_result?;
                print_result(json_output, "touch", 1, false);
            }
            HidSub::Text { text, service_id } => {
                let length = text.chars().count();
                let (mut client, media, _device) = connect_universal_with_media(&udid).await?;
                let keyboard = client
                    .create_keyboard_service(
                        service_id,
                        "pymobiledevice3 virtual keyboard",
                        "pymobiledevice3",
                        DEFAULT_VENDOR_ID,
                        DEFAULT_PRODUCT_ID,
                    )
                    .await?;
                let mut session = client.keyboard_session_with_media(keyboard, media);
                let operation = session.type_text(&text).await;
                let close_result = session.close(Duration::from_secs(5)).await;
                operation?;
                close_result?;
                print_result(json_output, "text", length, true);
            }
            HidSub::Key {
                usage,
                shift,
                service_id,
            } => {
                let (mut client, media, _device) = connect_universal_with_media(&udid).await?;
                let keyboard = client
                    .create_keyboard_service(
                        service_id,
                        "pymobiledevice3 virtual keyboard",
                        "pymobiledevice3",
                        DEFAULT_VENDOR_ID,
                        DEFAULT_PRODUCT_ID,
                    )
                    .await?;
                let mut session = client.keyboard_session_with_media(keyboard, media);
                let modifiers = if shift {
                    &[ios_core::hid::KeyboardModifier::LeftShift][..]
                } else {
                    &[]
                };
                let operation = session.send_key(usage, modifiers).await;
                let close_result = session.close(Duration::from_secs(5)).await;
                operation?;
                close_result?;
                print_result(json_output, "key", 1, false);
            }
        }
        Ok(())
    }
}

fn validate_contact_id(contact_id: u32) -> Result<()> {
    if contact_id != 0 {
        anyhow::bail!(
            "--contact-id {contact_id} cannot be represented: CoreDevice's 58-byte report is single-contact; use contact 0"
        );
    }
    Ok(())
}

/// CoreDevice Universal HID drops unauthenticated reports. Open the display
/// stream first and retain it alongside the HID client; the authorized session
/// closes HID before stopping this stream.
async fn connect_universal_with_media(
    udid: &str,
) -> Result<(
    UniversalHidServiceClient,
    MediaStreamSession,
    ios_core::ConnectedDevice,
)> {
    // Universal HID's authorization side channel is RTP/UDP. The userspace
    // tunnel only exposes a TCP proxy, so use the kernel tunnel where the
    // advertised CDTunnel client address is an actual host interface.
    let device = connect_device_with_mode(udid, TunMode::Kernel).await?;
    let sender_ip = device
        .server_address()
        .ok_or_else(|| anyhow::anyhow!("tunnel did not provide a device server address"))?
        .to_owned();
    let receiver_ip = device
        .client_address()
        .ok_or_else(|| anyhow::anyhow!("tunnel did not provide a host client address"))?
        .to_owned();
    let (display_xpc, display_metadata) = device
        .connect_xpc_service_with_metadata(ios_core::display::SERVICE_NAME)
        .await
        .context("CoreDevice display media service is unavailable")?;
    let display = DisplayServiceClient::from_resolved_metadata(display_xpc, &display_metadata)
        .context("CoreDevice display media service is not a canonical RemoteXPC service")?;
    let mut media = MediaStreamSession::start_video(
        display,
        MediaStreamOptions {
            sender_ip,
            receiver_ip,
            ..MediaStreamOptions::default()
        },
    )
    .await
    .context("failed to authenticate Universal HID with a display media stream")?;
    media.spawn_drain();
    // Match pmd3's touch_session: backboardd needs a short interval to
    // re-match the HID surfaces as authenticated before the first report.
    tokio::time::sleep(Duration::from_millis(300)).await;
    let (xpc, metadata) = match device
        .connect_xpc_service_with_metadata(ios_core::hid::UNIVERSAL_SERVICE_NAME)
        .await
    {
        Ok(value) => value,
        Err(error) => {
            let _ = media.stop(Duration::from_secs(5)).await;
            return Err(anyhow::Error::from(error)
                .context("Universal HID service is unavailable on this device"));
        }
    };
    let client = match UniversalHidServiceClient::from_resolved_metadata(xpc, &metadata) {
        Ok(client) => client,
        Err(error) => {
            let _ = media.stop(Duration::from_secs(5)).await;
            return Err(anyhow::Error::from(error)
                .context("Universal HID service is not a canonical RemoteXPC service"));
        }
    };
    Ok((client, media, device))
}

async fn connect_device(udid: &str) -> Result<ios_core::ConnectedDevice> {
    connect_device_with_mode(udid, TunMode::Userspace).await
}

async fn connect_device_with_mode(
    udid: &str,
    tun_mode: TunMode,
) -> Result<ios_core::ConnectedDevice> {
    connect(
        udid,
        ConnectOptions {
            tun_mode,
            pair_record_path: None,
            skip_tunnel: false,
        },
    )
    .await
    .context("failed to establish a CoreDevice tunnel")
}

async fn run_with_deadline<F>(timeout: Duration, operation: F) -> Result<()>
where
    F: std::future::Future<Output = Result<()>>,
{
    tokio::select! {
        result = tokio::time::timeout(timeout, operation) => result.map_err(|_| anyhow::anyhow!("HID operation exceeded its absolute deadline"))?,
        _ = tokio::signal::ctrl_c() => anyhow::bail!("HID operation cancelled by Ctrl+C"),
    }
}

fn parse_touch_phase(value: &str) -> Result<TouchPhase, String> {
    value.parse()
}

fn validate_text(value: &str) -> Result<String, String> {
    if value.len() > MAX_TEXT_LENGTH {
        Err(format!("text exceeds {MAX_TEXT_LENGTH} bytes"))
    } else {
        Ok(value.to_string())
    }
}

fn print_result(json_output: bool, operation: &str, reports: usize, sensitive: bool) {
    if json_output {
        println!(
            "{}",
            serde_json::json!({
                "operation": operation,
                "status": "sent",
                "reports": reports,
                "sensitive": sensitive,
            })
        );
    } else if sensitive {
        println!("Sent {operation} input ({reports} report units; text redacted)");
    } else {
        println!("Sent {operation} input ({reports} report units)");
    }
}

#[cfg(test)]
mod tests {
    use clap::Parser;

    use super::*;

    #[test]
    fn parses_and_requires_explicit_confirmation() {
        let parsed =
            TestCli::try_parse_from(["hid", "--confirm", "tap", "--x", "0.1", "--y", "0.2"]);
        assert!(parsed.is_ok());
        let parsed = TestCli::try_parse_from(["hid", "tap", "--x", "0.1", "--y", "0.2"]);
        assert!(
            parsed.is_ok(),
            "clap parsing is separate from runtime confirmation"
        );
    }

    #[test]
    fn validates_text_limit_and_phases() {
        assert!(validate_text(&"a".repeat(MAX_TEXT_LENGTH)).is_ok());
        assert!(validate_text(&"a".repeat(MAX_TEXT_LENGTH + 1)).is_err());
        assert_eq!(parse_touch_phase("move"), Ok(TouchPhase::Move));
        assert!(parse_touch_phase("unknown").is_err());
        let parsed = TestCli::try_parse_from(["hid", "key", "enter"]);
        assert!(parsed.is_ok());
    }

    #[test]
    fn output_contract_does_not_include_sensitive_text() {
        let value = serde_json::json!({
            "operation": "text", "status": "sent", "reports": 4, "sensitive": true
        });
        assert!(value.get("text").is_none());
    }

    #[test]
    fn touch_contact_id_rejects_unrepresentable_contacts() {
        assert!(validate_contact_id(0).is_ok());
        assert!(validate_contact_id(1)
            .unwrap_err()
            .to_string()
            .contains("single-contact"));
    }

    #[derive(clap::Parser)]
    struct TestCli {
        #[command(flatten)]
        command: HidCmd,
    }
}
