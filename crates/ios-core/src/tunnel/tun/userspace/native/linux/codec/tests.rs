use super::*;

#[test]
fn dump_reply_header_retains_multipart_interruption_and_done_flags_for_control_owner() {
    let request = Request::new(18, 0x301, 41, 73, &[]).unwrap();
    let message = Messages::new(request.bytes()).next().unwrap().unwrap();
    assert_eq!(message.flags & DUMP, 0x300);
    let reply = Request::new(DONE, 0x12, 41, 0, &[]).unwrap();
    let message = Messages::new(reply.bytes()).next().unwrap().unwrap();
    assert_eq!(message.kind, 3);
    assert_eq!(message.flags & MULTI, 2);
    assert_eq!(message.flags & DUMP_INTERRUPTED, 0x10);
}

fn request() -> Request {
    let mut request = Request::new(
        20,
        ACK | CREATE_EXCLUSIVE,
        41,
        73,
        &[10, 64, 0, 0, 7, 0, 0, 0],
    )
    .unwrap();
    request.attribute(1, &[0xfd; 16]).unwrap();
    request.attribute(2, &[0xfd; 16]).unwrap();
    request.attribute(8, &0x200u32.to_ne_bytes()).unwrap();
    request
}

fn acknowledgement(request: &Request, status: i32, capped: bool) -> Vec<u8> {
    let mut payload = status.to_ne_bytes().to_vec();
    payload.extend_from_slice(if capped {
        &request.bytes()[..HEADER]
    } else {
        request.bytes()
    });
    payload
}

#[test]
fn exact_concrete_address_request_and_borrowed_attributes() {
    let request = request();
    assert_eq!(request.bytes().len(), 72);
    let message = Messages::new(request.bytes()).next().unwrap().unwrap();
    assert_eq!((message.kind, message.sequence, message.port), (20, 41, 73));
    let attributes = Attributes::new(&message.payload[8..])
        .collect::<io::Result<Vec<_>>>()
        .unwrap();
    assert_eq!(attributes.len(), 3);
    assert_eq!(attributes[0].1.as_ptr(), request.bytes()[28..].as_ptr());
    assert_eq!(attributes[2], (8, &0x200u32.to_ne_bytes()[..]));
}

#[test]
fn capped_ack_requires_exact_request_header_and_uncapped_requires_key() {
    let request = request();
    request
        .ack(&acknowledgement(&request, 0, true), CAPPED)
        .unwrap();
    request
        .ack(&acknowledgement(&request, 0, false), 0)
        .unwrap();
    for offset in [4, 8, 12, 16] {
        let mut payload = acknowledgement(&request, 0, true);
        payload[offset] ^= 1;
        assert_eq!(
            request.ack(&payload, CAPPED).unwrap_err().kind(),
            io::ErrorKind::InvalidData
        );
    }
    let mut changed_key = acknowledgement(&request, 0, false);
    changed_key[4 + HEADER + 12] ^= 1;
    assert_eq!(
        request.ack(&changed_key, 0).unwrap_err().kind(),
        io::ErrorKind::InvalidData
    );
    assert!(request.ack(&acknowledgement(&request, 0, true), 0).is_err());
    let mut trailing = acknowledgement(&request, 0, true);
    trailing.extend_from_slice(&[4, 0, 1, 0]);
    assert!(request.ack(&trailing, CAPPED).is_err());
    request.ack(&trailing, CAPPED | ACK_TLVS).unwrap();
}

#[test]
fn ack_preserves_native_errno_and_refuses_invalid_status_or_truncation() {
    let request = request();
    let error = request
        .ack(&acknowledgement(&request, -17, true), CAPPED)
        .unwrap_err();
    assert_eq!(error.raw_os_error(), Some(17));
    let mut malformed_extension = acknowledgement(&request, -17, true);
    malformed_extension.extend_from_slice(&[0, 0, 1, 0]);
    assert_eq!(
        request
            .ack(&malformed_extension, CAPPED | ACK_TLVS)
            .unwrap_err()
            .raw_os_error(),
        Some(17)
    );
    for status in [1, i32::MIN] {
        assert_eq!(
            request
                .ack(&acknowledgement(&request, status, true), CAPPED)
                .unwrap_err()
                .kind(),
            io::ErrorKind::InvalidData
        );
    }
    for length in 0..4 + HEADER {
        assert!(request
            .ack(&acknowledgement(&request, 0, true)[..length], CAPPED)
            .is_err());
    }
}

#[test]
fn multipart_is_borrowed_and_malformed_lengths_terminate_iteration() {
    let mut bytes = request().bytes().to_vec();
    bytes.extend_from_slice(request().bytes());
    let messages = Messages::new(&bytes)
        .collect::<io::Result<Vec<_>>>()
        .unwrap();
    assert_eq!(messages.len(), 2);
    assert_eq!(messages[1].payload.as_ptr(), bytes[72 + HEADER..].as_ptr());
    for invalid_length in [0, 15, 145, u32::MAX] {
        let mut corrupt = bytes.clone();
        corrupt[..4].copy_from_slice(&invalid_length.to_ne_bytes());
        let mut messages = Messages::new(&corrupt);
        assert!(messages.next().unwrap().is_err());
        assert!(messages.next().is_none());
    }
    for bytes in [&[0, 0, 1, 0][..], &[3, 0, 1, 0], &[12, 0, 1, 0]] {
        assert!(Attributes::new(bytes).next().unwrap().is_err());
    }
}

#[test]
fn pending_request_comparison_allows_new_sequence_but_rejects_another_key() {
    let old = request();
    let mut same = old;
    same.bytes[8..12].copy_from_slice(&42u32.to_ne_bytes());
    assert!(old.same_operation(&same));
    same.bytes[30] ^= 1;
    assert!(!old.same_operation(&same));
}

#[test]
fn address_cacheinfo_requires_complete_consistent_lifetimes() {
    let mut bytes = [0; 16];
    bytes[..4].copy_from_slice(&u32::MAX.to_ne_bytes());
    bytes[4..8].copy_from_slice(&u32::MAX.to_ne_bytes());
    assert_eq!(
        AddressLifetimes::parse(&bytes).unwrap(),
        AddressLifetimes::INFINITE
    );
    bytes[8..12].copy_from_slice(&123u32.to_ne_bytes());
    bytes[12..].copy_from_slice(&456u32.to_ne_bytes());
    assert_eq!(
        AddressLifetimes::parse(&bytes).unwrap(),
        AddressLifetimes::INFINITE
    );
    bytes[4..8].copy_from_slice(&60u32.to_ne_bytes());
    assert!(AddressLifetimes::parse(&bytes).is_err());
    for length in 0..16 {
        assert!(AddressLifetimes::parse(&bytes[..length]).is_err());
    }
}

#[test]
fn response_growth_retains_the_exact_unread_datagram_without_new_backing() {
    let request = request();
    let queued = acknowledgement(&request, 0, false);
    let queue_pointer = queued.as_ptr();
    let queue_capacity = queued.capacity();
    let mut state = ResponseState::default();
    let empty_paid_backing: [u8; 0] = [];
    let pending = state
        .check(queued.len(), empty_paid_backing.len(), 41, ERROR)
        .unwrap_err();
    assert_eq!(state.pending(), Some(pending));
    assert_eq!(
        pending,
        PendingResponse {
            required: queued.len(),
            sequence: 41,
            message_kind: ERROR
        }
    );
    assert_eq!(
        (queued.as_ptr(), queued.capacity()),
        (queue_pointer, queue_capacity)
    );
    // An observation with sufficient backing does not claim the native read
    // happened. Only the actual consumer's explicit consumed boundary clears.
    state.check(queued.len(), queued.len(), 41, ERROR).unwrap();
    assert_eq!(state.pending(), Some(pending));
    request.ack(&queued, 0).unwrap();
    state.consumed();
    assert_eq!(state.pending(), None);
}

#[test]
fn overrun_consumption_advances_the_same_queue_and_keeps_original_loss_on_consume_failure() {
    use std::collections::VecDeque;
    let request = request();
    let mut queue = VecDeque::from([OVERRUN, ERROR]);
    let original = io::Error::from_raw_os_error(105);
    let (original, cleanup): (_, Option<io::Error>) = consume_loss(original, || {
        assert_eq!(queue.pop_front(), Some(OVERRUN));
        Ok(())
    });
    assert_eq!(original.raw_os_error(), Some(105));
    assert!(cleanup.is_none());
    assert_eq!(queue.front(), Some(&ERROR));
    request
        .ack(&acknowledgement(&request, 0, false), 0)
        .unwrap();

    #[derive(Debug)]
    struct OriginalLoss;
    impl std::fmt::Display for OriginalLoss {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            f.write_str("original loss")
        }
    }
    impl std::error::Error for OriginalLoss {}
    let original = io::Error::other(OriginalLoss);
    let deep_pointer = original
        .get_ref()
        .unwrap()
        .downcast_ref::<OriginalLoss>()
        .unwrap() as *const OriginalLoss;
    let cleanup = io::Error::from_raw_os_error(9);
    let (original, cleanup) = consume_loss(original, || Err(cleanup));
    assert_eq!(
        original
            .get_ref()
            .unwrap()
            .downcast_ref::<OriginalLoss>()
            .unwrap() as *const OriginalLoss,
        deep_pointer
    );
    assert_eq!(cleanup.unwrap().raw_os_error(), Some(9));
}
