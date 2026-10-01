use super::*;
use crate::{CoreError, LockdownError, XpcError};

#[test]
fn typed_sources_survive_context_and_keep_the_original_error_available() {
    let secret = "synthetic-private-URL-credential-and-device";
    let original = CoreError::Lockdown(LockdownError::Io(std::io::Error::new(
        ErrorKind::TimedOut,
        secret,
    )));
    assert_eq!(original.class(), ErrorClass::Timeout);
    let mut wrapped = anyhow::Error::new(original);
    for _ in 0..24 {
        wrapped = wrapped.context(secret);
    }
    assert_eq!(classify_error(wrapped.as_ref()), ErrorClass::Timeout);
    let io = wrapped
        .chain()
        .find_map(|error| error.downcast_ref::<std::io::Error>())
        .unwrap();
    assert_eq!(io.kind(), ErrorKind::TimedOut);
    assert_eq!(io.to_string(), secret);
    assert!(!format!("{}", crate::redaction::diagnostic_error(wrapped.as_ref())).contains(secret));
}

#[test]
fn classification_never_matches_legacy_messages() {
    let misleading = "timed out: permission denied, service missing, unsupported";
    assert_eq!(
        CoreError::Other(misleading.into()).class(),
        ErrorClass::Unknown
    );
    assert_eq!(
        CoreError::Protocol(misleading.into()).class(),
        ErrorClass::Protocol
    );
    assert_eq!(
        CoreError::Xpc(XpcError::ServiceNotFound(misleading.into())).class(),
        ErrorClass::ServiceMissing
    );
}

#[test]
fn nested_service_wrappers_retain_the_typed_transport_source() {
    let error =
        crate::backup2::Mobilebackup2Error::DeviceLink(crate::device_link::DeviceLinkError::Io(
            std::io::Error::new(ErrorKind::PermissionDenied, "synthetic-private-source"),
        ));
    assert_eq!(classify_error(&error), ErrorClass::PermissionDenied);
    let link = error.source().unwrap();
    assert!(link.is::<crate::device_link::DeviceLinkError>());
    let io = link
        .source()
        .unwrap()
        .downcast_ref::<std::io::Error>()
        .unwrap();
    assert_eq!(io.kind(), ErrorKind::PermissionDenied);
    assert_eq!(io.to_string(), "synthetic-private-source");
}

#[test]
fn io_payloads_are_visited_but_eintr_and_file_eof_are_not_guessed() {
    let missing = std::io::Error::other(XpcError::ServiceNotFound("requested service".into()));
    assert_eq!(classify_error(&missing), ErrorClass::ServiceMissing);
    assert!(missing.get_ref().unwrap().is::<XpcError>());
    for kind in [
        ErrorKind::Interrupted,
        ErrorKind::UnexpectedEof,
        ErrorKind::WouldBlock,
    ] {
        assert_eq!(
            CoreError::Io(std::io::Error::from(kind)).class(),
            ErrorClass::Io
        );
    }
}

#[test]
fn malformed_codec_data_differs_from_its_underlying_writer_failure() {
    struct ClosedWriter;
    impl std::io::Write for ClosedWriter {
        fn write(&mut self, _: &[u8]) -> std::io::Result<usize> {
            Err(std::io::Error::new(
                ErrorKind::PermissionDenied,
                "synthetic-private-destination",
            ))
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
    let malformed = plist::Value::from_reader_xml(b"<plist><dict>".as_slice()).unwrap_err();
    assert_eq!(classify_error(&malformed), ErrorClass::Protocol);
    let writer = plist::to_writer_xml(ClosedWriter, &plist::Value::Boolean(true)).unwrap_err();
    assert_eq!(classify_error(&writer), ErrorClass::PermissionDenied);
    let json = serde_json::to_writer(ClosedWriter, &true).unwrap_err();
    assert_eq!(classify_error(&json), ErrorClass::PermissionDenied);
}

#[test]
fn cycles_are_bounded_without_formatting_unknown_errors() {
    struct Cycle(std::sync::atomic::AtomicUsize);
    impl std::fmt::Display for Cycle {
        fn fmt(&self, _: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            panic!("classification must not format unknown payloads")
        }
    }
    impl std::fmt::Debug for Cycle {
        fn fmt(&self, _: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            panic!("classification must not format unknown payloads")
        }
    }
    impl Error for Cycle {
        fn source(&self) -> Option<&(dyn Error + 'static)> {
            self.0.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            Some(self)
        }
    }
    let cycle = Cycle(std::sync::atomic::AtomicUsize::new(0));
    assert_eq!(classify_error(&cycle), ErrorClass::Unknown);
    assert_eq!(cycle.0.load(std::sync::atomic::Ordering::Relaxed), 32);
}

#[tokio::test]
async fn actual_elapsed_and_aborted_owned_tasks_have_distinct_categories() {
    let elapsed = tokio::time::timeout(
        std::time::Duration::from_millis(1),
        std::future::pending::<()>(),
    )
    .await
    .unwrap_err();
    assert_eq!(classify_error(&elapsed), ErrorClass::Timeout);
    let task = tokio::spawn(std::future::pending::<()>());
    task.abort();
    let cancelled = task.await.unwrap_err();
    assert!(cancelled.is_cancelled());
    assert_eq!(classify_error(&cancelled), ErrorClass::Cancelled);
}

#[test]
fn machine_codes_do_not_depend_on_diagnostic_display() {
    let missing = CoreError::Xpc(XpcError::ServiceNotFound(
        "synthetic-private-service".into(),
    ));
    let denied = CoreError::Io(std::io::Error::from(ErrorKind::PermissionDenied));
    assert_eq!(
        serde_json::json!({"missing":missing.class(),"denied":denied.class(),"unsupported":ErrorClass::Unsupported}),
        serde_json::json!({"missing":"service_missing","denied":"permission_denied","unsupported":"unsupported"})
    );
    assert_eq!(missing.class().as_str(), "service_missing");
    assert_eq!(
        CoreError::Io(std::io::Error::from(ErrorKind::NotFound)).class(),
        ErrorClass::NotFound
    );
}

#[cfg(feature = "afc")]
#[test]
fn afc_known_statuses_and_unknown_device_codes_remain_distinct() {
    use crate::afc::{AfcError, AfcStatusCode as Status};
    for (status, expected) in [
        (Status::PermDenied, ErrorClass::PermissionDenied),
        (Status::Timeout, ErrorClass::Timeout),
        (Status::ServiceNotConnected, ErrorClass::Disconnected),
        (Status::ObjectNotFound, ErrorClass::NotFound),
        (Status::OpNotSupported, ErrorClass::Unsupported),
        (Status::InvalidArgument, ErrorClass::InvalidInput),
        (Status::Other(u64::MAX), ErrorClass::ServiceFailure),
        (Status::OpInterrupted, ErrorClass::ServiceFailure),
    ] {
        assert_eq!(classify_error(&AfcError::Status(status)), expected);
    }
}

#[cfg(feature = "webinspector")]
#[test]
fn webinspector_retains_timeout_protocol_and_ambiguous_disabled_types() {
    use crate::webinspector::WebInspectorError;
    assert_eq!(
        classify_error(&WebInspectorError::Timeout(std::time::Duration::from_secs(
            1
        ))),
        ErrorClass::Timeout
    );
    assert_eq!(
        classify_error(&WebInspectorError::Disabled),
        ErrorClass::Precondition
    );
    assert_eq!(
        classify_error(&WebInspectorError::Protocol("timeout denied".into())),
        ErrorClass::Protocol
    );
}

#[cfg(all(feature = "pasteboard", feature = "display", feature = "hid"))]
#[test]
fn typed_media_clipboard_and_hid_failures_keep_their_specific_meaning() {
    assert_eq!(
        classify_error(&crate::display::DisplayError::MediaInUse),
        ErrorClass::Conflict
    );
    assert_eq!(
        classify_error(&crate::hid::HidError::FeatureMissing("synthetic feature")),
        ErrorClass::Unsupported
    );
    assert_eq!(
        classify_error(&crate::pasteboard::PasteboardError::Timeout { seconds: 1 }),
        ErrorClass::Timeout
    );
    assert_eq!(
        classify_error(&crate::pasteboard::PasteboardError::Closed),
        ErrorClass::Disconnected
    );
    assert_eq!(
        classify_error(&crate::pasteboard::PasteboardError::Service {
            domain: "synthetic domain".into(),
            code: "unknown-code".into(),
            description: "permission denied".into()
        }),
        ErrorClass::ServiceFailure
    );
}
