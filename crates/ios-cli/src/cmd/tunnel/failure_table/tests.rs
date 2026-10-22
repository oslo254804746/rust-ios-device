//! Actual backing, single paid key, original node rollback and last owner.
use super::*;

const BANK: usize = 262_144;

#[test]
fn identity_full_allocation_parallel_last_clones_keep_original_host_and_refund_once() {
    let original = Route::Mobdev2("parallel-original-host".into());
    let length = super::super::candidate_input::route_length(&original);
    let total = Identity::allocation_bytes() + length;
    let bank = Arc::new(Semaphore::new(total));
    let identity = Identity::copied(&original, Some(19), &bank).unwrap();
    assert_eq!(
        std::mem::size_of::<Identity>(),
        std::mem::size_of::<usize>()
    );
    let host = super::super::candidate_input::RouteRef::from(&identity.route)
        .host()
        .unwrap()
        .as_ptr() as usize;
    let mut owners = (0..7).map(|_| identity.clone()).collect::<Vec<_>>();
    owners.push(identity);
    drop(original);
    assert_eq!(bank.available_permits(), 0);
    let barrier = Arc::new(std::sync::Barrier::new(owners.len()));
    std::thread::scope(|scope| {
        for owner in owners {
            let barrier = barrier.clone();
            scope.spawn(move || {
                assert_eq!(
                    super::super::candidate_input::RouteRef::from(&owner.route)
                        .host()
                        .unwrap()
                        .as_ptr() as usize,
                    host
                );
                assert_eq!(owner.native_id, Some(19));
                barrier.wait();
                drop(owner);
            });
        }
    });
    assert_eq!(bank.available_permits(), total);
}

#[test]
fn node_body_original_pointer_and_outer_grant_survive_table_removal_until_last_owner() {
    let bank = Arc::new(Semaphore::new(BANK));
    let input = Input::copied("original-body-node", super::super::RouteRef::Usbmux, &bank).unwrap();
    let body = Arc::new(std::sync::Mutex::new(Vec::new()));
    let captured = body.clone();
    let charged = bank.clone();
    let node = super::super::metadata_vec::with_allocation_hook(
        move |layout, pointer| {
            assert!(charged.available_permits() <= BANK - layout.size());
            captured.lock().unwrap().push((layout.size(), pointer));
        },
        || Node::input(input, Some(7), &bank).unwrap(),
    );
    let observed = body.lock().unwrap();
    assert_eq!(observed.len(), 3);
    assert_eq!(observed[0].0, std::mem::size_of::<Node>());
    assert_eq!(observed[1].0, Identity::allocation_bytes());
    assert_eq!(observed[2].0, FailureVersion::body_bytes());
    let original_version = node.row.version.as_ptr() as usize;
    assert!(observed[2].1 <= original_version && original_version <= observed[2].1 + observed[2].0);
    assert!(node.row.version.is_from(&bank));
    assert_ne!(observed[2].1, observed[0].1);
    assert_ne!(observed[2].1, observed[1].1);
    let original_identity = std::ptr::from_ref(&*node.row.identity.0) as usize;
    assert!(
        observed[1].1 <= original_identity && original_identity < observed[1].1 + observed[1].0
    );
    let pointer = observed[0].1;
    assert_eq!(&*node as *const Node as usize, pointer);
    drop(observed);
    assert_eq!(node.bytes_in(&bank).unwrap(), std::mem::size_of::<Node>());
    let mut table = table(&bank);
    assert!(Table::commit(table.insert_reserved(node)).is_none());
    let backing = table.backing_bytes();
    let held = bank
        .clone()
        .try_acquire_many_owned(bank.available_permits() as u32)
        .unwrap();
    let mut removed = table.take_all();
    assert_eq!(bank.available_permits(), backing);
    let retained = removed.next().unwrap();
    drop((removed, table));
    assert_eq!(&*retained as *const Node as usize, pointer);
    assert_eq!(
        retained.bytes_in(&bank).unwrap(),
        std::mem::size_of::<Node>()
    );
    assert_eq!(bank.available_permits(), backing);
    drop(retained);
    assert_eq!(bank.available_permits(), BANK - held.num_permits());
    drop(held);
    assert_eq!(bank.available_permits(), BANK);
}

#[test]
fn node_body_last_byte_refusal_precedes_allocation_identity_and_original_key_claim() {
    let bank = Arc::new(Semaphore::new(BANK));
    let input = Input::copied("body-refusal-key", super::super::RouteRef::Usbmux, &bank).unwrap();
    let leave = std::mem::size_of::<Node>() - 1;
    let held = bank
        .clone()
        .try_acquire_many_owned((bank.available_permits() - leave) as u32)
        .unwrap();
    let error = super::super::metadata_vec::with_allocation_hook(
        |_, _| panic!("refused Node body allocated"),
        || Node::input(input.clone(), None, &bank).err().unwrap(),
    );
    assert_eq!(
        crate::output::operation_error_class(error.as_ref()),
        ios_core::ErrorClass::ResourceLimit
    );
    assert_eq!(bank.available_permits(), leave);
    let key = input.outcome_key().unwrap();
    assert_eq!(key.0, "body-refusal-key");
    drop((key, error, held, input));
    assert_eq!(bank.available_permits(), BANK);
}

#[test]
fn node_body_empty_destination_refunds_after_identity_refusal_without_claiming_original_key() {
    let bank = Arc::new(Semaphore::new(BANK));
    let input = Input::copied(
        "identity-refusal-key",
        super::super::RouteRef::Usbmux,
        &bank,
    )
    .unwrap();
    let leave = std::mem::size_of::<Node>() + Identity::allocation_bytes() - 1;
    let held = bank
        .clone()
        .try_acquire_many_owned((bank.available_permits() - leave) as u32)
        .unwrap();
    let count = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let captured = count.clone();
    let error = super::super::metadata_vec::with_allocation_hook(
        move |layout, _| {
            assert_eq!(layout, std::alloc::Layout::new::<Node>());
            captured.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        },
        || Node::input(input.clone(), None, &bank).err().unwrap(),
    );
    assert_eq!(
        crate::output::operation_error_class(error.as_ref()),
        ios_core::ErrorClass::ResourceLimit
    );
    assert_eq!(count.load(std::sync::atomic::Ordering::SeqCst), 1);
    assert_eq!(bank.available_permits(), leave);
    let key = input.outcome_key().unwrap();
    assert_eq!(key.0, "identity-refusal-key");
    drop((key, error, held, input));
    assert_eq!(bank.available_permits(), BANK);
}

#[test]
fn prepaid_selector_actual_failure_identity_shares_original_buffer_through_last_clone() {
    let original = Route::Mobdev2("原-failure-host".into());
    let length = super::super::candidate_input::route_length(&original);
    let total = Identity::allocation_bytes() + length;
    let bank = Arc::new(Semaphore::new(total));
    let seen = Arc::new(std::sync::Mutex::new(Vec::new()));
    let captured = seen.clone();
    let charged = bank.clone();
    let identity = super::super::metadata_vec::with_allocation_hook(
        move |layout, pointer| {
            assert_eq!(charged.available_permits(), 0);
            captured.lock().unwrap().push((layout, pointer));
        },
        || Identity::copied(&original, Some(17), &bank).unwrap(),
    );
    let pointer = super::super::candidate_input::RouteRef::from(&identity.route)
        .host()
        .unwrap()
        .as_ptr() as usize;
    let observed = seen.lock().unwrap();
    assert_eq!(observed.len(), 2);
    assert_eq!(
        observed[0],
        (std::alloc::Layout::array::<u8>(length).unwrap(), pointer)
    );
    assert_eq!(observed[1].0.size(), Identity::allocation_bytes());
    let data = std::ptr::from_ref(&*identity.0) as usize;
    assert!(observed[1].1 <= data && data < observed[1].1 + observed[1].0.size());
    assert_eq!(identity.0.storage().num_permits(), total);
    assert!(Arc::ptr_eq(identity.0.storage().semaphore(), &bank));
    drop(observed);
    assert_eq!(identity.native_id, Some(17));
    assert_eq!(route_capacity(&identity.route), length);
    let retained = identity.clone();
    drop((identity, original));
    assert_eq!(bank.available_permits(), 0);
    assert_eq!(
        super::super::candidate_input::RouteRef::from(&retained.route)
            .host()
            .unwrap()
            .as_ptr() as usize,
        pointer
    );
    drop(retained);
    assert_eq!(bank.available_permits(), total);
}
fn identity(host: &str) -> AttemptIdentity {
    AttemptIdentity {
        route: Route::Mobdev2(host.into()),
        native_id: None,
    }
}
fn table(bank: &Arc<Semaphore>) -> Table {
    let mut table = Table::new(bank);
    table.adopt(Destination::new(bank).unwrap());
    table
}
fn host(row: &FailedTunnelAttempt) -> &str {
    match &row.identity.route {
        Route::Mobdev2(host) => host,
        _ => panic!("mobdev2 row"),
    }
}

#[test]
fn failure_destination_one_byte_refusal_and_original_backing_refund() {
    let bytes = Destination::minimum_bytes();
    let bank = Arc::new(Semaphore::new(bytes - 1));
    let error = Destination::new(&bank).err().unwrap();
    assert_eq!(
        crate::output::operation_error_class(error.as_ref()),
        ios_core::ErrorClass::ResourceLimit
    );
    assert_eq!(bank.available_permits(), bytes - 1);
    bank.add_permits(1);
    let destination = Destination::new(&bank).unwrap();
    assert_eq!(bank.available_permits(), 0);
    drop(destination);
    assert_eq!(bank.available_permits(), bytes);
}

#[test]
fn failure_input_single_outcome_key_and_last_node_owner_retains_original_fee() {
    let bank = Arc::new(Semaphore::new(BANK));
    let input = Input::owned("key".into(), Route::Mobdev2("host".into()), &bank).unwrap();
    let weak = input.downgrade();
    let mut table = table(&bank);
    let node = Node::input(input.clone(), None, &bank).unwrap();
    assert!(node.input_owner().unwrap().ptr_eq(&input));
    let key = node.udid.as_ptr();
    let route = host(&node.row).as_ptr();
    assert_ne!(
        route,
        match &input.route {
            Route::Mobdev2(host) => host.as_ptr(),
            _ => unreachable!(),
        }
    );
    assert!(Table::commit(table.insert_reserved(node)).is_none());
    drop(input);
    let removed = table.take_all();
    assert!(table.is_empty());
    drop(table);
    assert!(weak.upgrade().is_some());
    let mut removed = removed;
    let node = removed.next().unwrap();
    assert_eq!(node.udid.as_ptr(), key);
    assert_eq!(host(&node.row).as_ptr(), route);
    drop(removed);
    assert!(bank.available_permits() < BANK);
    drop(node);
    assert!(weak.upgrade().is_none());
    assert_eq!(bank.available_permits(), BANK - Input::body_bytes());
    drop(weak);
    assert_eq!(bank.available_permits(), BANK);
}

#[test]
fn failure_row_snapshot_shares_route_and_refunds_identity_only_after_last_clone() {
    let bank = Arc::new(Semaphore::new(BANK));
    let node = Node::owned("key".into(), identity("retained-route"), &bank).unwrap();
    let snapshot = node.row.clone();
    assert_eq!(host(&snapshot).as_ptr(), host(&node.row).as_ptr());
    let identity_bytes =
        Identity::allocation_bytes() + host(&snapshot).len() + FailureVersion::body_bytes();
    drop(node);
    assert_eq!(bank.available_permits(), BANK - identity_bytes);
    drop(snapshot);
    assert_eq!(bank.available_permits(), BANK);
}

#[test]
fn failure_late_replace_zero_quota_restores_exact_key_route_node_and_version() {
    let bank = Arc::new(Semaphore::new(BANK));
    let mut table = table(&bank);
    let mut old = Node::owned("key".into(), identity("original-host"), &bank).unwrap();
    old.row.fail_count = 7;
    let old_node = &*old as *const Node;
    let old_key = old.udid.as_ptr();
    let old_route = host(&old.row).as_ptr();
    let old_version = old.row.version.clone();
    assert!(Table::commit(table.insert_reserved(old)).is_none());
    let next = Node::owned("key".into(), identity("replacement-host"), &bank).unwrap();
    let next_node = &*next as *const Node;
    let held = bank
        .clone()
        .try_acquire_many_owned(bank.available_permits() as u32)
        .unwrap();
    let change = table.insert_reserved(next);
    let late = table.rollback(change);
    assert_eq!(&*late as *const Node, next_node);
    let current = &table.rows.as_slice()[0];
    assert_eq!(&**current as *const Node, old_node);
    assert_eq!(current.udid.as_ptr(), old_key);
    assert_eq!(host(&current.row).as_ptr(), old_route);
    assert!(FailureVersion::ptr_eq(&current.row.version, &old_version));
    assert_eq!(current.row.fail_count, 7);
    assert_eq!(bank.available_permits(), 0);
    drop(held);
    drop(late);
    drop(old_version);
    drop(table);
    assert_eq!(bank.available_permits(), BANK);
}

#[test]
fn failure_remove_batch_zero_quota_restores_original_nodes_keys_routes_and_order() {
    let bank = Arc::new(Semaphore::new(BANK));
    let mut table = table(&bank);
    for key in ["a", "b", "c"] {
        let node = Node::owned(key.into(), identity(key), &bank).unwrap();
        assert!(Table::commit(table.insert_reserved(node)).is_none());
    }
    let originals = table
        .rows
        .as_slice()
        .iter()
        .map(|node| {
            (
                &**node as *const Node,
                node.udid.as_ptr(),
                host(&node.row).as_ptr(),
                node.row.version.clone(),
            )
        })
        .collect::<Vec<_>>(); // test-observation allocation, not production removal
    let capacity = table.rows.capacity();
    let backing = table.rows.as_slice().as_ptr();
    let held = bank
        .clone()
        .try_acquire_many_owned(bank.available_permits() as u32)
        .unwrap();
    table.select(&|key, _| key == "b");
    let removed = table.remove_selected();
    assert_eq!(table.keys().map(String::as_str).collect::<Vec<_>>(), ["b"]);
    assert_eq!(bank.available_permits(), 0);
    table.restore_all(removed);
    assert_eq!(table.rows.capacity(), capacity);
    assert_eq!(table.rows.as_slice().as_ptr(), backing);
    assert_eq!(
        table.keys().map(String::as_str).collect::<Vec<_>>(),
        ["a", "b", "c"]
    );
    assert_eq!(table.iter().count(), 3);
    assert_eq!(table.values().count(), 3);
    for (node, original) in table.rows.as_slice().iter().zip(&originals) {
        assert_eq!(&**node as *const Node, original.0);
        assert_eq!(node.udid.as_ptr(), original.1);
        assert_eq!(host(&node.row).as_ptr(), original.2);
        assert!(FailureVersion::ptr_eq(&node.row.version, &original.3));
    }
    assert_eq!(bank.available_permits(), 0);
    drop(held);
    drop(originals);
    drop(table);
    assert_eq!(bank.available_permits(), BANK);
}

#[test]
fn failure_destination_survives_clear_and_adopts_without_new_quota() {
    let bank = Arc::new(Semaphore::new(BANK));
    let mut table = table(&bank);
    let old = Node::owned("old".into(), identity("old-host"), &bank).unwrap();
    assert!(Table::commit(table.insert_reserved(old)).is_none());
    let destination = Destination::new(&bank).unwrap();
    let next = Node::owned("next".into(), identity("next-host"), &bank).unwrap();
    drop(table.take_all());
    let held = bank
        .clone()
        .try_acquire_many_owned(bank.available_permits() as u32)
        .unwrap();
    table.adopt(destination);
    assert!(Table::commit(table.insert_reserved(next)).is_none());
    assert_eq!(bank.available_permits(), 0);
    assert!(table.contains_key("next"));
    drop(held);
    drop(table);
    assert_eq!(bank.available_permits(), BANK);
}

#[tokio::test]
async fn failure_actual_retain_deadline_restores_original_nodes_at_zero_quota() {
    let bank = Arc::new(Semaphore::new(BANK));
    let slots = Arc::new(Semaphore::new(3));
    let mut table = table(&bank);
    for key in ["a", "b", "c"] {
        let mut node = Node::owned(key.into(), identity(key), &bank).unwrap();
        node.row.slot = Some(super::super::failure_slot::Slot::reserve(&slots, &bank).unwrap());
        assert!(Table::commit(table.insert_reserved(node)).is_none());
    }
    let originals = table
        .rows
        .as_slice()
        .iter()
        .map(|node| {
            (
                &**node as *const Node,
                node.udid.as_ptr(),
                host(&node.row).as_ptr(),
                node.row.version.clone(),
                &**node.row.slot.as_ref().unwrap() as *const super::super::failure_slot::Slot,
            )
        })
        .collect::<Vec<_>>();
    let lifecycle = super::super::Lifecycle::with_metadata(Arc::clone(&bank));
    let control = lifecycle
        .scan_control(std::time::Duration::from_secs(1))
        .unwrap();
    let deadline = super::super::scan::Deadline::new(std::time::Duration::from_millis(40)).unwrap();
    let held = bank
        .clone()
        .try_acquire_many_owned(bank.available_permits() as u32)
        .unwrap();
    table.on_after_remove(|| std::thread::sleep(std::time::Duration::from_millis(100)));
    let error = table
        .retain(&deadline, &control.epoch, |key, _| key == "b")
        .err()
        .unwrap();
    assert!(error.is::<super::super::scan::DeadlineReached>());
    assert_eq!(bank.available_permits(), 0);
    assert_eq!(slots.available_permits(), 0);
    assert_eq!(
        table.keys().map(String::as_str).collect::<Vec<_>>(),
        ["a", "b", "c"]
    );
    for (node, original) in table.rows.as_slice().iter().zip(&originals) {
        assert_eq!(&**node as *const Node, original.0);
        assert_eq!(node.udid.as_ptr(), original.1);
        assert_eq!(host(&node.row).as_ptr(), original.2);
        assert!(FailureVersion::ptr_eq(&node.row.version, &original.3));
        assert_eq!(
            &**node.row.slot.as_ref().unwrap() as *const super::super::failure_slot::Slot,
            original.4
        );
    }
    drop(error);
    drop(held);
    drop(originals);
    drop(control);
    drop(lifecycle);
    drop(table);
    assert_eq!(bank.available_permits(), BANK);
    assert_eq!(slots.available_permits(), 3);
}

#[test]
fn identity_full_layout_short_refusal_precedes_copy_and_owned_capacity_follows_original_buffer() {
    let original = Route::Mobdev2("原-field-layout-host".into());
    let source = match &original {
        Route::Mobdev2(host) => host.as_ptr(),
        _ => unreachable!(),
    };
    let needed =
        Identity::allocation_bytes() + super::super::candidate_input::route_length(&original);
    let short = Arc::new(Semaphore::new(needed - 1));
    let error = super::super::metadata_vec::with_allocation_hook(
        |_, _| panic!("short identity allocated a root or copied a field"),
        || Identity::copied(&original, Some(11), &short).err().unwrap(),
    );
    assert_eq!(
        crate::output::operation_error_class(error.as_ref()),
        ios_core::ErrorClass::ResourceLimit
    );
    assert_eq!(short.available_permits(), needed - 1);
    assert_eq!(
        match &original {
            Route::Mobdev2(host) => host.as_ptr(),
            _ => unreachable!(),
        },
        source
    );
    let mut moved = String::with_capacity(4096);
    moved.push_str("original-capacity-host");
    let pointer = moved.as_ptr();
    let capacity = moved.capacity();
    let total = Identity::allocation_bytes() + capacity;
    let bank = Arc::new(Semaphore::new(total));
    let identity = Identity::owned(
        AttemptIdentity {
            route: Route::Mobdev2(moved),
            native_id: Some(12),
        },
        &bank,
    )
    .unwrap();
    let retained = identity.clone();
    drop(identity);
    bank.close();
    let Route::Mobdev2(host) = &retained.route else {
        unreachable!()
    };
    assert_eq!(host.as_ptr(), pointer);
    assert_eq!(host.capacity(), capacity);
    assert_eq!(retained.native_id, Some(12));
    assert_eq!(retained.0.storage().num_permits(), total);
    assert_eq!(bank.available_permits(), 0);
    drop(retained);
    assert_eq!(bank.available_permits(), total);
}
