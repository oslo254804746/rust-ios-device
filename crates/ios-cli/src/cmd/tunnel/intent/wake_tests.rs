use super::super::{birth_bank::BirthBank, marker_identity::ReleaseFence, metadata_vec};
use super::*;
use std::future::Future;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::task::{Context, Poll, Wake, Waker};

struct CellWake {
    cell: Arc<Cell>,
    outer: Arc<Mutex<()>>,
    calls: AtomicUsize,
    unlocked: AtomicBool,
}

impl Wake for CellWake {
    fn wake(self: Arc<Self>) {
        self.wake_by_ref();
    }

    fn wake_by_ref(self: &Arc<Self>) {
        let unlocked = self.outer.try_lock().is_ok()
            && self.cell.state.try_lock().is_ok()
            && self.cell.pending_success.try_lock().is_ok()
            && self.cell.failure.try_lock().is_ok()
            && self.cell.changes.value_is_unlocked();
        self.unlocked.fetch_and(unlocked, Ordering::SeqCst);
        self.calls.fetch_add(1, Ordering::SeqCst);
    }
}

fn observer(cell: &Arc<Cell>, outer: &Arc<Mutex<()>>) -> Arc<CellWake> {
    Arc::new(CellWake {
        cell: cell.clone(),
        outer: outer.clone(),
        calls: AtomicUsize::new(0),
        unlocked: AtomicBool::new(true),
    })
}

#[test]
fn checked_replace_rollback_refunds_its_actual_marker_after_cell_and_outer_guards() {
    let marker = ControlIdentity::body_bytes();
    let total = watch::body_bytes::<ControlIdentity>() + 3 * marker;
    let origin = BirthBank::new(total);
    let bank = origin.raw().clone();
    let mut seed = Cutover::<1>::new(&origin, &bank).unwrap();
    let cell = Arc::new(Cell::new(seed.place(&origin), origin.clone()).unwrap());
    drop(seed);
    let original = cell.snapshot();
    let mut cutover = cell.prepare_births::<1>(&bank).unwrap();
    let outer = Arc::new(Mutex::new(()));
    let observed = observer(&cell, &outer);
    let waker = Waker::from(observed.clone());
    let mut cx = Context::from_waker(&waker);
    let mut waiter = std::pin::pin!(bank.clone().acquire_many_owned(marker as u32));
    assert!(!bank.is_closed());
    assert_eq!(bank.available_permits(), 0);
    assert!(waiter.as_mut().poll(&mut cx).is_pending());
    {
        let _held = outer.lock().unwrap();
        let checks = AtomicUsize::new(0);
        let error = metadata_vec::with_allocation_hook(
            |_, _| panic!("prepared Cell rollback allocated a backing"),
            || {
                cell.checked_replace(
                    || Ok(Desired::Stopped),
                    None,
                    || {
                        if checks.fetch_add(1, Ordering::SeqCst) == 0 {
                            Ok(())
                        } else {
                            Err(anyhow::anyhow!("original late Cell check"))
                        }
                    },
                    &mut cutover,
                )
                .unwrap_err()
            },
        );
        assert_eq!(error.to_string(), "original late Cell check");
        assert_eq!(checks.load(Ordering::SeqCst), 2);
        let current = cell.snapshot();
        assert!(ControlIdentity::ptr_eq(&original.epoch, &current.epoch));
        assert!(ControlIdentity::ptr_eq(&original.version, &current.version));
        assert!(matches!(current.desired, Desired::Automatic));
        assert_eq!(bank.available_permits(), 0);
        assert_eq!(observed.calls.load(Ordering::SeqCst), 0);
    }
    drop(cutover);
    assert!(observed.calls.load(Ordering::SeqCst) > 0);
    assert!(observed.unlocked.load(Ordering::SeqCst));
    let permit = match waiter.as_mut().poll(&mut cx) {
        Poll::Ready(Ok(permit)) => permit,
        _ => panic!("rolled-back physical marker did not refund its exact body"),
    };
    assert_eq!(permit.num_permits(), marker);
    drop((permit, original, observed, waker, cell));
    assert_eq!(bank.available_permits(), total);
}

#[test]
fn failure_marker_payload_ends_in_original_assignment_but_refunds_outside_cell_guards() {
    let marker = ControlIdentity::body_bytes();
    let total = watch::body_bytes::<ControlIdentity>() + 3 * marker + FailureVersion::body_bytes();
    let origin = BirthBank::new(total);
    let bank = origin.raw().clone();
    let mut seed = Cutover::<2>::new(&origin, &bank).unwrap();
    let epoch = seed.place(&origin);
    let desired = seed.place(&origin);
    let weak = desired.downgrade();
    let cell = Arc::new(Cell::new(epoch, origin.clone()).unwrap());
    // Retain the existing FailureVersion so only the marker refund can wake
    // this waiter. This fixture does not change that separate fee's scope.
    let failure = FailureVersion::new(&bank).unwrap();
    let mut setup = ReleaseFence::<1>::new();
    cell.set_failure(Some((failure.clone(), desired)), &mut setup);
    drop((setup, seed));
    let mut releases = ReleaseFence::<1>::new();
    let outer = Arc::new(Mutex::new(()));
    let observed = observer(&cell, &outer);
    let waker = Waker::from(observed.clone());
    let mut cx = Context::from_waker(&waker);
    let mut waiter = std::pin::pin!(bank.clone().acquire_many_owned(marker as u32));
    assert!(!bank.is_closed());
    assert_eq!(bank.available_permits(), 0);
    assert!(waiter.as_mut().poll(&mut cx).is_pending());
    {
        let _held = outer.lock().unwrap();
        metadata_vec::with_allocation_hook(
            |_, _| panic!("Cell failure cleanup allocated a backing"),
            || cell.set_failure(None, &mut releases),
        );
        assert!(cell.failure().is_none());
        assert!(
            weak.upgrade().is_none(),
            "original payload ends at assignment"
        );
        drop(weak);
        assert_eq!(bank.available_permits(), 0);
        assert_eq!(observed.calls.load(Ordering::SeqCst), 0);
    }
    drop(releases);
    assert!(observed.calls.load(Ordering::SeqCst) > 0);
    assert!(observed.unlocked.load(Ordering::SeqCst));
    let permit = match waiter.as_mut().poll(&mut cx) {
        Poll::Ready(Ok(permit)) => permit,
        _ => panic!("retired failure marker did not refund its exact body"),
    };
    assert_eq!(permit.num_permits(), marker);
    drop((permit, failure, observed, waker, cell));
    assert_eq!(bank.available_permits(), total);
}
