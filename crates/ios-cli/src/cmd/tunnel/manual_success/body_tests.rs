use super::super::metadata_vec;
use super::*;
use std::sync::Mutex;

#[test]
fn preflight_body_original_empty_box_is_paid_first_and_body_ends_before_independent_batch() {
    let lifecycle = Lifecycle::new();
    let total = Preflight::bytes();
    let bank = Arc::new(Semaphore::new(total));
    let seen = Arc::new(Mutex::new(Vec::new()));
    let captured = seen.clone();
    let charged = bank.clone();
    let plan = metadata_vec::with_allocation_hook(
        move |layout, pointer| {
            let mut seen = captured.lock().unwrap();
            if seen.is_empty() {
                assert_eq!(layout, std::alloc::Layout::new::<Preflight>());
                assert_eq!(charged.available_permits(), total - layout.size());
            }
            seen.push((layout, pointer));
        },
        || Preflight::new(&bank, &lifecycle).unwrap(),
    );
    assert_eq!(
        &*plan as *const Preflight as usize,
        seen.lock().unwrap()[0].1
    );
    assert_eq!(
        plan.bytes_in(&bank).unwrap(),
        std::mem::size_of::<Preflight>()
    );
    assert_eq!(bank.available_permits(), 0);
    let value = metadata_vec::with_allocation_hook(
        |_, _| panic!("Preflight move allocated another body"),
        || plan.into_inner(),
    );
    assert_eq!(value.selected(), 0);
    assert_eq!(bank.available_permits(), std::mem::size_of::<Preflight>());
    drop(value);
    assert_eq!(bank.available_permits(), total);
}

#[test]
fn preflight_body_last_byte_refusal_and_later_batch_refusal_drop_original_empty_destination() {
    let lifecycle = Lifecycle::new();
    let body = std::mem::size_of::<Preflight>();
    let bank = Arc::new(Semaphore::new(body - 1));
    let error = metadata_vec::with_allocation_hook(
        |_, _| panic!("refused Preflight body allocated"),
        || Preflight::new(&bank, &lifecycle).err().unwrap(),
    );
    assert_eq!(
        crate::output::operation_error_class(error.as_ref()),
        ios_core::ErrorClass::ResourceLimit
    );
    assert_eq!(bank.available_permits(), body - 1);
    let bank = Arc::new(Semaphore::new(body));
    let seen = Arc::new(Mutex::new(Vec::new()));
    let captured = seen.clone();
    let error = metadata_vec::with_allocation_hook(
        move |layout, _| captured.lock().unwrap().push(layout),
        || Preflight::new(&bank, &lifecycle).err().unwrap(),
    );
    assert_eq!(
        crate::output::operation_error_class(error.as_ref()),
        ios_core::ErrorClass::ResourceLimit
    );
    assert_eq!(
        seen.lock().unwrap().as_slice(),
        &[std::alloc::Layout::new::<Preflight>()]
    );
    assert_eq!(bank.available_permits(), body);
}

#[tokio::test]
async fn preflight_body_cancellation_drops_original_box_and_all_children_without_source_or_task() {
    let lifecycle = Lifecycle::new();
    let total = Preflight::bytes();
    let bank = Arc::new(Semaphore::new(total));
    let plan = Preflight::new(&bank, &lifecycle).unwrap();
    let mut work = Box::pin(async move {
        // This future owns the original paid Box while it is pending. Its
        // concrete fixture frame is independent test observation storage.
        std::future::pending::<()>().await;
        drop(plan);
    });
    assert!(futures_util::poll!(&mut work).is_pending());
    assert_eq!(bank.available_permits(), 0);
    drop(work);
    assert_eq!(bank.available_permits(), total);
}
