use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use anyhow::{anyhow, bail, Context, Result};
use base64::Engine;
use ios_core::pasteboard::{self, DataInclusionPolicy, PasteboardClient, PasteboardEvent};
use ios_core::pasteboard::{PasteboardPayload, PasteboardSnapshot, PasteboardWriteItem};
use ios_core::{connect, ConnectOptions, TunMode, XpcValue};
use sha2::{Digest, Sha256};
use tokio::io::AsyncReadExt;

const MAX_CLI_DATA_BYTES: usize = 64 * 1024 * 1024;

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
        #[arg(long, help = "Show the full wire snapshot instead of text")]
        raw: bool,
        #[arg(
            long,
            default_value = "resolved",
            value_name = "POLICY",
            help = "Data policy: resolved, promised, matchsource, promisesecondary, threshold:N"
        )]
        policy: String,
        #[arg(
            long,
            help = "Include raw bytes as base64 (otherwise show only UTI, size and SHA-256)"
        )]
        show_data: bool,
    },
    /// Replace the named pasteboard with UTF-8 text, a URL, or UTI=base64 data items
    Set {
        #[arg(
            value_name = "TEXT",
            help = "Text to set; if omitted, read UTF-8 text from stdin"
        )]
        text: Option<String>,
        #[arg(
            long,
            value_name = "URL",
            help = "Set the value under the public.url UTI"
        )]
        url: Option<String>,
        #[arg(
            long = "uti",
            value_name = "UTI",
            action = clap::ArgAction::Append,
            help = "Additional UTI for text/URL; repeat for multiple representations"
        )]
        uti: Vec<String>,
        #[arg(
            long = "data",
            value_name = "UTI=BASE64",
            action = clap::ArgAction::Append,
            help = "Raw representation; repeat for multiple UTI=base64 values"
        )]
        data: Vec<String>,
        #[arg(
            long,
            help = "Require --data and treat all payloads as base64 raw data"
        )]
        raw: bool,
        #[arg(
            long,
            default_value = pasteboard::GENERAL_PASTEBOARD,
            help = "Pasteboard name"
        )]
        pasteboard: String,
    },
    /// Resolve one promised item and print a bounded summary or write its bytes
    Resolve {
        #[arg(value_name = "ITEM_INDEX")]
        item_index: i64,
        #[arg(value_name = "UTI")]
        uti: String,
        #[arg(
            long,
            default_value = pasteboard::GENERAL_PASTEBOARD,
            help = "Pasteboard name"
        )]
        pasteboard: String,
        #[arg(
            long,
            value_name = "PATH",
            help = "Atomically write resolved bytes with owner-only permissions"
        )]
        out: Option<PathBuf>,
        #[arg(
            long,
            default_value = "matchsource",
            value_name = "POLICY",
            help = "Pull policy before resolving: resolved, promised, matchsource, promisesecondary, threshold:N"
        )]
        policy: String,
        #[arg(long, help = "Print resolved bytes as base64")]
        show_data: bool,
        #[arg(
            long,
            help = "Enable experimental RESOLVE support (not implemented by go-ios/pymobiledevice3)"
        )]
        experimental: bool,
    },
    /// Subscribe to AUTONOTIFY/PUSH pasteboard changes
    Watch {
        #[arg(
            long,
            default_value = pasteboard::GENERAL_PASTEBOARD,
            help = "Pasteboard name"
        )]
        pasteboard: String,
        #[arg(
            long,
            default_value = "resolved",
            value_name = "POLICY",
            help = "Push data policy: resolved, promised, matchsource, promisesecondary, threshold:N"
        )]
        policy: String,
        #[arg(long, help = "Include inline bytes as base64 in JSON/human output")]
        show_data: bool,
        #[arg(
            long,
            help = "Enable experimental AUTONOTIFY/PUSH support (not implemented by go-ios/pymobiledevice3)"
        )]
        experimental: bool,
    },
    /// Resolve and export one item (an explicit alias for `resolve --out`)
    Export {
        #[arg(value_name = "ITEM_INDEX")]
        item_index: i64,
        #[arg(value_name = "UTI")]
        uti: String,
        #[arg(value_name = "PATH")]
        out: PathBuf,
        #[arg(
            long,
            default_value = pasteboard::GENERAL_PASTEBOARD,
            help = "Pasteboard name"
        )]
        pasteboard: String,
        #[arg(
            long,
            default_value = "matchsource",
            value_name = "POLICY",
            help = "Pull policy before resolving"
        )]
        policy: String,
        #[arg(
            long,
            help = "Enable experimental RESOLVE support (not implemented by go-ios/pymobiledevice3)"
        )]
        experimental: bool,
    },
}

impl PasteboardCmd {
    pub async fn run(self, udid: Option<String>, json_output: bool) -> Result<()> {
        self.sub.validate_experimental()?;
        let udid = udid.ok_or_else(|| anyhow!("--udid required for pasteboard"))?;
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
            PasteboardSub::Get {
                pasteboard,
                raw,
                policy,
                show_data,
            } => {
                let policy = parse_policy(&policy)?;
                if raw {
                    // Keep raw output represented by the direct PULL reply for
                    // every policy.  Converting a typed snapshot back to XPC
                    // would lose envelope metadata and is not a valid wire
                    // representation.
                    let reply = client.get_named_with_policy(&pasteboard, policy).await?;
                    if show_data {
                        println!(
                            "{}",
                            serde_json::to_string_pretty(&xpc_value_to_json(&reply))?
                        );
                    } else {
                        println!(
                            "{}",
                            serde_json::to_string_pretty(&xpc_value_to_redacted_json(&reply))?
                        );
                    }
                } else {
                    let snapshot = client.get_with_policy(&pasteboard, policy).await?;
                    render_get(&pasteboard, &snapshot, json_output, show_data)?;
                }
            }
            PasteboardSub::Set {
                text,
                url,
                uti,
                data,
                raw,
                pasteboard,
            } => {
                let item = build_cli_item(text, url, uti, data, raw).await?;
                let bytes = item.data.values().map(|value| value.len()).sum::<usize>();
                let representations = item.data.len();
                client.set_items(&pasteboard, &[item], None).await?;
                if json_output {
                    println!(
                        "{}",
                        serde_json::to_string_pretty(&serde_json::json!({
                            "operation": "set",
                            "pasteboard": pasteboard,
                            "representations": representations,
                            "bytes": bytes,
                        }))?
                    );
                } else {
                    println!("Set {bytes} byte(s) across one item on pasteboard '{pasteboard}'");
                }
            }
            PasteboardSub::Resolve {
                item_index,
                uti,
                pasteboard,
                out,
                policy,
                show_data,
                experimental: _,
            } => {
                resolve_and_render(
                    &mut client,
                    ResolveRenderOptions {
                        pasteboard_name: &pasteboard,
                        item_index,
                        uti: &uti,
                        policy_spec: &policy,
                        out: out.as_deref(),
                        show_data,
                        json_output,
                    },
                )
                .await?;
            }
            PasteboardSub::Export {
                item_index,
                uti,
                out,
                pasteboard,
                policy,
                experimental: _,
            } => {
                resolve_and_render(
                    &mut client,
                    ResolveRenderOptions {
                        pasteboard_name: &pasteboard,
                        item_index,
                        uti: &uti,
                        policy_spec: &policy,
                        out: Some(&out),
                        show_data: false,
                        json_output,
                    },
                )
                .await?;
            }
            PasteboardSub::Watch {
                pasteboard,
                policy,
                show_data,
                experimental: _,
            } => {
                let policy = parse_policy(&policy)?;
                let mut subscription = client.subscribe(&pasteboard, Some(policy)).await?;
                loop {
                    tokio::select! {
                        event = subscription.next_event() => {
                            match event {
                                Ok(event) => render_event(&event, show_data, json_output)?,
                                Err(error) => {
                                    let _ = subscription.unsubscribe().await;
                                    return Err(error.into());
                                }
                            }
                        }
                        signal = tokio::signal::ctrl_c() => {
                            signal.context("waiting for Ctrl-C")?;
                            subscription.unsubscribe().await?;
                            break;
                        }
                    }
                }
            }
        }

        Ok(())
    }
}

impl PasteboardSub {
    fn validate_experimental(&self) -> Result<()> {
        let (name, experimental) = match self {
            Self::Resolve { experimental, .. } => ("resolve", *experimental),
            Self::Watch { experimental, .. } => ("watch", *experimental),
            Self::Export { experimental, .. } => ("export", *experimental),
            Self::Get { .. } | Self::Set { .. } => return Ok(()),
        };
        if !experimental {
            bail!(
                "pasteboard {name} is experimental: upstream go-ios/pymobiledevice3 do not implement this verb; pass --experimental to continue"
            );
        }
        Ok(())
    }
}

async fn build_cli_item(
    text: Option<String>,
    url: Option<String>,
    utis: Vec<String>,
    raw_values: Vec<String>,
    raw: bool,
) -> Result<PasteboardWriteItem> {
    if raw && raw_values.is_empty() {
        bail!("--raw requires at least one --data UTI=BASE64 value");
    }
    if !raw_values.is_empty() && (text.is_some() || url.is_some()) {
        bail!("--data cannot be combined with text or --url");
    }
    if !raw_values.is_empty() && !utis.is_empty() {
        bail!("--data cannot be combined with --uti");
    }
    if text.is_some() && url.is_some() {
        bail!("text and --url are mutually exclusive");
    }
    if !raw_values.is_empty() {
        let mut types = Vec::with_capacity(raw_values.len());
        let mut data = Vec::with_capacity(raw_values.len());
        let mut total_bytes = 0usize;
        for spec in raw_values {
            let (uti, encoded) = spec
                .split_once('=')
                .ok_or_else(|| anyhow!("--data must use UTI=BASE64"))?;
            if uti.is_empty() {
                bail!("--data UTI cannot be empty");
            }
            if encoded.len() > (MAX_CLI_DATA_BYTES / 3) * 4 + 4 {
                bail!(
                    "--data for UTI {uti:?} exceeds the {} byte limit",
                    MAX_CLI_DATA_BYTES
                );
            }
            let bytes = base64::engine::general_purpose::STANDARD
                .decode(encoded)
                .with_context(|| format!("invalid base64 data for UTI {uti:?}"))?;
            if types.iter().any(|existing| existing == uti) {
                bail!("duplicate --data UTI {uti:?}");
            }
            total_bytes = total_bytes
                .checked_add(bytes.len())
                .ok_or_else(|| anyhow!("pasteboard data size overflow"))?;
            if total_bytes > MAX_CLI_DATA_BYTES {
                bail!(
                    "pasteboard data exceeds the {} byte limit",
                    MAX_CLI_DATA_BYTES
                );
            }
            data.push((uti.to_owned(), bytes::Bytes::from(bytes)));
            types.push(uti.to_owned());
        }
        return Ok(PasteboardWriteItem::new(types, data.into_iter().collect()));
    }

    let (value, default_uti) = match (text, url) {
        (Some(text), None) => (text, pasteboard::UTI_UTF8_PLAIN_TEXT),
        (None, Some(url)) => (url, pasteboard::UTI_URL),
        (None, None) => (read_stdin_text().await?, pasteboard::UTI_UTF8_PLAIN_TEXT),
        (Some(_), Some(_)) => unreachable!("validated above"),
    };
    let mut types = if default_uti == pasteboard::UTI_UTF8_PLAIN_TEXT {
        vec![
            pasteboard::UTI_UTF8_PLAIN_TEXT.to_owned(),
            pasteboard::UTI_PLAIN_TEXT.to_owned(),
            pasteboard::UTI_TEXT.to_owned(),
        ]
    } else {
        vec![default_uti.to_owned()]
    };
    for uti in utis {
        if uti.is_empty() {
            bail!("--uti cannot be empty");
        }
        if types.iter().any(|existing| existing == &uti) {
            bail!("duplicate --uti {uti:?}");
        }
        types.push(uti);
    }
    if types.iter().any(String::is_empty) {
        bail!("--uti cannot be empty");
    }
    if value.len() > MAX_CLI_DATA_BYTES {
        bail!(
            "pasteboard data exceeds the {} byte limit",
            MAX_CLI_DATA_BYTES
        );
    }
    let bytes = bytes::Bytes::copy_from_slice(value.as_bytes());
    let data = types
        .iter()
        .map(|uti| (uti.clone(), bytes.clone()))
        .collect();
    Ok(PasteboardWriteItem::new(types, data))
}

async fn read_stdin_text() -> Result<String> {
    let mut bytes = Vec::new();
    let mut input = tokio::io::stdin().take((MAX_CLI_DATA_BYTES + 1) as u64);
    input.read_to_end(&mut bytes).await?;
    if bytes.len() > MAX_CLI_DATA_BYTES {
        bail!(
            "stdin pasteboard data exceeds the {} byte limit",
            MAX_CLI_DATA_BYTES
        );
    }
    String::from_utf8(bytes).context("stdin pasteboard data is not valid UTF-8")
}

fn parse_policy(value: &str) -> Result<DataInclusionPolicy> {
    match value.to_ascii_lowercase().as_str() {
        "resolved" | "allresolved" => Ok(DataInclusionPolicy::AllResolved),
        "promised" | "allpromised" => Ok(DataInclusionPolicy::AllPromised),
        "matchsource" | "match-source" => Ok(DataInclusionPolicy::MatchSource),
        "promisesecondary" | "promise-secondary" => {
            Ok(DataInclusionPolicy::PromiseSecondary)
        }
        value if value.starts_with("threshold:") => {
            let threshold = value["threshold:".len()..]
                .parse::<i64>()
                .context("threshold policy must use threshold:<non-negative integer>")?;
            let policy = DataInclusionPolicy::Threshold(threshold);
            policy.validate_for_cli()?;
            Ok(policy)
        }
        other => bail!(
            "unknown pasteboard policy {other:?}; use resolved, promised, matchsource, promisesecondary, or threshold:N"
        ),
    }
}

trait PolicyValidation {
    fn validate_for_cli(self) -> Result<()>;
}

impl PolicyValidation for DataInclusionPolicy {
    fn validate_for_cli(self) -> Result<()> {
        if let DataInclusionPolicy::Threshold(value) = self {
            if value < 0 {
                bail!("threshold policy must be non-negative");
            }
        }
        Ok(())
    }
}

struct ResolveRenderOptions<'a> {
    pasteboard_name: &'a str,
    item_index: i64,
    uti: &'a str,
    policy_spec: &'a str,
    out: Option<&'a Path>,
    show_data: bool,
    json_output: bool,
}

async fn resolve_and_render(
    client: &mut PasteboardClient,
    options: ResolveRenderOptions<'_>,
) -> Result<()> {
    let ResolveRenderOptions {
        pasteboard_name,
        item_index,
        uti,
        policy_spec,
        out,
        show_data,
        json_output,
    } = options;
    if item_index < 0 {
        bail!("item index must be non-negative");
    }
    let policy = parse_policy(policy_spec)?;
    let snapshot = client.get_with_policy(pasteboard_name, policy).await?;
    let item = snapshot
        .items
        .get(usize::try_from(item_index).context("item index does not fit in usize")?)
        .ok_or_else(|| anyhow!("pasteboard item index {item_index} is out of range"))?;
    let entry = item
        .data
        .iter()
        .find(|entry| entry.uti == uti)
        .ok_or_else(|| anyhow!("pasteboard item {item_index} has no UTI {uti:?}"))?;
    let bytes = match &entry.payload {
        PasteboardPayload::Inline(bytes) => Some(bytes.clone()),
        PasteboardPayload::Promised { .. } => {
            client
                .resolve_data_for_snapshot(pasteboard_name, &snapshot, item_index, uti)
                .await?
                .data
        }
        PasteboardPayload::Error(error) => {
            bail!("device reported promise error for {uti:?}: {error}")
        }
    };
    let Some(bytes) = bytes else {
        bail!("device returned no data for item {item_index} UTI {uti:?}");
    };

    if let Some(path) = out {
        write_atomic_private(path, bytes.as_ref())?;
    }
    let summary = serde_json::json!({
        "operation": "resolve",
        "pasteboard": pasteboard_name,
        "item_index": item_index,
        "uti": uti,
        "size": bytes.len(),
        "sha256": sha256_hex(bytes.as_ref()),
        "out": out.map(|path| path.display().to_string()),
        "data_base64": show_data.then(|| base64::engine::general_purpose::STANDARD.encode(bytes.as_ref())),
    });
    if json_output || out.is_some() || show_data {
        println!("{}", serde_json::to_string_pretty(&summary)?);
    } else {
        println!(
            "{uti}: {} byte(s), sha256={}",
            bytes.len(),
            sha256_hex(bytes.as_ref())
        );
    }
    Ok(())
}

fn render_get(
    pasteboard: &str,
    snapshot: &PasteboardSnapshot,
    json_output: bool,
    show_data: bool,
) -> Result<()> {
    if json_output {
        println!(
            "{}",
            serde_json::to_string_pretty(&snapshot_to_json(snapshot, show_data))?
        );
    } else if let Some(text) = snapshot.text() {
        print!("{text}");
    } else if let Some(bytes) = snapshot.data_for_uti(pasteboard::UTI_URL) {
        if let Ok(url) = std::str::from_utf8(bytes.as_ref()) {
            println!("{url}");
        } else {
            println!(
                "Pasteboard '{pasteboard}' contains {} byte(s) of non-UTF-8 URL data (sha256={})",
                bytes.len(),
                sha256_hex(bytes.as_ref())
            );
        }
    } else {
        println!("{}", human_snapshot_summary(snapshot, show_data));
    }
    Ok(())
}

fn render_event(event: &PasteboardEvent, show_data: bool, json_output: bool) -> Result<()> {
    match event {
        PasteboardEvent::Push(push) => {
            if json_output {
                println!(
                    "{}",
                    serde_json::to_string(&snapshot_to_json(&push.snapshot, show_data))?
                );
            } else {
                println!("{}", human_snapshot_summary(&push.snapshot, show_data));
            }
        }
        PasteboardEvent::Data(data) => {
            let value = serde_json::json!({
                "event": "data",
                "item_index": data.item_index,
                "uti": data.uti,
                "size": data.data.as_ref().map(|bytes| bytes.len()),
                "sha256": data.data.as_ref().map(|bytes| sha256_hex(bytes.as_ref())),
                "data_base64": show_data.then(|| data.data.as_ref().map(|bytes| base64::engine::general_purpose::STANDARD.encode(bytes.as_ref()))).flatten(),
                "error": data.error,
            });
            if json_output {
                println!("{}", serde_json::to_string(&value)?);
            } else {
                println!("{value}");
            }
        }
    }
    Ok(())
}

fn snapshot_to_json(snapshot: &PasteboardSnapshot, show_data: bool) -> serde_json::Value {
    let items = snapshot
        .items
        .iter()
        .map(|item| {
            let data = item
                .data
                .iter()
                .map(|entry| {
                    let mut value = match &entry.payload {
                        PasteboardPayload::Inline(bytes) => serde_json::json!({
                            "state": "inline",
                            "size": bytes.len(),
                            "sha256": sha256_hex(bytes.as_ref()),
                        }),
                        PasteboardPayload::Promised { size } => serde_json::json!({
                            "state": "promised",
                            "size": size,
                        }),
                        PasteboardPayload::Error(error) => serde_json::json!({
                            "state": "error",
                            "error": error,
                        }),
                    };
                    if show_data {
                        if let PasteboardPayload::Inline(bytes) = &entry.payload {
                            value["data_base64"] = serde_json::Value::String(
                                base64::engine::general_purpose::STANDARD.encode(bytes.as_ref()),
                            );
                        }
                    }
                    serde_json::json!({"uti": entry.uti, "payload": value})
                })
                .collect::<Vec<_>>();
            serde_json::json!({
                "index": item.index,
                "types": item.types,
                "data": data,
            })
        })
        .collect::<Vec<_>>();
    let mut value = serde_json::json!({
        "command": snapshot.command,
        "pasteboard": snapshot.pasteboard_name,
        "change_count": snapshot.change_count,
        "uuid": snapshot.uuid.map(|uuid| UuidJson(uuid).to_string()),
        "items": items,
    });
    if show_data {
        if let Some(metadata) = &snapshot.metadata {
            value["metadata"] = xpc_value_to_json(metadata);
        }
        if let Some(source_metadata) = &snapshot.source_metadata {
            value["source_metadata"] = xpc_value_to_json(source_metadata);
        }
    }
    value
}

struct UuidJson([u8; 16]);

impl std::fmt::Display for UuidJson {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&uuid::Uuid::from_bytes(self.0).to_string())
    }
}

fn human_snapshot_summary(snapshot: &PasteboardSnapshot, show_data: bool) -> String {
    let mut lines = Vec::new();
    if let Some(change_count) = snapshot.change_count {
        lines.push(format!("changeCount: {change_count}"));
    }
    for item in &snapshot.items {
        for entry in &item.data {
            let line = match &entry.payload {
                PasteboardPayload::Inline(bytes) => {
                    let mut line = format!(
                        "[{}] {}: {} byte(s), sha256={}",
                        item.index,
                        entry.uti,
                        bytes.len(),
                        sha256_hex(bytes.as_ref())
                    );
                    if show_data {
                        line.push_str(&format!(
                            ", base64={}",
                            base64::engine::general_purpose::STANDARD.encode(bytes.as_ref())
                        ));
                    }
                    line
                }
                PasteboardPayload::Promised { size } => {
                    format!(
                        "[{}] {}: promised ({})",
                        item.index,
                        entry.uti,
                        size.map_or_else(|| "size unknown".into(), |size| format!("size {size}"))
                    )
                }
                PasteboardPayload::Error(error) => {
                    format!("[{}] {}: error: {error}", item.index, entry.uti)
                }
            };
            lines.push(line);
        }
    }
    if lines.is_empty() {
        "pasteboard is empty".to_owned()
    } else {
        lines.join("\n")
    }
}

fn sha256_hex(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    hex::encode(digest)
}

static TEMP_FILE_COUNTER: AtomicU64 = AtomicU64::new(0);

fn write_atomic_private(path: &Path, bytes: &[u8]) -> Result<()> {
    let parent = path.parent().unwrap_or_else(|| Path::new("."));
    if !parent.exists() {
        bail!("output directory does not exist: {}", parent.display());
    }
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_symlink() => {
            bail!("refusing to overwrite symlink output {}", path.display());
        }
        Ok(_) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(error.into()),
    }
    let name = path
        .file_name()
        .ok_or_else(|| anyhow!("output path has no file name"))?
        .to_string_lossy();
    let mut temporary = None;
    for _ in 0..8 {
        let suffix = TEMP_FILE_COUNTER.fetch_add(1, Ordering::Relaxed);
        let candidate = parent.join(format!(".{name}.ios-pasteboard.{suffix}.tmp"));
        let mut options = OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        match options.open(&candidate) {
            Ok(mut file) => {
                if let Err(error) = file.write_all(bytes).and_then(|_| file.sync_all()) {
                    drop(file);
                    let _ = fs::remove_file(&candidate);
                    return Err(error.into());
                }
                temporary = Some(candidate);
                break;
            }
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(error.into()),
        }
    }
    let temporary =
        temporary.ok_or_else(|| anyhow!("could not allocate a private temporary output"))?;
    let result = fs::rename(&temporary, path).map_err(anyhow::Error::from);
    if let Err(error) = result {
        let _ = fs::remove_file(&temporary);
        return Err(error);
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

/// Serialize an arbitrary wire reply without exposing inline pasteboard
/// bytes. Unlike the typed snapshot serializer this retains unknown fields
/// and the complete direct-service envelope, so `get --raw` remains useful
/// for forward-compatible inspection even with its safe default.
fn xpc_value_to_redacted_json(value: &XpcValue) -> serde_json::Value {
    match value {
        XpcValue::Data(value) => serde_json::json!({
            "size": value.len(),
            "sha256": sha256_hex(value),
        }),
        XpcValue::Array(values) => {
            serde_json::Value::Array(values.iter().map(xpc_value_to_redacted_json).collect())
        }
        XpcValue::Dictionary(values) => serde_json::Value::Object(
            values
                .iter()
                .map(|(key, value)| (key.clone(), xpc_value_to_redacted_json(value)))
                .collect(),
        ),
        XpcValue::FileTransfer { msg_id, data } => serde_json::json!({
            "msg_id": msg_id,
            "data": xpc_value_to_redacted_json(data),
        }),
        other => xpc_value_to_json(other),
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
    fn parses_get_with_raw_policy_and_named_pasteboard() {
        let cli = TestCli::parse_from([
            "pasteboard",
            "get",
            "--pasteboard",
            "general",
            "--raw",
            "--policy",
            "promised",
            "--show-data",
        ]);
        match cli.command {
            PasteboardSub::Get {
                pasteboard,
                raw,
                policy,
                show_data,
            } => {
                assert_eq!(pasteboard, "general");
                assert!(raw);
                assert_eq!(policy, "promised");
                assert!(show_data);
            }
            _ => panic!("expected get"),
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
                uti,
                data,
                raw,
            } => {
                assert_eq!(text.as_deref(), Some("hello"));
                assert!(url.is_none());
                assert_eq!(pasteboard, "general");
                assert!(uti.is_empty());
                assert!(data.is_empty());
                assert!(!raw);
            }
            _ => panic!("expected set"),
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
    fn parses_watch_resolve_and_export() {
        let watch = TestCli::parse_from([
            "pasteboard",
            "watch",
            "--policy",
            "threshold:32",
            "--experimental",
        ])
        .command;
        assert!(matches!(
            &watch,
            PasteboardSub::Watch {
                experimental: true,
                ..
            }
        ));
        assert!(watch.validate_experimental().is_ok());

        let resolve =
            TestCli::parse_from(["pasteboard", "resolve", "2", "public.png", "--show-data"])
                .command;
        assert!(matches!(
            &resolve,
            PasteboardSub::Resolve {
                item_index: 2,
                experimental: false,
                ..
            }
        ));
        assert!(resolve.validate_experimental().is_err());

        let export = TestCli::parse_from([
            "pasteboard",
            "export",
            "2",
            "public.png",
            "/tmp/x",
            "--experimental",
        ])
        .command;
        assert!(matches!(
            &export,
            PasteboardSub::Export {
                item_index: 2,
                experimental: true,
                ..
            }
        ));
    }

    #[tokio::test]
    async fn experimental_commands_are_rejected_before_connection() {
        let command = TestCli::parse_from(["pasteboard", "watch"]).command;
        let error = PasteboardCmd { sub: command }
            .run(None, true)
            .await
            .expect_err("watch without opt-in must stop before device access");
        assert!(error.to_string().contains("--experimental"));
        assert!(error.to_string().contains("watch"));
    }

    #[test]
    fn empty_text_argument_is_preserved_by_clap() {
        let cli = TestCli::parse_from(["pasteboard", "set", ""]);
        match cli.command {
            PasteboardSub::Set { text, .. } => assert_eq!(text.as_deref(), Some("")),
            _ => panic!("expected set"),
        }
    }

    #[tokio::test]
    async fn policy_and_raw_item_parsers_are_bounded_and_unicode_safe() {
        assert_eq!(
            parse_policy("threshold:42").unwrap(),
            DataInclusionPolicy::Threshold(42)
        );
        assert!(parse_policy("threshold:-1").is_err());
        assert!(parse_policy("nope").is_err());

        let item = build_cli_item(
            None,
            None,
            Vec::new(),
            vec!["public.data=AP8=".into(), "public.url=4pyT".into()],
            true,
        )
        .await
        .unwrap();
        assert_eq!(item.types, vec!["public.data", "public.url"]);
        assert_eq!(
            item.data["public.data"],
            bytes::Bytes::from_static(b"\0\xff")
        );

        let item = build_cli_item(
            Some("hello".into()),
            None,
            vec!["public.markdown".into()],
            Vec::new(),
            false,
        )
        .await
        .unwrap();
        assert_eq!(
            item.types,
            vec![
                pasteboard::UTI_UTF8_PLAIN_TEXT.to_owned(),
                pasteboard::UTI_PLAIN_TEXT.to_owned(),
                pasteboard::UTI_TEXT.to_owned(),
                "public.markdown".to_owned()
            ]
        );
        assert!(build_cli_item(
            Some("hello".into()),
            None,
            vec![pasteboard::UTI_TEXT.into()],
            Vec::new(),
            false,
        )
        .await
        .is_err());
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
    fn raw_json_redaction_preserves_wire_envelope_and_hides_data() {
        let item = XpcValue::Dictionary(IndexMap::from([(
            "data".into(),
            XpcValue::Data(bytes::Bytes::from_static(b"secret")),
        )]));
        let pasteboard = XpcValue::Dictionary(IndexMap::from([(
            "items".into(),
            XpcValue::Array(vec![item]),
        )]));
        let value = XpcValue::Dictionary(IndexMap::from([
            ("command".into(), XpcValue::String("PULL_REPLY".into())),
            (
                "metadata".into(),
                XpcValue::Dictionary(IndexMap::from([(
                    "provider".into(),
                    XpcValue::String("com.example.test".into()),
                )])),
            ),
            ("pasteboard".into(), pasteboard),
        ]));
        let redacted = xpc_value_to_redacted_json(&value);
        assert_eq!(redacted["command"], "PULL_REPLY");
        assert_eq!(redacted["metadata"]["provider"], "com.example.test");
        assert_eq!(redacted["pasteboard"]["items"][0]["data"]["size"], 6);
        assert!(redacted["pasteboard"]["items"][0]["data"]["sha256"].is_string());
        assert!(!redacted.to_string().contains("c2VjcmV0"));
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

    #[test]
    fn default_snapshot_json_redacts_inline_content() {
        let mut data = IndexMap::new();
        data.insert(
            pasteboard::UTI_TEXT.into(),
            XpcValue::Dictionary(IndexMap::from([(
                "data".into(),
                XpcValue::Data(bytes::Bytes::from_static(b"secret")),
            )])),
        );
        let snapshot = PasteboardSnapshot::from_xpc(&XpcValue::Dictionary(IndexMap::from([(
            "items".into(),
            XpcValue::Array(vec![XpcValue::Dictionary(IndexMap::from([
                (
                    "types".into(),
                    XpcValue::Array(vec![XpcValue::String(pasteboard::UTI_TEXT.into())]),
                ),
                ("data".into(), XpcValue::Dictionary(data)),
            ]))]),
        )])))
        .unwrap();
        let redacted = snapshot_to_json(&snapshot, false).to_string();
        assert!(!redacted.contains("c2VjcmV0"));
        assert!(redacted.contains("sha256"));
        let shown = snapshot_to_json(&snapshot, true).to_string();
        assert!(shown.contains("c2VjcmV0"));
    }
}
