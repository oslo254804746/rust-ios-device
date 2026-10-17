use super::*;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Barrier;

#[derive(Debug)]
struct Probe {
    bank: Arc<Semaphore>,
    drops: Arc<AtomicUsize>,
    expected_free: usize,
}
impl std::fmt::Display for Probe {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("original IO drop probe")
    }
}
impl std::error::Error for Probe {}
impl Drop for Probe {
    fn drop(&mut self) {
        assert_eq!(self.bank.available_permits(), self.expected_free);
        self.drops.fetch_add(1, Ordering::SeqCst);
    }
}
fn captured(
    bank: &Arc<Semaphore>,
    drops: Arc<AtomicUsize>,
    expected_free: usize,
) -> (Shared<Original>, usize) {
    let storage = super::super::super::admission::known_metadata_charge(
        bank,
        std::mem::size_of::<Original>(),
    )
    .unwrap();
    let original = Shared::new(Original {
        error: OnceLock::new(),
        _storage: storage,
    });
    let error = anyhow::Error::new(std::io::Error::new(
        std::io::ErrorKind::ConnectionReset,
        Probe {
            bank: bank.clone(),
            drops,
            expected_free,
        },
    ));
    let pointer = error.downcast_ref::<std::io::Error>().unwrap() as *const _ as usize;
    assert!(original.error.set(error).is_ok());
    (original, pointer)
}

#[test]
fn original_capsule_parallel_last_history_clones_keep_typed_io_and_drop_before_refund_once() {
    let total = std::mem::size_of::<Original>();
    let bank = Arc::new(Semaphore::new(total));
    let drops = Arc::new(AtomicUsize::new(0));
    let (original, pointer) = captured(&bank, drops.clone(), 0);
    assert_eq!(
        std::mem::size_of::<Shared<Original>>(),
        std::mem::size_of::<Arc<Original>>()
    );
    let mut owners = (0..7).map(|_| original.clone()).collect::<Vec<_>>();
    owners.push(original);
    let barrier = Arc::new(Barrier::new(8));
    std::thread::scope(|scope| {
        for owner in owners {
            let barrier = barrier.clone();
            scope.spawn(move || {
                assert_eq!(
                    owner.error().downcast_ref::<std::io::Error>().unwrap() as *const _ as usize,
                    pointer
                );
                assert_eq!(
                    owner
                        .error()
                        .downcast_ref::<std::io::Error>()
                        .unwrap()
                        .kind(),
                    std::io::ErrorKind::ConnectionReset
                );
                barrier.wait();
                drop(owner);
            });
        }
    });
    assert_eq!(drops.load(Ordering::SeqCst), 1);
    assert_eq!(bank.available_permits(), total);
}

#[test]
fn original_capsule_first_history_and_latest_have_independent_original_io_and_final_grants() {
    let bytes = std::mem::size_of::<Original>();
    let bank = Arc::new(Semaphore::new(2 * bytes));
    let drops = Arc::new(AtomicUsize::new(0));
    let (first, first_pointer) = captured(&bank, drops.clone(), 0);
    let (latest, latest_pointer) = captured(&bank, drops.clone(), bytes);
    let history = History {
        first: first.clone(),
        failures: 2,
    };
    assert_ne!(first_pointer, latest_pointer);
    drop(first);
    assert_eq!(drops.load(Ordering::SeqCst), 0);
    assert_eq!(bank.available_permits(), 0);
    assert_eq!(
        history
            .first
            .error()
            .downcast_ref::<std::io::Error>()
            .unwrap() as *const _ as usize,
        first_pointer
    );
    assert_eq!(history.failures, 2);
    drop(history);
    assert_eq!(drops.load(Ordering::SeqCst), 1);
    assert_eq!(bank.available_permits(), bytes);
    assert_eq!(
        latest.error().downcast_ref::<std::io::Error>().unwrap() as *const _ as usize,
        latest_pointer
    );
    drop(latest);
    assert_eq!(drops.load(Ordering::SeqCst), 2);
    assert_eq!(bank.available_permits(), 2 * bytes);
}
