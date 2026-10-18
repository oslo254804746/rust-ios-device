// This fragment is appended inside retirement.rs's module (not a new lease).
pub(super) struct RefreshProof {
    registry: RegistryOwner,
    key: Key,
    stamp: OwnershipStamp,
    handle: Arc<TunnelHandle>,
    metadata: super::metadata_lease::Owner,
    global_metadata: super::metadata_lease::Owner,
    previous: Option<super::freshness::MetadataOwner>,
    _storage: OwnedSemaphorePermit,
}
type RefreshSwap<T> = (
    T,
    Option<super::freshness::MetadataOwner>,
    [Option<super::freshness::ReceiptOwner>; 2],
);
pub(super) fn same_optional(
    a: Option<&ios_core::device::PreparedRsdOwner>,
    b: Option<&ios_core::device::PreparedRsdOwner>,
) -> bool {
    match (a, b) {
        (None, None) => true,
        (Some(a), Some(b)) => a.ptr_eq(b),
        _ => false,
    }
}
pub(super) fn same_metadata(
    a: Option<&super::freshness::MetadataOwner>,
    b: Option<&super::freshness::MetadataOwner>,
) -> bool {
    match (a, b) {
        (None, None) => true,
        (Some(a), Some(b)) => a.ptr_eq(b),
        _ => false,
    }
}
impl NativeLease {
    pub(super) fn capture_refresh(
        &self,
        stamp: OwnershipStamp,
        handle: &Arc<TunnelHandle>,
        metadata: &super::metadata_lease::Owner,
        global_metadata: &super::metadata_lease::Owner,
        previous: Option<&super::freshness::MetadataOwner>,
        bank: &Arc<Semaphore>,
    ) -> Result<RefreshProof> {
        let state = self.registry.lock();
        let entry = state
            .slots
            .get(self.key.slot)
            .and_then(Option::as_ref)
            .ok_or_else(super::lifecycle::cancelled)?;
        anyhow::ensure!(
            !state.sealed
                && entry.phase == Phase::Active
                && Arc::ptr_eq(&entry.incarnation, &self.key.incarnation)
                && Arc::ptr_eq(&entry.stamp.global, &stamp.global)
                && Arc::ptr_eq(&entry.stamp.device, &stamp.device)
                && entry
                    .handle
                    .as_ref()
                    .is_some_and(|original| Arc::ptr_eq(original, handle))
                && entry
                    .metadata
                    .as_ref()
                    .is_some_and(|original| original.ptr_eq(metadata))
                && entry
                    .global_metadata
                    .as_ref()
                    .is_some_and(|original| original.ptr_eq(global_metadata))
                && same_metadata(entry.fresh_metadata.as_ref(), previous)
                && handle.is_alive(),
            super::lifecycle::cancelled()
        );
        let storage =
            super::admission::known_metadata_charge(bank, std::mem::size_of::<RefreshProof>())?;
        Ok(RefreshProof {
            registry: self.registry.clone(),
            key: self.key.clone(),
            stamp,
            handle: Arc::clone(handle),
            metadata: metadata.clone(),
            global_metadata: global_metadata.clone(),
            previous: previous.cloned(),
            _storage: storage,
        })
    }
}
impl RefreshProof {
    pub(super) fn previous(&self) -> Option<&super::freshness::MetadataOwner> {
        self.previous.as_ref()
    }
    pub(super) fn check(&self) -> Result<()> {
        let state = self.registry.lock();
        let entry = state
            .slots
            .get(self.key.slot)
            .and_then(Option::as_ref)
            .ok_or_else(super::lifecycle::cancelled)?;
        anyhow::ensure!(
            !state.sealed
                && entry.phase == Phase::Active
                && Arc::ptr_eq(&entry.incarnation, &self.key.incarnation)
                && Arc::ptr_eq(&entry.stamp.global, &self.stamp.global)
                && Arc::ptr_eq(&entry.stamp.device, &self.stamp.device)
                && entry
                    .handle
                    .as_ref()
                    .is_some_and(|original| Arc::ptr_eq(original, &self.handle))
                && entry
                    .metadata
                    .as_ref()
                    .is_some_and(|original| original.ptr_eq(&self.metadata))
                && entry
                    .global_metadata
                    .as_ref()
                    .is_some_and(|original| original.ptr_eq(&self.global_metadata))
                && same_metadata(entry.fresh_metadata.as_ref(), self.previous.as_ref())
                && self.handle.is_alive(),
            super::lifecycle::cancelled()
        );
        Ok(())
    }
    pub(super) fn runtime(&self) -> &Arc<TunnelHandle> {
        &self.handle
    }
    pub(super) fn retain_failure(&self, original: super::freshness::ReceiptOwner) {
        let mut state = self.registry.lock();
        let displaced = match Registry::entry_mut(&mut state, &self.key) {
            Some(entry)
                if entry
                    .handle
                    .as_ref()
                    .is_some_and(|runtime| Arc::ptr_eq(runtime, &self.handle))
                    && Arc::ptr_eq(&entry.stamp.global, &self.stamp.global)
                    && Arc::ptr_eq(&entry.stamp.device, &self.stamp.device)
                    && same_metadata(entry.fresh_metadata.as_ref(), self.previous.as_ref()) =>
            {
                if entry.first_freshness.is_none() {
                    entry.first_freshness = Some(original.clone());
                }
                entry.latest_freshness.replace(original)
            }
            _ => {
                // A Source cause acquired after joins/slot release remains historical,
                // never a current row or retry/ready resurrection.
                if state.first_orphan_freshness.is_none() {
                    state.first_orphan_freshness = Some(original);
                }
                None
            }
        };
        drop(state);
        drop(displaced);
    }
    pub(super) fn swap_metadata<T>(
        &self,
        proposed: super::freshness::MetadataOwner,
        action: impl FnOnce() -> Result<T>,
    ) -> Result<RefreshSwap<T>> {
        let mut state = self.registry.lock();
        if state.sealed {
            return Err(super::lifecycle::cancelled());
        }
        let entry =
            Registry::entry_mut(&mut state, &self.key).ok_or_else(super::lifecycle::cancelled)?;
        anyhow::ensure!(
            entry.phase == Phase::Active
                && Arc::ptr_eq(&entry.stamp.global, &self.stamp.global)
                && Arc::ptr_eq(&entry.stamp.device, &self.stamp.device)
                && entry
                    .handle
                    .as_ref()
                    .is_some_and(|original| Arc::ptr_eq(original, &self.handle))
                && entry
                    .metadata
                    .as_ref()
                    .is_some_and(|original| original.ptr_eq(&self.metadata))
                && entry
                    .global_metadata
                    .as_ref()
                    .is_some_and(|original| original.ptr_eq(&self.global_metadata))
                && same_metadata(entry.fresh_metadata.as_ref(), self.previous.as_ref())
                && proposed
                    .bank_matches(self.metadata.semaphore(), self.global_metadata.semaphore())
                && self.handle.is_alive(),
            super::lifecycle::cancelled()
        );
        // Caller holds table and original Ticket/authority fences. No await,
        // watch reentry, new lease or native stop occurs under this mutex.
        // Caller action performs/rolls back cache and table fields before return.
        let value = action()?;
        let previous = entry.fresh_metadata.replace(proposed);
        let errors = [entry.first_freshness.take(), entry.latest_freshness.take()];
        drop(state);
        Ok((value, previous, errors))
    }
}
// RefreshProof has no Drop action: releasing observation cannot retire a runtime.
impl Registry {
    pub(super) fn payload_freshness(
        &self,
    ) -> Result<super::metadata_vec::KnownVec<super::freshness::ReceiptOwner>> {
        let state = self.lock();
        let count = usize::from(state.first_orphan_freshness.is_some())
            + state
                .slots
                .iter()
                .flatten()
                .map(|entry| {
                    usize::from(entry.first_freshness.is_some())
                        + usize::from(entry.latest_freshness.is_some())
                })
                .sum::<usize>();
        let mut roots = super::metadata_vec::KnownVec::new(
            Arc::clone(&self.byte_bank),
            state.slots.len() * 2 + 1,
        );
        // Admit the actual backing before cloning any original cause owner.
        // A refusal leaves registry roots untouched and attribution incomplete.
        roots.reserve(count)?;
        if let Some(original) = &state.first_orphan_freshness {
            roots.push_reserved(original.clone());
        }
        for entry in state.slots.iter().flatten() {
            for original in entry
                .first_freshness
                .iter()
                .chain(entry.latest_freshness.iter())
            {
                roots.push_reserved(original.clone());
            }
        }
        Ok(roots)
    }
}
