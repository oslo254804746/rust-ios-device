use super::super::{future_storage::Dynamic, metadata_vec};
use super::*;
use std::sync::atomic::{AtomicUsize, Ordering};

#[test]
fn removed_body_original_grant_funds_exact_box_at_zero_and_refuses_foreign_or_wrong_layout() {
    let bytes = std::mem::size_of::<u128>();
    let bank = Arc::new(Semaphore::new(bytes));
    let fee = super::super::admission::known_metadata_charge(&bank, bytes).unwrap();
    let pointer = Arc::new(AtomicUsize::new(0));
    let captured = pointer.clone();
    let charged = bank.clone();
    let owner = metadata_vec::with_allocation_hook(
        move |layout, original| {
            assert_eq!(layout, std::alloc::Layout::new::<u128>());
            assert_eq!(charged.available_permits(), 0);
            captured.store(original, Ordering::SeqCst);
        },
        || {
            Destination::<u128>::from_grant(&bank, fee)
                .unwrap()
                .place(716)
        },
    );
    assert_eq!(
        &*owner as *const u128 as usize,
        pointer.load(Ordering::SeqCst)
    );
    assert_eq!(*owner, 716);
    assert_eq!(owner.bytes_in(&bank).unwrap(), bytes);
    drop(owner);
    assert_eq!(bank.available_permits(), bytes);
    for (foreign, amount) in [(false, bytes - 1), (false, bytes + 1), (true, bytes)] {
        let bank = Arc::new(Semaphore::new(2 * bytes));
        let other = Arc::new(Semaphore::new(2 * bytes));
        let fee = super::super::admission::known_metadata_charge(
            if foreign { &other } else { &bank },
            amount,
        )
        .unwrap();
        let error = metadata_vec::with_allocation_hook(
            |_, _| panic!("invalid body grant allocated"),
            || Destination::<u128>::from_grant(&bank, fee).err().unwrap(),
        );
        assert_eq!(
            crate::output::operation_error_class(error.as_ref()),
            ios_core::ErrorClass::ResourceLimit
        );
        assert_eq!(bank.available_permits(), 2 * bytes);
        assert_eq!(other.available_permits(), 2 * bytes);
    }
}

#[test]
fn node_body_into_inner_frees_original_high_alignment_box_before_refund_and_value_drop() {
    #[repr(align(128))]
    struct Value(Arc<Semaphore>, Arc<AtomicUsize>, usize);
    impl Drop for Value {
        fn drop(&mut self) {
            assert_eq!(self.0.available_permits(), std::mem::size_of::<Self>());
            self.1.fetch_add(1, Ordering::SeqCst);
        }
    }
    let bytes = std::mem::size_of::<Value>();
    let bank = Arc::new(Semaphore::new(bytes));
    let drops = Arc::new(AtomicUsize::new(0));
    let owner = Destination::new(&bank)
        .unwrap()
        .place(Value(bank.clone(), drops.clone(), 713));
    assert_eq!(&*owner as *const Value as usize % 128, 0);
    assert_eq!(bank.available_permits(), 0);
    let value = metadata_vec::with_allocation_hook(
        |_, _| panic!("into_inner allocated"),
        || owner.into_inner(),
    );
    assert_eq!(value.2, 713);
    assert_eq!(drops.load(Ordering::SeqCst), 0);
    assert_eq!(bank.available_permits(), bytes);
    drop(value);
    assert_eq!(drops.load(Ordering::SeqCst), 1);
}

#[test]
fn node_body_into_inner_zst_moves_once_without_allocation_or_double_drop() {
    static DROPS: AtomicUsize = AtomicUsize::new(0);
    struct Value;
    impl Drop for Value {
        fn drop(&mut self) {
            DROPS.fetch_add(1, Ordering::SeqCst);
        }
    }
    DROPS.store(0, Ordering::SeqCst);
    let bank = Arc::new(Semaphore::new(0));
    let value = metadata_vec::with_allocation_hook(
        |_, _| panic!("ZST allocated"),
        || Destination::new(&bank).unwrap().place(Value).into_inner(),
    );
    assert_eq!(DROPS.load(Ordering::SeqCst), 0);
    assert_eq!(bank.available_permits(), 0);
    drop(value);
    assert_eq!(DROPS.load(Ordering::SeqCst), 1);
}

#[repr(align(128))]
struct Device {
    bank: Arc<Semaphore>,
    drops: Arc<AtomicUsize>,
    value: usize,
}
impl Drop for Device {
    fn drop(&mut self) {
        assert_eq!(self.bank.available_permits(), 0);
        assert_eq!(self as *const Self as usize % 128, 0);
        self.drops.fetch_add(1, Ordering::SeqCst);
    }
}

#[tokio::test]
async fn returned_device_box_original_buffer_pointer_alignment_and_owner_outlive_source_frame() {
    let bank = Arc::new(Semaphore::new(8192));
    let drops = Arc::new(AtomicUsize::new(0));
    let pointer = Arc::new(AtomicUsize::new(0));
    let captured = pointer.clone();
    let charged = bank.clone();
    let destination = metadata_vec::with_allocation_hook(
        move |layout, original| {
            assert_eq!(layout, std::alloc::Layout::new::<Device>());
            assert_eq!(charged.available_permits(), 8192 - layout.size());
            captured.store(original, Ordering::SeqCst);
        },
        || Destination::<Device>::new(&bank).unwrap(),
    );
    let data_bank = bank.clone();
    let dropped = drops.clone();
    let future = Dynamic::new(
        async move {
            destination.place(Device {
                bank: data_bank,
                drops: dropped,
                value: 11,
            })
        },
        &bank,
    )
    .unwrap();
    let mut owner = future.await;
    assert_eq!(
        &*owner as *const Device as usize,
        pointer.load(Ordering::SeqCst)
    );
    assert_eq!(
        owner.bytes_in(&bank).unwrap(),
        std::mem::size_of::<Device>()
    );
    assert_eq!(
        bank.available_permits(),
        8192 - std::mem::size_of::<Device>()
    );
    owner.value = 22;
    let moved = owner;
    assert_eq!(moved.value, 22);
    assert_eq!(drops.load(Ordering::SeqCst), 0);
    let other_usage = bank
        .clone()
        .try_acquire_many_owned(bank.available_permits() as u32)
        .unwrap();
    drop(moved);
    assert_eq!(drops.load(Ordering::SeqCst), 1);
    assert_eq!(bank.available_permits(), std::mem::size_of::<Device>());
    drop(other_usage);
    assert_eq!(bank.available_permits(), 8192);
}

#[test]
fn returned_device_box_empty_destination_and_last_byte_refusal_never_construct_a_value() {
    let bytes = std::mem::size_of::<Device>();
    let bank = Arc::new(Semaphore::new(bytes - 1));
    let error = metadata_vec::with_allocation_hook(
        |_, _| panic!("refused device body allocated"),
        || Destination::<Device>::new(&bank).err().unwrap(),
    );
    assert_eq!(
        crate::output::operation_error_class(error.as_ref()),
        ios_core::ErrorClass::ResourceLimit
    );
    assert_eq!(bank.available_permits(), bytes - 1);
    let bank = Arc::new(Semaphore::new(bytes));
    let empty = Destination::<Device>::new(&bank).unwrap();
    assert_eq!(bank.available_permits(), 0);
    drop(empty); // No initialized Device and no Device Drop against uninitialized data.
    assert_eq!(bank.available_permits(), bytes);
}

#[test]
fn returned_device_box_source_constructor_refusal_refunds_original_empty_output_destination() {
    let bank = Arc::new(Semaphore::new(8192));
    let destination = Destination::<Device>::new(&bank).unwrap();
    let future = async move {
        drop(destination);
        std::future::pending::<()>().await
    };
    let frame_bytes = std::mem::size_of_val(&future);
    assert!(frame_bytes > 0);
    let usage = bank
        .clone()
        .try_acquire_many_owned((bank.available_permits() - frame_bytes + 1) as u32)
        .unwrap();
    let error = metadata_vec::with_allocation_hook(
        |_, _| panic!("refused Source frame allocated"),
        || Dynamic::new(future, &bank).err().unwrap(),
    );
    assert_eq!(
        crate::output::operation_error_class(error.as_ref()),
        ios_core::ErrorClass::ResourceLimit
    );
    assert_eq!(
        bank.available_permits(),
        frame_bytes - 1 + std::mem::size_of::<Device>()
    );
    drop(usage);
    assert_eq!(bank.available_permits(), 8192);
}

#[tokio::test]
async fn returned_device_box_original_source_error_releases_empty_body_without_replacing_error() {
    let bank = Arc::new(Semaphore::new(8192));
    let destination = Destination::<Device>::new(&bank).unwrap();
    let original = anyhow::Error::new(std::io::Error::new(
        std::io::ErrorKind::PermissionDenied,
        "original-output-source-error",
    ));
    let pointer = original.downcast_ref::<std::io::Error>().unwrap() as *const _ as usize;
    let future = Dynamic::new(
        async move {
            drop(destination);
            Err::<Owner<Device>, _>(original)
        },
        &bank,
    )
    .unwrap();
    let error = future.await.err().unwrap();
    assert_eq!(
        error.downcast_ref::<std::io::Error>().unwrap() as *const _ as usize,
        pointer
    );
    assert_eq!(
        error.downcast_ref::<std::io::Error>().unwrap().kind(),
        std::io::ErrorKind::PermissionDenied
    );
    assert_eq!(bank.available_permits(), 8192);
}

#[test]
fn returned_device_box_foreign_bank_cannot_claim_credit_and_zst_needs_no_allocation() {
    let bytes = std::mem::size_of::<usize>();
    let bank = Arc::new(Semaphore::new(bytes));
    let foreign = Arc::new(Semaphore::new(bytes));
    let owner = Destination::new(&bank).unwrap().place(42_usize);
    let error = owner.bytes_in(&foreign).unwrap_err();
    assert_eq!(
        crate::output::operation_error_class(error.as_ref()),
        ios_core::ErrorClass::ResourceLimit
    );
    assert_eq!(owner.bytes_in(&bank).unwrap(), bytes);
    assert_eq!(bank.available_permits(), 0);
    assert_eq!(foreign.available_permits(), bytes);
    drop(owner);
    assert_eq!(bank.available_permits(), bytes);
    let bank = Arc::new(Semaphore::new(0));
    let owner = metadata_vec::with_allocation_hook(
        |_, _| panic!("ZST device body allocated"),
        || Destination::new(&bank).unwrap().place(()),
    );
    assert_eq!(owner.bytes_in(&bank).unwrap(), 0);
    drop(owner);
    assert_eq!(bank.available_permits(), 0);
}
