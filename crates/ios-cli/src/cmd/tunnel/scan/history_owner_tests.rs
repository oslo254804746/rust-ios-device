use super::*;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Barrier;

#[derive(Debug)]
struct Probe {
    bank: Arc<Semaphore>,
    drops: Arc<AtomicUsize>,
    total: usize,
}
impl std::fmt::Display for Probe {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("original scan history IO probe")
    }
}
impl std::error::Error for Probe {}
impl Drop for Probe {
    fn drop(&mut self) {
        assert_eq!(
            self.bank.available_permits(),
            self.total - std::mem::size_of::<AbandonedScan>()
        );
        self.drops.fetch_add(1, Ordering::SeqCst);
    }
}

#[tokio::test]
async fn history_owner_parallel_last_snapshots_and_census_keep_original_root_and_independent_fees()
{
    let total = Outcomes::body_bytes() + capsule_reservation_bytes() + 128;
    let bank = Arc::new(Semaphore::new(total));
    let drops = Arc::new(AtomicUsize::new(0));
    let outcomes = Outcomes::with_metadata(bank.clone()).unwrap();
    let lock = tokio::sync::Mutex::new(());
    let session = outcomes.begin(lock.lock().await).unwrap();
    let original = anyhow::Error::new(std::io::Error::new(
        std::io::ErrorKind::ConnectionReset,
        Probe {
            bank: bank.clone(),
            drops: drops.clone(),
            total,
        },
    ));
    let pointer = original.downcast_ref::<std::io::Error>().unwrap() as *const _ as usize;
    session.retain(Source::Usb, original);
    drop(session);
    let snapshot = outcomes.history().unwrap();
    let (_, census) = outcomes.payload_history();
    let census = census.unwrap();
    assert!(census.as_slice()[0].ptr_eq(&snapshot[0]));
    let root = census.as_slice()[0].clone();
    let root_pointer = &*root as *const AbandonedScan as usize;
    drop(census);
    drop(outcomes);
    assert_eq!(drops.load(Ordering::SeqCst), 0);
    assert_eq!(
        bank.available_permits(),
        total - std::mem::size_of::<HistorySnapshot>() - std::mem::size_of::<AbandonedScan>()
    );
    let mut snapshots = (0..7).map(|_| snapshot.clone()).collect::<Vec<_>>();
    snapshots.push(snapshot);
    let barrier = Arc::new(Barrier::new(8));
    std::thread::scope(|scope| {
        for snapshot in snapshots {
            let barrier = barrier.clone();
            scope.spawn(move || {
                assert_eq!(&*snapshot[0] as *const AbandonedScan as usize, root_pointer);
                assert_eq!(
                    snapshot[0]
                        .original()
                        .downcast_ref::<std::io::Error>()
                        .unwrap() as *const _ as usize,
                    pointer
                );
                barrier.wait();
                drop(snapshot);
            });
        }
    });
    assert_eq!(drops.load(Ordering::SeqCst), 0);
    assert_eq!(
        bank.available_permits(),
        total - std::mem::size_of::<AbandonedScan>()
    );
    assert_eq!(
        root.original().downcast_ref::<std::io::Error>().unwrap() as *const _ as usize,
        pointer
    );
    drop(root);
    assert_eq!(drops.load(Ordering::SeqCst), 1);
    assert_eq!(bank.available_permits(), total);
}

#[tokio::test]
async fn history_owner_zero_quota_closed_bank_snapshots_keep_frozen_order_and_original_error_pointer(
) {
    let total = Outcomes::body_bytes() + 2 * capsule_reservation_bytes();
    let bank = Arc::new(Semaphore::new(total));
    let outcomes = Outcomes::with_metadata(bank.clone()).unwrap();
    let first_lock = tokio::sync::Mutex::new(());
    let second_lock = tokio::sync::Mutex::new(());
    let first = outcomes.begin(first_lock.lock().await).unwrap();
    let second = outcomes.begin(second_lock.lock().await).unwrap();
    assert_eq!(bank.available_permits(), 0);
    let first_io = anyhow::Error::new(std::io::Error::from(std::io::ErrorKind::PermissionDenied));
    let first_pointer = first_io.downcast_ref::<std::io::Error>().unwrap() as *const _ as usize;
    let second_io = anyhow::Error::new(std::io::Error::from(std::io::ErrorKind::ConnectionRefused));
    let second_pointer = second_io.downcast_ref::<std::io::Error>().unwrap() as *const _ as usize;
    first.retain(Source::Usb, first_io);
    second.retain(Source::Paired, second_io);
    drop(second);
    let older = outcomes.history().unwrap();
    let held = bank
        .clone()
        .try_acquire_many_owned(bank.available_permits() as u32)
        .unwrap();
    assert_eq!(bank.available_permits(), 0);
    outcomes.close();
    bank.close();
    drop(first);
    let newer = super::super::metadata_vec::with_allocation_hook(
        |_, _| panic!("history read allocated another destination"),
        || outcomes.history().unwrap(),
    );
    assert!(!older.ptr_eq(&newer));
    assert_eq!(older.len(), 1);
    assert_eq!(newer.len(), 2);
    assert!(older[0].ptr_eq(&newer[1]));
    assert_eq!(
        newer[0]
            .original()
            .downcast_ref::<std::io::Error>()
            .unwrap() as *const _ as usize,
        first_pointer
    );
    assert_eq!(
        older[0]
            .original()
            .downcast_ref::<std::io::Error>()
            .unwrap() as *const _ as usize,
        second_pointer
    );
    drop(outcomes);
    drop(newer);
    assert_eq!(
        older[0]
            .original()
            .downcast_ref::<std::io::Error>()
            .unwrap() as *const _ as usize,
        second_pointer
    );
    drop((older, held));
    assert_eq!(bank.available_permits(), total);
}
