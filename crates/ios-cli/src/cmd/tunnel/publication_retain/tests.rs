use super::super::paid_watch as watch;
use super::*;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;
fn epoch(
    bank: &Arc<Semaphore>,
) -> (
    Epoch,
    watch::Sender<Option<crate::cmd::tunnel::ControlIdentity>>,
) {
    let generation = crate::cmd::tunnel::ControlIdentity::new(bank);
    let (sender, changes) = watch::channel(Some(generation.clone()), bank).unwrap();
    (
        Epoch {
            changes,
            generation,
        },
        sender,
    )
}
fn entries(bank: &Arc<Semaphore>) -> Table<Box<u8>> {
    let mut table = Table::new(bank, bank);
    for (key, value) in [("a", 1), ("b", 2)] {
        let prepared = table.prepare(key).unwrap();
        let slot = table.destination(&prepared).unwrap();
        drop(table.publish(slot, prepared, Box::new(value)).finish());
    }
    table
}
#[test]
fn publication_retain_original_paid_nodes_remove_with_depleted_quota_and_last_owner_release() {
    let bank = Arc::new(Semaphore::new(2 * 1024 * 1024));
    let (epoch, _sender) = epoch(&bank);
    let clock = Deadline::new(Duration::from_secs(1)).unwrap();
    let mut table = entries(&bank);
    let (key, value) = table.get_key_value("a").unwrap();
    let pointer = key.as_ptr();
    let payload = &**value as *const u8;
    let held = bank
        .clone()
        .try_acquire_many_owned(bank.available_permits() as u32)
        .unwrap();
    let removed = retain(&epoch, &clock, &mut table, &bank, |key, _| key == "a").unwrap();
    assert_eq!(removed.len(), 1);
    assert_eq!(bank.available_permits(), 0);
    assert_eq!(table.get_key_value("a").unwrap().0.as_ptr(), pointer);
    assert_eq!(&**table.get("a").unwrap() as *const u8, payload);
    drop(removed);
    assert!(bank.available_permits() > 0);
    drop(held);
}
#[test]
fn publication_retain_actual_late_clock_zero_quota_restores_original_nodes_keys_and_slots() {
    let bank = Arc::new(Semaphore::new(2 * 1024 * 1024));
    let mut table = entries(&bank);
    let before = [table.proof("a"), table.proof("b")];
    let before_keys = [
        table.get_key_value("a").unwrap().0.as_ptr(),
        table.get_key_value("b").unwrap().0.as_ptr(),
    ];
    let (epoch, _sender) = epoch(&bank);
    let clock = Deadline::new(Duration::from_millis(20)).unwrap();
    let held = bank
        .clone()
        .try_acquire_many_owned(bank.available_permits() as u32)
        .unwrap();
    on_after_insert(|| std::thread::sleep(Duration::from_millis(60)));
    let error = retain(&epoch, &clock, &mut table, &bank, |key, _| key == "a")
        .err()
        .unwrap();
    assert!(error.is::<super::super::scan::DeadlineReached>());
    assert_eq!([table.proof("a"), table.proof("b")], before);
    assert_eq!(
        [
            table.get_key_value("a").unwrap().0.as_ptr(),
            table.get_key_value("b").unwrap().0.as_ptr()
        ],
        before_keys
    );
    assert_eq!(table.len(), 2);
    assert_eq!(bank.available_permits(), 0);
    drop(held);
}
#[test]
fn publication_retain_stale_epoch_and_predicate_unwind_restore_each_original_slot() {
    for unwind in [false, true] {
        let bank = Arc::new(Semaphore::new(2 * 1024 * 1024));
        let mut table = entries(&bank);
        let before = [table.proof("a"), table.proof("b")];
        let (epoch, sender) = epoch(&bank);
        let clock = Deadline::new(Duration::from_secs(1)).unwrap();
        let calls = AtomicUsize::new(0);
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            retain(&epoch, &clock, &mut table, &bank, |_, _| {
                if calls.fetch_add(1, Ordering::SeqCst) == 1 {
                    if unwind {
                        panic!("synthetic predicate unwind");
                    }
                    sender.send_replace(Some(crate::cmd::tunnel::ControlIdentity::new(&bank)));
                }
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
        assert_eq!([table.proof("a"), table.proof("b")], before);
        assert_eq!(table.len(), 2);
    }
}
