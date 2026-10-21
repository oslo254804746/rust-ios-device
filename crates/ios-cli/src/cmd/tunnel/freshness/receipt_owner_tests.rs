use super::*;
use std::error::Error;
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
    let body = Receipt::body_bytes();
    let report = std::mem::size_of::<ReportPayload>();
    let total = Receipt::reservation_bytes();
    assert_eq!(total, body + 2 * report);
    for available in [0, body - 1, total - 1] {
        let bank = Arc::new(Semaphore::new(available));
        let error = super::super::metadata_vec::with_allocation_hook(
            |_, _| panic!("refresh whole capsule refused after owned backing"),
            || Receipt::prepare(&bank).err().unwrap(),
        );
        assert_eq!(
            crate::output::operation_error_class(error.as_ref()),
            ios_core::ErrorClass::ResourceLimit
        );
        assert_eq!(bank.available_permits(), available);
    }
    let bank = Arc::new(Semaphore::new(total));
    let drops = Arc::new(AtomicUsize::new(0));
    let allocated = Arc::new(Mutex::new(Vec::new()));
    let captured = allocated.clone();
    let charged = bank.clone();
    let root = super::super::metadata_vec::with_allocation_hook(
        move |layout, pointer| {
            assert_eq!(charged.available_permits(), 0);
            captured.lock().unwrap().push((layout, pointer));
        },
        || Receipt::prepare(&bank).unwrap(),
    );
    let observed = allocated.lock().unwrap();
    assert_eq!(observed.len(), 3);
    assert_eq!(observed[0].0, std::alloc::Layout::new::<ReportPayload>());
    assert_eq!(observed[1].0, std::alloc::Layout::new::<ReportPayload>());
    assert_eq!(observed[2].0.size(), body);
    let data = std::ptr::from_ref(&*root) as usize;
    assert!(observed[2].1 <= data && data < observed[2].1 + observed[2].0.size());
    let report_pointers = [observed[0].1, observed[1].1];
    {
        let reports = root.reports.lock().unwrap();
        for (destination, pointer) in reports.iter().zip(report_pointers) {
            assert_eq!(destination.as_ref().unwrap().as_ptr() as usize, pointer);
        }
    }
    drop(observed);
    assert!(root.original.get().is_none());
    assert_eq!(root.storage().num_permits(), body);
    assert!(Arc::ptr_eq(root.storage().semaphore(), &bank));
    assert_eq!(bank.available_permits(), 0);
    let error = original(&bank, &drops, 2 * report);
    let original_pointer = error.downcast_ref::<std::io::Error>().unwrap() as *const _ as usize;
    let original_class = crate::output::operation_error_class(error.as_ref());
    root.capture(error);
    bank.close();
    let (first, second) = super::super::metadata_vec::with_allocation_hook(
        |_, _| panic!("refresh capture/report replaced the original owned capsule"),
        || (root.report(), root.report()),
    );
    assert_eq!(
        crate::output::operation_error_class(first.as_ref()),
        original_class
    );
    assert_eq!(
        crate::output::operation_error_class(second.as_ref()),
        original_class
    );
    for (error, expected) in [&first, &second].into_iter().zip(report_pointers) {
        let failure = error.downcast_ref::<Failure>().unwrap();
        let kept = &failure.payload.receipt;
        assert_eq!(&*failure.payload as *const ReportPayload as usize, expected);
        assert_eq!(failure.payload.bytes_in(&bank).unwrap(), report);
        assert!(root.ptr_eq(kept));
        assert_eq!(pointer(kept), original_pointer);
        assert_eq!(
            failure
                .source()
                .unwrap()
                .downcast_ref::<std::io::Error>()
                .unwrap() as *const _ as usize,
            original_pointer
        );
    }
    assert!(
        std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| root.report())).is_err(),
        "a capsule must never create a third report"
    );
    let first = first.downcast::<Failure>().unwrap();
    let second = second.downcast::<Failure>().unwrap();
    assert_eq!(
        &*first.payload as *const ReportPayload as usize,
        report_pointers[0]
    );
    assert_eq!(
        &*second.payload as *const ReportPayload as usize,
        report_pointers[1]
    );
    assert_eq!(pointer(&first.payload.receipt), original_pointer);
    assert_eq!(pointer(&second.payload.receipt), original_pointer);
    let snapshot = root.clone();
    drop(root);
    assert_eq!(bank.available_permits(), 0);
    let charged = bank.clone();
    super::super::owned_box::after_free(
        move || {
            assert_eq!(
                charged.available_permits(),
                0,
                "first report credit preceded its actual Box free"
            )
        },
        || drop(first),
    );
    assert_eq!(bank.available_permits(), report);
    assert_eq!(drops.load(Ordering::SeqCst), 0);
    let charged = bank.clone();
    super::super::owned_box::after_free(
        move || {
            assert_eq!(
                charged.available_permits(),
                report,
                "second report credit preceded its actual Box free"
            )
        },
        || drop(second),
    );
    assert_eq!(bank.available_permits(), 2 * report);
    assert_eq!(pointer(&snapshot), original_pointer);
    assert_eq!(drops.load(Ordering::SeqCst), 0);
    drop(snapshot);
    assert_eq!(drops.load(Ordering::SeqCst), 1);
    assert_eq!(bank.available_permits(), total);
}

#[test]
fn freshness_receipt_owner_parallel_last_roots_keep_original_io_and_unused_reports_until_drop() {
    let total = Receipt::reservation_bytes();
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

#[test]
fn freshness_receipt_foreign_or_wrong_whole_grant_never_allocates_a_report_or_root() {
    let bytes = Receipt::reservation_bytes();
    for (foreign, amount) in [(true, bytes), (false, bytes - 1), (false, bytes + 1)] {
        let bank = Arc::new(Semaphore::new(2 * bytes));
        let other = Arc::new(Semaphore::new(2 * bytes));
        let storage =
            admission::known_metadata_charge(if foreign { &other } else { &bank }, amount).unwrap();
        let error = super::super::metadata_vec::with_allocation_hook(
            |_, _| panic!("invalid whole freshness grant allocated a backing"),
            || Receipt::from_grant(&bank, storage).err().unwrap(),
        );
        assert_eq!(
            crate::output::operation_error_class(error.as_ref()),
            ios_core::ErrorClass::ResourceLimit
        );
        assert_eq!(bank.available_permits(), 2 * bytes);
        assert_eq!(other.available_permits(), 2 * bytes);
    }
}

#[test]
fn freshness_report_last_owner_releases_unused_original_destination_and_root_without_a_cycle() {
    let total = Receipt::reservation_bytes();
    let report = std::mem::size_of::<ReportPayload>();
    let bank = Arc::new(Semaphore::new(total));
    let drops = Arc::new(AtomicUsize::new(0));
    let root = Receipt::prepare(&bank).unwrap();
    root.capture(original(&bank, &drops, 0));
    let original_pointer = pointer(&root);
    bank.close();
    let report_error = super::super::metadata_vec::with_allocation_hook(
        |_, _| panic!("closed zero pool report allocated a new backing"),
        || root.report(),
    );
    let failure = report_error.downcast::<Failure>().unwrap();
    assert_eq!(pointer(&failure.payload.receipt), original_pointer);
    drop(root);
    assert_eq!(bank.available_permits(), 0);
    assert_eq!(drops.load(Ordering::SeqCst), 0);
    let observed = Arc::new(Mutex::new(Vec::new()));
    let captured = observed.clone();
    let charged = bank.clone();
    super::super::owned_box::after_free(
        move || captured.lock().unwrap().push(charged.available_permits()),
        || drop(failure),
    );
    // The empty second destination frees first. The last filled report keeps
    // its own grant while its original root/body and unused sibling end.
    assert_eq!(observed.lock().unwrap().as_slice(), [0, total - report]);
    assert_eq!(drops.load(Ordering::SeqCst), 1);
    assert_eq!(bank.available_permits(), total);
}
