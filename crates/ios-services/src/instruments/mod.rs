//! Instruments services – performance monitoring via DTX.
//!
//! Service names:
//!   - `com.apple.instruments.remoteserver`                      (iOS ≤13)
//!   - `com.apple.instruments.remoteserver.DVTSecureSocketProxy` (iOS 14-16)
//!   - `com.apple.instruments.dtservicehub`                      (iOS 17+ via RSD/tunnel)
//!
//! Flow:
//!   1. Connect to service via lockdown StartService
//!   2. Wrap stream in DtxConnection
//!   3. request_channel(service_name) → channel_code
//!   4. method_call(channel_code, "setConfig:", [archived_config])
//!   5. method_call_async(channel_code, "start")
//!   6. Loop recv() → parse samples
//!
//! Reference: go-ios/ios/instruments/

pub mod activity_trace;
pub mod application_listing;
pub mod core_profile_session;
pub mod deviceinfo;
pub mod devicestate;
pub mod energy;
pub mod fps;
pub mod graphics;
pub mod network;
pub mod notifications;
pub mod process_control;
pub mod screenshot;
pub mod tap;

pub use activity_trace::{
    ActivityTraceClient, ActivityTraceDecoder, ActivityTraceEntry, ActivityTraceValue,
};
pub use application_listing::ApplicationListingClient;
pub use core_profile_session::{
    CoreProfileConfig, CoreProfileEvent, CoreProfileSessionClient, CORE_PROFILE_SESSION_SVC,
};
pub use deviceinfo::{DeviceInfoClient, RunningProcess};
pub use devicestate::{ConditionProfile, ConditionProfileType, DeviceStateClient};
pub use energy::EnergyMonitorClient;
pub use fps::{parse_frame_commit_timestamps, FpsSample, FpsWindowCalculator, MachTimeInfo};
pub use graphics::GraphicsMonitorClient;
pub use network::{
    ConnectionDetectionEvent, ConnectionUpdateEvent, InterfaceDetectionEvent, NetworkMonitorClient,
    NetworkMonitorEvent, SocketAddress,
};
pub use notifications::{NotificationClient, NotificationEvent};
pub use process_control::{ProcessControl, ProcessInfo};
pub use screenshot::take_screenshot_dtx;
pub use screenshot::take_screenshot_dtx as start_screenshot;
pub use tap::TapClient;

// ── Service name constants ────────────────────────────────────────────────────

pub const SERVICE_LEGACY: &str = "com.apple.instruments.remoteserver";
pub const SERVICE_IOS14: &str = "com.apple.instruments.remoteserver.DVTSecureSocketProxy";
pub const SERVICE_IOS17: &str = "com.apple.instruments.dtservicehub"; // via RSD
pub const SYSMONTAP: &str = "com.apple.instruments.server.services.sysmontap";
pub const DEVICE_INFO_SVC: &str = "com.apple.instruments.server.services.deviceinfo";
pub const PROCESS_CTRL_SVC: &str = "com.apple.instruments.server.services.processcontrol";
pub const SCREENSHOT_SVC: &str = "com.apple.instruments.server.services.screenshot";
pub const APP_LISTING_SVC: &str = "com.apple.instruments.server.services.device.applictionListing";
pub const ACTIVITY_TRACE_TAP_SVC: &str = "com.apple.instruments.server.services.activitytracetap";
pub const CONDITION_INDUCER_SVC: &str = "com.apple.instruments.server.services.ConditionInducer";
pub const ENERGY_MONITOR_SVC: &str = "com.apple.xcode.debug-gauge-data-providers.Energy";
pub const GRAPHICS_MONITOR_SVC: &str = "com.apple.instruments.server.services.graphics.opengl";
pub const MOBILE_NOTIFICATIONS_SVC: &str =
    "com.apple.instruments.server.services.mobilenotifications";
pub const NETWORK_MONITOR_SVC: &str = "com.apple.instruments.server.services.networking";

// ── CPU sample types ──────────────────────────────────────────────────────────

/// A CPU usage sample from sysmontap.
#[derive(Debug, Clone)]
pub struct CpuSample {
    pub cpu_count: u64,
    pub enabled_cpus: u64,
    pub end_mach_abs_time: u64,
    pub cpu_total_load: f64,
    pub sample_type: u64,
}

/// A memory usage sample from sysmontap.
#[derive(Debug, Clone)]
pub struct MemSample {
    pub memory_used: u64,  // bytes
    pub memory_total: u64, // bytes
}

/// Per-process stats from a sysmontap snapshot.
#[derive(Debug, Clone)]
pub struct ProcessSample {
    /// Process attribute values keyed by attribute name.
    pub processes: Vec<serde_json::Map<String, serde_json::Value>>,
    /// System CPU usage (if present in this sample).
    pub system_cpu: Option<CpuSample>,
}

// ── SysmontapConfig ───────────────────────────────────────────────────────────

/// Configuration for the sysmontap performance monitor.
pub struct SysmontapConfig {
    /// Update rate: lower = faster samples (Xcode default: 10)
    pub update_rate: i32,
    /// Report CPU usage
    pub cpu_usage: bool,
    /// Report physical memory footprint
    pub phys_footprint: bool,
    /// Sample interval in nanoseconds (500_000_000 = 0.5s)
    pub sample_interval: i64,
}

impl Default for SysmontapConfig {
    fn default() -> Self {
        Self {
            update_rate: 10,
            cpu_usage: true,
            phys_footprint: true,
            sample_interval: 500_000_000,
        }
    }
}

// ── Sysmontap client ──────────────────────────────────────────────────────────

use ios_proto::nskeyedarchiver_encode;
use tokio::io::{AsyncRead, AsyncWrite};

use crate::dtx::codec::{DtxConnection, DtxError};
use crate::dtx::primitive_enc::archived_object;
use crate::dtx::types::{DtxMessage, DtxPayload, NSObject};

/// Connect and start a sysmontap CPU monitoring session.
///
/// Returns an async stream of `CpuSample` values.
pub struct SysmontapService<S> {
    conn: DtxConnection<S>,
    channel_code: i32,
}

impl<S: AsyncRead + AsyncWrite + Unpin + Send> SysmontapService<S> {
    /// Initialize sysmontap on an already-connected instruments stream.
    ///
    /// `sys_attrs` and `proc_attrs` are optional attribute lists from the deviceinfo service.
    /// Pass `None` to use defaults (works on iOS 17+; older iOS may need them).
    pub async fn start(
        stream: S,
        config: &SysmontapConfig,
        sys_attrs: Option<Vec<plist::Value>>,
        proc_attrs: Option<Vec<plist::Value>>,
    ) -> Result<Self, DtxError> {
        let mut conn = DtxConnection::new(stream);

        // Request sysmontap channel
        let ch = conn.request_channel(SYSMONTAP).await?;

        // Build config dict (matches go-ios exactly)
        let mut config_dict = build_sysmontap_config(config);
        if let Some(attrs) = sys_attrs {
            if !attrs.is_empty() {
                config_dict.push(("sysAttrs".to_string(), plist::Value::Array(attrs)));
            }
        }
        if let Some(attrs) = proc_attrs {
            if !attrs.is_empty() {
                config_dict.push(("procAttrs".to_string(), plist::Value::Array(attrs)));
            }
        }
        let archived = nskeyedarchiver_encode::archive_dict(config_dict);

        // setConfig:
        let cfg_resp = conn
            .method_call(ch, "setConfig:", &[archived_object(archived)])
            .await?;
        tracing::debug!("sysmontap setConfig: response: {:?}", cfg_resp.payload);

        // start (fire-and-forget)
        conn.method_call_async(ch, "start", &[]).await?;

        Ok(Self {
            conn,
            channel_code: ch,
        })
    }

    /// Receive the next CPU sample from the device.
    /// Blocks until a sample arrives.
    pub async fn next_cpu_sample(&mut self) -> Result<Option<CpuSample>, DtxError> {
        loop {
            let msg = self.conn.recv().await?;
            tracing::debug!(
                "next_cpu_sample: id={} ch={} expects_reply={} payload={:?}",
                msg.identifier,
                msg.channel_code,
                msg.expects_reply,
                std::mem::discriminant(&msg.payload)
            );

            // Ack if needed
            if msg.expects_reply {
                self.conn.send_ack(&msg).await?;
            }

            // Only process messages on our channel or channel -1 (sysmontap broadcasts on -1)
            if msg.channel_code != self.channel_code && msg.channel_code != -1 {
                continue;
            }

            tracing::debug!(
                "sysmontap msg ch={} payload={:?}",
                msg.channel_code,
                &msg.payload
            );

            if let Some(sample) = parse_cpu_sample(&msg) {
                return Ok(Some(sample));
            }
        }
    }

    /// Receive the next per-process snapshot from sysmontap.
    ///
    /// `proc_attr_names` should be the ordered list of attribute names matching
    /// the `procAttrs` config (e.g. from `DeviceInfoClient::process_attributes()`).
    /// Each process's values array is zipped with these names.
    pub async fn next_process_snapshot(
        &mut self,
        proc_attr_names: &[String],
    ) -> Result<Option<ProcessSample>, DtxError> {
        loop {
            let msg = self.conn.recv().await?;
            if msg.expects_reply {
                self.conn.send_ack(&msg).await?;
            }
            if msg.channel_code != self.channel_code && msg.channel_code != -1 {
                continue;
            }
            if let Some(sample) = parse_process_snapshot(&msg, proc_attr_names) {
                if !sample.processes.is_empty() {
                    return Ok(Some(sample));
                }
            }
        }
    }

    /// Stop the monitoring session.
    pub async fn stop(&mut self) -> Result<(), DtxError> {
        self.conn
            .method_call_async(self.channel_code, "stop", &[])
            .await
    }
}

// ── Config builder ────────────────────────────────────────────────────────────

fn build_sysmontap_config(cfg: &SysmontapConfig) -> Vec<(String, plist::Value)> {
    vec![
        (
            "ur".to_string(),
            plist::Value::Integer(cfg.update_rate.into()),
        ),
        ("bm".to_string(), plist::Value::Integer(0.into())),
        ("cpuUsage".to_string(), plist::Value::Boolean(cfg.cpu_usage)),
        (
            "physFootprint".to_string(),
            plist::Value::Boolean(cfg.phys_footprint),
        ),
        (
            "sampleInterval".to_string(),
            plist::Value::Integer(cfg.sample_interval.into()),
        ),
    ]
}

// ── Sample parser ─────────────────────────────────────────────────────────────

fn parse_cpu_sample(msg: &DtxMessage) -> Option<CpuSample> {
    // Payload is a MethodInvocation from the device with selector and args
    // The args contain an NSArray of NSDictionary with CPU stats
    let args = match &msg.payload {
        DtxPayload::MethodInvocation { args, .. } => args,
        DtxPayload::Response(NSObject::Array(arr)) => {
            return parse_from_array(arr);
        }
        DtxPayload::Raw(bytes) => match unarchive_raw_payload(bytes) {
            Some(NSObject::Array(arr)) => {
                return parse_from_array(&arr);
            }
            _ => return None,
        },
        _ => return None,
    };

    // First arg should be an NSArray of sample dicts
    for arg in args {
        if let NSObject::Array(arr) = arg {
            return parse_from_array(arr);
        }
    }
    None
}

fn parse_from_array(arr: &[NSObject]) -> Option<CpuSample> {
    // Each element is a dict; only process dicts that have SystemCPUUsage (Type=43 system data)
    for item in arr {
        if let NSObject::Dict(d) = item {
            // Skip process-only messages (no SystemCPUUsage)
            let sys_cpu = match d.get("SystemCPUUsage") {
                Some(NSObject::Dict(s)) => s,
                _ => continue,
            };
            let cpu_count = get_uint(d, "CPUCount").unwrap_or(0);
            let enabled = get_uint(d, "EnabledCPUs").unwrap_or(0);
            let end_time = get_uint(d, "EndMachAbsTime").unwrap_or(0);
            let typ = get_uint(d, "Type").unwrap_or(0);
            let cpu_load = get_float(sys_cpu, "CPU_TotalLoad").unwrap_or(0.0);

            return Some(CpuSample {
                cpu_count,
                enabled_cpus: enabled,
                end_mach_abs_time: end_time,
                cpu_total_load: cpu_load,
                sample_type: typ,
            });
        }
    }
    None
}

// ── Per-process snapshot parser ──────────────────────────────────────────────

/// Parse per-process data from a sysmontap message.
///
/// The sysmontap sends data as NSKeyedArchiver-encoded arrays of dicts.
/// The data list contains multiple dicts:
///   - One with SystemCPUUsage (system-level CPU stats)
///   - One with Processes (per-process data, keyed by PID)
///
/// The "Processes" dict maps PID (as string key) → Array of values
/// in the same order as the `procAttrs` config.
fn parse_process_snapshot(msg: &DtxMessage, attr_names: &[String]) -> Option<ProcessSample> {
    match &msg.payload {
        DtxPayload::MethodInvocation { args, .. } => {
            for arg in args {
                if let NSObject::Array(arr) = arg {
                    return parse_process_from_array(arr, attr_names);
                }
            }
            None
        }
        DtxPayload::Response(NSObject::Array(arr)) => parse_process_from_array(arr, attr_names),
        DtxPayload::Response(NSObject::Dict(d)) => {
            // Some iOS versions wrap in a single dict
            parse_process_from_array(&[NSObject::Dict(d.clone())], attr_names)
        }
        DtxPayload::Raw(bytes) => match unarchive_raw_payload(bytes) {
            Some(NSObject::Array(arr)) => parse_process_from_array(&arr, attr_names),
            _ => None,
        },
        DtxPayload::RawWithAux { payload, aux } => {
            // Try aux args first (process data may come via auxiliary)
            for arg in aux {
                if let NSObject::Array(arr) = arg {
                    if let Some(sample) = parse_process_from_array(arr, attr_names) {
                        return Some(sample);
                    }
                }
            }
            // Fall back to payload
            match unarchive_raw_payload(payload) {
                Some(NSObject::Array(arr)) => parse_process_from_array(&arr, attr_names),
                _ => None,
            }
        }
        _ => None,
    }
}

fn parse_process_from_array(arr: &[NSObject], attr_names: &[String]) -> Option<ProcessSample> {
    // The array contains multiple dicts. We need to find the one with "Processes" key
    // and optionally the one with "SystemCPUUsage".
    let mut processes = Vec::new();
    let mut system_cpu = None;

    for item in arr {
        if let NSObject::Dict(d) = item {
            // Extract system CPU if present
            if system_cpu.is_none() {
                if let Some(NSObject::Dict(sys_cpu)) = d.get("SystemCPUUsage") {
                    system_cpu = Some(CpuSample {
                        cpu_count: get_uint(d, "CPUCount").unwrap_or(0),
                        enabled_cpus: get_uint(d, "EnabledCPUs").unwrap_or(0),
                        end_mach_abs_time: get_uint(d, "EndMachAbsTime").unwrap_or(0),
                        cpu_total_load: get_float(sys_cpu, "CPU_TotalLoad").unwrap_or(0.0),
                        sample_type: get_uint(d, "Type").unwrap_or(0),
                    });
                }
            }

            // Extract per-process data: look for "Processes" key
            // The value is a dict of PID → Array(values matching procAttrs order)
            if let Some(proc_val) = d.get("Processes") {
                match proc_val {
                    NSObject::Dict(processes_dict) => {
                        for (_pid_key, values) in processes_dict {
                            if let NSObject::Array(vals) = values {
                                let mut proc_map = serde_json::Map::new();
                                for (i, val) in vals.iter().enumerate() {
                                    let key = attr_names
                                        .get(i)
                                        .cloned()
                                        .unwrap_or_else(|| format!("attr_{i}"));
                                    proc_map.insert(key, nsobject_to_json(val));
                                }
                                processes.push(proc_map);
                            }
                        }
                    }
                    // Some iOS versions may use a different container
                    _ => {}
                }
            }
        }
    }

    if !processes.is_empty() || system_cpu.is_some() {
        Some(ProcessSample {
            processes,
            system_cpu,
        })
    } else {
        None
    }
}

fn get_uint(d: &indexmap::IndexMap<String, NSObject>, key: &str) -> Option<u64> {
    match d.get(key) {
        Some(NSObject::Uint(n)) => Some(*n),
        Some(NSObject::Int(n)) => Some(*n as u64),
        _ => None,
    }
}

fn get_float(d: &indexmap::IndexMap<String, NSObject>, key: &str) -> Option<f64> {
    match d.get(key) {
        Some(NSObject::Double(f)) => Some(*f),
        Some(NSObject::Int(n)) => Some(*n as f64),
        _ => None,
    }
}

pub(crate) fn archive_value_to_nsobject(
    value: ios_proto::nskeyedarchiver::ArchiveValue,
) -> NSObject {
    use ios_proto::nskeyedarchiver::ArchiveValue;

    match value {
        ArchiveValue::Null => NSObject::Null,
        ArchiveValue::Bool(value) => NSObject::Bool(value),
        ArchiveValue::Int(value) => NSObject::Int(value),
        ArchiveValue::Float(value) => NSObject::Double(value),
        ArchiveValue::String(value) => NSObject::String(value),
        ArchiveValue::Data(value) => NSObject::Data(value),
        ArchiveValue::Array(values) => {
            NSObject::Array(values.into_iter().map(archive_value_to_nsobject).collect())
        }
        ArchiveValue::Dict(dict) => NSObject::Dict(
            dict.into_iter()
                .map(|(key, value)| (key, archive_value_to_nsobject(value)))
                .collect(),
        ),
        ArchiveValue::Unknown(name) => NSObject::String(format!("<{name}>")),
    }
}

pub(crate) fn unarchive_raw_payload(payload: &bytes::Bytes) -> Option<NSObject> {
    ios_proto::nskeyedarchiver::unarchive(payload)
        .ok()
        .map(archive_value_to_nsobject)
}

pub(crate) fn nsobject_to_json(value: &NSObject) -> serde_json::Value {
    use serde_json::{Map, Number, Value};

    match value {
        NSObject::Int(value) => Value::from(*value),
        NSObject::Uint(value) => Value::from(*value),
        NSObject::Double(value) => Number::from_f64(*value)
            .map(Value::Number)
            .unwrap_or(Value::Null),
        NSObject::Bool(value) => Value::Bool(*value),
        NSObject::String(value) => Value::String(value.clone()),
        NSObject::Data(value) => Value::String(
            value
                .iter()
                .map(|byte| format!("{byte:02x}"))
                .collect::<String>(),
        ),
        NSObject::Array(values) => Value::Array(values.iter().map(nsobject_to_json).collect()),
        NSObject::Dict(dict) => Value::Object(
            dict.iter()
                .map(|(key, value)| (key.clone(), nsobject_to_json(value)))
                .collect::<Map<_, _>>(),
        ),
        NSObject::Null => Value::Null,
    }
}
