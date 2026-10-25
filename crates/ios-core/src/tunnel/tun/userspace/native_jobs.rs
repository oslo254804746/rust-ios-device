//! Creation-owned blocking calls. The actual join is installed before call IO.
use std::{
    mem,
    sync::{Arc, Condvar, Mutex, OnceLock},
};
use tokio::sync::{OwnedSemaphorePermit, Semaphore};

use super::{
    runtime_storage::{charge, RuntimeRoot},
    Ledger, OwnedTask, ParentRole, TunnelError,
};

enum Admission {
    Waiting,
    Open,
    Cancelled,
}
struct Gate {
    state: Mutex<Admission>,
    changed: Condvar,
}
#[derive(Clone)]
struct StartGate(RuntimeRoot<Gate>);
impl StartGate {
    fn prepare(bank: &Arc<Semaphore>) -> Result<Self, TunnelError> {
        Ok(Self(RuntimeRoot::new(
            Gate {
                state: Mutex::new(Admission::Waiting),
                changed: Condvar::new(),
            },
            Some(bank),
        )?))
    }
    fn wait(&self) -> bool {
        let mut state = self.0.state.lock().unwrap_or_else(|p| p.into_inner());
        loop {
            match *state {
                Admission::Open => return true,
                Admission::Cancelled => return false,
                Admission::Waiting => {
                    state = self
                        .0
                        .changed
                        .wait(state)
                        .unwrap_or_else(|p| p.into_inner());
                }
            }
        }
    }
    fn open(&self) {
        *self.0.state.lock().unwrap_or_else(|p| p.into_inner()) = Admission::Open;
        self.0.changed.notify_all();
    }
    fn cancel(&self) {
        let mut state = self.0.state.lock().unwrap_or_else(|p| p.into_inner());
        if matches!(*state, Admission::Waiting) {
            *state = Admission::Cancelled;
        }
        drop(state);
        self.0.changed.notify_all();
    }
}

/// Inline field of the separately paid phase owner. Never cloned into its own
/// blocking call. Cancellation of a waiter cannot take this original handle.
pub(super) struct BlockingSlot {
    task: OnceLock<OwnedTask>,
}
impl BlockingSlot {
    pub(super) fn new() -> Self {
        Self {
            task: OnceLock::new(),
        }
    }
    fn install(&self, task: OwnedTask) {
        assert!(
            self.task.set(task).is_ok(),
            "one original blocking join per phase slot"
        );
        #[cfg(test)]
        AFTER_INSTALL.with(|hook| {
            if let Some(callback) = hook.borrow_mut().take() {
                callback();
            }
        });
    }
    pub(super) async fn join(&self) {
        if let Some(task) = self.task.get() {
            task.join().await;
        }
    }
    pub(super) fn installed(&self) -> bool {
        self.task.get().is_some()
    }
    pub(super) fn is_finished(&self) -> bool {
        self.task.get().is_some_and(OwnedTask::is_finished)
    }
}

struct Call<F> {
    start: StartGate,
    body: F,
}
trait BlockingCall: Send {
    fn run(self: Box<Self>) -> Result<(), TunnelError>;
}
impl<F: FnOnce() -> Result<(), TunnelError> + Send> BlockingCall for Call<F> {
    fn run(self: Box<Self>) -> Result<(), TunnelError> {
        let Call { start, body } = *self;
        if start.wait() {
            body()
        } else {
            drop(body);
            Ok(())
        }
    }
}
struct AdmittedCall {
    // Box deallocation and complete F destruction precede the original fee,
    // including cancellation-before-start and a panicking original call.
    call: Box<dyn BlockingCall>,
    _storage: OwnedSemaphorePermit,
}
impl AdmittedCall {
    fn run(self) -> Result<(), TunnelError> {
        let Self { call, _storage } = self;
        call.run()
    }
}

/// The actual outer call captures the gate and all operation state. Its exact
/// Box is paid before constructing F; this is not a reserve for SDK internals.
/// A body records acquired errors and stores resource/buffer results in its
/// durable control destination before returning or dropping its captures.
pub(super) struct PreparedBlocking {
    call: Option<AdmittedCall>,
    start: StartGate,
}
impl PreparedBlocking {
    pub(super) fn prepare<F>(
        bank: &Arc<Semaphore>,
        make: impl FnOnce() -> F,
    ) -> Result<Self, TunnelError>
    where
        F: FnOnce() -> Result<(), TunnelError> + Send + 'static,
    {
        let start = StartGate::prepare(bank)?;
        let storage =
            charge(Some(bank), mem::size_of::<Call<F>>())?.expect("original blocking call bank");
        let call = AdmittedCall {
            call: Box::new(Call {
                start: start.clone(),
                body: make(),
            }),
            _storage: storage,
        };
        Ok(Self {
            call: Some(call),
            start,
        })
    }

    pub(super) fn start(mut self, slot: &BlockingSlot, role: ParentRole, ledger: Ledger) {
        assert!(
            !slot.installed(),
            "blocking phase cannot replace a prior join"
        );
        let call = self.call.take().expect("one original prepared call");
        // Tokio's returned handle is immediately installed. The gate's real
        // condvar cannot invoke an arbitrary async waker between these steps.
        let handle = tokio::task::spawn_blocking(move || call.run());
        slot.install(OwnedTask::new(handle, role, ledger));
        self.start.open();
    }
}
impl Drop for PreparedBlocking {
    fn drop(&mut self) {
        self.start.cancel();
    }
}

#[cfg(test)]
thread_local! {
    static AFTER_INSTALL: std::cell::RefCell<Option<Box<dyn FnOnce()>>> = const { std::cell::RefCell::new(None) };
}

#[cfg(test)]
mod tests;
