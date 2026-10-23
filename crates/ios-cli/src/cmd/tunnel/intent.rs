//! Current desired state on the existing bounded device coordinator.
//! No retry history, failure permit or independent device task lives here.

use super::paid_watch as watch;
use super::ControlIdentity;
use std::sync::{Arc, Mutex, MutexGuard};
use tokio::sync::{OwnedSemaphorePermit, Semaphore};

use super::candidate_input::RouteRef;
#[cfg(test)]
use super::source::Route;
use super::FailureVersion;
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

pub(super) type RouteOwner = super::shared_body::Strong<RetainedRoute>;

pub(super) struct RetainedRoute {
    pub(super) route: RouteIntent,
    _storage: OwnedSemaphorePermit,
    _text_storage: Option<OwnedSemaphorePermit>,
}

impl RetainedRoute {
    pub(super) fn body_bytes() -> usize {
        RouteOwner::body_bytes()
    }
    pub(super) fn text_bytes(&self) -> usize {
        match &self.route {
            RouteIntent::Usbmux => 0,
            RouteIntent::Usb(host) | RouteIntent::Wifi(host) | RouteIntent::Mobdev2(host) => {
                host.as_ref().map_or(0, String::capacity)
            }
        }
    }

    pub(super) fn established<'a>(
        route: impl Into<RouteRef<'a>>,
        host_filter: Option<&str>,
        bank: &Arc<Semaphore>,
    ) -> anyhow::Result<RouteOwner> {
        Self::copy_established(route.into(), host_filter, bank)
            .map_err(|error| error.context(RouteAdmission))
    }

    fn copy_established(
        route: RouteRef<'_>,
        host_filter: Option<&str>,
        bank: &Arc<Semaphore>,
    ) -> anyhow::Result<RouteOwner> {
        let selected = match route {
            RouteRef::Usbmux => None,
            RouteRef::Usb(host) | RouteRef::Wifi(host) => host,
            RouteRef::Mobdev2(_) => host_filter,
        };
        super::admission::selectors("", selected)?;
        let mut storage = super::admission::known_metadata_charge(bank, Self::body_bytes())?;
        let body = storage
            .split(Self::body_bytes())
            .expect("prepaid desired route body and guard");
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
            RouteRef::Usbmux => RouteIntent::Usbmux,
            RouteRef::Usb(_) => RouteIntent::Usb(host),
            RouteRef::Wifi(_) => RouteIntent::Wifi(host),
            RouteRef::Mobdev2(_) => RouteIntent::Mobdev2(host),
        };
        RouteOwner::new(
            Self {
                route,
                _storage: storage,
                _text_storage: text_storage,
            },
            bank,
            body,
        )
    }

    // Existing moved-input fixtures preserve the original String allocation;
    // production uses established() before creating any route destination.
    #[cfg(test)]
    fn new(incoming_route: RouteIntent, bank: &Arc<Semaphore>) -> anyhow::Result<RouteOwner> {
        let host = match &incoming_route {
            RouteIntent::Usbmux => None,
            RouteIntent::Usb(host) | RouteIntent::Wifi(host) | RouteIntent::Mobdev2(host) => {
                host.as_ref()
            }
        };
        super::admission::selectors("", host.map(String::as_str))?;
        let bytes = Self::body_bytes()
            .checked_add(host.map_or(0, String::capacity))
            .ok_or_else(|| crate::output::resource_decision("tunnel manual route size overflow"))?;
        let mut storage = super::admission::known_metadata_charge(bank, bytes)?;
        let body = storage
            .split(Self::body_bytes())
            .expect("prepaid desired route body and guard");
        let route = incoming_route;
        RouteOwner::new(
            Self {
                route,
                _storage: storage,
                _text_storage: None,
            },
            bank,
            body,
        )
    }

    #[cfg(test)]
    fn bytes(route: &RouteIntent) -> usize {
        Self::body_bytes()
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
        route: RouteOwner,
        authority: DeviceAuthority,
    },
    Stopped,
}

#[derive(Clone)]
pub(super) struct Snapshot {
    pub(super) epoch: ControlIdentity,
    pub(super) version: ControlIdentity,
    pub(super) desired: Desired,
}

pub(super) struct Cell {
    state: Mutex<Snapshot>,
    changes: watch::Sender<ControlIdentity>,
    // Mirrors the existing FailedTunnelAttempt.version; never an independent
    // failure history/permit. Updated by its original commit and rollback.
    failure: Mutex<Option<(FailureVersion, ControlIdentity)>>,
    pending_success: Mutex<Option<(ControlIdentity, FailureVersion)>>,
    metadata_bytes: Arc<Semaphore>,
    #[cfg(test)]
    before_checked_replace: Mutex<Option<ReplaceCallback>>,
    #[cfg(test)]
    after_checked_candidate: Mutex<Option<ReplaceCallback>>,
}

impl Cell {
    #[cfg(test)]
    pub(super) fn new(
        epoch: ControlIdentity,
        metadata_bytes: Arc<Semaphore>,
    ) -> anyhow::Result<Self> {
        let storage = super::admission::known_metadata_charge(
            &metadata_bytes,
            watch::body_bytes::<ControlIdentity>(),
        )?;
        Self::from_grant(epoch, metadata_bytes, storage)
    }
    pub(super) fn from_grant(
        epoch: ControlIdentity,
        metadata_bytes: Arc<Semaphore>,
        storage: OwnedSemaphorePermit,
    ) -> anyhow::Result<Self> {
        let version = ControlIdentity::new(&metadata_bytes);
        let (changes, _) = watch::from_grant(version.clone(), &metadata_bytes, storage)?;
        Ok(Self {
            state: Mutex::new(Snapshot {
                epoch,
                version: version.clone(),
                desired: Desired::Automatic,
            }),
            changes,
            failure: Mutex::new(None),
            pending_success: Mutex::new(None),
            metadata_bytes,
            #[cfg(test)]
            before_checked_replace: Mutex::new(None),
            #[cfg(test)]
            after_checked_candidate: Mutex::new(None),
        })
    }

    pub(super) fn lock(&self) -> MutexGuard<'_, Snapshot> {
        self.state
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
    }

    pub(super) fn snapshot(&self) -> Snapshot {
        self.lock().clone()
    }

    pub(super) fn is_epoch(&self, epoch: &ControlIdentity) -> bool {
        ControlIdentity::ptr_eq(&self.lock().epoch, epoch)
    }

    pub(super) fn reset(&self, epoch: ControlIdentity) {
        // Explicit global clear resets desired states, unlike durable per-device
        // stop across ordinary scans. Do this atomically with the new version.
        let version = {
            let mut state = self.lock();
            state.epoch = epoch;
            state.desired = Desired::Automatic;
            state.version = ControlIdentity::new(&self.metadata_bytes);
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

    pub(super) fn matches(&self, version: &ControlIdentity) -> bool {
        ControlIdentity::ptr_eq(&self.lock().version, version)
    }

    pub(super) fn replace(&self, desired: Desired) -> ControlIdentity {
        let mut state = self.lock();
        state.version = ControlIdentity::new(&self.metadata_bytes);
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
        failure: Option<FailureVersion>,
        check: impl Fn() -> anyhow::Result<()>,
    ) -> anyhow::Result<ControlIdentity> {
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
        let version = ControlIdentity::new(&self.metadata_bytes);
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
        route: RouteOwner,
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
    pub(super) fn subscribe(&self) -> watch::Receiver<ControlIdentity> {
        self.changes.subscribe()
    }

    pub(super) fn failure(&self) -> Option<(FailureVersion, ControlIdentity)> {
        self.failure
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .clone()
    }

    pub(super) fn set_failure(&self, failure: Option<(FailureVersion, ControlIdentity)>) {
        *self.failure.lock().unwrap_or_else(|p| p.into_inner()) = failure;
    }

    pub(super) fn pending_success(&self) -> Option<(ControlIdentity, FailureVersion)> {
        self.pending_success
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .clone()
    }

    /// The state and exact pending tuple use checked_replace/reset's lock order.
    /// Returning shared evidence makes latest-version collection allocation-free.
    pub(super) fn manual_success(
        &self,
        epoch: &ControlIdentity,
    ) -> Option<(ControlIdentity, FailureVersion, DeviceAuthority)> {
        let state = self.lock();
        if !ControlIdentity::ptr_eq(&state.epoch, epoch) {
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
        if !ControlIdentity::ptr_eq(desired, &state.version) {
            return None;
        }
        Some((desired.clone(), failure.clone(), authority.clone()))
    }

    pub(super) fn clear_success(&self, desired: &ControlIdentity, failure: &FailureVersion) {
        let mut pending = self
            .pending_success
            .lock()
            .unwrap_or_else(|p| p.into_inner());
        if pending.as_ref().is_some_and(|(current, row)| {
            ControlIdentity::ptr_eq(current, desired) && FailureVersion::ptr_eq(row, failure)
        }) {
            *pending = None;
        }
    }

    pub(super) fn clear_success_checked(
        &self,
        desired: &ControlIdentity,
        failure: &FailureVersion,
        check: impl Fn() -> anyhow::Result<()>,
    ) -> anyhow::Result<()> {
        let mut pending = self
            .pending_success
            .lock()
            .unwrap_or_else(|p| p.into_inner());
        check()?;
        if pending.as_ref().is_some_and(|(current, row)| {
            ControlIdentity::ptr_eq(current, desired) && FailureVersion::ptr_eq(row, failure)
        }) {
            let original = pending.take();
            if let Err(error) = check() {
                *pending = original;
                return Err(error);
            }
        }
        Ok(())
    }

    pub(super) async fn changed(&self, version: &ControlIdentity) {
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
mod body_tests;
#[cfg(test)]
mod tests;
