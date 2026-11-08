//! The real activate_tunnel caller attaches the exact dormant runtime first.
use super::*;
use std::pin::Pin;
use std::sync::atomic::AtomicBool;
use std::task::{Context, Poll};
use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};

#[derive(Clone, Copy, PartialEq, Eq)]
enum Action {
    Accept,
    Reject,
    Stop,
}

#[derive(Default, Debug)]
struct Traffic {
    worker_reads: AtomicUsize,
    worker_writes: AtomicUsize,
    stream_drops: AtomicUsize,
    error_drops: AtomicUsize,
}
struct Stream {
    inner: DuplexStream,
    traffic: Arc<Traffic>,
    response_left: usize,
}
impl AsyncRead for Stream {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        bytes: &mut ReadBuf<'_>,
    ) -> Poll<std::io::Result<()>> {
        if self.response_left == 0 {
            self.traffic.worker_reads.fetch_add(1, Ordering::SeqCst);
        }
        let before = bytes.filled().len();
        let result = Pin::new(&mut self.inner).poll_read(cx, bytes);
        if matches!(&result, Poll::Ready(Ok(()))) {
            self.response_left = self
                .response_left
                .saturating_sub(bytes.filled().len() - before);
        }
        result
    }
}
impl AsyncWrite for Stream {
    fn poll_write(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        bytes: &[u8],
    ) -> Poll<std::io::Result<usize>> {
        if self.response_left == 0 {
            self.traffic.worker_writes.fetch_add(1, Ordering::SeqCst);
        }
        Pin::new(&mut self.inner).poll_write(cx, bytes)
    }
    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        Pin::new(&mut self.inner).poll_flush(cx)
    }
    fn poll_shutdown(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        Pin::new(&mut self.inner).poll_shutdown(cx)
    }
}
impl Drop for Stream {
    fn drop(&mut self) {
        self.traffic.stream_drops.fetch_add(1, Ordering::SeqCst);
    }
}

struct AttachOwner {
    action: Action,
    traffic: Arc<Traffic>,
    bank: Arc<tokio::sync::Semaphore>,
    handle: StdMutex<Option<Arc<TunnelHandle>>>,
    observed_empty: AtomicBool,
    observed_dormant: AtomicBool,
    observed_reads: AtomicUsize,
    observed_writes: AtomicUsize,
    original: AtomicUsize,
    completed: AtomicUsize,
}
struct AttachReservation<'a> {
    owner: &'a AttachOwner,
    attached: Option<Arc<TunnelHandle>>,
    complete: bool,
}
impl TunnelOwner for AttachOwner {
    fn reserve(&self) -> Result<Box<dyn TunnelReservation + '_>, CoreError> {
        Ok(Box::new(AttachReservation {
            owner: self,
            attached: None,
            complete: false,
        }))
    }
}
#[derive(Debug)]
struct Refusal(Arc<Traffic>);
impl std::fmt::Display for Refusal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("original dormant attach refusal")
    }
}
impl std::error::Error for Refusal {}
impl Drop for Refusal {
    fn drop(&mut self) {
        self.0.error_drops.fetch_add(1, Ordering::SeqCst);
    }
}
impl TunnelReservation for AttachReservation<'_> {
    fn byte_bank(&self) -> Option<Arc<tokio::sync::Semaphore>> {
        Some(self.owner.bank.clone())
    }
    fn attach(&mut self, handle: Arc<TunnelHandle>) -> Result<(), CoreError> {
        let status = handle.shutdown_status().unwrap_err();
        let mut cause: &(dyn std::error::Error + 'static) = &status;
        let report = loop {
            if let Some(report) =
                cause.downcast_ref::<crate::tunnel::owner::TunnelShutdownFailure>()
            {
                break report;
            }
            cause = cause.source().expect("dormant launch report");
        };
        self.owner.observed_empty.store(
            report.pending_joins() == 0
                && report.launch_pending()
                && !report.joins_complete()
                && !report.has_retained_outcomes(),
            Ordering::SeqCst,
        );
        self.owner
            .observed_dormant
            .store(!handle.is_alive(), Ordering::SeqCst);
        self.owner.observed_reads.store(
            self.owner.traffic.worker_reads.load(Ordering::SeqCst),
            Ordering::SeqCst,
        );
        self.owner.observed_writes.store(
            self.owner.traffic.worker_writes.load(Ordering::SeqCst),
            Ordering::SeqCst,
        );
        *self.owner.handle.lock().unwrap() = Some(handle.clone());
        self.attached = Some(handle.clone());
        if self.owner.action == Action::Stop {
            handle.request_stop();
        }
        if self.owner.action == Action::Reject {
            let error = std::io::Error::new(
                std::io::ErrorKind::PermissionDenied,
                Refusal(self.owner.traffic.clone()),
            );
            self.owner.original.store(
                error.get_ref().unwrap().downcast_ref::<Refusal>().unwrap() as *const _ as usize,
                Ordering::SeqCst,
            );
            return Err(CoreError::Io(error));
        }
        Ok(())
    }
    fn complete(&mut self) {
        self.complete = true;
        self.owner.completed.fetch_add(1, Ordering::SeqCst);
    }
}
impl Drop for AttachReservation<'_> {
    fn drop(&mut self) {
        if !self.complete {
            if let Some(handle) = &self.attached {
                handle.request_stop();
            }
        }
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn real_activation_attach_accept_reject_and_stop_precede_every_worker_poll_and_probe() {
    for action in [Action::Accept, Action::Reject, Action::Stop] {
        let baseline = tokio::runtime::Handle::current()
            .metrics()
            .num_alive_tasks();
        let quota = 32 * 1024 * 1024;
        let bank = Arc::new(tokio::sync::Semaphore::new(quota));
        let traffic = Arc::new(Traffic::default());
        let owner = AttachOwner {
            action,
            traffic: traffic.clone(),
            bank: bank.clone(),
            handle: StdMutex::new(None),
            observed_empty: AtomicBool::new(false),
            observed_dormant: AtomicBool::new(false),
            observed_reads: AtomicUsize::new(usize::MAX),
            observed_writes: AtomicUsize::new(usize::MAX),
            original: AtomicUsize::new(0),
            completed: AtomicUsize::new(0),
        };
        let server = "invalid-rsd-address";
        let response = serde_json::to_vec(&serde_json::json!({
            "serverAddress": server, "serverRSDPort": 58783,
            "clientParameters": { "address": "fd00::2", "mtu": 1280 },
        }))
        .unwrap();
        let (stream, peer_task) = peer(server);
        let result = activate_tunnel(
            connection(),
            Stream {
                inner: stream,
                traffic: traffic.clone(),
                response_left: 10 + response.len(),
            },
            TunMode::Userspace,
            Some(owner.reserve_owned().unwrap()),
        )
        .await;
        let handle = owner.handle.lock().unwrap().as_ref().unwrap().clone();
        let same = result
            .as_ref()
            .ok()
            .and_then(ConnectedDevice::tunnel_handle)
            .is_some_and(|returned| Arc::ptr_eq(returned, &handle));
        let cleanup = handle.shutdown(OBSERVE).await;
        let mut peer = peer_task.await.unwrap();
        let eof = tokio::time::timeout(OBSERVE, peer.read(&mut [0; 1])).await;
        tokio::time::timeout(OBSERVE, async {
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
        cleanup.unwrap();
        handle.shutdown_status().unwrap();
        assert!(matches!(eof, Ok(Ok(0))));
        assert_eq!(traffic.stream_drops.load(Ordering::SeqCst), 1);
        assert!(owner.observed_empty.load(Ordering::SeqCst));
        assert!(owner.observed_dormant.load(Ordering::SeqCst));
        assert_eq!(owner.observed_reads.load(Ordering::SeqCst), 0);
        assert_eq!(owner.observed_writes.load(Ordering::SeqCst), 0);
        match action {
            Action::Accept => {
                assert!(result.is_ok() && same);
                assert_eq!(owner.completed.load(Ordering::SeqCst), 1);
            }
            Action::Reject => {
                let error = result.as_ref().err().unwrap();
                let CoreError::Io(io) = error else {
                    panic!("attach error variant changed")
                };
                assert_eq!(io.kind(), std::io::ErrorKind::PermissionDenied);
                assert_eq!(
                    io.get_ref().unwrap().downcast_ref::<Refusal>().unwrap() as *const _ as usize,
                    owner.original.load(Ordering::SeqCst)
                );
                assert_eq!(traffic.error_drops.load(Ordering::SeqCst), 0);
                assert_eq!(traffic.worker_reads.load(Ordering::SeqCst), 0);
                assert_eq!(traffic.worker_writes.load(Ordering::SeqCst), 0);
                assert_eq!(owner.completed.load(Ordering::SeqCst), 0);
            }
            Action::Stop => {
                assert_eq!(
                    crate::classify_error(result.as_ref().err().unwrap()),
                    crate::ErrorClass::Cancelled
                );
                assert_eq!(traffic.worker_reads.load(Ordering::SeqCst), 0);
                assert_eq!(traffic.worker_writes.load(Ordering::SeqCst), 0);
                assert_eq!(owner.completed.load(Ordering::SeqCst), 0);
            }
        }
        drop((result, handle, owner));
        assert_eq!(bank.available_permits(), quota);
        assert_eq!(
            traffic.error_drops.load(Ordering::SeqCst),
            usize::from(action == Action::Reject)
        );
    }
}
