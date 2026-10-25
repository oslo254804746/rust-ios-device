use super::*;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

const TOTAL: usize = 1024 * 1024;

struct CaptureProbe {
    hub: crate::tunnel::owner::phase_outcomes::PhaseOutcomes,
    original: Ledger,
    native: Ledger,
    bank: Arc<Semaphore>,
    cause: Arc<dyn std::error::Error + Send + Sync>,
    captured: Arc<AtomicBool>,
}
impl Drop for CaptureProbe {
    fn drop(&mut self) {
        let report = self
            .hub
            .work_result(&self.original, Some(&self.native), None)
            .unwrap_err();
        self.captured.store(
            self.bank.available_permits() == 0
                && Arc::ptr_eq(report.primary_shared().unwrap(), &self.cause),
            Ordering::SeqCst,
        );
    }
}

#[tokio::test]
async fn actual_blocking_body_captures_original_native_io_before_capture_drop_at_closed_zero() {
    use crate::tunnel::owner::phase_outcomes::{Phase, PhaseOutcomes};
    let socket = tokio::net::TcpSocket::new_v4().unwrap();
    socket.bind("127.0.0.1:0".parse().unwrap()).unwrap();
    let error = tokio::net::TcpStream::connect(socket.local_addr().unwrap())
        .await
        .unwrap_err();
    assert_eq!(error.kind(), std::io::ErrorKind::ConnectionRefused);
    let cause: Arc<dyn std::error::Error + Send + Sync> = Arc::new(error);
    let bank = Arc::new(Semaphore::new(TOTAL));
    let original = Ledger::with_metadata(Some(&bank)).unwrap();
    let native = Ledger::with_metadata(Some(&bank)).unwrap();
    let hub = PhaseOutcomes::new(Some(&bank)).unwrap();
    hub.bind(&original, Phase::Original).unwrap();
    hub.bind(&native, Phase::Native).unwrap();
    let slot = RuntimeRoot::new(BlockingSlot::new(), Some(&bank)).unwrap();
    let captured = Arc::new(AtomicBool::new(false));
    let probe = CaptureProbe {
        hub: hub.clone(),
        original: original.clone(),
        native: native.clone(),
        bank: bank.clone(),
        cause: cause.clone(),
        captured: captured.clone(),
    };
    let worker_ledger = native.clone();
    let worker_cause = cause.clone();
    let prepared = PreparedBlocking::prepare(&bank, || {
        move || {
            worker_ledger.record_work(1, worker_cause);
            drop(probe);
            Ok(())
        }
    })
    .unwrap();
    let held = bank
        .clone()
        .try_acquire_many_owned(bank.available_permits() as u32)
        .unwrap();
    bank.close();
    prepared.start(&slot, ParentRole::Writer, native.clone());
    slot.join().await;
    assert!(captured.load(Ordering::SeqCst));
    let report = hub.status(&original, Some(&native), None).unwrap_err();
    assert!(report.joins_complete());
    assert!(Arc::ptr_eq(report.primary_shared().unwrap(), &cause));
    assert_eq!(report.additional_unretained_outcomes(), 0);
    drop((report, slot, original, native, hub, cause, socket, held));
    assert_eq!(bank.available_permits(), TOTAL);
}

#[tokio::test]
async fn original_blocking_join_precedes_body_and_waiter_drop_keeps_real_call() {
    let bank = Arc::new(Semaphore::new(TOTAL));
    let ledger = Ledger::with_metadata(Some(&bank)).unwrap();
    let slot = RuntimeRoot::new(BlockingSlot::new(), Some(&bank)).unwrap();
    let installed = Arc::new(AtomicBool::new(false));
    let observed = Arc::new(AtomicBool::new(false));
    let release = Arc::new((Mutex::new(false), Condvar::new()));
    let entered = Arc::new(tokio::sync::Notify::new());
    let body_observed = observed.clone();
    let body_installed = installed.clone();
    let body_release = release.clone();
    let body_entered = entered.clone();
    let prepared = PreparedBlocking::prepare(&bank, || {
        move || {
            body_observed.store(body_installed.load(Ordering::Acquire), Ordering::Release);
            body_entered.notify_one();
            let (state, changed) = &*body_release;
            let mut ready = state.lock().unwrap();
            while !*ready {
                ready = changed.wait(ready).unwrap();
            }
            Ok(())
        }
    })
    .unwrap();
    AFTER_INSTALL.with(|hook| {
        *hook.borrow_mut() = Some(Box::new(move || {
            installed.store(true, Ordering::Release);
        }))
    });
    let held = bank
        .clone()
        .try_acquire_many_owned(bank.available_permits() as u32)
        .unwrap();
    bank.close();
    prepared.start(&slot, ParentRole::ForwardOrReader, ledger.clone());
    entered.notified().await;
    assert!(observed.load(Ordering::Acquire));
    assert!(slot.installed() && !slot.is_finished());
    let mut wait = Box::pin(slot.join());
    assert!(futures_util::poll!(&mut wait).is_pending());
    drop(wait);
    assert!(!slot.is_finished());
    {
        let (state, changed) = &*release;
        *state.lock().unwrap() = true;
        changed.notify_one();
    }
    slot.join().await;
    ledger.result(None).unwrap();
    assert!(slot.is_finished());
    drop((slot, ledger, held));
    assert_eq!(bank.available_permits(), TOTAL);
}

#[tokio::test]
async fn synchronous_install_panic_cancels_unstarted_body_but_keeps_actual_join() {
    let bank = Arc::new(Semaphore::new(TOTAL));
    let ledger = Ledger::with_metadata(Some(&bank)).unwrap();
    let slot = RuntimeRoot::new(BlockingSlot::new(), Some(&bank)).unwrap();
    let calls = Arc::new(AtomicUsize::new(0));
    let body_calls = calls.clone();
    let prepared = PreparedBlocking::prepare(&bank, || {
        move || {
            body_calls.fetch_add(1, Ordering::SeqCst);
            Ok(())
        }
    })
    .unwrap();
    AFTER_INSTALL.with(|hook| {
        *hook.borrow_mut() = Some(Box::new(|| {
            panic!("scripted notification unwind after actual join installation")
        }))
    });
    assert!(std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        prepared.start(&slot, ParentRole::ForwardOrReader, ledger.clone());
    }))
    .is_err());
    assert!(slot.installed());
    slot.join().await;
    ledger.result(None).unwrap();
    assert_eq!(calls.load(Ordering::SeqCst), 0);
    drop((slot, ledger));
    assert_eq!(bank.available_permits(), TOTAL);
}

#[test]
fn last_byte_call_refusal_never_constructs_native_body_and_refunds_real_gate() {
    fn bytes<F: FnOnce() -> Result<(), TunnelError>>(_: impl FnOnce() -> F) -> usize {
        RuntimeRoot::<Gate>::body_bytes() + mem::size_of::<Call<F>>()
    }
    let constructed = AtomicUsize::new(0);
    fn work() -> Result<(), TunnelError> {
        Ok(())
    }
    let make = || {
        constructed.fetch_add(1, Ordering::SeqCst);
        work as fn() -> Result<(), TunnelError>
    };
    let amount = bytes(make);
    let bank = Arc::new(Semaphore::new(amount - 1));
    let error = PreparedBlocking::prepare(&bank, make).err().unwrap();
    assert_eq!(
        crate::classify_error(&error),
        crate::ErrorClass::ResourceLimit
    );
    assert_eq!(constructed.load(Ordering::SeqCst), 0);
    assert_eq!(bank.available_permits(), amount - 1);
}
