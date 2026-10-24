use super::*;
use std::pin::Pin;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::task::{Context, Poll};

fn poll<F: Future>(future: Pin<&mut F>) -> Poll<F::Output> {
    future.poll(&mut Context::from_waker(
        futures_util::task::noop_waker_ref(),
    ))
}

#[tokio::test]
async fn remote_epoch_readonly_snapshot_creates_no_device_and_rejects_old_work_after_clear() {
    let lifecycle = Lifecycle::new();
    let bank = lifecycle.metadata_bytes.clone();
    let before = bank.available_permits();
    let epoch = lifecycle.global_epoch().unwrap();
    assert_eq!(lifecycle.coordinator_count().await, 0);
    assert_eq!(bank.available_permits(), before);
    let mut stop = lifecycle
        .prepare_cutover::<3>(lifecycle.byte_bank())
        .unwrap();
    lifecycle.stop("not-a-local-row", &mut stop).await;
    drop(stop);
    assert!(epoch.check().is_ok());
    assert_eq!(lifecycle.coordinator_count().await, 0);
    let mut clear = lifecycle
        .prepare_cutover::<2>(lifecycle.byte_bank())
        .unwrap();
    lifecycle.clear(&mut clear);
    drop(clear);
    let polls = AtomicUsize::new(0);
    let error = epoch
        .run(async {
            polls.fetch_add(1, Ordering::SeqCst);
            Ok(())
        })
        .await
        .unwrap_err();
    assert!(error.is::<ios_core::OperationCancelled>());
    assert_eq!(polls.load(Ordering::SeqCst), 0);
    let next = lifecycle.global_epoch().unwrap();
    assert!(next.run(async { Ok(()) }).await.is_ok());
    let mut close = lifecycle
        .prepare_cutover::<1>(lifecycle.byte_bank())
        .unwrap();
    lifecycle.close(&mut close);
    drop(close);
    assert!(next.check().is_err());
    assert!(lifecycle.global_epoch().is_err());
    assert_eq!(
        bank.available_permits(),
        before - ControlIdentity::body_bytes()
    );
    drop((epoch, next));
    assert_eq!(
        bank.available_permits(),
        before + ControlIdentity::body_bytes()
    );
}

#[tokio::test]
async fn remote_epoch_pending_work_drops_on_clear_close_or_lifecycle_owner_drop() {
    struct Held(Arc<AtomicUsize>);
    impl Drop for Held {
        fn drop(&mut self) {
            self.0.fetch_add(1, Ordering::SeqCst);
        }
    }
    for action in 0..3 {
        let lifecycle = Lifecycle::new();
        let epoch = lifecycle.global_epoch().unwrap();
        let drops = Arc::new(AtomicUsize::new(0));
        let mut clear = (action == 0).then(|| {
            lifecycle
                .prepare_cutover::<2>(lifecycle.byte_bank())
                .unwrap()
        });
        let mut close = (action == 1).then(|| {
            lifecycle
                .prepare_cutover::<1>(lifecycle.byte_bank())
                .unwrap()
        });
        let mut operation = Box::pin(epoch.run(async {
            let _held = Held(drops.clone());
            std::future::pending::<Result<()>>().await
        }));
        assert!(poll(operation.as_mut()).is_pending());
        match action {
            0 => lifecycle.clear(clear.as_mut().unwrap()),
            1 => lifecycle.close(close.as_mut().unwrap()),
            _ => {}
        }
        drop((clear, close));
        drop(lifecycle);
        assert!(operation
            .await
            .unwrap_err()
            .is::<ios_core::OperationCancelled>());
        assert_eq!(drops.load(Ordering::SeqCst), 1);
        assert!(epoch.check().is_err());
        let calls = AtomicUsize::new(0);
        assert!(epoch
            .commit(|| {
                calls.fetch_add(1, Ordering::SeqCst);
                Ok(())
            })
            .is_err());
        assert_eq!(calls.load(Ordering::SeqCst), 0);
    }
}

#[tokio::test]
async fn remote_epoch_ready_original_error_crossing_clear_keeps_typed_pointer() {
    let lifecycle = Lifecycle::new();
    let epoch = lifecycle.global_epoch().unwrap();
    let original = anyhow::Error::new(std::io::Error::new(
        std::io::ErrorKind::BrokenPipe,
        "original-remote-IO",
    ));
    let pointer = original.downcast_ref::<std::io::Error>().unwrap() as *const _ as usize;
    let mut clear = lifecycle
        .prepare_cutover::<2>(lifecycle.byte_bank())
        .unwrap();
    let error = epoch
        .run(async {
            lifecycle.clear(&mut clear);
            Err::<(), _>(original)
        })
        .await
        .unwrap_err();
    drop(clear);
    assert_eq!(
        error.downcast_ref::<std::io::Error>().unwrap() as *const _ as usize,
        pointer
    );
    assert_eq!(
        error.downcast_ref::<std::io::Error>().unwrap().kind(),
        std::io::ErrorKind::BrokenPipe
    );
}

#[tokio::test]
async fn remote_epoch_ready_success_crossing_clear_cannot_commit_a_retired_handoff() {
    struct Stream(Arc<AtomicUsize>);
    impl Drop for Stream {
        fn drop(&mut self) {
            self.0.fetch_add(1, Ordering::SeqCst);
        }
    }
    let lifecycle = Lifecycle::new();
    let epoch = lifecycle.global_epoch().unwrap();
    let drops = Arc::new(AtomicUsize::new(0));
    let mut clear = lifecycle
        .prepare_cutover::<2>(lifecycle.byte_bank())
        .unwrap();
    let ready = epoch
        .run(async {
            lifecycle.clear(&mut clear);
            Ok(Stream(drops.clone()))
        })
        .await
        .unwrap();
    drop(clear);
    let commits = AtomicUsize::new(0);
    let error = epoch
        .commit(|| {
            commits.fetch_add(1, Ordering::SeqCst);
            Ok(ready)
        })
        .err()
        .unwrap();
    assert!(error.is::<ios_core::OperationCancelled>());
    assert_eq!(commits.load(Ordering::SeqCst), 0);
    assert_eq!(drops.load(Ordering::SeqCst), 1);
}

#[test]
fn closed_zero_global_epoch_views_keep_eight_distinct_old_identities_with_physical_fees() {
    const ROUNDS: usize = 8;
    let bytes = Lifecycle::watch_bytes();
    let marker_bytes = ControlIdentity::body_bytes();
    let total = bytes + (2 + 2 * ROUNDS) * marker_bytes;
    let authority = super::super::birth_bank::BirthBank::new(total);
    let bank = authority.raw().clone();
    let lifecycle = Lifecycle::with_metadata(authority.clone()).unwrap();
    let mut cutover = lifecycle.prepare_cutover::<{ 2 * ROUNDS }>(&bank).unwrap();
    assert_eq!(bank.available_permits(), 0);
    bank.close();
    let mut retained: [Option<GlobalEpoch>; ROUNDS] = std::array::from_fn(|_| None);
    let hook = super::super::metadata_vec::on_allocation(|_, _| {
        panic!("closed-zero view/control allocated admitted backing")
    });
    for index in 0..ROUNDS {
        let current = lifecycle.global_epoch().unwrap();
        assert!(current.check().is_ok());
        assert!(Arc::ptr_eq(current.generation.bank(), &bank));
        for old in retained[..index].iter().flatten() {
            assert!(!ControlIdentity::ptr_eq(
                &old.generation,
                &current.generation
            ));
            assert!(old
                .check()
                .unwrap_err()
                .is::<ios_core::OperationCancelled>());
        }
        retained[index] = Some(current);
        lifecycle.clear(&mut cutover);
        assert_eq!(bank.available_permits(), 0);
    }
    drop(hook);
    drop(cutover);
    assert_eq!(lifecycle.devices.try_lock().unwrap().rows.len(), 0);
    drop(lifecycle);
    // All eight old generations are still physically owned by these views.
    // Their shared shutdown watch stays charged; the independent scan watch ends.
    assert_eq!(
        bank.available_permits(),
        watch::body_bytes::<ControlIdentity>() + (ROUNDS + 1) * marker_bytes
    );
    for view in retained.iter().flatten() {
        assert!(view
            .check()
            .unwrap_err()
            .is::<ios_core::OperationCancelled>());
        assert!(ControlIdentity::downgrade(&view.generation)
            .upgrade()
            .is_some());
    }
    let weak: [_; ROUNDS] =
        std::array::from_fn(|index| retained[index].as_ref().unwrap().generation.downgrade());
    drop(retained);
    assert!(weak.iter().all(|identity| identity.upgrade().is_none()));
    assert_eq!(bank.available_permits(), total - ROUNDS * marker_bytes);
    drop(weak);
    assert_eq!(bank.available_permits(), total);
}
