use std::path::Path;

use anyhow::Result;
use comfy_table::{presets::UTF8_FULL, Table};
use ios_core::crashreport::{
    prepare_reports, CrashReportClient, CRASHREPORT_COPY_MOBILE_SERVICE, CRASHREPORT_MOVER_SERVICE,
};
use ios_core::tunnel::TunMode;
use ios_core::{connect, ConnectOptions};
use tokio::fs;

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
    },
    /// Remove matching crash reports from the device
    Rm {
        #[arg(help = "Optional filename glob pattern", default_value = "*")]
        pattern: String,
    },
}

impl CrashCmd {
    pub async fn run(self, udid: Option<String>) -> Result<()> {
        let udid = udid.ok_or_else(|| anyhow::anyhow!("--udid required for crash commands"))?;
        let opts = ConnectOptions {
            tun_mode: TunMode::Userspace,
            pair_record_path: None,
            skip_tunnel: true,
        };
        let device = connect(&udid, opts).await?;

        match self.sub {
            CrashSub::Ls { pattern, json } => {
                let mut mover = device.connect_service(CRASHREPORT_MOVER_SERVICE).await?;
                prepare_reports(&mut mover).await?;

                let stream = device
                    .connect_service(CRASHREPORT_COPY_MOBILE_SERVICE)
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
            CrashSub::Pull { report, local } => {
                let mut mover = device.connect_service(CRASHREPORT_MOVER_SERVICE).await?;
                prepare_reports(&mut mover).await?;

                let stream = device
                    .connect_service(CRASHREPORT_COPY_MOBILE_SERVICE)
                    .await?;
                let mut client = CrashReportClient::new(stream);
                let data = client.read_report(&report).await?;
                let local_path = local.unwrap_or_else(|| default_local_path(&report));
                fs::write(&local_path, &data).await?;
                println!("Downloaded {} bytes to {}", data.len(), local_path);
            }
            CrashSub::Show { report, head } => {
                let mut mover = device.connect_service(CRASHREPORT_MOVER_SERVICE).await?;
                prepare_reports(&mut mover).await?;

                let stream = device
                    .connect_service(CRASHREPORT_COPY_MOBILE_SERVICE)
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
            CrashSub::PullAll { pattern, local_dir } => {
                let mut mover = device.connect_service(CRASHREPORT_MOVER_SERVICE).await?;
                prepare_reports(&mut mover).await?;

                let stream = device
                    .connect_service(CRASHREPORT_COPY_MOBILE_SERVICE)
                    .await?;
                let mut client = CrashReportClient::new(stream);
                let reports = client.list_reports(Some(&pattern)).await?;

                fs::create_dir_all(&local_dir).await?;
                for report in reports {
                    let data = client.read_report(&report.path).await?;
                    let local_path = Path::new(&local_dir).join(default_local_path(&report.path));
                    fs::write(&local_path, &data).await?;
                    println!(
                        "Downloaded {} bytes to {}",
                        data.len(),
                        local_path.display()
                    );
                }
            }
            CrashSub::Rm { pattern } => {
                let mut mover = device.connect_service(CRASHREPORT_MOVER_SERVICE).await?;
                prepare_reports(&mut mover).await?;

                let stream = device
                    .connect_service(CRASHREPORT_COPY_MOBILE_SERVICE)
                    .await?;
                let mut client = CrashReportClient::new(stream);
                let removed = client.remove_reports(Some(&pattern)).await?;
                if removed == 0 {
                    println!("No crash reports matched {pattern}");
                    return Ok(());
                }
                println!("Removed {removed} crash report(s)");
            }
        }

        Ok(())
    }
}

fn default_local_path(report: &str) -> String {
    Path::new(report)
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .filter(|name| !name.is_empty())
        .unwrap_or_else(|| report.to_string())
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
            CrashSub::Pull { report, local } => {
                assert_eq!(report, "Example.ips");
                assert_eq!(local.as_deref(), Some("local.ips"));
            }
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
            CrashSub::PullAll { pattern, local_dir } => {
                assert_eq!(pattern, "*");
                assert_eq!(local_dir, ".");
            }
            _ => panic!("expected pull-all subcommand"),
        }
    }

    #[test]
    fn parses_crash_pull_all_subcommand_with_args() {
        let cmd = TestCli::parse_from(["crash", "pull-all", "*.ips", "exports"]);
        match cmd.command {
            CrashSub::PullAll { pattern, local_dir } => {
                assert_eq!(pattern, "*.ips");
                assert_eq!(local_dir, "exports");
            }
            _ => panic!("expected pull-all subcommand"),
        }
    }

    #[test]
    fn parses_crash_rm_subcommand_with_default_pattern() {
        let cmd = TestCli::parse_from(["crash", "rm"]);
        match cmd.command {
            CrashSub::Rm { pattern } => assert_eq!(pattern, "*"),
            _ => panic!("expected rm subcommand"),
        }
    }

    #[test]
    fn parses_crash_rm_subcommand_with_args() {
        let cmd = TestCli::parse_from(["crash", "rm", "*.ips"]);
        match cmd.command {
            CrashSub::Rm { pattern } => assert_eq!(pattern, "*.ips"),
            _ => panic!("expected rm subcommand"),
        }
    }

    #[test]
    fn default_local_path_uses_basename() {
        assert_eq!(
            super::default_local_path("./foo/Example.ips"),
            "Example.ips"
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
}
