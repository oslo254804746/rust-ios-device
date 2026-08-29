//! Supervised MCInstall passcode and security operations.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use base64::Engine;
use ios_core::mcinstall::{McInstallClient, UnlockToken};
use ios_core::{connect, ConnectOptions, ServiceStream, TunMode};
use zeroize::{Zeroize, Zeroizing};

#[derive(clap::Args)]
pub struct MdmCmd {
    #[command(subcommand)]
    sub: MdmSub,
}

#[derive(clap::Subcommand)]
enum MdmSub {
    /// Fetch a passcode-unlock token into a protected file
    FetchUnlockToken {
        #[command(flatten)]
        supervisor: SupervisorArgs,
        #[arg(long, value_name = "FILE", help = "Protected output token file")]
        output: PathBuf,
        #[arg(
            long,
            help = "Write base64 text to the protected file instead of raw token bytes"
        )]
        base64: bool,
        #[arg(long, help = "Allow replacing an existing token file")]
        force: bool,
    },
    /// Query the supervised device security-information dictionary
    SecurityInfo {
        #[command(flatten)]
        supervisor: SupervisorArgs,
    },
    /// Report whether a device lock passcode is configured
    PasscodePresent {
        #[command(flatten)]
        supervisor: SupervisorArgs,
    },
    /// Clear the device lock passcode using a previously fetched token file
    ClearPasscode {
        #[command(flatten)]
        supervisor: SupervisorArgs,
        #[arg(long, value_name = "FILE", help = "Raw or base64 token file")]
        token: PathBuf,
        #[arg(long, help = "Interpret the token file as base64 text")]
        token_base64: bool,
        #[arg(
            long,
            help = "Required confirmation: this permanently removes the device lock passcode"
        )]
        force: bool,
    },
    /// Clear the Screen Time restrictions passcode
    ClearScreenTimePassword {
        #[command(flatten)]
        supervisor: SupervisorArgs,
        #[arg(
            long,
            help = "Required confirmation: this removes the Screen Time passcode"
        )]
        force: bool,
    },
}

#[derive(clap::Args)]
struct SupervisorArgs {
    #[arg(
        long,
        visible_alias = "p12file",
        value_name = "FILE",
        help = "Supervisor identity in PKCS#12 format"
    )]
    p12: PathBuf,
    #[arg(long, env = "P12_PASSWORD", help = "Password for the PKCS#12 identity")]
    password: Option<String>,
}

impl Drop for SupervisorArgs {
    fn drop(&mut self) {
        // Clap owns the password as a plain String until the command starts.
        // Clear it even when validation or connection setup fails before the
        // P12 escalation helper can move it into `Zeroizing`.
        if let Some(password) = self.password.as_mut() {
            password.zeroize();
        }
    }
}

impl MdmCmd {
    pub async fn run(self, udid: Option<String>, json: bool) -> Result<()> {
        let udid = udid.ok_or_else(|| anyhow::anyhow!("--udid required for mdm"))?;

        match self.sub {
            MdmSub::FetchUnlockToken {
                supervisor,
                output,
                base64,
                force,
            } => {
                validate_token_output_path(&output, force)?;
                let mut client = open_escalated_session(&udid, supervisor).await?;
                let token = client.fetch_unlock_token().await?;
                let contents = encode_token_file(&token, base64);
                write_token_file(&output, &contents, force)?;
                print_token_saved(&output, token.len(), contents.len(), base64, json);
            }
            MdmSub::SecurityInfo { supervisor } => {
                let mut client = open_escalated_session(&udid, supervisor).await?;
                let info = client.security_info().await?;
                print_security_info(&info, json)?;
            }
            MdmSub::PasscodePresent { supervisor } => {
                let mut client = open_escalated_session(&udid, supervisor).await?;
                let present = client.passcode_present().await?;
                if json {
                    println!(
                        "{}",
                        serde_json::to_string_pretty(&serde_json::json!({
                            "passcode_present": present,
                        }))?
                    );
                } else {
                    println!("Passcode present: {}", if present { "yes" } else { "no" });
                }
            }
            MdmSub::ClearPasscode {
                supervisor,
                token,
                token_base64,
                force,
            } => {
                crate::output::require_force(
                    force,
                    "clear the device lock passcode",
                    "the existing passcode is removed and cannot be recovered by this command",
                )?;
                validate_token_input_path(&token)?;
                let token = read_token_file(&token, token_base64)?;
                let mut client = open_escalated_session(&udid, supervisor).await?;
                client.clear_passcode(&token).await?;
                print_cleared("clear-passcode", json);
            }
            MdmSub::ClearScreenTimePassword { supervisor, force } => {
                crate::output::require_force(
                    force,
                    "clear the Screen Time passcode",
                    "the existing Screen Time restrictions passcode is removed",
                )?;
                let mut client = open_escalated_session(&udid, supervisor).await?;
                client.clear_screen_time_password().await?;
                print_cleared("clear-screen-time-password", json);
            }
        }

        Ok(())
    }
}

async fn open_escalated_session(
    udid: &str,
    mut supervisor: SupervisorArgs,
) -> Result<McInstallClient<ServiceStream>> {
    let p12_bytes = Zeroizing::new(
        std::fs::read(&supervisor.p12)
            .with_context(|| format!("failed to read {}", supervisor.p12.display()))?,
    );
    let password = Zeroizing::new(supervisor.password.take().unwrap_or_default());
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
        .connect_service(ios_core::mcinstall::SERVICE_NAME)
        .await?;
    let mut client = McInstallClient::new(stream);
    client
        .escalate_with_p12(p12_bytes.as_ref(), password.as_str())
        .await?;
    Ok(client)
}

fn validate_token_output_path(path: &Path, force: bool) -> Result<()> {
    reject_stdio_token_path(path, "output")?;
    if path.is_dir() {
        anyhow::bail!("token output path is a directory: {}", path.display());
    }
    if path.exists() && !force {
        anyhow::bail!(
            "refusing to overwrite existing token file {}; pass --force to replace it",
            path.display()
        );
    }
    Ok(())
}

fn validate_token_input_path(path: &Path) -> Result<()> {
    reject_stdio_token_path(path, "token")?;
    if !path.is_file() {
        anyhow::bail!(
            "token file does not exist or is not a regular file: {}",
            path.display()
        );
    }
    Ok(())
}

fn reject_stdio_token_path(path: &Path, role: &str) -> Result<()> {
    if path.as_os_str() == "-" {
        anyhow::bail!(
            "{role} must name a protected file; stdin/stdout token transport is disabled"
        );
    }
    Ok(())
}

fn write_token_file(path: &Path, contents: &[u8], force: bool) -> Result<()> {
    validate_token_output_path(path, force)?;
    ios_core::secret_file::write_secret(path, contents)
        .with_context(|| format!("failed to write protected token file {}", path.display()))?;
    Ok(())
}

fn read_token_file(path: &Path, base64_input: bool) -> Result<UnlockToken> {
    let encoded = Zeroizing::new(
        std::fs::read(path)
            .with_context(|| format!("failed to read token file {}", path.display()))?,
    );
    let bytes = if base64_input {
        decode_base64_token(encoded.as_ref())?
    } else {
        encoded.to_vec()
    };
    if bytes.is_empty() {
        anyhow::bail!("token file is empty");
    }
    Ok(UnlockToken::from_bytes(bytes))
}

fn encode_token_file(token: &UnlockToken, base64_output: bool) -> Zeroizing<Vec<u8>> {
    if base64_output {
        Zeroizing::new(
            base64::engine::general_purpose::STANDARD
                .encode(token.as_bytes())
                .into_bytes(),
        )
    } else {
        Zeroizing::new(token.as_bytes().to_vec())
    }
}

fn decode_base64_token(bytes: &[u8]) -> Result<Vec<u8>> {
    let text = std::str::from_utf8(bytes).context("base64 token file is not UTF-8")?;
    let compact = Zeroizing::new(text.split_whitespace().collect::<String>());
    if compact.is_empty() {
        anyhow::bail!("base64 token file is empty");
    }
    for encoding in [
        &base64::engine::general_purpose::STANDARD,
        &base64::engine::general_purpose::URL_SAFE,
    ] {
        if let Ok(decoded) = encoding.decode(&compact) {
            return Ok(decoded);
        }
        if let Ok(decoded) = encoding.decode(compact.trim_end_matches('=')) {
            return Ok(decoded);
        }
    }
    anyhow::bail!("token file is not valid padded or unpadded base64")
}

fn print_token_saved(
    path: &Path,
    token_bytes: usize,
    file_bytes: usize,
    base64_output: bool,
    json: bool,
) {
    if json {
        println!(
            "{}",
            serde_json::json!({
                "operation": "fetch-unlock-token",
                "path": path.display().to_string(),
                "token_bytes": token_bytes,
                "file_bytes": file_bytes,
                "encoding": if base64_output { "base64" } else { "raw" },
            })
        );
    } else {
        println!(
            "Wrote unlock token ({} bytes, {}) to {}",
            token_bytes,
            if base64_output { "base64" } else { "raw" },
            path.display()
        );
    }
}

fn print_cleared(operation: &str, json: bool) {
    if json {
        println!(
            "{}",
            serde_json::json!({
                "operation": operation,
                "status": "ok",
            })
        );
    } else {
        println!("{operation} completed");
    }
}

fn print_security_info(info: &plist::Dictionary, json: bool) -> Result<()> {
    let value = plist_dictionary_to_redacted_json(info);
    if json {
        println!("{}", serde_json::to_string_pretty(&value)?);
    } else if let serde_json::Value::Object(fields) = value {
        let mut fields: Vec<_> = fields.into_iter().collect();
        fields.sort_by(|left, right| left.0.cmp(&right.0));
        for (key, value) in fields {
            println!("{key}: {}", value_to_human_string(&value));
        }
    }
    Ok(())
}

fn plist_to_redacted_json(value: &plist::Value, key: Option<&str>) -> serde_json::Value {
    if key.is_some_and(|key| is_sensitive_key(key) && !is_safe_security_status_value(key, value)) {
        return serde_json::Value::String("<redacted>".into());
    }

    match value {
        plist::Value::Array(values) => serde_json::Value::Array(
            values
                .iter()
                .map(|value| plist_to_redacted_json(value, key))
                .collect(),
        ),
        plist::Value::Dictionary(values) => plist_dictionary_to_redacted_json(values),
        plist::Value::Boolean(value) => serde_json::Value::Bool(*value),
        plist::Value::Data(value) => serde_json::json!({
            "data_base64": base64::engine::general_purpose::STANDARD.encode(value),
        }),
        plist::Value::Date(value) => serde_json::Value::String(value.to_xml_format()),
        plist::Value::Integer(value) => {
            if let Some(value) = value.as_signed() {
                serde_json::json!(value)
            } else if let Some(value) = value.as_unsigned() {
                serde_json::json!(value)
            } else {
                serde_json::Value::Null
            }
        }
        plist::Value::Real(value) => serde_json::json!(*value),
        plist::Value::String(value) => serde_json::Value::String(value.clone()),
        plist::Value::Uid(value) => serde_json::json!(value.get()),
        _ => serde_json::Value::Null,
    }
}

fn plist_dictionary_to_redacted_json(values: &plist::Dictionary) -> serde_json::Value {
    serde_json::Value::Object(
        values
            .iter()
            .map(|(key, value)| (key.clone(), plist_to_redacted_json(value, Some(key))))
            .collect(),
    )
}

fn is_safe_security_status_value(key: &str, value: &plist::Value) -> bool {
    let key = key.to_ascii_lowercase();
    matches!(
        key.as_str(),
        "passcodepresent"
            | "passcodecompliant"
            | "passcodecompliantwithprofiles"
            | "passcodelockgraceperiod"
            | "passcodelockgraceperiodremaining"
    ) && matches!(
        value,
        plist::Value::Boolean(_) | plist::Value::Integer(_) | plist::Value::Real(_)
    )
}

fn value_to_human_string(value: &serde_json::Value) -> String {
    match value {
        serde_json::Value::String(value) => value.clone(),
        _ => serde_json::to_string(value).unwrap_or_else(|_| "null".into()),
    }
}

fn is_sensitive_key(key: &str) -> bool {
    let key = key.to_ascii_lowercase();
    key.contains("unlocktoken")
        || key.contains("token")
        || key.contains("password")
        || key.contains("passcode")
        || key.contains("p12")
        || key.contains("privatekey")
        || key.contains("secret")
        || key.contains("signedrequest")
        || key.contains("supervisorcertificate")
}

#[cfg(test)]
mod tests {
    use std::fs;

    use clap::Parser;
    use ios_core::mcinstall::UnlockToken;
    use plist::Value;

    use super::*;

    #[derive(Parser)]
    struct TestCli {
        #[command(subcommand)]
        command: MdmSub,
    }

    #[test]
    fn parses_fetch_token_file_and_base64_options() {
        let cli = TestCli::parse_from([
            "mdm",
            "fetch-unlock-token",
            "--p12",
            "identity.p12",
            "--output",
            "token.txt",
            "--base64",
            "--force",
        ]);
        match cli.command {
            MdmSub::FetchUnlockToken {
                supervisor,
                output,
                base64,
                force,
            } => {
                assert_eq!(supervisor.p12, PathBuf::from("identity.p12"));
                assert_eq!(output, PathBuf::from("token.txt"));
                assert!(base64);
                assert!(force);
            }
            _ => panic!("expected fetch-unlock-token"),
        }
    }

    #[test]
    fn parses_all_mdm_operations_and_force_flags() {
        assert!(matches!(
            TestCli::try_parse_from(["mdm", "security-info", "--p12file", "identity.p12"])
                .unwrap()
                .command,
            MdmSub::SecurityInfo { .. }
        ));
        assert!(matches!(
            TestCli::try_parse_from(["mdm", "passcode-present", "--p12", "identity.p12"])
                .unwrap()
                .command,
            MdmSub::PasscodePresent { .. }
        ));
        assert!(matches!(
            TestCli::try_parse_from([
                "mdm",
                "clear-passcode",
                "--p12",
                "identity.p12",
                "--token",
                "token.bin",
                "--token-base64",
                "--force"
            ])
            .unwrap()
            .command,
            MdmSub::ClearPasscode {
                token_base64: true,
                force: true,
                ..
            }
        ));
        assert!(matches!(
            TestCli::try_parse_from([
                "mdm",
                "clear-screen-time-password",
                "--p12",
                "identity.p12",
                "--force"
            ])
            .unwrap()
            .command,
            MdmSub::ClearScreenTimePassword { force: true, .. }
        ));
    }

    #[test]
    fn base64_token_roundtrip_is_whitespace_tolerant() {
        let token = UnlockToken::from_bytes(vec![0, 1, 2, 0xff, 0xfe]);
        let encoded = encode_token_file(&token, true);
        let wrapped = [
            b"  ".as_slice(),
            &encoded[..2],
            b"\n".as_slice(),
            &encoded[2..],
            b"\n".as_slice(),
        ]
        .concat();
        assert_eq!(decode_base64_token(&wrapped).unwrap(), token.as_bytes());
    }

    #[test]
    fn token_file_requires_force_to_overwrite_and_is_private() {
        let dir = std::env::temp_dir().join(format!("ios_rs_mdm_token_cli_{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join("token");
        write_token_file(&path, b"first", false).unwrap();
        assert!(write_token_file(&path, b"second", false).is_err());
        write_token_file(&path, b"second", true).unwrap();
        assert_eq!(fs::read(&path).unwrap(), b"second");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                fs::metadata(&path).unwrap().permissions().mode() & 0o777,
                0o600
            );
        }
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn security_json_redacts_secret_values_but_keeps_status_booleans() {
        let secret = vec![0xde, 0xad, 0xbe, 0xef];
        let info = plist::Dictionary::from_iter([
            ("PasscodePresent".to_string(), Value::Boolean(true)),
            ("UnlockToken".to_string(), Value::Data(secret)),
            ("TokenCount".to_string(), Value::Integer(1234.into())),
            (
                "PasscodeLockGracePeriod".to_string(),
                Value::Integer(60.into()),
            ),
            (
                "Nested".to_string(),
                Value::Dictionary(plist::Dictionary::from_iter([(
                    "Password".to_string(),
                    Value::String("do-not-print".into()),
                )])),
            ),
        ]);
        let rendered = plist_to_redacted_json(&Value::Dictionary(info), None).to_string();
        assert!(rendered.contains("PasscodePresent"));
        assert!(rendered.contains("true"));
        assert!(rendered.contains("<redacted>"));
        assert!(!rendered.contains("do-not-print"));
        assert!(rendered.contains("TokenCount"));
        assert!(!rendered.contains("1234"));
        assert!(rendered.contains("PasscodeLockGracePeriod"));
        assert!(rendered.contains("60"));
    }

    #[test]
    fn token_stdout_and_missing_file_are_rejected() {
        assert!(validate_token_output_path(Path::new("-"), false).is_err());
        assert!(validate_token_input_path(Path::new("-")).is_err());
    }
}
