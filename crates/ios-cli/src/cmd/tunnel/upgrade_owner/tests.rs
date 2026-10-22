use super::*;
use std::{
    marker::PhantomPinned,
    pin::Pin,
    sync::atomic::{AtomicUsize, Ordering},
    task::{Context, Poll},
};

struct Payload {
    original: Box<u8>,
    drops: Arc<AtomicUsize>,
}
impl Drop for Payload {
    fn drop(&mut self) {
        self.drops.fetch_add(1, Ordering::SeqCst);
    }
}
struct Capture {
    bank: Arc<Semaphore>,
    expected: Arc<AtomicUsize>,
    drops: Arc<AtomicUsize>,
    panic: bool,
}
#[repr(align(64))]
struct OriginalFuture {
    input: Mutex<Option<Input<Payload>>>,
    capture: Capture,
    _pin: PhantomPinned,
}
impl Future for OriginalFuture {
    type Output = Payload;
    fn poll(self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<Payload> {
        let this = self.as_ref().get_ref();
        assert_eq!(
            this as *const _ as usize,
            this.capture.expected.load(Ordering::SeqCst)
        );
        Poll::Ready(this.input.lock().unwrap().take().unwrap().take())
    }
}
impl Drop for OriginalFuture {
    fn drop(&mut self) {
        assert_eq!(
            self as *const _ as usize,
            self.capture.expected.load(Ordering::SeqCst)
        );
        assert!(self.capture.bank.available_permits() <= input_bytes());
        self.capture.drops.fetch_add(1, Ordering::SeqCst);
        if self.capture.panic {
            std::panic::panic_any(self.capture.expected.clone());
        }
    }
}
fn create(input: Input<Payload>, capture: Capture) -> OriginalFuture {
    OriginalFuture {
        input: Mutex::new(Some(input)),
        capture,
        _pin: PhantomPinned,
    }
}
fn input_bytes() -> usize {
    Strong::<Mutex<Option<Payload>>, ()>::body_bytes()
}
fn total_bytes() -> usize {
    input_bytes() + std::mem::size_of::<OriginalFuture>()
}
fn capture(
    bank: &Arc<Semaphore>,
    expected: &Arc<AtomicUsize>,
    drops: &Arc<AtomicUsize>,
) -> Capture {
    Capture {
        bank: bank.clone(),
        expected: expected.clone(),
        drops: drops.clone(),
        panic: false,
    }
}
fn prepared(
    bank: &Arc<Semaphore>,
    expected: &Arc<AtomicUsize>,
) -> Prepared<Payload, OriginalFuture> {
    let observed_bank = bank.clone();
    let observed_expected = expected.clone();
    let mut index = 0;
    let result = super::super::metadata_vec::with_allocation_hook(
        move |layout, pointer| {
            assert_eq!(
                observed_bank.available_permits(),
                0,
                "whole grant precedes both backings"
            );
            if index == 0 {
                assert_eq!(layout.size(), input_bytes());
            } else {
                assert_eq!(index, 1);
                assert_eq!(layout.size(), std::mem::size_of::<OriginalFuture>());
                observed_expected.store(pointer, Ordering::SeqCst);
            }
            index += 1;
        },
        || prepare(bank, create).unwrap(),
    );
    assert_ne!(expected.load(Ordering::SeqCst), 0);
    result
}
fn no_backing<R>(work: impl FnOnce() -> R) -> R {
    super::super::metadata_vec::with_allocation_hook(|_, _| panic!("late backing"), work)
}

#[test]
fn one_byte_short_owner_preflight_allocates_neither_root_nor_future() {
    let bank = Arc::new(Semaphore::new(total_bytes() - 1));
    let error = no_backing(|| {
        prepare(&bank, |_: Input<Payload>, _: Capture| -> OriginalFuture {
            panic!("refusal must not construct or poll the original future")
        })
        .err()
        .unwrap()
    });
    assert_eq!(
        crate::output::operation_error_class(error.as_ref()),
        ios_core::ErrorClass::ResourceLimit
    );
    assert_eq!(bank.available_permits(), total_bytes() - 1);
}

#[test]
fn pending_callback_keeps_original_paid_future_and_refunds_only_on_actual_drop() {
    let bank = Arc::new(Semaphore::new(total_bytes()));
    let expected = Arc::new(AtomicUsize::new(0));
    let drops = Arc::new(AtomicUsize::new(0));
    let owner = prepared(&bank, &expected);
    bank.close();
    let callback = no_backing(|| owner.complete(capture(&bank, &expected, &drops), create));
    assert_eq!(bank.available_permits(), 0);
    assert_eq!(
        drops.load(Ordering::SeqCst),
        0,
        "callback has not been invoked or polled"
    );
    let observed_bank = bank.clone();
    let freed = Arc::new(AtomicUsize::new(0));
    let observed_freed = freed.clone();
    super::super::shared_weak_allocation::after_free(
        move || {
            assert!(
                observed_bank.available_permits() <= std::mem::size_of::<OriginalFuture>(),
                "input root fee still held after physical free"
            );
            observed_freed.fetch_add(1, Ordering::SeqCst);
        },
        || no_backing(|| drop(callback)),
    );
    assert_eq!(freed.load(Ordering::SeqCst), 1);
    assert_eq!(drops.load(Ordering::SeqCst), 1);
    assert_eq!(bank.available_permits(), total_bytes());
}

#[test]
fn callback_transfers_unpolled_original_payload_at_closed_zero_without_late_allocation() {
    let bank = Arc::new(Semaphore::new(total_bytes()));
    let expected = Arc::new(AtomicUsize::new(0));
    let drops = Arc::new(AtomicUsize::new(0));
    let payload_drops = Arc::new(AtomicUsize::new(0));
    let payload = Payload {
        original: Box::new(9),
        drops: payload_drops.clone(),
    };
    let callback = prepared(&bank, &expected).complete(capture(&bank, &expected, &drops), create);
    bank.close();
    let future = no_backing(|| callback(payload));
    assert_eq!(bank.available_permits(), 0);
    assert_eq!(payload_drops.load(Ordering::SeqCst), 0);
    let observed_bank = bank.clone();
    super::super::shared_weak_allocation::after_free(
        move || {
            assert_eq!(
                observed_bank.available_permits(),
                0,
                "root and future fees survive input deallocation"
            );
        },
        || no_backing(|| drop(future)),
    );
    assert_eq!(payload_drops.load(Ordering::SeqCst), 1);
    assert_eq!(drops.load(Ordering::SeqCst), 1);
    assert_eq!(bank.available_permits(), total_bytes());
}

#[tokio::test]
async fn original_pinned_future_returns_same_payload_then_keeps_its_own_fee_until_drop() {
    let bank = Arc::new(Semaphore::new(total_bytes()));
    let expected = Arc::new(AtomicUsize::new(0));
    let drops = Arc::new(AtomicUsize::new(0));
    let payload_drops = Arc::new(AtomicUsize::new(0));
    let payload = Payload {
        original: Box::new(7),
        drops: payload_drops.clone(),
    };
    let pointer = payload.original.as_ref() as *const u8;
    let callback = prepared(&bank, &expected).complete(capture(&bank, &expected, &drops), create);
    bank.close();
    let mut future = callback(payload);
    let returned = (&mut future).await;
    assert_eq!(returned.original.as_ref() as *const u8, pointer);
    assert_eq!(bank.available_permits(), input_bytes());
    assert_eq!(drops.load(Ordering::SeqCst), 0);
    drop(future);
    assert_eq!(bank.available_permits(), total_bytes());
    assert_eq!(drops.load(Ordering::SeqCst), 1);
    assert_eq!(payload_drops.load(Ordering::SeqCst), 0);
    drop(returned);
    assert_eq!(payload_drops.load(Ordering::SeqCst), 1);
}

#[test]
fn original_future_destructor_unwind_preserves_payload_and_refunds_both_backings_once() {
    let bank = Arc::new(Semaphore::new(total_bytes()));
    let expected = Arc::new(AtomicUsize::new(0));
    let drops = Arc::new(AtomicUsize::new(0));
    let owner = prepared(&bank, &expected);
    let mut capture = capture(&bank, &expected, &drops);
    capture.panic = true;
    let callback = owner.complete(capture, create);
    bank.close();
    let panic =
        std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| drop(callback))).unwrap_err();
    let original = panic.downcast::<Arc<AtomicUsize>>().unwrap();
    assert!(Arc::ptr_eq(&original, &expected));
    assert_eq!(drops.load(Ordering::SeqCst), 1);
    assert_eq!(bank.available_permits(), total_bytes());
}
