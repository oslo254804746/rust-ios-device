//! Scripted authenticated transport only; native interface APIs are never called.
use super::*;
use crate::{TunnelOwner, TunnelReservation};
use std::{
    error::Error,
    pin::Pin,
    sync::{
        atomic::{AtomicUsize, Ordering},
        Mutex,
    },
    task::{Context, Poll},
    time::Duration,
};
use tokio::{
    io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt, DuplexStream, ReadBuf},
    sync::{OwnedSemaphorePermit, Semaphore},
    time::Instant,
};

const TOTAL: usize = 32 * 1024 * 1024;
const OBSERVE: Duration = Duration::from_secs(4);

#[tokio::test]
async fn preparation_frame_refusal_closes_unpolled_transport_before_handshake_or_attach() {
    let bank = Arc::new(Semaphore::new(0));
    let owner = Owner {
        bank: bank.clone(),
        mode: TunMode::Kernel,
        attached: Mutex::new(None),
    };
    let (stream, mut peer) = tokio::io::duplex(64);
    let connection = TunnelConnection::new(
        DeviceInfo {
            udid: String::new(),
            device_id: 0,
            connection_type: String::new(),
            product_id: 0,
        },
        None,
        LockdownTransport::Tcp {
            host: String::new(),
        },
        "future-size-only",
    );
    let error = prepare_tunnel(
        connection,
        stream,
        TunMode::Kernel,
        Some(owner.reserve_owned().unwrap()),
        Prepared {
            deadline: Instant::now() + OBSERVE,
            bank: &bank,
        },
    )
    .await
    .err()
    .unwrap();
    assert_eq!(error.class(), crate::ErrorClass::ResourceLimit);
    assert!(owner.attached.lock().unwrap().is_none());
    assert_eq!(peer.read(&mut [0; 1]).await.unwrap(), 0);
    assert_eq!(bank.available_permits(), 0);
}

struct Owner {
    bank: Arc<Semaphore>,
    mode: TunMode,
    attached: Mutex<Option<Arc<TunnelHandle>>>,
}
struct Reservation<'a>(&'a Owner);
impl TunnelOwner for Owner {
    fn reserve(&self) -> Result<Box<dyn TunnelReservation + '_>, CoreError> {
        Ok(Box::new(Reservation(self)))
    }
}
impl TunnelReservation for Reservation<'_> {
    fn byte_bank(&self) -> Option<Arc<Semaphore>> {
        Some(self.0.bank.clone())
    }
    fn attach(&mut self, handle: Arc<TunnelHandle>) -> Result<(), CoreError> {
        assert!(!handle.is_alive());
        assert!(!handle.ready_for_publication());
        assert_eq!(handle.prepared_mode(), Some(self.0.mode));
        assert_eq!(
            handle.userspace_port.is_none(),
            self.0.mode == TunMode::Kernel
        );
        *self.0.attached.lock().unwrap() = Some(handle);
        Ok(())
    }
    fn complete(&mut self) {
        panic!("failed private verification cannot transfer preparation")
    }
}

#[derive(Debug)]
struct Original {
    native: std::io::Error,
    drops: Arc<AtomicUsize>,
}
impl std::fmt::Display for Original {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("actual localhost refusal held by scripted packet read")
    }
}
impl Error for Original {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        Some(&self.native)
    }
}
impl Drop for Original {
    fn drop(&mut self) {
        self.drops.fetch_add(1, Ordering::SeqCst);
    }
}
fn original<'a>(error: &'a (dyn Error + 'static)) -> &'a Original {
    let mut cause = error;
    loop {
        if let Some(original) = cause.downcast_ref::<Original>() {
            return original;
        }
        cause = if let Some(io) = cause.downcast_ref::<std::io::Error>() {
            io.get_ref()
                .map(|inner| inner as &(dyn Error + 'static))
                .expect("original native payload")
        } else {
            cause.source().expect("original typed source chain")
        };
    }
}
struct Stream {
    inner: DuplexStream,
    response_left: usize,
    fault: Option<std::io::Error>,
    bank: Arc<Semaphore>,
    hold: Arc<Mutex<Option<OwnedSemaphorePermit>>>,
    observed_balance: Arc<AtomicUsize>,
}
impl AsyncRead for Stream {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        bytes: &mut ReadBuf<'_>,
    ) -> Poll<std::io::Result<()>> {
        if self.response_left == 0 {
            if let Some(fault) = self.fault.take() {
                *self.hold.lock().unwrap() = Some(
                    self.bank
                        .clone()
                        .try_acquire_many_owned(self.bank.available_permits() as u32)
                        .unwrap(),
                );
                self.bank.close();
                self.observed_balance
                    .store(self.bank.available_permits(), Ordering::SeqCst);
                return Poll::Ready(Err(fault));
            }
        }
        let before = bytes.filled().len();
        let result = Pin::new(&mut self.inner).poll_read(cx, bytes);
        if matches!(result, Poll::Ready(Ok(()))) {
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
        Pin::new(&mut self.inner).poll_write(cx, bytes)
    }
    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        Pin::new(&mut self.inner).poll_flush(cx)
    }
    fn poll_shutdown(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        Pin::new(&mut self.inner).poll_shutdown(cx)
    }
}

#[tokio::test]
async fn actual_prepared_policy_keeps_first_private_native_source_before_closed_bank_cleanup() {
    let modes = [
        #[cfg(feature = "tunnel-kernel")]
        TunMode::Kernel,
        #[cfg(feature = "tunnel-userspace")]
        TunMode::Userspace,
    ];
    for mode in modes {
        let baseline = tokio::runtime::Handle::current()
            .metrics()
            .num_alive_tasks();
        let reserved = tokio::net::TcpSocket::new_v4().unwrap();
        reserved.bind("127.0.0.1:0".parse().unwrap()).unwrap();
        let addr = reserved.local_addr().unwrap();
        #[cfg(target_os = "macos")]
        let reserved = {
            drop(reserved);
        };
        let refusal = tokio::net::TcpStream::connect(addr).await.unwrap_err();
        assert_eq!(refusal.kind(), std::io::ErrorKind::ConnectionRefused);
        let drops = Arc::new(AtomicUsize::new(0));
        let fault = std::io::Error::other(Original {
            native: refusal,
            drops: drops.clone(),
        });
        let native_pointer = &fault
            .get_ref()
            .unwrap()
            .downcast_ref::<Original>()
            .unwrap()
            .native as *const _ as usize;
        let hold = Arc::new(Mutex::new(None));
        let observed_balance = Arc::new(AtomicUsize::new(usize::MAX));
        let bank = Arc::new(Semaphore::new(TOTAL));
        let owner = Owner {
            bank: bank.clone(),
            mode,
            attached: Mutex::new(None),
        };
        let (stream, mut peer) = tokio::io::duplex(4096);
        let response = serde_json::to_vec(&serde_json::json!({
            "serverAddress": "fd00::1", "serverRSDPort": 58783,
            "clientParameters": { "address": "fd00::2", "mtu": 1280 },
        }))
        .unwrap();
        let response_length = response.len() + 10;
        let peer_task = tokio::spawn(async move {
            let mut header = [0; 10];
            peer.read_exact(&mut header).await.unwrap();
            assert_eq!(&header[..8], b"CDTunnel");
            let mut body = vec![0; u16::from_be_bytes([header[8], header[9]]) as usize];
            peer.read_exact(&mut body).await.unwrap();
            peer.write_all(b"CDTunnel").await.unwrap();
            peer.write_all(&(response.len() as u16).to_be_bytes())
                .await
                .unwrap();
            peer.write_all(&response).await.unwrap();
            peer
        });
        let connection = TunnelConnection::new(
            DeviceInfo {
                udid: "prepared-private-native-fixture".into(),
                device_id: 0,
                connection_type: "Network".into(),
                product_id: 0,
            },
            None,
            LockdownTransport::Tcp { host: "::1".into() },
            "scripted prepared source",
        )
        .with_initial_rsd_bank(Some(&bank));
        let result = prepare_tunnel(
            connection,
            Stream {
                inner: stream,
                response_left: response_length,
                fault: Some(fault),
                bank: bank.clone(),
                hold: hold.clone(),
                observed_balance: observed_balance.clone(),
            },
            mode,
            Some(owner.reserve_owned().unwrap()),
            Prepared {
                deadline: Instant::now() + OBSERVE,
                bank: &bank,
            },
        )
        .await;
        let error = result
            .err()
            .expect("private IO failure must reject preparation");
        assert_eq!(
            &original(&error).native as *const _ as usize,
            native_pointer
        );
        assert_eq!(
            original(&error).native.kind(),
            std::io::ErrorKind::ConnectionRefused
        );
        assert_eq!(drops.load(Ordering::SeqCst), 0);
        assert_eq!(observed_balance.load(Ordering::SeqCst), 0);
        assert!(bank.is_closed());
        let handle = owner.attached.lock().unwrap().as_ref().unwrap().clone();
        assert!(!handle.ready_for_publication());
        let cleanup = handle.shutdown(OBSERVE).await.unwrap_err();
        assert_eq!(
            &original(&cleanup).native as *const _ as usize,
            native_pointer
        );
        let status = handle.shutdown_status().unwrap_err();
        assert_eq!(
            &original(&status).native as *const _ as usize,
            native_pointer
        );
        let mut peer = peer_task.await.unwrap();
        let mut tail = Vec::new();
        tokio::time::timeout(OBSERVE, peer.read_to_end(&mut tail))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(peer.read(&mut [0; 1]).await.unwrap(), 0);
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
        drop((error, cleanup, status, handle, owner, peer, reserved));
        drop(hold.lock().unwrap().take());
        assert_eq!(bank.available_permits(), TOTAL);
        assert_eq!(drops.load(Ordering::SeqCst), 1);
    }
}
