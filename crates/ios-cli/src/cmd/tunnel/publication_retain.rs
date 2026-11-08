//! Allocation-free pruning of original paid publication nodes and slots.
pub(super) use super::publication_table::Removed;
use super::{
    publication_table::Table,
    scan::{Deadline, Epoch},
};
use anyhow::Result;
use std::sync::Arc;
use tokio::sync::Semaphore;

#[cfg(test)]
pub(super) fn first_bytes<T>() -> usize {
    // Intrusive removal metadata is part of each actual pre-Source Node.
    // No scratch is created at prune time, for any value layout.
    let _ = std::mem::size_of::<T>();
    0
}

pub(super) fn retain<T>(
    epoch: &Epoch,
    clock: &Deadline,
    table: &mut Table<T>,
    _bank: &Arc<Semaphore>,
    keep: impl Fn(&str, &T) -> bool,
) -> Result<Removed<T>> {
    epoch.commit(clock, || Ok(()))?;
    let mut originals = table.selection();
    originals.select(keep);
    epoch.commit(clock, || {
        #[cfg(test)]
        after_insert_test_hook();
        clock.check()
    })?;
    // Selection's Drop restores original node/key/value/slot on every error or
    // predicate unwind. Commit transfers the paid original nodes to the caller.
    Ok(originals.commit())
}

#[cfg(test)]
thread_local! {
    static AFTER_INSERT: std::cell::RefCell<Option<Box<dyn FnOnce()>>> = const { std::cell::RefCell::new(None) };
}
#[cfg(test)]
fn after_insert_test_hook() {
    let hook = AFTER_INSERT.with(|callback| callback.borrow_mut().take());
    if let Some(hook) = hook {
        hook();
    }
}
#[cfg(test)]
pub(super) fn on_after_insert(callback: impl FnOnce() + 'static) {
    AFTER_INSERT.with(|slot| {
        assert!(slot.borrow().is_none());
        *slot.borrow_mut() = Some(Box::new(callback));
    });
}
#[cfg(test)]
mod tests;
