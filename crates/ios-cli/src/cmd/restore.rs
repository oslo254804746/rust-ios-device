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
        /// Explicit hardware DeviceClass override (e.g. d63ap) if not querying device
        #[arg(long)]
        device_class: Option<String>,
        /// Total deadline in seconds, including connection and all updater requests
        #[arg(long, default_value_t = 30)]
        timeout_secs: u64,
    },
    /// Read-only restore Update inspection; execution is not implemented
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
    /// Run a controlled local restore protocol simulation (IMG4 personalization + Recovery/Ramdisk handoff + restored/ASR/TSS peer) without touching physical hardware
    SimulateRestore {
        /// Path to local IPSW ZIP archive
        #[arg(long)]
        ipsw: std::path::PathBuf,
        /// Path to saved TSS / SHSH blob plist
        #[arg(long)]
        tss: std::path::PathBuf,
        /// Restore behavior: Update (default) or Erase
        #[arg(long, default_value = "Update")]
        behavior: String,
        /// Explicit ApBoardID (hex or decimal)
        #[arg(long, default_value = "0x0c")]
        board_id: String,
        /// Explicit ApChipID (hex or decimal)
        #[arg(long, default_value = "0x8110")]
        chip_id: String,
        /// Explicit hardware DeviceClass
        #[arg(long, default_value = "d63ap")]
        device_class: String,
        /// Explicit ECID (hex or decimal)
        #[arg(long, default_value = "42")]
        ecid: String,
        /// Expected hex AP nonce
        #[arg(long, default_value = "01020304")]
        ap_nonce_hex: String,
        /// Required confirmation to run the local simulated restore session
        #[arg(long)]
        confirm_simulation: bool,
        /// Overall timeout in seconds
        #[arg(long, default_value_t = 60)]
        timeout_secs: u64,
    },
}

impl RestoreCmd {
    pub async fn run(self, udid: Option<String>, json: bool) -> Result<()> {
        if let RestoreSub::SimulateRestore {
            ipsw,
            tss,
            behavior,
            board_id,
            chip_id,
            device_class,
            ecid,
            ap_nonce_hex,
            confirm_simulation,
            timeout_secs,
        } = self.sub
        {
            crate::output::require_force(
                confirm_simulation,
                "run simulated restore pipeline",
                "simulation requires explicit --confirm-simulation and never touches physical hardware",
            )?;
            return handle_simulate_restore(
                ipsw,
                tss,
                behavior,
                board_id,
                chip_id,
                device_class,
                ecid,
                ap_nonce_hex,
                timeout_secs,
                json,
            )
            .await;
        }
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
            RestoreSub::Update {
                force,
                dry_run,
                tss,
                ..
            } if !*dry_run => {
                crate::output::require_force(
                    *force,
                    "execute experimental restore update",
                    "this operation attempts to modify device firmware",
                )?;
                ensure_update_prerequisites(*dry_run, tss.as_deref())?;
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
            | RestoreSub::Update { .. }
            | RestoreSub::SimulateRestore { .. } => unreachable!("handled above"),
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

async fn read_device_version(device: &ios_core::ConnectedDevice, key: &str) -> Result<String> {
    let value = device.lockdown_get_value(Some(key)).await?;
    value
        .as_string()
        .filter(|text| !text.trim().is_empty())
        .map(ToString::to_string)
        .ok_or_else(|| anyhow::anyhow!("device returned invalid {key}"))
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

    let target_product_version = Some(read_device_version(device, "ProductVersion").await?);
    let target_build_version = Some(read_device_version(device, "BuildVersion").await?);
    ipsw_preflight.verify_target_version(
        target_product_version.as_deref(),
        target_build_version.as_deref(),
    )?;

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
        if let Some(ver) = ipsw_preflight.product_version() {
            crate::output::outln!(
                "  IPSW target version: iOS {ver} (build {})",
                ipsw_preflight.product_build_version().unwrap_or("unknown")
            );
        }
        if let Some(ref ver) = target_product_version {
            crate::output::outln!(
                "  Device current version: iOS {ver} (build {})",
                target_build_version.as_deref().unwrap_or("unknown")
            );
        }
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
    ensure_update_prerequisites(dry_run, tss.as_deref())?;
    anyhow::ensure!(
        behavior == "Update",
        "only behavior 'Update' is supported by restore update; got '{behavior}'"
    );

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
    ipsw_preflight.verify_hardware_evidence(board_id, chip_id, &device_class)?;
    let components = ipsw_preflight.validate_components(32 * 1024 * 1024 * 1024)?;

    let target_product_version = Some(read_device_version(device, "ProductVersion").await?);
    let target_build_version = Some(read_device_version(device, "BuildVersion").await?);
    ipsw_preflight.verify_target_version(
        target_product_version.as_deref(),
        target_build_version.as_deref(),
    )?;

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
    if dry_run {
        let report = serde_json::json!({
            "operation": "restore_update_inspection", "device_mode_changed": false,
            "restore_started": false, "firmware_bytes_transferred": 0,
            "identity": {
                "board_id": board_id, "chip_id": chip_id, "device_class": device_class, "ecid": ecid,
                "target_version": target_product_version, "target_build": target_build_version,
                "ipsw_version": ipsw_preflight.product_version(),
                "ipsw_build": ipsw_preflight.product_build_version(),
            },
            "components": components, "archive_crc_checked": true,
            "saved_tss_metadata_correlated": tss_correlated,
            "apple_ticket_signature_verified": false,
            "execution_supported": false,
            "version_compatibility_verified": false,
            "update_downgrade_check": "product_version_only",
        });
        if json {
            crate::output::outln!("{}", serde_json::to_string_pretty(&report)?);
        } else {
            crate::output::outln!("Restore Update read-only inspection:");
            crate::output::outln!(
                "  Identity: board={:#x}, chip={:#x}, class={}, ecid={:#x}",
                board_id,
                chip_id,
                device_class,
                ecid
            );
            if let Some(ver) = ipsw_preflight.product_version() {
                crate::output::outln!(
                    "  IPSW target version: iOS {ver} (build {})",
                    ipsw_preflight.product_build_version().unwrap_or("unknown")
                );
            }
            if let Some(ref ver) = target_product_version {
                crate::output::outln!(
                    "  Device current version: iOS {ver} (build {})",
                    target_build_version.as_deref().unwrap_or("unknown")
                );
            }
            crate::output::outln!(
                "  ZIP components and CRC checked; saved TSS metadata correlated: {tss_correlated}"
            );
        }
        return Ok(());
    }

    ensure_update_prerequisites(false, tss.as_deref())
}

fn ensure_update_prerequisites(dry_run: bool, _tss: Option<&std::path::Path>) -> Result<()> {
    if !dry_run {
        return Err(std::io::Error::new(
            std::io::ErrorKind::Unsupported,
            "restore Update execution is not implemented: recovery ramdisk boot, USB re-identification, component personalization and required restore data services are missing; --tss metadata and --force do not enable execution; use --dry-run for read-only inspection",
        ).into());
    }
    Ok(())
}

#[cfg(test)]
fn ensure_update_inspection_only(dry_run: bool) -> Result<()> {
    ensure_update_prerequisites(dry_run, None)
}

#[allow(clippy::too_many_arguments)]
async fn handle_simulate_restore(
    ipsw: std::path::PathBuf,
    tss: std::path::PathBuf,
    behavior: String,
    board_id_str: String,
    chip_id_str: String,
    device_class: String,
    ecid_str: String,
    ap_nonce_hex: String,
    timeout_secs: u64,
    json: bool,
) -> Result<()> {
    use std::io::Read;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    anyhow::ensure!(timeout_secs > 0, "timeout must be positive");
    anyhow::ensure!(
        behavior == "Update",
        "only behavior 'Update' is supported by simulated restore; got '{behavior}'"
    );

    let board_id = parse_id_u64(&board_id_str)?;
    let chip_id = parse_id_u64(&chip_id_str)?;
    let ecid = parse_id_u64(&ecid_str)?;
    let ap_nonce = hex::decode(ap_nonce_hex.trim())
        .map_err(|e| anyhow::anyhow!("invalid hex --ap-nonce-hex: {e}"))?;
    anyhow::ensure!(!ap_nonce.is_empty(), "--ap-nonce-hex must not be empty");

    let ipsw_file = std::fs::File::open(&ipsw)
        .map_err(|e| anyhow::anyhow!("failed to open IPSW at {}: {e}", ipsw.display()))?;
    let selector = ios_core::restore::preflight::IpswIdentitySelector {
        board_id,
        chip_id,
        device_class: &device_class,
        restore_behavior: &behavior,
    };
    let mut ipsw_preflight =
        ios_core::restore::preflight::IpswPreflight::open(ipsw_file, selector)?;
    ipsw_preflight.verify_hardware_evidence(board_id, chip_id, &device_class)?;
    let components = ipsw_preflight.validate_components(32 * 1024 * 1024 * 1024)?;

    let tss_file = std::fs::File::open(&tss)
        .map_err(|e| anyhow::anyhow!("failed to open saved TSS at {}: {e}", tss.display()))?;
    anyhow::ensure!(
        tss_file.metadata()?.is_file(),
        "saved TSS must be a regular file"
    );
    let mut tss_bytes = Vec::new();
    tss_file
        .take(16 * 1024 * 1024 + 1)
        .read_to_end(&mut tss_bytes)?;
    anyhow::ensure!(
        tss_bytes.len() <= 16 * 1024 * 1024,
        "saved TSS exceeds byte budget"
    );
    let saved_tss: plist::Value = plist::from_bytes(&tss_bytes)?;
    ios_core::restore::preflight::inspect_saved_tss(
        &saved_tss, ecid, board_id, chip_id, &ap_nonce, None,
    )?;
    let ap_ticket = ios_core::restore::preflight::extract_saved_tss_ticket(&saved_tss)?;
    let tss_response_dict = saved_tss
        .as_dictionary()
        .and_then(|d| d.get("Response"))
        .and_then(plist::Value::as_dictionary)
        .cloned()
        .unwrap_or_default();

    let identity = ipsw_preflight.identity().clone();
    let ramdisk_plan = ios_core::restore::build_recovery_ramdisk_plan(
        ios_core::restore::RecoveryDeviceMode::NormalLockdown,
        ecid,
        &identity,
        &ap_ticket,
        Some(&tss_response_dict),
        Some(&ap_nonce),
        |comp_name, _path| ipsw_preflight.read_manifest_component(comp_name, 64 * 1024 * 1024),
    )?;

    let mut os_image_buf = Vec::new();
    let cancel = ios_core::session::OperationCancel::new();
    let _os_inspection = ipsw_preflight.extract_system_image(
        &mut os_image_buf,
        512 * 1024 * 1024,
        Duration::from_secs(timeout_secs),
        &cancel,
    )?;

    let (control_client_stream, mut control_peer_stream) = tokio::io::duplex(65536);
    let (asr_client_stream, mut asr_peer_stream) = tokio::io::duplex(65536);
    let (asr_done_tx, asr_done_rx) = tokio::sync::oneshot::channel();

    let expected_os_len = os_image_buf.len() as u64;
    let peer_task = tokio::spawn(async move {
        let write_frame = |w: &mut tokio::io::DuplexStream, dict: plist::Dictionary| {
            let mut buf = Vec::new();
            plist::to_writer_xml(&mut buf, &dict).unwrap();
            let mut out = (buf.len() as u32).to_be_bytes().to_vec();
            out.extend_from_slice(&buf);
            let _ = w;
            out
        };
        async fn read_frame(r: &mut tokio::io::DuplexStream) -> plist::Dictionary {
            use tokio::io::AsyncReadExt;
            let mut len_bytes = [0u8; 4];
            r.read_exact(&mut len_bytes).await.unwrap();
            let len = u32::from_be_bytes(len_bytes) as usize;
            let mut buf = vec![0u8; len];
            r.read_exact(&mut buf).await.unwrap();
            plist::from_bytes::<plist::Dictionary>(&buf).unwrap()
        }

        // 1. QueryType
        let q = read_frame(&mut control_peer_stream).await;
        assert_eq!(q["Request"].as_string(), Some("QueryType"));
        let reply = write_frame(
            &mut control_peer_stream,
            plist::Dictionary::from_iter([
                (
                    "Type",
                    plist::Value::String("com.apple.mobile.restored".into()),
                ),
                ("RestoreProtocolVersion", plist::Value::Integer(15.into())),
            ]),
        );
        control_peer_stream.write_all(&reply).await.unwrap();

        // 2. HardwareInfo
        let hw = read_frame(&mut control_peer_stream).await;
        assert_eq!(hw["QueryKey"].as_string(), Some("HardwareInfo"));
        let reply = write_frame(
            &mut control_peer_stream,
            plist::Dictionary::from_iter([(
                "HardwareInfo",
                plist::Value::Dictionary(plist::Dictionary::from_iter([(
                    "UniqueChipID",
                    plist::Value::Integer(ecid.into()),
                )])),
            )]),
        );
        control_peer_stream.write_all(&reply).await.unwrap();

        // 3. StartRestore
        let start = read_frame(&mut control_peer_stream).await;
        assert_eq!(start["Request"].as_string(), Some("StartRestore"));

        // 4. Request BuildIdentityDict, RootTicket, NORData, KernelCache, DeviceTree, FDRTrustData, SystemImageData
        for dt in [
            "BuildIdentityDict",
            "RootTicket",
            "NORData",
            "FDRTrustData",
            "KernelCache",
            "DeviceTree",
            "HostSystemTime",
        ] {
            let req = write_frame(
                &mut control_peer_stream,
                plist::Dictionary::from_iter([
                    ("MsgType", plist::Value::String("DataRequestMsg".into())),
                    ("DataType", plist::Value::String(dt.into())),
                ]),
            );
            control_peer_stream.write_all(&req).await.unwrap();
            let _resp = read_frame(&mut control_peer_stream).await;
        }

        // 5. SystemImageData via ASR
        let asr_req = write_frame(
            &mut control_peer_stream,
            plist::Dictionary::from_iter([
                (
                    "MsgType",
                    plist::Value::String("AsyncDataRequestMsg".into()),
                ),
                ("DataType", plist::Value::String("SystemImageData".into())),
                ("DataPort", plist::Value::Integer(12345.into())),
            ]),
        );
        control_peer_stream.write_all(&asr_req).await.unwrap();
        asr_done_rx.await.unwrap();

        // 6. StatusMsg(0) and wait for ReceivedFinalStatusMsg
        let done_msg = write_frame(
            &mut control_peer_stream,
            plist::Dictionary::from_iter([
                ("MsgType", plist::Value::String("StatusMsg".into())),
                ("Status", plist::Value::Integer(0.into())),
                (
                    "Log",
                    plist::Value::String("Simulated restore complete".into()),
                ),
            ]),
        );
        control_peer_stream.write_all(&done_msg).await.unwrap();
        let ack = read_frame(&mut control_peer_stream).await;
        assert_eq!(ack["MsgType"].as_string(), Some("ReceivedFinalStatusMsg"));
    });

    let asr_task = tokio::spawn(async move {
        asr_peer_stream
            .write_all(b"<?xml version=\"1.0\"?><plist version=\"1.0\"><dict><key>Command</key><string>Initiate</string></dict></plist>\n")
            .await
            .unwrap();
        let mut offer = Vec::new();
        while !offer.ends_with(b"</plist>\n") {
            offer.push(asr_peer_stream.read_u8().await.unwrap());
        }
        let offer_dict: plist::Dictionary = plist::from_bytes(&offer).unwrap();
        assert_eq!(
            offer_dict["Payload"].as_dictionary().unwrap()["Size"].as_unsigned_integer(),
            Some(expected_os_len)
        );
        asr_peer_stream
            .write_all(b"<?xml version=\"1.0\"?><plist version=\"1.0\"><dict><key>Command</key><string>Payload</string></dict></plist>\n")
            .await
            .unwrap();
        let mut received = vec![0u8; expected_os_len as usize];
        asr_peer_stream.read_exact(&mut received).await.unwrap();
        let _ = asr_done_tx.send(());
    });

    let mut transport = ios_core::restore::SimulatedRecoveryTransport::new(
        ios_core::restore::RecoveryDeviceMode::NormalLockdown,
        ecid,
        control_client_stream,
    );
    let (restored_stream, handoff_receipt) =
        ios_core::restore::execute_recovery_ramdisk_handoff(&ramdisk_plan, &mut transport).await?;

    let mut restored_client = ios_core::restore::RestoredClient::connect(
        restored_stream,
        ecid,
        Duration::from_secs(timeout_secs),
        cancel.clone(),
    )
    .await?;

    let mut sm = ios_core::restore::RestoreStateMachine::new();
    sm.start_mode_transition()?;
    sm.transition_to_recovery()?;
    sm.receive_nonces(Some(ap_nonce.clone()), None)?;

    let assets = ios_core::restore::RestoredProtocolAssets::new(
        identity,
        ap_ticket,
        tss_response_dict,
        Some(ap_nonce),
    );
    let mut os_cursor = std::io::Cursor::new(os_image_buf);
    let mut asr_opt = Some(asr_client_stream);

    let session_report = ios_core::restore::run_restored_protocol_session(
        &mut restored_client,
        sm,
        plist::Dictionary::from_iter([(
            "RestoreBehavior".to_owned(),
            plist::Value::String(behavior),
        )]),
        &assets,
        |comp_name| ipsw_preflight.read_manifest_component(comp_name, 64 * 1024 * 1024),
        |updater, _req| {
            Ok((
                plist::Dictionary::from_iter([(
                    "Updater".to_owned(),
                    plist::Value::String(updater.to_owned()),
                )]),
                b"SIMULATED_FW".to_vec(),
            ))
        },
        &mut os_cursor,
        |port| {
            assert_eq!(port, 12345);
            std::future::ready(Ok(asr_opt.take().unwrap()))
        },
        &cancel,
    )
    .await?;

    asr_task.await?;
    peer_task.await?;

    let output = serde_json::json!({
        "operation": "restore_simulate_update",
        "physical_device_touched": false,
        "components_validated": components.len(),
        "recovery_handoff": handoff_receipt,
        "restored_session": session_report,
    });
    if json {
        crate::output::outln!("{}", serde_json::to_string_pretty(&output)?);
    } else {
        crate::output::outln!(
            "Simulated Restore Session Completed (final_state={:?}, requests_served={})",
            session_report.final_state,
            session_report.data_requests_served.len()
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

    #[tokio::test]
    async fn review_regression_forced_update_rejected_before_discovery_or_file_access() {
        let cmd = RestoreCmd {
            sub: RestoreSub::Update {
                ipsw: "missing.ipsw".into(),
                tss: None,
                behavior: "Update".into(),
                force: true,
                dry_run: false,
                timeout_secs: 1,
            },
        };
        let error = cmd
            .run(Some("offline-selected-device".into()), true)
            .await
            .unwrap_err();
        assert_eq!(
            error.downcast_ref::<std::io::Error>().unwrap().kind(),
            std::io::ErrorKind::Unsupported
        );
        assert!(ensure_update_inspection_only(true).is_ok());
    }

    #[tokio::test]
    async fn review_regression_saved_tss_cannot_enable_unimplemented_update() {
        let cmd = RestoreCmd {
            sub: RestoreSub::Update {
                ipsw: "missing.ipsw".into(),
                tss: Some("missing-but-supplied.plist".into()),
                behavior: "Update".into(),
                force: true,
                dry_run: false,
                timeout_secs: 1,
            },
        };
        let error = cmd
            .run(Some("offline-selected-device".into()), true)
            .await
            .unwrap_err();
        assert_eq!(
            error.downcast_ref::<std::io::Error>().unwrap().kind(),
            std::io::ErrorKind::Unsupported
        );
        assert!(error.to_string().contains("recovery ramdisk"));
    }

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

    #[test]
    fn parses_restore_update_with_force_and_tss_subcommand() {
        let parsed = TestCli::try_parse_from([
            "restore",
            "update",
            "--ipsw",
            "dummy.ipsw",
            "--tss",
            "saved.shsh",
            "--force",
            "--timeout-secs",
            "120",
        ]);
        assert!(
            parsed.is_ok(),
            "restore update with --force and --tss should parse"
        );
    }

    #[test]
    fn test_update_prerequisites_validation() {
        assert!(ensure_update_prerequisites(true, None).is_ok());
        let err = ensure_update_prerequisites(false, None).unwrap_err();
        assert_eq!(
            err.downcast_ref::<std::io::Error>().unwrap().kind(),
            std::io::ErrorKind::Unsupported
        );
        // Saved metadata cannot supply the missing recovery executor.
        assert!(
            ensure_update_prerequisites(false, Some(std::path::Path::new("test.tss"))).is_err()
        );
    }

    async fn write_test_plist<W: tokio::io::AsyncWriteExt + Unpin>(
        w: &mut W,
        dict: &plist::Dictionary,
    ) {
        let mut buf = Vec::new();
        plist::to_writer_xml(&mut buf, dict).unwrap();
        w.write_all(&(buf.len() as u32).to_be_bytes())
            .await
            .unwrap();
        w.write_all(&buf).await.unwrap();
    }

    async fn read_test_plist<R: tokio::io::AsyncReadExt + Unpin>(r: &mut R) -> plist::Dictionary {
        let mut len_bytes = [0u8; 4];
        r.read_exact(&mut len_bytes).await.unwrap();
        let len = u32::from_be_bytes(len_bytes) as usize;
        let mut buf = vec![0u8; len];
        r.read_exact(&mut buf).await.unwrap();
        plist::from_bytes(&buf).unwrap()
    }

    #[tokio::test]
    async fn test_simulated_full_restore_update_workflow() {
        use tokio::io::AsyncReadExt;
        let (stream, mut peer) = tokio::io::duplex(32768);

        let remote = tokio::spawn(async move {
            // 1. QueryType handshake
            let q_type = read_test_plist(&mut peer).await;
            assert_eq!(q_type["Request"].as_string(), Some("QueryType"));
            write_test_plist(
                &mut peer,
                &plist::Dictionary::from_iter([
                    (
                        "Type".to_owned(),
                        plist::Value::String("com.apple.mobile.restored".into()),
                    ),
                    (
                        "RestoreProtocolVersion".to_owned(),
                        plist::Value::Integer(15.into()),
                    ),
                ]),
            )
            .await;

            // 2. HardwareInfo query
            let q_hw = read_test_plist(&mut peer).await;
            assert_eq!(q_hw["QueryKey"].as_string(), Some("HardwareInfo"));
            write_test_plist(
                &mut peer,
                &plist::Dictionary::from_iter([(
                    "HardwareInfo".to_owned(),
                    plist::Value::Dictionary(plist::Dictionary::from_iter([(
                        "UniqueChipID".to_owned(),
                        plist::Value::Integer(42.into()),
                    )])),
                )]),
            )
            .await;

            // 3. StartRestore
            let start = read_test_plist(&mut peer).await;
            assert_eq!(start["Request"].as_string(), Some("StartRestore"));
            assert_eq!(
                start["RestoreOptions"].as_dictionary().unwrap()["RestoreBehavior"].as_string(),
                Some("Update")
            );

            // 4. ProgressMsg
            write_test_plist(
                &mut peer,
                &plist::Dictionary::from_iter([
                    (
                        "MsgType".to_owned(),
                        plist::Value::String("ProgressMsg".into()),
                    ),
                    (
                        "Operation".to_owned(),
                        plist::Value::String("Updating baseband".into()),
                    ),
                    ("Progress".to_owned(), plist::Value::Integer(30.into())),
                ]),
            )
            .await;

            // 5. CheckpointMsg
            write_test_plist(
                &mut peer,
                &plist::Dictionary::from_iter([
                    (
                        "MsgType".to_owned(),
                        plist::Value::String("CheckpointMsg".into()),
                    ),
                    (
                        "Checkpoint".to_owned(),
                        plist::Value::String("CHECKPOINT_FLASH_START".into()),
                    ),
                ]),
            )
            .await;

            // 6. DataRequestMsg
            write_test_plist(
                &mut peer,
                &plist::Dictionary::from_iter([
                    (
                        "MsgType".to_owned(),
                        plist::Value::String("DataRequestMsg".into()),
                    ),
                    (
                        "DataType".to_owned(),
                        plist::Value::String("FirmwareUpdaterData".into()),
                    ),
                    (
                        "Arguments".to_owned(),
                        plist::Value::Dictionary(plist::Dictionary::from_iter([
                            (
                                "UpdaterName".to_owned(),
                                plist::Value::String("Rose".into()),
                            ),
                            ("DataChunkSize".to_owned(), plist::Value::Integer(2.into())),
                        ])),
                    ),
                ]),
            )
            .await;

            // Read firmware chunks from client
            let header = read_test_plist(&mut peer).await;
            assert_eq!(header["DataSize"].as_unsigned_integer(), Some(5));
            let mut got = Vec::new();
            loop {
                let chunk = read_test_plist(&mut peer).await;
                got.extend_from_slice(chunk["FirmwareResponseData"].as_data().unwrap());
                if chunk.get("DataDone").and_then(plist::Value::as_boolean) == Some(true) {
                    break;
                }
            }
            assert_eq!(got, b"12345");

            // 7. Final ProgressMsg
            write_test_plist(
                &mut peer,
                &plist::Dictionary::from_iter([
                    (
                        "MsgType".to_owned(),
                        plist::Value::String("ProgressMsg".into()),
                    ),
                    (
                        "Operation".to_owned(),
                        plist::Value::String("Restoration complete".into()),
                    ),
                    ("Progress".to_owned(), plist::Value::Integer(100.into())),
                ]),
            )
            .await;

            // 8. Terminal StatusMsg (0)
            write_test_plist(
                &mut peer,
                &plist::Dictionary::from_iter([
                    (
                        "MsgType".to_owned(),
                        plist::Value::String("StatusMsg".into()),
                    ),
                    ("Status".to_owned(), plist::Value::Integer(0.into())),
                    (
                        "Log".to_owned(),
                        plist::Value::String("Update applied successfully".into()),
                    ),
                ]),
            )
            .await;

            let mut eof = [0u8; 1];
            assert_eq!(peer.read(&mut eof).await.unwrap(), 0);
        });

        let cancel = ios_core::session::OperationCancel::new();
        let mut client = ios_core::restore::RestoredClient::connect(
            stream,
            42,
            Duration::from_secs(5),
            cancel.clone(),
        )
        .await
        .unwrap();

        let mut sm = ios_core::restore::RestoreStateMachine::new();
        sm.receive_nonces(Some(vec![1, 2, 3]), Some(vec![4, 5, 6]))
            .unwrap();

        let options = plist::Dictionary::from_iter([(
            "RestoreBehavior".to_owned(),
            plist::Value::String("Update".into()),
        )]);

        let report = ios_core::restore::run_restored_update_loop(
            &mut client,
            sm,
            options,
            |name, _req| {
                assert_eq!(name, "Rose");
                Ok((
                    plist::Dictionary::from_iter([(
                        "Ticket".to_owned(),
                        plist::Value::Data(vec![0xAA]),
                    )]),
                    b"12345".to_vec(),
                ))
            },
            &cancel,
        )
        .await
        .unwrap();

        assert_eq!(
            report.final_state,
            ios_core::restore::RestoreState::Completed
        );
        assert_eq!(report.progress_percent, Some(100));
        assert!(report
            .checkpoints_reached
            .contains(&"CHECKPOINT_FLASH_START".to_string()));
        assert_eq!(report.total_bytes_transferred, 5);
        assert_eq!(
            report.data_requests_served,
            vec!["FirmwareUpdaterData:Rose"]
        );

        drop(client);
        remote.await.unwrap();
    }

    #[tokio::test]
    async fn test_simulated_restore_update_device_status_error() {
        let (stream, mut peer) = tokio::io::duplex(32768);

        let remote = tokio::spawn(async move {
            let _q_type = read_test_plist(&mut peer).await;
            write_test_plist(
                &mut peer,
                &plist::Dictionary::from_iter([
                    (
                        "Type".to_owned(),
                        plist::Value::String("com.apple.mobile.restored".into()),
                    ),
                    (
                        "RestoreProtocolVersion".to_owned(),
                        plist::Value::Integer(15.into()),
                    ),
                ]),
            )
            .await;

            let _q_hw = read_test_plist(&mut peer).await;
            write_test_plist(
                &mut peer,
                &plist::Dictionary::from_iter([(
                    "HardwareInfo".to_owned(),
                    plist::Value::Dictionary(plist::Dictionary::from_iter([(
                        "UniqueChipID".to_owned(),
                        plist::Value::Integer(42.into()),
                    )])),
                )]),
            )
            .await;

            let _start = read_test_plist(&mut peer).await;

            // Restored returns Status 14 (failure)
            write_test_plist(
                &mut peer,
                &plist::Dictionary::from_iter([
                    (
                        "MsgType".to_owned(),
                        plist::Value::String("StatusMsg".into()),
                    ),
                    ("Status".to_owned(), plist::Value::Integer(14.into())),
                    (
                        "Log".to_owned(),
                        plist::Value::String("NAND write error".into()),
                    ),
                ]),
            )
            .await;
        });

        let cancel = ios_core::session::OperationCancel::new();
        let mut client = ios_core::restore::RestoredClient::connect(
            stream,
            42,
            Duration::from_secs(5),
            cancel.clone(),
        )
        .await
        .unwrap();

        let mut sm = ios_core::restore::RestoreStateMachine::new();
        sm.receive_nonces(Some(vec![1, 2, 3]), None).unwrap();

        let options = plist::Dictionary::from_iter([(
            "RestoreBehavior".to_owned(),
            plist::Value::String("Update".into()),
        )]);

        let err = ios_core::restore::run_restored_update_loop(
            &mut client,
            sm,
            options,
            |_name, _req| Ok((plist::Dictionary::new(), Vec::new())),
            &cancel,
        )
        .await
        .unwrap_err();

        let msg = err.to_string();
        assert!(msg.contains("StatusCode") || msg.contains("14") || msg.contains("failed"));
        remote.await.unwrap();
    }

    #[tokio::test]
    async fn test_simulated_restore_update_cancellation_halts_cleanly() {
        let (stream, mut peer) = tokio::io::duplex(32768);

        let remote = tokio::spawn(async move {
            let _q_type = read_test_plist(&mut peer).await;
            write_test_plist(
                &mut peer,
                &plist::Dictionary::from_iter([
                    (
                        "Type".to_owned(),
                        plist::Value::String("com.apple.mobile.restored".into()),
                    ),
                    (
                        "RestoreProtocolVersion".to_owned(),
                        plist::Value::Integer(15.into()),
                    ),
                ]),
            )
            .await;

            let _q_hw = read_test_plist(&mut peer).await;
            write_test_plist(
                &mut peer,
                &plist::Dictionary::from_iter([(
                    "HardwareInfo".to_owned(),
                    plist::Value::Dictionary(plist::Dictionary::from_iter([(
                        "UniqueChipID".to_owned(),
                        plist::Value::Integer(42.into()),
                    )])),
                )]),
            )
            .await;

            let _start = read_test_plist(&mut peer).await;
            // Stalls waiting for client
            tokio::time::sleep(Duration::from_secs(5)).await;
        });

        let cancel = ios_core::session::OperationCancel::new();
        let mut client = ios_core::restore::RestoredClient::connect(
            stream,
            42,
            Duration::from_secs(5),
            cancel.clone(),
        )
        .await
        .unwrap();

        let mut sm = ios_core::restore::RestoreStateMachine::new();
        sm.receive_nonces(Some(vec![1, 2, 3]), None).unwrap();

        let options = plist::Dictionary::from_iter([(
            "RestoreBehavior".to_owned(),
            plist::Value::String("Update".into()),
        )]);

        let cancel_trigger = cancel.clone();
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(50)).await;
            cancel_trigger.cancel();
        });

        let err = ios_core::restore::run_restored_update_loop(
            &mut client,
            sm,
            options,
            |_name, _req| Ok((plist::Dictionary::new(), Vec::new())),
            &cancel,
        )
        .await
        .unwrap_err();

        assert_eq!(
            ios_core::classify_error(&err),
            ios_core::ErrorClass::Cancelled
        );
        remote.abort();
    }

    #[tokio::test]
    async fn simulate_restore_rejects_without_explicit_confirmation_before_opening_files() {
        let cmd = RestoreCmd {
            sub: RestoreSub::SimulateRestore {
                ipsw: "nonexistent.ipsw".into(),
                tss: "nonexistent.shsh".into(),
                behavior: "Update".into(),
                board_id: "0x0c".into(),
                chip_id: "0x8110".into(),
                device_class: "d63ap".into(),
                ecid: "42".into(),
                ap_nonce_hex: "01020304".into(),
                confirm_simulation: false,
                timeout_secs: 5,
            },
        };
        let err = cmd.run(None, true).await.unwrap_err();
        assert!(err.to_string().contains("--force"));
    }

    #[tokio::test]
    async fn simulate_restore_executes_full_img4_ramdisk_reenumeration_and_restored_asr_session() {
        use std::io::Write;

        let temp_dir =
            std::env::temp_dir().join(format!("ios_cli_sim_restore_{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&temp_dir).unwrap();
        let ipsw_path = temp_dir.join("synthetic.ipsw");
        let tss_path = temp_dir.join("saved.shsh");

        let comp = |path: &str, fw: bool| {
            let mut info =
                plist::Dictionary::from_iter([("Path", plist::Value::String(path.into()))]);
            if fw {
                info.insert("IsFirmwarePayload".into(), plist::Value::Boolean(true));
            }
            plist::Value::Dictionary(plist::Dictionary::from_iter([(
                "Info",
                plist::Value::Dictionary(info),
            )]))
        };

        let identity = plist::Dictionary::from_iter([
            ("ApBoardID", plist::Value::Integer(0x0c.into())),
            ("ApChipID", plist::Value::Integer(0x8110.into())),
            ("ProductBuildVersion", plist::Value::String("21F90".into())),
            (
                "Info",
                plist::Value::Dictionary(plist::Dictionary::from_iter([
                    ("DeviceClass", plist::Value::String("d63ap".into())),
                    ("RestoreBehavior", plist::Value::String("Update".into())),
                    (
                        "Variant",
                        plist::Value::String("Upgrade Install (IPSW)".into()),
                    ),
                ])),
            ),
            (
                "Manifest",
                plist::Value::Dictionary(plist::Dictionary::from_iter([
                    ("LLB", comp("Firmware/LLB.im4p", true)),
                    ("iBoot", comp("Firmware/iBoot.im4p", true)),
                    ("DeviceTree", comp("Firmware/DeviceTree.im4p", true)),
                    ("RestoreLogo", comp("Firmware/RestoreLogo.im4p", false)),
                    ("RestoreRamDisk", comp("Firmware/RestoreRamDisk.dmg", false)),
                    (
                        "RestoreDeviceTree",
                        comp("Firmware/RestoreDeviceTree.im4p", false),
                    ),
                    ("RestoreSEP", comp("Firmware/RestoreSEP.im4p", false)),
                    ("SEP", comp("Firmware/SEP.im4p", false)),
                    (
                        "RestoreKernelCache",
                        comp("Firmware/RestoreKernelCache.im4p", false),
                    ),
                    ("KernelCache", comp("Firmware/KernelCache.im4p", false)),
                    ("OS", comp("090-OS.dmg", false)),
                ])),
            ),
        ]);

        let build_manifest = plist::Value::Dictionary(plist::Dictionary::from_iter([
            ("ProductVersion", plist::Value::String("17.5.1".into())),
            ("ProductBuildVersion", plist::Value::String("21F90".into())),
            (
                "BuildIdentities",
                plist::Value::Array(vec![plist::Value::Dictionary(identity)]),
            ),
        ]));
        let mut manifest_xml = Vec::new();
        plist::to_writer_xml(&mut manifest_xml, &build_manifest).unwrap();

        let ipsw_file = std::fs::File::create(&ipsw_path).unwrap();
        let mut zip = zip::ZipWriter::new(ipsw_file);
        let opts = zip::write::SimpleFileOptions::default();
        zip.start_file("BuildManifest.plist", opts).unwrap();
        zip.write_all(&manifest_xml).unwrap();
        for path in [
            "Firmware/LLB.im4p",
            "Firmware/iBoot.im4p",
            "Firmware/DeviceTree.im4p",
            "Firmware/RestoreLogo.im4p",
            "Firmware/RestoreRamDisk.dmg",
            "Firmware/RestoreDeviceTree.im4p",
            "Firmware/RestoreSEP.im4p",
            "Firmware/SEP.im4p",
            "Firmware/RestoreKernelCache.im4p",
            "Firmware/KernelCache.im4p",
            "090-OS.dmg",
        ] {
            zip.start_file(path, opts).unwrap();
            zip.write_all(format!("SYNTHETIC_BYTES_FOR_{path}").as_bytes())
                .unwrap();
        }
        zip.finish().unwrap();

        let saved_tss = plist::Value::Dictionary(plist::Dictionary::from_iter([
            (
                "Request",
                plist::Value::Dictionary(plist::Dictionary::from_iter([
                    ("ApECID", plist::Value::Integer(42.into())),
                    ("ApBoardID", plist::Value::Integer(0x0c.into())),
                    ("ApChipID", plist::Value::Integer(0x8110.into())),
                    ("ApNonce", plist::Value::Data(vec![1, 2, 3, 4])),
                ])),
            ),
            (
                "Response",
                plist::Value::Dictionary(plist::Dictionary::from_iter([(
                    "ApImg4Ticket",
                    plist::Value::Data(b"SYNTHETIC_AP_IMG4_TICKET".to_vec()),
                )])),
            ),
        ]));
        let mut tss_xml = Vec::new();
        plist::to_writer_xml(&mut tss_xml, &saved_tss).unwrap();
        std::fs::write(&tss_path, &tss_xml).unwrap();

        let cmd = RestoreCmd {
            sub: RestoreSub::SimulateRestore {
                ipsw: ipsw_path,
                tss: tss_path,
                behavior: "Update".into(),
                board_id: "0x0c".into(),
                chip_id: "0x8110".into(),
                device_class: "d63ap".into(),
                ecid: "42".into(),
                ap_nonce_hex: "01020304".into(),
                confirm_simulation: true,
                timeout_secs: 10,
            },
        };
        cmd.run(None, true).await.unwrap();
        let _ = std::fs::remove_dir_all(&temp_dir);
    }
}
