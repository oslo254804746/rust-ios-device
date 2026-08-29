use std::future::Future;
use std::path::PathBuf;
use std::pin::Pin;
use std::time::Duration;

use anyhow::Result;

use crate::cmd::connect::connect_by_ios_major;

type StopSignal = Pin<Box<dyn Future<Output = std::io::Result<()>> + Send>>;

fn stop_signal() -> StopSignal {
    Box::pin(tokio::signal::ctrl_c())
}

fn trace_deadline(timeout: Option<u64>) -> Result<Option<tokio::time::Instant>> {
    timeout
        .map(|seconds| {
            tokio::time::Instant::now()
                .checked_add(Duration::from_secs(seconds))
                .ok_or_else(|| anyhow::anyhow!("--timeout is too large"))
        })
        .transpose()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RunTermination {
    /// The operator requested a clean stop with Ctrl+C.
    Cancelled,
    /// The caller supplied deadline elapsed before the stage completed.
    TimedOut,
}

fn finish_termination(termination: RunTermination, timeout: Option<u64>) -> Result<()> {
    match termination {
        RunTermination::Cancelled => Ok(()),
        RunTermination::TimedOut => match timeout {
            Some(seconds) => Err(anyhow::anyhow!(
                "OS trace operation timed out after {seconds} seconds"
            )),
            None => Err(anyhow::anyhow!("OS trace operation timed out")),
        },
    }
}

/// Await one device lifecycle stage while sharing the stream's Ctrl+C signal
/// and one absolute timeout. Dropping the stage future closes/cancels the
/// current connection attempt or read; callers then drop any partially built
/// device/client as well.
async fn wait_for_trace_stage<F, T, E>(
    future: F,
    stop: &mut StopSignal,
    deadline: Option<tokio::time::Instant>,
) -> Result<std::result::Result<T, RunTermination>>
where
    F: Future<Output = std::result::Result<T, E>>,
    E: Into<anyhow::Error>,
{
    if let Some(deadline) = deadline {
        if deadline <= tokio::time::Instant::now() {
            return Ok(Err(RunTermination::TimedOut));
        }
        tokio::select! {
            _ = stop.as_mut() => Ok(Err(RunTermination::Cancelled)),
            _ = tokio::time::sleep_until(deadline) => Ok(Err(RunTermination::TimedOut)),
            result = future => result.map(Ok).map_err(Into::into),
        }
    } else {
        tokio::select! {
            _ = stop.as_mut() => Ok(Err(RunTermination::Cancelled)),
            result = future => result.map(Ok).map_err(Into::into),
        }
    }
}

#[derive(clap::Args)]
pub struct OsTraceCmd {
    #[command(subcommand)]
    sub: OsTraceSub,
}

#[derive(clap::Subcommand)]
#[allow(clippy::large_enum_variant)]
enum OsTraceSub {
    /// Show the process list reported by os_trace_relay
    Ps,
    /// Stream structured os_trace activity records until Ctrl+C or --count;
    /// --timeout reports a non-zero timeout error
    #[command(alias = "live")]
    Stream {
        /// Restrict records at the device to one PID
        #[arg(long, value_name = "PID")]
        pid: Option<u32>,
        /// Filter output by process executable/image name
        #[arg(long)]
        process: Option<String>,
        /// Comma-separated levels: default, info, debug, error, fault
        #[arg(long, default_value = "")]
        level: String,
        /// Filter output by subsystem substring
        #[arg(long)]
        subsystem: Option<String>,
        /// Filter output by category substring
        #[arg(long)]
        category: Option<String>,
        /// Only output messages containing all supplied texts (repeatable)
        #[arg(long = "match", action = clap::ArgAction::Append)]
        message_match: Vec<String>,
        /// Exclude messages containing any supplied text (repeatable)
        #[arg(long, action = clap::ArgAction::Append)]
        exclude: Vec<String>,
        /// Only output messages matching any supplied regular expression (repeatable)
        #[arg(long, action = clap::ArgAction::Append)]
        regex: Vec<String>,
        /// Apply text and regular-expression filters case-insensitively
        #[arg(long)]
        ignore_case: bool,
        /// Stop after this many matching records
        #[arg(long)]
        count: Option<u64>,
        /// Stop after this many seconds
        #[arg(long)]
        timeout: Option<u64>,
    },
    /// Save the device's raw PAX log archive without extracting it.
    Archive {
        /// Destination archive path. Existing files are atomically replaced.
        #[arg(value_name = "OUTPUT")]
        output: PathBuf,
        /// Maximum archive size requested from the device and enforced locally.
        #[arg(long)]
        size_limit: Option<u64>,
        /// Maximum age in days of entries requested from the device.
        #[arg(long)]
        age_limit: Option<u64>,
        /// Earliest entry as a Unix timestamp.
        #[arg(long)]
        start_time: Option<i64>,
        /// Stop after this many seconds, including connection and extraction.
        #[arg(long)]
        timeout: Option<u64>,
    },
    /// Fetch and safely extract the device's log archive into a new directory.
    Collect {
        /// Destination .logarchive directory. It must not already exist.
        #[arg(value_name = "OUTPUT")]
        output: PathBuf,
        /// Maximum archive size requested from the device and enforced locally.
        #[arg(long)]
        size_limit: Option<u64>,
        /// Maximum age in days of entries requested from the device.
        #[arg(long)]
        age_limit: Option<u64>,
        /// Earliest entry as a Unix timestamp.
        #[arg(long)]
        start_time: Option<i64>,
        /// Stop after this many seconds, including connection and extraction.
        #[arg(long)]
        timeout: Option<u64>,
    },
}

impl OsTraceCmd {
    pub async fn run(self, udid: Option<String>, json: bool) -> Result<()> {
        let udid = udid.ok_or_else(|| anyhow::anyhow!("--udid required for os-trace"))?;

        match self.sub {
            OsTraceSub::Ps => run_ps(&udid, json).await,
            OsTraceSub::Stream {
                pid,
                process,
                level,
                subsystem,
                category,
                message_match,
                exclude,
                regex,
                ignore_case,
                count,
                timeout,
            } => {
                run_stream(
                    &udid,
                    StreamArgs {
                        pid,
                        process,
                        level,
                        subsystem,
                        category,
                        message_match,
                        exclude,
                        regex,
                        ignore_case,
                        count,
                        timeout,
                    },
                    json,
                )
                .await
            }
            OsTraceSub::Archive {
                output,
                size_limit,
                age_limit,
                start_time,
                timeout,
            } => {
                run_archive(
                    &udid, output, size_limit, age_limit, start_time, timeout, json,
                )
                .await
            }
            OsTraceSub::Collect {
                output,
                size_limit,
                age_limit,
                start_time,
                timeout,
            } => {
                run_collect(
                    &udid, output, size_limit, age_limit, start_time, timeout, json,
                )
                .await
            }
        }
    }
}

fn archive_options(
    size_limit: Option<u64>,
    age_limit: Option<u64>,
    start_time: Option<i64>,
) -> ios_core::ostrace::ArchiveOptions {
    ios_core::ostrace::ArchiveOptions {
        size_limit,
        age_limit,
        start_time,
        max_total_bytes: size_limit.unwrap_or(ios_core::ostrace::DEFAULT_MAX_ARCHIVE_BYTES),
        ..Default::default()
    }
}

async fn connect_trace_client(
    udid: &str,
    stop: &mut StopSignal,
    deadline: Option<tokio::time::Instant>,
) -> Result<
    std::result::Result<
        ios_core::ostrace::OsTraceClient<ios_core::device::ServiceStream>,
        RunTermination,
    >,
> {
    let (device, _version) = match wait_for_trace_stage(
        connect_by_ios_major(udid, |major| major >= 17),
        stop,
        deadline,
    )
    .await?
    {
        Ok(value) => value,
        Err(termination) => return Ok(Err(termination)),
    };
    let stream = if device.rsd().is_some() {
        match wait_for_trace_stage(
            device.connect_rsd_service(ios_core::ostrace::SHIM_SERVICE_NAME),
            stop,
            deadline,
        )
        .await?
        {
            Ok(stream) => stream,
            Err(termination) => return Ok(Err(termination)),
        }
    } else {
        match wait_for_trace_stage(
            device.connect_service(ios_core::ostrace::SERVICE_NAME),
            stop,
            deadline,
        )
        .await?
        {
            Ok(stream) => stream,
            Err(termination) => return Ok(Err(termination)),
        }
    };
    Ok(Ok(ios_core::ostrace::OsTraceClient::new(stream)))
}

async fn run_archive(
    udid: &str,
    output: PathBuf,
    size_limit: Option<u64>,
    age_limit: Option<u64>,
    start_time: Option<i64>,
    timeout: Option<u64>,
    json: bool,
) -> Result<()> {
    let options = archive_options(size_limit, age_limit, start_time);
    let deadline = trace_deadline(timeout)?;
    let mut stop = stop_signal();
    let mut client = match connect_trace_client(udid, &mut stop, deadline).await? {
        Ok(client) => client,
        Err(termination) => return finish_termination(termination, timeout),
    };
    let stats = match wait_for_trace_stage(
        client.archive_to_path(&output, options),
        &mut stop,
        deadline,
    )
    .await?
    {
        Ok(stats) => stats,
        Err(termination) => return finish_termination(termination, timeout),
    };
    if json {
        println!(
            "{}",
            serde_json::to_string_pretty(&serde_json::json!({
                "operation": "archive",
                "format": "pax-tar",
                "output": output,
                "bytes": stats.bytes,
                "chunks": stats.chunks,
            }))?
        );
    } else {
        println!(
            "Saved raw PAX log archive to {} ({} bytes, {} chunks)",
            output.display(),
            stats.bytes,
            stats.chunks
        );
    }
    Ok(())
}

async fn run_collect(
    udid: &str,
    output: PathBuf,
    size_limit: Option<u64>,
    age_limit: Option<u64>,
    start_time: Option<i64>,
    timeout: Option<u64>,
    json: bool,
) -> Result<()> {
    let options = archive_options(size_limit, age_limit, start_time);
    let deadline = trace_deadline(timeout)?;
    let mut stop = stop_signal();
    let mut client = match connect_trace_client(udid, &mut stop, deadline).await? {
        Ok(client) => client,
        Err(termination) => return finish_termination(termination, timeout),
    };
    let stats =
        match wait_for_trace_stage(client.collect(&output, options), &mut stop, deadline).await? {
            Ok(stats) => stats,
            Err(termination) => return finish_termination(termination, timeout),
        };
    if json {
        println!(
            "{}",
            serde_json::to_string_pretty(&serde_json::json!({
                "operation": "collect",
                "format": "pax-tar",
                "output": output,
                "bytes": stats.bytes,
                "chunks": stats.chunks,
                "entries": stats.entries,
                "extracted_bytes": stats.extracted_bytes,
            }))?
        );
    } else {
        println!(
            "Collected log archive in {} ({} entries, {} bytes)",
            output.display(),
            stats.entries,
            stats.extracted_bytes
        );
    }
    Ok(())
}

async fn run_ps(udid: &str, json: bool) -> Result<()> {
    let mut stop = stop_signal();
    let (device, _version) = match wait_for_trace_stage(
        connect_by_ios_major(udid, |major| major >= 17),
        &mut stop,
        None,
    )
    .await?
    {
        Ok(value) => value,
        Err(termination) => return finish_termination(termination, None),
    };
    let stream = if device.rsd().is_some() {
        match wait_for_trace_stage(
            device.connect_rsd_service(ios_core::ostrace::SHIM_SERVICE_NAME),
            &mut stop,
            None,
        )
        .await?
        {
            Ok(stream) => stream,
            Err(termination) => return finish_termination(termination, None),
        }
    } else {
        match wait_for_trace_stage(
            device.connect_service(ios_core::ostrace::SERVICE_NAME),
            &mut stop,
            None,
        )
        .await?
        {
            Ok(stream) => stream,
            Err(termination) => return finish_termination(termination, None),
        }
    };
    let mut client = ios_core::ostrace::OsTraceClient::new(stream);
    let response = match wait_for_trace_stage(client.get_pid_list(), &mut stop, None).await? {
        Ok(response) => response,
        Err(termination) => return finish_termination(termination, None),
    };
    if json {
        println!(
            "{}",
            serde_json::to_string_pretty(&plist_to_json(&plist::Value::Dictionary(
                response.clone(),
            )))?
        );
    } else {
        print_pid_table(&response);
    }
    Ok(())
}

struct StreamArgs {
    pid: Option<u32>,
    process: Option<String>,
    level: String,
    subsystem: Option<String>,
    category: Option<String>,
    message_match: Vec<String>,
    exclude: Vec<String>,
    regex: Vec<String>,
    ignore_case: bool,
    count: Option<u64>,
    timeout: Option<u64>,
}

async fn run_stream(udid: &str, args: StreamArgs, json: bool) -> Result<()> {
    let level_filter = ios_core::ostrace::parse_level_filter(&args.level)
        .map_err(|error| anyhow::anyhow!("invalid --level value: {error}"))?;
    let pid = args
        .pid
        .map(|pid| {
            i32::try_from(pid)
                .map_err(|_| anyhow::anyhow!("--pid must be no greater than {}", i32::MAX))
        })
        .transpose()?;
    let filter_options = ios_core::ostrace::MessageFilterOptions {
        levels: level_filter.client_levels,
        process: args.process,
        subsystem: args.subsystem,
        category: args.category,
        matches: args.message_match,
        excludes: args.exclude,
        regex: args.regex,
        ignore_case: args.ignore_case,
    };
    let client_filter = filter_options
        .compile()
        .map_err(|error| anyhow::anyhow!("invalid OS trace filter: {error}"))?;

    let deadline = trace_deadline(args.timeout)?;
    let mut stop = stop_signal();
    let (device, _version) = match wait_for_trace_stage(
        connect_by_ios_major(udid, |major| major >= 17),
        &mut stop,
        deadline,
    )
    .await?
    {
        Ok(value) => value,
        Err(termination) => return finish_termination(termination, args.timeout),
    };
    let stream = if device.rsd().is_some() {
        match wait_for_trace_stage(
            device.connect_rsd_service(ios_core::ostrace::SHIM_SERVICE_NAME),
            &mut stop,
            deadline,
        )
        .await?
        {
            Ok(stream) => stream,
            Err(termination) => return finish_termination(termination, args.timeout),
        }
    } else {
        match wait_for_trace_stage(
            device.connect_service(ios_core::ostrace::SERVICE_NAME),
            &mut stop,
            deadline,
        )
        .await?
        {
            Ok(stream) => stream,
            Err(termination) => return finish_termination(termination, args.timeout),
        }
    };
    let options = ios_core::ostrace::TraceOptions {
        pid: pid.unwrap_or(-1),
        message_filter: level_filter.message_filter,
        stream_flags: level_filter.stream_flags,
    };
    let mut trace = match wait_for_trace_stage(
        ios_core::ostrace::OsTraceClient::new(stream).start_activity(options),
        &mut stop,
        deadline,
    )
    .await?
    {
        Ok(trace) => trace,
        Err(termination) => return finish_termination(termination, args.timeout),
    };

    eprintln!("Streaming os_trace activity (Ctrl+C to stop)...");
    let mut received = 0u64;

    loop {
        let result = match wait_for_trace_stage(
            trace.next_filtered_compiled(&client_filter),
            &mut stop,
            deadline,
        )
        .await?
        {
            Ok(result) => result,
            Err(termination) => return finish_termination(termination, args.timeout),
        };
        let Some(entry) = result else { break };
        received = received.saturating_add(1);
        if json {
            println!("{}", serde_json::to_string(&trace_entry_to_json(&entry))?);
        } else {
            println!("{}", format_trace_entry(&entry));
        }
        if args
            .count
            .is_some_and(|count| count > 0 && received >= count)
        {
            break;
        }
    }
    Ok(())
}

fn print_pid_table(response: &plist::Dictionary) {
    let Some(rows) = pid_rows(response) else {
        println!(
            "{}",
            serde_json::to_string_pretty(response).unwrap_or_default()
        );
        return;
    };

    println!("{:<8} NAME", "PID");
    println!("{}", "-".repeat(48));
    for (pid, name) in rows {
        println!("{:<8} {}", pid, name);
    }
}

fn pid_rows(response: &plist::Dictionary) -> Option<Vec<(u64, String)>> {
    let payload = response.get("Payload")?;
    let mut rows: Vec<(u64, String)> = match payload {
        plist::Value::Array(items) => items
            .iter()
            .filter_map(|item| {
                let dict = item.as_dictionary()?;
                let pid = dict
                    .get("PID")
                    .and_then(plist::Value::as_unsigned_integer)
                    .or_else(|| dict.get("Pid").and_then(plist::Value::as_unsigned_integer))
                    .or_else(|| dict.get("pid").and_then(plist::Value::as_unsigned_integer))
                    .unwrap_or_default();
                let name = process_name(dict).unwrap_or_default();
                Some((pid, name))
            })
            .collect(),
        plist::Value::Dictionary(processes) => processes
            .iter()
            .map(|(pid, value)| {
                let name = value
                    .as_dictionary()
                    .and_then(process_name)
                    .unwrap_or_default();
                (pid.parse().unwrap_or_default(), name)
            })
            .collect(),
        _ => return None,
    };
    if matches!(payload, plist::Value::Dictionary(_)) {
        rows.sort_by_key(|(pid, _)| *pid);
    }
    Some(rows)
}

fn process_name(dict: &plist::Dictionary) -> Option<String> {
    dict.get("Name")
        .and_then(plist::Value::as_string)
        .or_else(|| dict.get("ProcessName").and_then(plist::Value::as_string))
        .or_else(|| dict.get("name").and_then(plist::Value::as_string))
        .map(ToOwned::to_owned)
}

fn trace_entry_to_json(entry: &ios_core::ostrace::LogEntry) -> serde_json::Value {
    let process_uuid = uuid::Uuid::from_bytes(entry.process_uuid);
    let image_uuid = uuid::Uuid::from_bytes(entry.image_uuid);
    serde_json::json!({
        "schema_version": 2,
        "record_type": entry.record_type,
        "pid": entry.pid,
        "procid": entry.procid,
        "process_uuid": process_uuid.to_string(),
        "process_uuid_hex": hex::encode(entry.process_uuid),
        "activity_id": entry.activity_id,
        "parent_activity_id": entry.parent_activity_id,
        "timestamp": trace_timestamp_iso(entry),
        "timestamp_parts": {
            "seconds": entry.timestamp.seconds,
            "microseconds": entry.timestamp.microseconds,
        },
        "mach_timestamp": entry.mach_timestamp,
        "level": entry.level.name(),
        "level_value": entry.level.0,
        "level_name": entry.level.display_name(),
        "thread_id": entry.thread_id,
        "image_uuid": image_uuid.to_string(),
        "image_uuid_hex": hex::encode(entry.image_uuid),
        "image_name": entry.image_name,
        "image_offset": entry.image_offset,
        "filename": entry.filename,
        "message": entry.message,
        "label": entry.label.as_ref().map(|label| serde_json::json!({
            "subsystem": label.subsystem,
            "category": label.category,
        })),
    })
}

fn trace_timestamp_iso(entry: &ios_core::ostrace::LogEntry) -> Option<String> {
    let seconds = i64::try_from(entry.timestamp.seconds).ok()?;
    let nanos = entry.timestamp.microseconds.checked_mul(1_000)?;
    chrono::DateTime::<chrono::Utc>::from_timestamp(seconds, nanos)
        .map(|timestamp| timestamp.to_rfc3339_opts(chrono::SecondsFormat::Micros, true))
}

fn format_trace_entry(entry: &ios_core::ostrace::LogEntry) -> String {
    let label = entry
        .label
        .as_ref()
        .map(|label| format!("{}:{}", label.subsystem, label.category))
        .unwrap_or_else(|| "-".into());
    let process = if entry.filename.is_empty() {
        entry.image_name.as_str()
    } else {
        entry.filename.as_str()
    };
    format!(
        "{}.{:06} [{}] {}[{}] <{}>: {}",
        entry.timestamp.seconds,
        entry.timestamp.microseconds,
        entry.level,
        process,
        entry.pid,
        label,
        entry.message
    )
}

fn plist_to_json(value: &plist::Value) -> serde_json::Value {
    match value {
        plist::Value::Array(values) => {
            serde_json::Value::Array(values.iter().map(plist_to_json).collect())
        }
        plist::Value::Dictionary(dict) => serde_json::Value::Object(
            dict.iter()
                .map(|(key, value)| (key.clone(), plist_to_json(value)))
                .collect(),
        ),
        plist::Value::Boolean(value) => serde_json::Value::Bool(*value),
        plist::Value::Data(bytes) => serde_json::Value::String(hex::encode(bytes)),
        plist::Value::Date(value) => serde_json::Value::String(value.to_xml_format()),
        plist::Value::Integer(value) => value
            .as_signed()
            .map(serde_json::Value::from)
            .or_else(|| value.as_unsigned().map(serde_json::Value::from))
            .unwrap_or(serde_json::Value::Null),
        plist::Value::Real(value) => serde_json::json!(value),
        plist::Value::String(value) => serde_json::Value::String(value.clone()),
        plist::Value::Uid(value) => serde_json::Value::from(value.get()),
        _ => serde_json::Value::Null,
    }
}

#[cfg(test)]
mod tests {
    use clap::Parser;

    use super::*;

    #[derive(Parser)]
    struct TestCli {
        #[command(subcommand)]
        command: OsTraceSub,
    }

    #[test]
    fn parses_os_trace_ps_subcommand() {
        let parsed = TestCli::try_parse_from(["os-trace", "ps"]);
        assert!(parsed.is_ok(), "os-trace ps should parse");
    }

    #[test]
    fn parses_os_trace_stream_filters_and_live_alias() {
        let parsed = TestCli::try_parse_from([
            "os-trace",
            "live",
            "--pid",
            "42",
            "--process",
            "SpringBoard",
            "--level",
            "error,info",
            "--subsystem",
            "com.apple",
            "--category",
            "network",
            "--match",
            "timeout",
            "--match",
            "世界",
            "--exclude",
            "success",
            "--exclude",
            "denied",
            "--regex",
            "timeout\\s+.*",
            "--regex",
            "世界",
            "--ignore-case",
            "--count",
            "2",
            "--timeout",
            "5",
        ])
        .expect("os-trace live should parse");
        let OsTraceSub::Stream {
            pid,
            process,
            level,
            subsystem,
            category,
            message_match,
            exclude,
            regex,
            ignore_case,
            count,
            timeout,
        } = parsed.command
        else {
            panic!("expected stream subcommand");
        };
        assert_eq!(pid, Some(42));
        assert_eq!(process.as_deref(), Some("SpringBoard"));
        assert_eq!(level, "error,info");
        assert_eq!(subsystem.as_deref(), Some("com.apple"));
        assert_eq!(category.as_deref(), Some("network"));
        assert_eq!(message_match, ["timeout", "世界"]);
        assert_eq!(exclude, ["success", "denied"]);
        assert_eq!(regex, [r"timeout\s+.*", "世界"]);
        assert!(ignore_case);
        assert_eq!(count, Some(2));
        assert_eq!(timeout, Some(5));
    }

    #[test]
    fn parses_archive_and_collect_limits() {
        let parsed = TestCli::try_parse_from([
            "os-trace",
            "archive",
            "logs.tar",
            "--size-limit",
            "4096",
            "--age-limit",
            "7",
            "--start-time",
            "42",
            "--timeout",
            "30",
        ])
        .expect("archive should parse");
        let OsTraceSub::Archive {
            output,
            size_limit,
            age_limit,
            start_time,
            timeout,
        } = parsed.command
        else {
            panic!("expected archive subcommand");
        };
        assert_eq!(output, PathBuf::from("logs.tar"));
        assert_eq!(size_limit, Some(4096));
        assert_eq!(age_limit, Some(7));
        assert_eq!(start_time, Some(42));
        assert_eq!(timeout, Some(30));

        let parsed = TestCli::try_parse_from(["os-trace", "collect", "logs.logarchive"])
            .expect("collect should parse");
        assert!(matches!(parsed.command, OsTraceSub::Collect { .. }));
    }

    #[test]
    fn pid_rows_supports_upstream_dictionary_payload() {
        let response = plist::Dictionary::from_iter([(
            "Payload".to_string(),
            plist::Value::Dictionary(plist::Dictionary::from_iter([
                (
                    "42".to_string(),
                    plist::Value::Dictionary(plist::Dictionary::from_iter([(
                        "ProcessName".to_string(),
                        plist::Value::String("SpringBoard".into()),
                    )])),
                ),
                (
                    "7".to_string(),
                    plist::Value::Dictionary(plist::Dictionary::from_iter([(
                        "Name".to_string(),
                        plist::Value::String("launchd".into()),
                    )])),
                ),
            ])),
        )]);

        assert_eq!(
            pid_rows(&response),
            Some(vec![(7, "launchd".into()), (42, "SpringBoard".into())])
        );
    }

    #[test]
    fn structured_json_uses_standard_fields_and_legacy_aliases() {
        let entry = ios_core::ostrace::LogEntry {
            record_type: 0x0400,
            pid: 42,
            procid: 7,
            process_uuid: [1; 16],
            activity_id: 9,
            parent_activity_id: 8,
            timestamp: ios_core::ostrace::TraceTimestamp {
                seconds: 1_705_312_200,
                microseconds: 500_000,
            },
            mach_timestamp: 11,
            level: ios_core::ostrace::LogLevel::ERROR,
            thread_id: 12,
            image_uuid: [2; 16],
            image_name: "Test".into(),
            image_offset: 13,
            filename: "/usr/lib/Test".into(),
            message: "hello".into(),
            label: None,
        };
        let json = trace_entry_to_json(&entry);
        assert_eq!(json["process_uuid"], "01010101-0101-0101-0101-010101010101");
        assert_eq!(json["image_uuid"], "02020202-0202-0202-0202-020202020202");
        assert_eq!(json["process_uuid_hex"], "01010101010101010101010101010101");
        assert_eq!(json["timestamp"], "2024-01-15T09:50:00.500000Z");
        assert_eq!(json["timestamp_parts"]["microseconds"], 500_000);
        assert_eq!(json["level"], "Error");
        assert_eq!(json["level_value"], 0x10);
    }

    #[tokio::test]
    async fn connection_timeout_is_typed_and_diagnostic() {
        let mut stop: StopSignal = Box::pin(std::future::pending());
        let deadline = tokio::time::Instant::now() + Duration::from_millis(1);
        let termination = wait_for_trace_stage(
            std::future::pending::<std::result::Result<(), anyhow::Error>>(),
            &mut stop,
            Some(deadline),
        )
        .await
        .unwrap();
        assert_eq!(termination, Err(RunTermination::TimedOut));
        let error = finish_termination(RunTermination::TimedOut, Some(3)).unwrap_err();
        assert!(error.to_string().contains("timed out after 3 seconds"));
    }

    #[tokio::test]
    async fn handshake_timeout_is_typed() {
        let mut stop: StopSignal = Box::pin(std::future::pending());
        let deadline = tokio::time::Instant::now() + Duration::from_millis(1);
        let termination = wait_for_trace_stage(
            std::future::pending::<std::result::Result<(), anyhow::Error>>(),
            &mut stop,
            Some(deadline),
        )
        .await
        .unwrap();
        assert_eq!(termination, Err(RunTermination::TimedOut));
    }

    #[tokio::test]
    async fn read_timeout_is_typed() {
        let mut stop: StopSignal = Box::pin(std::future::pending());
        let deadline = tokio::time::Instant::now() + Duration::from_millis(1);
        let termination = wait_for_trace_stage(
            std::future::pending::<std::result::Result<(), anyhow::Error>>(),
            &mut stop,
            Some(deadline),
        )
        .await
        .unwrap();
        assert_eq!(termination, Err(RunTermination::TimedOut));
    }

    #[tokio::test]
    async fn cancelled_trace_stage_is_typed_and_clean() {
        let (sender, receiver) = tokio::sync::oneshot::channel();
        let mut stop: StopSignal = Box::pin(async move {
            receiver
                .await
                .map_err(|_| std::io::Error::other("test cancellation"))
        });
        sender.send(()).unwrap();
        let termination = wait_for_trace_stage(
            std::future::pending::<std::result::Result<(), anyhow::Error>>(),
            &mut stop,
            None,
        )
        .await
        .unwrap();
        assert_eq!(termination, Err(RunTermination::Cancelled));
        assert!(finish_termination(RunTermination::Cancelled, Some(3)).is_ok());
    }
}
