use super::*;
use tokio::sync::Semaphore;

fn report(error: &TunnelError) -> &TunnelShutdownFailure {
    let mut cause: Option<&(dyn Error + 'static)> = Some(error);
    for _ in 0..32 {
        let current = cause.expect("native report missing");
        if let Some(report) = current.downcast_ref() {
            return report;
        }
        cause = current.source();
    }
    panic!("native report source depth exceeded");
}

#[test]
fn ledger_actual_body_one_byte_refusal_and_last_shared_owner_refund() {
    let required = Ledger::reservation_bytes();
    for bytes in [required - 1, required] {
        let bank = Arc::new(Semaphore::new(bytes));
        let allocations = Arc::new(AtomicUsize::new(0));
        let observed = allocations.clone();
        let observed_bank = bank.clone();
        let result = crate::device::prepared_rsd::with_allocation_hook(
            move |layout, _| {
                assert_eq!(bytes, required, "short whole native grant allocated a root");
                assert_eq!(
                    observed_bank.available_permits(),
                    0,
                    "whole debit must precede the identity backing"
                );
                assert_eq!(layout.size(), Identity::body_bytes());
                observed.fetch_add(1, Ordering::SeqCst);
            },
            || Ledger::with_metadata(Some(&bank)),
        );
        match result {
            Ok(owner) => {
                assert_eq!(bytes, required);
                assert_eq!(allocations.load(Ordering::SeqCst), 1);
                assert_eq!(bank.available_permits(), 0);
                let last = Arc::clone(&owner);
                drop(owner);
                assert_eq!(bank.available_permits(), 0);
                drop(last);
            }
            Err(error) => {
                assert_eq!(bytes, required - 1);
                assert_eq!(allocations.load(Ordering::SeqCst), 0);
                assert_eq!(
                    crate::classify_error(&error),
                    crate::ErrorClass::ResourceLimit
                );
            }
        }
        assert_eq!(bank.available_permits(), bytes);
    }
}

#[tokio::test]
async fn child_set_actual_body_keeps_fee_across_original_join_until_last_owner() {
    let required = std::mem::size_of::<SharedChildTaskSet>();
    let short = Arc::new(Semaphore::new(required - 1));
    let ledger = Ledger::new();
    assert!(SharedChildTaskSet::with_metadata(
        Arc::clone(&ledger),
        ChildRole::ProxyClient,
        Some(&short)
    )
    .is_err());
    assert_eq!(short.available_permits(), required - 1);
    assert_eq!(ledger.pending.load(Ordering::Acquire), 0);
    let bank = Arc::new(Semaphore::new(required));
    let owner =
        SharedChildTaskSet::with_metadata(Arc::clone(&ledger), ChildRole::ProxyClient, Some(&bank))
            .unwrap();
    let last = Arc::clone(&owner);
    owner
        .lock()
        .await
        .spawn(async { std::future::pending::<()>().await });
    assert_eq!(ledger.pending.load(Ordering::Acquire), 1);
    owner.drain().await;
    assert_eq!(ledger.pending.load(Ordering::Acquire), 0);
    assert_eq!(bank.available_permits(), 0);
    drop(owner);
    assert_eq!(bank.available_permits(), 0);
    drop(last);
    assert_eq!(bank.available_permits(), required);
}

#[tokio::test]
async fn inline_cleanup_all_originals_and_attempt_expiry_survive_ledger_drop_at_zero_quota() {
    let required = Ledger::reservation_bytes();
    let bank = Arc::new(Semaphore::new(required));
    let ledger = Ledger::with_metadata(Some(&bank)).unwrap();
    let originals: [usize; OUTCOME_SLOTS] = std::array::from_fn(|index| {
        let original: Cause = Arc::new(std::io::Error::from_raw_os_error(index as i32 + 1));
        let pointer = Arc::as_ptr(&original) as *const () as usize;
        ledger.record_work(index, original);
        pointer
    });
    let expiry = tokio::time::timeout(Duration::ZERO, pending::<()>())
        .await
        .unwrap_err();
    bank.close();
    let (first, second) = crate::device::prepared_rsd::with_allocation_hook(
        |_, _| panic!("closed-pool report allocated another provenance root"),
        || {
            (
                ledger.result(None).unwrap_err(),
                ledger.result(Some(expiry)).unwrap_err(),
            )
        },
    );
    assert!(report(&first).identity.ptr_eq(&ledger.identity));
    assert!(report(&second).identity.ptr_eq(&ledger.identity));
    assert_eq!(bank.available_permits(), 0);
    assert_eq!(report(&first).cleanup().count(), OUTCOME_SLOTS - 1);
    assert_eq!(report(&second).cleanup().count(), OUTCOME_SLOTS);
    drop(ledger);
    let ledger_bytes = std::mem::size_of::<Ledger>();
    assert_eq!(bank.available_permits(), ledger_bytes);
    assert_eq!(
        report(&first).primary() as *const dyn Error as *const () as usize,
        originals[0]
    );
    for (index, cause) in report(&first).cleanup().enumerate() {
        assert_eq!(
            cause as *const dyn Error as *const () as usize,
            originals[index + 1]
        );
    }
    assert!(report(&second).cleanup().any(|cause| cause.is::<Elapsed>()));
    assert!(report(&first).joins_complete() && report(&second).joins_complete());
    drop(first);
    assert_eq!(bank.available_permits(), ledger_bytes);
    let observed_bank = bank.clone();
    native_identity::after_free(
        move || {
            assert_eq!(
                observed_bank.available_permits(),
                ledger_bytes,
                "provenance refunded before its last report's actual identity free"
            )
        },
        || drop(second),
    );
    assert_eq!(bank.available_permits(), required);
}
