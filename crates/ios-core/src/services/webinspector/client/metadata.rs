//! LOCAL metadata admission, not a device/protocol limit. Compact JSON bytes
//! include map keys/containers and availability. A separate flattened snapshot
//! budget prevents repeating a large application for every cached page.
//! Borrowed projections/counting allocate no full encoded/snapshot copy.

use super::*;
use serde::{
    ser::{SerializeMap, SerializeSeq},
    Serialize, Serializer,
};

pub(super) const MAX_APPLICATIONS: usize = 1024;
pub(super) const MAX_PAGE_APPLICATIONS: usize = 1024;
pub(super) const MAX_PAGES: usize = 4096;
pub(super) const MAX_LISTING_ENTRIES: usize = 4096;
pub(super) const MAX_METADATA_BYTES: usize = 16 * 1024 * 1024;
pub(super) const MAX_SNAPSHOT_BYTES: usize = 16 * 1024 * 1024;

#[derive(Clone, Copy)]
pub(super) enum Change<'a> {
    Availability(&'a AutomationAvailability),
    Applications(&'a IndexMap<String, Application>),
    Application(&'a Application),
    Pages(&'a str, &'a IndexMap<u64, Page>),
}

pub(super) fn count(
    value: usize,
    limit: usize,
    resource: &'static str,
) -> Result<(), WebInspectorError> {
    if value > limit {
        return Err(super::super::local_limit(resource, limit as u64));
    }
    Ok(())
}

fn bytes(
    value: &(impl Serialize + ?Sized),
    resource: &'static str,
    limit: usize,
) -> Result<usize, WebInspectorError> {
    super::super::retained::encoded_size(value, limit, resource, limit)
}

struct Entry<'a, K: ?Sized, V: ?Sized>(&'a K, &'a V);
impl<K: Serialize + ?Sized, V: Serialize + ?Sized> Serialize for Entry<'_, K, V> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut map = serializer.serialize_map(Some(1))?;
        map.serialize_entry(self.0, self.1)?;
        map.end()
    }
}

// Both staged maps and the original duplicate-preserving listing Vec are
// admitted incrementally. No reserve/clone/append follows a failed check.
pub(super) fn staged_map_bytes<K: Serialize + ?Sized, V: Serialize + ?Sized>(
    current: usize,
    entries: usize,
    key: &K,
    value: &V,
    replaced: Option<&V>,
) -> Result<usize, WebInspectorError> {
    let incoming = bytes(
        &Entry(key, value),
        "staged metadata bytes",
        MAX_METADATA_BYTES,
    )? - 2;
    let previous = replaced
        .map(|value| {
            bytes(
                &Entry(key, value),
                "staged metadata bytes",
                MAX_METADATA_BYTES,
            )
            .map(|size| size - 2)
        })
        .transpose()?
        .unwrap_or(0);
    let comma = usize::from(replaced.is_none() && entries > 0);
    let total = current
        .checked_sub(previous)
        .and_then(|size| size.checked_add(incoming))
        .and_then(|size| size.checked_add(comma))
        .ok_or_else(|| {
            super::super::local_limit("staged metadata bytes", MAX_METADATA_BYTES as u64)
        })?;
    count(total, MAX_METADATA_BYTES, "staged metadata bytes")?;
    Ok(total)
}

pub(super) fn staged_list_bytes(
    current: usize,
    entries: usize,
    page: &Page,
) -> Result<usize, WebInspectorError> {
    let incoming = bytes(page, "staged listing bytes", MAX_METADATA_BYTES)?;
    let total = current
        .checked_add(incoming)
        .and_then(|size| size.checked_add(usize::from(entries > 0)))
        .ok_or_else(|| {
            super::super::local_limit("staged listing bytes", MAX_METADATA_BYTES as u64)
        })?;
    count(total, MAX_METADATA_BYTES, "staged listing bytes")?;
    Ok(total)
}

struct Projection<'a> {
    availability: &'a Option<AutomationAvailability>,
    applications: &'a IndexMap<String, Application>,
    pages: &'a IndexMap<String, IndexMap<u64, Page>>,
    change: Change<'a>,
}

impl Projection<'_> {
    fn applications(&self) -> impl Iterator<Item = (&str, &Application)> {
        let source = match self.change {
            Change::Applications(apps) => apps,
            _ => self.applications,
        };
        let replacement = match self.change {
            Change::Application(app) => Some(app),
            _ => None,
        };
        source
            .iter()
            .map(move |(key, value)| {
                (
                    key.as_str(),
                    replacement.filter(|app| app.id == *key).unwrap_or(value),
                )
            })
            .chain(
                replacement
                    .filter(|app| !source.contains_key(&app.id))
                    .into_iter()
                    .map(|app| (app.id.as_str(), app)),
            )
    }

    fn page_sets(&self) -> impl Iterator<Item = (&str, &IndexMap<u64, Page>)> {
        self.pages
            .iter()
            .filter(move |(key, _)| match self.change {
                Change::Applications(apps) => apps.contains_key(*key),
                Change::Pages(id, _) => key.as_str() != id,
                _ => true,
            })
            .map(|(key, pages)| (key.as_str(), pages))
            .chain(match self.change {
                Change::Pages(id, pages) => Some((id, pages)),
                _ => None,
            })
    }

    fn pages_for(&self, id: &str) -> Option<&IndexMap<u64, Page>> {
        match self.change {
            Change::Pages(key, pages) if key == id => Some(pages),
            _ => self.pages.get(id),
        }
    }
}

struct Applications<'a>(&'a Projection<'a>);
impl Serialize for Applications<'_> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut map = serializer.serialize_map(None)?;
        for (id, app) in self.0.applications() {
            map.serialize_entry(id, app)?;
        }
        map.end()
    }
}

struct Pages<'a>(&'a IndexMap<u64, Page>);
impl Serialize for Pages<'_> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut map = serializer.serialize_map(Some(self.0.len()))?;
        for (id, page) in self.0 {
            map.serialize_entry(id, page)?;
        }
        map.end()
    }
}

struct PageSets<'a>(&'a Projection<'a>);
impl Serialize for PageSets<'_> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut map = serializer.serialize_map(None)?;
        for (id, pages) in self.0.page_sets() {
            map.serialize_entry(id, &Pages(pages))?;
        }
        map.end()
    }
}

#[derive(Serialize)]
struct Metadata<'a> {
    automation_availability: Option<&'a AutomationAvailability>,
    applications: Applications<'a>,
    application_pages: PageSets<'a>,
}

struct Snapshot<'a>(&'a Projection<'a>);
impl Serialize for Snapshot<'_> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        #[derive(Serialize)]
        struct Entry<'a> {
            application: &'a Application,
            page: &'a Page,
        }
        let mut sequence = serializer.serialize_seq(None)?;
        for (_, application) in self.0.applications() {
            if let Some(pages) = self.0.pages_for(&application.id) {
                for page in pages.values() {
                    sequence.serialize_element(&Entry { application, page })?;
                }
            }
        }
        sequence.end()
    }
}

impl<S> WebInspectorClient<S> {
    pub(super) fn admit_metadata(&self, change: Change<'_>) -> Result<(), WebInspectorError> {
        let projection = Projection {
            availability: &self.automation_availability,
            applications: &self.applications,
            pages: &self.application_pages,
            change,
        };
        count(
            projection.applications().count(),
            MAX_APPLICATIONS,
            "cached application count",
        )?;
        let mut sets = 0usize;
        let mut pages = 0usize;
        for (_, values) in projection.page_sets() {
            sets += 1;
            count(sets, MAX_PAGE_APPLICATIONS, "page application count")?;
            pages = pages
                .checked_add(values.len())
                .ok_or_else(|| super::super::local_limit("cached page count", MAX_PAGES as u64))?;
            count(pages, MAX_PAGES, "cached page count")?;
        }
        let availability = match change {
            Change::Availability(value) => Some(value),
            _ => projection.availability.as_ref(),
        };
        bytes(
            &Metadata {
                automation_availability: availability,
                applications: Applications(&projection),
                application_pages: PageSets(&projection),
            },
            "cached metadata bytes",
            MAX_METADATA_BYTES,
        )?;
        bytes(
            &Snapshot(&projection),
            "flattened snapshot bytes",
            MAX_SNAPSHOT_BYTES,
        )?;
        Ok(())
    }
}
