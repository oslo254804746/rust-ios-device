use super::*;

#[test]
fn membership_trace_matches_original_set_and_replacement_reuses_paid_slot_at_zero_quota() {
    let quota = 1024 * 1024;
    let bank = Arc::new(Semaphore::new(quota));
    let mut actual = Membership::new(Some(&bank));
    let mut original = HashSet::new();
    let mut seed = 11u32;
    for _ in 0..3000 {
        seed = seed.wrapping_mul(1664525).wrapping_add(1013904223);
        let id = seed % 41;
        if seed % 3 == 0 {
            assert_eq!(actual.remove(&id), original.remove(&id));
        } else {
            actual.prepare(id).unwrap();
            assert_eq!(actual.insert(id), original.insert(id));
        }
        assert_eq!(actual.len(), original.len());
        assert!(actual.iter().all(|id| original.contains(id)));
        assert_eq!(bank.available_permits(), quota - actual.backing_bytes());
    }
    actual.clear();
    assert_eq!(actual.len(), 0);
    drop(actual);
    assert_eq!(bank.available_permits(), quota);
    let bank = Arc::new(Semaphore::new(16));
    let mut set = Membership::new(Some(&bank));
    for id in 0..4 {
        set.prepare(id).unwrap();
        set.insert(id);
    }
    set.prepare_replacing(5, Some(0)).unwrap();
    assert!(set.remove(&0));
    set.insert(5);
    assert_eq!(bank.available_permits(), 0);
    assert_eq!(set.len(), 4);
}

#[test]
fn arrival_growth_rejection_keeps_original_order_and_refunds_only_destroyed_backing() {
    let bank = Arc::new(Semaphore::new(4 * 12 - 1));
    let mut order = Arrival::new(Some(&bank));
    for id in 0..4 {
        order.prepare_push(false).unwrap();
        order.push_back(id);
    }
    assert!(order.prepare_push(false).is_err());
    assert_eq!(order.front(), Some(&0));
    assert_eq!(order.len(), 4);
    assert_eq!(bank.available_permits(), 4 * 8 - 1);
    bank.add_permits(1);
    order.prepare_push(false).unwrap();
    assert_eq!(order.backing_bytes(), 4 * 8);
    assert_eq!(bank.available_permits(), 4 * 4);
    order.retain(|id| *id != 1);
    assert_eq!(order.pop_front(), Some(0));
    assert_eq!(order.pop_front(), Some(2));
    order.clear();
    assert_eq!(bank.available_permits(), 4 * 4);
    drop(order);
    assert_eq!(bank.available_permits(), 4 * 12);
}

#[test]
fn full_arrival_eviction_at_zero_quota_preserves_original_oldest_order_without_growth() {
    let bank = Arc::new(Semaphore::new(32 * 4 + 16 * 4));
    let mut order = Arrival::new(Some(&bank));
    for id in 0..32 {
        order.prepare_push(false).unwrap();
        order.push_back(id);
    }
    let held = Arc::clone(&bank)
        .try_acquire_many_owned(bank.available_permits().try_into().unwrap())
        .unwrap();
    for id in 32..200 {
        order.prepare_push(true).unwrap();
        assert_eq!(order.pop_front(), Some(id - 32));
        order.push_back(id);
    }
    assert_eq!(order.front(), Some(&168));
    assert_eq!(order.len(), 32);
    assert_eq!(bank.available_permits(), 0);
    drop(held);
    drop(order);
    assert_eq!(bank.available_permits(), 32 * 4 + 16 * 4);
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
        framer
    }
    fn held_all(bank: &Arc<Semaphore>) -> OwnedSemaphorePermit {
        Arc::clone(bank)
            .try_acquire_many_owned(bank.available_permits().try_into().unwrap())
            .unwrap()
    }
    async fn eof(peer: &mut tokio::io::DuplexStream) {
        let mut byte = [0];
        assert!(matches!(
            timeout(Duration::from_secs(2), peer.read(&mut byte)).await,
            Ok(Ok(0))
        ));
    }
    fn state_bytes(framer: &H2Framer<tokio::io::DuplexStream>) -> usize {
        framer.stream_bufs.backing_bytes()
            + framer.reset_streams.backing_bytes()
            + framer.closed_streams.backing_bytes()
            + framer.peer_open_streams.backing_bytes()
            + framer.locally_open_streams.backing_bytes()
            + framer.locally_finished_streams.backing_bytes()
            + framer.closed_unknown_order.backing_bytes()
            + framer.reset_unknown_order.backing_bytes()
            + framer.outbound_stream_windows.backing_bytes()
            + framer.pending_window_updates.backing_bytes()
    }

    #[tokio::test]
    async fn original_stream_buffer_slice_survives_record_removal_and_framer_drop() {
        let quota = 1024 * 1024;
        let bank = Arc::new(Semaphore::new(quota));
        let (stream, mut peer) = tokio::io::duplex(4096);
        let mut framer = metered(stream, &bank);
        framer.append_stream_data(100, b"abcdefgh").unwrap();
        let original = framer.take_stream_bytes(100, 8).unwrap();
        let pointer = original.as_ptr();
        let last = original.slice(2..3);
        drop(original);
        assert_eq!(last.as_ptr(), pointer.wrapping_add(2));
        assert!(!framer.stream_bufs.contains_key(&100));
        assert_eq!(bank.available_permits(), quota - state_bytes(&framer) - 8);
        drop(framer);
        eof(&mut peer).await;
        assert_eq!(bank.available_permits(), quota - 8);
        assert_eq!(&last[..], b"c");
        drop(last);
        assert_eq!(bank.available_permits(), quota);
    }

    #[tokio::test]
    async fn local_buffer_record_refusal_is_before_headers_and_original_connection_can_retry() {
        let quota = 16 * 1024 * 1024;
        let bank = Arc::new(Semaphore::new(quota));
        let held = Arc::new(std::sync::Mutex::new(None));
        let hold = Arc::clone(&held);
        let target = Arc::clone(&bank);
        let requested = 4 * std::mem::size_of::<(u32, RawBuffer)>();
        super::super::super::windows::ADMISSION.with(|hook| {
            *hook.borrow_mut() = Some(Box::new(move |bytes| {
                if bytes == requested {
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
        let error = framer.open_stream(5).await.unwrap_err();
        super::super::super::windows::ADMISSION.with(|hook| hook.borrow_mut().take());
        assert!(held.lock().unwrap().is_some());
        assert!(
            matches!(error,H2Error::Io(ref io)if crate::classify_error(io)==crate::ErrorClass::ResourceLimit)
        );
        assert!(framer.is_usable());
        assert!(!framer.stream.progressed);
        assert!(!framer.locally_open_streams.contains(&5));
        assert!(framer.outbound_stream_windows.get(&5).is_none());
        assert!(framer.stream_bufs.get(&5).is_none());
        let mut header = [0; 9];
        assert!(timeout(Duration::from_millis(10), peer.read(&mut header))
            .await
            .is_err());
        drop(held.lock().unwrap().take());
        framer.open_stream(5).await.unwrap();
        peer.read_exact(&mut header).await.unwrap();
        assert_eq!(header, build_headers_frame(5)[..]);
        assert!(framer.locally_open_streams.contains(&5));
        drop(framer);
        eof(&mut peer).await;
        assert_eq!(bank.available_permits(), quota);
    }

    #[tokio::test]
    async fn local_finish_membership_refusal_emits_no_end_and_preserves_original_open_owner() {
        let quota = 16 * 1024 * 1024;
        let bank = Arc::new(Semaphore::new(quota));
        let (stream, mut peer) = tokio::io::duplex(4096);
        let mut framer = metered(stream, &bank);
        framer.open_stream(5).await.unwrap();
        let mut frame = [0; 9];
        peer.read_exact(&mut frame).await.unwrap();
        let original = framer.stream_bufs.get(&5).unwrap() as *const RawBuffer;
        let held = Arc::new(std::sync::Mutex::new(None));
        let hold = Arc::clone(&held);
        let target = Arc::clone(&bank);
        super::super::super::windows::ADMISSION.with(|hook| {
            *hook.borrow_mut() = Some(Box::new(move |bytes| {
                let take = target.available_permits() - (bytes - 1);
                *hold.lock().unwrap() = Some(
                    Arc::clone(&target)
                        .try_acquire_many_owned(take.try_into().unwrap())
                        .unwrap(),
                );
            }))
        });
        let error = framer.finish_stream(5).await.unwrap_err();
        super::super::super::windows::ADMISSION.with(|hook| hook.borrow_mut().take());
        assert!(held.lock().unwrap().is_some());
        assert!(
            matches!(error,H2Error::Io(ref io)if crate::classify_error(io)==crate::ErrorClass::ResourceLimit)
        );
        assert!(framer.is_usable());
        assert!(!framer.locally_finished_streams.contains(&5));
        assert_eq!(
            framer.stream_bufs.get(&5).unwrap() as *const RawBuffer,
            original
        );
        assert!(timeout(Duration::from_millis(10), peer.read(&mut frame))
            .await
            .is_err());
        drop(held.lock().unwrap().take());
        framer.finish_stream(5).await.unwrap();
        peer.read_exact(&mut frame).await.unwrap();
        assert_eq!(frame, build_frame(FRAME_DATA, FLAG_END_STREAM, 5, &[])[..]);
        assert!(framer.locally_finished_streams.contains(&5));
        assert!(framer.write_stream(5, b"late").await.is_err());
        drop(framer);
        eof(&mut peer).await;
        assert_eq!(bank.available_permits(), quota);
    }

    #[tokio::test]
    async fn refused_peer_membership_never_advances_original_stream_high_water_mark() {
        let bank = Arc::new(Semaphore::new(0));
        let (stream, mut peer) = tokio::io::duplex(128);
        let mut framer = metered(stream, &bank);
        let error = framer.track_peer_stream(100).unwrap_err();
        assert!(
            matches!(error,H2Error::Io(ref io)if crate::classify_error(io)==crate::ErrorClass::ResourceLimit)
        );
        assert_eq!(framer.highest_peer_stream_id, 0);
        assert!(framer.peer_open_streams.is_empty());
        bank.add_permits(16);
        framer.track_peer_stream(100).unwrap();
        assert_eq!(framer.highest_peer_stream_id, 100);
        assert!(framer.peer_open_streams.contains(&100));
        drop(framer);
        eof(&mut peer).await;
        assert_eq!(bank.available_permits(), 16);
    }

    #[tokio::test]
    async fn original_known_eof_states_stay_pinned_and_oldest_unknown_eviction_reuses_zero_quota() {
        let quota = 1024 * 1024;
        let bank = Arc::new(Semaphore::new(quota));
        let (stream, mut peer) = tokio::io::duplex(4096);
        let mut framer = metered(stream, &bank);
        framer.open_stream(5).await.unwrap();
        let mut header = [0; 9];
        peer.read_exact(&mut header).await.unwrap();
        let ids = [1, 3, 5].into_iter().chain(100..133).collect::<Vec<_>>();
        for id in &ids {
            peer.write_all(&build_frame(FRAME_DATA, FLAG_END_STREAM, *id, &[]))
                .await
                .unwrap();
        }
        for id in ids {
            let actual = framer.read_next_data_frame().await.unwrap();
            assert_eq!(
                (actual.stream_id, actual.flags, actual.payload.len()),
                (id, FLAG_END_STREAM, 0)
            );
        }
        for id in [1, 3, 5] {
            assert!(framer.closed_streams.contains(&id));
        }
        assert_eq!(framer.closed_unknown_order.front(), Some(&101));
        let held = held_all(&bank);
        framer.track_peer_stream(200).unwrap();
        framer.mark_end_stream(200, FLAG_END_STREAM).unwrap();
        assert_eq!(bank.available_permits(), 0);
        assert_eq!(framer.closed_unknown_order.front(), Some(&102));
        assert_eq!(framer.closed_unknown_order.len(), MAX_CLOSED_STREAMS);
        for id in [1, 3, 5] {
            assert!(framer.closed_streams.contains(&id));
        }
        assert!(!framer.closed_streams.contains(&100));
        assert!(framer.ensure_peer_stream_not_reused(100).is_err());
        drop(held);
        drop(framer);
        eof(&mut peer).await;
        assert_eq!(bank.available_permits(), quota);
    }

    #[tokio::test]
    async fn original_known_resets_stay_pinned_with_exact_codes_and_unknown_order_at_zero_quota() {
        let quota = 1024 * 1024;
        let bank = Arc::new(Semaphore::new(quota));
        let (stream, mut peer) = tokio::io::duplex(4096);
        let mut framer = metered(stream, &bank);
        framer.open_stream(5).await.unwrap();
        let mut header = [0; 9];
        peer.read_exact(&mut header).await.unwrap();
        let ids = [1, 3, 5].into_iter().chain(100..133).collect::<Vec<_>>();
        for id in &ids {
            if *id >= 100 {
                peer.write_all(&build_headers_frame(*id)).await.unwrap();
            }
            peer.write_all(&build_frame(
                FRAME_RST_STREAM,
                0,
                *id,
                &0x22u32.to_be_bytes(),
            ))
            .await
            .unwrap();
        }
        for id in ids {
            let error = framer.read_next_data_frame().await.unwrap_err();
            assert!(
                matches!(error,H2Error::Protocol(ref message)if message.contains(&format!("stream {id} was reset"))&&message.contains("34"))
            );
            assert!(framer.is_usable());
        }
        let held = held_all(&bank);
        framer.track_peer_stream(200).unwrap();
        framer.record_reset_stream(200, 0x77).unwrap();
        assert_eq!(bank.available_permits(), 0);
        assert_eq!(framer.reset_unknown_order.front(), Some(&102));
        assert_eq!(framer.reset_unknown_order.len(), MAX_CLOSED_STREAMS);
        for id in [1, 3, 5] {
            assert_eq!(framer.reset_streams.get(&id), Some(&0x22));
        }
        assert_eq!(framer.reset_streams.get(&200), Some(&0x77));
        assert!(!framer.reset_streams.contains_key(&100));
        assert!(framer.ensure_peer_stream_not_reused(100).is_err());
        drop(held);
        drop(framer);
        eof(&mut peer).await;
        assert_eq!(bank.available_permits(), quota);
    }

    #[tokio::test]
    async fn observed_reset_and_original_quota_source_survive_failed_record_and_framer_drop() {
        let quota = 16 * 1024 * 1024;
        let bank = Arc::new(Semaphore::new(quota));
        let held = Arc::new(std::sync::Mutex::new(None));
        let hold = Arc::clone(&held);
        let target = Arc::clone(&bank);
        super::super::super::windows::ADMISSION.with(|hook| {
            *hook.borrow_mut() = Some(Box::new(move |bytes| {
                let take = target.available_permits() - (bytes - 1);
                *hold.lock().unwrap() = Some(
                    Arc::clone(&target)
                        .try_acquire_many_owned(take.try_into().unwrap())
                        .unwrap(),
                );
            }))
        });
        let (stream, mut peer) = tokio::io::duplex(4096);
        let mut framer = metered(stream, &bank);
        peer.write_all(&build_frame(
            FRAME_RST_STREAM,
            0,
            1,
            &0xdead_u32.to_be_bytes(),
        ))
        .await
        .unwrap();
        let error = framer.read_next_data_frame().await.unwrap_err();
        super::super::super::windows::ADMISSION.with(|hook| hook.borrow_mut().take());
        assert!(held.lock().unwrap().is_some());
        let H2Error::Io(original) = error else {
            panic!("reset admission source lost");
        };
        assert_eq!(
            crate::classify_error(&original),
            crate::ErrorClass::ResourceLimit
        );
        assert!(!framer.is_usable());
        let fact = original
            .get_ref()
            .unwrap()
            .downcast_ref::<PeerStateAdmission>()
            .unwrap();
        assert_eq!((fact.stream_id, fact.reset_code), (1, Some(0xdead)));
        let pointer =
            fact.source.get_ref().unwrap() as *const (dyn std::error::Error + Send + Sync);
        drop(framer);
        eof(&mut peer).await;
        drop(held.lock().unwrap().take());
        assert_eq!(bank.available_permits(), quota);
        let fact = original
            .get_ref()
            .unwrap()
            .downcast_ref::<PeerStateAdmission>()
            .unwrap();
        assert_eq!((fact.stream_id, fact.reset_code), (1, Some(0xdead)));
        assert!(std::ptr::eq(fact.source.get_ref().unwrap(), pointer));
        assert_eq!(
            crate::classify_error(&original),
            crate::ErrorClass::ResourceLimit
        );
    }

    #[tokio::test]
    async fn observed_end_stream_survives_arrival_quota_refusal_with_prepared_backing_paid_to_drop()
    {
        let quota = 16 * 1024 * 1024;
        let bank = Arc::new(Semaphore::new(quota));
        let held = Arc::new(std::sync::Mutex::new(None));
        let hold = Arc::clone(&held);
        let target = Arc::clone(&bank);
        ORDER_ADMISSION.with(|hook| {
            *hook.borrow_mut() = Some(Box::new(move |bytes| {
                let take = target.available_permits() - (bytes - 1);
                *hold.lock().unwrap() = Some(
                    Arc::clone(&target)
                        .try_acquire_many_owned(take.try_into().unwrap())
                        .unwrap(),
                );
            }))
        });
        let (stream, mut peer) = tokio::io::duplex(4096);
        let mut framer = metered(stream, &bank);
        peer.write_all(&build_frame(FRAME_DATA, FLAG_END_STREAM, 100, &[]))
            .await
            .unwrap();
        let error = framer.read_next_data_frame().await.unwrap_err();
        ORDER_ADMISSION.with(|hook| hook.borrow_mut().take());
        let H2Error::Io(original) = error else {
            panic!("end-stream admission cause lost");
        };
        let fact = original
            .get_ref()
            .unwrap()
            .downcast_ref::<PeerStateAdmission>()
            .unwrap();
        assert_eq!((fact.stream_id, fact.reset_code), (100, None));
        assert_eq!(
            crate::classify_error(&original),
            crate::ErrorClass::ResourceLimit
        );
        assert!(!framer.is_usable());
        let held_bytes = held.lock().unwrap().as_ref().unwrap().num_permits();
        assert_eq!(
            bank.available_permits(),
            quota - held_bytes - state_bytes(&framer)
        );
        drop(framer);
        eof(&mut peer).await;
        drop(held.lock().unwrap().take());
        assert_eq!(bank.available_permits(), quota);
        assert_eq!(
            original
                .get_ref()
                .unwrap()
                .downcast_ref::<PeerStateAdmission>()
                .unwrap()
                .stream_id,
            100
        );
    }

    #[tokio::test]
    async fn ascending_stream_selection_preserves_ready_and_original_error_priority_without_scratch(
    ) {
        let quota = 1024 * 1024;
        let bank = Arc::new(Semaphore::new(quota));
        let (stream, mut peer) = tokio::io::duplex(4096);
        let mut framer = metered(stream, &bank);
        let message = crate::xpc::message::encode_message(&crate::xpc::message::XpcMessage {
            flags: crate::xpc::message::flags::ALWAYS_SET,
            msg_id: 0,
            body: Some(crate::xpc::message::XpcValue::Uint64(7)),
        })
        .unwrap();
        for (id, bytes) in [(7, &message[..]), (5, &message[..24]), (3, &message[..])] {
            framer.append_stream_data(id, bytes).unwrap();
        }
        framer.mark_end_stream(5, FLAG_END_STREAM).unwrap();
        let held = held_all(&bank);
        assert_eq!(framer.first_ready_xpc_stream().unwrap(), Some(3));
        assert_eq!(framer.first_unfinished_closed_stream().unwrap(), Some(5));
        assert_eq!(bank.available_permits(), 0);
        drop(held);
        let mut malformed = message[..24].to_vec();
        malformed[8..16].copy_from_slice(&u64::MAX.to_le_bytes());
        framer.append_stream_data(1, &malformed).unwrap();
        let held = held_all(&bank);
        assert!(framer.first_ready_xpc_stream().is_err());
        assert_eq!(bank.available_permits(), 0);
        drop(held);
        drop(framer);
        eof(&mut peer).await;
        assert_eq!(bank.available_permits(), quota);
    }
}
