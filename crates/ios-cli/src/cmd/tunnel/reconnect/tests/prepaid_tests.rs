use super::*;
use crate::cmd::tunnel::metadata_vec;

#[tokio::test]
async fn prepaid_selector_actual_receipt_all_capsules_precede_copy_and_last_owner_refunds() {
    let identity = buffered_identity();
    let host_length = super::super::super::candidate_input::route_length(&identity.route);
    let bytes = FailureReceipt::initial_bytes(A, &identity);
    let lifecycle = lifecycle();
    let ticket = lifecycle.ticket(A).await.unwrap();
    for short in [true, false] {
        let total = bytes - usize::from(short) + slot_bytes() + FailureVersion::body_bytes();
        let bank = Arc::new(Semaphore::new(total));
        let admitted = slot(&bank);
        let version = FailureVersion::new(&bank).unwrap();
        let seen = Arc::new(Mutex::new(Vec::new()));
        let captured = seen.clone();
        let charged = bank.clone();
        let result = metadata_vec::with_allocation_hook(
            move |layout, pointer| {
                assert!(
                    !short,
                    "a selector was copied before all capsules were admitted"
                );
                assert_eq!(charged.available_permits(), 0);
                captured.lock().unwrap().push((layout.size(), pointer));
            },
            || FailureReceipt::reserve(A, &identity, &ticket, admitted, version, &bank),
        );
        if short {
            let error = result.err().unwrap();
            assert_eq!(
                crate::output::operation_error_class(error.as_ref()),
                ErrorClass::ResourceLimit
            );
            assert!(seen.lock().unwrap().is_empty());
            assert_eq!(bank.available_permits(), total);
            continue;
        }
        let receipt = result.unwrap();
        let host = super::super::super::candidate_input::RouteRef::from(&receipt.identity.route)
            .host()
            .unwrap();
        let allocations = seen.lock().unwrap();
        assert_eq!(allocations.len(), 4);
        assert_eq!(
            &allocations[..2],
            &[
                (A.len(), receipt.udid.as_ptr() as usize),
                (host_length, host.as_ptr() as usize)
            ]
        );
        // The exact shared original capsule is now visible to this allocator
        // observer too. Its whole allocation follows the same complete grant.
        let original_root = bytes
            - FailureReceipt::body_bytes()
            - A.len()
            - host_length
            - std::mem::size_of::<SourceFailure>();
        assert_eq!(allocations[2].0, original_root);
        assert_ne!(allocations[2].1, 0);
        assert_ne!(allocations[2].1, receipt.udid.as_ptr() as usize);
        assert_ne!(allocations[2].1, host.as_ptr() as usize);
        assert_eq!(allocations[3].0, FailureReceipt::body_bytes());
        let value = std::ptr::from_ref(&*receipt) as usize;
        assert!(allocations[3].1 <= value && value < allocations[3].1 + allocations[3].0);
        assert_ne!(allocations[3].1, allocations[2].1);
        drop(allocations);
        assert_eq!(receipt.udid.capacity(), A.len());
        assert_eq!(
            super::super::super::candidate_input::route_capacity(&receipt.identity.route),
            host_length
        );
        assert_eq!(receipt.identity.native_id, identity.native_id);
        let retained = receipt.clone();
        drop(receipt);
        assert_eq!(bank.available_permits(), 0);
        drop(retained);
        assert_eq!(bank.available_permits(), total);
    }
}

#[tokio::test]
async fn prepaid_selector_actual_retry_node_keys_and_route_use_original_exact_buffers() {
    let identity = buffered_identity();
    let host_length = super::super::super::candidate_input::route_length(&identity.route);
    let total = std::mem::size_of::<Removed>()
        + A.len()
        + host_length
        + slot_bytes()
        + FailureVersion::body_bytes();
    let bank = Arc::new(Semaphore::new(total));
    let admitted = slot(&bank);
    let version = FailureVersion::new(&bank).unwrap();
    let lifecycle = lifecycle();
    let ticket = lifecycle.ticket(A).await.unwrap();
    let seen = Arc::new(Mutex::new(Vec::new()));
    let captured = seen.clone();
    let charged = bank.clone();
    let node = metadata_vec::with_allocation_hook(
        move |layout, pointer| {
            assert_eq!(charged.available_permits(), 0);
            captured.lock().unwrap().push((layout.size(), pointer));
        },
        || {
            Removed::episode(
                A,
                &identity,
                |identity| Episode {
                    identity,
                    ticket,
                    slot: admitted,
                    version,
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
                },
                &bank,
            )
            .unwrap()
        },
    );
    let row = node.row.as_ref().unwrap();
    let host = super::super::super::candidate_input::RouteRef::from(&row.identity.route)
        .host()
        .unwrap();
    assert_eq!(
        seen.lock().unwrap().as_slice(),
        &[
            (
                std::mem::size_of::<Removed>(),
                &*node as *const Removed as usize
            ),
            (A.len(), node.udid.as_ptr() as usize),
            (host_length, host.as_ptr() as usize)
        ]
    );
    assert_eq!(node.udid.capacity(), A.len());
    assert_eq!(
        super::super::super::candidate_input::route_capacity(&row.identity.route),
        host_length
    );
    assert_eq!(row.identity.native_id, identity.native_id);
    drop(identity);
    assert_eq!(bank.available_permits(), 0);
    drop(node);
    assert_eq!(bank.available_permits(), total);
}
