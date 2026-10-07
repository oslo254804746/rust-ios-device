use std::collections::HashMap;

#[cfg(all(test, feature = "tunnel-kernel"))]
mod shutdown_tests;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

#[cfg(any(feature = "tunnel-kernel", feature = "tunnel-userspace"))]
use crate::tunnel::owner::{self, Ledger};
#[cfg(feature = "tunnel-kernel")]
use crate::tunnel::owner::{ChildRole, OwnedTask, ParentRole, SharedChildTaskSet};
#[cfg(any(feature = "tunnel-kernel", feature = "tunnel-userspace"))]
use crate::tunnel::TunnelError;

use tokio::sync::{watch, RwLock};

#[cfg(feature = "tunnel-userspace")]
use crate::tunnel::tun::userspace::UserspaceTunDevice;
use crate::tunnel::TunnelInfo;

/// TUN mode selection.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TunMode {
    /// Use the OS kernel TUN device (requires admin/root on most platforms).
    Kernel,
    /// Use smoltcp userspace network stack (no special privileges needed).
    #[default]
    Userspace,
}

/// A tunnel runtime and its local metadata.
///
/// Final owner Drop requests cancellation. Explicit shutdown joins tasks owned
/// by this library; shared Arc owners can stop it without releasing every clone.
pub struct TunnelHandle {
    pub udid: String,
    pub info: TunnelInfo,
    pub userspace_port: Option<u16>,
    _runtime: TunnelRuntime,
}

enum TunnelRuntime {
    Kernel {
        /// Dropping this sender cancels the tunnel (receivers get RecvError).
        _cancel_tx: watch::Sender<()>,
        stopped: AtomicBool,
        #[cfg(any(feature = "tunnel-kernel", feature = "tunnel-userspace"))]
        outcomes: Arc<Ledger>,
        #[cfg(feature = "tunnel-kernel")]
        tasks: Vec<OwnedTask>,
        #[cfg(feature = "tunnel-kernel")]
        reader: Arc<SharedChildTaskSet>,
    },
    #[cfg(feature = "tunnel-userspace")]
    Userspace { _runtime: UserspaceTunDevice },
}

impl TunnelHandle {
    /// Create the legacy control-channel handle and receiver. Any task the
    /// caller spawns with that receiver belongs to the caller, not this handle.
    /// Shutdown cannot collect those external task joins.
    pub fn new(
        udid: String,
        info: TunnelInfo,
        userspace_port: Option<u16>,
    ) -> (Self, watch::Receiver<()>) {
        let (tx, rx) = watch::channel(());
        #[cfg(any(feature = "tunnel-kernel", feature = "tunnel-userspace"))]
        let outcomes = Ledger::new();
        (
            Self {
                udid,
                info,
                userspace_port,
                _runtime: TunnelRuntime::Kernel {
                    _cancel_tx: tx,
                    stopped: AtomicBool::new(false),
                    #[cfg(any(feature = "tunnel-kernel", feature = "tunnel-userspace"))]
                    outcomes: Arc::clone(&outcomes),
                    #[cfg(feature = "tunnel-kernel")]
                    tasks: Vec::new(),
                    #[cfg(feature = "tunnel-kernel")]
                    reader: SharedChildTaskSet::new(outcomes, ChildRole::ForwardReader),
                },
            },
            rx,
        )
    }

    #[cfg(feature = "tunnel-kernel")]
    pub(crate) fn new_owned_kernel(
        udid: String,
        info: TunnelInfo,
        spawn: impl FnOnce(
            watch::Receiver<()>,
            Arc<Ledger>,
            Arc<SharedChildTaskSet>,
        ) -> tokio::task::JoinHandle<Result<(), TunnelError>>,
    ) -> Self {
        let (cancel, receiver) = watch::channel(());
        let outcomes = Ledger::new();
        let reader = SharedChildTaskSet::new(Arc::clone(&outcomes), ChildRole::ForwardReader);
        let task = OwnedTask::new(
            spawn(receiver, Arc::clone(&outcomes), Arc::clone(&reader)),
            ParentRole::ForwardOrReader,
            Arc::clone(&outcomes),
        );
        Self {
            udid,
            info,
            userspace_port: None,
            _runtime: TunnelRuntime::Kernel {
                _cancel_tx: cancel,
                stopped: AtomicBool::new(false),
                outcomes,
                tasks: vec![task],
                reader,
            },
        }
    }

    /// Synchronously mark this shared runtime stopped and request cancellation.
    /// Repeated calls are safe. This does not wait for native task completion.
    pub fn request_stop(&self) {
        match &self._runtime {
            TunnelRuntime::Kernel {
                _cancel_tx,
                stopped,
                ..
            } => {
                stopped.store(true, Ordering::Release);
                _cancel_tx.send_replace(());
            }
            #[cfg(feature = "tunnel-userspace")]
            TunnelRuntime::Userspace { _runtime } => _runtime.request_stop(),
        }
    }

    #[cfg(any(feature = "tunnel-kernel", feature = "tunnel-userspace"))]
    /// Stop and join owned parents and children under one cooperative budget.
    ///
    /// The absolute clock and stop request start at this method call, before
    /// polling its future. Join-mutex waits share the clock. Zero requests stop
    /// and returns native expiry; overflow returns InvalidInput without stopping.
    /// Timeout or caller-future Drop retains unfinished joins and acquired
    /// outcomes while this owner remains held. Call again with a new budget to
    /// continue. Completed joins are not repolled; failures remain inspectable
    /// after resource recovery. A single blocking poll/Drop is not preempted.
    pub fn shutdown(
        &self,
        budget: std::time::Duration,
    ) -> impl std::future::Future<Output = Result<(), TunnelError>> + Send + '_ {
        let deadline = owner::deadline(budget);
        if deadline.is_ok() {
            self.request_stop();
        }
        async move {
            match &self._runtime {
                #[cfg(feature = "tunnel-kernel")]
                TunnelRuntime::Kernel {
                    tasks,
                    reader,
                    outcomes,
                    ..
                } => {
                    owner::join_until(deadline, outcomes, async {
                        for task in tasks {
                            task.join().await;
                        }
                        reader.drain().await;
                    })
                    .await
                }
                #[cfg(not(feature = "tunnel-kernel"))]
                TunnelRuntime::Kernel { outcomes, .. } => {
                    // Legacy sender-only constructor owns no native task.
                    owner::join_until(deadline, outcomes, std::future::ready(())).await
                }
                #[cfg(feature = "tunnel-userspace")]
                TunnelRuntime::Userspace { _runtime } => _runtime.shutdown_until(deadline).await,
            }
        }
    }

    #[cfg(feature = "tunnel-userspace")]
    pub fn new_userspace(udid: String, info: TunnelInfo, runtime: UserspaceTunDevice) -> Self {
        Self {
            udid,
            info,
            userspace_port: Some(runtime.local_port),
            _runtime: TunnelRuntime::Userspace { _runtime: runtime },
        }
    }

    /// Local admission state; false does not certify native joins or a peer ACK.
    pub fn is_alive(&self) -> bool {
        match &self._runtime {
            TunnelRuntime::Kernel {
                _cancel_tx,
                stopped,
                ..
            } => !stopped.load(Ordering::Acquire) && _cancel_tx.receiver_count() > 0,
            #[cfg(feature = "tunnel-userspace")]
            TunnelRuntime::Userspace { _runtime } => _runtime.is_alive(),
        }
    }
}

/// Manager for active tunnel instances.
#[derive(Clone)]
pub struct TunnelManager {
    tunnels: Arc<RwLock<HashMap<String, Arc<TunnelHandle>>>>,
    pub mode: TunMode,
}

impl TunnelManager {
    pub fn new(mode: TunMode) -> Self {
        Self {
            tunnels: Arc::new(RwLock::new(HashMap::new())),
            mode,
        }
    }

    /// Replacing a different Arc requests stop on the displaced runtime.
    /// Registering the same Arc keeps it live. Displaced owners are not retained
    /// or joined by the manager; retain them to perform explicit shutdown.
    pub async fn register(&self, handle: Arc<TunnelHandle>) {
        let old = self
            .tunnels
            .write()
            .await
            .insert(handle.udid.clone(), handle.clone());
        if let Some(old) = old {
            if !Arc::ptr_eq(&old, &handle) {
                old.request_stop();
            }
        }
    }

    pub async fn list(&self) -> Vec<Arc<TunnelHandle>> {
        self.tunnels.read().await.values().cloned().collect()
    }

    pub async fn find(&self, udid: &str) -> Option<Arc<TunnelHandle>> {
        self.tunnels.read().await.get(udid).cloned()
    }

    /// Signal the shared runtime and remove the manager's handle. Explicit join
    /// responsibility requires retaining an Arc and calling shutdown.
    /// The bool reports presence, not native completion.
    pub async fn stop(&self, udid: &str) -> bool {
        self.retire(udid).await.is_some()
    }

    /// Transfer a stopped owner to the caller for explicit, retryable shutdown.
    /// No manager/global table lock is held while the caller joins tasks.
    /// Retain the returned Arc across timeout/cancellation to continue shutdown.
    pub async fn retire(&self, udid: &str) -> Option<Arc<TunnelHandle>> {
        let handle = self.tunnels.write().await.remove(udid)?;
        handle.request_stop();
        Some(handle)
    }
}

impl Default for TunnelManager {
    fn default() -> Self {
        Self::new(TunMode::Userspace)
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::*;

    fn tunnel_info() -> TunnelInfo {
        TunnelInfo {
            server_address: "fd59:2381:6956::1".into(),
            server_rsd_port: 58783,
            client_address: "fd59:2381:6956::2".into(),
            client_mtu: 1280,
        }
    }

    #[test]
    fn tunnel_handle_kernel_liveness_tracks_cancel_receiver() {
        let (handle, cancel_rx) = TunnelHandle::new("test-udid".into(), tunnel_info(), None);

        assert!(handle.is_alive());

        drop(cancel_rx);
        assert!(!handle.is_alive());
    }

    #[tokio::test]
    async fn tunnel_manager_register_find_list_and_stop() {
        let manager = TunnelManager::new(TunMode::Kernel);
        let (handle, cancel_rx) = TunnelHandle::new("test-udid".into(), tunnel_info(), None);
        let handle = Arc::new(handle);

        manager.register(handle.clone()).await;

        assert!(Arc::ptr_eq(
            &manager.find("test-udid").await.unwrap(),
            &handle
        ));
        assert!(manager.find("missing").await.is_none());
        assert_eq!(manager.list().await.len(), 1);
        assert_eq!(manager.list().await[0].udid, "test-udid");

        assert!(manager.stop("test-udid").await);
        assert!(!manager.stop("test-udid").await);
        assert!(manager.find("test-udid").await.is_none());
        assert!(manager.list().await.is_empty());

        drop(cancel_rx);
    }

    #[test]
    fn tunnel_manager_default_uses_userspace_mode() {
        let manager = TunnelManager::default();
        assert_eq!(manager.mode, TunMode::Userspace);
    }
}
