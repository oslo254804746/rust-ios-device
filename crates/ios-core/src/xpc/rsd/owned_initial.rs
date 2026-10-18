//! Immutable initial-directory storage with legacy parsing semantics.
use std::alloc::Layout;
use std::sync::Arc;

use tokio::sync::{OwnedSemaphorePermit, Semaphore};

use super::{RsdFeatureEvidence, XpcError};
use crate::device::existing_rsd::ExistingRsdProbeError;
use crate::device::prepared_rsd::{charge, exact_vec_from_grant};
use crate::xpc::message::borrowed::{Arena, Array, Value};

struct Text {
    value: String,
    _fee: Option<OwnedSemaphorePermit>,
}
impl Text {
    fn empty() -> Self {
        Self {
            value: String::new(),
            _fee: None,
        }
    }
    fn copy(
        bank: &Arc<Semaphore>,
        text: &str,
        grant: &mut OwnedSemaphorePermit,
    ) -> Result<Self, XpcError> {
        let (fee, mut bytes) = vector::<u8>(bank, text.len(), grant)?;
        bytes.extend_from_slice(text.as_bytes());
        Ok(Self {
            value: String::from_utf8(bytes).expect("copied UTF-8 remains valid"),
            _fee: fee,
        })
    }
}
struct Row {
    name: Text,
    port: u16,
    position: usize,
    features: Vec<Text>,
    _features_fee: Option<OwnedSemaphorePermit>,
}
struct Directory {
    udid: Text,
    rows: Vec<Row>,
    bytes: usize,
    _rows_fee: Option<OwnedSemaphorePermit>,
    _body: OwnedSemaphorePermit,
}

/// An immutable initial RSD directory. Clones share the original buffers and
/// grants; there is no raw Arc, Weak, mutable buffer or unmetered export API.
pub struct OwnedRsdHandshake {
    // Always Some while live; None is only the consuming Drop move state.
    inner: Option<Arc<Directory>>,
}
impl Clone for OwnedRsdHandshake {
    fn clone(&self) -> Self {
        Self {
            inner: self.inner.clone(),
        }
    }
}
impl Drop for OwnedRsdHandshake {
    fn drop(&mut self) {
        // Every clone consumes this no-Weak Arc path. The allocation ends
        // before the returned directory's original buffers and permits drop.
        drop(Arc::into_inner(
            self.inner.take().expect("live owned RSD directory"),
        ));
    }
}
impl std::fmt::Debug for OwnedRsdHandshake {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("OwnedRsdHandshake")
            .field("services", &self.len())
            .field("known_bytes", &self.known_bytes())
            .finish_non_exhaustive()
    }
}

fn vector<T>(
    bank: &Arc<Semaphore>,
    capacity: usize,
    grant: &mut OwnedSemaphorePermit,
) -> Result<(Option<OwnedSemaphorePermit>, Vec<T>), XpcError> {
    if capacity == 0 {
        return Ok((None, Vec::new()));
    }
    let size = Layout::array::<T>(capacity)
        .map_err(|_| ExistingRsdProbeError::metadata_limit())?
        .size();
    let fee = grant
        .split(size)
        .expect("validated complete initial directory grant");
    #[cfg(test)]
    BEFORE_ALLOC.with(|hook| {
        if let Some(hook) = hook.borrow_mut().as_mut() {
            hook(Layout::array::<T>(capacity).expect("validated output layout"));
        }
    });
    let (fee, values) = exact_vec_from_grant(bank, capacity, fee)?;
    Ok((Some(fee), values))
}
fn add(total: &mut usize, amount: usize) -> Result<(), XpcError> {
    *total = total
        .checked_add(amount)
        .ok_or_else(ExistingRsdProbeError::metadata_limit)?;
    Ok(())
}
fn layout<T>(count: usize) -> Result<usize, XpcError> {
    Ok(Layout::array::<T>(count)
        .map_err(|_| ExistingRsdProbeError::metadata_limit())?
        .size())
}
fn descriptor<'t, 'a>(value: Value<'t, 'a>) -> Option<(u16, Option<Array<'t, 'a>>)> {
    let descriptor = value.as_dict()?;
    let port = descriptor
        .get("Port")
        .and_then(|value| match value.as_str() {
            Some(text) => text.parse::<u16>().ok(),
            None => value.as_uint64().and_then(|port| u16::try_from(port).ok()),
        })?;
    let features = descriptor
        .get("Properties")
        .and_then(|value| value.as_dict())
        .and_then(|properties| properties.get("Features"))
        .and_then(|value| value.as_array());
    Some((port, features))
}

impl OwnedRsdHandshake {
    pub(super) fn prepare(bank: &Arc<Semaphore>) -> Result<Self, XpcError> {
        let body = charge(bank, std::mem::size_of::<Directory>())?;
        Ok(Self {
            inner: Some(Arc::new(Directory {
                udid: Text::empty(),
                rows: Vec::new(),
                bytes: body.num_permits(),
                _rows_fee: None,
                _body: body,
            })),
        })
    }

    pub(super) fn fill(&mut self, tree: Option<&Arena<'_>>) -> Result<(), XpcError> {
        let dictionary = tree
            .and_then(|tree| tree.root().as_dict())
            .ok_or_else(|| XpcError::Tls("RSD: expected XPC dict body".into()))?;
        let message_type = dictionary
            .get("MessageType")
            .and_then(|value| value.as_str())
            .ok_or_else(|| XpcError::Tls("RSD: missing Handshake MessageType".into()))?;
        if message_type != "Handshake" {
            return Err(XpcError::Tls(
                "RSD: unexpected MessageType; expected Handshake".into(),
            ));
        }
        let udid = dictionary
            .get("Properties")
            .and_then(|value| value.as_dict())
            .and_then(|properties| properties.get("UniqueDeviceID"))
            .and_then(|value| value.as_str())
            .ok_or_else(|| XpcError::Tls("RSD: missing UniqueDeviceID".into()))?;
        let entries = dictionary.get("Services").and_then(|value| value.as_dict());
        let mut count = 0usize;
        let mut bytes = udid.len();
        if let Some(entries) = entries {
            tracing::debug!("RSD handshake exposed {} service entries", entries.len());
            for (name, value) in entries.iter() {
                if let Some((_, features)) = descriptor(value) {
                    count += 1;
                    add(&mut bytes, name.len())?;
                    if let Some(features) = features {
                        let mut strings = 0;
                        for value in features.iter() {
                            if let Some(text) = value.as_str() {
                                strings += 1;
                                add(&mut bytes, text.len())?;
                            }
                        }
                        add(&mut bytes, layout::<Text>(strings)?)?;
                    }
                } else if value.as_dict().is_some() {
                    tracing::debug!("RSD service has an invalid or missing Port");
                }
            }
        } else if dictionary.get("Services").is_some() {
            tracing::debug!("RSD handshake Services has unexpected XPC type");
        } else {
            tracing::debug!("RSD handshake missing Services key");
        }
        add(&mut bytes, layout::<Row>(count)?)?;
        let data = Arc::get_mut(self.inner.as_mut().expect("live owned RSD directory"))
            .expect("initial directory is private until parsing completes");
        let bank = data._body.semaphore().clone();
        // The entire actual output layout precedes the first output copy.
        let mut grant = if bytes == 0 {
            data._body
                .split(0)
                .expect("admitted empty directory needs no new allocation")
        } else {
            charge(&bank, bytes)?
        };
        let (rows_fee, mut rows) = vector::<Row>(&bank, count, &mut grant)?;
        let uid = Text::copy(&bank, udid, &mut grant)?;
        if let Some(entries) = entries {
            for (position, (name, value)) in entries.iter().enumerate() {
                let Some((port, features)) = descriptor(value) else {
                    continue;
                };
                let name = Text::copy(&bank, name, &mut grant)?;
                let strings = features.map_or(0, |features| {
                    features.iter().filter_map(|value| value.as_str()).count()
                });
                let (features_fee, mut values) = vector::<Text>(&bank, strings, &mut grant)?;
                if let Some(features) = features {
                    for value in features.iter() {
                        if let Some(text) = value.as_str() {
                            values.push(Text::copy(&bank, text, &mut grant)?);
                        }
                    }
                }
                rows.push(Row {
                    name,
                    port,
                    position,
                    features: values,
                    _features_fee: features_fee,
                });
            }
        }
        assert_eq!(grant.num_permits(), 0);
        // HashMap insertion's last valid duplicate wins. Sort without scratch
        // allocation, retaining the later valid entry before deduplication.
        rows.sort_unstable_by(|a, b| {
            a.name
                .value
                .cmp(&b.name.value)
                .then_with(|| b.position.cmp(&a.position))
        });
        rows.dedup_by(|later, first| later.name.value == first.name.value);
        let mut actual = data._body.num_permits();
        add(&mut actual, uid.value.capacity())?;
        add(&mut actual, layout::<Row>(rows.capacity())?)?;
        for row in &rows {
            add(&mut actual, row.name.value.capacity())?;
            add(&mut actual, layout::<Text>(row.features.capacity())?)?;
            for text in &row.features {
                add(&mut actual, text.value.capacity())?;
            }
        }
        data.udid = uid;
        data.rows = rows;
        data._rows_fee = rows_fee;
        data.bytes = actual;
        Ok(())
    }

    fn data(&self) -> &Directory {
        self.inner.as_deref().expect("live owned RSD directory")
    }
    pub fn udid(&self) -> &str {
        &self.data().udid.value
    }
    pub fn len(&self) -> usize {
        self.data().rows.len()
    }
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
    pub fn known_bytes(&self) -> usize {
        self.data().bytes
    }
    pub fn belongs_to(&self, bank: &Arc<Semaphore>) -> bool {
        Arc::ptr_eq(self.data()._body.semaphore(), bank)
    }
    fn find(&self, name: &str) -> Option<usize> {
        self.data()
            .rows
            .binary_search_by(|row| row.name.value.as_str().cmp(name))
            .ok()
    }
    fn resolve(&self, name: &str) -> Option<usize> {
        self.find(name).or_else(|| {
            self.data()
                .rows
                .binary_search_by(|row| {
                    row.name
                        .value
                        .bytes()
                        .cmp(name.bytes().chain(".shim.remote".bytes()))
                })
                .ok()
        })
    }
    pub fn get_port(&self, name: &str) -> Option<u16> {
        self.resolve(name).map(|index| self.data().rows[index].port)
    }
    pub fn get_service(&self, name: &str) -> Option<OwnedRsdService> {
        self.find(name).map(|index| OwnedRsdService {
            directory: self.clone(),
            index,
        })
    }
    pub fn get_resolved_service(&self, name: &str) -> Option<OwnedRsdService> {
        self.resolve(name).map(|index| OwnedRsdService {
            directory: self.clone(),
            index,
        })
    }
    pub fn feature_evidence(&self, name: &str, feature: &str) -> RsdFeatureEvidence {
        self.find(name)
            .map_or(RsdFeatureEvidence::ServiceMissing, |index| {
                evidence(&self.data().rows[index], feature)
            })
    }
    pub fn resolved_feature_evidence(&self, name: &str, feature: &str) -> RsdFeatureEvidence {
        self.resolve(name)
            .map_or(RsdFeatureEvidence::ServiceMissing, |index| {
                evidence(&self.data().rows[index], feature)
            })
    }
    pub fn supports_feature(&self, name: &str, feature: &str) -> Option<bool> {
        self.find(name).map(|index| {
            let row = &self.data().rows[index];
            row.features.is_empty() || row.features.iter().any(|text| text.value == feature)
        })
    }
    pub fn services(&self) -> impl ExactSizeIterator<Item = OwnedRsdService> + '_ {
        (0..self.len()).map(|index| OwnedRsdService {
            directory: self.clone(),
            index,
        })
    }
    pub fn into_services(self) -> OwnedRsdServices {
        OwnedRsdServices {
            directory: self,
            next: 0,
        }
    }
}

fn evidence(row: &Row, feature: &str) -> RsdFeatureEvidence {
    if row.features.is_empty() {
        RsdFeatureEvidence::MetadataUnavailable
    } else if row.features.iter().any(|text| text.value == feature) {
        RsdFeatureEvidence::Advertised
    } else {
        RsdFeatureEvidence::NotAdvertised
    }
}

/// An owned view of an original service; retaining it retains the same directory.
#[derive(Clone)]
pub struct OwnedRsdService {
    directory: OwnedRsdHandshake,
    index: usize,
}
impl OwnedRsdService {
    fn row(&self) -> &Row {
        &self.directory.data().rows[self.index]
    }
    pub fn name(&self) -> &str {
        &self.row().name.value
    }
    pub fn port(&self) -> u16 {
        self.row().port
    }
    pub fn features(&self) -> impl ExactSizeIterator<Item = &str> {
        self.row().features.iter().map(|text| text.value.as_str())
    }
    pub fn supports_feature(&self, feature: &str) -> bool {
        self.row().features.is_empty()
            || self.row().features.iter().any(|text| text.value == feature)
    }
    pub fn feature_evidence(&self, feature: &str) -> RsdFeatureEvidence {
        evidence(self.row(), feature)
    }
}
pub struct OwnedRsdServices {
    directory: OwnedRsdHandshake,
    next: usize,
}
impl Iterator for OwnedRsdServices {
    type Item = OwnedRsdService;
    fn next(&mut self) -> Option<Self::Item> {
        if self.next == self.directory.len() {
            return None;
        }
        let service = OwnedRsdService {
            directory: self.directory.clone(),
            index: self.next,
        };
        self.next += 1;
        Some(service)
    }
    fn size_hint(&self) -> (usize, Option<usize>) {
        let left = self.directory.len() - self.next;
        (left, Some(left))
    }
}
impl ExactSizeIterator for OwnedRsdServices {}
impl std::iter::FusedIterator for OwnedRsdServices {}

#[cfg(test)]
type OutputAllocationHook = std::cell::RefCell<Option<Box<dyn FnMut(Layout)>>>;
#[cfg(test)]
thread_local! {
    static BEFORE_ALLOC: OutputAllocationHook = const { std::cell::RefCell::new(None) };
}

#[cfg(test)]
fn with_output_allocator<T>(hook: impl FnMut(Layout) + 'static, work: impl FnOnce() -> T) -> T {
    struct Guard;
    impl Drop for Guard {
        fn drop(&mut self) {
            BEFORE_ALLOC.with(|hook| hook.borrow_mut().take());
        }
    }
    BEFORE_ALLOC.with(|current| {
        assert!(current.borrow().is_none());
        *current.borrow_mut() = Some(Box::new(hook));
    });
    let _guard = Guard;
    work()
}

#[cfg(test)]
mod tests;
