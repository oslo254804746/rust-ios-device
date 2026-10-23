use super::super::Ticket;
use super::*;

fn admitted(data: (String, Route, Ticket, Option<u32>)) -> Candidate {
    let (udid, route, ticket, native_id) = data;
    let bank = Arc::new(Semaphore::new(64 * 1024));
    let input = super::super::candidate_input::Input::owned(udid, route, &bank).unwrap();
    let node = super::super::failure_table::Node::input(input.clone(), native_id, &bank).unwrap();
    Candidate {
        input,
        ticket: TicketOwner::new(&ticket, &bank).unwrap(),
        native_id,
        node,
    }
}

#[tokio::test]
async fn candidate_index_bytes_borrows_original_route_buffer_and_preserves_native_identity() {
    let lifecycle = super::super::Lifecycle::new();
    let ticket = lifecycle.ticket("synthetic-index").await.unwrap();
    let mut host = String::with_capacity(8192);
    host.push_str("synthetic-index-host");
    let buffer = host.as_ptr();
    let candidates = vec![admitted((
        "a".into(),
        Route::Mobdev2(host),
        ticket,
        Some(7),
    ))];
    let bytes = std::mem::size_of::<usize>();
    let bank = Arc::new(Semaphore::new(bytes));
    let index = Index::new(&candidates, &bank).unwrap();
    let (route, native_id) = index.get("a").unwrap();
    assert!(std::ptr::eq(route, &candidates[0].input.route));
    let Route::Mobdev2(host) = route else {
        panic!("route family changed")
    };
    assert_eq!(host.as_ptr(), buffer);
    assert_eq!(host.capacity(), 8192);
    assert_eq!(native_id, Some(7));
    assert_eq!(bank.available_permits(), 0);
    assert!(index.get("unknown").is_none());
    drop(index);
    assert_eq!(bank.available_permits(), bytes);
    assert_eq!(candidates[0].input.udid, "a");
}

#[tokio::test]
async fn candidate_index_bytes_keeps_last_duplicate_and_empty_nul_unicode_keys() {
    let lifecycle = super::super::Lifecycle::new();
    let ticket = lifecycle.ticket("synthetic-index").await.unwrap();
    let candidates = vec![
        ("z".into(), Route::Wifi(None), ticket.clone(), None),
        ("".into(), Route::Usbmux, ticket.clone(), Some(1)),
        ("é".into(), Route::Usb(None), ticket.clone(), Some(2)),
        (
            "z".into(),
            Route::Mobdev2("last".into()),
            ticket.clone(),
            Some(3),
        ),
        ("\0".into(), Route::Wifi(Some("nul".into())), ticket, None),
    ]
    .into_iter()
    .map(admitted)
    .collect::<Vec<_>>();
    let bank = Arc::new(Semaphore::new(4096));
    let index = Index::new(&candidates, &bank).unwrap();
    for (key, position) in [("z", 3), ("", 1), ("é", 2), ("\0", 4)] {
        let (route, native_id) = index.get(key).unwrap();
        assert!(std::ptr::eq(route, &candidates[position].input.route));
        assert_eq!(native_id, candidates[position].native_id);
    }
    assert!(index.get("zz").is_none());
    assert_eq!(
        bank.available_permits(),
        4096 - candidates.len() * std::mem::size_of::<usize>()
    );
    drop(index);
    assert_eq!(bank.available_permits(), 4096);
}

#[tokio::test]
async fn candidate_index_bytes_one_byte_refusal_preserves_original_input_and_refunds() {
    let lifecycle = super::super::Lifecycle::new();
    let ticket = lifecycle.ticket("synthetic-index").await.unwrap();
    let candidates = vec![admitted((
        "a".into(),
        Route::Wifi(Some("first".into())),
        ticket,
        None,
    ))];
    let bytes = std::mem::size_of::<usize>() - 1;
    let bank = Arc::new(Semaphore::new(bytes));
    let error = Index::new(&candidates, &bank)
        .err()
        .expect("unadmitted candidate index accepted");
    assert_eq!(
        crate::output::operation_error_class(error.as_ref()),
        ios_core::ErrorClass::ResourceLimit
    );
    assert_eq!(bank.available_permits(), bytes);
    assert_eq!(candidates[0].input.udid, "a");
    assert!(matches!(&candidates[0].input.route, Route::Wifi(Some(host)) if host == "first"));
    assert!(candidates[0].ticket.is_current());
}

#[test]
fn candidate_index_bytes_empty_view_accepts_zero_bank() {
    let bank = Arc::new(Semaphore::new(0));
    let index = Index::new(&[], &bank).unwrap();
    assert!(index.get("").is_none());
    drop(index);
    assert_eq!(bank.available_permits(), 0);
}
