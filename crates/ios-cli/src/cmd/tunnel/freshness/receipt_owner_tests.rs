use super::*;
use std::sync::atomic::{AtomicUsize, Ordering};

#[derive(Debug)]
struct Probe {
    bank: Arc<Semaphore>,
    drops: Arc<AtomicUsize>,
    free_at_original_drop: usize,
}
impl std::fmt::Display for Probe {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("original private freshness IO")
    }
}
impl std::error::Error for Probe {}
impl Drop for Probe {
    fn drop(&mut self) {
        assert_eq!(self.bank.available_permits(), self.free_at_original_drop);
        self.drops.fetch_add(1, Ordering::SeqCst);
    }
}

fn original(bank: &Arc<Semaphore>, drops: &Arc<AtomicUsize>, free: usize) -> anyhow::Error {
    anyhow::Error::new(std::io::Error::new(
        std::io::ErrorKind::ConnectionReset,
        Probe {
            bank: bank.clone(),
            drops: drops.clone(),
            free_at_original_drop: free,
        },
    ))
}
fn pointer(root: &Receipt) -> usize {
    root.original().downcast_ref::<std::io::Error>().unwrap() as *const _ as usize
}

#[test]
fn freshness_receipt_owner_exact_pre_source_body_reports_and_closed_pool_last_snapshot_refund() {
    let body = std::mem::size_of::<Receipt>();
    let report = std::mem::size_of::<Failure>();
    let total = body + 2 * report;
    for available in [0, total - 1] {
        let bank = Arc::new(Semaphore::new(available));
        let error = Receipt::prepare(&bank).err().unwrap();
        assert_eq!(
            crate::output::operation_error_class(error.as_ref()),
            ios_core::ErrorClass::ResourceLimit
        );
        assert_eq!(bank.available_permits(), available);
    }
    let bank = Arc::new(Semaphore::new(total));
    let drops = Arc::new(AtomicUsize::new(0));
    let root = Receipt::prepare(&bank).unwrap();
    assert_eq!(root._storage.num_permits(), body);
    assert!(Arc::ptr_eq(root._storage.semaphore(), &bank));
    assert_eq!(bank.available_permits(), 0);
    let error = original(&bank, &drops, 2 * report);
    let original_pointer = error.downcast_ref::<std::io::Error>().unwrap() as *const _ as usize;
    let original_class = crate::output::operation_error_class(error.as_ref());
    root.capture(error);
    bank.close();
    let first = root.report();
    let second = root.report();
    assert_eq!(
        crate::output::operation_error_class(first.as_ref()),
        original_class
    );
    assert_eq!(
        crate::output::operation_error_class(second.as_ref()),
        original_class
    );
    for report in [&first, &second] {
        let kept = &report.downcast_ref::<Failure>().unwrap().receipt;
        assert!(root.ptr_eq(kept));
        assert_eq!(pointer(kept), original_pointer);
    }
    let snapshot = root.clone();
    drop(root);
    assert_eq!(bank.available_permits(), 0);
    drop(first);
    assert_eq!(bank.available_permits(), report);
    assert_eq!(drops.load(Ordering::SeqCst), 0);
    drop(second);
    assert_eq!(bank.available_permits(), 2 * report);
    assert_eq!(pointer(&snapshot), original_pointer);
    assert_eq!(drops.load(Ordering::SeqCst), 0);
    drop(snapshot);
    assert_eq!(drops.load(Ordering::SeqCst), 1);
    assert_eq!(bank.available_permits(), total);
}

#[test]
fn freshness_receipt_owner_parallel_last_roots_keep_original_io_and_unused_reports_until_drop() {
    let total = std::mem::size_of::<Receipt>() + 2 * std::mem::size_of::<Failure>();
    let bank = Arc::new(Semaphore::new(total));
    let drops = Arc::new(AtomicUsize::new(0));
    let root = Receipt::prepare(&bank).unwrap();
    root.capture(original(&bank, &drops, 0));
    let original_pointer = pointer(&root);
    let root_pointer = &*root as *const Receipt as usize;
    bank.close();
    let barrier = Arc::new(std::sync::Barrier::new(8));
    std::thread::scope(|scope| {
        for _ in 0..7 {
            let kept = root.clone();
            assert!(kept.ptr_eq(&root));
            let barrier = barrier.clone();
            scope.spawn(move || {
                assert_eq!(&*kept as *const Receipt as usize, root_pointer);
                assert_eq!(pointer(&kept), original_pointer);
                barrier.wait();
                drop(kept);
            });
        }
        barrier.wait();
        drop(root);
    });
    assert_eq!(drops.load(Ordering::SeqCst), 1);
    assert_eq!(bank.available_permits(), total);
}
