use super::*;

struct Release(WindowsScript);
impl Drop for Release {
    fn drop(&mut self) {
        self.0.unblock();
    }
}

#[tokio::test]
async fn cancelled_actual_identity_call_stays_owned_until_genuine_join_before_interface_close() {
    let bank = Arc::new(Semaphore::new(TOTAL));
    let script = WindowsScript::new().unwrap();
    script.block(Op::Resolve);
    let _release = Release(script.clone());
    let (handle, mut peer) = candidate(&bank).await;
    let mut activation = Box::pin(activate(&handle, &bank, &script));
    tokio::select! {
        biased;
        result = &mut activation => panic!("blocked identity completed: {result:?}"),
        _ = script.wait_for(Op::Resolve) => {},
    }
    assert_eq!(script.count(Op::Create), 1);
    let held = bank
        .clone()
        .try_acquire_many_owned(bank.available_permits() as u32)
        .unwrap();
    bank.close();
    drop(activation);
    let timed = handle
        .shutdown(Duration::from_millis(20))
        .await
        .unwrap_err();
    let TunnelError::Phases(timed) = timed else {
        panic!("original phase timer")
    };
    assert!(timed.elapsed().is_some());
    assert!(!timed.joins_complete());
    assert_eq!(script.count(Op::Close), 0);
    assert_eq!(script.count(Op::AddAddress), 0);
    script.unblock();
    handle.shutdown(OBSERVE).await.unwrap();
    handle.shutdown_status().unwrap();
    eof(&mut peer).await;
    assert_eq!(script.count(Op::Close), 1);
    assert_eq!(script.count(Op::AddAddress), 0);
    drop((timed, handle, held));
    assert_eq!(bank.available_permits(), TOTAL);
}

fn deep_io(error: &(dyn std::error::Error + 'static)) -> (usize, Option<i32>) {
    let mut current = error;
    let mut observed = None;
    loop {
        if let Some(io) = current.downcast_ref::<std::io::Error>() {
            observed = Some((io as *const _ as usize, io.raw_os_error()));
        }
        match current.source() {
            Some(source) => current = source,
            None => break,
        }
    }
    observed.expect("same original native IO")
}

#[tokio::test]
async fn original_native_status_and_deep_io_stay_identical_after_actual_join_and_close() {
    let bank = Arc::new(Semaphore::new(TOTAL));
    let script = WindowsScript::new().unwrap();
    script.fail(Op::Resolve, 5);
    let (handle, mut peer) = candidate(&bank).await;
    let first = activate(&handle, &bank, &script).await.unwrap_err();
    let TunnelError::Phases(first) = first else {
        panic!("original native phase failure")
    };
    let pointer = first.primary_shared().unwrap().clone();
    let first_io = deep_io(first.primary());
    assert_eq!(first_io.1, Some(5));
    assert_eq!(script.count(Op::Close), 0);
    let held = bank
        .clone()
        .try_acquire_many_owned(bank.available_permits() as u32)
        .unwrap();
    bank.close();
    let later = handle.shutdown(OBSERVE).await.unwrap_err();
    let TunnelError::Phases(later) = later else {
        panic!("same native phase failure after close")
    };
    assert!(later.joins_complete());
    assert!(Arc::ptr_eq(later.primary_shared().unwrap(), &pointer));
    assert_eq!(deep_io(later.primary()), first_io);
    assert_eq!(script.count(Op::Close), 1);
    eof(&mut peer).await;
    drop((first, later, pointer, handle, held));
    assert_eq!(bank.available_permits(), TOTAL);
}

#[tokio::test]
async fn tentative_native_address_uses_original_elapsed_and_never_seals_ready() {
    let bank = Arc::new(Semaphore::new(TOTAL));
    let script = WindowsScript::new().unwrap();
    script.tentative(true);
    let (handle, mut peer) = candidate(&bank).await;
    let error = handle
        .finish_prepared_kernel_with(
            bank.clone(),
            Instant::now() + Duration::from_millis(80),
            |runtime, bank, info, deadline| {
                runtime.prepare_kernel_scripted(bank, info, deadline, &script)
            },
        )
        .await
        .unwrap_err();
    let TunnelError::Phases(report) = error else {
        panic!("one original activation timer")
    };
    assert!(report.primary().is::<tokio::time::error::Elapsed>());
    assert!(report.elapsed().is_some());
    assert!(!report.has_retained_outcomes());
    assert!(!handle.ready_for_publication());
    handle.shutdown(OBSERVE).await.unwrap();
    eof(&mut peer).await;
    drop((report, handle));
    assert_eq!(bank.available_permits(), TOTAL);
}

#[tokio::test]
async fn timed_out_actual_undo_call_keeps_its_join_and_fresh_interface_until_retry() {
    let bank = Arc::new(Semaphore::new(TOTAL));
    let script = WindowsScript::new().unwrap();
    script.automatic_route(false);
    let (handle, mut peer) = candidate(&bank).await;
    activate(&handle, &bank, &script).await.unwrap();
    script.block(Op::RemoveRoute);
    let _release = Release(script.clone());
    let held = bank
        .clone()
        .try_acquire_many_owned(bank.available_permits() as u32)
        .unwrap();
    bank.close();
    let mut closing = Box::pin(handle.shutdown(Duration::from_millis(50)));
    tokio::select! {
        biased;
        result = &mut closing => panic!("undo did not start before original shutdown budget: {result:?}"),
        _ = script.wait_for(Op::RemoveRoute) => {},
    }
    let error = closing.await.unwrap_err();
    let TunnelError::Phases(report) = error else {
        panic!("same original shutdown elapsed")
    };
    assert!(report.elapsed().is_some());
    assert_eq!(report.pending_joins(), Some(1));
    assert_eq!(script.count(Op::Close), 0);
    assert_eq!(script.count(Op::RemoveAddress), 0);
    script.unblock();
    handle.shutdown(OBSERVE).await.unwrap();
    handle.shutdown_status().unwrap();
    assert_eq!(script.count(Op::RemoveRoute), 1);
    assert_eq!(script.count(Op::RemoveAddress), 1);
    assert_eq!(script.count(Op::Close), 1);
    assert_eq!(report.pending_joins(), Some(1));
    eof(&mut peer).await;
    drop((report, handle, held));
    assert_eq!(bank.available_permits(), TOTAL);
}
