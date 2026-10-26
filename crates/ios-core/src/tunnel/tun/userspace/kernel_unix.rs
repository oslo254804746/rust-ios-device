//! Same-owner Unix kernel phase over the original packet port.
//! No transport split, hidden native task, or default device factory in tests.
mod platform;
pub(crate) use platform::Plan;
use std::{
    error::Error,
    future::pending,
    future::Future,
    io,
    pin::Pin,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Condvar, Mutex, OnceLock, RwLock,
    },
    task::{Context, Poll},
    time::Duration,
};
use tokio::{
    sync::{Mutex as AsyncMutex, Notify, OwnedSemaphorePermit, Semaphore},
    time::Instant,
};

use super::{
    backend::PortLease,
    kernel_unix::platform::{
        AddressReadiness, AddressReceipt, Device, Identity, InterruptEvent, MtuReceipt,
        NativeError, PacketIo, RouteReceipt,
    },
    native_jobs::{BlockingSlot, PreparedBlocking},
    owned_buffer::OwnedBuffer,
    private_connect::{Direction, FilterOutput, FlowLease, PrivateConnector, PrivateFilter},
    runtime_storage::{charge, RuntimeRoot, TaskHandles},
    Ledger, OwnedTask, ParentRole, TunnelError, UserspaceTunDevice,
};
use crate::tunnel::owner::phase_outcomes::{Phase, PhaseOutcomes};

/// One actual packet slot in a paid complete root. The queue retains its packet
/// across caller cancellation and notifier panic; no payload is in a wake token.
struct PipeState {
    packet: Option<OwnedBuffer>,
    closed: bool,
}
struct Pipe {
    state: Mutex<PipeState>,
    space: Condvar,
    changed: Notify,
    #[cfg(test)]
    full_send_observed: AtomicBool,
}
type PipeRoot = RuntimeRoot<Pipe>;
impl Pipe {
    fn prepare(bank: &Arc<Semaphore>) -> Result<PipeRoot, TunnelError> {
        RuntimeRoot::new(
            Self {
                state: Mutex::new(PipeState {
                    packet: None,
                    closed: false,
                }),
                space: Condvar::new(),
                changed: Notify::new(),
                #[cfg(test)]
                full_send_observed: AtomicBool::new(false),
            },
            Some(bank),
        )
    }
    fn close_state(&self) {
        self.state.lock().unwrap_or_else(|p| p.into_inner()).closed = true;
        self.space.notify_all();
    }
    fn wake(&self) {
        self.changed.notify_waiters();
    }
    fn put_blocking(&self, packet: OwnedBuffer) -> Result<(), OwnedBuffer> {
        let mut state = self.state.lock().unwrap_or_else(|p| p.into_inner());
        while state.packet.is_some() && !state.closed {
            state = self.space.wait(state).unwrap_or_else(|p| p.into_inner());
        }
        if state.closed {
            return Err(packet);
        }
        state.packet = Some(packet);
        drop(state);
        self.changed.notify_waiters();
        Ok(())
    }
    fn take_blocking(&self) -> Option<OwnedBuffer> {
        let mut state = self.state.lock().unwrap_or_else(|p| p.into_inner());
        while state.packet.is_none() && !state.closed {
            state = self.space.wait(state).unwrap_or_else(|p| p.into_inner());
        }
        if state.closed {
            return None;
        }
        let packet = state.packet.take();
        drop(state);
        self.space.notify_all();
        self.changed.notify_waiters();
        packet
    }
    async fn recv(&self) -> Option<OwnedBuffer> {
        loop {
            let changed = self.changed.notified();
            tokio::pin!(changed);
            changed.as_mut().enable();
            let packet = {
                let mut state = self.state.lock().unwrap_or_else(|p| p.into_inner());
                if state.closed {
                    return None;
                }
                state.packet.take()
            };
            if let Some(packet) = packet {
                self.space.notify_all();
                return Some(packet);
            }
            changed.await;
        }
    }
    async fn send_from(&self, pending: &mut Option<OwnedBuffer>) -> Result<(), ()> {
        loop {
            let changed = self.changed.notified();
            tokio::pin!(changed);
            changed.as_mut().enable();
            let sent = {
                let mut state = self.state.lock().unwrap_or_else(|p| p.into_inner());
                if state.closed {
                    return Err(());
                }
                if state.packet.is_none() {
                    state.packet = pending.take();
                    true
                } else {
                    false
                }
            };
            if sent {
                #[cfg(test)]
                self.full_send_observed.store(false, Ordering::Release);
                self.space.notify_all();
                self.changed.notify_waiters();
                return Ok(());
            }
            #[cfg(test)]
            self.full_send_observed.store(true, Ordering::Release);
            changed.await;
        }
    }
    fn release(&self) {
        let packet = self
            .state
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .packet
            .take();
        drop(packet);
    }
}

struct PacketGate {
    state: Mutex<GateState>,
    changed: Condvar,
}
struct GateState {
    open: bool,
    stopped: bool,
    read_started: bool,
    write_started: bool,
    bridge_started: bool,
}
impl PacketGate {
    fn new() -> Self {
        Self {
            state: Mutex::new(GateState {
                open: false,
                stopped: false,
                read_started: false,
                write_started: false,
                bridge_started: false,
            }),
            changed: Condvar::new(),
        }
    }
    fn stop(&self) {
        self.state.lock().unwrap_or_else(|p| p.into_inner()).stopped = true;
        self.changed.notify_all();
    }
    fn open(&self) {
        self.state.lock().unwrap_or_else(|p| p.into_inner()).open = true;
        self.changed.notify_all();
    }
    fn start_packet(&self, read: bool) -> bool {
        let mut state = self.state.lock().unwrap_or_else(|p| p.into_inner());
        while !state.open && !state.stopped {
            state = self.changed.wait(state).unwrap_or_else(|p| p.into_inner());
        }
        if state.stopped {
            return false;
        }
        if read {
            state.read_started = true;
        } else {
            state.write_started = true;
        }
        drop(state);
        self.changed.notify_all();
        true
    }
    fn bridge_started(&self) {
        self.state
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .bridge_started = true;
        self.changed.notify_all();
    }
    fn all_started(&self) -> bool {
        let state = self.state.lock().unwrap_or_else(|p| p.into_inner());
        state.read_started && state.write_started && state.bridge_started
    }
}

/// Sealed: only init's actual successful native observations construct this.
#[derive(Clone, Copy)]
struct NativeReadyProof {
    identity: Identity,
    address: AddressReceipt,
    route: RouteReceipt,
    mtu: MtuReceipt,
}

/// One prepaid cleanup job, renewed only by another explicit shutdown caller.
/// There is no new cleanup future, join slot, receive allocation or fallback
/// clock after exposure. Expiry parks this same real job with its native owner.
struct CleanupClock {
    deadline: Mutex<Option<Instant>>,
    changed: Condvar,
}
impl CleanupClock {
    fn new() -> Self {
        Self {
            deadline: Mutex::new(None),
            changed: Condvar::new(),
        }
    }
    fn set(&self, deadline: Instant) {
        let mut current = self.deadline.lock().unwrap_or_else(|p| p.into_inner());
        // Every waiter has its own outer timer. A shorter later waiter cannot
        // revoke the clock already granted to this same still-running job.
        *current = Some(current.map_or(deadline, |previous| previous.max(deadline)));
        drop(current);
        self.changed.notify_all();
    }
    fn live(&self) -> Instant {
        let mut state = self.deadline.lock().unwrap_or_else(|p| p.into_inner());
        loop {
            if let Some(deadline) = *state {
                if Instant::now() < deadline {
                    return deadline;
                }
            }
            state = self.changed.wait(state).unwrap_or_else(|p| p.into_inner());
        }
    }
    fn retry(&self, deadline: Instant) {
        let state = self.deadline.lock().unwrap_or_else(|p| p.into_inner());
        if *state != Some(deadline) {
            return;
        }
        if let Some(remaining) = deadline.checked_duration_since(Instant::now()) {
            let _wait = self
                .changed
                .wait_timeout(state, remaining.min(Duration::from_millis(10)))
                .unwrap_or_else(|p| p.into_inner());
        }
    }
}
struct Control {
    bank: Arc<Semaphore>,
    plan: Plan,
    deadline: Instant,
    original: Ledger,
    native: Ledger,
    undo: Ledger,
    hub: PhaseOutcomes,
    transport: TaskHandles,
    transport_stop: super::watch::Sender,
    connector: PrivateConnector,
    device: RwLock<Option<Device>>,
    packet: RwLock<Option<PacketIo>>,
    response: Mutex<OwnedBuffer>,
    exposed: AtomicBool,
    cleanup_clock: CleanupClock,
    stop_event: OnceLock<InterruptEvent>,
    gate: PacketGate,
    stopped: AtomicBool,
    changed: Notify,
    launch_finished: AtomicBool,
    work_joined: AtomicBool,
    undo_joined: AtomicBool,
    undo_executed: AtomicBool,
    #[cfg(test)]
    before_undo: Mutex<Option<Box<dyn FnOnce() + Send>>>,
    read_current: Mutex<Option<OwnedBuffer>>,
    write_current: Mutex<Option<OwnedBuffer>>,
    to_bridge: PipeRoot,
    to_native: PipeRoot,
    proof: Mutex<Option<NativeReadyProof>>,
    flow: Mutex<Option<FlowLease>>,
}
type ControlRoot = RuntimeRoot<Control>;
impl Control {
    fn capture<E: Error + Send + Sync + 'static>(&self, role: usize, error: E) {
        self.native.record_work(role, Arc::new(error));
    }
    fn capture_undo<E: Error + Send + Sync + 'static>(&self, role: usize, error: E) {
        self.undo.record_work(role, Arc::new(error));
    }
    fn native<T>(&self, role: usize, result: Result<T, NativeError>) -> Result<T, ()> {
        result.map_err(|error| {
            if !platform::is_deadline(&error) {
                self.capture(role, error);
            }
        })
    }

    fn configure<T>(
        &self,
        device: &mut Device,
        mut operation: impl FnMut(&mut Device, &mut [u8], std::time::Instant) -> Result<T, NativeError>,
    ) -> Result<T, ()> {
        loop {
            if !self.checkpoint() {
                return Err(());
            }
            let mut response = self.response.lock().unwrap_or_else(|p| p.into_inner());
            match operation(device, &mut response, self.deadline.into_std()) {
                Ok(value) => return Ok(value),
                Err(error) => {
                    if let Some(required) = platform::storage_required(&error) {
                        if !self.exposed.load(Ordering::Acquire) {
                            // This is the exact already observed inert response
                            // size. No acquired native IO error is replaced.
                            match OwnedBuffer::zeroed(required, Some(&self.bank)) {
                                Ok(backing) => {
                                    *response = backing;
                                    continue;
                                }
                                Err(original) => {
                                    self.capture(0, original);
                                    return Err(());
                                }
                            }
                        }
                    }
                    return self.native(0, Err(error));
                }
            }
        }
    }
    fn checkpoint(&self) -> bool {
        if self.stopped.load(Ordering::Acquire) || self.original.is_runtime_stopped() {
            return false;
        }
        if Instant::now() >= self.deadline {
            // The single caller timer retains its genuine Elapsed. Observing
            // that clock here cannot manufacture a new native IO failure.
            return false;
        }
        true
    }
    fn stop(&self) {
        let first = !self.stopped.swap(true, Ordering::AcqRel);
        // Complete every synchronous stop state before any arbitrary waker.
        let _abort = super::launch::AbortInstalled::first_three(&self.transport);
        if first {
            self.original.request_runtime_stop();
            self.connector.seal();
            self.gate.stop();
            self.to_bridge.close_state();
            self.to_native.close_state();
        }
        // A stopped admission flag is not proof that a failed OS signal ever
        // woke the actual blocking receive. Retrying shutdown retries this same
        // owned event; the original bounded cleanup slot retains its first IO.
        if let Some(event) = self.stop_event.get() {
            if let Err(error) = event.trigger() {
                self.capture_undo(3, error);
            }
        }
        if !std::thread::panicking() {
            self.to_bridge.wake();
            self.to_native.wake();
            self.changed.notify_waiters();
            self.transport_stop.send_replace(());
        }
    }
    async fn stopped(&self) {
        loop {
            let changed = self.changed.notified();
            tokio::pin!(changed);
            changed.as_mut().enable();
            if self.stopped.load(Ordering::Acquire) {
                return;
            }
            changed.await;
        }
    }
    fn work_result(&self) -> Result<(), TunnelError> {
        self.hub
            .work_result(&self.original, Some(&self.native), Some(&self.undo))
            .map_err(TunnelError::Phases)
    }
}
struct StopOnDrop {
    control: ControlRoot,
    armed: bool,
}
impl StopOnDrop {
    fn new(control: ControlRoot) -> Self {
        Self {
            control,
            armed: true,
        }
    }
}
impl Drop for StopOnDrop {
    fn drop(&mut self) {
        if self.armed {
            self.control.stop();
        }
    }
}

struct WorkSlots {
    init: BlockingSlot,
    read: BlockingSlot,
    write: BlockingSlot,
}
struct BridgeSlot(OnceLock<OwnedTask>);
impl BridgeSlot {
    fn new() -> Self {
        Self(OnceLock::new())
    }
    fn install(&self, task: OwnedTask) {
        assert!(self.0.set(task).is_ok(), "one actual native bridge");
    }
    async fn join(&self) {
        if let Some(task) = self.0.get() {
            task.join().await;
        }
    }
    fn is_finished(&self) -> bool {
        self.0.get().is_some_and(OwnedTask::is_finished)
    }
}
struct PreparedCalls {
    init: PreparedBlocking,
    read: PreparedBlocking,
    write: PreparedBlocking,
}
struct Launch {
    calls: Option<PreparedCalls>,
    undo: Option<PreparedBlocking>,
    bridge: Option<PreparedBridge>,
    started: bool,
}

/// The original concrete bridge F is admitted before erasing its type. The
/// trait-object coercion keeps that same allocation; no hidden inner Box.
struct PreparedBridge {
    future: Pin<Box<dyn Future<Output = Result<(), TunnelError>> + Send>>,
    _storage: OwnedSemaphorePermit,
}
impl PreparedBridge {
    fn prepare(
        control: ControlRoot,
        work: RuntimeRoot<WorkSlots>,
        state: BridgeRoot,
    ) -> Result<Self, TunnelError> {
        let future = run_bridge(control.clone(), work, state);
        let storage = charge(Some(&control.bank), std::mem::size_of_val(&future))?
            .expect("actual original Unix bridge F");
        Ok(Self {
            future: Box::pin(future),
            _storage: storage,
        })
    }
    fn spawn(self, ledger: Ledger) -> OwnedTask {
        OwnedTask::new(tokio::spawn(self), ParentRole::Listener, ledger)
    }
}
impl Future for PreparedBridge {
    type Output = Result<(), TunnelError>;
    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        self.get_mut().future.as_mut().poll(cx)
    }
}
struct BridgeState {
    port: Option<PortLease>,
    filter: Option<PrivateFilter>,
    to_host: Option<OwnedBuffer>,
    to_device: Option<OwnedBuffer>,
    ready_packet: Option<(Direction, OwnedBuffer)>,
    failed: Option<OwnedBuffer>,
}
type BridgeRoot = RuntimeRoot<AsyncMutex<BridgeState>>;

/// No worker captures this owner or its bridge slot. Only bridge F captures the
/// subordinate init/read/write slots, none of which own the bridge JoinHandle.
pub(crate) struct KernelRuntime {
    control: ControlRoot,
    work: RuntimeRoot<WorkSlots>,
    bridge: RuntimeRoot<BridgeSlot>,
    undo: RuntimeRoot<BlockingSlot>,
    launch: RuntimeRoot<Mutex<Launch>>,
    state: BridgeRoot,
}
impl KernelRuntime {
    pub(crate) fn prepare(
        engine: &UserspaceTunDevice,
        bank: Arc<Semaphore>,
        plan: Plan,
        original_deadline: Instant,
    ) -> Result<Self, TunnelError> {
        Self::prepare_with(engine, bank, plan, original_deadline, move |plan| {
            Device::allocate(plan, original_deadline.into_std())
        })
    }
    fn prepare_with<F>(
        engine: &UserspaceTunDevice,
        bank: Arc<Semaphore>,
        plan: Plan,
        deadline: Instant,
        allocate: F,
    ) -> Result<Self, TunnelError>
    where
        F: FnOnce(Plan) -> Result<Device, NativeError> + Send + 'static,
    {
        Self::prepare_with_event(engine, bank, plan, deadline, allocate, || {
            InterruptEvent::new().map_err(NativeError::Native)
        })
    }
    fn prepare_with_event<F, E>(
        engine: &UserspaceTunDevice,
        bank: Arc<Semaphore>,
        plan: Plan,
        deadline: Instant,
        allocate: F,
        event: E,
    ) -> Result<Self, TunnelError>
    where
        F: FnOnce(Plan) -> Result<Device, NativeError> + Send + 'static,
        E: FnOnce() -> Result<InterruptEvent, NativeError>,
    {
        let original = engine.ledger().clone();
        let hub = engine
            .phase_outcomes()
            .ok_or_else(|| protocol("Kernel preparation requires original phases"))?
            .clone();
        let connector = engine
            .private_connector()
            .ok_or_else(|| protocol("Kernel preparation requires private connector"))?;
        let flow = connector.reserve_flows()?;
        let filter = connector.filter(plan.client, &bank)?;
        let native = Ledger::with_metadata(Some(&bank))?;
        let undo = Ledger::with_metadata(Some(&bank))?;
        let to_bridge = Pipe::prepare(&bank)?;
        let to_native = Pipe::prepare(&bank)?;
        let control = RuntimeRoot::new(
            Control {
                bank: bank.clone(),
                plan,
                deadline,
                original,
                native,
                undo,
                hub,
                transport: engine.task_handles.clone(),
                transport_stop: engine.stop.clone(),
                connector,
                device: RwLock::new(None),
                packet: RwLock::new(None),
                response: Mutex::new(OwnedBuffer::zeroed(
                    platform::response_capacity(),
                    Some(&bank),
                )?),
                exposed: AtomicBool::new(false),
                cleanup_clock: CleanupClock::new(),
                stop_event: OnceLock::new(),
                gate: PacketGate::new(),
                stopped: AtomicBool::new(false),
                changed: Notify::new(),
                launch_finished: AtomicBool::new(false),
                work_joined: AtomicBool::new(false),
                undo_joined: AtomicBool::new(false),
                undo_executed: AtomicBool::new(false),
                #[cfg(test)]
                before_undo: Mutex::new(None),
                read_current: Mutex::new(None),
                write_current: Mutex::new(None),
                to_bridge,
                to_native,
                proof: Mutex::new(None),
                flow: Mutex::new(Some(flow)),
            },
            Some(&bank),
        )?;
        let event = event().map_err(|error| TunnelError::Io(io::Error::other(error)))?;
        assert!(
            control.stop_event.set(event).is_ok(),
            "one real native stop event"
        );
        let initial = OwnedBuffer::zeroed(usize::from(plan.mtu) + 1, Some(&bank))?;
        *control
            .read_current
            .lock()
            .unwrap_or_else(|p| p.into_inner()) = Some(initial);
        let work = RuntimeRoot::new(
            WorkSlots {
                init: BlockingSlot::new(),
                read: BlockingSlot::new(),
                write: BlockingSlot::new(),
            },
            Some(&bank),
        )?;
        let bridge = RuntimeRoot::new(BridgeSlot::new(), Some(&bank))?;
        let undo_slot = RuntimeRoot::new(BlockingSlot::new(), Some(&bank))?;
        let state = RuntimeRoot::new(
            AsyncMutex::new(BridgeState {
                port: None,
                filter: Some(filter),
                to_host: None,
                to_device: None,
                ready_packet: None,
                failed: None,
            }),
            Some(&bank),
        )?;
        let init = PreparedBlocking::prepare(&bank, || {
            let control = control.clone();
            move || {
                run_init(&control, allocate);
                Ok(())
            }
        })?;
        let read = PreparedBlocking::prepare(&bank, || {
            let control = control.clone();
            move || {
                run_read(&control);
                Ok(())
            }
        })?;
        let write = PreparedBlocking::prepare(&bank, || {
            let control = control.clone();
            move || {
                run_write(&control);
                Ok(())
            }
        })?;
        let undo_call = PreparedBlocking::prepare(&bank, || {
            let control = control.clone();
            move || {
                run_undo(&control);
                Ok(())
            }
        })?;
        let prepared_bridge =
            PreparedBridge::prepare(control.clone(), work.clone(), state.clone())?;
        let launch = RuntimeRoot::new(
            Mutex::new(Launch {
                calls: Some(PreparedCalls { init, read, write }),
                undo: Some(undo_call),
                bridge: Some(prepared_bridge),
                started: false,
            }),
            Some(&bank),
        )?;
        // Owner/F admission is complete before either bind. Linux's exact
        // observed response backing is admitted while inert, before exposure.
        // The caller must
        // synchronously install this single-attempt owner in its original handle.
        control
            .hub
            .bind(&control.native, Phase::Native)
            .map_err(|_| protocol("native phase already bound"))?;
        control
            .hub
            .bind(&control.undo, Phase::Undo)
            .map_err(|_| protocol("undo phase already bound"))?;
        control.native.prepare_launch();
        control.undo.prepare_launch();
        Ok(Self {
            control,
            work,
            bridge,
            undo: undo_slot,
            launch,
            state,
        })
    }

    pub(crate) fn native_ledger(&self) -> &Ledger {
        &self.control.native
    }
    pub(crate) fn undo_ledger(&self) -> &Ledger {
        &self.control.undo
    }
    pub(crate) fn work_result(&self) -> Result<(), TunnelError> {
        self.control.work_result()
    }

    pub(crate) async fn activate(&self, lease: PortLease) -> Result<(), TunnelError> {
        let mut stop = StopOnDrop::new(self.control.clone());
        if self.control.stopped.load(Ordering::Acquire) {
            self.work_result()?;
            return Err(cancelled());
        }
        {
            let mut state = self.state.lock().await;
            if state.port.is_some() {
                return Err(protocol("one actual Kernel port handoff"));
            }
            state.port = Some(lease);
        }
        let (calls, bridge) = {
            let mut launch = self.launch.lock().unwrap_or_else(|p| p.into_inner());
            if launch.started || self.control.stopped.load(Ordering::Acquire) {
                drop(launch);
                self.work_result()?;
                return Err(cancelled());
            }
            launch.started = true;
            (
                launch.calls.take().expect("actual prepared native calls"),
                launch.bridge.take().expect("actual prepaid Unix bridge F"),
            )
        };
        {
            // Spawning and finishing this real batch can wake arbitrary callers.
            // The launch mutex protects detachment only, never those callbacks.
            let mut batch = Batch {
                init: Some(calls.init),
                read: Some(calls.read),
                write: Some(calls.write),
                _finish: FinishLaunch(self.control.clone()),
            };
            self.bridge
                .install(bridge.spawn(self.control.native.clone()));
            batch.read.take().expect("read call").start(
                &self.work.read,
                ParentRole::Writer,
                self.control.native.clone(),
            );
            batch.write.take().expect("write call").start(
                &self.work.write,
                ParentRole::Smoltcp,
                self.control.native.clone(),
            );
            batch.init.take().expect("init call").start(
                &self.work.init,
                ParentRole::ForwardOrReader,
                self.control.native.clone(),
            );
            drop(batch);
        }
        // Init's actual JoinError must be acquired before accepting a proof
        // stored just before a panicking completion notification.
        self.work.init.join().await;
        if self.control.stopped.load(Ordering::Acquire) {
            self.join_work().await;
            self.work_result()?;
            return Err(cancelled());
        }
        self.work_result()?;
        if !self.ready() || !self.is_alive() {
            self.request_stop();
            self.join_work().await;
            self.work_result()?;
            return Err(cancelled());
        }
        stop.armed = false;
        Ok(())
    }

    pub(crate) fn request_stop(&self) {
        self.control.stop();
        let calls = {
            let mut launch = self.launch.lock().unwrap_or_else(|p| p.into_inner());
            if launch.started {
                None
            } else {
                let calls = launch.calls.take();
                let bridge = launch.bridge.take();
                if calls.is_none() && bridge.is_none() {
                    None
                } else {
                    Some((calls, bridge))
                }
            }
        };
        if let Some(calls) = calls {
            // Actual unspawned Fs die before publishing the settled fence.
            drop(calls);
            finish_launch(&self.control);
        }
    }
    pub(crate) async fn join_work(&self) {
        self.request_stop();
        loop {
            let changed = self.control.changed.notified();
            tokio::pin!(changed);
            changed.as_mut().enable();
            if self.control.launch_finished.load(Ordering::Acquire) {
                break;
            }
            changed.await;
        }
        self.work.init.join().await;
        self.work.read.join().await;
        self.work.write.join().await;
        self.bridge.join().await;
        self.control.work_joined.store(true, Ordering::Release);
    }
    pub(crate) async fn undo_at(&self, deadline: Instant) {
        self.join_work().await;
        if Instant::now() >= deadline {
            return;
        }
        self.control.cleanup_clock.set(deadline);
        let call = self
            .launch
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .undo
            .take();
        if let Some(call) = call {
            let _finished = FinishUndo(self.control.clone());
            call.start(&self.undo, ParentRole::Smoltcp, self.control.undo.clone());
        }
        // Another collector may have detached the same prepaid call and not
        // installed its actual JoinHandle yet. Wait for that real launch fence.
        loop {
            let changed = self.control.changed.notified();
            tokio::pin!(changed);
            changed.as_mut().enable();
            if !self.control.undo.is_launch_pending() {
                break;
            }
            changed.await;
        }
        self.undo.join().await;
        if !self.control.undo_executed.load(Ordering::Acquire)
            || self
                .control
                .device
                .read()
                .unwrap_or_else(|p| p.into_inner())
                .is_some()
        {
            self.control.capture_undo(
                2,
                NativeError::Conflict(
                    "native cleanup body did not complete or retained its original resource",
                ),
            );
        }
        self.control.undo_joined.store(true, Ordering::Release);
    }
    pub(crate) fn is_terminal(&self) -> bool {
        self.control.work_joined.load(Ordering::Acquire)
            && self.control.undo_joined.load(Ordering::Acquire)
            && self.control.undo_executed.load(Ordering::Acquire)
            && self
                .control
                .device
                .read()
                .unwrap_or_else(|p| p.into_inner())
                .as_ref()
                .map_or(true, |d| !d.has_pending_response())
            && self
                .control
                .device
                .read()
                .unwrap_or_else(|p| p.into_inner())
                .is_none()
            && self
                .control
                .packet
                .read()
                .unwrap_or_else(|p| p.into_inner())
                .is_none()
    }
    pub(crate) fn release_flows(&self) {
        if !self.is_terminal() {
            return;
        }
        let mut state = match self.state.try_lock() {
            Ok(state) => state,
            Err(_) => return,
        };
        let detached = (
            state.port.take(),
            state.filter.take(),
            state.to_host.take(),
            state.to_device.take(),
            state.ready_packet.take(),
            state.failed.take(),
        );
        drop(state);
        drop(detached);
        self.control.to_bridge.release();
        self.control.to_native.release();
        let read = self
            .control
            .read_current
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .take();
        let write = self
            .control
            .write_current
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .take();
        drop((read, write));
        let flow = self
            .control
            .flow
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .take();
        drop(flow);
    }
    pub(crate) fn ready(&self) -> bool {
        if self.control.stopped.load(Ordering::Acquire) || self.control.work_result().is_err() {
            return false;
        }
        let proof = *self.control.proof.lock().unwrap_or_else(|p| p.into_inner());
        let Some(proof) = proof else {
            return false;
        };
        let guard = self
            .control
            .device
            .read()
            .unwrap_or_else(|p| p.into_inner());
        guard.as_ref().is_some_and(|device| {
            device.owns(proof.identity, proof.address, proof.route, proof.mtu)
        })
    }
    pub(crate) fn is_alive(&self) -> bool {
        self.ready()
            && self.work.read.installed()
            && !self.work.read.is_finished()
            && self.work.write.installed()
            && !self.work.write.is_finished()
            && self.bridge.0.get().is_some()
            && !self.bridge.is_finished()
    }
}
impl Drop for KernelRuntime {
    fn drop(&mut self) {
        self.request_stop();
    }
}
struct FinishLaunch(ControlRoot);
impl Drop for FinishLaunch {
    fn drop(&mut self) {
        finish_launch(&self.0);
    }
}
struct Batch {
    init: Option<PreparedBlocking>,
    read: Option<PreparedBlocking>,
    write: Option<PreparedBlocking>,
    _finish: FinishLaunch,
}
fn finish_launch(control: &Control) {
    control.native.finish_launch();
    control.launch_finished.store(true, Ordering::Release);
    if !std::thread::panicking() {
        control.changed.notify_waiters();
    }
}
struct FinishUndo(ControlRoot);
impl Drop for FinishUndo {
    fn drop(&mut self) {
        self.0.undo.finish_launch();
        if !std::thread::panicking() {
            self.0.changed.notify_waiters();
        }
    }
}

fn run_init<F>(control: &Control, allocate: F)
where
    F: FnOnce(Plan) -> Result<Device, NativeError>,
{
    let result = init_inner(control, allocate);
    match result {
        Ok(Some(proof)) => {
            *control.proof.lock().unwrap_or_else(|p| p.into_inner()) = Some(proof);
            control.changed.notify_waiters();
        }
        _ => control.stop(),
    }
}
fn init_inner<F>(control: &Control, allocate: F) -> Result<Option<NativeReadyProof>, ()>
where
    F: FnOnce(Plan) -> Result<Device, NativeError>,
{
    if !control.checkpoint() {
        return Ok(None);
    }
    let (address, route, inert_identity) = {
        let mut owner = control.device.write().unwrap_or_else(|p| p.into_inner());
        if owner.is_some() {
            control.capture(
                0,
                NativeError::Conflict("native interface already allocated"),
            );
            return Err(());
        }
        let actual = control.native(0, allocate(control.plan))?;
        *owner = Some(actual); // Original FD durable BEFORE any resolve/configuration.
        let device = owner
            .as_mut()
            .expect("stored genuine original native resource");
        if !control.checkpoint() {
            return Ok(None);
        }
        let inert_identity =
            control.native(0, device.resolve_identity(control.deadline.into_std()))?;
        control.configure(device, Device::make_inert)?;
        control.configure(device, Device::inspect_inert)?;
        control.configure(device, Device::configure_mtu)?;
        if !control.checkpoint() {
            return Ok(None);
        }
        let packet = control.native(0, device.prepare_packet_io(control.deadline.into_std()))?;
        *control.packet.write().unwrap_or_else(|p| p.into_inner()) = Some(packet);
        control.configure(device, Device::start_session)?;
        // The first address call may succeed partially before returning. From
        // this exact point no response growth/quota admission is permitted.
        control.exposed.store(true, Ordering::Release);
        let address = control.configure(device, Device::add_address)?;
        let route = control.configure(device, Device::add_route)?;
        (address, route, inert_identity)
    };
    control.gate.open();
    let handle = tokio::runtime::Handle::current();
    loop {
        if !control.checkpoint() {
            return Ok(None);
        }
        if control.gate.all_started() {
            // Packet jobs only borrow the independent actual PacketIo alias.
            // This mutable control lock never waits behind their blocking IO.
            let mut owner = control.device.write().unwrap_or_else(|p| p.into_inner());
            let device = owner
                .as_mut()
                .expect("original native owner before readiness");
            let identity =
                control.native(0, device.resolve_identity(control.deadline.into_std()))?;
            if identity != inert_identity {
                control.capture(
                    0,
                    NativeError::Conflict("ready identity differs from original inert interface"),
                );
                return Err(());
            }
            let mtu = control.configure(device, Device::inspect_mtu)?;
            control.configure(device, |device, response, deadline| {
                device.inspect_route(route, response, deadline)
            })?;
            match control.configure(device, |device, response, deadline| {
                device.address_ready(address, response, deadline)
            })? {
                AddressReadiness::Preferred => {
                    if !control.checkpoint() {
                        return Ok(None);
                    }
                    return Ok(Some(NativeReadyProof {
                        identity,
                        address,
                        route,
                        mtu,
                    }));
                }
                AddressReadiness::Tentative => {}
                AddressReadiness::Conflict => {
                    control.capture(
                        0,
                        NativeError::Conflict(
                            "native address readback reports DAD/conflicting state",
                        ),
                    );
                    return Err(());
                }
            }
        }
        handle.block_on(async {
            tokio::select! {
                _ = tokio::time::sleep_until((Instant::now() + Duration::from_millis(10)).min(control.deadline)) => {},
                _ = control.stopped() => {},
            }
        });
    }
}
fn run_read(control: &Control) {
    if !control.gate.start_packet(true) {
        return;
    }
    loop {
        if control.stopped.load(Ordering::Acquire) {
            return;
        }
        let mut current = control
            .read_current
            .lock()
            .unwrap_or_else(|p| p.into_inner());
        if current.is_none() {
            match OwnedBuffer::zeroed(usize::from(control.plan.mtu) + 1, Some(&control.bank)) {
                Ok(buffer) => *current = Some(buffer),
                Err(error) => {
                    control.capture(1, error);
                    drop(current);
                    control.stop();
                    return;
                }
            }
        }
        let read = {
            let device = control.packet.read().unwrap_or_else(|p| p.into_inner());
            device
                .as_ref()
                .expect("open packet gate owns interface")
                .recv(
                    current.as_mut().expect("paid receive destination"),
                    control
                        .stop_event
                        .get()
                        .expect("prepared native stop event"),
                )
        };
        let n = match read {
            Ok(n) => n,
            Err(error)
                if platform::is_interrupted(&error) && control.stopped.load(Ordering::Acquire) =>
            {
                return
            }
            Err(error) => {
                control.capture(1, error);
                drop(current);
                control.stop();
                return;
            }
        };
        if n > usize::from(control.plan.mtu) {
            control.capture(1, protocol("native packet exceeds authenticated MTU"));
            drop(current);
            control.stop();
            return;
        }
        current
            .as_mut()
            .expect("original receive buffer")
            .truncate(n);
        let packet = current.take().expect("completed native packet");
        drop(current);
        if let Err(packet) = control.to_bridge.put_blocking(packet) {
            *control
                .read_current
                .lock()
                .unwrap_or_else(|p| p.into_inner()) = Some(packet);
            return;
        }
    }
}
fn run_write(control: &Control) {
    if !control.gate.start_packet(false) {
        return;
    }
    while let Some(packet) = control.to_native.take_blocking() {
        *control
            .write_current
            .lock()
            .unwrap_or_else(|p| p.into_inner()) = Some(packet);
        if control.stopped.load(Ordering::Acquire) {
            return;
        }
        let mut current = control
            .write_current
            .lock()
            .unwrap_or_else(|p| p.into_inner());
        let packet = current.as_ref().expect("same current native write packet");
        let result = {
            let device = control.packet.read().unwrap_or_else(|p| p.into_inner());
            device
                .as_ref()
                .expect("open packet gate owns interface")
                .send(
                    packet,
                    control
                        .stop_event
                        .get()
                        .expect("prepared native stop event"),
                )
        };
        match result {
            Ok(n) if n == packet.len() => {
                drop(current.take());
            }
            Ok(_) => {
                control.capture(
                    2,
                    io::Error::new(io::ErrorKind::WriteZero, "short whole native packet write"),
                );
                drop(current);
                control.stop();
                return;
            }
            Err(error)
                if platform::is_interrupted(&error) && control.stopped.load(Ordering::Acquire) =>
            {
                return
            }
            Err(error) => {
                control.capture(2, error);
                drop(current);
                control.stop();
                return;
            }
        }
    }
}

async fn run_bridge(
    control: ControlRoot,
    work: RuntimeRoot<WorkSlots>,
    state: BridgeRoot,
) -> Result<(), TunnelError> {
    let _stop = StopOnDrop::new(control.clone());
    control.gate.bridge_started();
    loop {
        let changed = control.changed.notified();
        tokio::pin!(changed);
        changed.as_mut().enable();
        if control.launch_finished.load(Ordering::Acquire) {
            break;
        }
        if control.stopped.load(Ordering::Acquire) {
            return Ok(());
        }
        changed.await;
    }
    let mut state = state.lock().await;
    let mut init_collected = false;
    loop {
        if control.stopped.load(Ordering::Acquire) {
            return Ok(());
        }
        if control.original.is_runtime_stopped() {
            control.stop();
            return Ok(());
        }
        if state.ready_packet.is_none() {
            state.ready_packet = state
                .filter
                .as_mut()
                .expect("original private filter")
                .pop_ready();
        }
        if let Some((direction, _)) = state.ready_packet.as_ref() {
            let free = match direction {
                Direction::ToHost => state.to_host.is_none(),
                Direction::ToDevice => state.to_device.is_none(),
            };
            if free {
                let (direction, packet) = state
                    .ready_packet
                    .take()
                    .expect("observed original filtered packet");
                match direction {
                    Direction::ToHost => state.to_host = Some(packet),
                    Direction::ToDevice => state.to_device = Some(packet),
                }
            }
        }
        let expiry = state
            .filter
            .as_ref()
            .expect("original private filter")
            .next_expiry();
        let send_host = state.to_host.is_some();
        let send_device = state.to_device.is_some();
        let BridgeState {
            port,
            filter,
            to_host,
            to_device,
            failed,
            ..
        } = &mut *state;
        let port = port.as_mut().expect("same original port lease").port();
        tokio::select! {
            biased;
            _ = control.stopped() => return Ok(()),
            _ = control.transport.get(0).expect("original persistent reader").join() => {
                control.stop(); return Ok(());
            },
            _ = control.transport.get(1).expect("original persistent writer").join() => {
                control.stop(); return Ok(());
            },
            _ = work.init.join(), if !init_collected => {
                init_collected = true;
                if control.stopped.load(Ordering::Acquire) { return Ok(()); }
            },
            _ = work.read.join() => { control.stop(); return Ok(()); },
            _ = work.write.join() => { control.stop(); return Ok(()); },
            result = control.to_native.send_from(to_host), if send_host => {
                if result.is_err() { work.write.join().await; control.stop(); return Ok(()); }
            },
            result = port.outgoing.send_from(to_device), if send_device => {
                if result.is_err() {
                    control.transport.get(1).expect("original writer").join().await;
                    control.stop(); return Ok(());
                }
            },
            packet = port.incoming.recv(), if !send_host => {
                match packet {
                    Some(packet) => ingest(&control, filter.as_mut().expect("same private filter"), Direction::ToHost, packet, to_host, failed),
                    None => {
                        control.transport.get(0).expect("original reader").join().await;
                        control.stop(); return Ok(());
                    }
                }
            },
            packet = control.to_bridge.recv(), if !send_device => {
                match packet {
                    Some(packet) => ingest(&control, filter.as_mut().expect("same private filter"), Direction::ToDevice, packet, to_device, failed),
                    None => { work.read.join().await; control.stop(); return Ok(()); }
                }
            },
            _ = async { if let Some(at) = expiry { tokio::time::sleep_until(at).await } else { pending::<()>().await } } => {
                filter.as_mut().expect("same private filter").expire();
            },
        }
    }
}
fn ingest(
    control: &Control,
    filter: &mut PrivateFilter,
    direction: Direction,
    packet: OwnedBuffer,
    pending: &mut Option<OwnedBuffer>,
    failed: &mut Option<OwnedBuffer>,
) {
    match filter.ingest(direction, packet) {
        Ok(FilterOutput::Forward(packet)) => *pending = Some(packet),
        Ok(FilterOutput::Pending | FilterOutput::Ready) => {}
        Ok(FilterOutput::Discard(reason)) => {
            tracing::trace!(?reason, "native packet excluded by bridge filter")
        }
        Err(error) => {
            *failed = Some(error.packet);
            control.capture(3, error.error);
            control.stop();
        }
    }
}

fn run_undo(control: &Control) {
    #[cfg(test)]
    {
        let callback = control.before_undo.lock().unwrap().take();
        if let Some(callback) = callback {
            callback();
        }
    }
    // This body runs only after the containing collector observed every
    // original/native work join. The same actual packet descriptor alias dies
    // before the original control owner is allowed to close its native fd.
    let packet = control
        .packet
        .write()
        .unwrap_or_else(|p| p.into_inner())
        .take();
    drop(packet);
    loop {
        let deadline = control.cleanup_clock.live();
        let mut owner = control.device.write().unwrap_or_else(|p| p.into_inner());
        if owner.is_none() {
            break;
        }
        let outcome = {
            let mut response = control.response.lock().unwrap_or_else(|p| p.into_inner());
            owner
                .as_mut()
                .expect("retained same native control owner")
                .undo_step(&mut response, deadline.into_std())
        };
        match outcome {
            Ok(true) => {
                let actual = owner
                    .take()
                    .expect("same genuine native owner before close");
                match actual.close() {
                    Ok(()) => {
                        drop(owner);
                        break;
                    }
                    Err((actual, error)) => {
                        // Restore physical ownership BEFORE any arbitrary
                        // notification/panic from capturing cleanup evidence.
                        *owner = Some(actual);
                        drop(owner);
                        control.capture_undo(2, error);
                    }
                }
            }
            Ok(false) => {
                drop(owner);
            }
            Err((role, error)) => {
                drop(owner);
                // A native clock check cannot invent a syscall or Elapsed.
                // The outer same-clock collector owns that real Tokio timer.
                if !platform::is_deadline(&error) {
                    control.capture_undo(role, error);
                }
            }
        }
        // ResponseStorage keeps the original pending ACK/datagram and owner.
        // No quota, buffer growth or newly allocated cleanup call happens here.
        control.cleanup_clock.retry(deadline);
    }
    control
        .proof
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .take();
    control.undo_executed.store(true, Ordering::Release);
}

fn protocol(message: &'static str) -> TunnelError {
    TunnelError::Protocol(message.into())
}
fn cancelled() -> TunnelError {
    io::Error::other(crate::OperationCancelled::new()).into()
}

#[cfg(test)]
mod tests;
