//! Move original publication entries; admit actual temporary backing first.
use std::collections::HashMap;
use std::sync::Arc;

use anyhow::Result;
use tokio::sync::Semaphore;

use super::{
    metadata_vec::KnownVec,
    scan::{Deadline, Epoch},
};

struct Row<T> {
    keep: bool,
    entry: Option<(String, T)>,
}

pub(super) struct Removed<T> {
    _rows: KnownVec<Row<T>>,
}

#[cfg(test)]
pub(super) fn first_bytes<T>() -> usize {
    std::mem::size_of::<Row<T>>()
}

struct Originals<'a, T> {
    table: &'a mut HashMap<String, T>,
    rows: KnownVec<Row<T>>,
    committed: bool,
}

impl<'a, T> Originals<'a, T> {
    fn prepare(table: &'a mut HashMap<String, T>, bank: &Arc<Semaphore>) -> Result<Self> {
        let mut rows = KnownVec::new(Arc::clone(bank), table.len());
        rows.reserve(table.len())?;
        // drain preserves the map's backing. All fallible preparation precedes
        // moving a key, value, native lease or publication fee owner.
        for entry in table.drain() {
            rows.push_reserved(Row {
                keep: false,
                entry: Some(entry),
            });
        }
        Ok(Self {
            table,
            rows,
            committed: false,
        })
    }

    fn restore(&mut self) {
        for row in self.rows.as_mut_slice() {
            if let Some((key, value)) = row.entry.take() {
                // No insertion can exceed the original len/capacity. Retained
                // rows already in the map and removed originals are disjoint.
                let old = self.table.insert(key, value);
                assert!(old.is_none(), "distinct original publication keys");
            }
        }
    }
}

impl<T> Drop for Originals<'_, T> {
    fn drop(&mut self) {
        if !self.committed {
            self.restore();
        }
    }
}

pub(super) fn retain<T>(
    epoch: &Epoch,
    clock: &Deadline,
    table: &mut HashMap<String, T>,
    bank: &Arc<Semaphore>,
    keep: impl Fn(&str, &T) -> bool,
) -> Result<Removed<T>> {
    // Preserve the actual epoch/clock decisions even with a depleted bank.
    epoch.commit(clock, || Ok(()))?;
    let mut originals = Originals::prepare(table, bank)?;
    for row in originals.rows.as_mut_slice() {
        let (key, value) = row.entry.as_ref().expect("original row before selection");
        row.keep = keep(key, value);
    }
    epoch.commit(clock, || {
        for row in originals.rows.as_mut_slice() {
            if row.keep {
                let (key, value) = row.entry.take().expect("selected original");
                let old = originals.table.insert(key, value);
                assert!(old.is_none(), "distinct original publication keys");
            }
        }
        #[cfg(test)]
        after_insert_test_hook();
        clock.check()?;
        originals.committed = true;
        Ok(())
    })?;
    // The watch guard ended before removed values and their original fees drop.
    Ok(Removed {
        _rows: originals.rows.take(),
    })
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
