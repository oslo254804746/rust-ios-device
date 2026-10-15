use super::*;

#[test]
fn exact_payload_grant_follows_the_last_clone_and_slice() {
    let bank = Arc::new(Semaphore::new(32));
    let mut payload = MutableBytes::zeroed(&bank, 32).unwrap();
    assert_eq!(payload.bytes.capacity(), 32);
    payload.bytes.copy_from_slice(&[7; 32]);
    assert_eq!(bank.available_permits(), 0);
    let bytes = payload.freeze();
    let slice = bytes.slice(4..8);
    let last = slice.clone();
    drop(bytes);
    drop(slice);
    assert_eq!(
        bank.available_permits(),
        0,
        "a short view still owns the whole allocation"
    );
    assert_eq!(&last[..], &[7; 4]);
    drop(last);
    assert_eq!(bank.available_permits(), 32);
}

#[test]
fn failed_exact_replacement_keeps_old_bytes_and_old_grant() {
    let bank = Arc::new(Semaphore::new(11));
    let mut buffer = RawBuffer::new(Some(bank.clone()));
    buffer.extend_from_slice(b"old!").unwrap();
    assert_eq!(bank.available_permits(), 7);
    let _error = buffer.extend_from_slice(b"new!").unwrap_err();
    assert_eq!(&buffer[..], b"old!");
    assert_eq!(bank.available_permits(), 7);
    #[cfg(feature = "tunnel")]
    assert_eq!(
        crate::classify_error(&_error),
        crate::ErrorClass::ResourceLimit
    );
    drop(buffer);
    assert_eq!(bank.available_permits(), 11);
}

#[test]
fn simultaneous_old_new_and_external_slice_are_all_charged() {
    let bank = Arc::new(Semaphore::new(20));
    let mut buffer = RawBuffer::new(Some(bank.clone()));
    buffer.extend_from_slice(b"abcdefgh").unwrap();
    let old_slice = buffer.split_to(4);
    buffer.extend_from_slice(b"ijkl").unwrap();
    assert_eq!(
        bank.available_permits(),
        4,
        "old allocation 8 plus replacement allocation 8"
    );
    buffer.clear();
    assert_eq!(bank.available_permits(), 12);
    drop(old_slice);
    assert_eq!(bank.available_permits(), 20);
}

#[test]
fn denial_before_zeroed_payload_creation_leaks_no_permits() {
    let bank = Arc::new(Semaphore::new(31));
    assert!(MutableBytes::zeroed(&bank, 32).is_err());
    assert_eq!(bank.available_permits(), 31);
}

#[test]
fn exact_vector_early_failure_and_unwind_drop_original_elements_before_grant() {
    struct Element(Arc<Semaphore>, Arc<std::sync::atomic::AtomicUsize>);
    impl Drop for Element {
        fn drop(&mut self) {
            assert_eq!(
                self.0.available_permits(),
                0,
                "grant returned before original element"
            );
            self.1.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        }
    }
    let bytes = std::mem::size_of::<Element>();
    for unwind in [false, true] {
        let bank = Arc::new(Semaphore::new(bytes));
        let drops = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let result =
            std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| -> io::Result<()> {
                let (_fee, mut values) = exact_vec(&bank, 1)?;
                values.push(Element(Arc::clone(&bank), Arc::clone(&drops)));
                if unwind {
                    panic!("owned vector unwind fixture");
                }
                let _ = exact_vec::<u8>(&bank, 1)?;
                Ok(())
            }));
        if unwind {
            assert!(result.is_err());
        } else {
            assert!(result.unwrap().is_err());
        }
        assert_eq!(drops.load(std::sync::atomic::Ordering::SeqCst), 1);
        assert_eq!(bank.available_permits(), bytes);
    }
}
