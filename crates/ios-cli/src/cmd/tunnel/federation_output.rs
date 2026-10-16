//! Borrow the admitted local records instead of creating a second Value graph.
//! Received upstream Value graphs remain an independent creation boundary.
use serde::ser::{SerializeMap, SerializeSeq};
use serde::{Serialize, Serializer};

use super::{metadata_vec::KnownVec, FederationListing, PaidRecord, TunnelRecord};

pub(super) struct Listing {
    local: KnownVec<PaidRecord>,
    pub(super) remote: FederationListing,
}

impl Listing {
    pub(super) fn new(mut local: KnownVec<PaidRecord>) -> Self {
        // Unstable in-place sort has no scratch allocation. Each local UID is
        // unique in the publication table; binary search avoids a merge index.
        local
            .as_mut_slice()
            .sort_unstable_by(|left, right| left.udid.cmp(&right.udid));
        Self {
            local,
            remote: FederationListing::new(),
        }
    }

    fn local_contains(&self, uid: &str) -> bool {
        self.local
            .as_slice()
            .binary_search_by(|record| record.udid.as_str().cmp(uid))
            .is_ok()
    }

    pub(super) fn retain(&mut self, mut keep: impl FnMut(&str) -> bool) {
        self.local.retain(|record| keep(&record.udid));
        self.remote.retain(|uid, _| keep(uid));
    }
}

impl Serialize for Listing {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let count = self.local.len()
            + self
                .remote
                .keys()
                .filter(|uid| !self.local_contains(uid))
                .count();
        let mut map = serializer.serialize_map(Some(count))?;
        for record in self.local.as_slice() {
            map.serialize_entry(
                &record.udid,
                &Entries {
                    local: record,
                    remote: self
                        .remote
                        .get(&record.udid)
                        .map(Vec::as_slice)
                        .unwrap_or_default(),
                },
            )?;
        }
        for (uid, entries) in &self.remote {
            if !self.local_contains(uid) {
                map.serialize_entry(uid, entries)?;
            }
        }
        map.end()
    }
}

struct Entries<'a> {
    local: &'a TunnelRecord,
    remote: &'a [serde_json::Value],
}
impl Serialize for Entries<'_> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut sequence = serializer.serialize_seq(Some(1 + self.remote.len()))?;
        sequence.serialize_element(&LocalObject(self.local))?;
        for entry in self.remote {
            sequence.serialize_element(entry)?;
        }
        sequence.end()
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
