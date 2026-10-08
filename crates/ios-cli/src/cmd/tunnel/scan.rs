//! Whole-scan control and bounded ownership of errors acquired before cancellation.
use std::error::Error;
use std::future::Future;
use std::sync::{Arc, Mutex as StdMutex};
use std::time::Duration;

use anyhow::{Context, Result};
use tokio::sync::{watch, MutexGuard};
use tokio::time::Instant;

pub(super) const MAX_ABANDONED_SCANS: usize = 8;

enum Slot {
    Empty,
    Active,
    Abandoned(Arc<AbandonedScan>),
}

struct Slots {
    closed: bool,
    entries: [Slot; MAX_ABANDONED_SCANS],
}

pub(super) struct Outcomes(StdMutex<Slots>);

impl Outcomes {
    pub(super) fn new() -> Arc<Self> {
        Arc::new(Self(StdMutex::new(Slots {
            closed: false,
            entries: std::array::from_fn(|_| Slot::Empty),
        })))
    }

    pub(super) fn begin<'a>(self: &Arc<Self>, lock: MutexGuard<'a, ()>) -> Result<Session<'a>> {
        let mut slots = self.0.lock().unwrap_or_else(|p| p.into_inner());
        if slots.closed {
            return Err(super::lifecycle::cancelled());
        }
        let index = slots
            .entries
            .iter()
            .position(|slot| matches!(slot, Slot::Empty))
            .ok_or_else(|| {
                crate::output::resource_decision("tunnel abandoned scan error capacity")
            })?;
        slots.entries[index] = Slot::Active;
        Ok(Session {
            outcomes: Arc::clone(self),
            index,
            primary: StdMutex::new(None),
            _lock: lock,
        })
    }

    pub(super) fn close(&self) {
        self.0.lock().unwrap_or_else(|p| p.into_inner()).closed = true;
    }

    pub(super) fn history(&self) -> Vec<Arc<AbandonedScan>> {
        self.0
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .entries
            .iter()
            .filter_map(|slot| match slot {
                Slot::Abandoned(error) => Some(Arc::clone(error)),
                _ => None,
            })
            .collect()
    }
}

pub(super) struct Session<'a> {
    outcomes: Arc<Outcomes>,
    index: usize,
    primary: StdMutex<Option<anyhow::Error>>,
    // Drop records the original error before this guard admits another scanner.
    _lock: MutexGuard<'a, ()>,
}

impl Session<'_> {
    pub(super) fn has_primary(&self) -> bool {
        self.primary
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .is_some()
    }
    pub(super) fn retain(&self, error: anyhow::Error) {
        let mut primary = self.primary.lock().unwrap_or_else(|p| p.into_inner());
        debug_assert!(primary.is_none(), "one discovery primary per scan");
        if primary.is_none() {
            *primary = Some(error);
        }
    }

    pub(super) fn take(&self) -> Option<anyhow::Error> {
        self.primary
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .take()
    }

    pub(super) fn finish(&self, result: Result<Option<()>>) -> Result<()> {
        match (self.take(), result) {
            (Some(primary), Err(secondary)) => {
                Err(primary.context(super::bookkeeping::FailureContext::new(secondary)))
            }
            (Some(primary), Ok(None)) => Err(primary.context(
                super::bookkeeping::FailureContext::new(super::lifecycle::cancelled()),
            )),
            (Some(primary), Ok(Some(()))) => Err(primary),
            (None, result) => result.map(|_| ()),
        }
    }
}

impl Drop for Session<'_> {
    fn drop(&mut self) {
        let primary = self.take();
        let mut slots = self.outcomes.0.lock().unwrap_or_else(|p| p.into_inner());
        slots.entries[self.index] = match primary {
            Some(primary) => Slot::Abandoned(Arc::new(AbandonedScan(primary))),
            None => Slot::Empty,
        };
    }
}

pub(super) struct AbandonedScan(anyhow::Error);

impl AbandonedScan {
    pub(super) fn original(&self) -> &anyhow::Error {
        &self.0
    }
}

impl std::fmt::Display for AbandonedScan {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("tunnel scan was abandoned after a discovery failure")
    }
}

impl std::fmt::Debug for AbandonedScan {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AbandonedTunnelScan")
            .field(
                "primary_class",
                &ios_core::classify_error(self.original().as_ref()),
            )
            .finish()
    }
}

impl Error for AbandonedScan {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        Some(self.0.as_ref())
    }
}

#[derive(Clone)]
pub(super) struct Epoch {
    pub(super) changes: watch::Receiver<Option<Arc<()>>>,
    pub(super) generation: Arc<()>,
}

impl Epoch {
    fn current(&self) -> bool {
        self.changes
            .borrow()
            .as_ref()
            .is_some_and(|epoch| Arc::ptr_eq(epoch, &self.generation))
    }

    pub(super) fn commit<T>(
        &self,
        clock: &Deadline,
        action: impl FnOnce() -> Result<T>,
    ) -> Result<T> {
        let epoch = self.changes.borrow();
        if !epoch
            .as_ref()
            .is_some_and(|epoch| Arc::ptr_eq(epoch, &self.generation))
        {
            return Err(super::lifecycle::cancelled());
        }
        clock.check()?;
        action()
    }

    // Evaluate device metadata before taking a watch guard. If the clock
    // crosses during removals, restore every unobserved original owner.
    pub(super) fn retain<T>(
        &self,
        clock: &Deadline,
        table: &mut std::collections::HashMap<String, T>,
        keep: impl Fn(&str, &T) -> bool,
    ) -> Result<()> {
        let keys = table
            .iter()
            .filter(|(key, value)| !keep(key, value))
            .map(|(key, _)| key.clone())
            .collect::<Vec<_>>();
        let mut removed = Vec::new();
        self.commit(clock, || {
            for key in keys {
                if let Some(value) = table.remove(&key) {
                    removed.push((key, value));
                }
            }
            if let Err(error) = clock.check() {
                table.extend(removed.drain(..));
                return Err(error);
            }
            Ok(())
        })
    }
}

pub(super) struct Deadline {
    pub(super) at: Instant,
}

#[derive(Debug)]
pub(super) struct DeadlineReached;
impl std::fmt::Display for DeadlineReached {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("tunnel whole-scan deadline reached")
    }
}
impl Error for DeadlineReached {}

impl Deadline {
    pub(super) fn new(timeout: Duration) -> Result<Self> {
        let at = Instant::now()
            .checked_add(timeout)
            .ok_or_else(|| crate::output::input_decision("tunnel whole-scan deadline overflow"))?;
        Ok(Self { at })
    }

    pub(super) fn check(&self) -> Result<()> {
        if Instant::now() >= self.at {
            Err(DeadlineReached.into())
        } else {
            Ok(())
        }
    }

    pub(super) async fn run<T>(
        &self,
        epoch: &Epoch,
        work: impl Future<Output = Result<T>>,
    ) -> Result<Option<T>> {
        let mut changes = epoch.changes.clone();
        if !epoch.current() {
            return Ok(None);
        }
        tokio::pin!(work);
        tokio::select! {
            biased;
            _ = changes.changed() => Ok(None),
            expired = tokio::time::timeout_at(self.at, std::future::pending::<T>()) => {
                expired.map(Some).context("tunnel whole-scan total expired")
            }
            result = std::future::poll_fn(|cx| {
                if self.check().is_err() || !epoch.current() {
                    return std::task::Poll::Pending;
                }
                match work.as_mut().poll(cx) {
                    std::task::Poll::Ready(Err(error)) if error.is::<DeadlineReached>() => std::task::Poll::Pending,
                    std::task::Poll::Ready(Err(error)) if error.is::<ios_core::OperationCancelled>() && !epoch.current() => std::task::Poll::Pending,
                    std::task::Poll::Ready(Ok(_)) if self.check().is_err() || !epoch.current() => std::task::Poll::Pending,
                    result => result,
                }
            }) => result.map(Some),
        }
    }
}
