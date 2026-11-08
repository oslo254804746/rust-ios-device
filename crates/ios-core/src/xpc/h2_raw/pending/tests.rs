use super::*;
use crate::xpc::raw_storage::MutableBytes;
use bytes::Bytes;

fn frame(id: u32, flag: u8, payload: Bytes) -> DataFrame {
    DataFrame {
        stream_id: id,
        flags: flag,
        payload,
    }
}
fn push(queue: &mut PendingFrames, frame: DataFrame) {
    queue.prepare_push().unwrap();
    queue.push_back(frame);
}
fn snapshot(queue: &PendingFrames) -> Vec<(u32, u8, Vec<u8>)> {
    queue
        .iter()
        .map(|frame| (frame.stream_id, frame.flags, frame.payload.to_vec()))
        .collect()
}
fn ring(queue: &PendingFrames) -> &Ring {
    let PendingFrames::Banked(ring) = queue else {
        panic!("expected real banked ring");
    };
    ring
}
fn backing(capacity: usize) -> usize {
    capacity * std::mem::size_of::<Option<DataFrame>>()
}

#[test]
fn mixed_fifo_wrap_rollback_clear_stream_matches_original_deque() {
    let bank = Arc::new(Semaphore::new(1024 * 1024));
    let mut actual = PendingFrames::new(Some(&bank));
    let mut original = PendingFrames::new(None);
    let mut seed = 29u32;
    for step in 0..3000u32 {
        seed = seed.wrapping_mul(1664525).wrapping_add(1013904223);
        match seed % 11 {
            0..=5 => {
                let payload = Bytes::copy_from_slice(&step.to_le_bytes());
                let id = seed % 7 + 1;
                push(&mut actual, frame(id, step as u8, payload.clone()));
                push(&mut original, frame(id, step as u8, payload));
            }
            6..=7 => {
                let a = actual.pop_front();
                let b = original.pop_front();
                assert_eq!(
                    a.as_ref().map(|f| (&f.payload, f.flags)),
                    b.as_ref().map(|f| (&f.payload, f.flags))
                );
                if step % 3 == 0 {
                    if let (Some(a), Some(b)) = (a, b) {
                        actual.push_front(a);
                        original.push_front(b);
                    }
                }
            }
            8..=9 => assert_eq!(
                actual.clear_stream(seed % 7 + 1),
                original.clear_stream(seed % 7 + 1)
            ),
            _ => {
                actual.clear();
                original.clear();
            }
        }
        assert_eq!(snapshot(&actual), snapshot(&original));
        assert_eq!(
            bank.available_permits(),
            1024 * 1024 - backing(ring(&actual).slots.capacity())
        );
    }
    drop(actual);
    assert_eq!(bank.available_permits(), 1024 * 1024);
}

#[test]
fn replacement_prepays_actual_peak_and_one_byte_refusal_keeps_original_slots_and_frames() {
    let quota = backing(12) - 1;
    let bank = Arc::new(Semaphore::new(quota));
    let mut queue = PendingFrames::new(Some(&bank));
    for id in 1..=4 {
        push(&mut queue, frame(id, 0, Bytes::from_static(b"original")));
    }
    let first = queue.pop_front().unwrap();
    queue.push_back(first);
    let pointer = ring(&queue).slots.as_ptr();
    let before = snapshot(&queue);
    let error = queue.prepare_push().unwrap_err();
    #[cfg(feature = "tunnel")]
    assert_eq!(
        crate::classify_error(&error),
        crate::ErrorClass::ResourceLimit
    );
    assert_eq!(error.kind(), std::io::ErrorKind::Other);
    assert_eq!(snapshot(&queue), before);
    assert_eq!(ring(&queue).slots.as_ptr(), pointer);
    assert_eq!(bank.available_permits(), backing(8) - 1);
    bank.add_permits(1);
    queue.prepare_push().unwrap();
    assert_eq!(snapshot(&queue), before);
    assert_eq!(ring(&queue).slots.capacity(), 8);
    assert_eq!(bank.available_permits(), backing(4));
    drop(queue);
    assert_eq!(bank.available_permits(), backing(12));
}

#[test]
fn popped_original_payload_clone_outlives_ring_and_clear_keeps_actual_backing_paid() {
    let quota = backing(4) + 8;
    let bank = Arc::new(Semaphore::new(quota));
    let mut payload = MutableBytes::zeroed(&bank, 8).unwrap();
    payload.bytes.fill(9);
    let mut queue = PendingFrames::new(Some(&bank));
    push(&mut queue, frame(3, 1, payload.freeze()));
    let returned = queue.pop_front().unwrap();
    let pointer = returned.payload.as_ptr();
    let last = returned.payload.slice(2..3);
    queue.push_front(returned);
    assert_eq!(queue.iter().next().unwrap().payload.as_ptr(), pointer);
    assert_eq!(bank.available_permits(), 0);
    queue.clear();
    assert_eq!(
        bank.available_permits(),
        0,
        "slice still owns the full payload, ring keeps backing"
    );
    drop(queue);
    assert_eq!(bank.available_permits(), backing(4));
    assert_eq!(&last[..], &[9]);
    drop(last);
    assert_eq!(bank.available_permits(), quota);
}

#[test]
fn zero_quota_front_rollback_and_wrapped_vacant_slot_reuse_never_allocate() {
    let bank = Arc::new(Semaphore::new(backing(4)));
    let mut queue = PendingFrames::new(Some(&bank));
    for id in 1..=4 {
        push(&mut queue, frame(id, id as u8, Bytes::new()));
    }
    let pointer = ring(&queue).slots.as_ptr();
    for _ in 0..20 {
        let first = queue.pop_front().unwrap();
        queue.push_front(first);
        let first = queue.pop_front().unwrap();
        push(&mut queue, first);
        assert_eq!(ring(&queue).slots.as_ptr(), pointer);
        assert_eq!(bank.available_permits(), 0);
    }
    assert_eq!(
        queue.iter().map(|f| f.stream_id).collect::<Vec<_>>(),
        [1, 2, 3, 4]
    );
}

#[cfg(feature = "tunnel")]
mod wire {
    use super::super::super::*;
    use super::*;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::time::{timeout, Duration};

    fn metered(
        stream: tokio::io::DuplexStream,
        bank: &Arc<Semaphore>,
    ) -> H2Framer<tokio::io::DuplexStream> {
        let mut framer = H2Framer::new(stream);
        framer.configure_bank(Some(bank));
        framer.client_server_open = true;
        framer.outbound_connection_window = 0;
        framer.outbound_stream_windows.prepare_keys(&[1]).unwrap();
        framer.outbound_stream_windows.insert_reserved(1, 0);
        framer
    }
    async fn strict_eof(peer: &mut tokio::io::DuplexStream) {
        let mut tail = [0; 1024];
        loop {
            match timeout(Duration::from_secs(2), peer.read(&mut tail)).await {
                Ok(Ok(0)) => break,
                Ok(Ok(_)) => (),
                other => panic!("strict original peer EOF missing: {other:?}"),
            }
        }
    }

    #[tokio::test]
    async fn actual_blocked_writer_returns_original_business_frames_flags_and_last_payload_owner() {
        let quota = 16 * 1024 * 1024;
        let bank = Arc::new(Semaphore::new(quota));
        let (stream, mut peer) = tokio::io::duplex(4096);
        let mut framer = metered(stream, &bank);
        let task = tokio::spawn(async move {
            for (id, flag, bytes) in [
                (3, 0, &b"a"[..]),
                (1, 0, &b""[..]),
                (3, FLAG_END_STREAM, &b"bb"[..]),
            ] {
                peer.write_all(&build_frame(FRAME_DATA, flag, id, bytes))
                    .await
                    .unwrap();
            }
            for id in [0, 1] {
                peer.write_all(&build_window_update_frame(id, 1))
                    .await
                    .unwrap();
            }
            let mut original_write = [0; 10];
            peer.read_exact(&mut original_write).await.unwrap();
            assert_eq!(original_write, build_data_frame(1, b"x")[..]);
            strict_eof(&mut peer).await;
        });
        timeout(Duration::from_secs(2), framer.write_stream(1, b"x"))
            .await
            .unwrap()
            .unwrap();
        let counters = framer.outbound_stream_windows.backing_bytes()
            + framer.pending_window_updates.backing_bytes()
            + framer.closed_streams.backing_bytes();
        assert_eq!(bank.available_permits(), quota - backing(4) - 3 - counters);
        let mut original = Vec::new();
        for _ in 0..3 {
            original.push(framer.read_next_data_frame().await.unwrap());
        }
        assert_eq!(
            original
                .iter()
                .map(|f| (f.stream_id, f.flags, f.payload.len()))
                .collect::<Vec<_>>(),
            [(3, 0, 1), (1, 0, 0), (3, 1, 2)]
        );
        assert_eq!(&original[0].payload[..], b"a");
        assert_eq!(&original[2].payload[..], b"bb");
        assert_eq!(framer.buffered_bytes, 0);
        assert_eq!(bank.available_permits(), quota - backing(4) - 3 - counters);
        drop(framer);
        task.await.unwrap();
        assert_eq!(bank.available_permits(), quota - 3);
        drop(original);
        assert_eq!(bank.available_permits(), quota);
    }

    #[tokio::test]
    async fn actual_writer_growth_quota_failure_keeps_typed_cause_invalidates_and_pays_old_backing()
    {
        let quota = 16 * 1024 * 1024;
        let bank = Arc::new(Semaphore::new(quota));
        let held = Arc::new(std::sync::Mutex::new(None));
        let hold = Arc::clone(&held);
        let target = Arc::clone(&bank);
        ADMISSION.with(|hook| {
            *hook.borrow_mut() = Some(Box::new(move |bytes| {
                if bytes == backing(8) {
                    let take = target.available_permits() - (bytes - 1);
                    *hold.lock().unwrap() = Some(
                        Arc::clone(&target)
                            .try_acquire_many_owned(take.try_into().unwrap())
                            .unwrap(),
                    );
                }
            }))
        });
        let (stream, mut peer) = tokio::io::duplex(4096);
        let mut framer = metered(stream, &bank);
        let task = tokio::spawn(async move {
            for _ in 0..5 {
                peer.write_all(&build_data_frame(3, b"a")).await.unwrap();
            }
            strict_eof(&mut peer).await;
        });
        let error = timeout(Duration::from_secs(2), framer.write_stream(1, b"x"))
            .await
            .unwrap()
            .unwrap_err();
        ADMISSION.with(|hook| hook.borrow_mut().take());
        let H2Error::Io(original) = &error else {
            panic!("original admission IO lost");
        };
        assert_eq!(
            crate::classify_error(original),
            crate::ErrorClass::ResourceLimit
        );
        assert!(
            held.lock().unwrap().is_some(),
            "did not reach actual queue replacement"
        );
        assert!(!framer.is_usable());
        assert_eq!(framer.buffered_bytes, 0);
        assert!(framer.pending_data_frames.is_empty());
        let held_bytes = held.lock().unwrap().as_ref().unwrap().num_permits();
        assert_eq!(
            bank.available_permits(),
            quota
                - held_bytes
                - backing(4)
                - framer.outbound_stream_windows.backing_bytes()
                - framer.pending_window_updates.backing_bytes()
        );
        assert!(framer.write_stream(1, b"late").await.is_err());
        assert_eq!(
            crate::classify_error(original),
            crate::ErrorClass::ResourceLimit
        );
        drop(framer);
        task.await.unwrap();
        drop(held.lock().unwrap().take());
        assert_eq!(bank.available_permits(), quota);
    }

    #[tokio::test]
    async fn demux_admission_failure_returns_same_frame_to_front_with_zero_quota_and_exact_accounting(
    ) {
        let quota = 16 * 1024 * 1024;
        let bank = Arc::new(Semaphore::new(quota));
        let (stream, mut peer) = tokio::io::duplex(4096);
        let mut framer = metered(stream, &bank);
        for id in [1, 3] {
            let mut payload = MutableBytes::zeroed(&bank, 5).unwrap();
            payload.bytes.fill(id as u8);
            push(
                &mut framer.pending_data_frames,
                frame(id, 0, payload.freeze()),
            );
        }
        framer.buffered_bytes = 10;
        let before = snapshot(&framer.pending_data_frames);
        let pointer = framer
            .pending_data_frames
            .iter()
            .next()
            .unwrap()
            .payload
            .as_ptr();
        let hold = Arc::clone(&bank)
            .try_acquire_many_owned(bank.available_permits().try_into().unwrap())
            .unwrap();
        assert!(framer.drain_pending_data_frames().is_err());
        assert_eq!(snapshot(&framer.pending_data_frames), before);
        assert_eq!(
            framer
                .pending_data_frames
                .iter()
                .next()
                .unwrap()
                .payload
                .as_ptr(),
            pointer
        );
        assert_eq!(framer.buffered_bytes, 10);
        assert_eq!(bank.available_permits(), 0);
        drop(hold);
        framer.drain_pending_data_frames().unwrap();
        assert!(framer.pending_data_frames.is_empty());
        assert_eq!(&framer.take_stream_bytes(1, 5).unwrap()[..], &[1; 5]);
        assert_eq!(&framer.take_stream_bytes(3, 5).unwrap()[..], &[3; 5]);
        assert_eq!(framer.buffered_bytes, 0);
        drop(framer);
        strict_eof(&mut peer).await;
        assert_eq!(bank.available_permits(), quota);
    }

    #[tokio::test]
    async fn queue_refusal_restores_logical_bytes_before_the_original_operation_guard() {
        let outbound = 4 * std::mem::size_of::<(u32, i64)>();
        let quota = backing(4) + 4 + outbound;
        let bank = Arc::new(Semaphore::new(quota));
        let (stream, mut peer) = tokio::io::duplex(128);
        let mut framer = metered(stream, &bank);
        framer.server_client_buf.extend_from_slice(b"old!").unwrap();
        framer.buffered_bytes = 4;
        let owned = MutableBytes::zeroed(&bank, 1).unwrap();
        let (fees, payload) = owned.into_parts();
        let error = framer
            .queue_incoming_data_frame(RawFrame {
                frame_type: FRAME_DATA,
                flags: 0,
                stream_id: 1,
                payload,
                raw_fees: Some(fees),
            })
            .await
            .unwrap_err();
        let H2Error::Io(original) = error else {
            panic!("original quota IO lost");
        };
        assert_eq!(
            crate::classify_error(&original),
            crate::ErrorClass::ResourceLimit
        );
        assert_eq!(
            framer.buffered_bytes, 4,
            "new payload logical reservation not rolled back"
        );
        assert_eq!(&framer.server_client_buf[..], b"old!");
        assert!(framer.pending_data_frames.is_empty());
        assert_eq!(bank.available_permits(), quota - 4 - outbound);
        drop(framer);
        strict_eof(&mut peer).await;
        assert_eq!(bank.available_permits(), quota);
    }

    #[tokio::test]
    async fn original_invalid_stream_protocol_precedes_queue_admission_at_zero_quota() {
        let bank = Arc::new(Semaphore::new(0));
        let (stream, mut peer) = tokio::io::duplex(128);
        let mut framer = H2Framer::new(stream);
        framer.raw_bank = Some(Arc::clone(&bank));
        framer.pending_data_frames = PendingFrames::new(Some(&bank));
        let error = framer
            .queue_incoming_data_frame(RawFrame {
                frame_type: FRAME_DATA,
                flags: 0,
                stream_id: 0,
                payload: Vec::new(),
                raw_fees: None,
            })
            .await
            .unwrap_err();
        assert!(matches!(error, H2Error::Protocol(_)));
        assert_eq!(bank.available_permits(), 0);
        drop(framer);
        strict_eof(&mut peer).await;
    }
}
