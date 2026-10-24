//! Dormant worker launch, with control separate from the original join owners.
use std::future::Future;
use std::sync::{Arc, Mutex};

use tokio::sync::{Notify, Semaphore};

use super::runtime_storage::{RuntimeRoot, TaskHandles};
use super::{watch, Ledger, ParentRole, PreparedWorker, TunnelError};

#[derive(Clone, Copy, PartialEq, Eq)]
enum Phase {
    Prepared,
    Open,
    Cancelled,
}

struct Control {
    phase: Mutex<Phase>,
    changed: Notify,
    ledger: Ledger,
}

#[derive(Clone)]
pub(super) struct LaunchControl(RuntimeRoot<Control>);

impl LaunchControl {
    pub(super) fn prepare(
        bank: Option<&Arc<Semaphore>>,
        ledger: &Ledger,
    ) -> Result<Self, TunnelError> {
        let control = Self(RuntimeRoot::new(
            Control {
                phase: Mutex::new(Phase::Prepared),
                changed: Notify::new(),
                ledger: ledger.clone(),
            },
            bank,
        )?);
        ledger.prepare_launch();
        Ok(control)
    }

    pub(super) fn opened(&self) -> bool {
        *self.0.phase.lock().unwrap_or_else(|p| p.into_inner()) == Phase::Open
    }

    fn open(&self) -> bool {
        let opened;
        {
            let mut phase = self.0.phase.lock().unwrap_or_else(|p| p.into_inner());
            if *phase == Phase::Prepared {
                *phase = Phase::Open;
            }
            opened = *phase == Phase::Open;
        }
        self.0.changed.notify_waiters();
        opened
    }

    pub(super) fn cancel_state(&self) {
        *self.0.phase.lock().unwrap_or_else(|p| p.into_inner()) = Phase::Cancelled;
    }

    pub(super) fn cancel(&self) {
        self.cancel_state();
        self.0.changed.notify_waiters();
    }

    pub(super) async fn wait(&self) -> bool {
        loop {
            let changed = self.0.changed.notified();
            tokio::pin!(changed);
            changed.as_mut().enable();
            let phase = *self.0.phase.lock().unwrap_or_else(|p| p.into_inner());
            match phase {
                Phase::Open => return true,
                Phase::Cancelled => return false,
                Phase::Prepared => changed.await,
            }
        }
    }

    pub(super) async fn finished(&self) {
        loop {
            let changed = self.0.changed.notified();
            tokio::pin!(changed);
            changed.as_mut().enable();
            if !self.0.ledger.is_launch_pending() {
                return;
            }
            changed.await;
        }
    }

    #[cfg(test)]
    pub(super) fn body_bytes() -> usize {
        RuntimeRoot::<Control>::body_bytes()
    }
}

/// This is the actual outer worker F passed to PreparedWorker::prepare. No
/// worker owns TaskHandles, and a cancelled dormant gate polls no original IO.
pub(super) async fn gated<F>(
    control: LaunchControl,
    ledger: Ledger,
    work: F,
) -> Result<(), TunnelError>
where
    F: Future<Output = Result<(), TunnelError>>,
{
    if !control.wait().await {
        return ledger.work_result();
    }
    work.await
}

pub(super) struct LaunchGuard {
    tasks: TaskHandles,
    control: LaunchControl,
    ledger: Ledger,
    stop: watch::Sender,
    armed: bool,
}

impl LaunchGuard {
    pub(super) fn new(
        tasks: TaskHandles,
        control: LaunchControl,
        ledger: Ledger,
        stop: watch::Sender,
    ) -> Self {
        Self {
            tasks,
            control,
            ledger,
            stop,
            armed: true,
        }
    }

    pub(super) fn spawn<F>(&self, index: usize, worker: PreparedWorker<F>, role: ParentRole)
    where
        F: Future<Output = Result<(), TunnelError>> + Send + 'static,
    {
        // No await, allocation admission or callback separates the original
        // spawn from its permanent slot. Later launch unwind retains this join.
        let task = worker.spawn(role, self.ledger.clone());
        self.tasks.install(index, task);
        #[cfg(test)]
        INSTALLED.with(|hook| {
            if let Some(callback) = hook.borrow_mut().as_mut() {
                callback(index);
            }
        });
    }

    pub(super) fn check(&self) -> Result<(), TunnelError> {
        if *self
            .control
            .0
            .phase
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            == Phase::Cancelled
            || self.ledger.is_runtime_stopped()
        {
            // An already captured native work cause wins over local stop.
            self.ledger.work_result()?;
            return Err(cancelled());
        }
        Ok(())
    }

    pub(super) fn open(mut self) -> Result<(), TunnelError> {
        assert!(
            self.tasks.all_installed(),
            "all original joins precede launch"
        );
        self.check()?;
        if !self.control.open() {
            self.ledger.work_result()?;
            return Err(cancelled());
        }
        self.armed = false;
        Ok(())
    }
}

fn cancelled() -> TunnelError {
    std::io::Error::other(crate::OperationCancelled::new()).into()
}

/// This stack guard performs no notification. Even a synchronous waker panic
/// cannot skip abort requests; the actual typed join owners remain in slots.
pub(super) struct AbortInstalled<'a> {
    tasks: &'a TaskHandles,
    count: usize,
}
impl<'a> AbortInstalled<'a> {
    fn all(tasks: &'a TaskHandles) -> Self {
        Self { tasks, count: 4 }
    }
    pub(super) fn first_three(tasks: &'a TaskHandles) -> Self {
        Self { tasks, count: 3 }
    }
}
impl Drop for AbortInstalled<'_> {
    fn drop(&mut self) {
        for task in self.tasks.iter().take(self.count) {
            task.abort();
        }
    }
}

impl Drop for LaunchGuard {
    fn drop(&mut self) {
        // The containing PreparedLaunch declares all original unspawned Fs
        // before this guard. Their Drop completes before launch settles.
        self.ledger.finish_launch();
        if self.armed {
            let _abort = AbortInstalled::all(&self.tasks);
            self.ledger.request_runtime_stop();
            self.control.cancel_state();
            // Unwind already owns the original panic. Abort wakes the actual
            // Tokio workers without invoking arbitrary control waiters again.
            if !std::thread::panicking() {
                self.control.0.changed.notify_waiters();
                self.stop.send_replace(());
            }
        } else if !std::thread::panicking() {
            self.control.0.changed.notify_waiters();
        }
    }
}

/// Field order also covers an uncalled launch and unwind between installs.
/// Every remaining original F dies before the last guard publishes finished.
pub(super) struct PreparedLaunch<R, W, S, L> {
    reader: Option<PreparedWorker<R>>,
    writer: Option<PreparedWorker<W>>,
    smoltcp: Option<PreparedWorker<S>>,
    listener: Option<PreparedWorker<L>>,
    guard: LaunchGuard,
}
impl<R, W, S, L> PreparedLaunch<R, W, S, L>
where
    R: Future<Output = Result<(), TunnelError>> + Send + 'static,
    W: Future<Output = Result<(), TunnelError>> + Send + 'static,
    S: Future<Output = Result<(), TunnelError>> + Send + 'static,
    L: Future<Output = Result<(), TunnelError>> + Send + 'static,
{
    pub(super) fn new(
        reader: PreparedWorker<R>,
        writer: PreparedWorker<W>,
        smoltcp: PreparedWorker<S>,
        listener: PreparedWorker<L>,
        guard: LaunchGuard,
    ) -> Self {
        Self {
            reader: Some(reader),
            writer: Some(writer),
            smoltcp: Some(smoltcp),
            listener: Some(listener),
            guard,
        }
    }
    pub(super) fn run(mut self) -> Result<(), TunnelError> {
        self.guard.check()?;
        self.guard.spawn(
            0,
            self.reader.take().expect("prepared reader"),
            ParentRole::ForwardOrReader,
        );
        self.guard.spawn(
            1,
            self.writer.take().expect("prepared writer"),
            ParentRole::Writer,
        );
        self.guard.spawn(
            2,
            self.smoltcp.take().expect("prepared smoltcp"),
            ParentRole::Smoltcp,
        );
        self.guard.spawn(
            3,
            self.listener.take().expect("prepared listener"),
            ParentRole::Listener,
        );
        self.guard.open()
    }
}

#[cfg(test)]
type InstalledHook = std::cell::RefCell<Option<Box<dyn FnMut(usize)>>>;
#[cfg(test)]
thread_local! { static INSTALLED: InstalledHook = const { std::cell::RefCell::new(None) }; }
#[cfg(test)]
pub(super) struct HookGuard;
#[cfg(test)]
impl Drop for HookGuard {
    fn drop(&mut self) {
        INSTALLED.with(|hook| hook.borrow_mut().take());
    }
}
#[cfg(test)]
pub(super) fn on_installed(callback: impl FnMut(usize) + 'static) -> HookGuard {
    INSTALLED.with(|hook| {
        assert!(hook.borrow().is_none());
        *hook.borrow_mut() = Some(Box::new(callback));
    });
    HookGuard
}
