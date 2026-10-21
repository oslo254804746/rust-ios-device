//! Native work-report provenance and original cause retention fixtures.
use super::*;

async fn identity_panic_seed(
    ledger: &Ledger,
) -> (
    Option<Arc<JoinError>>,
    Arc<AtomicUsize>,
    Result<(), TunnelError>,
) {
    let drops = Arc::new(AtomicUsize::new(0));
    let native_drops = Arc::clone(&drops);
    let handle = tokio::spawn(async move {
        let _resource = DropProbe(native_drops);
        std::panic::panic_any(NativePanic);
        #[allow(unreachable_code)]
        Ok(())
    });
    let task = OwnedTask::new(handle, ParentRole::ForwardOrReader, ledger.clone());
    let result = join_task(&task, ledger).await;
    let native = {
        let state = task.join.lock().await;
        match &state.completion {
            Some(Completion::UnexpectedJoin { _cause }) => Some(Arc::clone(_cause)),
            _ => None,
        }
    };
    drop(task);
    // The seed has finished its actual native join before any followup task
    // is created. Return evidence; defer test assertions until all joins.
    (native, drops, result)
}

async fn identity_work_completion(task: &OwnedTask) -> Option<Arc<TunnelError>> {
    let state = task.join.lock().await;
    match &state.completion {
        Some(Completion::WorkFailed { _cause }) => Some(Arc::clone(_cause)),
        _ => None,
    }
}

fn identity_assert_balanced(ledger: &Ledger) {
    assert_eq!(ledger.pending.load(Ordering::Acquire), 0);
    assert!(!ledger.invalid_count.load(Ordering::Acquire));
}

#[tokio::test]
async fn native_owner_identity_same_ledger_work_report_keeps_direct_native_cleanup() {
    let ledger = Ledger::new();
    let (native, seed_drops, seeded) = identity_panic_seed(&ledger).await;
    let work_ledger = ledger.clone();
    let work_drops = Arc::new(AtomicUsize::new(0));
    let native_work_drops = Arc::clone(&work_drops);
    let handle = tokio::spawn(async move {
        let _resource = DropProbe(native_work_drops);
        work_ledger.work_result()
    });
    let task = OwnedTask::new(handle, ParentRole::ForwardOrReader, ledger.clone());
    let first = join_task(&task, &ledger).await;
    let cached = identity_work_completion(&task).await;
    let repeated = join_task(&task, &ledger).await;
    drop(task);

    // Both actual parents are joined and native resources released before
    // checking the identity/promotion/counter invariant.
    let native = native.expect("seed's original native panic JoinError was lost");
    let seeded = seeded.unwrap_err();
    let first = first.unwrap_err();
    let repeated = repeated.unwrap_err();
    let cached = cached.expect("real typed WorkReport completion was not retained");
    assert!(native.is_panic() && !native.is_cancelled());
    assert_eq!(seed_drops.load(Ordering::SeqCst), 1);
    assert_eq!(work_drops.load(Ordering::SeqCst), 1);
    for error in [&seeded, &first, &repeated, cached.as_ref()] {
        let report = failure(error);
        let original = report.primary().downcast_ref::<JoinError>().unwrap();
        assert!(std::ptr::eq(original, native.as_ref()));
        assert_eq!(report.cleanup().count(), 0);
        assert_eq!(report.additional_unretained_outcomes(), 0);
        assert_eq!(crate::classify_error(error), crate::ErrorClass::Unknown);
        assert_safe(error);
    }
    assert_eq!(failure(&first).pending_joins(), 0);
    assert_eq!(failure(&repeated).pending_joins(), 0);
    identity_assert_balanced(&ledger);
    let outcomes = ledger.outcomes.lock().unwrap();
    assert!(outcomes.primary.is_none());
    assert!(outcomes.slots[0].is_none());
    assert!(outcomes.slots[4].is_some());
    assert_eq!(
        outcomes.slots.iter().filter(|slot| slot.is_some()).count(),
        1
    );
    assert_eq!(outcomes.additional_unretained_outcomes, 0);
}

#[tokio::test]
async fn native_owner_identity_foreign_work_report_is_original_typed_work_fault() {
    let foreign = Ledger::new();
    let (native, seed_drops, seeded) = identity_panic_seed(&foreign).await;
    let foreign_report = foreign.work_result();
    let own = Ledger::new();
    let work_drops = Arc::new(AtomicUsize::new(0));
    let native_work_drops = Arc::clone(&work_drops);
    let handle = tokio::spawn(async move {
        let _resource = DropProbe(native_work_drops);
        foreign_report
    });
    let task = OwnedTask::new(handle, ParentRole::ForwardOrReader, own.clone());
    let joined = join_task(&task, &own).await;
    let cached = identity_work_completion(&task).await;
    let repeated = join_task(&task, &own).await;
    drop(task);

    let native = native.expect("foreign native panic JoinError was lost");
    seeded.unwrap_err();
    let joined = joined.unwrap_err();
    let repeated = repeated.unwrap_err();
    let cached = cached.expect("foreign typed WorkReport was incorrectly skipped");
    assert_eq!(seed_drops.load(Ordering::SeqCst), 1);
    assert_eq!(work_drops.load(Ordering::SeqCst), 1);
    for error in [&joined, &repeated] {
        let report = failure(error);
        let retained = report.primary().downcast_ref::<TunnelError>().unwrap();
        assert!(std::ptr::eq(retained, cached.as_ref()));
        let original = failure(retained)
            .primary()
            .downcast_ref::<JoinError>()
            .unwrap();
        assert!(std::ptr::eq(original, native.as_ref()));
        assert!(original.is_panic() && !original.is_cancelled());
        assert_eq!(report.pending_joins(), 0);
        assert_eq!(report.cleanup().count(), 0);
        assert_eq!(report.additional_unretained_outcomes(), 0);
        assert_eq!(crate::classify_error(error), crate::ErrorClass::Unknown);
        assert_safe(error);
    }
    identity_assert_balanced(&foreign);
    identity_assert_balanced(&own);
    {
        let outcomes = foreign.outcomes.lock().unwrap();
        assert!(outcomes.primary.is_none());
        assert!(outcomes.slots[0].is_none());
        assert_eq!(
            outcomes.slots.iter().filter(|slot| slot.is_some()).count(),
            1
        );
        assert_eq!(outcomes.additional_unretained_outcomes, 0);
    }
    let outcomes = own.outcomes.lock().unwrap();
    assert_eq!(outcomes.primary, Some(0));
    assert!(outcomes.slots[0].is_some());
    assert_eq!(
        outcomes.slots.iter().filter(|slot| slot.is_some()).count(),
        1
    );
    assert_eq!(outcomes.additional_unretained_outcomes, 0);
}

#[tokio::test]
async fn native_owner_identity_same_ledger_full_snapshot_is_not_a_work_report() {
    let ledger = Ledger::new();
    let (native, seed_drops, seeded) = identity_panic_seed(&ledger).await;
    // This is the full explicit-shutdown result, not work_result. Its ledger
    // identity matches but its role/marker must prevent the narrow skip.
    let full_report = ledger.result(None);
    let work_drops = Arc::new(AtomicUsize::new(0));
    let native_work_drops = Arc::clone(&work_drops);
    let handle = tokio::spawn(async move {
        let _resource = DropProbe(native_work_drops);
        full_report
    });
    let task = OwnedTask::new(handle, ParentRole::ForwardOrReader, ledger.clone());
    let joined = join_task(&task, &ledger).await;
    let cached = identity_work_completion(&task).await;
    let repeated = join_task(&task, &ledger).await;
    drop(task);

    let native = native.expect("original native panic JoinError was lost");
    seeded.unwrap_err();
    let joined = joined.unwrap_err();
    let repeated = repeated.unwrap_err();
    let cached = cached.expect("full shutdown result was incorrectly skipped");
    assert_eq!(seed_drops.load(Ordering::SeqCst), 1);
    assert_eq!(work_drops.load(Ordering::SeqCst), 1);
    for error in [&joined, &repeated] {
        let report = failure(error);
        let retained = report.primary().downcast_ref::<TunnelError>().unwrap();
        assert!(std::ptr::eq(retained, cached.as_ref()));
        let original = failure(retained)
            .primary()
            .downcast_ref::<JoinError>()
            .unwrap();
        assert!(std::ptr::eq(original, native.as_ref()));
        assert!(report.cleanup().any(|cause| {
            cause
                .downcast_ref::<JoinError>()
                .is_some_and(|cause| std::ptr::eq(cause, native.as_ref()))
        }));
        assert_eq!(report.cleanup().count(), 1);
        assert_eq!(report.pending_joins(), 0);
        assert_eq!(report.additional_unretained_outcomes(), 0);
        assert_safe(error);
    }
    identity_assert_balanced(&ledger);
    let outcomes = ledger.outcomes.lock().unwrap();
    assert_eq!(outcomes.primary, Some(0));
    assert!(outcomes.slots[0].is_some());
    assert!(outcomes.slots[4].is_some());
    assert_eq!(
        outcomes.slots.iter().filter(|slot| slot.is_some()).count(),
        2
    );
    assert_eq!(outcomes.additional_unretained_outcomes, 0);
}

struct IdentityExtraContext {
    source: TunnelError,
}

impl std::fmt::Display for IdentityExtraContext {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("ordinary owner fixture context")
    }
}

impl std::fmt::Debug for IdentityExtraContext {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("OwnerFixtureContext")
    }
}

impl Error for IdentityExtraContext {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        Some(&self.source)
    }
}

#[tokio::test]
async fn native_owner_identity_extra_ordinary_wrapper_cannot_trigger_deep_skip() {
    let ledger = Ledger::new();
    let (native, seed_drops, seeded) = identity_panic_seed(&ledger).await;
    let source = ledger.work_result().unwrap_err();
    let wrapped = TunnelError::Io(std::io::Error::other(IdentityExtraContext { source }));
    let work_drops = Arc::new(AtomicUsize::new(0));
    let native_work_drops = Arc::clone(&work_drops);
    let handle = tokio::spawn(async move {
        let _resource = DropProbe(native_work_drops);
        Err(wrapped)
    });
    let task = OwnedTask::new(handle, ParentRole::ForwardOrReader, ledger.clone());
    let joined = join_task(&task, &ledger).await;
    let cached = identity_work_completion(&task).await;
    let repeated = join_task(&task, &ledger).await;
    drop(task);

    let native = native.expect("original native panic JoinError was lost");
    seeded.unwrap_err();
    let joined = joined.unwrap_err();
    let repeated = repeated.unwrap_err();
    let cached = cached.expect("ordinary wrapped fault was incorrectly skipped");
    assert_eq!(seed_drops.load(Ordering::SeqCst), 1);
    assert_eq!(work_drops.load(Ordering::SeqCst), 1);
    let TunnelError::Io(io) = cached.as_ref() else {
        panic!("ordinary IO wrapper was replaced");
    };
    assert!(io.get_ref().unwrap().is::<IdentityExtraContext>());
    for error in [&joined, &repeated] {
        let report = failure(error);
        let retained = report.primary().downcast_ref::<TunnelError>().unwrap();
        assert!(std::ptr::eq(retained, cached.as_ref()));
        let original = failure(retained)
            .primary()
            .downcast_ref::<JoinError>()
            .unwrap();
        assert!(std::ptr::eq(original, native.as_ref()));
        assert_eq!(report.pending_joins(), 0);
        assert_eq!(report.cleanup().count(), 1);
        assert_eq!(report.additional_unretained_outcomes(), 0);
        assert_safe(error);
    }
    identity_assert_balanced(&ledger);
    let outcomes = ledger.outcomes.lock().unwrap();
    assert_eq!(outcomes.primary, Some(0));
    assert!(outcomes.slots[0].is_some());
    assert_eq!(
        outcomes.slots.iter().filter(|slot| slot.is_some()).count(),
        2
    );
    assert_eq!(outcomes.additional_unretained_outcomes, 0);
}

#[tokio::test]
async fn native_owner_identity_existing_work_slot_does_not_skip_another_native_fault() {
    let ledger = Ledger::new();
    let drops = Arc::new(AtomicUsize::new(0));
    let mut results = Vec::new();
    let mut first_completion = None;
    for index in 0..2 {
        let native_drops = Arc::clone(&drops);
        let handle = tokio::spawn(async move {
            let _resource = DropProbe(native_drops);
            let (mut stream, peer) = tokio::io::duplex(1);
            drop(peer);
            // Actual Tokio duplex BrokenPipe, not a supplied marker payload.
            let error = stream.write_all(b"x").await.unwrap_err();
            Err(TunnelError::Io(error))
        });
        let task = OwnedTask::new(handle, ParentRole::ForwardOrReader, ledger.clone());
        let result = join_task(&task, &ledger).await;
        if index == 0 {
            first_completion = identity_work_completion(&task).await;
        }
        drop(task);
        results.push(result);
    }
    // Both actual native IO tasks have joined before any outcome assertion.
    let first = first_completion.expect("original native work fault was lost");
    assert_eq!(drops.load(Ordering::SeqCst), 2);
    for result in results {
        let error = result.unwrap_err();
        let report = failure(&error);
        let original = report.primary().downcast_ref::<TunnelError>().unwrap();
        assert!(std::ptr::eq(original, first.as_ref()));
        assert_eq!(original_io(report).kind(), std::io::ErrorKind::BrokenPipe);
        assert_eq!(
            crate::classify_error(&error),
            crate::ErrorClass::Disconnected
        );
        assert_eq!(report.pending_joins(), 0);
        assert_eq!(report.cleanup().count(), 0);
    }
    let current = ledger.result(None).unwrap_err();
    assert_eq!(failure(&current).additional_unretained_outcomes(), 1);
    identity_assert_balanced(&ledger);
    let outcomes = ledger.outcomes.lock().unwrap();
    assert_eq!(outcomes.primary, Some(0));
    assert_eq!(
        outcomes.slots.iter().filter(|slot| slot.is_some()).count(),
        1
    );
    assert_eq!(outcomes.additional_unretained_outcomes, 1);
}
