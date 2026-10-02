use super::*;
use crate::{classify_error, test_util::MockStream, CoreError, ErrorClass};
use std::{error::Error, io};

fn native(error: &MuxError, kind: io::ErrorKind, class: ErrorClass) -> &io::Error {
    let MuxError::Io(source) = error else {
        panic!("native mux cause lost")
    };
    assert_eq!(source.kind(), kind);
    assert_eq!(classify_error(error), class);
    assert!(!format!("{error:?} {:?}", source.get_ref()).contains("owned-private"));
    source
}

#[tokio::test]
async fn actual_serializer_error_retains_plist_before_any_header_write() {
    struct Refused;
    impl Serialize for Refused {
        fn serialize<S: serde::Serializer>(&self, _: S) -> Result<S::Ok, S::Error> {
            Err(serde::ser::Error::custom("owned-private timeout cancelled"))
        }
    }
    let mut stream = MockStream::eof();
    let error = send_plist(&mut stream, &Refused, 7).await.unwrap_err();
    let original = native(
        &error,
        io::ErrorKind::InvalidInput,
        ErrorClass::InvalidInput,
    )
    .source()
    .unwrap()
    .downcast_ref::<plist::Error>()
    .unwrap();
    assert!(original.to_string().contains("owned-private"));
    assert!(stream.written.is_empty());
    assert_eq!(CoreError::from(error).class(), ErrorClass::InvalidInput);
}

#[tokio::test]
async fn actual_received_xml_and_schema_failures_retain_plist_after_complete_frame() {
    let mut missing_field = Vec::new();
    plist::to_writer_xml(
        &mut missing_field,
        &plist::Value::Dictionary(plist::Dictionary::new()),
    )
    .unwrap();
    for payload in [b"<plist><dict>owned-private".to_vec(), missing_field] {
        let mut stream = MockStream::new(encode_message(&payload, 7).unwrap());
        let error = recv_plist::<_, DeviceList>(&mut stream).await.unwrap_err();
        assert!(
            native(&error, io::ErrorKind::InvalidData, ErrorClass::Protocol)
                .source()
                .unwrap()
                .is::<plist::Error>()
        );
        assert_eq!(stream.read_pos, stream.read_buf.len());
        let outer = CoreError::from(error);
        assert_eq!(outer.class(), ErrorClass::Protocol);
        assert!(outer.source().unwrap().is::<MuxError>());
    }
}

#[tokio::test]
async fn actual_write_header_and_body_failures_keep_original_io() {
    let (mut stream, peer) = tokio::io::duplex(64);
    drop(peer);
    let error = send_plist(&mut stream, &plist::Value::Boolean(true), 7)
        .await
        .unwrap_err();
    native(&error, io::ErrorKind::BrokenPipe, ErrorClass::Disconnected);
    for bytes in [
        Vec::new(),
        encode_message(b"body", 7).unwrap()[..18].to_vec(),
    ] {
        let mut stream = io::Cursor::new(bytes);
        let error = recv_plist::<_, plist::Value>(&mut stream)
            .await
            .unwrap_err();
        native(&error, io::ErrorKind::UnexpectedEof, ErrorClass::Io);
    }
}
