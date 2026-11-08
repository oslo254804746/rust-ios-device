//! Creation-owned userspace packet, queue and socket capacity. No heap/RSS claim.

use std::collections::VecDeque;
use std::mem;
use std::ops::Deref;
use std::sync::{Arc, OnceLock};

use tokio::sync::{OwnedSemaphorePermit, Semaphore};

use super::owned_buffer::{rejected, BufferAdmission, OwnedBuffer};
use super::{watch, Ledger, ParentRole, TunnelError};

#[derive(Clone)]
pub(super) struct RuntimeStorage {
    pub(super) bank: Option<Arc<Semaphore>>,
    pub(super) ledger: Ledger,
    pub(super) stop: watch::Sender,
}

impl RuntimeStorage {
    pub(super) fn fail(&self, role: ParentRole, original: TunnelError) {
        // The original cause becomes durable before any worker sees stop. The
        // existing ledger retains per-role errors and original typed join slots.
        self.ledger.record_work(role as usize, Arc::new(original));
        self.ledger.request_runtime_stop();
        self.stop.send_replace(());
    }

    pub(super) fn result(&self) -> Result<(), TunnelError> {
        self.ledger.work_result()
    }

    pub(super) fn stopped(&self) -> bool {
        self.ledger.is_runtime_stopped()
    }

    #[cfg(test)]
    pub(super) fn unmetered() -> Self {
        let ledger = Ledger::new();
        let (stop, _) = watch::channel(ledger.clone());
        Self {
            bank: None,
            ledger,
            stop,
        }
    }
}

pub(crate) fn charge(
    bank: Option<&Arc<Semaphore>>,
    bytes: usize,
) -> Result<Option<OwnedSemaphorePermit>, TunnelError> {
    bank.map(|bank| {
        let count = u32::try_from(bytes)
            .map_err(|original| rejected(BufferAdmission::Overflow(original)))?;
        Arc::clone(bank)
            .try_acquire_many_owned(count)
            .map_err(|_| rejected(BufferAdmission::Exhausted))
    })
    .transpose()
}

/// An actual complete native control/handle root. Workers only clone control
/// roots; none can retain a root which owns its own JoinHandle.
pub(super) enum RuntimeRoot<T> {
    Legacy(Arc<T>),
    Metered(super::owned_channel::roots::Root<T>),
}
impl<T> RuntimeRoot<T> {
    pub(super) fn new(value: T, bank: Option<&Arc<Semaphore>>) -> Result<Self, TunnelError> {
        let Some(bank) = bank else {
            return Ok(Self::Legacy(Arc::new(value)));
        };
        let storage = charge(Some(bank), Self::body_bytes())?.expect("whole native root grant");
        let destination = super::owned_channel::roots::Destination::from_grant(bank, storage)?;
        Ok(Self::Metered(destination.finish(value)))
    }
    pub(super) fn body_bytes() -> usize {
        super::owned_channel::roots::Root::<T>::body_bytes()
    }
}
impl<T> Clone for RuntimeRoot<T> {
    fn clone(&self) -> Self {
        match self {
            Self::Legacy(value) => Self::Legacy(value.clone()),
            Self::Metered(value) => Self::Metered(value.clone()),
        }
    }
}
impl<T> Deref for RuntimeRoot<T> {
    type Target = T;
    fn deref(&self) -> &T {
        match self {
            Self::Legacy(value) => value,
            Self::Metered(value) => value,
        }
    }
}

struct TaskSlots {
    values: [OnceLock<super::OwnedTask>; 4],
}

#[derive(Clone)]
pub(super) struct TaskHandles {
    slots: RuntimeRoot<TaskSlots>,
}
pub(super) type TaskIter<'a> = std::iter::FilterMap<
    std::slice::Iter<'a, OnceLock<super::OwnedTask>>,
    fn(&'a OnceLock<super::OwnedTask>) -> Option<&'a super::OwnedTask>,
>;
impl TaskHandles {
    #[cfg(test)]
    pub(super) fn initialize(&mut self, tasks: [super::OwnedTask; 4]) {
        for (index, task) in tasks.into_iter().enumerate() {
            self.install(index, task);
        }
    }
    pub(super) fn install(&self, index: usize, task: super::OwnedTask) {
        assert!(
            self.slots.values[index].set(task).is_ok(),
            "one original task per launch slot"
        );
    }
    pub(super) fn iter(&self) -> TaskIter<'_> {
        self.slots.values.iter().filter_map(OnceLock::get)
    }
    pub(super) fn get(&self, index: usize) -> Option<&super::OwnedTask> {
        self.slots.values[index].get()
    }
    pub(super) fn all_installed(&self) -> bool {
        self.slots.values.iter().all(|slot| slot.get().is_some())
    }
    #[cfg(test)]
    pub(super) fn body_bytes() -> usize {
        RuntimeRoot::<TaskSlots>::body_bytes()
    }
    #[cfg(test)]
    pub(super) fn len(&self) -> usize {
        self.iter().count()
    }
}
impl<'a> IntoIterator for &'a TaskHandles {
    type Item = &'a super::OwnedTask;
    type IntoIter = TaskIter<'a>;
    fn into_iter(self) -> Self::IntoIter {
        self.iter()
    }
}

pub(super) fn task_handles(bank: Option<&Arc<Semaphore>>) -> Result<TaskHandles, TunnelError> {
    Ok(TaskHandles {
        slots: RuntimeRoot::new(
            TaskSlots {
                values: std::array::from_fn(|_| OnceLock::new()),
            },
            bank,
        )?,
    })
}

/// Real backing follows the queue. Growth prepays the replacement allocation
/// while the original backing remains charged; no post-read guessed top-up.
pub(super) struct OwnedQueue<T> {
    items: VecDeque<T>,
    bank: Option<Arc<Semaphore>>,
    // Declared after items, and replaced only after the original allocation dies.
    backing: Option<OwnedSemaphorePermit>,
}

impl<T> OwnedQueue<T> {
    pub(super) fn new(bank: Option<&Arc<Semaphore>>) -> Self {
        Self {
            items: VecDeque::new(),
            bank: bank.cloned(),
            backing: None,
        }
    }

    pub(super) fn prepare_push(&mut self) -> Result<(), TunnelError> {
        if self.items.len() < self.items.capacity() {
            return Ok(());
        }
        let capacity = self
            .items
            .capacity()
            .checked_mul(2)
            .and_then(|capacity| capacity.max(4).checked_next_power_of_two())
            .ok_or_else(|| rejected(BufferAdmission::SizeOverflow))?;
        let bytes = capacity
            .checked_mul(mem::size_of::<T>())
            .ok_or_else(|| rejected(BufferAdmission::SizeOverflow))?;
        let mut backing = charge(self.bank.as_ref(), bytes)?;
        let mut replacement = VecDeque::new();
        replacement
            .try_reserve_exact(capacity)
            .map_err(|original| rejected(BufferAdmission::Allocation(original)))?;
        let actual = replacement
            .capacity()
            .checked_mul(mem::size_of::<T>())
            .ok_or_else(|| rejected(BufferAdmission::SizeOverflow))?;
        if actual > bytes {
            return Err(rejected(BufferAdmission::UnexpectedCapacity));
        }
        backing = backing.map(|mut fee| {
            let actual = fee.split(actual).expect("prepaid queue backing");
            drop(fee);
            actual
        });
        replacement.append(&mut self.items);
        drop(mem::replace(&mut self.items, replacement));
        self.backing = backing;
        Ok(())
    }

    pub(super) fn push_back(&mut self, value: T) -> Result<(), (TunnelError, T)> {
        if let Err(original) = self.prepare_push() {
            return Err((original, value));
        }
        self.push_reserved(value);
        Ok(())
    }

    pub(super) fn push_reserved(&mut self, value: T) {
        assert!(
            self.items.len() < self.items.capacity(),
            "prepared queue slot missing"
        );
        self.items.push_back(value);
    }

    pub(super) fn push_front_reserved(&mut self, value: T) {
        assert!(
            self.items.len() < self.items.capacity(),
            "returned queue slot missing"
        );
        self.items.push_front(value);
    }

    pub(super) fn pop_front(&mut self) -> Option<T> {
        self.items.pop_front()
    }
    pub(super) fn is_empty(&self) -> bool {
        self.items.is_empty()
    }
    pub(super) fn clear(&mut self) {
        self.items.clear();
    }
    pub(super) fn iter(&self) -> impl Iterator<Item = &T> {
        self.items.iter()
    }
    pub(super) fn iter_mut(&mut self) -> impl Iterator<Item = &mut T> {
        self.items.iter_mut()
    }
    pub(super) fn remove(&mut self, index: usize) -> Option<T> {
        self.items.remove(index)
    }

    #[cfg(test)]
    pub(super) fn len(&self) -> usize {
        self.items.len()
    }
    #[cfg(test)]
    pub(super) fn backing_bytes(&self) -> usize {
        self.items.capacity() * mem::size_of::<T>()
    }
    #[cfg(test)]
    pub(super) fn values(&self) -> &VecDeque<T> {
        &self.items
    }
}

/// The smoltcp-owned Vec buffers and these permits are split only at the socket
/// construction boundary. Private loop drop/removal order releases Socket first.
pub(super) struct SocketFees {
    _rx: Option<OwnedSemaphorePermit>,
    _tx: Option<OwnedSemaphorePermit>,
}

pub(super) fn socket_buffers(
    bank: Option<&Arc<Semaphore>>,
    size: usize,
) -> Result<(Vec<u8>, Vec<u8>, SocketFees), TunnelError> {
    let rx = OwnedBuffer::zeroed(size, bank)?;
    let tx = OwnedBuffer::zeroed(size, bank)?;
    let (rx, rx_fee) = rx.into_socket_parts();
    let (tx, tx_fee) = tx.into_socket_parts();
    Ok((
        rx,
        tx,
        SocketFees {
            _rx: rx_fee,
            _tx: tx_fee,
        },
    ))
}

#[cfg(test)]
mod stop_tests {
    use super::*;
    use crate::device::prepared_rsd::with_allocation_hook;
    use crate::tunnel::owner::LedgerState;
    use std::{
        error::Error,
        fmt, io,
        sync::atomic::{AtomicUsize, Ordering},
    };

    #[test]
    fn runtime_stop_clones_use_the_original_paid_ledger_at_closed_zero_quota_and_isolate_other_runtimes(
    ) {
        let bytes = Ledger::reservation_bytes();
        let first_bank = Arc::new(Semaphore::new(bytes));
        let second_bank = Arc::new(Semaphore::new(bytes));
        let first_ledger = Ledger::with_metadata(Some(&first_bank)).unwrap();
        let second_ledger = Ledger::with_metadata(Some(&second_bank)).unwrap();
        let (first_stop, _) = watch::channel(first_ledger.clone());
        let (second_stop, _) = watch::channel(second_ledger.clone());
        let first = RuntimeStorage {
            bank: Some(first_bank.clone()),
            ledger: first_ledger.clone(),
            stop: first_stop,
        };
        let second = RuntimeStorage {
            bank: Some(second_bank.clone()),
            ledger: second_ledger.clone(),
            stop: second_stop,
        };
        let first_state = (&*first_ledger as *const LedgerState) as usize;
        let second_state = (&*second_ledger as *const LedgerState) as usize;
        assert_ne!(first_state, second_state);
        first_bank.close();
        second_bank.close();
        let kept = with_allocation_hook(
            |_, _| panic!("stop views allocated a controlled backing"),
            || {
                let clone = first.clone();
                assert_eq!((&*clone.ledger as *const LedgerState) as usize, first_state);
                assert!(!clone.stopped() && !first.stopped() && !second.stopped());
                first_ledger.request_runtime_stop();
                assert!(clone.stopped() && first.stopped());
                assert!(!second.stopped());
                clone
            },
        );
        assert_eq!(first_bank.available_permits(), 0);
        assert_eq!(second_bank.available_permits(), 0);
        drop((first, first_ledger));
        assert!(kept.stopped());
        assert_eq!(first_bank.available_permits(), 0);
        drop(kept);
        assert_eq!(first_bank.available_permits(), bytes);
        assert_eq!(second_bank.available_permits(), 0);
        assert!(!second.stopped());
        drop((second, second_ledger));
        assert_eq!(second_bank.available_permits(), bytes);
    }

    struct OriginalFailure(Arc<AtomicUsize>);
    impl fmt::Debug for OriginalFailure {
        fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            f.write_str("OriginalFailure")
        }
    }
    impl fmt::Display for OriginalFailure {
        fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            f.write_str("original runtime failure")
        }
    }
    impl Error for OriginalFailure {}
    impl Drop for OriginalFailure {
        fn drop(&mut self) {
            self.0.fetch_add(1, Ordering::SeqCst);
        }
    }
    fn failure(error: &TunnelError) -> &crate::TunnelShutdownFailure {
        let mut current: Option<&(dyn Error + 'static)> = Some(error);
        for _ in 0..32 {
            let value = current.expect("original runtime report missing");
            if let Some(report) = value.downcast_ref() {
                return report;
            }
            current = value.source();
        }
        panic!("original runtime report depth changed");
    }

    #[tokio::test]
    async fn runtime_failure_is_durable_before_original_stop_notification_with_exact_cause_at_closed_zero_quota(
    ) {
        let bytes = Ledger::reservation_bytes();
        let bank = Arc::new(Semaphore::new(bytes));
        let ledger = Ledger::with_metadata(Some(&bank)).unwrap();
        let (stop, mut notified) = watch::channel(ledger.clone());
        let storage = RuntimeStorage {
            bank: Some(bank.clone()),
            ledger,
            stop,
        };
        let state = (&*storage.ledger as *const LedgerState) as usize;
        let worker = storage.clone();
        let drops = Arc::new(AtomicUsize::new(0));
        let error = io::Error::new(
            io::ErrorKind::PermissionDenied,
            OriginalFailure(drops.clone()),
        );
        let pointer = error.get_ref().unwrap() as *const dyn Error as *const () as usize;
        bank.close();
        let state_before = (&*worker.ledger as *const LedgerState) as usize;
        assert_eq!(state_before, state);
        let task = tokio::spawn(async move {
            with_allocation_hook(
                |_, _| panic!("runtime failure created a new controlled stop root"),
                || worker.fail(ParentRole::Writer, TunnelError::Io(error)),
            );
        });
        notified.changed().await.unwrap();
        assert!(
            storage.stopped(),
            "stop notification preceded the original release store"
        );
        let first = storage.result().unwrap_err();
        let second = storage.result().unwrap_err();
        task.await.unwrap();
        let mut original = None;
        for report in [&first, &second] {
            assert_eq!(
                crate::classify_error(report),
                crate::ErrorClass::PermissionDenied
            );
            let report = failure(report);
            assert_eq!(report.pending_joins(), 0);
            assert_eq!(report.cleanup().count(), 0);
            let cause = report.primary().downcast_ref::<TunnelError>().unwrap();
            let original_pointer = cause as *const TunnelError;
            if let Some(previous) = original {
                assert_eq!(original_pointer, previous);
            }
            original = Some(original_pointer);
            let TunnelError::Io(source) = cause else {
                panic!("original IO variant changed");
            };
            assert_eq!(
                source.get_ref().unwrap() as *const dyn Error as *const () as usize,
                pointer
            );
            assert!(source.get_ref().unwrap().is::<OriginalFailure>());
        }
        assert_eq!(bank.available_permits(), 0);
        drop((storage, notified));
        assert_eq!(bank.available_permits(), Ledger::body_bytes());
        assert_eq!(drops.load(Ordering::SeqCst), 0);
        drop(first);
        assert_eq!(bank.available_permits(), Ledger::body_bytes());
        drop(second);
        assert_eq!(bank.available_permits(), bytes);
        assert_eq!(drops.load(Ordering::SeqCst), 1);
    }
}
