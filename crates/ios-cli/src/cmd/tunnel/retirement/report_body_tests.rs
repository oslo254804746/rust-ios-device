use super::*;

#[test]
fn native_report_preflight_bodies_and_slots_refuse_before_backing_then_empty_roots_refund_independently(
) {
    let bank = Arc::new(Semaphore::new(65536));
    let probe = Registry::allocate(&bank, 1).unwrap();
    let total = probe.structural_bytes();
    drop(probe);
    let reports = 2 * NativeReport::body_bytes();
    for available in [
        0,
        Registry::body_bytes() - 1,
        Registry::body_bytes() + reports - 1,
        total - 1,
    ] {
        let bank = Arc::new(Semaphore::new(available));
        let error = super::super::metadata_vec::with_allocation_hook(
            |_, _| panic!("native report preflight refusal allocated slot backing"),
            || Registry::allocate(&bank, 1).err().unwrap(),
        );
        assert_eq!(
            crate::output::operation_error_class(error.as_ref()),
            ErrorClass::ResourceLimit
        );
        assert_eq!(bank.available_permits(), available);
    }
    let bank = Arc::new(Semaphore::new(total));
    let allocations = Arc::new(StdMutex::new(Vec::new()));
    let captured = allocations.clone();
    let charged = bank.clone();
    let registry = super::super::metadata_vec::with_allocation_hook(
        move |layout, pointer| {
            assert_eq!(charged.available_permits(), 0);
            captured.lock().unwrap().push((layout, pointer));
        },
        || Registry::allocate(&bank, 1).unwrap(),
    );
    let observed = allocations.lock().unwrap();
    assert_eq!(observed.len(), 4);
    assert_eq!(
        observed[0].0,
        std::alloc::Layout::array::<Option<Entry>>(1).unwrap()
    );
    assert_eq!(observed[1].0.size(), NativeReport::body_bytes());
    assert_eq!(observed[2].0.size(), NativeReport::body_bytes());
    assert_eq!(observed[3].0.size(), Registry::body_bytes());
    for (index, data) in [
        (1, std::ptr::from_ref(&*registry.error_stock) as usize),
        (2, std::ptr::from_ref(&*registry.expiry_stock) as usize),
        (3, std::ptr::from_ref(&*registry) as usize),
    ] {
        assert!(observed[index].1 <= data && data < observed[index].1 + observed[index].0.size());
    }
    drop(observed);
    assert_eq!(bank.available_permits(), 0);
    let error = registry.error_stock.clone();
    let expiry = registry.expiry_stock.clone();
    for stock in [&error, &expiry] {
        assert!(stock.original.get().is_none());
        assert_eq!(stock.storage().num_permits(), NativeReport::body_bytes());
        assert!(Arc::ptr_eq(stock.storage().semaphore(), &bank));
    }
    bank.close();
    drop(registry);
    assert_eq!(bank.available_permits(), total - reports);
    drop(error);
    assert_eq!(bank.available_permits(), total - NativeReport::body_bytes());
    drop(expiry);
    assert_eq!(bank.available_permits(), total);
}

#[test]
fn native_report_captured_history_parallel_last_roots_keep_original_io_and_body_after_registry_drop(
) {
    let bank = Arc::new(Semaphore::new(65536));
    let registry = Registry::allocate(&bank, 1).unwrap();
    let lease = registry
        .reserve(OwnershipStamp::new(Arc::new(()), Arc::new(())))
        .unwrap();
    let key = lease.key.as_ref().unwrap().clone();
    let original = TunnelError::Io(std::io::Error::from(std::io::ErrorKind::ConnectionReset));
    let captured = registry.record_failure(&key, original);
    let root = match captured {
        PayloadOutcome::History(root) => root,
        PayloadOutcome::Current(_) => panic!("first original must use its prepaid root"),
    };
    let pointer = root.error() as *const TunnelError as usize;
    let io_pointer = match root.error() {
        TunnelError::Io(io) => io as *const std::io::Error as usize,
        _ => panic!("original native IO changed type"),
    };
    let history = registry.historical_failure().unwrap();
    assert!(std::ptr::eq(history.primary(), root.error()));
    bank.close();
    drop((history, lease, registry));
    assert_eq!(
        bank.available_permits(),
        65536 - NativeReport::body_bytes() - Registry::incarnation_bytes()
    );
    let barrier = Arc::new(std::sync::Barrier::new(8));
    std::thread::scope(|scope| {
        for _ in 0..7 {
            let kept = root.clone();
            assert!(kept.ptr_eq(&root));
            let barrier = barrier.clone();
            scope.spawn(move || {
                assert_eq!(kept.error() as *const TunnelError as usize, pointer);
                match kept.error() {
                    TunnelError::Io(io) => {
                        assert_eq!(io as *const std::io::Error as usize, io_pointer)
                    }
                    _ => panic!("original native IO changed type"),
                }
                barrier.wait();
                drop(kept);
            });
        }
        barrier.wait();
        drop(root);
    });
    assert_eq!(
        bank.available_permits(),
        65536 - Registry::incarnation_bytes()
    );
    drop(key);
    assert_eq!(bank.available_permits(), 65536);
}
