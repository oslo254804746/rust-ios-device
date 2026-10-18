use super::super::admission;
use super::*;

#[test]
fn metadata_lease_foreign_or_body_short_in_either_original_grant_refunds_both_pools() {
    let bytes = Lease::pair_bytes();
    for selected in 0..2 {
        for foreign in [false, true] {
            let global_bank = Arc::new(Semaphore::new(bytes));
            let publication_bank = Arc::new(Semaphore::new(bytes));
            let other = Arc::new(Semaphore::new(bytes));
            let global = admission::known_metadata_charge(
                if selected == 0 && foreign {
                    &other
                } else {
                    &global_bank
                },
                if selected == 0 && !foreign {
                    bytes - 1
                } else {
                    bytes
                },
            )
            .unwrap();
            let publication = admission::known_metadata_charge(
                if selected == 1 && foreign {
                    &other
                } else {
                    &publication_bank
                },
                if selected == 1 && !foreign {
                    bytes - 1
                } else {
                    bytes
                },
            )
            .unwrap();
            let error = Lease::from_grants(&global_bank, &publication_bank, global, publication)
                .err()
                .unwrap();
            assert_eq!(
                crate::output::operation_error_class(error.as_ref()),
                ios_core::ErrorClass::ResourceLimit
            );
            assert_eq!(global_bank.available_permits(), bytes);
            assert_eq!(publication_bank.available_permits(), bytes);
            assert_eq!(other.available_permits(), bytes);
        }
    }
}

fn pair(
    global: &Arc<Semaphore>,
    publication: &Arc<Semaphore>,
    g: usize,
    p: usize,
) -> (Owner, Owner) {
    let global_grant = admission::known_metadata_charge(global, g).unwrap();
    let publication_grant = admission::known_metadata_charge(publication, p).unwrap();
    Lease::from_grants(global, publication, global_grant, publication_grant).unwrap()
}

#[test]
fn metadata_lease_either_last_root_keeps_its_body_in_both_closed_pools_independently() {
    let body = Lease::body_bytes();
    for retain_publication in [false, true] {
        let global_total = Lease::pair_bytes() + 11;
        let publication_total = Lease::pair_bytes() + 17;
        let global = Arc::new(Semaphore::new(global_total));
        let publication = Arc::new(Semaphore::new(publication_total));
        let (published, general) = pair(&global, &publication, global_total, publication_total);
        assert!(Arc::ptr_eq(published.semaphore(), &publication));
        assert!(Arc::ptr_eq(general.semaphore(), &global));
        assert!(Arc::ptr_eq(published._cross_body.semaphore(), &global));
        assert!(Arc::ptr_eq(general._cross_body.semaphore(), &publication));
        assert_eq!(published.num_permits(), body + 17);
        assert_eq!(general.num_permits(), body + 11);
        assert_eq!(published._cross_body.num_permits(), body);
        assert_eq!(general._cross_body.num_permits(), body);
        assert_eq!(global.available_permits(), 0);
        assert_eq!(publication.available_permits(), 0);
        global.close();
        publication.close();
        let (held, freed) = if retain_publication {
            (published, general)
        } else {
            (general, published)
        };
        let pointer = &*held as *const Lease;
        let retained = held.clone();
        assert!(held.ptr_eq(&retained));
        drop(freed);
        let g_free = if retain_publication { body + 11 } else { body };
        let p_free = if retain_publication { body } else { body + 17 };
        assert_eq!(global.available_permits(), g_free);
        assert_eq!(publication.available_permits(), p_free);
        drop(held);
        assert_eq!(&*retained as *const Lease, pointer);
        assert_eq!(global.available_permits(), g_free);
        assert_eq!(publication.available_permits(), p_free);
        drop(retained);
        assert_eq!(global.available_permits(), global_total);
        assert_eq!(publication.available_permits(), publication_total);
    }
}

#[test]
fn metadata_lease_parallel_original_roots_keep_bank_identity_and_refund_each_pair_once() {
    let total = Lease::pair_bytes();
    let global = Arc::new(Semaphore::new(total));
    let publication = Arc::new(Semaphore::new(total));
    let (published, general) = pair(&global, &publication, total, total);
    global.close();
    publication.close();
    let p_pointer = &*published as *const Lease as usize;
    let g_pointer = &*general as *const Lease as usize;
    let barrier = Arc::new(std::sync::Barrier::new(8));
    std::thread::scope(|scope| {
        for _ in 0..7 {
            let p = published.clone();
            let g = general.clone();
            let publication = publication.clone();
            let global = global.clone();
            let barrier = barrier.clone();
            scope.spawn(move || {
                assert_eq!(&*p as *const Lease as usize, p_pointer);
                assert_eq!(&*g as *const Lease as usize, g_pointer);
                assert!(Arc::ptr_eq(p.semaphore(), &publication));
                assert!(Arc::ptr_eq(g.semaphore(), &global));
                barrier.wait();
                drop((g, p));
            });
        }
        barrier.wait();
        drop((published, general));
    });
    assert_eq!(global.available_permits(), total);
    assert_eq!(publication.available_permits(), total);
}
