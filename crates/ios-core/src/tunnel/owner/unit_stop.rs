//! Private unit-stop protocol in the original complete, prepaid Ledger root.
//!
//! Only the userspace stop namespace uses this protocol. Its callers discard
//! changed's result; public native watch APIs keep their original Tokio types.
use super::Ledger;
use std::sync::{Mutex, MutexGuard};
use tokio::{sync::Notify, task::coop::cooperative};

struct Protocol {
    initialized: bool,
    senders: usize,
    version: usize,
}

pub(super) struct State {
    protocol: Mutex<Protocol>,
    notify: Notify,
}
impl State {
    pub(super) fn new() -> Self {
        Self {
            protocol: Mutex::new(Protocol {
                initialized: false,
                senders: 0,
                version: 0,
            }),
            notify: Notify::new(),
        }
    }
    fn lock(&self) -> MutexGuard<'_, Protocol> {
        self.protocol
            .lock()
            .unwrap_or_else(|error| error.into_inner())
    }
    fn changed(&self, seen: &mut usize) -> Option<Result<(), Closed>> {
        let state = self.lock();
        // Preserve watch's final unseen value before reporting sender closure.
        if *seen != state.version {
            *seen = state.version;
            Some(Ok(()))
        } else if state.senders == 0 {
            Some(Err(Closed))
        } else {
            None
        }
    }
}

pub(crate) struct Sender {
    ledger: Ledger,
}
#[derive(Clone)]
pub(crate) struct Receiver {
    ledger: Ledger,
    seen: usize,
}
// This private result never enters the runtime's original IO/source graph.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct Closed;

pub(crate) fn channel(ledger: Ledger) -> (Sender, Receiver) {
    {
        let mut state = ledger.stop_watch.lock();
        assert!(!state.initialized, "one stop namespace per original Ledger");
        state.initialized = true;
        state.senders = 1;
    }
    (
        Sender {
            ledger: ledger.clone(),
        },
        Receiver { ledger, seen: 0 },
    )
}

impl Sender {
    pub(crate) fn send_replace(&self, _value: ()) {
        {
            let mut state = self.ledger.stop_watch.lock();
            // Tokio's unit watch advances even for equal values and no readers.
            // +2 retains the pinned watch version's exact wrapping period.
            state.version = state.version.wrapping_add(2);
        }
        self.ledger.stop_watch.notify.notify_waiters();
    }
    pub(crate) fn subscribe(&self) -> Receiver {
        let ledger = self.ledger.clone();
        let seen = ledger.stop_watch.lock().version;
        Receiver { ledger, seen }
    }
}
impl Clone for Sender {
    fn clone(&self) -> Self {
        // The original root count bounds sender count. Obtain that root first.
        let ledger = self.ledger.clone();
        ledger.stop_watch.lock().senders += 1;
        Self { ledger }
    }
}
impl Drop for Sender {
    fn drop(&mut self) {
        let closed = {
            let mut state = self.ledger.stop_watch.lock();
            state.senders -= 1;
            state.senders == 0
        };
        if closed {
            // Close does not advance the version; wake only outside the lock.
            self.ledger.stop_watch.notify.notify_waiters();
        }
    }
}
impl Receiver {
    pub(crate) async fn changed(&mut self) -> Result<(), Closed> {
        cooperative(async {
            let state = &self.ledger.stop_watch;
            loop {
                // notify_waiters covers a Notified from its creation, including
                // a send between this registration and the locked version check.
                let notified = state.notify.notified();
                if let Some(result) = state.changed(&mut self.seen) {
                    return result;
                }
                notified.await;
            }
        })
        .await
    }
    #[cfg(test)]
    pub(crate) fn has_changed(&self) -> Result<bool, Closed> {
        let state = self.ledger.stop_watch.lock();
        // Unlike changed(), the original snapshot query checks closure first.
        if state.senders == 0 {
            Err(Closed)
        } else {
            Ok(self.seen != state.version)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{device::prepared_rsd::with_allocation_hook, tunnel::owner::LedgerState};
    use std::{
        future::Future,
        pin::{pin, Pin},
        sync::{
            atomic::{AtomicUsize, Ordering},
            Arc, Barrier,
        },
        task::{Context, Poll, Wake, Waker},
    };
    use tokio::sync::{watch as reference, Semaphore};

    #[derive(Default)]
    struct Wakes(AtomicUsize);
    impl Wake for Wakes {
        fn wake(self: Arc<Self>) {
            self.0.fetch_add(1, Ordering::SeqCst);
        }
        fn wake_by_ref(self: &Arc<Self>) {
            self.0.fetch_add(1, Ordering::SeqCst);
        }
    }
    #[derive(Debug, PartialEq, Eq)]
    enum Status {
        Pending,
        Changed,
        Closed,
    }
    fn poll<F: Future<Output = Result<(), E>>, E>(future: Pin<&mut F>, waker: &Waker) -> Status {
        match future.poll(&mut Context::from_waker(waker)) {
            Poll::Pending => Status::Pending,
            Poll::Ready(Ok(())) => Status::Changed,
            Poll::Ready(Err(_)) => Status::Closed,
        }
    }
    fn compare(ours: &mut Receiver, native: &mut reference::Receiver<()>, expected: Status) {
        let waker = Waker::from(Arc::new(Wakes::default()));
        assert_eq!(
            ours.has_changed().map_err(|_| ()),
            native.has_changed().map_err(|_| ())
        );
        assert_eq!(poll(pin!(ours.changed()), &waker), expected);
        assert_eq!(poll(pin!(native.changed()), &waker), expected);
    }

    #[test]
    fn differential_equal_sends_coalescing_seen_clone_late_subscribe_and_zero_receivers() {
        let (tx, mut rx) = channel(Ledger::new());
        let (native_tx, mut native_rx) = reference::channel(());
        compare(&mut rx, &mut native_rx, Status::Pending);
        for _ in 0..3 {
            tx.send_replace(());
            native_tx.send_replace(());
        }
        let (mut clone, mut native_clone) = (rx.clone(), native_rx.clone());
        let (mut late, mut native_late) = (tx.subscribe(), native_tx.subscribe());
        compare(&mut late, &mut native_late, Status::Pending);
        assert_eq!(rx.has_changed(), Ok(true));
        assert_eq!(rx.has_changed(), Ok(true));
        compare(&mut rx, &mut native_rx, Status::Changed);
        compare(&mut clone, &mut native_clone, Status::Changed);
        compare(&mut clone, &mut native_clone, Status::Pending);
        let (mut seen_clone, mut native_seen_clone) = (rx.clone(), native_rx.clone());
        compare(&mut seen_clone, &mut native_seen_clone, Status::Pending);
        drop((rx, clone, late, seen_clone));
        drop((native_rx, native_clone, native_late, native_seen_clone));
        for _ in 0..2 {
            tx.send_replace(());
            native_tx.send_replace(());
        }
        let (mut rx, mut native_rx) = (tx.subscribe(), native_tx.subscribe());
        compare(&mut rx, &mut native_rx, Status::Pending);
        tx.send_replace(());
        native_tx.send_replace(());
        compare(&mut rx, &mut native_rx, Status::Changed);
        compare(&mut rx, &mut native_rx, Status::Pending);
    }

    #[test]
    fn differential_last_sender_close_checks_unseen_version_before_closed_only_in_changed() {
        for unseen in [false, true] {
            let (tx, mut rx) = channel(Ledger::new());
            let (native_tx, mut native_rx) = reference::channel(());
            let (kept, native_kept) = (tx.clone(), native_tx.clone());
            drop((tx, native_tx));
            compare(&mut rx, &mut native_rx, Status::Pending);
            if unseen {
                kept.send_replace(());
                native_kept.send_replace(());
            }
            drop((kept, native_kept));
            assert!(rx.has_changed().is_err());
            assert!(native_rx.has_changed().is_err());
            let (mut clone, mut native_clone) = (rx.clone(), native_rx.clone());
            let expected = if unseen {
                Status::Changed
            } else {
                Status::Closed
            };
            compare(&mut rx, &mut native_rx, expected);
            compare(
                &mut clone,
                &mut native_clone,
                if unseen {
                    Status::Changed
                } else {
                    Status::Closed
                },
            );
            compare(&mut rx, &mut native_rx, Status::Closed);
            compare(&mut clone, &mut native_clone, Status::Closed);
        }
    }

    #[test]
    fn differential_cancellation_before_send_and_after_registered_wake_keeps_version_unseen() {
        let (tx, mut rx) = channel(Ledger::new());
        let (native_tx, mut native_rx) = reference::channel(());
        let waker = Waker::from(Arc::new(Wakes::default()));
        for send_while_registered in [false, true] {
            {
                let (mut waiting, mut native_waiting) =
                    (pin!(rx.changed()), pin!(native_rx.changed()));
                assert_eq!(poll(waiting.as_mut(), &waker), Status::Pending);
                assert_eq!(poll(native_waiting.as_mut(), &waker), Status::Pending);
                if send_while_registered {
                    tx.send_replace(());
                    native_tx.send_replace(());
                }
                // A different biased select branch wins after the futures were
                // registered, including when they have already been woken.
                let winner = async {
                    tokio::select! {
                        biased;
                        _ = std::future::ready(()) => {},
                        _ = &mut waiting => panic!("stop branch won"),
                        _ = &mut native_waiting => panic!("reference stop branch won"),
                    }
                    Ok::<_, Closed>(())
                };
                assert_eq!(poll(pin!(winner), &waker), Status::Changed);
            }
            if !send_while_registered {
                assert_eq!(rx.has_changed(), Ok(false));
                tx.send_replace(());
                native_tx.send_replace(());
            }
            assert_eq!(rx.has_changed(), Ok(true));
            compare(&mut rx, &mut native_rx, Status::Changed);
            compare(&mut rx, &mut native_rx, Status::Pending);
        }
    }

    #[test]
    fn differential_all_registered_receivers_wake_on_equal_send_and_last_sender_close() {
        for close in [false, true] {
            let (tx, rx) = channel(Ledger::new());
            let (native_tx, native_rx) = reference::channel(());
            let wakes = Arc::new(Wakes::default());
            let native_wakes = Arc::new(Wakes::default());
            let waker = Waker::from(wakes.clone());
            let native_waker = Waker::from(native_wakes.clone());
            let mut waiting: Vec<_> = (0..32)
                .map(|_| {
                    let mut rx = rx.clone();
                    Box::pin(async move { rx.changed().await })
                })
                .collect();
            let mut native_waiting: Vec<_> = (0..32)
                .map(|_| {
                    let mut rx = native_rx.clone();
                    Box::pin(async move { rx.changed().await })
                })
                .collect();
            for future in &mut waiting {
                assert_eq!(poll(future.as_mut(), &waker), Status::Pending);
            }
            for future in &mut native_waiting {
                assert_eq!(poll(future.as_mut(), &native_waker), Status::Pending);
            }
            if close {
                drop((tx, native_tx));
            } else {
                tx.send_replace(());
                native_tx.send_replace(());
            }
            assert!(wakes.0.load(Ordering::SeqCst) >= 32);
            assert!(native_wakes.0.load(Ordering::SeqCst) >= 32);
            for future in &mut waiting {
                assert_eq!(
                    poll(future.as_mut(), &waker),
                    if close {
                        Status::Closed
                    } else {
                        Status::Changed
                    }
                );
            }
            for future in &mut native_waiting {
                assert_eq!(
                    poll(future.as_mut(), &native_waker),
                    if close {
                        Status::Closed
                    } else {
                        Status::Changed
                    }
                );
            }
        }
    }

    #[test]
    fn same_poll_send_registration_and_spurious_wakes_never_lose_a_version() {
        let waker = Waker::from(Arc::new(Wakes::default()));
        for iteration in 0..128 {
            let (tx, mut rx) = channel(Ledger::new());
            let barrier = Arc::new(Barrier::new(2));
            let ready = barrier.clone();
            let thread = std::thread::spawn(move || {
                ready.wait();
                tx.send_replace(());
            });
            let mut waiting = pin!(rx.changed());
            barrier.wait();
            let first = poll(waiting.as_mut(), &waker);
            thread.join().unwrap();
            if first == Status::Pending {
                assert_eq!(
                    poll(waiting.as_mut(), &waker),
                    Status::Changed,
                    "iteration {iteration}"
                );
            } else {
                assert_eq!(first, Status::Changed);
            }
        }
        let (tx, mut rx) = channel(Ledger::new());
        let mut waiting = pin!(rx.changed());
        assert_eq!(poll(waiting.as_mut(), &waker), Status::Pending);
        tx.ledger.stop_watch.notify.notify_waiters();
        assert_eq!(poll(waiting.as_mut(), &waker), Status::Pending);
        tx.send_replace(());
        assert_eq!(poll(waiting.as_mut(), &waker), Status::Changed);
    }

    #[test]
    fn version_wrap_keeps_pinned_tokio_step_and_unseen_final_delivery() {
        let (tx, mut rx) = channel(Ledger::new());
        tx.ledger.stop_watch.lock().version = usize::MAX - 1;
        rx.seen = usize::MAX - 1;
        tx.send_replace(());
        assert_eq!(tx.ledger.stop_watch.lock().version, 0);
        assert_eq!(rx.has_changed(), Ok(true));
        let waker = Waker::from(Arc::new(Wakes::default()));
        assert_eq!(poll(pin!(rx.changed()), &waker), Status::Changed);
        assert_eq!(rx.seen, 0);
        tx.send_replace(());
        assert_eq!(tx.ledger.stop_watch.lock().version, 2);
        drop(tx);
        assert!(rx.has_changed().is_err());
        assert_eq!(poll(pin!(rx.changed()), &waker), Status::Changed);
        assert_eq!(rx.seen, 2);
        assert_eq!(poll(pin!(rx.changed()), &waker), Status::Closed);
    }

    #[tokio::test]
    async fn differential_exhausted_cooperative_budget_does_not_mark_a_ready_version_seen() {
        tokio::spawn(async {
            let (tx, mut rx) = channel(Ledger::new());
            let (native_tx, mut native_rx) = reference::channel(());
            tx.send_replace(());
            native_tx.send_replace(());
            let mut spent = 0;
            while tokio::task::coop::has_budget_remaining() {
                tokio::task::consume_budget().await;
                spent += 1;
                assert!(spent <= 1024, "task never exhausted its cooperative budget");
            }
            assert!(spent > 0);
            let waker = Waker::from(Arc::new(Wakes::default()));
            assert_eq!(poll(pin!(rx.changed()), &waker), Status::Pending);
            assert_eq!(poll(pin!(native_rx.changed()), &waker), Status::Pending);
            assert_eq!(rx.has_changed(), Ok(true));
            assert!(native_rx.has_changed().unwrap());
            tokio::task::yield_now().await;
            compare(&mut rx, &mut native_rx, Status::Changed);
            compare(&mut rx, &mut native_rx, Status::Pending);
        })
        .await
        .unwrap();
    }

    #[test]
    fn closed_zero_pool_handles_use_the_same_admitted_root_and_isolate_other_runtimes() {
        let total = Ledger::reservation_bytes();
        let short = Arc::new(Semaphore::new(total - 1));
        let error = with_allocation_hook(
            |_, _| panic!("incomplete Ledger/watch grant allocated a backing"),
            || {
                Ledger::with_metadata(Some(&short))
                    .err()
                    .expect("short root admitted")
            },
        );
        assert_eq!(
            crate::classify_error(&error),
            crate::ErrorClass::ResourceLimit
        );
        assert_eq!(short.available_permits(), total - 1);
        let bank = Arc::new(Semaphore::new(total));
        let allocations = Arc::new(Mutex::new(Vec::new()));
        let recorded = allocations.clone();
        let credit = bank.clone();
        let ledger = with_allocation_hook(
            move |layout, pointer| {
                assert_eq!(credit.available_permits(), 0);
                recorded.lock().unwrap().push((layout.size(), pointer));
            },
            || Ledger::with_metadata(Some(&bank)).unwrap(),
        );
        let state = (&*ledger as *const LedgerState) as usize;
        let watch = (&ledger.stop_watch as *const State) as usize;
        let allocations = allocations.lock().unwrap();
        assert_eq!(allocations.len(), 2);
        let (bytes, pointer) = allocations[0];
        assert_eq!(bytes, Ledger::body_bytes());
        assert!((pointer..pointer + bytes).contains(&state));
        assert!(watch >= pointer && watch + std::mem::size_of::<State>() <= pointer + bytes);
        drop(allocations);
        bank.close();
        let (other, mut other_rx) = channel(Ledger::new());
        let waker = Waker::from(Arc::new(Wakes::default()));
        with_allocation_hook(
            |_, _| panic!("inline watch allocated a new controlled backing"),
            || {
                let (tx, mut rx) = channel(ledger.clone());
                let kept = tx.clone();
                let mut clone = rx.clone();
                let mut late = tx.subscribe();
                for owner in [
                    &tx.ledger,
                    &kept.ledger,
                    &rx.ledger,
                    &clone.ledger,
                    &late.ledger,
                ] {
                    assert_eq!((&**owner as *const LedgerState) as usize, state);
                    assert_eq!((&owner.stop_watch as *const State) as usize, watch);
                }
                tx.send_replace(());
                for rx in [&mut rx, &mut clone, &mut late] {
                    assert_eq!(poll(pin!(rx.changed()), &waker), Status::Changed);
                }
                assert_eq!(other_rx.has_changed(), Ok(false));
                assert_eq!(poll(pin!(other_rx.changed()), &waker), Status::Pending);
                drop((tx, kept));
                assert_eq!(poll(pin!(rx.changed()), &waker), Status::Closed);
            },
        );
        assert_eq!(bank.available_permits(), 0);
        drop(ledger);
        assert_eq!(bank.available_permits(), total);
        assert_eq!(other_rx.has_changed(), Ok(false));
        drop(other);
        assert_eq!(poll(pin!(other_rx.changed()), &waker), Status::Closed);
    }

    #[test]
    fn cancellation_of_last_pending_receiver_keeps_whole_root_fee_until_actual_free() {
        let total = Ledger::reservation_bytes();
        let bank = Arc::new(Semaphore::new(total));
        let ledger = Ledger::with_metadata(Some(&bank)).unwrap();
        let (tx, mut rx) = channel(ledger.clone());
        let mut waiting = Box::pin(async move { rx.changed().await });
        let waker = Waker::from(Arc::new(Wakes::default()));
        assert_eq!(poll(waiting.as_mut(), &waker), Status::Pending);
        bank.close();
        drop((ledger, tx));
        assert_eq!(bank.available_permits(), 0);
        let credit = bank.clone();
        let frees = Arc::new(AtomicUsize::new(0));
        let observed = frees.clone();
        super::super::ledger_root::after_free(
            move || {
                assert_eq!(
                    credit.available_permits(),
                    0,
                    "whole fee refunded before actual root free"
                );
                observed.fetch_add(1, Ordering::SeqCst);
            },
            || {
                with_allocation_hook(
                    |_, _| panic!("pending receiver cancellation allocated a controlled backing"),
                    || drop(waiting),
                );
            },
        );
        assert_eq!(frees.load(Ordering::SeqCst), 1);
        assert_eq!(bank.available_permits(), total);
    }

    #[test]
    fn parallel_sender_clones_keep_original_channel_open_until_the_last_actual_sender() {
        let (tx, mut rx) = channel(Ledger::new());
        let (native_tx, mut native_rx) = reference::channel(());
        let barrier = Arc::new(Barrier::new(9));
        let threads: Vec<_> = (0..8)
            .map(|_| {
                let (tx, native_tx) = (tx.clone(), native_tx.clone());
                let barrier = barrier.clone();
                std::thread::spawn(move || {
                    barrier.wait();
                    tx.send_replace(());
                    native_tx.send_replace(());
                })
            })
            .collect();
        drop((tx, native_tx));
        compare(&mut rx, &mut native_rx, Status::Pending);
        barrier.wait();
        for thread in threads {
            thread.join().unwrap();
        }
        assert!(rx.has_changed().is_err());
        compare(&mut rx, &mut native_rx, Status::Changed);
        compare(&mut rx, &mut native_rx, Status::Closed);
    }
}
