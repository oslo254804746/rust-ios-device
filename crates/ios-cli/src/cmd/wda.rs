use anyhow::{Context, Result};
use serde_json::Value;

const DEFAULT_BASE_URL: &str = "http://127.0.0.1:8100";

#[derive(clap::Args)]
pub struct WdaCmd {
    #[arg(long, global = true, default_value = DEFAULT_BASE_URL)]
    pub base_url: String,
    #[arg(long, global = true, default_value_t = 10)]
    pub timeout_secs: u64,
    #[command(subcommand)]
    sub: WdaSub,
}

#[derive(clap::Subcommand)]
enum WdaSub {
    /// GET /status from a running WDA endpoint
    Status,
    /// POST /session and print the returned session id/payload
    Session {
        #[arg(long)]
        bundle_id: Option<String>,
    },
    /// GET the XML source tree
    Source {
        #[arg(long)]
        session_id: Option<String>,
    },
    /// Capture a WDA screenshot and write PNG bytes
    Screenshot {
        output: String,
        #[arg(long)]
        session_id: Option<String>,
    },
    /// Find an element and print its id
    Find {
        session_id: String,
        using: String,
        value: String,
    },
    /// Click an element by id
    Click {
        session_id: String,
        element_id: String,
    },
    /// Press a WDA device button such as home, volumeUp, volumeDown, or lock
    PressButton {
        name: String,
        #[arg(long)]
        session_id: Option<String>,
    },
    /// Unlock through WDA
    Unlock {
        #[arg(long)]
        session_id: Option<String>,
    },
    /// Send text through WDA
    SendKeys { session_id: String, text: String },
    /// Swipe by screen coordinates
    Swipe {
        session_id: String,
        start_x: i64,
        start_y: i64,
        end_x: i64,
        end_y: i64,
        #[arg(long, default_value_t = 0.2)]
        duration: f64,
    },
}

impl WdaCmd {
    pub async fn run(self, json_output: bool) -> Result<()> {
        let client = WdaHttpClient::new(self.base_url, self.timeout_secs);
        match self.sub {
            WdaSub::Status => print_json(client.get("/status").await?, json_output)?,
            WdaSub::Session { bundle_id } => {
                let payload = session_payload(bundle_id.as_deref());
                let response = client.post("/session", payload).await?;
                print_json(response, json_output)?;
            }
            WdaSub::Source { session_id } => {
                let path = session_path(session_id.as_deref(), "source");
                let response = client.get(&path).await?;
                if json_output {
                    print_json(response, true)?;
                } else if let Some(source) = response.get("value").and_then(Value::as_str) {
                    println!("{source}");
                } else {
                    print_json(response, true)?;
                }
            }
            WdaSub::Screenshot { output, session_id } => {
                let path = session_path(session_id.as_deref(), "screenshot");
                let response = client.get(&path).await?;
                let encoded = response
                    .get("value")
                    .and_then(Value::as_str)
                    .context("WDA response did not contain screenshot data")?;
                let data =
                    base64::Engine::decode(&base64::engine::general_purpose::STANDARD, encoded)
                        .context("WDA screenshot was not valid base64")?;
                tokio::fs::write(&output, data).await?;
                if json_output {
                    println!(
                        "{}",
                        serde_json::to_string_pretty(&serde_json::json!({ "output": output }))?
                    );
                } else {
                    println!("Wrote {output}");
                }
            }
            WdaSub::Find {
                session_id,
                using,
                value,
            } => {
                let response = client
                    .post(
                        &format!("/session/{session_id}/element"),
                        serde_json::json!({ "using": using, "value": value }),
                    )
                    .await?;
                if json_output {
                    print_json(response, true)?;
                } else {
                    println!(
                        "{}",
                        element_id(&response).context("WDA did not return an element id")?
                    );
                }
            }
            WdaSub::Click {
                session_id,
                element_id,
            } => {
                let response = client
                    .post(
                        &format!("/session/{session_id}/element/{element_id}/click"),
                        serde_json::json!({}),
                    )
                    .await?;
                print_json(response, json_output)?;
            }
            WdaSub::PressButton { name, session_id } => {
                let normalized = normalize_button_name(&name);
                let path = match session_id {
                    Some(session_id) => format!("/session/{session_id}/wda/pressButton"),
                    None if normalized == "home" => "/wda/homescreen".to_string(),
                    None => anyhow::bail!("--session-id is required for non-home WDA buttons"),
                };
                let response = client
                    .post(&path, serde_json::json!({ "name": normalized }))
                    .await?;
                print_json(response, json_output)?;
            }
            WdaSub::Unlock { session_id } => {
                let path = match session_id {
                    Some(session_id) => format!("/session/{session_id}/wda/unlock"),
                    None => "/wda/unlock".to_string(),
                };
                let response = client.post(&path, serde_json::json!({})).await?;
                print_json(response, json_output)?;
            }
            WdaSub::SendKeys { session_id, text } => {
                let response = client
                    .post(
                        &format!("/session/{session_id}/wda/keys"),
                        serde_json::json!({ "value": text.chars().map(|ch| ch.to_string()).collect::<Vec<_>>() }),
                    )
                    .await?;
                print_json(response, json_output)?;
            }
            WdaSub::Swipe {
                session_id,
                start_x,
                start_y,
                end_x,
                end_y,
                duration,
            } => {
                let response = client
                    .post(
                        &format!("/session/{session_id}/wda/dragfromtoforduration"),
                        serde_json::json!({
                            "fromX": start_x,
                            "fromY": start_y,
                            "toX": end_x,
                            "toY": end_y,
                            "duration": duration,
                        }),
                    )
                    .await?;
                print_json(response, json_output)?;
            }
        }
        Ok(())
    }
}

struct WdaHttpClient {
    client: reqwest::Client,
    base_url: String,
}

impl WdaHttpClient {
    fn new(base_url: String, timeout_secs: u64) -> Self {
        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(timeout_secs))
            .build()
            .expect("valid reqwest client");
        Self {
            client,
            base_url: base_url.trim_end_matches('/').to_string(),
        }
    }

    async fn get(&self, path: &str) -> Result<Value> {
        self.request(self.client.get(self.url(path))).await
    }

    async fn post(&self, path: &str, payload: Value) -> Result<Value> {
        self.request(
            self.client
                .post(self.url(path))
                .header(reqwest::header::CONTENT_TYPE, "application/json")
                .body(payload.to_string()),
        )
        .await
    }

    async fn request(&self, request: reqwest::RequestBuilder) -> Result<Value> {
        let response = request.send().await?;
        let status = response.status();
        let bytes = response
            .bytes()
            .await
            .with_context(|| format!("WDA returned non-JSON response with HTTP {status}"))?;
        let value = serde_json::from_slice::<Value>(&bytes)
            .with_context(|| format!("WDA returned non-JSON response with HTTP {status}"))?;
        if status.is_client_error() || status.is_server_error() || wda_status_is_error(&value) {
            anyhow::bail!(
                "WDA request failed with HTTP {status}: {}",
                format_wda_error(&value)
            );
        }
        Ok(value)
    }

    fn url(&self, path: &str) -> String {
        format!("{}{}", self.base_url, path)
    }
}

fn print_json(value: Value, json_output: bool) -> Result<()> {
    if json_output {
        println!("{}", serde_json::to_string_pretty(&value)?);
    } else {
        println!("{}", serde_json::to_string_pretty(&value)?);
    }
    Ok(())
}

fn session_payload(bundle_id: Option<&str>) -> Value {
    let mut caps = serde_json::Map::new();
    if let Some(bundle_id) = bundle_id {
        caps.insert("bundleId".to_string(), Value::String(bundle_id.to_string()));
    }
    serde_json::json!({
        "capabilities": { "alwaysMatch": caps },
        "desiredCapabilities": caps,
    })
}

fn session_path(session_id: Option<&str>, suffix: &str) -> String {
    match session_id {
        Some(session_id) => format!("/session/{session_id}/{suffix}"),
        None => format!("/{suffix}"),
    }
}

fn element_id(response: &Value) -> Option<&str> {
    let value = response.get("value")?.as_object()?;
    value
        .get("ELEMENT")
        .or_else(|| value.get("element-6066-11e4-a52e-4f735466cecf"))
        .or_else(|| value.get("element"))
        .and_then(Value::as_str)
}

fn wda_status_is_error(value: &Value) -> bool {
    match value.get("status") {
        None => false,
        Some(Value::Number(n)) => n.as_i64() != Some(0),
        Some(Value::String(s)) => s != "0",
        Some(_) => true,
    }
}

fn format_wda_error(value: &Value) -> String {
    value
        .get("value")
        .and_then(|value| {
            value
                .get("message")
                .or_else(|| value.get("error"))
                .and_then(Value::as_str)
                .or_else(|| value.as_str())
        })
        .unwrap_or("unknown WDA error")
        .to_string()
}

fn normalize_button_name(name: &str) -> &str {
    match name
        .trim()
        .to_ascii_lowercase()
        .replace(['-', '_'], "")
        .as_str()
    {
        "home" => "home",
        "volumeup" | "volup" | "volumeupbutton" => "volumeUp",
        "volumedown" | "voldown" | "volumedownbutton" => "volumeDown",
        "lock" | "lockscreen" | "sleep" | "power" => "lock",
        _ => name,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_both_legacy_and_w3c_element_ids() {
        assert_eq!(
            element_id(&serde_json::json!({ "value": { "ELEMENT": "legacy" }})),
            Some("legacy")
        );
        assert_eq!(
            element_id(&serde_json::json!({
                "value": { "element-6066-11e4-a52e-4f735466cecf": "w3c" }
            })),
            Some("w3c")
        );
    }

    #[test]
    fn session_payload_sets_bundle_id_in_both_capability_shapes() {
        let payload = session_payload(Some("com.example.Aut"));
        assert_eq!(
            payload["capabilities"]["alwaysMatch"]["bundleId"],
            "com.example.Aut"
        );
        assert_eq!(
            payload["desiredCapabilities"]["bundleId"],
            "com.example.Aut"
        );
    }
}
