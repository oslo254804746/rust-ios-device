use super::*;

#[test]
fn observed_bytes_borrow_all_exact_routes_and_refund_actual_backing() {
    let targets = [
        ("z", "first"),
        ("a", "other"),
        ("z", "later"),
        ("z", "first"),
    ]
    .map(|(udid, host)| PairedMobdev2Device {
        udid: udid.into(),
        host: host.into(),
    });
    let pointer = targets[0].host.as_ptr();
    let bank = Arc::new(Semaphore::new(4096));
    let index = Index::new(&targets, &bank).unwrap();
    assert_eq!(
        bank.available_permits(),
        4096 - index.positions.capacity() * std::mem::size_of::<usize>()
    );
    assert!(index.contains(&targets, "z", "first"));
    assert!(index.contains(&targets, "z", "later"));
    assert!(index.contains(&targets, "a", "other"));
    assert!(!index.contains(&targets, "a", "first"));
    assert!(!index.contains(&targets, "absent", "other"));
    assert_eq!(targets[0].host.as_ptr(), pointer);
    drop(index);
    assert_eq!(bank.available_permits(), 4096);
}

#[test]
fn observed_bytes_exact_and_one_byte_refusal_release_every_temporary_owner() {
    let targets = [PairedMobdev2Device {
        udid: "a".into(),
        host: "host".into(),
    }];
    let bytes = std::mem::size_of::<usize>();
    for limit in [bytes - 1, bytes] {
        let bank = Arc::new(Semaphore::new(limit));
        match Index::new(&targets, &bank) {
            Ok(index) => {
                assert_eq!(limit, bytes);
                assert_eq!(bank.available_permits(), 0);
                drop(index);
            }
            Err(error) => {
                assert_eq!(limit, bytes - 1);
                assert_eq!(
                    crate::output::operation_error_class(error.as_ref()),
                    ios_core::ErrorClass::ResourceLimit
                );
            }
        }
        assert_eq!(bank.available_permits(), limit);
    }
}
