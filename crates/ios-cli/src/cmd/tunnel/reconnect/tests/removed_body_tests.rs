use super::*;
use crate::cmd::tunnel::metadata_vec;

fn row(identity: AttemptIdentity, ticket: Ticket) -> Episode {
    Episode {
        identity,
        ticket,
        slot: slot(),
        version: Arc::new(()),
        snapshot: Snapshot {
            schedule: ScheduleProvenance::Episode,
            phase: Phase::Connecting,
            attempts: 0,
            failures: 0,
            first_attempt: Instant::now(),
            deadline: None,
            next_attempt: None,
            last_class: None,
            sequence: 0,
        },
        pending_event: None,
    }
}

#[tokio::test]
async fn removed_body_episode_into_inner_refunds_only_original_body_while_selectors_remain() {
    let identity = buffered_identity();
    let body = std::mem::size_of::<Removed>();
    let fields = A.len() + super::super::super::candidate_input::route_length(&identity.route);
    let bank = Arc::new(Semaphore::new(body + fields));
    let lifecycle = lifecycle();
    let ticket = lifecycle.ticket(A).await.unwrap();
    let node = Removed::episode(A, &identity, |identity| row(identity, ticket), &bank).unwrap();
    let key = node.udid.as_ptr();
    let host = super::super::super::candidate_input::RouteRef::from(
        &node.row.as_ref().unwrap().identity.route,
    )
    .host()
    .unwrap()
    .as_ptr();
    assert_eq!(bank.available_permits(), 0);
    assert_eq!(node.bytes_in(&bank).unwrap(), body);
    let value = metadata_vec::with_allocation_hook(
        |_, _| panic!("Removed into_inner allocated"),
        || node.into_inner(),
    );
    assert_eq!(bank.available_permits(), body);
    assert_eq!(value.udid.as_ptr(), key);
    assert_eq!(
        super::super::super::candidate_input::RouteRef::from(
            &value.row.as_ref().unwrap().identity.route
        )
        .host()
        .unwrap()
        .as_ptr(),
        host
    );
    assert_eq!(
        value.row.as_ref().unwrap().identity.native_id,
        identity.native_id
    );
    drop(value);
    assert_eq!(bank.available_permits(), body + fields);
}

#[tokio::test]
async fn removed_body_cause_last_byte_refusal_and_original_receipt_handoff_are_exact() {
    let total = 32 * 1024;
    let bank = Arc::new(Semaphore::new(total));
    let lifecycle = lifecycle();
    let ticket = lifecycle.ticket(A).await.unwrap();
    let receipt =
        FailureReceipt::reserve(A, &identity(), &ticket, slot(), Arc::new(()), &bank).unwrap();
    let body = std::mem::size_of::<Removed>();
    let leave = body - 1;
    let held = bank
        .clone()
        .try_acquire_many_owned((bank.available_permits() - leave) as u32)
        .unwrap();
    let error = metadata_vec::with_allocation_hook(
        |_, _| panic!("refused cause body allocated"),
        || Removed::receipt(receipt.clone(), &bank).err().unwrap(),
    );
    assert_eq!(
        crate::output::operation_error_class(error.as_ref()),
        ErrorClass::ResourceLimit
    );
    assert_eq!(bank.available_permits(), leave);
    drop((error, held));
    let before = bank.available_permits();
    let seen = Arc::new(Mutex::new(Vec::new()));
    let captured = seen.clone();
    let charged = bank.clone();
    let node = metadata_vec::with_allocation_hook(
        move |layout, pointer| {
            assert_eq!(layout, std::alloc::Layout::new::<Removed>());
            assert_eq!(charged.available_permits(), before - body);
            captured.lock().unwrap().push(pointer);
        },
        || Removed::receipt(receipt.clone(), &bank).unwrap(),
    );
    assert_eq!(
        seen.lock().unwrap().as_slice(),
        &[&*node as *const Removed as usize]
    );
    assert_eq!(node.bytes_in(&bank).unwrap(), body);
    let value = node.into_inner();
    assert_eq!(bank.available_permits(), before);
    assert!(value.udid.is_empty());
    assert!(Arc::ptr_eq(value.cause.as_ref().unwrap(), &receipt));
    drop(value);
    assert_eq!(bank.available_permits(), before);
    drop(receipt);
    assert_eq!(bank.available_permits(), total);
}

#[tokio::test]
async fn removed_body_actual_empty_destination_and_selectors_refund_on_row_constructor_unwind() {
    let identity = buffered_identity();
    let total = std::mem::size_of::<Removed>()
        + A.len()
        + super::super::super::candidate_input::route_length(&identity.route);
    let bank = Arc::new(Semaphore::new(total));
    let seen = Arc::new(Mutex::new(Vec::new()));
    let captured = seen.clone();
    let charged = bank.clone();
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        metadata_vec::with_allocation_hook(
            move |layout, _| {
                assert_eq!(charged.available_permits(), 0);
                captured.lock().unwrap().push(layout);
            },
            || {
                Removed::episode(
                    A,
                    &identity,
                    |_| panic!("fixture row construction unwind"),
                    &bank,
                )
            },
        )
    }));
    assert!(result.is_err());
    let seen = seen.lock().unwrap();
    assert_eq!(seen.len(), 3);
    assert_eq!(seen[0], std::alloc::Layout::new::<Removed>());
    assert_eq!(bank.available_permits(), total);
}

#[tokio::test]
async fn removed_body_actual_ledger_rollback_and_last_batch_keep_original_boxes_and_io_receipt() {
    let total = 32 * 1024;
    let bank = Arc::new(Semaphore::new(total));
    let policy = RetryPolicy::new(
        Duration::from_secs(1),
        Duration::from_secs(1),
        NonZeroU32::new(1),
        None,
    )
    .unwrap();
    let ledger = Ledger::with_metadata(policy, bank.clone()).unwrap();
    let lifecycle = lifecycle();
    let identity = buffered_identity();
    let mut first_pointer = 0;
    for key in [A, B] {
        let ticket = lifecycle.ticket(key).await.unwrap();
        let attempt = ticket
            .commit(|| ledger.start(key, &identity, &ticket, slot()))
            .unwrap();
        let (original, pointer) = io_original("original-body-owner-io");
        if key == A {
            first_pointer = pointer;
        }
        let result = failed_result(attempt.source_failed(original));
        attempt.finish(&result);
        drop(result);
    }
    let receipt = ledger.receipt(A).unwrap();
    let held = bank
        .clone()
        .try_acquire_many_owned(bank.available_permits() as u32)
        .unwrap();
    let mut state = ledger.lock();
    let originals = state
        .rows
        .as_slice()
        .iter()
        .chain(state.causes.as_slice())
        .map(|node| {
            assert_eq!(
                node.bytes_in(&bank).unwrap(),
                std::mem::size_of::<Removed>()
            );
            &**node as *const Removed as usize
        })
        .collect::<Vec<_>>();
    state.select(&|_, _, _| true);
    let removed = state.remove_selected();
    assert_eq!(bank.available_permits(), 0);
    state.restore(removed);
    let restored = state
        .rows
        .as_slice()
        .iter()
        .chain(state.causes.as_slice())
        .map(|node| &**node as *const Removed as usize)
        .collect::<Vec<_>>();
    assert_eq!(restored, originals);
    assert_eq!(pointer(state.cause(A).unwrap().primary()), first_pointer);
    assert_eq!(bank.available_permits(), 0);
    state.select(&|_, _, _| true);
    let mut removed = state.remove_selected();
    state.release_empty_backing();
    drop(state);
    drop(ledger);
    let first = removed.next().unwrap();
    assert_eq!(first.udid, A);
    assert_eq!(&*first as *const Removed as usize, originals[0]);
    let before = bank.available_permits();
    drop(first);
    assert_eq!(
        bank.available_permits(),
        before
            + std::mem::size_of::<Removed>()
            + A.len()
            + super::super::super::candidate_input::route_length(&identity.route)
    );
    drop(removed);
    assert_eq!(pointer(receipt.primary()), first_pointer);
    let receipt_bytes =
        FailureReceipt::initial_bytes(A, &identity) - std::mem::size_of::<SourceFailure>();
    assert_eq!(
        bank.available_permits(),
        total - held.num_permits() - receipt_bytes
    );
    drop((held, receipt));
    assert_eq!(bank.available_permits(), total);
}
