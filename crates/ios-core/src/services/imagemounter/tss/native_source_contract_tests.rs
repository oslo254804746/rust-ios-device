use super::*;
use crate::services::imagemounter::test_assets::http;
use crate::test_native::has_source;

#[test]
fn native_source_request_serialization_keeps_local_plist_error() {
    let dict =
        plist::Dictionary::from_iter([("owned-private", plist::Value::Uid(plist::Uid::new(1)))]);
    let error = serialize_request(&dict).unwrap_err();
    assert!(has_source::<plist::Error>(&error));
    assert_eq!(
        crate::classify_error(&error),
        crate::ErrorClass::InvalidInput
    );
    assert!(!format!("{error:?}").contains("owned-private"));
}

#[test]
fn native_source_ticket_utf8_and_plist_errors_are_retained() {
    let utf8 = parse_ticket_response(b"STATUS=0&\xff", &["ApImg4Ticket"]).unwrap_err();
    let plist = parse_ticket_response(
        b"STATUS=0&<?xml version=\"1.0\"?><plist version=\"1.0\"><dict><key>owned-private</key>",
        &["ApImg4Ticket"],
    )
    .unwrap_err();
    assert!(has_source::<std::str::Utf8Error>(&utf8));
    assert!(has_source::<plist::Error>(&plist));
    for error in [utf8, plist] {
        assert_eq!(crate::classify_error(&error), crate::ErrorClass::Protocol);
        assert!(!format!("{error:?}").contains("owned-private"));
    }
}

#[test]
fn native_source_client_builder_retains_reqwest() {
    let error =
        build_client_with_proxy(reqwest::Client::builder().no_proxy().user_agent("\n"), None)
            .err()
            .unwrap();
    assert!(has_source::<reqwest::Error>(&error));
    assert_eq!(
        crate::classify_error(&error),
        crate::ErrorClass::InvalidInput
    );
}

#[tokio::test]
async fn native_source_ticket_http_request_and_chunk_keep_reqwest_timeout() {
    let mut errors = Vec::new();
    for response in [
        None,
        Some(b"HTTP/1.1 200 OK\r\nContent-Length: 4\r\nConnection: close\r\n\r\n".as_slice()),
    ] {
        let (error, request) = http(response, true, |client, url| async move {
            request_ticket_at(&client, &url, b"<plist/>".to_vec(), &["ApImg4Ticket"])
                .await
                .unwrap_err()
        })
        .await;
        assert!(request.starts_with(b"POST /owned-private HTTP/1.1\r\n"));
        errors.push(error);
    }
    for error in errors {
        assert!(has_source::<reqwest::Error>(&error));
        assert_eq!(crate::classify_error(&error), crate::ErrorClass::Timeout);
        assert!(!format!("{error:?}").contains("owned-private"));
    }
}

#[tokio::test]
async fn asset_guards_keep_tss_status_ticket_and_optional_proxy_policy() {
    // Match the existing permissive TSS proxy policy without changing process env.
    assert!(
        build_client_with_proxy(reqwest::Client::builder().no_proxy(), Some("http://[")).is_ok()
    );
    for body in [
        b"STATUS=94&<plist/>".as_slice(),
        b"STATUS=0&missing plist".as_slice(),
        b"STATUS=0&<plist><dict/></plist>".as_slice(),
    ] {
        let error = parse_ticket_response(body, &["ApImg4Ticket"]).unwrap_err();
        assert!(matches!(error, ImageMounterError::Tss(_)));
        assert_eq!(
            crate::classify_error(&error),
            crate::ErrorClass::ServiceFailure
        );
    }
    let (error, _) = http(
        Some(b"HTTP/1.1 403 Forbidden\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"),
        false,
        |client, url| async move {
            request_ticket_at(&client, &url, b"<plist/>".to_vec(), &["ApImg4Ticket"])
                .await
                .unwrap_err()
        },
    )
    .await;
    assert!(matches!(error, ImageMounterError::Tss(_)));
    assert!(!has_source::<reqwest::Error>(&error));
}

#[tokio::test]
async fn asset_guards_http_ticket_success_keeps_request_headers_and_body() {
    let dict = plist::Dictionary::from_iter([("fixture", plist::Value::String("owned".into()))]);
    let request = serialize_request(&dict).unwrap();
    let expected = request.clone();
    let mut body = b"STATUS=0&MESSAGE=SUCCESS&REQUEST_STRING=".to_vec();
    plist::to_writer_xml(
        &mut body,
        &plist::Dictionary::from_iter([("ApImg4Ticket", plist::Value::Data(b"ticket".to_vec()))]),
    )
    .unwrap();
    let mut response = format!(
        "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        body.len()
    )
    .into_bytes();
    response.extend(body);
    let (ticket, wire) = http(Some(&response), false, |client, url| async move {
        request_ticket_at(&client, &url, request, &["ApImg4Ticket"])
            .await
            .unwrap()
    })
    .await;
    assert_eq!(ticket, b"ticket");
    let start = wire
        .windows(4)
        .position(|window| window == b"\r\n\r\n")
        .unwrap()
        + 4;
    assert_eq!(&wire[start..], expected);
    let headers = std::str::from_utf8(&wire[..start])
        .unwrap()
        .to_ascii_lowercase();
    assert!(headers.contains("content-type: text/xml; charset=\"utf-8\"\r\n"));
    assert!(headers.contains("user-agent: ineturl/1.0\r\n"));
}
