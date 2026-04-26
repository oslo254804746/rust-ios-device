use anyhow::Result;
use ios_core::tunnel::TunMode;
use ios_core::{connect, ConnectOptions};

const ACTIVATION_USER_AGENT: &str = "iOS Device Activator (MobileActivation-592.103.2)";
const DRM_HANDSHAKE_URL: &str = "https://albert.apple.com/deviceservices/drmHandshake";

#[derive(clap::Args)]
pub struct ActivationCmd {
    #[command(subcommand)]
    sub: ActivationSub,
}

#[derive(clap::Subcommand)]
enum ActivationSub {
    /// Show the current activation state
    State,
    /// Show the mobileactivationd Tunnel1 session-info payload
    SessionInfo,
    /// Show the activation-info payload without writing activation back to the device
    Info,
}

impl ActivationCmd {
    pub async fn run(self, udid: Option<String>, json: bool) -> Result<()> {
        let udid = udid.ok_or_else(|| anyhow::anyhow!("--udid required for activation"))?;

        let opts = ConnectOptions {
            tun_mode: TunMode::Userspace,
            pair_record_path: None,
            skip_tunnel: true,
        };
        let device = connect(&udid, opts).await?;

        match self.sub {
            ActivationSub::State => {
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
            ActivationSub::SessionInfo => {
                let stream = device
                    .connect_service(ios_core::services::mobileactivation::SERVICE_NAME)
                    .await?;
                let mut client =
                    ios_core::services::mobileactivation::MobileActivationClient::new(stream);
                let value = expand_embedded_plists(plist::Value::Dictionary(
                    client.request_session_info().await?,
                ));
                println!("{}", serde_json::to_string_pretty(&value)?);
            }
            ActivationSub::Info => {
                let stream = device
                    .connect_service(ios_core::services::mobileactivation::SERVICE_NAME)
                    .await?;
                let mut client =
                    ios_core::services::mobileactivation::MobileActivationClient::new(stream);
                let session_info = client.request_session_info().await?;
                let session_value = session_info
                    .get("Value")
                    .and_then(plist::Value::as_dictionary)
                    .ok_or_else(|| {
                        anyhow::anyhow!("session-info response missing Value dictionary")
                    })?;

                let handshake_response = post_drm_handshake(session_value).await?;

                let stream = device
                    .connect_service(ios_core::services::mobileactivation::SERVICE_NAME)
                    .await?;
                let mut client =
                    ios_core::services::mobileactivation::MobileActivationClient::new(stream);
                let value = expand_embedded_plists(plist::Value::Dictionary(
                    client.request_activation_info(&handshake_response).await?,
                ));
                println!("{}", serde_json::to_string_pretty(&value)?);
            }
        }

        Ok(())
    }
}

async fn post_drm_handshake(session_value: &plist::Dictionary) -> Result<Vec<u8>> {
    let mut body = Vec::new();
    plist::to_writer_xml(&mut body, &plist::Value::Dictionary(session_value.clone()))?;

    let response = reqwest::Client::builder()
        .user_agent(ACTIVATION_USER_AGENT)
        .build()?
        .post(DRM_HANDSHAKE_URL)
        .header(reqwest::header::CONTENT_TYPE, "application/x-apple-plist")
        .header(reqwest::header::ACCEPT, "application/xml")
        .body(body)
        .send()
        .await?
        .error_for_status()?;

    Ok(response.bytes().await?.to_vec())
}

fn expand_embedded_plists(value: plist::Value) -> plist::Value {
    match value {
        plist::Value::Array(items) => {
            plist::Value::Array(items.into_iter().map(expand_embedded_plists).collect())
        }
        plist::Value::Dictionary(dict) => plist::Value::Dictionary(
            dict.into_iter()
                .map(|(key, value)| (key, expand_embedded_plists(value)))
                .collect(),
        ),
        plist::Value::Data(bytes) => try_decode_embedded_data(&bytes)
            .map(expand_embedded_plists)
            .unwrap_or(plist::Value::Data(bytes)),
        other => other,
    }
}

fn try_decode_embedded_data(bytes: &[u8]) -> Option<plist::Value> {
    try_decode_embedded_plist(bytes)
        .or_else(|| try_decode_embedded_json(bytes))
        .or_else(|| try_decode_embedded_text(bytes))
}

fn try_decode_embedded_plist(bytes: &[u8]) -> Option<plist::Value> {
    let start = find_bytes(bytes, b"<?xml").or_else(|| find_bytes(bytes, b"<plist"))?;
    let end = find_bytes(bytes, b"</plist>")?;
    if end < start {
        return None;
    }
    let slice = &bytes[start..end + b"</plist>".len()];
    plist::from_bytes(slice).ok()
}

fn try_decode_embedded_json(bytes: &[u8]) -> Option<plist::Value> {
    let trimmed = std::str::from_utf8(bytes).ok()?.trim();
    if !(trimmed.starts_with('{') || trimmed.starts_with('[')) {
        return None;
    }
    let json = serde_json::from_str::<serde_json::Value>(trimmed).ok()?;
    Some(json_to_plist(json))
}

fn try_decode_embedded_text(bytes: &[u8]) -> Option<plist::Value> {
    let text = std::str::from_utf8(bytes).ok()?;
    if !text
        .chars()
        .all(|ch| ch.is_ascii_graphic() || ch.is_ascii_whitespace())
    {
        return None;
    }
    Some(plist::Value::String(text.to_string()))
}

fn find_bytes(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack
        .windows(needle.len())
        .position(|window| window == needle)
}

fn json_to_plist(value: serde_json::Value) -> plist::Value {
    match value {
        serde_json::Value::Null => plist::Value::String("null".to_string()),
        serde_json::Value::Bool(value) => plist::Value::Boolean(value),
        serde_json::Value::Number(value) => {
            if let Some(value) = value.as_i64() {
                plist::Value::Integer(value.into())
            } else if let Some(value) = value.as_u64() {
                plist::Value::Integer(value.into())
            } else if let Some(value) = value.as_f64() {
                plist::Value::Real(value)
            } else {
                plist::Value::String(value.to_string())
            }
        }
        serde_json::Value::String(value) => plist::Value::String(value),
        serde_json::Value::Array(values) => {
            plist::Value::Array(values.into_iter().map(json_to_plist).collect())
        }
        serde_json::Value::Object(values) => plist::Value::Dictionary(
            values
                .into_iter()
                .map(|(key, value)| (key, json_to_plist(value)))
                .collect(),
        ),
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
            ActivationSub::SessionInfo | ActivationSub::Info => {
                panic!("expected state subcommand")
            }
        }
    }

    #[test]
    fn parses_activation_session_info_subcommand() {
        let cmd = TestCli::parse_from(["activation", "session-info"]);
        match cmd.command {
            ActivationSub::SessionInfo => {}
            ActivationSub::State | ActivationSub::Info => {
                panic!("expected session-info subcommand")
            }
        }
    }

    #[test]
    fn parses_activation_info_subcommand() {
        let cmd = TestCli::parse_from(["activation", "info"]);
        match cmd.command {
            ActivationSub::Info => {}
            ActivationSub::State | ActivationSub::SessionInfo => {
                panic!("expected info subcommand")
            }
        }
    }

    #[test]
    fn expands_embedded_plist_data_to_nested_value() {
        let raw = br#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0"><dict><key>Hello</key><string>World</string></dict></plist>"#;
        let value = plist::Value::Dictionary(plist::Dictionary::from_iter([(
            "Value".to_string(),
            plist::Value::Dictionary(plist::Dictionary::from_iter([(
                "ActivationInfoXML".to_string(),
                plist::Value::Data(raw.to_vec()),
            )])),
        )]));

        let expanded = expand_embedded_plists(value);
        let dict = expanded.into_dictionary().unwrap();
        let inner = dict["Value"].as_dictionary().unwrap();
        let nested = inner["ActivationInfoXML"].as_dictionary().unwrap();
        assert_eq!(nested["Hello"].as_string(), Some("World"));
    }

    #[test]
    fn expands_embedded_json_data_to_nested_value() {
        let value = plist::Value::Dictionary(plist::Dictionary::from_iter([(
            "Value".to_string(),
            plist::Value::Dictionary(plist::Dictionary::from_iter([(
                "CollectionBlob".to_string(),
                plist::Value::Dictionary(plist::Dictionary::from_iter([(
                    "IngestBody".to_string(),
                    plist::Value::Data(br#"{"serial-number":"ABC123","flag":true}"#.to_vec()),
                )])),
            )])),
        )]));

        let expanded = expand_embedded_plists(value);
        let dict = expanded.into_dictionary().unwrap();
        let ingest = dict["Value"].as_dictionary().unwrap()["CollectionBlob"]
            .as_dictionary()
            .unwrap()["IngestBody"]
            .as_dictionary()
            .unwrap();
        assert_eq!(ingest["serial-number"].as_string(), Some("ABC123"));
        assert_eq!(ingest["flag"].as_boolean(), Some(true));
    }

    #[test]
    fn expands_embedded_utf8_text_data_to_string() {
        let value = plist::Value::Data(
            b"-----BEGIN CERTIFICATE-----\nabc\n-----END CERTIFICATE-----".to_vec(),
        );
        let expanded = expand_embedded_plists(value);
        assert_eq!(
            expanded.as_string(),
            Some("-----BEGIN CERTIFICATE-----\nabc\n-----END CERTIFICATE-----")
        );
    }
}
