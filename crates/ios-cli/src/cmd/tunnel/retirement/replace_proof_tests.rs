//! Registry identity transactions only. Sender-only handles own no native IO;
//! these tests do not claim worker joins, prepared verification or device use.
use super::*;
use super::super::{admission, birth_bank::BirthBank, metadata_lease};
use std::sync::{atomic::{AtomicBool, Ordering}, Arc};
use tokio::sync::watch;

const TOTAL: usize = 1_048_576;
struct Fixture {
    registry: RegistryOwner,
    global: Arc<Semaphore>,
    published: Arc<Semaphore>,
    stamp: OwnershipStamp,
    old: Option<NativeLease>,
    handle: Arc<TunnelHandle>,
    receiver: Option<watch::Receiver<()>>,
    metadata: metadata_lease::Owner,
    global_metadata: metadata_lease::Owner,
}
fn handle() -> (Arc<TunnelHandle>, watch::Receiver<()>) {
    let (handle, receiver) = TunnelHandle::new("scripted-old-proof".into(),
        ios_core::TunnelInfo {
            server_address: "fd00::1".into(), client_address: "fd00::2".into(),
            server_rsd_port: 58783, client_mtu: 1280,
        }, None);
    (Arc::new(handle), receiver)
}
fn fees(global: &Arc<Semaphore>, published: &Arc<Semaphore>) -> (metadata_lease::Owner, metadata_lease::Owner) {
    metadata_lease::PreparedPair::new(global, published).unwrap().finish(
        admission::known_metadata_charge(global, 7).unwrap(),
        admission::known_metadata_charge(published, 11).unwrap(),
    ).unwrap()
}
fn attach(registry: &RegistryOwner, stamp: &OwnershipStamp,
    metadata: &metadata_lease::Owner, global_metadata: &metadata_lease::Owner)
    -> (NativeLease, Arc<TunnelHandle>, watch::Receiver<()>)
{
    let (handle, receiver) = handle();
    let lease = registry.reserve(stamp.clone()).unwrap().attach(handle.clone());
    lease.retain_metadata(metadata.clone()).unwrap();
    lease.retain_global_metadata(global_metadata.clone()).unwrap();
    (lease, handle, receiver)
}
impl Fixture {
    fn new() -> Self {
        let registry = Registry::allocate(&BirthBank::new(TOTAL), 4).unwrap();
        let global = registry.byte_bank();
        let published = Arc::new(Semaphore::new(TOTAL));
        let mut births = registry.prepare_births::<2>(&global).unwrap();
        let stamp = OwnershipStamp::standalone(&registry, &mut births);
        let (metadata, global_metadata) = fees(&global, &published);
        let (old, handle, receiver) = attach(&registry, &stamp, &metadata, &global_metadata);
        old.publish(&stamp).unwrap();
        Self { registry, global, published, stamp, old: Some(old), handle,
            receiver: Some(receiver), metadata, global_metadata }
    }
    fn proof(&self) -> RefreshProof {
        self.old.as_ref().unwrap().capture_existing(self.stamp.clone(), &self.handle,
            &self.metadata, &self.global_metadata, None, &self.global).unwrap()
    }
    fn fresh(&self) -> (NativeLease, Arc<TunnelHandle>, watch::Receiver<()>) {
        let (metadata, global_metadata) = fees(&self.global, &self.published);
        attach(&self.registry, &self.stamp, &metadata, &global_metadata)
    }
    fn phase(&self, key: &Key) -> Phase {
        self.registry.lock().slots[key.slot].as_ref().unwrap().phase
    }
}

#[test]
fn natural_dead_capture_is_replacement_only_and_row_matches_remain_physical() {
    let mut fixture = Fixture::new();
    let before = fixture.registry.snapshot().occupied();
    drop(fixture.receiver.take());
    assert!(!fixture.handle.is_alive());
    assert!(fixture.old.as_ref().unwrap().capture_refresh(fixture.stamp.clone(),
        &fixture.handle, &fixture.metadata, &fixture.global_metadata, None, &fixture.global).is_err());
    let proof = fixture.proof();
    assert!(proof.check().is_err());
    proof.check_replacement().unwrap();
    assert!(proof.matches_row(&fixture.handle, &fixture.metadata, &fixture.global_metadata, None));
    let (other, _rx) = handle();
    assert!(!proof.matches_row(&other, &fixture.metadata, &fixture.global_metadata, None));
    let (wrong_p, wrong_g) = fees(&fixture.global, &fixture.published);
    assert!(!proof.matches_row(&fixture.handle, &wrong_p, &fixture.global_metadata, None));
    assert!(!proof.matches_row(&fixture.handle, &fixture.metadata, &wrong_g, None));
    assert_eq!(fixture.registry.snapshot().occupied(), before);
    assert_eq!(fixture.registry.snapshot().retired, 0);
}

#[test]
fn joint_replacement_moves_new_lease_without_borrow_and_does_not_retire_old_or_new_under_lock() {
    let mut fixture = Fixture::new();
    let proof = fixture.proof();
    let (new, new_handle, _new_rx) = fixture.fresh();
    let new_key = new.key.clone();
    let witness = new.publication_witness().unwrap();
    drop(fixture.receiver.take()); // after capture, before conditional commit
    let held_g = fixture.global.clone().try_acquire_many_owned(fixture.global.available_permits() as u32).unwrap();
    let held_p = fixture.published.clone().try_acquire_many_owned(fixture.published.available_permits() as u32).unwrap();
    fixture.global.close();
    fixture.published.close();
    let new = proof.publish_replacement(&witness, &fixture.stamp, || Ok(new)).unwrap();
    assert_eq!(fixture.phase(&new_key) as u8, Phase::Active as u8);
    assert_eq!(fixture.registry.snapshot().active, 2);
    assert!(new_handle.is_alive());
    drop(fixture.old.take()); // exact old key only, outside all commit guards
    assert_eq!(fixture.registry.snapshot().retired, 1);
    assert_eq!(fixture.registry.snapshot().active, 1);
    assert!(new_handle.is_alive());
    assert_eq!(fixture.global.available_permits(), 0);
    assert_eq!(fixture.published.available_permits(), 0);
    drop((new, new_handle, witness, proof, held_g, held_p, fixture));
}

#[test]
fn rejected_action_restores_new_unpublished_and_keeps_exact_original_error() {
    let fixture = Fixture::new();
    let proof = fixture.proof();
    let (new, _new_handle, _rx) = fixture.fresh();
    let witness = new.publication_witness().unwrap();
    let original = anyhow::Error::new(std::io::Error::from(std::io::ErrorKind::ConnectionRefused));
    let pointer = original.downcast_ref::<std::io::Error>().unwrap() as *const std::io::Error;
    let result = proof.publish_replacement::<()>(&witness, &fixture.stamp, || Err(original));
    let error = result.unwrap_err();
    assert_eq!(error.downcast_ref::<std::io::Error>().unwrap() as *const std::io::Error, pointer);
    assert_eq!(fixture.phase(&new.key) as u8, Phase::Unpublished as u8);
    proof.check_replacement().unwrap();
    assert_eq!(fixture.registry.snapshot().retired, 0);
}

#[test]
fn panicking_action_restores_phase_without_callbacks_or_native_owner_drop() {
    let fixture = Fixture::new();
    let proof = fixture.proof();
    let (new, new_handle, _rx) = fixture.fresh();
    let witness = new.publication_witness().unwrap();
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        proof.publish_replacement::<()>(&witness, &fixture.stamp, || std::panic::panic_any(17u8))
    }));
    assert!(result.unwrap_err().is::<u8>());
    assert_eq!(fixture.phase(&new.key) as u8, Phase::Unpublished as u8);
    assert!(new_handle.is_alive());
    proof.check_replacement().unwrap();
    assert_eq!(fixture.registry.snapshot().retired, 0);
}

struct OutsideDrop { registry: RegistryOwner, seen: Arc<AtomicBool> }
impl Drop for OutsideDrop {
    fn drop(&mut self) {
        assert!(self.registry.state.try_lock().is_ok(), "payload dropped inside registry");
        self.seen.store(true, Ordering::SeqCst);
    }
}
#[test]
fn returned_displaced_payload_is_destroyed_after_registry_guard_releases() {
    let fixture = Fixture::new();
    let proof = fixture.proof();
    let (new, _new_handle, _rx) = fixture.fresh();
    let witness = new.publication_witness().unwrap();
    let seen = Arc::new(AtomicBool::new(false));
    let returned = proof.publish_replacement(&witness, &fixture.stamp, || Ok(OutsideDrop {
        registry: fixture.registry.clone(), seen: seen.clone(),
    })).unwrap();
    assert!(!seen.load(Ordering::SeqCst));
    drop(returned);
    assert!(seen.load(Ordering::SeqCst));
}

#[test]
fn old_registry_metadata_or_phase_changes_cannot_authorize_replacement() {
    let fixture = Fixture::new();
    let proof = fixture.proof();
    let (new, _new_handle, _rx) = fixture.fresh();
    let witness = new.publication_witness().unwrap();
    let (wrong, _wrong_g) = fees(&fixture.global, &fixture.published);
    let displaced = {
        let mut state = fixture.registry.lock();
        Registry::entry_mut(&mut state, &fixture.old.as_ref().unwrap().key).unwrap().metadata.replace(wrong)
    };
    assert!(proof.check_replacement().is_err());
    assert!(proof.publish_replacement(&witness, &fixture.stamp, || Ok(())).is_err());
    let wrong = {
        let mut state = fixture.registry.lock();
        Registry::entry_mut(&mut state, &fixture.old.as_ref().unwrap().key).unwrap().metadata.replace(displaced.unwrap())
    };
    drop(wrong);
    proof.check_replacement().unwrap();
    fixture.old.as_ref().unwrap().retire();
    assert!(proof.check_replacement().is_err());
    assert!(proof.publish_replacement(&witness, &fixture.stamp, || Ok(())).is_err());
    assert_eq!(fixture.phase(&new.key) as u8, Phase::Unpublished as u8);
}

#[test]
fn witnessed_new_metadata_changed_or_missing_never_enters_action() {
    let fixture = Fixture::new();
    let proof = fixture.proof();
    let (new, _new_handle, _rx) = fixture.fresh();
    let witness = new.publication_witness().unwrap();
    let displaced = {
        let mut state = fixture.registry.lock();
        Registry::entry_mut(&mut state, &new.key).unwrap().global_metadata.take()
    };
    let entered = AtomicBool::new(false);
    assert!(proof.publish_replacement(&witness, &fixture.stamp, || {
        entered.store(true, Ordering::SeqCst); Ok(())
    }).is_err());
    assert!(!entered.load(Ordering::SeqCst));
    assert!(new.publication_witness().is_err());
    {
        let mut state = fixture.registry.lock();
        Registry::entry_mut(&mut state, &new.key).unwrap().global_metadata = displaced;
    }
    assert_eq!(fixture.phase(&new.key) as u8, Phase::Unpublished as u8);
}

#[test]
fn other_registry_or_other_control_stamp_and_same_runtime_cannot_be_published() {
    let fixture = Fixture::new();
    let proof = fixture.proof();
    let other = Fixture::new();
    let (foreign, _foreign_handle, _foreign_rx) = other.fresh();
    let foreign = foreign.publication_witness().unwrap();
    assert!(proof.publish_replacement(&foreign, &fixture.stamp, || Ok(())).is_err());
    let (new, _handle, _rx) = fixture.fresh();
    let witness = new.publication_witness().unwrap();
    assert!(proof.publish_replacement(&witness, &other.stamp, || Ok(())).is_err());
    let duplicate = fixture.registry.reserve(fixture.stamp.clone()).unwrap().attach(fixture.handle.clone());
    duplicate.retain_metadata(fixture.metadata.clone()).unwrap();
    duplicate.retain_global_metadata(fixture.global_metadata.clone()).unwrap();
    let duplicate_witness = duplicate.publication_witness().unwrap();
    assert!(proof.publish_replacement(&duplicate_witness, &fixture.stamp, || Ok(())).is_err());
}

