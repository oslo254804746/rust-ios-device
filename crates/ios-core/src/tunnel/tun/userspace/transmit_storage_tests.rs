//! Isolated duplex and localhost fixtures; no physical device or native TUN.

use super::*;
use smoltcp::phy::TxToken;
use std::error::Error;
use std::pin::Pin;
use std::sync::atomic::AtomicUsize;
use std::task::{Context, Poll};
use tokio::io::{AsyncRead, AsyncWrite, DuplexStream, ReadBuf};

const OBSERVE: Duration = Duration::from_secs(2);

#[test]
fn unmetered_transmit_keeps_legacy_requested_length_and_real_closure_result() {
    let mtu = ValidatedMtu::new(1280).unwrap();
    let mut device = ChannelDevice::new(mtu, RuntimeStorage::unmetered());
    let token = device.transmit(smol_now()).unwrap();
    assert!(token.buffer.is_none());
    let result = token.consume(41, |packet| {
        packet.fill(0x6a);
        73usize
    });
    assert_eq!(result, 73);
    let packet = device.tx_buf.pop_front().unwrap();
    assert_eq!(packet.len(), 41);
    assert_eq!(packet.capacity(), 41);
    assert!(packet.iter().all(|byte| *byte == 0x6a));
}

fn storage(bank: &Arc<Semaphore>) -> RuntimeStorage {
    let mut storage = RuntimeStorage::unmetered();
    storage.bank = Some(Arc::clone(bank));
    storage
}

fn report(error: &TunnelError) -> &crate::TunnelShutdownFailure {
    let mut current: Option<&(dyn Error + 'static)> = Some(error);
    for _ in 0..32 {
        let cause = current.expect("typed shutdown report missing");
        if let Some(report) = cause.downcast_ref() {
            return report;
        }
        current = cause.source();
    }
    panic!("source chain exceeded fixture bound");
}

fn alive() -> usize {
    tokio::runtime::Handle::current()
        .metrics()
        .num_alive_tasks()
}

async fn until(condition: impl Fn() -> bool) -> bool {
    tokio::time::timeout(OBSERVE, async {
        while !condition() {
            tokio::task::yield_now().await;
        }
    })
    .await
    .is_ok()
}

#[test]
fn transmit_token_quota_refusal_records_original_error_stops_and_exposes_no_token() {
    let mtu = ValidatedMtu::new(1280).unwrap();
    let backing = 4 * std::mem::size_of::<OwnedBuffer>();
    let bank = Arc::new(Semaphore::new(backing + mtu.as_usize() - 1));
    let storage = storage(&bank);
    let stop = storage.stop.subscribe();
    let mut device = ChannelDevice::new(mtu, storage.clone());
    assert!(device.transmit(smol_now()).is_none());
    assert!(storage.stopped.load(Ordering::Acquire));
    assert!(stop.has_changed().unwrap());
    let error = storage.result().unwrap_err();
    assert_eq!(
        crate::classify_error(&error),
        crate::ErrorClass::ResourceLimit
    );
    let TunnelError::Io(original) = report(&error)
        .primary()
        .downcast_ref::<TunnelError>()
        .unwrap()
    else {
        panic!("typed original quota cause lost");
    };
    assert!(matches!(
        original
            .get_ref()
            .unwrap()
            .downcast_ref::<owned_buffer::BufferAdmission>(),
        Some(owned_buffer::BufferAdmission::Exhausted)
    ));
    assert!(
        device.transmit(smol_now()).is_none(),
        "fatal device offered a later fake token"
    );
    assert_eq!(bank.available_permits(), mtu.as_usize() - 1);
    drop(device);
    assert_eq!(bank.available_permits(), backing + mtu.as_usize() - 1);
}

#[test]
fn transmit_token_prepares_full_mtu_and_returns_the_real_closure_value_without_refund() {
    let mtu = ValidatedMtu::new(1280).unwrap();
    let backing = 4 * std::mem::size_of::<OwnedBuffer>();
    let quota = backing + mtu.as_usize();
    let bank = Arc::new(Semaphore::new(quota));
    let mut device = ChannelDevice::new(mtu, storage(&bank));
    let token = device.transmit(smol_now()).unwrap();
    assert_eq!(bank.available_permits(), 0);
    let expected = Arc::new(7usize);
    let actual = token.consume(41, |packet| {
        assert_eq!(packet.len(), 41);
        assert_eq!(bank.available_permits(), 0);
        packet.fill(0x6a);
        Arc::clone(&expected)
    });
    assert!(Arc::ptr_eq(&actual, &expected));
    assert_eq!(bank.available_permits(), 0);
    let packet = device.tx_buf.pop_front().unwrap();
    assert_eq!(packet.len(), 41);
    assert_eq!(packet.capacity(), mtu.as_usize());
    drop(device);
    assert_eq!(
        bank.available_permits(),
        backing,
        "device Drop refunded a transferred packet"
    );
    drop(packet);
    assert_eq!(bank.available_permits(), quota);
}

#[test]
fn receive_token_preparation_refusal_keeps_the_original_received_packet() {
    let mtu = ValidatedMtu::new(1280).unwrap();
    let backing = 4 * std::mem::size_of::<OwnedBuffer>();
    let quota = backing * 2 + 40 + mtu.as_usize() - 1;
    let bank = Arc::new(Semaphore::new(quota));
    let packet = OwnedBuffer::zeroed(40, Some(&bank)).unwrap();
    let pointer = packet.as_ptr();
    let mut device = ChannelDevice::new(mtu, storage(&bank));
    device.rx_buf.push_back(packet).unwrap();
    assert!(device.receive(smol_now()).is_none());
    assert_eq!(
        device.rx_buf.len(),
        1,
        "refused response token discarded received data"
    );
    let packet = device.rx_buf.pop_front().unwrap();
    assert_eq!(packet.as_ptr(), pointer);
    assert_eq!(bank.available_permits(), mtu.as_usize() - 1);
    drop(device);
    assert_eq!(bank.available_permits(), quota - 40);
    drop(packet);
    assert_eq!(bank.available_permits(), quota);
}

#[test]
fn queue_replacement_prepays_peak_and_one_byte_short_preserves_old_backing_and_items() {
    let old_bytes = 4 * std::mem::size_of::<u64>();
    let new_bytes = old_bytes * 2;
    for shortage in [0, 1] {
        let quota = old_bytes + new_bytes - shortage;
        let bank = Arc::new(Semaphore::new(quota));
        let mut queue = OwnedQueue::new(Some(&bank));
        for value in 0u64..4 {
            queue.push_back(value).unwrap();
        }
        let original = queue.values().front().unwrap() as *const u64;
        let result = queue.push_back(4u64);
        if shortage == 1 {
            let (error, returned) = result.unwrap_err();
            assert_eq!(returned, 4);
            assert_eq!(
                crate::classify_error(&error),
                crate::ErrorClass::ResourceLimit
            );
            assert_eq!(queue.values().front().unwrap() as *const u64, original);
            assert_eq!(queue.len(), 4);
            assert_eq!(bank.available_permits(), new_bytes - 1);
        } else {
            result.unwrap();
            assert_eq!(queue.len(), 5);
            assert_eq!(bank.available_permits(), old_bytes);
        }
        assert!(queue.iter().copied().eq(0..queue.len() as u64));
        drop(queue);
        assert_eq!(bank.available_permits(), quota);
    }
}

#[test]
fn socket_pair_actual_two_mebibyte_capacity_and_one_byte_short_preserve_owner_order() {
    let quota = SOCKET_BUFFER_BYTES * 2;
    let bank = Arc::new(Semaphore::new(quota));
    let (rx, tx, fees) = socket_buffers(Some(&bank), SOCKET_BUFFER_BYTES).unwrap();
    assert_eq!(rx.capacity(), SOCKET_BUFFER_BYTES);
    assert_eq!(tx.capacity(), SOCKET_BUFFER_BYTES);
    assert_eq!(bank.available_permits(), 0);
    let socket = tcp::Socket::new(tcp::SocketBuffer::new(rx), tcp::SocketBuffer::new(tx));
    let mut sockets = SocketSet::new(vec![]);
    let handle = sockets.add(socket);
    let original = sockets.remove(handle);
    drop(sockets);
    assert_eq!(
        bank.available_permits(),
        0,
        "socket set removal refunded a returned live socket"
    );
    drop(original);
    assert_eq!(
        bank.available_permits(),
        0,
        "fee owner did not await explicit socket destruction"
    );
    drop(fees);
    assert_eq!(bank.available_permits(), quota);
    let short = Arc::new(Semaphore::new(quota - 1));
    let error = socket_buffers(Some(&short), SOCKET_BUFFER_BYTES)
        .err()
        .unwrap();
    assert_eq!(
        crate::classify_error(&error),
        crate::ErrorClass::ResourceLimit
    );
    assert_eq!(short.available_permits(), quota - 1);
}

#[test]
fn one_byte_chunk_and_partial_cursor_keep_original_capacity_and_backing_fee() {
    let quota = SOCKET_RECV_CHUNK_BYTES + 4 * std::mem::size_of::<OwnedBuffer>();
    let bank = Arc::new(Semaphore::new(quota));
    let mut packet = OwnedBuffer::zeroed(SOCKET_RECV_CHUNK_BYTES, Some(&bank)).unwrap();
    let original = packet.as_ptr();
    packet.advance(SOCKET_RECV_CHUNK_BYTES - 1);
    assert_eq!(packet.len(), 1);
    assert_eq!(
        packet.as_ptr(),
        original.wrapping_add(SOCKET_RECV_CHUNK_BYTES - 1)
    );
    let mut pending = OwnedQueue::new(Some(&bank));
    pending.push_back(packet).unwrap();
    assert_eq!(bank.available_permits(), 0);
    let packet = pending.pop_front().unwrap();
    drop(pending);
    assert_eq!(
        bank.available_permits(),
        4 * std::mem::size_of::<OwnedBuffer>()
    );
    assert_eq!(packet.capacity(), SOCKET_RECV_CHUNK_BYTES);
    drop(packet);
    assert_eq!(bank.available_permits(), quota);
}

#[derive(Default)]
struct Probe {
    written: AtomicUsize,
    pending: AtomicBool,
    drops: AtomicUsize,
}

struct Tracked {
    inner: DuplexStream,
    probe: Arc<Probe>,
}

impl AsyncRead for Tracked {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<std::io::Result<()>> {
        Pin::new(&mut self.inner).poll_read(cx, buf)
    }
}

impl AsyncWrite for Tracked {
    fn poll_write(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        bytes: &[u8],
    ) -> Poll<std::io::Result<usize>> {
        let result = Pin::new(&mut self.inner).poll_write(cx, bytes);
        match &result {
            Poll::Ready(Ok(count)) => {
                self.probe.written.fetch_add(*count, Ordering::SeqCst);
            }
            Poll::Pending => {
                self.probe.pending.store(true, Ordering::SeqCst);
            }
            Poll::Ready(Err(_)) => {}
        }
        result
    }
    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        Pin::new(&mut self.inner).poll_flush(cx)
    }
    fn poll_shutdown(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        Pin::new(&mut self.inner).poll_shutdown(cx)
    }
}

impl Drop for Tracked {
    fn drop(&mut self) {
        self.probe.drops.fetch_add(1, Ordering::SeqCst);
    }
}

#[tokio::test]
async fn transmit_actual_partial_duplex_write_keeps_full_packet_capacity_until_final_write_join() {
    let baseline = alive();
    let quota = 4096;
    let bank = Arc::new(Semaphore::new(quota));
    let mut packet = OwnedBuffer::zeroed(quota, Some(&bank)).unwrap();
    packet[..3].copy_from_slice(b"abc");
    packet.truncate(3);
    let (tx, rx) = mpsc::channel(1);
    tx.send(packet).await.unwrap();
    drop(tx);
    let (inner, mut peer) = tokio::io::duplex(1);
    let probe = Arc::new(Probe::default());
    let mut writer = Tracked {
        inner,
        probe: Arc::clone(&probe),
    };
    let ledger = Ledger::new();
    let handle = tokio::spawn(async move {
        write_ios_packets_owned(&mut writer, ValidatedMtu::new(1280).unwrap(), rx).await
    });
    let task = OwnedTask::new(handle, ParentRole::Writer, Arc::clone(&ledger));
    let pending = until(|| probe.pending.load(Ordering::SeqCst)).await;
    let fee_while_pending = bank.available_permits();
    let mut bytes = Vec::new();
    let read = tokio::time::timeout(OBSERVE, peer.read_to_end(&mut bytes)).await;
    let result = owner::join_until(owner::deadline(OBSERVE), &ledger, task.join()).await;
    let tasks = alive();
    let drops = probe.drops.load(Ordering::SeqCst);
    let mut byte = [0];
    let eof = tokio::time::timeout(OBSERVE, peer.read(&mut byte)).await;
    assert!(pending);
    assert_eq!(
        fee_while_pending, 0,
        "partial write released a retained 4096-byte Vec"
    );
    read.unwrap().unwrap();
    result.unwrap();
    assert_eq!(bytes, b"abc");
    assert!(matches!(eof, Ok(Ok(0))));
    assert_eq!(tasks, baseline);
    assert_eq!(drops, 1);
    assert_eq!(bank.available_permits(), quota);
}

#[tokio::test]
async fn transmit_actual_runtime_quota_failure_stops_and_joins_all_original_workers_and_client() {
    let baseline = alive();
    let mtu = ValidatedMtu::new(1280).unwrap();
    let tx_backing = 4 * std::mem::size_of::<OwnedBuffer>();
    let quota = 16 * 1024 * 1024;
    let bank = Arc::new(Semaphore::new(quota));
    let held = Arc::new(std::sync::Mutex::new(None));
    let hold = Arc::clone(&held);
    let target_bank = Arc::clone(&bank);
    BEFORE_TX_ADMISSION.with(|hook| {
        *hook.borrow_mut() = Some(Box::new(move || {
            let available = tx_backing + mtu.as_usize() - 1;
            let take = target_bank.available_permits() - available;
            *hold.lock().unwrap() = Some(
                Arc::clone(&target_bank)
                    .try_acquire_many_owned(take.try_into().unwrap())
                    .unwrap(),
            );
        }));
    });
    let tx_phase = Arc::new(AtomicBool::new(false));
    let reached = Arc::clone(&tx_phase);
    let observed_bank = Arc::clone(&bank);
    TX_ADMISSION.with(|hook| {
        *hook.borrow_mut() = Some(Box::new(move |refused| {
            assert_eq!(observed_bank.available_permits(), mtu.as_usize() - 1);
            assert!(
                refused,
                "runtime quota did not target the actual Tx buffer admission"
            );
            reached.store(true, Ordering::SeqCst);
        }))
    });
    let (inner, mut peer) = tokio::io::duplex(4096);
    let probe = Arc::new(Probe::default());
    let stream = Tracked {
        inner,
        probe: Arc::clone(&probe),
    };
    let runtime = UserspaceTunDevice::start_validated_with_bank(
        "fd00::1",
        "fd00::2",
        mtu,
        stream,
        Some(Arc::clone(&bank)),
    )
    .await
    .unwrap();
    let mut client =
        tokio::net::TcpStream::connect((std::net::Ipv4Addr::LOCALHOST, runtime.local_port))
            .await
            .unwrap();
    let mut preamble = "fd00::2"
        .parse::<std::net::Ipv6Addr>()
        .unwrap()
        .octets()
        .to_vec();
    preamble.extend_from_slice(&62078u32.to_le_bytes());
    let write = tokio::time::timeout(OBSERVE, client.write_all(&preamble)).await;
    let stopped = until(|| !runtime.is_alive()).await;
    let native = Arc::new(crate::TunnelHandle::new_userspace(
        "tx-quota-fixture".into(),
        crate::TunnelInfo {
            server_address: "fd00::2".into(),
            server_rsd_port: 58783,
            client_address: "fd00::1".into(),
            client_mtu: 1280,
        },
        runtime,
    ));
    let retained = Arc::clone(&native);
    let result = native.shutdown(OBSERVE).await;
    let tasks = alive();
    let drops = probe.drops.load(Ordering::SeqCst);
    let second = retained.shutdown(OBSERVE).await;
    let mut byte = [0];
    let peer_eof = tokio::time::timeout(OBSERVE, peer.read(&mut byte)).await;
    let client_eof = tokio::time::timeout(OBSERVE, client.read(&mut byte)).await;
    write.unwrap().unwrap();
    assert!(stopped, "quota refusal did not stop the original runtime");
    assert!(
        tx_phase.load(Ordering::SeqCst),
        "runtime stopped before actual Tx admission"
    );
    for error in [result.unwrap_err(), second.unwrap_err()] {
        assert_eq!(
            crate::classify_error(&error),
            crate::ErrorClass::ResourceLimit
        );
        assert!(report(&error).joins_complete());
        let TunnelError::Io(original) = report(&error)
            .primary()
            .downcast_ref::<TunnelError>()
            .unwrap()
        else {
            panic!("fatal original cause lost");
        };
        assert!(matches!(
            original
                .get_ref()
                .unwrap()
                .downcast_ref::<owned_buffer::BufferAdmission>(),
            Some(owned_buffer::BufferAdmission::Exhausted)
        ));
    }
    assert!(
        matches!(peer_eof, Ok(Ok(0))),
        "strict tunnel EOF missing: {peer_eof:?}"
    );
    assert!(
        matches!(client_eof, Ok(Ok(0))),
        "strict local client EOF missing: {client_eof:?}"
    );
    assert_eq!(tasks, baseline);
    assert_eq!(drops, 1);
    drop(held.lock().unwrap().take());
    assert_eq!(
        bank.available_permits(),
        quota - runtime_owner_bytes(),
        "only actual persistent owner bodies/backing may remain after buffer joins"
    );
    drop(native);
    assert_eq!(bank.available_permits(), quota - runtime_owner_bytes());
    drop(retained);
    assert_eq!(bank.available_permits(), quota);
}
