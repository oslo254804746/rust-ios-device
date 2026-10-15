//! Byte-owned current RSD catalogue, prepared without cloning into an opaque HashMap.
//! Charges cover requested application-owned layouts, not allocator headers, Arc internals,
//! OS sockets or process RSS. The checked prepared path separately admits raw
//! payload/body storage and an explicit borrowed parser arena from the same bank.
use super::existing_rsd::{ExistingRsdProbeError, RsdSelectionReceipt};
#[cfg(test)]
use crate::xpc::message::XpcValue;
use crate::{CoreError, XpcError};
#[cfg(test)]
use indexmap::IndexMap;
use std::{
    alloc::{alloc, Layout},
    ptr::NonNull,
    sync::Arc,
};
use tokio::sync::{OwnedSemaphorePermit, Semaphore};

pub(crate) fn charge(
    bank: &Arc<Semaphore>,
    bytes: usize,
) -> Result<OwnedSemaphorePermit, XpcError> {
    let bytes = u32::try_from(bytes).map_err(|_| ExistingRsdProbeError::metadata_limit())?;
    bank.clone()
        .try_acquire_many_owned(bytes)
        .map_err(|_| ExistingRsdProbeError::metadata_limit())
}
/// Allocate precisely the charged layout; unlike try_reserve_exact, this does
/// not ask an implementation to choose a capacity that can exceed the grant.
pub(crate) fn exact_vec<T>(
    bank: &Arc<Semaphore>,
    capacity: usize,
) -> Result<(OwnedSemaphorePermit, Vec<T>), XpcError> {
    let layout =
        Layout::array::<T>(capacity).map_err(|_| ExistingRsdProbeError::metadata_limit())?;
    let fee = charge(bank, layout.size())?;
    if layout.size() == 0 {
        return Ok((fee, Vec::with_capacity(capacity)));
    }
    // SAFETY: allocation uses T's exact nonzero layout; no initialized T is
    // assumed. Vec owns the same pointer/layout and initializes only via push.
    let pointer = unsafe { alloc(layout) };
    let pointer =
        NonNull::new(pointer.cast::<T>()).ok_or_else(ExistingRsdProbeError::metadata_limit)?;
    let values = unsafe { Vec::from_raw_parts(pointer.as_ptr(), 0, capacity) };
    Ok((fee, values))
}
struct ExactText {
    text: String,
    _fee: OwnedSemaphorePermit,
}
impl ExactText {
    fn copy(bank: &Arc<Semaphore>, text: &str) -> Result<Self, XpcError> {
        let (fee, mut bytes) = exact_vec::<u8>(bank, text.len())?;
        bytes.extend_from_slice(text.as_bytes());
        // SAFETY: copied UTF-8 bytes from a str; exact allocation remains owned.
        Ok(Self {
            text: unsafe { String::from_utf8_unchecked(bytes) },
            _fee: fee,
        })
    }
}
struct Row {
    name: ExactText,
    port: u16,
    features: Option<Vec<ExactText>>,
    _features_fee: Option<OwnedSemaphorePermit>,
}

/// Current checked directory with immutable original buffers and their grants.
/// Missing feature metadata remains None; this does not certify a service ACK.
pub struct PreparedRsdCatalogue {
    udid: ExactText,
    rows: Vec<Row>,
    selection: Option<RsdSelectionReceipt>,
    bytes: usize,
    // Fees drop after their corresponding owned buffers above.
    _rows_fee: OwnedSemaphorePermit,
    _selection_fee: Option<OwnedSemaphorePermit>,
    _body_fee: OwnedSemaphorePermit,
}
impl std::fmt::Debug for PreparedRsdCatalogue {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PreparedRsdCatalogue")
            .field("services", &self.rows.len())
            .field("known_bytes", &self.bytes)
            .finish_non_exhaustive()
    }
}
impl PreparedRsdCatalogue {
    pub fn udid(&self) -> &str {
        &self.udid.text
    }
    pub fn known_bytes(&self) -> usize {
        self.bytes
    }
    pub fn belongs_to(&self, bank: &Arc<Semaphore>) -> bool {
        Arc::ptr_eq(self._body_fee.semaphore(), bank)
    }
    pub fn selection(&self) -> Option<&RsdSelectionReceipt> {
        self.selection.as_ref()
    }
    pub fn service_port(&self, name: &str) -> Option<u16> {
        self.find(name).map(|row| row.port)
    }
    pub fn service_features(&self, name: &str) -> Option<impl Iterator<Item = &str>> {
        self.find(name)?
            .features
            .as_ref()
            .map(|features| features.iter().map(|feature| feature.text.as_str()))
    }
    fn find(&self, name: &str) -> Option<&Row> {
        self.rows
            .binary_search_by(|row| row.name.text.as_str().cmp(name))
            .ok()
            .map(|index| &self.rows[index])
    }
    pub(super) fn resolved(
        &self,
        requested: &str,
    ) -> Option<super::service_routing::ResolvedRsdService> {
        let row = self.find(requested).or_else(|| {
            self.rows
                .iter()
                .find(|row| row.name.text.strip_suffix(".shim.remote") == Some(requested))
        })?;
        Some(super::service_routing::ResolvedRsdService {
            resolved_service_name: row.name.text.clone(),
            port: row.port,
            features: row.features.as_ref().map_or_else(Vec::new, |features| {
                features
                    .iter()
                    .map(|feature| feature.text.clone())
                    .collect()
            }),
        })
    }
    #[cfg(test)]
    pub(crate) fn decode(
        dictionary: &IndexMap<String, XpcValue>,
        bank: &Arc<Semaphore>,
    ) -> Result<Self, XpcError> {
        let (uid, services) = crate::xpc::rsd::checked_handshake_parts(dictionary)?;
        let uid =
            uid.ok_or_else(|| XpcError::Tls("RSD: current catalogue missing identity".into()))?;
        let services = services
            .ok_or_else(|| XpcError::Tls("RSD: current catalogue missing Services".into()))?;
        // Validate every required port before retaining any catalogue prefix.
        for (_, value) in services {
            let descriptor = value.as_dict().expect("checked service dictionary");
            crate::xpc::rsd::checked_service_port(
                descriptor
                    .get("Port")
                    .ok_or_else(|| XpcError::Tls("RSD: current catalogue missing Port".into()))?,
            )?;
        }
        let body_fee = charge(bank, std::mem::size_of::<Self>())?;
        let (rows_fee, mut rows) = exact_vec::<Row>(bank, services.len())?;
        let udid = ExactText::copy(bank, uid)?;
        let mut bytes = std::mem::size_of::<Self>()
            .checked_add(udid.text.capacity())
            .and_then(|bytes| bytes.checked_add(rows.capacity() * std::mem::size_of::<Row>()))
            .ok_or_else(ExistingRsdProbeError::metadata_limit)?;
        for (name, value) in services {
            let descriptor = value.as_dict().expect("checked service dictionary");
            let name = ExactText::copy(bank, name)?;
            bytes = bytes
                .checked_add(name.text.capacity())
                .ok_or_else(ExistingRsdProbeError::metadata_limit)?;
            let raw_features = descriptor
                .get("Properties")
                .and_then(XpcValue::as_dict)
                .and_then(|properties| properties.get("Features"));
            let (features_fee, features) = match raw_features {
                None => (None, None),
                Some(XpcValue::Array(raw)) => {
                    let (fee, mut features) = exact_vec::<ExactText>(bank, raw.len())?;
                    bytes = bytes
                        .checked_add(features.capacity() * std::mem::size_of::<ExactText>())
                        .ok_or_else(ExistingRsdProbeError::metadata_limit)?;
                    for feature in raw {
                        let feature = ExactText::copy(
                            bank,
                            feature.as_str().expect("checked feature string"),
                        )?;
                        bytes = bytes
                            .checked_add(feature.text.capacity())
                            .ok_or_else(ExistingRsdProbeError::metadata_limit)?;
                        features.push(feature);
                    }
                    (Some(fee), Some(features))
                }
                Some(_) => unreachable!("checked Features array"),
            };
            rows.push(Row {
                name,
                port: crate::xpc::rsd::checked_service_port(
                    descriptor.get("Port").expect("complete Port"),
                )?,
                features,
                _features_fee: features_fee,
            });
        }
        rows.sort_unstable_by(|a, b| a.name.text.cmp(&b.name.text));
        Ok(Self {
            udid,
            rows,
            selection: None,
            bytes,
            _rows_fee: rows_fee,
            _selection_fee: None,
            _body_fee: body_fee,
        })
    }
    pub(crate) fn decode_borrowed(
        dictionary: crate::xpc::message::borrowed::Dictionary<'_, '_>,
        bank: &Arc<Semaphore>,
    ) -> Result<Self, XpcError> {
        let (uid, services) = crate::xpc::rsd::checked_borrowed_parts(dictionary)?;
        let uid =
            uid.ok_or_else(|| XpcError::Tls("RSD: current catalogue missing identity".into()))?;
        let services = services
            .ok_or_else(|| XpcError::Tls("RSD: current catalogue missing Services".into()))?;
        for (_, value) in services.iter() {
            let descriptor = value.as_dict().expect("checked service dictionary");
            crate::xpc::rsd::checked_borrowed_port(
                descriptor
                    .get("Port")
                    .ok_or_else(|| XpcError::Tls("RSD: current catalogue missing Port".into()))?,
            )?;
        }
        let body_fee = charge(bank, std::mem::size_of::<Self>())?;
        let (rows_fee, mut rows) = exact_vec::<Row>(bank, services.len())?;
        let udid = ExactText::copy(bank, uid)?;
        let mut bytes = std::mem::size_of::<Self>()
            .checked_add(udid.text.capacity())
            .and_then(|bytes| bytes.checked_add(rows.capacity() * std::mem::size_of::<Row>()))
            .ok_or_else(ExistingRsdProbeError::metadata_limit)?;
        for (name, value) in services.iter() {
            let descriptor = value.as_dict().expect("checked service dictionary");
            let name = ExactText::copy(bank, name)?;
            bytes = bytes
                .checked_add(name.text.capacity())
                .ok_or_else(ExistingRsdProbeError::metadata_limit)?;
            let raw_features = descriptor
                .get("Properties")
                .and_then(|value| value.as_dict())
                .and_then(|properties| properties.get("Features"));
            let (features_fee, features) = match raw_features {
                None => (None, None),
                Some(value) => {
                    let raw = value.as_array().expect("checked Features array");
                    let (fee, mut features) = exact_vec::<ExactText>(bank, raw.len())?;
                    bytes = bytes
                        .checked_add(features.capacity() * std::mem::size_of::<ExactText>())
                        .ok_or_else(ExistingRsdProbeError::metadata_limit)?;
                    for feature in raw.iter() {
                        let feature = ExactText::copy(
                            bank,
                            feature.as_str().expect("checked feature string"),
                        )?;
                        bytes = bytes
                            .checked_add(feature.text.capacity())
                            .ok_or_else(ExistingRsdProbeError::metadata_limit)?;
                        features.push(feature);
                    }
                    (Some(fee), Some(features))
                }
            };
            rows.push(Row {
                name,
                port: crate::xpc::rsd::checked_borrowed_port(
                    descriptor.get("Port").expect("complete Port"),
                )?,
                features,
                _features_fee: features_fee,
            });
        }
        rows.sort_unstable_by(|a, b| a.name.text.cmp(&b.name.text));
        Ok(Self {
            udid,
            rows,
            selection: None,
            bytes,
            _rows_fee: rows_fee,
            _selection_fee: None,
            _body_fee: body_fee,
        })
    }

    pub(crate) fn finish(
        mut self,
        selection: RsdSelectionReceipt,
        fee: OwnedSemaphorePermit,
    ) -> Result<Arc<Self>, CoreError> {
        self.bytes = self
            .bytes
            .checked_add(
                selection.attempted_modes.capacity()
                    * std::mem::size_of::<super::existing_rsd::RsdHandshakeMode>(),
            )
            .ok_or_else(ExistingRsdProbeError::metadata_limit)?;
        self.selection = Some(selection);
        self._selection_fee = Some(fee);
        Ok(Arc::new(self))
    }
}

#[cfg(test)]
#[path = "prepared_rsd/tests.rs"]
mod tests;
