//! The private CLI watch API in an original-bank complete declared root.
//! Value-owned heaps, runtime waiters and Wakers remain separate accounting.
use super::shared_weak_allocation::Strong;
use anyhow::Result;
use std::{
    ops::Deref,
    panic::{catch_unwind, resume_unwind, AssertUnwindSafe},
    sync::{
        atomic::{AtomicUsize, Ordering},
        Arc, RwLock, RwLockReadGuard,
    },
};
use tokio::{sync::Notify, sync::Semaphore, task::coop::cooperative};

const CLOSED: usize = 1;
struct State<T> {
    value: RwLock<T>,
    version: AtomicUsize,
    senders: AtomicUsize,
    notify: Notify,
}
impl<T> State<T> {
    fn read(&self) -> RwLockReadGuard<'_, T> {
        self.value.read().unwrap_or_else(|error| error.into_inner())
    }
    fn maybe_changed(&self, seen: &mut usize) -> Option<std::result::Result<(), Closed>> {
        let state = self.version.load(Ordering::Acquire);
        let version = state & !CLOSED;
        if *seen != version {
            *seen = version;
            Some(Ok(()))
        } else if state & CLOSED != 0 {
            Some(Err(Closed))
        } else {
            None
        }
    }
    async fn changed(&self, seen: &mut usize) -> std::result::Result<(), Closed> {
        loop {
            // notify_waiters covers creation before the version check, even
            // when the notification arrives before Notified's first poll.
            let notified = self.notify.notified();
            if let Some(result) = self.maybe_changed(seen) {
                return result;
            }
            notified.await;
        }
    }
}

pub(super) struct Sender<T> {
    shared: Strong<State<T>, ()>,
}
pub(super) struct Receiver<T> {
    shared: Strong<State<T>, ()>,
    seen: usize,
}
pub(super) struct Ref<'a, T> {
    inner: RwLockReadGuard<'a, T>,
    #[cfg(test)]
    changed: bool,
}
impl<T> Deref for Ref<'_, T> {
    type Target = T;
    fn deref(&self) -> &T {
        &self.inner
    }
}
#[derive(Debug)]
pub(super) struct Closed;

pub(super) fn body_bytes<T>() -> usize {
    Strong::<State<T>, ()>::body_bytes()
}
pub(super) fn channel<T>(value: T, bank: &Arc<Semaphore>) -> Result<(Sender<T>, Receiver<T>)> {
    let grant = super::admission::known_metadata_charge(bank, body_bytes::<T>())?;
    from_grant(value, bank, grant)
}
pub(super) fn from_grant<T>(
    value: T,
    bank: &Arc<Semaphore>,
    grant: tokio::sync::OwnedSemaphorePermit,
) -> Result<(Sender<T>, Receiver<T>)> {
    let shared = Strong::new(
        State {
            value: RwLock::new(value),
            version: AtomicUsize::new(0),
            senders: AtomicUsize::new(1),
            notify: Notify::new(),
        },
        bank,
        grant,
        (),
    )?;
    Ok((
        Sender {
            shared: shared.clone(),
        },
        Receiver { shared, seen: 0 },
    ))
}
impl<T> Sender<T> {
    #[cfg(test)]
    pub(super) fn is_unique_initial(&self) -> bool {
        self.shared.strong_count() == 1
            && self.shared.senders.load(Ordering::Acquire) == 1
            && self.shared.version.load(Ordering::Acquire) == 0
    }
    pub(super) fn borrow(&self) -> Ref<'_, T> {
        Ref {
            inner: self.shared.read(),
            #[cfg(test)]
            changed: false,
        }
    }
    pub(super) fn subscribe(&self) -> Receiver<T> {
        let shared = self.shared.clone();
        let seen = shared.version.load(Ordering::Acquire) & !CLOSED;
        Receiver { shared, seen }
    }
    pub(super) fn send_if_modified(&self, modify: impl FnOnce(&mut T) -> bool) -> bool {
        let mut value = self
            .shared
            .value
            .write()
            .unwrap_or_else(|error| error.into_inner());
        match catch_unwind(AssertUnwindSafe(|| modify(&mut value))) {
            Ok(false) => return false,
            Ok(true) => {
                // Same wrapping +2 period and publication-before-unlock as
                // pinned Tokio watch; metadata reads never acquire value.
                self.shared.version.fetch_add(2, Ordering::Release);
            }
            Err(panic) => {
                drop(value);
                resume_unwind(panic);
            }
        }
        drop(value);
        self.shared.notify.notify_waiters();
        true
    }
    pub(super) fn send_replace(&self, mut incoming: T) -> T {
        self.send_if_modified(|value| {
            std::mem::swap(value, &mut incoming);
            true
        });
        incoming
    }
}
impl<T> Clone for Sender<T> {
    fn clone(&self) -> Self {
        let shared = self.shared.clone();
        // Root count is acquired first and aborts at isize::MAX; it bounds
        // sender count, so this count has no independent overflow path.
        shared.senders.fetch_add(1, Ordering::Relaxed);
        Self { shared }
    }
}
impl<T> Drop for Sender<T> {
    fn drop(&mut self) {
        if self.shared.senders.fetch_sub(1, Ordering::AcqRel) == 1 {
            self.shared.version.fetch_or(CLOSED, Ordering::Release);
            self.shared.notify.notify_waiters();
        }
    }
}
impl<T> Clone for Receiver<T> {
    fn clone(&self) -> Self {
        Self {
            shared: self.shared.clone(),
            seen: self.seen,
        }
    }
}
impl<T> std::fmt::Debug for Receiver<T> {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("PrivateWatchReceiver")
    }
}
impl<T> Receiver<T> {
    pub(super) fn borrow(&self) -> Ref<'_, T> {
        let inner = self.shared.read();
        Ref {
            inner,
            #[cfg(test)]
            changed: self.seen != self.shared.version.load(Ordering::Acquire) & !CLOSED,
        }
    }
    pub(super) fn borrow_and_update(&mut self) -> Ref<'_, T> {
        let inner = self.shared.read();
        let seen = self.shared.version.load(Ordering::Acquire) & !CLOSED;
        #[cfg(test)]
        let changed = self.seen != seen;
        self.seen = seen;
        Ref {
            inner,
            #[cfg(test)]
            changed,
        }
    }
    pub(super) fn has_changed(&self) -> std::result::Result<bool, Closed> {
        // Independent atomic query: commit intentionally calls this with a
        // value Ref held, including while a writer could already be queued.
        let state = self.shared.version.load(Ordering::Acquire);
        if state & CLOSED != 0 {
            Err(Closed)
        } else {
            Ok(self.seen != state & !CLOSED)
        }
    }
    pub(super) async fn changed(&mut self) -> std::result::Result<(), Closed> {
        cooperative(self.shared.changed(&mut self.seen)).await
    }
    pub(super) async fn wait_for(
        &mut self,
        predicate: impl FnMut(&T) -> bool,
    ) -> std::result::Result<Ref<'_, T>, Closed> {
        cooperative(self.wait_for_inner(predicate)).await
    }
    async fn wait_for_inner(
        &mut self,
        mut predicate: impl FnMut(&T) -> bool,
    ) -> std::result::Result<Ref<'_, T>, Closed> {
        let mut closed = false;
        loop {
            {
                let inner = self.shared.read();
                let seen = self.shared.version.load(Ordering::Acquire) & !CLOSED;
                let changed = self.seen != seen;
                self.seen = seen;
                if !closed || changed {
                    match catch_unwind(AssertUnwindSafe(|| predicate(&inner))) {
                        Ok(true) => {
                            return Ok(Ref {
                                inner,
                                #[cfg(test)]
                                changed,
                            })
                        }
                        Ok(false) => {}
                        Err(panic) => {
                            drop(inner);
                            resume_unwind(panic);
                        }
                    }
                }
            }
            if closed {
                return Err(Closed);
            }
            closed = self.shared.changed(&mut self.seen).await.is_err();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cmd::tunnel::metadata_vec::with_allocation_hook;
    use std::{
        future::Future,
        marker::PhantomPinned,
        pin::{pin, Pin},
        sync::{atomic::AtomicBool, mpsc, Mutex},
        task::{Context, Poll, Wake, Waker},
        time::Duration,
    };
    use tokio::sync::watch as native;

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
    fn poll<F: Future>(future: Pin<&mut F>) -> Poll<F::Output> {
        let waker = Waker::from(Arc::new(Wakes::default()));
        future.poll(&mut Context::from_waker(&waker))
    }
    #[derive(Debug, PartialEq, Eq)]
    enum Status {
        Pending,
        Changed,
        Closed,
    }
    fn status<F: Future<Output = std::result::Result<(), E>>, E>(future: Pin<&mut F>) -> Status {
        match poll(future) {
            Poll::Pending => Status::Pending,
            Poll::Ready(Ok(())) => Status::Changed,
            Poll::Ready(Err(_)) => Status::Closed,
        }
    }
    fn compare<T>(rx: &mut Receiver<T>, reference: &mut native::Receiver<T>, expected: Status) {
        assert_eq!(
            rx.has_changed().map_err(|_| ()),
            reference.has_changed().map_err(|_| ())
        );
        assert_eq!(status(pin!(rx.changed())), expected);
        assert_eq!(status(pin!(reference.changed())), expected);
    }
    fn bank<T>() -> Arc<Semaphore> {
        Arc::new(Semaphore::new(body_bytes::<T>()))
    }

    #[test]
    fn private_watch_differential_original_values_borrows_clones_subscribe_and_zero_receivers() {
        let value = Arc::new(());
        let (tx, mut rx) = channel(value.clone(), &bank::<Arc<()>>()).unwrap();
        let (reference_tx, mut reference) = native::channel(value.clone());
        compare(&mut rx, &mut reference, Status::Pending);
        for _ in 0..3 {
            assert!(Arc::ptr_eq(&tx.send_replace(value.clone()), &value));
            assert!(Arc::ptr_eq(
                &reference_tx.send_replace(value.clone()),
                &value
            ));
        }
        assert!(rx.borrow().changed && reference.borrow().has_changed());
        assert!(rx.has_changed().unwrap() && reference.has_changed().unwrap());
        let (mut clone, mut reference_clone) = (rx.clone(), reference.clone());
        let (mut late, mut reference_late) = (tx.subscribe(), reference_tx.subscribe());
        compare(&mut late, &mut reference_late, Status::Pending);
        assert!(rx.borrow_and_update().changed && reference.borrow_and_update().has_changed());
        compare(&mut rx, &mut reference, Status::Pending);
        compare(&mut clone, &mut reference_clone, Status::Changed);
        assert!(!tx.borrow().changed && !reference_tx.borrow().has_changed());
        drop((rx, reference, clone, reference_clone, late, reference_late));
        let next = Arc::new(());
        assert!(Arc::ptr_eq(&tx.send_replace(next.clone()), &value));
        assert!(Arc::ptr_eq(
            &reference_tx.send_replace(next.clone()),
            &value
        ));
        let (mut rx, mut reference) = (tx.subscribe(), reference_tx.subscribe());
        assert!(Arc::ptr_eq(&rx.borrow(), &next));
        compare(&mut rx, &mut reference, Status::Pending);
    }

    #[test]
    fn private_watch_differential_last_sender_closed_and_unseen_final_value() {
        for unseen in [false, true] {
            let (tx, mut rx) = channel(0, &bank::<i32>()).unwrap();
            let (reference_tx, mut reference) = native::channel(0);
            let kept = tx.clone();
            let reference_kept = reference_tx.clone();
            drop((tx, reference_tx));
            compare(&mut rx, &mut reference, Status::Pending);
            if unseen {
                kept.send_replace(1);
                reference_kept.send_replace(1);
            }
            drop((kept, reference_kept));
            assert!(rx.has_changed().is_err() && reference.has_changed().is_err());
            compare(
                &mut rx,
                &mut reference,
                if unseen {
                    Status::Changed
                } else {
                    Status::Closed
                },
            );
            compare(&mut rx, &mut reference, Status::Closed);
            assert_eq!(*rx.borrow(), *reference.borrow());
        }
    }

    struct OriginalPanic(Arc<()>);
    #[test]
    fn private_watch_differential_modifier_false_partial_mutation_and_original_panic_without_poison(
    ) {
        let (tx, mut rx) = channel(0, &bank::<i32>()).unwrap();
        let (reference_tx, mut reference) = native::channel(0);
        assert!(!tx.send_if_modified(|v| {
            *v = 1;
            false
        }));
        assert!(!reference_tx.send_if_modified(|v| {
            *v = 1;
            false
        }));
        assert_eq!(*rx.borrow(), *reference.borrow());
        compare(&mut rx, &mut reference, Status::Pending);
        let marker = Arc::new(());
        let own = std::panic::catch_unwind(AssertUnwindSafe(|| {
            tx.send_if_modified(|v| {
                *v = 2;
                std::panic::panic_any(OriginalPanic(marker.clone()));
            });
        }))
        .unwrap_err();
        let other = std::panic::catch_unwind(AssertUnwindSafe(|| {
            reference_tx.send_if_modified(|v| {
                *v = 2;
                std::panic::panic_any(OriginalPanic(marker.clone()));
            });
        }))
        .unwrap_err();
        assert!(Arc::ptr_eq(
            &own.downcast_ref::<OriginalPanic>().unwrap().0,
            &marker
        ));
        assert!(Arc::ptr_eq(
            &other.downcast_ref::<OriginalPanic>().unwrap().0,
            &marker
        ));
        assert_eq!(*rx.borrow(), 2);
        assert_eq!(*reference.borrow(), 2);
        assert!(!tx.shared.value.is_poisoned());
        compare(&mut rx, &mut reference, Status::Pending);
        assert!(tx.send_if_modified(|v| {
            *v = 3;
            true
        }));
        assert!(reference_tx.send_if_modified(|v| {
            *v = 3;
            true
        }));
        compare(&mut rx, &mut reference, Status::Changed);
    }

    #[test]
    fn private_watch_differential_wait_for_current_closed_final_cancel_and_predicate_panic() {
        for initial in [false, true] {
            for close in [false, true] {
                let (tx, mut rx) = channel(initial, &bank::<bool>()).unwrap();
                let (reference_tx, mut reference) = native::channel(initial);
                if close {
                    drop((tx, reference_tx));
                }
                let own_calls = AtomicUsize::new(0);
                let other_calls = AtomicUsize::new(0);
                let mut own = pin!(rx.wait_for(|v| {
                    own_calls.fetch_add(1, Ordering::SeqCst);
                    *v
                }));
                let mut other = pin!(reference.wait_for(|v| {
                    other_calls.fetch_add(1, Ordering::SeqCst);
                    *v
                }));
                match (poll(own.as_mut()), poll(other.as_mut())) {
                    (Poll::Ready(Ok(a)), Poll::Ready(Ok(b))) => {
                        assert!(initial && *a && *b);
                        assert_eq!(a.changed, b.has_changed());
                    }
                    (Poll::Ready(Err(_)), Poll::Ready(Err(_))) => assert!(close && !initial),
                    (Poll::Pending, Poll::Pending) => assert!(!close && !initial),
                    _ => panic!("wait_for diverged"),
                }
                assert_eq!(
                    own_calls.load(Ordering::SeqCst),
                    other_calls.load(Ordering::SeqCst)
                );
                assert_eq!(own_calls.load(Ordering::SeqCst), 1);
            }
        }
        let (tx, mut rx) = channel(false, &bank::<bool>()).unwrap();
        let (reference_tx, mut reference) = native::channel(false);
        tx.send_replace(false);
        reference_tx.send_replace(false);
        assert!(poll(pin!(rx.wait_for(|v| *v))).is_pending());
        assert!(poll(pin!(reference.wait_for(|v| *v))).is_pending());
        assert!(!rx.has_changed().unwrap() && !reference.has_changed().unwrap());
        let marker = Arc::new(());
        let own = std::panic::catch_unwind(AssertUnwindSafe(|| {
            let _ = poll(pin!(
                rx.wait_for(|_| std::panic::panic_any(OriginalPanic(marker.clone())))
            ));
        }))
        .unwrap_err();
        let other = std::panic::catch_unwind(AssertUnwindSafe(|| {
            let _ = poll(pin!(
                reference.wait_for(|_| std::panic::panic_any(OriginalPanic(marker.clone())))
            ));
        }))
        .unwrap_err();
        assert!(Arc::ptr_eq(
            &own.downcast_ref::<OriginalPanic>().unwrap().0,
            &marker
        ));
        assert!(Arc::ptr_eq(
            &other.downcast_ref::<OriginalPanic>().unwrap().0,
            &marker
        ));
        tx.send_replace(true);
        reference_tx.send_replace(true);
        let mut late = tx.subscribe();
        let mut reference_late = reference_tx.subscribe();
        drop((tx, reference_tx));
        for (own, other) in [(&mut rx, &mut reference), (&mut late, &mut reference_late)] {
            let a = poll(pin!(own.wait_for(|v| *v)));
            let b = poll(pin!(other.wait_for(|v| *v)));
            let (Poll::Ready(Ok(a)), Poll::Ready(Ok(b))) = (a, b) else {
                panic!("final true missed")
            };
            assert!(*a && *b);
            assert_eq!(a.changed, b.has_changed());
        }
    }

    #[test]
    fn private_watch_original_read_fence_excludes_writer_and_metadata_never_waits_for_value() {
        let (tx, rx) = channel(0, &bank::<i32>()).unwrap();
        let (reference_tx, reference) = native::channel(0);
        let own_query = rx.clone();
        let other_query = reference.clone();
        let own_guard = rx.borrow();
        let other_guard = reference.borrow();
        let (begun, started) = mpsc::channel();
        let (applied, done) = mpsc::channel();
        let writer = std::thread::spawn(move || {
            begun.send(()).unwrap();
            tx.send_replace(1);
            applied.send(()).unwrap();
        });
        started.recv_timeout(Duration::from_secs(1)).unwrap();
        let (native_begun, native_started) = mpsc::channel();
        let (native_applied, native_done) = mpsc::channel();
        let native_writer = std::thread::spawn(move || {
            native_begun.send(()).unwrap();
            reference_tx.send_replace(1);
            native_applied.send(()).unwrap();
        });
        native_started.recv_timeout(Duration::from_secs(1)).unwrap();
        let fenced = done.recv_timeout(Duration::from_millis(25)).is_err();
        let native_fenced = native_done.recv_timeout(Duration::from_millis(25)).is_err();
        let (answer, answers) = mpsc::channel();
        let query = std::thread::spawn(move || {
            answer
                .send((
                    own_query.has_changed().map_err(|_| ()),
                    other_query.has_changed().map_err(|_| ()),
                ))
                .unwrap();
        });
        let metadata = answers.recv_timeout(Duration::from_secs(1));
        drop((own_guard, other_guard));
        writer.join().unwrap();
        native_writer.join().unwrap();
        query.join().unwrap();
        assert!(fenced && native_fenced);
        assert_eq!(metadata.unwrap(), (Ok(false), Ok(false)));
        done.recv_timeout(Duration::from_secs(1)).unwrap();
        native_done.recv_timeout(Duration::from_secs(1)).unwrap();
        assert_eq!(*rx.borrow(), 1);
    }

    #[test]
    fn private_watch_differential_cancel_after_registered_wake_and_all_receivers_wake() {
        let (tx, mut rx) = channel(0, &bank::<i32>()).unwrap();
        let (reference_tx, mut reference) = native::channel(0);
        {
            let mut own = pin!(rx.changed());
            let mut other = pin!(reference.changed());
            assert_eq!(status(own.as_mut()), Status::Pending);
            assert_eq!(status(other.as_mut()), Status::Pending);
            tx.send_replace(1);
            reference_tx.send_replace(1);
            // Cancel the registered/woken futures before either is repolled.
        }
        compare(&mut rx, &mut reference, Status::Changed);
        for close in [false, true] {
            let (sender, first) = channel(0usize, &bank::<usize>()).unwrap();
            let (native_sender, native_first) = native::channel(0usize);
            let mut own: Vec<_> = (0..16)
                .map(|_| {
                    let mut receiver = first.clone();
                    Box::pin(async move { receiver.changed().await })
                })
                .collect();
            let mut other: Vec<_> = (0..16)
                .map(|_| {
                    let mut receiver = native_first.clone();
                    Box::pin(async move { receiver.changed().await })
                })
                .collect();
            let own_wakes: Vec<_> = (0..16).map(|_| Arc::new(Wakes::default())).collect();
            let other_wakes: Vec<_> = (0..16).map(|_| Arc::new(Wakes::default())).collect();
            for (future, wakes) in own.iter_mut().zip(&own_wakes) {
                let waker = Waker::from(wakes.clone());
                assert!(future
                    .as_mut()
                    .poll(&mut Context::from_waker(&waker))
                    .is_pending());
            }
            for (future, wakes) in other.iter_mut().zip(&other_wakes) {
                let waker = Waker::from(wakes.clone());
                assert!(future
                    .as_mut()
                    .poll(&mut Context::from_waker(&waker))
                    .is_pending());
            }
            assert!(own_wakes.iter().all(|w| w.0.load(Ordering::SeqCst) == 0));
            assert!(other_wakes.iter().all(|w| w.0.load(Ordering::SeqCst) == 0));
            if close {
                drop((sender, native_sender));
            } else {
                sender.send_replace(1);
                native_sender.send_replace(1);
            }
            // Assert every actual wake BEFORE actively polling any future again.
            assert!(own_wakes.iter().all(|w| w.0.load(Ordering::SeqCst) > 0));
            assert!(other_wakes.iter().all(|w| w.0.load(Ordering::SeqCst) > 0));
            for future in &mut own {
                assert_eq!(
                    status(future.as_mut()),
                    if close {
                        Status::Closed
                    } else {
                        Status::Changed
                    }
                );
            }
            for future in &mut other {
                assert_eq!(
                    status(future.as_mut()),
                    if close {
                        Status::Closed
                    } else {
                        Status::Changed
                    }
                );
            }
        }
    }

    #[tokio::test]
    async fn private_watch_differential_cooperative_budget_blocks_changed_and_wait_for_before_seen()
    {
        tokio::spawn(async {
            let (tx, mut rx) = channel(false, &bank::<bool>()).unwrap();
            let (reference_tx, mut reference) = native::channel(false);
            tx.send_replace(true);
            reference_tx.send_replace(true);
            let mut spent = 0;
            while tokio::task::coop::has_budget_remaining() {
                tokio::task::consume_budget().await;
                spent += 1;
                assert!(spent <= 1024);
            }
            assert!(spent > 0);
            compare(&mut rx, &mut reference, Status::Pending);
            assert!(poll(pin!(rx.wait_for(|v| *v))).is_pending());
            assert!(poll(pin!(reference.wait_for(|v| *v))).is_pending());
            assert!(rx.has_changed().unwrap() && reference.has_changed().unwrap());
            tokio::task::yield_now().await;
            compare(&mut rx, &mut reference, Status::Changed);
            let a = poll(pin!(rx.wait_for(|v| *v)));
            let b = poll(pin!(reference.wait_for(|v| *v)));
            let (Poll::Ready(Ok(a)), Poll::Ready(Ok(b))) = (a, b) else {
                panic!("current true missed")
            };
            assert!(*a && *b && !a.changed && !b.has_changed());
        })
        .await
        .unwrap();
    }

    #[test]
    fn private_watch_whole_grant_before_backing_and_same_closed_zero_root() {
        let bytes = body_bytes::<usize>();
        let short = Arc::new(Semaphore::new(bytes - 1));
        let error = with_allocation_hook(
            |_, _| panic!("short watch allocated"),
            || channel(0usize, &short).err().unwrap(),
        );
        assert_eq!(
            crate::output::operation_error_class(error.as_ref()),
            ios_core::ErrorClass::ResourceLimit
        );
        assert_eq!(short.available_permits(), bytes - 1);
        for (foreign, amount) in [(true, bytes), (false, bytes - 1), (false, bytes + 1)] {
            let bank = Arc::new(Semaphore::new(bytes * 2));
            let other = Arc::new(Semaphore::new(bytes * 2));
            let grant = super::super::admission::known_metadata_charge(
                if foreign { &other } else { &bank },
                amount,
            )
            .unwrap();
            let error = with_allocation_hook(
                |_, _| panic!("invalid watch grant allocated"),
                || from_grant(0usize, &bank, grant).err().unwrap(),
            );
            assert_eq!(
                crate::output::operation_error_class(error.as_ref()),
                ios_core::ErrorClass::ResourceLimit
            );
            assert_eq!(bank.available_permits(), bytes * 2);
            assert_eq!(other.available_permits(), bytes * 2);
        }
        let bank = bank::<usize>();
        let actual = Arc::new(Mutex::new(Vec::new()));
        let seen = actual.clone();
        let credit = bank.clone();
        let (tx, mut rx) = with_allocation_hook(
            move |layout, pointer| {
                assert_eq!(credit.available_permits(), 0);
                seen.lock().unwrap().push((layout.size(), pointer));
            },
            || channel(0usize, &bank).unwrap(),
        );
        let actual = actual.lock().unwrap();
        assert_eq!(actual.len(), 1);
        assert_eq!(actual[0].0, bytes);
        let value = {
            let guard = tx.borrow();
            &*guard as *const usize as usize
        };
        assert!(
            actual[0].1 <= value && value + std::mem::size_of::<usize>() <= actual[0].1 + bytes
        );
        drop(actual);
        bank.close();
        with_allocation_hook(
            |_, _| panic!("closed watch operation allocated"),
            || {
                let kept = tx.clone();
                let clone = rx.clone();
                let mut late = tx.subscribe();
                assert!(kept.shared.ptr_eq(&tx.shared) && clone.shared.ptr_eq(&rx.shared));
                tx.send_replace(1);
                assert_eq!(status(pin!(rx.changed())), Status::Changed);
                assert_eq!(status(pin!(late.changed())), Status::Changed);
                assert_eq!(*clone.borrow(), 1);
            },
        );
        assert_eq!(bank.available_permits(), 0);
        drop((tx, rx));
        assert_eq!(bank.available_permits(), bytes);
    }

    struct DropValue {
        address: Arc<AtomicUsize>,
        drops: Arc<AtomicUsize>,
        bank: Arc<Semaphore>,
        panic: bool,
        _pin: PhantomPinned,
    }
    impl Drop for DropValue {
        fn drop(&mut self) {
            assert_eq!(
                self as *const Self as usize,
                self.address.load(Ordering::SeqCst)
            );
            assert_eq!(self.bank.available_permits(), 0);
            self.drops.fetch_add(1, Ordering::SeqCst);
            if self.panic {
                std::panic::panic_any(OriginalPanic(Arc::new(())));
            }
        }
    }
    #[test]
    fn private_watch_original_pinned_value_drops_in_place_before_actual_free_and_fee_even_on_unwind(
    ) {
        for panic in [false, true] {
            let bytes = body_bytes::<DropValue>();
            let bank = Arc::new(Semaphore::new(bytes));
            let address = Arc::new(AtomicUsize::new(0));
            let drops = Arc::new(AtomicUsize::new(0));
            let (tx, rx) = channel(
                DropValue {
                    address: address.clone(),
                    drops: drops.clone(),
                    bank: bank.clone(),
                    panic,
                    _pin: PhantomPinned,
                },
                &bank,
            )
            .unwrap();
            address.store(&*rx.borrow() as *const DropValue as usize, Ordering::SeqCst);
            let weak = rx.shared.downgrade();
            bank.close();
            drop(tx);
            let result = std::panic::catch_unwind(AssertUnwindSafe(|| drop(rx)));
            assert_eq!(result.is_err(), panic);
            if let Err(original) = result {
                assert!(original.downcast_ref::<OriginalPanic>().is_some());
            }
            assert_eq!(drops.load(Ordering::SeqCst), 1);
            assert!(weak.upgrade().is_none());
            assert_eq!(bank.available_permits(), 0);
            let freed = Arc::new(AtomicBool::new(false));
            let mark = freed.clone();
            let credit = bank.clone();
            super::super::shared_weak_allocation::after_free(
                move || {
                    assert_eq!(credit.available_permits(), 0);
                    mark.store(true, Ordering::SeqCst);
                },
                || drop(weak),
            );
            assert!(freed.load(Ordering::SeqCst));
            assert_eq!(bank.available_permits(), bytes);
        }
    }

    #[test]
    fn private_watch_pending_last_receiver_cancellation_and_wrapping_keep_original_root() {
        let bytes = body_bytes::<usize>();
        let bank = bank::<usize>();
        let (tx, mut rx) = channel(0usize, &bank).unwrap();
        tx.shared.version.store(usize::MAX - 1, Ordering::Release);
        rx.seen = usize::MAX - 1;
        tx.send_replace(1);
        assert_eq!(tx.shared.version.load(Ordering::Acquire), 0);
        assert_eq!(status(pin!(rx.changed())), Status::Changed);
        assert_eq!(rx.seen, 0);
        let mut pending = Box::pin(async move { rx.changed().await });
        assert_eq!(status(pending.as_mut()), Status::Pending);
        bank.close();
        drop(tx);
        assert_eq!(bank.available_permits(), 0);
        let freed = Arc::new(AtomicBool::new(false));
        let mark = freed.clone();
        let credit = bank.clone();
        super::super::shared_weak_allocation::after_free(
            move || {
                assert_eq!(credit.available_permits(), 0);
                mark.store(true, Ordering::SeqCst);
            },
            || drop(pending),
        );
        assert!(freed.load(Ordering::SeqCst));
        assert_eq!(bank.available_permits(), bytes);
    }
}
