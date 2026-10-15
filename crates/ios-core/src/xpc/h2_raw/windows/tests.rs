use super::*;

fn bytes<V>(slots: usize) -> usize {
    slots * std::mem::size_of::<(u32, V)>()
}
fn put<V>(map: &mut WindowMap<V>, key: u32, value: V) {
    map.prepare_keys(&[key]).unwrap();
    map.insert_reserved(key, value);
}
fn owner<V>(map: &WindowMap<V>) -> &Counters<V> {
    let WindowMap::Banked(owner) = map else {
        panic!("expected banked counters");
    };
    owner
}

#[test]
fn actual_counter_growth_one_byte_refusal_retains_original_values_and_pointer() {
    let bank = Arc::new(Semaphore::new(bytes::<i64>(12) - 1));
    let mut map = WindowMap::new(Some(&bank));
    for key in 0..4 {
        put(&mut map, key, key as i64 - 3);
    }
    let pointer = owner(&map).values.as_ptr();
    let before = owner(&map).values.clone();
    assert!(map.prepare_keys(&[4, 5]).is_err());
    assert_eq!(owner(&map).values, before);
    assert_eq!(owner(&map).values.as_ptr(), pointer);
    assert_eq!(bank.available_permits(), bytes::<i64>(8) - 1);
    bank.add_permits(1);
    map.prepare_keys(&[4, 5]).unwrap();
    assert_eq!(map.backing_bytes(), bytes::<i64>(8));
    assert_eq!(bank.available_permits(), bytes::<i64>(4));
    map.insert_reserved(4, 11);
    map.insert_reserved(5, -9);
    assert_eq!(map.get(&5), Some(&-9));
    drop(map);
    assert_eq!(bank.available_permits(), bytes::<i64>(12));
}

#[test]
fn duplicate_key_batch_replacement_and_vacant_slot_reuse_need_no_second_grant() {
    let bank = Arc::new(Semaphore::new(bytes::<u32>(4)));
    let mut map = WindowMap::new(Some(&bank));
    map.prepare_keys(&[0, 0, 1, 1]).unwrap();
    assert_eq!(bank.available_permits(), 0);
    map.insert_reserved(0, 12);
    map.insert_reserved(1, 13);
    let pointer = owner(&map).values.as_ptr();
    map.prepare_keys(&[0, 1, 2, 2, 3]).unwrap();
    map.insert_reserved(2, 14);
    map.insert_reserved(3, 15);
    assert_eq!(map.insert_reserved(0, 16), Some(12));
    assert_eq!(map.remove(&1), Some(13));
    put(&mut map, 5, 19);
    assert_eq!(owner(&map).values.as_ptr(), pointer);
    map.clear();
    assert_eq!(bank.available_permits(), 0);
    drop(map);
    assert_eq!(bank.available_permits(), bytes::<u32>(4));
}

#[test]
fn mixed_counter_trace_matches_original_map_and_returns_all_backing_on_drop() {
    let quota = 1024 * 1024;
    let bank = Arc::new(Semaphore::new(quota));
    let mut map = WindowMap::new(Some(&bank));
    let mut old = HashMap::new();
    let mut seed = 13u32;
    for _ in 0..3000 {
        seed = seed.wrapping_mul(1664525).wrapping_add(1013904223);
        let key = seed % 71;
        if seed % 4 == 0 {
            assert_eq!(map.remove(&key), old.remove(&key));
        } else {
            map.prepare_keys(&[key]).unwrap();
            assert_eq!(map.insert_reserved(key, seed), old.insert(key, seed));
        }
        for (key, value) in &old {
            assert_eq!(map.get(key), Some(value));
        }
        assert_eq!(map.iter().count(), old.len());
        assert_eq!(bank.available_permits(), quota - map.backing_bytes());
    }
    drop(map);
    assert_eq!(bank.available_permits(), quota);
}

#[cfg(feature = "tunnel")]
mod wire {
    use super::super::super::*;
    use super::*;
    use tokio::io::AsyncReadExt;
    use tokio::time::{timeout, Duration};

    fn metered(
        stream: tokio::io::DuplexStream,
        bank: &Arc<Semaphore>,
    ) -> H2Framer<tokio::io::DuplexStream> {
        let mut framer = H2Framer::new(stream);
        framer.configure_bank(Some(bank));
        framer
    }
    async fn eof(peer: &mut tokio::io::DuplexStream) {
        let mut byte = [0];
        assert!(matches!(
            timeout(Duration::from_secs(2), peer.read(&mut byte)).await,
            Ok(Ok(0))
        ));
    }
    #[tokio::test]
    async fn receive_credit_new_keys_refusal_does_not_reset_existing_counter_or_emit_frame() {
        let quota = 16 * 1024 * 1024;
        let bank = Arc::new(Semaphore::new(quota));
        let (stream, mut peer) = tokio::io::duplex(4096);
        let mut framer = metered(stream, &bank);
        for key in [0, 5, 7, 9] {
            put(
                &mut framer.pending_window_updates,
                key,
                WINDOW_UPDATE_THRESHOLD - 1,
            );
        }
        let held = Arc::clone(&bank)
            .try_acquire_many_owned(bank.available_permits().try_into().unwrap())
            .unwrap();
        let error = framer.replenish_receive_window(3, 1).await.unwrap_err();
        assert!(
            matches!(error,H2Error::Io(ref io) if crate::classify_error(io)==crate::ErrorClass::ResourceLimit)
        );
        assert_eq!(
            framer.pending_window_updates[&0],
            WINDOW_UPDATE_THRESHOLD - 1
        );
        assert!(framer.pending_window_updates.get(&3).is_none());
        let mut frame = [0; 13];
        assert!(timeout(Duration::from_millis(10), peer.read(&mut frame))
            .await
            .is_err());
        drop(held);
        framer.replenish_receive_window(3, 1).await.unwrap();
        peer.read_exact(&mut frame).await.unwrap();
        assert_eq!(frame, window_update_bytes(0, WINDOW_UPDATE_THRESHOLD));
        assert_eq!(framer.pending_window_updates[&0], 0);
        assert_eq!(framer.pending_window_updates[&3], 1);
        drop(framer);
        eof(&mut peer).await;
        assert_eq!(bank.available_permits(), quota);
    }

    #[tokio::test]
    async fn new_outbound_window_refusal_precedes_headers_and_keeps_same_original_connection() {
        let quota = 16 * 1024 * 1024;
        let bank = Arc::new(Semaphore::new(quota));
        let held = Arc::new(std::sync::Mutex::new(None));
        let hold = Arc::clone(&held);
        let target = Arc::clone(&bank);
        ADMISSION.with(|hook| {
            *hook.borrow_mut() = Some(Box::new(move |requested| {
                let take = target.available_permits() - (requested - 1);
                *hold.lock().unwrap() = Some(
                    Arc::clone(&target)
                        .try_acquire_many_owned(take.try_into().unwrap())
                        .unwrap(),
                );
            }))
        });
        let (stream, mut peer) = tokio::io::duplex(4096);
        let mut framer = metered(stream, &bank);
        let error = framer.open_stream(5).await.unwrap_err();
        ADMISSION.with(|hook| hook.borrow_mut().take());
        assert!(
            held.lock().unwrap().is_some(),
            "not the actual map allocation"
        );
        assert!(
            matches!(error,H2Error::Io(ref io) if crate::classify_error(io)==crate::ErrorClass::ResourceLimit)
        );
        assert!(framer.is_usable());
        assert!(!framer.stream.progressed);
        assert!(framer.outbound_stream_windows.is_empty());
        let mut header = [0; 9];
        assert!(timeout(Duration::from_millis(10), peer.read(&mut header))
            .await
            .is_err());
        drop(held.lock().unwrap().take());
        framer.open_stream(5).await.unwrap();
        peer.read_exact(&mut header).await.unwrap();
        assert_eq!(header, build_headers_frame(5)[..]);
        drop(framer);
        eof(&mut peer).await;
        assert_eq!(bank.available_permits(), quota);
    }

    #[tokio::test]
    async fn settings_apply_all_validated_deltas_without_new_scratch_grant_at_zero_quota() {
        let quota = 16 * 1024 * 1024;
        let bank = Arc::new(Semaphore::new(quota));
        let (stream, mut peer) = tokio::io::duplex(4096);
        let mut framer = metered(stream, &bank);
        put(&mut framer.outbound_stream_windows, 1, 10);
        put(&mut framer.outbound_stream_windows, 3, -2);
        let held = Arc::clone(&bank)
            .try_acquire_many_owned(bank.available_permits().try_into().unwrap())
            .unwrap();
        let mut payload = Vec::new();
        payload.extend_from_slice(&SETTINGS_INITIAL_WINDOW_SIZE.to_be_bytes());
        payload.extend_from_slice(&65537u32.to_be_bytes());
        payload.extend_from_slice(&SETTINGS_MAX_FRAME_SIZE.to_be_bytes());
        payload.extend_from_slice(&32768u32.to_be_bytes());
        framer.process_settings(0, &payload).await.unwrap();
        assert_eq!(framer.outbound_stream_windows[&1], 12);
        assert_eq!(framer.outbound_stream_windows[&3], 0);
        assert_eq!(framer.peer_initial_window_size, 65537);
        assert_eq!(framer.outbound_max_frame_size, 32768);
        assert_eq!(bank.available_permits(), 0);
        let mut ack = [0; 9];
        peer.read_exact(&mut ack).await.unwrap();
        assert_eq!(ack, build_settings_ack()[..]);
        drop(held);
        drop(framer);
        eof(&mut peer).await;
        assert_eq!(bank.available_permits(), quota);
    }

    #[tokio::test]
    async fn settings_invalid_native_window_precedes_quota_and_never_commits_or_acknowledges() {
        let quota = bytes::<i64>(4);
        let bank = Arc::new(Semaphore::new(quota));
        let (stream, mut peer) = tokio::io::duplex(4096);
        let mut framer = metered(stream, &bank);
        put(
            &mut framer.outbound_stream_windows,
            1,
            MAX_FLOW_CONTROL_WINDOW,
        );
        framer.peer_initial_window_size = 0;
        let mut payload = SETTINGS_INITIAL_WINDOW_SIZE.to_be_bytes().to_vec();
        payload.extend_from_slice(&1u32.to_be_bytes());
        let error = framer.process_settings(0, &payload).await.unwrap_err();
        assert!(matches!(error, H2Error::Protocol(_)));
        assert_eq!(framer.peer_initial_window_size, 0);
        assert!(!framer.is_usable());
        assert_eq!(bank.available_permits(), 0);
        drop(framer);
        eof(&mut peer).await;
        assert_eq!(bank.available_permits(), quota);
    }

    #[test]
    fn inline_window_frames_match_original_builder_including_reserved_bit_masking() {
        for id in [0, 1, 3, 0x7fff_ffff, u32::MAX] {
            for increment in [0, 1, WINDOW_UPDATE_THRESHOLD, 0x7fff_ffff, u32::MAX] {
                assert_eq!(
                    window_update_bytes(id, increment),
                    build_window_update_frame(id, increment)[..]
                );
            }
        }
    }
}
