use super::*;
use crate::{test_native::has_source, ErrorClass};

#[test]
fn input_native_app_pid_preserves_error_in_both_request_builders() {
    let errors = [
        build_send_signal_input(u64::MAX, SIGKILL).unwrap_err(),
        build_monitor_process_termination_input(u64::MAX).unwrap_err(),
    ];
    for error in errors {
        assert!(has_source::<std::num::TryFromIntError>(&error));
        assert_eq!(crate::classify_error(&error), ErrorClass::InvalidInput);
        assert!(matches!(error, AppServiceError::Xpc(XpcError::Io(_))));
        assert!(!format!("{error:?}").contains(&u64::MAX.to_string()));
    }
    assert_eq!(
        process_identifier_to_i64(i64::MAX as u64).unwrap(),
        i64::MAX
    );
    assert_eq!(process_identifier_to_i64(0).unwrap(), 0);
}

#[test]
fn input_native_app_empty_plist_keeps_original_injected_writer_error() {
    let bytes = empty_binary_plist("platformSpecificOptions").unwrap();
    assert_eq!(bytes.as_ref(), b"bplist00\xd0\x08\x00\x00\x00\x00\x00\x00\x01\x01\x00\x00\x00\x00\x00\x00\x00\x01\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x09");
    use std::io::{self, Write};
    struct Writer(Option<io::Error>);
    impl Write for Writer {
        fn write(&mut self, _: &[u8]) -> io::Result<usize> {
            Err(self.0.take().expect("one native injected fault"))
        }
        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }
    let path = std::env::temp_dir().join(format!("ios-app-input-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir(&path).unwrap();
    let original = std::fs::File::open(path.join("owned-private-missing")).unwrap_err();
    std::fs::remove_dir(&path).unwrap();
    let os_code = original.raw_os_error();
    let error = write_empty_binary_plist(Writer(Some(original)), "owned-private").unwrap_err();
    assert!(has_source::<plist::Error>(&error));
    assert_eq!(crate::classify_error(&error), ErrorClass::InvalidInput);
    let AppServiceError::Xpc(XpcError::Io(outer)) = &error else {
        panic!("source lost");
    };
    assert_eq!(outer.kind(), io::ErrorKind::InvalidInput);
    let mut source = std::error::Error::source(outer);
    let mut native = None;
    while let Some(error) = source {
        if let Some(error) = error.downcast_ref::<io::Error>() {
            native = Some(error);
            break;
        }
        source = error.source();
    }
    assert_eq!(native.unwrap().raw_os_error(), os_code);
    assert!(!format!("{error:?} {:?}", outer.get_ref()).contains("owned-private"));
}
