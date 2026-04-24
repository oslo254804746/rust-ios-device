use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result};
use ios_core::{connect, ConnectOptions, ConnectedDevice};
use ios_tunnel::TunMode;

const AFC_SERVICE_NAME: &str = "com.apple.afc";
const SKIP_SETUP_DIR: &str = "iTunes_Control/iTunes";
const SKIP_SETUP_FILE: &str = "iTunes_Control/iTunes/SkipSetup";

#[derive(clap::Args)]
pub struct PrepareCmd {
    #[command(subcommand)]
    sub: Option<PrepareSub>,
    #[arg(long, help = "Supervisor certificate in DER format")]
    cert_der: Option<PathBuf>,
    #[arg(long, default_value = ios_services::prepare::DEFAULT_ORGANIZATION_NAME)]
    organization_name: String,
    #[arg(long, default_value = ios_services::prepare::DEFAULT_LANGUAGE)]
    language: String,
    #[arg(long, default_value = ios_services::prepare::DEFAULT_LOCALE)]
    locale: String,
    #[arg(long, help = "Also set this lockdown time zone during prepare")]
    time_zone: Option<String>,
    #[arg(
        long = "skip",
        help = "Skip-setup keys to apply; repeat to override defaults",
        value_delimiter = ','
    )]
    skip_setup: Vec<String>,
}

#[derive(clap::Subcommand)]
enum PrepareSub {
    /// Generate a supervision certificate bundle (.der/.pem/-key.pem/.p12)
    CreateCert {
        output_prefix: PathBuf,
        #[arg(long, default_value = ios_services::prepare::DEFAULT_ORGANIZATION_NAME)]
        common_name: String,
        #[arg(long, env = "P12_PASSWORD", default_value = "")]
        password: String,
    },
}

impl PrepareCmd {
    pub async fn run(self, udid: Option<String>, json: bool) -> Result<()> {
        match self.sub {
            Some(PrepareSub::CreateCert {
                output_prefix,
                common_name,
                password,
            }) => run_create_cert(output_prefix, &common_name, &password, json),
            None => {
                let udid = udid.ok_or_else(|| anyhow::anyhow!("--udid required for prepare"))?;
                run_prepare(
                    &udid,
                    self.cert_der,
                    &self.organization_name,
                    &self.language,
                    &self.locale,
                    self.time_zone.as_deref(),
                    &self.skip_setup,
                    json,
                )
                .await
            }
        }
    }
}

fn run_create_cert(
    output_prefix: PathBuf,
    common_name: &str,
    password: &str,
    json: bool,
) -> Result<()> {
    let identity = ios_services::prepare::generate_supervision_identity(common_name, password)
        .map_err(|err| anyhow::anyhow!("{err}"))?;

    if let Some(parent) = output_prefix.parent() {
        if !parent.as_os_str().is_empty() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("failed to create {}", parent.display()))?;
        }
    }

    let der_path = with_suffix(&output_prefix, ".der");
    let pem_path = with_suffix(&output_prefix, ".pem");
    let key_path = with_suffix(&output_prefix, "-key.pem");
    let p12_path = with_suffix(&output_prefix, ".p12");

    std::fs::write(&der_path, &identity.certificate_der)?;
    std::fs::write(&pem_path, &identity.certificate_pem)?;
    std::fs::write(&key_path, &identity.private_key_pem)?;
    std::fs::write(&p12_path, &identity.pkcs12_der)?;

    if json {
        println!(
            "{}",
            serde_json::to_string_pretty(&serde_json::json!({
                "certificate_der": der_path.display().to_string(),
                "certificate_pem": pem_path.display().to_string(),
                "private_key_pem": key_path.display().to_string(),
                "pkcs12": p12_path.display().to_string(),
                "password_protected": !password.is_empty(),
            }))?
        );
    } else {
        println!("DER: {}", der_path.display());
        println!("PEM: {}", pem_path.display());
        println!("Key: {}", key_path.display());
        println!("P12: {}", p12_path.display());
    }

    Ok(())
}

#[allow(clippy::too_many_arguments)]
async fn run_prepare(
    udid: &str,
    cert_der: Option<PathBuf>,
    organization_name: &str,
    language: &str,
    locale: &str,
    time_zone: Option<&str>,
    skip_setup: &[String],
    json: bool,
) -> Result<()> {
    let device = connect(
        udid,
        ConnectOptions {
            tun_mode: TunMode::Userspace,
            pair_record_path: None,
            skip_tunnel: true,
        },
    )
    .await?;

    ensure_activated(&device).await?;

    let supervision_certificate = match cert_der {
        Some(path) => Some(
            std::fs::read(&path).with_context(|| format!("failed to read {}", path.display()))?,
        ),
        None => None,
    };
    let skip_keys = if skip_setup.is_empty() {
        ios_services::prepare::DEFAULT_SKIP_SETUP_KEYS
            .iter()
            .map(|value| (*value).to_string())
            .collect::<Vec<_>>()
    } else {
        skip_setup.to_vec()
    };

    let mcinstall_stream = device
        .connect_service(ios_services::mcinstall::SERVICE_NAME)
        .await?;
    let mut mcinstall = ios_services::mcinstall::McInstallClient::new(mcinstall_stream);
    mcinstall.flush().await?;
    let _ = mcinstall.get_cloud_configuration().await?;
    mcinstall.hello_host_identifier().await?;
    mcinstall
        .set_cloud_configuration(ios_services::prepare::build_cloud_configuration(
            &skip_keys,
            supervision_certificate.as_deref(),
            Some(organization_name),
        ))
        .await?;
    mcinstall.hello_host_identifier().await?;
    let _ = mcinstall.get_cloud_configuration().await?;
    mcinstall.hello_host_identifier().await?;
    if let Err(err) = mcinstall.escalate_unsupervised().await {
        tracing::debug!("prepare escalate-unsupervised returned non-fatal error: {err}");
    }
    mcinstall.hello_host_identifier().await?;
    mcinstall
        .install_profile(
            &ios_services::prepare::build_initial_profile()
                .map_err(|err| anyhow::anyhow!("{err}"))?,
        )
        .await?;

    configure_lockdown(&device, language, locale, time_zone).await?;
    ensure_skip_setup_marker(&device).await?;

    if json {
        println!(
            "{}",
            serde_json::to_string_pretty(&serde_json::json!({
                "prepared": true,
                "supervised": supervision_certificate.is_some(),
                "organization_name": if supervision_certificate.is_some() { Some(organization_name) } else { None::<&str> },
                "language": language,
                "locale": locale,
                "time_zone": time_zone,
                "skip_setup_count": skip_keys.len(),
            }))?
        );
    } else {
        println!("Prepared device {udid}");
        println!("Language: {language}");
        println!("Locale: {locale}");
        if let Some(time_zone) = time_zone {
            println!("Time zone: {time_zone}");
        }
        println!(
            "Supervised: {}",
            if supervision_certificate.is_some() {
                "yes"
            } else {
                "no"
            }
        );
        println!("Skip-setup keys: {}", skip_keys.len());
    }

    Ok(())
}

async fn ensure_activated(device: &ConnectedDevice) -> Result<()> {
    let activation_state = device.lockdown_get_value(Some("ActivationState")).await?;
    let state = activation_state
        .as_string()
        .ok_or_else(|| anyhow::anyhow!("ActivationState was not a string"))?;
    if state == "Unactivated" {
        return Err(anyhow::anyhow!("please activate the device first"));
    }
    Ok(())
}

async fn configure_lockdown(
    device: &ConnectedDevice,
    language: &str,
    locale: &str,
    time_zone: Option<&str>,
) -> Result<()> {
    device
        .lockdown_set_value_in_domain(
            Some("com.apple.international"),
            Some("Language"),
            plist::Value::String(language.to_string()),
        )
        .await?;
    device
        .lockdown_set_value_in_domain(
            Some("com.apple.international"),
            Some("Locale"),
            plist::Value::String(locale.to_string()),
        )
        .await?;
    let epoch = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|err| anyhow::anyhow!("system clock before unix epoch: {err}"))?
        .as_secs() as i64;
    device
        .lockdown_set_value(
            Some("TimeIntervalSince1970"),
            plist::Value::Integer(epoch.into()),
        )
        .await?;
    if let Some(time_zone) = time_zone {
        device
            .lockdown_set_value(
                Some("TimeZone"),
                plist::Value::String(time_zone.to_string()),
            )
            .await?;
    }
    Ok(())
}

async fn ensure_skip_setup_marker(device: &ConnectedDevice) -> Result<()> {
    let stream = device.connect_service(AFC_SERVICE_NAME).await?;
    let mut afc = ios_services::afc::AfcClient::new(stream);
    if let Err(err) = afc.remove_all(SKIP_SETUP_FILE).await {
        tracing::debug!("skip-setup marker cleanup ignored: {err}");
    }
    if let Err(err) = afc.make_dir(SKIP_SETUP_DIR).await {
        tracing::debug!("skip-setup dir creation returned: {err}");
    }
    afc.write_file(SKIP_SETUP_FILE, b"").await?;
    Ok(())
}

fn with_suffix(prefix: &Path, suffix: &str) -> PathBuf {
    let mut base = prefix.as_os_str().to_os_string();
    base.push(suffix);
    PathBuf::from(base)
}

#[cfg(test)]
mod tests {
    use clap::Parser;

    use super::*;

    #[derive(Parser)]
    struct TestCli {
        #[command(flatten)]
        command: PrepareCmd,
    }

    #[test]
    fn parses_prepare_create_cert_subcommand() {
        let cmd = TestCli::parse_from(["prepare", "create-cert", "ios-rs-tmp/supervision"]);
        assert!(matches!(
            cmd.command.sub,
            Some(PrepareSub::CreateCert { .. })
        ));
    }

    #[test]
    fn parses_prepare_apply_flags() {
        let cmd = TestCli::parse_from([
            "prepare",
            "--cert-der",
            "ios-rs-tmp/supervision.der",
            "--organization-name",
            "Example Org",
            "--language",
            "en",
            "--locale",
            "en_US",
            "--skip",
            "WiFi,Privacy",
        ]);
        assert!(cmd.command.sub.is_none());
        assert_eq!(
            cmd.command.cert_der,
            Some(PathBuf::from("ios-rs-tmp/supervision.der"))
        );
        assert_eq!(cmd.command.skip_setup, vec!["WiFi", "Privacy"]);
    }
}
