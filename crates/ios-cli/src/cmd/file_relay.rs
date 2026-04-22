use std::io::Write;
use std::path::PathBuf;

use anyhow::Result;
use ios_core::{connect, ConnectOptions};
use ios_tunnel::TunMode;

#[derive(clap::Args)]
pub struct FileRelayCmd {
    #[arg(required = true, help = "Requested relay sources")]
    sources: Vec<String>,
    #[arg(long, help = "Write the relay archive to a file instead of stdout")]
    output: Option<PathBuf>,
}

impl FileRelayCmd {
    pub async fn run(self, udid: Option<String>) -> Result<()> {
        let udid = udid.ok_or_else(|| anyhow::anyhow!("--udid required for file-relay"))?;
        let device = connect(
            &udid,
            ConnectOptions {
                tun_mode: TunMode::Userspace,
                pair_record_path: None,
                skip_tunnel: true,
            },
        )
        .await?;
        let stream = device
            .connect_service(ios_services::file_relay::SERVICE_NAME)
            .await?;
        let mut client = ios_services::file_relay::FileRelayClient::new(stream);
        let sources: Vec<&str> = self.sources.iter().map(|source| source.as_str()).collect();
        let archive = client.request_sources(&sources).await?;

        if let Some(output) = self.output {
            std::fs::write(&output, &archive)?;
            println!(
                "Wrote {} bytes of file relay data to {}",
                archive.len(),
                output.display()
            );
        } else {
            let mut stdout = std::io::stdout().lock();
            stdout.write_all(&archive)?;
            stdout.flush()?;
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
        command: FileRelayCmd,
    }

    #[test]
    fn parses_file_relay_flags() {
        let parsed = TestCli::try_parse_from([
            "file-relay",
            "Network",
            "CrashReporter",
            "--output",
            "relay.cpio.gz",
        ]);
        assert!(parsed.is_ok(), "file-relay flags should parse");
    }
}
