use std::io::Write;
use std::path::PathBuf;

use anyhow::Result;
use ios_core::tunnel::TunMode;
use ios_core::{connect, ConnectOptions};

#[derive(clap::Args)]
pub struct PreboardCmd {
    #[command(subcommand)]
    sub: PreboardSub,
}

#[derive(clap::Subcommand)]
enum PreboardSub {
    /// Create a stashbag manifest
    Create {
        #[arg(long, help = "Optional plist manifest file")]
        manifest: Option<PathBuf>,
    },
    /// Commit a stashbag manifest
    Commit {
        #[arg(long, help = "Optional plist manifest file")]
        manifest: Option<PathBuf>,
    },
}

impl PreboardCmd {
    pub async fn run(self, udid: Option<String>, json: bool) -> Result<()> {
        let udid = udid.ok_or_else(|| anyhow::anyhow!("--udid required for preboard"))?;
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
            .connect_service(ios_core::services::preboard::SERVICE_NAME)
            .await?;
        let mut client = ios_core::services::preboard::PreboardClient::new(stream);

        let response = match self.sub {
            PreboardSub::Create { manifest } => {
                client.create_stashbag(load_manifest(manifest)?).await?
            }
            PreboardSub::Commit { manifest } => {
                client.commit_stashbag(load_manifest(manifest)?).await?
            }
        };

        let value = plist::Value::Dictionary(response);
        if json {
            println!("{}", serde_json::to_string_pretty(&value)?);
        } else {
            let mut stdout = std::io::stdout().lock();
            plist::to_writer_xml(&mut stdout, &value)?;
            writeln!(&mut stdout)?;
        }
        Ok(())
    }
}

fn load_manifest(path: Option<PathBuf>) -> Result<plist::Dictionary> {
    match path {
        Some(path) => {
            let value = plist::Value::from_file(path)?;
            value
                .into_dictionary()
                .ok_or_else(|| anyhow::anyhow!("manifest file must contain a plist dictionary"))
        }
        None => Ok(plist::Dictionary::new()),
    }
}

#[cfg(test)]
mod tests {
    use clap::Parser;

    use super::*;

    #[derive(Parser)]
    struct TestCli {
        #[command(subcommand)]
        command: PreboardSub,
    }

    #[test]
    fn parses_preboard_create_subcommand() {
        let parsed = TestCli::try_parse_from(["preboard", "create", "--manifest", "stash.plist"]);
        assert!(parsed.is_ok(), "preboard create command should parse");
    }

    #[test]
    fn parses_preboard_commit_subcommand() {
        let parsed = TestCli::try_parse_from(["preboard", "commit"]);
        assert!(parsed.is_ok(), "preboard commit command should parse");
    }
}
