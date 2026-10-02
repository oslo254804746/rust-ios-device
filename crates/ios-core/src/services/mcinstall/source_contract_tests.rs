use super::*;
use crate::{classify_error, test_util::MockStream, ErrorClass};
use std::{error::Error, io};

fn source(error: &McInstallError, kind: io::ErrorKind, class: ErrorClass) -> &io::Error {
    let McInstallError::Io(io) = error else {
        panic!("typed cause lost")
    };
    assert_eq!(io.kind(), kind);
    assert_eq!(classify_error(error), class);
    assert!(!format!("{error:?} {:?}", io.get_ref()).contains("owned-private"));
    io
}

#[cfg(not(feature = "supervised-pair"))]
#[tokio::test]
async fn actual_feature_disabled_calls_report_typed_unsupported_without_io() {
    let mut client = McInstallClient::new(MockStream::eof());
    let error = client
        .escalate_with_p12(b"owned-private", "owned-password")
        .await
        .unwrap_err();
    assert!(matches!(
        source(&error, io::ErrorKind::Other, ErrorClass::Unsupported)
            .source()
            .unwrap()
            .downcast_ref::<crate::CoreError>()
            .unwrap(),
        crate::CoreError::Unsupported(_)
    ));
    let error = client
        .install_profile_silent(b"owned-payload", b"owned-private", "owned-password")
        .await
        .unwrap_err();
    source(&error, io::ErrorKind::Other, ErrorClass::Unsupported);
    assert!(client.stream.written.is_empty());
}

#[tokio::test]
async fn actual_serializer_failure_still_retains_native_plist_without_writes() {
    struct Refused;
    impl serde::Serialize for Refused {
        fn serialize<S: serde::Serializer>(&self, _: S) -> Result<S::Ok, S::Error> {
            Err(serde::ser::Error::custom("owned-private timeout cancelled"))
        }
    }
    let mut client = McInstallClient::new(MockStream::eof());
    let error = client.send_plist(&Refused).await.unwrap_err();
    let McInstallError::Plist(native) = &error else {
        panic!("plist source lost")
    };
    assert_eq!(classify_error(&error), ErrorClass::Protocol);
    assert!(native.to_string().contains("owned-private"));
    assert!(error.source().unwrap().is::<plist::Error>());
    assert!(client.stream.written.is_empty());
}

#[cfg(feature = "supervised-pair")]
mod supervised {
    use super::*;
    use openssl::{
        asn1::Asn1Time,
        bn::BigNum,
        error::ErrorStack,
        hash::MessageDigest,
        pkey::{PKey, Private},
        rsa::Rsa,
        x509::{store::X509StoreBuilder, X509NameBuilder, X509},
    };
    use std::time::Duration;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    fn identity(ed25519: bool) -> (Vec<u8>, X509) {
        let key: PKey<Private> = if ed25519 {
            PKey::generate_ed25519().unwrap()
        } else {
            PKey::from_rsa(Rsa::generate(2048).unwrap()).unwrap()
        };
        let name = X509NameBuilder::new().unwrap().build();
        let mut cert = X509::builder().unwrap();
        cert.set_version(2).unwrap();
        cert.set_serial_number(&BigNum::from_u32(1).unwrap().to_asn1_integer().unwrap())
            .unwrap();
        cert.set_subject_name(&name).unwrap();
        cert.set_issuer_name(&name).unwrap();
        cert.set_pubkey(&key).unwrap();
        cert.set_not_before(&Asn1Time::days_from_now(0).unwrap())
            .unwrap();
        cert.set_not_after(&Asn1Time::days_from_now(1).unwrap())
            .unwrap();
        cert.sign(
            &key,
            if ed25519 {
                MessageDigest::null()
            } else {
                MessageDigest::sha256()
            },
        )
        .unwrap();
        let cert = cert.build();
        let bundle = Pkcs12::builder()
            .name("owned-organization")
            .pkey(&key)
            .cert(&cert)
            .build2("owned-password")
            .unwrap()
            .to_der()
            .unwrap();
        (bundle, cert)
    }

    fn ack() -> plist::Value {
        plist::Value::Dictionary(plist::Dictionary::from_iter([("Status", "Acknowledged")]))
    }
    fn challenge() -> plist::Value {
        let mut fields = ack().into_dictionary().unwrap();
        fields.insert(
            "Challenge".into(),
            plist::Value::Data(b"owned-challenge\0\xff\r\n".to_vec()),
        );
        fields.into()
    }
    fn stack(error: &McInstallError, class: ErrorClass, kind: io::ErrorKind) {
        let native = source(error, kind, class)
            .source()
            .unwrap()
            .downcast_ref::<ErrorStack>()
            .unwrap();
        assert!(!native.errors().is_empty());
    }

    #[tokio::test]
    async fn actual_bad_der_password_and_nul_keep_causes_before_escalate_write() {
        let (valid, _) = identity(false);
        for (bytes, password) in [
            (b"owned-private invalid DER".as_slice(), "owned-password"),
            (valid.as_slice(), "owned-private wrong-password"),
        ] {
            let mut client = McInstallClient::new(MockStream::eof());
            let error = client.escalate_with_p12(bytes, password).await.unwrap_err();
            stack(
                &error,
                ErrorClass::InvalidInput,
                io::ErrorKind::InvalidInput,
            );
            assert!(client.stream.written.is_empty());
        }
        let mut client = McInstallClient::new(MockStream::eof());
        let error = client
            .escalate_with_p12(&valid, "owned-private\0password")
            .await
            .unwrap_err();
        let native = source(
            &error,
            io::ErrorKind::InvalidInput,
            ErrorClass::InvalidInput,
        )
        .source()
        .unwrap()
        .downcast_ref::<std::ffi::NulError>()
        .unwrap();
        assert_eq!(native.nul_position(), 13);
        assert!(client.stream.written.is_empty());
    }

    #[tokio::test]
    async fn actual_nul_password_is_rejected_before_dependency_unwrap() {
        let (valid, _) = identity(false);
        let mut client = McInstallClient::new(MockStream::eof());
        let error = client
            .install_profile_silent(b"owned-payload", &valid, "owned-private\0password")
            .await
            .unwrap_err();
        assert!(source(
            &error,
            io::ErrorKind::InvalidInput,
            ErrorClass::InvalidInput
        )
        .source()
        .unwrap()
        .is::<std::ffi::NulError>());
        assert!(client.stream.written.is_empty());
    }

    #[tokio::test]
    async fn actual_incomplete_p12_reports_input_decision_without_invented_stack() {
        let (_, cert) = identity(false);
        let bytes = Pkcs12::builder()
            .name("owned-incomplete")
            .cert(&cert)
            .build2("owned-password")
            .unwrap()
            .to_der()
            .unwrap();
        let parsed = Pkcs12::from_der(&bytes)
            .unwrap()
            .parse2("owned-password")
            .unwrap();
        assert!(parsed.cert.is_none() || parsed.pkey.is_none());
        let mut client = McInstallClient::new(MockStream::eof());
        let error = client
            .escalate_with_p12(&bytes, "owned-password")
            .await
            .unwrap_err();
        let native = source(
            &error,
            io::ErrorKind::InvalidInput,
            ErrorClass::InvalidInput,
        );
        assert!(native.source().is_none());
        assert!(client.stream.written.is_empty());
    }

    #[tokio::test]
    async fn actual_pkcs7_failure_keeps_opaque_stack_and_stops_before_response() {
        let (bundle, _) = identity(true);
        let mut client = McInstallClient::new(MockStream::with_response(challenge()));
        let error = client
            .escalate_with_p12(&bundle, "owned-password")
            .await
            .unwrap_err();
        stack(&error, ErrorClass::Unknown, io::ErrorKind::Other);
        let mut sent = MockStream::new(client.stream.written);
        let request: plist::Dictionary = recv_plist(&mut sent).await.unwrap();
        assert_eq!(request["RequestType"].as_string(), Some("Escalate"));
        assert_eq!(sent.read_pos, sent.read_buf.len());
    }

    #[tokio::test]
    async fn actual_transport_and_codec_failures_keep_sources_after_valid_p12() {
        let (bundle, _) = identity(false);
        let (stream, peer) = tokio::io::duplex(4096);
        drop(peer);
        let mut client = McInstallClient::new(stream);
        let error = client
            .escalate_with_p12(&bundle, "owned-password")
            .await
            .unwrap_err();
        source(&error, io::ErrorKind::BrokenPipe, ErrorClass::Disconnected);
        let mut client =
            McInstallClient::new(MockStream::with_raw_frames(vec![b"<plist><dict>".to_vec()]));
        let error = client
            .escalate_with_p12(&bundle, "owned-password")
            .await
            .unwrap_err();
        assert_eq!(classify_error(&error), ErrorClass::Protocol);
        assert!(error.source().unwrap().is::<plist::Error>());
    }

    #[tokio::test]
    async fn actual_ack_failures_gate_each_stage_and_silent_install() {
        let (bundle, _) = identity(false);
        for stop in 0..4 {
            let mut replies = vec![challenge(), ack(), ack(), ack()];
            replies[stop] =
                plist::Value::Dictionary(plist::Dictionary::from_iter([("Status", "Error")]));
            replies.truncate(stop + 1);
            let mut client = McInstallClient::new(MockStream::with_responses(replies));
            let error = client
                .install_profile_silent(b"owned-payload", &bundle, "owned-password")
                .await
                .unwrap_err();
            assert!(matches!(error, McInstallError::Protocol(_)));
            assert_eq!(classify_error(&error), ErrorClass::Protocol);
            let mut sent = MockStream::new(client.stream.written);
            for expected in [
                "Escalate",
                "EscalateResponse",
                "ProceedWithKeybagMigration",
                "InstallProfileSilent",
            ]
            .into_iter()
            .take(stop + 1)
            {
                let request: plist::Dictionary = recv_plist(&mut sent).await.unwrap();
                assert_eq!(request["RequestType"].as_string(), Some(expected));
            }
            assert_eq!(sent.read_pos, sent.read_buf.len());
        }
    }

    async fn receive(peer: &mut tokio::io::DuplexStream) -> plist::Dictionary {
        let length = peer.read_u32().await.unwrap() as usize;
        assert!(length <= 64 * 1024);
        let mut payload = vec![0; length];
        peer.read_exact(&mut payload).await.unwrap();
        plist::from_bytes(&payload).unwrap()
    }
    async fn reply(peer: &mut tokio::io::DuplexStream, value: plist::Value) {
        let mut payload = Vec::new();
        plist::to_writer_xml(&mut payload, &value).unwrap();
        assert!(payload.len() <= 64 * 1024);
        peer.write_all(&(payload.len() as u32).to_be_bytes())
            .await
            .unwrap();
        peer.write_all(&payload).await.unwrap();
    }

    #[tokio::test]
    async fn actual_public_silent_install_keeps_escalation_signature_and_all_ack_requests() {
        let (bundle, cert) = identity(false);
        let (stream, mut peer) = tokio::io::duplex(64 * 1024);
        let device = tokio::spawn(async move {
            let request = receive(&mut peer).await;
            assert_eq!(request.len(), 2);
            assert_eq!(request["RequestType"].as_string(), Some("Escalate"));
            assert_eq!(
                request["SupervisorCertificate"].as_data().unwrap(),
                cert.to_der().unwrap()
            );
            reply(&mut peer, challenge()).await;
            let response = receive(&mut peer).await;
            assert_eq!(response.len(), 2);
            assert_eq!(
                response["RequestType"].as_string(),
                Some("EscalateResponse")
            );
            let signed = Pkcs7::from_der(response["SignedRequest"].as_data().unwrap()).unwrap();
            let mut certs = Stack::new().unwrap();
            certs.push(cert).unwrap();
            let store = X509StoreBuilder::new().unwrap().build();
            let mut plaintext = Vec::new();
            signed
                .verify(
                    &certs,
                    &store,
                    None,
                    Some(&mut plaintext),
                    Pkcs7Flags::NOVERIFY | Pkcs7Flags::NOINTERN,
                )
                .unwrap();
            assert_eq!(plaintext, b"owned-challenge\0\xff\r\n");
            reply(&mut peer, ack()).await;
            let proceed = receive(&mut peer).await;
            assert_eq!(proceed.len(), 1);
            assert_eq!(
                proceed["RequestType"].as_string(),
                Some("ProceedWithKeybagMigration")
            );
            reply(&mut peer, ack()).await;
            let install = receive(&mut peer).await;
            assert_eq!(install.len(), 2);
            assert_eq!(
                install["RequestType"].as_string(),
                Some("InstallProfileSilent")
            );
            assert_eq!(
                install["Payload"].as_data(),
                Some(b"owned-payload".as_slice())
            );
            reply(&mut peer, ack()).await;
            peer.shutdown().await.unwrap();
        });
        tokio::time::timeout(Duration::from_secs(15), async {
            let mut client = crate::mcinstall::McInstallClient::new(stream);
            client
                .install_profile_silent(b"owned-payload", &bundle, "owned-password")
                .await
                .unwrap();
            device.await.unwrap();
            drop(client);
        })
        .await
        .expect("bounded owned MCInstall exchange");
    }
}
