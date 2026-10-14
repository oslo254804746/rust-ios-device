use super::super::owned_buffer::OwnedBuffer;
use super::*;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

#[test]
fn channel_actual_body_backing_exact_and_one_byte_refusal_refund() {
    let needed = required_bytes::<u64>(3);
    for limit in [needed - 1, needed] {
        let bank = Arc::new(Semaphore::new(limit));
        match with_bank::<u64>(3, Some(&bank)) {
            Ok((sender, receiver)) => {
                assert_eq!(limit, needed);
                assert_eq!(bank.available_permits(), 0);
                let Sender::Metered(shared) = &sender else {
                    panic!("metered channel used legacy storage")
                };
                assert_eq!(shared.lock().items.capacity(), 3);
                let body = shared._body.num_permits();
                drop(receiver);
                assert_eq!(bank.available_permits(), needed - body);
                drop(sender);
            }
            Err(error) => {
                assert_eq!(limit, needed - 1);
                assert_eq!(
                    crate::classify_error(&error),
                    crate::ErrorClass::ResourceLimit
                );
            }
        }
        assert_eq!(bank.available_permits(), limit);
    }
}

#[tokio::test]
async fn channel_blocked_send_cancel_preserves_fifo_and_original_buffer_last_owner_fee() {
    let needed = required_bytes::<OwnedBuffer>(1);
    let bank = Arc::new(Semaphore::new(needed + 8192));
    let (sender, mut receiver) = with_bank(1, Some(&bank)).unwrap();
    let mut first = OwnedBuffer::zeroed(4096, Some(&bank)).unwrap();
    first.truncate(1);
    let pointer = first.as_ptr();
    sender.send(first).await.unwrap();
    let mut pending = Box::pin(sender.send(OwnedBuffer::zeroed(4096, Some(&bank)).unwrap()));
    assert!(futures_util::poll!(&mut pending).is_pending());
    assert_eq!(receiver.len(), 1);
    assert_eq!(bank.available_permits(), 0);
    drop(pending);
    assert_eq!(bank.available_permits(), 4096);
    let first = receiver.recv().await.unwrap();
    assert_eq!(first.as_ptr(), pointer);
    assert_eq!(first.capacity(), 4096);
    assert_eq!(receiver.len(), 0);
    drop((receiver, sender));
    assert_eq!(bank.available_permits(), needed + 4096);
    drop(first);
    assert_eq!(bank.available_permits(), needed + 8192);
}

#[tokio::test]
async fn channel_sender_close_drains_fifo_and_cancelled_receiver_does_not_lose_wakeup() {
    let bank = Arc::new(Semaphore::new(required_bytes::<u8>(2)));
    let (sender, mut receiver) = with_bank(2, Some(&bank)).unwrap();
    let clone = sender.clone();
    let mut pending = Box::pin(receiver.recv());
    assert!(futures_util::poll!(&mut pending).is_pending());
    drop(pending);
    sender.send(1).await.unwrap();
    clone.send(2).await.unwrap();
    drop((sender, clone));
    assert_eq!(receiver.recv().await, Some(1));
    assert_eq!(receiver.recv().await, Some(2));
    assert_eq!(receiver.recv().await, None);
    drop(receiver);
    assert_eq!(bank.available_permits(), required_bytes::<u8>(2));
}

#[tokio::test]
async fn channel_receiver_close_wakes_blocked_sender_and_returns_original_message() {
    let bank = Arc::new(Semaphore::new(required_bytes::<Box<u64>>(1)));
    let (sender, receiver) = with_bank::<Box<u64>>(1, Some(&bank)).unwrap();
    sender.try_send(Box::new(1)).unwrap();
    let second = Box::new(2);
    let pointer = &*second as *const u64;
    let mut pending = Box::pin(sender.send(second));
    assert!(futures_util::poll!(&mut pending).is_pending());
    let mut closed = Box::pin(sender.closed());
    assert!(futures_util::poll!(&mut closed).is_pending());
    drop(receiver);
    tokio::time::timeout(Duration::from_secs(1), closed)
        .await
        .unwrap();
    let original = tokio::time::timeout(Duration::from_secs(1), pending)
        .await
        .unwrap()
        .unwrap_err()
        .0;
    assert_eq!(&*original as *const u64, pointer);
    assert!(matches!(
        sender.try_send(Box::new(3)),
        Err(error::TrySendError::Closed(_))
    ));
    drop((original, sender));
    assert_eq!(bank.available_permits(), required_bytes::<Box<u64>>(1));
}

#[tokio::test]
async fn channel_blocked_producers_keep_admission_fifo_and_cancelled_waiter_releases_its_place() {
    for cancel_first in [false, true] {
        let bank = Arc::new(Semaphore::new(required_bytes::<u8>(1)));
        let (sender, mut receiver) = with_bank(1, Some(&bank)).unwrap();
        sender.send(0).await.unwrap();
        let mut first = Box::pin(sender.send(1));
        let mut second = Box::pin(sender.send(2));
        assert!(futures_util::poll!(&mut first).is_pending());
        assert!(futures_util::poll!(&mut second).is_pending());
        if cancel_first {
            drop(first);
        } else {
            assert_eq!(receiver.recv().await, Some(0));
            assert!(futures_util::poll!(&mut second).is_pending());
            first.await.unwrap();
            assert_eq!(receiver.recv().await, Some(1));
        }
        if cancel_first {
            assert_eq!(receiver.recv().await, Some(0));
        }
        tokio::time::timeout(Duration::from_secs(1), second)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(receiver.recv().await, Some(2));
        drop((receiver, sender));
        assert_eq!(bank.available_permits(), required_bytes::<u8>(1));
    }
}

#[derive(Debug)]
struct Destroyed {
    count: Arc<AtomicUsize>,
}
impl Drop for Destroyed {
    fn drop(&mut self) {
        self.count.fetch_add(1, Ordering::SeqCst);
    }
}

#[test]
fn channel_receiver_drop_destroys_each_original_once_and_body_survives_last_sender() {
    let needed = required_bytes::<Destroyed>(3);
    let bank = Arc::new(Semaphore::new(needed));
    let count = Arc::new(AtomicUsize::new(0));
    let (sender, receiver) = with_bank(3, Some(&bank)).unwrap();
    let clone = sender.clone();
    for _ in 0..3 {
        sender
            .try_send(Destroyed {
                count: Arc::clone(&count),
            })
            .unwrap();
    }
    drop(receiver);
    assert_eq!(count.load(Ordering::SeqCst), 3);
    let remaining = bank.available_permits();
    assert!(remaining < needed);
    drop(sender);
    assert_eq!(bank.available_permits(), remaining);
    drop(clone);
    assert_eq!(bank.available_permits(), needed);
}

#[tokio::test]
async fn channel_constructor_last_byte_refusal_is_before_listener_workers_and_keeps_strict_eof() {
    use super::super::{runtime_channel_bytes, UserspaceTunDevice, ValidatedMtu, IPV6_HEADER_LEN};
    use tokio::io::AsyncReadExt;
    let baseline = tokio::runtime::Handle::current()
        .metrics()
        .num_alive_tasks();
    let needed = IPV6_HEADER_LEN + runtime_channel_bytes();
    let bank = Arc::new(Semaphore::new(needed - 1));
    let (stream, mut peer) = tokio::io::duplex(1280);
    let result = UserspaceTunDevice::start_validated_with_bank(
        "fd00::2",
        "fd00::1",
        ValidatedMtu::new(1280).unwrap(),
        stream,
        Some(Arc::clone(&bank)),
    )
    .await;
    let error = result
        .err()
        .expect("last-byte shortage started runtime workers");
    assert_eq!(
        crate::classify_error(&error),
        crate::ErrorClass::ResourceLimit
    );
    assert_eq!(
        tokio::runtime::Handle::current()
            .metrics()
            .num_alive_tasks(),
        baseline
    );
    assert_eq!(bank.available_permits(), needed - 1);
    let mut byte = [0];
    assert!(matches!(
        tokio::time::timeout(Duration::from_secs(1), peer.read(&mut byte)).await,
        Ok(Ok(0))
    ));
}
