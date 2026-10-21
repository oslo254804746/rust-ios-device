//! The complete owned catalogue allocation, prepared before identity or IO.
use super::*;
use std::{
    marker::PhantomData,
    mem::{self, ManuallyDrop},
    sync::atomic::{fence, AtomicUsize, Ordering},
};

#[repr(C)]
struct Allocation {
    strong: AtomicUsize,
    catalogue: ManuallyDrop<PreparedRsdCatalogue>,
}

/// Immutable original catalogue and its full declared allocation. No Weak,
/// raw owning pointer, mutation or pinning interface escapes this owner.
pub struct PreparedRsdOwner {
    pointer: NonNull<Allocation>,
    _allocation: PhantomData<Allocation>,
}
// SAFETY: immutable catalogues share the same atomic strong count. Only its
// unique final owner moves the unpinned value after all prior releases.
unsafe impl Send for PreparedRsdOwner where PreparedRsdCatalogue: Send + Sync {}
unsafe impl Sync for PreparedRsdOwner where PreparedRsdCatalogue: Send + Sync {}

// Field order frees an unused actual backing before returning its whole fee.
pub(crate) struct OwnedDestination {
    values: Vec<Allocation>,
    #[cfg(test)]
    _after_free: AfterFree,
    storage: OwnedSemaphorePermit,
}
impl OwnedDestination {
    pub(crate) fn body_bytes() -> usize {
        mem::size_of::<Allocation>()
    }
    pub(crate) fn new(bank: &Arc<Semaphore>) -> Result<Self, XpcError> {
        Self::from_grant(bank, charge(bank, Self::body_bytes())?)
    }
    fn from_grant(bank: &Arc<Semaphore>, storage: OwnedSemaphorePermit) -> Result<Self, XpcError> {
        let (storage, values) = exact_vec_from_grant(bank, 1, storage)?;
        Ok(Self {
            values,
            #[cfg(test)]
            _after_free: AfterFree,
            storage,
        })
    }
}

// Parts own the original fields; the independent EMPTY body keeps its grant
// throughout every fallible field copy. An error drops fields, then body/fee.
pub(crate) struct PendingOwned {
    parts: CatalogueParts,
    destination: OwnedDestination,
}
impl PendingOwned {
    #[cfg(test)]
    pub(crate) fn decode(
        dictionary: &IndexMap<String, XpcValue>,
        bank: &Arc<Semaphore>,
    ) -> Result<Self, XpcError> {
        let destination = OwnedDestination::new(bank)?;
        let (_, parts) =
            CatalogueParts::decode(dictionary, bank, OwnedDestination::body_bytes(), || Ok(()))?;
        Ok(Self { parts, destination })
    }
    pub(crate) fn decode_borrowed(
        dictionary: crate::xpc::message::borrowed::Dictionary<'_, '_>,
        bank: &Arc<Semaphore>,
        destination: &mut Option<OwnedDestination>,
    ) -> Result<Self, XpcError> {
        let (_, parts) = CatalogueParts::decode_borrowed(
            dictionary,
            bank,
            OwnedDestination::body_bytes(),
            || Ok(()),
        )?;
        Ok(Self {
            parts,
            destination: destination.take().expect("one complete owned handshake"),
        })
    }
    pub(crate) fn finish(
        self,
        incoming_selection: RsdSelectionReceipt,
        incoming_fee: OwnedSemaphorePermit,
    ) -> Result<PreparedRsdOwner, CoreError> {
        // Bind the original fee before its Vec so any arithmetic refusal drops
        // the selection buffer before returning its separate byte credit.
        let fee = incoming_fee;
        let selection = incoming_selection;
        let pending = self;
        let bytes = pending
            .parts
            .bytes
            .checked_add(
                selection
                    .attempted_modes
                    .capacity()
                    .checked_mul(mem::size_of::<super::super::existing_rsd::RsdHandshakeMode>())
                    .ok_or_else(ExistingRsdProbeError::metadata_limit)?,
            )
            .ok_or_else(ExistingRsdProbeError::metadata_limit)?;
        let PendingOwned { parts, destination } = pending;
        // Keep the original whole fee in the actual destination until all
        // validation/arithmetic has succeeded. Placement below cannot fail.
        let OwnedDestination {
            values,
            storage,
            #[cfg(test)]
            _after_free,
        } = destination;
        let mut catalogue = parts.catalogue(storage);
        catalogue.bytes = bytes;
        catalogue.selection = Some(selection);
        catalogue._selection_fee = Some(fee);
        assert!(values.is_empty() && values.capacity() == 1);
        let mut values = values;
        values.push(Allocation {
            strong: AtomicUsize::new(1),
            catalogue: ManuallyDrop::new(catalogue),
        });
        let pointer = NonNull::from(&mut values[0]);
        mem::forget(values);
        #[cfg(test)]
        mem::forget(_after_free);
        Ok(PreparedRsdOwner {
            pointer,
            _allocation: PhantomData,
        })
    }
}

impl PreparedRsdOwner {
    pub fn ptr_eq(&self, other: &Self) -> bool {
        self.pointer == other.pointer
    }
    fn allocation(&self) -> &Allocation {
        // SAFETY: every live owner retains a positive count at this address.
        unsafe { self.pointer.as_ref() }
    }
}
impl Clone for PreparedRsdOwner {
    fn clone(&self) -> Self {
        if self.allocation().strong.fetch_add(1, Ordering::Relaxed) >= isize::MAX as usize {
            std::process::abort();
        }
        Self {
            pointer: self.pointer,
            _allocation: PhantomData,
        }
    }
}
impl std::ops::Deref for PreparedRsdOwner {
    type Target = PreparedRsdCatalogue;
    fn deref(&self) -> &Self::Target {
        &self.allocation().catalogue
    }
}
impl std::fmt::Debug for PreparedRsdOwner {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        std::fmt::Debug::fmt(&**self, f)
    }
}
impl Drop for PreparedRsdOwner {
    fn drop(&mut self) {
        if self.allocation().strong.fetch_sub(1, Ordering::Release) != 1 {
            return;
        }
        fence(Ordering::Acquire);
        fn require_unpin<T: Unpin>() {}
        require_unpin::<PreparedRsdCatalogue>();
        let pointer = self.pointer.as_ptr();
        // SAFETY: the final count acquired all previous releases, with no Weak
        // or pin/raw escape. Move the original Unpin catalogue once, then free
        // the SAME actual capacity-one declared block before any of its fees.
        let catalogue = unsafe { ManuallyDrop::take(&mut (*pointer).catalogue) };
        unsafe { drop(Vec::from_raw_parts(pointer, 0, 1)) };
        #[cfg(test)]
        drop(AfterFree);
        drop(catalogue);
    }
}

#[cfg(test)]
struct AfterFree;
#[cfg(test)]
impl Drop for AfterFree {
    fn drop(&mut self) {
        AFTER_FREE.with(|hook| {
            if let Some(callback) = hook.borrow_mut().as_mut() {
                callback();
            }
        });
    }
}
#[cfg(test)]
type FreeHook = std::cell::RefCell<Option<Box<dyn FnMut()>>>;
#[cfg(test)]
thread_local! { static AFTER_FREE: FreeHook = const { std::cell::RefCell::new(None) }; }
#[cfg(test)]
fn after_free<R>(hook: impl FnMut() + 'static, work: impl FnOnce() -> R) -> R {
    struct Reset;
    impl Drop for Reset {
        fn drop(&mut self) {
            AFTER_FREE.with(|hook| hook.borrow_mut().take());
        }
    }
    AFTER_FREE.with(|pending| {
        assert!(pending.borrow().is_none());
        *pending.borrow_mut() = Some(Box::new(hook));
    });
    let _reset = Reset;
    work()
}

#[cfg(test)]
mod tests {
    use super::super::with_allocation_hook;
    use super::*;
    use std::sync::{Barrier, Mutex};
    fn dictionary() -> IndexMap<String, XpcValue> {
        IndexMap::from([
            ("MessageType".into(), XpcValue::String("Handshake".into())),
            (
                "Properties".into(),
                XpcValue::Dictionary(IndexMap::from([(
                    "UniqueDeviceID".into(),
                    XpcValue::String("owned-root-local".into()),
                )])),
            ),
            (
                "Services".into(),
                XpcValue::Dictionary(IndexMap::from([(
                    "svc".into(),
                    XpcValue::Dictionary(IndexMap::from([(
                        "Port".into(),
                        XpcValue::Uint64(12345),
                    )])),
                )])),
            ),
        ])
    }
    #[test]
    fn owned_root_whole_original_grant_refuses_foreign_short_long_before_any_backing() {
        let bytes = OwnedDestination::body_bytes();
        assert_eq!(bytes, Layout::new::<Allocation>().size());
        assert!(bytes > mem::size_of::<PreparedRsdCatalogue>());
        for (foreign, amount) in [(true, bytes), (false, bytes - 1), (false, bytes + 1)] {
            let bank = Arc::new(Semaphore::new(2 * bytes));
            let other = Arc::new(Semaphore::new(2 * bytes));
            let storage = charge(if foreign { &other } else { &bank }, amount).unwrap();
            let error = with_allocation_hook(
                |_, _| panic!("invalid root grant allocated"),
                || OwnedDestination::from_grant(&bank, storage).err().unwrap(),
            );
            assert_eq!(
                crate::classify_error(&error),
                crate::ErrorClass::ResourceLimit
            );
            assert_eq!(bank.available_permits(), 2 * bytes);
            assert_eq!(other.available_permits(), 2 * bytes);
        }
    }
    #[test]
    fn owned_root_field_refusal_frees_original_empty_body_before_credit() {
        let bytes = OwnedDestination::body_bytes();
        for fields in [0, mem::size_of::<Row>()] {
            let bank = Arc::new(Semaphore::new(bytes + fields));
            let observed = Arc::new(Mutex::new(Vec::new()));
            let captured = observed.clone();
            let charged = bank.clone();
            let freed = Arc::new(AtomicUsize::new(0));
            let capture_free = freed.clone();
            let free_bank = bank.clone();
            let error = after_free(
                move || {
                    // A preceding partial Row backing may already have ended.
                    // Only that real field credit may precede this root's free.
                    assert_eq!(
                        free_bank.available_permits(),
                        fields,
                        "root credit returned before empty backing free"
                    );
                    capture_free.fetch_add(1, Ordering::SeqCst);
                },
                || {
                    with_allocation_hook(
                        move |layout, pointer| {
                            captured.lock().unwrap().push((
                                layout,
                                pointer,
                                charged.available_permits(),
                            ));
                        },
                        || PendingOwned::decode(&dictionary(), &bank).err().unwrap(),
                    )
                },
            );
            assert_eq!(
                crate::classify_error(&error),
                crate::ErrorClass::ResourceLimit
            );
            let observed = observed.lock().unwrap();
            assert_eq!(observed.len(), 1 + usize::from(fields != 0));
            assert_eq!(observed[0].0, Layout::new::<Allocation>());
            assert_eq!(observed[0].2, fields);
            if fields != 0 {
                assert_eq!(observed[1].0, Layout::array::<Row>(1).unwrap());
                assert_eq!(observed[1].2, 0);
            }
            assert_eq!(freed.load(Ordering::SeqCst), 1);
            assert_eq!(bank.available_permits(), bytes + fields);
        }
    }

    #[test]
    fn owned_root_closed_finish_keeps_original_preallocated_body_fields_and_full_mode_capacity() {
        let quota = 65536;
        let bank = Arc::new(Semaphore::new(quota));
        let observed = Arc::new(Mutex::new(Vec::new()));
        let captured = observed.clone();
        let charged = bank.clone();
        let pending = with_allocation_hook(
            move |layout, pointer| {
                captured
                    .lock()
                    .unwrap()
                    .push((layout, pointer, charged.available_permits()));
            },
            || PendingOwned::decode(&dictionary(), &bank).unwrap(),
        );
        let root_pointer = pending.destination.values.as_ptr() as usize;
        let uid_pointer = pending.parts.udid.text.as_ptr();
        let rows_pointer = pending.parts.rows.as_ptr();
        let first = observed.lock().unwrap()[0];
        assert_eq!(
            first,
            (
                Layout::new::<Allocation>(),
                root_pointer,
                quota - OwnedDestination::body_bytes()
            )
        );
        let (fee, mut modes) = exact_vec(&bank, 4).unwrap();
        modes.push(super::super::super::existing_rsd::RsdHandshakeMode::Direct);
        let modes_pointer = modes.as_ptr();
        let paid = pending.parts.bytes
            + 4 * mem::size_of::<super::super::super::existing_rsd::RsdHandshakeMode>();
        let held = bank
            .clone()
            .try_acquire_many_owned(bank.available_permits() as u32)
            .unwrap();
        bank.close();
        let root = with_allocation_hook(
            |_, _| panic!("owned finish allocated after Source"),
            || {
                pending
                    .finish(
                        RsdSelectionReceipt {
                            handshake_mode:
                                super::super::super::existing_rsd::RsdHandshakeMode::Direct,
                            attempted_modes: modes,
                        },
                        fee,
                    )
                    .unwrap()
            },
        );
        assert_eq!(root.pointer.as_ptr() as usize, root_pointer);
        assert_eq!(root.udid.text.as_ptr(), uid_pointer);
        assert_eq!(root.rows.as_ptr(), rows_pointer);
        assert_eq!(
            root.selection().unwrap().attempted_modes.as_ptr(),
            modes_pointer
        );
        assert_eq!(root.selection().unwrap().attempted_modes.capacity(), 4);
        assert_eq!(root.known_bytes(), paid);
        assert_eq!(root._body_fee.num_permits(), OwnedDestination::body_bytes());
        assert!(Arc::ptr_eq(root._body_fee.semaphore(), &bank));
        let last = root.clone();
        assert!(last.ptr_eq(&root));
        drop(root);
        assert_eq!(bank.available_permits(), 0);
        let charged = bank.clone();
        after_free(
            move || {
                assert_eq!(
                    charged.available_permits(),
                    0,
                    "root or fields refunded before actual full root free"
                )
            },
            || drop(last),
        );
        assert_eq!(bank.available_permits(), paid);
        drop(held);
        assert_eq!(bank.available_permits(), quota);
    }
    #[test]
    fn owned_root_parallel_last_clones_keep_original_value_and_refund_once() {
        let quota = 65536;
        let bank = Arc::new(Semaphore::new(quota));
        let pending = PendingOwned::decode(&dictionary(), &bank).unwrap();
        let (fee, mut modes) = exact_vec(&bank, 1).unwrap();
        modes.push(super::super::super::existing_rsd::RsdHandshakeMode::Direct);
        let root = pending
            .finish(
                RsdSelectionReceipt {
                    handshake_mode: super::super::super::existing_rsd::RsdHandshakeMode::Direct,
                    attempted_modes: modes,
                },
                fee,
            )
            .unwrap();
        let paid = root.known_bytes();
        let value = &*root as *const PreparedRsdCatalogue as usize;
        let uid = root.udid().as_ptr() as usize;
        let held = bank
            .clone()
            .try_acquire_many_owned(bank.available_permits() as u32)
            .unwrap();
        bank.close();
        let barrier = Arc::new(Barrier::new(8));
        std::thread::scope(|scope| {
            for _ in 0..7 {
                let owner = root.clone();
                let barrier = barrier.clone();
                scope.spawn(move || {
                    assert_eq!(&*owner as *const PreparedRsdCatalogue as usize, value);
                    assert_eq!(owner.udid().as_ptr() as usize, uid);
                    assert_eq!(owner.service_port("svc"), Some(12345));
                    barrier.wait();
                    drop(owner);
                });
            }
            barrier.wait();
            drop(root);
        });
        assert_eq!(bank.available_permits(), paid);
        drop(held);
        assert_eq!(bank.available_permits(), quota);
    }
}
