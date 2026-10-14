// This fragment is appended inside retirement.rs's module (not a new lease).
pub(super) struct RefreshProof {
    registry: Arc<Registry>,
    key: Key,
    stamp: OwnershipStamp,
    handle: Arc<TunnelHandle>,
    metadata: Arc<OwnedSemaphorePermit>,
    global_metadata: Arc<OwnedSemaphorePermit>,
    previous: Option<Arc<super::freshness::Metadata>>,
    _storage: OwnedSemaphorePermit,
}
type RefreshSwap<T> = (
    T,
    Option<Arc<super::freshness::Metadata>>,
    [Option<Arc<super::freshness::Receipt>>; 2],
);
pub(super) fn same_optional<T>(a: Option<&Arc<T>>, b: Option<&Arc<T>>) -> bool {
    match (a, b) {
        (None, None) => true,
        (Some(a), Some(b)) => Arc::ptr_eq(a, b),
        _ => false,
    }
}
impl NativeLease {
    pub(super) fn capture_refresh(
        &self,
        stamp: OwnershipStamp,
        handle: &Arc<TunnelHandle>,
        metadata: &Arc<OwnedSemaphorePermit>,
        global_metadata: &Arc<OwnedSemaphorePermit>,
        previous: Option<&Arc<super::freshness::Metadata>>,
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
                    .is_some_and(|original| Arc::ptr_eq(original, metadata))
                && entry
                    .global_metadata
                    .as_ref()
                    .is_some_and(|original| Arc::ptr_eq(original, global_metadata))
                && same_optional(entry.fresh_metadata.as_ref(), previous)
                && handle.is_alive(),
            super::lifecycle::cancelled()
        );
        let storage =
            super::admission::known_metadata_charge(bank, std::mem::size_of::<RefreshProof>())?;
        Ok(RefreshProof {
            registry: Arc::clone(&self.registry),
            key: self.key.clone(),
            stamp,
            handle: Arc::clone(handle),
            metadata: Arc::clone(metadata),
            global_metadata: Arc::clone(global_metadata),
            previous: previous.cloned(),
            _storage: storage,
        })
    }
}
impl RefreshProof {
    pub(super) fn previous(&self) -> Option<&Arc<super::freshness::Metadata>> {
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
                    .is_some_and(|original| Arc::ptr_eq(original, &self.metadata))
                && entry
                    .global_metadata
                    .as_ref()
                    .is_some_and(|original| Arc::ptr_eq(original, &self.global_metadata))
                && same_optional(entry.fresh_metadata.as_ref(), self.previous.as_ref())
                && self.handle.is_alive(),
            super::lifecycle::cancelled()
        );
        Ok(())
    }
    pub(super) fn runtime(&self) -> &Arc<TunnelHandle> {
        &self.handle
    }
    pub(super) fn retain_failure(&self, original: Arc<super::freshness::Receipt>) {
        let mut state = self.registry.lock();
        let displaced = match Registry::entry_mut(&mut state, &self.key) {
            Some(entry)
                if entry
                    .handle
                    .as_ref()
                    .is_some_and(|runtime| Arc::ptr_eq(runtime, &self.handle))
                    && Arc::ptr_eq(&entry.stamp.global, &self.stamp.global)
                    && Arc::ptr_eq(&entry.stamp.device, &self.stamp.device)
                    && same_optional(entry.fresh_metadata.as_ref(), self.previous.as_ref()) =>
            {
                if entry.first_freshness.is_none() {
                    entry.first_freshness = Some(Arc::clone(&original));
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
        proposed: Arc<super::freshness::Metadata>,
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
                    .is_some_and(|original| Arc::ptr_eq(original, &self.metadata))
                && entry
                    .global_metadata
                    .as_ref()
                    .is_some_and(|original| Arc::ptr_eq(original, &self.global_metadata))
                && same_optional(entry.fresh_metadata.as_ref(), self.previous.as_ref())
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
    ) -> Result<super::metadata_vec::KnownVec<Arc<super::freshness::Receipt>>> {
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
            roots.push_reserved(Arc::clone(original));
        }
        for entry in state.slots.iter().flatten() {
            for original in entry
                .first_freshness
                .iter()
                .chain(entry.latest_freshness.iter())
            {
                roots.push_reserved(Arc::clone(original));
            }
        }
        Ok(roots)
    }
}
