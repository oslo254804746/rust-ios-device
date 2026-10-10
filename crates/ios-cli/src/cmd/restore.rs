use std::time::Duration;

use anyhow::Result;
use ios_core::TunMode;
use ios_core::{connect, ConnectOptions};

#[derive(clap::Args)]
pub struct RestoreCmd {
    #[command(subcommand)]
    sub: RestoreSub,
}

#[derive(clap::Subcommand)]
enum RestoreSub {
    /// Reboot the device into recovery mode
    EnterRecovery {
        #[arg(long, help = "Required confirmation for entering recovery mode")]
        force: bool,
    },
    /// Set delay-recovery-image for supported devices
    DelayRecoveryImage {
        #[arg(
            long,
            help = "Required confirmation for changing the recovery image delay"
        )]
        force: bool,
    },
    /// Reboot the device via restore service
    Reboot {
        #[arg(long, help = "Required confirmation for the reboot")]
        force: bool,
    },
    /// Query restore preflight info
    PreflightInfo,
    /// Query AP/SEP nonces from restore service
    Nonces,
    /// Query restore app parameters
    AppParameters,
    /// Set the restore language
    RestoreLang {
        /// Language identifier to request from restore service
        language: String,
    },
    /// Read restore lifecycle events without starting a destructive restore
    Events {
        /// Maximum number of lifecycle events to read
        #[arg(long)]
        count: Option<usize>,
        /// Stop waiting when no event arrives before this timeout
        #[arg(long, default_value_t = 300)]
        timeout_secs: u64,
    },
    /// Ask the device to build TSS requests of each peripheral updater for an IPSW
    PreflightRequests {
        /// Path to local IPSW ZIP archive
        #[arg(long)]
        ipsw: std::path::PathBuf,
        /// Restore behavior: Update (default) or Erase
        #[arg(long, default_value = "Update")]
        behavior: String,
        /// Specific updaters to query (e.g. T200, Rose, SE, Vinyl, Savage, Centauri). Defaults to all peripheral updaters except Baseband
        #[arg(long)]
        updater: Vec<String>,
        /// Explicit ApBoardID (hex or decimal) override if not querying device
        #[arg(long)]
        board_id: Option<String>,
        /// Explicit ApChipID (hex or decimal) override if not querying device
        #[arg(long)]
        chip_id: Option<String>,
        /// Explicit DeviceClass override (e.g. d63ap, iphone14,3) if not querying device
        #[arg(long)]
        device_class: Option<String>,
        /// Timeout in seconds per updater request
        #[arg(long, default_value_t = 30)]
        timeout_secs: u64,
    },
    /// Experimental restore update execution state machine
    Update {
        /// Path to local IPSW ZIP archive
        #[arg(long)]
        ipsw: std::path::PathBuf,
        /// Optional path to saved TSS / SHSH blob plist
        #[arg(long)]
        tss: Option<std::path::PathBuf>,
        /// Restore behavior: Update (default) or Erase
        #[arg(long, default_value = "Update")]
        behavior: String,
        /// Required confirmation for running restore update
        #[arg(long)]
        force: bool,
        /// Dry run mode: validates IPSW identity, nonces, and preflight without modifying device
        #[arg(long)]
        dry_run: bool,
        /// Overall timeout in seconds
        #[arg(long, default_value_t = 600)]
        timeout_secs: u64,
    },
}

impl RestoreCmd {
    pub async fn run(self, udid: Option<String>, json: bool) -> Result<()> {
        let udid = udid.ok_or_else(|| anyhow::anyhow!("--udid required for restore"))?;
        // Guard before connecting: these three change device state, unlike the
        // read-only queries below.
        match &self.sub {
            RestoreSub::EnterRecovery { force } => crate::output::require_force(
                *force,
                "put the device into recovery mode",
                "the device leaves normal operation until it is restored or force-restarted",
            )?,
            RestoreSub::DelayRecoveryImage { force } => crate::output::require_force(
                *force,
                "set delay-recovery-image",
                "this changes how the device boots into recovery",
            )?,
            RestoreSub::Reboot { force } => crate::output::require_force(
                *force,
                "reboot the device",
                "the device restarts immediately and any running session is lost",
            )?,
            RestoreSub::Update { force, dry_run, .. } => {
                if !*dry_run {
                    crate::output::require_force(
                        *force,
                        "execute experimental restore update",
                        "this operation attempts to modify device firmware",
                    )?;
                }
            }
            _ => {}
        }

        let success_message = match &self.sub {
            RestoreSub::EnterRecovery { .. } => Some("Recovery request accepted."),
            RestoreSub::DelayRecoveryImage { .. } => Some("Delay recovery image request accepted."),
            RestoreSub::Reboot { .. } => Some("Reboot request accepted."),
            _ => None,
        };
        let device = connect(
            &udid,
            ConnectOptions {
                tun_mode: TunMode::Userspace,
                pair_record_path: None,
                skip_tunnel: false,
            },
        )
        .await?;

        match self.sub {
            RestoreSub::PreflightRequests {
                ipsw,
                behavior,
                updater,
                board_id,
                chip_id,
                device_class,
                timeout_secs,
            } => {
                return handle_preflight_requests(
                    &device,
                    ipsw,
                    behavior,
                    updater,
                    board_id,
                    chip_id,
                    device_class,
                    timeout_secs,
                    json,
                )
                .await;
            }
            RestoreSub::Update {
                ipsw,
                tss,
                behavior,
                force: _,
                dry_run,
                timeout_secs,
            } => {
                return handle_update(&device, ipsw, tss, behavior, dry_run, timeout_secs, json)
                    .await;
            }
            _ => {}
        }

        let stream = device
            .connect_rsd_service(ios_core::restore::SERVICE_NAME)
            .await?;
        let mut client = ios_core::restore::RestoreServiceClient::connect(stream).await?;

        if let RestoreSub::Events {
            count,
            timeout_secs,
        } = self.sub
        {
            let events = read_restore_events(&mut client, count, timeout_secs).await?;
            let rendered: Vec<_> = events
                .iter()
                .map(ios_core::restore::restore_lifecycle_event_to_json)
                .collect();
            if json {
                crate::output::outln!("{}", serde_json::to_string_pretty(&rendered)?);
            } else {
                for event in rendered {
                    crate::output::outln!("{}", serde_json::to_string_pretty(&event)?);
                }
            }
            return Ok(());
        }

        let response = match self.sub {
            RestoreSub::EnterRecovery { .. } => client.enter_recovery().await?,
            RestoreSub::DelayRecoveryImage { .. } => client.delay_recovery_image().await?,
            RestoreSub::Reboot { .. } => client.reboot().await?,
            RestoreSub::PreflightInfo => client.get_preflight_info().await?,
            RestoreSub::Nonces => client.get_nonces().await?,
            RestoreSub::AppParameters => client.get_app_parameters().await?,
            RestoreSub::RestoreLang { language } => client.restore_lang(language).await?,
            RestoreSub::Events { .. }
            | RestoreSub::PreflightRequests { .. }
            | RestoreSub::Update { .. } => unreachable!("handled above"),
        };

        let rendered =
            ios_core::restore::xpc_value_to_json(&ios_core::XpcValue::Dictionary(response.clone()));
        if json {
            crate::output::outln!("{}", serde_json::to_string_pretty(&rendered)?);
        } else if let Some(message) = success_message {
            crate::output::outln!("{message}");
        } else {
            crate::output::outln!("{}", serde_json::to_string_pretty(&rendered)?);
        }

        Ok(())
    }
}

fn parse_id_u64(s: &str) -> Result<u64> {
    let s = s.trim();
    if let Some(hex) = s.strip_prefix("0x").or_else(|| s.strip_prefix("0X")) {
        u64::from_str_radix(hex, 16).map_err(|e| anyhow::anyhow!("invalid hex integer {s}: {e}"))
    } else {
        s.parse::<u64>()
            .map_err(|e| anyhow::anyhow!("invalid integer {s}: {e}"))
    }
}

fn parse_plist_u64(val: &plist::Value) -> Option<u64> {
    match val {
        plist::Value::Integer(i) => i.as_unsigned(),
        plist::Value::String(s) => {
            let s = s.trim();
            if let Some(hex) = s.strip_prefix("0x").or_else(|| s.strip_prefix("0X")) {
                u64::from_str_radix(hex, 16).ok()
            } else {
                s.parse::<u64>().ok()
            }
        }
        _ => None,
    }
}

async fn read_lockdown_fallback(
    device: &ios_core::ConnectedDevice,
    primary: &str,
    secondary: &str,
) -> Result<plist::Value> {
    match device.lockdown_get_value(Some(primary)).await {
        Ok(val) => Ok(val),
        Err(_) => device
            .lockdown_get_value(Some(secondary))
            .await
            .map_err(|e| {
                anyhow::anyhow!("failed to read {primary} or {secondary} from device: {e}")
            }),
    }
}

#[allow(clippy::too_many_arguments)]
async fn handle_preflight_requests(
    device: &ios_core::ConnectedDevice,
    ipsw: std::path::PathBuf,
    behavior: String,
    updater: Vec<String>,
    board_id_opt: Option<String>,
    chip_id_opt: Option<String>,
    device_class_opt: Option<String>,
    timeout_secs: u64,
    json: bool,
) -> Result<()> {
    let board_id = match board_id_opt {
        Some(s) => parse_id_u64(&s)?,
        None => {
            let val = read_lockdown_fallback(device, "BoardId", "ApBoardID").await?;
            parse_plist_u64(&val)
                .ok_or_else(|| anyhow::anyhow!("device returned non-numeric BoardId"))?
        }
    };
    let chip_id = match chip_id_opt {
        Some(s) => parse_id_u64(&s)?,
        None => {
            let val = read_lockdown_fallback(device, "ChipID", "ApChipID").await?;
            parse_plist_u64(&val)
                .ok_or_else(|| anyhow::anyhow!("device returned non-numeric ChipID"))?
        }
    };
    let device_class = match device_class_opt {
        Some(s) => s,
        None => {
            let val = read_lockdown_fallback(device, "HardwareModel", "DeviceClass").await?;
            val.as_string()
                .map(|s| s.to_ascii_lowercase())
                .ok_or_else(|| anyhow::anyhow!("device returned non-string HardwareModel"))?
        }
    };

    let file = std::fs::File::open(&ipsw)
        .map_err(|e| anyhow::anyhow!("failed to open IPSW at {}: {e}", ipsw.display()))?;
    let selector = ios_core::restore::preflight::IpswIdentitySelector {
        board_id,
        chip_id,
        device_class: &device_class,
        restore_behavior: &behavior,
    };
    let mut ipsw_preflight = ios_core::restore::preflight::IpswPreflight::open(file, selector)?;

    let updaters_list: Vec<&str> = if updater.is_empty() {
        ios_core::restore::preflight::DEFAULT_PREFLIGHT_UPDATERS.to_vec()
    } else {
        updater.iter().map(|s| s.as_str()).collect()
    };

    let collected = ios_core::restore::preflight::collect_device_side_preflight_requests(
        &mut ipsw_preflight,
        &updaters_list,
        || async {
            let stream = device
                .connect_rsd_service(ios_core::restore::SERVICE_NAME)
                .await
                .map_err(|e| ios_core::restore::RestoreError::Protocol(e.to_string()))?;
            ios_core::restore::RestoreServiceClient::connect(stream).await
        },
        Duration::from_secs(timeout_secs),
    )
    .await?;

    if json {
        crate::output::outln!("{}", serde_json::to_string_pretty(&collected)?);
    } else {
        crate::output::outln!("Preflight Requests Collection Summary:");
        crate::output::outln!(
            "  Identity: board={:#x}, chip={:#x}, class={}",
            board_id,
            chip_id,
            device_class
        );
        crate::output::outln!("  Queried updaters: {}", updaters_list.join(", "));
        crate::output::outln!(
            "  Successful requests: {}",
            collected.device_info_requests.len()
        );
        crate::output::outln!("  Failed updaters: {}", collected.failed_updaters.len());
        for failure in &collected.failed_updaters {
            crate::output::outln!("    - {}: {}", failure.updater, failure.error);
        }
        if let Some(ref ap) = collected.ap_parameters {
            crate::output::outln!("  ApParameters: {}", serde_json::to_string(ap)?);
        }
    }
    Ok(())
}

async fn handle_update(
    device: &ios_core::ConnectedDevice,
    ipsw: std::path::PathBuf,
    tss: Option<std::path::PathBuf>,
    behavior: String,
    dry_run: bool,
    timeout_secs: u64,
    json: bool,
) -> Result<()> {
    let file = std::fs::File::open(&ipsw)
        .map_err(|e| anyhow::anyhow!("failed to open IPSW at {}: {e}", ipsw.display()))?;

    let board_id_val = read_lockdown_fallback(device, "BoardId", "ApBoardID").await?;
    let board_id =
        parse_plist_u64(&board_id_val).ok_or_else(|| anyhow::anyhow!("non-numeric BoardId"))?;
    let chip_id_val = read_lockdown_fallback(device, "ChipID", "ApChipID").await?;
    let chip_id =
        parse_plist_u64(&chip_id_val).ok_or_else(|| anyhow::anyhow!("non-numeric ChipID"))?;
    let model_val = read_lockdown_fallback(device, "HardwareModel", "DeviceClass").await?;
    let device_class = model_val
        .as_string()
        .map(|s| s.to_ascii_lowercase())
        .ok_or_else(|| anyhow::anyhow!("non-string HardwareModel"))?;

    let selector = ios_core::restore::preflight::IpswIdentitySelector {
        board_id,
        chip_id,
        device_class: &device_class,
        restore_behavior: &behavior,
    };
    let _ipsw_preflight = ios_core::restore::preflight::IpswPreflight::open(file, selector)?;

    let mut sm = ios_core::restore::state_machine::RestoreStateMachine::new();
    sm.start_mode_transition()
        .map_err(|e| anyhow::anyhow!("{e}"))?;

    if dry_run {
        let stream = device
            .connect_rsd_service(ios_core::restore::SERVICE_NAME)
            .await?;
        let mut client = ios_core::restore::RestoreServiceClient::connect(stream).await?;
        let nonces = client.get_nonces().await?;
        let ap_nonce = nonces.get("ApNonce").and_then(|v| match v {
            ios_core::XpcValue::Data(b) => Some(b.to_vec()),
            _ => None,
        });
        let sep_nonce = nonces.get("SEPNonce").and_then(|v| match v {
            ios_core::XpcValue::Data(b) => Some(b.to_vec()),
            _ => None,
        });
        sm.transition_to_recovery()
            .map_err(|e| anyhow::anyhow!("{e}"))?;
        sm.receive_nonces(ap_nonce, sep_nonce)
            .map_err(|e| anyhow::anyhow!("{e}"))?;
        if let Some(ref tss_path) = tss {
            let tss_bytes = std::fs::read(tss_path)?;
            sm.store_tss_ticket("AP", tss_bytes, None)
                .map_err(|e| anyhow::anyhow!("{e}"))?;
        }
        let report = sm.generate_report();
        if json {
            crate::output::outln!("{}", serde_json::to_string_pretty(&report)?);
        } else {
            crate::output::outln!("Experimental Restore Update (Dry Run) Validation:");
            crate::output::outln!(
                "  Identity: board={:#x}, chip={:#x}, class={}",
                board_id,
                chip_id,
                device_class
            );
            crate::output::outln!("  Final state: {:?}", report.final_state);
            crate::output::outln!("  Protocol Gaps:");
            crate::output::outln!(
                "    - TSS Live Signing: {}",
                report.protocol_gaps.tss_live_signing
            );
            crate::output::outln!(
                "    - Erase FDR Proxy: {}",
                report.protocol_gaps.erase_fdr_proxy
            );
            crate::output::outln!(
                "    - DFU Bootstrap: {}",
                report.protocol_gaps.dfu_bootstrap
            );
            crate::output::outln!(
                "    - Wireless Tunnel: {}",
                report.protocol_gaps.wireless_tunnel_drop
            );
            crate::output::outln!(
                "    - Unknown Chipsets: {}",
                report.protocol_gaps.unknown_chipset_prefixes
            );
        }
        return Ok(());
    }

    let stream = device
        .connect_rsd_service(ios_core::restore::SERVICE_NAME)
        .await?;
    let mut client = ios_core::restore::RestoreServiceClient::connect(stream).await?;
    sm.transition_to_recovery()
        .map_err(|e| anyhow::anyhow!("{e}"))?;
    let report = ios_core::restore::state_machine::run_experimental_restore_loop(
        &mut client,
        sm,
        |_dt, _port| Ok(Vec::new()),
        Duration::from_secs(timeout_secs),
        None,
    )
    .await
    .map_err(|e| anyhow::anyhow!("{e}"))?;

    if json {
        crate::output::outln!("{}", serde_json::to_string_pretty(&report)?);
    } else {
        crate::output::outln!(
            "Experimental Restore Execution Finished: {:?}",
            report.final_state
        );
        crate::output::outln!("  Checkpoints: {:?}", report.checkpoints_reached);
        crate::output::outln!("  Served Data Requests: {:?}", report.data_requests_served);
    }
    Ok(())
}

async fn read_restore_events<S>(
    client: &mut ios_core::restore::RestoreServiceClient<S>,
    count: Option<usize>,
    timeout_secs: u64,
) -> Result<Vec<ios_core::restore::RestoreLifecycleEvent>>
where
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin,
{
    let mut events = Vec::new();
    loop {
        if count.is_some_and(|count| events.len() >= count) {
            break;
        }

        let event = tokio::time::timeout(
            Duration::from_secs(timeout_secs),
            client.next_lifecycle_event(),
        )
        .await
        .map_err(|error| {
            anyhow::Error::new(error).context("timed out waiting for restore lifecycle event")
        })??;
        let finished = matches!(
            event,
            ios_core::restore::RestoreLifecycleEvent::Status { finished: true, .. }
        );
        events.push(event);
        if finished {
            break;
        }
    }
    Ok(events)
}

#[cfg(test)]
mod tests {
    use clap::Parser;

    use super::*;

    #[derive(Parser)]
    struct TestCli {
        #[command(flatten)]
        command: RestoreCmd,
    }

    #[test]
    fn parses_restore_enter_recovery_subcommand() {
        let parsed = TestCli::try_parse_from(["restore", "enter-recovery"]);
        assert!(parsed.is_ok(), "restore enter-recovery should parse");
    }

    #[test]
    fn parses_restore_preflight_info_subcommand() {
        let parsed = TestCli::try_parse_from(["restore", "preflight-info"]);
        assert!(parsed.is_ok(), "restore preflight-info should parse");
    }

    #[test]
    fn parses_restore_delay_recovery_image_subcommand() {
        let parsed = TestCli::try_parse_from(["restore", "delay-recovery-image"]);
        assert!(parsed.is_ok(), "restore delay-recovery-image should parse");
    }

    #[test]
    fn parses_restore_reboot_subcommand() {
        let parsed = TestCli::try_parse_from(["restore", "reboot"]);
        assert!(parsed.is_ok(), "restore reboot should parse");
    }

    #[test]
    fn parses_restore_app_parameters_subcommand() {
        let parsed = TestCli::try_parse_from(["restore", "app-parameters"]);
        assert!(parsed.is_ok(), "restore app-parameters should parse");
    }

    #[test]
    fn parses_restore_lang_subcommand() {
        let parsed = TestCli::try_parse_from(["restore", "restore-lang", "en"]);
        assert!(parsed.is_ok(), "restore restore-lang should parse");
    }

    #[test]
    fn parses_restore_events_subcommand() {
        let parsed =
            TestCli::try_parse_from(["restore", "events", "--count", "2", "--timeout-secs", "30"]);
        assert!(parsed.is_ok(), "restore events should parse");
    }

    #[test]
    fn parses_restore_preflight_requests_subcommand() {
        let parsed = TestCli::try_parse_from([
            "restore",
            "preflight-requests",
            "--ipsw",
            "dummy.ipsw",
            "--updater",
            "Rose",
            "--board-id",
            "0x10",
            "--chip-id",
            "32",
            "--device-class",
            "d63ap",
        ]);
        assert!(parsed.is_ok(), "restore preflight-requests should parse");
    }

    #[test]
    fn parses_restore_update_dry_run_subcommand() {
        let parsed =
            TestCli::try_parse_from(["restore", "update", "--ipsw", "dummy.ipsw", "--dry-run"]);
        assert!(parsed.is_ok(), "restore update --dry-run should parse");
    }
}
