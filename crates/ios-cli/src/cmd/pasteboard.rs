use anyhow::Result;
use base64::Engine;
use ios_core::pasteboard::{self, PasteboardClient};
use ios_core::{connect, ConnectOptions, TunMode, XpcValue};
use tokio::io::AsyncReadExt;

#[derive(clap::Args)]
pub struct PasteboardCmd {
    #[command(subcommand)]
    sub: PasteboardSub,
}

#[derive(clap::Subcommand)]
enum PasteboardSub {
    /// Read the named pasteboard (general by default)
    Get {
        #[arg(
            long,
            default_value = pasteboard::GENERAL_PASTEBOARD,
            help = "Pasteboard name"
        )]
        pasteboard: String,
        #[arg(
            long,
            help = "Include the raw snapshot with binary data base64-encoded"
        )]
        raw: bool,
    },
    /// Replace the named pasteboard with UTF-8 text or a URL
    Set {
        #[arg(
            value_name = "TEXT",
            conflicts_with = "url",
            help = "Text to set; if omitted, read UTF-8 text from stdin"
        )]
        text: Option<String>,
        #[arg(
            long,
            value_name = "URL",
            conflicts_with = "text",
            help = "Set the value under the public.url UTI"
        )]
        url: Option<String>,
        #[arg(
            long,
            default_value = pasteboard::GENERAL_PASTEBOARD,
            help = "Pasteboard name"
        )]
        pasteboard: String,
    },
}

impl PasteboardCmd {
    pub async fn run(self, udid: Option<String>, json_output: bool) -> Result<()> {
        let udid = udid.ok_or_else(|| anyhow::anyhow!("--udid required for pasteboard"))?;
        let device = connect(
            &udid,
            ConnectOptions {
                tun_mode: TunMode::Userspace,
                pair_record_path: None,
                skip_tunnel: false,
            },
        )
        .await?;
        let xpc = device.connect_xpc_service(pasteboard::SERVICE_NAME).await?;
        let mut client = PasteboardClient::new(xpc);

        match self.sub {
            PasteboardSub::Get { pasteboard, raw } => {
                let reply = client.get_named(&pasteboard).await?;
                if raw {
                    println!(
                        "{}",
                        serde_json::to_string_pretty(&xpc_value_to_json(&reply))?
                    );
                } else {
                    render_get(&pasteboard, &reply, json_output)?;
                }
            }
            PasteboardSub::Set {
                text,
                url,
                pasteboard,
            } => {
                let (kind, bytes) = if let Some(url) = url {
                    client.set_url_named(&pasteboard, &url).await?;
                    ("url", url.into_bytes())
                } else {
                    let text = match text {
                        Some(text) => text,
                        None => read_stdin_text().await?,
                    };
                    let bytes = text.as_bytes().to_vec();
                    client.set_text_named(&pasteboard, text).await?;
                    ("text", bytes)
                };

                if json_output {
                    println!(
                        "{}",
                        serde_json::to_string_pretty(&serde_json::json!({
                            "operation": "set",
                            "pasteboard": pasteboard,
                            "kind": kind,
                            "bytes": bytes.len(),
                        }))?
                    );
                } else {
                    println!(
                        "Set {kind} ({} bytes) on pasteboard '{pasteboard}'",
                        bytes.len()
                    );
                }
            }
        }

        Ok(())
    }
}

async fn read_stdin_text() -> Result<String> {
    let mut text = String::new();
    tokio::io::stdin().read_to_string(&mut text).await?;
    Ok(text)
}

fn render_get(pasteboard: &str, reply: &XpcValue, json_output: bool) -> Result<()> {
    let text = pasteboard::snapshot_text(reply);
    let url = pasteboard::snapshot_uti_text(reply, pasteboard::UTI_URL);

    if json_output {
        println!(
            "{}",
            serde_json::to_string_pretty(&serde_json::json!({
                "pasteboard": pasteboard,
                "text": text,
                "url": url,
            }))?
        );
    } else if let Some(text) = text {
        print!("{text}");
    } else if let Some(url) = url {
        println!("{url}");
    } else {
        println!("Pasteboard '{pasteboard}' has no inline UTF-8 text or URL data");
    }

    Ok(())
}

fn xpc_value_to_json(value: &XpcValue) -> serde_json::Value {
    match value {
        XpcValue::Null => serde_json::Value::Null,
        XpcValue::Bool(value) => serde_json::Value::Bool(*value),
        XpcValue::Int64(value) => serde_json::json!(*value),
        XpcValue::Uint64(value) => serde_json::json!(*value),
        XpcValue::Double(value) => serde_json::json!(*value),
        XpcValue::Date(value) => serde_json::json!(*value),
        XpcValue::Data(value) => serde_json::json!({
            "data_base64": base64::engine::general_purpose::STANDARD.encode(value),
        }),
        XpcValue::String(value) => serde_json::Value::String(value.clone()),
        XpcValue::Uuid(value) => serde_json::json!({
            "uuid": uuid::Uuid::from_bytes(*value).to_string(),
        }),
        XpcValue::Array(values) => {
            serde_json::Value::Array(values.iter().map(xpc_value_to_json).collect())
        }
        XpcValue::Dictionary(values) => serde_json::Value::Object(
            values
                .iter()
                .map(|(key, value)| (key.clone(), xpc_value_to_json(value)))
                .collect(),
        ),
        XpcValue::FileTransfer { msg_id, data } => serde_json::json!({
            "msg_id": msg_id,
            "data": xpc_value_to_json(data),
        }),
    }
}

#[cfg(test)]
mod tests {
    use clap::Parser;
    use indexmap::IndexMap;
    use ios_core::pasteboard::{snapshot_text, snapshot_uti_text};
    use ios_core::XpcValue;

    use super::*;

    #[derive(Parser)]
    struct TestCli {
        #[command(subcommand)]
        command: PasteboardSub,
    }

    #[test]
    fn parses_get_with_raw_and_named_pasteboard() {
        let cli = TestCli::parse_from(["pasteboard", "get", "--pasteboard", "general", "--raw"]);
        match cli.command {
            PasteboardSub::Get { pasteboard, raw } => {
                assert_eq!(pasteboard, "general");
                assert!(raw);
            }
            PasteboardSub::Set { .. } => panic!("expected get"),
        }
    }

    #[test]
    fn parses_set_text_url_and_defaults() {
        let cli = TestCli::parse_from(["pasteboard", "set", "hello"]);
        match cli.command {
            PasteboardSub::Set {
                text,
                url,
                pasteboard,
            } => {
                assert_eq!(text.as_deref(), Some("hello"));
                assert!(url.is_none());
                assert_eq!(pasteboard, "general");
            }
            PasteboardSub::Get { .. } => panic!("expected set"),
        }

        let cli = TestCli::parse_from(["pasteboard", "set", "--url", "https://example.test"]);
        assert!(matches!(
            cli.command,
            PasteboardSub::Set {
                text: None,
                url: Some(_),
                ..
            }
        ));
    }

    #[test]
    fn empty_text_argument_is_preserved_by_clap() {
        let cli = TestCli::parse_from(["pasteboard", "set", ""]);
        match cli.command {
            PasteboardSub::Set { text, .. } => assert_eq!(text.as_deref(), Some("")),
            PasteboardSub::Get { .. } => panic!("expected set"),
        }
    }

    #[test]
    fn raw_json_encodes_binary_data_without_loss() {
        let value = XpcValue::Dictionary(IndexMap::from([(
            "data".into(),
            XpcValue::Data(bytes::Bytes::from_static(b"\0\xff")),
        )]));
        assert_eq!(
            xpc_value_to_json(&value)["data"]["data_base64"],
            serde_json::Value::String("AP8=".into())
        );
    }

    #[test]
    fn get_render_helpers_distinguish_text_and_url() {
        let mut url_datum = IndexMap::new();
        url_datum.insert(
            "data".into(),
            XpcValue::Data(bytes::Bytes::from_static(b"https://example.test")),
        );
        let mut data = IndexMap::new();
        data.insert(pasteboard::UTI_URL.into(), XpcValue::Dictionary(url_datum));
        let mut item = IndexMap::new();
        item.insert("data".into(), XpcValue::Dictionary(data));
        let reply = XpcValue::Dictionary(IndexMap::from_iter([(
            "items".into(),
            XpcValue::Array(vec![XpcValue::Dictionary(item)]),
        )]));
        assert_eq!(snapshot_text(&reply), None);
        assert_eq!(
            snapshot_uti_text(&reply, pasteboard::UTI_URL).as_deref(),
            Some("https://example.test")
        );
    }
}
