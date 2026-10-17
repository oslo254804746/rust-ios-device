use super::super::{admission, metadata_vec};
use super::*;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Barrier;

#[repr(align(128))]
struct Payload {
    text: String,
    bank: Arc<Semaphore>,
    drops: Arc<AtomicUsize>,
    panic_on_drop: bool,
    _fields: OwnedSemaphorePermit,
}
impl Drop for Payload {
    fn drop(&mut self) {
        assert_eq!(self.bank.available_permits(), 0);
        assert_eq!(self.text, "original-input-field");
        self.drops.fetch_add(1, Ordering::SeqCst);
        assert!(!self.panic_on_drop, "fixture payload destructor unwind");
    }
}
fn payload(panic_on_drop: bool) -> (Strong<Payload>, Arc<Semaphore>, Arc<AtomicUsize>, usize) {
    let text = "original-input-field";
    let total = Strong::<Payload>::body_bytes() + text.len();
    let bank = Arc::new(Semaphore::new(total));
    let body = admission::known_metadata_charge(&bank, Strong::<Payload>::body_bytes()).unwrap();
    let allocation = metadata_vec::exact_backing::<u8>(&bank, text.len()).unwrap();
    let fields = allocation.storage;
    let mut bytes = allocation.values;
    bytes.extend_from_slice(text.as_bytes());
    let text = String::from_utf8(bytes).unwrap();
    let drops = Arc::new(AtomicUsize::new(0));
    let value = Strong::new(
        Payload {
            text,
            bank: bank.clone(),
            drops: drops.clone(),
            panic_on_drop,
            _fields: fields,
        },
        &bank,
        body,
    )
    .unwrap();
    (value, bank, drops, total)
}

#[test]
fn input_weak_body_last_strong_destroys_fields_but_last_weak_keeps_body_and_paid_guard() {
    let (value, bank, drops, total) = payload(false);
    let pointer = &*value as *const Payload;
    assert_eq!(pointer as usize % 128, 0);
    let text = value.text.as_ptr();
    let weak = value.downgrade();
    let retained_weak = weak.clone();
    let retained = weak.upgrade().unwrap();
    assert!(value.ptr_eq(&retained));
    assert_eq!(retained.text.as_ptr(), text);
    drop(value);
    assert_eq!(bank.available_permits(), 0);
    assert_eq!(drops.load(Ordering::SeqCst), 0);
    drop(retained);
    assert_eq!(drops.load(Ordering::SeqCst), 1);
    assert!(weak.upgrade().is_none());
    assert_eq!(
        bank.available_permits(),
        total - Strong::<Payload>::body_bytes()
    );
    drop(weak);
    assert_eq!(
        bank.available_permits(),
        total - Strong::<Payload>::body_bytes()
    );
    drop(retained_weak);
    assert_eq!(bank.available_permits(), total);
}

#[test]
fn input_weak_body_upgrades_racing_last_strong_drop_keep_original_field_and_refund_once() {
    let (value, bank, drops, total) = payload(false);
    let pointer = &*value as *const Payload as usize;
    let text = value.text.as_ptr() as usize;
    let weak = value.downgrade();
    let barrier = Arc::new(Barrier::new(9));
    std::thread::scope(|scope| {
        for _ in 0..8 {
            let weak = weak.clone();
            let barrier = barrier.clone();
            scope.spawn(move || {
                barrier.wait();
                for _ in 0..128 {
                    if let Some(value) = weak.upgrade() {
                        assert_eq!(&*value as *const Payload as usize, pointer);
                        assert_eq!(value.text.as_ptr() as usize, text);
                    }
                }
                drop(weak);
            });
        }
        barrier.wait();
        drop(value);
    });
    assert_eq!(drops.load(Ordering::SeqCst), 1);
    assert!(weak.upgrade().is_none());
    assert_eq!(
        bank.available_permits(),
        total - Strong::<Payload>::body_bytes()
    );
    drop(weak);
    assert_eq!(bank.available_permits(), total);
}

#[test]
fn input_weak_body_payload_unwind_keeps_dead_allocation_guard_until_last_weak() {
    let (value, bank, drops, total) = payload(true);
    let weak = value.downgrade();
    assert!(std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| drop(value))).is_err());
    assert_eq!(drops.load(Ordering::SeqCst), 1);
    assert!(weak.upgrade().is_none());
    assert_eq!(
        bank.available_permits(),
        total - Strong::<Payload>::body_bytes()
    );
    drop(weak);
    assert_eq!(bank.available_permits(), total);
}

#[test]
fn input_weak_body_foreign_short_long_grant_refusal_returns_credit_and_drops_value_once() {
    struct Value(Arc<AtomicUsize>);
    impl Drop for Value {
        fn drop(&mut self) {
            self.0.fetch_add(1, Ordering::SeqCst);
        }
    }
    let required = Strong::<Value>::body_bytes();
    for (foreign, amount) in [
        (true, required),
        (false, required - 1),
        (false, required + 1),
    ] {
        let bank = Arc::new(Semaphore::new(2 * required));
        let other = Arc::new(Semaphore::new(2 * required));
        let fee =
            admission::known_metadata_charge(if foreign { &other } else { &bank }, amount).unwrap();
        let drops = Arc::new(AtomicUsize::new(0));
        let error = Strong::new(Value(drops.clone()), &bank, fee).err().unwrap();
        assert_eq!(
            crate::output::operation_error_class(error.as_ref()),
            ios_core::ErrorClass::ResourceLimit
        );
        assert_eq!(drops.load(Ordering::SeqCst), 1);
        assert_eq!(bank.available_permits(), 2 * required);
        assert_eq!(other.available_permits(), 2 * required);
    }
}

#[test]
fn input_weak_body_zero_sized_payload_still_pays_guard_and_releases_value_at_last_strong() {
    static DROPS: AtomicUsize = AtomicUsize::new(0);
    struct Value;
    impl Drop for Value {
        fn drop(&mut self) {
            DROPS.fetch_add(1, Ordering::SeqCst);
        }
    }
    DROPS.store(0, Ordering::SeqCst);
    let total = Strong::<Value>::body_bytes();
    assert_eq!(total, std::mem::size_of::<OwnedSemaphorePermit>());
    let bank = Arc::new(Semaphore::new(total));
    let fee = admission::known_metadata_charge(&bank, total).unwrap();
    let value = Strong::new(Value, &bank, fee).unwrap();
    let weak = value.downgrade();
    drop(value);
    assert_eq!(DROPS.load(Ordering::SeqCst), 1);
    assert!(weak.upgrade().is_none());
    assert_eq!(bank.available_permits(), 0);
    drop(weak);
    assert_eq!(bank.available_permits(), total);
}
