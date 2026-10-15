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
    let required = std::mem::size_of::<Ledger>();
    for bytes in [required - 1, required] {
        let bank = Arc::new(Semaphore::new(bytes));
        match Ledger::with_metadata(Some(&bank)) {
            Ok(owner) => {
                assert_eq!(bytes, required);
                assert_eq!(bank.available_permits(), 0);
                let last = Arc::clone(&owner);
                drop(owner);
                assert_eq!(bank.available_permits(), 0);
                drop(last);
            }
            Err(error) => {
                assert_eq!(bytes, required - 1);
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
    let required = std::mem::size_of::<Ledger>();
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
    let first = ledger.result(None).unwrap_err();
    let second = ledger.result(Some(expiry)).unwrap_err();
    assert_eq!(bank.available_permits(), 0);
    assert_eq!(report(&first).cleanup().count(), OUTCOME_SLOTS - 1);
    assert_eq!(report(&second).cleanup().count(), OUTCOME_SLOTS);
    drop(ledger);
    assert_eq!(bank.available_permits(), required);
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
}
