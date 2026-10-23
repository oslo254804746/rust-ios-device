use super::*;
use crate::cmd::tunnel::{
    admission, lifecycle::Lifecycle, marker_identity::Identity, publication, retirement,
};

#[test]
fn birth_bank_original_total_survives_held_refunds_closure_clone_and_identity_weak() {
    let bank = BirthBank::new(17);
    let identity = Identity::new(&bank);
    let weak = identity.downgrade();
    let raw = bank.raw().clone();
    let held = raw.clone().try_acquire_many_owned(16).unwrap();
    assert_eq!(bank.total(), 17);
    assert_eq!(raw.available_permits(), 1);
    assert!(identity.origin_charge(2).is_err());
    let one = identity.origin_charge(1).unwrap();
    assert_eq!(raw.available_permits(), 0);
    drop(held);
    assert_eq!(raw.available_permits(), 16);
    assert_eq!(bank.total(), 17);
    let last = weak.upgrade().unwrap();
    assert!(last.ptr_eq(&identity) && Arc::ptr_eq(last.bank(), &raw));
    drop((identity, bank));
    let sixteen = last.origin_charge(16).unwrap();
    assert_eq!(raw.available_permits(), 0);
    raw.close();
    drop((sixteen, one));
    assert_eq!(raw.available_permits(), 17);
    assert!(last.origin_charge(1).is_err());
    drop(last);
    assert!(weak.upgrade().is_none());
    assert_eq!(raw.available_permits(), 17);
}

#[test]
fn birth_bank_total_refusal_preserves_original_limit_and_overflow_decisions() {
    let bank = BirthBank::new(17);
    for bytes in [18, usize::MAX] {
        let original = admission::known_metadata_charge(bank.raw(), bytes).unwrap_err();
        let guarded = bank.origin_charge(bytes).unwrap_err();
        assert_eq!(original.to_string(), guarded.to_string());
        assert_eq!(
            crate::output::operation_error_class(original.as_ref()),
            ios_core::ErrorClass::ResourceLimit
        );
        assert_eq!(
            crate::output::operation_error_class(guarded.as_ref()),
            ios_core::ErrorClass::ResourceLimit
        );
        assert_eq!(bank.raw().available_permits(), 17);
    }
    let grant = bank.origin_charge(17).unwrap();
    assert_eq!(grant.num_permits(), bank.total());
    assert!(Arc::ptr_eq(grant.semaphore(), bank.raw()));
    drop(grant);
    assert_eq!(bank.raw().available_permits(), 17);
}

#[tokio::test]
async fn birth_bank_real_lifecycle_registry_and_foreign_ticket_keep_original_producer_after_rebind()
{
    let total = crate::cmd::tunnel::MAX_GLOBAL_KNOWN_METADATA_BYTES;
    let origin = BirthBank::new(total);
    let old_raw = origin.raw().clone();
    let lifecycle = Lifecycle::with_metadata(origin.clone()).unwrap();
    let ticket = lifecycle.ticket("authentic-birth-bank").await.unwrap();
    let mut registry = retirement::Registry::with_metadata(&origin).unwrap();
    assert!(Arc::ptr_eq(&registry.byte_bank(), &old_raw));
    let destination = BirthBank::new(total);
    registry.rebind_empty_metadata(&destination).unwrap();
    assert!(Arc::ptr_eq(&registry.byte_bank(), destination.raw()));
    assert!(!Arc::ptr_eq(&registry.byte_bank(), &old_raw));
    let global = Arc::new(Semaphore::new(publication::TicketOwner::body_bytes()));
    let published = Arc::new(Semaphore::new(publication::TicketOwner::body_bytes()));
    let before = old_raw.available_permits();
    let root = publication::TicketOwner::new(&ticket, &global, &published).unwrap();
    assert_eq!(
        old_raw.available_permits(),
        before - publication::TicketOwner::body_bytes()
    );
    assert_eq!(origin.total(), total);
    let held = old_raw
        .clone()
        .try_acquire_many_owned(old_raw.available_permits() as u32)
        .unwrap();
    old_raw.close();
    lifecycle.clear();
    assert!(!root.is_current());
    drop((ticket, lifecycle, origin, registry));
    assert_eq!(destination.raw().available_permits(), total);
    drop(held);
    assert!(old_raw.available_permits() < total);
    drop(root);
    assert_eq!(old_raw.available_permits(), total);
    assert_eq!(
        global.available_permits(),
        publication::TicketOwner::body_bytes()
    );
    assert_eq!(
        published.available_permits(),
        publication::TicketOwner::body_bytes()
    );
}
