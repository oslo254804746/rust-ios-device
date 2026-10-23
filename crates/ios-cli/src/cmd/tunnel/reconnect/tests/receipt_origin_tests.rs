use super::*;
use crate::cmd::tunnel::{metadata_vec, shared_weak_allocation, ControlIdentity};
use std::error::Error;
use std::sync::atomic::{AtomicUsize, Ordering};

const TOTAL: usize = 128 * 1024;

#[tokio::test]
async fn receipt_origin_refusal_precedes_capsule_or_selectors_and_refunds_original_destination() {
    let body = FailureReceipt::body_bytes();
    for closed in [false, true] {
        let own = Lifecycle::watch_bytes() + Lifecycle::initial_metadata_bytes(A);
        let available = if closed { body } else { body - 1 };
        let origin = Arc::new(Semaphore::new(own + available));
        let lifecycle = Lifecycle::with_metadata(origin.clone()).unwrap();
        let ticket = lifecycle.ticket(A).await.unwrap().fenced();
        assert_eq!(origin.available_permits(), available);
        if closed {
            origin.close();
        }
        let bank = Arc::new(Semaphore::new(TOTAL));
        let admitted = slot(&bank);
        let version = FailureVersion::new(&bank).unwrap();
        let hook = metadata_vec::on_allocation(|_, _| {
            panic!("denied origin allocated a capsule or selector")
        });
        let error = FailureReceipt::reserve(A, &identity(), &ticket, admitted, version, &bank)
            .err()
            .unwrap();
        drop(hook);
        assert_eq!(
            crate::output::operation_error_class(error.as_ref()),
            ErrorClass::ResourceLimit
        );
        assert_eq!(origin.available_permits(), available);
        assert_eq!(bank.available_permits(), TOTAL);
        assert!(ticket.is_current());
        drop((ticket, lifecycle));
        assert_eq!(origin.available_permits(), own + available);
    }
}

#[tokio::test]
async fn receipt_origin_optional_captured_desired_producer_rolls_back_then_survives_last_weak_free()
{
    let body = FailureReceipt::body_bytes();
    let origin = Arc::new(Semaphore::new(TOTAL));
    let lifecycle = Lifecycle::with_metadata(origin.clone()).unwrap();
    let unfenced = lifecycle.ticket(A).await.unwrap();
    let desired_bank = Arc::new(Semaphore::new(body));
    let desired = ControlIdentity::new(&desired_bank);
    let observed = unfenced.observe_version(&desired);
    // The constructor must account for captured identities even for a stale
    // observational Ticket; it cannot replace its captured desired marker.
    let before = origin.available_permits();
    let bank = Arc::new(Semaphore::new(TOTAL));
    let held = desired_bank.clone().try_acquire_owned().unwrap();
    let error = FailureReceipt::reserve(
        A,
        &identity(),
        &observed,
        slot(&bank),
        FailureVersion::new(&bank).unwrap(),
        &bank,
    )
    .err()
    .unwrap();
    assert_eq!(
        crate::output::operation_error_class(error.as_ref()),
        ErrorClass::ResourceLimit
    );
    assert_eq!(origin.available_permits(), before);
    assert_eq!(desired_bank.available_permits(), body - 1);
    assert_eq!(bank.available_permits(), TOTAL);
    drop(held);
    let calls = Arc::new(AtomicUsize::new(0));
    let seen = calls.clone();
    let primary = origin.clone();
    let optional = desired_bank.clone();
    let admitted = slot(&bank);
    let version = FailureVersion::new(&bank).unwrap();
    let hook = metadata_vec::on_allocation(move |_, _| {
        assert_eq!(primary.available_permits(), before - body);
        assert_eq!(optional.available_permits(), 0);
        seen.fetch_add(1, Ordering::SeqCst);
    });
    let receipt =
        FailureReceipt::reserve(A, &identity(), &observed, admitted, version, &bank).unwrap();
    drop(hook);
    assert!(calls.load(Ordering::SeqCst) >= 2);
    assert_eq!(receipt.extra().iter().flatten().count(), 2);
    assert!(receipt
        .extra()
        .iter()
        .flatten()
        .all(|grant| grant.num_permits() == body));
    let weak = receipt.downgrade();
    let second = weak.clone();
    let upgraded = weak.upgrade().unwrap();
    assert!(upgraded.ptr_eq(&receipt));
    drop((upgraded, receipt));
    assert!(weak.upgrade().is_none());
    assert_eq!(origin.available_permits(), before - body);
    assert_eq!(desired_bank.available_permits(), 0);
    assert_eq!(bank.available_permits(), TOTAL - body);
    drop(weak);
    let origin_observer = origin.clone();
    let desired_observer = desired_bank.clone();
    let destination_observer = bank.clone();
    let freed = Arc::new(AtomicUsize::new(0));
    let seen = freed.clone();
    shared_weak_allocation::after_free(
        move || {
            assert_eq!(origin_observer.available_permits(), before - body);
            assert_eq!(desired_observer.available_permits(), 0);
            assert_eq!(destination_observer.available_permits(), TOTAL - body);
            seen.fetch_add(1, Ordering::SeqCst);
        },
        || drop(second),
    );
    assert_eq!(freed.load(Ordering::SeqCst), 1);
    assert_eq!(origin.available_permits(), before);
    assert_eq!(desired_bank.available_permits(), body);
    assert_eq!(bank.available_permits(), TOTAL);
}

#[derive(Debug)]
struct Deep(std::io::Error);
impl std::fmt::Display for Deep {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("original cross-bank receipt IO")
    }
}
impl Error for Deep {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        Some(&self.0)
    }
}

#[tokio::test]
async fn receipt_origin_actual_ledger_keeps_original_deep_io_at_closed_zero_until_last_physical_weak(
) {
    let origin = Arc::new(Semaphore::new(TOTAL));
    let lifecycle = Lifecycle::with_metadata(origin.clone()).unwrap();
    let ticket = lifecycle.ticket(A).await.unwrap().fenced();
    let bank = Arc::new(Semaphore::new(TOTAL));
    let ledger = Ledger::with_metadata(RetryPolicy::default(), bank.clone()).unwrap();
    let attempt = ticket
        .commit(|| ledger.start(A, identity(), &ticket, slot(&bank)))
        .unwrap();
    let receipt = attempt.receipt.clone();
    let weak = receipt.downgrade();
    let body = FailureReceipt::body_bytes();
    assert_eq!(receipt.extra().iter().flatten().count(), 1);
    let a_held = origin
        .clone()
        .try_acquire_many_owned(origin.available_permits() as u32)
        .unwrap();
    let b_held = bank
        .clone()
        .try_acquire_many_owned(bank.available_permits() as u32)
        .unwrap();
    origin.close();
    bank.close();
    assert_eq!(
        (origin.available_permits(), bank.available_permits()),
        (0, 0)
    );
    lifecycle.clear();
    ledger.close();
    let original = anyhow::Error::new(Deep(std::io::Error::new(
        std::io::ErrorKind::PermissionDenied,
        "original-deep-cross-bank-io",
    )));
    let original_pointer = original.downcast_ref::<Deep>().unwrap() as *const Deep as usize;
    let io_pointer = &original.downcast_ref::<Deep>().unwrap().0 as *const _ as usize;
    let result = failed_result(attempt.source_failed(original));
    attempt.finish(&result);
    assert_eq!(
        receipt.primary().downcast_ref::<Deep>().unwrap() as *const _ as usize,
        original_pointer
    );
    assert_eq!(
        &receipt.primary().downcast_ref::<Deep>().unwrap().0 as *const _ as usize,
        io_pointer
    );
    assert_eq!(
        crate::output::operation_error_class(receipt.primary().as_ref()),
        ErrorClass::PermissionDenied
    );
    assert_eq!(receipt.completion(), Completion::Abandoned);
    drop(ledger.take_retired_causes());
    drop((ledger, result, ticket, lifecycle, a_held, b_held));
    assert_eq!(
        receipt.latest().downcast_ref::<Deep>().unwrap() as *const _ as usize,
        original_pointer
    );
    drop(receipt);
    assert!(weak.upgrade().is_none());
    assert_eq!(origin.available_permits(), TOTAL - body);
    assert_eq!(bank.available_permits(), TOTAL - body);
    let primary = origin.clone();
    let destination = bank.clone();
    shared_weak_allocation::after_free(
        move || {
            assert_eq!(primary.available_permits(), TOTAL - body);
            assert_eq!(destination.available_permits(), TOTAL - body);
        },
        || drop(weak),
    );
    assert_eq!(origin.available_permits(), TOTAL);
    assert_eq!(bank.available_permits(), TOTAL);
}
