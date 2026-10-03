use super::*;
use crate::services::restore::test_assets::http;
use crate::{test_native::has_source, ErrorClass};

#[test]
fn native_source_tss_builder_retains_reqwest_input_error() {
    let error = build_tss_client(reqwest::Client::builder().no_proxy().user_agent("\n"))
        .err()
        .unwrap();
    assert!(has_source::<reqwest::Error>(&error));
    assert_eq!(crate::classify_error(&error), ErrorClass::InvalidInput);
}

#[tokio::test]
async fn native_source_tss_encode_keeps_local_plist_error_before_network() {
    let client = reqwest::Client::builder().no_proxy().build().unwrap();
    let request =
        plist::Dictionary::from_iter([("owned-private", plist::Value::Uid(plist::Uid::new(1)))]);
    let error = sign_tss_request_with_client(&client, "http://127.0.0.1:1", &request)
        .await
        .unwrap_err();
    assert!(has_source::<plist::Error>(&error));
    assert_eq!(crate::classify_error(&error), ErrorClass::InvalidInput);
    assert!(!format!("{error:?}").contains("owned-private"));
}

#[test]
fn native_source_tss_response_keeps_utf8_and_plist_protocol_causes() {
    let utf8 = parse_tss_response(b"STATUS=0&\xff").unwrap_err();
    let plist = parse_tss_response(
        b"STATUS=0&<?xml version=\"1.0\"?><plist><dict><key>owned-private</key>",
    )
    .unwrap_err();
    assert!(has_source::<std::str::Utf8Error>(&utf8));
    assert!(has_source::<plist::Error>(&plist));
    for error in [utf8, plist] {
        assert_eq!(crate::classify_error(&error), ErrorClass::Protocol);
        assert!(!format!("{error:?}").contains("owned-private"));
    }
}

#[tokio::test]
async fn native_source_tss_request_and_body_retain_actual_http_timeout() {
    let mut errors = Vec::new();
    for response in [
        None,
        Some(b"HTTP/1.1 200 OK\r\nContent-Length: 4\r\nConnection: close\r\n\r\n".as_slice()),
    ] {
        let (error, wire) = http(response, true, |client, url| async move {
            sign_tss_request_with_client(&client, &url, &plist::Dictionary::new())
                .await
                .unwrap_err()
        })
        .await;
        assert!(wire.starts_with(b"POST /owned-private HTTP/1.1\r\n"));
        errors.push(error);
    }
    for error in errors {
        assert!(has_source::<reqwest::Error>(&error));
        assert_eq!(crate::classify_error(&error), ErrorClass::Timeout);
        assert!(!format!("{error:?}").contains("owned-private"));
    }
}

#[tokio::test]
async fn native_source_tss_endpoint_builder_and_status_guards_stay_distinct() {
    let client = reqwest::Client::builder().no_proxy().build().unwrap();
    let error = sign_tss_request_with_client(&client, "http://[", &plist::Dictionary::new())
        .await
        .unwrap_err();
    assert!(has_source::<reqwest::Error>(&error));
    assert_eq!(crate::classify_error(&error), ErrorClass::InvalidInput);
    for body in [
        b"STATUS=94&private".as_slice(),
        b"STATUS=0&missing",
        b"STATUS=0&<plist><array/></plist>",
    ] {
        let error = parse_tss_response(body).unwrap_err();
        assert!(matches!(error, RestoreError::Protocol(_)));
        assert!(!has_source::<reqwest::Error>(&error));
    }
}
