use std::collections::HashMap;
use std::sync::Arc;

use tokio::sync::{watch, RwLock};

use crate::tunnel::handshake::TunnelInfo;
use crate::tunnel::tun::userspace::UserspaceTunDevice;

/// TUN mode selection.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TunMode {
    /// Use the OS kernel TUN device (requires admin/root on most platforms).
    Kernel,
    /// Use smoltcp userspace network stack (no special privileges needed).
    #[default]
    Userspace,
}

/// A live tunnel handle. Dropping this handle signals the tunnel task to stop.
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
    },
    Userspace {
        _runtime: UserspaceTunDevice,
    },
}

impl TunnelHandle {
    pub fn new(
        udid: String,
        info: TunnelInfo,
        userspace_port: Option<u16>,
    ) -> (Self, watch::Receiver<()>) {
        let (tx, rx) = watch::channel(());
        (
            Self {
                udid,
                info,
                userspace_port,
                _runtime: TunnelRuntime::Kernel { _cancel_tx: tx },
            },
            rx,
        )
    }

    pub fn new_userspace(udid: String, info: TunnelInfo, runtime: UserspaceTunDevice) -> Self {
        Self {
            udid,
            info,
            userspace_port: Some(runtime.local_port),
            _runtime: TunnelRuntime::Userspace { _runtime: runtime },
        }
    }

    pub fn is_alive(&self) -> bool {
        match &self._runtime {
            TunnelRuntime::Kernel { _cancel_tx } => _cancel_tx.receiver_count() > 0,
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

    pub async fn register(&self, handle: Arc<TunnelHandle>) {
        self.tunnels
            .write()
            .await
            .insert(handle.udid.clone(), handle);
    }

    pub async fn list(&self) -> Vec<Arc<TunnelHandle>> {
        self.tunnels.read().await.values().cloned().collect()
    }

    pub async fn find(&self, udid: &str) -> Option<Arc<TunnelHandle>> {
        self.tunnels.read().await.get(udid).cloned()
    }

    /// Remove and drop the tunnel handle (which cancels the tunnel task). Returns true if found.
    pub async fn stop(&self, udid: &str) -> bool {
        self.tunnels.write().await.remove(udid).is_some()
    }
}

impl Default for TunnelManager {
    fn default() -> Self {
        Self::new(TunMode::Userspace)
    }
}
