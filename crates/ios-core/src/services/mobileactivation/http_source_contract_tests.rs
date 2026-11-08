//! Owned host HTTPS dial faults; no request to an Apple/device endpoint.

use super::*;

#[test]
fn endpoint_inputs_and_native_builder_error_keep_typed_sources() {
    for (url, custom) in [
        ("owned-private-invalid", true),
        ("http://example.invalid/owned-private", true),
        ("https://owned-private:owned-private@example.invalid", true),
        ("https://example.invalid/owned-private", false),
    ] {
        let error = ActivationHttpClient::with_endpoints(
            url,
            DRM_HANDSHAKE_URL,
            custom,
            Duration::from_secs(1),
        )
        .unwrap_err();
        assert_eq!(
            crate::classify_error(&error),
            crate::ErrorClass::InvalidInput
        );
        let MobileActivationError::Io(outer) = &error else {
            panic!("endpoint reason lost");
        };
        assert!(outer
            .get_ref()
            .unwrap()
            .is::<http_error::EndpointInputError>());
        assert!(!format!("{error:?} {:?}", outer.get_ref()).contains("owned-private"));
        if url == "owned-private-invalid" {
            assert!(std::error::Error::source(outer).is_some());
        }
    }
    let native = reqwest::Client::builder()
        .user_agent("owned-private\ninvalid")
        .build()
        .unwrap_err();
    assert!(native.is_builder());
    let error = http_error::retain("build activation HTTP client", native);
    assert_eq!(
        crate::classify_error(&error),
        crate::ErrorClass::InvalidInput
    );
    let MobileActivationError::Io(outer) = &error else {
        panic!("HTTP builder cause lost");
    };
    let native = std::error::Error::source(outer)
        .unwrap()
        .downcast_ref::<reqwest::Error>()
        .unwrap();
    assert!(native.is_builder());
    assert!(!format!("{error:?} {:?}", outer.get_ref()).contains("owned-private"));
}

#[tokio::test]
async fn public_https_refusal_keeps_request_cause_and_private_url() {
    let socket = tokio::net::TcpSocket::new_v4().unwrap();
    socket.bind("127.0.0.1:0".parse().unwrap()).unwrap();
    let endpoint = format!(
        "https://{}/owned-private?token=owned-private",
        socket.local_addr().unwrap()
    );
    #[cfg(target_os = "macos")]
    drop(socket);
    let client =
        // This fixture tests refusal, independently of the timeout fixture
        // below. Winsock needs ~2s to produce the original dial failure.
        ActivationHttpClient::with_endpoints(&endpoint, &endpoint, true, Duration::from_secs(if cfg!(windows) { 5 } else { 1 }))
            .unwrap();
    let error = client
        .post_drm_handshake(&plist::Dictionary::new())
        .await
        .unwrap_err();
    #[cfg(not(target_os = "macos"))]
    drop(socket);
    let MobileActivationError::Io(outer) = &error else {
        panic!("request cause lost");
    };
    let native = std::error::Error::source(outer)
        .unwrap()
        .downcast_ref::<reqwest::Error>()
        .unwrap();
    assert!(native.is_connect());
    assert_eq!(native.url().unwrap().as_str(), endpoint);
    assert_eq!(crate::classify_error(&error), crate::ErrorClass::Io);
    assert!(!format!("{error:?} {:?}", outer.get_ref()).contains("owned-private"));
}

#[tokio::test]
async fn public_https_timeout_retains_native_reqwest_cause() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let endpoint = format!(
        "https://{}/owned-private?token=owned-private",
        listener.local_addr().unwrap()
    );
    let peer = tokio::spawn(async move {
        let (_stream, _) = listener.accept().await.unwrap();
        std::future::pending::<()>().await;
    });
    let client = ActivationHttpClient::with_endpoints(
        &endpoint,
        &endpoint,
        true,
        Duration::from_millis(100),
    )
    .unwrap();
    let error = client
        .post_drm_handshake(&plist::Dictionary::new())
        .await
        .unwrap_err();
    peer.abort();
    assert!(peer.await.unwrap_err().is_cancelled());
    assert_eq!(crate::classify_error(&error), crate::ErrorClass::Timeout);
    let MobileActivationError::Io(outer) = &error else {
        panic!("HTTP source discarded");
    };
    let original = std::error::Error::source(outer)
        .unwrap()
        .downcast_ref::<reqwest::Error>()
        .unwrap();
    assert!(original.is_timeout());
    assert!(original
        .url()
        .unwrap()
        .query()
        .unwrap()
        .contains("owned-private"));
    assert!(!format!("{error:?} {:?}", outer.get_ref()).contains("owned-private"));
}

#[test]
fn endpoint_debug_hides_custom_query_values() {
    let client = ActivationHttpClient::with_endpoints(
        "https://example.invalid/owned-private?token=owned-private",
        DRM_HANDSHAKE_URL,
        true,
        Duration::from_secs(1),
    )
    .unwrap();
    assert!(!format!("{client:?}").contains("owned-private"));
}

// OpenSSL is already an optional dependency of tunnel. Minimal
// mobileactivation-only builds still run the endpoint/dial tests above.
#[cfg(feature = "tunnel")]
mod verified_tls {
    use super::*;
    use openssl::{
        asn1::Asn1Time,
        bn::BigNum,
        ec::{EcGroup, EcKey},
        hash::MessageDigest,
        nid::Nid,
        pkey::{PKey, Private},
        x509::{
            extension::{BasicConstraints, ExtendedKeyUsage, KeyUsage, SubjectAlternativeName},
            X509NameBuilder, X509,
        },
    };
    use std::error::Error;
    use std::sync::Arc;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    fn key() -> PKey<Private> {
        let group = EcGroup::from_curve_name(Nid::X9_62_PRIME256V1).unwrap();
        PKey::from_ec_key(EcKey::generate(&group).unwrap()).unwrap()
    }

    fn certificate(key: &PKey<Private>, issuer: Option<(&X509, &PKey<Private>)>) -> X509 {
        let mut name = X509NameBuilder::new().unwrap();
        name.append_entry_by_text("CN", "owned local HTTPS fixture")
            .unwrap();
        let name = name.build();
        let mut cert = X509::builder().unwrap();
        cert.set_version(2).unwrap();
        cert.set_serial_number(
            &BigNum::from_u32(if issuer.is_some() { 2 } else { 1 })
                .unwrap()
                .to_asn1_integer()
                .unwrap(),
        )
        .unwrap();
        cert.set_subject_name(&name).unwrap();
        cert.set_issuer_name(issuer.map_or(name.as_ref(), |(ca, _)| ca.subject_name()))
            .unwrap();
        cert.set_pubkey(key).unwrap();
        cert.set_not_before(&Asn1Time::days_from_now(0).unwrap())
            .unwrap();
        cert.set_not_after(&Asn1Time::days_from_now(1).unwrap())
            .unwrap();
        if let Some((ca, _)) = issuer {
            cert.append_extension(BasicConstraints::new().critical().build().unwrap())
                .unwrap();
            cert.append_extension(KeyUsage::new().digital_signature().build().unwrap())
                .unwrap();
            cert.append_extension(ExtendedKeyUsage::new().server_auth().build().unwrap())
                .unwrap();
            let san = SubjectAlternativeName::new()
                .ip("127.0.0.1")
                .build(&cert.x509v3_context(Some(ca), None))
                .unwrap();
            cert.append_extension(san).unwrap();
        } else {
            cert.append_extension(BasicConstraints::new().critical().ca().build().unwrap())
                .unwrap();
            cert.append_extension(
                KeyUsage::new()
                    .critical()
                    .key_cert_sign()
                    .crl_sign()
                    .build()
                    .unwrap(),
            )
            .unwrap();
        }
        cert.sign(
            issuer.map_or(key, |(_, ca_key)| ca_key),
            MessageDigest::sha256(),
        )
        .unwrap();
        cert.build()
    }

    async fn fixture(
        response: &'static [u8],
        stall_body: bool,
        trust_fixture_ca: bool,
    ) -> (
        ActivationHttpClient,
        tokio::task::JoinHandle<std::io::Result<()>>,
    ) {
        let ca_key = key();
        let ca = certificate(&ca_key, None);
        let leaf_key = key();
        let leaf = certificate(&leaf_key, Some((&ca, &ca_key)));
        let root = reqwest::Certificate::from_der(&ca.to_der().unwrap()).unwrap();
        let config = rustls::ServerConfig::builder_with_provider(Arc::new(
            rustls::crypto::ring::default_provider(),
        ))
        .with_safe_default_protocol_versions()
        .unwrap()
        .with_no_client_auth()
        .with_single_cert(
            vec![leaf.to_der().unwrap().into(), ca.to_der().unwrap().into()],
            rustls::pki_types::PrivatePkcs8KeyDer::from(leaf_key.private_key_to_pkcs8().unwrap())
                .into(),
        )
        .unwrap();
        let acceptor = tokio_rustls::TlsAcceptor::from(Arc::new(config));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let endpoint = format!(
            "https://{}/owned-private?token=owned-private",
            listener.local_addr().unwrap()
        );
        let timeout = Duration::from_secs(3);
        let mut client =
            ActivationHttpClient::with_endpoints(&endpoint, &endpoint, true, timeout).unwrap();
        if trust_fixture_ca {
            // Test-only trust installation, with certificate verification enabled.
            client.client = reqwest::Client::builder()
                .add_root_certificate(root)
                .user_agent(ACTIVATION_USER_AGENT)
                .timeout(timeout)
                .redirect(reqwest::redirect::Policy::none())
                .no_proxy()
                .build()
                .unwrap();
        }
        let peer = tokio::spawn(async move {
            let (stream, _) = listener.accept().await?;
            let mut stream = acceptor.accept(stream).await?;
            let mut header = Vec::new();
            while !header.ends_with(b"\r\n\r\n") {
                if header.len() >= 16 * 1024 {
                    return Err(std::io::Error::from(std::io::ErrorKind::InvalidData));
                }
                header.push(stream.read_u8().await?);
            }
            let length = std::str::from_utf8(&header)
                .unwrap()
                .lines()
                .filter_map(|line| line.split_once(':'))
                .find(|(name, _)| name.eq_ignore_ascii_case("content-length"))
                .map(|(_, value)| value.trim().parse::<usize>().unwrap())
                .unwrap();
            assert!(length <= 16 * 1024);
            let mut body = vec![0; length];
            stream.read_exact(&mut body).await?;
            stream.write_all(response).await?;
            stream.flush().await?;
            if stall_body {
                std::future::pending::<()>().await;
            }
            stream.shutdown().await
        });
        (client, peer)
    }

    fn original(error: &MobileActivationError) -> &reqwest::Error {
        let MobileActivationError::Io(outer) = error else {
            panic!("native HTTP cause lost");
        };
        assert!(!format!("{error:?} {:?}", outer.get_ref()).contains("owned-private"));
        outer
            .source()
            .unwrap()
            .downcast_ref::<reqwest::Error>()
            .unwrap()
    }

    #[tokio::test]
    async fn truncated_https_body_keeps_native_chunk_error_without_partial_success() {
        let (client, peer) = fixture(
            b"HTTP/1.1 200 OK\r\nContent-Length: 6\r\n\r\nab",
            false,
            true,
        )
        .await;
        let result = client.post_drm_handshake(&plist::Dictionary::new()).await;
        peer.await.unwrap().unwrap();
        let error = result.unwrap_err();
        let native = original(&error);
        assert!(native.is_body() || native.is_decode());
        let MobileActivationError::Io(outer) = error else {
            unreachable!()
        };
        assert!(format!("{:?}", outer.get_ref()).contains("read activation HTTP body"));
    }

    #[tokio::test]
    async fn delayed_https_body_keeps_timeout_and_read_phase() {
        let (client, peer) = fixture(
            b"HTTP/1.1 200 OK\r\nContent-Length: 6\r\n\r\nab",
            true,
            true,
        )
        .await;
        let result = client.post_activation_info(&plist::Dictionary::new()).await;
        peer.abort();
        assert!(peer.await.unwrap_err().is_cancelled());
        let error = result.unwrap_err();
        assert!(original(&error).is_timeout());
        assert_eq!(crate::classify_error(&error), crate::ErrorClass::Timeout);
        let MobileActivationError::Io(outer) = error else {
            unreachable!()
        };
        assert!(format!("{:?}", outer.get_ref()).contains("read activation HTTP body"));
    }

    #[tokio::test]
    async fn verified_https_success_and_status_failure_keep_existing_contracts() {
        let (client, peer) = fixture(b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\nX-Apple-Session-Id: owned-private\r\nContent-Type: application/xml\r\n\r\nOK", false, true).await;
        let result = client.post_drm_handshake(&plist::Dictionary::new()).await;
        peer.await.unwrap().unwrap();
        let response = result.unwrap();
        assert_eq!(response.body, b"OK");
        assert_eq!(
            response.headers.get("X-Apple-Session-Id").unwrap(),
            "owned-private"
        );
        assert!(response.content_type_is("application/xml"));
        assert!(!format!("{response:?}").contains("owned-private"));
        let (client, peer) = fixture(
            b"HTTP/1.1 403 Forbidden\r\nContent-Length: 2\r\n\r\nNO",
            false,
            true,
        )
        .await;
        let result = client.post_activation_info(&plist::Dictionary::new()).await;
        peer.await.unwrap().unwrap();
        let error = result.unwrap_err();
        assert!(matches!(error, MobileActivationError::Http(_)));
        assert_eq!(
            crate::classify_error(&error),
            crate::ErrorClass::ServiceFailure
        );
    }

    #[tokio::test]
    async fn public_https_constructor_rejects_untrusted_fixture_certificate() {
        let (client, peer) = fixture(b"", false, false).await;
        let result = client.post_drm_handshake(&plist::Dictionary::new()).await;
        assert!(peer.await.unwrap().is_err());
        let error = result.unwrap_err();
        assert!(original(&error).is_connect());
    }
}
