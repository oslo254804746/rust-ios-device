use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use ios_core::TunMode;
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
    /// Download all symbol files into a directory
    Download { output_dir: PathBuf },
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
            .connect_service(ios_core::fetchsymbols::SERVICE_NAME)
            .await?;
        let mut client = ios_core::fetchsymbols::FetchSymbolsClient::new(stream);
        match self.sub {
            SymbolsSub::List => render_list(client.list_files().await?, json),
            SymbolsSub::Pull {
                index,
                output,
                max_bytes,
            } => {
                let file = create_output(&output).await?;
                let bytes = client.download(index, file, max_bytes).await?;
                render_pull(index, &output, bytes, max_bytes.is_some(), json)
            }
            SymbolsSub::Download { output_dir } => {
                tokio::fs::create_dir_all(&output_dir)
                    .await
                    .with_context(|| format!("failed to create {}", output_dir.display()))?;
                let files = client.list_files().await?;
                let mut downloaded = std::collections::HashSet::<PathBuf>::new();
                let mut total = 0u64;
                for (index, remote_path) in files.iter().enumerate() {
                    let output =
                        ios_core::fetchsymbols::remote_symbol_output_path(&output_dir, remote_path);
                    if downloaded.insert(output.clone()) {
                        let _ = tokio::fs::remove_file(&output).await;
                    }
                    let file = open_append(&output).await?;
                    total += client.download(index as u32, file, None).await?;
                }
                render_download(&output_dir, files.len(), total, json)
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
            .connect_rsd_service(ios_core::fetchsymbols::REMOTE_SERVICE_NAME)
            .await?;
        let mut client = ios_core::fetchsymbols::RemoteFetchSymbolsClient::connect(stream)
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
                let file = create_output(&output).await?;
                let bytes = client.download(index, file, max_bytes).await?;
                render_pull(index, &output, bytes, max_bytes.is_some(), json)
            }
            SymbolsSub::Download { output_dir } => {
                tokio::fs::create_dir_all(&output_dir)
                    .await
                    .with_context(|| format!("failed to create {}", output_dir.display()))?;
                let files = client.list_files().await?;
                let mut total = 0u64;
                for (index, remote_file) in files.iter().enumerate() {
                    let output = ios_core::fetchsymbols::remote_symbol_output_path(
                        &output_dir,
                        &remote_file.path,
                    );
                    let file = create_output(&output).await?;
                    total += client
                        .download_known(index as u32, remote_file, file, None)
                        .await?;
                }
                render_download(&output_dir, files.len(), total, json)
            }
        }
    }
}

/// Open the download sink without blocking the runtime.
///
/// The download itself streams into a `std::io::Write`, which is the sink type
/// `fetchsymbols` takes.
async fn create_output(output: &Path) -> Result<std::fs::File> {
    if let Some(parent) = output.parent() {
        if !parent.as_os_str().is_empty() {
            tokio::fs::create_dir_all(parent)
                .await
                .with_context(|| format!("failed to create {}", parent.display()))?;
        }
    }
    let file = tokio::fs::File::create(output)
        .await
        .with_context(|| format!("failed to create {}", output.display()))?;
    Ok(file.into_std().await)
}

/// Same as [`create_output`], but appends so a symbol file split across several
/// remote entries accumulates instead of being truncated each time.
async fn open_append(output: &Path) -> Result<std::fs::File> {
    if let Some(parent) = output.parent() {
        if !parent.as_os_str().is_empty() {
            tokio::fs::create_dir_all(parent)
                .await
                .with_context(|| format!("failed to create {}", parent.display()))?;
        }
    }
    let file = tokio::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(output)
        .await
        .with_context(|| format!("failed to open {}", output.display()))?;
    Ok(file.into_std().await)
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

fn render_download(output_dir: &Path, files: usize, bytes: u64, json: bool) -> Result<()> {
    if json {
        println!(
            "{}",
            serde_json::to_string_pretty(&serde_json::json!({
                "output_dir": output_dir.display().to_string(),
                "files": files,
                "bytes": bytes,
            }))?
        );
    } else {
        println!(
            "Downloaded {files} symbol files ({bytes} bytes) to {}",
            output_dir.display()
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

    #[test]
    fn parses_symbols_download_subcommand() {
        let parsed = TestCli::try_parse_from(["symbols", "download", "ios-rs-tmp/Symbols"]);
        assert!(parsed.is_ok(), "symbols download should parse");
    }
}
