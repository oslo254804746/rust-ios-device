use std::{error::Error, io, sync::atomic::Ordering};

use super::test_assets::{frame, Stream};
use super::*;
use crate::xpc::h2_raw::H2Error;
use crate::{test_native::has_source, ErrorClass};

fn h2_io(error: &RestoreError, kind: io::ErrorKind) {
    assert!(has_source::<H2Error>(error));
    let RestoreError::Io(outer) = error else {
        panic!("native source lost")
    };
    assert_eq!(outer.kind(), kind);
    assert!(!format!("{error:?} {:?}", outer.get_ref()).contains("owned-private"));
    let h2 = outer.source().unwrap().downcast_ref::<H2Error>().unwrap();
    let H2Error::Io(original) = h2 else {
        panic!("original IO lost")
    };
    assert_eq!(original.kind(), kind);
    assert_eq!(
        crate::classify_error(error),
        crate::classify_error(original)
    );
    if kind == io::ErrorKind::NotFound {
        assert!(original.raw_os_error().is_some());
    }
}

#[tokio::test]
async fn native_source_connect_and_all_bootstrap_writes_keep_h2_os_cause() {
    // Writes 0-3 are H2 preface/settings/window/ACK; 4-8 are bootstrap
    // HEADERS+DATA, stream 3 HEADERS, step 2 DATA and step 3 DATA.
    let mut errors = Vec::new();
    for stop in [0, 4, 5, 6, 7, 8] {
        let stream = Stream::new(frame(4, &[]), stop);
        let writes = stream.writes.clone();
        let error = RestoreServiceClient::connect(stream).await.err().unwrap();
        assert_eq!(writes.load(Ordering::SeqCst), stop + 1);
        errors.push(error);
    }
    for error in errors {
        h2_io(&error, io::ErrorKind::NotFound);
    }
}

#[tokio::test]
async fn native_source_connect_eof_and_invalid_settings_keep_h2_type() {
    let eof = RestoreServiceClient::connect(Stream::new(Vec::new(), usize::MAX))
        .await
        .err()
        .unwrap();
    h2_io(&eof, io::ErrorKind::UnexpectedEof);
    let invalid = RestoreServiceClient::connect(Stream::new(frame(4, &[1]), usize::MAX))
        .await
        .err()
        .unwrap();
    assert!(has_source::<H2Error>(&invalid));
    assert_eq!(crate::classify_error(&invalid), ErrorClass::Protocol);
}

#[tokio::test]
async fn native_source_request_write_and_response_read_keep_h2_cause() {
    let mut errors = Vec::new();
    for fail_write in [true, false] {
        let stream = Stream::new(frame(4, &[]), usize::MAX);
        let fail = stream.fail.clone();
        let mut client = RestoreServiceClient::connect(stream).await.unwrap();
        fail.store(fail_write, Ordering::SeqCst);
        let error = client.get_nonces().await.unwrap_err();
        assert_eq!(client.next_msg_id, if fail_write { 1 } else { 2 });
        errors.push((error, fail_write));
    }
    for (error, fail_write) in errors {
        h2_io(
            &error,
            if fail_write {
                io::ErrorKind::NotFound
            } else {
                io::ErrorKind::UnexpectedEof
            },
        );
    }
}

#[tokio::test]
async fn native_source_control_decode_keeps_xpc_and_utf8_causes() {
    let mut message = crate::xpc::message::encode_message(&XpcMessage {
        flags: crate::xpc::message::flags::ALWAYS_SET | crate::xpc::message::flags::REPLY,
        msg_id: 1,
        body: Some(XpcValue::Dictionary(IndexMap::from([(
            "owned-private".into(),
            XpcValue::Int64(1),
        )]))),
    })
    .unwrap()
    .to_vec();
    let at = message
        .windows(b"owned-private".len())
        .position(|bytes| bytes == b"owned-private")
        .unwrap();
    message[at] = 0xff;
    let mut input = frame(4, &[]);
    input.extend(frame(0, &message));
    let mut client = RestoreServiceClient::connect(Stream::new(input, usize::MAX))
        .await
        .unwrap();
    let error = client.get_nonces().await.unwrap_err();
    assert!(has_source::<crate::XpcError>(&error));
    assert!(has_source::<std::str::Utf8Error>(&error));
    assert_eq!(crate::classify_error(&error), ErrorClass::Protocol);
    assert!(!format!("{error:?}").contains("owned-private"));
}

#[tokio::test]
async fn native_source_control_buffer_limit_keeps_xpc_guard() {
    let mut client = RestoreServiceClient::connect(Stream::new(frame(4, &[]), usize::MAX))
        .await
        .unwrap();
    client
        .push_control_message(&vec![0; crate::xpc::message::XPC_PENDING_BUFFER_LIMIT])
        .unwrap();
    let error = client.push_control_message(&[0]).unwrap_err();
    assert!(has_source::<crate::XpcError>(&error));
    assert_eq!(crate::classify_error(&error), ErrorClass::Protocol);
}
