use super::*;

fn target(udid: &str, host: &str) -> PairedMobdev2Device {
    PairedMobdev2Device {
        udid: udid.into(),
        host: host.into(),
    }
}

#[test]
fn paired_plan_bytes_preserves_first_matching_host_order_and_original_buffers() {
    let bank = Arc::new(Semaphore::new(4096));
    let targets = vec![
        target("z", "first-z"),
        target("a", "first-a"),
        target("z", "later-z"),
        target("usb", "excluded"),
        target("denied", "excluded"),
        target("a", "later-a"),
    ];
    let host = targets[0].host.as_ptr();
    let capacity = targets[0].host.capacity();
    let mut seen = HashSet::new();
    seen.insert("usb".to_owned());
    let result =
        CandidateBudget::default().paired_candidates(targets, &seen, |key| key != "denied", &bank);
    let PairedCandidates::Complete {
        targets,
        indices,
        _storage,
    } = result
    else {
        panic!("complete plan rejected")
    };
    assert_eq!(indices, vec![0, 1]);
    assert_eq!(targets[0].host.as_ptr(), host);
    assert_eq!(targets[0].host.capacity(), capacity);
    assert_eq!(
        bank.available_permits(),
        4096 - indices.capacity() * std::mem::size_of::<usize>()
    );
    drop(targets);
    drop(indices);
    drop(_storage);
    assert_eq!(bank.available_permits(), 4096);
}

#[test]
fn paired_plan_bytes_exact_capacity_and_one_byte_rejection_refund() {
    let bytes = 3 * std::mem::size_of::<usize>();
    for limit in [bytes - 1, bytes] {
        let bank = Arc::new(Semaphore::new(limit));
        let result = CandidateBudget::default().paired_candidates(
            vec![target("a", "one"), target("a", "two"), target("b", "three")],
            &HashSet::new(),
            |_| true,
            &bank,
        );
        match result {
            PairedCandidates::Incomplete(error) => {
                assert_eq!(limit, bytes - 1);
                assert_eq!(
                    crate::output::operation_error_class(error.as_ref()),
                    ios_core::ErrorClass::ResourceLimit
                );
                assert_eq!(bank.available_permits(), limit);
            }
            complete @ PairedCandidates::Complete { .. } => {
                assert_eq!(limit, bytes);
                assert_eq!(bank.available_permits(), 0);
                drop(complete);
                assert_eq!(bank.available_permits(), bytes);
            }
        }
    }
}

#[test]
fn paired_plan_bytes_logical_refusal_keeps_original_candidate_budget_and_refunds_backing() {
    let bank = Arc::new(Semaphore::new(4096));
    let mut budget = CandidateBudget::default();
    budget.admit(MAX_CANDIDATE_TEXT_BYTES, 0).unwrap();
    let original = (budget.count, budget.bytes);
    let result =
        budget.paired_candidates(vec![target("a", "one")], &HashSet::new(), |_| true, &bank);
    let PairedCandidates::Incomplete(error) = result else {
        panic!("over-budget plan accepted")
    };
    assert_eq!(
        crate::output::operation_error_class(error.as_ref()),
        ios_core::ErrorClass::ResourceLimit
    );
    assert_eq!((budget.count, budget.bytes), original);
    assert_eq!(bank.available_permits(), 4096);
}

#[test]
fn paired_plan_bytes_empty_plan_has_zero_backing_with_zero_bank() {
    let bank = Arc::new(Semaphore::new(0));
    let complete =
        CandidateBudget::default().paired_candidates(Vec::new(), &HashSet::new(), |_| true, &bank);
    assert!(
        matches!(&complete, PairedCandidates::Complete { indices, .. } if indices.capacity() == 0)
    );
    drop(complete);
    assert_eq!(bank.available_permits(), 0);
}
