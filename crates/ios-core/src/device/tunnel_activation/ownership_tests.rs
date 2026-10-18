//! Actual activation and userspace workers; only the CDTunnel peer is local.
use super::*;
use crate::{TunnelOwner, TunnelReservation};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Mutex as StdMutex;
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt, DuplexStream};
use tokio::sync::Notify;

const OBSERVE: Duration = Duration::from_secs(2);

struct Owner {
    handles: StdMutex<Vec<Arc<TunnelHandle>>>,
    attached: Notify,
    completed: AtomicUsize,
    retired: AtomicUsize,
    reject: bool,
    byte_bank: Option<Arc<tokio::sync::Semaphore>>,
}
impl Owner {
    fn new(reject: bool) -> Self {
        Self {
            handles: StdMutex::new(Vec::new()),
            attached: Notify::new(),
            completed: AtomicUsize::new(0),
            retired: AtomicUsize::new(0),
            reject,
            byte_bank: None,
        }
    }
    fn handle(&self) -> Option<Arc<TunnelHandle>> {
        self.handles.lock().unwrap().last().cloned()
    }
}
struct Reservation<'a> {
    owner: &'a Owner,
    handle: Option<Arc<TunnelHandle>>,
    completed: bool,
}
impl TunnelOwner for Owner {
    fn reserve(&self) -> Result<Box<dyn TunnelReservation + '_>, CoreError> {
        Ok(Box::new(Reservation {
            owner: self,
            handle: None,
            completed: false,
        }))
    }
}
impl TunnelReservation for Reservation<'_> {
    fn byte_bank(&self) -> Option<Arc<tokio::sync::Semaphore>> {
        assert!(
            self.handle.is_none(),
            "bank requested after attach consumed reservation"
        );
        self.owner.byte_bank.clone()
    }

    fn attach(&mut self, handle: Arc<TunnelHandle>) -> Result<(), CoreError> {
        self.owner.handles.lock().unwrap().push(Arc::clone(&handle));
        self.handle = Some(handle);
        self.owner.attached.notify_one();
        if self.owner.reject {
            Err(CoreError::Io(std::io::Error::new(
                std::io::ErrorKind::PermissionDenied,
                "synthetic-private-owner-rejection",
            )))
        } else {
            Ok(())
        }
    }
    fn complete(&mut self) {
        self.completed = true;
        self.owner.completed.fetch_add(1, Ordering::SeqCst);
    }
}
impl Drop for Reservation<'_> {
    fn drop(&mut self) {
        if !self.completed {
            if let Some(handle) = &self.handle {
                handle.request_stop();
                self.owner.retired.fetch_add(1, Ordering::SeqCst);
            }
        }
    }
}

fn connection() -> TunnelConnection {
    TunnelConnection::new(
        DeviceInfo {
            udid: "native-activation-owner-fixture".into(),
            device_id: 0,
            connection_type: "Network".into(),
            product_id: 0,
        },
        None,
        LockdownTransport::Tcp {
            host: String::new(),
        },
        "native owner fixture",
    )
}

fn peer(server: &str) -> (DuplexStream, tokio::task::JoinHandle<DuplexStream>) {
    let (stream, mut peer) = tokio::io::duplex(4096);
    let response = serde_json::to_vec(&serde_json::json!({
        "serverAddress": server, "serverRSDPort": 58783,
        "clientParameters": { "address": "fd00::2", "mtu": 1280 },
    }))
    .unwrap();
    let task = tokio::spawn(async move {
        let mut header = [0; 10];
        peer.read_exact(&mut header).await.unwrap();
        let length = u16::from_be_bytes([header[8], header[9]]) as usize;
        let mut body = vec![0; length];
        peer.read_exact(&mut body).await.unwrap();
        let mut packet = b"CDTunnel".to_vec();
        packet.extend_from_slice(&(response.len() as u16).to_be_bytes());
        packet.extend_from_slice(&response);
        peer.write_all(&packet).await.unwrap();
        peer
    });
    (stream, task)
}

async fn recover(mut peer: DuplexStream, baseline: usize) -> bool {
    tokio::time::timeout(OBSERVE, async {
        let mut bytes = Vec::new();
        peer.read_to_end(&mut bytes).await?;
        while tokio::runtime::Handle::current()
            .metrics()
            .num_alive_tasks()
            != baseline
        {
            tokio::task::yield_now().await;
        }
        Ok::<(), std::io::Error>(())
    })
    .await
    .is_ok_and(|result| result.is_ok())
}

#[tokio::test]
async fn native_activation_caller_drop_before_rsd_return_keeps_original_owner_for_join() {
    let baseline = tokio::runtime::Handle::current()
        .metrics()
        .num_alive_tasks();
    let owner = Owner::new(false);
    let (stream, peer_task) = peer("fd00::1");
    let mut activation = Box::pin(activate_tunnel(
        connection(),
        stream,
        TunMode::Userspace,
        Some(owner.reserve().unwrap()),
    ));
    let attached = tokio::time::timeout(OBSERVE, async {
        tokio::select! {
            biased;
            _ = owner.attached.notified() => true,
            _ = &mut activation => false,
        }
    })
    .await
    .unwrap_or(false);
    let pending = futures_util::poll!(&mut activation).is_pending();
    drop(activation);
    let native = owner.handle();
    let stopped = native.as_ref().is_some_and(|native| !native.is_alive());
    let result = match &native {
        Some(native) => native.shutdown(OBSERVE).await,
        None => Ok(()),
    };
    let peer = peer_task.await.unwrap();
    let recovered = recover(peer, baseline).await;
    result.unwrap();
    assert!(attached && pending && stopped && recovered);
    assert_eq!(owner.completed.load(Ordering::SeqCst), 0);
    assert_eq!(owner.retired.load(Ordering::SeqCst), 1);
    assert!(native.unwrap().shutdown_status().is_ok());
}

#[tokio::test]
async fn native_activation_success_returns_the_exact_attached_arc_and_transfers_cleanup() {
    let baseline = tokio::runtime::Handle::current()
        .metrics()
        .num_alive_tasks();
    let owner = Owner::new(false);
    // Invalid optional RSD address makes the probe complete without a target IO.
    let (stream, peer_task) = peer("invalid-rsd-address");
    let device = activate_tunnel(
        connection(),
        stream,
        TunMode::Userspace,
        Some(owner.reserve().unwrap()),
    )
    .await;
    let retained = owner.handle();
    let same = device
        .as_ref()
        .ok()
        .and_then(ConnectedDevice::tunnel_handle)
        .zip(retained.as_ref())
        .is_some_and(|(actual, retained)| Arc::ptr_eq(actual, retained));
    let live = retained.as_ref().is_some_and(|native| native.is_alive());
    let result = match &retained {
        Some(native) => native.shutdown(OBSERVE).await,
        None => Ok(()),
    };
    let peer = peer_task.await.unwrap();
    let recovered = recover(peer, baseline).await;
    result.unwrap();
    assert!(device.is_ok() && same && live && recovered);
    assert_eq!(owner.completed.load(Ordering::SeqCst), 1);
    assert_eq!(owner.retired.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn native_activation_rejected_attachment_stops_workers_and_retains_original_error() {
    let baseline = tokio::runtime::Handle::current()
        .metrics()
        .num_alive_tasks();
    let owner = Owner::new(true);
    let (stream, peer_task) = peer("fd00::1");
    let result = activate_tunnel(
        connection(),
        stream,
        TunMode::Userspace,
        Some(owner.reserve().unwrap()),
    )
    .await;
    let native = owner.handle();
    let stopped = native.as_ref().is_some_and(|native| !native.is_alive());
    let cleanup = match &native {
        Some(native) => native.shutdown(OBSERVE).await,
        None => Ok(()),
    };
    let peer = peer_task.await.unwrap();
    let recovered = recover(peer, baseline).await;
    cleanup.unwrap();
    assert!(stopped && recovered);
    let Err(CoreError::Io(error)) = result else {
        panic!("attachment error lost")
    };
    assert_eq!(error.kind(), std::io::ErrorKind::PermissionDenied);
    assert_eq!(error.to_string(), "synthetic-private-owner-rejection");
    assert_eq!(owner.completed.load(Ordering::SeqCst), 0);
    assert_eq!(owner.retired.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn native_activation_passes_the_original_reservation_bank_before_any_userspace_worker() {
    let baseline = tokio::runtime::Handle::current()
        .metrics()
        .num_alive_tasks();
    let bank = Arc::new(tokio::sync::Semaphore::new(
        crate::tunnel::IPV6_HEADER_LEN - 1,
    ));
    let mut owner = Owner::new(false);
    owner.byte_bank = Some(Arc::clone(&bank));
    let (stream, peer_task) = peer("invalid-rsd-address");
    let result = activate_tunnel(
        connection(),
        stream,
        TunMode::Userspace,
        Some(owner.reserve().unwrap()),
    )
    .await;
    let mut peer = peer_task.await.unwrap();
    let mut byte = [0];
    let eof = tokio::time::timeout(OBSERVE, peer.read(&mut byte)).await;
    assert!(
        matches!(eof, Ok(Ok(0))),
        "failed bank handoff retained native transport: {eof:?}"
    );
    assert_eq!(
        tokio::runtime::Handle::current()
            .metrics()
            .num_alive_tasks(),
        baseline
    );
    assert!(
        owner.handle().is_none(),
        "header refusal attached a worker runtime"
    );
    assert_eq!(owner.completed.load(Ordering::SeqCst), 0);
    assert_eq!(owner.retired.load(Ordering::SeqCst), 0);
    assert_eq!(bank.available_permits(), crate::tunnel::IPV6_HEADER_LEN - 1);
    let error = result
        .err()
        .expect("activation ignored the reservation's byte bank");
    assert_eq!(
        crate::classify_error(&error),
        crate::ErrorClass::ResourceLimit
    );
}

#[tokio::test]
async fn owned_initial_native_activation_keeps_original_bank_attach_owner_and_actual_joins() {
    let baseline = tokio::runtime::Handle::current()
        .metrics()
        .num_alive_tasks();
    let quota = 32 * 1024 * 1024;
    let bank = Arc::new(tokio::sync::Semaphore::new(quota));
    let mut owner = Owner::new(false);
    owner.byte_bank = Some(bank.clone());
    let (stream, peer_task) = peer("fixture-invalid-rsd-endpoint");
    let result = activate_tunnel(
        connection().with_initial_rsd_bank(Some(&bank)),
        stream,
        TunMode::Userspace,
        super::super::tunnel_owner::InitialOwner::new(Some(&owner), Some(&bank))
            .reserve()
            .unwrap(),
    )
    .await;
    let retained = owner.handle();
    let same = result
        .as_ref()
        .ok()
        .and_then(ConnectedDevice::tunnel_handle)
        .zip(retained.as_ref())
        .is_some_and(|(actual, original)| Arc::ptr_eq(actual, original));
    let device = result.map(crate::OwnedConnectedDevice::new);
    let failed = device.as_ref().is_ok_and(|device| {
        device.rsd().is_none()
            && device.initial_rsd_probe().state() == crate::InitialRsdProbeState::Failed
    });
    let cleanup = match &retained {
        Some(native) => native.shutdown(OBSERVE).await,
        None => Ok(()),
    };
    let peer = peer_task.await.unwrap();
    let recovered = recover(peer, baseline).await;
    cleanup.unwrap();
    assert!(same && failed && recovered);
    assert!(retained.as_ref().unwrap().shutdown_status().is_ok());
    assert_eq!(owner.completed.load(Ordering::SeqCst), 1);
    assert_eq!(owner.retired.load(Ordering::SeqCst), 0);
    drop((device, retained, owner));
    assert_eq!(bank.available_permits(), quota);
}
