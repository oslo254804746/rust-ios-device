//! Private manager coordination. No network future runs under the device table lock.

use std::collections::HashMap;
use std::future::Future;
use std::sync::{Arc, Weak};
use std::time::Duration;

use anyhow::{Context, Result};
use tokio::sync::{watch, Mutex, Semaphore};
use tokio::time::Instant;

pub(super) const MAX_CONNECTS: usize = 4;
const MAX_DEVICE_GATES: usize = 4096;

#[cfg(test)]
type LifecycleCallback = Box<dyn FnOnce(&Lifecycle) + Send>;

pub(super) struct Lifecycle {
    devices: Mutex<HashMap<String, Weak<DeviceGate>>>,
    pub(super) connects: Semaphore,
    // Some is a reusable clear epoch; None is terminal server shutdown.
    shutdown: watch::Sender<Option<Arc<()>>>,
    scan: watch::Sender<Arc<()>>,
    #[cfg(test)]
    scan_after_open: std::sync::Mutex<Option<LifecycleCallback>>,
    #[cfg(test)]
    failure_before_commit: std::sync::Mutex<Option<LifecycleCallback>>,
}

struct DeviceGate {
    serial: Mutex<()>,
    generation: watch::Sender<Arc<()>>,
}

pub(super) struct Ticket {
    gate: Arc<DeviceGate>,
    generation: Arc<()>,
    shutdown: watch::Receiver<Option<Arc<()>>>,
    shutdown_generation: Arc<()>,
}

pub(super) struct ScanControl {
    changes: watch::Receiver<Arc<()>>,
    pub(super) generation: Arc<()>,
    deadline: Instant,
}

pub(super) struct ConnectionDeadline {
    deadline: Instant,
    timeout_context: &'static str,
}

#[derive(Debug)]
struct ConnectionDeadlineReached;

impl std::fmt::Display for ConnectionDeadlineReached {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("tunnel connection deadline reached")
    }
}

impl std::error::Error for ConnectionDeadlineReached {}

impl ConnectionDeadline {
    pub(super) fn new(timeout: Duration) -> Result<Self> {
        Self::with_context(timeout, "timed out while creating tunnel")
    }

    pub(super) fn for_scan_candidate(timeout: Duration) -> Result<Self> {
        Self::with_context(timeout, "timed out establishing scanned tunnel")
    }

    pub(super) fn for_scan_bookkeeping(timeout: Duration) -> Result<Self> {
        Self::with_context(timeout, "tunnel scan failure bookkeeping total expired")
    }

    fn with_context(timeout: Duration, timeout_context: &'static str) -> Result<Self> {
        let deadline = Instant::now()
            .checked_add(timeout)
            .ok_or_else(|| crate::output::input_decision("tunnel connection deadline overflow"))?;
        Ok(Self {
            deadline,
            timeout_context,
        })
    }

    /// A synchronous fence for side effects inside a poll. Only run converts
    /// this private decision into the independent timer's original Elapsed.
    pub(super) fn check(&self) -> Result<()> {
        if Instant::now() >= self.deadline {
            Err(ConnectionDeadlineReached.into())
        } else {
            Ok(())
        }
    }

    pub(super) async fn run<T>(&self, work: impl Future<Output = Result<T>>) -> Result<T> {
        tokio::pin!(work);
        tokio::select! {
            biased;
            expired = tokio::time::timeout_at(self.deadline, std::future::pending::<T>()) => {
                expired.context(self.timeout_context)
            }
            result = std::future::poll_fn(|cx| {
                // Tokio's timeout polls its work before the timer. Suppress
                // late ready work, including publication, at the nominal deadline.
                if Instant::now() >= self.deadline {
                    std::task::Poll::Pending
                } else {
                    match work.as_mut().poll(cx) {
                        std::task::Poll::Ready(Err(error)) if error.is::<ConnectionDeadlineReached>() => {
                            std::task::Poll::Pending
                        }
                        std::task::Poll::Ready(Ok(_)) if Instant::now() >= self.deadline => {
                            std::task::Poll::Pending
                        }
                        result => result,
                    }
                }
            }) => result,
        }
    }
}

impl Lifecycle {
    pub(super) fn new() -> Self {
        Self {
            devices: Mutex::new(HashMap::new()),
            connects: Semaphore::new(MAX_CONNECTS),
            shutdown: watch::channel(Some(Arc::new(()))).0,
            scan: watch::channel(Arc::new(())).0,
            #[cfg(test)]
            scan_after_open: std::sync::Mutex::new(None),
            #[cfg(test)]
            failure_before_commit: std::sync::Mutex::new(None),
        }
    }

    pub(super) async fn ticket(&self, udid: &str) -> Result<Ticket> {
        let mut shutdown = self.shutdown.subscribe();
        let shutdown_generation = shutdown.borrow().clone().ok_or_else(cancelled)?;
        let mut devices = tokio::select! {
            biased;
            _ = shutdown.changed() => return Err(cancelled()),
            devices = self.devices.lock() => devices,
        };
        // Closing never waits for this lock, and an old queued admission must
        // not acquire a new epoch after clear or terminal shutdown.
        if !shutdown
            .borrow()
            .as_ref()
            .is_some_and(|epoch| Arc::ptr_eq(epoch, &shutdown_generation))
        {
            return Err(cancelled());
        }
        let gate = match devices.get(udid).and_then(Weak::upgrade) {
            Some(gate) => gate,
            None => {
                if devices.len() >= MAX_DEVICE_GATES {
                    devices.retain(|_, gate| gate.strong_count() != 0);
                    anyhow::ensure!(
                        devices.len() < MAX_DEVICE_GATES,
                        crate::output::resource_decision("tunnel device coordinator limit")
                    );
                }
                let gate = Arc::new(DeviceGate {
                    serial: Mutex::new(()),
                    generation: watch::channel(Arc::new(())).0,
                });
                devices.insert(udid.to_owned(), Arc::downgrade(&gate));
                gate
            }
        };
        let generation = gate.generation.borrow().clone();
        Ok(Ticket {
            gate,
            generation,
            shutdown,
            shutdown_generation,
        })
    }

    #[cfg(test)]
    pub(super) async fn hold_coordinator(&self) -> impl Drop + '_ {
        self.devices.lock().await
    }

    #[cfg(test)]
    pub(super) async fn coordinator_count(&self) -> usize {
        self.devices.lock().await.len()
    }

    #[cfg(test)]
    pub(super) fn on_next_scan_capture(&self, callback: impl FnOnce(&Lifecycle) + Send + 'static) {
        let mut pending = self.scan_after_open.lock().unwrap();
        assert!(pending.is_none(), "scan capture hook already installed");
        *pending = Some(Box::new(callback));
    }

    #[cfg(test)]
    pub(super) fn on_next_failure_record(
        &self,
        callback: impl FnOnce(&Lifecycle) + Send + 'static,
    ) {
        let mut pending = self.failure_before_commit.lock().unwrap();
        assert!(pending.is_none(), "failure record hook already installed");
        *pending = Some(Box::new(callback));
    }

    #[cfg(test)]
    pub(super) fn before_failure_record(&self) {
        let callback = {
            let mut pending = self.failure_before_commit.lock().unwrap();
            let callback = pending.take();
            drop(pending);
            callback
        };
        if let Some(callback) = callback {
            callback(self);
        }
    }

    // The caller holds the tunnel table write guard. This fences publication
    // together with removal without waiting for a connecting device's serial lock.
    #[cfg(test)]
    pub(super) async fn stop(&self, udid: &str) {
        let _ = self.stop_owned(udid).await;
    }

    pub(super) async fn stop_owned(&self, udid: &str) -> Option<Arc<()>> {
        let gate = {
            let devices = self.devices.lock().await;
            devices.get(udid).and_then(Weak::upgrade)
        };
        let previous = gate.map(|gate| gate.generation.send_replace(Arc::new(())));
        self.scan.send_replace(Arc::new(()));
        previous
    }

    pub(super) fn clear(&self) {
        let _ = self.clear_owned();
    }

    pub(super) fn clear_owned(&self) -> Option<Arc<()>> {
        let mut previous = None;
        self.shutdown.send_if_modified(|epoch| {
            let Some(current) = epoch else { return false };
            previous = Some(Arc::clone(current));
            *epoch = Some(Arc::new(()));
            true
        });
        self.scan.send_replace(Arc::new(()));
        previous
    }

    pub(super) fn close(&self) {
        self.shutdown.send_replace(None);
        self.scan.send_replace(Arc::new(()));
    }

    pub(super) fn scan_control(&self, timeout: Duration) -> Result<ScanControl> {
        if self.shutdown.borrow().is_none() {
            return Err(cancelled());
        }
        let deadline = Instant::now().checked_add(timeout).ok_or_else(|| {
            crate::output::input_decision("tunnel scan discovery deadline overflow")
        })?;
        #[cfg(test)]
        {
            // Force one admission boundary on this instance, without retaining
            // any watch Ref or hook mutex while invoking close/clear.
            let callback = {
                let mut pending = self.scan_after_open.lock().unwrap();
                let callback = pending.take();
                drop(pending);
                callback
            };
            if let Some(callback) = callback {
                callback(self);
            }
        }
        let mut changes = self.scan.subscribe();
        let generation = changes.borrow_and_update().clone();
        // A completed close may already be marked seen by this subscription.
        // Terminal shutdown is monotonic, so reject it after capturing the epoch.
        if self.shutdown.borrow().is_none() {
            return Err(cancelled());
        }
        Ok(ScanControl {
            changes,
            generation,
            deadline,
        })
    }

    pub(super) fn scan_is_current(&self, generation: &Arc<()>) -> bool {
        self.shutdown.borrow().is_some() && Arc::ptr_eq(&self.scan.borrow(), generation)
    }
}

impl ScanControl {
    pub(super) async fn drive<T>(&mut self, work: impl Future<Output = T>) -> Result<Option<T>> {
        if !Arc::ptr_eq(&self.changes.borrow(), &self.generation) {
            return Ok(None);
        }
        let deadline = self.deadline;
        tokio::pin!(work);
        tokio::select! {
            biased;
            _ = self.changes.changed() => Ok(None),
            expired = tokio::time::timeout_at(deadline, std::future::pending::<T>()) => {
                expired.map(Some).context("tunnel scan discovery total expired")
            }
            result = std::future::poll_fn(|cx| {
                // Suppress late ready locks and tickets on every poll, even
                // before the independently polled native timer becomes ready.
                if Instant::now() >= deadline {
                    std::task::Poll::Pending
                } else {
                    work.as_mut().poll(cx)
                }
            }) => Ok(Some(result)),
        }
    }
}

impl Ticket {
    pub(super) fn ownership_stamp(&self) -> super::retirement::OwnershipStamp {
        super::retirement::OwnershipStamp::new(
            Arc::clone(&self.shutdown_generation),
            Arc::clone(&self.generation),
        )
    }

    pub(super) fn is_current(&self) -> bool {
        Arc::ptr_eq(&self.gate.generation.borrow(), &self.generation)
            && self
                .shutdown
                .borrow()
                .as_ref()
                .is_some_and(|epoch| Arc::ptr_eq(epoch, &self.shutdown_generation))
    }

    // The caller already owns its destination table guard. Keep both epoch
    // borrows through the synchronous mutation so stop/clear/close either win
    // before admission or wait until this commit finishes. The action must not
    // await or reenter these watches, including through is_current().
    pub(super) fn commit<T>(&self, action: impl FnOnce() -> Result<T>) -> Result<T> {
        let generation = self.gate.generation.borrow();
        let shutdown = self.shutdown.borrow();
        if !Arc::ptr_eq(&generation, &self.generation)
            || !shutdown
                .as_ref()
                .is_some_and(|epoch| Arc::ptr_eq(epoch, &self.shutdown_generation))
        {
            return Err(cancelled());
        }
        let result = action();
        drop(shutdown);
        drop(generation);
        result
    }

    pub(super) async fn serial(&self) -> tokio::sync::MutexGuard<'_, ()> {
        self.gate.serial.lock().await
    }

    pub(super) async fn run<T>(&self, work: impl Future<Output = Result<T>>) -> Result<T> {
        let mut device = self.gate.generation.subscribe();
        let mut shutdown = self.shutdown.clone();
        if !self.is_current() {
            return Err(cancelled());
        }
        tokio::select! {
            biased;
            _ = device.changed() => Err(cancelled()),
            _ = shutdown.changed() => Err(cancelled()),
            result = work => result,
        }
    }
}

pub(super) fn cancelled() -> anyhow::Error {
    anyhow::Error::new(ios_core::OperationCancelled::new()).context("tunnel operation cancelled")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn lifecycle_tunnel_coordinator_cancel_returns_before_held_lock_release() {
        for terminal in [false, true] {
            let lifecycle = Lifecycle::new();
            let held = lifecycle.devices.lock().await;
            let mut queued = Box::pin(lifecycle.ticket("synthetic-waiting-device"));
            assert!(futures_util::poll!(&mut queued).is_pending());
            if terminal {
                lifecycle.close();
            } else {
                lifecycle.clear();
            }
            let outcome = tokio::time::timeout(Duration::from_secs(1), queued).await;
            assert!(
                held.is_empty(),
                "cancelled admission created coordinator state"
            );
            drop(held);
            match outcome.expect("cancellation waited for the held coordinator lock") {
                Err(error) => {
                    assert!(error.is::<ios_core::OperationCancelled>());
                    assert_eq!(
                        ios_core::classify_error(error.as_ref()),
                        ios_core::ErrorClass::Cancelled
                    );
                }
                Ok(_) => panic!("old admission resumed after cancellation"),
            }
            if !terminal {
                assert!(lifecycle.ticket("synthetic-waiting-device").await.is_ok());
            }
        }
    }

    #[tokio::test]
    async fn lifecycle_tunnel_shutdown_admission_coordinator_wait_keeps_original_epoch() {
        for terminal in [false, true] {
            let lifecycle = Lifecycle::new();
            let held = lifecycle.devices.lock().await;
            let mut queued = Box::pin(lifecycle.ticket("synthetic-queued-device"));
            assert!(futures_util::poll!(&mut queued).is_pending());
            if terminal {
                lifecycle.close();
            } else {
                lifecycle.clear();
            }
            drop(held);
            let outcome = tokio::time::timeout(Duration::from_secs(1), queued)
                .await
                .unwrap();
            match outcome {
                Err(error) => assert!(error.is::<ios_core::OperationCancelled>()),
                Ok(_) => panic!("queued admission acquired a replacement epoch"),
            }
            assert!(lifecycle.devices.lock().await.is_empty());
            if !terminal {
                assert!(lifecycle.ticket("synthetic-queued-device").await.is_ok());
            }
        }
    }
}
