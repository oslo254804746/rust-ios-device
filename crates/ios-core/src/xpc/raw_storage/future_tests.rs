use super::*;
use std::future::Future;
use std::pin::Pin;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::task::{Context, Poll};

struct Pending {
    bank: Arc<Semaphore>,
    drops: Arc<AtomicUsize>,
    address: Arc<AtomicUsize>,
    _pin: std::marker::PhantomPinned,
}
impl Future for Pending {
    type Output = ();
    fn poll(self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<()> {
        self.address.store(
            self.as_ref().get_ref() as *const Self as usize,
            Ordering::SeqCst,
        );
        Poll::Pending
    }
}
impl Drop for Pending {
    fn drop(&mut self) {
        assert_eq!(
            self.bank.available_permits(),
            0,
            "fee returned before original future"
        );
        assert_eq!(
            self as *const Self as usize,
            self.address.load(Ordering::SeqCst)
        );
        self.drops.fetch_add(1, Ordering::SeqCst);
    }
}

#[tokio::test]
async fn exact_box_payload_is_pinned_and_paid_until_caller_cancels_original() {
    let bytes = std::mem::size_of::<Pending>();
    let bank = Arc::new(Semaphore::new(bytes));
    let drops = Arc::new(AtomicUsize::new(0));
    let address = Arc::new(AtomicUsize::new(0));
    let mut operation = boxed(
        Pending {
            bank: Arc::clone(&bank),
            drops: Arc::clone(&drops),
            address,
            _pin: std::marker::PhantomPinned,
        },
        Some(&bank),
    )
    .unwrap();
    assert_eq!(bank.available_permits(), 0);
    std::future::poll_fn(|cx| {
        assert!(Pin::new(&mut operation).poll(cx).is_pending());
        Poll::Ready(())
    })
    .await;
    drop(operation);
    assert_eq!(drops.load(Ordering::SeqCst), 1);
    assert_eq!(bank.available_permits(), bytes);
}

#[test]
fn one_byte_box_refusal_drops_unpolled_borrowed_original_once() {
    struct Unpolled<'a>(&'a AtomicUsize);
    impl Future for Unpolled<'_> {
        type Output = ();
        fn poll(self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<()> {
            panic!("refused future was polled");
        }
    }
    impl Drop for Unpolled<'_> {
        fn drop(&mut self) {
            self.0.fetch_add(1, Ordering::SeqCst);
        }
    }
    let dropped = AtomicUsize::new(0);
    let bytes = std::mem::size_of::<Unpolled<'_>>();
    let bank = Arc::new(Semaphore::new(bytes - 1));
    let _error = boxed(Unpolled(&dropped), Some(&bank)).err().unwrap();
    #[cfg(feature = "tunnel")]
    assert_eq!(
        crate::classify_error(&_error),
        crate::ErrorClass::ResourceLimit
    );
    assert_eq!(dropped.load(Ordering::SeqCst), 1);
    assert_eq!(bank.available_permits(), bytes - 1);
}

#[tokio::test]
async fn original_typed_io_result_outlives_future_box_without_being_replaced() {
    #[derive(Debug)]
    struct Native(u8);
    impl std::fmt::Display for Native {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            f.write_str("native fixture")
        }
    }
    impl std::error::Error for Native {}
    let original = io::Error::new(io::ErrorKind::BrokenPipe, Native(23));
    let pointer = original
        .get_ref()
        .unwrap()
        .downcast_ref::<Native>()
        .unwrap() as *const Native;
    let future = std::future::ready(Err::<(), _>(original));
    let bytes = std::mem::size_of_val(&future);
    let bank = Arc::new(Semaphore::new(bytes));
    let error = boxed(future, Some(&bank)).unwrap().await.unwrap_err();
    assert_eq!(error.kind(), io::ErrorKind::BrokenPipe);
    assert_eq!(
        error.get_ref().unwrap().downcast_ref::<Native>().unwrap().0,
        23
    );
    assert_eq!(
        error.get_ref().unwrap().downcast_ref::<Native>().unwrap() as *const Native,
        pointer
    );
    assert_eq!(bank.available_permits(), bytes);
}

#[tokio::test]
async fn no_bank_keeps_original_borrowed_nonsend_operation() {
    let local = std::rc::Rc::new(std::cell::Cell::new(0));
    let future = async {
        local.set(11);
        local.get()
    };
    let operation = boxed(future, None).unwrap();
    assert!(operation._fee.is_none());
    assert_eq!(operation.await, 11);
}
