use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::Result;
use ios_core::services::pcap::{
    write_global_header, write_packet_record, CapturedPacket, PacketFilter, PcapClient,
    SERVICE_NAME,
};
use ios_core::tunnel::TunMode;
use ios_core::{connect, ConnectOptions};
use serde::Serialize;
use tokio::time::{timeout, Instant};

#[derive(clap::Args)]
pub struct PcapCmd {
    #[arg(
        long,
        default_value_t = 10,
        help = "Maximum capture duration in seconds"
    )]
    duration: u64,
    #[arg(long, help = "Stop after writing this many packets")]
    count: Option<usize>,
    #[arg(long, help = "Only capture packets whose metadata matches this PID")]
    pid: Option<i32>,
    #[arg(
        long,
        help = "Only capture packets whose process name starts with this prefix"
    )]
    process: Option<String>,
    #[arg(long, help = "Output pcap path", default_value = "capture.pcap")]
    output: PathBuf,
    #[arg(
        long,
        help = "Print a JSON completion summary for this command only; this does not change the global CLI JSON policy"
    )]
    json: bool,
}

impl PcapCmd {
    pub async fn run(self, udid: Option<String>) -> Result<()> {
        let udid = udid.ok_or_else(|| anyhow::anyhow!("--udid required for pcap"))?;

        let opts = ConnectOptions {
            tun_mode: TunMode::Userspace,
            pair_record_path: None,
            skip_tunnel: true,
        };
        let device = connect(&udid, opts).await?;
        let stream = device.connect_service(SERVICE_NAME).await?;
        let mut client = PcapClient::new(stream);

        let mut file = std::fs::File::create(&self.output)?;
        write_global_header(&mut file)?;

        let deadline = Instant::now() + Duration::from_secs(self.duration);
        let filter = PacketFilter {
            pid: self.pid,
            process_prefix: self.process.clone(),
        };
        let mut written = 0usize;
        let mut capture_metadata = PcapSummaryCaptureMetadataBuilder::new(&filter);

        loop {
            if let Some(limit) = self.count {
                if written >= limit {
                    break;
                }
            }
            let now = Instant::now();
            if now >= deadline {
                break;
            }

            let remaining = deadline.saturating_duration_since(now);
            match timeout(remaining, client.next_packet()).await {
                Ok(Ok(packet)) => {
                    if filter.matches(&packet) {
                        write_packet_record(&mut file, &packet)?;
                        written += 1;
                        capture_metadata.record(&packet);
                    }
                }
                Ok(Err(err)) => return Err(anyhow::anyhow!("pcap: {err}")),
                Err(_) => break,
            }
        }

        if self.json {
            let summary =
                PcapSummary::new(&self.output, written, &filter, capture_metadata.finish());
            println!("{}", serde_json::to_string_pretty(&summary)?);
        } else {
            println!(
                "Saved {written} packet(s) to {}",
                self.output.to_string_lossy()
            );
        }
        Ok(())
    }
}

#[derive(Debug, Serialize, PartialEq, Eq)]
struct PcapSummary {
    output: String,
    written_packets: usize,
    filters: PcapSummaryFilters,
    capture_metadata: PcapSummaryCaptureMetadata,
}

#[derive(Debug, Serialize, PartialEq, Eq)]
struct PcapSummaryFilters {
    pid: Option<i32>,
    process_prefix: Option<String>,
}

#[derive(Debug, Default, Serialize, PartialEq, Eq)]
struct PcapSummaryCaptureMetadata {
    interfaces: Vec<String>,
    processes: Vec<String>,
    pids: Vec<i32>,
    first_timestamp: Option<PcapSummaryTimestamp>,
    last_timestamp: Option<PcapSummaryTimestamp>,
    filter_hit: Option<bool>,
}

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq, PartialOrd, Ord)]
struct PcapSummaryTimestamp {
    sec: u32,
    usec: u32,
}

#[derive(Debug, Default)]
struct PcapSummaryCaptureMetadataBuilder {
    interfaces: BTreeSet<String>,
    processes: BTreeSet<String>,
    pids: BTreeSet<i32>,
    first_timestamp: Option<PcapSummaryTimestamp>,
    last_timestamp: Option<PcapSummaryTimestamp>,
    filter_hit: Option<bool>,
}

impl PcapSummary {
    fn new(
        output: &Path,
        written_packets: usize,
        filter: &PacketFilter,
        capture_metadata: PcapSummaryCaptureMetadata,
    ) -> Self {
        Self {
            output: output.to_string_lossy().to_string(),
            written_packets,
            filters: PcapSummaryFilters {
                pid: filter.pid,
                process_prefix: filter.process_prefix.clone(),
            },
            capture_metadata,
        }
    }
}

impl PcapSummaryCaptureMetadataBuilder {
    fn new(filter: &PacketFilter) -> Self {
        Self {
            filter_hit: filter.is_active().then_some(false),
            ..Self::default()
        }
    }

    fn record(&mut self, packet: &CapturedPacket) {
        self.interfaces.insert(packet.interface_name.clone());
        if !packet.proc_name.is_empty() {
            self.processes.insert(packet.proc_name.clone());
        }
        if !packet.proc_name2.is_empty() {
            self.processes.insert(packet.proc_name2.clone());
        }
        self.pids.insert(packet.pid);
        self.pids.insert(packet.pid2);

        let timestamp = PcapSummaryTimestamp {
            sec: packet.ts_sec,
            usec: packet.ts_usec,
        };
        self.first_timestamp = match self.first_timestamp {
            Some(current) if current <= timestamp => Some(current),
            _ => Some(timestamp),
        };
        self.last_timestamp = match self.last_timestamp {
            Some(current) if current >= timestamp => Some(current),
            _ => Some(timestamp),
        };

        if self.filter_hit.is_some() {
            self.filter_hit = Some(true);
        }
    }

    fn finish(self) -> PcapSummaryCaptureMetadata {
        PcapSummaryCaptureMetadata {
            interfaces: self.interfaces.into_iter().collect(),
            processes: self.processes.into_iter().collect(),
            pids: self.pids.into_iter().collect(),
            first_timestamp: self.first_timestamp,
            last_timestamp: self.last_timestamp,
            filter_hit: self.filter_hit,
        }
    }
}

trait PacketFilterExt {
    fn is_active(&self) -> bool;
}

impl PacketFilterExt for PacketFilter {
    fn is_active(&self) -> bool {
        self.pid.is_some() || self.process_prefix.is_some()
    }
}

#[cfg(test)]
mod tests {
    use clap::Parser;

    use super::*;

    #[derive(Parser)]
    struct TestCli {
        #[command(flatten)]
        command: PcapCmd,
    }

    #[test]
    fn parses_pcap_args() {
        let cmd = TestCli::parse_from([
            "pcap",
            "--duration",
            "5",
            "--count",
            "3",
            "--pid",
            "123",
            "--process",
            "WebKit",
            "--output",
            "dump.pcap",
        ]);
        assert_eq!(cmd.command.duration, 5);
        assert_eq!(cmd.command.count, Some(3));
        assert_eq!(cmd.command.pid, Some(123));
        assert_eq!(cmd.command.process.as_deref(), Some("WebKit"));
        assert_eq!(cmd.command.output, PathBuf::from("dump.pcap"));
    }

    #[test]
    fn parses_pcap_json_flag() {
        let cmd = TestCli::parse_from(["pcap", "--json"]);
        assert!(cmd.command.json);
    }

    #[test]
    fn builds_pcap_json_summary_with_filters() {
        let summary = PcapSummary::new(
            &PathBuf::from("dump.pcap"),
            7,
            &PacketFilter {
                pid: Some(123),
                process_prefix: Some("WebKit".to_string()),
            },
            PcapSummaryCaptureMetadata {
                interfaces: vec!["en0".to_string()],
                processes: vec!["WebKit".to_string()],
                pids: vec![123],
                first_timestamp: Some(PcapSummaryTimestamp { sec: 10, usec: 20 }),
                last_timestamp: Some(PcapSummaryTimestamp { sec: 11, usec: 30 }),
                filter_hit: Some(true),
            },
        );

        assert_eq!(
            serde_json::to_value(&summary).expect("summary serializes"),
            serde_json::json!({
                "output": "dump.pcap",
                "written_packets": 7,
                "filters": {
                    "pid": 123,
                    "process_prefix": "WebKit",
                },
                "capture_metadata": {
                    "interfaces": ["en0"],
                    "processes": ["WebKit"],
                    "pids": [123],
                    "first_timestamp": {
                        "sec": 10,
                        "usec": 20
                    },
                    "last_timestamp": {
                        "sec": 11,
                        "usec": 30
                    },
                    "filter_hit": true
                }
            })
        );
    }

    #[test]
    fn builds_pcap_json_summary_with_empty_filters() {
        let summary = PcapSummary::new(
            &PathBuf::from("capture.pcap"),
            0,
            &PacketFilter {
                pid: None,
                process_prefix: None,
            },
            PcapSummaryCaptureMetadata::default(),
        );

        assert_eq!(
            summary,
            PcapSummary {
                output: "capture.pcap".to_string(),
                written_packets: 0,
                filters: PcapSummaryFilters {
                    pid: None,
                    process_prefix: None,
                },
                capture_metadata: PcapSummaryCaptureMetadata::default(),
            }
        );
        assert_eq!(
            serde_json::to_value(&summary).expect("summary serializes"),
            serde_json::json!({
                "output": "capture.pcap",
                "written_packets": 0,
                "filters": {
                    "pid": null,
                    "process_prefix": null,
                },
                "capture_metadata": {
                    "interfaces": [],
                    "processes": [],
                    "pids": [],
                    "first_timestamp": null,
                    "last_timestamp": null,
                    "filter_hit": null
                }
            })
        );
    }

    #[test]
    fn collects_summary_metadata_from_written_packets() {
        let filter = PacketFilter {
            pid: Some(222),
            process_prefix: Some("Web".into()),
        };
        let mut metadata = PcapSummaryCaptureMetadataBuilder::new(&filter);

        metadata.record(&CapturedPacket {
            ts_sec: 20,
            ts_usec: 10,
            interface_name: "en1".into(),
            pid: 111,
            pid2: 222,
            proc_name: "Safari".into(),
            proc_name2: "WebKit.Networking".into(),
            payload: vec![1],
        });
        metadata.record(&CapturedPacket {
            ts_sec: 19,
            ts_usec: 999,
            interface_name: "en0".into(),
            pid: 222,
            pid2: 333,
            proc_name: "WebKit".into(),
            proc_name2: "".into(),
            payload: vec![2],
        });

        assert_eq!(
            metadata.finish(),
            PcapSummaryCaptureMetadata {
                interfaces: vec!["en0".into(), "en1".into()],
                processes: vec!["Safari".into(), "WebKit".into(), "WebKit.Networking".into()],
                pids: vec![111, 222, 333],
                first_timestamp: Some(PcapSummaryTimestamp { sec: 19, usec: 999 }),
                last_timestamp: Some(PcapSummaryTimestamp { sec: 20, usec: 10 }),
                filter_hit: Some(true),
            }
        );
    }

    #[test]
    fn reports_filter_miss_without_written_packets() {
        let filter = PacketFilter {
            pid: Some(999),
            process_prefix: None,
        };

        assert_eq!(
            PcapSummaryCaptureMetadataBuilder::new(&filter).finish(),
            PcapSummaryCaptureMetadata {
                interfaces: Vec::new(),
                processes: Vec::new(),
                pids: Vec::new(),
                first_timestamp: None,
                last_timestamp: None,
                filter_hit: Some(false),
            }
        );
    }

    #[test]
    fn json_flag_is_local_to_pcap_command() {
        let cmd = TestCli::parse_from(["pcap", "--json"]);
        assert!(cmd.command.json);
        assert_eq!(cmd.command.output, PathBuf::from("capture.pcap"));
    }

    #[test]
    fn defaults_to_no_packet_filter() {
        let cmd = TestCli::parse_from(["pcap"]);
        assert_eq!(cmd.command.pid, None);
        assert_eq!(cmd.command.process, None);
    }
}
