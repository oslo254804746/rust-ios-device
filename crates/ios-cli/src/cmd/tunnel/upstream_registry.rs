//! Sorted registered roots with actual array and text admission before creation.
use std::sync::Arc;

use anyhow::Result;
use serde::{Serialize, Serializer};
use tokio::sync::Semaphore;

use super::{metadata_vec::KnownVec, paid_json::Text, MAX_UPSTREAMS};

pub(super) struct Registry {
    rows: KnownVec<Text>,
    bank: Arc<Semaphore>,
}

impl Registry {
    pub(super) fn new(bank: &Arc<Semaphore>) -> Self {
        Self {
            rows: KnownVec::new(bank.clone(), MAX_UPSTREAMS),
            bank: bank.clone(),
        }
    }

    pub(super) fn from_urls<'a>(
        bank: &Arc<Semaphore>,
        urls: impl Iterator<Item = &'a String>,
    ) -> Result<Self> {
        let mut registry = Self::new(bank);
        for url in urls {
            registry.insert(url)?;
        }
        Ok(registry)
    }

    fn position(&self, url: &str) -> std::result::Result<usize, usize> {
        self.rows
            .as_slice()
            .binary_search_by(|row| row.as_str().cmp(url))
    }

    pub(super) fn contains(&self, url: &str) -> bool {
        self.position(url).is_ok()
    }

    pub(super) fn insert(&mut self, url: impl AsRef<str>) -> Result<bool> {
        let url = url.as_ref();
        let index = match self.position(url) {
            Ok(_) => return Ok(false),
            Err(index) => index,
        };
        if self.len() == MAX_UPSTREAMS {
            return Err(crate::output::resource_decision(
                "tunnel registered upstream entry limit",
            ));
        }
        // Both admissions precede mutation. If backing growth fails, only
        // this unused text is released; every original row/pointer survives.
        let text = Text::copy(url, &self.bank)?;
        self.rows.reserve(1)?;
        self.rows.insert_reserved(index, text);
        Ok(true)
    }

    pub(super) fn remove(&mut self, url: &str) -> Option<Text> {
        self.position(url).ok().map(|index| self.rows.remove(index))
    }

    pub(super) fn len(&self) -> usize {
        self.rows.len()
    }

    pub(super) fn iter(&self) -> impl Iterator<Item = &Text> {
        self.rows.as_slice().iter()
    }
}

impl Serialize for Registry {
    fn serialize<S: Serializer>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error> {
        self.rows.as_slice().serialize(serializer)
    }
}

#[cfg(test)]
mod tests;
