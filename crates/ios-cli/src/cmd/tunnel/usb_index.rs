//! Borrow original USB identities; admit actual sorted position backing.
use std::sync::Arc;

use anyhow::Result;
use tokio::sync::{OwnedSemaphorePermit, Semaphore};

use super::source::UsbDevice;

pub(super) struct Index<'a> {
    devices: &'a [UsbDevice],
    positions: Vec<usize>,
    _storage: OwnedSemaphorePermit,
}

impl<'a> Index<'a> {
    pub(super) fn new(devices: &'a [UsbDevice], bank: &Arc<Semaphore>) -> Result<Self> {
        let allocation = super::metadata_vec::exact_backing(bank, devices.len())?;
        let storage = allocation.storage;
        let mut positions = allocation.values;
        positions.extend(0..devices.len());
        positions.sort_unstable_by(|a, b| devices[*a].udid.cmp(&devices[*b].udid).then(a.cmp(b)));
        Ok(Self {
            devices,
            positions,
            _storage: storage,
        })
    }

    fn range(&self, udid: &str) -> std::ops::Range<usize> {
        let start = self
            .positions
            .partition_point(|index| self.devices[*index].udid.as_str() < udid);
        let end = self
            .positions
            .partition_point(|index| self.devices[*index].udid.as_str() <= udid);
        start..end
    }

    pub(super) fn contains(&self, udid: &str, device_id: u32) -> bool {
        // Each publication UID is unique. Across a retain pass these disjoint
        // UID ranges inspect at most all raw rows, not rows * publications.
        self.positions[self.range(udid)]
            .iter()
            .any(|index| self.devices[*index].device_id == device_id)
    }

    pub(super) fn get(&self, udid: &str) -> Option<u32> {
        let range = self.range(udid);
        if range.is_empty() {
            return None;
        }
        Some(self.devices[self.positions[range.end - 1]].device_id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn device(uid: &str, id: u32) -> UsbDevice {
        UsbDevice {
            udid: uid.into(),
            device_id: id,
            usb: true,
        }
    }

    #[test]
    fn attachment_index_preserves_raw_pairs_last_key_and_original_buffers() {
        let devices = [
            device("b", 4),
            device("a", 7),
            device("a", 3),
            device("b", 2),
        ];
        let pointers = devices
            .iter()
            .map(|device| device.udid.as_ptr())
            .collect::<Vec<_>>();
        let bank = Arc::new(Semaphore::new(4096));
        let index = Index::new(&devices, &bank).unwrap();
        let bytes = index.positions.capacity() * std::mem::size_of::<usize>();
        assert_eq!(bank.available_permits(), 4096 - bytes);
        assert!(index.contains("a", 7));
        assert!(index.contains("a", 3));
        assert!(!index.contains("a", 4));
        assert!(!index.contains("absent", 7));
        assert_eq!(index.get("a"), Some(3));
        assert_eq!(index.get("b"), Some(2));
        assert_eq!(index.get("absent"), None);
        for (device, pointer) in devices.iter().zip(pointers) {
            assert_eq!(device.udid.as_ptr(), pointer);
        }
        drop(index);
        assert_eq!(bank.available_permits(), 4096);
    }

    #[test]
    fn attachment_index_one_byte_refusal_keeps_source_and_refunds_all_index_storage() {
        let devices = [device("a", 7)];
        let bytes = std::mem::size_of::<usize>() - 1;
        let bank = Arc::new(Semaphore::new(bytes));
        assert!(Index::new(&devices, &bank).is_err());
        assert_eq!(bank.available_permits(), bytes);
        assert_eq!(devices[0].udid, "a");
        assert_eq!(devices[0].device_id, 7);
        let empty = Index::new(&[], &bank).unwrap();
        assert_eq!(empty.get("a"), None);
        assert_eq!(bank.available_permits(), bytes);
    }
}
