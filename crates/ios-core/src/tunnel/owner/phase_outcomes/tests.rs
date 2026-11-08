use super::*;
use crate::tunnel::owner::{OwnedTask, ParentRole};
use std::sync::atomic::AtomicBool;
use std::time::Duration;

const TOTAL: usize = 1024 * 1024;

#[tokio::test]
async fn real_join_and_closed_bank_phase_reports_keep_first_arc_and_frozen_progress() {
    let bank = Arc::new(Semaphore::new(TOTAL));
    let original = Ledger::with_metadata(Some(&bank)).unwrap();
    let native = Ledger::with_metadata(Some(&bank)).unwrap();
    let undo = Ledger::with_metadata(Some(&bank)).unwrap();
    let hub = PhaseOutcomes::new(Some(&bank)).unwrap();
    for (ledger, phase) in [
        (&original, Phase::Original),
        (&native, Phase::Native),
        (&undo, Phase::Undo),
    ] {
        hub.bind(ledger, phase).unwrap();
    }
    let gate = Arc::new(tokio::sync::Notify::new());
    let task_gate = gate.clone();
    let task = OwnedTask::new(
        tokio::spawn(async move {
            task_gate.notified().await;
            Ok(())
        }),
        ParentRole::ForwardOrReader,
        original.clone(),
    );
    hub.work_result(&original, Some(&native), Some(&undo))
        .unwrap();
    let running = hub
        .status(&original, Some(&native), Some(&undo))
        .unwrap_err();
    assert!(!running.has_retained_outcomes());
    assert_eq!(running.pending_joins(), Some(1));
    assert!(!running.joins_complete());
    let socket = tokio::net::TcpSocket::new_v4().unwrap();
    socket.bind("127.0.0.1:0".parse().unwrap()).unwrap();
    let addr = socket.local_addr().unwrap();
    #[cfg(target_os = "macos")]
    let socket = {
        drop(socket);
    };
    let error = tokio::net::TcpStream::connect(addr).await.unwrap_err();
    assert_eq!(error.kind(), std::io::ErrorKind::ConnectionRefused);
    let first: Cause = Arc::new(error);
    let cleanup: Cause = Arc::new(std::io::Error::from_raw_os_error(5));
    let held = bank
        .clone()
        .try_acquire_many_owned(bank.available_permits() as u32)
        .unwrap();
    bank.close();
    native.record_work(0, first.clone());
    let acquired = hub
        .work_result(&original, Some(&native), Some(&undo))
        .unwrap_err();
    assert!(Arc::ptr_eq(acquired.primary_shared().unwrap(), &first));
    assert_eq!(acquired.cleanup().count(), 0);
    assert_eq!(acquired.pending_joins(), Some(1));
    // Local zero sequence in another ledger is later than the native cause.
    original.record_work(1, Arc::new(std::io::Error::from_raw_os_error(13)));
    undo.record_work(0, cleanup.clone());
    gate.notify_one();
    task.join().await;
    let ended = hub
        .status(&original, Some(&native), Some(&undo))
        .unwrap_err();
    assert!(Arc::ptr_eq(ended.primary_shared().unwrap(), &first));
    assert_eq!(ended.pending_joins(), Some(0));
    assert!(ended.joins_complete());
    assert_eq!(acquired.pending_joins(), Some(1));
    assert_eq!(acquired.cleanup().count(), 0);
    let later = ended.cleanup().collect::<Vec<_>>();
    assert!(later
        .iter()
        .any(|value| matches!(value, PhaseCleanup::Cause(cause) if Arc::ptr_eq(cause, &cleanup))));
    assert_eq!(bank.available_permits(), 0);
    drop(later);
    drop((
        ended, acquired, running, task, original, native, undo, hub, first, cleanup, socket, held,
    ));
    assert_eq!(bank.available_permits(), TOTAL);
}

struct Rejected {
    hub: PhaseOutcomes,
    ledger: Ledger,
    unlocked: Arc<AtomicBool>,
}
impl fmt::Debug for Rejected {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("rejected phase cause")
    }
}
impl fmt::Display for Rejected {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("rejected phase cause")
    }
}
impl Error for Rejected {}
impl Drop for Rejected {
    fn drop(&mut self) {
        self.unlocked.store(
            self.hub.body().state.try_lock().is_ok() && self.ledger.outcomes.try_lock().is_ok(),
            Ordering::SeqCst,
        );
    }
}

#[test]
fn rejected_phase_cause_drops_after_both_guards_and_preserves_exact_original() {
    let ledger = Ledger::new();
    let hub = PhaseOutcomes::new(None).unwrap();
    hub.bind(&ledger, Phase::Original).unwrap();
    let cause: Cause = Arc::new(std::io::Error::from_raw_os_error(5));
    ledger.record_work(0, cause.clone());
    let unlocked = Arc::new(AtomicBool::new(false));
    ledger.record_work(
        0,
        Arc::new(Rejected {
            hub: hub.clone(),
            ledger: ledger.clone(),
            unlocked: unlocked.clone(),
        }),
    );
    assert!(unlocked.load(Ordering::SeqCst));
    let report = hub.work_result(&ledger, None, None).unwrap_err();
    assert!(Arc::ptr_eq(report.primary_shared().unwrap(), &cause));
    assert_eq!(report.additional_unretained_outcomes(), 1);
    assert_eq!(
        report.snapshot().phases().next().unwrap().records().count(),
        1
    );
}

#[tokio::test]
async fn original_real_join_panic_is_the_shared_primary_before_later_undo_error() {
    let original = Ledger::new();
    let native = Ledger::new();
    let undo = Ledger::new();
    let hub = PhaseOutcomes::new(None).unwrap();
    for (ledger, phase) in [
        (&original, Phase::Original),
        (&native, Phase::Native),
        (&undo, Phase::Undo),
    ] {
        hub.bind(ledger, phase).unwrap();
    }
    let task = OwnedTask::new(
        tokio::spawn(async {
            std::panic::panic_any("scripted native blocking job panic");
            #[allow(unreachable_code)]
            Ok(())
        }),
        ParentRole::ForwardOrReader,
        native.clone(),
    );
    task.join().await;
    let report = hub
        .status(&original, Some(&native), Some(&undo))
        .unwrap_err();
    let first = report
        .primary()
        .downcast_ref::<tokio::task::JoinError>()
        .unwrap();
    assert!(first.is_panic());
    let first_pointer = first as *const _;
    undo.record_work(0, Arc::new(std::io::Error::from_raw_os_error(5)));
    let repeated = hub
        .status(&original, Some(&native), Some(&undo))
        .unwrap_err();
    assert_eq!(
        repeated
            .primary()
            .downcast_ref::<tokio::task::JoinError>()
            .unwrap() as *const _,
        first_pointer
    );
    assert_eq!(report.cleanup().count(), 0);
    assert_eq!(repeated.cleanup().count(), 1);
    assert!(repeated.joins_complete());
}

#[tokio::test]
async fn phase_deadline_keeps_original_elapsed_and_never_turns_pending_work_into_success() {
    let original = Ledger::new();
    let hub = PhaseOutcomes::new(None).unwrap();
    hub.bind(&original, Phase::Original).unwrap();
    let elapsed = tokio::time::timeout(Duration::ZERO, std::future::pending::<()>())
        .await
        .unwrap_err();
    let error = hub
        .result(&original, None, None, Some(elapsed))
        .unwrap_err();
    assert!(error.primary().is::<Elapsed>());
    assert!(error.elapsed().is_some());
    assert!(!error.has_retained_outcomes());
}

#[test]
fn phase_binding_refuses_preexisting_outcome_instead_of_inventing_order() {
    let original = Ledger::new();
    let hub = PhaseOutcomes::new(None).unwrap();
    original.record_work(0, Arc::new(std::io::Error::from_raw_os_error(5)));
    assert_eq!(
        hub.bind(&original, Phase::Original).unwrap_err(),
        BindError::OutcomePrecedesBinding
    );
    let error = hub.work_result(&original, None, None).unwrap_err();
    assert!(error.has_retained_outcomes());
    assert!(!error.snapshot().binding_valid());
    assert_eq!(
        error
            .primary()
            .downcast_ref::<std::io::Error>()
            .unwrap()
            .raw_os_error(),
        Some(5)
    );
}

#[tokio::test(start_paused = true)]
async fn phase_join_deadline_does_not_poll_expired_collector_or_lose_original_cause() {
    let bank = Arc::new(Semaphore::new(TOTAL));
    let original = Ledger::with_metadata(Some(&bank)).unwrap();
    let native = Ledger::with_metadata(Some(&bank)).unwrap();
    let undo = Ledger::with_metadata(Some(&bank)).unwrap();
    let hub = PhaseOutcomes::new(Some(&bank)).unwrap();
    for (ledger, phase) in [
        (&original, Phase::Original),
        (&native, Phase::Native),
        (&undo, Phase::Undo),
    ] {
        hub.bind(ledger, phase).unwrap();
    }
    let gate = Arc::new(tokio::sync::Notify::new());
    let job_gate = gate.clone();
    let task = OwnedTask::new(
        tokio::spawn(async move {
            job_gate.notified().await;
            Ok(())
        }),
        ParentRole::ForwardOrReader,
        native.clone(),
    );
    let first: Cause = Arc::new(std::io::Error::from_raw_os_error(5));
    native.record_work(0, first.clone());
    let held = bank
        .clone()
        .try_acquire_many_owned(bank.available_permits() as u32)
        .unwrap();
    bank.close();
    let polled = AtomicBool::new(false);
    let error = join_until(Ok(Instant::now()), &hub, &original, &native, &undo, async {
        polled.store(true, Ordering::SeqCst);
        task.join().await;
    })
    .await
    .unwrap_err();
    let TunnelError::Phases(report) = error else {
        panic!("same phase report");
    };
    assert!(!polled.load(Ordering::SeqCst));
    assert!(Arc::ptr_eq(report.primary_shared().unwrap(), &first));
    assert!(report.elapsed().is_some());
    assert_eq!(report.pending_joins(), Some(1));
    gate.notify_one();
    let error = join_until(
        Ok(Instant::now() + Duration::from_secs(1)),
        &hub,
        &original,
        &native,
        &undo,
        task.join(),
    )
    .await
    .unwrap_err();
    let TunnelError::Phases(recovered) = error else {
        panic!("same phase report after recovery");
    };
    assert!(recovered.elapsed().is_none());
    assert!(recovered.joins_complete());
    assert!(Arc::ptr_eq(recovered.primary_shared().unwrap(), &first));
    assert_eq!(report.pending_joins(), Some(1));
    assert_eq!(bank.available_permits(), 0);
    drop((
        report, recovered, task, original, native, undo, hub, first, held,
    ));
    assert_eq!(bank.available_permits(), TOTAL);
}
