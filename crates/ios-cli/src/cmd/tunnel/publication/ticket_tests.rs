use super::*;
use crate::cmd::tunnel::{
    lifecycle::Lifecycle, marker_identity::Identity, metadata_vec, publication_table::Table,
    shared_weak_allocation,
};
use std::sync::atomic::AtomicUsize;

const KEY: &str = "publication-ticket-root";
const TOTAL: usize = 128 * 1024;

#[tokio::test]
async fn publication_ticket_each_real_bank_refuses_before_backing_and_rolls_back_other_grants() {
    let bytes = TicketOwner::body_bytes();
    for target in 0..4 {
        for closed in [false, true] {
            let global = Arc::new(Semaphore::new(TOTAL));
            let published_birth = crate::cmd::tunnel::birth_bank::BirthBank::new(TOTAL);
            let published = published_birth.raw().clone();
            let origin_birth = crate::cmd::tunnel::birth_bank::BirthBank::new(TOTAL);
            let origin = origin_birth.raw().clone();
            let desired_bank_birth = crate::cmd::tunnel::birth_bank::BirthBank::new(TOTAL);
            let desired_bank = desired_bank_birth.raw().clone();
            let lifecycle = Lifecycle::with_metadata(origin_birth.clone()).unwrap();
            let desired = Identity::new(&desired_bank_birth);
            let ticket = lifecycle
                .ticket(KEY)
                .await
                .unwrap()
                .observe_version(&desired);
            let banks = [&global, &published, &origin, &desired_bank];
            let leave = bytes - usize::from(!closed);
            let denied = banks[target];
            let held = denied
                .clone()
                .try_acquire_many_owned((denied.available_permits() - leave) as u32)
                .unwrap();
            if closed {
                denied.close();
            }
            let before = banks.map(|bank| bank.available_permits());
            let hook = metadata_vec::on_allocation(|_, _| {
                panic!("unadmitted publication Ticket allocated")
            });
            let error = TicketOwner::new(&ticket, &global, &published)
                .err()
                .unwrap();
            drop(hook);
            assert!(error.is::<Admission>());
            assert_eq!(
                crate::output::operation_error_class(error.as_ref()),
                ios_core::ErrorClass::ResourceLimit
            );
            assert_eq!(banks.map(|bank| bank.available_permits()), before);
            drop(held);
        }
    }
    let bank_birth = crate::cmd::tunnel::birth_bank::BirthBank::new(TOTAL);
    let bank = bank_birth.raw().clone();
    let global = Arc::new(Semaphore::new(bytes));
    let lifecycle = Lifecycle::with_metadata(bank_birth.clone()).unwrap();
    let ticket = lifecycle.ticket(KEY).await.unwrap();
    let wrong = bank
        .clone()
        .try_acquire_many_owned((bytes - 1) as u32)
        .unwrap();
    let before = bank.available_permits();
    let error = ticket
        .origin_grants_with_covered(&global, Some(&wrong), bytes)
        .err()
        .unwrap();
    assert_eq!(
        crate::output::operation_error_class(error.as_ref()),
        ios_core::ErrorClass::ResourceLimit
    );
    assert_eq!(bank.available_permits(), before);
    assert_eq!(global.available_permits(), bytes);
}

#[tokio::test]
async fn publication_ticket_actual_dual_root_deduplicates_published_origin_and_pays_until_final_free(
) {
    let bytes = TicketOwner::body_bytes();
    let global = Arc::new(Semaphore::new(bytes));
    let published_birth = crate::cmd::tunnel::birth_bank::BirthBank::new(TOTAL);
    let published = published_birth.raw().clone();
    let desired_bank_birth = crate::cmd::tunnel::birth_bank::BirthBank::new(bytes);
    let desired_bank = desired_bank_birth.raw().clone();
    // Actual global/device producers are P. Both are already covered by P's
    // whole root grant and must not need another origin grant in that bank.
    let lifecycle = Lifecycle::with_metadata(published_birth.clone()).unwrap();
    let desired = Identity::new(&desired_bank_birth);
    let ticket = lifecycle
        .ticket(KEY)
        .await
        .unwrap()
        .observe_version(&desired);
    let canonical = ticket.udid().as_ptr();
    let held = published
        .clone()
        .try_acquire_many_owned((published.available_permits() - bytes) as u32)
        .unwrap();
    let address = Arc::new(AtomicUsize::new(0));
    let observed = address.clone();
    let g = global.clone();
    let p = published.clone();
    let c = desired_bank.clone();
    let hook = metadata_vec::on_allocation(move |layout, pointer| {
        assert_eq!(layout.size(), bytes);
        assert_eq!(
            (
                g.available_permits(),
                p.available_permits(),
                c.available_permits()
            ),
            (0, 0, 0)
        );
        assert_eq!(observed.swap(pointer, Ordering::SeqCst), 0);
    });
    let owner = TicketOwner::new(&ticket, &global, &published).unwrap();
    drop(hook);
    let value = &*owner as *const super::super::Ticket as usize;
    let pointer = address.load(Ordering::SeqCst);
    assert!(
        pointer <= value && value + std::mem::size_of::<super::super::Ticket>() <= pointer + bytes
    );
    assert!(owner.same_incarnation(&ticket));
    assert_eq!(owner.udid().as_ptr(), canonical);
    assert_eq!(owner.0.extra().1.iter().flatten().count(), 1);
    assert!(Arc::ptr_eq(owner.0.extra().0.semaphore(), &published));
    assert!(Arc::ptr_eq(owner.0.first().semaphore(), &global));
    let kept = owner.clone();
    assert!(kept.ptr_eq(&owner));
    global.close();
    published.close();
    desired_bank.close();
    drop((owner, ticket, lifecycle, held));
    let g = global.clone();
    let p = published.clone();
    let c = desired_bank.clone();
    let final_p = Arc::new(AtomicUsize::new(0));
    let observed = final_p.clone();
    shared_weak_allocation::after_free(
        move || {
            assert_eq!(g.available_permits(), 0);
            assert_eq!(c.available_permits(), 0);
            assert!(p.available_permits() <= TOTAL - bytes);
            observed.store(p.available_permits(), Ordering::SeqCst);
        },
        || drop(kept),
    );
    assert_eq!(final_p.load(Ordering::SeqCst), TOTAL - bytes);
    assert_eq!(
        (
            global.available_permits(),
            published.available_permits(),
            desired_bank.available_permits()
        ),
        (bytes, TOTAL, bytes)
    );
}

#[tokio::test]
async fn publication_ticket_replacement_rollback_and_retired_values_keep_original_real_roots_at_zero(
) {
    // The original fixed 4096-slot table backing alone exceeds the small
    // root-only fixture bank. Fund that actual table without changing its cap.
    let total = 2 * 1024 * 1024;
    let global = Arc::new(Semaphore::new(total));
    let published_birth = crate::cmd::tunnel::birth_bank::BirthBank::new(total);
    let published = published_birth.raw().clone();
    let origin_birth = crate::cmd::tunnel::birth_bank::BirthBank::new(total);
    let origin = origin_birth.raw().clone();
    let lifecycle = Lifecycle::with_metadata(origin_birth.clone()).unwrap();
    let old = TicketOwner::new(&lifecycle.ticket(KEY).await.unwrap(), &global, &published).unwrap();
    let pointer = &*old as *const super::super::Ticket as usize;
    let key_pointer = old.udid().as_ptr();
    let mut table = Table::new(&global, &published);
    let destination = table.prepare(KEY).unwrap();
    let slot = table.destination(&destination).unwrap();
    drop(table.publish(slot, destination, old).finish());
    lifecycle.clear();
    let late =
        TicketOwner::new(&lifecycle.ticket(KEY).await.unwrap(), &global, &published).unwrap();
    let late_marker = late.global_epoch().downgrade();
    let destination = table.prepare(KEY).unwrap();
    let slot = table.destination(&destination).unwrap();
    let change = table.publish(slot, destination, late);
    lifecycle.clear();
    let held_g = global
        .clone()
        .try_acquire_many_owned(global.available_permits() as u32)
        .unwrap();
    let held_p = published
        .clone()
        .try_acquire_many_owned(published.available_permits() as u32)
        .unwrap();
    let held_a = origin
        .clone()
        .try_acquire_many_owned(origin.available_permits() as u32)
        .unwrap();
    global.close();
    published.close();
    origin.close();
    let hook = metadata_vec::on_allocation(|_, _| panic!("publication rollback allocated"));
    let retired = table.rollback(change);
    drop(hook);
    let restored = table.get(KEY).unwrap();
    assert_eq!(&**restored as *const super::super::Ticket as usize, pointer);
    assert_eq!(restored.udid().as_ptr(), key_pointer);
    assert_eq!(
        (
            global.available_permits(),
            published.available_permits(),
            origin.available_permits()
        ),
        (0, 0, 0)
    );
    drop(table.take_all());
    drop((table, lifecycle, held_g, held_p, held_a));
    assert!(late_marker.upgrade().is_some());
    assert!(
        global.available_permits() < total
            && published.available_permits() < total
            && origin.available_permits() < total
    );
    drop(retired);
    assert!(late_marker.upgrade().is_none());
    assert_eq!(
        (
            global.available_permits(),
            published.available_permits(),
            origin.available_permits()
        ),
        (total, total, total)
    );
}
