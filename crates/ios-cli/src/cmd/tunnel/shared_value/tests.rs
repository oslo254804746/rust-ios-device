use super::super::{admission, metadata_vec, selector_copy::Copier};
use super::*;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Barrier;
use tokio::sync::{OwnedSemaphorePermit, Semaphore};

#[repr(align(128))]
struct Value {
    text: String,
    original: AtomicUsize,
    bank: Arc<Semaphore>,
    drops: Arc<AtomicUsize>,
    panic_on_drop: bool,
    _storage: OwnedSemaphorePermit,
}
impl Drop for Value {
    fn drop(&mut self) {
        // The sealed final Arc yielded T after freeing its own allocation;
        // this unpinned value is now in a different place, with fields intact.
        assert_ne!(
            self as *const Self as usize,
            self.original.load(Ordering::SeqCst)
        );
        assert_eq!(self.text, "original-shared-field");
        assert_eq!(self.bank.available_permits(), 0);
        self.drops.fetch_add(1, Ordering::SeqCst);
        assert!(!self.panic_on_drop, "fixture final value destructor unwind");
    }
}
fn value(panic_on_drop: bool) -> (Shared<Value>, Arc<Semaphore>, Arc<AtomicUsize>, usize) {
    let text = "original-shared-field";
    let total = std::mem::size_of::<Value>() + text.len();
    let bank = Arc::new(Semaphore::new(total));
    let drops = Arc::new(AtomicUsize::new(0));
    let mut storage = admission::known_metadata_charge(&bank, total).unwrap();
    let mut copier = Copier::new(storage, std::mem::size_of::<Value>()).unwrap();
    let text = copier.text(text).unwrap();
    storage = copier.finish();
    let value = Shared::new(Value {
        text,
        original: AtomicUsize::new(0),
        bank: bank.clone(),
        drops: drops.clone(),
        panic_on_drop,
        _storage: storage,
    });
    let pointer = &*value as *const Value as usize;
    assert_eq!(pointer % 128, 0);
    value.original.store(pointer, Ordering::SeqCst);
    (value, bank, drops, total)
}

#[test]
fn identity_arc_live_clones_share_original_value_and_field_until_final_moved_value_drop() {
    let (value, bank, drops, total) = value(false);
    let original = &*value as *const Value;
    let text = value.text.as_ptr();
    let retained = metadata_vec::with_allocation_hook(
        |_, _| panic!("shared clone allocated another field"),
        || value.clone(),
    );
    assert_eq!(&*retained as *const Value, original);
    assert_eq!(retained.text.as_ptr(), text);
    assert_eq!(
        std::mem::size_of::<Shared<Value>>(),
        std::mem::size_of::<Arc<Value>>()
    );
    drop(value);
    assert_eq!(bank.available_permits(), 0);
    assert_eq!(drops.load(Ordering::SeqCst), 0);
    assert_eq!(retained.text.as_ptr(), text);
    drop(retained);
    assert_eq!(drops.load(Ordering::SeqCst), 1);
    assert_eq!(bank.available_permits(), total);
}

#[test]
fn identity_arc_parallel_clone_drops_return_original_value_once_before_exact_refund() {
    let (value, bank, drops, total) = value(false);
    let pointer = &*value as *const Value as usize;
    let text = value.text.as_ptr() as usize;
    let mut owners = (0..7).map(|_| value.clone()).collect::<Vec<_>>();
    owners.push(value);
    let barrier = Arc::new(Barrier::new(owners.len()));
    std::thread::scope(|scope| {
        for owner in owners {
            let barrier = barrier.clone();
            scope.spawn(move || {
                assert_eq!(&*owner as *const Value as usize, pointer);
                assert_eq!(owner.text.as_ptr() as usize, text);
                barrier.wait();
                drop(owner);
            });
        }
    });
    assert_eq!(drops.load(Ordering::SeqCst), 1);
    assert_eq!(bank.available_permits(), total);
}

#[test]
fn identity_arc_final_destructor_unwind_still_drops_original_fields_and_grant_once() {
    let (value, bank, drops, total) = value(true);
    let temporary = value.clone();
    drop(temporary);
    assert_eq!(drops.load(Ordering::SeqCst), 0);
    assert_eq!(bank.available_permits(), 0);
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| drop(value)));
    assert!(result.is_err());
    assert_eq!(drops.load(Ordering::SeqCst), 1);
    assert_eq!(bank.available_permits(), total);
}

#[test]
fn identity_arc_zero_sized_value_is_destroyed_once_after_last_clone() {
    static DROPS: AtomicUsize = AtomicUsize::new(0);
    struct Value;
    impl Drop for Value {
        fn drop(&mut self) {
            DROPS.fetch_add(1, Ordering::SeqCst);
        }
    }
    DROPS.store(0, Ordering::SeqCst);
    let value = Shared::new(Value);
    let retained = value.clone();
    drop(value);
    assert_eq!(DROPS.load(Ordering::SeqCst), 0);
    drop(retained);
    assert_eq!(DROPS.load(Ordering::SeqCst), 1);
}
