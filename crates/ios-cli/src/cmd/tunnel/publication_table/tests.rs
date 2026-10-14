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
        assert!(table.prepare("a").is_err());
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
        let error = table
            .prepare("new-node")
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
