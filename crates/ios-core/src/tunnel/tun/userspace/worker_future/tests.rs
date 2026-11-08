use super::*;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;
use tokio::io::AsyncReadExt;

fn report(error: &TunnelError) -> &crate::TunnelShutdownFailure {
    let mut current: &(dyn std::error::Error + 'static) = error;
    loop {
        if let Some(report) = current.downcast_ref() {
            return report;
        }
        current = current.source().expect("typed native report missing");
    }
}

struct Pending {
    bank: Arc<Semaphore>,
    address: Arc<AtomicUsize>,
    dropped: Arc<AtomicUsize>,
}
impl Future for Pending {
    type Output = Result<(), TunnelError>;
    fn poll(self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<Self::Output> {
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
            "payload fee returned before original drop"
        );
        let address = self.address.load(Ordering::SeqCst);
        if address != 0 {
            assert_eq!(
                self as *const Self as usize, address,
                "pinned original moved"
            );
        }
        self.dropped.fetch_add(1, Ordering::SeqCst);
    }
}

#[tokio::test]
async fn exact_payload_stays_paid_through_poll_cancel_and_original_drop() {
    let bytes = std::mem::size_of::<Pending>();
    let bank = Arc::new(Semaphore::new(bytes));
    let address = Arc::new(AtomicUsize::new(0));
    let dropped = Arc::new(AtomicUsize::new(0));
    let future = Pending {
        bank: Arc::clone(&bank),
        address: Arc::clone(&address),
        dropped: Arc::clone(&dropped),
    };
    let prepared = PreparedWorker::prepare(future, Some(&bank)).unwrap();
    assert_eq!(bank.available_permits(), 0);
    let task = prepared.spawn(ParentRole::Writer, Ledger::new());
    tokio::time::timeout(Duration::from_secs(1), async {
        while address.load(Ordering::SeqCst) == 0 {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    task.abort();
    task.join().await;
    assert_eq!(dropped.load(Ordering::SeqCst), 1);
    assert_eq!(bank.available_permits(), bytes);
    task.join().await;
    assert_eq!(bank.available_permits(), bytes);
}

#[test]
fn one_byte_refusal_drops_original_without_allocation_or_worker() {
    struct Original(Arc<AtomicUsize>);
    impl Drop for Original {
        fn drop(&mut self) {
            self.0.fetch_add(1, Ordering::SeqCst);
        }
    }
    let dropped = Arc::new(AtomicUsize::new(0));
    let original = Original(Arc::clone(&dropped));
    let future = async move {
        drop(original);
        Ok::<(), TunnelError>(())
    };
    let bytes = std::mem::size_of_val(&future);
    let bank = Arc::new(Semaphore::new(bytes - 1));
    let error = PreparedWorker::prepare(future, Some(&bank)).err().unwrap();
    assert_eq!(
        crate::classify_error(&error),
        crate::ErrorClass::ResourceLimit
    );
    assert_eq!(dropped.load(Ordering::SeqCst), 1);
    assert_eq!(bank.available_permits(), bytes - 1);
}

#[tokio::test]
async fn fourth_actual_parent_payload_refusal_starts_no_workers_and_refunds_first_three() {
    let baseline = tokio::runtime::Handle::current()
        .metrics()
        .num_alive_tasks();
    let quota = 16 * 1024 * 1024;
    let bank = Arc::new(Semaphore::new(quota));
    let held = Arc::new(std::sync::Mutex::new(None));
    let count = Arc::new(AtomicUsize::new(0));
    let observed = Arc::clone(&count);
    let observed_bank = Arc::clone(&bank);
    let hold = Arc::clone(&held);
    ADMISSION.with(|hook| {
        *hook.borrow_mut() = Some(Box::new(move |bytes| {
            if observed.fetch_add(1, Ordering::SeqCst) == 3 {
                let take = observed_bank.available_permits() - (bytes - 1);
                *hold.lock().unwrap() = Some(
                    Arc::clone(&observed_bank)
                        .try_acquire_many_owned(take.try_into().unwrap())
                        .unwrap(),
                );
            }
        }))
    });
    let (stream, mut peer) = tokio::io::duplex(1280);
    let result = super::super::UserspaceTunDevice::start_validated_with_bank(
        "fd00::2",
        "fd00::1",
        super::super::ValidatedMtu::new(1280).unwrap(),
        stream,
        Some(Arc::clone(&bank)),
    )
    .await;
    ADMISSION.with(|hook| hook.borrow_mut().take());
    let error = result.err().expect("fourth shortage started workers");
    assert_eq!(
        crate::classify_error(&error),
        crate::ErrorClass::ResourceLimit
    );
    assert_eq!(count.load(Ordering::SeqCst), 4);
    assert_eq!(
        tokio::runtime::Handle::current()
            .metrics()
            .num_alive_tasks(),
        baseline
    );
    let held_bytes = held.lock().unwrap().as_ref().unwrap().num_permits();
    assert_eq!(bank.available_permits(), quota - held_bytes);
    drop(held.lock().unwrap().take());
    assert_eq!(bank.available_permits(), quota);
    let mut byte = [0];
    assert!(matches!(
        tokio::time::timeout(Duration::from_secs(1), peer.read(&mut byte)).await,
        Ok(Ok(0))
    ));
}

#[tokio::test]
async fn original_child_joinset_keeps_payload_paid_through_cancel_and_drain() {
    let bank = Arc::new(Semaphore::new(0));
    let address = Arc::new(AtomicUsize::new(0));
    let dropped = Arc::new(AtomicUsize::new(0));
    let pending = Pending {
        bank: Arc::clone(&bank),
        address: Arc::clone(&address),
        dropped: Arc::clone(&dropped),
    };
    let future = async move {
        let _ = pending.await;
    };
    let bytes = std::mem::size_of_val(&future);
    bank.add_permits(bytes);
    let prepared = PreparedWorker::prepare(future, Some(&bank)).unwrap();
    let ledger = Ledger::new();
    let clients =
        super::super::SharedChildTaskSet::new(ledger.clone(), super::super::ChildRole::ProxyClient);
    let mut guard = clients.lock().await;
    prepared.spawn_child(&mut guard);
    tokio::time::timeout(Duration::from_secs(1), async {
        while address.load(Ordering::SeqCst) == 0 {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    assert_eq!(bank.available_permits(), 0);
    guard.abort_all();
    while guard.join_next().await.is_some() {}
    assert_eq!(dropped.load(Ordering::SeqCst), 1);
    assert_eq!(bank.available_permits(), bytes);
    ledger.result(None).unwrap();
}

#[tokio::test]
async fn actual_client_payload_refusal_keeps_original_fatal_joins_and_both_strict_eofs() {
    let baseline = tokio::runtime::Handle::current()
        .metrics()
        .num_alive_tasks();
    let quota = 16 * 1024 * 1024;
    let bank = Arc::new(Semaphore::new(quota));
    let held = Arc::new(std::sync::Mutex::new(None));
    let count = Arc::new(AtomicUsize::new(0));
    let observed = Arc::clone(&count);
    let observed_bank = Arc::clone(&bank);
    let hold = Arc::clone(&held);
    ADMISSION.with(|hook| {
        *hook.borrow_mut() = Some(Box::new(move |bytes| {
            if observed.fetch_add(1, Ordering::SeqCst) == 4 {
                let take = observed_bank.available_permits() - (bytes - 1);
                *hold.lock().unwrap() = Some(
                    Arc::clone(&observed_bank)
                        .try_acquire_many_owned(take.try_into().unwrap())
                        .unwrap(),
                );
            }
        }))
    });
    let (stream, mut peer) = tokio::io::duplex(1280);
    let runtime = super::super::UserspaceTunDevice::start_validated_with_bank(
        "fd00::2",
        "fd00::1",
        super::super::ValidatedMtu::new(1280).unwrap(),
        stream,
        Some(Arc::clone(&bank)),
    )
    .await
    .unwrap();
    let mut client = tokio::net::TcpStream::connect(("127.0.0.1", runtime.local_port))
        .await
        .unwrap();
    tokio::time::timeout(Duration::from_secs(1), async {
        while runtime.is_alive() {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    ADMISSION.with(|hook| hook.borrow_mut().take());
    assert_eq!(
        count.load(Ordering::SeqCst),
        5,
        "not the actual child payload admission"
    );
    for error in [
        runtime.shutdown(Duration::from_secs(1)).await.unwrap_err(),
        runtime.shutdown(Duration::from_secs(1)).await.unwrap_err(),
    ] {
        assert_eq!(
            crate::classify_error(&error),
            crate::ErrorClass::ResourceLimit
        );
        let report = report(&error);
        assert!(report.joins_complete());
        let TunnelError::Io(original) = report.primary().downcast_ref::<TunnelError>().unwrap()
        else {
            panic!("original cause missing");
        };
        assert!(matches!(
            original
                .get_ref()
                .unwrap()
                .downcast_ref::<super::super::owned_buffer::BufferAdmission>(),
            Some(super::super::owned_buffer::BufferAdmission::Exhausted)
        ));
    }
    let mut byte = [0];
    assert!(matches!(
        tokio::time::timeout(Duration::from_secs(1), peer.read(&mut byte)).await,
        Ok(Ok(0))
    ));
    assert!(matches!(
        tokio::time::timeout(Duration::from_secs(1), client.read(&mut byte)).await,
        Ok(Ok(0))
    ));
    assert_eq!(
        tokio::runtime::Handle::current()
            .metrics()
            .num_alive_tasks(),
        baseline
    );
    drop(runtime);
    drop(held.lock().unwrap().take());
    assert_eq!(bank.available_permits(), quota);
}

#[test]
fn legacy_prepared_worker_boxes_large_future_off_stack() {
    let payload = [0u8; 65_536];
    let future = async move {
        let _ = payload[0];
        Ok::<(), TunnelError>(())
    };
    let prepared = PreparedWorker::prepare(future, None).unwrap();
    assert!(
        std::mem::size_of_val(&prepared) <= 64,
        "PreparedWorker should stay pointer-sized on the stack"
    );
}
