use super::*;
use tokio::io::AsyncReadExt;
use tokio::time::{timeout, Duration};

#[test]
fn original_host_messages_and_bootstrap_flags_match_metered_encoding() {
    let quota = 65536;
    let bank = Arc::new(Semaphore::new(quota));
    for id in [
        [0; 16],
        [1; 16],
        [255; 16],
        [0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15],
    ] {
        let identity = uuid::Uuid::from_bytes(id);
        let original = host(None, identity).unwrap();
        let wire = host(Some(&bank), identity).unwrap();
        assert_eq!(&*wire, &*original);
        assert_eq!(bank.available_permits(), quota - wire.len());
        drop(wire);
        assert_eq!(bank.available_permits(), quota);
    }
    for (flag, dictionary) in [
        (flags::ALWAYS_SET, true),
        (flags::ALWAYS_SET | 0x200, false),
        (flags::INIT_HANDSHAKE | flags::ALWAYS_SET, false),
    ] {
        let original = control(None, flag, dictionary).unwrap();
        let wire = control(Some(&bank), flag, dictionary).unwrap();
        assert_eq!(&*wire, &*original);
        drop(wire);
    }
    assert_eq!(bank.available_permits(), quota);
}

#[test]
fn three_retained_bootstrap_wire_owners_are_admitted_together_until_each_original_drop() {
    let quota = 44 + 24 + 24;
    let bank = Arc::new(Semaphore::new(quota));
    let first = control(Some(&bank), flags::ALWAYS_SET, true).unwrap();
    let second = control(Some(&bank), flags::ALWAYS_SET | 0x200, false).unwrap();
    let third = control(
        Some(&bank),
        flags::INIT_HANDSHAKE | flags::ALWAYS_SET,
        false,
    )
    .unwrap();
    assert_eq!(bank.available_permits(), 0);
    assert_eq!((first.len(), second.len(), third.len()), (44, 24, 24));
    drop(first);
    assert_eq!(bank.available_permits(), 44);
    drop(second);
    assert_eq!(bank.available_permits(), 68);
    drop(third);
    assert_eq!(bank.available_permits(), quota);
}

#[tokio::test]
async fn actual_host_wire_quota_refusal_sends_no_headers_and_original_framer_retries_same_identity()
{
    let quota = 1024 * 1024;
    let bank = Arc::new(Semaphore::new(quota));
    let identity = uuid::Uuid::from_bytes([9; 16]);
    let expected = host(None, identity).unwrap();
    let len = expected.len();
    let (stream, mut peer) = tokio::io::duplex(4096);
    let mut framer = H2Framer::begin_connect_with_bank(stream, Some(&bank))
        .await
        .unwrap();
    let mut preface = [0; 58];
    peer.read_exact(&mut preface).await.unwrap();
    assert_eq!(&preface[..24], crate::xpc::h2_raw::H2_PREFACE);
    let held = Arc::clone(&bank)
        .try_acquire_many_owned((bank.available_permits() - (len - 1)).try_into().unwrap())
        .unwrap();
    let error = send_device_handshake_on_framer(&mut framer, identity)
        .await
        .unwrap_err();
    assert_eq!(
        crate::classify_error(&error),
        crate::ErrorClass::ResourceLimit
    );
    assert!(framer.is_usable());
    let mut header = [0; 9];
    assert!(timeout(Duration::from_millis(10), peer.read(&mut header))
        .await
        .is_err());
    drop(held);
    send_device_handshake_on_framer(&mut framer, identity)
        .await
        .unwrap();
    peer.read_exact(&mut header).await.unwrap();
    assert_eq!((header[3], header[4], header[8]), (1, 4, 1));
    peer.read_exact(&mut header).await.unwrap();
    let wire_len = ((header[0] as usize) << 16) | ((header[1] as usize) << 8) | header[2] as usize;
    assert_eq!((header[3], header[8], wire_len), (0, 1, len));
    let mut body = vec![0; wire_len];
    peer.read_exact(&mut body).await.unwrap();
    assert_eq!(&body[..], &*expected);
    drop(framer);
    let mut byte = [0];
    assert!(matches!(
        timeout(Duration::from_secs(2), peer.read(&mut byte)).await,
        Ok(Ok(0))
    ));
    assert_eq!(bank.available_permits(), quota);
}
