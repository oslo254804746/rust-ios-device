use std::error::Error;
#[cfg(unix)]
use std::future::pending;
use std::future::Future;
use std::io;
use std::pin::Pin;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::task::{Context, Poll};
use std::time::Duration;

use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt, ReadBuf};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::{oneshot, Notify};

use super::*;

const OUTER: Duration = Duration::from_secs(5);
const TOTAL: Duration = Duration::from_secs(3);

type DropRecords = Arc<Mutex<Vec<(usize, usize)>>>;

/// Observe the original S being destroyed, without panicking in Drop. The
/// recorded registration/admission are asserted only after EOF and task join.
struct Observed<S> {
    inner: Option<S>,
    owner: SessionOwner,
    shared: SharedAdmission,
    records: DropRecords,
    pending: Arc<Notify>,
}

fn observed<S>(
    inner: S,
    owner: &SessionOwner,
    shared: &SharedAdmission,
) -> (Observed<S>, DropRecords, Arc<Notify>) {
    let records = Arc::new(Mutex::new(Vec::new()));
    let pending = Arc::new(Notify::new());
    (
        Observed {
            inner: Some(inner),
            owner: owner.clone(),
            shared: shared.clone(),
            records: Arc::clone(&records),
            pending: Arc::clone(&pending),
        },
        records,
        pending,
    )
}

impl<S> Drop for Observed<S> {
    fn drop(&mut self) {
        drop(self.inner.take());
        self.records
            .lock()
            .unwrap()
            .push((self.owner.active(), self.shared.available()));
    }
}

impl<S: AsyncRead + Unpin> AsyncRead for Observed<S> {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buffer: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        let result = Pin::new(self.inner.as_mut().unwrap()).poll_read(cx, buffer);
        if result.is_pending() {
            self.pending.notify_one();
        }
        result
    }
}

impl<S: AsyncWrite + Unpin> AsyncWrite for Observed<S> {
    fn poll_write(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        bytes: &[u8],
    ) -> Poll<io::Result<usize>> {
        let result = Pin::new(self.inner.as_mut().unwrap()).poll_write(cx, bytes);
        if result.is_pending() {
            self.pending.notify_one();
        }
        result
    }

    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(self.inner.as_mut().unwrap()).poll_flush(cx)
    }

    fn poll_shutdown(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(self.inner.as_mut().unwrap()).poll_shutdown(cx)
    }
}

async fn tcp_pair() -> (TcpStream, TcpStream) {
    let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
    let (client, peer) = tokio::join!(
        TcpStream::connect(listener.local_addr().unwrap()),
        listener.accept(),
    );
    (client.unwrap(), peer.unwrap().0)
}

async fn eof<S: AsyncRead + Unpin>(mut peer: S) -> Vec<u8> {
    tokio::time::timeout(OUTER, async {
        let mut received = Vec::new();
        peer.read_to_end(&mut received).await.unwrap();
        received
    })
    .await
    .unwrap()
}

fn find_source<'a, T: Error + 'static>(error: &'a (dyn Error + 'static)) -> Option<&'a T> {
    let mut current = Some(error);
    for _ in 0..32 {
        let error = current?;
        if let Some(found) = error.downcast_ref::<T>() {
            return Some(found);
        }
        current = if let Some(io) = error.downcast_ref::<io::Error>() {
            io.get_ref()
                .map(|source| source as &(dyn Error + 'static))
                .or_else(|| error.source())
        } else {
            error.source()
        };
    }
    None
}

fn has_elapsed(error: &CoreError) -> bool {
    find_source::<tokio::time::error::Elapsed>(error).is_some()
}

#[tokio::test]
async fn admission_zero_overflow_and_precancel_leave_transport_unpolled() {
    let shared = SharedAdmission::new(1).unwrap();
    let owner = SessionOwner::new(shared.clone(), 1).unwrap();
    let mut outcomes = Vec::new();
    for (budget, pre_cancel) in [
        (Duration::ZERO, false),
        (Duration::MAX, false),
        (TOTAL, true),
    ] {
        let (stream, peer) = tcp_pair().await;
        let cancel = OperationCancel::new();
        if pre_cancel {
            cancel.cancel();
        }
        let result = owner.try_begin(OperationOptions::new(budget), cancel);
        let error = match result {
            Err(error) => Some(error),
            Ok(lease) => {
                drop(lease);
                None
            }
        };
        drop(stream);
        let received = eof(peer).await;
        outcomes.push((error, received));
    }
    owner.close(OUTER).await.unwrap();
    assert_eq!(owner.active(), 0);
    assert_eq!(shared.available(), 1);
    let expected = [
        ErrorClass::Timeout,
        ErrorClass::InvalidInput,
        ErrorClass::Cancelled,
    ];
    for ((error, received), expected) in outcomes.into_iter().zip(expected) {
        assert_eq!(error.unwrap().class(), expected);
        assert!(received.is_empty());
    }
}

#[tokio::test]
async fn prepared_factory_is_not_called_after_cancel_close_or_expiry() {
    for reason in 0..3 {
        let shared = SharedAdmission::new(1).unwrap();
        let owner = SessionOwner::new(shared.clone(), 1).unwrap();
        let cancel = OperationCancel::new();
        let budget = if reason == 2 {
            Duration::from_millis(1)
        } else {
            TOTAL
        };
        let lease = owner
            .try_begin(OperationOptions::new(budget), cancel.clone())
            .unwrap();
        let (stream, peer) = tcp_pair().await;
        let (stream, records, _) = observed(stream, &owner, &shared);
        let calls = Arc::new(AtomicUsize::new(0));
        let called = Arc::clone(&calls);
        let prepared = lease.run(move |context| {
            called.fetch_add(1, Ordering::SeqCst);
            Box::pin(async move {
                let mut stream = context.guard(stream);
                stream.write_all(b"must not be sent").await?;
                Ok(())
            })
        });
        match reason {
            0 => cancel.cancel(),
            1 => owner.request_close(),
            _ => tokio::time::sleep(Duration::from_millis(10)).await,
        }
        let result = tokio::time::timeout(OUTER, prepared).await.unwrap();
        let received = eof(peer).await;
        owner.close(OUTER).await.unwrap();
        let error = result.unwrap_err();
        assert_eq!(calls.load(Ordering::SeqCst), 0);
        assert!(received.is_empty());
        assert_eq!(*records.lock().unwrap(), vec![(1, 0)]);
        assert_eq!(owner.active(), 0);
        assert_eq!(shared.available(), 1);
        assert_eq!(
            error.class(),
            if reason == 2 {
                ErrorClass::Timeout
            } else {
                ErrorClass::Cancelled
            }
        );
        if reason == 2 {
            assert!(has_elapsed(&error));
        }
    }
}

#[tokio::test]
async fn unpolled_future_drop_destroys_original_stream_before_admission() {
    let shared = SharedAdmission::new(1).unwrap();
    let owner = SessionOwner::new(shared.clone(), 1).unwrap();
    let lease = owner
        .try_begin(OperationOptions::new(TOTAL), OperationCancel::new())
        .unwrap();
    let (stream, peer) = tcp_pair().await;
    let (stream, records, _) = observed(stream, &owner, &shared);
    let calls = Arc::new(AtomicUsize::new(0));
    let called = Arc::clone(&calls);
    let future = lease.run(move |context| {
        called.fetch_add(1, Ordering::SeqCst);
        Box::pin(async move {
            let mut stream = context.guard(stream);
            stream.read_exact(&mut [0]).await?;
            Ok(())
        })
    });
    drop(future);
    let received = eof(peer).await;
    owner.close(OUTER).await.unwrap();
    assert!(received.is_empty());
    assert_eq!(*records.lock().unwrap(), vec![(1, 0)]);
    assert_eq!(calls.load(Ordering::SeqCst), 0);
    assert_eq!(owner.active(), 0);
    assert_eq!(shared.available(), 1);
}

#[tokio::test]
async fn close_timeout_retains_unpolled_lease_and_repeat_close_waits_for_real_drop() {
    let shared = SharedAdmission::new(1).unwrap();
    let owner = SessionOwner::new(shared.clone(), 1).unwrap();
    let lease = owner
        .try_begin(OperationOptions::new(TOTAL), OperationCancel::new())
        .unwrap();
    let (stream, mut peer) = tcp_pair().await;
    let (stream, records, _) = observed(stream, &owner, &shared);
    let prepared = lease.run(move |context| {
        Box::pin(async move {
            let mut stream = context.guard(stream);
            stream.read_exact(&mut [0]).await?;
            Ok(())
        })
    });
    let first = owner.close(Duration::from_millis(10)).await;
    let retained = (owner.is_closed(), owner.active(), shared.available());
    let no_eof = tokio::time::timeout(Duration::from_millis(10), peer.read(&mut [0])).await;
    let rejected = owner.try_begin(OperationOptions::new(TOTAL), OperationCancel::new());
    let rejected = match rejected {
        Err(error) => Some(error),
        Ok(lease) => {
            drop(lease);
            None
        }
    };
    drop(prepared);
    let received = eof(peer).await;
    let second = owner.close(OUTER).await;
    let third = owner.close(OUTER).await;
    assert_eq!(retained, (true, 1, 0));
    assert!(no_eof.is_err());
    assert!(received.is_empty());
    assert!(has_elapsed(&first.unwrap_err()));
    assert_eq!(rejected.unwrap().class(), ErrorClass::Precondition);
    assert!(second.is_ok() && third.is_ok());
    assert_eq!(*records.lock().unwrap(), vec![(1, 0)]);
    assert_eq!(owner.active(), 0);
    assert_eq!(shared.available(), 1);
}

#[tokio::test]
async fn caller_drop_and_blocked_duplex_send_release_original_resources() {
    for blocked_send in [false, true] {
        let shared = SharedAdmission::new(1).unwrap();
        let owner = SessionOwner::new(shared.clone(), 1).unwrap();
        let cancel = OperationCancel::new();
        let lease = owner
            .try_begin(OperationOptions::new(TOTAL), cancel.clone())
            .unwrap();
        let (stream, peer) = tokio::io::duplex(8);
        let (stream, records, waiting) = observed(stream, &owner, &shared);
        let mut future = Box::pin(lease.run(move |context| {
            Box::pin(async move {
                let mut stream = context.guard(stream);
                if blocked_send {
                    stream.write_all(&[42; 64]).await?;
                } else {
                    stream.read_exact(&mut [0]).await?;
                }
                Ok(())
            })
        }));
        let early = tokio::select! {
            biased;
            _ = waiting.notified() => None,
            result = future.as_mut() => Some(result),
        };
        let was_pending = early.is_none();
        let result = if blocked_send && was_pending {
            cancel.cancel();
            Some(tokio::time::timeout(OUTER, future.as_mut()).await.unwrap())
        } else {
            early
        };
        drop(future);
        let received = eof(peer).await;
        owner.close(OUTER).await.unwrap();
        assert_eq!(*records.lock().unwrap(), vec![(1, 0)]);
        assert_eq!(owner.active(), 0);
        assert_eq!(shared.available(), 1);
        assert!(was_pending);
        if let Some(result) = result {
            assert_eq!(result.unwrap_err().class(), ErrorClass::Cancelled);
            assert_eq!(received, vec![42; 8]);
        } else {
            assert!(received.is_empty());
        }
    }
}

#[tokio::test]
async fn admission_has_no_queue_and_closing_one_owner_keeps_other_native_session() {
    let shared = SharedAdmission::new(2).unwrap();
    let a = SessionOwner::new(shared.clone(), 1).unwrap();
    let b = SessionOwner::new(shared.clone(), 1).unwrap();
    let c = SessionOwner::new(shared.clone(), 1).unwrap();
    let lease_a = a
        .try_begin(OperationOptions::new(TOTAL), OperationCancel::new())
        .unwrap();
    let busy = a.try_begin(OperationOptions::new(TOTAL), OperationCancel::new());
    let busy = match busy {
        Err(error) => Some(error),
        Ok(lease) => {
            drop(lease);
            None
        }
    };
    let lease_b = b
        .try_begin(OperationOptions::new(TOTAL), OperationCancel::new())
        .unwrap();
    let full = c.try_begin(OperationOptions::new(TOTAL), OperationCancel::new());
    let full = match full {
        Err(error) => Some(error),
        Ok(lease) => {
            drop(lease);
            None
        }
    };
    let (stream_a, peer_a) = tcp_pair().await;
    let (stream_b, mut peer_b) = tcp_pair().await;
    let (stream_a, drops_a, waiting_a) = observed(stream_a, &a, &shared);
    let (stream_b, drops_b, waiting_b) = observed(stream_b, &b, &shared);
    let task_a = tokio::spawn(lease_a.run(move |context| {
        Box::pin(async move {
            let mut stream = context.guard(stream_a);
            stream.read_exact(&mut [0]).await?;
            Ok(())
        })
    }));
    let task_b = tokio::spawn(lease_b.run(move |context| {
        Box::pin(async move {
            let mut stream = context.guard(stream_b);
            let mut byte = [0];
            stream.read_exact(&mut byte).await?;
            Ok(byte.to_vec())
        })
    }));
    tokio::time::timeout(OUTER, waiting_a.notified())
        .await
        .unwrap();
    tokio::time::timeout(OUTER, waiting_b.notified())
        .await
        .unwrap();
    let close_a = a.close(OUTER).await;
    let result_a = task_a.await.unwrap();
    let eof_a = eof(peer_a).await;
    let b_during_a_close = (b.active(), b.is_closed(), task_b.is_finished());
    peer_b.write_all(&[91]).await.unwrap();
    let result_b = task_b.await.unwrap();
    let eof_b = eof(peer_b).await;
    b.close(OUTER).await.unwrap();
    c.close(OUTER).await.unwrap();
    assert_eq!(busy.unwrap().class(), ErrorClass::Conflict);
    assert_eq!(full.unwrap().class(), ErrorClass::ResourceLimit);
    assert!(close_a.is_ok());
    assert_eq!(result_a.unwrap_err().class(), ErrorClass::Cancelled);
    assert_eq!(b_during_a_close, (1, false, false));
    assert_eq!(result_b.unwrap(), vec![91]);
    assert!(eof_a.is_empty() && eof_b.is_empty());
    assert_eq!(drops_a.lock().unwrap()[0].0, 1);
    assert_eq!(drops_b.lock().unwrap()[0].0, 1);
    assert_eq!(a.active() + b.active() + c.active(), 0);
    assert_eq!(shared.available(), 2);
}

#[tokio::test]
async fn last_owner_handle_drop_broadcasts_to_original_pending_tcp_job() {
    let shared = SharedAdmission::new(1).unwrap();
    let owner = SessionOwner::new(shared.clone(), 1).unwrap();
    let other_handle = owner.clone();
    let lease = owner
        .try_begin(OperationOptions::new(TOTAL), OperationCancel::new())
        .unwrap();
    let (stream, peer) = tcp_pair().await;
    let (started, ready) = oneshot::channel();
    let task = tokio::spawn(lease.run(move |context| {
        Box::pin(async move {
            let mut stream = context.guard(stream);
            let mut byte = [0];
            let mut read = Box::pin(stream.read_exact(&mut byte));
            let mut started = Some(started);
            std::future::poll_fn(|cx| match read.as_mut().poll(cx) {
                Poll::Pending => {
                    if let Some(started) = started.take() {
                        let _ = started.send(());
                    }
                    Poll::Pending
                }
                result => result,
            })
            .await?;
            Ok(())
        })
    }));
    let ready = tokio::time::timeout(OUTER, ready).await;
    drop(other_handle);
    let before_last = task.is_finished();
    drop(owner);
    let result = tokio::time::timeout(OUTER, task).await.unwrap().unwrap();
    let received = eof(peer).await;
    assert!(!before_last);
    assert!(matches!(ready, Ok(Ok(()))));
    assert_eq!(result.unwrap_err().class(), ErrorClass::Cancelled);
    assert!(received.is_empty());
    assert_eq!(shared.available(), 1);
}

struct AfterReady<F> {
    future: Pin<Box<F>>,
    cancel: OperationCancel,
    delay: Duration,
    do_cancel: bool,
}

impl<F: Future> Future for AfterReady<F> {
    type Output = F::Output;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let this = self.get_mut();
        let result = this.future.as_mut().poll(cx);
        if result.is_ready() {
            if this.do_cancel {
                this.cancel.cancel();
            }
            std::thread::sleep(this.delay);
        }
        result
    }
}

#[tokio::test]
async fn native_read_error_wins_same_poll_cancel_and_late_success_keeps_actual_elapsed() {
    for native_error in [false, true] {
        let shared = SharedAdmission::new(1).unwrap();
        let owner = SessionOwner::new(shared.clone(), 1).unwrap();
        let cancel = OperationCancel::new();
        let lease = owner
            .try_begin(
                OperationOptions::new(Duration::from_millis(250)),
                cancel.clone(),
            )
            .unwrap();
        let (stream, mut peer) = tcp_pair().await;
        let (stream, records, waiting) = observed(stream, &owner, &shared);
        let task = tokio::spawn(lease.run(move |context| {
            Box::pin(AfterReady {
                future: Box::pin(async move {
                    let mut stream = context.guard(stream);
                    if native_error {
                        stream.read_exact(&mut [0; 2]).await?;
                    } else {
                        stream.read_exact(&mut [0]).await?;
                    }
                    Ok(())
                }),
                cancel,
                delay: Duration::from_millis(300),
                do_cancel: native_error,
            })
        }));
        tokio::time::timeout(OUTER, waiting.notified())
            .await
            .unwrap();
        peer.write_all(&[7]).await.unwrap();
        let peer_task = if native_error {
            peer.shutdown().await.unwrap();
            tokio::spawn(eof(peer))
        } else {
            tokio::spawn(eof(peer))
        };
        let result = task.await.unwrap();
        let received = peer_task.await.unwrap();
        owner.close(OUTER).await.unwrap();
        let error = result.unwrap_err();
        assert!(received.is_empty());
        assert_eq!(*records.lock().unwrap(), vec![(1, 0)]);
        assert_eq!(owner.active(), 0);
        assert_eq!(shared.available(), 1);
        if native_error {
            assert_eq!(
                find_source::<io::Error>(&error).unwrap().kind(),
                io::ErrorKind::UnexpectedEof
            );
            assert_eq!(error.class(), ErrorClass::Io);
            assert!(!has_elapsed(&error));
        } else {
            assert_eq!(error.class(), ErrorClass::Timeout);
            assert!(has_elapsed(&error));
        }
    }
}

#[tokio::test]
async fn pending_total_timeout_drops_native_future_and_preserves_elapsed() {
    let shared = SharedAdmission::new(1).unwrap();
    let owner = SessionOwner::new(shared.clone(), 1).unwrap();
    let lease = owner
        .try_begin(
            OperationOptions::new(Duration::from_millis(30)),
            OperationCancel::new(),
        )
        .unwrap();
    let (stream, peer) = tokio::io::duplex(64);
    let (stream, records, waiting) = observed(stream, &owner, &shared);
    let task = tokio::spawn(lease.run(move |context| {
        Box::pin(async move {
            let mut stream = context.guard(stream);
            stream.read_exact(&mut [0]).await?;
            Ok(())
        })
    }));
    tokio::time::timeout(OUTER, waiting.notified())
        .await
        .unwrap();
    let result = task.await.unwrap();
    let received = eof(peer).await;
    owner.close(OUTER).await.unwrap();
    let error = result.unwrap_err();
    assert!(has_elapsed(&error));
    assert_eq!(error.class(), ErrorClass::Timeout);
    assert!(received.is_empty());
    assert_eq!(*records.lock().unwrap(), vec![(1, 0)]);
    assert_eq!(owner.active(), 0);
    assert_eq!(shared.available(), 1);
}

#[cfg(unix)]
#[tokio::test]
async fn real_tcp_reset_pending_cause_is_retained_and_handled_errors_do_not_pollute_results() {
    use std::os::fd::AsRawFd;
    // pending -> cancel; pending -> total; handled -> success; handled ->
    // typed terminal; handled + explicit acknowledgement -> pending cancel.
    for mode in 0..5 {
        let shared = SharedAdmission::new(1).unwrap();
        let owner = SessionOwner::new(shared.clone(), 1).unwrap();
        let cancel = OperationCancel::new();
        let budget = if mode == 1 {
            Duration::from_millis(500)
        } else {
            TOTAL
        };
        let lease = owner
            .try_begin(OperationOptions::new(budget), cancel.clone())
            .unwrap();
        let (stream, mut peer) = tcp_pair().await;
        let (stream, records, _) = observed(stream, &owner, &shared);
        let (reset, reset_now) = oneshot::channel();
        let peer_task = tokio::spawn(async move {
            peer.write_all(&[3]).await?;
            reset_now.await.map_err(io::Error::other)?;
            let linger = libc::linger {
                l_onoff: 1,
                l_linger: 0,
            };
            // A real SO_LINGER reset produces native Ready(Err), unlike the
            // zero-byte successful read from which read_exact creates EOF.
            let status = unsafe {
                libc::setsockopt(
                    peer.as_raw_fd(),
                    libc::SOL_SOCKET,
                    libc::SO_LINGER,
                    (&linger as *const libc::linger).cast(),
                    std::mem::size_of::<libc::linger>() as libc::socklen_t,
                )
            };
            let result = if status == 0 {
                Ok(())
            } else {
                Err(io::Error::last_os_error())
            };
            drop(peer);
            result
        });
        let (acquired, source_ready) = oneshot::channel();
        let trigger = cancel.clone();
        let task = tokio::spawn(lease.run(move |context| {
            Box::pin(async move {
                let mut stream = context.guard(stream);
                stream.read_exact(&mut [0]).await?;
                let _ = reset.send(());
                let result = stream.read_exact(&mut [0]).await.map(|_| ());
                let source = context.native.lock().unwrap().as_ref().cloned();
                let _ = acquired.send((source, result.as_ref().err().map(io::Error::kind)));
                if mode == 2 {
                    Ok(())
                } else if mode == 3 {
                    trigger.cancel();
                    Err(CoreError::Protocol("owned typed terminal outcome".into()))
                } else {
                    if mode == 4 {
                        context.discard_handled_native_error();
                    }
                    let _held_native_result = result;
                    pending::<Result<(), CoreError>>().await
                }
            })
        }));
        let acquired = tokio::time::timeout(OUTER, source_ready)
            .await
            .unwrap()
            .unwrap();
        if mode == 0 || mode == 4 {
            cancel.cancel();
        }
        let result = tokio::time::timeout(OUTER, task).await.unwrap().unwrap();
        let reset_result = peer_task.await.unwrap();
        owner.close(OUTER).await.unwrap();
        assert!(reset_result.is_ok());
        let original = acquired.0.expect("fixture must acquire real native reset");
        assert_eq!(acquired.1, Some(original.kind()));
        assert_eq!(original.kind(), io::ErrorKind::ConnectionReset);
        assert_eq!(*records.lock().unwrap(), vec![(1, 0)]);
        assert_eq!(owner.active(), 0);
        assert_eq!(shared.available(), 1);
        match mode {
            0 | 1 => {
                let error = result.unwrap_err();
                assert_eq!(error.class(), ErrorClass::Disconnected);
                let mut current: Option<&(dyn Error + 'static)> = Some(&error);
                let mut same_original = false;
                for _ in 0..32 {
                    let Some(error) = current else { break };
                    if let Some(io) = error.downcast_ref::<io::Error>() {
                        same_original |= std::ptr::eq(io, original.as_ref());
                        current = io
                            .get_ref()
                            .map(|source| source as &(dyn Error + 'static))
                            .or_else(|| error.source());
                    } else {
                        current = error.source();
                    }
                }
                assert!(same_original);
            }
            2 => assert!(result.is_ok()),
            3 => assert_eq!(result.unwrap_err().class(), ErrorClass::Protocol),
            _ => assert_eq!(result.unwrap_err().class(), ErrorClass::Cancelled),
        }
    }
}

fn pair_record() -> Arc<crate::PairRecord> {
    Arc::new(crate::PairRecord {
        device_certificate: Vec::new(),
        host_certificate: Vec::new(),
        host_private_key: Vec::new().into(),
        root_certificate: Vec::new(),
        host_id: "owned-synthetic-host".into(),
        system_buid: "owned-synthetic-buid".into(),
        wifi_mac_address: None,
    })
}

fn dictionary<const N: usize>(fields: [(&str, plist::Value); N]) -> plist::Dictionary {
    let mut dictionary = plist::Dictionary::new();
    for (key, value) in fields {
        dictionary.insert(key.into(), value);
    }
    dictionary
}

#[tokio::test]
async fn finite_prepared_entry_drives_existing_trust_wire_and_closes_original_stream() {
    let shared = SharedAdmission::new(1).unwrap();
    let owner = SessionOwner::new(shared.clone(), 1).unwrap();
    let lease = owner
        .try_begin(OperationOptions::new(TOTAL), OperationCancel::new())
        .unwrap();
    let (stream, mut peer) = tcp_pair().await;
    let (stream, records, _) = observed(stream, &owner, &shared);
    let peer_task = tokio::spawn(async move {
        let mut requests = Vec::new();
        for reply in [
            dictionary([
                ("Request", plist::Value::String("QueryType".into())),
                (
                    "Type",
                    plist::Value::String("com.apple.mobile.lockdown".into()),
                ),
            ]),
            dictionary([
                ("Request", plist::Value::String("StartSession".into())),
                ("SessionID", plist::Value::String("private-session".into())),
                ("EnableSessionSSL", plist::Value::Boolean(false)),
            ]),
            dictionary([
                ("Request", plist::Value::String("StartService".into())),
                ("Port", plist::Value::Integer(12345u64.into())),
                ("EnableServiceSSL", plist::Value::Boolean(true)),
            ]),
        ] {
            let request: plist::Dictionary = crate::recv_lockdown(&mut peer).await.unwrap();
            requests.push(request);
            crate::send_lockdown(&mut peer, &reply).await.unwrap();
        }
        (requests, eof(peer).await)
    });
    let operation = prepared_service_observation(
        stream,
        pair_record(),
        lease,
        "com.apple.mobile.installation_proxy".into(),
    )
    .unwrap();
    let result = tokio::time::timeout(OUTER, operation).await.unwrap();
    let (requests, extra) = peer_task.await.unwrap();
    owner.close(OUTER).await.unwrap();
    assert_eq!(
        result.unwrap(),
        ServiceObservation {
            port: 12345,
            enable_service_ssl: true,
            enable_session_ssl: false,
        }
    );
    assert_eq!(
        requests
            .iter()
            .map(|request| request["Request"].as_string().unwrap())
            .collect::<Vec<_>>(),
        vec!["QueryType", "StartSession", "StartService"]
    );
    assert_eq!(
        requests[2]["Service"].as_string(),
        Some("com.apple.mobile.installation_proxy")
    );
    assert!(extra.is_empty());
    assert_eq!(*records.lock().unwrap(), vec![(1, 0)]);
    assert_eq!(owner.active(), 0);
    assert_eq!(shared.available(), 1);
}

#[tokio::test]
async fn selector_rejection_and_unpolled_prepared_entry_destroy_transport_before_lease() {
    let mut results = Vec::new();
    for service in [
        String::new(),
        "embedded\0selector".into(),
        "x".repeat(4097),
        "valid.selector".into(),
    ] {
        let shared = SharedAdmission::new(1).unwrap();
        let owner = SessionOwner::new(shared.clone(), 1).unwrap();
        let lease = owner
            .try_begin(OperationOptions::new(TOTAL), OperationCancel::new())
            .unwrap();
        let (stream, peer) = tcp_pair().await;
        let (stream, records, _) = observed(stream, &owner, &shared);
        let prepared = prepared_service_observation(stream, pair_record(), lease, service);
        let error = match prepared {
            Err(error) => Some(error),
            Ok(future) => {
                drop(future);
                None
            }
        };
        let received = eof(peer).await;
        owner.close(OUTER).await.unwrap();
        results.push((
            error,
            received,
            records.lock().unwrap().clone(),
            owner.active(),
            shared.available(),
        ));
    }
    for (index, (error, received, records, active, available)) in results.into_iter().enumerate() {
        assert!(received.is_empty());
        assert_eq!(records, vec![(1, 0)]);
        assert_eq!((active, available), (0, 1));
        match index {
            0 | 1 => assert_eq!(error.unwrap().class(), ErrorClass::InvalidInput),
            2 => assert_eq!(error.unwrap().class(), ErrorClass::ResourceLimit),
            _ => assert!(error.is_none()),
        }
    }
}
