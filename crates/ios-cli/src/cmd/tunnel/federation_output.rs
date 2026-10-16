//! Borrow paid local records and received JSON trees; overlay routes without copies.
use anyhow::Result;
use serde::ser::{SerializeMap, SerializeSeq};
use serde::{Serialize, Serializer};
use std::sync::Arc;
use tokio::sync::Semaphore;

use super::{metadata_vec::KnownVec, paid_json, PaidRecord, TunnelRecord};

struct Remote {
    listing: paid_json::Listing,
    upstream: paid_json::Text,
}

pub(super) struct Listing {
    local: KnownVec<PaidRecord>,
    remote: KnownVec<Remote>,
    bank: Arc<Semaphore>,
}

impl Listing {
    pub(super) fn new(mut local: KnownVec<PaidRecord>, bank: &Arc<Semaphore>) -> Self {
        local
            .as_mut_slice()
            .sort_unstable_by(|left, right| left.udid.cmp(&right.udid));
        Self {
            local,
            remote: KnownVec::new(bank.clone(), super::MAX_UPSTREAMS),
            bank: bank.clone(),
        }
    }

    pub(super) fn reserve_remote(&mut self, count: usize) -> Result<()> {
        self.remote.reserve(count)
    }
    pub(super) fn push_remote(&mut self, upstream: paid_json::Text, listing: paid_json::Listing) {
        self.remote.push_reserved(Remote { listing, upstream });
    }

    fn uid(&self, key: Key) -> &str {
        match key {
            Key::Local(index) => &self.local.as_slice()[index].udid,
            Key::Remote(source, group) => self.remote.as_slice()[source].listing.groups()[group]
                .0
                .as_str(),
        }
    }

    fn local_record(&self, uid: &str) -> Option<&TunnelRecord> {
        self.local
            .as_slice()
            .binary_search_by(|record| record.udid.as_str().cmp(uid))
            .ok()
            .map(|index| &*self.local.as_slice()[index])
    }

    pub(super) fn retain(&mut self, mut keep: impl FnMut(&str) -> bool) {
        self.local.retain(|record| keep(&record.udid));
        for remote in self.remote.as_mut_slice() {
            remote.listing.retain(&mut keep);
        }
    }

    pub(super) fn prepare(self) -> Result<Prepared> {
        let count = self
            .remote
            .as_slice()
            .iter()
            .try_fold(self.local.len(), |total, remote| {
                total.checked_add(remote.listing.groups().len())
            })
            .ok_or_else(|| {
                crate::output::resource_decision("federation merge index layout overflow")
            })?;
        let mut keys = KnownVec::new(self.bank.clone(), count);
        keys.reserve(count)?;
        for index in 0..self.local.len() {
            keys.push_reserved(Key::Local(index));
        }
        for (source, remote) in self.remote.as_slice().iter().enumerate() {
            for (group, (_, values, _)) in remote.listing.groups().iter().enumerate() {
                // Preserve the old merge: non-object entries and empty groups
                // contribute no output key. Remote target selection is separate.
                if !values
                    .array()
                    .expect("validated array")
                    .iter()
                    .any(|value| value.object().is_some())
                {
                    continue;
                }
                keys.push_reserved(Key::Remote(source, group));
            }
        }
        keys.as_mut_slice()
            .sort_unstable_by(|left, right| self.uid(*left).cmp(self.uid(*right)));
        keys.dedup_by(|left, right| self.uid(*left) == self.uid(*right));
        Ok(Prepared {
            listing: self,
            keys,
        })
    }
}

#[derive(Clone, Copy)]
enum Key {
    Local(usize),
    Remote(usize, usize),
}

pub(super) struct Prepared {
    listing: Listing,
    keys: KnownVec<Key>,
}
impl Serialize for Prepared {
    fn serialize<S: Serializer>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error> {
        let mut map = serializer.serialize_map(Some(self.keys.len()))?;
        for key in self.keys.as_slice() {
            let uid = self.listing.uid(*key);
            map.serialize_entry(
                uid,
                &Entries {
                    listing: &self.listing,
                    uid,
                },
            )?;
        }
        map.end()
    }
}

struct Entries<'a> {
    listing: &'a Listing,
    uid: &'a str,
}
impl Serialize for Entries<'_> {
    fn serialize<S: Serializer>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error> {
        let local = self.listing.local_record(self.uid);
        let count = usize::from(local.is_some())
            + self
                .listing
                .remote
                .as_slice()
                .iter()
                .map(|remote| {
                    remote
                        .listing
                        .entries(self.uid)
                        .unwrap_or_default()
                        .iter()
                        .filter(|entry| entry.object().is_some())
                        .count()
                })
                .sum::<usize>();
        let mut sequence = serializer.serialize_seq(Some(count))?;
        if let Some(record) = local {
            sequence.serialize_element(&LocalObject(record))?;
        }
        for remote in self.listing.remote.as_slice() {
            for entry in remote.listing.entries(self.uid).unwrap_or_default() {
                if let Some(fields) = entry.object() {
                    sequence.serialize_element(&RemoteObject {
                        fields,
                        upstream: remote.upstream.as_str(),
                    })?;
                }
            }
        }
        sequence.end()
    }
}

struct RemoteObject<'a> {
    fields: &'a [paid_json::Entry],
    upstream: &'a str,
}
impl Serialize for RemoteObject<'_> {
    fn serialize<S: Serializer>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error> {
        let existing = self
            .fields
            .binary_search_by(|(key, _, _)| key.as_str().cmp("upstream"))
            .is_ok();
        let mut map = serializer.serialize_map(Some(self.fields.len() + usize::from(!existing)))?;
        let mut written = false;
        for (key, value, _) in self.fields {
            if !written && key.as_str() >= "upstream" {
                map.serialize_entry("upstream", self.upstream)?;
                written = true;
            }
            if key.as_str() != "upstream" {
                map.serialize_entry(key, value)?;
            }
        }
        if !written {
            map.serialize_entry("upstream", self.upstream)?;
        }
        map.end()
    }
}

// Root listing historically serialized local records through Value's sorted
// object map. Preserve that key order and the same optional-field omissions.
struct LocalObject<'a>(&'a TunnelRecord);
impl Serialize for LocalObject<'_> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let record = self.0;
        let count = 9
            + usize::from(record.userspace_port.is_some())
            + usize::from(record.userspace_tun_host.is_some())
            + usize::from(record.userspace_tun_port.is_some());
        let mut map = serializer.serialize_map(Some(count))?;
        map.serialize_entry("address", &record.address)?;
        map.serialize_entry("connectionType", &record.connection_type)?;
        map.serialize_entry("interface", &record.interface)?;
        map.serialize_entry("mode", &record.mode)?;
        map.serialize_entry("rsdPort", &record.rsd_port)?;
        map.serialize_entry("tunnel-address", &record.tunnel_address)?;
        map.serialize_entry("tunnel-port", &record.tunnel_port)?;
        map.serialize_entry("udid", &record.udid)?;
        if let Some(port) = record.userspace_port {
            map.serialize_entry("userspace-port", &port)?;
        }
        map.serialize_entry("userspaceTun", &record.userspace_tun)?;
        if let Some(host) = &record.userspace_tun_host {
            map.serialize_entry("userspaceTunHost", host)?;
        }
        if let Some(port) = record.userspace_tun_port {
            map.serialize_entry("userspaceTunPort", &port)?;
        }
        map.end()
    }
}

#[cfg(test)]
mod tests;
