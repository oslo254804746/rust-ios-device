use super::*;
use crate::{test_native::has_source, ErrorClass};

#[tokio::test]
async fn input_native_test_pid_fails_before_control_lookup_or_writes() {
    let (stream, mut peer) = tokio::io::duplex(64);
    let mut client = TestmanagerClient::from_session_connection_for_test(stream, 1);
    let error = client
        .authorize_test_session_with_process_id(u64::MAX)
        .await
        .unwrap_err();
    assert!(has_source::<std::num::TryFromIntError>(&error));
    assert_eq!(crate::classify_error(&error), ErrorClass::InvalidInput);
    assert!(matches!(error, DtxError::Io(_)));
    // A valid signed PID proceeds to the existing missing-control guard.
    let error = client
        .authorize_test_session_with_process_id(i64::MAX as u64)
        .await
        .unwrap_err();
    assert!(matches!(error, DtxError::Protocol(_)));
    drop(client);
    let mut wire = Vec::new();
    tokio::time::timeout(
        std::time::Duration::from_secs(1),
        tokio::io::AsyncReadExt::read_to_end(&mut peer, &mut wire),
    )
    .await
    .unwrap()
    .unwrap();
    assert!(wire.is_empty());
}
