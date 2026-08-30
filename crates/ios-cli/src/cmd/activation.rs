use std::path::PathBuf;
use std::time::Duration;

use anyhow::{Context, Result};
use ios_core::TunMode;
use ios_core::{connect, ConnectOptions};
use zeroize::Zeroizing;

#[derive(clap::Args)]
pub struct ActivationCmd {
    #[command(subcommand)]
    sub: ActivationSub,
}

#[derive(clap::Subcommand)]
enum ActivationSub {
    /// Show the current activation state
    State,
    /// Show the mobileactivationd Tunnel1 session-info payload (redacted)
    SessionInfo {
        /// Absolute operation timeout in seconds
        #[arg(long, default_value_t = 30)]
        timeout_secs: u64,
    },
    /// Show the activation-info payload without writing activation back to the device (redacted)
    Info {
        /// Absolute operation timeout in seconds
        #[arg(long, default_value_t = 30)]
        timeout_secs: u64,
    },
    /// Activate the device online or apply an offline activation record
    Activate {
        /// Read a previously returned activation record and skip Apple HTTP
        #[arg(long, value_name = "PATH", conflicts_with = "record_output")]
        record_input: Option<PathBuf>,
        /// Save the online activation response as a private file before applying it
        #[arg(long, value_name = "PATH", conflicts_with = "record_input")]
        record_output: Option<PathBuf>,
        /// Override the Apple activation endpoint (requires --unsafe-custom-server)
        #[arg(long, value_name = "HTTPS_URL")]
        activation_url: Option<String>,
        /// Override the Apple DRM handshake endpoint (requires --unsafe-custom-server)
        #[arg(long, value_name = "HTTPS_URL")]
        drm_handshake_url: Option<String>,
        /// Permit custom HTTPS hosts; certificate verification remains enabled
        #[arg(long)]
        unsafe_custom_server: bool,
        /// Start immediately without waiting for mobileactivationd to publish a fresh nonce.
        /// This can fail if the daemon still exposes a session consumed by an earlier request.
        #[arg(long)]
        now: bool,
        /// Absolute operation timeout in seconds
        #[arg(long, default_value_t = 120)]
        timeout_secs: u64,
    },
    /// Deactivate the device (destructive)
    Deactivate {
        /// Required confirmation for deactivation
        #[arg(long)]
        force: bool,
        /// Absolute operation timeout in seconds
        #[arg(long, default_value_t = 30)]
        timeout_secs: u64,
    },
    /// Mark the device as connected to iTunes (legacy activation helper)
    ItunesActivate {
        /// Absolute operation timeout in seconds
        #[arg(long, default_value_t = 30)]
        timeout_secs: u64,
    },
}

impl ActivationCmd {
    pub async fn run(self, udid: Option<String>, json: bool) -> Result<()> {
        let udid = udid.ok_or_else(|| anyhow::anyhow!("--udid required for activation"))?;

        match self.sub {
            ActivationSub::State => {
                let device = connect_activation_device(&udid).await?;
                let value = device.lockdown_get_value(Some("ActivationState")).await?;
                let state = value.as_string().unwrap_or("Unknown");
                let activated = state != "Unactivated";

                if json {
                    println!(
                        "{}",
                        serde_json::to_string_pretty(&serde_json::json!({
                            "ActivationState": state,
                            "Activated": activated,
                        }))?
                    );
                } else {
                    println!("ActivationState: {state}");
                    println!("Activated:       {}", if activated { "yes" } else { "no" });
                }
            }
            ActivationSub::SessionInfo { timeout_secs } => {
                let deadline = absolute_deadline(timeout_secs)?;
                let device = with_deadline_at(
                    connect_activation_device(&udid),
                    deadline,
                    "device connection",
                )
                .await?;
                let stream = with_deadline_at(
                    device.connect_service(ios_core::mobileactivation::SERVICE_NAME),
                    deadline,
                    "mobileactivationd connection",
                )
                .await?;
                let mut client = ios_core::mobileactivation::MobileActivationClient::new(stream);
                let session_info = with_deadline_at(
                    client.request_session_info(),
                    deadline,
                    "session-info request",
                )
                .await?;
                print_value(redact_activation_diagnostic(session_info), json)?;
            }
            ActivationSub::Info { timeout_secs } => {
                let deadline = absolute_deadline(timeout_secs)?;
                let device = with_deadline_at(
                    connect_activation_device(&udid),
                    deadline,
                    "device connection",
                )
                .await?;
                let stream = with_deadline_at(
                    device.connect_service(ios_core::mobileactivation::SERVICE_NAME),
                    deadline,
                    "mobileactivationd connection",
                )
                .await?;
                let mut client = ios_core::mobileactivation::MobileActivationClient::new(stream);
                let session_info = with_deadline_at(
                    client.request_session_info(),
                    deadline,
                    "session-info request",
                )
                .await?;
                let session_value = session_info
                    .get("Value")
                    .and_then(plist::Value::as_dictionary)
                    .ok_or_else(|| {
                        anyhow::anyhow!("session-info response missing Value dictionary")
                    })?;

                let http = ios_core::mobileactivation::ActivationHttpClient::official()?;
                let handshake_response = with_deadline_at(
                    http.post_drm_handshake(session_value),
                    deadline,
                    "DRM handshake",
                )
                .await?;

                let stream = with_deadline_at(
                    device.connect_service(ios_core::mobileactivation::SERVICE_NAME),
                    deadline,
                    "mobileactivationd connection",
                )
                .await?;
                let mut client = ios_core::mobileactivation::MobileActivationClient::new(stream);
                // A successful Tunnel1 session selects the session protocol.  Do not
                // replay a potentially already-consumed request as the legacy command
                // merely because the Tunnel1 request failed: the failure may have
                // happened after mobileactivationd committed the nonce/session.
                let value = with_deadline_at(
                    client.request_tunnel1_activation_info(&handshake_response.body),
                    deadline,
                    "activation-info request",
                )
                .await?;
                print_value(redact_activation_diagnostic(value), json)?;
            }
            ActivationSub::Activate {
                record_input,
                record_output,
                activation_url,
                drm_handshake_url,
                unsafe_custom_server,
                now,
                timeout_secs,
            } => {
                let timeout = checked_timeout(timeout_secs)?;
                let deadline = absolute_deadline(timeout_secs)?;
                let device = with_deadline_at(
                    connect_activation_device(&udid),
                    deadline,
                    "device connection",
                )
                .await?;
                let state = with_deadline_at(
                    device.lockdown_get_value(Some("ActivationState")),
                    deadline,
                    "activation state query",
                )
                .await?;
                if state.as_string() != Some("Unactivated") && record_input.is_none() {
                    print_status(
                        json,
                        "already_activated",
                        "Device is already activated; no changes made.",
                    )?;
                    return Ok(());
                }

                let record = if let Some(path) = record_input {
                    read_secret_record(&path)?
                } else {
                    let activation_url = activation_url
                        .as_deref()
                        .unwrap_or(ios_core::mobileactivation::ACTIVATION_URL);
                    let drm_handshake_url = drm_handshake_url
                        .as_deref()
                        .unwrap_or(ios_core::mobileactivation::DRM_HANDSHAKE_URL);
                    let http = ios_core::mobileactivation::ActivationHttpClient::with_endpoints(
                        activation_url,
                        drm_handshake_url,
                        unsafe_custom_server,
                        timeout,
                    )?;
                    let session_info = with_deadline_at(
                        request_session_info(&device, !now, deadline),
                        deadline,
                        "session-info request",
                    )
                    .await?;
                    let response = match session_info {
                        Some(session_info) => {
                            let session_value = session_info
                                .get("Value")
                                .and_then(plist::Value::as_dictionary)
                                .ok_or_else(|| {
                                    anyhow::anyhow!(
                                        "session-info response missing Value dictionary"
                                    )
                                })?;
                            let handshake = with_deadline_at(
                                http.post_drm_handshake(session_value),
                                deadline,
                                "DRM handshake",
                            )
                            .await?;
                            let activation_info = with_deadline_at(
                                request_activation_info(&device, &handshake.body),
                                deadline,
                                "activation-info request",
                            )
                            .await?;
                            let activation_value = activation_info
                                .get("Value")
                                .and_then(plist::Value::as_dictionary)
                                .ok_or_else(|| {
                                    anyhow::anyhow!(
                                        "activation-info response missing Value dictionary"
                                    )
                                })?;
                            with_deadline_at(
                                http.post_activation_info(activation_value),
                                deadline,
                                "activation request",
                            )
                            .await?
                        }
                        None => {
                            let legacy_flow = async {
                                let all_values = device.lockdown_get_value(None).await?;
                                let values = all_values.as_dictionary().ok_or_else(|| {
                                    anyhow::anyhow!(
                                        "legacy lockdown values response was not a dictionary"
                                    )
                                })?;
                                let activation_info = values
                                    .get("ActivationInfo")
                                    .and_then(plist::Value::as_dictionary)
                                    .ok_or_else(|| {
                                        anyhow::anyhow!(
                                            "legacy lockdown values missing ActivationInfo dictionary"
                                        )
                                    })?;
                                let mut fields = std::collections::BTreeMap::from([(
                                    "InStoreActivation".to_string(),
                                    "False".to_string(),
                                )]);
                                if let Some(serial) =
                                    values.get("SerialNumber").and_then(plist::Value::as_string)
                                {
                                    fields.insert(
                                        "AppleSerialNumber".to_string(),
                                        serial.to_string(),
                                    );
                                }
                                if values
                                    .get("TelephonyCapability")
                                    .and_then(plist::Value::as_boolean)
                                    .unwrap_or(false)
                                {
                                    let mut has_imei_or_meid = false;
                                    for (field, source) in [
                                        ("IMEI", "InternationalMobileEquipmentIdentity"),
                                        ("MEID", "MobileEquipmentIdentifier"),
                                        ("IMSI", "InternationalMobileSubscriberIdentity"),
                                        ("ICCID", "IntegratedCircuitCardIdentity"),
                                    ] {
                                        if let Some(value) =
                                            values.get(source).and_then(plist::Value::as_string)
                                        {
                                            if matches!(field, "IMEI" | "MEID") {
                                                has_imei_or_meid = true;
                                            }
                                            fields.insert(field.to_string(), value.to_string());
                                        }
                                    }
                                    if !has_imei_or_meid {
                                        anyhow::bail!(
                                            "legacy activation values missing both IMEI and MEID"
                                        );
                                    }
                                }
                                http.post_activation_info_with_fields(activation_info, &fields)
                                    .await
                                    .map_err(anyhow::Error::from)
                            };
                            with_deadline_at(legacy_flow, deadline, "legacy activation flow")
                                .await
                                .map_err(|_| {
                                    // Do not include either failure's display text: transport
                                    // and lockdown errors can contain server/device material.
                                    anyhow::anyhow!(
                                        "session activation was unavailable; legacy activation failed"
                                    )
                                })?
                        }
                    };
                    if response.content_type_is("application/x-buddyml") {
                        anyhow::bail!(
                            "Apple requested interactive account credentials (BuddyML); refusing to prompt or print secrets"
                        );
                    }
                    if response.body.is_empty() || !looks_like_plist(&response.body) {
                        anyhow::bail!("activation endpoint returned an unsupported response type");
                    }
                    if let Some(path) = record_output {
                        ios_core::secret_file::write_secret(&path, &response.body).with_context(
                            || format!("failed to write activation record {}", path.display()),
                        )?;
                    }
                    apply_record(&device, &response.body, &response.headers, deadline).await?;
                    print_status(
                        json,
                        "activated",
                        "Activation request applied successfully.",
                    )?;
                    return Ok(());
                };
                apply_record(
                    &device,
                    &record,
                    &std::collections::BTreeMap::new(),
                    deadline,
                )
                .await?;
                print_status(
                    json,
                    "activated",
                    "Offline activation record applied successfully.",
                )?;
            }
            ActivationSub::Deactivate {
                force,
                timeout_secs,
            } => {
                crate::output::require_force(
                    force,
                    "deactivate the device",
                    "the device will lose its activation state",
                )?;
                let deadline = absolute_deadline(timeout_secs)?;
                let device = with_deadline_at(
                    connect_activation_device(&udid),
                    deadline,
                    "device connection",
                )
                .await?;
                with_deadline_at(deactivate_device(&device), deadline, "deactivation").await?;
                print_status(json, "deactivated", "Deactivation request sent.")?;
            }
            ActivationSub::ItunesActivate { timeout_secs } => {
                let deadline = absolute_deadline(timeout_secs)?;
                let device = with_deadline_at(
                    connect_activation_device(&udid),
                    deadline,
                    "device connection",
                )
                .await?;
                with_deadline_at(device.itunes_activate(), deadline, "iTunes activation").await?;
                print_status(
                    json,
                    "itunes_activation_recorded",
                    "iTunes activation marker set.",
                )?;
            }
        }

        Ok(())
    }
}

async fn connect_activation_device(udid: &str) -> Result<ios_core::ConnectedDevice> {
    connect(
        udid,
        ConnectOptions {
            tun_mode: TunMode::Userspace,
            pair_record_path: None,
            skip_tunnel: true,
        },
    )
    .await
    .with_context(|| format!("failed to connect to device {udid}"))
}

async fn request_session_info(
    device: &ios_core::ConnectedDevice,
    wait_for_fresh_session: bool,
    deadline: tokio::time::Instant,
) -> Result<Option<plist::Dictionary>> {
    // pmd3 treats failure of the initial session probe as the feature probe for
    // the legacy flow.  Once a response has been observed, the wait helper
    // propagates poll/final-request errors so they cannot trigger a duplicate
    // legacy activation side effect.
    let stream = match device
        .connect_service(ios_core::mobileactivation::SERVICE_NAME)
        .await
    {
        Ok(stream) => stream,
        Err(_) => return Ok(None),
    };
    let mut client = ios_core::mobileactivation::MobileActivationClient::new(stream);
    if wait_for_fresh_session {
        // The wait helper returns the exact response whose handshake changed.
        // Keep that blob: asking for session-info again here can rotate the
        // nonce a second time and leave the DRM handshake paired with a
        // different session than the one that satisfied the wait.
        let session_info = client
            .try_wait_for_activation_session_until(
                deadline,
                ios_core::mobileactivation::DEFAULT_ACTIVATION_POLL_INTERVAL,
            )
            .await?;
        Ok(session_info)
    } else {
        match client.request_session_info().await {
            Ok(session_info) => Ok(Some(session_info)),
            Err(error) if session_feature_unavailable(&error) => Ok(None),
            Err(error) => Err(error.into()),
        }
    }
}

fn session_feature_unavailable(error: &ios_core::mobileactivation::MobileActivationError) -> bool {
    matches!(
        error,
        ios_core::mobileactivation::MobileActivationError::Protocol(message)
            if message.starts_with("CreateTunnel1SessionInfoRequest failed")
    )
}

async fn request_activation_info(
    device: &ios_core::ConnectedDevice,
    handshake_response: &[u8],
) -> Result<plist::Dictionary> {
    let stream = device
        .connect_service(ios_core::mobileactivation::SERVICE_NAME)
        .await?;
    let mut client = ios_core::mobileactivation::MobileActivationClient::new(stream);
    // Session info has already selected Tunnel1.  Retrying with the legacy
    // command after an error can duplicate a nonce-bearing operation and is
    // not a safe feature probe.
    Ok(client
        .request_tunnel1_activation_info(handshake_response)
        .await?)
}

async fn apply_record(
    device: &ios_core::ConnectedDevice,
    record: &[u8],
    headers: &std::collections::BTreeMap<String, String>,
    deadline: tokio::time::Instant,
) -> Result<()> {
    with_deadline_at(
        async {
            device
                .activate(record, headers)
                .await
                .map_err(|error| anyhow::anyhow!(error.to_string()))
        },
        deadline,
        "activation record application",
    )
    .await
}

async fn deactivate_device(device: &ios_core::ConnectedDevice) -> Result<()> {
    device.deactivate().await?;
    Ok(())
}

fn read_secret_record(path: &std::path::Path) -> Result<Zeroizing<Vec<u8>>> {
    let metadata = std::fs::metadata(path)
        .with_context(|| format!("failed to inspect activation record {}", path.display()))?;
    const MAX_RECORD_SIZE: u64 = 16 * 1024 * 1024;
    if metadata.len() > MAX_RECORD_SIZE {
        anyhow::bail!("activation record exceeds {MAX_RECORD_SIZE} bytes");
    }
    let bytes = std::fs::read(path)
        .with_context(|| format!("failed to read activation record {}", path.display()))?;
    Ok(Zeroizing::new(bytes))
}

fn checked_timeout(seconds: u64) -> Result<Duration> {
    let duration = Duration::from_secs(seconds);
    tokio::time::Instant::now()
        .checked_add(duration)
        .map(|_| duration)
        .ok_or_else(|| anyhow::anyhow!("timeout is too large"))
}

fn absolute_deadline(seconds: u64) -> Result<tokio::time::Instant> {
    let duration = checked_timeout(seconds)?;
    tokio::time::Instant::now()
        .checked_add(duration)
        .ok_or_else(|| anyhow::anyhow!("timeout is too large"))
}

async fn with_deadline_at<F, T, E>(
    future: F,
    deadline: tokio::time::Instant,
    operation: &str,
) -> Result<T>
where
    F: std::future::Future<Output = Result<T, E>>,
    E: Into<anyhow::Error>,
{
    let mut ctrl_c = Box::pin(tokio::signal::ctrl_c());
    tokio::select! {
        result = tokio::time::timeout_at(deadline, future) => {
            result
                .map_err(|_| anyhow::anyhow!("{operation} timed out"))?
                .map_err(Into::into)
        }
        signal = &mut ctrl_c => {
            signal.map_err(|error| anyhow::anyhow!("failed waiting for Ctrl+C: {error}"))?;
            Err(anyhow::anyhow!("{operation} cancelled by Ctrl+C"))
        }
    }
}

fn looks_like_plist(bytes: &[u8]) -> bool {
    plist::from_bytes::<plist::Value>(bytes).is_ok()
}

fn print_status(json: bool, status: &str, message: &str) -> Result<()> {
    if json {
        println!("{}", serde_json::json!({"status": status}));
    } else {
        println!("{message}");
    }
    Ok(())
}

fn print_value(value: plist::Value, json: bool) -> Result<()> {
    if json {
        println!("{}", serde_json::to_string_pretty(&value)?);
    } else {
        println!("{value:?}");
    }
    Ok(())
}

/// Keep diagnostic activation output within a strict credential boundary.
///
/// Session and activation-info responses carry opaque attestation material
/// whose field names and nesting change across iOS releases.  Showing an
/// unknown field is therefore unsafe even after recursively hiding known
/// identifiers.  Preserve only the protocol status and redact every payload.
fn redact_activation_diagnostic(response: plist::Dictionary) -> plist::Value {
    plist::Value::Dictionary(
        response
            .into_iter()
            .map(|(key, value)| {
                let value =
                    if key == "Status" && matches!(value.as_string(), Some("Success" | "Error")) {
                        value
                    } else {
                        redacted_value(value)
                    };
                (key, value)
            })
            .collect(),
    )
}

fn redacted_value(value: plist::Value) -> plist::Value {
    match value {
        plist::Value::Data(bytes) => {
            plist::Value::String(format!("<redacted data: {} bytes>", bytes.len()))
        }
        plist::Value::Array(items) => plist::Value::Array(
            items
                .into_iter()
                .map(|_| plist::Value::String("<redacted>".into()))
                .collect(),
        ),
        _ => plist::Value::String("<redacted>".into()),
    }
}

#[cfg(test)]
mod tests {
    use clap::Parser;

    use super::*;

    #[derive(Parser)]
    struct TestCli {
        #[command(subcommand)]
        command: ActivationSub,
    }

    #[test]
    fn parses_activation_state_subcommand() {
        let cmd = TestCli::parse_from(["activation", "state"]);
        match cmd.command {
            ActivationSub::State => {}
            ActivationSub::SessionInfo { .. }
            | ActivationSub::Info { .. }
            | ActivationSub::Activate { .. }
            | ActivationSub::Deactivate { .. }
            | ActivationSub::ItunesActivate { .. } => {
                panic!("expected state subcommand")
            }
        }
    }

    #[test]
    fn parses_activation_session_info_subcommand() {
        let cmd = TestCli::parse_from(["activation", "session-info", "--timeout-secs", "2"]);
        match cmd.command {
            ActivationSub::SessionInfo { timeout_secs } => assert_eq!(timeout_secs, 2),
            ActivationSub::State
            | ActivationSub::Info { .. }
            | ActivationSub::Activate { .. }
            | ActivationSub::Deactivate { .. }
            | ActivationSub::ItunesActivate { .. } => {
                panic!("expected session-info subcommand")
            }
        }
    }

    #[test]
    fn parses_activation_info_subcommand() {
        let cmd = TestCli::parse_from(["activation", "info"]);
        match cmd.command {
            ActivationSub::Info { .. } => {}
            ActivationSub::State
            | ActivationSub::SessionInfo { .. }
            | ActivationSub::Activate { .. }
            | ActivationSub::Deactivate { .. }
            | ActivationSub::ItunesActivate { .. } => {
                panic!("expected info subcommand")
            }
        }
    }

    #[test]
    fn parses_activation_mutation_subcommands_and_force() {
        let cmd = TestCli::parse_from(["activation", "activate", "--now"]);
        match cmd.command {
            ActivationSub::Activate { now, .. } => assert!(now),
            ActivationSub::State
            | ActivationSub::SessionInfo { .. }
            | ActivationSub::Info { .. }
            | ActivationSub::Deactivate { .. }
            | ActivationSub::ItunesActivate { .. } => panic!("expected activate --now"),
        }
        assert!(TestCli::try_parse_from([
            "activation",
            "activate",
            "--record-input",
            "record.plist",
            "--timeout-secs",
            "4",
        ])
        .is_ok());
        assert!(TestCli::try_parse_from([
            "activation",
            "activate",
            "--record-output",
            "record.plist",
            "--unsafe-custom-server",
            "--activation-url",
            "https://example.invalid/activate",
            "--drm-handshake-url",
            "https://example.invalid/drm",
        ])
        .is_ok());
        assert!(TestCli::try_parse_from(["activation", "deactivate", "--force"]).is_ok());
        assert!(TestCli::try_parse_from(["activation", "itunes-activate"]).is_ok());
        assert!(TestCli::try_parse_from(["activation", "itunes-activate", "--now"]).is_err());
        assert!(TestCli::try_parse_from([
            "activation",
            "activate",
            "--record-input",
            "a",
            "--record-output",
            "b",
        ])
        .is_err());
    }

    #[test]
    fn legacy_session_fallback_only_accepts_explicit_daemon_rejection() {
        assert!(session_feature_unavailable(
            &ios_core::mobileactivation::MobileActivationError::Protocol(
                "CreateTunnel1SessionInfoRequest failed: <redacted error>".into(),
            )
        ));
        assert!(!session_feature_unavailable(
            &ios_core::mobileactivation::MobileActivationError::Protocol(
                "CreateTunnel1SessionInfoRequest response is missing Value".into(),
            )
        ));
        assert!(!session_feature_unavailable(
            &ios_core::mobileactivation::MobileActivationError::Timeout(Duration::from_secs(1))
        ));
    }

    #[test]
    fn activation_diagnostic_redaction_hides_unknown_and_embedded_payloads() {
        let value = plist::Dictionary::from_iter([
            ("Status".to_string(), plist::Value::String("Success".into())),
            (
                "ActivationInfoXML".to_string(),
                plist::Value::Data(b"PRIVATE-ATTESTATION".to_vec()),
            ),
            (
                "Value".to_string(),
                plist::Value::Dictionary(plist::Dictionary::from_iter([(
                    "UnknownCertificateField".to_string(),
                    plist::Value::String("-----BEGIN CERTIFICATE-----secret".into()),
                )])),
            ),
            (
                "UnknownString".to_string(),
                plist::Value::String("opaque activation token".into()),
            ),
        ]);
        let redacted = redact_activation_diagnostic(value);
        let rendered = format!("{redacted:?}");
        assert!(rendered.contains("Success"));
        assert!(!rendered.contains("PRIVATE-ATTESTATION"));
        assert!(!rendered.contains("CERTIFICATE"));
        assert!(!rendered.contains("opaque activation token"));
    }

    #[test]
    fn activation_record_size_and_missing_file_are_rejected() {
        assert!(read_secret_record(std::path::Path::new("/definitely/missing/record")).is_err());
    }
}
