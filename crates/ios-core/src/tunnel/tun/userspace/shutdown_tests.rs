use super::*;
use std::pin::Pin;
use std::sync::atomic::AtomicUsize;
use std::task::{Context, Poll};
use tokio::io::{AsyncRead, DuplexStream, ReadBuf};

const OBSERVATION: Duration = Duration::from_secs(1);

#[derive(Default)]
struct Probe {
    reads: AtomicUsize,
    writes: AtomicUsize,
    drops: AtomicUsize,
    pending_write: AtomicBool,
    panic_read: AtomicBool,
}

struct Tracked {
    inner: DuplexStream,
    probe: Arc<Probe>,
}

impl AsyncRead for Tracked {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<std::io::Result<()>> {
        self.probe.reads.fetch_add(1, Ordering::SeqCst);
        assert!(
            !self.probe.panic_read.load(Ordering::SeqCst),
            "controlled native reader panic marker"
        );
        Pin::new(&mut self.inner).poll_read(cx, buf)
    }
}

impl tokio::io::AsyncWrite for Tracked {
    fn poll_write(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        bytes: &[u8],
    ) -> Poll<std::io::Result<usize>> {
        let result = Pin::new(&mut self.inner).poll_write(cx, bytes);
        match &result {
            Poll::Ready(Ok(count)) => {
                self.probe.writes.fetch_add(*count, Ordering::SeqCst);
            }
            Poll::Pending => {
                self.probe.pending_write.store(true, Ordering::SeqCst);
            }
            Poll::Ready(Err(_)) => {}
        }
        result
    }
    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        Pin::new(&mut self.inner).poll_flush(cx)
    }
    fn poll_shutdown(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        Pin::new(&mut self.inner).poll_shutdown(cx)
    }
}

impl Drop for Tracked {
    fn drop(&mut self) {
        self.probe.drops.fetch_add(1, Ordering::SeqCst);
    }
}

fn stream(capacity: usize) -> (Tracked, DuplexStream, Arc<Probe>) {
    let (inner, peer) = tokio::io::duplex(capacity);
    let probe = Arc::new(Probe::default());
    (
        Tracked {
            inner,
            probe: probe.clone(),
        },
        peer,
        probe,
    )
}

fn alive() -> usize {
    tokio::runtime::Handle::current()
        .metrics()
        .num_alive_tasks()
}

async fn until(condition: impl Fn() -> bool) -> bool {
    tokio::time::timeout(OBSERVATION, async {
        while !condition() {
            tokio::task::yield_now().await;
        }
    })
    .await
    .is_ok()
}

#[derive(Debug)]
struct Completion {
    drops_before_owner_drop: usize,
    tasks_before_owner_drop: usize,
}

fn completion(probe: &Probe) -> Completion {
    Completion {
        drops_before_owner_drop: probe.drops.load(Ordering::SeqCst),
        tasks_before_owner_drop: alive(),
    }
}

// Capture production result and immediate resource state BEFORE caller Drop.
// Recovery cannot turn a premature successful join into passing evidence.
async fn recover(
    mut peer: DuplexStream,
    probe: &Probe,
    baseline: usize,
) -> (Option<Vec<u8>>, bool) {
    let mut bytes = Vec::new();
    let read = tokio::time::timeout(OBSERVATION, peer.read_to_end(&mut bytes)).await;
    let observed = matches!(read, Ok(Ok(_))).then_some(bytes);
    drop(peer);
    let recovered = until(|| alive() == baseline && probe.drops.load(Ordering::SeqCst) == 1).await;
    (observed, recovered)
}

fn assert_joined(observed: Completion, recovered: bool, baseline: usize) {
    assert!(recovered, "fixture failed to restore live runtime baseline");
    assert_eq!(
        observed.drops_before_owner_drop, 1,
        "shutdown did not release stream before final owner Drop"
    );
    assert_eq!(
        observed.tasks_before_owner_drop, baseline,
        "shutdown left native tasks before final owner Drop"
    );
}

#[cfg(feature = "tunnel-userspace")]
fn info() -> crate::tunnel::TunnelInfo {
    crate::tunnel::TunnelInfo {
        server_address: "fd00::2".into(),
        server_rsd_port: 58783,
        client_address: "fd00::1".into(),
        client_mtu: 1280,
    }
}

fn failure(error: &TunnelError) -> &crate::tunnel::TunnelShutdownFailure {
    let mut current: Option<&(dyn std::error::Error + 'static)> = Some(error);
    for _ in 0..32 {
        let cause = current.expect("shutdown failure payload missing");
        if let Some(failure) = cause.downcast_ref() {
            return failure;
        }
        current = cause.source();
    }
    panic!("shutdown source chain exceeded fixture limit");
}

#[tokio::test]
async fn native_shutdown_userspace_joins_before_retained_owner_drop() {
    let baseline = alive();
    for _ in 0..3 {
        let (stream, peer, probe) = stream(128);
        let runtime = UserspaceTunDevice::start("fd00::1", "fd00::2", 1280, stream)
            .await
            .unwrap();
        #[cfg(feature = "tunnel-userspace")]
        let owner = Arc::new(crate::tunnel::TunnelHandle::new_userspace(
            "native-shutdown-test".into(),
            info(),
            runtime,
        ));
        #[cfg(not(feature = "tunnel-userspace"))]
        let owner = Arc::new(runtime);
        let retained = owner.clone();
        let entered = until(|| probe.reads.load(Ordering::SeqCst) > 0).await;
        let result = owner.shutdown(OBSERVATION).await;
        let stopped = !retained.is_alive();
        let observed = completion(&probe);
        let repeated = retained.shutdown(OBSERVATION).await;
        drop(owner);
        drop(retained);
        let (bytes, recovered) = recover(peer, &probe, baseline).await;
        assert!(entered, "actual packet reader was never polled");
        assert!(stopped, "retained Arc bypassed terminal stop");
        result.unwrap();
        repeated.unwrap();
        assert!(bytes.is_some(), "peer did not observe EOF");
        assert_joined(observed, recovered, baseline);
    }
}

#[tokio::test]
async fn native_shutdown_userspace_joins_stalled_proxy_clients() {
    let baseline = alive();
    let (stream, peer, probe) = stream(4096);
    let runtime = UserspaceTunDevice::start("fd00::1", "fd00::2", 1280, stream)
        .await
        .unwrap();
    let mut partial =
        tokio::net::TcpStream::connect((std::net::Ipv4Addr::LOCALHOST, runtime.local_port))
            .await
            .unwrap();
    partial.write_all(&[0x20]).await.unwrap();
    let mut admitted =
        tokio::net::TcpStream::connect((std::net::Ipv4Addr::LOCALHOST, runtime.local_port))
            .await
            .unwrap();
    let mut preamble = "fd00::2"
        .parse::<std::net::Ipv6Addr>()
        .unwrap()
        .octets()
        .to_vec();
    preamble.extend_from_slice(&62078u32.to_le_bytes());
    admitted.write_all(&preamble).await.unwrap();
    let children_entered =
        until(|| alive() == baseline + 6 && probe.writes.load(Ordering::SeqCst) > 0).await;
    let result = runtime.shutdown(OBSERVATION).await;
    let observed = completion(&probe);
    let mut byte = [0; 1];
    let first_closed = tokio::time::timeout(OBSERVATION, partial.read(&mut byte)).await;
    let second_closed = tokio::time::timeout(OBSERVATION, admitted.read(&mut byte)).await;
    drop(runtime);
    drop(partial);
    drop(admitted);
    let (bytes, recovered) = recover(peer, &probe, baseline).await;
    assert!(
        children_entered,
        "actual partial-preamble and awaiting-device proxy children did not both enter"
    );
    result.unwrap();
    assert!(matches!(first_closed, Ok(Ok(0) | Err(_))));
    assert!(matches!(second_closed, Ok(Ok(0) | Err(_))));
    assert!(bytes.is_some());
    assert_joined(observed, recovered, baseline);
}

#[tokio::test]
async fn native_shutdown_userspace_joins_actual_pending_packet_write() {
    let baseline = alive();
    let (stream, peer, probe) = stream(1);
    let runtime = UserspaceTunDevice::start("fd00::1", "fd00::2", 1280, stream)
        .await
        .unwrap();
    let mut client =
        tokio::net::TcpStream::connect((std::net::Ipv4Addr::LOCALHOST, runtime.local_port))
            .await
            .unwrap();
    let mut preamble = "fd00::2"
        .parse::<std::net::Ipv6Addr>()
        .unwrap()
        .octets()
        .to_vec();
    preamble.extend_from_slice(&62078u32.to_le_bytes());
    client.write_all(&preamble).await.unwrap();
    let entered = until(|| probe.pending_write.load(Ordering::SeqCst)).await;
    let bytes_written = probe.writes.load(Ordering::SeqCst);
    let result = runtime.shutdown(OBSERVATION).await;
    let observed = completion(&probe);
    drop(runtime);
    drop(client);
    let (bytes, recovered) = recover(peer, &probe, baseline).await;
    assert!(entered, "actual capacity-one packet writer did not block");
    result.unwrap();
    assert_eq!(bytes_written, 1);
    assert_eq!(bytes, Some(vec![0x60]));
    assert_joined(observed, recovered, baseline);
}

#[tokio::test]
#[cfg(feature = "tunnel-userspace")]
async fn native_shutdown_userspace_manager_replacement_and_retire_stop_shared_owners() {
    let baseline = alive();
    let (first_stream, first_peer, first_probe) = stream(128);
    let first_runtime = UserspaceTunDevice::start("fd00::1", "fd00::2", 1280, first_stream)
        .await
        .unwrap();
    let first = Arc::new(crate::tunnel::TunnelHandle::new_userspace(
        "shared-native-owner".into(),
        info(),
        first_runtime,
    ));
    let (second_stream, second_peer, second_probe) = stream(128);
    let second_runtime = UserspaceTunDevice::start("fd00::1", "fd00::2", 1280, second_stream)
        .await
        .unwrap();
    let second = Arc::new(crate::tunnel::TunnelHandle::new_userspace(
        "shared-native-owner".into(),
        info(),
        second_runtime,
    ));
    let manager = crate::tunnel::TunnelManager::default();
    manager.register(first.clone()).await;
    manager.register(first.clone()).await;
    let same_owner_stayed_live = first.is_alive();
    manager.register(second.clone()).await;
    let old_stopped_new_live = !first.is_alive() && second.is_alive();
    let first_result = first.shutdown(OBSERVATION).await;
    let first_completed = completion(&first_probe);
    let retired = manager.retire("shared-native-owner").await;
    let transferred_original = retired
        .as_ref()
        .is_some_and(|owner| Arc::ptr_eq(owner, &second));
    let shared_stopped = !second.is_alive();
    let second_result = second.shutdown(OBSERVATION).await;
    let second_completed = completion(&second_probe);
    let no_record = manager.list().await.is_empty();
    drop(retired);
    drop(manager);
    drop(first);
    drop(second);
    let (first_bytes, first_recovered) = recover(first_peer, &first_probe, baseline).await;
    let (second_bytes, second_recovered) = recover(second_peer, &second_probe, baseline).await;
    assert!(
        same_owner_stayed_live,
        "registering the same Arc stopped its runtime"
    );
    assert!(
        old_stopped_new_live,
        "replacement failed to signal the displaced shared owner"
    );
    assert!(transferred_original && shared_stopped && no_record);
    first_result.unwrap();
    second_result.unwrap();
    assert_eq!(first_completed.drops_before_owner_drop, 1);
    assert!(first_recovered && first_bytes.is_some() && second_bytes.is_some());
    assert_joined(second_completed, second_recovered, baseline);
}

#[tokio::test]
async fn native_shutdown_userspace_actual_partial_header_eof_retains_original_failure() {
    let baseline = alive();
    let (stream, mut peer, probe) = stream(128);
    let runtime = UserspaceTunDevice::start("fd00::1", "fd00::2", 1280, stream)
        .await
        .unwrap();
    peer.write_all(&[0x60; 20]).await.unwrap();
    peer.shutdown().await.unwrap();
    let failed_reader = until(|| !runtime.is_alive()).await;
    let result = runtime.shutdown(OBSERVATION).await;
    let observed = completion(&probe);
    let repeated = runtime.shutdown(OBSERVATION).await;
    drop(runtime);
    let (bytes, recovered) = recover(peer, &probe, baseline).await;
    assert!(
        failed_reader,
        "actual partial header EOF never reached native reader"
    );
    let error = result.unwrap_err();
    let repeated_error = repeated.unwrap_err();
    let evidence = failure(&error);
    let TunnelError::Io(native) = evidence.primary().downcast_ref::<TunnelError>().unwrap() else {
        panic!("original native IO missing");
    };
    assert_eq!(native.kind(), std::io::ErrorKind::UnexpectedEof);
    assert_eq!(crate::classify_error(&error), crate::ErrorClass::Io);
    assert!(std::ptr::eq(
        evidence.primary(),
        failure(&repeated_error).primary()
    ));
    assert_eq!(evidence.pending_joins(), 0);
    assert!(bytes.is_some());
    assert_joined(observed, recovered, baseline);
}

#[tokio::test]
async fn native_shutdown_userspace_reader_panic_retains_real_join_error() {
    let baseline = alive();
    let (stream, peer, probe) = stream(128);
    probe.panic_read.store(true, Ordering::SeqCst);
    let runtime = UserspaceTunDevice::start("fd00::1", "fd00::2", 1280, stream)
        .await
        .unwrap();
    let native_failure = until(|| !runtime.is_alive()).await;
    let result = runtime.shutdown(OBSERVATION).await;
    let observed = completion(&probe);
    drop(runtime);
    let (bytes, recovered) = recover(peer, &probe, baseline).await;
    assert!(native_failure && probe.reads.load(Ordering::SeqCst) > 0);
    let error = result.unwrap_err();
    let evidence = failure(&error);
    assert!(evidence
        .primary()
        .downcast_ref::<tokio::task::JoinError>()
        .unwrap()
        .is_panic());
    assert_eq!(crate::classify_error(&error), crate::ErrorClass::Unknown);
    assert_eq!(evidence.pending_joins(), 0);
    assert!(!format!("{error:?} {evidence:?}").contains("controlled native reader panic marker"));
    assert!(bytes.is_some());
    assert_joined(observed, recovered, baseline);
}
