use super::*;
use std::pin::Pin;
use std::sync::atomic::AtomicUsize;
use std::task::Context;

fn poll<F: Future>(future: Pin<&mut F>) -> Poll<F::Output> {
    future.poll(&mut Context::from_waker(
        futures_util::task::noop_waker_ref(),
    ))
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
    assert_eq!(Arc::as_ptr(&life) as usize, pointer.load(Ordering::SeqCst));
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
