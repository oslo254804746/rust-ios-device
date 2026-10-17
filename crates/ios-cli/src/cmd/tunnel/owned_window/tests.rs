use super::*;
use futures_util::StreamExt;
use std::cell::Cell;
use std::marker::PhantomPinned;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Mutex;

struct Probe {
    id: usize,
    ready: Arc<AtomicUsize>,
    polls: Arc<Mutex<Vec<usize>>>,
    drops: Arc<AtomicUsize>,
    address: Cell<usize>,
    _pin: PhantomPinned,
}
impl Future for Probe {
    type Output = usize;
    fn poll(self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<usize> {
        let address = &*self as *const Self as usize;
        let old = self.address.replace(address);
        assert!(
            old == 0 || old == address,
            "original pinned scan future moved"
        );
        self.polls.lock().unwrap().push(self.id);
        if self.ready.load(Ordering::SeqCst) & (1 << self.id) != 0 {
            Poll::Ready(self.id)
        } else {
            Poll::Pending
        }
    }
}
impl Drop for Probe {
    fn drop(&mut self) {
        let address = self.address.get();
        assert!(address == 0 || address == self as *const Self as usize);
        self.drops.fetch_add(1, Ordering::SeqCst);
    }
}
fn probe(
    id: usize,
    ready: &Arc<AtomicUsize>,
    polls: &Arc<Mutex<Vec<usize>>>,
    drops: &Arc<AtomicUsize>,
) -> Probe {
    Probe {
        id,
        ready: ready.clone(),
        polls: polls.clone(),
        drops: drops.clone(),
        address: Cell::new(0),
        _pin: PhantomPinned,
    }
}
fn poll<I: ExactSizeIterator<Item = F>, F: Future>(
    window: &mut Window<I, F>,
) -> Poll<Option<Result<F::Output>>> {
    Pin::new(window).poll_next(&mut Context::from_waker(
        futures_util::task::noop_waker_ref(),
    ))
}
fn ready<I: ExactSizeIterator<Item = F>, F: Future>(window: &mut Window<I, F>) -> F::Output {
    match poll(window) {
        Poll::Ready(Some(Ok(output))) => output,
        _ => panic!("expected original ready child"),
    }
}
fn slot_bytes(count: usize) -> usize {
    count * std::mem::size_of::<Slot<Probe>>()
}

#[test]
fn scan_window_prepaid_buffer_becomes_original_pinned_future_at_zero_remaining_quota() {
    struct AddressProbe {
        original: usize,
        bank: Arc<Semaphore>,
    }
    impl Future for AddressProbe {
        type Output = usize;
        fn poll(self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<usize> {
            assert_eq!(&*self as *const Self as usize, self.original);
            assert_eq!(self.bank.available_permits(), 0);
            Poll::Ready(12)
        }
    }
    impl Drop for AddressProbe {
        fn drop(&mut self) {
            assert_eq!(self.bank.available_permits(), 0);
        }
    }
    let bank = Arc::new(Semaphore::new(8192));
    let pointer = Arc::new(AtomicUsize::new(0));
    let captured = pointer.clone();
    let factory = |_| AddressProbe {
        original: pointer.load(Ordering::SeqCst),
        bank: bank.clone(),
    };
    let prepared = super::super::metadata_vec::with_allocation_hook(
        move |layout, original| {
            if layout.size() == std::mem::size_of::<AddressProbe>() {
                captured.store(original, Ordering::SeqCst);
            }
        },
        || prepare(&bank, 1, factory).unwrap(),
    );
    assert_ne!(pointer.load(Ordering::SeqCst), 0);
    let held = bank
        .clone()
        .try_acquire_many_owned(bank.available_permits() as u32)
        .unwrap();
    let mut window = super::super::metadata_vec::with_allocation_hook(
        |_, _| panic!("prepaid frame was allocated again"),
        || Window::from_prepared(prepared, (0..1).map(factory)).unwrap(),
    );
    assert_eq!(ready(&mut window), 12);
    assert_eq!(
        bank.available_permits(),
        std::mem::size_of::<AddressProbe>()
    );
    drop(window);
    drop(held);
    assert_eq!(bank.available_permits(), 8192);
}

#[test]
fn scan_window_actual_slots_and_pinned_bodies_prepaid_with_four_concurrent_children() {
    let bank = Arc::new(Semaphore::new(8192));
    let available = Arc::new(AtomicUsize::new(0));
    let polls = Arc::new(Mutex::new(Vec::new()));
    let drops = Arc::new(AtomicUsize::new(0));
    let seen = Arc::new(Mutex::new(Vec::new()));
    let captured = seen.clone();
    let charged = bank.clone();
    let mut window = super::super::metadata_vec::with_allocation_hook(
        move |layout, pointer| {
            let mut seen = captured.lock().unwrap();
            let before: usize = seen.iter().map(|(bytes, _)| bytes).sum();
            assert_eq!(charged.available_permits(), 8192 - before - layout.size());
            seen.push((layout.size(), pointer));
        },
        || {
            Window::new(
                &bank,
                (0..6).map(|id| probe(id, &available, &polls, &drops)),
            )
            .unwrap()
        },
    );
    assert!(polls.lock().unwrap().is_empty());
    assert_eq!(seen.lock().unwrap().len(), 5);
    assert_eq!(seen.lock().unwrap()[0].0, slot_bytes(4));
    for &(bytes, _) in &seen.lock().unwrap()[1..] {
        assert_eq!(bytes, std::mem::size_of::<Probe>());
    }
    assert!(poll(&mut window).is_pending());
    assert_eq!(*polls.lock().unwrap(), [0, 1, 2, 3]);
    available.store(8, Ordering::SeqCst);
    assert_eq!(ready(&mut window), 3);
    assert_eq!(drops.load(Ordering::SeqCst), 1);
    available.store(16, Ordering::SeqCst);
    assert_eq!(
        ready(&mut window),
        4,
        "refill can finish before earlier pending inputs"
    );
    available.store(63, Ordering::SeqCst);
    let remaining = (0..4).map(|_| ready(&mut window)).collect::<Vec<_>>();
    assert_eq!(remaining, [0, 1, 2, 5]);
    assert!(matches!(poll(&mut window), Poll::Ready(None)));
    assert!(matches!(poll(&mut window), Poll::Ready(None)));
    assert_eq!(drops.load(Ordering::SeqCst), 6);
    assert_eq!(bank.available_permits(), 8192 - slot_bytes(4));
    drop(window);
    assert_eq!(bank.available_permits(), 8192);
}

#[test]
fn scan_window_slot_and_last_initial_body_refusals_drop_all_unpolled_original_inputs() {
    for (limit, allocations) in [
        (slot_bytes(4) - 1, 0),
        (slot_bytes(4) + 4 * std::mem::size_of::<Probe>() - 1, 4),
    ] {
        let bank = Arc::new(Semaphore::new(limit));
        let available = Arc::new(AtomicUsize::new(63));
        let polls = Arc::new(Mutex::new(Vec::new()));
        let drops = Arc::new(AtomicUsize::new(0));
        let futures = std::array::from_fn::<_, 6, _>(|id| probe(id, &available, &polls, &drops));
        let calls = Arc::new(AtomicUsize::new(0));
        let captured = calls.clone();
        let error = super::super::metadata_vec::with_allocation_hook(
            move |_, _| {
                captured.fetch_add(1, Ordering::SeqCst);
            },
            || Window::new(&bank, futures.into_iter()).err().unwrap(),
        );
        assert_eq!(
            crate::output::operation_error_class(error.as_ref()),
            ios_core::ErrorClass::ResourceLimit
        );
        assert_eq!(calls.load(Ordering::SeqCst), allocations);
        assert!(polls.lock().unwrap().is_empty());
        assert_eq!(drops.load(Ordering::SeqCst), 6);
        assert_eq!(bank.available_permits(), limit);
    }
}

#[test]
fn scan_window_refill_refusal_ends_original_pending_and_queued_futures_without_new_polls() {
    let total = slot_bytes(4) + 4 * std::mem::size_of::<Probe>();
    let bank = Arc::new(Semaphore::new(total));
    let available = Arc::new(AtomicUsize::new(0));
    let polls = Arc::new(Mutex::new(Vec::new()));
    let drops = Arc::new(AtomicUsize::new(0));
    let futures = std::array::from_fn::<_, 6, _>(|id| probe(id, &available, &polls, &drops));
    let mut window = Window::new(&bank, futures.into_iter()).unwrap();
    assert!(poll(&mut window).is_pending());
    available.store(1, Ordering::SeqCst);
    assert_eq!(ready(&mut window), 0);
    let before = polls.lock().unwrap().clone();
    let held = bank.clone().try_acquire_owned().unwrap();
    let result = super::super::metadata_vec::with_allocation_hook(
        |_, _| panic!("refused refill allocated"),
        || poll(&mut window),
    );
    let Poll::Ready(Some(Err(error))) = result else {
        panic!("expected exact refill refusal")
    };
    assert_eq!(
        crate::output::operation_error_class(error.as_ref()),
        ios_core::ErrorClass::ResourceLimit
    );
    assert_eq!(*polls.lock().unwrap(), before);
    assert_eq!(drops.load(Ordering::SeqCst), 6);
    assert!(matches!(poll(&mut window), Poll::Ready(None)));
    assert_eq!(bank.available_permits(), total - slot_bytes(4) - 1);
    drop(window);
    assert_eq!(bank.available_permits(), total - 1);
    drop(held);
    assert_eq!(bank.available_permits(), total);
}

#[test]
fn scan_window_caller_drop_destroys_original_pinned_children_and_unpolled_iterator() {
    let bank = Arc::new(Semaphore::new(8192));
    let available = Arc::new(AtomicUsize::new(0));
    let polls = Arc::new(Mutex::new(Vec::new()));
    let drops = Arc::new(AtomicUsize::new(0));
    let futures = std::array::from_fn::<_, 6, _>(|id| probe(id, &available, &polls, &drops));
    let mut window = Window::new(&bank, futures.into_iter()).unwrap();
    assert!(poll(&mut window).is_pending());
    drop(window);
    assert_eq!(*polls.lock().unwrap(), [0, 1, 2, 3]);
    assert_eq!(drops.load(Ordering::SeqCst), 6);
    assert_eq!(bank.available_permits(), 8192);
}

#[tokio::test]
async fn scan_window_child_error_keeps_original_pointer_and_parent_waker_processes_later_children()
{
    let bank = Arc::new(Semaphore::new(8192));
    let original = Arc::new(std::io::Error::new(
        std::io::ErrorKind::PermissionDenied,
        "original-window-error",
    ));
    let (tx, rx) = tokio::sync::oneshot::channel::<usize>();
    let mut rx = Some(rx);
    let futures = (0..2).map(|id| {
        let original = original.clone();
        let receive = if id == 1 { rx.take() } else { None };
        async move {
            if let Some(receive) = receive {
                Ok(receive.await.unwrap())
            } else {
                Err(original)
            }
        }
    });
    let mut window = Window::new(&bank, futures).unwrap();
    let error = window.next().await.unwrap().unwrap().unwrap_err();
    assert!(Arc::ptr_eq(&error, &original));
    let (next, ()) = tokio::time::timeout(std::time::Duration::from_secs(1), async {
        tokio::join!(window.next(), async {
            tokio::task::yield_now().await;
            tx.send(17).unwrap();
        })
    })
    .await
    .unwrap();
    assert_eq!(next.unwrap().unwrap().unwrap(), 17);
    assert!(window.next().await.is_none());
    drop(window);
    assert_eq!(bank.available_permits(), 8192);
}

#[tokio::test]
async fn scan_window_paid_output_outlives_collector_and_empty_input_allocates_nothing() {
    let bank = Arc::new(Semaphore::new(8192));
    let mut window = Window::new(
        &bank,
        (0..2).map(|_| async { super::super::paid_json::Text::copy("original", &bank) }),
    )
    .unwrap();
    let output = window.next().await.unwrap().unwrap().unwrap();
    drop(window);
    assert_eq!(output.as_str(), "original");
    assert_eq!(bank.available_permits(), 8192 - "original".len());
    drop(output);
    assert_eq!(bank.available_permits(), 8192);
    let bank = Arc::new(Semaphore::new(0));
    let mut empty = Window::new(&bank, std::iter::empty::<std::future::Ready<()>>()).unwrap();
    assert!(empty.next().await.is_none());
    assert_eq!(bank.available_permits(), 0);
}
