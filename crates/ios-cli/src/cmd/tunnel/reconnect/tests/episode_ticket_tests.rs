use super::*;
use crate::cmd::tunnel::{metadata_vec, shared_weak_allocation};
use std::error::Error;
use std::sync::atomic::{AtomicUsize, Ordering};

const TOTAL: usize = 128 * 1024;

#[tokio::test]
async fn episode_ticket_actual_root_refusal_and_one_allocation_keep_original_captured_data() {
    let origin = Arc::new(Semaphore::new(TOTAL));
    let lifecycle = Lifecycle::with_metadata(origin.clone()).unwrap();
    let ticket = lifecycle.ticket(A).await.unwrap().fenced();
    let bytes = TicketOwner::body_bytes();
    let before = origin.available_permits();
    for closed in [false, true] {
        let available = if closed { bytes } else { bytes - 1 };
        let bank = Arc::new(Semaphore::new(available));
        if closed {
            bank.close();
        }
        let hook = metadata_vec::on_allocation(|_, _| panic!("unpaid Ticket root allocated"));
        let error = TicketOwner::new(&ticket, &bank).err().unwrap();
        drop(hook);
        assert_eq!(
            crate::output::operation_error_class(error.as_ref()),
            ErrorClass::ResourceLimit
        );
        assert_eq!(bank.available_permits(), available);
        assert_eq!(origin.available_permits(), before);
    }
    let held = origin
        .clone()
        .try_acquire_many_owned((before - bytes + 1) as u32)
        .unwrap();
    let bank = Arc::new(Semaphore::new(bytes));
    let hook = metadata_vec::on_allocation(|_, _| panic!("origin-refused Ticket root allocated"));
    assert!(TicketOwner::new(&ticket, &bank).is_err());
    drop(hook);
    assert_eq!(bank.available_permits(), bytes);
    assert_eq!(origin.available_permits(), bytes - 1);
    drop(held);

    let held = origin
        .clone()
        .try_acquire_many_owned((before - bytes) as u32)
        .unwrap();
    let original_key = ticket.udid().as_ptr();
    let address = Arc::new(AtomicUsize::new(0));
    let seen = address.clone();
    let charged_a = origin.clone();
    let charged_b = bank.clone();
    let hook = metadata_vec::on_allocation(move |layout, pointer| {
        assert_eq!(layout.size(), bytes);
        assert_eq!(charged_a.available_permits(), 0);
        assert_eq!(charged_b.available_permits(), 0);
        assert_eq!(seen.swap(pointer, Ordering::SeqCst), 0);
    });
    let owner = TicketOwner::new(&ticket, &bank).unwrap();
    drop(hook);
    let value = &*owner as *const Ticket as usize;
    let pointer = address.load(Ordering::SeqCst);
    assert!(pointer <= value && value + std::mem::size_of::<Ticket>() <= pointer + bytes);
    assert_eq!(owner.udid().as_ptr(), original_key);
    assert!(owner.same_incarnation(&ticket));
    let kept = owner.clone();
    assert!(kept.ptr_eq(&owner));
    origin.close();
    bank.close();
    drop((owner, ticket, lifecycle, held));
    assert_eq!(kept.udid().as_ptr(), original_key);
    let last_credit = Arc::new(AtomicUsize::new(0));
    let observed = last_credit.clone();
    let a = origin.clone();
    let b = bank.clone();
    shared_weak_allocation::after_free(
        move || {
            // Child watch allocations may free first. The Ticket root is last.
            assert!(a.available_permits() <= TOTAL - bytes);
            assert_eq!(b.available_permits(), 0);
            observed.store(a.available_permits(), Ordering::SeqCst);
        },
        || drop(kept),
    );
    assert_eq!(last_credit.load(Ordering::SeqCst), TOTAL - bytes);
    assert_eq!(origin.available_permits(), TOTAL);
    assert_eq!(bank.available_permits(), bytes);
}

#[tokio::test]
async fn episode_ticket_into_inner_keeps_real_root_after_outer_box_free_without_new_allocation() {
    let origin = Arc::new(Semaphore::new(TOTAL));
    let lifecycle = Lifecycle::with_metadata(origin.clone()).unwrap();
    let ticket = lifecycle.ticket(A).await.unwrap().fenced();
    let bank = Arc::new(Semaphore::new(TOTAL));
    let owner = TicketOwner::new(&ticket, &bank).unwrap();
    let original = &*owner as *const Ticket as usize;
    let canonical = owner.udid().as_ptr();
    let admitted = slot(&bank);
    let version = FailureVersion::new(&bank).unwrap();
    let node = Removed::episode(
        A,
        &buffered_identity(),
        |identity| Episode {
            identity,
            ticket: owner,
            slot: admitted,
            version,
            snapshot: Snapshot {
                schedule: ScheduleProvenance::Episode,
                phase: Phase::Connecting,
                attempts: 0,
                failures: 0,
                first_attempt: Instant::now(),
                deadline: None,
                next_attempt: None,
                last_class: None,
                sequence: 0,
            },
            pending_event: None,
        },
        &bank,
    )
    .unwrap();
    drop((ticket, lifecycle));
    let a_before = origin.available_permits();
    let b_before = bank.available_permits();
    let outer = node.bytes_in(&bank).unwrap();
    let hook = metadata_vec::on_allocation(|_, _| panic!("Removed into_inner allocated"));
    let value = node.into_inner();
    drop(hook);
    assert_eq!(bank.available_permits(), b_before + outer);
    assert_eq!(origin.available_permits(), a_before);
    let row = value.row.as_ref().unwrap();
    assert_eq!(&*row.ticket as *const Ticket as usize, original);
    assert_eq!(row.ticket.udid().as_ptr(), canonical);
    assert!(origin.available_permits() < TOTAL - TicketOwner::body_bytes());
    drop(value);
    assert_eq!(origin.available_permits(), TOTAL);
    assert_eq!(bank.available_permits(), TOTAL);
}

#[derive(Debug)]
struct Work {
    io: std::io::Error,
    drops: Arc<AtomicUsize>,
}
impl std::fmt::Display for Work {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("original episode work")
    }
}
impl Error for Work {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        Some(&self.io)
    }
}
impl Drop for Work {
    fn drop(&mut self) {
        self.drops.fetch_add(1, Ordering::SeqCst);
    }
}

#[tokio::test(start_paused = true)]
async fn episode_ticket_shares_first_receipt_root_without_retaining_receipt_or_original_io() {
    let bank = Arc::new(Semaphore::new(TOTAL));
    let origin = Arc::new(Semaphore::new(TOTAL));
    let lifecycle = Lifecycle::with_metadata(origin.clone()).unwrap();
    let ticket = lifecycle.ticket(A).await.unwrap().fenced();
    let policy = RetryPolicy::new(
        Duration::from_secs(1),
        Duration::from_secs(1),
        NonZeroU32::new(3),
        Some(Duration::from_secs(30)),
    )
    .unwrap();
    let ledger = Ledger::with_metadata(policy, bank.clone()).unwrap();
    let first = ticket
        .commit(|| ledger.start(A, identity(), &ticket, slot(&bank)))
        .unwrap();
    let receipt = first.receipt.clone();
    let weak = receipt.downgrade();
    let root = ledger.lock().row(A).unwrap().ticket.clone();
    assert!(root.ptr_eq(&receipt.ticket));
    let drops = Arc::new(AtomicUsize::new(0));
    let original = anyhow::Error::new(Work {
        io: std::io::Error::new(
            std::io::ErrorKind::ConnectionReset,
            "original-episode-shared-root",
        ),
        drops: drops.clone(),
    });
    let pointer = original.downcast_ref::<Work>().unwrap() as *const Work as usize;
    let io_pointer = &original.downcast_ref::<Work>().unwrap().io as *const _ as usize;
    let result = failed_result(first.source_failed(original));
    first.finish(&result);
    let first_snapshot = ledger.snapshot(A).unwrap();
    tokio::time::advance(Duration::from_secs(1)).await;
    let second = ticket
        .commit(|| ledger.start(A, identity(), &ticket, slot(&bank)))
        .unwrap();
    let latest = second.receipt.clone();
    assert!(!root.ptr_eq(&latest.ticket));
    assert!(ledger.lock().row(A).unwrap().ticket.ptr_eq(&root));
    let (later, _) = io_original("second-episode-root-work");
    let second_result = failed_result(second.source_failed(later));
    second.finish(&second_result);
    let second_snapshot = ledger.snapshot(A).unwrap();
    assert_eq!(second_snapshot.attempts, first_snapshot.attempts + 1);
    assert_eq!(second_snapshot.first_attempt, first_snapshot.first_attempt);
    assert_eq!(second_snapshot.deadline, first_snapshot.deadline);
    assert_eq!(
        latest.primary().downcast_ref::<Work>().unwrap() as *const _ as usize,
        pointer
    );
    assert_eq!(
        &latest.primary().downcast_ref::<Work>().unwrap().io as *const _ as usize,
        io_pointer
    );
    drop((receipt, result));
    assert!(
        weak.upgrade().is_none(),
        "Episode retained the old receipt rather than just its Ticket"
    );
    assert_eq!(
        drops.load(Ordering::SeqCst),
        0,
        "latest history still owns the original IO"
    );
    let mut removed = ledger.remove_where(|_, _, _| true);
    let node = removed.next().unwrap();
    assert!(node.row.is_some());
    assert!(node.row.as_ref().unwrap().ticket.ptr_eq(&root));
    drop((removed, ledger, latest, second_result, weak));
    assert_eq!(
        drops.load(Ordering::SeqCst),
        1,
        "Ticket root delayed original IO destruction"
    );
    let a_before = origin.available_permits();
    let b_before = bank.available_permits();
    let body = node.bytes_in(&bank).unwrap();
    let value = node.into_inner();
    assert_eq!(bank.available_permits(), b_before + body);
    assert_eq!(origin.available_permits(), a_before);
    assert!(value.row.as_ref().unwrap().ticket.ptr_eq(&root));
    drop((root, ticket, lifecycle));
    assert!(origin.available_permits() < TOTAL);
    drop(value);
    assert_eq!(origin.available_permits(), TOTAL);
    assert_eq!(bank.available_permits(), TOTAL);
}
