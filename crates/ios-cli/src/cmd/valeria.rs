use std::path::PathBuf;

use anyhow::Context;

#[derive(Debug, clap::Args)]
pub struct ValeriaCmd {
    #[command(subcommand)]
    sub: ValeriaSubcommand,
}

#[derive(Debug, clap::Subcommand)]
enum ValeriaSubcommand {
    Record(RecordCmd),
}

#[derive(Debug, clap::Args)]
pub struct RecordCmd {
    #[arg(short, long, default_value = "capture.h264")]
    output: PathBuf,
    #[arg(
        long,
        default_value_t = 0,
        help = "Stop after N seconds; 0 records until interrupted"
    )]
    duration: u64,
    #[arg(long, help = "Print a JSON completion summary for this command")]
    json: bool,
}

impl ValeriaCmd {
    pub async fn run(self, udid: Option<String>) -> anyhow::Result<()> {
        match self.sub {
            ValeriaSubcommand::Record(cmd) => cmd.run(udid).await,
        }
    }
}

impl RecordCmd {
    async fn run(self, udid: Option<String>) -> anyhow::Result<()> {
        let output = self.output.clone();
        let duration = self.duration;
        let json = self.json;

        let summary = tokio::task::spawn_blocking(move || {
            let options = ios_core::valeria::CaptureOptions {
                udid,
                queue_capacity: 90,
            };
            ios_core::valeria::UsbValeriaCapture::record_annex_b(options, &output, duration)
                .map_err(anyhow::Error::from)
        })
        .await
        .context("Valeria recording worker panicked")??;

        if json {
            println!("{}", serde_json::to_string_pretty(&summary)?);
        } else {
            eprintln!(
                "Wrote {} frames ({} bytes) to {}",
                summary.frames,
                summary.bytes,
                self.output.display()
            );
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use clap::Parser;

    use super::*;

    #[derive(Parser)]
    struct TestCli {
        #[command(flatten)]
        command: ValeriaCmd,
    }

    #[test]
    fn parses_record_defaults() {
        let cmd = TestCli::parse_from(["valeria", "record"]);
        let ValeriaSubcommand::Record(record) = cmd.command.sub;
        assert_eq!(record.output, PathBuf::from("capture.h264"));
        assert_eq!(record.duration, 0);
        assert!(!record.json);
    }

    #[test]
    fn parses_record_options() {
        let cmd = TestCli::parse_from([
            "valeria",
            "record",
            "--output",
            "out.h264",
            "--duration",
            "10",
            "--json",
        ]);
        let ValeriaSubcommand::Record(record) = cmd.command.sub;
        assert_eq!(record.output, PathBuf::from("out.h264"));
        assert_eq!(record.duration, 10);
        assert!(record.json);
    }

    #[test]
    fn summary_serializes_for_json_output() {
        let summary = ios_core::valeria::CaptureSummary {
            frames: 2,
            bytes: 128,
            width: 1179,
            height: 2556,
            dropped_frames: 0,
        };
        let value = serde_json::to_value(summary).unwrap();
        assert_eq!(value["frames"], 2);
        assert_eq!(value["bytes"], 128);
        assert_eq!(value["width"], 1179);
        assert_eq!(value["height"], 2556);
    }
}
