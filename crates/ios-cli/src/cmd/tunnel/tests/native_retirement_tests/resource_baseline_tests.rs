//! Explicit L04 host measurements, using original production native workers.
//! Discovery/metadata are private fixtures; native IO, owners and HTTP are real.
use super::*;
use crate::access::test_support::{grant, PolicyFixture, TOKEN};
use std::error::Error;
use std::time::Instant as MeasureInstant;

const REPORT_PREFIX: &str = "IOS_MANAGER_RESOURCE_BASELINE_V1 ";
const EVENT_PREFIX: &str = "IOS_MANAGER_RESOURCE_BASELINE_EVENT_V1 ";
const CLEANUP: Duration = Duration::from_secs(5);
const MAX_DEVICES: usize = 16;

#[derive(Clone, Serialize)]
struct Input {
    samples_per_scale: usize,
    longrun_duration_ms: u64,
    longrun_min_cycles: usize,
    longrun_max_cycles: usize,
    cycle_pause_ms: u64,
    #[serde(default)]
    workload: String,
    #[serde(default)]
    workload_payload_kb: usize,
}

impl Input {
    fn environment() -> Result<Self> {
        fn number(name: &str, default: u64, minimum: u64, maximum: u64) -> Result<u64> {
            let value = match std::env::var(name) {
                Ok(value) => value.parse::<u64>().context("invalid measurement input")?,
                Err(std::env::VarError::NotPresent) => default,
                Err(_) => anyhow::bail!("non-UTF-8 measurement input"),
            };
            anyhow::ensure!(
                (minimum..=maximum).contains(&value),
                "measurement input outside documented bound"
            );
            Ok(value)
        }
        let workload =
            std::env::var("IOS_MANAGER_BASELINE_WORKLOAD").unwrap_or_else(|_| "all".to_string());
        let workload_payload_kb =
            number("IOS_MANAGER_BASELINE_PAYLOAD_KB", 256, 16, 16384)? as usize;
        let input = Self {
            samples_per_scale: number("IOS_MANAGER_BASELINE_SAMPLES", 5, 1, 20)? as usize,
            longrun_duration_ms: number("IOS_MANAGER_BASELINE_DURATION_MS", 30_000, 1, 1_800_000)?,
            longrun_min_cycles: number("IOS_MANAGER_BASELINE_MIN_CYCLES", 16, 2, 256)? as usize,
            longrun_max_cycles: number("IOS_MANAGER_BASELINE_MAX_CYCLES", 128, 2, 256)? as usize,
            cycle_pause_ms: number("IOS_MANAGER_BASELINE_PAUSE_MS", 250, 0, 5_000)?,
            workload,
            workload_payload_kb,
        };
        anyhow::ensure!(
            input.longrun_min_cycles <= input.longrun_max_cycles,
            "minimum cycles exceed the bounded maximum"
        );
        Ok(input)
    }
}

struct MeasuredDevice {
    native: Arc<TunnelHandle>,
    native_id: u32,
}

impl source::FakeDevice for MeasuredDevice {
    fn record(&self) -> TunnelRecord {
        let mut record = fake_record(self.native.userspace_port.unwrap());
        record.udid = self.native.udid.clone();
        record.address = self.native.info.server_address.clone();
        record.tunnel_address = self.native.info.server_address.clone();
        record
    }

    fn is_alive(&self) -> bool {
        self.native.is_alive()
    }

    fn device_id(&self) -> u32 {
        self.native_id
    }

    fn native_handle(&self) -> Option<&Arc<TunnelHandle>> {
        Some(&self.native)
    }
}

struct Original {
    native: Arc<TunnelHandle>,
    peer: TcpStream,
    proxy: Option<TcpStream>,
    intentional_disconnect: bool,
}

struct MeasurementSource {
    ids: Vec<String>,
    originals: StdMutex<Vec<Original>>,
    owned_calls: AtomicUsize,
}

impl MeasurementSource {
    fn new() -> Arc<Self> {
        Arc::new(Self {
            ids: (0..MAX_DEVICES)
                .map(|index| format!("owned-l04-host-device-{index:02}"))
                .collect(),
            originals: StdMutex::new(Vec::with_capacity(MAX_DEVICES)),
            owned_calls: AtomicUsize::new(0),
        })
    }
}

impl Source for MeasurementSource {
    fn connect<'a>(
        &'a self,
        _: &'a str,
        _: Route,
        _: TunMode,
    ) -> BoxFuture<'a, Result<ManagedDevice>> {
        Box::pin(async { anyhow::bail!("measurement requires production connect_owned") })
    }

    fn connect_owned<'a>(
        &'a self,
        udid: &'a str,
        route: Route,
        mode: TunMode,
        owner: &'a dyn TunnelOwner,
    ) -> BoxFuture<'a, Result<ManagedDevice>> {
        Box::pin(async move {
            anyhow::ensure!(route == Route::Usbmux, "unexpected fixture route");
            anyhow::ensure!(
                matches!(mode, TunMode::Userspace),
                "unexpected fixture mode"
            );
            let index = self
                .ids
                .iter()
                .position(|candidate| candidate == udid)
                .context("unknown fixture device")?;
            {
                let originals = self.originals.lock().unwrap();
                anyhow::ensure!(originals.len() < MAX_DEVICES, "original owner fixture full");
                anyhow::ensure!(
                    originals
                        .iter()
                        .all(|original| original.native.udid != udid),
                    "duplicate original fixture owner"
                );
            }
            let mut reservation = owner.reserve()?;
            let listener = TcpListener::bind(("127.0.0.1", 0)).await?;
            let address = listener.local_addr()?;
            let (stream, accepted) =
                tokio::try_join!(TcpStream::connect(address), listener.accept())?;
            let (peer, _) = accepted;
            drop(listener);
            stream.set_nodelay(true)?;
            peer.set_nodelay(true)?;
            let runtime =
                ios_core::UserspaceTunDevice::start("fd00::2", "fd00::1", 1280, stream).await?;
            let native = Arc::new(TunnelHandle::new_userspace(
                udid.into(),
                ios_core::TunnelInfo {
                    server_address: "fd00::1".into(),
                    server_rsd_port: 58783,
                    client_address: "fd00::2".into(),
                    client_mtu: 1280,
                },
                runtime,
            ));
            // Retain the exact handle before attachment/publication. A later
            // error still leaves the harness an original join capability.
            self.originals.lock().unwrap().push(Original {
                native: native.clone(),
                peer,
                proxy: None,
                intentional_disconnect: false,
            });
            reservation.attach(native.clone())?;
            reservation.complete();
            self.owned_calls.fetch_add(1, Ordering::SeqCst);
            Ok(ManagedDevice::Fake(Box::new(MeasuredDevice {
                native,
                native_id: (index + 1) as u32,
            })))
        })
    }

    fn usb_devices(&self) -> BoxFuture<'_, UsbSnapshot> {
        Box::pin(async move {
            UsbSnapshot::Devices(
                self.ids
                    .iter()
                    .enumerate()
                    .map(|(index, udid)| source::UsbDevice {
                        udid: udid.clone(),
                        device_id: (index + 1) as u32,
                        usb: true,
                    })
                    .collect(),
            )
        })
    }

    fn paired_devices(&self) -> BoxFuture<'_, Result<Vec<ios_core::device::PairedMobdev2Device>>> {
        Box::pin(async { Ok(Vec::new()) })
    }
}

// Returned on an early measurement error so cleanup retains all original peers
// and join capabilities. The vector never exceeds the fixed sixteen owners.
struct HeldOriginals {
    source: Arc<MeasurementSource>,
    originals: Option<Vec<Original>>,
}

impl HeldOriginals {
    fn take(source: &Arc<MeasurementSource>) -> Self {
        let originals = std::mem::take(&mut *source.originals.lock().unwrap());
        Self {
            source: source.clone(),
            originals: Some(originals),
        }
    }
}

impl Drop for HeldOriginals {
    fn drop(&mut self) {
        if let Some(mut originals) = self.originals.take() {
            self.source.originals.lock().unwrap().append(&mut originals);
        }
    }
}

fn native_report(error: &ios_core::TunnelError) -> Option<&ios_core::TunnelShutdownFailure> {
    let mut cause: Option<&(dyn Error + 'static)> = Some(error);
    for _ in 0..32 {
        let current = cause?;
        if let Some(report) = current.downcast_ref::<ios_core::TunnelShutdownFailure>() {
            return Some(report);
        }
        cause = if let Some(io) = current.downcast_ref::<io::Error>() {
            io.get_ref()
                .map(|inner| inner as &(dyn Error + 'static))
                .or_else(|| current.source())
        } else {
            current.source()
        };
    }
    None
}

fn original_eof(error: &(dyn Error + 'static)) -> bool {
    let mut cause = Some(error);
    for _ in 0..32 {
        let Some(current) = cause else { break };
        if current
            .downcast_ref::<io::Error>()
            .is_some_and(|error| error.kind() == io::ErrorKind::UnexpectedEof)
        {
            return true;
        }
        cause = if let Some(io) = current.downcast_ref::<io::Error>() {
            io.get_ref()
                .map(|inner| inner as &(dyn Error + 'static))
                .or_else(|| current.source())
        } else {
            current.source()
        };
    }
    false
}

fn proc_sample() -> serde_json::Value {
    #[cfg(target_os = "linux")]
    {
        let status = std::fs::read_to_string("/proc/self/status").ok();
        let field = |key: &str| {
            status.as_ref().and_then(|status| {
                status.lines().find_map(|line| {
                    line.strip_prefix(key)
                        .and_then(|tail| tail.split_whitespace().next())
                        .and_then(|value| value.parse::<u64>().ok())
                })
            })
        };
        let descriptors = std::fs::read_dir("/proc/self/fd")
            .ok()
            .and_then(|directory| {
                let mut count = 0usize;
                let mut sockets = 0usize;
                for entry in directory {
                    let entry = entry.ok()?;
                    let target = match std::fs::read_link(entry.path()) {
                        Ok(target) => target,
                        Err(error) if error.kind() == io::ErrorKind::NotFound => continue,
                        Err(_) => return None,
                    };
                    count += 1;
                    sockets += usize::from(target.to_string_lossy().starts_with("socket:["));
                }
                Some((count, sockets))
            });
        json!({
            "method": "linux-proc-self-status-and-fd-readlink",
            "rss_bytes": field("VmRSS:").and_then(|value| value.checked_mul(1024)),
            "peak_rss_bytes": field("VmHWM:").and_then(|value| value.checked_mul(1024)),
            "process_threads": field("Threads:"),
            "fd_count": descriptors.map(|(count, _)| count),
            "socket_fd_count": descriptors.map(|(_, sockets)| sockets),
            "fd_sampler_directory_included": true,
        })
    }
    #[cfg(not(target_os = "linux"))]
    {
        json!({
            "method": "not-run",
            "reason": "this fixture implements process sampling only on Linux",
            "rss_bytes": null, "peak_rss_bytes": null, "fd_count": null,
            "socket_fd_count": null, "process_threads": null,
        })
    }
}

#[derive(Default)]
struct Evidence {
    latencies: Vec<serde_json::Value>,
    resources: Vec<serde_json::Value>,
    rounds: Vec<serde_json::Value>,
    longrun_elapsed_ms: u128,
    longrun_cycles: usize,
    events_emitted: usize,
}

impl Evidence {
    fn flush(&mut self) {
        // Retain at most one round's JSON in the measured process. Full-run
        // aggregation happens in the external bounded driver after process exit.
        let event = json!({
            "event_index": self.events_emitted,
            "latency_samples": self.latencies,
            "resource_samples": self.resources,
            "rounds": self.rounds,
        });
        println!("{EVENT_PREFIX}{}", serde_json::to_string(&event).unwrap());
        self.events_emitted += 1;
        self.latencies.clear();
        self.resources.clear();
        self.rounds.clear();
    }

    fn latency(
        &mut self,
        phase: &str,
        scenario: &str,
        devices: usize,
        round: usize,
        start: MeasureInstant,
    ) {
        self.latencies.push(json!({
            "phase": phase, "scenario": scenario, "devices": devices, "round": round,
            "nanoseconds": start.elapsed().as_nanos(),
        }));
    }

    async fn resource(
        &mut self,
        state: &TunnelAgentState,
        phase: &str,
        scenario: &str,
        devices: usize,
        round: usize,
    ) {
        let native = state.inner.native.snapshot();
        self.resources.push(json!({
            "phase": phase, "scenario": scenario, "devices": devices, "round": round,
            "process": proc_sample(),
            "tokio_alive_tasks": baseline(),
            "manager_records": state.inner.tunnels.read().await.len(),
            "device_coordinators": state.inner.lifecycle.coordinator_count().await,
            "native_registry": {
                "sealed": native.sealed, "capacity": native.capacity,
                "reserved": native.reserved, "inflight": native.inflight,
                "unpublished": native.unpublished, "active": native.active,
                "retired": native.retired, "native_handles": native.native_handles,
                "occupied": native.occupied(),
                "historical_failure_class": native.historical_failure_class,
                "additional_failed_owners": native.additional_failed_owners,
            },
            "connect_permits": state.inner.lifecycle.connects.available_permits(),
            "connect_capacity": lifecycle::MAX_CONNECTS,
            "failure_permits": state.inner.failure_slots.available_permits(),
            "failure_capacity": MAX_FAILURE_RECORDS,
            "failed_attempt_rows": state.inner.failed_attempts.read().await.len(),
        }));
    }
}

async fn response(
    client: &reqwest::Client,
    method: reqwest::Method,
    url: String,
    body: Option<serde_json::Value>,
) -> Result<(StatusCode, serde_json::Value)> {
    let mut request = client.request(method, url).bearer_auth(TOKEN);
    if let Some(body) = body {
        request = request.json(&body);
    }
    let mut reply = request.send().await?;
    let status = reply.status();
    let mut bytes = Vec::new();
    while let Some(chunk) = reply.chunk().await? {
        anyhow::ensure!(
            bytes
                .len()
                .checked_add(chunk.len())
                .is_some_and(|length| length <= 8192),
            "measurement HTTP reply exceeded its bound"
        );
        bytes.extend_from_slice(&chunk);
    }
    Ok((status, serde_json::from_slice(&bytes)?))
}

async fn strict_peer_eof(peer: &mut TcpStream) -> Result<()> {
    let mut byte = [0];
    let count = tokio::time::timeout(CLEANUP, peer.read(&mut byte))
        .await
        .context("strict peer EOF deadline")??;
    // A reset, timeout, or byte is a failure. No RST-to-EOF conversion.
    anyhow::ensure!(count == 0, "peer produced unexpected bytes before EOF");
    Ok(())
}

struct CollectedOriginals {
    certificates: Vec<serde_json::Value>,
    validation: Result<()>,
}

async fn collect_originals(originals: &mut [Original]) -> CollectedOriginals {
    let mut certificates = Vec::with_capacity(originals.len());
    let mut first_failure = None;
    for original in originals {
        // Always collect the original join handle, including after a failed
        // HTTP operation. Keep acquired native faults separately from recovery.
        let shutdown = original.native.shutdown(CLEANUP).await;
        let mut certificate = match &shutdown {
            Ok(()) => json!({"joins_complete": true, "native_outcome": "clean"}),
            Err(error) => {
                let report = native_report(error);
                json!({
                    "joins_complete": report.is_some_and(|report| report.joins_complete()),
                    "native_outcome": "error",
                    "class": ios_core::classify_error(error),
                    "pending_joins": report.map(|report| report.pending_joins()),
                    "retained_outcomes": report.map(|report| report.has_retained_outcomes()),
                    "original_unexpected_eof": report.is_some_and(|report| original_eof(report.primary())),
                })
            }
        };
        let joined = shutdown.as_ref().map_or_else(
            |error| native_report(error).is_some_and(|report| report.joins_complete()),
            |_| true,
        );
        let expected = if original.intentional_disconnect {
            shutdown.as_ref().is_err_and(|error| {
                native_report(error).is_some_and(|report| {
                    report.joins_complete()
                        && report.has_retained_outcomes()
                        && original_eof(report.primary())
                })
            })
        } else {
            shutdown.is_ok()
        };
        let peer_eof = strict_peer_eof(&mut original.peer).await;
        let proxy_eof = if let Some(proxy) = &mut original.proxy {
            strict_peer_eof(proxy).await
        } else {
            Ok(())
        };
        certificate["strict_native_peer_eof"] = json!(peer_eof.is_ok());
        certificate["strict_proxy_peer_eof"] = json!(proxy_eof.is_ok());
        certificate["expected_native_outcome"] = json!(expected);
        if first_failure.is_none() {
            first_failure = if !joined {
                Some(anyhow::anyhow!("original native joins remain incomplete"))
            } else if let Err(error) = peer_eof {
                Some(error.context("native peer EOF"))
            } else if let Err(error) = proxy_eof {
                Some(error.context("proxy peer EOF"))
            } else if !expected {
                Some(anyhow::anyhow!("unexpected original native outcome"))
            } else {
                None
            };
        }
        certificates.push(certificate);
    }
    CollectedOriginals {
        certificates,
        validation: first_failure.map_or(Ok(()), Err),
    }
}

struct RoundSpec<'a> {
    scenario: &'a str,
    devices: usize,
    round: usize,
    disconnect: bool,
}

async fn measure_round(
    state: &TunnelAgentState,
    source: &Arc<MeasurementSource>,
    client: &reqwest::Client,
    base: &str,
    evidence: &mut Evidence,
    spec: RoundSpec<'_>,
) -> Result<()> {
    let RoundSpec {
        scenario,
        devices,
        round,
        disconnect,
    } = spec;
    let round_start = MeasureInstant::now();
    evidence
        .resource(state, "before", scenario, devices, round)
        .await;
    let calls = source.owned_calls.load(Ordering::SeqCst);
    for udid in &source.ids[..devices] {
        let start = MeasureInstant::now();
        let (status, _) = response(
            client,
            reqwest::Method::POST,
            format!("{base}/start-tunnel"),
            Some(json!({"udid": udid, "connection_type": "usbmux"})),
        )
        .await?;
        evidence.latency("post_start", scenario, devices, round, start);
        anyhow::ensure!(status == StatusCode::OK, "HTTP start refused");
    }
    let mut originals = HeldOriginals::take(source);
    let originals_slice = originals.originals.as_mut().unwrap();
    let original_count = originals_slice.len();
    let fresh_calls = source.owned_calls.load(Ordering::SeqCst) - calls;
    for udid in &source.ids[..devices] {
        let start = MeasureInstant::now();
        let (status, body) = response(
            client,
            reqwest::Method::GET,
            format!("{base}/tunnel/{udid}"),
            None,
        )
        .await?;
        evidence.latency("get", scenario, devices, round, start);
        anyhow::ensure!(status == StatusCode::OK, "HTTP record unavailable");
        let record: TunnelRecord = serde_json::from_value(body)?;
        let original = originals_slice
            .iter_mut()
            .find(|original| original.native.udid == *udid)
            .context("published record has no original native owner")?;
        let same = state
            .inner
            .tunnels
            .read()
            .await
            .get(udid)
            .and_then(|tunnel| tunnel.device.native_handle())
            .is_some_and(|published| Arc::ptr_eq(published, &original.native));
        anyhow::ensure!(
            same && record.udid == *udid && record.userspace_port == original.native.userspace_port,
            "HTTP metadata changed original native identity"
        );
        original.proxy =
            Some(TcpStream::connect(("127.0.0.1", original.native.userspace_port.unwrap())).await?);
    }
    tokio::time::timeout(CLEANUP, async {
        while originals_slice.iter().any(|original| {
            !original.native.shutdown_status().is_err_and(|error| {
                native_report(&error).is_some_and(|report| report.pending_joins() >= 5)
            })
        }) {
            tokio::task::yield_now().await;
        }
    })
    .await
    .context("original proxy child was not admitted")?;
    evidence
        .resource(state, "active", scenario, devices, round)
        .await;

    if disconnect {
        let start = MeasureInstant::now();
        for original in originals_slice.iter_mut() {
            original.intentional_disconnect = true;
            // Real FIN on the original native TCP peer. Its read half stays
            // open so native worker teardown must produce a strict peer EOF.
            original.peer.shutdown().await?;
        }
        tokio::time::timeout(CLEANUP, async {
            while originals_slice
                .iter()
                .any(|original| original.native.is_alive())
            {
                tokio::task::yield_now().await;
            }
        })
        .await
        .context("original TCP reader did not acquire EOF")?;
        for udid in &source.ids[..devices] {
            let (status, _) = response(
                client,
                reqwest::Method::GET,
                format!("{base}/tunnel/{udid}"),
                None,
            )
            .await?;
            anyhow::ensure!(
                status == StatusCode::NOT_FOUND,
                "disconnected owner stayed published"
            );
        }
        evidence.latency("disconnect_to_absent", scenario, devices, round, start);
    } else {
        for udid in &source.ids[..devices] {
            let start = MeasureInstant::now();
            let (status, body) = response(
                client,
                reqwest::Method::DELETE,
                format!("{base}/tunnel/{udid}"),
                None,
            )
            .await?;
            evidence.latency("delete_stop", scenario, devices, round, start);
            anyhow::ensure!(
                status == StatusCode::OK && body["data"] == true,
                "HTTP stop failed"
            );
        }
    }
    let cleanup_start = MeasureInstant::now();
    let drain = state.drain_native(CLEANUP).await;
    let certificates = collect_originals(originals_slice).await;
    let snapshot = state.inner.native.snapshot();
    let recovered = snapshot.occupied() == 0
        && state.inner.tunnels.read().await.is_empty()
        && state.inner.lifecycle.connects.available_permits() == lifecycle::MAX_CONNECTS
        && state.inner.failure_slots.available_permits() == MAX_FAILURE_RECORDS;
    evidence.latency("native_cleanup", scenario, devices, round, cleanup_start);
    let joined_and_eof = certificates.validation.is_ok();
    let round_evidence = json!({
        "scenario": scenario, "devices": devices, "round": round,
        "termination": if disconnect {"original-peer-fin"} else {"http-delete"},
        "original_owners": original_count, "source_owned_calls": fresh_calls,
        "native_drain_class": drain.as_ref().err().map(|error| ios_core::classify_error(error.as_ref())),
        "original_certificates": certificates.certificates,
        "strict_native_peer_eof": joined_and_eof, "strict_proxy_peer_eof": joined_and_eof,
        "registry_and_permits_recovered": recovered,
    });
    // Preserve the bounded original certificates even when validation fails.
    evidence.rounds.push(round_evidence);
    certificates.validation?;
    // Original joins and both actual peers are recovered before judging fresh
    // publication/count behavior. Retained EOF remains a native error.
    anyhow::ensure!(
        recovered,
        "manager resources remain occupied after original joins"
    );
    anyhow::ensure!(
        disconnect || drain.is_ok(),
        "clean stop recorded native failure"
    );
    anyhow::ensure!(
        original_count == devices && fresh_calls == devices,
        "round did not acquire one fresh owned runtime per device"
    );
    drop(originals.originals.take()); // Drop certified original Arcs and FDs.
    evidence.flush();
    evidence
        .resource(state, "released", scenario, devices, round)
        .await;
    evidence.latency("round_total", scenario, devices, round, round_start);
    evidence.flush();
    Ok(())
}

async fn measurement(
    input: &Input,
    state: &TunnelAgentState,
    source: &Arc<MeasurementSource>,
    client: &reqwest::Client,
    base: &str,
    evidence: &mut Evidence,
) -> Result<()> {
    let unauthorized = reqwest::Client::builder()
        .no_proxy()
        .timeout(CLEANUP)
        .build()?;
    let reply = unauthorized.get(format!("{base}/tunnels")).send().await?;
    let status = reply.status();
    let _ = reply.bytes().await?;
    drop(unauthorized);
    anyhow::ensure!(
        status == StatusCode::UNAUTHORIZED,
        "manager was not authenticated"
    );
    let (status, body) = response(
        client,
        reqwest::Method::GET,
        format!("{base}/tunnels"),
        None,
    )
    .await?;
    anyhow::ensure!(
        status == StatusCode::OK && body == json!([]),
        "manager warm-up failed"
    );
    anyhow::ensure!(
        source.owned_calls.load(Ordering::SeqCst) == 0,
        "warm-up acquired a device"
    );
    evidence.resource(state, "warmed", "setup", 0, 0).await;
    for devices in [1, 4, MAX_DEVICES] {
        for round in 0..input.samples_per_scale {
            measure_round(
                state,
                source,
                client,
                base,
                evidence,
                RoundSpec {
                    scenario: "scale",
                    devices,
                    round,
                    disconnect: false,
                },
            )
            .await?;
        }
    }
    let start = MeasureInstant::now();
    for round in 0..input.longrun_max_cycles {
        measure_round(
            state,
            source,
            client,
            base,
            evidence,
            RoundSpec {
                scenario: "longrun",
                devices: MAX_DEVICES,
                round,
                disconnect: round % 2 == 1,
            },
        )
        .await?;
        evidence.longrun_cycles = round + 1;
        evidence.longrun_elapsed_ms = start.elapsed().as_millis();
        if evidence.longrun_cycles >= input.longrun_min_cycles
            && evidence.longrun_elapsed_ms >= u128::from(input.longrun_duration_ms)
        {
            return Ok(());
        }
        tokio::time::sleep(Duration::from_millis(input.cycle_pause_ms)).await;
    }
    anyhow::bail!("bounded cycle count exhausted before requested duration/minimum cycles")
}

/// Workload measurement: throughput benchmark over a local duplex TCP pair.
async fn measure_throughput_workload(payload_bytes: usize) -> Result<serde_json::Value> {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    let listener = TcpListener::bind(("127.0.0.1", 0)).await?;
    let address = listener.local_addr()?;
    let (mut client, accepted) = tokio::try_join!(TcpStream::connect(address), listener.accept())?;
    let (mut server, _) = accepted;
    drop(listener);
    client.set_nodelay(true)?;
    server.set_nodelay(true)?;

    let chunk_size = 64 * 1024;
    let total_chunks = payload_bytes.div_ceil(chunk_size).max(1);
    let actual_total_bytes = total_chunks * chunk_size;
    let chunk_data = vec![0x5au8; chunk_size];

    let mut chunk_latencies = Vec::with_capacity(total_chunks);

    let start = MeasureInstant::now();

    let server_task = tokio::spawn(async move {
        let mut buf = vec![0u8; chunk_size];
        let mut received = 0;
        while received < actual_total_bytes {
            let n = server.read_exact(&mut buf).await?;
            received += n;
            server.write_all(&buf[..n]).await?;
        }
        Ok::<_, anyhow::Error>(received)
    });

    let mut echo_buf = vec![0u8; chunk_size];
    for _ in 0..total_chunks {
        let chunk_start = MeasureInstant::now();
        client.write_all(&chunk_data).await?;
        client.read_exact(&mut echo_buf).await?;
        let chunk_dur = chunk_start.elapsed().as_nanos() as u64;
        chunk_latencies.push(chunk_dur);
    }

    let server_received = server_task.await??;
    let elapsed = start.elapsed();
    let elapsed_ns = elapsed.as_nanos() as u64;
    let elapsed_secs = elapsed.as_secs_f64().max(0.000001);
    let bytes_per_sec = (server_received as f64) / elapsed_secs;

    chunk_latencies.sort_unstable();
    let p50 = chunk_latencies[chunk_latencies.len() / 2];
    let p95_idx = ((chunk_latencies.len() as f64) * 0.95).floor() as usize;
    let p95 = chunk_latencies[p95_idx.min(chunk_latencies.len() - 1)];

    Ok(json!({
        "status": "passed",
        "transferred_bytes": actual_total_bytes,
        "elapsed_nanoseconds": elapsed_ns,
        "throughput_bytes_per_sec": bytes_per_sec,
        "p50_chunk_ns": p50,
        "p95_chunk_ns": p95,
    }))
}

/// Workload measurement: file transfer benchmark (chunked write, read back, sha256 checksum).
async fn measure_file_transfer_workload(
    file_count: usize,
    file_size_bytes: usize,
) -> Result<serde_json::Value> {
    use sha2::{Digest, Sha256};
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    let listener = TcpListener::bind(("127.0.0.1", 0)).await?;
    let address = listener.local_addr()?;
    let (mut client, accepted) = tokio::try_join!(TcpStream::connect(address), listener.accept())?;
    let (mut server, _) = accepted;
    drop(listener);
    client.set_nodelay(true)?;
    server.set_nodelay(true)?;

    let total_bytes = file_count * file_size_bytes;

    let server_task = tokio::spawn(async move {
        let mut stored_files = Vec::with_capacity(file_count);
        for _ in 0..file_count {
            let mut file_buf = vec![0u8; file_size_bytes];
            server.read_exact(&mut file_buf).await?;
            stored_files.push(file_buf);
        }
        for file in stored_files {
            server.write_all(&file).await?;
        }
        Ok::<_, anyhow::Error>(())
    });

    let mut generated_files = Vec::with_capacity(file_count);
    for idx in 0..file_count {
        let mut file_data = vec![0u8; file_size_bytes];
        for (b_idx, b) in file_data.iter_mut().enumerate() {
            *b = ((idx * 31 + b_idx) % 251) as u8;
        }
        generated_files.push(file_data);
    }

    let upload_start = MeasureInstant::now();
    for file in &generated_files {
        client.write_all(file).await?;
    }
    let upload_elapsed = upload_start.elapsed();

    let download_start = MeasureInstant::now();
    let mut verified = true;
    for file in &generated_files {
        let mut downloaded = vec![0u8; file_size_bytes];
        client.read_exact(&mut downloaded).await?;
        let expected_hash = Sha256::digest(file);
        let actual_hash = Sha256::digest(&downloaded);
        if expected_hash != actual_hash {
            verified = false;
        }
    }
    let download_elapsed = download_start.elapsed();

    server_task.await??;

    let upload_secs = upload_elapsed.as_secs_f64().max(0.000001);
    let download_secs = download_elapsed.as_secs_f64().max(0.000001);

    Ok(json!({
        "status": "passed",
        "files_transferred": file_count,
        "total_bytes": total_bytes,
        "upload_throughput_bytes_per_sec": (total_bytes as f64) / upload_secs,
        "download_throughput_bytes_per_sec": (total_bytes as f64) / download_secs,
        "integrity_verified": verified,
    }))
}

/// Workload measurement: service stream benchmark (continuous event messaging).
async fn measure_service_stream_workload(
    duration: Duration,
    events_count: usize,
) -> Result<serde_json::Value> {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    let listener = TcpListener::bind(("127.0.0.1", 0)).await?;
    let address = listener.local_addr()?;
    let (mut client, accepted) = tokio::try_join!(TcpStream::connect(address), listener.accept())?;
    let (mut server, _) = accepted;
    drop(listener);
    client.set_nodelay(true)?;
    server.set_nodelay(true)?;

    let server_task = tokio::spawn(async move {
        let mut stream_bytes = 0;
        let mut received_events = 0;
        let mut len_buf = [0u8; 4];
        while received_events < events_count {
            server.read_exact(&mut len_buf).await?;
            let len = u32::from_be_bytes(len_buf) as usize;
            let mut event_buf = vec![0u8; len];
            server.read_exact(&mut event_buf).await?;
            stream_bytes += 4 + len;
            received_events += 1;
        }
        Ok::<_, anyhow::Error>((received_events, stream_bytes))
    });

    let stream_start = MeasureInstant::now();
    let mut total_sent_bytes = 0;
    for seq in 0..events_count {
        let event_line = format!(
            "[service_stream seq={seq} ts={} event=syslog_line_log_entry_payload_data]\n",
            seq * 10
        );
        let event_bytes = event_line.as_bytes();
        let len = event_bytes.len() as u32;
        client.write_all(&len.to_be_bytes()).await?;
        client.write_all(event_bytes).await?;
        total_sent_bytes += 4 + event_bytes.len();
        if duration > Duration::ZERO && seq % 100 == 0 {
            tokio::task::yield_now().await;
        }
    }
    let elapsed = stream_start.elapsed();
    let (received_events, _) = server_task.await??;

    let elapsed_secs = elapsed.as_secs_f64().max(0.000001);
    let events_per_sec = (received_events as f64) / elapsed_secs;
    let bandwidth = (total_sent_bytes as f64) / elapsed_secs;

    Ok(json!({
        "status": "passed",
        "stream_duration_ns": elapsed.as_nanos() as u64,
        "events_streamed": received_events,
        "stream_bytes": total_sent_bytes,
        "events_per_second": events_per_sec,
        "bandwidth_bytes_per_sec": bandwidth,
        "dropped_events": events_count.saturating_sub(received_events),
    }))
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "explicit L04 measured gate; run scripts/manager_resource_baseline.py against this compiled test binary"]
async fn manager_native_resource_baseline() {
    let input = Input::environment().expect("bounded L04 input");
    let before_tasks = baseline();
    let before_process = proc_sample();
    let source = MeasurementSource::new();
    let policy = PolicyFixture::new(vec![grant(
        TOKEN,
        vec![Permission::List, Permission::ManageTunnels],
    )]);
    let mut policy_json: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&policy.path).unwrap()).unwrap();
    policy_json["grants"][0]["devices"] = json!(source.ids);
    ios_core::secret_file::write_secret(&policy.path, &serde_json::to_vec(&policy_json).unwrap())
        .unwrap();
    let access = AccessControl::from_file(policy.path.clone()).await.unwrap();
    let state = TunnelAgentState::with_source(
        TunMode::Userspace,
        access,
        Upstreams::default(),
        source.clone(),
        CLEANUP,
    );
    let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
    let address = listener.local_addr().unwrap();
    let router = crate::access::HttpBoundaryArgs::default()
        .validate()
        .unwrap()
        .for_listener(address)
        .protect(
            agent_router(state.clone()),
            crate::http_diagnostics::ErrorShape::Manager,
        );
    let (stop, stopped) = tokio::sync::oneshot::channel::<()>();
    let mut http = tokio::spawn(async move {
        axum::serve(listener, router)
            .with_graceful_shutdown(async {
                let _ = stopped.await;
            })
            .await
    });
    let client = reqwest::Client::builder()
        .no_proxy()
        .redirect(reqwest::redirect::Policy::none())
        .timeout(CLEANUP)
        .build()
        .unwrap();
    let mut evidence = Evidence::default();
    let total =
        Duration::from_millis(input.longrun_duration_ms).saturating_add(Duration::from_secs(180));
    let measured = tokio::time::timeout(
        total,
        measurement(
            &input,
            &state,
            &source,
            &client,
            &format!("http://{address}"),
            &mut evidence,
        ),
    )
    .await;

    // Cleanup is unconditional after measurement failure/timeout. No original
    // runtime is replaced with AbortHandle/is_alive-only recovery evidence.
    state.notify_shutdown();
    let final_drain = state.drain_native(CLEANUP).await;
    state.clear().await;
    let mut held = HeldOriginals::take(&source);
    let final_originals = collect_originals(held.originals.as_mut().unwrap()).await;
    drop(held.originals.take());
    drop(held);
    drop(client);
    let _ = stop.send(());
    let http_result = tokio::time::timeout(CLEANUP, &mut http).await;
    let http_joined = matches!(&http_result, Ok(Ok(Ok(()))));
    if http_result.is_err() {
        http.abort();
        let _ = http.await; // Owned failure cleanup; never a successful receipt.
    }
    let task_recovery = tokio::time::timeout(CLEANUP, async {
        while baseline() != before_tasks {
            tokio::task::yield_now().await;
        }
    })
    .await
    .is_ok();
    let snapshot = state.inner.native.snapshot();
    let resources_recovered = final_originals.validation.is_ok()
        && http_joined
        && task_recovery
        && snapshot.closed_and_empty()
        && state.inner.lifecycle.connects.available_permits() == lifecycle::MAX_CONNECTS
        && state.inner.failure_slots.available_permits() == MAX_FAILURE_RECORDS;
    let measured_ok = matches!(&measured, Ok(Ok(())));
    let throughput_workload = if input.workload == "idle"
        || (!input.workload.contains("throughput") && input.workload != "all")
    {
        json!({"status": "NOT_RUN", "reason": "workload set to idle"})
    } else {
        measure_throughput_workload(input.workload_payload_kb * 1024)
            .await
            .expect("throughput workload measurement")
    };
    let file_transfer_workload = if input.workload == "idle"
        || (!input.workload.contains("file_transfer") && input.workload != "all")
    {
        json!({"status": "NOT_RUN", "reason": "workload set to idle"})
    } else {
        measure_file_transfer_workload(4, (input.workload_payload_kb * 1024) / 4)
            .await
            .expect("file transfer workload measurement")
    };
    let service_stream_workload = if input.workload == "idle"
        || (!input.workload.contains("service_stream") && input.workload != "all")
    {
        json!({"status": "NOT_RUN", "reason": "workload set to idle"})
    } else {
        measure_service_stream_workload(Duration::from_millis(50), 500)
            .await
            .expect("service stream workload measurement")
    };

    let report = json!({
        "schema_version": 1, "fixture": "production-manager-original-native-host-workers",
        "status": if measured_ok && resources_recovered {"passed"} else {"failed"},
        "input": input, "device_scales": [1, 4, 16],
        "clock": "unpaused-monotonic-Instant", "tokio_worker_threads": 2,
        "rust_debug_assertions": cfg!(debug_assertions), "process_id": std::process::id(),
        "fixture_native_io": "independent-loopback-TCP-per-device",
        "metadata_boundary": "private discovery and device record only",
        "latency_samples": evidence.latencies, "resource_samples": evidence.resources,
        "rounds": evidence.rounds,
        "events_emitted": evidence.events_emitted,
        "instrumentation": "external event stream; only one round of JSON retained in measured process",
        "longrun": {"elapsed_ms": evidence.longrun_elapsed_ms, "cycles": evidence.longrun_cycles, "maximum_concurrent_devices": 16},
        "throughput": throughput_workload,
        "file_transfer": file_transfer_workload,
        "service_stream": service_stream_workload,
        "physical_devices": {"status": "NOT_RUN", "reason": "authorized physical device and USB/Wi-Fi workflows required"},
        "cleanup": {
            "original_joins_and_strict_eof": final_originals.validation.is_ok(),
            "final_original_certificates": final_originals.certificates,
            "http_original_join": http_joined, "tokio_task_recovery": task_recovery,
            "before_tokio_tasks": before_tasks, "after_tokio_tasks": baseline(),
            "registry_closed_and_empty": snapshot.closed_and_empty(),
            "permits_recovered": state.inner.lifecycle.connects.available_permits() == lifecycle::MAX_CONNECTS && state.inner.failure_slots.available_permits() == MAX_FAILURE_RECORDS,
            "final_native_drain_class": final_drain.as_ref().err().map(|error| ios_core::classify_error(error.as_ref())),
            "before_process": before_process, "after_process": proc_sample(),
        },
        "failure": if measured_ok {serde_json::Value::Null} else {json!({
            "phase": if measured.is_err() {"measurement-total"} else {"measurement"},
            "class": match &measured {Ok(Err(error)) => ios_core::classify_error(error.as_ref()), Err(error) => ios_core::classify_error(error), _ => ios_core::ErrorClass::Unknown},
        })},
    });
    println!("{REPORT_PREFIX}{}", serde_json::to_string(&report).unwrap());
    // Assertions follow original joins, strict actual EOF and final recovery.
    assert!(
        resources_recovered,
        "L04 original resource recovery failed; inspect bounded receipt"
    );
    measured
        .expect("L04 measurement total expired")
        .expect("L04 measurement failed");
}

#[tokio::test]
async fn test_throughput_workload_measurement() {
    let result = measure_throughput_workload(64 * 1024).await.unwrap();
    assert_eq!(
        result.get("status").and_then(serde_json::Value::as_str),
        Some("passed")
    );
    assert!(
        result
            .get("throughput_bytes_per_sec")
            .unwrap()
            .as_f64()
            .unwrap()
            > 0.0
    );
}

#[tokio::test]
async fn test_file_transfer_workload_measurement() {
    let result = measure_file_transfer_workload(2, 16 * 1024).await.unwrap();
    assert_eq!(
        result.get("status").and_then(serde_json::Value::as_str),
        Some("passed")
    );
    assert_eq!(
        result
            .get("integrity_verified")
            .and_then(serde_json::Value::as_bool),
        Some(true)
    );
}

#[tokio::test]
async fn test_service_stream_workload_measurement() {
    let result = measure_service_stream_workload(Duration::from_millis(20), 100)
        .await
        .unwrap();
    assert_eq!(
        result.get("status").and_then(serde_json::Value::as_str),
        Some("passed")
    );
    assert_eq!(
        result
            .get("dropped_events")
            .and_then(serde_json::Value::as_u64),
        Some(0)
    );
}
