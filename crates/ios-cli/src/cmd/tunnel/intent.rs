//! Current desired state on the existing bounded device coordinator.
//! No retry history, failure permit or independent device task lives here.

use std::sync::{Arc, Mutex, MutexGuard};
use tokio::sync::watch;
use tokio::sync::{OwnedSemaphorePermit, Semaphore};

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

pub(super) struct RetainedRoute {
    pub(super) route: RouteIntent,
    _storage: OwnedSemaphorePermit,
    _text_storage: Option<OwnedSemaphorePermit>,
}

impl RetainedRoute {
    pub(super) fn text_bytes(&self) -> usize {
        match &self.route {
            RouteIntent::Usbmux => 0,
            RouteIntent::Usb(host) | RouteIntent::Wifi(host) | RouteIntent::Mobdev2(host) => {
                host.as_ref().map_or(0, String::capacity)
            }
        }
    }

    pub(super) fn established(
        route: &Route,
        host_filter: Option<&str>,
        bank: &Arc<Semaphore>,
    ) -> anyhow::Result<Arc<Self>> {
        Self::copy_established(route, host_filter, bank)
            .map_err(|error| error.context(RouteAdmission))
    }

    fn copy_established(
        route: &Route,
        host_filter: Option<&str>,
        bank: &Arc<Semaphore>,
    ) -> anyhow::Result<Arc<Self>> {
        let selected = match route {
            Route::Usbmux => None,
            Route::Usb(host) | Route::Wifi(host) => host.as_deref(),
            Route::Mobdev2(_) => host_filter,
        };
        super::admission::selectors("", selected)?;
        let storage = super::admission::known_metadata_charge(bank, std::mem::size_of::<Self>())?;
        let mut text_storage = None;
        let host = match selected {
            Some(host) => {
                let allocation = super::metadata_vec::exact_backing(bank, host.len())?;
                text_storage = Some(allocation.storage);
                let mut bytes = allocation.values;
                bytes.extend_from_slice(host.as_bytes());
                Some(String::from_utf8(bytes).expect("copied route str remains UTF-8"))
            }
            None => None,
        };
        let route = match route {
            Route::Usbmux => RouteIntent::Usbmux,
            Route::Usb(_) => RouteIntent::Usb(host),
            Route::Wifi(_) => RouteIntent::Wifi(host),
            Route::Mobdev2(_) => RouteIntent::Mobdev2(host),
        };
        Ok(Arc::new(Self {
            route,
            _storage: storage,
            _text_storage: text_storage,
        }))
    }

    // Existing moved-input fixtures preserve the original String allocation;
    // production uses established() before creating any route destination.
    #[cfg(test)]
    fn new(route: RouteIntent, bank: &Arc<Semaphore>) -> anyhow::Result<Arc<Self>> {
        let host = match &route {
            RouteIntent::Usbmux => None,
            RouteIntent::Usb(host) | RouteIntent::Wifi(host) | RouteIntent::Mobdev2(host) => {
                host.as_ref()
            }
        };
        super::admission::selectors("", host.map(String::as_str))?;
        let bytes = std::mem::size_of::<Self>()
            .checked_add(host.map_or(0, String::capacity))
            .ok_or_else(|| crate::output::resource_decision("tunnel manual route size overflow"))?;
        let storage = super::admission::known_metadata_charge(bank, bytes)?;
        Ok(Arc::new(Self {
            route,
            _storage: storage,
            _text_storage: None,
        }))
    }

    #[cfg(test)]
    fn bytes(route: &RouteIntent) -> usize {
        std::mem::size_of::<Self>()
            + match route {
                RouteIntent::Usbmux => 0,
                RouteIntent::Usb(host) | RouteIntent::Wifi(host) | RouteIntent::Mobdev2(host) => {
                    host.as_ref().map_or(0, String::capacity)
                }
            }
    }
}

#[derive(Debug)]
pub(super) struct RouteAdmission;
impl std::fmt::Display for RouteAdmission {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("tunnel route destination admission failed")
    }
}

#[derive(Clone)]
pub(super) enum Desired {
    Automatic,
    Manual {
        route: Arc<RetainedRoute>,
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
    metadata_bytes: Arc<Semaphore>,
    #[cfg(test)]
    before_checked_replace: Mutex<Option<ReplaceCallback>>,
    #[cfg(test)]
    after_checked_candidate: Mutex<Option<ReplaceCallback>>,
}

impl Cell {
    pub(super) fn new(epoch: Arc<()>, metadata_bytes: Arc<Semaphore>) -> Self {
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
            metadata_bytes,
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
        // Explicit global clear resets desired states, unlike durable per-device
        // stop across ordinary scans. Do this atomically with the new version.
        let version = {
            let mut state = self.lock();
            state.epoch = epoch;
            state.desired = Desired::Automatic;
            state.version = Arc::new(());
            *self
                .pending_success
                .lock()
                .unwrap_or_else(|p| p.into_inner()) = None;
            state.version.clone()
        };
        self.changes.send_replace(version);
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
        desired: impl FnOnce() -> anyhow::Result<Desired>,
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
        let desired = desired()?;
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

    pub(super) fn manual_shared(
        &self,
        route: Arc<RetainedRoute>,
        authority: DeviceAuthority,
    ) -> anyhow::Result<Desired> {
        anyhow::ensure!(
            Arc::ptr_eq(route._storage.semaphore(), &self.metadata_bytes),
            crate::output::resource_decision("tunnel manual route bank mismatch")
        );
        Ok(Desired::Manual { route, authority })
    }

    #[cfg(test)]
    pub(super) fn manual(
        &self,
        route: RouteIntent,
        authority: DeviceAuthority,
    ) -> anyhow::Result<Desired> {
        Ok(Desired::Manual {
            route: RetainedRoute::new(route, &self.metadata_bytes)?,
            authority,
        })
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

    /// The state and exact pending tuple use checked_replace/reset's lock order.
    /// Returning shared evidence makes latest-version collection allocation-free.
    pub(super) fn manual_success(
        &self,
        epoch: &Arc<()>,
    ) -> Option<(Arc<()>, Arc<()>, DeviceAuthority)> {
        let state = self.lock();
        if !Arc::ptr_eq(&state.epoch, epoch) {
            return None;
        }
        let Desired::Manual { authority, .. } = &state.desired else {
            return None;
        };
        let pending = self
            .pending_success
            .lock()
            .unwrap_or_else(|p| p.into_inner());
        let (desired, failure) = pending.as_ref()?;
        if !Arc::ptr_eq(desired, &state.version) {
            return None;
        }
        Some((Arc::clone(desired), Arc::clone(failure), authority.clone()))
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

    pub(super) fn clear_success_checked(
        &self,
        desired: &Arc<()>,
        failure: &Arc<()>,
        check: impl Fn() -> anyhow::Result<()>,
    ) -> anyhow::Result<()> {
        let mut pending = self
            .pending_success
            .lock()
            .unwrap_or_else(|p| p.into_inner());
        check()?;
        if pending.as_ref().is_some_and(|(current, row)| {
            Arc::ptr_eq(current, desired) && Arc::ptr_eq(row, failure)
        }) {
            let original = pending.take();
            if let Err(error) = check() {
                *pending = original;
                return Err(error);
            }
        }
        Ok(())
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

pub(super) struct ManualRequest<'a> {
    pub(super) authority: DeviceAuthority,
    pub(super) host_filter: Option<&'a str>,
}

#[cfg(test)]
mod tests;
