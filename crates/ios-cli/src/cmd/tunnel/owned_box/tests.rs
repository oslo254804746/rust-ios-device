use super::super::{future_storage::Dynamic, metadata_vec};
use super::*;
use std::sync::atomic::{AtomicUsize, Ordering};

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
