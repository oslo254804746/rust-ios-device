use super::*;
use crate::tunnel::tun::userspace::{
    ConnState, RuntimeStorage, UserspaceTunDevice, IPV6_HEADER_LEN,
};
use smoltcp::iface::{Config, Interface, SocketHandle};
use smoltcp::socket::tcp;
use smoltcp::wire::{HardwareAddress, IpAddress, IpEndpoint, Ipv6Address};
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

fn socket(
    port: u16,
    device: &mut crate::tunnel::tun::userspace::ChannelDevice,
) -> tcp::Socket<'static> {
    let mut interface = Interface::new(
        Config::new(HardwareAddress::Ip),
        device,
        super::super::smol_now(),
    );
    let mut socket = tcp::Socket::new(
        tcp::SocketBuffer::new(vec![0; 64]),
        tcp::SocketBuffer::new(vec![0; 64]),
    );
    socket
        .connect(
            interface.context(),
            IpEndpoint::new(IpAddress::Ipv6(Ipv6Address::LOOPBACK), port),
            IpEndpoint::new(IpAddress::Ipv6(Ipv6Address::LOOPBACK), port + 1),
        )
        .unwrap();
    socket
}

fn add(
    slots: &mut SocketSlots,
    device: &mut crate::tunnel::tun::userspace::ChannelDevice,
    port: u16,
) -> SocketHandle {
    slots.prepare_add().unwrap();
    slots.with_set(|set| set.add(socket(port, device)))
}

#[test]
fn socket_storage_exact_backing_and_last_owner_drop_refund() {
    let bytes = 4 * mem::size_of::<SocketStorage<'static>>();
    let bank = Arc::new(Semaphore::new(bytes));
    let mut slots = SocketSlots::new(Some(&bank));
    slots.prepare_add().unwrap();
    assert_eq!(slots.slots.capacity(), 4);
    assert_eq!(slots.backing_bytes(), bytes);
    assert_eq!(bank.available_permits(), 0);
    let mut device = super::super::ChannelDevice::new(
        super::super::ValidatedMtu::new(1280).unwrap(),
        RuntimeStorage::unmetered(),
    );
    let handle = add(&mut slots, &mut device, 10001);
    slots.with_set(|set| {
        assert_eq!(
            set.get::<tcp::Socket>(handle)
                .remote_endpoint()
                .unwrap()
                .port,
            10001
        )
    });
    assert_eq!(bank.available_permits(), 0);
    drop(slots);
    assert_eq!(bank.available_permits(), bytes);
}

#[test]
fn socket_storage_growth_preserves_full_original_meta_handles_and_transport_state() {
    let old_bytes = 4 * mem::size_of::<SocketStorage<'static>>();
    let new_bytes = old_bytes * 2;
    for shortage in [0, 1] {
        let bank = Arc::new(Semaphore::new(old_bytes + new_bytes - shortage));
        let mut slots = SocketSlots::new(Some(&bank));
        let mut device = super::super::ChannelDevice::new(
            super::super::ValidatedMtu::new(1280).unwrap(),
            RuntimeStorage::unmetered(),
        );
        let handles = (10001..10005)
            .map(|port| add(&mut slots, &mut device, port))
            .collect::<Vec<_>>();
        let originals = format!("{:?}", &slots.slots[..4]);
        let pointer = slots.slots.as_ptr();
        let result = slots.prepare_add();
        if shortage == 1 {
            let error = result.unwrap_err();
            assert_eq!(
                crate::classify_error(&error),
                crate::ErrorClass::ResourceLimit
            );
            assert_eq!(slots.slots.as_ptr(), pointer);
            assert_eq!(bank.available_permits(), new_bytes - 1);
        } else {
            result.unwrap();
            assert_eq!(slots.slots.capacity(), 8);
            assert_eq!(bank.available_permits(), old_bytes);
        }
        // SocketStorage's Debug includes its original private Meta, not just
        // the public tcp state; moving whole slots must preserve it exactly.
        assert_eq!(format!("{:?}", &slots.slots[..4]), originals);
        slots.with_set(|set| {
            for (index, handle) in handles.iter().enumerate() {
                let socket = set.get::<tcp::Socket>(*handle);
                assert_eq!(socket.state(), tcp::State::SynSent);
                assert_eq!(socket.remote_endpoint().unwrap().port, 10001 + index as u16);
                assert_eq!(socket.recv_capacity(), 64);
                assert_eq!(socket.send_capacity(), 64);
            }
        });
        drop(slots);
        assert_eq!(bank.available_permits(), old_bytes + new_bytes - shortage);
    }
}

#[test]
fn socket_storage_vacant_slot_reuses_original_handle_without_quota_or_reallocation() {
    let bank = Arc::new(Semaphore::new(4 * mem::size_of::<SocketStorage<'static>>()));
    let mut slots = SocketSlots::new(Some(&bank));
    let mut device = super::super::ChannelDevice::new(
        super::super::ValidatedMtu::new(1280).unwrap(),
        RuntimeStorage::unmetered(),
    );
    let handles = (10001..10005)
        .map(|port| add(&mut slots, &mut device, port))
        .collect::<Vec<_>>();
    let pointer = slots.slots.as_ptr();
    slots.with_set(|set| drop(set.remove(handles[1])));
    assert_eq!(bank.available_permits(), 0);
    let replacement = add(&mut slots, &mut device, 11001);
    assert_eq!(replacement, handles[1]);
    assert_eq!(slots.slots.as_ptr(), pointer);
    assert_eq!(bank.available_permits(), 0);
    drop(slots);
    assert_eq!(
        bank.available_permits(),
        4 * mem::size_of::<SocketStorage<'static>>()
    );
}

#[tokio::test]
async fn socket_storage_actual_runtime_last_byte_refusal_keeps_typed_fatal_and_original_joins() {
    let baseline = tokio::runtime::Handle::current()
        .metrics()
        .num_alive_tasks();
    let bytes = IPV6_HEADER_LEN
        + super::super::runtime_channel_bytes()
        + super::super::client_channel_bytes()
        + 4 * mem::size_of::<(SocketHandle, ConnState)>()
        + 4 * mem::size_of::<SocketStorage<'static>>()
        - 1;
    let bank = Arc::new(Semaphore::new(bytes));
    let (stream, mut peer) = tokio::io::duplex(1280);
    let runtime = UserspaceTunDevice::start_validated_with_bank(
        "fd00::2",
        "fd00::1",
        super::super::ValidatedMtu::new(1280).unwrap(),
        stream,
        Some(Arc::clone(&bank)),
    )
    .await
    .unwrap();
    let mut client = tokio::net::TcpStream::connect(("127.0.0.1", runtime.local_port))
        .await
        .unwrap();
    let mut preamble = [0; 20];
    preamble[..16].copy_from_slice(&"fd00::1".parse::<std::net::Ipv6Addr>().unwrap().octets());
    preamble[16..].copy_from_slice(&62078u32.to_le_bytes());
    client.write_all(&preamble).await.unwrap();
    tokio::time::timeout(Duration::from_secs(2), async {
        while runtime.is_alive() {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    let error = runtime.shutdown(Duration::from_secs(2)).await.unwrap_err();
    assert_eq!(
        crate::classify_error(&error),
        crate::ErrorClass::ResourceLimit
    );
    assert!(runtime.shutdown_status().is_err());
    let mut byte = [0];
    assert!(matches!(
        tokio::time::timeout(Duration::from_secs(2), peer.read(&mut byte)).await,
        Ok(Ok(0))
    ));
    assert!(matches!(
        tokio::time::timeout(Duration::from_secs(2), client.read(&mut byte)).await,
        Ok(Ok(0))
    ));
    drop(runtime);
    tokio::time::timeout(Duration::from_secs(2), async {
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
    assert_eq!(bank.available_permits(), bytes);
}
