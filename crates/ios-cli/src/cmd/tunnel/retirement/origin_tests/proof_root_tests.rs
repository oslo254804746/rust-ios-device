use super::super::super::{birth_bank::BirthBank, metadata_vec, shared_weak_allocation};
use super::*;

struct Fixture {
    registry: RegistryOwner,
    lease: Option<NativeLease>,
    stamp: OwnershipStamp,
    origin: BirthBank,
    other: BirthBank,
    total: usize,
    runtime: Arc<TunnelHandle>,
    peer: DuplexStream,
    metadata: super::super::super::metadata_lease::Owner,
    global_metadata: super::super::super::metadata_lease::Owner,
    _origin_birth: Cutover<1>,
    _other_birth: Cutover<1>,
}

impl Fixture {
    async fn new() -> Self {
        let registry = registry();
        let total = registry._slots_storage.num_permits()
            + RefreshProof::body_bytes()
            + ControlIdentity::body_bytes();
        let origin = BirthBank::new(total);
        let other = BirthBank::new(total);
        let mut origin_birth = Cutover::<1>::new(&origin, origin.raw()).unwrap();
        let mut other_birth = Cutover::<1>::new(&other, other.raw()).unwrap();
        let stamp = OwnershipStamp::new(origin_birth.place(&origin), other_birth.place(&other));
        let (runtime, peer) = native().await;
        let lease = registry
            .reserve(stamp.clone())
            .unwrap()
            .attach(runtime.clone());
        let (metadata, global_metadata) =
            metadata(&registry.byte_bank(), &Arc::new(Semaphore::new(65536)));
        lease.retain_metadata(metadata.clone()).unwrap();
        lease
            .retain_global_metadata(global_metadata.clone())
            .unwrap();
        lease.publish(&stamp).unwrap();
        Self {
            registry,
            lease: Some(lease),
            stamp,
            origin,
            other,
            total,
            runtime,
            peer,
            metadata,
            global_metadata,
            _origin_birth: origin_birth,
            _other_birth: other_birth,
        }
    }

    fn capture(&self, bank: &Arc<Semaphore>) -> Result<RefreshProof> {
        self.lease.as_ref().unwrap().capture_refresh(
            self.stamp.clone(),
            &self.runtime,
            &self.metadata,
            &self.global_metadata,
            None,
            bank,
        )
    }

    async fn finish(mut self, expected: usize) {
        drop(self.lease.take());
        join(&self.registry, &self.runtime, &mut self.peer, expected).await;
    }
}

#[tokio::test]
async fn refresh_proof_root_each_actual_bank_refuses_before_backing_and_rolls_back_other_grants() {
    let bytes = RefreshProof::body_bytes();
    for denied_index in 0..3 {
        for closed in [false, true] {
            let expected = tokio::runtime::Handle::current()
                .metrics()
                .num_alive_tasks();
            let fixture = Fixture::new().await;
            let destination = Arc::new(Semaphore::new(bytes));
            let banks = [&destination, fixture.origin.raw(), fixture.other.raw()];
            let held = if closed {
                None
            } else {
                Some(banks[denied_index].clone().try_acquire_owned().unwrap())
            };
            if closed {
                banks[denied_index].close();
            }
            let before = banks.map(|bank| bank.available_permits());
            let hook =
                metadata_vec::on_allocation(|_, _| panic!("unadmitted RefreshProof backing"));
            let error = fixture.capture(&destination).err().unwrap();
            drop(hook);
            assert_eq!(
                crate::output::operation_error_class(error.as_ref()),
                ios_core::ErrorClass::ResourceLimit
            );
            assert_eq!(banks.map(|bank| bank.available_permits()), before);
            assert_eq!(fixture.registry.snapshot().active, 1);
            assert!(fixture.runtime.is_alive());
            drop(held);
            fixture.finish(expected).await;
        }
    }
}

#[tokio::test]
async fn refresh_proof_root_real_allocation_refusal_returns_all_fees_without_changing_native_owner()
{
    let expected = tokio::runtime::Handle::current()
        .metrics()
        .num_alive_tasks();
    let fixture = Fixture::new().await;
    let bytes = RefreshProof::body_bytes();
    let destination = Arc::new(Semaphore::new(bytes));
    let calls = Arc::new(AtomicUsize::new(0));
    let observed = calls.clone();
    let a = fixture.origin.raw().clone();
    let c = fixture.other.raw().clone();
    let b = destination.clone();
    let refusal = metadata_vec::refuse_allocation(move |layout| {
        assert_eq!(layout.size(), bytes);
        assert_eq!(
            (
                a.available_permits(),
                b.available_permits(),
                c.available_permits()
            ),
            (0, 0, 0)
        );
        observed.fetch_add(1, Ordering::SeqCst);
        true
    });
    let error = fixture.capture(&destination).err().unwrap();
    drop(refusal);
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    assert_eq!(
        crate::output::operation_error_class(error.as_ref()),
        ios_core::ErrorClass::ResourceLimit
    );
    assert_eq!(
        (
            destination.available_permits(),
            fixture.origin.available_permits(),
            fixture.other.available_permits()
        ),
        (bytes, bytes, bytes)
    );
    assert_eq!(fixture.registry.snapshot().active, 1);
    assert!(fixture.runtime.is_alive());
    fixture.finish(expected).await;
}

#[tokio::test]
async fn refresh_proof_root_actual_span_original_fields_and_physical_free_keep_complete_fees() {
    let expected = tokio::runtime::Handle::current()
        .metrics()
        .num_alive_tasks();
    let fixture = Fixture::new().await;
    let bytes = RefreshProof::body_bytes();
    let destination = Arc::new(Semaphore::new(bytes));
    let pointer = Arc::new(AtomicUsize::new(0));
    let observed = pointer.clone();
    let a = fixture.origin.raw().clone();
    let c = fixture.other.raw().clone();
    let b = destination.clone();
    let hook = metadata_vec::on_allocation(move |layout, pointer| {
        assert_eq!(layout.size(), bytes);
        assert_eq!(
            (
                a.available_permits(),
                b.available_permits(),
                c.available_permits()
            ),
            (0, 0, 0)
        );
        assert_eq!(observed.swap(pointer, Ordering::SeqCst), 0);
    });
    let proof = fixture.capture(&destination).unwrap();
    drop(hook);
    let start = pointer.load(Ordering::SeqCst);
    let value = &*proof as *const RefreshProofData as usize;
    assert!(start <= value && value + std::mem::size_of::<RefreshProofData>() <= start + bytes);
    assert!(Arc::ptr_eq(proof.runtime(), &fixture.runtime));
    assert!(proof.metadata.ptr_eq(&fixture.metadata));
    assert!(proof.global_metadata.ptr_eq(&fixture.global_metadata));
    assert!(proof
        .key
        .incarnation
        .ptr_eq(&fixture.lease.as_ref().unwrap().key.incarnation));
    proof.check().unwrap();
    destination.close();
    fixture.origin.close();
    fixture.other.close();
    let a = fixture.origin.raw().clone();
    let c = fixture.other.raw().clone();
    let b = destination.clone();
    shared_weak_allocation::after_free(
        move || {
            assert_eq!(
                (
                    a.available_permits(),
                    b.available_permits(),
                    c.available_permits()
                ),
                (0, 0, 0)
            )
        },
        || drop(proof),
    );
    assert_eq!(
        (
            destination.available_permits(),
            fixture.origin.available_permits(),
            fixture.other.available_permits()
        ),
        (bytes, bytes, bytes)
    );
    assert!(
        fixture.runtime.is_alive(),
        "ending proof cannot retire original native runtime"
    );
    assert_eq!(fixture.registry.snapshot().active, 1);
    let total = fixture.total;
    let a = fixture.origin.raw().clone();
    let c = fixture.other.raw().clone();
    fixture.finish(expected).await;
    assert_eq!(
        (a.available_permits(), c.available_permits()),
        (total, total)
    );
}
