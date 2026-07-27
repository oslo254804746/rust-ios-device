use std::io::Write;
use std::path::PathBuf;
use std::time::Duration;

use anyhow::Result;
use ios_core::bt_packet_logger::{
    parse_packetlogger_record, write_packetlogger_record, write_pcapng_header, write_pcapng_record,
    BtPacketLoggerClient, SERVICE_NAME,
};
use ios_core::{connect, ConnectOptions, TunMode};
use tokio::time::{timeout, Instant};

#[derive(clap::Args)]
pub struct BtLoggerCmd {
    #[command(subcommand)]
    sub: BtLoggerSubcommand,
}

#[derive(Debug, clap::Subcommand)]
enum BtLoggerSubcommand {
    /// Capture Bluetooth HCI traffic via BTPacketLogger
    Capture {
        /// Output path, or '-' for stdout
        output: PathBuf,
        #[arg(
            short = 'f',
            long,
            value_enum,
            default_value_t = BtLoggerFormat::Packetlogger,
            help = "Output format"
        )]
        format: BtLoggerFormat,
        #[arg(long, help = "Stop after writing this many records")]
        count: Option<usize>,
        #[arg(long, help = "Maximum capture duration in seconds")]
        duration: Option<u64>,
        #[arg(long, help = "Print a JSON completion summary")]
        json: bool,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, clap::ValueEnum)]
enum BtLoggerFormat {
    Packetlogger,
    Pcapng,
}

impl BtLoggerCmd {
    pub async fn run(self, udid: Option<String>, global_json: bool) -> Result<()> {
        let udid = udid.ok_or_else(|| anyhow::anyhow!("--udid required for btlogger"))?;
        match self.sub {
            BtLoggerSubcommand::Capture {
                output,
                format,
                count,
                duration,
                json,
            } => run_capture(&udid, output, format, count, duration, json || global_json).await,
        }
    }
}

async fn run_capture(
    udid: &str,
    output: PathBuf,
    format: BtLoggerFormat,
    count: Option<usize>,
    duration: Option<u64>,
    json: bool,
) -> Result<()> {
    if output.as_os_str() == std::ffi::OsStr::new("-") && json {
        anyhow::bail!("btlogger capture cannot combine stdout output '-' with JSON summary");
    }

    let device = connect(
        udid,
        ConnectOptions {
            tun_mode: TunMode::Userspace,
            pair_record_path: None,
            skip_tunnel: true,
        },
    )
    .await?;
    let tz_offset_seconds = if format == BtLoggerFormat::Pcapng {
        lockdown_number(&device, "TimeZoneOffsetFromUTC")
            .await
            .unwrap_or(0.0)
    } else {
        0.0
    };
    let stream = device.connect_service(SERVICE_NAME).await?;
    let mut client = BtPacketLoggerClient::new(stream);
    let mut writer = CaptureOutput::create(&output).await?;

    if format == BtLoggerFormat::Pcapng {
        write_pcapng_header(&mut writer)?;
    }

    let deadline = duration.map(|seconds| Instant::now() + Duration::from_secs(seconds));
    let mut written = 0usize;
    let mut skipped = 0usize;

    loop {
        if count.is_some_and(|limit| written >= limit) {
            break;
        }
        let raw = match next_record_with_deadline(&mut client, deadline).await? {
            Some(raw) => raw,
            None => break,
        };

        match format {
            BtLoggerFormat::Packetlogger => {
                write_packetlogger_record(&mut writer, &raw)?;
                written += 1;
            }
            BtLoggerFormat::Pcapng => match parse_packetlogger_record(&raw) {
                Ok(record) => {
                    if write_pcapng_record(&mut writer, &record, tz_offset_seconds)? {
                        written += 1;
                    } else {
                        skipped += 1;
                    }
                }
                Err(err) => {
                    tracing::debug!("skipping malformed BTPacketLogger record: {err}");
                    skipped += 1;
                }
            },
        }
    }

    if json {
        println!(
            "{}",
            serde_json::to_string_pretty(&serde_json::json!({
                "output": output.display().to_string(),
                "format": format!("{format:?}").to_ascii_lowercase(),
                "written_records": written,
                "skipped_records": skipped,
                "service_name": SERVICE_NAME,
            }))?
        );
    } else {
        println!(
            "Saved {written} Bluetooth record(s) to {}",
            output.display()
        );
        if skipped > 0 {
            println!("SkippedRecords: {skipped}");
        }
    }

    Ok(())
}

async fn next_record_with_deadline<S>(
    client: &mut BtPacketLoggerClient<S>,
    deadline: Option<Instant>,
) -> Result<Option<Vec<u8>>>
where
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin,
{
    if let Some(deadline) = deadline {
        let now = Instant::now();
        if now >= deadline {
            return Ok(None);
        }
        let remaining = deadline.saturating_duration_since(now);
        match timeout(remaining, client.next_packetlogger_record()).await {
            Ok(result) => Ok(Some(result?)),
            Err(_) => Ok(None),
        }
    } else {
        Ok(Some(client.next_packetlogger_record().await?))
    }
}

async fn lockdown_number(device: &ios_core::ConnectedDevice, key: &str) -> Option<f64> {
    let value = device.lockdown_get_value(Some(key)).await.ok()?;
    plist_number_to_f64(&value)
}

fn plist_number_to_f64(value: &plist::Value) -> Option<f64> {
    match value {
        plist::Value::Integer(value) => value
            .as_signed()
            .map(|value| value as f64)
            .or_else(|| value.as_unsigned().map(|value| value as f64)),
        plist::Value::Real(value) => Some(*value),
        _ => None,
    }
}

enum CaptureOutput {
    Stdout(std::io::Stdout),
    File(std::fs::File),
}

impl CaptureOutput {
    /// Open the capture sink without blocking the runtime.
    ///
    /// The packet writes themselves go through `std::io::Write`, which is what
    /// the pcap writer helpers take.
    async fn create(path: &PathBuf) -> Result<Self> {
        if path.as_os_str() == std::ffi::OsStr::new("-") {
            Ok(Self::Stdout(std::io::stdout()))
        } else {
            let file = tokio::fs::File::create(path).await?;
            Ok(Self::File(file.into_std().await))
        }
    }
}

impl Write for CaptureOutput {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        match self {
            Self::Stdout(stdout) => stdout.write(buf),
            Self::File(file) => file.write(buf),
        }
    }

    fn flush(&mut self) -> std::io::Result<()> {
        match self {
            Self::Stdout(stdout) => stdout.flush(),
            Self::File(file) => file.flush(),
        }
    }
}

#[cfg(test)]
mod tests {
    use clap::Parser;

    use super::*;

    #[derive(clap::Parser)]
    struct TestCli {
        #[command(flatten)]
        command: BtLoggerCmd,
    }

    #[test]
    fn parses_btlogger_capture_defaults() {
        let cmd = TestCli::parse_from(["btlogger", "capture", "trace.pklg"]);
        let BtLoggerSubcommand::Capture {
            output,
            format,
            count,
            duration,
            json,
        } = cmd.command.sub;
        assert_eq!(output, PathBuf::from("trace.pklg"));
        assert_eq!(format, BtLoggerFormat::Packetlogger);
        assert_eq!(count, None);
        assert_eq!(duration, None);
        assert!(!json);
    }

    #[test]
    fn parses_btlogger_capture_pcapng_options() {
        let cmd = TestCli::parse_from([
            "btlogger",
            "capture",
            "trace.pcapng",
            "--format",
            "pcapng",
            "--count",
            "2",
            "--duration",
            "5",
            "--json",
        ]);
        let BtLoggerSubcommand::Capture {
            format,
            count,
            duration,
            json,
            ..
        } = cmd.command.sub;
        assert_eq!(format, BtLoggerFormat::Pcapng);
        assert_eq!(count, Some(2));
        assert_eq!(duration, Some(5));
        assert!(json);
    }

    #[test]
    fn plist_number_to_f64_accepts_lockdown_number_shapes() {
        assert_eq!(
            plist_number_to_f64(&plist::Value::Integer(42.into())),
            Some(42.0)
        );
        assert_eq!(plist_number_to_f64(&plist::Value::Real(42.5)), Some(42.5));
        assert_eq!(
            plist_number_to_f64(&plist::Value::String("42".into())),
            None
        );
    }
}
