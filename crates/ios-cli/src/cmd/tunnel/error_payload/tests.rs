use super::*;
use crate::cmd::tunnel::{scan, TunnelAgentState};
use std::sync::Arc;
use tokio::sync::Mutex;

fn payload(capacity: usize) -> String {
    let mut value = String::with_capacity(capacity);
    value.push_str("synthetic-private-payload");
    value
}

#[test]
fn payload_census_measures_capacity_preserves_original_and_deduplicates_shared_roots() {
    let value = payload(8192);
    let capacity = value.capacity();
    let root = Arc::new(CoreError::Mux(MuxError::Protocol(value)));
    let pointer = Arc::as_ptr(&root);
    let mut census = Census::default();
    census.error(root.as_ref(), 0);
    census.error(root.as_ref(), 0);
    assert_eq!(census.snapshot.known_payload_capacity_bytes, capacity);
    assert!(census.snapshot.fully_measured_payloads());
    assert_eq!(census.snapshot.visited_nodes, 2);
    assert_eq!(Arc::as_ptr(&root), pointer);
    assert!(
        matches!(root.as_ref(), CoreError::Mux(MuxError::Protocol(value))
        if value == "synthetic-private-payload" && value.capacity() == capacity)
    );
    assert!(!format!("{:?}", census.snapshot).contains("synthetic-private"));
}

struct Opaque(Vec<u8>);
impl std::fmt::Debug for Opaque {
    fn fmt(&self, _: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        panic!("payload census must not call Debug")
    }
}
impl std::fmt::Display for Opaque {
    fn fmt(&self, _: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        panic!("payload census must not call Display")
    }
}
impl Error for Opaque {}

#[test]
fn payload_census_opaque_heap_and_io_custom_payload_cannot_claim_zero_measured_bytes() {
    let error = std::io::Error::other(Opaque(vec![7; 1024 * 1024]));
    let original = error.get_ref().unwrap().downcast_ref::<Opaque>().unwrap();
    let buffer = original.0.as_ptr();
    let mut census = Census::default();
    census.error(&error, 0);
    assert_eq!(census.snapshot.known_payload_capacity_bytes, 0);
    assert_eq!(census.snapshot.opaque_nodes, 1);
    assert!(!census.snapshot.fully_measured_payloads());
    assert_eq!(original.0.len(), 1024 * 1024);
    assert_eq!(original.0.as_ptr(), buffer);
}

#[test]
fn payload_census_anyhow_independent_cleanup_and_string_capacities_are_retained() {
    let first = payload(4096);
    let second = payload(8192);
    let capacity = first.capacity() + second.capacity();
    let root = anyhow::Error::new(CoreError::Other(first)).context(
        crate::cmd::tunnel::bookkeeping::FailureContext::new(anyhow::Error::new(
            TunnelError::Protocol(second),
        )),
    );
    let mut census = Census::default();
    census.anyhow(&root, 0);
    assert_eq!(census.snapshot.known_payload_capacity_bytes, capacity);
    // Anyhow's context container has opaque backing; known payload is a lower bound.
    assert!(!census.snapshot.fully_measured_payloads());
    assert!(root.downcast_ref::<CoreError>().is_some());
    let text = payload(16384);
    let expected = text.capacity();
    let error = anyhow::Error::msg(text);
    let mut census = Census::default();
    census.anyhow(&error, 0);
    assert_eq!(census.snapshot.known_payload_capacity_bytes, expected);
    assert!(!census.snapshot.fully_measured_payloads());
}

#[derive(Debug)]
struct Cycle;
impl std::fmt::Display for Cycle {
    fn fmt(&self, _: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        panic!("cycle must not be formatted")
    }
}
impl Error for Cycle {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        Some(self)
    }
}

#[derive(Debug)]
struct Chain(Option<Box<Chain>>);
impl std::fmt::Display for Chain {
    fn fmt(&self, _: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        panic!("chain must not be formatted")
    }
}
impl Error for Chain {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        self.0.as_deref().map(|source| source as _)
    }
}

#[test]
fn payload_census_cycles_and_deep_chains_are_bounded_and_report_incomplete() {
    let cycle = Cycle;
    let mut census = Census::default();
    census.error(&cycle, 0);
    assert_eq!(census.snapshot.visited_nodes, 1);
    assert_eq!(census.snapshot.opaque_nodes, 1);
    let chain = (0..64).fold(Chain(None), |next, _| Chain(Some(Box::new(next))));
    let mut census = Census::default();
    census.error(&chain, 0);
    assert!(census.snapshot.truncated);
    assert_eq!(census.snapshot.visited_nodes, MAX_DEPTH);
    assert!(!census.snapshot.fully_measured_payloads());
}

#[tokio::test]
async fn payload_census_scan_originals_and_independent_sources_survive_abandonment() {
    let outcomes = scan::Outcomes::new();
    let lock = Mutex::new(());
    let session = outcomes.begin(lock.lock().await).unwrap();
    let usb = payload(4096);
    let paired = payload(8192);
    let coordinator = payload(16384);
    let expected = usb.capacity() + paired.capacity() + coordinator.capacity();
    session.retain(scan::Source::Usb, CoreError::Other(usb).into());
    session.retain(scan::Source::Paired, TunnelError::Protocol(paired).into());
    session.retain(
        scan::Source::Coordinator,
        MuxError::Protocol(coordinator).into(),
    );
    assert_eq!(outcomes.payload_history().0, 1);
    drop(session);
    let (active, roots) = outcomes.payload_history();
    assert_eq!(active, 0);
    assert_eq!(roots.len(), 1);
    let mut census = Census::default();
    census.anyhow(roots[0].original(), 0);
    assert_eq!(census.snapshot.known_payload_capacity_bytes, expected);
    assert!(!census.snapshot.fully_measured_payloads());
    assert!(roots[0].original().downcast_ref::<CoreError>().is_some());
    let context = roots[0]
        .original()
        .downcast_ref::<scan::SourceContext>()
        .unwrap();
    assert!(context
        .secondary(scan::Source::Paired)
        .unwrap()
        .is::<TunnelError>());
}

#[tokio::test]
async fn payload_census_manager_marks_active_scan_unmeasured_and_retains_abandoned_root() {
    let state = TunnelAgentState::new(ios_core::TunMode::Userspace);
    let session = state
        .inner
        .scan_outcomes
        .begin(state.inner.scan_lock.lock().await)
        .unwrap();
    let value = payload(65536);
    let expected = value.capacity();
    session.retain(scan::Source::Usb, CoreError::Other(value).into());
    let before = state.retained_error_payloads();
    assert_eq!(before.active_scans, 1);
    assert!(!before.fully_measured_payloads());
    drop(session);
    let after = state.retained_error_payloads();
    assert_eq!(after.active_scans, 0);
    assert_eq!(after.known_payload_capacity_bytes, expected);
    let again = state.retained_error_payloads();
    assert_eq!(again, after);
    state.notify_shutdown();
    state.clear().await;
    // Scan history is intentionally retained for the original terminal return.
    assert_eq!(
        state.retained_error_payloads().known_payload_capacity_bytes,
        expected
    );
}

struct PayloadSource {
    calls: std::sync::atomic::AtomicUsize,
    attached: std::sync::atomic::AtomicBool,
}

impl crate::cmd::tunnel::Source for PayloadSource {
    fn connect<'a>(
        &'a self,
        _: &'a str,
        _: crate::cmd::tunnel::Route,
        _: ios_core::TunMode,
    ) -> futures_util::future::BoxFuture<'a, anyhow::Result<crate::cmd::tunnel::ManagedDevice>>
    {
        Box::pin(async move {
            let call = self.calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            Err(CoreError::Other(payload(4096 << call)).into())
        })
    }

    fn usb_devices(&self) -> futures_util::future::BoxFuture<'_, crate::cmd::tunnel::UsbSnapshot> {
        Box::pin(async {
            crate::cmd::tunnel::UsbSnapshot::Devices(
                if self.attached.load(std::sync::atomic::Ordering::SeqCst) {
                    vec![crate::cmd::tunnel::source::UsbDevice {
                        udid: "payload-fixture".into(),
                        device_id: 7,
                        usb: true,
                    }]
                } else {
                    Vec::new()
                },
            )
        })
    }

    fn paired_devices(
        &self,
    ) -> futures_util::future::BoxFuture<'_, anyhow::Result<Vec<ios_core::PairedMobdev2Device>>>
    {
        Box::pin(async { Ok(Vec::new()) })
    }
}

#[tokio::test]
async fn payload_census_manager_first_latest_share_original_and_release_on_observed_detach() {
    use std::sync::atomic::Ordering;
    let source = Arc::new(PayloadSource {
        calls: std::sync::atomic::AtomicUsize::new(0),
        attached: std::sync::atomic::AtomicBool::new(true),
    });
    let state = TunnelAgentState::with_source(
        ios_core::TunMode::Userspace,
        crate::access::AccessControl::trusted_test_local(),
        crate::access::Upstreams::default(),
        source.clone(),
        std::time::Duration::from_secs(1),
    );
    state.refresh_once().await.unwrap();
    let first = state.inner.retry.receipt("payload-fixture").unwrap();
    let original = first.primary().downcast_ref::<CoreError>().unwrap() as *const CoreError;
    assert_eq!(
        state.retained_error_payloads().known_payload_capacity_bytes,
        4096 + "payload-fixture".len()
    );
    state
        .inner
        .failed_attempts
        .write()
        .await
        .get_mut("payload-fixture")
        .unwrap()
        .last_attempt -= std::time::Duration::from_secs(31);
    state.refresh_once().await.unwrap();
    let latest = state.inner.retry.receipt("payload-fixture").unwrap();
    assert_eq!(
        latest.primary().downcast_ref::<CoreError>().unwrap() as *const CoreError,
        original
    );
    assert_ne!(
        latest.latest().downcast_ref::<CoreError>().unwrap() as *const CoreError,
        original
    );
    assert_eq!(
        state.retained_error_payloads().known_payload_capacity_bytes,
        4096 + 8192 + 2 * "payload-fixture".len()
    );
    assert_eq!(source.calls.load(Ordering::SeqCst), 2);
    drop(first);
    drop(latest);
    source.attached.store(false, Ordering::SeqCst);
    state.refresh_once().await.unwrap();
    assert_eq!(state.retained_error_payloads(), Snapshot::default());
    state.notify_shutdown();
    state.clear().await;
}

struct NativeFault {
    stream: tokio::io::DuplexStream,
    failure: Option<CoreError>,
}
impl tokio::io::AsyncRead for NativeFault {
    fn poll_read(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
        buf: &mut tokio::io::ReadBuf<'_>,
    ) -> std::task::Poll<std::io::Result<()>> {
        if let Some(error) = self.failure.take() {
            std::task::Poll::Ready(Err(std::io::Error::other(error)))
        } else {
            std::pin::Pin::new(&mut self.stream).poll_read(cx, buf)
        }
    }
}
impl tokio::io::AsyncWrite for NativeFault {
    fn poll_write(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
        data: &[u8],
    ) -> std::task::Poll<std::io::Result<usize>> {
        std::pin::Pin::new(&mut self.stream).poll_write(cx, data)
    }
    fn poll_flush(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<std::io::Result<()>> {
        std::pin::Pin::new(&mut self.stream).poll_flush(cx)
    }
    fn poll_shutdown(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<std::io::Result<()>> {
        std::pin::Pin::new(&mut self.stream).poll_shutdown(cx)
    }
}

#[tokio::test]
async fn payload_census_native_join_exposes_original_and_history_retains_same_payload() {
    use tokio::io::AsyncReadExt;
    let baseline = tokio::runtime::Handle::current()
        .metrics()
        .num_alive_tasks();
    let state = TunnelAgentState::new(ios_core::TunMode::Userspace);
    let (stream, mut peer) = tokio::io::duplex(1280);
    let runtime = ios_core::UserspaceTunDevice::start(
        "fd00::2",
        "fd00::1",
        1280,
        NativeFault {
            stream,
            failure: Some(CoreError::Other(payload(16384))),
        },
    )
    .await
    .unwrap();
    let native = Arc::new(ios_core::TunnelHandle::new_userspace(
        "payload-fixture".into(),
        ios_core::TunnelInfo {
            server_address: "fd00::1".into(),
            server_rsd_port: 58783,
            client_address: "fd00::2".into(),
            client_mtu: 1280,
        },
        runtime,
    ));
    let lease = state
        .inner
        .native
        .reserve(crate::cmd::tunnel::retirement::OwnershipStamp::new(
            Arc::new(()),
            Arc::new(()),
        ))
        .unwrap()
        .attach(native.clone());
    tokio::time::timeout(std::time::Duration::from_secs(2), async {
        while native.is_alive() {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    let pending = state.retained_error_payloads();
    assert_eq!(pending.unjoined_native_owners, 1);
    assert_eq!(pending.known_payload_capacity_bytes, 0);
    assert!(!pending.fully_measured_payloads());
    assert!(native
        .shutdown(std::time::Duration::from_secs(2))
        .await
        .is_err());
    let live = state.retained_error_payloads();
    assert_eq!(live.known_payload_capacity_bytes, 16384);
    assert_eq!(live.unjoined_native_owners, 0);
    assert!(!live.fully_measured_payloads());
    drop(lease);
    let result = state.drain_native(std::time::Duration::from_secs(2)).await;
    assert!(result.is_err());
    assert_eq!(state.inner.native.snapshot().occupied(), 0);
    assert_eq!(
        state.retained_error_payloads().known_payload_capacity_bytes,
        16384
    );
    assert_eq!(
        state.retained_error_payloads().known_payload_capacity_bytes,
        16384
    );
    assert!(
        tokio::time::timeout(std::time::Duration::from_secs(2), peer.read_u8())
            .await
            .unwrap()
            .is_err_and(|error| error.kind() == std::io::ErrorKind::UnexpectedEof)
    );
    tokio::time::timeout(std::time::Duration::from_secs(2), async {
        while tokio::runtime::Handle::current()
            .metrics()
            .num_alive_tasks()
            != baseline
        {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    state.notify_shutdown();
    state.clear().await;
    drop(native);
}
