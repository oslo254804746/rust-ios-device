use super::*;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Mutex, Weak};
use tokio::sync::RwLock;
const TOTAL: usize = 2 * 1024 * 1024;
fn table<T>() -> (Table<T>, Arc<Semaphore>, Arc<Semaphore>) {
    let global = Arc::new(Semaphore::new(TOTAL));
    let published = Arc::new(Semaphore::new(TOTAL));
    (Table::new(&global, &published), global, published)
}
fn insert<T>(table: &mut Table<T>, key: &str, value: T) {
    let destination = table.prepare(key).unwrap();
    let slot = table.destination(&destination).unwrap();
    drop(table.publish(slot, destination, value).finish());
}
#[test]
fn publication_table_actual_vec_box_key_layout_dual_banks_and_last_owner() {
    let (mut table, global, published) = table::<Box<u8>>();
    insert(&mut table, "original", Box::new(7));
    let node = table.slots.iter().flatten().next().unwrap();
    let bytes = table.backing_bytes() + std::mem::size_of::<Node<Box<u8>>>() + node.key.capacity();
    let key = node.key.as_ptr();
    let value = &**node.value.as_ref().unwrap() as *const u8;
    assert_eq!(global.available_permits(), TOTAL - bytes);
    assert_eq!(published.available_permits(), TOTAL - bytes);
    let removed = table.remove("original").unwrap();
    assert_eq!(removed.len(), 1);
    assert_eq!(removed.head.as_ref().unwrap().key.as_ptr(), key);
    assert_eq!(
        &**removed.head.as_ref().unwrap().value.as_ref().unwrap() as *const u8,
        value
    );
    assert_eq!(global.available_permits(), TOTAL - bytes);
    drop(removed);
    assert_eq!(global.available_permits(), TOTAL - table.backing_bytes());
    let whole = table.take_all();
    assert_eq!(
        global.available_permits(),
        TOTAL - whole._global.as_ref().unwrap().num_permits()
    );
    assert_eq!(table.capacity(), 0);
    drop(whole);
    assert_eq!(global.available_permits(), TOTAL);
    assert_eq!(published.available_permits(), TOTAL);
}
#[test]
fn publication_table_actual_backing_one_byte_refusal_preserves_empty_destination() {
    let (mut probe, _, _) = table::<u8>();
    drop(probe.prepare("a").unwrap());
    let required = probe.backing_bytes();
    for shared in [true, false] {
        let global = Arc::new(Semaphore::new(if shared { required - 1 } else { TOTAL }));
        let published = Arc::new(Semaphore::new(if shared { TOTAL } else { required - 1 }));
        let mut table = Table::<u8>::new(&global, &published);
        assert!(with_allocation_hook(
            |_, _| panic!("publication backing refusal allocated"),
            || table.prepare("a"),
        )
        .is_err());
        assert!(table.is_empty());
        assert_eq!(table.capacity(), 0);
        assert_eq!(
            global.available_permits(),
            if shared { required - 1 } else { TOTAL }
        );
        assert_eq!(
            published.available_permits(),
            if shared { TOTAL } else { required - 1 }
        );
    }
}
#[test]
fn publication_table_node_one_byte_refusal_preserves_original_pointer_incarnation_and_dual_fees() {
    for short_global in [true, false] {
        let (mut table, global, published) = table::<u8>();
        insert(&mut table, "original", 7);
        let proof = table.proof("original");
        let key = table.get_key_value("original").unwrap().0.as_ptr();
        let before = (global.available_permits(), published.available_permits());
        let required = std::mem::size_of::<Node<u8>>() + "new-node".len();
        let bank = if short_global { &global } else { &published };
        let held = bank
            .clone()
            .try_acquire_many_owned((bank.available_permits() - required + 1) as u32)
            .unwrap();
        let admitted = (global.available_permits(), published.available_permits());
        let error = with_allocation_hook(
            |_, _| panic!("whole node/key refusal allocated before BOTH grants"),
            || table.prepare("new-node"),
        )
        .err()
        .expect("one-byte shortage created publication");
        assert_eq!(
            crate::output::operation_error_class(error.as_ref()),
            ios_core::ErrorClass::ResourceLimit
        );
        assert_eq!(table.len(), 1);
        assert_eq!(table.proof("original"), proof);
        assert_eq!(table.get_key_value("original").unwrap().0.as_ptr(), key);
        assert_eq!(
            (global.available_permits(), published.available_permits()),
            admitted
        );
        drop(held);
        assert_eq!(
            (global.available_permits(), published.available_permits()),
            before
        );
        drop(table.take_all());
        assert_eq!(global.available_permits(), TOTAL);
        assert_eq!(published.available_permits(), TOTAL);
    }
}
#[test]
fn publication_table_original_node_key_fee_and_incarnation_late_replacement_zero_quota_rollback() {
    let (mut table, global, published) = table::<Box<u8>>();
    insert(&mut table, "same", Box::new(1));
    let (key, value) = table.get_key_value("same").unwrap();
    let key_pointer = key.as_ptr();
    let value_pointer = &**value as *const u8;
    let proof = table.proof("same");
    let destination = table.prepare("same").unwrap();
    let held = global
        .clone()
        .try_acquire_many_owned(global.available_permits() as u32)
        .unwrap();
    let held_sub = published
        .clone()
        .try_acquire_many_owned(published.available_permits() as u32)
        .unwrap();
    let slot = table.destination(&destination).unwrap();
    let change = table.publish(slot, destination, Box::new(2));
    assert_eq!(table.len(), 1);
    assert_eq!(table.get_key_value("same").unwrap().0.as_ptr(), key_pointer);
    assert_eq!(table.proof("same").unwrap().0, proof.unwrap().0);
    assert_ne!(table.proof("same").unwrap().1, proof.unwrap().1);
    let late = table.rollback(change);
    assert_eq!(table.get_key_value("same").unwrap().0.as_ptr(), key_pointer);
    assert_eq!(&**table.get("same").unwrap() as *const u8, value_pointer);
    assert_eq!(table.proof("same"), proof);
    assert_eq!(global.available_permits(), 0);
    drop(late);
    assert!(global.available_permits() > 0);
    drop((held, held_sub));
    drop(table.take_all());
    assert_eq!(global.available_permits(), TOTAL);
    assert_eq!(published.available_permits(), TOTAL);
}
#[test]
fn publication_table_clear_new_generation_reuses_real_destination_without_incarnation_aba() {
    let (mut table, global, published) = table::<u8>();
    insert(&mut table, "a", 1);
    let before = table.proof("a").unwrap().1;
    let old = table.take_all();
    assert!(table.is_empty());
    assert_eq!(table.capacity(), 0);
    insert(&mut table, "a", 2);
    assert!(table.proof("a").unwrap().1 > before);
    assert!(global.available_permits() < TOTAL - table.backing_bytes());
    drop(old);
    assert_eq!(table["a"], 2);
    drop(table.take_all());
    assert_eq!(global.available_permits(), TOTAL);
    assert_eq!(published.available_permits(), TOTAL);
}
struct GuardObserved {
    table: Weak<RwLock<Table<GuardObserved>>>,
    released: Arc<AtomicUsize>,
}
impl Drop for GuardObserved {
    fn drop(&mut self) {
        assert!(self.table.upgrade().unwrap().try_read().is_ok());
        self.released.fetch_add(1, Ordering::SeqCst);
    }
}
#[tokio::test]
async fn publication_table_remove_retain_whole_returns_actual_owners_until_guard_ends() {
    for mode in 0..3 {
        let global = Arc::new(Semaphore::new(TOTAL));
        let published = Arc::new(Semaphore::new(TOTAL));
        let table = Arc::new(RwLock::new(Table::new(&global, &published)));
        let released = Arc::new(AtomicUsize::new(0));
        let mut guard = table.write().await;
        insert(
            &mut guard,
            "a",
            GuardObserved {
                table: Arc::downgrade(&table),
                released: Arc::clone(&released),
            },
        );
        match mode {
            0 => {
                let owner = guard.remove("a").unwrap();
                assert_eq!(released.load(Ordering::SeqCst), 0);
                drop(guard);
                drop(owner);
            }
            1 => {
                let owner = guard.retain(|_, _| false);
                assert_eq!(released.load(Ordering::SeqCst), 0);
                drop(guard);
                drop(owner);
            }
            _ => {
                let owner = guard.take_all();
                assert_eq!(released.load(Ordering::SeqCst), 0);
                drop(guard);
                drop(owner);
            }
        }
        assert_eq!(released.load(Ordering::SeqCst), 1);
        let whole = table.write().await.take_all();
        drop(whole);
        assert_eq!(global.available_permits(), TOTAL);
        assert_eq!(published.available_permits(), TOTAL);
    }
}
#[test]
fn publication_table_predicate_unwind_restores_box_key_value_original_slots_at_zero_quota() {
    let (mut table, global, _) = table::<Box<u8>>();
    insert(&mut table, "a", Box::new(1));
    insert(&mut table, "b", Box::new(2));
    let before = [table.proof("a"), table.proof("b")];
    let held = global
        .clone()
        .try_acquire_many_owned(global.available_permits() as u32)
        .unwrap();
    let calls = Mutex::new(0);
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        table.retain(|_, _| {
            let mut count = calls.lock().unwrap();
            *count += 1;
            if *count == 2 {
                panic!("synthetic original predicate unwind");
            }
            false
        })
    }));
    assert!(result.is_err());
    assert_eq!(table.len(), 2);
    assert_eq!([table.proof("a"), table.proof("b")], before);
    assert_eq!(global.available_permits(), 0);
    drop(held);
}

#[test]
fn publication_body_exact_dual_grants_precede_actual_key_and_whole_box_backings() {
    let (mut table, global, published) = table::<u8>();
    // Establish the real fixed slots first; this test isolates the complete
    // per-node grants, and the original backing test covers the initial slots.
    drop(table.prepare("seed").unwrap());
    let before = (global.available_permits(), published.available_permits());
    let body = std::mem::size_of::<Node<u8>>();
    let key = "actual-original-node";
    let bytes = body + key.len();
    let seen = Arc::new(Mutex::new(Vec::new()));
    let captured = seen.clone();
    let g = global.clone();
    let p = published.clone();
    let node = with_allocation_hook(
        move |layout, pointer| {
            assert_eq!(g.available_permits(), before.0 - bytes);
            assert_eq!(p.available_permits(), before.1 - bytes);
            captured.lock().unwrap().push((layout, pointer));
        },
        || table.prepare(key).unwrap(),
    );
    let node_pointer = &*node as *const Node<u8> as usize;
    let key_pointer = node.key.as_ptr() as usize;
    let seen = seen.lock().unwrap();
    assert_eq!(
        seen.len(),
        2,
        "exactly the original key and original Node Box"
    );
    assert_eq!(
        seen[0],
        (Layout::array::<u8>(key.len()).unwrap(), key_pointer)
    );
    assert_eq!(seen[1], (Layout::new::<Node<u8>>(), node_pointer));
    assert_eq!(node.key.capacity(), key.len());
    assert_eq!(node._global.num_permits(), bytes);
    assert_eq!(node._published.num_permits(), bytes);
    assert!(Arc::ptr_eq(node._global.semaphore(), &global));
    assert!(Arc::ptr_eq(node._published.semaphore(), &published));
    assert_eq!(
        table.backing_bytes(),
        table.capacity() * std::mem::size_of::<Option<OwnedNode<u8>>>(),
    );
    drop(seen);
    drop(node);
    assert_eq!(
        (global.available_permits(), published.available_permits()),
        before
    );
    drop(table.take_all());
    assert_eq!(global.available_permits(), TOTAL);
    assert_eq!(published.available_permits(), TOTAL);
}

#[repr(align(128))]
struct InPlaceValue {
    location: Arc<AtomicUsize>,
    drops: Arc<AtomicUsize>,
    global: Arc<Semaphore>,
    published: Arc<Semaphore>,
    unwind: bool,
    _pinned: std::marker::PhantomPinned,
}
impl Drop for InPlaceValue {
    fn drop(&mut self) {
        assert_eq!(
            self as *const Self as usize,
            self.location.load(Ordering::SeqCst)
        );
        assert_eq!(self.global.available_permits(), 0);
        assert_eq!(self.published.available_permits(), 0);
        self.drops.fetch_add(1, Ordering::SeqCst);
        assert!(!self.unwind, "original publication value destructor unwind");
    }
}

#[test]
fn publication_body_box_dealloc_precedes_both_refunds_on_unused_removed_and_value_unwind() {
    for mode in 0..3 {
        let (mut table, global, published) = table::<InPlaceValue>();
        let node = table.prepare("closed-original").unwrap();
        let node_pointer = &*node as *const Node<InPlaceValue> as usize;
        assert_eq!(node_pointer % 128, 0);
        let paid = std::mem::size_of::<Node<InPlaceValue>>() + node.key.capacity();
        assert_eq!(node._global.num_permits(), paid);
        assert_eq!(node._published.num_permits(), paid);
        let drops = Arc::new(AtomicUsize::new(0));
        let location = Arc::new(AtomicUsize::new(0));
        let (unused, removed) = if mode == 0 {
            (Some(node), None)
        } else {
            let slot = table.destination(&node).unwrap();
            drop(
                table
                    .publish(
                        slot,
                        node,
                        InPlaceValue {
                            location: location.clone(),
                            drops: drops.clone(),
                            global: global.clone(),
                            published: published.clone(),
                            unwind: mode == 2,
                            _pinned: std::marker::PhantomPinned,
                        },
                    )
                    .finish(),
            );
            location.store(
                table.get("closed-original").unwrap() as *const InPlaceValue as usize,
                Ordering::SeqCst,
            );
            assert_eq!(table.proof("closed-original").unwrap().0, node_pointer);
            let removed = table.remove("closed-original").unwrap();
            assert_eq!(
                &**removed.head.as_ref().unwrap() as *const Node<InPlaceValue> as usize,
                node_pointer
            );
            (None, Some(removed))
        };
        let held_global = global
            .clone()
            .try_acquire_many_owned(global.available_permits() as u32)
            .unwrap();
        let held_published = published
            .clone()
            .try_acquire_many_owned(published.available_permits() as u32)
            .unwrap();
        global.close();
        published.close();
        let freed = Arc::new(AtomicUsize::new(0));
        let captured = freed.clone();
        let captured_drops = drops.clone();
        let g = global.clone();
        let p = published.clone();
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            after_box_free(
                move || {
                    // Box has completed its exact allocation's deallocation here.
                    // Its ORIGINAL own-bank grants must both still be held.
                    assert_eq!(g.available_permits(), 0);
                    assert_eq!(p.available_permits(), 0);
                    assert_eq!(
                        captured_drops.load(Ordering::SeqCst),
                        usize::from(mode != 0)
                    );
                    captured.fetch_add(1, Ordering::SeqCst);
                },
                || drop((unused, removed)),
            );
        }));
        assert_eq!(result.is_err(), mode == 2);
        assert_eq!(freed.load(Ordering::SeqCst), 1);
        assert_eq!(drops.load(Ordering::SeqCst), usize::from(mode != 0));
        assert_eq!(global.available_permits(), paid);
        assert_eq!(published.available_permits(), paid);
        drop(table.take_all());
        drop((held_global, held_published));
        assert_eq!(global.available_permits(), TOTAL);
        assert_eq!(published.available_permits(), TOTAL);
    }
}

struct CountedValue(Arc<AtomicUsize>);
impl Drop for CountedValue {
    fn drop(&mut self) {
        self.0.fetch_add(1, Ordering::SeqCst);
    }
}

#[test]
fn publication_body_full_intrusive_chain_closed_pool_drops_iteratively_after_original_restore() {
    let (mut table, global, published) = table::<CountedValue>();
    let drops = Arc::new(AtomicUsize::new(0));
    for index in 0..MAX_ROWS {
        insert(
            &mut table,
            &format!("node-{index:04}"),
            CountedValue(drops.clone()),
        );
    }
    let first = table.proof("node-0000").unwrap();
    let first_key = table.get_key_value("node-0000").unwrap().0.as_ptr();
    let paid = table
        .slots
        .iter()
        .flatten()
        .next()
        .unwrap()
        ._global
        .num_permits();
    let held_global = global
        .clone()
        .try_acquire_many_owned(global.available_permits() as u32)
        .unwrap();
    let held_published = published
        .clone()
        .try_acquire_many_owned(published.available_permits() as u32)
        .unwrap();
    global.close();
    published.close();
    let selected = table.retain(|_, _| false);
    assert_eq!(selected.len(), MAX_ROWS);
    assert_eq!(drops.load(Ordering::SeqCst), 0);
    assert_eq!(global.available_permits(), 0);
    assert_eq!(published.available_permits(), 0);
    table.restore(selected);
    assert_eq!(table.proof("node-0000"), Some(first));
    assert_eq!(
        table.get_key_value("node-0000").unwrap().0.as_ptr(),
        first_key
    );
    assert_eq!(table.len(), MAX_ROWS);
    let selected = table.retain(|_, _| false);
    let freed = Arc::new(AtomicUsize::new(0));
    let captured = freed.clone();
    let g = global.clone();
    let p = published.clone();
    after_box_free(
        move || {
            let completed = captured.fetch_add(1, Ordering::SeqCst);
            assert_eq!(g.available_permits(), completed * paid);
            assert_eq!(p.available_permits(), completed * paid);
        },
        || drop(selected),
    );
    assert_eq!(freed.load(Ordering::SeqCst), MAX_ROWS);
    assert_eq!(drops.load(Ordering::SeqCst), MAX_ROWS);
    assert_eq!(global.available_permits(), MAX_ROWS * paid);
    assert_eq!(published.available_permits(), MAX_ROWS * paid);
    assert_eq!(table.len(), 0);
    drop(table.take_all());
    drop((held_global, held_published));
    assert_eq!(global.available_permits(), TOTAL);
    assert_eq!(published.available_permits(), TOTAL);
}
