use super::*;
use std::{
    future::Future,
    marker::PhantomPinned,
    pin::Pin,
    task::{Context, Poll},
};

#[repr(align(64))]
struct Signal {
    bank: Arc<Semaphore>,
    polls: Arc<AtomicUsize>,
    drops: Arc<AtomicUsize>,
    address: Arc<AtomicUsize>,
    range: Arc<StdMutex<(usize, usize)>>,
    drop_at_zero: bool,
    data: [u8; 103],
    _pin: PhantomPinned,
}
impl Future for Signal {
    type Output = std::io::Result<()>;
    fn poll(self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<Self::Output> {
        let this = self.as_ref().get_ref();
        let address = this as *const _ as usize;
        let previous =
            this.address
                .compare_exchange(0, address, Ordering::SeqCst, Ordering::SeqCst);
        if let Err(previous) = previous {
            assert_eq!(address, previous);
        }
        let (base, bytes) = *this.range.lock().unwrap();
        assert!(address >= base && address + std::mem::size_of::<Self>() <= base + bytes);
        this.polls.fetch_add(1, Ordering::SeqCst);
        Poll::Pending
    }
}
impl Drop for Signal {
    fn drop(&mut self) {
        let address = self.address.load(Ordering::SeqCst);
        if address != 0 {
            assert_eq!(self as *const _ as usize, address);
        }
        assert_eq!(self.data, [42; 103]);
        if self.drop_at_zero {
            assert_eq!(
                self.bank.available_permits(),
                0,
                "signal ends before original F fee returns"
            );
        }
        self.drops.fetch_add(1, Ordering::SeqCst);
    }
}
fn signal(bank: &Arc<Semaphore>, drop_at_zero: bool) -> Signal {
    Signal {
        bank: bank.clone(),
        polls: Arc::new(AtomicUsize::new(0)),
        drops: Arc::new(AtomicUsize::new(0)),
        address: Arc::new(AtomicUsize::new(0)),
        range: Arc::new(StdMutex::new((0, 0))),
        drop_at_zero,
        data: [42; 103],
        _pin: PhantomPinned,
    }
}
fn no_source(source: &FakeSource) {
    assert_eq!(source.usb_calls.load(Ordering::SeqCst), 0);
    assert_eq!(source.paired_calls.load(Ordering::SeqCst), 0);
    assert!(source.calls.lock().unwrap().is_empty());
}
async fn listener() -> (TcpListener, u16) {
    let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
    let port = listener.local_addr().unwrap().port();
    (listener, port)
}

#[tokio::test]
async fn server_frame_short_or_closed_preparation_never_allocates_polls_or_starts_owner() {
    for closed in [false, true] {
        let source = FakeSource::new();
        let state = state(source.clone(), LOCAL_BUDGET);
        let bank = state.inner.global_metadata_bytes.clone();
        let signal = signal(&bank, false);
        let polls = signal.polls.clone();
        let drops = signal.drops.clone();
        let leave = if closed {
            0
        } else {
            server::frame_bytes(&signal) - 1
        };
        let held = bank
            .clone()
            .try_acquire_many_owned((bank.available_permits() - leave) as u32)
            .unwrap();
        if closed {
            bank.close();
        }
        let (listener, port) = listener().await;
        let hook = metadata_vec::on_allocation(|_, _| panic!("refused server allocation"));
        let error = server::prepare(
            listener,
            agent_router(state.clone()),
            state.clone(),
            LOCAL_BUDGET,
            LOCAL_BUDGET,
            signal,
        )
        .err()
        .unwrap();
        drop(hook);
        assert_eq!(
            crate::output::operation_error_class(error.as_ref()),
            ios_core::ErrorClass::ResourceLimit
        );
        assert_eq!(polls.load(Ordering::SeqCst), 0);
        assert_eq!(drops.load(Ordering::SeqCst), 1);
        assert!(!*state.inner.shutdown.borrow());
        assert!(state.inner.lifecycle.global_epoch().is_ok());
        no_source(&source);
        assert!(TcpStream::connect(("127.0.0.1", port)).await.is_err());
        assert_eq!(bank.available_permits(), leave);
        drop(error);
        drop(held);
        drop(state);
        assert_eq!(bank.available_permits(), MAX_GLOBAL_KNOWN_METADATA_BYTES);
    }
}

#[tokio::test]
async fn server_frame_exact_unpolled_original_signal_ends_before_fee_without_starting_owner() {
    let source = FakeSource::new();
    let state = state(source.clone(), LOCAL_BUDGET);
    let bank = state.inner.global_metadata_bytes.clone();
    let signal = signal(&bank, true);
    let polls = signal.polls.clone();
    let drops = signal.drops.clone();
    let frame = server::frame_bytes(&signal);
    let held = bank
        .clone()
        .try_acquire_many_owned((bank.available_permits() - frame) as u32)
        .unwrap();
    let observed_bank = bank.clone();
    let range = signal.range.clone();
    let hook = metadata_vec::on_allocation(move |layout, pointer| {
        assert_eq!(layout.size(), frame);
        assert_eq!(layout.align(), 64);
        assert_eq!(pointer % 64, 0);
        assert_eq!(observed_bank.available_permits(), 0);
        *range.lock().unwrap() = (pointer, frame);
    });
    let (listener, port) = listener().await;
    let prepared = server::prepare(
        listener,
        agent_router(state.clone()),
        state.clone(),
        LOCAL_BUDGET,
        LOCAL_BUDGET,
        signal,
    )
    .unwrap();
    drop(hook);
    bank.close();
    assert_eq!(bank.available_permits(), 0);
    drop(prepared);
    assert_eq!(drops.load(Ordering::SeqCst), 1);
    assert_eq!(polls.load(Ordering::SeqCst), 0);
    assert_eq!(bank.available_permits(), frame);
    assert!(!*state.inner.shutdown.borrow());
    assert!(state.inner.lifecycle.global_epoch().is_ok());
    no_source(&source);
    assert!(TcpStream::connect(("127.0.0.1", port)).await.is_err());
    drop(held);
    drop(state);
    assert_eq!(bank.available_permits(), MAX_GLOBAL_KNOWN_METADATA_BYTES);
}

#[tokio::test]
async fn server_frame_pins_generic_signal_in_same_paid_allocation_at_closed_zero() {
    let source = FakeSource::new();
    let state = state(source.clone(), LOCAL_BUDGET);
    let bank = state.inner.global_metadata_bytes.clone();
    let signal = signal(&bank, true);
    let polls = signal.polls.clone();
    let drops = signal.drops.clone();
    let frame = server::frame_bytes(&signal);
    let held = bank
        .clone()
        .try_acquire_many_owned((bank.available_permits() - frame) as u32)
        .unwrap();
    let range = signal.range.clone();
    let hook = metadata_vec::on_allocation(move |layout, pointer| {
        assert_eq!(layout.size(), frame);
        assert_eq!(layout.align(), 64);
        *range.lock().unwrap() = (pointer, frame);
    });
    let (listener, port) = listener().await;
    let prepared = server::prepare(
        listener,
        agent_router(state.clone()),
        state.clone(),
        LOCAL_BUDGET,
        LOCAL_BUDGET,
        signal,
    )
    .unwrap();
    drop(hook);
    bank.close();
    let hook = metadata_vec::on_allocation(|_, _| panic!("late allocation at closed zero"));
    let mut running = Box::pin(prepared);
    assert!(futures_util::poll!(&mut running).is_pending());
    assert!(futures_util::poll!(&mut running).is_pending());
    assert!(polls.load(Ordering::SeqCst) >= 2);
    assert_eq!(bank.available_permits(), 0);
    no_source(&source);
    drop(running);
    drop(hook);
    assert_eq!(drops.load(Ordering::SeqCst), 1);
    assert_eq!(bank.available_permits(), frame);
    assert!(*state.inner.shutdown.borrow());
    assert!(state.inner.lifecycle.global_epoch().is_err());
    assert!(TcpStream::connect(("127.0.0.1", port)).await.is_err());
    drop(held);
    drop(state);
    tokio::time::timeout(LOCAL_BUDGET, async {
        while bank.available_permits() != MAX_GLOBAL_KNOWN_METADATA_BYTES {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn server_frame_actual_active_scanner_drop_releases_nested_source_before_parent_refund() {
    let source = FakeSource::new();
    source.usb.lock().unwrap().push(A.into());
    source.script(A, [Behavior::Pending]);
    let state = state(source.clone(), LOCAL_BUDGET);
    let bank = state.inner.global_metadata_bytes.clone();
    let before = bank.available_permits();
    let signal = std::future::pending::<std::io::Result<()>>();
    let frame = server::frame_bytes(&signal);
    let observed_bank = bank.clone();
    let first = Arc::new(AtomicBool::new(false));
    let observed = first.clone();
    let hook = metadata_vec::on_allocation(move |layout, _| {
        if !observed.swap(true, Ordering::SeqCst) {
            assert_eq!(layout.size(), frame);
            assert_eq!(observed_bank.available_permits(), before - frame);
        }
    });
    let (listener, _) = listener().await;
    let prepared = server::prepare(
        listener,
        agent_router(state.clone()),
        state.clone(),
        LOCAL_BUDGET,
        LOCAL_BUDGET,
        signal,
    )
    .unwrap();
    drop(hook);
    assert!(first.load(Ordering::SeqCst));
    let mut running = Box::pin(prepared);
    tokio::time::timeout(LOCAL_BUDGET, async {
        while source.active.load(Ordering::SeqCst) == 0 {
            assert!(futures_util::poll!(&mut running).is_pending());
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    bank.close();
    drop(running);
    assert_eq!(source.active.load(Ordering::SeqCst), 0);
    assert_eq!(source.exited.load(Ordering::SeqCst), 1);
    assert_eq!(source.paired_calls.load(Ordering::SeqCst), 1);
    assert!(state.inner.scan_lock.try_lock().is_ok());
    assert_eq!(
        bank.available_permits(),
        before - Lifecycle::initial_metadata_bytes(A)
    );
    assert!(*state.inner.shutdown.borrow());
    drop(state);
    tokio::time::timeout(LOCAL_BUDGET, async {
        while bank.available_permits() != MAX_GLOBAL_KNOWN_METADATA_BYTES {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
}

#[derive(Debug)]
struct SignalCause([u8; 3]);
impl std::fmt::Display for SignalCause {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("fixture signal IO")
    }
}
impl std::error::Error for SignalCause {}

#[tokio::test]
async fn server_frame_signal_ready_original_io_survives_closed_zero_cleanup_and_frame_end() {
    let source = FakeSource::new();
    let state = state(source.clone(), LOCAL_BUDGET);
    let bank = state.inner.global_metadata_bytes.clone();
    let held = Arc::new(StdMutex::new(None));
    let observed = held.clone();
    let observed_bank = bank.clone();
    let mut original = Some(std::io::Error::new(
        std::io::ErrorKind::PermissionDenied,
        SignalCause([1, 2, 3]),
    ));
    let pointer = original
        .as_ref()
        .unwrap()
        .get_ref()
        .unwrap()
        .downcast_ref::<SignalCause>()
        .unwrap() as *const SignalCause;
    let signal = std::future::poll_fn(move |_| {
        let permit = observed_bank
            .clone()
            .try_acquire_many_owned(observed_bank.available_permits() as u32)
            .unwrap();
        *observed.lock().unwrap() = Some(permit);
        observed_bank.close();
        Poll::Ready(Err(original.take().expect("one signal result")))
    });
    let frame = server::frame_bytes(&signal);
    let (listener, _) = listener().await;
    let prepared = server::prepare(
        listener,
        agent_router(state.clone()),
        state.clone(),
        LOCAL_BUDGET,
        LOCAL_BUDGET,
        signal,
    )
    .unwrap();
    let error = tokio::time::timeout(LOCAL_BUDGET, prepared)
        .await
        .unwrap()
        .unwrap_err();
    let io = error.downcast_ref::<std::io::Error>().unwrap();
    assert_eq!(io.kind(), std::io::ErrorKind::PermissionDenied);
    let cause = io.get_ref().unwrap().downcast_ref::<SignalCause>().unwrap();
    assert_eq!(cause as *const SignalCause, pointer);
    assert_eq!(cause.0, [1, 2, 3]);
    assert_eq!(bank.available_permits(), frame);
    no_source(&source);
    drop(error);
    drop(held.lock().unwrap().take());
    drop(state);
    assert_eq!(bank.available_permits(), MAX_GLOBAL_KNOWN_METADATA_BYTES);
}
