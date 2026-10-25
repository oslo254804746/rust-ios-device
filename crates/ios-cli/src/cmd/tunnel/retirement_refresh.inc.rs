// This fragment is appended inside retirement.rs's module (not a new lease).
type RefreshAllocation =
    super::shared_weak_allocation::Strong<RefreshProofData, super::native_origins::Grants>;

#[derive(Clone)]
pub(super) struct RefreshProof(RefreshAllocation);

/// Non-retiring inline observation of one new unpublished entry. Its retained
/// handle is accompanied by the SAME actual fee owners, not a guessed Arc fee.
/// The caller's already-admitted connection frame owns these inline fields.
pub(super) struct PublicationWitness {
    registry: RegistryOwner,
    key: Key,
    stamp: OwnershipStamp,
    handle: Arc<TunnelHandle>,
    metadata: super::metadata_lease::Owner,
    global_metadata: super::metadata_lease::Owner,
    fresh_metadata: Option<super::freshness::MetadataOwner>,
}

pub(super) struct RefreshProofData {
    registry: RegistryOwner,
    key: Key,
    stamp: OwnershipStamp,
    handle: Arc<TunnelHandle>,
    metadata: super::metadata_lease::Owner,
    global_metadata: super::metadata_lease::Owner,
    previous: Option<super::freshness::MetadataOwner>,
}

impl std::ops::Deref for RefreshProof {
    type Target = RefreshProofData;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
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
        self.capture_exact(stamp, handle, metadata, global_metadata, previous, bank, true)
    }

    /// Observe the same existing publication for conditional replacement. A
    /// naturally ended runtime remains eligible; this does not permit refresh
    /// IO or an in-place catalogue swap on that ended runtime.
    pub(super) fn capture_existing(
        &self,
        stamp: OwnershipStamp,
        handle: &Arc<TunnelHandle>,
        metadata: &super::metadata_lease::Owner,
        global_metadata: &super::metadata_lease::Owner,
        previous: Option<&super::freshness::MetadataOwner>,
        bank: &Arc<Semaphore>,
    ) -> Result<RefreshProof> {
        self.capture_exact(stamp, handle, metadata, global_metadata, previous, bank, false)
    }

    fn capture_exact(
        &self,
        stamp: OwnershipStamp,
        handle: &Arc<TunnelHandle>,
        metadata: &super::metadata_lease::Owner,
        global_metadata: &super::metadata_lease::Owner,
        previous: Option<&super::freshness::MetadataOwner>,
        bank: &Arc<Semaphore>,
        require_alive: bool,
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
                && entry.incarnation.ptr_eq(&self.key.incarnation)
                && ControlIdentity::ptr_eq(&entry.stamp.global, &stamp.global)
                && ControlIdentity::ptr_eq(&entry.stamp.device, &stamp.device)
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
                && (!require_alive || handle.is_alive()),
            super::lifecycle::cancelled()
        );
        let storage =
            super::admission::known_metadata_charge(bank, RefreshProof::body_bytes())?;
        let origin_storage = super::native_origins::proof_grants(
            bank,
            RefreshProof::body_bytes(),
            [&stamp.global, &stamp.device],
        )?;
        Ok(RefreshProof(RefreshAllocation::new(
            RefreshProofData {
                registry: self.registry.clone(),
                key: self.key.clone(),
                stamp,
                handle: Arc::clone(handle),
                metadata: metadata.clone(),
                global_metadata: global_metadata.clone(),
                previous: previous.cloned(),
            },
            bank,
            storage,
            origin_storage,
        )?))
    }

    /// Capture after retaining the output's G/P/fresh metadata and before the
    /// finalizer may expose native state. Readiness is checked at publication,
    /// so this observation can accompany a genuinely prepared inert runtime.
    pub(super) fn publication_witness(&self) -> Result<PublicationWitness> {
        let state = self.registry.lock();
        let entry = state.slots.get(self.key.slot).and_then(Option::as_ref)
            .ok_or_else(super::lifecycle::cancelled)?;
        anyhow::ensure!(
            !state.sealed && entry.phase == Phase::Unpublished
                && entry.incarnation.ptr_eq(&self.key.incarnation),
            super::lifecycle::cancelled()
        );
        let handle = entry.handle.as_ref().ok_or_else(super::lifecycle::cancelled)?;
        let metadata = entry.metadata.as_ref().ok_or_else(super::lifecycle::cancelled)?;
        let global_metadata = entry.global_metadata.as_ref().ok_or_else(super::lifecycle::cancelled)?;
        anyhow::ensure!(
            Arc::ptr_eq(global_metadata.semaphore(), &self.registry.byte_bank)
                && entry.fresh_metadata.as_ref().map_or(true, |fresh|
                    fresh.bank_matches(metadata.semaphore(), global_metadata.semaphore())),
            super::lifecycle::cancelled()
        );
        Ok(PublicationWitness {
            registry: self.registry.clone(), key: self.key.clone(), stamp: entry.stamp.clone(),
            handle: handle.clone(), metadata: metadata.clone(), global_metadata: global_metadata.clone(),
            fresh_metadata: entry.fresh_metadata.clone(),
        })
    }
}
impl RefreshProof {
    pub(super) fn body_bytes() -> usize {
        RefreshAllocation::body_bytes()
    }

    pub(super) fn previous(&self) -> Option<&super::freshness::MetadataOwner> {
        self.previous.as_ref()
    }
    pub(super) fn check(&self) -> Result<()> {
        let state = self.registry.lock();
        self.check_exact_locked(&state)?;
        anyhow::ensure!(self.handle.is_alive(), super::lifecycle::cancelled());
        Ok(())
    }

    pub(super) fn check_replacement(&self) -> Result<()> {
        let state = self.registry.lock();
        self.check_exact_locked(&state)
    }

    /// Pure owner comparison for a caller already holding the publication
    /// table. No watch/native/registry lock or quota operation occurs here.
    pub(super) fn matches_row(
        &self,
        handle: &Arc<TunnelHandle>,
        metadata: &super::metadata_lease::Owner,
        global_metadata: &super::metadata_lease::Owner,
        previous: Option<&super::freshness::MetadataOwner>,
    ) -> bool {
        Arc::ptr_eq(handle, &self.handle) && metadata.ptr_eq(&self.metadata)
            && global_metadata.ptr_eq(&self.global_metadata)
            && same_metadata(previous, self.previous.as_ref())
    }

    fn check_exact_locked(&self, state: &State) -> Result<()> {
        let entry = state
            .slots
            .get(self.key.slot)
            .and_then(Option::as_ref)
            .ok_or_else(super::lifecycle::cancelled)?;
        anyhow::ensure!(
            !state.sealed
                && entry.phase == Phase::Active
                && entry.incarnation.ptr_eq(&self.key.incarnation)
                && ControlIdentity::ptr_eq(&entry.stamp.global, &self.stamp.global)
                && ControlIdentity::ptr_eq(&entry.stamp.device, &self.stamp.device)
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
                && same_metadata(entry.fresh_metadata.as_ref(), self.previous.as_ref()),
            super::lifecycle::cancelled()
        );
        Ok(())
    }

    /// Called inside the EXISTING table/ticket/authority commit, with no await,
    /// quota or native operation in action. The inline witness does not borrow
    /// the new lease, so action may move that actual lease into the table row.
    /// Displaced rows/errors/fees must be returned, never dropped in action.
    pub(super) fn publish_replacement<T>(
        &self,
        new: &PublicationWitness,
        stamp: &OwnershipStamp,
        action: impl FnOnce() -> Result<T>,
    ) -> Result<T> {
        anyhow::ensure!(self.registry.ptr_eq(&new.registry), super::lifecycle::cancelled());
        let mut state = self.registry.lock();
        self.check_exact_locked(&state)?;
        let entry = Registry::entry_mut(&mut state, &new.key)
            .ok_or_else(super::lifecycle::cancelled)?;
        anyhow::ensure!(
            entry.phase == Phase::Unpublished
                && ControlIdentity::ptr_eq(&entry.stamp.global, &new.stamp.global)
                && ControlIdentity::ptr_eq(&entry.stamp.device, &new.stamp.device)
                && ControlIdentity::ptr_eq(&entry.stamp.global, &stamp.global)
                && ControlIdentity::ptr_eq(&entry.stamp.device, &stamp.device)
                && ControlIdentity::ptr_eq(&self.stamp.global, &stamp.global)
                && ControlIdentity::ptr_eq(&self.stamp.device, &stamp.device)
                && entry.handle.as_ref().is_some_and(|handle| Arc::ptr_eq(handle, &new.handle))
                && !Arc::ptr_eq(&self.handle, &new.handle)
                && entry.metadata.as_ref().is_some_and(|metadata| metadata.ptr_eq(&new.metadata))
                && entry.global_metadata.as_ref().is_some_and(|metadata| metadata.ptr_eq(&new.global_metadata))
                && same_metadata(entry.fresh_metadata.as_ref(), new.fresh_metadata.as_ref())
                && Arc::ptr_eq(new.metadata.semaphore(), self.metadata.semaphore())
                && Arc::ptr_eq(new.global_metadata.semaphore(), self.global_metadata.semaphore())
                && Arc::ptr_eq(new.global_metadata.semaphore(), &self.registry.byte_bank)
                && new.fresh_metadata.as_ref().map_or(true, |fresh|
                    fresh.bank_matches(new.metadata.semaphore(), new.global_metadata.semaphore()))
                && new.handle.is_alive() && new.handle.ready_for_publication(),
            super::lifecycle::cancelled()
        );
        entry.phase = Phase::Active;
        let mut rollback = ReplacementPhase { entry, committed: false };
        let result = action();
        rollback.committed = result.is_ok();
        drop(rollback);
        drop(state);
        result
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
                    && ControlIdentity::ptr_eq(&entry.stamp.global, &self.stamp.global)
                    && ControlIdentity::ptr_eq(&entry.stamp.device, &self.stamp.device)
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
                && ControlIdentity::ptr_eq(&entry.stamp.global, &self.stamp.global)
                && ControlIdentity::ptr_eq(&entry.stamp.device, &self.stamp.device)
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
/// Enum-only rollback also runs while unwinding; no owner is destroyed and no
/// watch/native operation can reenter the registry mutex from this guard.
struct ReplacementPhase<'a> { entry: &'a mut Entry, committed: bool }
impl Drop for ReplacementPhase<'_> {
    fn drop(&mut self) {
        if !self.committed { self.entry.phase = Phase::Unpublished; }
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

#[cfg(test)] #[path = "retirement/replace_proof_tests.rs"] mod replace_proof_tests;
