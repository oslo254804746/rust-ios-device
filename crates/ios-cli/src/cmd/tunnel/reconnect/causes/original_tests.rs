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
    let storage =
        super::super::super::admission::known_metadata_charge(bank, Original::allocation_bytes())
            .unwrap();
    let original = Shared::from_grant(
        Original {
            error: OnceLock::new(),
        },
        bank,
        storage,
    )
    .unwrap();
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
    let total = Original::allocation_bytes();
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
    let bytes = Original::allocation_bytes();
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

#[tokio::test]
async fn original_capsule_real_ledger_prepays_whole_control_block_and_captures_same_io_at_closed_zero(
) {
    use crate::cmd::tunnel::{failure_slot::Slot, lifecycle::Lifecycle, metadata_vec, Route};
    use std::sync::Mutex as StdMutex;
    let key = "original-control-block-fixture";
    let identity = AttemptIdentity {
        route: Route::Usbmux,
        native_id: Some(7),
    };
    let total = Ledger::structural_bytes()
        + Ledger::initial_attempt_bytes(key, &identity)
        + Slot::body_bytes();
    // These exact owners follow receipt construction in Ledger::start. Their
    // grants are still unspent when the original capsule backing is created.
    let later_episode_storage = 2 * std::mem::size_of::<Removed>()
        + key.len()
        + crate::cmd::tunnel::candidate_input::route_length(&identity.route);
    let bank = Arc::new(Semaphore::new(total));
    let ledger = Ledger::with_metadata(RetryPolicy::default(), bank.clone()).unwrap();
    let lifecycle = Lifecycle::new();
    let ticket = lifecycle.ticket(key).await.unwrap();
    let admitted =
        Slot::reserve(&crate::cmd::tunnel::failure_slot::Counts::fixture(1), &bank).unwrap();
    let allocations = Arc::new(StdMutex::new(Vec::new()));
    let captured = allocations.clone();
    let charged = bank.clone();
    let attempt = metadata_vec::with_allocation_hook(
        move |layout, pointer| {
            captured
                .lock()
                .unwrap()
                .push((layout, pointer, charged.available_permits()));
        },
        || {
            ticket
                .commit(|| ledger.start(key, identity, &ticket, admitted))
                .unwrap()
        },
    );
    let original_pointer = std::ptr::from_ref(&*attempt.receipt.original) as usize;
    let observed = allocations.lock().unwrap();
    let actual = observed
        .iter()
        .filter(|(layout, pointer, _)| {
            *pointer <= original_pointer && original_pointer < *pointer + layout.size()
        })
        .collect::<Vec<_>>();
    assert_eq!(actual.len(), 1);
    assert_eq!(actual[0].0.size(), Original::allocation_bytes());
    assert_eq!(actual[0].2, later_episode_storage);
    assert_eq!(
        attempt.receipt.original.storage().num_permits(),
        actual[0].0.size()
    );
    assert!(Arc::ptr_eq(
        attempt.receipt.original.storage().semaphore(),
        &bank
    ));
    drop(observed);
    assert_eq!(bank.available_permits(), 0);
    bank.close();
    let original = anyhow::Error::new(std::io::Error::new(
        std::io::ErrorKind::ConnectionReset,
        "private-original-control-source",
    ));
    let pointer = original.downcast_ref::<std::io::Error>().unwrap() as *const _ as usize;
    let original_class = crate::output::operation_error_class(original.as_ref());
    assert_eq!(original_class, ErrorClass::Disconnected);
    // Fill the already-admitted capsule after close with no replacement root,
    // allocation or pool admission. Outer public error wrapper is separate.
    let result: Result<()> = metadata_vec::with_allocation_hook(
        |_, _| panic!("Source capture created a new owned allocation"),
        || Err(attempt.source_failed(original)),
    );
    let receipt = result
        .as_ref()
        .unwrap_err()
        .downcast_ref::<SourceFailure>()
        .unwrap()
        .receipt();
    assert_eq!(
        std::ptr::from_ref(&*receipt.original) as usize,
        original_pointer
    );
    assert_eq!(
        receipt.primary().downcast_ref::<std::io::Error>().unwrap() as *const _ as usize,
        pointer
    );
    assert_eq!(
        receipt.latest().downcast_ref::<std::io::Error>().unwrap() as *const _ as usize,
        pointer
    );
    attempt.finish(&result);
    ledger.close();
    lifecycle.clear();
    drop(ledger.take_retired_causes());
    drop((ledger, ticket, lifecycle, result));
    assert!(bank.available_permits() < total);
    assert_eq!(
        std::ptr::from_ref(&*receipt.original) as usize,
        original_pointer
    );
    assert_eq!(
        receipt.primary().downcast_ref::<std::io::Error>().unwrap() as *const _ as usize,
        pointer
    );
    assert_eq!(
        crate::output::operation_error_class(receipt.primary().as_ref()),
        original_class
    );
    drop(receipt);
    assert_eq!(bank.available_permits(), total);
}
