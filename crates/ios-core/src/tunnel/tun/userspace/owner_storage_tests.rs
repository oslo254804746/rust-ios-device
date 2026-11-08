use super::*;
use tokio::io::AsyncReadExt;

#[test]
fn listener_limit_whole_original_root_admission_precedes_backing_and_native_capacity_at_closed_zero_pool(
) {
    use crate::device::prepared_rsd::with_allocation_hook;
    let bytes = mpsc::Slots::body_bytes();
    let short = Arc::new(Semaphore::new(bytes - 1));
    let error = with_allocation_hook(
        |_, _| panic!("short listener limit grant allocated a backing"),
        || match ListenerSlots::new(1, Some(&short)) {
            Err(error) => error,
            Ok(_) => panic!("short listener root accepted"),
        },
    );
    assert_eq!(
        crate::classify_error(&error),
        crate::ErrorClass::ResourceLimit
    );
    assert_eq!(short.available_permits(), bytes - 1);
    for capacity in [0, 1] {
        let bank = Arc::new(Semaphore::new(bytes));
        let allocations = Arc::new(std::sync::Mutex::new(Vec::new()));
        let observed = allocations.clone();
        let observed_bank = bank.clone();
        let slots = with_allocation_hook(
            move |layout, pointer| {
                assert_eq!(layout.size(), bytes);
                assert_eq!(observed_bank.available_permits(), 0);
                observed.lock().unwrap().push(pointer);
            },
            || ListenerSlots::new(capacity, Some(&bank)).unwrap(),
        );
        let ListenerSlots::Metered(root) = &slots else {
            panic!("metadata bank bypassed");
        };
        let address = root.allocation_address();
        assert_eq!(&*allocations.lock().unwrap(), &[address]);
        bank.close();
        let clone = with_allocation_hook(
            |_, _| panic!("listener limit clone allocated a controlled backing"),
            || slots.clone(),
        );
        let ListenerSlots::Metered(other) = &clone else {
            panic!("clone bypassed metadata");
        };
        assert_eq!(other.allocation_address(), address);
        if capacity == 0 {
            assert!(matches!(
                clone.try_acquire_owned(),
                Err(tokio::sync::TryAcquireError::NoPermits)
            ));
            drop(slots);
        } else {
            let permit = clone.try_acquire_owned().unwrap();
            assert!(matches!(
                slots.clone().try_acquire_owned(),
                Err(tokio::sync::TryAcquireError::NoPermits)
            ));
            drop(permit);
            let kept = slots.clone().try_acquire_owned().unwrap();
            drop(slots);
            assert_eq!(
                bank.available_permits(),
                0,
                "last native client permit must retain its original complete root"
            );
            drop(kept);
        }
        assert_eq!(bank.available_permits(), bytes);
    }
}

#[tokio::test]
async fn task_handle_real_backing_one_byte_refusal_and_original_joins_keep_fee_until_drop() {
    let required = TaskHandles::body_bytes();
    let short = Arc::new(Semaphore::new(required - 1));
    assert!(runtime_storage::task_handles(Some(&short)).is_err());
    assert_eq!(short.available_permits(), required - 1);
    let bank = Arc::new(Semaphore::new(required));
    let mut handles = runtime_storage::task_handles(Some(&bank)).unwrap();
    assert_eq!(bank.available_permits(), 0);
    let ledger = Ledger::new();
    handles.initialize(std::array::from_fn(|_| {
        OwnedTask::new(
            tokio::spawn(async { Ok(()) }),
            ParentRole::Writer,
            ledger.clone(),
        )
    }));
    assert_eq!(handles.len(), 4);
    for handle in &handles {
        handle.join().await;
    }
    ledger.result(None).unwrap();
    assert_eq!(bank.available_permits(), 0);
    drop(handles);
    assert_eq!(bank.available_permits(), required);
}

#[tokio::test]
async fn owner_constructor_last_byte_refusal_refunds_actual_bodies_before_workers_and_strict_eof() {
    let baseline = tokio::runtime::Handle::current()
        .metrics()
        .num_alive_tasks();
    let needed = IPV6_HEADER_LEN
        + runtime_channel_bytes()
        + runtime_owner_bytes()
        + mpsc::Slots::body_bytes();
    let bank = Arc::new(Semaphore::new(needed - 1));
    let (stream, mut peer) = tokio::io::duplex(1280);
    let error = UserspaceTunDevice::start_validated_with_bank(
        "fd00::2",
        "fd00::1",
        ValidatedMtu::new(1280).unwrap(),
        stream,
        Some(Arc::clone(&bank)),
    )
    .await
    .err()
    .expect("owner last-byte shortage started workers");
    assert_eq!(
        crate::classify_error(&error),
        crate::ErrorClass::ResourceLimit
    );
    assert_eq!(bank.available_permits(), needed - 1);
    assert_eq!(
        tokio::runtime::Handle::current()
            .metrics()
            .num_alive_tasks(),
        baseline
    );
    let mut byte = [0];
    assert!(matches!(
        tokio::time::timeout(Duration::from_secs(1), peer.read(&mut byte)).await,
        Ok(Ok(0))
    ));
}
