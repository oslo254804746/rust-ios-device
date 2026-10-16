use super::*;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Mutex;

use super::super::metadata_vec::with_allocation_hook;

#[test]
fn upstream_registry_exact_text_and_array_layouts_precede_allocation_and_keep_sorted_keys() {
    let bank = Arc::new(Semaphore::new(4096));
    let observed = Arc::new(Mutex::new(Vec::new()));
    let captured = observed.clone();
    let charged = bank.clone();
    let mut rows = Registry::new(&bank);
    with_allocation_hook(
        move |layout, _| {
            let mut seen = captured.lock().unwrap();
            let previous: usize = seen.iter().map(|(bytes, _)| bytes).sum();
            assert_eq!(charged.available_permits(), 4096 - previous - layout.size());
            seen.push((layout.size(), charged.available_permits()));
        },
        || {
            assert!(rows.insert("z").unwrap());
            assert!(rows.insert("a").unwrap());
        },
    );
    let width = std::mem::size_of::<Text>();
    assert_eq!(
        *observed.lock().unwrap(),
        [
            (1, 4095),
            (2 * width, 4095 - 2 * width),
            (1, 4094 - 2 * width)
        ]
    );
    assert_eq!(serde_json::to_string(&rows).unwrap(), r#"["a","z"]"#);
    assert!(rows.contains("z") && !rows.contains("missing"));
    let pointer = rows.iter().next().unwrap().as_str().as_ptr();
    let removed = rows.remove("a").unwrap();
    assert_eq!(removed.as_str().as_ptr(), pointer);
    assert_eq!(bank.available_permits(), 4094 - 2 * width);
    drop(rows);
    assert_eq!(bank.available_permits(), 4095);
    assert_eq!(removed.as_str(), "a");
    drop(removed);
    assert_eq!(bank.available_permits(), 4096);
}

#[test]
fn upstream_registry_text_and_growth_one_byte_refusal_preserve_original_storage_and_refund() {
    let bank = Arc::new(Semaphore::new(4096));
    let mut rows = Registry::new(&bank);
    rows.insert("original-z").unwrap();
    rows.insert("original-b").unwrap();
    let before = bank.available_permits();
    let backing = rows.rows.as_slice().as_ptr();
    let text = rows.iter().next().unwrap().as_str().as_ptr();
    let candidate = "candidate-a";
    let new_backing = 4 * std::mem::size_of::<Text>();
    for (remaining, allocations) in [
        (candidate.len() - 1, 0),
        (candidate.len() + new_backing - 1, 1),
    ] {
        let held = bank
            .clone()
            .try_acquire_many_owned((before - remaining) as u32)
            .unwrap();
        let calls = Arc::new(AtomicUsize::new(0));
        let captured = calls.clone();
        let error = with_allocation_hook(
            move |layout, _| {
                assert_eq!(layout.size(), candidate.len());
                captured.fetch_add(1, Ordering::SeqCst);
            },
            || rows.insert(candidate).unwrap_err(),
        );
        assert_eq!(
            crate::output::operation_error_class(error.as_ref()),
            ios_core::ErrorClass::ResourceLimit
        );
        assert_eq!(calls.load(Ordering::SeqCst), allocations);
        assert_eq!(rows.rows.as_slice().as_ptr(), backing);
        assert_eq!(rows.iter().next().unwrap().as_str().as_ptr(), text);
        assert_eq!(
            serde_json::to_string(&rows).unwrap(),
            r#"["original-b","original-z"]"#
        );
        assert_eq!(bank.available_permits(), remaining);
        drop(held);
        assert_eq!(bank.available_permits(), before);
    }
    drop(rows);
    assert_eq!(bank.available_permits(), 4096);
}

#[test]
fn upstream_registry_duplicate_at_zero_quota_and_full_limit_allocate_nothing() {
    let bank = Arc::new(Semaphore::new(16384));
    let mut rows = Registry::new(&bank);
    for id in (0..MAX_UPSTREAMS).rev() {
        rows.insert(format!("root-{id:02}")).unwrap();
    }
    let held = bank
        .clone()
        .try_acquire_many_owned(bank.available_permits() as u32)
        .unwrap();
    with_allocation_hook(
        |_, _| panic!("duplicate or count refusal must allocate nothing"),
        || {
            assert!(!rows.insert("root-00").unwrap());
            assert!(rows.insert("another-root").is_err());
            assert!(rows.remove("missing").is_none());
            assert_eq!(rows.len(), MAX_UPSTREAMS);
        },
    );
    let removed = rows.remove("root-00").unwrap();
    assert_eq!(bank.available_permits(), 0);
    drop(removed);
    assert_eq!(bank.available_permits(), "root-00".len());
    assert!(
        rows.insert("root-00").unwrap(),
        "reuse paid backing after delete"
    );
    assert_eq!(bank.available_permits(), 0);
    drop(held);
    drop(rows);
    assert_eq!(bank.available_permits(), 16384);
}

#[test]
fn upstream_registry_partial_initializer_refusal_releases_actual_prior_keys_and_backing() {
    let urls = [String::from("first"), String::from("second")];
    let limit = 2 * std::mem::size_of::<Text>() + urls[0].len() + urls[1].len() - 1;
    let bank = Arc::new(Semaphore::new(limit));
    let calls = Arc::new(AtomicUsize::new(0));
    let captured = calls.clone();
    let error = with_allocation_hook(
        move |_, _| {
            captured.fetch_add(1, Ordering::SeqCst);
        },
        || Registry::from_urls(&bank, urls.iter()).err().unwrap(),
    );
    assert_eq!(calls.load(Ordering::SeqCst), 2);
    assert_eq!(
        crate::output::operation_error_class(error.as_ref()),
        ios_core::ErrorClass::ResourceLimit
    );
    assert_eq!(bank.available_permits(), limit);
}
