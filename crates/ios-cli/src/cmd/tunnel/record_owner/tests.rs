use super::*;
use futures_util::StreamExt;
use std::sync::atomic::{AtomicUsize, Ordering};

fn fixture() -> TunnelRecord {
    TunnelRecord {
        udid: "synthetic-雪🦀".into(),
        interface: "usbmux-synthetic-雪🦀".into(),
        connection_type: "USB".into(),
        mode: "userspace".into(),
        address: "fd00::1".into(),
        rsd_port: 12345,
        tunnel_address: "fd00::1".into(),
        tunnel_port: 12345,
        userspace_tun: true,
        userspace_tun_host: Some("127.0.0.1".into()),
        userspace_tun_port: Some(23456),
        userspace_port: Some(23456),
    }
}

fn text_bytes(record: &TunnelRecord) -> usize {
    [
        &record.udid,
        &record.interface,
        &record.connection_type,
        &record.mode,
        &record.address,
        &record.tunnel_address,
    ]
    .iter()
    .map(|s| s.len())
    .sum::<usize>()
        + record.userspace_tun_host.as_ref().map_or(0, |s| s.len())
}

#[test]
fn record_copy_one_byte_refusal_precedes_every_text_allocation() {
    let raw = fixture();
    let bytes = text_bytes(&raw);
    let bank = Arc::new(Semaphore::new(bytes - 1));
    let calls = Arc::new(AtomicUsize::new(0));
    let observed = calls.clone();
    let _hook = on_text_allocated(move |_, _| {
        observed.fetch_add(1, Ordering::SeqCst);
    });
    let error = PaidRecord::from_fixture(&raw, &bank).unwrap_err();
    assert!(error.is::<CopyAdmission>());
    assert_eq!(
        crate::output::operation_error_class(error.as_ref()),
        ios_core::ErrorClass::ResourceLimit
    );
    assert_eq!(calls.load(Ordering::SeqCst), 0);
    assert_eq!(bank.available_permits(), bytes - 1);
}

#[test]
fn record_copy_actual_layouts_are_all_paid_before_first_copy_and_schema_is_unchanged() {
    let raw = fixture();
    let expected = serde_json::to_vec(&raw).unwrap();
    let bytes = text_bytes(&raw);
    let bank = Arc::new(Semaphore::new(bytes));
    let captured = bank.clone();
    let layouts = Arc::new(std::sync::Mutex::new(Vec::new()));
    let observed = layouts.clone();
    let hook = on_text_allocated(move |layout, pointer| {
        assert_eq!(captured.available_permits(), 0);
        observed.lock().unwrap().push((pointer, layout.size()));
    });
    let paid = PaidRecord::from_fixture(&raw, &bank).unwrap();
    drop(hook);
    assert_eq!(paid.data, raw);
    for text in [
        &paid.udid,
        &paid.interface,
        &paid.connection_type,
        &paid.mode,
        &paid.address,
        &paid.tunnel_address,
        paid.userspace_tun_host.as_ref().unwrap(),
    ] {
        assert_eq!(text.capacity(), text.len());
        assert!(layouts
            .lock()
            .unwrap()
            .contains(&(text.as_ptr() as usize, text.capacity())));
    }
    assert_ne!(paid.address.as_ptr(), paid.tunnel_address.as_ptr());
    assert_ne!(paid.udid.as_ptr(), raw.udid.as_ptr());
    drop(raw);
    assert_eq!(serde_json::to_vec(&paid).unwrap(), expected);
    assert_eq!(bank.available_permits(), 0);
    drop(paid);
    assert_eq!(bank.available_permits(), bytes);
}

#[test]
fn record_partial_unwind_releases_all_text_and_aggregate_grant() {
    let raw = fixture();
    let original = raw.udid.as_ptr();
    let bytes = text_bytes(&raw);
    let bank = Arc::new(Semaphore::new(bytes));
    let attempts = Arc::new(AtomicUsize::new(0));
    let captured = attempts.clone();
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let _hook = on_text_allocated(move |_, _| {
            if captured.fetch_add(1, Ordering::SeqCst) == 2 {
                panic!("third allocation fixture unwind");
            }
        });
        PaidRecord::from_fixture(&raw, &bank).unwrap()
    }));
    assert!(result.is_err());
    assert_eq!(attempts.load(Ordering::SeqCst), 3);
    assert_eq!(bank.available_permits(), bytes);
    assert_eq!(raw.udid.as_ptr(), original);
    drop(PaidRecord::from_fixture(&raw, &bank).unwrap());
    assert_eq!(bank.available_permits(), bytes);
}

#[tokio::test]
async fn record_borrowed_start_reply_and_last_wire_slice_keep_independent_fees() {
    let raw = fixture();
    let bytes = text_bytes(&raw);
    let bank = Arc::new(Semaphore::new(16384));
    let paid = PaidRecord::from_fixture(&raw, &bank).unwrap();
    let reply = paid.start_reply();
    assert_eq!(reply.interface.as_ptr(), paid.interface.as_ptr());
    assert_eq!(reply.address.as_ptr(), paid.address.as_ptr());
    let expected = serde_json::to_vec(&reply).unwrap();
    let replies = super::super::response_json::Replies::new(&bank).unwrap();
    let response = replies.json(&reply).unwrap();
    assert_eq!(
        bank.available_permits(),
        16384 - bytes - 2048 - expected.len()
    );
    let mut body = response.into_body().into_data_stream();
    let frame = body.next().await.unwrap().unwrap();
    let last = frame.slice(1..2);
    assert_eq!(&frame[..], expected);
    drop(paid);
    drop(frame);
    drop(body);
    drop(replies);
    assert_eq!(bank.available_permits(), 16384 - expected.len());
    drop(last);
    assert_eq!(bank.available_permits(), 16384);
}

#[test]
fn record_empty_and_absent_text_preserve_options_without_data_allocation() {
    let raw = TunnelRecord {
        udid: String::new(),
        interface: String::new(),
        connection_type: String::new(),
        mode: String::new(),
        address: String::new(),
        rsd_port: 0,
        tunnel_address: String::new(),
        tunnel_port: 0,
        userspace_tun: false,
        userspace_tun_host: None,
        userspace_tun_port: None,
        userspace_port: None,
    };
    let bank = Arc::new(Semaphore::new(0));
    let _hook = on_text_allocated(|_, _| panic!("empty text allocated"));
    let paid = PaidRecord::from_fixture(&raw, &bank).unwrap();
    assert_eq!(paid.data, raw);
    assert_eq!(
        serde_json::to_vec(&paid).unwrap(),
        serde_json::to_vec(&raw).unwrap()
    );
    assert_eq!(bank.available_permits(), 0);
}
