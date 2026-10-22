use super::*;
use std::sync::atomic::{AtomicUsize, Ordering};
use tokio::sync::Semaphore;

#[repr(align(128))]
struct Original {
    bank: Arc<Semaphore>,
    drops: Arc<AtomicUsize>,
    address: usize,
    panic: bool,
}
impl TunnelReservation for Original {
    fn byte_bank(&self) -> Option<Arc<Semaphore>> {
        Some(self.bank.clone())
    }
    fn attach(&mut self, _: Arc<TunnelHandle>) -> Result<(), CoreError> {
        panic!("no native work in reservation storage tests")
    }
    fn complete(&mut self) {
        assert_eq!(self as *const Self as usize, self.address);
        assert_eq!(self.bank.available_permits(), 0);
    }
}
impl Drop for Original {
    fn drop(&mut self) {
        assert_eq!(self as *const Self as usize, self.address);
        assert_eq!(self.bank.available_permits(), 0);
        self.drops.fetch_add(1, Ordering::SeqCst);
        if self.panic {
            std::panic::panic_any("original-reservation-drop");
        }
    }
}
fn original(bank: &Arc<Semaphore>, drops: &Arc<AtomicUsize>, panic: bool) -> Box<Original> {
    let mut original = Box::new(Original {
        bank: bank.clone(),
        drops: drops.clone(),
        address: 0,
        panic,
    });
    original.address = &*original as *const Original as usize;
    assert_eq!(original.address % 128, 0);
    original
}
struct Reset;
impl Drop for Reset {
    fn drop(&mut self) {
        AFTER_RESERVATION_FREE.with(|hook| hook.borrow_mut().take());
    }
}

#[test]
fn reservation_owner_same_original_aligned_box_frees_before_fee_even_when_drop_panics() {
    for panic in [false, true] {
        let bytes = std::mem::size_of::<Original>();
        let bank = Arc::new(Semaphore::new(bytes));
        let storage = bank.clone().try_acquire_many_owned(bytes as u32).unwrap();
        let drops = Arc::new(AtomicUsize::new(0));
        let boxed = original(&bank, &drops, panic);
        let pointer = boxed.address;
        let mut owner = TunnelReservationOwner::try_from_grant(boxed, storage)
            .unwrap_or_else(|_| panic!("exact original grant rejected"));
        assert_eq!(&*owner as *const _ as *const () as usize, pointer);
        bank.close();
        owner.complete();
        let freed = Arc::new(AtomicUsize::new(0));
        let observed = freed.clone();
        let charged = bank.clone();
        let ended = drops.clone();
        let _reset = Reset;
        AFTER_RESERVATION_FREE.with(|hook| {
            *hook.borrow_mut() = Some(Box::new(move || {
                assert_eq!(ended.load(Ordering::SeqCst), 1);
                assert_eq!(charged.available_permits(), 0);
                observed.fetch_add(1, Ordering::SeqCst);
            }));
        });
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| drop(owner)));
        if panic {
            assert_eq!(
                result.unwrap_err().downcast_ref::<&'static str>(),
                Some(&"original-reservation-drop")
            );
        } else {
            result.unwrap();
        }
        assert_eq!(freed.load(Ordering::SeqCst), 1);
        assert_eq!(drops.load(Ordering::SeqCst), 1);
        assert_eq!(bank.available_permits(), bytes);
    }
}

#[test]
fn reservation_owner_refuses_short_and_foreign_grants_without_replacing_original_box() {
    let bytes = std::mem::size_of::<Original>();
    for foreign in [false, true] {
        let bank = Arc::new(Semaphore::new(bytes));
        let other = Arc::new(Semaphore::new(bytes));
        let drops = Arc::new(AtomicUsize::new(0));
        let bank_hold = bank.clone().try_acquire_many_owned(bytes as u32).unwrap();
        let (storage, held) = if foreign {
            (
                other.clone().try_acquire_many_owned(bytes as u32).unwrap(),
                bank_hold,
            )
        } else {
            let mut storage = bank_hold;
            let held = storage.split(1).unwrap();
            (storage, held)
        };
        let boxed = original(&bank, &drops, false);
        let pointer = boxed.address;
        let (boxed, storage) = TunnelReservationOwner::try_from_grant(boxed, storage)
            .err()
            .expect("invalid grant accepted");
        assert_eq!(&*boxed as *const _ as *const () as usize, pointer);
        assert_eq!(drops.load(Ordering::SeqCst), 0);
        assert_eq!(bank.available_permits(), 0);
        drop(boxed);
        assert_eq!(drops.load(Ordering::SeqCst), 1);
        assert_eq!(bank.available_permits(), 0);
        drop(storage);
        drop(held);
        assert_eq!(bank.available_permits(), bytes);
        assert_eq!(other.available_permits(), bytes);
    }
}
