//! Private backend control never cancels the persistent transport workers.
use std::sync::{Arc, Mutex};

use tokio::sync::{Notify, Semaphore};

use super::packet_link::PacketPort;
use super::runtime_storage::RuntimeRoot;
use super::TunnelError;

enum PortState {
    Running,
    Returned(PacketPort),
    Leased,
    Ended,
}

struct State {
    listener_closed: bool,
    quiescing: bool,
    closed: bool,
    port: PortState,
}

struct Control {
    state: Mutex<State>,
    changed: Notify,
}

#[derive(Clone)]
pub(super) struct BackendControl(RuntimeRoot<Control>);

impl BackendControl {
    pub(super) fn prepare(bank: Option<&Arc<Semaphore>>) -> Result<Self, TunnelError> {
        Ok(Self(RuntimeRoot::new(
            Control {
                state: Mutex::new(State {
                    listener_closed: false,
                    quiescing: false,
                    closed: false,
                    port: PortState::Running,
                }),
                changed: Notify::new(),
            },
            bank,
        )?))
    }

    pub(super) fn close_listener(&self) {
        self.0
            .state
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .listener_closed = true;
        self.0.changed.notify_waiters();
    }

    pub(super) async fn listener_closed(&self) {
        loop {
            let changed = self.0.changed.notified();
            tokio::pin!(changed);
            changed.as_mut().enable();
            if self
                .0
                .state
                .lock()
                .unwrap_or_else(|p| p.into_inner())
                .listener_closed
            {
                return;
            }
            changed.await;
        }
    }

    pub(super) fn quiesce(&self) {
        self.0
            .state
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .quiescing = true;
        self.0.changed.notify_waiters();
    }

    pub(super) fn is_quiescing(&self) -> bool {
        self.0
            .state
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .quiescing
    }

    pub(super) async fn quiescing(&self) {
        loop {
            let changed = self.0.changed.notified();
            tokio::pin!(changed);
            changed.as_mut().enable();
            if self.is_quiescing() {
                return;
            }
            changed.await;
        }
    }

    /// Called by the original smoltcp worker before that task returns Ready.
    /// The private socket/device scope has already ended before this transfer.
    pub(super) fn returned(&self, port: Option<PacketPort>) {
        {
            let mut state = self.0.state.lock().unwrap_or_else(|p| p.into_inner());
            assert!(matches!(state.port, PortState::Running));
            state.port = match port {
                Some(port) => PortState::Returned(port),
                None => PortState::Ended,
            };
        }
        self.0.changed.notify_waiters();
    }

    /// The caller must first collect the actual private producer joins.
    /// This slot is a durable port owner, not a substitute for those joins.
    pub(super) fn lease(&self) -> Result<PortLease, TunnelError> {
        let port = {
            let mut state = self.0.state.lock().unwrap_or_else(|p| p.into_inner());
            if !matches!(state.port, PortState::Returned(_)) {
                return Err(TunnelError::Protocol(
                    "private backend did not return its original packet port".into(),
                ));
            }
            let PortState::Returned(port) = std::mem::replace(&mut state.port, PortState::Leased)
            else {
                unreachable!("checked original port state")
            };
            port
        };
        Ok(PortLease {
            control: self.clone(),
            port: Some(port),
        })
    }

    /// Terminal joins have ended. Return detached port fields so their queue
    /// backing and buffer fees end outside this control mutex.
    pub(super) fn close_port(&self) -> Option<PacketPort> {
        let mut state = self.0.state.lock().unwrap_or_else(|p| p.into_inner());
        state.closed = true;
        if matches!(state.port, PortState::Leased) {
            return None;
        }
        match std::mem::replace(&mut state.port, PortState::Ended) {
            PortState::Returned(port) => Some(port),
            _ => None,
        }
    }

    #[cfg(test)]
    pub(super) fn body_bytes() -> usize {
        RuntimeRoot::<Control>::body_bytes()
    }
}

/// Cancelling a handoff waiter parks the same unique receiver synchronously.
/// No task handles or raw stream capability are held by this small control root.
pub(crate) struct PortLease {
    control: BackendControl,
    port: Option<PacketPort>,
}

impl PortLease {
    pub(super) fn port(&mut self) -> &mut PacketPort {
        self.port.as_mut().expect("original leased packet port")
    }
}

impl Drop for PortLease {
    fn drop(&mut self) {
        let Some(port) = self.port.take() else { return };
        let detached = {
            let mut state = self
                .control
                .0
                .state
                .lock()
                .unwrap_or_else(|p| p.into_inner());
            assert!(matches!(state.port, PortState::Leased));
            if state.closed {
                state.port = PortState::Ended;
                Some(port)
            } else {
                state.port = PortState::Returned(port);
                None
            }
        };
        drop(detached);
        if !std::thread::panicking() {
            self.control.0.changed.notify_waiters();
        }
    }
}

#[cfg(test)]
mod tests;
