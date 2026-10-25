//! Same-owner Windows kernel phase over the original packet port.
//! No transport split, hidden native task, or default device factory in tests.
use std::{
    error::Error,
    future::pending,
    io,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Condvar, Mutex, OnceLock, RwLock,
    },
    time::Duration,
};
use tokio::{
    sync::{Mutex as AsyncMutex, Notify, Semaphore},
    time::Instant,
};

use super::{
    backend::PortLease,
    native::windows::{
        self, AddressReadiness, AddressReceipt, DerivedRouteObservation, Identity, InterruptEvent,
        NativeError, RouteReceipt, WindowsDevice,
    },
    native_jobs::{BlockingSlot, PreparedBlocking},
    owned_buffer::OwnedBuffer,
    private_connect::{Direction, FilterOutput, FlowLease, PrivateConnector, PrivateFilter},
    runtime_storage::{RuntimeRoot, TaskHandles},
    worker_future::PreparedWorker,
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

#[derive(Clone, Copy)]
// Both real receipt variants stay inline in the already paid Control root.
// Boxing the derived readback would introduce a late native-result allocation.
#[allow(clippy::large_enum_variant)]
enum RouteProof {
    Explicit(RouteReceipt),
    Derived(DerivedRouteObservation),
}
/// Sealed: only init's actual successful native observations construct this.
#[derive(Clone, Copy)]
struct NativeReadyProof {
    identity: Identity,
    address: AddressReceipt,
    route: RouteProof,
    mtu: u16,
}
struct Control {
    bank: Arc<Semaphore>,
    plan: windows::Plan,
    deadline: Instant,
    original: Ledger,
    native: Ledger,
    undo: Ledger,
    hub: PhaseOutcomes,
    transport: TaskHandles,
    transport_stop: super::watch::Sender,
    connector: PrivateConnector,
    device: RwLock<Option<WindowsDevice>>,
    stop_event: OnceLock<InterruptEvent>,
    gate: PacketGate,
    stopped: AtomicBool,
    changed: Notify,
    launch_finished: AtomicBool,
    work_joined: AtomicBool,
    undo_joined: AtomicBool,
    undo_executed: AtomicBool,
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
        result.map_err(|error| self.capture(role, error))
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
    started: bool,
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
        plan: windows::Plan,
        original_deadline: Instant,
    ) -> Result<Self, TunnelError> {
        Self::prepare_with(
            engine,
            bank,
            plan,
            original_deadline,
            WindowsDevice::allocate,
        )
    }
    fn prepare_with<F>(
        engine: &UserspaceTunDevice,
        bank: Arc<Semaphore>,
        plan: windows::Plan,
        deadline: Instant,
        allocate: F,
    ) -> Result<Self, TunnelError>
    where
        F: FnOnce(windows::Plan) -> Result<WindowsDevice, NativeError> + Send + 'static,
    {
        Self::prepare_with_event(engine, bank, plan, deadline, allocate, InterruptEvent::new)
    }
    fn prepare_with_event<F, E>(
        engine: &UserspaceTunDevice,
        bank: Arc<Semaphore>,
        plan: windows::Plan,
        deadline: Instant,
        allocate: F,
        event: E,
    ) -> Result<Self, TunnelError>
    where
        F: FnOnce(windows::Plan) -> Result<WindowsDevice, NativeError> + Send + 'static,
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
                stop_event: OnceLock::new(),
                gate: PacketGate::new(),
                stopped: AtomicBool::new(false),
                changed: Notify::new(),
                launch_finished: AtomicBool::new(false),
                work_joined: AtomicBool::new(false),
                undo_joined: AtomicBool::new(false),
                undo_executed: AtomicBool::new(false),
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
        let launch = RuntimeRoot::new(
            Mutex::new(Launch {
                calls: Some(PreparedCalls { init, read, write }),
                undo: Some(undo_call),
                started: false,
            }),
            Some(&bank),
        )?;
        // All fallible admission is over before either bind. The caller must
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

    #[cfg(test)]
    pub(crate) fn prepare_scripted<F>(
        engine: &UserspaceTunDevice,
        bank: Arc<Semaphore>,
        plan: windows::Plan,
        deadline: Instant,
        allocate: F,
    ) -> Result<Self, TunnelError>
    where
        F: FnOnce(windows::Plan) -> Result<WindowsDevice, NativeError> + Send + 'static,
    {
        Self::prepare_with(engine, bank, plan, deadline, allocate)
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
        let future = run_bridge(self.control.clone(), self.work.clone(), self.state.clone());
        let bridge = match PreparedWorker::prepare(future, Some(&self.control.bank)) {
            Ok(bridge) => bridge,
            Err(error) => {
                self.request_stop();
                return Err(error);
            }
        };
        let calls = {
            let mut launch = self.launch.lock().unwrap_or_else(|p| p.into_inner());
            if launch.started || self.control.stopped.load(Ordering::Acquire) {
                drop(launch);
                self.work_result()?;
                return Err(cancelled());
            }
            launch.started = true;
            launch.calls.take().expect("actual prepared native calls")
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
                .install(bridge.spawn(ParentRole::Listener, self.control.native.clone()));
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
                launch.calls.take()
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
    pub(crate) async fn undo(&self) {
        self.join_work().await;
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
            && self
                .control
                .device
                .read()
                .unwrap_or_else(|p| p.into_inner())
                .is_some()
        {
            self.control
                .capture_undo(2, io::Error::other("native undo call did not execute"));
        }
        self.control.undo_joined.store(true, Ordering::Release);
    }
    pub(crate) fn release_flows(&self) {
        if !self.control.work_joined.load(Ordering::Acquire)
            || !self.control.undo_joined.load(Ordering::Acquire)
            || self
                .control
                .device
                .read()
                .unwrap_or_else(|p| p.into_inner())
                .is_some()
        {
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
            device.owns_ready_address(proof.identity, &proof.address, proof.mtu)
                && match proof.route {
                    RouteProof::Explicit(receipt) => device.owns_ready_route(&receipt),
                    RouteProof::Derived(observed) => device.owns_ready_derived(&observed),
                }
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
    F: FnOnce(windows::Plan) -> Result<WindowsDevice, NativeError>,
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
    F: FnOnce(windows::Plan) -> Result<WindowsDevice, NativeError>,
{
    if !control.checkpoint() {
        return Ok(None);
    }
    let (address, route, inert_identity) = {
        let mut guard = control.device.write().unwrap_or_else(|p| p.into_inner());
        if guard.is_some() {
            control.capture(0, io::Error::other("native interface already allocated"));
            return Err(());
        }
        let device = control.native(0, allocate(control.plan))?;
        *guard = Some(device); // Genuine resource persists BEFORE conversion.
        let device = guard.as_mut().expect("stored fresh native interface");
        if !control.checkpoint() {
            return Ok(None);
        }
        control.native(0, device.resolve_identity())?;
        if !control.checkpoint() {
            return Ok(None);
        }
        let inert_identity = control.native(0, device.inspect_inert())?.identity;
        if !control.checkpoint() {
            return Ok(None);
        }
        control.native(0, device.configure_mtu())?;
        if !control.checkpoint() {
            return Ok(None);
        }
        control.native(0, device.start_session(0x20_0000))?;
        if !control.checkpoint() {
            return Ok(None);
        }
        let address = control.native(0, device.add_address())?;
        if !control.checkpoint() {
            return Ok(None);
        }
        let route = match device.add_route() {
            Ok(route) => RouteProof::Explicit(route),
            Err(NativeError::Call(error))
                if error.status()
                    == windows_sys::Win32::Foundation::ERROR_OBJECT_ALREADY_EXISTS =>
            {
                RouteProof::Derived(control.native(0, device.inspect_address_route(&address))?)
            }
            Err(error) => {
                control.capture(0, error);
                return Err(());
            }
        };
        (address, route, inert_identity)
    };
    control.gate.open(); // Never wait on write lock behind native recv.
    let handle = tokio::runtime::Handle::current();
    loop {
        if !control.checkpoint() {
            return Ok(None);
        }
        if control.gate.all_started() {
            let guard = control.device.read().unwrap_or_else(|p| p.into_inner());
            let device = guard.as_ref().expect("original native interface");
            let identity = control.native(0, device.inspect_identity())?;
            if !identity.same(inert_identity) {
                control.capture(
                    0,
                    NativeError::Conflict("ready identity differs from original inert interface"),
                );
                return Err(());
            }
            control.native(0, device.inspect_mtu())?;
            match route {
                RouteProof::Explicit(receipt) => {
                    control.native(0, device.inspect_route(&receipt))?
                }
                RouteProof::Derived(observation) => {
                    control.native(0, device.inspect_derived_route(&observation))?
                }
            }
            // This last real readback may regress after an earlier Preferred
            // observation. Only its actual Preferred state seals a proof.
            match control.native(0, device.address_ready(&address))? {
                AddressReadiness::Preferred => {
                    if !control.checkpoint() {
                        return Ok(None);
                    }
                    return Ok(Some(NativeReadyProof {
                        identity,
                        address,
                        route,
                        mtu: control.plan.mtu,
                    }));
                }
                AddressReadiness::Tentative => {}
                AddressReadiness::Conflict { dad_state } => {
                    control.capture(0, NativeError::DadState(dad_state));
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
            let device = control.device.read().unwrap_or_else(|p| p.into_inner());
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
            Err(NativeError::Interrupted) if control.stopped.load(Ordering::Acquire) => return,
            Err(error) => {
                control.capture(1, error);
                drop(current);
                control.stop();
                return;
            }
        };
        if n > usize::from(control.plan.mtu) {
            control.capture(
                1,
                NativeError::PacketTooLarge {
                    length: n,
                    capacity: usize::from(control.plan.mtu),
                },
            );
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
            let device = control.device.read().unwrap_or_else(|p| p.into_inner());
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
            Err(NativeError::Interrupted) if control.stopped.load(Ordering::Acquire) => return,
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
    control.undo_executed.store(true, Ordering::Release);
    let mut guard = control.device.write().unwrap_or_else(|p| p.into_inner());
    if let Some(mut device) = guard.take() {
        if let Some(route) = device.route_receipt() {
            if let Err(error) = device.remove_route(&route) {
                control.capture_undo(0, error);
            }
        }
        if let Some(address) = device.address_receipt() {
            if let Err(error) = device.remove_address(&address) {
                control.capture_undo(1, error);
            }
        }
        device.end_session();
        device.close();
    }
    control
        .proof
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .take();
}
fn protocol(message: &'static str) -> TunnelError {
    TunnelError::Protocol(message.into())
}
fn cancelled() -> TunnelError {
    io::Error::other(crate::OperationCancelled::new()).into()
}

#[cfg(test)]
mod tests;
