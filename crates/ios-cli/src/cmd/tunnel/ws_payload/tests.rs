use super::*;
use std::future::Future;
use std::pin::Pin;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Mutex;
use std::task::{Context, Poll};

#[derive(Clone, Copy)]
enum Mode {
    Pending,
    Shared,
    StartError,
    FlushError,
    Success,
}

struct Shared {
    data: Mutex<Option<Vec<u8>>>,
    bank: Arc<Semaphore>,
    drops: Arc<AtomicUsize>,
    events: Arc<Mutex<Vec<usize>>>,
}
impl Drop for Shared {
    fn drop(&mut self) {
        if self.data.get_mut().unwrap().is_some() {
            assert_eq!(self.bank.available_permits(), 0);
            self.events.lock().unwrap().push(3);
            self.drops.fetch_add(1, Ordering::SeqCst);
        }
    }
}

#[derive(Debug)]
struct Original {
    data: Vec<u8>,
    bank: Arc<Semaphore>,
    drops: Arc<AtomicUsize>,
    cause: Arc<std::io::Error>,
}
impl std::fmt::Display for Original {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("original retaining sink error")
    }
}
impl std::error::Error for Original {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        Some(&*self.cause)
    }
}
impl Drop for Original {
    fn drop(&mut self) {
        assert_eq!(self.bank.available_permits(), 0);
        assert!(!self.data.is_empty());
        self.drops.fetch_add(1, Ordering::SeqCst);
    }
}

struct TestSink {
    mode: Mode,
    data: Option<Vec<u8>>,
    shared: Arc<Shared>,
    error_pointer: Arc<AtomicUsize>,
    data_pointer: Arc<AtomicUsize>,
    cause: Arc<std::io::Error>,
}
impl TestSink {
    fn failure(&self, data: Vec<u8>) -> anyhow::Error {
        let error = anyhow::Error::new(Original {
            data,
            bank: self.shared.bank.clone(),
            drops: self.shared.drops.clone(),
            cause: self.cause.clone(),
        });
        self.error_pointer.store(
            error.downcast_ref::<Original>().unwrap() as *const Original as usize,
            Ordering::SeqCst,
        );
        error
    }
}
impl Sink<Message> for TestSink {
    type Error = anyhow::Error;
    fn poll_ready(self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<Result<()>> {
        Poll::Ready(Ok(()))
    }
    fn start_send(mut self: Pin<&mut Self>, message: Message) -> Result<()> {
        let Message::Binary(data) = message else {
            unreachable!()
        };
        assert_eq!(self.shared.bank.available_permits(), 0);
        self.data_pointer
            .store(data.as_ptr() as usize, Ordering::SeqCst);
        if matches!(self.mode, Mode::StartError) {
            return Err(self.failure(data));
        }
        if matches!(self.mode, Mode::Shared) {
            *self.shared.data.lock().unwrap() = Some(data);
        } else {
            self.data = Some(data);
        }
        Ok(())
    }
    fn poll_flush(mut self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<Result<()>> {
        match self.mode {
            Mode::Pending | Mode::Shared => Poll::Pending,
            Mode::FlushError => {
                let data = self.data.take().unwrap();
                Poll::Ready(Err(self.failure(data)))
            }
            Mode::Success => {
                assert_eq!(self.shared.bank.available_permits(), 0);
                self.data.take();
                self.shared.drops.fetch_add(1, Ordering::SeqCst);
                Poll::Ready(Ok(()))
            }
            Mode::StartError => unreachable!(),
        }
    }
    fn poll_close(self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<Result<()>> {
        unreachable!()
    }
}
impl Drop for TestSink {
    fn drop(&mut self) {
        self.shared.events.lock().unwrap().push(1);
        if self.data.is_some() {
            assert_eq!(self.shared.bank.available_permits(), 0);
            self.data.take();
            self.shared.drops.fetch_add(1, Ordering::SeqCst);
        }
    }
}
struct Peer(Arc<Shared>);
impl Drop for Peer {
    fn drop(&mut self) {
        self.0.events.lock().unwrap().push(2);
    }
}

struct Fixture {
    endpoints: Endpoints<TestSink, Peer>,
    drops: Arc<AtomicUsize>,
    events: Arc<Mutex<Vec<usize>>>,
    error_pointer: Arc<AtomicUsize>,
    data_pointer: Arc<AtomicUsize>,
    cause: Arc<std::io::Error>,
}
fn setup(mode: Mode, bank: &Arc<Semaphore>) -> Fixture {
    let drops = Arc::new(AtomicUsize::new(0));
    let events = Arc::new(Mutex::new(Vec::new()));
    let shared = Arc::new(Shared {
        data: Mutex::new(None),
        bank: bank.clone(),
        drops: drops.clone(),
        events: events.clone(),
    });
    let error_pointer = Arc::new(AtomicUsize::new(0));
    let data_pointer = Arc::new(AtomicUsize::new(0));
    let cause = Arc::new(std::io::Error::new(
        std::io::ErrorKind::BrokenPipe,
        "original IO",
    ));
    let sink = TestSink {
        mode,
        data: None,
        shared: shared.clone(),
        error_pointer: error_pointer.clone(),
        data_pointer: data_pointer.clone(),
        cause: cause.clone(),
    };
    Fixture {
        endpoints: Endpoints::new(sink, Peer(shared)),
        drops,
        events,
        error_pointer,
        data_pointer,
        cause,
    }
}
fn poll<F: Future>(future: Pin<&mut F>) -> Poll<F::Output> {
    future.poll(&mut Context::from_waker(
        futures_util::task::noop_waker_ref(),
    ))
}

#[tokio::test]
async fn upgrade_epoch_pending_original_sink_is_destroyed_and_refunded_on_stop_clear_shutdown() {
    for action in 0..3 {
        let lifecycle = super::super::lifecycle::Lifecycle::new();
        let connection = lifecycle
            .ticket("original-A")
            .await
            .unwrap()
            .fenced()
            .for_connection();
        let b = lifecycle
            .ticket("original-B")
            .await
            .unwrap()
            .for_connection();
        let mut stop = lifecycle
            .prepare_cutover::<3>(lifecycle.byte_bank())
            .unwrap();
        let mut clear = lifecycle
            .prepare_cutover::<2>(lifecycle.byte_bank())
            .unwrap();
        let mut close = lifecycle
            .prepare_cutover::<1>(lifecycle.byte_bank())
            .unwrap();
        let bank = Arc::new(Semaphore::new(4));
        let fixture = setup(Mode::Shared, &bank);
        let drops = fixture.drops.clone();
        let events = fixture.events.clone();
        let mut endpoints = fixture.endpoints;
        let stored = bank.clone();
        let mut operation = Box::pin(connection.run(async move {
            {
                let (sink, _, pending) = endpoints.parts();
                binary(sink, pending, b"data", &stored).await?;
            }
            Ok(endpoints.finish(Ok(())))
        }));
        assert!(poll(operation.as_mut()).is_pending());
        assert_eq!(bank.available_permits(), 0);
        match action {
            0 => lifecycle.stop("original-A", &mut stop).await,
            1 => lifecycle.clear(&mut clear),
            _ => lifecycle.close(&mut close),
        }
        let error = operation.await.err().unwrap();
        assert!(error.is::<ios_core::OperationCancelled>());
        assert_eq!(drops.load(Ordering::SeqCst), 1);
        assert_eq!(*events.lock().unwrap(), [1, 2, 3]);
        assert_eq!(bank.available_permits(), 4);
        assert_eq!(b.is_current(), action == 0);
    }
}

#[tokio::test]
async fn upgrade_epoch_original_ready_payload_error_crossing_clear_keeps_typed_pointer_and_fee() {
    let lifecycle = super::super::lifecycle::Lifecycle::new();
    let connection = lifecycle
        .ticket("original-A")
        .await
        .unwrap()
        .for_connection();
    let bank = Arc::new(Semaphore::new(4));
    let mut clear = lifecycle
        .prepare_cutover::<2>(lifecycle.byte_bank())
        .unwrap();
    let fixture = setup(Mode::FlushError, &bank);
    let pointer = fixture.error_pointer.clone();
    let drops = fixture.drops.clone();
    let mut endpoints = fixture.endpoints;
    let completion = connection
        .run(async {
            let result = {
                let (sink, _, pending) = endpoints.parts();
                binary(sink, pending, b"data", &bank).await
            };
            let completion = endpoints.finish(result);
            lifecycle.clear(&mut clear);
            Ok(completion)
        })
        .await
        .unwrap();
    assert!(!connection.is_current());
    let original = completion
        .result()
        .as_ref()
        .unwrap_err()
        .downcast_ref::<Original>()
        .unwrap();
    assert_eq!(
        original as *const Original as usize,
        pointer.load(Ordering::SeqCst)
    );
    assert_eq!(original.cause.kind(), std::io::ErrorKind::BrokenPipe);
    assert_eq!(bank.available_permits(), 0);
    assert_eq!(drops.load(Ordering::SeqCst), 0);
    drop(completion);
    assert_eq!(drops.load(Ordering::SeqCst), 1);
    assert_eq!(bank.available_permits(), 4);
}

#[test]
fn websocket_payload_cancel_drops_original_slot_and_both_shared_halves_before_refund() {
    for mode in [Mode::Pending, Mode::Shared] {
        let bank = Arc::new(Semaphore::new(4));
        let mut fixture = setup(mode, &bank);
        {
            let (sink, _, pending) = fixture.endpoints.parts();
            let mut sending = std::pin::pin!(binary(sink, pending, b"data", &bank));
            assert!(poll(sending.as_mut()).is_pending());
        }
        assert_eq!(bank.available_permits(), 0);
        assert_eq!(fixture.drops.load(Ordering::SeqCst), 0);
        drop(fixture.endpoints);
        assert_eq!(fixture.drops.load(Ordering::SeqCst), 1);
        assert_eq!(bank.available_permits(), 4);
        let expected = if matches!(mode, Mode::Shared) {
            vec![1, 2, 3]
        } else {
            vec![1, 2]
        };
        assert_eq!(*fixture.events.lock().unwrap(), expected);
    }
}

#[test]
fn websocket_payload_other_direction_completion_disposes_pending_original_before_last_fee() {
    let bank = Arc::new(Semaphore::new(4));
    let mut fixture = setup(Mode::Shared, &bank);
    {
        let (sink, _, pending) = fixture.endpoints.parts();
        let mut sending = std::pin::pin!(binary(sink, pending, b"data", &bank));
        assert!(poll(sending.as_mut()).is_pending());
    }
    let completion = fixture.endpoints.finish(Ok(()));
    assert!(completion.result().is_ok());
    assert_eq!(*fixture.events.lock().unwrap(), [1, 2, 3]);
    assert_eq!(fixture.drops.load(Ordering::SeqCst), 1);
    assert_eq!(bank.available_permits(), 0);
    drop(completion);
    assert_eq!(bank.available_permits(), 4);
}

#[tokio::test]
async fn websocket_payload_original_error_owns_data_downcast_and_io_pointer_through_completion() {
    for mode in [Mode::StartError, Mode::FlushError] {
        let bank = Arc::new(Semaphore::new(4));
        let mut fixture = setup(mode, &bank);
        let result = {
            let (sink, _, pending) = fixture.endpoints.parts();
            binary(sink, pending, b"data", &bank).await
        };
        let completion = fixture.endpoints.finish(result);
        let error = completion.result().as_ref().unwrap_err();
        let original = error.downcast_ref::<Original>().unwrap();
        assert_eq!(
            original as *const Original as usize,
            fixture.error_pointer.load(Ordering::SeqCst)
        );
        assert_eq!(
            original.data.as_ptr() as usize,
            fixture.data_pointer.load(Ordering::SeqCst)
        );
        assert_eq!(original.data, b"data");
        assert!(Arc::ptr_eq(&original.cause, &fixture.cause));
        assert_eq!(original.cause.kind(), std::io::ErrorKind::BrokenPipe);
        assert_eq!(fixture.drops.load(Ordering::SeqCst), 0);
        assert_eq!(bank.available_permits(), 0);
        assert_eq!(*fixture.events.lock().unwrap(), [1, 2]);
        drop(completion);
        assert_eq!(fixture.drops.load(Ordering::SeqCst), 1);
        assert_eq!(bank.available_permits(), 4);
    }
}

#[tokio::test]
async fn websocket_payload_successful_flush_reuses_exact_quota_and_original_allocation() {
    let bank = Arc::new(Semaphore::new(4));
    let mut fixture = setup(Mode::Success, &bank);
    let allocated = Arc::new(AtomicUsize::new(0));
    let captured = allocated.clone();
    let charged = bank.clone();
    let hook = super::super::metadata_vec::on_allocation(move |layout, pointer| {
        assert_eq!(layout.size(), 4);
        assert_eq!(charged.available_permits(), 0);
        captured.store(pointer, Ordering::SeqCst);
    });
    for _ in 0..128 {
        let (sink, _, pending) = fixture.endpoints.parts();
        binary(sink, pending, b"data", &bank).await.unwrap();
        assert_eq!(
            fixture.data_pointer.load(Ordering::SeqCst),
            allocated.load(Ordering::SeqCst)
        );
        assert_eq!(bank.available_permits(), 4);
    }
    drop(hook);
    assert_eq!(fixture.drops.load(Ordering::SeqCst), 128);
    drop(fixture.endpoints.finish(Ok(())));
    assert_eq!(bank.available_permits(), 4);
}

#[tokio::test]
async fn websocket_payload_last_byte_refusal_and_cancelled_retry_create_no_second_destination() {
    let limited = Arc::new(Semaphore::new(3));
    let mut fixture = setup(Mode::Success, &limited);
    let hook =
        super::super::metadata_vec::on_allocation(|_, _| panic!("refused payload allocated"));
    let result = {
        let (sink, _, pending) = fixture.endpoints.parts();
        binary(sink, pending, b"data", &limited).await
    };
    drop(hook);
    let completion = fixture.endpoints.finish(result);
    assert_eq!(
        crate::output::operation_error_class(completion.result().as_ref().unwrap_err().as_ref()),
        ios_core::ErrorClass::ResourceLimit
    );
    assert_eq!(fixture.data_pointer.load(Ordering::SeqCst), 0);
    drop(completion);
    assert_eq!(limited.available_permits(), 3);
    let bank = Arc::new(Semaphore::new(4));
    let mut fixture = setup(Mode::Pending, &bank);
    {
        let (sink, _, pending) = fixture.endpoints.parts();
        let mut sending = std::pin::pin!(binary(sink, pending, b"data", &bank));
        assert!(poll(sending.as_mut()).is_pending());
    }
    let hook =
        super::super::metadata_vec::on_allocation(|_, _| panic!("pending payload was overwritten"));
    let result = {
        let (sink, _, pending) = fixture.endpoints.parts();
        binary(sink, pending, b"next", &bank).await
    };
    drop(hook);
    let completion = fixture.endpoints.finish(result);
    assert_eq!(
        crate::output::operation_error_class(completion.result().as_ref().unwrap_err().as_ref()),
        ios_core::ErrorClass::ResourceLimit
    );
    assert_eq!(fixture.drops.load(Ordering::SeqCst), 1);
    assert_eq!(bank.available_permits(), 0);
    drop(completion);
    assert_eq!(bank.available_permits(), 4);
}
