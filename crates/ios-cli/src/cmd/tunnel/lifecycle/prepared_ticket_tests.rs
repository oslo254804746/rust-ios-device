use super::super::{metadata_vec, shared_weak_allocation};
use super::*;
use std::sync::atomic::{AtomicUsize, Ordering};

const TOTAL: usize = 128 * 1024;

#[test]
fn prepared_ticket_actual_destination_and_authentic_origin_refusals_allocate_nothing_and_refund() {
    let origin = super::super::birth_bank::BirthBank::new(TOTAL);
    let lifecycle = Lifecycle::with_metadata(origin.clone()).unwrap();
    let before = origin.available_permits();
    let bytes = TicketOwner::body_bytes();
    for closed in [false, true] {
        let total = if closed { bytes } else { bytes - 1 };
        let bank = Arc::new(Semaphore::new(total));
        if closed {
            bank.close();
        }
        let hook =
            metadata_vec::on_allocation(|_, _| panic!("refused empty Ticket root allocated"));
        let error = lifecycle.prepare_manual_ticket(&bank).err().unwrap();
        drop(hook);
        assert_eq!(
            crate::output::operation_error_class(error.as_ref()),
            ios_core::ErrorClass::ResourceLimit
        );
        assert_eq!(bank.available_permits(), total);
        assert_eq!(origin.available_permits(), before);
    }
    let bank = Arc::new(Semaphore::new(bytes));
    let held = origin
        .raw()
        .clone()
        .try_acquire_many_owned((before - bytes + 1) as u32)
        .unwrap();
    let hook =
        metadata_vec::on_allocation(|_, _| panic!("origin-refused empty Ticket root allocated"));
    assert!(lifecycle.prepare_manual_ticket(&bank).is_err());
    drop(hook);
    assert_eq!(origin.available_permits(), bytes - 1);
    assert_eq!(bank.available_permits(), bytes);
    drop(held);
    let guard = metadata_vec::refuse_allocation(move |layout| {
        assert_eq!(layout.size(), bytes);
        true
    });
    assert!(lifecycle.prepare_manual_ticket(&bank).is_err());
    drop(guard);
    assert_eq!(origin.available_permits(), before);
    assert_eq!(bank.available_permits(), bytes);
    drop(lifecycle.prepare_manual_ticket(&bank).unwrap());
    assert_eq!(origin.available_permits(), before);
    assert_eq!(bank.available_permits(), bytes);
    let held = origin
        .raw()
        .clone()
        .try_acquire_many_owned((before - bytes) as u32)
        .unwrap();
    let same_bank = lifecycle.prepare_manual_ticket(origin.raw()).unwrap();
    assert_eq!(
        origin.available_permits(),
        0,
        "producer and destination must deduplicate"
    );
    drop((same_bank, held));
    assert_eq!(origin.available_permits(), before);
    origin.close();
    assert!(lifecycle.prepare_manual_ticket(&bank).is_err());
    assert_eq!(bank.available_permits(), bytes);
    drop(lifecycle);
    assert_eq!(origin.available_permits(), TOTAL);
}

#[tokio::test]
async fn prepared_ticket_places_current_after_clear_at_closed_zero_and_projection_preserves_physical_fee_only(
) {
    let origin = super::super::birth_bank::BirthBank::new(TOTAL);
    let lifecycle = Lifecycle::with_metadata(origin.clone()).unwrap();
    let bytes = TicketOwner::body_bytes();
    let bank = Arc::new(Semaphore::new(bytes));
    let address = Arc::new(AtomicUsize::new(0));
    let seen = address.clone();
    let producer = origin.clone();
    let destination = bank.clone();
    let before = origin.available_permits();
    let prepared = metadata_vec::with_allocation_hook(
        move |layout, pointer| {
            assert_eq!(layout.size(), bytes);
            assert_eq!(destination.available_permits(), 0);
            assert_eq!(producer.available_permits(), before - bytes);
            assert_eq!(seen.swap(pointer, Ordering::SeqCst), 0);
        },
        || lifecycle.prepare_manual_ticket(&bank).unwrap(),
    );
    let old = lifecycle.ticket("CURRENT-root").await.unwrap().fenced();
    lifecycle.clear();
    let ticket = lifecycle.ticket("CURRENT-root").await.unwrap().fenced();
    assert!(!ControlIdentity::ptr_eq(
        &old.shutdown_generation,
        &ticket.shutdown_generation
    ));
    assert!(!ControlIdentity::ptr_eq(
        old.intent_version.as_ref().unwrap(),
        ticket.intent_version.as_ref().unwrap()
    ));
    drop(old);
    let key = ticket.udid().as_ptr();
    let global = ControlIdentity::downgrade(&ticket.shutdown_generation);
    let device = ControlIdentity::downgrade(&ticket.generation);
    let desired = ControlIdentity::downgrade(ticket.intent_version.as_ref().unwrap());
    let held = origin
        .raw()
        .clone()
        .try_acquire_many_owned(origin.available_permits() as u32)
        .unwrap();
    bank.close();
    origin.close();
    let hook =
        metadata_vec::on_allocation(|_, _| panic!("CURRENT Ticket placement allocated late"));
    let root = prepared.place(ticket);
    let captured = root.captured_version();
    let last = captured.clone();
    let other = root.clone();
    drop(hook);
    let value = &*root as *const Ticket as usize;
    let start = address.load(Ordering::SeqCst);
    assert!(start <= value && value + std::mem::size_of::<Ticket>() <= start + bytes);
    assert_eq!(root.udid().as_ptr(), key);
    assert!(root.ptr_eq(&other));
    assert!(ControlIdentity::ptr_eq(
        &captured,
        root.intent_version.as_ref().unwrap()
    ));
    drop((root, other, lifecycle, held));
    assert!(
        last.keeper.upgrade().is_none(),
        "projection retained Ticket payload"
    );
    assert!(
        global.upgrade().is_none(),
        "projection retained global/watch payload"
    );
    assert!(
        device.upgrade().is_none(),
        "projection retained device/gate payload"
    );
    assert!(
        desired.upgrade().is_some(),
        "original desired value ended early"
    );
    assert_eq!(origin.available_permits(), TOTAL - bytes);
    assert_eq!(bank.available_permits(), 0);
    drop(captured);
    let a = origin.clone();
    let b = bank.clone();
    shared_weak_allocation::after_free(
        move || {
            assert!(
                desired.upgrade().is_none(),
                "derived value must end before physical free"
            );
            assert_eq!(a.available_permits(), TOTAL - bytes);
            assert_eq!(b.available_permits(), 0);
        },
        || drop(last),
    );
    assert_eq!(origin.available_permits(), TOTAL);
    assert_eq!(bank.available_permits(), bytes);
}
