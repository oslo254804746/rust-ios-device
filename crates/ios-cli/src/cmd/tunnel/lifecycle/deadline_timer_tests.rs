use super::*;
use std::sync::atomic::{AtomicUsize, Ordering};
use tokio::time::error::Elapsed;

#[tokio::test]
async fn inline_deadline_timer_external_clamp_after_pending_never_repolls_original_work() {
    let clock = ConnectionDeadline::new(Duration::from_secs(2)).unwrap();
    let polls = AtomicUsize::new(0);
    let work = clock.run(std::future::poll_fn(|_| {
        polls.fetch_add(1, Ordering::SeqCst);
        std::task::Poll::<Result<()>>::Pending
    }));
    tokio::pin!(work);
    assert!(futures_util::poll!(&mut work).is_pending());
    assert_eq!(polls.load(Ordering::SeqCst), 1);
    clock.clamp_retry(Instant::now());
    let error = tokio::time::timeout(Duration::from_secs(1), work)
        .await
        .expect("new absolute timer was not registered")
        .unwrap_err();
    assert!(error.is::<Elapsed>());
    assert_eq!(polls.load(Ordering::SeqCst), 1);
}

#[derive(Debug)]
struct OriginalValue(Arc<AtomicUsize>);
impl Drop for OriginalValue {
    fn drop(&mut self) {
        self.0.fetch_add(1, Ordering::SeqCst);
    }
}

#[tokio::test]
async fn inline_deadline_timer_same_poll_clamp_discards_late_success_but_keeps_original_ready_io() {
    for native_error in [false, true] {
        let clock = ConnectionDeadline::new(Duration::from_secs(2)).unwrap();
        let polls = AtomicUsize::new(0);
        let drops = Arc::new(AtomicUsize::new(0));
        let original = anyhow::Error::new(std::io::Error::from(std::io::ErrorKind::BrokenPipe));
        let pointer = original.downcast_ref::<std::io::Error>().unwrap() as *const _ as usize;
        let run = clock.run(async {
            polls.fetch_add(1, Ordering::SeqCst);
            clock.clamp_retry(Instant::now());
            if native_error {
                Err(original)
            } else {
                Ok(OriginalValue(drops.clone()))
            }
        });
        let error = tokio::time::timeout(Duration::from_secs(1), run)
            .await
            .expect("same-poll retry clamp did not wake the independent timer")
            .unwrap_err();
        if native_error {
            assert_eq!(
                error.downcast_ref::<std::io::Error>().unwrap() as *const _ as usize,
                pointer
            );
            assert_eq!(drops.load(Ordering::SeqCst), 0);
        } else {
            assert!(error.is::<Elapsed>());
            assert_eq!(drops.load(Ordering::SeqCst), 1);
        }
        assert_eq!(polls.load(Ordering::SeqCst), 1);
    }
}
