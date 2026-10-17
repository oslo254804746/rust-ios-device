//! Actual backing, single paid key, original node rollback and last owner.
use super::*;

const BANK: usize = 262_144;

#[test]
fn prepaid_selector_actual_failure_identity_shares_original_buffer_through_last_clone() {
    let original = Route::Mobdev2("原-failure-host".into());
    let length = super::super::candidate_input::route_length(&original);
    let total = std::mem::size_of::<IdentityOwner>() + length;
    let bank = Arc::new(Semaphore::new(total));
    let seen = Arc::new(std::sync::Mutex::new(Vec::new()));
    let captured = seen.clone();
    let charged = bank.clone();
    let identity = super::super::metadata_vec::with_allocation_hook(
        move |layout, pointer| {
            assert_eq!(charged.available_permits(), 0);
            assert_eq!(layout, std::alloc::Layout::array::<u8>(length).unwrap());
            captured.lock().unwrap().push(pointer);
        },
        || Identity::copied(&original, Some(17), &bank).unwrap(),
    );
    let pointer = super::super::candidate_input::RouteRef::from(&identity.route)
        .host()
        .unwrap()
        .as_ptr() as usize;
    assert_eq!(seen.lock().unwrap().as_slice(), &[pointer]);
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
    let weak = Arc::downgrade(&input);
    let mut table = table(&bank);
    let node = Node::input(Arc::clone(&input), None, &bank).unwrap();
    assert!(Arc::ptr_eq(node.input_owner().unwrap(), &input));
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
    assert_eq!(bank.available_permits(), BANK);
}

#[test]
fn failure_row_snapshot_shares_route_and_refunds_identity_only_after_last_clone() {
    let bank = Arc::new(Semaphore::new(BANK));
    let node = Node::owned("key".into(), identity("retained-route"), &bank).unwrap();
    let snapshot = node.row.clone();
    assert_eq!(host(&snapshot).as_ptr(), host(&node.row).as_ptr());
    let identity_bytes = std::mem::size_of::<IdentityOwner>() + host(&snapshot).len();
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
    let old_version = Arc::clone(&old.row.version);
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
    assert!(Arc::ptr_eq(&current.row.version, &old_version));
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
                Arc::clone(&node.row.version),
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
        assert!(Arc::ptr_eq(&node.row.version, &original.3));
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
        node.row.slot = Some(Arc::new(slots.clone().try_acquire_owned().unwrap()));
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
                Arc::clone(&node.row.version),
                Arc::as_ptr(node.row.slot.as_ref().unwrap()),
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
        assert!(Arc::ptr_eq(&node.row.version, &original.3));
        assert_eq!(Arc::as_ptr(node.row.slot.as_ref().unwrap()), original.4);
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
