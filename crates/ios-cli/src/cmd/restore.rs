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
        /// Total deadline in seconds, including connection and all updater requests
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
        /// Read-only inspection: identity, ZIP components and optional saved TSS metadata; no restore execution
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
                    return Err(ios_core::CoreError::Unsupported("Update execution has no proven device/build path: recovery ramdisk, USB re-enumeration, IMG4 personalization, ASR/FDR and data-port owners are required; no device IO was made".into()).into());
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
        let total = match &self.sub {
            RestoreSub::Update { timeout_secs, .. }
            | RestoreSub::PreflightRequests { timeout_secs, .. } => Some(*timeout_secs),
            _ => None,
        };
        let deadline = total
            .map(|secs| {
                anyhow::ensure!(secs > 0, "timeout must be positive");
                tokio::time::Instant::now()
                    .checked_add(Duration::from_secs(secs))
                    .ok_or_else(|| anyhow::anyhow!("restore deadline overflow"))
            })
            .transpose()?;
        let connection = connect(
            &udid,
            ConnectOptions {
                tun_mode: TunMode::Userspace,
                pair_record_path: None,
                skip_tunnel: false,
            },
        );
        let device = if let Some(deadline) = deadline {
            tokio::select! {
                biased;
                _ = tokio::signal::ctrl_c() => return Err(std::io::Error::other(ios_core::OperationCancelled::new()).into()),
                result = tokio::time::timeout_at(deadline, connection) => result??,
            }
        } else {
            connection.await?
        };

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
                let operation = handle_preflight_requests(
                    &device,
                    ipsw,
                    behavior,
                    updater,
                    board_id,
                    chip_id,
                    device_class,
                    timeout_secs,
                    json,
                );
                return tokio::select! {
                    biased;
                    _ = tokio::signal::ctrl_c() => Err(std::io::Error::other(ios_core::OperationCancelled::new()).into()),
                    result = tokio::time::timeout_at(deadline.expect("preflight deadline"), operation) => result?,
                };
            }
            RestoreSub::Update {
                ipsw,
                tss,
                behavior,
                force: _,
                dry_run,
                timeout_secs,
            } => {
                let operation =
                    handle_update(&device, ipsw, tss, behavior, dry_run, timeout_secs, json);
                return tokio::select! {
                    biased;
                    _ = tokio::signal::ctrl_c() => Err(std::io::Error::other(ios_core::OperationCancelled::new()).into()),
                    result = tokio::time::timeout_at(deadline.expect("update inspection deadline"), operation) => result?,
                };
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
        Err(primary_error) => {
            device
                .lockdown_get_value(Some(secondary))
                .await
                .map_err(|fallback_error| {
                    anyhow::Error::new(IdentityQueryFailure {
                        primary: primary_error,
                        fallback: fallback_error,
                    })
                })
        }
    }
}

struct IdentityQueryFailure {
    primary: ios_core::CoreError,
    fallback: ios_core::CoreError,
}
impl std::fmt::Display for IdentityQueryFailure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("both device identity queries failed")
    }
}
impl std::fmt::Debug for IdentityQueryFailure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("IdentityQueryFailure")
            .field("primary_class", &self.primary.class())
            .field("fallback_class", &self.fallback.class())
            .finish_non_exhaustive()
    }
}
impl std::error::Error for IdentityQueryFailure {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        Some(&self.fallback)
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

    let outcome = ios_core::restore::preflight::collect_device_side_preflight_outcomes(
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
    let collected = &outcome.summary;

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
    if !outcome.failures.is_empty() {
        return Err(anyhow::Error::new(outcome));
    }
    anyhow::ensure!(
        collected.device_info_failures.is_empty(),
        "device reported updater failures; partial results emitted"
    );
    Ok(())
}

async fn handle_update(
    device: &ios_core::ConnectedDevice,
    ipsw: std::path::PathBuf,
    tss: Option<std::path::PathBuf>,
    behavior: String,
    dry_run: bool,
    _timeout_secs: u64,
    json: bool,
) -> Result<()> {
    anyhow::ensure!(dry_run, "restore execution has no verified Update path");
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
    let mut ipsw_preflight = ios_core::restore::preflight::IpswPreflight::open(file, selector)?;
    let components = ipsw_preflight.validate_components(32 * 1024 * 1024 * 1024)?;
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
    let ap_nonce = ap_nonce.ok_or_else(|| anyhow::anyhow!("device did not provide AP nonce"))?;
    let ecid_val = device.lockdown_get_value(Some("UniqueChipID")).await?;
    let ecid = parse_plist_u64(&ecid_val)
        .ok_or_else(|| anyhow::anyhow!("device returned non-numeric ECID"))?;
    let mut tss_correlated = false;
    if let Some(ref tss_path) = tss {
        use std::io::Read;
        let tss_file = std::fs::File::open(tss_path)?;
        anyhow::ensure!(
            tss_file.metadata()?.is_file(),
            "saved TSS must be a regular file"
        );
        let mut bytes = Vec::new();
        tss_file
            .take(16 * 1024 * 1024 + 1)
            .read_to_end(&mut bytes)?;
        anyhow::ensure!(
            bytes.len() <= 16 * 1024 * 1024,
            "saved TSS exceeds byte budget"
        );
        let saved: plist::Value = plist::from_bytes(&bytes)?;
        ios_core::restore::preflight::inspect_saved_tss(
            &saved,
            ecid,
            board_id,
            chip_id,
            &ap_nonce,
            sep_nonce.as_deref(),
        )?;
        tss_correlated = true;
    }
    let report = serde_json::json!({
        "operation": "restore_update_inspection", "device_mode_changed": false,
        "restore_started": false, "firmware_bytes_transferred": 0,
        "identity": { "board_id": board_id, "chip_id": chip_id, "device_class": device_class, "ecid": ecid },
        "components": components, "archive_crc_checked": true,
        "saved_tss_metadata_correlated": tss_correlated,
        "apple_ticket_signature_verified": false,
        "execution_supported": false,
    });
    if json {
        crate::output::outln!("{}", serde_json::to_string_pretty(&report)?);
    } else {
        crate::output::outln!("Restore Update read-only inspection:");
        crate::output::outln!(
            "  Identity: board={:#x}, chip={:#x}, class={}",
            board_id,
            chip_id,
            device_class
        );
        crate::output::outln!(
            "  ZIP components and CRC checked; saved TSS metadata correlated: {tss_correlated}"
        );
        crate::output::outln!(
            "  Apple ticket signature verification and restore execution remain unsupported."
        );
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
