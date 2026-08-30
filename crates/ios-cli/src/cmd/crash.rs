use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::Result;
use comfy_table::{presets::UTF8_FULL, Table};
use futures_util::StreamExt;
use ios_core::crashreport::{
    flush_reports_at, flush_reports_rsd_at, matches_pattern, parse_report_bytes, prepare_reports,
    prepare_reports_rsd, sort_parsed_reports, CrashReportClient, CrashWatchOptions,
    ParsedCrashReport, CRASHREPORT_COPY_MOBILE_SERVICE, CRASHREPORT_MOVER_SERVICE,
    RSD_CRASHREPORT_COPY_MOBILE_SERVICE, RSD_CRASHREPORT_MOVER_SERVICE,
};
use ios_core::TunMode;
use ios_core::{connect, ConnectOptions};
use tokio::fs;
use tokio::io::AsyncReadExt;

#[derive(clap::Args)]
pub struct CrashCmd {
    #[command(subcommand)]
    sub: CrashSub,
}

#[derive(clap::Subcommand)]
enum CrashSub {
    /// List crash reports on the device
    Ls {
        #[arg(help = "Optional filename glob pattern", default_value = "*")]
        pattern: String,
        #[arg(short = 'j', long, help = "Output JSON")]
        json: bool,
    },
    /// Download a crash report from the device
    Pull {
        #[arg(help = "Crash report path or basename")]
        report: String,
        #[arg(help = "Optional local destination path")]
        local: Option<String>,
        #[arg(long, help = "Overwrite the local destination if it already exists")]
        force: bool,
    },
    /// Print a crash report to stdout
    Show {
        #[arg(help = "Crash report path or basename")]
        report: String,
        #[arg(long, help = "Only print the first line/header of the report")]
        head: bool,
    },
    /// Download all matching crash reports from the device
    PullAll {
        #[arg(help = "Optional filename glob pattern", default_value = "*")]
        pattern: String,
        #[arg(help = "Optional local destination directory", default_value = ".")]
        local_dir: String,
        #[arg(long, help = "Overwrite local files that already exist")]
        force: bool,
    },
    /// Remove matching crash reports from the device
    Rm {
        #[arg(help = "Optional filename glob pattern", default_value = "*")]
        pattern: String,
        #[arg(long, help = "Required confirmation for destructive removal")]
        force: bool,
    },
    /// Flush pending crash products into the device crash-report directory
    Flush {
        #[arg(long, default_value_t = 20, help = "Handshake timeout in seconds")]
        timeout: u64,
        #[arg(short = 'j', long, help = "Output JSON")]
        json: bool,
    },
    /// Parse a local .ips or legacy .crash report without a device
    Parse {
        #[arg(help = "Local report path")]
        input: String,
        #[arg(short = 'j', long, help = "Output JSON")]
        json: bool,
    },
    /// Parse the newest local reports by event timestamp
    ParseLatest {
        #[arg(help = "Local report directory")]
        directory: String,
        #[arg(long, default_value = "*", help = "Filename glob pattern")]
        pattern: String,
        #[arg(long, default_value_t = 1, help = "Maximum number of reports")]
        count: usize,
        #[arg(short = 'j', long, help = "Output JSON")]
        json: bool,
    },
    /// Remove all crash reports below a device path
    Clear {
        #[arg(long, default_value = ".", help = "Path in the crash-report AFC jail")]
        path: String,
        #[arg(long, help = "Required confirmation for destructive removal")]
        force: bool,
    },
    /// Poll for new reports until Ctrl+C or the optional timeout
    Watch {
        #[arg(long, default_value = "*", help = "Filename glob pattern")]
        pattern: String,
        #[arg(long, help = "Stop after this many seconds")]
        timeout: Option<u64>,
        #[arg(long, default_value_t = 1, help = "Polling interval in seconds")]
        interval: u64,
        #[arg(short = 'j', long, help = "Output one JSON object per report")]
        json: bool,
    },
    /// Sysdiagnose collection is not exposed by the pinned crash mover
    Sysdiagnose {
        #[arg(
            long,
            help = "Output archive path (reserved for a future protocol implementation)"
        )]
        output: Option<String>,
        #[arg(long, help = "Required confirmation")]
        force: bool,
        #[arg(long, help = "Collection timeout in seconds")]
        timeout: Option<u64>,
    },
}

impl CrashCmd {
    pub async fn run(self, udid: Option<String>) -> Result<()> {
        if matches!(
            &self.sub,
            CrashSub::Parse { .. } | CrashSub::ParseLatest { .. }
        ) {
            return self.run_local().await;
        }
        if let CrashSub::Sysdiagnose { .. } = &self.sub {
            return Err(anyhow::anyhow!(
                "crash-report sysdiagnose collection is not implemented; `ios diagnostics sysdiagnose` only probes CoreDevice metadata (dry-run)"
            ));
        }
        let udid = udid.ok_or_else(|| anyhow::anyhow!("--udid required for crash commands"))?;
        let flush_deadline = match &self.sub {
            CrashSub::Flush { timeout, .. } => Some(flush_deadline(*timeout)?),
            _ => None,
        };
        let device = connect_crash_device(&udid, flush_deadline).await?;
        let use_rsd = device.rsd().is_some();

        match self.sub {
            CrashSub::Ls { pattern, json } => {
                let mut mover = connect_crash_service(
                    &device,
                    CRASHREPORT_MOVER_SERVICE,
                    RSD_CRASHREPORT_MOVER_SERVICE,
                )
                .await?;
                prepare_crash_reports(&mut mover, use_rsd).await?;

                let stream = connect_crash_service(
                    &device,
                    CRASHREPORT_COPY_MOBILE_SERVICE,
                    RSD_CRASHREPORT_COPY_MOBILE_SERVICE,
                )
                .await?;
                let mut client = CrashReportClient::new(stream);
                let reports = client.list_reports(Some(&pattern)).await?;

                if json {
                    println!(
                        "{}",
                        serde_json::to_string_pretty(&reports_to_json(&reports))?
                    );
                } else {
                    let mut table = Table::new();
                    table.load_preset(UTF8_FULL);
                    table.set_header(["Modified", "Size", "Path"]);
                    for report in reports {
                        table.add_row([
                            report.modified.unwrap_or_else(|| "-".to_string()),
                            report
                                .size
                                .map(|size| size.to_string())
                                .unwrap_or_else(|| "-".to_string()),
                            report.path,
                        ]);
                    }
                    println!("{table}");
                }
            }
            CrashSub::Pull {
                report,
                local,
                force,
            } => {
                let local_path = local.unwrap_or_else(|| default_local_path(&report));
                // Checked before the transfer so a doomed pull costs nothing.
                crate::cmd::file::ensure_local_overwrite_allowed(Path::new(&local_path), force)?;

                let mut mover = connect_crash_service(
                    &device,
                    CRASHREPORT_MOVER_SERVICE,
                    RSD_CRASHREPORT_MOVER_SERVICE,
                )
                .await?;
                prepare_crash_reports(&mut mover, use_rsd).await?;

                let stream = connect_crash_service(
                    &device,
                    CRASHREPORT_COPY_MOBILE_SERVICE,
                    RSD_CRASHREPORT_COPY_MOBILE_SERVICE,
                )
                .await?;
                let mut client = CrashReportClient::new(stream);
                let data = client.read_report(&report).await?;
                crate::cmd::file::write_local_bytes_atomic(Path::new(&local_path), &data, force)
                    .await?;
                println!("Downloaded {} bytes to {}", data.len(), local_path);
            }
            CrashSub::Show { report, head } => {
                let mut mover = connect_crash_service(
                    &device,
                    CRASHREPORT_MOVER_SERVICE,
                    RSD_CRASHREPORT_MOVER_SERVICE,
                )
                .await?;
                prepare_crash_reports(&mut mover, use_rsd).await?;

                let stream = connect_crash_service(
                    &device,
                    CRASHREPORT_COPY_MOBILE_SERVICE,
                    RSD_CRASHREPORT_COPY_MOBILE_SERVICE,
                )
                .await?;
                let mut client = CrashReportClient::new(stream);
                let data = client.read_report(&report).await?;
                let text = decode_report_text(&data)?;
                if head {
                    println!("{}", head_line(&text)?);
                } else {
                    print!("{text}");
                }
            }
            CrashSub::PullAll {
                pattern,
                local_dir,
                force,
            } => {
                let mut mover = connect_crash_service(
                    &device,
                    CRASHREPORT_MOVER_SERVICE,
                    RSD_CRASHREPORT_MOVER_SERVICE,
                )
                .await?;
                prepare_crash_reports(&mut mover, use_rsd).await?;

                let stream = connect_crash_service(
                    &device,
                    CRASHREPORT_COPY_MOBILE_SERVICE,
                    RSD_CRASHREPORT_COPY_MOBILE_SERVICE,
                )
                .await?;
                let mut client = CrashReportClient::new(stream);
                let reports = client.list_reports(Some(&pattern)).await?;

                fs::create_dir_all(&local_dir).await?;
                let mut taken = HashSet::new();
                for report in reports {
                    let local_path =
                        unique_local_path(Path::new(&local_dir), &report.path, &mut taken);
                    crate::cmd::file::ensure_local_overwrite_allowed(&local_path, force)?;
                    let data = client.read_report(&report.path).await?;
                    crate::cmd::file::write_local_bytes_atomic(&local_path, &data, force).await?;
                    println!(
                        "Downloaded {} bytes to {}",
                        data.len(),
                        local_path.display()
                    );
                }
            }
            CrashSub::Rm { pattern, force } => {
                if !force {
                    return Err(anyhow::anyhow!(
                        "refusing to remove crash reports without --force"
                    ));
                }
                let mut mover = connect_crash_service(
                    &device,
                    CRASHREPORT_MOVER_SERVICE,
                    RSD_CRASHREPORT_MOVER_SERVICE,
                )
                .await?;
                prepare_crash_reports(&mut mover, use_rsd).await?;

                let stream = connect_crash_service(
                    &device,
                    CRASHREPORT_COPY_MOBILE_SERVICE,
                    RSD_CRASHREPORT_COPY_MOBILE_SERVICE,
                )
                .await?;
                let mut client = CrashReportClient::new(stream);
                let removed = client.remove_reports(Some(&pattern)).await?;
                if removed == 0 {
                    println!("No crash reports matched {pattern}");
                    return Ok(());
                }
                println!("Removed {removed} crash report(s)");
            }
            CrashSub::Flush { json, .. } => {
                let mut mover = connect_crash_service_until(
                    &device,
                    CRASHREPORT_MOVER_SERVICE,
                    RSD_CRASHREPORT_MOVER_SERVICE,
                    flush_deadline,
                )
                .await?;
                let deadline = flush_deadline
                    .ok_or_else(|| anyhow::anyhow!("missing crash operation deadline"))?;
                flush_crash_reports(&mut mover, use_rsd, deadline).await?;
                if json {
                    println!(r#"{{"flushed":true}}"#);
                } else {
                    println!("Crash reports flushed");
                }
            }
            CrashSub::Clear { path, force } => {
                if !force {
                    return Err(anyhow::anyhow!(
                        "refusing to clear crash reports without --force"
                    ));
                }
                let mut mover = connect_crash_service(
                    &device,
                    CRASHREPORT_MOVER_SERVICE,
                    RSD_CRASHREPORT_MOVER_SERVICE,
                )
                .await?;
                prepare_crash_reports(&mut mover, use_rsd).await?;
                let stream = connect_crash_service(
                    &device,
                    CRASHREPORT_COPY_MOBILE_SERVICE,
                    RSD_CRASHREPORT_COPY_MOBILE_SERVICE,
                )
                .await?;
                let mut client = CrashReportClient::new(stream);
                let removed = client.clear_reports(&path).await?;
                println!(
                    "Removed {removed} crash report entr{}",
                    if removed == 1 { "y" } else { "ies" }
                );
            }
            CrashSub::Watch {
                pattern,
                timeout,
                interval,
                json,
            } => {
                let mut mover = connect_crash_service(
                    &device,
                    CRASHREPORT_MOVER_SERVICE,
                    RSD_CRASHREPORT_MOVER_SERVICE,
                )
                .await?;
                prepare_crash_reports(&mut mover, use_rsd).await?;
                let stream = connect_crash_service(
                    &device,
                    CRASHREPORT_COPY_MOBILE_SERVICE,
                    RSD_CRASHREPORT_COPY_MOBILE_SERVICE,
                )
                .await?;
                let mut client = CrashReportClient::new(stream);
                let options = CrashWatchOptions {
                    poll_interval: Duration::from_secs(interval.max(1)),
                    timeout: timeout.map(Duration::from_secs),
                    ..CrashWatchOptions::default()
                };
                let mut reports = Box::pin(client.watch_reports(Some(&pattern), options));
                loop {
                    tokio::select! {
                        _ = tokio::signal::ctrl_c() => break,
                        item = reports.next() => match item {
                            Some(Ok(report)) => print_parsed_report(&report, json)?,
                            Some(Err(error)) => return Err(error.into()),
                            None => break,
                        },
                    }
                }
            }
            CrashSub::Parse { .. }
            | CrashSub::ParseLatest { .. }
            | CrashSub::Sysdiagnose { .. } => {
                unreachable!("handled before device connection")
            }
        }

        Ok(())
    }

    async fn run_local(self) -> Result<()> {
        match self.sub {
            CrashSub::Parse { input, json } => {
                let data = read_local_report(Path::new(&input)).await?;
                let report = parse_report_bytes(&input, &data)?;
                print_parsed_report(&report, json)?;
            }
            CrashSub::ParseLatest {
                directory,
                pattern,
                count,
                json,
            } => {
                if count == 0 {
                    return Err(anyhow::anyhow!("--count must be greater than zero"));
                }
                let mut dir = fs::read_dir(&directory).await?;
                let mut reports = Vec::new();
                while let Some(entry) = dir.next_entry().await? {
                    if !entry.file_type().await?.is_file() {
                        continue;
                    }
                    let path = entry.path();
                    let path_string = path.to_string_lossy().into_owned();
                    if !matches!(
                        path.extension()
                            .and_then(|extension| extension.to_str())
                            .map(|extension| extension.to_ascii_lowercase())
                            .as_deref(),
                        Some("ips") | Some("panic") | Some("crash")
                    ) {
                        continue;
                    }
                    if !matches_pattern(&path_string, &pattern)? {
                        continue;
                    }
                    let data = read_local_report(&path).await?;
                    reports.push(parse_report_bytes(&path_string, &data)?);
                }
                if reports.is_empty() {
                    return Err(anyhow::anyhow!("no crash reports found"));
                }
                sort_parsed_reports(&mut reports);
                reports.truncate(count);
                if json {
                    println!("{}", serde_json::to_string_pretty(&reports)?);
                } else {
                    for report in reports {
                        print_parsed_report(&report, false)?;
                    }
                }
            }
            _ => unreachable!("local command guard"),
        }
        Ok(())
    }
}

async fn connect_crash_device(
    udid: &str,
    deadline: Option<tokio::time::Instant>,
) -> Result<ios_core::ConnectedDevice> {
    let classic = ConnectOptions {
        tun_mode: TunMode::Userspace,
        pair_record_path: None,
        skip_tunnel: true,
    };
    let device = connect_with_deadline(udid, classic, deadline).await?;

    // A classic lockdown connection is needed to identify pre-iOS 17 devices
    // without eagerly starting a CoreDevice tunnel that they cannot support.
    // Modern devices are then reconnected once with the normal tunnel/RSD
    // setup, preserving the existing classic fallback.
    let modern = match deadline {
        Some(deadline) => match tokio::time::timeout_at(deadline, device.product_version()).await {
            Ok(Ok(version)) => version.major >= 17,
            Ok(Err(error)) => return Err(error.into()),
            Err(_) => return Err(anyhow::anyhow!("crash service connection timed out")),
        },
        None => device.product_version().await?.major >= 17,
    };
    if !modern {
        return Ok(device);
    }

    let tunneled = ConnectOptions {
        tun_mode: TunMode::Userspace,
        pair_record_path: None,
        skip_tunnel: false,
    };
    connect_with_deadline(udid, tunneled, deadline).await
}

async fn connect_with_deadline(
    udid: &str,
    options: ConnectOptions,
    deadline: Option<tokio::time::Instant>,
) -> Result<ios_core::ConnectedDevice> {
    match deadline {
        Some(deadline) => {
            let device = tokio::time::timeout_at(deadline, connect(udid, options))
                .await
                .map_err(|_| anyhow::anyhow!("crash service connection timed out"))??;
            Ok(device)
        }
        None => Ok(connect(udid, options).await?),
    }
}

async fn connect_crash_service(
    device: &ios_core::ConnectedDevice,
    classic_service: &str,
    rsd_service: &str,
) -> Result<ios_core::ServiceStream> {
    if device.rsd().is_some() {
        Ok(device.connect_rsd_service(rsd_service).await?)
    } else {
        Ok(device.connect_service(classic_service).await?)
    }
}

async fn connect_crash_service_until(
    device: &ios_core::ConnectedDevice,
    classic_service: &str,
    rsd_service: &str,
    deadline: Option<tokio::time::Instant>,
) -> Result<ios_core::ServiceStream> {
    match deadline {
        Some(deadline) => {
            let result = if device.rsd().is_some() {
                tokio::time::timeout_at(deadline, device.connect_rsd_service(rsd_service)).await
            } else {
                tokio::time::timeout_at(deadline, device.connect_service(classic_service)).await
            };
            Ok(result.map_err(|_| anyhow::anyhow!("crash service connection timed out"))??)
        }
        None => connect_crash_service(device, classic_service, rsd_service).await,
    }
}

async fn prepare_crash_reports(stream: &mut ios_core::ServiceStream, use_rsd: bool) -> Result<()> {
    if use_rsd {
        prepare_reports_rsd(stream).await?;
    } else {
        prepare_reports(stream).await?;
    }
    Ok(())
}

async fn flush_crash_reports(
    stream: &mut ios_core::ServiceStream,
    use_rsd: bool,
    deadline: tokio::time::Instant,
) -> Result<()> {
    if use_rsd {
        flush_reports_rsd_at(stream, deadline).await?;
    } else {
        flush_reports_at(stream, deadline).await?;
    }
    Ok(())
}

fn flush_deadline(timeout: u64) -> Result<tokio::time::Instant> {
    tokio::time::Instant::now()
        .checked_add(Duration::from_secs(timeout))
        .ok_or_else(|| anyhow::anyhow!("flush timeout is too large"))
}

/// Read a local report without allowing a sparse or growing file to bypass the
/// parser's 16 MiB report limit.  `take` also makes the check race-safe after
/// the initial metadata lookup has become stale.
async fn read_local_report(path: &Path) -> Result<Vec<u8>> {
    let file = fs::File::open(path).await?;
    let mut data = Vec::new();
    file.take((ios_core::crashreport::MAX_CRASH_REPORT_BYTES as u64) + 1)
        .read_to_end(&mut data)
        .await?;
    if data.len() > ios_core::crashreport::MAX_CRASH_REPORT_BYTES {
        return Err(anyhow::anyhow!(
            "crash report exceeds {} byte limit",
            ios_core::crashreport::MAX_CRASH_REPORT_BYTES
        ));
    }
    Ok(data)
}

fn print_parsed_report(report: &ParsedCrashReport, json: bool) -> Result<()> {
    if json {
        println!("{}", serde_json::to_string(report)?);
        return Ok(());
    }
    println!("{} {}", report.format, report.path);
    if let Some(value) = &report.incident_id {
        println!("incident_id: {value}");
    }
    if let Some(value) = &report.timestamp {
        println!("timestamp: {value}");
    }
    if let Some(value) = &report.process {
        println!("process: {value}");
    }
    if let Some(value) = &report.bundle_id {
        println!("bundle_id: {value}");
    }
    if let Some(value) = &report.process_path {
        println!("process_path: {value}");
    }
    if let Some(value) = report.pid {
        println!("pid: {value}");
    }
    if let Some(value) = &report.exception {
        println!("exception: {value}");
    }
    if let Some(value) = &report.termination {
        println!("termination: {value}");
    }
    if let Some(value) = report.triggered_thread {
        println!("triggered_thread: {value}");
    }
    println!(
        "threads: {}  images: {}",
        report.threads.len(),
        report.images.len()
    );
    Ok(())
}

fn default_local_path(report: &str) -> String {
    Path::new(report)
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .filter(|name| !name.is_empty())
        .unwrap_or_else(|| report.to_string())
}

/// Pick a destination inside `local_dir` that no earlier report in this run claimed.
///
/// Reports keep their device directory structure, so `pull-all` flattening to
/// basenames makes same-named reports from different directories collide; without
/// a suffix the last download would be the only one left on disk.
fn unique_local_path(local_dir: &Path, report: &str, taken: &mut HashSet<PathBuf>) -> PathBuf {
    let file_name = default_local_path(report);
    let candidate = local_dir.join(&file_name);
    if taken.insert(candidate.clone()) {
        return candidate;
    }

    let name = Path::new(&file_name);
    let stem = name
        .file_stem()
        .map(|stem| stem.to_string_lossy().into_owned())
        .unwrap_or_else(|| file_name.clone());
    let extension = name
        .extension()
        .map(|extension| extension.to_string_lossy().into_owned());

    let mut suffix = 1u32;
    loop {
        let candidate_name = match &extension {
            Some(extension) => format!("{stem}-{suffix}.{extension}"),
            None => format!("{stem}-{suffix}"),
        };
        let candidate = local_dir.join(candidate_name);
        if taken.insert(candidate.clone()) {
            return candidate;
        }
        suffix += 1;
    }
}

fn decode_report_text(data: &[u8]) -> Result<String> {
    Ok(String::from_utf8_lossy(data).into_owned())
}

fn head_line(text: &str) -> Result<String> {
    text.lines()
        .next()
        .map(ToOwned::to_owned)
        .filter(|line| !line.is_empty())
        .ok_or_else(|| anyhow::anyhow!("crash report was empty"))
}

fn reports_to_json(reports: &[ios_core::crashreport::CrashReportEntry]) -> serde_json::Value {
    serde_json::Value::Array(
        reports
            .iter()
            .map(|report| {
                serde_json::json!({
                    "path": report.path,
                    "size": report.size,
                    "modified": report.modified,
                })
            })
            .collect(),
    )
}

#[cfg(test)]
mod tests {
    use std::time::{SystemTime, UNIX_EPOCH};

    use clap::Parser;

    use super::CrashSub;

    #[derive(Parser)]
    struct TestCli {
        #[command(subcommand)]
        command: CrashSub,
    }

    #[test]
    fn parses_crash_ls_subcommand() {
        let cmd = TestCli::parse_from(["crash", "ls", "*.ips", "--json"]);
        match cmd.command {
            CrashSub::Ls { pattern, json } => {
                assert_eq!(pattern, "*.ips");
                assert!(json);
            }
            _ => panic!("expected ls subcommand"),
        }
    }

    #[test]
    fn parses_crash_pull_subcommand() {
        let cmd = TestCli::parse_from(["crash", "pull", "Example.ips", "local.ips"]);
        match cmd.command {
            CrashSub::Pull {
                report,
                local,
                force,
            } => {
                assert_eq!(report, "Example.ips");
                assert_eq!(local.as_deref(), Some("local.ips"));
                assert!(!force);
            }
            _ => panic!("expected pull subcommand"),
        }
    }

    #[test]
    fn parses_crash_pull_force_flag() {
        let cmd = TestCli::parse_from(["crash", "pull", "Example.ips", "local.ips", "--force"]);
        match cmd.command {
            CrashSub::Pull { force, .. } => assert!(force),
            _ => panic!("expected pull subcommand"),
        }
    }

    #[test]
    fn parses_crash_show_subcommand() {
        let cmd = TestCli::parse_from(["crash", "show", "Example.ips", "--head"]);
        match cmd.command {
            CrashSub::Show { report, head } => {
                assert_eq!(report, "Example.ips");
                assert!(head);
            }
            _ => panic!("expected show subcommand"),
        }
    }

    #[test]
    fn parses_crash_pull_all_subcommand_with_defaults() {
        let cmd = TestCli::parse_from(["crash", "pull-all"]);
        match cmd.command {
            CrashSub::PullAll {
                pattern,
                local_dir,
                force,
            } => {
                assert_eq!(pattern, "*");
                assert_eq!(local_dir, ".");
                assert!(!force);
            }
            _ => panic!("expected pull-all subcommand"),
        }
    }

    #[test]
    fn parses_crash_pull_all_subcommand_with_args() {
        let cmd = TestCli::parse_from(["crash", "pull-all", "*.ips", "exports", "--force"]);
        match cmd.command {
            CrashSub::PullAll {
                pattern,
                local_dir,
                force,
            } => {
                assert_eq!(pattern, "*.ips");
                assert_eq!(local_dir, "exports");
                assert!(force);
            }
            _ => panic!("expected pull-all subcommand"),
        }
    }

    #[test]
    fn parses_crash_rm_subcommand_with_default_pattern() {
        let cmd = TestCli::parse_from(["crash", "rm"]);
        match cmd.command {
            CrashSub::Rm { pattern, force } => {
                assert_eq!(pattern, "*");
                assert!(!force);
            }
            _ => panic!("expected rm subcommand"),
        }
    }

    #[test]
    fn parses_crash_rm_subcommand_with_args() {
        let cmd = TestCli::parse_from(["crash", "rm", "*.ips", "--force"]);
        match cmd.command {
            CrashSub::Rm { pattern, force } => {
                assert_eq!(pattern, "*.ips");
                assert!(force);
            }
            _ => panic!("expected rm subcommand"),
        }
    }

    #[test]
    fn parses_crash_parse_and_watch_commands() {
        let cmd = TestCli::parse_from(["crash", "parse", "report.ips", "--json"]);
        assert!(
            matches!(cmd.command, CrashSub::Parse { input, json } if input == "report.ips" && json)
        );
        let cmd = TestCli::parse_from([
            "crash",
            "watch",
            "--pattern",
            "*.ips",
            "--timeout",
            "5",
            "--interval",
            "2",
            "--json",
        ]);
        assert!(
            matches!(cmd.command, CrashSub::Watch { pattern, timeout: Some(5), interval: 2, json } if pattern == "*.ips" && json)
        );
    }

    #[test]
    fn flush_deadline_rejects_duration_overflow() {
        assert!(super::flush_deadline(u64::MAX).is_err());
        assert!(super::flush_deadline(0).is_ok());
        assert!(super::flush_deadline(1).is_ok());
    }

    #[test]
    fn destructive_crash_commands_expose_force() {
        let cmd = TestCli::parse_from(["crash", "clear", "--path", "./nested", "--force"]);
        assert!(
            matches!(cmd.command, CrashSub::Clear { path, force } if path == "./nested" && force)
        );
    }

    #[test]
    fn default_local_path_uses_basename() {
        assert_eq!(
            super::default_local_path("./foo/Example.ips"),
            "Example.ips"
        );
    }

    #[test]
    fn unique_local_path_suffixes_colliding_basenames() {
        let dir = std::path::Path::new("exports");
        let mut taken = std::collections::HashSet::new();

        assert_eq!(
            super::unique_local_path(dir, "./Foo/Example.ips", &mut taken),
            dir.join("Example.ips")
        );
        assert_eq!(
            super::unique_local_path(dir, "./Bar/Example.ips", &mut taken),
            dir.join("Example-1.ips")
        );
        assert_eq!(
            super::unique_local_path(dir, "./Baz/Example.ips", &mut taken),
            dir.join("Example-2.ips")
        );
        assert_eq!(
            super::unique_local_path(dir, "./Foo/stacks", &mut taken),
            dir.join("stacks")
        );
        assert_eq!(
            super::unique_local_path(dir, "./Bar/stacks", &mut taken),
            dir.join("stacks-1")
        );
    }

    #[test]
    fn decode_report_text_accepts_utf8() {
        assert_eq!(
            super::decode_report_text(br#"{"bug_type":"109"}"#).unwrap(),
            r#"{"bug_type":"109"}"#
        );
    }

    #[test]
    fn decode_report_text_replaces_invalid_utf8() {
        let text = super::decode_report_text(&[0x66, 0x6f, 0x80, 0x6f]).unwrap();
        assert_eq!(text, "fo\u{fffd}o");
    }

    #[test]
    fn head_line_extracts_first_line() {
        let text = super::head_line("{\"bug_type\":\"221\"}\nBINARY\u{fffd}blob").unwrap();
        assert_eq!(text, "{\"bug_type\":\"221\"}");
    }

    #[test]
    fn crash_entry_json_shape_includes_path_size_and_modified() {
        let value = serde_json::json!({
            "path": "./Example.ips",
            "size": 1234,
            "modified": "2026-04-09 01:44:25 UTC"
        });
        assert_eq!(value["path"], "./Example.ips");
        assert_eq!(value["size"], 1234);
        assert_eq!(value["modified"], "2026-04-09 01:44:25 UTC");
    }

    #[tokio::test]
    async fn local_report_reader_rejects_oversized_files_before_parse() {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = std::env::temp_dir().join(format!(
            "ios-crash-reader-{}-{nonce}.ips",
            std::process::id()
        ));
        tokio::fs::write(
            &path,
            vec![b'x'; ios_core::crashreport::MAX_CRASH_REPORT_BYTES + 1],
        )
        .await
        .unwrap();

        let error = super::read_local_report(&path).await.unwrap_err();
        assert!(error.to_string().contains("byte limit"));
        tokio::fs::remove_file(path).await.unwrap();
    }
}
