use anyhow::Result;
use ios_core::device::{ConnectOptions, ConnectedDevice, ServiceStream};
use ios_core::services::instruments::{SERVICE_IOS14, SERVICE_IOS17, SERVICE_LEGACY};
use ios_core::tunnel::TunMode;

#[derive(clap::Args)]
pub struct InstrumentsCmd {
    #[command(subcommand)]
    sub: InstrumentsSub,
}

#[derive(clap::Subcommand)]
enum InstrumentsSub {
    /// Stream CPU usage
    Cpu {
        #[arg(
            short = 'n',
            long,
            default_value = "10",
            help = "Number of samples (0 = infinite)"
        )]
        count: u64,
        #[arg(
            short = 'r',
            long,
            default_value = "10",
            help = "Update rate (lower = faster, Xcode default = 10)"
        )]
        rate: i32,
        #[arg(long, help = "Overall timeout in seconds")]
        timeout: Option<u64>,
        #[arg(short = 'j', long, help = "Output JSON lines")]
        json: bool,
    },
    /// List running processes
    Ps {
        #[arg(long, help = "Only include processes marked by iOS as applications")]
        apps: bool,
        #[arg(
            long,
            help = "Only include processes whose name contains this substring"
        )]
        name: Option<String>,
        #[arg(short = 'j', long, help = "Output JSON")]
        json: bool,
    },
    /// List sysmontap system attributes exposed by DeviceInfo
    Sysattrs {
        #[arg(short = 'j', long, help = "Output JSON")]
        json: bool,
    },
    /// List sysmontap process attributes exposed by DeviceInfo
    Procattrs {
        #[arg(short = 'j', long, help = "Output JSON")]
        json: bool,
    },
    /// Launch an app by bundle ID
    Launch {
        bundle_id: String,
        #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
        args: Vec<String>,
    },
    /// Kill a process by PID
    Kill { pid: u64 },
    /// Listen for application and memory notifications
    Notifications {
        #[arg(
            short = 'n',
            long,
            default_value = "0",
            help = "Number of notifications to print (0 = infinite)"
        )]
        count: u64,
        #[arg(
            long,
            default_value = "0",
            help = "Maximum time to wait for each notification in seconds (0 = no timeout)"
        )]
        timeout: u64,
    },
    /// List applications via the DTX applicationListing channel
    Apps {
        #[arg(short = 'j', long, help = "Output JSON")]
        json: bool,
    },
    /// Sample energy metrics for one or more processes
    Energy {
        #[arg(long = "pid", required = true, num_args = 1.., value_delimiter = ',')]
        pid: Vec<i32>,
        #[arg(
            short = 'n',
            long,
            default_value = "10",
            help = "Number of samples to print (0 = infinite)"
        )]
        count: u64,
        #[arg(short = 'j', long, help = "Output JSON lines")]
        json: bool,
    },
    /// Monitor network activity via the Instruments networking channel
    Network {
        #[arg(
            short = 'n',
            long,
            default_value = "0",
            help = "Number of events to print (0 = infinite)"
        )]
        count: u64,
        #[arg(short = 'j', long, help = "Output JSON lines")]
        json: bool,
    },
    /// Monitor GPU and renderer counters
    Gpu {
        #[arg(
            short = 'n',
            long,
            default_value = "10",
            help = "Number of samples to print (0 = infinite)"
        )]
        count: u64,
        #[arg(short = 'j', long, help = "Output JSON lines")]
        json: bool,
    },
    /// Stream activity trace / oslog entries
    Trace {
        #[arg(long, help = "Only include entries for this PID")]
        pid: Option<u32>,
        #[arg(
            short = 'n',
            long,
            default_value = "0",
            help = "Number of entries to print (0 = infinite)"
        )]
        count: u64,
        #[arg(short = 'j', long, help = "Output JSON lines")]
        json: bool,
    },
    /// Enable HAR (HTTP Archive) logging
    Har {
        #[arg(long, help = "Only include entries for this PID")]
        pid: Option<u32>,
        #[arg(
            short = 'n',
            long,
            default_value = "0",
            help = "Number of entries to print (0 = infinite)"
        )]
        count: u64,
        #[arg(short = 'j', long, help = "Output JSON lines")]
        json: bool,
    },
    /// Stream FPS/jank samples via coreprofilesessiontap
    Fps {
        #[arg(
            short = 'n',
            long,
            default_value = "10",
            help = "Number of samples to print (0 = infinite)"
        )]
        count: u64,
        #[arg(
            long,
            default_value = "1000",
            help = "Sampling window size in milliseconds"
        )]
        window_ms: u64,
        #[arg(short = 'j', long, help = "Output JSON lines")]
        json: bool,
    },
    /// Stream raw KDebug events from coreprofilesessiontap
    Kdebug {
        #[arg(
            short = 'n',
            long,
            default_value = "0",
            help = "Number of events to print (0 = infinite)"
        )]
        count: u64,
        #[arg(
            long,
            value_delimiter = ',',
            help = "Filter by KDebug class (decimal, e.g. 49 for QuartzCore)"
        )]
        class_filter: Vec<u32>,
        #[arg(
            long,
            value_delimiter = ',',
            help = "Filter by KDebug subclass (decimal, e.g. 128)"
        )]
        subclass_filter: Vec<u32>,
        #[arg(short = 'j', long, help = "Output JSON lines")]
        json: bool,
    },
    /// Monitor per-process stats (CPU, memory, etc.) via sysmontap
    SysmonProcess {
        #[arg(
            short = 'n',
            long,
            default_value = "0",
            help = "Number of snapshots (0 = infinite)"
        )]
        count: u64,
        #[arg(long, help = "Filter by process name (substring match)")]
        name: Option<String>,
        #[arg(long, help = "Filter by PID")]
        pid: Option<u64>,
        #[arg(short = 'j', long, help = "Output JSON lines")]
        json: bool,
    },
    /// Monitor processes exceeding a CPU usage threshold
    SysmonThreshold {
        /// CPU usage threshold percentage (0-100)
        threshold: f64,
        #[arg(
            short = 'n',
            long,
            default_value = "0",
            help = "Number of snapshots (0 = infinite)"
        )]
        count: u64,
        #[arg(short = 'j', long, help = "Output JSON lines")]
        json: bool,
    },
}

impl InstrumentsCmd {
    pub async fn run(self, udid: Option<String>) -> Result<()> {
        let udid = udid.ok_or_else(|| anyhow::anyhow!("--udid required for instruments"))?;

        match self.sub {
            InstrumentsSub::Cpu {
                count,
                rate,
                timeout,
                json,
            } => run_cpu(udid, rate, count, timeout, json).await,
            InstrumentsSub::Ps { apps, name, json } => run_ps(udid, apps, name, json).await,
            InstrumentsSub::Sysattrs { json } => run_device_info_attrs(udid, true, json).await,
            InstrumentsSub::Procattrs { json } => run_device_info_attrs(udid, false, json).await,
            InstrumentsSub::Launch { bundle_id, args } => run_launch(udid, bundle_id, args).await,
            InstrumentsSub::Kill { pid } => run_kill(udid, pid).await,
            InstrumentsSub::Notifications { count, timeout } => {
                run_notifications(udid, count, timeout).await
            }
            InstrumentsSub::Apps { json } => run_apps(udid, json).await,
            InstrumentsSub::Energy { pid, count, json } => run_energy(udid, pid, count, json).await,
            InstrumentsSub::Network { count, json } => run_network(udid, count, json).await,
            InstrumentsSub::Gpu { count, json } => run_gpu(udid, count, json).await,
            InstrumentsSub::Trace { pid, count, json } => {
                run_trace(udid, pid, count, json, false).await
            }
            InstrumentsSub::Har { pid, count, json } => {
                run_trace(udid, pid, count, json, true).await
            }
            InstrumentsSub::Fps {
                count,
                window_ms,
                json,
            } => run_fps(udid, count, window_ms, json).await,
            InstrumentsSub::Kdebug {
                count,
                class_filter,
                subclass_filter,
                json,
            } => run_kdebug(udid, count, class_filter, subclass_filter, json).await,
            InstrumentsSub::SysmonProcess {
                count,
                name,
                pid,
                json,
            } => run_sysmon_process(udid, count, name, pid, json).await,
            InstrumentsSub::SysmonThreshold {
                threshold,
                count,
                json,
            } => run_sysmon_threshold(udid, threshold, count, json).await,
        }
    }
}

async fn run_notifications(_udid: String, _count: u64, _timeout_secs: u64) -> Result<()> {
    use ios_core::services::dtx::NSObject;
    use ios_core::services::instruments::NotificationClient;
    use serde_json::{Map, Number, Value};

    let (_device, stream) = connect_instruments(&_udid).await?;
    let mut client = NotificationClient::connect(stream)
        .await
        .map_err(|e| anyhow::anyhow!("DTX error: {e}"))?;

    let mut received = 0u64;
    loop {
        if _count > 0 && received >= _count {
            break;
        }

        let event = if _timeout_secs > 0 {
            tokio::time::timeout(
                std::time::Duration::from_secs(_timeout_secs),
                client.next_notification(),
            )
            .await
            .map_err(|_| anyhow::anyhow!("timed out waiting for notification"))?
            .map_err(|e| anyhow::anyhow!("DTX error: {e}"))?
        } else {
            client
                .next_notification()
                .await
                .map_err(|e| anyhow::anyhow!("DTX error: {e}"))?
        };

        received += 1;
        let mut obj = Map::new();
        obj.insert("selector".to_string(), Value::String(event.selector));
        obj.insert("channel_code".to_string(), Value::from(event.channel_code));
        obj.insert("payload".to_string(), nsobject_to_json(&event.payload));
        println!("{}", serde_json::to_string_pretty(&Value::Object(obj))?);
    }

    fn nsobject_to_json(value: &NSObject) -> Value {
        match value {
            NSObject::Int(v) => Value::from(*v),
            NSObject::Uint(v) => Value::from(*v),
            NSObject::Double(v) => Number::from_f64(*v)
                .map(Value::Number)
                .unwrap_or(Value::Null),
            NSObject::Bool(v) => Value::Bool(*v),
            NSObject::String(v) => Value::String(v.clone()),
            NSObject::Data(bytes) => Value::String(hex::encode(bytes)),
            NSObject::Array(items) => Value::Array(items.iter().map(nsobject_to_json).collect()),
            NSObject::Dict(dict) => Value::Object(
                dict.iter()
                    .map(|(k, v)| (k.clone(), nsobject_to_json(v)))
                    .collect(),
            ),
            NSObject::Null => Value::Null,
        }
    }

    Ok(())
}

/// Connect to the instruments service, auto-detecting iOS version.
///
/// - iOS 17+: tunnel + RSD → `connect_rsd_service(dtservicehub)`
/// - iOS 14-16: lockdown → `DVTSecureSocketProxy`
/// - iOS ≤13: lockdown → `remoteserver`
pub async fn connect_instruments(udid: &str) -> Result<(ConnectedDevice, ServiceStream)> {
    // First, query the device version with a lightweight lockdown-only connection
    let probe_opts = ConnectOptions {
        tun_mode: TunMode::Userspace,
        pair_record_path: None,
        skip_tunnel: true,
    };
    let probe = ios_core::connect(udid, probe_opts).await?;
    let version = probe.product_version().await?;
    drop(probe);

    if version.major >= 17 {
        // iOS 17+: need tunnel + RSD for dtservicehub
        let opts = ConnectOptions {
            tun_mode: TunMode::Userspace,
            pair_record_path: None,
            skip_tunnel: false,
        };
        let device = ios_core::connect(udid, opts).await?;
        // Give the device time to initialize RSD after tunnel establishment
        tokio::time::sleep(std::time::Duration::from_secs(2)).await;
        let stream = device.connect_rsd_service(SERVICE_IOS17).await?;
        Ok((device, stream))
    } else {
        // iOS ≤16: lockdown path, no tunnel needed
        let opts = ConnectOptions {
            tun_mode: TunMode::Userspace,
            pair_record_path: None,
            skip_tunnel: true,
        };
        let device = ios_core::connect(udid, opts).await?;
        let stream = match device.connect_service(SERVICE_IOS14).await {
            Ok(s) => s,
            Err(_) => device.connect_service(SERVICE_LEGACY).await?,
        };
        Ok((device, stream))
    }
}

async fn run_cpu(
    udid: String,
    rate: i32,
    count: u64,
    timeout_secs: Option<u64>,
    json_output: bool,
) -> Result<()> {
    use ios_core::services::instruments::{DeviceInfoClient, SysmontapConfig, SysmontapService};

    let (_device, stream) = connect_instruments(&udid).await?;

    // Get sysmon attributes via a second independent connection (matches go-ios behavior)
    let (sys_attrs, proc_attrs) = match connect_instruments(&udid).await {
        Ok((_dev2, stream2)) => match DeviceInfoClient::connect(stream2).await {
            Ok(mut di) => {
                let sys = di.system_attributes().await.unwrap_or_default();
                let proc = di.process_attributes().await.unwrap_or_default();
                (Some(sys), Some(proc))
            }
            Err(_) => (None, None),
        },
        Err(_) => (None, None),
    };

    let cfg = SysmontapConfig {
        update_rate: rate,
        ..Default::default()
    };

    eprintln!("Starting sysmontap (rate={rate})... Ctrl+C to stop");

    let mut svc = SysmontapService::start(stream, &cfg, sys_attrs, proc_attrs)
        .await
        .map_err(|e| anyhow::anyhow!("DTX error: {e}"))?;

    let deadline =
        timeout_secs.map(|secs| tokio::time::Instant::now() + std::time::Duration::from_secs(secs));
    let mut received = 0u64;
    loop {
        if count > 0 && received >= count {
            break;
        }

        let sample = match deadline {
            Some(deadline) => {
                let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
                if remaining.is_zero() {
                    break;
                }
                match tokio::time::timeout(remaining, svc.next_cpu_sample()).await {
                    Ok(result) => result,
                    Err(_) => break,
                }
            }
            None => svc.next_cpu_sample().await,
        };

        match sample {
            Ok(Some(s)) => {
                received += 1;
                if json_output {
                    println!(
                        "{{\"cpu_total_load\":{:.2},\"cpu_count\":{},\"enabled_cpus\":{}}}",
                        s.cpu_total_load, s.cpu_count, s.enabled_cpus
                    );
                } else {
                    println!(
                        "[{:>4}] CPU: {:>6.2}%  ({}/{} cores)",
                        received, s.cpu_total_load, s.enabled_cpus, s.cpu_count
                    );
                }
            }
            Ok(None) => break,
            Err(e) => {
                eprintln!("Error: {e}");
                break;
            }
        }
    }

    svc.stop().await.ok();
    Ok(())
}

async fn run_ps(
    udid: String,
    apps_only: bool,
    name_filter: Option<String>,
    json_output: bool,
) -> Result<()> {
    use ios_core::services::instruments::DeviceInfoClient;

    let (_device, stream) = connect_instruments(&udid).await?;
    let mut di = DeviceInfoClient::connect(stream)
        .await
        .map_err(|e| anyhow::anyhow!("DTX error: {e}"))?;

    let procs = di
        .running_processes()
        .await
        .map_err(|e| anyhow::anyhow!("runningProcesses error: {e}"))?;

    // Sort by pid
    let mut procs = procs;
    if apps_only {
        procs.retain(|p| p.is_application);
    }
    if let Some(name_filter) = name_filter {
        let needle = name_filter.to_ascii_lowercase();
        procs.retain(|p| p.name.to_ascii_lowercase().contains(&needle));
    }
    procs.sort_by_key(|p| p.pid);

    if json_output {
        let list: Vec<_> = procs
            .iter()
            .map(|p| {
                serde_json::json!({
                    "pid": p.pid,
                    "is_application": p.is_application,
                    "name": p.name,
                    "real_app_name": p.real_app_name,
                })
            })
            .collect();
        println!("{}", serde_json::to_string_pretty(&list)?);
        return Ok(());
    }

    println!("{:<8} {:<6} NAME", "PID", "APP");
    println!("{}", "-".repeat(50));
    for p in &procs {
        println!(
            "{:<8} {:<6} {}",
            p.pid,
            if p.is_application { "yes" } else { "no" },
            p.name
        );
    }
    eprintln!("Total: {} processes", procs.len());
    Ok(())
}

async fn run_device_info_attrs(udid: String, system: bool, json_output: bool) -> Result<()> {
    use ios_core::services::instruments::DeviceInfoClient;

    let (_device, stream) = connect_instruments(&udid).await?;
    let mut di = DeviceInfoClient::connect(stream)
        .await
        .map_err(|e| anyhow::anyhow!("DTX error: {e}"))?;

    let attrs = if system {
        di.system_attributes()
            .await
            .map_err(|e| anyhow::anyhow!("system_attributes error: {e}"))?
    } else {
        di.process_attributes()
            .await
            .map_err(|e| anyhow::anyhow!("process_attributes error: {e}"))?
    };

    if json_output {
        println!("{}", serde_json::to_string_pretty(&attrs)?);
    } else {
        for attr in attrs {
            println!("{}", format_plist_value(&attr));
        }
    }
    Ok(())
}

fn format_plist_value(value: &plist::Value) -> String {
    match value {
        plist::Value::String(s) => s.clone(),
        plist::Value::Boolean(v) => v.to_string(),
        plist::Value::Integer(n) => n
            .as_signed()
            .map(|v| v.to_string())
            .or_else(|| n.as_unsigned().map(|v| v.to_string()))
            .unwrap_or_else(|| "0".to_string()),
        plist::Value::Real(v) => v.to_string(),
        other => serde_json::to_string(other).unwrap_or_else(|_| "null".to_string()),
    }
}

async fn run_launch(udid: String, bundle_id: String, args: Vec<String>) -> Result<()> {
    use std::collections::HashMap;

    use ios_core::services::instruments::process_control::ProcessControl;

    let (_device, stream) = connect_instruments(&udid).await?;
    let mut pc = ProcessControl::connect(stream)
        .await
        .map_err(|e| anyhow::anyhow!("DTX error: {e}"))?;

    let args_ref: Vec<&str> = args.iter().map(|s| s.as_str()).collect();
    let env = HashMap::new();
    let pid = pc
        .launch(&bundle_id, &args_ref, &env)
        .await
        .map_err(|e| anyhow::anyhow!("launch error: {e}"))?;

    println!("Launched {bundle_id} with PID {pid}");
    Ok(())
}

async fn run_kill(udid: String, pid: u64) -> Result<()> {
    use ios_core::services::instruments::process_control::ProcessControl;

    let (_device, stream) = connect_instruments(&udid).await?;
    let mut pc = ProcessControl::connect(stream)
        .await
        .map_err(|e| anyhow::anyhow!("DTX error: {e}"))?;

    pc.kill(pid)
        .await
        .map_err(|e| anyhow::anyhow!("kill error: {e}"))?;

    println!("Sent SIGKILL to PID {pid}");
    Ok(())
}

async fn run_apps(udid: String, json_output: bool) -> Result<()> {
    use ios_core::services::instruments::ApplicationListingClient;

    let (_device, stream) = connect_instruments(&udid).await?;
    let mut client = ApplicationListingClient::connect(stream)
        .await
        .map_err(|e| anyhow::anyhow!("DTX error: {e}"))?;
    let apps = client
        .installed_applications()
        .await
        .map_err(|e| anyhow::anyhow!("application listing error: {e}"))?;

    if json_output {
        println!("{}", serde_json::to_string_pretty(&apps)?);
    } else {
        let mut table = comfy_table::Table::new();
        table.set_header(["BundleID", "Name", "Version", "Type"]);
        for app in apps {
            if let Some(dict) = app.as_dictionary() {
                table.add_row([
                    comfy_table::Cell::new(
                        dict.get("CFBundleIdentifier")
                            .and_then(plist::Value::as_string)
                            .unwrap_or(""),
                    ),
                    comfy_table::Cell::new(
                        dict.get("CFBundleDisplayName")
                            .or_else(|| dict.get("CFBundleName"))
                            .and_then(plist::Value::as_string)
                            .unwrap_or(""),
                    ),
                    comfy_table::Cell::new(
                        dict.get("CFBundleShortVersionString")
                            .and_then(plist::Value::as_string)
                            .unwrap_or(""),
                    ),
                    comfy_table::Cell::new(
                        dict.get("ApplicationType")
                            .and_then(plist::Value::as_string)
                            .unwrap_or(""),
                    ),
                ]);
            }
        }
        println!("{table}");
    }

    Ok(())
}

async fn run_energy(udid: String, pids: Vec<i32>, count: u64, json_output: bool) -> Result<()> {
    use ios_core::services::instruments::EnergyMonitorClient;

    let (_device, stream) = connect_instruments(&udid).await?;
    let mut client = EnergyMonitorClient::connect(stream, &pids)
        .await
        .map_err(|e| anyhow::anyhow!("DTX error: {e}"))?;

    let mut received = 0u64;
    loop {
        if count > 0 && received >= count {
            break;
        }

        let sample = client
            .sample()
            .await
            .map_err(|e| anyhow::anyhow!("energy sample error: {e}"))?;
        received += 1;

        if json_output {
            println!("{}", serde_json::to_string(&sample)?);
        } else {
            println!(
                "[{:>4}] {}",
                received,
                serde_json::to_string_pretty(&sample.payload)?
            );
        }
    }

    client.stop_sampling().await.ok();
    Ok(())
}

async fn run_network(udid: String, count: u64, json_output: bool) -> Result<()> {
    use ios_core::services::instruments::{NetworkMonitorClient, NetworkMonitorEvent};

    let (_device, stream) = connect_instruments(&udid).await?;
    let mut client = NetworkMonitorClient::connect(stream)
        .await
        .map_err(|e| anyhow::anyhow!("DTX error: {e}"))?;

    let mut received = 0u64;
    loop {
        if count > 0 && received >= count {
            break;
        }

        let event = client
            .next_event()
            .await
            .map_err(|e| anyhow::anyhow!("network event error: {e}"))?;
        received += 1;

        if json_output {
            println!("{}", serde_json::to_string(&event)?);
            continue;
        }

        match event {
            NetworkMonitorEvent::InterfaceDetection(event) => {
                println!(
                    "[{:>4}] interface idx={} name={}",
                    received, event.interface_index, event.name
                );
            }
            NetworkMonitorEvent::ConnectionDetection(event) => {
                println!(
                    "[{:>4}] {}:{} -> {}:{} pid={} if={} serial={} kind={}",
                    received,
                    event.local_address.address,
                    event.local_address.port,
                    event.remote_address.address,
                    event.remote_address.port,
                    event.pid,
                    event.interface_index,
                    event.serial_number,
                    event.kind
                );
            }
            NetworkMonitorEvent::ConnectionUpdate(event) => {
                println!(
                    "[{:>4}] serial={} rx={}B/{}pkts tx={}B/{}pkts rtt(min/avg)={}/{}",
                    received,
                    event.connection_serial,
                    event.rx_bytes,
                    event.rx_packets,
                    event.tx_bytes,
                    event.tx_packets,
                    event.min_rtt,
                    event.avg_rtt
                );
            }
        }
    }

    client.stop().await.ok();
    Ok(())
}

async fn run_gpu(udid: String, count: u64, json_output: bool) -> Result<()> {
    use ios_core::services::instruments::GraphicsMonitorClient;

    let (_device, stream) = connect_instruments(&udid).await?;
    let mut client = GraphicsMonitorClient::connect(stream)
        .await
        .map_err(|e| anyhow::anyhow!("DTX error: {e}"))?;

    let mut received = 0u64;
    loop {
        if count > 0 && received >= count {
            break;
        }

        let sample = client
            .next_sample()
            .await
            .map_err(|e| anyhow::anyhow!("gpu sample error: {e}"))?;
        received += 1;

        if json_output {
            println!("{}", serde_json::to_string(&sample)?);
        } else {
            println!(
                "[{:>4}] {}",
                received,
                serde_json::to_string_pretty(&sample.payload)?
            );
        }
    }

    client.stop().await.ok();
    Ok(())
}

async fn run_trace(
    udid: String,
    pid: Option<u32>,
    count: u64,
    json_output: bool,
    enable_har: bool,
) -> Result<()> {
    use chrono::{Local, SecondsFormat};
    use ios_core::services::instruments::ActivityTraceClient;

    let (_device, stream) = connect_instruments(&udid).await?;
    let mut client = ActivityTraceClient::connect(stream, pid, enable_har)
        .await
        .map_err(|e| anyhow::anyhow!("DTX error: {e}"))?;

    let mut received = 0u64;
    loop {
        if count > 0 && received >= count {
            break;
        }

        let entry = client
            .next_entry()
            .await
            .map_err(|e| anyhow::anyhow!("activity trace error: {e}"))?;
        received += 1;

        if json_output {
            println!("{}", serde_json::to_string(&entry)?);
            continue;
        }

        let subsystem = if entry.subsystem.is_empty() {
            "-"
        } else {
            entry.subsystem.as_str()
        };
        let category = if entry.category.is_empty() {
            "-"
        } else {
            entry.category.as_str()
        };
        let message_type = if entry.message_type.is_empty() {
            entry.event_type.as_deref().unwrap_or("unknown")
        } else {
            entry.message_type.as_str()
        };
        let image_name = entry
            .sender_image_path
            .rsplit('/')
            .next()
            .filter(|name| !name.is_empty())
            .unwrap_or("-");
        let rendered_message = if entry.rendered_message.is_empty() {
            entry.name.as_deref().unwrap_or("")
        } else {
            entry.rendered_message.as_str()
        };

        println!(
            "[{}][{}][{}][{}][{}] <{}>: {}",
            Local::now().to_rfc3339_opts(SecondsFormat::Millis, false),
            subsystem,
            category,
            entry.process,
            image_name,
            message_type,
            rendered_message
        );
    }

    client.stop().await.ok();
    Ok(())
}

async fn run_fps(udid: String, count: u64, window_ms: u64, json_output: bool) -> Result<()> {
    use ios_core::services::instruments::{
        parse_frame_commit_timestamps, CoreProfileConfig, CoreProfileEvent,
        CoreProfileSessionClient, FpsSample, FpsWindowCalculator,
    };

    let (_device, stream) = connect_instruments(&udid).await?;
    let mut session = CoreProfileSessionClient::connect(stream)
        .await
        .map_err(|e| anyhow::anyhow!("DTX error: {e}"))?;
    session
        .start(&CoreProfileConfig::fps_defaults())
        .await
        .map_err(|e| anyhow::anyhow!("core profile start error: {e}"))?;
    let mach_time_info = session.mach_time_info().clone();
    let (event_tx, mut event_rx) =
        tokio::sync::mpsc::unbounded_channel::<std::result::Result<CoreProfileEvent, String>>();
    let (stop_tx, stop_rx) = tokio::sync::oneshot::channel::<()>();
    let reader = tokio::spawn(async move {
        let mut stop_rx = stop_rx;
        loop {
            tokio::select! {
                _ = &mut stop_rx => break,
                event = session.next_event() => {
                    match event {
                        Ok(event) => {
                            if event_tx.send(Ok(event)).is_err() {
                                break;
                            }
                        }
                        Err(error) => {
                            let _ = event_tx.send(Err(error.to_string()));
                            break;
                        }
                    }
                }
            }
        }

        session.stop().await.ok();
    });

    let window = std::time::Duration::from_millis(window_ms);
    let mut calculator = FpsWindowCalculator::new();
    let mut timestamps = Vec::new();
    let mut next_deadline = tokio::time::Instant::now() + window;
    let mut emitted = 0u64;

    loop {
        if count > 0 && emitted >= count {
            break;
        }

        let now = tokio::time::Instant::now();
        let remaining = next_deadline.saturating_duration_since(now);

        match tokio::time::timeout(remaining, event_rx.recv()).await {
            Ok(Some(Ok(CoreProfileEvent::Notice(_)))) => {}
            Ok(Some(Ok(CoreProfileEvent::RawChunk(chunk)))) => {
                timestamps.extend(parse_frame_commit_timestamps(&chunk, &mach_time_info));
            }
            Ok(Some(Err(error))) => {
                let _ = stop_tx.send(());
                let _ = reader.await;
                return Err(anyhow::anyhow!("core profile event error: {error}"));
            }
            Ok(None) => {
                let _ = stop_tx.send(());
                let _ = reader.await;
                return Err(anyhow::anyhow!(
                    "core profile event stream closed unexpectedly"
                ));
            }
            Err(_) => {
                let sample = if timestamps.is_empty() {
                    FpsSample {
                        fps: 0.0,
                        jank: 0,
                        big_jank: 0,
                        stutter: 0.0,
                        frame_count: 0,
                        window_ms: window_ms as f64,
                    }
                } else {
                    calculator
                        .push_timestamps(&timestamps)
                        .unwrap_or(FpsSample {
                            fps: 0.0,
                            jank: 0,
                            big_jank: 0,
                            stutter: 0.0,
                            frame_count: 0,
                            window_ms: window_ms as f64,
                        })
                };

                emitted += 1;
                if json_output {
                    println!("{}", serde_json::to_string(&sample)?);
                } else {
                    println!(
                        "[{:>4}] FPS: {:>5.1} | Jank: {} | BigJank: {} | Stutter: {:>5.1}% | Frames: {} | Window: {:.0}ms",
                        emitted,
                        sample.fps,
                        sample.jank,
                        sample.big_jank,
                        sample.stutter * 100.0,
                        sample.frame_count,
                        sample.window_ms
                    );
                }

                timestamps.clear();
                next_deadline += window;
            }
        }
    }

    let _ = stop_tx.send(());
    let _ = reader.await;
    Ok(())
}

async fn run_kdebug(
    udid: String,
    count: u64,
    class_filter: Vec<u32>,
    subclass_filter: Vec<u32>,
    json_output: bool,
) -> Result<()> {
    use ios_core::services::instruments::{
        CoreProfileConfig, CoreProfileEvent, CoreProfileSessionClient,
    };

    let (_device, stream) = connect_instruments(&udid).await?;
    let mut session = CoreProfileSessionClient::connect(stream)
        .await
        .map_err(|e| anyhow::anyhow!("DTX error: {e}"))?;

    let mut config = CoreProfileConfig::fps_defaults();
    // Use all-events filter by default (u32::MAX)
    config.filters = vec![u32::MAX];

    session
        .start(&config)
        .await
        .map_err(|e| anyhow::anyhow!("core profile start error: {e}"))?;
    let mach_time_info = session.mach_time_info().clone();

    let has_class_filter = !class_filter.is_empty();
    let has_subclass_filter = !subclass_filter.is_empty();

    let mut emitted = 0u64;

    if !json_output {
        println!(
            "{:<20} {:>5} {:>5} {:>6} {:>3} {:>16} {:>16} {:>16} {:>16} {:>10}",
            "TIMESTAMP_NS", "CLASS", "SUBCL", "CODE", "FQ", "ARG1", "ARG2", "ARG3", "ARG4", "TID"
        );
    }

    loop {
        if count > 0 && emitted >= count {
            break;
        }

        let event = session
            .next_event()
            .await
            .map_err(|e| anyhow::anyhow!("core profile event error: {e}"))?;

        match event {
            CoreProfileEvent::Notice(_) => continue,
            CoreProfileEvent::RawChunk(chunk) => {
                const RECORD_SIZE: usize = 64;
                for record in chunk.chunks_exact(RECORD_SIZE) {
                    let mach_time = u64::from_le_bytes(record[0..8].try_into().unwrap_or_default());
                    let debug_id =
                        u32::from_le_bytes(record[48..52].try_into().unwrap_or_default());

                    let class = (debug_id >> 24) & 0xFF;
                    let subclass = (debug_id >> 16) & 0xFF;
                    let code = (debug_id >> 2) & 0x3FFF;
                    let func_qual = debug_id & 0x3;

                    if has_class_filter && !class_filter.contains(&class) {
                        continue;
                    }
                    if has_subclass_filter && !subclass_filter.contains(&subclass) {
                        continue;
                    }

                    let timestamp_ns = if mach_time_info.denom > 0 {
                        (mach_time as f64 * mach_time_info.numer as f64
                            / mach_time_info.denom as f64) as u64
                    } else {
                        mach_time
                    };

                    let arg1 = u64::from_le_bytes(record[8..16].try_into().unwrap_or_default());
                    let arg2 = u64::from_le_bytes(record[16..24].try_into().unwrap_or_default());
                    let arg3 = u64::from_le_bytes(record[24..32].try_into().unwrap_or_default());
                    let arg4 = u64::from_le_bytes(record[32..40].try_into().unwrap_or_default());
                    let tid = u64::from_le_bytes(record[40..48].try_into().unwrap_or_default());

                    let fq_label = match func_qual {
                        0 => "NONE",
                        1 => "START",
                        2 => "END",
                        3 => "ALL",
                        _ => "?",
                    };

                    emitted += 1;
                    if json_output {
                        println!(
                            "{}",
                            serde_json::json!({
                                "timestamp_ns": timestamp_ns,
                                "event_id": format!("0x{:08x}", debug_id & 0xFFFF_FFFC),
                                "class": class,
                                "subclass": subclass,
                                "code": code,
                                "func_qualifier": fq_label,
                                "arg1": format!("0x{:x}", arg1),
                                "arg2": format!("0x{:x}", arg2),
                                "arg3": format!("0x{:x}", arg3),
                                "arg4": format!("0x{:x}", arg4),
                                "tid": tid,
                            })
                        );
                    } else {
                        println!(
                            "{:<20} {:>5} {:>5} {:>6} {:>5} {:>16x} {:>16x} {:>16x} {:>16x} {:>10}",
                            timestamp_ns,
                            class,
                            subclass,
                            code,
                            fq_label,
                            arg1,
                            arg2,
                            arg3,
                            arg4,
                            tid,
                        );
                    }

                    if count > 0 && emitted >= count {
                        break;
                    }
                }
            }
        }
    }

    session.stop().await.ok();
    Ok(())
}

async fn run_sysmon_process(
    udid: String,
    count: u64,
    name_filter: Option<String>,
    pid_filter: Option<u64>,
    json_output: bool,
) -> Result<()> {
    use ios_core::services::instruments::{DeviceInfoClient, SysmontapConfig, SysmontapService};

    let (_device, stream) = connect_instruments(&udid).await?;

    // Get proc attribute names for column mapping
    let (sys_attrs, proc_attrs, proc_attr_names) = match connect_instruments(&udid).await {
        Ok((_dev2, stream2)) => match DeviceInfoClient::connect(stream2).await {
            Ok(mut di) => {
                let sys = di.system_attributes().await.unwrap_or_default();
                let proc = di.process_attributes().await.unwrap_or_default();
                let names: Vec<String> = proc
                    .iter()
                    .filter_map(|v| v.as_string().map(String::from))
                    .collect();
                (Some(sys), Some(proc), names)
            }
            Err(_) => (None, None, vec![]),
        },
        Err(_) => (None, None, vec![]),
    };

    let cfg = SysmontapConfig::default();
    let mut svc = SysmontapService::start(stream, &cfg, sys_attrs, proc_attrs)
        .await
        .map_err(|e| anyhow::anyhow!("DTX error: {e}"))?;

    eprintln!("Monitoring per-process stats... Ctrl+C to stop");

    let mut emitted = 0u64;
    let mut first = true;
    loop {
        if count > 0 && emitted >= count {
            break;
        }

        let snapshot = svc
            .next_process_snapshot(&proc_attr_names)
            .await
            .map_err(|e| anyhow::anyhow!("DTX error: {e}"))?;

        let snapshot = match snapshot {
            Some(s) => s,
            None => break,
        };

        // Skip first snapshot (cpuUsage not yet initialized)
        if first {
            first = false;
            continue;
        }

        let mut filtered: Vec<_> = snapshot
            .processes
            .into_iter()
            .filter(|p| {
                if let Some(ref name) = name_filter {
                    let needle = name.to_ascii_lowercase();
                    let proc_name = p.get("name").and_then(|v| v.as_str()).unwrap_or("");
                    if !proc_name.to_ascii_lowercase().contains(&needle) {
                        return false;
                    }
                }
                if let Some(pid) = pid_filter {
                    let proc_pid = p.get("pid").and_then(|v| v.as_u64()).unwrap_or(0);
                    if proc_pid != pid {
                        return false;
                    }
                }
                true
            })
            .collect();

        // Sort by CPU usage descending
        filtered.sort_by(|a, b| {
            let cpu_a = a.get("cpuUsage").and_then(|v| v.as_f64()).unwrap_or(0.0);
            let cpu_b = b.get("cpuUsage").and_then(|v| v.as_f64()).unwrap_or(0.0);
            cpu_b
                .partial_cmp(&cpu_a)
                .unwrap_or(std::cmp::Ordering::Equal)
        });

        emitted += 1;

        if json_output {
            println!("{}", serde_json::to_string(&filtered)?);
        } else {
            println!("--- Snapshot #{emitted} ({} processes) ---", filtered.len());
            println!("{:<8} {:<6.1} {:<12} NAME", "PID", "CPU%", "MEM");
            for p in &filtered {
                let pid = p.get("pid").and_then(|v| v.as_u64()).unwrap_or(0);
                let cpu = p.get("cpuUsage").and_then(|v| v.as_f64()).unwrap_or(0.0);
                let mem = p.get("physFootprint").and_then(|v| v.as_u64()).unwrap_or(0);
                let name = p.get("name").and_then(|v| v.as_str()).unwrap_or("?");
                println!("{:<8} {:<6.1} {:<12} {}", pid, cpu, format_bytes(mem), name);
            }
        }
    }

    svc.stop().await.ok();
    Ok(())
}

async fn run_sysmon_threshold(
    udid: String,
    threshold: f64,
    count: u64,
    json_output: bool,
) -> Result<()> {
    use ios_core::services::instruments::{DeviceInfoClient, SysmontapConfig, SysmontapService};

    let (_device, stream) = connect_instruments(&udid).await?;

    let (sys_attrs, proc_attrs, proc_attr_names) = match connect_instruments(&udid).await {
        Ok((_dev2, stream2)) => match DeviceInfoClient::connect(stream2).await {
            Ok(mut di) => {
                let sys = di.system_attributes().await.unwrap_or_default();
                let proc = di.process_attributes().await.unwrap_or_default();
                let names: Vec<String> = proc
                    .iter()
                    .filter_map(|v| v.as_string().map(String::from))
                    .collect();
                (Some(sys), Some(proc), names)
            }
            Err(_) => (None, None, vec![]),
        },
        Err(_) => (None, None, vec![]),
    };

    let cfg = SysmontapConfig::default();
    let mut svc = SysmontapService::start(stream, &cfg, sys_attrs, proc_attrs)
        .await
        .map_err(|e| anyhow::anyhow!("DTX error: {e}"))?;

    eprintln!("Monitoring processes with CPU > {threshold:.1}%... Ctrl+C to stop");

    let mut emitted = 0u64;
    let mut first = true;
    loop {
        if count > 0 && emitted >= count {
            break;
        }

        let snapshot = svc
            .next_process_snapshot(&proc_attr_names)
            .await
            .map_err(|e| anyhow::anyhow!("DTX error: {e}"))?;

        let snapshot = match snapshot {
            Some(s) => s,
            None => break,
        };

        if first {
            first = false;
            continue;
        }

        let above_threshold: Vec<_> = snapshot
            .processes
            .into_iter()
            .filter(|p| p.get("cpuUsage").and_then(|v| v.as_f64()).unwrap_or(0.0) >= threshold)
            .collect();

        if above_threshold.is_empty() {
            continue;
        }

        emitted += 1;

        if json_output {
            for p in &above_threshold {
                println!("{}", serde_json::to_string(p)?);
            }
        } else {
            for p in &above_threshold {
                let pid = p.get("pid").and_then(|v| v.as_u64()).unwrap_or(0);
                let cpu = p.get("cpuUsage").and_then(|v| v.as_f64()).unwrap_or(0.0);
                let mem = p.get("physFootprint").and_then(|v| v.as_u64()).unwrap_or(0);
                let name = p.get("name").and_then(|v| v.as_str()).unwrap_or("?");
                println!(
                    "PID={:<8} CPU={:.1}% MEM={} NAME={}",
                    pid,
                    cpu,
                    format_bytes(mem),
                    name
                );
            }
        }
    }

    svc.stop().await.ok();
    Ok(())
}

fn format_bytes(bytes: u64) -> String {
    if bytes >= 1_073_741_824 {
        format!("{:.1}GB", bytes as f64 / 1_073_741_824.0)
    } else if bytes >= 1_048_576 {
        format!("{:.1}MB", bytes as f64 / 1_048_576.0)
    } else if bytes >= 1024 {
        format!("{:.1}KB", bytes as f64 / 1024.0)
    } else {
        format!("{bytes}B")
    }
}

#[cfg(test)]
mod tests {
    use clap::Parser;

    use super::*;

    #[derive(Parser)]
    struct TestCli {
        #[command(subcommand)]
        command: InstrumentsSub,
    }

    #[test]
    fn parses_notifications_subcommand() {
        let cmd = TestCli::parse_from([
            "instruments",
            "notifications",
            "--count",
            "3",
            "--timeout",
            "15",
        ]);
        match cmd.command {
            InstrumentsSub::Notifications { count, timeout } => {
                assert_eq!(count, 3);
                assert_eq!(timeout, 15);
            }
            _ => panic!("expected notifications subcommand"),
        }
    }

    #[test]
    fn parses_cpu_timeout_flag() {
        let cmd = TestCli::parse_from(["instruments", "cpu", "--count", "0", "--timeout", "5"]);
        match cmd.command {
            InstrumentsSub::Cpu {
                count,
                rate,
                timeout,
                json,
            } => {
                assert_eq!(count, 0);
                assert_eq!(rate, 10);
                assert_eq!(timeout, Some(5));
                assert!(!json);
            }
            _ => panic!("expected cpu subcommand"),
        }
    }

    #[test]
    fn parses_ps_apps_flag() {
        let cmd = TestCli::parse_from(["instruments", "ps", "--apps"]);
        match cmd.command {
            InstrumentsSub::Ps { apps, name, json } => {
                assert!(apps);
                assert_eq!(name, None);
                assert!(!json);
            }
            _ => panic!("expected ps subcommand"),
        }
    }

    #[test]
    fn parses_ps_json_flag() {
        let cmd = TestCli::parse_from(["instruments", "ps", "--json"]);
        match cmd.command {
            InstrumentsSub::Ps { apps, name, json } => {
                assert!(!apps);
                assert_eq!(name, None);
                assert!(json);
            }
            _ => panic!("expected ps subcommand"),
        }
    }

    #[test]
    fn parses_sysattrs_subcommand() {
        let cmd = TestCli::parse_from(["instruments", "sysattrs", "--json"]);
        match cmd.command {
            InstrumentsSub::Sysattrs { json } => assert!(json),
            _ => panic!("expected sysattrs subcommand"),
        }
    }

    #[test]
    fn parses_procattrs_subcommand() {
        let cmd = TestCli::parse_from(["instruments", "procattrs"]);
        match cmd.command {
            InstrumentsSub::Procattrs { json } => assert!(!json),
            _ => panic!("expected procattrs subcommand"),
        }
    }

    #[test]
    fn parses_ps_name_filter() {
        let cmd = TestCli::parse_from(["instruments", "ps", "--name", "Chat"]);
        match cmd.command {
            InstrumentsSub::Ps { apps, name, json } => {
                assert!(!apps);
                assert_eq!(name.as_deref(), Some("Chat"));
                assert!(!json);
            }
            _ => panic!("expected ps subcommand"),
        }
    }

    #[test]
    fn format_plist_value_renders_scalars() {
        assert_eq!(
            format_plist_value(&plist::Value::String("cpuUsage".into())),
            "cpuUsage"
        );
        assert_eq!(format_plist_value(&plist::Value::Integer(42.into())), "42");
        assert_eq!(format_plist_value(&plist::Value::Boolean(true)), "true");
    }

    #[test]
    fn parses_apps_subcommand() {
        let cmd = TestCli::parse_from(["instruments", "apps", "--json"]);
        match cmd.command {
            InstrumentsSub::Apps { json } => assert!(json),
            _ => panic!("expected apps subcommand"),
        }
    }

    #[test]
    fn parses_energy_subcommand() {
        let cmd = TestCli::parse_from([
            "instruments",
            "energy",
            "--pid",
            "12,34",
            "--count",
            "5",
            "--json",
        ]);
        match cmd.command {
            InstrumentsSub::Energy { pid, count, json } => {
                assert_eq!(pid, vec![12, 34]);
                assert_eq!(count, 5);
                assert!(json);
            }
            _ => panic!("expected energy subcommand"),
        }
    }

    #[test]
    fn parses_network_subcommand() {
        let cmd = TestCli::parse_from(["instruments", "network", "--count", "3"]);
        match cmd.command {
            InstrumentsSub::Network { count, json } => {
                assert_eq!(count, 3);
                assert!(!json);
            }
            _ => panic!("expected network subcommand"),
        }
    }

    #[test]
    fn parses_gpu_subcommand() {
        let cmd = TestCli::parse_from(["instruments", "gpu", "--count", "2", "--json"]);
        match cmd.command {
            InstrumentsSub::Gpu { count, json } => {
                assert_eq!(count, 2);
                assert!(json);
            }
            _ => panic!("expected gpu subcommand"),
        }
    }

    #[test]
    fn parses_trace_subcommand() {
        let cmd = TestCli::parse_from(["instruments", "trace", "--pid", "42", "--count", "3"]);
        match cmd.command {
            InstrumentsSub::Trace { pid, count, json } => {
                assert_eq!(pid, Some(42));
                assert_eq!(count, 3);
                assert!(!json);
            }
            _ => panic!("expected trace subcommand"),
        }
    }

    #[test]
    fn parses_fps_subcommand() {
        let cmd = TestCli::parse_from([
            "instruments",
            "fps",
            "--count",
            "5",
            "--window-ms",
            "750",
            "--json",
        ]);
        match cmd.command {
            InstrumentsSub::Fps {
                count,
                window_ms,
                json,
            } => {
                assert_eq!(count, 5);
                assert_eq!(window_ms, 750);
                assert!(json);
            }
            _ => panic!("expected fps subcommand"),
        }
    }

    #[test]
    fn parses_sysmon_process_subcommand() {
        let cmd = TestCli::parse_from([
            "instruments",
            "sysmon-process",
            "--count",
            "3",
            "--name",
            "Safari",
            "--json",
        ]);
        match cmd.command {
            InstrumentsSub::SysmonProcess {
                count,
                name,
                pid,
                json,
            } => {
                assert_eq!(count, 3);
                assert_eq!(name.as_deref(), Some("Safari"));
                assert_eq!(pid, None);
                assert!(json);
            }
            _ => panic!("expected sysmon-process subcommand"),
        }
    }

    #[test]
    fn parses_sysmon_process_pid_filter() {
        let cmd = TestCli::parse_from(["instruments", "sysmon-process", "--pid", "42"]);
        match cmd.command {
            InstrumentsSub::SysmonProcess {
                count,
                name,
                pid,
                json,
            } => {
                assert_eq!(count, 0);
                assert_eq!(name, None);
                assert_eq!(pid, Some(42));
                assert!(!json);
            }
            _ => panic!("expected sysmon-process subcommand"),
        }
    }

    #[test]
    fn parses_sysmon_threshold_subcommand() {
        let cmd = TestCli::parse_from([
            "instruments",
            "sysmon-threshold",
            "5.0",
            "--count",
            "10",
            "--json",
        ]);
        match cmd.command {
            InstrumentsSub::SysmonThreshold {
                threshold,
                count,
                json,
            } => {
                assert!((threshold - 5.0).abs() < f64::EPSILON);
                assert_eq!(count, 10);
                assert!(json);
            }
            _ => panic!("expected sysmon-threshold subcommand"),
        }
    }

    #[test]
    fn format_bytes_human_readable() {
        assert_eq!(format_bytes(500), "500B");
        assert_eq!(format_bytes(1536), "1.5KB");
        assert_eq!(format_bytes(5_242_880), "5.0MB");
        assert_eq!(format_bytes(1_610_612_736), "1.5GB");
    }
}
