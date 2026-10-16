use super::*;
use std::cell::Cell;
use std::marker::PhantomPinned;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Mutex;

use super::super::metadata_vec::with_allocation_hook;

struct Probe {
    id: usize,
    ready: Arc<AtomicUsize>,
    polls: Arc<Mutex<Vec<usize>>>,
    drops: Arc<AtomicUsize>,
    address: Cell<usize>,
    _pinned: PhantomPinned,
}

impl Future for Probe {
    type Output = usize;

    fn poll(self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<usize> {
        let address = &*self as *const Self as usize;
        match self.address.replace(address) {
            0 => (),
            previous => assert_eq!(previous, address, "pinned child moved"),
        }
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
        if self.address.get() != 0 {
            assert_eq!(self.address.get(), self as *const Self as usize);
        }
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
        _pinned: PhantomPinned,
    }
}

fn poll<F: Future>(future: Pin<&mut F>) -> Poll<F::Output> {
    future.poll(&mut Context::from_waker(
        futures_util::task::noop_waker_ref(),
    ))
}

#[test]
fn federation_join_all_actual_layouts_are_paid_before_allocation_and_pin_is_stable() {
    let bank = Arc::new(Semaphore::new(8192));
    let ready = Arc::new(AtomicUsize::new(0));
    let polls = Arc::new(Mutex::new(Vec::new()));
    let drops = Arc::new(AtomicUsize::new(0));
    let charged = bank.clone();
    let observed = Arc::new(Mutex::new(Vec::new()));
    let captured = observed.clone();
    let mut join = std::pin::pin!(all(
        &bank,
        (0..3).map(|id| probe(id, &ready, &polls, &drops)),
    ));
    with_allocation_hook(
        move |layout, pointer| {
            let mut seen = captured.lock().unwrap();
            let previous: usize = seen.iter().map(|(bytes, _)| bytes).sum();
            assert_eq!(charged.available_permits(), 8192 - previous - layout.size());
            seen.push((layout.size(), pointer));
        },
        || assert!(poll(join.as_mut()).is_pending()),
    );
    let observed = observed.lock().unwrap();
    assert_eq!(observed.len(), 5);
    assert_eq!(observed[0].0, 3 * std::mem::size_of::<Slot<Probe>>());
    assert_eq!(observed[1].0, 3 * std::mem::size_of::<usize>());
    for &(bytes, pointer) in &observed[2..] {
        assert_eq!(bytes, std::mem::size_of::<Probe>());
        assert_ne!(pointer, 0);
    }
    assert_eq!(*polls.lock().unwrap(), [0, 1, 2]);
    ready.store(4, Ordering::SeqCst);
    assert!(poll(join.as_mut()).is_pending());
    ready.store(6, Ordering::SeqCst);
    assert!(poll(join.as_mut()).is_pending());
    ready.store(7, Ordering::SeqCst);
    let Poll::Ready(Ok(outputs)) = poll(join.as_mut()) else {
        panic!("all children completed");
    };
    assert_eq!(outputs.as_slice(), [0, 1, 2]);
    assert_eq!(drops.load(Ordering::SeqCst), 3);
    assert_eq!(*polls.lock().unwrap(), [0, 1, 2, 0, 1, 2, 0, 1, 0]);
    assert_eq!(bank.available_permits(), 8192 - observed[1].0);
    let mut iter = outputs.into_iter();
    assert_eq!(iter.next(), Some(0));
    assert_eq!(bank.available_permits(), 8192 - observed[1].0);
    drop(iter);
    assert_eq!(bank.available_permits(), 8192);
}

#[test]
fn federation_join_array_and_late_future_refusals_never_poll_children_and_refund() {
    let slot = std::mem::size_of::<Slot<Probe>>();
    let output = std::mem::size_of::<usize>();
    let future = std::mem::size_of::<Probe>();
    for (limit, expected_allocations) in [
        (3 * slot - 1, 0),
        (3 * (slot + output) - 1, 1),
        (3 * (slot + output + future) - 1, 4),
    ] {
        let bank = Arc::new(Semaphore::new(limit));
        let ready = Arc::new(AtomicUsize::new(7));
        let polls = Arc::new(Mutex::new(Vec::new()));
        let drops = Arc::new(AtomicUsize::new(0));
        let allocations = Arc::new(AtomicUsize::new(0));
        let captured = allocations.clone();
        // Array::IntoIter owns every original child, including the children
        // not yet visited when admission fails on the last boxed future.
        let children = std::array::from_fn::<_, 3, _>(|id| probe(id, &ready, &polls, &drops));
        let mut join = std::pin::pin!(all(&bank, children.into_iter()));
        let error = with_allocation_hook(
            move |_, _| {
                captured.fetch_add(1, Ordering::SeqCst);
            },
            || match poll(join.as_mut()) {
                Poll::Ready(Err(error)) => error,
                _ => panic!("preflight must refuse"),
            },
        );
        assert_eq!(
            crate::output::operation_error_class(error.as_ref()),
            ios_core::ErrorClass::ResourceLimit
        );
        assert_eq!(allocations.load(Ordering::SeqCst), expected_allocations);
        assert!(polls.lock().unwrap().is_empty());
        assert_eq!(drops.load(Ordering::SeqCst), 3);
        assert_eq!(bank.available_permits(), limit);
    }
}

#[test]
fn federation_join_cancel_drops_pending_original_futures_and_completed_outputs() {
    let bank = Arc::new(Semaphore::new(8192));
    let ready = Arc::new(AtomicUsize::new(2));
    let polls = Arc::new(Mutex::new(Vec::new()));
    let drops = Arc::new(AtomicUsize::new(0));
    {
        let mut join = std::pin::pin!(all(
            &bank,
            (0..3).map(|id| probe(id, &ready, &polls, &drops)),
        ));
        assert!(poll(join.as_mut()).is_pending());
        assert_eq!(drops.load(Ordering::SeqCst), 1);
        assert_eq!(
            bank.available_permits(),
            8192 - 3 * std::mem::size_of::<Slot<Probe>>()
                - 3 * std::mem::size_of::<usize>()
                - 2 * std::mem::size_of::<Probe>()
        );
    }
    assert_eq!(drops.load(Ordering::SeqCst), 3);
    assert_eq!(bank.available_permits(), 8192);
}

#[test]
fn federation_join_waits_after_error_and_retains_original_typed_error_pointer() {
    let bank = Arc::new(Semaphore::new(8192));
    let error = Arc::new(std::io::Error::new(
        std::io::ErrorKind::PermissionDenied,
        "original",
    ));
    let ready = AtomicUsize::new(0);
    let polls = AtomicUsize::new(0);
    let children = (0..2).map(|id| {
        let error = error.clone();
        let ready = &ready;
        let polls = &polls;
        async move {
            poll_fn(|cx| {
                polls.fetch_or(1 << id, Ordering::SeqCst);
                if id == 1 && ready.load(Ordering::SeqCst) == 0 {
                    cx.waker().wake_by_ref();
                    Poll::Pending
                } else if id == 0 {
                    Poll::Ready(Err(error.clone()))
                } else {
                    Poll::Ready(Ok(id))
                }
            })
            .await
        }
    });
    let mut join = std::pin::pin!(all(&bank, children));
    assert!(poll(join.as_mut()).is_pending());
    assert_eq!(polls.load(Ordering::SeqCst), 3);
    ready.store(1, Ordering::SeqCst);
    let Poll::Ready(Ok(outputs)) = poll(join.as_mut()) else {
        panic!("wait-all complete");
    };
    assert!(Arc::ptr_eq(
        outputs.as_slice()[0].as_ref().unwrap_err(),
        &error
    ));
    assert_eq!(outputs.as_slice()[1].as_ref().unwrap(), &1);
    assert_eq!(error.kind(), std::io::ErrorKind::PermissionDenied);
    drop(outputs);
    assert_eq!(bank.available_permits(), 8192);
}

#[test]
fn federation_join_output_owns_its_payload_after_collector_and_iterator_drop() {
    let bank = Arc::new(Semaphore::new(8192));
    let output = {
        let children =
            (0..2).map(|_| async { super::super::paid_json::Text::copy("owned", &bank) });
        let mut join = std::pin::pin!(all(&bank, children));
        let Poll::Ready(Ok(outputs)) = poll(join.as_mut()) else {
            panic!("immediate children");
        };
        let mut iterator = outputs.into_iter();
        let output = iterator.next().unwrap().unwrap();
        drop(iterator);
        output
    };
    assert_eq!(output.as_str(), "owned");
    assert_eq!(bank.available_permits(), 8192 - 5);
    drop(output);
    assert_eq!(bank.available_permits(), 8192);
}

#[test]
fn federation_join_empty_and_zero_sized_future_need_no_payload_allocation() {
    struct Zero;
    impl Future for Zero {
        type Output = ();
        fn poll(self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<()> {
            Poll::Ready(())
        }
    }
    let bank = Arc::new(Semaphore::new(0));
    let mut empty = std::pin::pin!(all(&bank, std::iter::empty::<Zero>()));
    assert!(matches!(poll(empty.as_mut()), Poll::Ready(Ok(_))));
    let future = PaidFuture::new(Zero, &bank).unwrap();
    let mut future = std::pin::pin!(future);
    assert!(poll(future.as_mut()).is_ready());
    assert_eq!(bank.available_permits(), 0);
}

#[test]
fn federation_join_future_alignment_and_destruction_precede_actual_payload_refund() {
    #[repr(align(128))]
    struct Body(Arc<Semaphore>, Arc<AtomicUsize>, PhantomPinned);
    impl Future for Body {
        type Output = ();
        fn poll(self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<()> {
            Poll::Pending
        }
    }
    impl Drop for Body {
        fn drop(&mut self) {
            assert_eq!(self.0.available_permits(), 0);
            assert_eq!(self as *const Self as usize % 128, 0);
            self.1.fetch_add(1, Ordering::SeqCst);
        }
    }
    let bytes = std::mem::size_of::<Body>();
    let bank = Arc::new(Semaphore::new(bytes));
    let drops = Arc::new(AtomicUsize::new(0));
    let future = PaidFuture::new(Body(bank.clone(), drops.clone(), PhantomPinned), &bank).unwrap();
    assert_eq!(bank.available_permits(), 0);
    drop(future);
    assert_eq!(drops.load(Ordering::SeqCst), 1);
    assert_eq!(bank.available_permits(), bytes);
}

#[tokio::test]
async fn federation_join_parent_waker_resumes_all_children_without_private_wake_set() {
    let bank = Arc::new(Semaphore::new(8192));
    let (first_tx, first_rx) = tokio::sync::oneshot::channel::<usize>();
    let (second_tx, second_rx) = tokio::sync::oneshot::channel::<usize>();
    let join = all(&bank, [first_rx, second_rx].into_iter());
    let release = async move {
        tokio::task::yield_now().await;
        second_tx.send(22).unwrap();
        tokio::task::yield_now().await;
        first_tx.send(11).unwrap();
    };
    let (outputs, ()) = tokio::time::timeout(std::time::Duration::from_secs(1), async {
        tokio::join!(join, release)
    })
    .await
    .unwrap();
    let outputs = outputs.unwrap();
    assert_eq!(outputs.as_slice(), &[Ok(11), Ok(22)]);
    drop(outputs);
    assert_eq!(bank.available_permits(), 8192);
}

#[test]
fn federation_join_cancel_refunds_already_completed_paid_output() {
    let bank = Arc::new(Semaphore::new(8192));
    {
        let children = (0..2).map(|id| {
            let bank = &bank;
            async move {
                if id == 0 {
                    super::super::paid_json::Text::copy("completed", bank)
                } else {
                    std::future::pending().await
                }
            }
        });
        let mut join = std::pin::pin!(all(&bank, children));
        assert!(poll(join.as_mut()).is_pending());
        assert!(bank.available_permits() < 8192 - "completed".len());
    }
    assert_eq!(bank.available_permits(), 8192);
}
