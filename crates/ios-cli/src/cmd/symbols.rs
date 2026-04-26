use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use ios_core::tunnel::TunMode;
use ios_core::{connect, ConnectOptions};

#[derive(clap::Args)]
pub struct SymbolsCmd {
    #[command(subcommand)]
    sub: SymbolsSub,
}

#[derive(clap::Subcommand)]
enum SymbolsSub {
    /// List symbol files exposed by the device
    List,
    /// Download a symbol file by index
    Pull {
        index: u32,
        output: PathBuf,
        #[arg(long, help = "Maximum number of bytes to copy for probing")]
        max_bytes: Option<u64>,
    },
}

impl SymbolsCmd {
    pub async fn run(self, udid: Option<String>, json: bool) -> Result<()> {
        let udid = udid.ok_or_else(|| anyhow::anyhow!("--udid required for symbols"))?;
        let probe = connect(
            &udid,
            ConnectOptions {
                tun_mode: TunMode::Userspace,
                pair_record_path: None,
                skip_tunnel: true,
            },
        )
        .await?;
        let version = probe.product_version().await?;
        drop(probe);

        if version.major >= 17 {
            self.run_remote(&udid, json).await
        } else {
            self.run_legacy(&udid, json).await
        }
    }

    async fn run_legacy(self, udid: &str, json: bool) -> Result<()> {
        let device = connect(
            udid,
            ConnectOptions {
                tun_mode: TunMode::Userspace,
                pair_record_path: None,
                skip_tunnel: true,
            },
        )
        .await?;
        let stream = device
            .connect_service(ios_core::services::fetchsymbols::SERVICE_NAME)
            .await?;
        let mut client = ios_core::services::fetchsymbols::FetchSymbolsClient::new(stream);
        match self.sub {
            SymbolsSub::List => render_list(client.list_files().await?, json),
            SymbolsSub::Pull {
                index,
                output,
                max_bytes,
            } => {
                let file = create_output(&output)?;
                let bytes = client.download(index, file, max_bytes).await?;
                render_pull(index, &output, bytes, max_bytes.is_some(), json)
            }
        }
    }

    async fn run_remote(self, udid: &str, json: bool) -> Result<()> {
        let device = connect(
            udid,
            ConnectOptions {
                tun_mode: TunMode::Userspace,
                pair_record_path: None,
                skip_tunnel: false,
            },
        )
        .await?;
        tokio::time::sleep(std::time::Duration::from_secs(2)).await;
        let stream = device
            .connect_rsd_service(ios_core::services::fetchsymbols::REMOTE_SERVICE_NAME)
            .await?;
        let mut client =
            ios_core::services::fetchsymbols::RemoteFetchSymbolsClient::connect(stream)
                .await
                .map_err(|err| anyhow::anyhow!(err.to_string()))?;
        match self.sub {
            SymbolsSub::List => {
                let files = client
                    .list_files()
                    .await?
                    .into_iter()
                    .map(|file| file.path)
                    .collect();
                render_list(files, json)
            }
            SymbolsSub::Pull {
                index,
                output,
                max_bytes,
            } => {
                let file = create_output(&output)?;
                let bytes = client.download(index, file, max_bytes).await?;
                render_pull(index, &output, bytes, max_bytes.is_some(), json)
            }
        }
    }
}

fn create_output(output: &Path) -> Result<std::fs::File> {
    if let Some(parent) = output.parent() {
        if !parent.as_os_str().is_empty() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("failed to create {}", parent.display()))?;
        }
    }
    std::fs::File::create(output).with_context(|| format!("failed to create {}", output.display()))
}

fn render_list(files: Vec<String>, json: bool) -> Result<()> {
    if json {
        println!("{}", serde_json::to_string_pretty(&files)?);
    } else {
        for (index, path) in files.iter().enumerate() {
            println!("[{index}] {path}");
        }
    }
    Ok(())
}

fn render_pull(index: u32, output: &Path, bytes: u64, truncated: bool, json: bool) -> Result<()> {
    if json {
        println!(
            "{}",
            serde_json::to_string_pretty(&serde_json::json!({
                "index": index,
                "output": output.display().to_string(),
                "bytes": bytes,
                "truncated": truncated,
            }))?
        );
    } else {
        println!(
            "Downloaded {bytes} bytes from symbol index {index} to {}",
            output.display()
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use clap::Parser;

    use super::*;

    #[derive(Parser)]
    struct TestCli {
        #[command(subcommand)]
        command: SymbolsSub,
    }

    #[test]
    fn parses_symbols_list_subcommand() {
        let parsed = TestCli::try_parse_from(["symbols", "list"]);
        assert!(parsed.is_ok(), "symbols list should parse");
    }

    #[test]
    fn parses_symbols_pull_subcommand() {
        let parsed = TestCli::try_parse_from([
            "symbols",
            "pull",
            "1",
            "ios-rs-tmp/dyld_shared_cache",
            "--max-bytes",
            "1024",
        ]);
        assert!(parsed.is_ok(), "symbols pull should parse");
    }
}
