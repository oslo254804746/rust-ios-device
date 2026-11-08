use super::*;
use std::pin::Pin;
use std::sync::atomic::AtomicUsize;
use std::task::Context;

fn poll<F: Future>(future: Pin<&mut F>) -> Poll<F::Output> {
    future.poll(&mut Context::from_waker(
        futures_util::task::noop_waker_ref(),
    ))
}

#[tokio::test]
async fn publication_dual_body_closure_drops_original_pending_work_while_weak_retains_both_fees() {
    struct PendingOwner(Arc<AtomicUsize>);
    impl Drop for PendingOwner {
        fn drop(&mut self) {
            self.0.fetch_add(1, Ordering::SeqCst);
        }
    }
    let bytes = Life::bytes();
    let global = Arc::new(Semaphore::new(bytes));
    let published = Arc::new(Semaphore::new(bytes));
    let life = Life::new(&global, &published).unwrap();
    assert!(life.belongs_to(&global));
    assert!(life.banks_match(&global, &published));
    assert!(!life.belongs_to(&published));
    let pointer = &*life as *const Life as usize;
    let weak = life.downgrade();
    let last = weak.clone();
    global.close();
    published.close();
    let drops = Arc::new(AtomicUsize::new(0));
    {
        let original = PendingOwner(drops.clone());
        let mut work = std::pin::pin!(life.run(async move {
            let _original = original;
            std::future::pending::<Result<()>>().await
        }));
        assert!(poll(work.as_mut()).is_pending());
        let closer = weak.upgrade().unwrap();
        assert_eq!(&*closer as *const Life as usize, pointer);
        assert!(closer.ptr_eq(&life));
        closer.close();
        assert!(work.await.unwrap_err().is::<ios_core::OperationCancelled>());
        drop(closer);
    }
    assert_eq!(drops.load(Ordering::SeqCst), 1);
    assert_eq!(global.available_permits(), 0);
    assert_eq!(published.available_permits(), 0);
    drop(life);
    assert!(weak.upgrade().is_none());
    assert_eq!(global.available_permits(), 0);
    assert_eq!(published.available_permits(), 0);
    drop(weak);
    assert_eq!(global.available_permits(), 0);
    assert_eq!(published.available_permits(), 0);
    drop(last);
    assert_eq!(global.available_permits(), bytes);
    assert_eq!(published.available_permits(), bytes);
}

#[test]
fn publication_dual_body_parallel_weak_upgrades_keep_original_closed_identity_and_last_refunds() {
    let bytes = Life::bytes();
    let global = Arc::new(Semaphore::new(bytes));
    let published = Arc::new(Semaphore::new(bytes));
    let life = Life::new(&global, &published).unwrap();
    let pointer = &*life as *const Life as usize;
    life.close();
    let weak = life.downgrade();
    global.close();
    published.close();
    let barrier = Arc::new(std::sync::Barrier::new(9));
    std::thread::scope(|scope| {
        for _ in 0..8 {
            let weak = weak.clone();
            let barrier = barrier.clone();
            scope.spawn(move || {
                barrier.wait();
                for _ in 0..128 {
                    if let Some(root) = weak.upgrade() {
                        assert_eq!(&*root as *const Life as usize, pointer);
                        assert!(root
                            .check()
                            .unwrap_err()
                            .is::<ios_core::OperationCancelled>());
                    }
                }
            });
        }
        barrier.wait();
        drop(life);
    });
    assert!(weak.upgrade().is_none());
    assert_eq!(global.available_permits(), 0);
    assert_eq!(published.available_permits(), 0);
    drop(weak);
    assert_eq!(global.available_permits(), bytes);
    assert_eq!(published.available_permits(), bytes);
}

#[test]
fn publication_lifetime_closed_flag_prevents_another_work_poll_before_notifier_runs() {
    let global = Arc::new(Semaphore::new(4096));
    let published = Arc::new(Semaphore::new(4096));
    let life = Life::new(&global, &published).unwrap();
    let polls = AtomicUsize::new(0);
    {
        let mut work = std::pin::pin!(life.run(std::future::poll_fn(|_| {
            polls.fetch_add(1, Ordering::SeqCst);
            Poll::<Result<()>>::Pending
        })));
        assert!(poll(work.as_mut()).is_pending());
        // Model a closer thread being preempted between its atomic store and
        // notify_waiters, while an IO wake causes another parent poll.
        life.closed.store(true, Ordering::Release);
        assert!(matches!(poll(work.as_mut()), Poll::Ready(Err(_))));
    }
    assert_eq!(polls.load(Ordering::SeqCst), 1);
    drop(life);
    assert_eq!(global.available_permits(), 4096);
    assert_eq!(published.available_permits(), 4096);
}

#[test]
fn publication_lifetime_body_is_paid_in_both_banks_before_creation_until_last_closed_owner() {
    let bytes = Life::bytes();
    let global = Arc::new(Semaphore::new(bytes));
    let published = Arc::new(Semaphore::new(bytes));
    let g = global.clone();
    let p = published.clone();
    let pointer = Arc::new(AtomicUsize::new(0));
    let captured = pointer.clone();
    let _hook = on_created(move |created| {
        assert_eq!(g.available_permits(), 0);
        assert_eq!(p.available_permits(), 0);
        captured.store(created, Ordering::SeqCst);
    });
    let life = Life::new(&global, &published).unwrap();
    assert_eq!(
        (&*life as *const Life) as usize,
        pointer.load(Ordering::SeqCst)
    );
    let last = life.clone();
    life.close();
    life.close();
    drop(life);
    assert!(last.check().is_err());
    assert_eq!(global.available_permits(), 0);
    assert_eq!(published.available_permits(), 0);
    drop(last);
    assert_eq!(global.available_permits(), bytes);
    assert_eq!(published.available_permits(), bytes);
}

#[test]
fn publication_lifetime_each_bank_one_byte_refusal_creates_nothing_and_refunds_other_bank() {
    let bytes = Life::bytes();
    for (g, p) in [(bytes - 1, bytes), (bytes, bytes - 1)] {
        let global = Arc::new(Semaphore::new(g));
        let published = Arc::new(Semaphore::new(p));
        let _hook = on_created(|_| panic!("refused publication body created"));
        let error = Life::new(&global, &published).unwrap_err();
        assert!(error.is::<Admission>());
        assert_eq!(
            crate::output::operation_error_class(error.as_ref()),
            ios_core::ErrorClass::ResourceLimit
        );
        assert_eq!(global.available_permits(), g);
        assert_eq!(published.available_permits(), p);
    }
}

#[tokio::test]
async fn publication_lifetime_closed_before_first_poll_never_runs_work_and_pending_work_is_dropped()
{
    struct Held(Arc<AtomicUsize>);
    impl Drop for Held {
        fn drop(&mut self) {
            self.0.fetch_add(1, Ordering::SeqCst);
        }
    }
    let global = Arc::new(Semaphore::new(4096));
    let published = Arc::new(Semaphore::new(4096));
    let life = Life::new(&global, &published).unwrap();
    let drops = Arc::new(AtomicUsize::new(0));
    {
        let mut pending = std::pin::pin!(life.run(async {
            let _held = Held(drops.clone());
            std::future::pending::<Result<()>>().await
        }));
        assert!(poll(pending.as_mut()).is_pending());
        life.close();
        assert!(pending
            .await
            .unwrap_err()
            .is::<ios_core::OperationCancelled>());
    }
    assert_eq!(drops.load(Ordering::SeqCst), 1);
    let polls = AtomicUsize::new(0);
    let error = life
        .run(async {
            polls.fetch_add(1, Ordering::SeqCst);
            Ok(())
        })
        .await
        .unwrap_err();
    assert!(error.is::<ios_core::OperationCancelled>());
    assert_eq!(polls.load(Ordering::SeqCst), 0);
    drop(life);
    assert_eq!(global.available_permits(), 4096);
    assert_eq!(published.available_permits(), 4096);
}

#[tokio::test]
async fn publication_lifetime_ready_original_io_crossing_close_keeps_typed_pointer() {
    let global = Arc::new(Semaphore::new(4096));
    let published = Arc::new(Semaphore::new(4096));
    let life = Life::new(&global, &published).unwrap();
    let original = anyhow::Error::new(std::io::Error::new(
        std::io::ErrorKind::BrokenPipe,
        "original-publication-IO",
    ));
    let pointer = original.downcast_ref::<std::io::Error>().unwrap() as *const _ as usize;
    let error = life
        .run(async {
            life.close();
            Err::<(), _>(original)
        })
        .await
        .unwrap_err();
    assert_eq!(
        error.downcast_ref::<std::io::Error>().unwrap() as *const _ as usize,
        pointer
    );
    assert_eq!(
        error.downcast_ref::<std::io::Error>().unwrap().kind(),
        std::io::ErrorKind::BrokenPipe
    );
    drop(life);
    assert_eq!(global.available_permits(), 4096);
    assert_eq!(published.available_permits(), 4096);
}
