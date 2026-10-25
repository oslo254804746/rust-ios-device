use super::*;
use crate::tunnel::tun::userspace::{ScriptOperation as Op, WindowsScript};

#[tokio::test]
async fn real_two_private_rsd_directories_promote_same_handle_and_catalogue_with_scripted_native_ops(
) {
    let baseline = tokio::runtime::Handle::current()
        .metrics()
        .num_alive_tasks();
    let bank = Arc::new(Semaphore::new(TOTAL));
    let owner = Owner {
        bank: bank.clone(),
        mode: TunMode::Kernel,
        attached: Mutex::new(None),
        completions: AtomicUsize::new(0),
        attached_dormant: Mutex::new(None),
    };
    let (stream, peer) = tokio::io::duplex(97);
    let peer_task = tokio::spawn(remote(peer, UID));
    let deadline = Instant::now() + OBSERVE;
    let prepared = prepare_tunnel(
        connection(&bank),
        stream,
        TunMode::Kernel,
        Some(owner.reserve_owned().unwrap()),
        Prepared {
            deadline,
            bank: &bank,
        },
    )
    .await
    .unwrap();
    let handle = owner.attached.lock().unwrap().as_ref().unwrap().clone();
    let checked = prepared.current_rsd_catalogue().clone();
    let catalogue_pointer = std::ptr::from_ref(&*checked) as usize;
    let info_pointer = prepared.info().udid.as_ptr();
    let initial_pointer = prepared.rsd().unwrap().udid().as_ptr();
    assert!(!handle.ready_for_publication());
    let script = WindowsScript::new().unwrap();
    let finished = prepared
        .finish_kernel_scripted_at(deadline + Duration::from_secs(3600), &script)
        .await;
    let ready = finished.as_ref().ok().is_some_and(|device| {
        Arc::ptr_eq(device.tunnel_handle().unwrap(), &handle)
            && handle.ready_for_publication()
            && handle.userspace_port.is_none()
            && device.prepared_rsd_owner().unwrap().ptr_eq(&checked)
            && device.info().udid.as_ptr() == info_pointer
            && device.rsd().unwrap().udid().as_ptr() == initial_pointer
            && device
                .current_rsd_catalogue()
                .unwrap()
                .service_port(NEW_SERVICE)
                == Some(23456)
    });
    // Always recover actual joins, including when a readiness assertion fails.
    let cleanup = handle.shutdown(OBSERVE).await;
    let peer_report = tokio::time::timeout(OBSERVE, peer_task)
        .await
        .unwrap()
        .unwrap();
    cleanup.unwrap();
    assert!(
        ready,
        "same checked owner failed native promotion: {:?}",
        finished.as_ref().err()
    );
    assert_eq!(script.count(Op::Create), 1);
    assert_eq!(script.count(Op::RemoveAddress), 1);
    assert_eq!(script.count(Op::EndSession), 1);
    assert_eq!(script.count(Op::Close), 1);
    assert!(peer_report.literal_eof);
    assert_eq!(std::ptr::from_ref(&*checked) as usize, catalogue_pointer);
    assert_eq!(owner.completions.load(Ordering::SeqCst), 1);
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
    drop((finished, handle, owner));
    assert_eq!(bank.available_permits(), TOTAL - checked.known_bytes());
    drop(checked);
    assert_eq!(bank.available_permits(), TOTAL);
}
