//! Current desired state on the existing bounded device coordinator.
//! No retry history, failure permit or independent device task lives here.

use std::sync::{Arc, Mutex, MutexGuard};
use tokio::sync::watch;

use super::source::Route;
use crate::access::DeviceAuthority;

#[cfg(test)]
type ReplaceCallback = Box<dyn FnOnce() + Send>;

#[derive(Clone, PartialEq, Eq)]
pub(super) enum RouteIntent {
    Usbmux,
    Usb(Option<String>),
    Wifi(Option<String>),
    // Keep the actual established transport family, rediscover its host using
    // the original filter rather than replaying a stale discovered address.
    Mobdev2(Option<String>),
}

impl RouteIntent {
    pub(super) fn established(route: &Route, host_filter: Option<&str>) -> Self {
        match route {
            Route::Usbmux => Self::Usbmux,
            Route::Usb(host) => Self::Usb(host.clone()),
            Route::Wifi(host) => Self::Wifi(host.clone()),
            Route::Mobdev2(_) => Self::Mobdev2(host_filter.map(str::to_owned)),
        }
    }
}

#[derive(Clone)]
pub(super) enum Desired {
    Automatic,
    Manual {
        route: RouteIntent,
        authority: DeviceAuthority,
    },
    Stopped,
}

#[derive(Clone)]
pub(super) struct Snapshot {
    pub(super) epoch: Arc<()>,
    pub(super) version: Arc<()>,
    pub(super) desired: Desired,
}

pub(super) struct Cell {
    state: Mutex<Snapshot>,
    changes: watch::Sender<Arc<()>>,
    // Mirrors the existing FailedTunnelAttempt.version; never an independent
    // failure history/permit. Updated by its original commit and rollback.
    failure: Mutex<Option<(Arc<()>, Arc<()>)>>,
    pending_success: Mutex<Option<(Arc<()>, Arc<()>)>>,
    #[cfg(test)]
    before_checked_replace: Mutex<Option<ReplaceCallback>>,
    #[cfg(test)]
    after_checked_candidate: Mutex<Option<ReplaceCallback>>,
}

impl Cell {
    pub(super) fn new(epoch: Arc<()>) -> Self {
        let version = Arc::new(());
        Self {
            state: Mutex::new(Snapshot {
                epoch,
                version: version.clone(),
                desired: Desired::Automatic,
            }),
            changes: watch::channel(version).0,
            failure: Mutex::new(None),
            pending_success: Mutex::new(None),
            #[cfg(test)]
            before_checked_replace: Mutex::new(None),
            #[cfg(test)]
            after_checked_candidate: Mutex::new(None),
        }
    }

    pub(super) fn lock(&self) -> MutexGuard<'_, Snapshot> {
        self.state
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
    }

    pub(super) fn snapshot(&self) -> Snapshot {
        self.lock().clone()
    }

    pub(super) fn is_epoch(&self, epoch: &Arc<()>) -> bool {
        Arc::ptr_eq(&self.lock().epoch, epoch)
    }

    pub(super) fn reset(&self, epoch: Arc<()>) {
        self.lock().epoch = epoch;
        self.replace(Desired::Automatic);
        self.set_failure(None);
    }

    pub(super) fn persistent(&self) -> bool {
        !matches!(self.lock().desired, Desired::Automatic)
    }

    pub(super) fn matches(&self, version: &Arc<()>) -> bool {
        Arc::ptr_eq(&self.lock().version, version)
    }

    pub(super) fn replace(&self, desired: Desired) -> Arc<()> {
        let mut state = self.lock();
        state.version = Arc::new(());
        state.desired = desired;
        let version = state.version.clone();
        drop(state);
        *self
            .pending_success
            .lock()
            .unwrap_or_else(|p| p.into_inner()) = None;
        self.changes.send_replace(version.clone());
        version
    }

    // The caller holds the authority revision and lifecycle epoch fences.
    // Check both clocks only after every mutable destination lock is acquired,
    // then certify the candidate before allowing readers or watch subscribers
    // to observe it. A failed candidate restores the exact previous identities.
    pub(super) fn checked_replace(
        &self,
        desired: Desired,
        failure: Option<Arc<()>>,
        check: impl Fn() -> anyhow::Result<()>,
    ) -> anyhow::Result<Arc<()>> {
        #[cfg(test)]
        Self::run_replace_callback(&self.before_checked_replace);
        let mut state = self.lock();
        let mut pending = self
            .pending_success
            .lock()
            .unwrap_or_else(|p| p.into_inner());
        check()?;
        let previous = state.clone();
        let previous_pending = pending.clone();
        let version = Arc::new(());
        state.version = version.clone();
        state.desired = desired;
        *pending = failure.map(|failure| (version.clone(), failure));
        #[cfg(test)]
        Self::run_replace_callback(&self.after_checked_candidate);
        if let Err(error) = check() {
            *state = previous;
            *pending = previous_pending;
            return Err(error);
        }
        self.changes.send_replace(version.clone());
        Ok(version)
    }

    #[cfg(test)]
    fn run_replace_callback(slot: &Mutex<Option<ReplaceCallback>>) {
        let callback = slot.lock().unwrap().take();
        if let Some(callback) = callback {
            callback();
        }
    }

    #[cfg(test)]
    pub(super) fn on_next_checked_replace(
        &self,
        after_candidate: bool,
        callback: impl FnOnce() + Send + 'static,
    ) {
        let slot = if after_candidate {
            &self.after_checked_candidate
        } else {
            &self.before_checked_replace
        };
        let mut pending = slot.lock().unwrap();
        assert!(pending.is_none(), "manual intent hook already installed");
        *pending = Some(Box::new(callback));
    }

    #[cfg(test)]
    pub(super) fn hold_replace_lock(&self, pending: bool, action: impl FnOnce()) {
        if pending {
            let _held = self.pending_success.lock().unwrap();
            action();
        } else {
            let _held = self.lock();
            action();
        }
    }

    #[cfg(test)]
    pub(super) fn subscribe(&self) -> watch::Receiver<Arc<()>> {
        self.changes.subscribe()
    }

    pub(super) fn failure(&self) -> Option<(Arc<()>, Arc<()>)> {
        self.failure
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .clone()
    }

    pub(super) fn set_failure(&self, failure: Option<(Arc<()>, Arc<()>)>) {
        *self.failure.lock().unwrap_or_else(|p| p.into_inner()) = failure;
    }

    pub(super) fn pending_success(&self) -> Option<(Arc<()>, Arc<()>)> {
        self.pending_success
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .clone()
    }

    pub(super) fn clear_success(&self, desired: &Arc<()>, failure: &Arc<()>) {
        let mut pending = self
            .pending_success
            .lock()
            .unwrap_or_else(|p| p.into_inner());
        if pending.as_ref().is_some_and(|(current, row)| {
            Arc::ptr_eq(current, desired) && Arc::ptr_eq(row, failure)
        }) {
            *pending = None;
        }
    }

    pub(super) async fn changed(&self, version: &Arc<()>) {
        let mut changes = self.changes.subscribe();
        // Subscribe before checking the immutable version to close the race.
        if !self.matches(version) {
            return;
        }
        let _ = changes.changed().await;
    }
}

pub(super) struct ManualRequest {
    pub(super) authority: DeviceAuthority,
    pub(super) host_filter: Option<String>,
}
