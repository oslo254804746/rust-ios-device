use super::*;
use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Mutex,
};
use std::time::Duration;
use tokio::sync::{watch, RwLock};

fn epoch() -> (Epoch, watch::Sender<Option<Arc<()>>>) {
    let generation = Arc::new(());
    let (sender, changes) = watch::channel(Some(Arc::clone(&generation)));
    (
        Epoch {
            changes,
            generation,
        },
        sender,
    )
}

fn entries() -> HashMap<String, Box<u8>> {
    [("a".to_owned(), Box::new(1)), ("b".to_owned(), Box::new(2))].into()
}

#[test]
fn publication_retain_actual_backing_last_owner_and_original_key_value_buffers() {
    let bank = Arc::new(Semaphore::new(4096));
    let (epoch, _sender) = epoch();
    let clock = Deadline::new(Duration::from_secs(1)).unwrap();
    let mut table = entries();
    let capacity = table.capacity();
    let before = table
        .iter()
        .map(|(key, value)| (key.clone(), key.as_ptr(), &**value as *const u8))
        .collect::<Vec<_>>();
    let removed = retain(&epoch, &clock, &mut table, &bank, |key, _| key == "a").unwrap();
    let bytes = removed._rows.capacity() * std::mem::size_of::<Row<Box<u8>>>();
    assert_eq!(bank.available_permits(), 4096 - bytes);
    assert_eq!(table.capacity(), capacity);
    for (key, pointer, value) in &before {
        let (found_key, found_value) = if key == "a" {
            table.get_key_value(key).unwrap()
        } else {
            let entry = removed
                ._rows
                .as_slice()
                .iter()
                .find_map(|row| row.entry.as_ref())
                .unwrap();
            (&entry.0, &entry.1)
        };
        assert_eq!(found_key.as_ptr(), *pointer);
        assert_eq!(&**found_value as *const u8, *value);
    }
    drop(removed);
    assert_eq!(bank.available_permits(), 4096);
}

#[test]
fn publication_retain_one_byte_refusal_preserves_original_table_before_predicate() {
    let mut table = entries();
    let probe = Arc::new(Semaphore::new(4096));
    let prepared = Originals::prepare(&mut table, &probe).unwrap();
    assert_eq!(prepared.table.len(), 0);
    let bytes = 4096 - probe.available_permits();
    drop(prepared);
    let before = table
        .iter()
        .map(|(key, value)| (key.clone(), key.as_ptr(), &**value as *const u8))
        .collect::<Vec<_>>();
    let capacity = table.capacity();
    let bank = Arc::new(Semaphore::new(bytes - 1));
    let (epoch, _sender) = epoch();
    let clock = Deadline::new(Duration::from_secs(1)).unwrap();
    let calls = AtomicUsize::new(0);
    let error = retain(&epoch, &clock, &mut table, &bank, |_, _| {
        calls.fetch_add(1, Ordering::SeqCst);
        false
    })
    .err()
    .unwrap();
    assert_eq!(
        crate::output::operation_error_class(error.as_ref()),
        ios_core::ErrorClass::ResourceLimit
    );
    assert_eq!(calls.load(Ordering::SeqCst), 0);
    assert_eq!(table.capacity(), capacity);
    for (key, pointer, value) in before {
        let (found, found_value) = table.get_key_value(&key).unwrap();
        assert_eq!(found.as_ptr(), pointer);
        assert_eq!(&**found_value as *const u8, value);
    }
    assert_eq!(bank.available_permits(), bytes - 1);
}

#[test]
fn publication_retain_actual_late_clock_zero_quota_restores_original_entries_and_capacity() {
    let bank = Arc::new(Semaphore::new(4096));
    let (epoch, _sender) = epoch();
    let clock = Deadline::new(Duration::from_millis(20)).unwrap();
    let mut table = entries();
    let before = table
        .iter()
        .map(|(key, value)| (key.clone(), key.as_ptr(), &**value as *const u8))
        .collect::<Vec<_>>();
    let capacity = table.capacity();
    let held = Arc::new(Mutex::new(None));
    let captured = Arc::clone(&held);
    let allocated_bank = Arc::clone(&bank);
    AFTER_INSERT.with(|hook| {
        *hook.borrow_mut() = Some(Box::new(move || {
            *captured.lock().unwrap() = Some(
                allocated_bank
                    .clone()
                    .try_acquire_many_owned(allocated_bank.available_permits() as u32)
                    .unwrap(),
            );
            std::thread::sleep(Duration::from_millis(60));
        }))
    });
    let error = retain(&epoch, &clock, &mut table, &bank, |key, _| key == "a")
        .err()
        .unwrap();
    assert!(error.is::<super::super::scan::DeadlineReached>());
    assert_eq!(table.capacity(), capacity);
    for (key, pointer, value) in before {
        let (found, found_value) = table.get_key_value(&key).unwrap();
        assert_eq!(found.as_ptr(), pointer);
        assert_eq!(&**found_value as *const u8, value);
    }
    assert!(
        bank.available_permits() > 0,
        "temporary backing refunded after zero-quota rollback"
    );
    drop(held.lock().unwrap().take());
    assert_eq!(bank.available_permits(), 4096);
}

#[test]
fn publication_retain_stale_epoch_and_predicate_unwind_restore_original_entries() {
    for unwind in [false, true] {
        let bank = Arc::new(Semaphore::new(4096));
        let (epoch, sender) = epoch();
        let clock = Deadline::new(Duration::from_secs(1)).unwrap();
        let mut table = entries();
        let before = table
            .iter()
            .map(|(key, value)| (key.clone(), key.as_ptr(), &**value as *const u8))
            .collect::<Vec<_>>();
        let capacity = table.capacity();
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            retain(&epoch, &clock, &mut table, &bank, |_, _| {
                if unwind {
                    panic!("synthetic predicate unwind");
                }
                sender.send_replace(Some(Arc::new(())));
                false
            })
        }));
        if unwind {
            assert!(result.is_err());
        } else {
            assert!(result
                .unwrap()
                .err()
                .unwrap()
                .is::<ios_core::OperationCancelled>());
        }
        assert_eq!(table.capacity(), capacity);
        for (key, pointer, value) in before {
            let (found, found_value) = table.get_key_value(&key).unwrap();
            assert_eq!(found.as_ptr(), pointer);
            assert_eq!(&**found_value as *const u8, value);
        }
        assert_eq!(bank.available_permits(), 4096);
    }
}

struct GuardObserved {
    table: std::sync::Weak<RwLock<HashMap<String, GuardObserved>>>,
    released: Arc<AtomicUsize>,
}
impl Drop for GuardObserved {
    fn drop(&mut self) {
        let table = self.table.upgrade().unwrap();
        assert!(
            table.try_read().is_ok(),
            "original value must drop outside table guard"
        );
        self.released.fetch_add(1, Ordering::SeqCst);
    }
}

#[tokio::test]
async fn publication_retain_returns_original_removal_until_table_guard_is_released() {
    let bank = Arc::new(Semaphore::new(4096));
    let (epoch, _sender) = epoch();
    let clock = Deadline::new(Duration::from_secs(1)).unwrap();
    let table = Arc::new(RwLock::new(HashMap::new()));
    let released = Arc::new(AtomicUsize::new(0));
    let mut guard = table.write().await;
    guard.insert(
        "key".into(),
        GuardObserved {
            table: Arc::downgrade(&table),
            released: Arc::clone(&released),
        },
    );
    let removed = retain(&epoch, &clock, &mut guard, &bank, |_, _| false).unwrap();
    assert_eq!(released.load(Ordering::SeqCst), 0);
    assert!(guard.is_empty());
    assert!(bank.available_permits() < 4096);
    drop(guard);
    drop(removed);
    assert_eq!(released.load(Ordering::SeqCst), 1);
    assert_eq!(bank.available_permits(), 4096);
}
