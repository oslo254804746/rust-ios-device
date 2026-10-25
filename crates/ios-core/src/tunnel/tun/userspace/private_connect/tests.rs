use super::super::{UserspaceTunDevice, ValidatedMtu};
use super::*;
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

const TOTAL: usize = 16 * 1024 * 1024;
const OBSERVE: Duration = Duration::from_secs(3);

fn prelude(endpoint: SocketAddrV6) -> [u8; 20] {
    let mut bytes = [0; 20];
    bytes[..16].copy_from_slice(&endpoint.ip().octets());
    bytes[16..].copy_from_slice(&(endpoint.port() as u32).to_le_bytes());
    bytes
}

#[tokio::test]
async fn actual_private_listener_rejects_unregistered_and_mismatched_flows_before_syn() {
    let baseline = tokio::runtime::Handle::current()
        .metrics()
        .num_alive_tasks();
    let bank = Arc::new(Semaphore::new(TOTAL));
    let (stream, mut peer) = tokio::io::duplex(4096);
    let (runtime, launch) = UserspaceTunDevice::prepare_private_validated_with_bank(
        "fd00::2",
        "fd00::1",
        ValidatedMtu::new(1280).unwrap(),
        stream,
        bank.clone(),
        2,
        true,
    )
    .await
    .unwrap();
    let connector = runtime.private_connector().unwrap();
    let endpoint: SocketAddrV6 = "[fd00::1]:58783".parse().unwrap();
    assert!(!runtime.is_alive());
    launch().unwrap();
    let owner_bytes = bank.available_permits();
    let mut anonymous = TcpStream::connect((Ipv4Addr::LOCALHOST, runtime.local_port))
        .await
        .unwrap();
    // The server rejects before reading a preamble or preparing a child future.
    assert!(matches!(
        tokio::time::timeout(OBSERVE, anonymous.read(&mut [0; 1])).await,
        Ok(Ok(0))
    ));
    assert_eq!(bank.available_permits(), owner_bytes);
    drop(anonymous);
    let mut mismatch = connector.connect(endpoint).await.unwrap();
    mismatch
        .write_all(&prelude("[fd00::9]:12345".parse().unwrap()))
        .await
        .unwrap();
    assert!(matches!(
        tokio::time::timeout(OBSERVE, mismatch.read(&mut [0; 1])).await,
        Ok(Ok(0))
    ));
    drop(mismatch);
    let mut actual = connector.connect(endpoint).await.unwrap();
    let source = connector
        .0
        .state
        .lock()
        .unwrap()
        .entries
        .as_ref()
        .unwrap()
        .values[actual.index]
        .as_ref()
        .unwrap()
        .source_port;
    actual.write_all(&prelude(endpoint)).await.unwrap();
    let packet = tokio::time::timeout(OBSERVE, async {
        let mut header = [0; 40];
        peer.read_exact(&mut header).await.unwrap();
        assert_eq!(header[0] >> 4, 6);
        assert_eq!(header[6], 6);
        assert_eq!(
            &header[8..24],
            &"fd00::2".parse::<Ipv6Addr>().unwrap().octets()
        );
        let mut payload = vec![0; u16::from_be_bytes([header[4], header[5]]) as usize];
        peer.read_exact(&mut payload).await.unwrap();
        payload
    })
    .await
    .unwrap();
    assert_eq!(u16::from_be_bytes([packet[0], packet[1]]), source);
    assert_eq!(u16::from_be_bytes([packet[2], packet[3]]), endpoint.port());
    assert_eq!(packet[13] & 0x12, 0x02);
    drop(actual);
    runtime.shutdown(OBSERVE).await.unwrap();
    assert!(matches!(
        tokio::time::timeout(OBSERVE, peer.read(&mut [0; 1])).await,
        Ok(Ok(0))
    ));
    drop((runtime, connector, peer));
    tokio::time::timeout(OBSERVE, async {
        while tokio::runtime::Handle::current()
            .metrics()
            .num_alive_tasks()
            != baseline
        {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    assert_eq!(bank.available_permits(), TOTAL);
}

#[tokio::test]
async fn original_private_slots_keep_real_wildcard_and_loopback_descriptors_with_closed_zero_bank()
{
    let bank = Arc::new(Semaphore::new(TOTAL));
    let listener = tokio::net::TcpListener::bind((Ipv4Addr::LOCALHOST, 0))
        .await
        .unwrap();
    let connector =
        PrivateConnector::prepare(listener.local_addr().unwrap().port(), 1, true, &bank).unwrap();
    let held = bank
        .clone()
        .try_acquire_many_owned(bank.available_permits() as u32)
        .unwrap();
    bank.close();
    let endpoint: SocketAddrV6 = "[fd00::1]:58783".parse().unwrap();
    let mut stream = connector.connect(endpoint).await.unwrap();
    let (mut peer, tuple) = listener.accept().await.unwrap();
    let Access::Private {
        endpoint: registered,
        source_port,
    } = connector.claim(tuple).unwrap()
    else {
        panic!("private registration lost")
    };
    assert_eq!(registered, endpoint);
    assert!(connector.claim(tuple).is_none());
    let reservation = exclusive_socket(Domain::IPV6).unwrap();
    reservation.set_only_v6(true).unwrap();
    let conflict = reservation
        .bind(&SocketAddr::from((Ipv6Addr::UNSPECIFIED, source_port)).into())
        .unwrap_err();
    assert_eq!(conflict.kind(), io::ErrorKind::AddrInUse);
    assert!(conflict.raw_os_error().is_some());
    let local_conflict = exclusive_socket(Domain::IPV4)
        .unwrap()
        .bind(&tuple.into())
        .unwrap_err();
    assert_eq!(local_conflict.kind(), io::ErrorKind::AddrInUse);
    assert_eq!(bank.available_permits(), 0);
    stream.write_all(&prelude(endpoint)).await.unwrap();
    let mut bytes = [0; 20];
    peer.read_exact(&mut bytes).await.unwrap();
    assert_eq!(bytes, prelude(endpoint));
    let exhausted = connector.connect(endpoint).await.err().unwrap();
    assert_eq!(
        crate::classify_error(&exhausted),
        crate::ErrorClass::ResourceLimit
    );
    drop(stream);
    assert!(matches!(
        tokio::time::timeout(OBSERVE, peer.read(&mut [0; 1])).await,
        Ok(Ok(0))
    ));
    // The original duplicate still holds the tuple after the stream has ended.
    assert_eq!(
        exclusive_socket(Domain::IPV4)
            .unwrap()
            .bind(&tuple.into())
            .unwrap_err()
            .kind(),
        io::ErrorKind::AddrInUse
    );
    assert_eq!(bank.available_permits(), 0);
    connector.close();
    drop((connector, peer, listener, held));
    assert_eq!(bank.available_permits(), TOTAL);
}
