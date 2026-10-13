//! Fixed discovery cause slots and unchanged one-source completion contract.
use super::*;

#[tokio::test]
async fn intent_discovery_fixed_sources_keep_priority_and_bound_repeated_refusals() {
    let outcomes = scan::Outcomes::new();
    let lock = Mutex::new(());
    let session = outcomes.begin(lock.lock().await).unwrap();
    session.retain(
        scan::Source::Paired,
        std::io::Error::new(
            std::io::ErrorKind::ConnectionRefused,
            "paired-private-marker",
        )
        .into(),
    );
    session.retain(
        scan::Source::Paired,
        std::io::Error::new(std::io::ErrorKind::TimedOut, "later-private-paired").into(),
    );
    session.retain(
        scan::Source::Coordinator,
        crate::output::resource_decision("first-private-refusal"),
    );
    for _ in 0..4096 {
        session.retain(
            scan::Source::Coordinator,
            crate::output::resource_decision("later-private-refusal"),
        );
    }
    let primary: anyhow::Error =
        std::io::Error::new(std::io::ErrorKind::BrokenPipe, "usb-private-marker").into();
    let primary_depth = primary.chain().count();
    session.retain(scan::Source::Usb, primary);
    session.retain(
        scan::Source::Usb,
        std::io::Error::new(std::io::ErrorKind::PermissionDenied, "later-private-usb").into(),
    );
    let elapsed = tokio::time::timeout(Duration::ZERO, std::future::pending::<()>())
        .await
        .unwrap_err();
    let error = session.finish(Err(elapsed.into())).unwrap_err();
    assert!(outcomes.history().is_empty());
    assert_eq!(
        error.downcast_ref::<std::io::Error>().unwrap().kind(),
        std::io::ErrorKind::BrokenPipe
    );
    let context = error.downcast_ref::<scan::SourceContext>().unwrap();
    assert_eq!(
        context
            .secondary(scan::Source::Paired)
            .unwrap()
            .downcast_ref::<std::io::Error>()
            .unwrap()
            .kind(),
        std::io::ErrorKind::ConnectionRefused
    );
    assert_eq!(
        crate::output::operation_error_class(
            context
                .secondary(scan::Source::Coordinator)
                .unwrap()
                .as_ref()
        ),
        ios_core::ErrorClass::ResourceLimit
    );
    assert!(context
        .secondary(scan::Source::Coordinator)
        .unwrap()
        .to_string()
        .contains("first-private-refusal"));
    assert!(context
        .completion()
        .unwrap()
        .is::<tokio::time::error::Elapsed>());
    assert_eq!(
        error.chain().count(),
        primary_depth + 1,
        "unknown keys add only one fixed context to the original primary chain"
    );
    let safe = format!("{context:?} {context}");
    assert!(!safe.contains("private"));
    let rendered = crate::output::operation_failure_message(
        crate::output::ErrorFormat::Json,
        "tunnel-scan",
        &error,
    )
    .unwrap();
    assert!(!rendered.contains("private"));
}

#[tokio::test]
async fn intent_discovery_single_source_preserves_existing_failure_context_accessor() {
    let outcomes = scan::Outcomes::new();
    let lock = Mutex::new(());
    let session = outcomes.begin(lock.lock().await).unwrap();
    session.retain(
        scan::Source::Paired,
        std::io::Error::new(std::io::ErrorKind::UnexpectedEof, "original-private-paired").into(),
    );
    let elapsed = tokio::time::timeout(Duration::ZERO, std::future::pending::<()>())
        .await
        .unwrap_err();
    let error = session.finish(Err(elapsed.into())).unwrap_err();
    assert!(outcomes.history().is_empty());
    assert!(error.downcast_ref::<scan::SourceContext>().is_none());
    assert!(error
        .downcast_ref::<bookkeeping::FailureContext>()
        .unwrap()
        .secondary()
        .is::<tokio::time::error::Elapsed>());
    assert_eq!(
        error.downcast_ref::<std::io::Error>().unwrap().kind(),
        std::io::ErrorKind::UnexpectedEof
    );
}
