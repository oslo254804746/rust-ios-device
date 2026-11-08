//! One local FIFO write/flush result, prepaid before any transport worker.
use std::sync::{Arc, Mutex};

use tokio::sync::{Notify, Semaphore};

use super::super::runtime_storage::RuntimeRoot;
use super::super::{watch, Ledger, TunnelError};

#[derive(Clone, Copy, PartialEq, Eq)]
enum Phase {
    Idle,
    Submitting,
    Queued,
    Flushed,
    WriterEnded,
    Uncertain,
}

struct State {
    phase: Mutex<Phase>,
    changed: Notify,
}

#[derive(Clone)]
pub(in crate::tunnel::tun::userspace) struct Barrier(RuntimeRoot<State>);

impl Barrier {
    pub(super) fn prepare(bank: Option<&Arc<Semaphore>>) -> Result<Self, TunnelError> {
        Ok(Self(RuntimeRoot::new(
            State {
                phase: Mutex::new(Phase::Idle),
                changed: Notify::new(),
            },
            bank,
        )?))
    }

    pub(super) fn submission(&self) -> Result<Option<Submission>, TunnelError> {
        let mut phase = self.0.phase.lock().unwrap_or_else(|p| p.into_inner());
        match *phase {
            Phase::Idle => {
                *phase = Phase::Submitting;
                Ok(Some(Submission {
                    barrier: self.clone(),
                    submitted: false,
                }))
            }
            Phase::Queued | Phase::Flushed => Ok(None),
            // Only the unique port lease may submit. A panicking opaque
            // enqueue cannot be retried as an invented unsubmitted barrier.
            _ => Err(cancelled()),
        }
    }

    pub(super) fn flushed(&self) {
        {
            let mut phase = self.0.phase.lock().unwrap_or_else(|p| p.into_inner());
            if matches!(*phase, Phase::Submitting | Phase::Queued) {
                *phase = Phase::Flushed;
            }
        }
        self.0.changed.notify_waiters();
    }

    pub(super) async fn wait(
        &self,
        ledger: &Ledger,
        mut stop: watch::Receiver,
    ) -> Result<(), TunnelError> {
        loop {
            let changed = self.0.changed.notified();
            tokio::pin!(changed);
            changed.as_mut().enable();
            let phase = *self.0.phase.lock().unwrap_or_else(|p| p.into_inner());
            // Acquired original work/join failures take precedence over a
            // local barrier terminal state; no replacement IO is invented.
            ledger.work_result()?;
            match phase {
                Phase::Flushed => return Ok(()),
                Phase::WriterEnded | Phase::Uncertain => return Err(cancelled()),
                _ if ledger.is_runtime_stopped() => return Err(cancelled()),
                _ => {}
            }
            tokio::select! {
                biased;
                _ = changed => {},
                _ = stop.changed() => {},
            }
        }
    }

    #[cfg(test)]
    pub(in super::super) fn body_bytes() -> usize {
        RuntimeRoot::<State>::body_bytes()
    }
}

pub(super) struct Submission {
    barrier: Barrier,
    submitted: bool,
}

impl Submission {
    /// Metered enqueue calls this after the actual queue insertion and before
    /// waking its receiver. The callback itself performs no notification.
    pub(super) fn queued(&mut self) {
        {
            let mut phase = self
                .barrier
                .0
                .phase
                .lock()
                .unwrap_or_else(|p| p.into_inner());
            if *phase == Phase::Submitting {
                *phase = Phase::Queued;
            }
        }
        self.submitted = true;
    }
}

impl Drop for Submission {
    fn drop(&mut self) {
        if self.submitted {
            return;
        }
        {
            let mut phase = self
                .barrier
                .0
                .phase
                .lock()
                .unwrap_or_else(|p| p.into_inner());
            if *phase == Phase::Submitting {
                *phase = if std::thread::panicking() {
                    Phase::Uncertain
                } else {
                    Phase::Idle
                };
            }
        }
        if !std::thread::panicking() {
            self.barrier.0.changed.notify_waiters();
        }
    }
}

pub(super) struct WriterEnd(pub(super) Barrier);

impl Drop for WriterEnd {
    fn drop(&mut self) {
        {
            let mut phase = self.0 .0.phase.lock().unwrap_or_else(|p| p.into_inner());
            if *phase != Phase::Flushed {
                *phase = Phase::WriterEnded;
            }
        }
        if !std::thread::panicking() {
            self.0 .0.changed.notify_waiters();
        }
    }
}

fn cancelled() -> TunnelError {
    std::io::Error::other(crate::OperationCancelled::new()).into()
}
