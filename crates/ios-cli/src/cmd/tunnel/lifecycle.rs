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

pub(super) struct Lifecycle {
    devices: Mutex<HashMap<String, Weak<DeviceGate>>>,
    pub(super) connects: Semaphore,
    shutdown: watch::Sender<Arc<()>>,
    scan: watch::Sender<Arc<()>>,
}

struct DeviceGate {
    serial: Mutex<()>,
    generation: watch::Sender<Arc<()>>,
}

pub(super) struct Ticket {
    gate: Arc<DeviceGate>,
    generation: Arc<()>,
    shutdown: watch::Receiver<Arc<()>>,
    shutdown_generation: Arc<()>,
}

pub(super) struct ScanControl {
    changes: watch::Receiver<Arc<()>>,
    pub(super) generation: Arc<()>,
    deadline: Instant,
}

impl Lifecycle {
    pub(super) fn new() -> Self {
        Self {
            devices: Mutex::new(HashMap::new()),
            connects: Semaphore::new(MAX_CONNECTS),
            shutdown: watch::channel(Arc::new(())).0,
            scan: watch::channel(Arc::new(())).0,
        }
    }

    pub(super) async fn ticket(&self, udid: &str) -> Result<Ticket> {
        let mut devices = self.devices.lock().await;
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
        let shutdown = self.shutdown.subscribe();
        let shutdown_generation = shutdown.borrow().clone();
        Ok(Ticket {
            gate,
            generation,
            shutdown,
            shutdown_generation,
        })
    }

    // The caller holds the tunnel table write guard. This fences publication
    // together with removal without waiting for a connecting device's serial lock.
    pub(super) async fn stop(&self, udid: &str) {
        if let Some(gate) = self.devices.lock().await.get(udid).and_then(Weak::upgrade) {
            gate.generation.send_replace(Arc::new(()));
        }
        self.scan.send_replace(Arc::new(()));
    }

    pub(super) fn clear(&self) {
        self.shutdown.send_replace(Arc::new(()));
        self.scan.send_replace(Arc::new(()));
    }

    pub(super) fn scan_control(&self, timeout: Duration) -> Result<ScanControl> {
        let deadline = Instant::now().checked_add(timeout).ok_or_else(|| {
            crate::output::input_decision("tunnel scan discovery deadline overflow")
        })?;
        let mut changes = self.scan.subscribe();
        let generation = changes.borrow_and_update().clone();
        Ok(ScanControl {
            changes,
            generation,
            deadline,
        })
    }

    pub(super) fn scan_is_current(&self, generation: &Arc<()>) -> bool {
        Arc::ptr_eq(&self.scan.borrow(), generation)
    }
}

impl ScanControl {
    pub(super) async fn drive<T>(&mut self, work: impl Future<Output = T>) -> Result<Option<T>> {
        if !Arc::ptr_eq(&self.changes.borrow(), &self.generation) {
            return Ok(None);
        }
        // A due deadline must retain native Elapsed without polling real work.
        if Instant::now() >= self.deadline {
            return tokio::time::timeout_at(self.deadline, std::future::pending::<T>())
                .await
                .map(Some)
                .context("tunnel scan discovery total expired");
        }
        tokio::select! {
            biased;
            _ = self.changes.changed() => Ok(None),
            result = tokio::time::timeout_at(self.deadline, work) => {
                result.map(Some).context("tunnel scan discovery total expired")
            }
        }
    }
}

impl Ticket {
    pub(super) fn is_current(&self) -> bool {
        Arc::ptr_eq(&self.gate.generation.borrow(), &self.generation)
            && Arc::ptr_eq(&self.shutdown.borrow(), &self.shutdown_generation)
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
