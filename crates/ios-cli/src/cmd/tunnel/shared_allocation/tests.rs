use super::*;
use std::sync::{
    atomic::{AtomicBool, AtomicUsize, Ordering},
    Mutex,
};

#[repr(align(256))]
struct Aligned(u64);
#[test]
fn exact_shared_full_declared_layout_is_admitted_before_only_allocation_and_one_byte_refuses() {
    let needed = Shared::<Aligned>::allocation_bytes();
    assert_eq!(
        needed,
        std::alloc::Layout::new::<Allocation<Aligned>>().size()
    );
    assert!(needed > mem::size_of::<Aligned>());
    for available in [0, needed - 1, needed] {
        let bank = Arc::new(Semaphore::new(available));
        let seen = Arc::new(Mutex::new(Vec::new()));
        let capture = seen.clone();
        let charged = bank.clone();
        let result = super::super::metadata_vec::with_allocation_hook(
            move |layout, pointer| {
                assert_eq!(charged.available_permits(), 0);
                assert_eq!(layout, std::alloc::Layout::new::<Allocation<Aligned>>());
                assert_eq!(pointer % layout.align(), 0);
                capture.lock().unwrap().push(pointer);
            },
            || {
                let fee = super::super::admission::known_metadata_charge(&bank, needed)?;
                Shared::from_grant(Aligned(42), &bank, fee)
            },
        );
        if available < needed {
            assert_eq!(
                crate::output::operation_error_class(result.err().unwrap().as_ref()),
                ios_core::ErrorClass::ResourceLimit
            );
            assert!(seen.lock().unwrap().is_empty());
        } else {
            let root = result.unwrap();
            assert_eq!(root.0, 42);
            assert_eq!(
                seen.lock().unwrap().as_slice(),
                [root.pointer.as_ptr() as usize]
            );
            let copy = root.clone();
            assert!(root.ptr_eq(&copy));
            assert_eq!(root.storage().num_permits(), needed);
            drop(root);
            assert_eq!(bank.available_permits(), 0);
            drop(copy);
        }
        assert_eq!(bank.available_permits(), available);
    }
}
#[test]
fn exact_shared_wrong_original_bank_or_layout_refuses_before_any_backing_and_refunds() {
    let needed = Shared::<u64>::allocation_bytes();
    for same_bank in [true, false] {
        let bank = Arc::new(Semaphore::new(needed));
        let original = if same_bank {
            bank.clone()
        } else {
            Arc::new(Semaphore::new(needed))
        };
        let size = if same_bank { needed - 1 } else { needed };
        let grant = original
            .clone()
            .try_acquire_many_owned(size as u32)
            .unwrap();
        let allocated = Arc::new(AtomicBool::new(false));
        let capture = allocated.clone();
        let result = super::super::metadata_vec::with_allocation_hook(
            move |_, _| {
                capture.store(true, Ordering::SeqCst);
            },
            || Shared::from_grant(7_u64, &bank, grant),
        );
        assert_eq!(
            crate::output::operation_error_class(result.err().unwrap().as_ref()),
            ios_core::ErrorClass::ResourceLimit
        );
        assert!(!allocated.load(Ordering::SeqCst));
        assert_eq!(bank.available_permits(), needed);
        assert_eq!(original.available_permits(), needed);
    }
}
struct Probe {
    bank: Arc<Semaphore>,
    freed: Arc<AtomicBool>,
    dropped: Arc<AtomicUsize>,
    panic: bool,
}
impl Drop for Probe {
    fn drop(&mut self) {
        assert!(
            self.freed.load(Ordering::SeqCst),
            "original allocation must end before value Drop"
        );
        assert_eq!(
            self.bank.available_permits(),
            0,
            "grant must cover allocation and value destruction"
        );
        assert_eq!(self.dropped.fetch_add(1, Ordering::SeqCst), 0);
        assert!(!self.panic, "synthetic original destructor unwind");
    }
}
#[test]
fn exact_shared_deallocation_precedes_original_destructor_and_final_fee_even_during_unwind() {
    for panics in [false, true] {
        let needed = Shared::<Probe>::allocation_bytes();
        let bank = Arc::new(Semaphore::new(needed));
        let freed = Arc::new(AtomicBool::new(false));
        let dropped = Arc::new(AtomicUsize::new(0));
        let fee = bank.clone().try_acquire_many_owned(needed as u32).unwrap();
        let root = Shared::from_grant(
            Probe {
                bank: bank.clone(),
                freed: freed.clone(),
                dropped: dropped.clone(),
                panic: panics,
            },
            &bank,
            fee,
        )
        .unwrap();
        let last = root.clone();
        drop(root);
        assert!(!freed.load(Ordering::SeqCst));
        assert_eq!(dropped.load(Ordering::SeqCst), 0);
        let charged = bank.clone();
        let flag = freed.clone();
        let called = dropped.clone();
        bank.close();
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            after_free(
                move || {
                    assert_eq!(charged.available_permits(), 0);
                    assert_eq!(called.load(Ordering::SeqCst), 0);
                    flag.store(true, Ordering::SeqCst);
                },
                || drop(last),
            );
        }));
        assert_eq!(result.is_err(), panics);
        assert!(freed.load(Ordering::SeqCst));
        assert_eq!(dropped.load(Ordering::SeqCst), 1);
        assert_eq!(bank.available_permits(), needed);
    }
}
#[test]
fn exact_shared_parallel_clone_release_keeps_original_pointer_and_frees_one_complete_control_block()
{
    let needed = Shared::<Probe>::allocation_bytes();
    let bank = Arc::new(Semaphore::new(needed));
    let freed = Arc::new(AtomicBool::new(false));
    let dropped = Arc::new(AtomicUsize::new(0));
    let fee = bank.clone().try_acquire_many_owned(needed as u32).unwrap();
    let original = Shared::from_grant(
        Probe {
            bank: bank.clone(),
            freed: freed.clone(),
            dropped: dropped.clone(),
            panic: false,
        },
        &bank,
        fee,
    )
    .unwrap();
    let pointer = std::ptr::from_ref(&*original) as usize;
    let barrier = Arc::new(std::sync::Barrier::new(9));
    std::thread::scope(|scope| {
        for _ in 0..8 {
            let root = original.clone();
            let barrier = barrier.clone();
            let flag = freed.clone();
            scope.spawn(move || {
                after_free(
                    move || {
                        flag.store(true, Ordering::SeqCst);
                    },
                    || {
                        for _ in 0..1024 {
                            let copy = root.clone();
                            assert!(copy.ptr_eq(&root));
                            assert_eq!(std::ptr::from_ref(&*copy) as usize, pointer);
                            drop(copy);
                        }
                        barrier.wait();
                        assert_eq!(root.bank.available_permits(), 0);
                        barrier.wait();
                        drop(root);
                    },
                )
            });
        }
        bank.close();
        drop(original);
        barrier.wait();
        assert!(!freed.load(Ordering::SeqCst));
        barrier.wait();
    });
    assert!(freed.load(Ordering::SeqCst));
    assert_eq!(dropped.load(Ordering::SeqCst), 1);
    assert_eq!(bank.available_permits(), needed);
}

struct FieldsProbe {
    bytes: Vec<u8>,
    bank: Arc<Semaphore>,
    freed: Arc<AtomicBool>,
    drops: Arc<AtomicUsize>,
}
impl Drop for FieldsProbe {
    fn drop(&mut self) {
        assert!(self.freed.load(Ordering::SeqCst));
        assert_eq!(self.bank.available_permits(), 0);
        assert_eq!(self.drops.fetch_add(1, Ordering::SeqCst), 0);
    }
}
#[test]
fn exact_shared_fields_whole_original_grant_pays_buffer_and_control_before_allocation_until_last_drop(
) {
    let text = b"original-paid-field";
    let total = Shared::<FieldsProbe>::allocation_bytes() + text.len();
    let bank = Arc::new(Semaphore::new(total));
    let mut grant = super::super::admission::known_metadata_charge(&bank, total).unwrap();
    let freed = Arc::new(AtomicBool::new(false));
    let drops = Arc::new(AtomicUsize::new(0));
    let observed = Arc::new(Mutex::new(Vec::new()));
    let capture = observed.clone();
    let charged = bank.clone();
    let root = super::super::metadata_vec::with_allocation_hook(
        move |layout, pointer| {
            assert_eq!(charged.available_permits(), 0);
            capture.lock().unwrap().push((layout, pointer));
        },
        || {
            let field = super::super::metadata_vec::from_grant::<u8>(
                &bank,
                text.len(),
                grant.split(text.len()).unwrap(),
            )
            .unwrap();
            grant.merge(field.storage);
            let mut bytes = field.values;
            bytes.extend_from_slice(text);
            bank.close(); // The whole original grant is already admitted.
            Shared::from_grant_with_fields(
                FieldsProbe {
                    bytes,
                    bank: bank.clone(),
                    freed: freed.clone(),
                    drops: drops.clone(),
                },
                &bank,
                grant,
                text.len(),
            )
            .unwrap()
        },
    );
    let layouts = observed.lock().unwrap();
    assert_eq!(layouts.len(), 2);
    assert_eq!(
        layouts[0],
        (
            std::alloc::Layout::array::<u8>(text.len()).unwrap(),
            root.bytes.as_ptr() as usize
        )
    );
    assert_eq!(
        layouts[1].0,
        std::alloc::Layout::new::<Allocation<FieldsProbe>>()
    );
    assert_eq!(layouts[1].1, root.pointer.as_ptr() as usize);
    assert_eq!(root.storage().num_permits(), total);
    let last = root.clone();
    let pointer = root.bytes.as_ptr();
    drop(root);
    assert_eq!(bank.available_permits(), 0);
    assert_eq!(last.bytes.as_ptr(), pointer);
    assert_eq!(last.bytes, text);
    drop(layouts);
    after_free(
        move || {
            freed.store(true, Ordering::SeqCst);
        },
        || drop(last),
    );
    assert_eq!(drops.load(Ordering::SeqCst), 1);
    assert_eq!(bank.available_permits(), total);
}
struct RefusedFields {
    bank: Arc<Semaphore>,
    dropped: Arc<AtomicUsize>,
    _bytes: Vec<u8>,
}
impl Drop for RefusedFields {
    fn drop(&mut self) {
        assert_eq!(
            self.bank.available_permits(),
            0,
            "original field grant refunded before input destruction"
        );
        self.dropped.fetch_add(1, Ordering::SeqCst);
    }
}
#[test]
fn exact_shared_fields_mismatched_bank_size_and_overflow_drop_input_before_refund_without_backing()
{
    for case in 0..3 {
        let field_bytes = 17;
        let total = Shared::<RefusedFields>::allocation_bytes() + field_bytes;
        let bank = Arc::new(Semaphore::new(total));
        let other = Arc::new(Semaphore::new(total));
        let grant = bank.clone().try_acquire_many_owned(total as u32).unwrap();
        let drops = Arc::new(AtomicUsize::new(0));
        let value = RefusedFields {
            bank: bank.clone(),
            dropped: drops.clone(),
            _bytes: vec![7; field_bytes],
        };
        let target = if case == 0 { &other } else { &bank };
        let declared = if case == 2 {
            usize::MAX
        } else {
            field_bytes - usize::from(case == 1)
        };
        let error = super::super::metadata_vec::with_allocation_hook(
            |_, _| panic!("unadmitted fields root allocation"),
            || {
                Shared::from_grant_with_fields(value, target, grant, declared)
                    .err()
                    .unwrap()
            },
        );
        assert_eq!(
            crate::output::operation_error_class(error.as_ref()),
            ios_core::ErrorClass::ResourceLimit
        );
        assert_eq!(drops.load(Ordering::SeqCst), 1);
        assert_eq!(bank.available_permits(), total);
        assert_eq!(other.available_permits(), total);
    }
}
