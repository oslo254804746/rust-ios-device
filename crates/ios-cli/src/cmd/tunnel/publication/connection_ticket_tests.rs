use super::*;
use crate::cmd::tunnel::{lifecycle::Lifecycle, marker_identity::Identity, metadata_vec};
use std::sync::atomic::AtomicUsize;

const TOTAL: usize = 128 * 1024;
const KEY: &str = "connection-ticket-root";

#[tokio::test]
async fn connection_ticket_each_retained_bank_refuses_without_cloning_or_backing() {
    let bytes = TicketOwner::body_bytes();
    for target in 0..3 {
        for closed in [false, true] {
            let global = Arc::new(Semaphore::new(TOTAL));
            let published = Arc::new(Semaphore::new(TOTAL));
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
            let owner = TicketOwner::new(&ticket, &global, &published).unwrap();
            let desired_held = desired_bank
                .clone()
                .try_acquire_many_owned(desired_bank.available_permits() as u32)
                .unwrap();
            desired_bank.close();
            let banks = [&global, &published, &origin];
            let denied = banks[target];
            let leave = bytes - usize::from(!closed);
            let held = denied
                .clone()
                .try_acquire_many_owned((denied.available_permits() - leave) as u32)
                .unwrap();
            if closed {
                denied.close();
            }
            let before = banks.map(|bank| bank.available_permits());
            let count = desired.strong_count();
            let hook = metadata_vec::on_allocation(|_, _| panic!("unadmitted connection root"));
            let error = owner.connection(&global, &published).err().unwrap();
            drop(hook);
            assert!(error.is::<Admission>());
            assert_eq!(
                crate::output::operation_error_class(error.as_ref()),
                ios_core::ErrorClass::ResourceLimit
            );
            assert_eq!(banks.map(|bank| bank.available_permits()), before);
            assert_eq!(desired.strong_count(), count);
            assert_eq!(desired_bank.available_permits(), 0);
            drop((held, desired_held));
        }
    }
}

#[tokio::test]
async fn connection_ticket_actual_projection_omits_closed_desired_and_keeps_paid_epochs_until_free()
{
    let bytes = TicketOwner::body_bytes();
    let global = Arc::new(Semaphore::new(2 * bytes));
    let published = Arc::new(Semaphore::new(2 * bytes));
    let origin_birth = crate::cmd::tunnel::birth_bank::BirthBank::new(TOTAL);
    let origin = origin_birth.raw().clone();
    let desired_bank_birth = crate::cmd::tunnel::birth_bank::BirthBank::new(TOTAL);
    let desired_bank = desired_bank_birth.raw().clone();
    let lifecycle = Lifecycle::with_metadata(origin_birth.clone()).unwrap();
    let desired = Identity::new(&desired_bank_birth);
    let weak_desired = desired.downgrade();
    let ticket = lifecycle
        .ticket(KEY)
        .await
        .unwrap()
        .observe_version(&desired);
    let owner = TicketOwner::new(&ticket, &global, &published).unwrap();
    assert!(
        !owner.is_current(),
        "foreign desired is deliberately not current"
    );
    let held_c = desired_bank
        .clone()
        .try_acquire_many_owned(desired_bank.available_permits() as u32)
        .unwrap();
    desired_bank.close();
    let count = desired.strong_count();
    let key = ticket.udid().as_ptr();
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
    let root = owner.connection(&global, &published).unwrap();
    drop(hook);
    let value = &*root as *const super::super::Ticket as usize;
    let pointer = address.load(Ordering::SeqCst);
    assert!(
        pointer <= value && value + std::mem::size_of::<super::super::Ticket>() <= pointer + bytes
    );
    assert!(!root.ptr_eq(&owner));
    assert_eq!(root.udid().as_ptr(), key);
    assert!(root.same_incarnation(&ticket) && root.is_current());
    assert_eq!(desired.strong_count(), count);
    assert_eq!(root.0.extra().1.iter().flatten().count(), 1);
    assert!(root.0.extra().1[2].is_none());
    let kept = root.clone();
    assert!(kept.ptr_eq(&root));
    let held_a = origin
        .clone()
        .try_acquire_many_owned(origin.available_permits() as u32)
        .unwrap();
    let held_bytes = held_a.num_permits();
    global.close();
    published.close();
    origin.close();
    lifecycle.clear();
    assert!(!root.is_current());
    drop((root, owner, ticket, lifecycle, desired, held_c));
    assert!(
        weak_desired.upgrade().is_none(),
        "connection must not retain removed desired"
    );
    let g = global.clone();
    let p = published.clone();
    let a = origin.clone();
    super::super::shared_weak_allocation::after_free(
        move || {
            assert_eq!(
                (g.available_permits(), p.available_permits()),
                (bytes, bytes)
            );
            assert!(a.available_permits() <= TOTAL - bytes - held_bytes);
        },
        || drop(kept),
    );
    assert_eq!(
        (global.available_permits(), published.available_permits()),
        (2 * bytes, 2 * bytes)
    );
    assert_eq!(origin.available_permits(), TOTAL - held_bytes);
    drop(held_a);
    assert_eq!(origin.available_permits(), TOTAL);
    assert_eq!(desired_bank.available_permits(), TOTAL);
}
