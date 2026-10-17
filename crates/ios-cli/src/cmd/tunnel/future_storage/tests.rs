use super::super::metadata_vec::with_allocation_hook;
use super::*;
use std::cell::Cell;
use std::marker::PhantomPinned;
use std::sync::atomic::{AtomicUsize, Ordering};

#[repr(align(128))]
struct Original {
    bank: Arc<Semaphore>,
    polls: Arc<AtomicUsize>,
    drops: Arc<AtomicUsize>,
    address: Cell<usize>,
    _pinned: PhantomPinned,
}
impl Future for Original {
    type Output = ();
    fn poll(self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<()> {
        let address = &*self as *const Self as usize;
        assert_eq!(address % 128, 0);
        let old = self.address.replace(address);
        assert!(old == 0 || old == address);
        assert_eq!(self.bank.available_permits(), 0);
        self.polls.fetch_add(1, Ordering::SeqCst);
        Poll::Pending
    }
}
impl Drop for Original {
    fn drop(&mut self) {
        if self.address.get() != 0 {
            assert_eq!(self.address.get(), self as *const Self as usize);
            assert_eq!(self.bank.available_permits(), 0);
        }
        self.drops.fetch_add(1, Ordering::SeqCst);
    }
}
fn original(bank: &Arc<Semaphore>, polls: &Arc<AtomicUsize>, drops: &Arc<AtomicUsize>) -> Original {
    Original {
        bank: bank.clone(),
        polls: polls.clone(),
        drops: drops.clone(),
        address: Cell::new(0),
        _pinned: PhantomPinned,
    }
}

#[test]
fn source_future_dynamic_unsize_retains_original_allocation_alignment_pin_and_last_fee() {
    let bytes = std::mem::size_of::<Original>();
    let bank = Arc::new(Semaphore::new(bytes));
    let polls = Arc::new(AtomicUsize::new(0));
    let drops = Arc::new(AtomicUsize::new(0));
    let pointer = Arc::new(AtomicUsize::new(0));
    let captured = pointer.clone();
    let charged = bank.clone();
    let mut future = with_allocation_hook(
        move |layout, address| {
            assert_eq!(layout, std::alloc::Layout::new::<Original>());
            assert_eq!(charged.available_permits(), 0);
            assert_eq!(
                captured.swap(address, Ordering::SeqCst),
                0,
                "extra concrete allocation"
            );
        },
        || Dynamic::new(original(&bank, &polls, &drops), &bank).unwrap(),
    );
    assert_eq!(future.data_pointer(), pointer.load(Ordering::SeqCst));
    for _ in 0..2 {
        assert!(Pin::new(&mut future)
            .poll(&mut Context::from_waker(
                futures_util::task::noop_waker_ref()
            ))
            .is_pending());
    }
    assert_eq!(polls.load(Ordering::SeqCst), 2);
    drop(future);
    assert_eq!(drops.load(Ordering::SeqCst), 1);
    assert_eq!(bank.available_permits(), bytes);
}

#[test]
fn source_future_dynamic_one_byte_refusal_precedes_original_allocation_and_poll() {
    let bytes = std::mem::size_of::<Original>() - 1;
    let bank = Arc::new(Semaphore::new(bytes));
    let polls = Arc::new(AtomicUsize::new(0));
    let drops = Arc::new(AtomicUsize::new(0));
    let error = with_allocation_hook(
        |_, _| panic!("refused Source future allocated"),
        || {
            Dynamic::new(original(&bank, &polls, &drops), &bank)
                .err()
                .unwrap()
        },
    );
    assert_eq!(
        crate::output::operation_error_class(error.as_ref()),
        ios_core::ErrorClass::ResourceLimit
    );
    assert_eq!(polls.load(Ordering::SeqCst), 0);
    assert_eq!(drops.load(Ordering::SeqCst), 1);
    assert_eq!(bank.available_permits(), bytes);
}

#[tokio::test]
async fn source_future_original_typed_error_and_paid_output_outlive_the_source_box() {
    let bank = Arc::new(Semaphore::new(8192));
    let original = anyhow::Error::new(std::io::Error::new(
        std::io::ErrorKind::PermissionDenied,
        "original-dynamic-error",
    ));
    let pointer = original.downcast_ref::<std::io::Error>().unwrap() as *const _ as usize;
    let future = Dynamic::new(async move { Err::<(), _>(original) }, &bank).unwrap();
    let error = future.await.unwrap_err();
    assert_eq!(
        error.downcast_ref::<std::io::Error>().unwrap() as *const _ as usize,
        pointer
    );
    assert_eq!(
        error.downcast_ref::<std::io::Error>().unwrap().kind(),
        std::io::ErrorKind::PermissionDenied
    );
    assert_eq!(bank.available_permits(), 8192);
    let future = Dynamic::new(
        async { super::super::paid_json::Text::copy("original", &bank) },
        &bank,
    )
    .unwrap();
    let output = future.await.unwrap();
    assert_eq!(output.as_str(), "original");
    assert_eq!(bank.available_permits(), 8192 - "original".len());
    drop(output);
    assert_eq!(bank.available_permits(), 8192);
}

#[test]
fn source_future_zero_sized_dynamic_payload_keeps_aligned_pointer_without_allocation() {
    struct Zero;
    impl Future for Zero {
        type Output = ();
        fn poll(self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<()> {
            Poll::Ready(())
        }
    }
    let bank = Arc::new(Semaphore::new(0));
    let mut future = with_allocation_hook(
        |_, _| panic!("zero-sized Source allocated"),
        || Dynamic::new(Zero, &bank).unwrap(),
    );
    assert_ne!(future.data_pointer(), 0);
    assert!(Pin::new(&mut future)
        .poll(&mut Context::from_waker(
            futures_util::task::noop_waker_ref()
        ))
        .is_ready());
    drop(future);
    assert_eq!(bank.available_permits(), 0);
}
