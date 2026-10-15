//! Real keyed window counters; no estimate of HashMap buckets or scratch.
use crate::xpc::raw_storage::{exact_vec, limit};
use std::collections::HashMap;
use std::io;
use std::sync::Arc;
use tokio::sync::{OwnedSemaphorePermit, Semaphore};

pub(super) enum WindowMap<V> {
    Legacy(HashMap<u32, V>),
    Banked(Counters<V>),
}
pub(super) struct Counters<V> {
    values: Vec<(u32, V)>,
    bank: Arc<Semaphore>,
    fee: Option<OwnedSemaphorePermit>,
}
#[cfg(test)]
type AdmissionHook = std::cell::RefCell<Option<Box<dyn FnMut(usize)>>>;
#[cfg(test)]
thread_local! {
    pub(super) static ADMISSION: AdmissionHook = const { std::cell::RefCell::new(None) };
}
impl<V> WindowMap<V> {
    pub(super) fn new(bank: Option<&Arc<Semaphore>>) -> Self {
        match bank {
            None => Self::Legacy(HashMap::new()),
            Some(bank) => Self::Banked(Counters {
                values: Vec::new(),
                bank: Arc::clone(bank),
                fee: None,
            }),
        }
    }
    // All new keys for one receive-credit batch are admitted together, before
    // any counter resets or WINDOW_UPDATE bytes are committed.
    pub(super) fn prepare_keys(&mut self, keys: &[u32]) -> io::Result<()> {
        let Self::Banked(owner) = self else {
            return Ok(());
        };
        let missing = keys
            .iter()
            .enumerate()
            .filter(|(index, key)| {
                !keys[..*index].contains(key) && !owner.values.iter().any(|(id, _)| id == *key)
            })
            .count();
        let needed = owner.values.len().checked_add(missing).ok_or_else(limit)?;
        if needed <= owner.values.capacity() {
            return Ok(());
        }
        let capacity = owner
            .values
            .capacity()
            .max(2)
            .checked_mul(2)
            .ok_or_else(limit)?
            .max(needed);
        let _bytes = capacity
            .checked_mul(std::mem::size_of::<(u32, V)>())
            .ok_or_else(limit)?;
        #[cfg(test)]
        ADMISSION.with(|hook| {
            if let Some(callback) = hook.borrow_mut().as_mut() {
                callback(_bytes);
            }
        });
        let (fee, mut replacement) = exact_vec(&owner.bank, capacity)?;
        replacement.append(&mut owner.values);
        drop(std::mem::replace(&mut owner.values, replacement));
        owner.fee = Some(fee);
        Ok(())
    }
    pub(super) fn insert_reserved(&mut self, key: u32, value: V) -> Option<V> {
        match self {
            Self::Legacy(map) => map.insert(key, value),
            Self::Banked(owner) => {
                if let Some((_, old)) = owner.values.iter_mut().find(|(id, _)| *id == key) {
                    return Some(std::mem::replace(old, value));
                }
                assert!(
                    owner.values.len() < owner.values.capacity(),
                    "window key not admitted"
                );
                owner.values.push((key, value));
                None
            }
        }
    }
    pub(super) fn get(&self, key: &u32) -> Option<&V> {
        match self {
            Self::Legacy(map) => map.get(key),
            Self::Banked(owner) => owner
                .values
                .iter()
                .find(|(id, _)| id == key)
                .map(|(_, value)| value),
        }
    }
    pub(super) fn get_mut(&mut self, key: &u32) -> Option<&mut V> {
        match self {
            Self::Legacy(map) => map.get_mut(key),
            Self::Banked(owner) => owner
                .values
                .iter_mut()
                .find(|(id, _)| id == key)
                .map(|(_, value)| value),
        }
    }
    pub(super) fn contains_key(&self, key: &u32) -> bool {
        self.get(key).is_some()
    }
    pub(super) fn remove(&mut self, key: &u32) -> Option<V> {
        match self {
            Self::Legacy(map) => map.remove(key),
            Self::Banked(owner) => {
                let index = owner.values.iter().position(|(id, _)| id == key)?;
                Some(owner.values.swap_remove(index).1)
            }
        }
    }
    pub(super) fn clear(&mut self) {
        match self {
            Self::Legacy(map) => map.clear(),
            Self::Banked(owner) => owner.values.clear(),
        }
    }
    pub(super) fn iter(&self) -> Iter<'_, V> {
        match self {
            Self::Legacy(map) => Iter::Legacy(map.iter()),
            Self::Banked(owner) => Iter::Banked(owner.values.iter()),
        }
    }
    pub(super) fn iter_mut(&mut self) -> impl Iterator<Item = &mut V> {
        enum Values<'a, V> {
            Legacy(std::collections::hash_map::ValuesMut<'a, u32, V>),
            Banked(std::slice::IterMut<'a, (u32, V)>),
        }
        impl<'a, V> Iterator for Values<'a, V> {
            type Item = &'a mut V;
            fn next(&mut self) -> Option<Self::Item> {
                match self {
                    Self::Legacy(values) => values.next(),
                    Self::Banked(values) => values.next().map(|(_, value)| value),
                }
            }
        }
        match self {
            Self::Legacy(map) => Values::Legacy(map.values_mut()),
            Self::Banked(owner) => Values::Banked(owner.values.iter_mut()),
        }
    }
    #[cfg(test)]
    pub(super) fn is_empty(&self) -> bool {
        self.iter().next().is_none()
    }
    #[cfg(test)]
    pub(super) fn backing_bytes(&self) -> usize {
        match self {
            Self::Legacy(_) => 0,
            Self::Banked(owner) => owner.values.capacity() * std::mem::size_of::<(u32, V)>(),
        }
    }
}
pub(super) enum Iter<'a, V> {
    Legacy(std::collections::hash_map::Iter<'a, u32, V>),
    Banked(std::slice::Iter<'a, (u32, V)>),
}
impl<'a, V> Iterator for Iter<'a, V> {
    type Item = (&'a u32, &'a V);
    fn next(&mut self) -> Option<Self::Item> {
        match self {
            Self::Legacy(values) => values.next(),
            Self::Banked(values) => values.next().map(|(key, value)| (key, value)),
        }
    }
}
impl<V> std::ops::Index<&u32> for WindowMap<V> {
    type Output = V;
    fn index(&self, key: &u32) -> &V {
        self.get(key).expect("existing window key")
    }
}

#[cfg(test)]
mod tests;
