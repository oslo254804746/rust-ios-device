use std::path::PathBuf;

use anyhow::{Context, Result};
use ios_core::tunnel::TunMode;
use ios_core::{connect, ConnectOptions};

const GLOBAL_HTTP_PROXY_UUID: &str = "86a52338-52f7-4c09-b005-52baf3dc4882";

#[derive(clap::Args)]
pub struct HttpProxyCmd {
    #[command(subcommand)]
    sub: HttpProxySub,
}

#[derive(clap::Subcommand)]
enum HttpProxySub {
    /// Install a global HTTP proxy profile on a supervised device
    Set {
        host: String,
        port: u16,
        #[arg(long, help = "Supervisor identity in .p12 format")]
        p12: PathBuf,
        #[arg(long, env = "P12_PASSWORD", help = "Password for the .p12 file")]
        password: Option<String>,
    },
    /// Remove the global HTTP proxy profile installed by this tool
    Remove,
}

impl HttpProxyCmd {
    pub async fn run(self, udid: Option<String>, json: bool) -> Result<()> {
        let udid = udid.ok_or_else(|| anyhow::anyhow!("--udid required for httpproxy"))?;
        let device = connect(
            &udid,
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
        let mut client = ios_core::mcinstall::McInstallClient::new(stream);

        match self.sub {
            HttpProxySub::Set {
                host,
                port,
                p12,
                password,
            } => {
                let profile = build_http_proxy_profile(&host, port)?;
                let p12_bytes = std::fs::read(&p12)
                    .with_context(|| format!("failed to read {}", p12.display()))?;
                client
                    .install_profile_silent(&profile, &p12_bytes, password.as_deref().unwrap_or(""))
                    .await?;

                if json {
                    println!(
                        "{}",
                        serde_json::to_string_pretty(&serde_json::json!({
                            "installed": true,
                            "host": host,
                            "port": port,
                            "profile_identifier": GLOBAL_HTTP_PROXY_UUID,
                        }))?
                    );
                } else {
                    println!("Installed global HTTP proxy {host}:{port}");
                }
            }
            HttpProxySub::Remove => {
                client.remove_profile(GLOBAL_HTTP_PROXY_UUID).await?;
                if json {
                    println!(
                        "{}",
                        serde_json::to_string_pretty(&serde_json::json!({
                            "removed": true,
                            "profile_identifier": GLOBAL_HTTP_PROXY_UUID,
                        }))?
                    );
                } else {
                    println!("Removed global HTTP proxy profile");
                }
            }
        }

        Ok(())
    }
}

fn build_http_proxy_profile(host: &str, port: u16) -> Result<Vec<u8>> {
    if host.trim().is_empty() {
        return Err(anyhow::anyhow!("proxy host must not be empty"));
    }

    let payload_content = plist::Dictionary::from_iter([
        (
            "PayloadDescription".to_string(),
            plist::Value::String("Global HTTP Proxy".into()),
        ),
        (
            "PayloadDisplayName".to_string(),
            plist::Value::String("Global HTTP Proxy".into()),
        ),
        (
            "PayloadIdentifier".to_string(),
            plist::Value::String(format!(
                "com.apple.proxy.http.global.{GLOBAL_HTTP_PROXY_UUID}"
            )),
        ),
        (
            "PayloadType".to_string(),
            plist::Value::String("com.apple.proxy.http.global".into()),
        ),
        (
            "PayloadUUID".to_string(),
            plist::Value::String(GLOBAL_HTTP_PROXY_UUID.into()),
        ),
        (
            "PayloadVersion".to_string(),
            plist::Value::Integer(1.into()),
        ),
        (
            "ProxyCaptiveLoginAllowed".to_string(),
            plist::Value::Boolean(false),
        ),
        (
            "ProxyServer".to_string(),
            plist::Value::String(host.to_string()),
        ),
        (
            "ProxyServerPort".to_string(),
            plist::Value::Integer((port as i64).into()),
        ),
        (
            "ProxyType".to_string(),
            plist::Value::String("Manual".into()),
        ),
    ]);
    let profile = plist::Value::Dictionary(plist::Dictionary::from_iter([
        (
            "PayloadContent".to_string(),
            plist::Value::Array(vec![plist::Value::Dictionary(payload_content)]),
        ),
        (
            "PayloadDisplayName".to_string(),
            plist::Value::String("Global HTTP Proxy".into()),
        ),
        (
            "PayloadIdentifier".to_string(),
            plist::Value::String(GLOBAL_HTTP_PROXY_UUID.into()),
        ),
        (
            "PayloadRemovalDisallowed".to_string(),
            plist::Value::Boolean(false),
        ),
        (
            "PayloadType".to_string(),
            plist::Value::String("Configuration".into()),
        ),
        (
            "PayloadUUID".to_string(),
            plist::Value::String(GLOBAL_HTTP_PROXY_UUID.into()),
        ),
        (
            "PayloadVersion".to_string(),
            plist::Value::Integer(1.into()),
        ),
    ]));

    let mut buf = Vec::new();
    plist::to_writer_xml(&mut buf, &profile)?;
    Ok(buf)
}

#[cfg(test)]
mod tests {
    use clap::Parser;

    use super::*;

    #[derive(Parser)]
    struct TestCli {
        #[command(subcommand)]
        command: HttpProxySub,
    }

    #[test]
    fn parses_set_subcommand() {
        let cmd = TestCli::parse_from([
            "httpproxy",
            "set",
            "proxy.example.com",
            "8080",
            "--p12",
            "identity.p12",
        ]);
        match cmd.command {
            HttpProxySub::Set {
                host, port, p12, ..
            } => {
                assert_eq!(host, "proxy.example.com");
                assert_eq!(port, 8080);
                assert_eq!(p12, PathBuf::from("identity.p12"));
            }
            _ => panic!("expected httpproxy set"),
        }
    }

    #[test]
    fn build_http_proxy_profile_embeds_host_and_port() {
        let profile = build_http_proxy_profile("proxy.example.com", 8080).unwrap();
        let value = plist::Value::from_reader_xml(std::io::Cursor::new(profile)).unwrap();
        let dict = value.as_dictionary().unwrap();
        assert_eq!(
            dict.get("PayloadIdentifier")
                .and_then(plist::Value::as_string),
            Some(GLOBAL_HTTP_PROXY_UUID)
        );
        let payload = dict["PayloadContent"].as_array().unwrap()[0]
            .as_dictionary()
            .unwrap();
        assert_eq!(
            payload.get("ProxyServer").and_then(plist::Value::as_string),
            Some("proxy.example.com")
        );
        assert_eq!(
            payload
                .get("ProxyServerPort")
                .and_then(|value| value.as_signed_integer())
                .and_then(|value| u16::try_from(value).ok()),
            Some(8080)
        );
    }
}
