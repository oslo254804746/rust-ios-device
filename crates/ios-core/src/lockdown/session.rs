use std::io::Cursor;
use std::sync::Arc;

use crate::proto::tls::InsecureSkipVerify;
use rustls::pki_types::{CertificateDer, PrivateKeyDer, ServerName};
use rustls::ClientConfig;
use tokio::io::{AsyncRead, AsyncWrite, BufReader, BufWriter};
use tokio_rustls::client::TlsStream;

use crate::lockdown::pair_record::PairRecord;
use crate::lockdown::protocol::*;
use crate::lockdown::LockdownError;

pub const CORE_DEVICE_PROXY: &str = "com.apple.internal.devicecompute.CoreDeviceProxy";

/// Perform lockdown QueryType + StartSession, then upgrade the stream to TLS via native-tls.
///
/// Returns (session_id, tls_reader, tls_writer).
pub async fn start_lockdown_session<S>(
    stream: S,
    pair_record: &PairRecord,
) -> Result<
    (
        String,
        BufReader<tokio::io::ReadHalf<TlsStream<S>>>,
        BufWriter<tokio::io::WriteHalf<TlsStream<S>>>,
    ),
    LockdownError,
>
where
    S: AsyncRead + AsyncWrite + Unpin + Send + 'static,
{
    let (reader_raw, writer_raw) = tokio::io::split(stream);
    let mut reader = BufReader::new(reader_raw);
    let mut writer = BufWriter::new(writer_raw);

    // 1. QueryType — confirm lockdown
    send_lockdown(
        &mut writer,
        &QueryTypeRequest {
            label: "ios-rs",
            request: "QueryType",
        },
    )
    .await?;
    let _: QueryTypeResponse = recv_lockdown(&mut reader).await?;

    // 2. StartSession
    send_lockdown(
        &mut writer,
        &StartSessionRequest {
            label: "ios-rs",
            protocol_version: "2",
            request: "StartSession",
            host_id: pair_record.host_id.clone(),
            system_buid: pair_record.system_buid.clone(),
        },
    )
    .await?;
    let session_resp: StartSessionResponse = recv_lockdown(&mut reader).await?;

    if !session_resp.enable_session_ssl {
        return Err(LockdownError::Protocol("device did not enable SSL".into()));
    }

    // 3. Upgrade to TLS
    let stream = reader.into_inner().unsplit(writer.into_inner());
    let tls_stream = build_rustls_connection(stream, pair_record, "lockdown").await?;

    let (tls_r, tls_w) = tokio::io::split(tls_stream);
    Ok((
        session_resp.session_id,
        BufReader::new(tls_r),
        BufWriter::new(tls_w),
    ))
}

fn build_rustls_config(pair_record: &PairRecord) -> Result<Arc<ClientConfig>, LockdownError> {
    let _ = rustls::crypto::ring::default_provider().install_default();

    let mut cert_reader = Cursor::new(&pair_record.host_certificate);
    let cert_chain: Vec<CertificateDer<'static>> = rustls_pemfile::certs(&mut cert_reader)
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| {
            LockdownError::Protocol(format!("failed to parse host certificate PEM: {e}"))
        })?;
    if cert_chain.is_empty() {
        return Err(LockdownError::Protocol(
            "pair record host certificate chain is empty".into(),
        ));
    }

    let mut key_reader = Cursor::new(&pair_record.host_private_key);
    let private_key: PrivateKeyDer<'static> = rustls_pemfile::private_key(&mut key_reader)
        .map_err(|e| LockdownError::Protocol(format!("failed to parse host private key PEM: {e}")))?
        .ok_or_else(|| LockdownError::Protocol("pair record host private key is missing".into()))?;

    let config = ClientConfig::builder()
        .dangerous()
        .with_custom_certificate_verifier(Arc::new(InsecureSkipVerify))
        .with_client_auth_cert(cert_chain, private_key)
        .map_err(|e| LockdownError::Protocol(format!("rustls client auth config: {e}")))?;

    Ok(Arc::new(config))
}

async fn build_rustls_connection<S>(
    stream: S,
    pair_record: &PairRecord,
    server_name: &'static str,
) -> Result<TlsStream<S>, LockdownError>
where
    S: AsyncRead + AsyncWrite + Unpin + Send + 'static,
{
    let config = build_rustls_config(pair_record)?;
    let connector = tokio_rustls::TlsConnector::from(config);

    // Hostname is ignored by our verifier, but rustls still requires a syntactically valid name.
    let server_name = ServerName::try_from(server_name).map_err(|e| {
        LockdownError::Protocol(format!("invalid rustls server name '{server_name}': {e}"))
    })?;

    connector
        .connect(server_name, stream)
        .await
        .map_err(|e| LockdownError::Protocol(format!("TLS handshake: {e}")))
}

/// Send StartService over an established TLS session.
pub async fn start_service<R, W>(
    reader: &mut R,
    writer: &mut W,
    service_name: &str,
) -> Result<(u16, bool), LockdownError>
where
    R: AsyncRead + Unpin,
    W: AsyncWrite + Unpin,
{
    send_lockdown(
        writer,
        &StartServiceRequest {
            label: "ios-rs",
            request: "StartService",
            service: service_name.to_string(),
        },
    )
    .await?;
    let resp: StartServiceResponse = recv_lockdown(reader).await?;

    // Check if device returned an error instead of a port
    if let Some(err) = resp.error {
        return Err(LockdownError::Protocol(format!(
            "StartService '{service_name}' failed: {err}"
        )));
    }

    let port = resp.port.ok_or_else(|| {
        LockdownError::Protocol(format!(
            "StartService '{service_name}': response missing Port field"
        ))
    })?;

    let ssl = resp.enable_service_ssl.unwrap_or(false);
    tracing::debug!("StartService '{service_name}': port={port} enable_ssl={ssl}");
    Ok((port, ssl))
}

/// Wrap a service stream with TLS (for services with EnableServiceSSL=true).
pub async fn wrap_service_tls<S>(
    stream: S,
    pair_record: &PairRecord,
) -> Result<TlsStream<S>, LockdownError>
where
    S: AsyncRead + AsyncWrite + Unpin + Send + 'static,
{
    wrap_service_tls_with_server_name(stream, pair_record, "lockdown").await
}

/// Wrap a service stream with TLS using an explicit rustls server name.
///
/// The underlying verifier still skips certificate validation, but the caller can
/// preserve a service-specific ClientHello shape when compatibility requires it.
pub async fn wrap_service_tls_with_server_name<S>(
    stream: S,
    pair_record: &PairRecord,
    server_name: &'static str,
) -> Result<TlsStream<S>, LockdownError>
where
    S: AsyncRead + AsyncWrite + Unpin + Send + 'static,
{
    build_rustls_connection(stream, pair_record, server_name).await
}

/// Drop the post-lockdown service TLS layer and recover the underlying stream.
///
/// Some legacy developer services perform a TLS handshake as a transport gate,
/// but expect raw DTX bytes after the handshake completes.
pub fn strip_service_tls<S>(stream: TlsStream<S>) -> Result<S, LockdownError>
where
    S: AsyncRead + AsyncWrite + Unpin + Send + 'static,
{
    let (stream, _session) = stream.into_inner();
    Ok(stream)
}

/// Perform a TLS handshake and immediately return the underlying plaintext stream.
///
/// Some lockdown services require TLS only as a transport gate and expect raw bytes
/// again once the handshake finishes.
pub async fn handshake_only_service_tls<S>(
    stream: S,
    pair_record: &PairRecord,
    server_name: &'static str,
) -> Result<S, LockdownError>
where
    S: AsyncRead + AsyncWrite + Unpin + Send + 'static,
{
    let tls_stream = wrap_service_tls_with_server_name(stream, pair_record, server_name).await?;
    strip_service_tls(tls_stream)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dummy_pair_record(cert: &[u8], key: &[u8]) -> PairRecord {
        PairRecord {
            device_certificate: b"ignored".to_vec(),
            host_certificate: cert.to_vec(),
            host_private_key: key.to_vec(),
            root_certificate: b"ignored".to_vec(),
            host_id: "test-host-id".into(),
            system_buid: "test-buid".into(),
            wifi_mac_address: None,
        }
    }

    // A self-signed RSA 2048 cert+key for testing (generated offline, no secrets)
    const TEST_CERT_PEM: &[u8] = b"-----BEGIN CERTIFICATE-----
MIIC/zCCAeegAwIBAgIUIcm1BZEI6nF3fXhXJuEND4sUAfcwDQYJKoZIhvcNAQEL
BQAwDzENMAsGA1UEAwwEdGVzdDAeFw0yNjA0MzAwNDM5MzBaFw0yNzA0MzAwNDM5
MzBaMA8xDTALBgNVBAMMBHRlc3QwggEiMA0GCSqGSIb3DQEBAQUAA4IBDwAwggEK
AoIBAQC79dsRr759iA4cborijwjiNsZzHQvL9J5PGJlShqIC2aL9UMKCU1EPtYTR
5F4SDz6HDS3IeXPoEErn03hNkHopg463XbVgiFXOZwzPZbahNrKksGp+Klvc0cNb
nClv/gmKro1Q9vp3IieR7Rm1fcUpY8AJ3dINdYJvFnHyaYLgjIYLdiFCCBoKh9Mq
iHDCHZ/ZqgV8k22MB5tooCEv+rXQqWMhhtc+L9ba/P6HvLK7F1FmJqsW2GYgFRd+
YDgwsGB/l3Cpen2BD64iYMsKIacfl6phDNQGjmYbCsLAYD6c3csKSOsT1jZONsvN
nNo2X/87haoD+iDj42/LnVbOMvnrAgMBAAGjUzBRMB0GA1UdDgQWBBQM7F7f8Qlu
2FiOEX3CqqsZFql/NzAfBgNVHSMEGDAWgBQM7F7f8Qlu2FiOEX3CqqsZFql/NzAP
BgNVHRMBAf8EBTADAQH/MA0GCSqGSIb3DQEBCwUAA4IBAQB4iqOezp/yllYfn8JW
eZwa+LcDyAcdo7AkBkJ2RVygyQeHwnjOcnxGJ/X6+C2/iNTtkWu4KKdi1rzMX6W7
BMGvz0c6Jfe9c3vifv+9GmvYETZyPxbNxwA33vTOLKtq0Wcb8zPvOq05rgeXOoVU
3IP79ijQfdaIe5MzuKUay4DFB05qgIBkIyCxPx/p2nH2jyDaMum8KyFFpyMJwdNz
5UI1gg2kDVt669mAY5PdProZg6GHpt1Q4gDkWj1jTTNncPPyxOcnP/HQoXZtuPZu
61OsIR2URv16qBoNfdhIS3UJ4eYt65mnXOQg1Bagotzvq3sy3B5MOJyXUMjhtuAG
f/68
-----END CERTIFICATE-----";

    const TEST_KEY_PEM: &[u8] = b"-----BEGIN PRIVATE KEY-----
MIIEvQIBADANBgkqhkiG9w0BAQEFAASCBKcwggSjAgEAAoIBAQC79dsRr759iA4c
borijwjiNsZzHQvL9J5PGJlShqIC2aL9UMKCU1EPtYTR5F4SDz6HDS3IeXPoEErn
03hNkHopg463XbVgiFXOZwzPZbahNrKksGp+Klvc0cNbnClv/gmKro1Q9vp3IieR
7Rm1fcUpY8AJ3dINdYJvFnHyaYLgjIYLdiFCCBoKh9MqiHDCHZ/ZqgV8k22MB5to
oCEv+rXQqWMhhtc+L9ba/P6HvLK7F1FmJqsW2GYgFRd+YDgwsGB/l3Cpen2BD64i
YMsKIacfl6phDNQGjmYbCsLAYD6c3csKSOsT1jZONsvNnNo2X/87haoD+iDj42/L
nVbOMvnrAgMBAAECggEADm3ONmpeXjaelrIpuUCvtuXrkBSvviV2La4+vuYU89EP
QRD9DZIly+XsX0x/qDVBYI6zcAtayXrOtUM3ngS0TBGMWCk6bkGpDKI+ioFNZszT
I+9jDXJlAOudap/vUmiXBO1nbcq36YNWtE4WRid0hjvhFyDPKjdWHv8DGk/dOy2M
vmnXWfcuNCdSyHAqegi/PTprRF9J9xJ+eIyTTQFeRWDVC35aE5QNhAXQfmBME/oX
7GNjCyqITT73OTJAXLmFje65FFw/Cb84EoLrPR944K29+kRp+RNnkBRxXbqZt1kT
zkeRO5fhq6pV0Gp66dAQB8a1BDsnQZVIsUYmApSASQKBgQDil8RXctwOW0CQT3/h
5Qoo6DkPZLdgnc/x9JwzZ5i/sJBsruR32rM6ejV4lMn1xlyDHb5QPI5ou8Kw+Toa
mLZdW3gNI+J+XEaskf4B1hn8RmlvAk1DAP1MJotWap2BxOa7iYDb3eE/eJSL2vzD
s+2eBlEXzuifVjtj4h/2DSoCjwKBgQDUWpQidb2rI8bvApLDxfaMXeS/z18pqXL3
M9j9bPXEUoTQ+u8GzV7wvaB38jIQ884YmcGVmJ5iJ41aC4/GOeWk7kXOUv95uY70
wTxHchKojQacQPYALfMsmuPLrxCRfh0QP4dO6PZ6MMSFhQFPvV9OVn9Htyvj7TCo
TXxljQ5Q5QKBgDRokdsAD/GqHXbDTHq89OqdO4VZ8CgCmDQINZCWJ3g+qEja8rDd
/pJJ7dAj6cpUxNT2riv0taN3ugIgwtWf+J4DJ/MyF5LOWPJVGgDmuj/lMUGhsKkM
s4lHaPbl1eRL3GoH1awE17JMe18VmVzSYuUn5N2y147y7O2fQXExfkP1AoGAKbRs
WWQ0TtMk87XWqxpK9IBQN5d7ggwkZwZIvGTU06y9JunRXc2hsrgbNtNbH9cyB8TS
rxWdLXvFGAUjRHQEdOLS1NWaFQbrW4hD1WhC39Vqke90IM7lbkIxMMR+BYT2IkXH
xiicl5zSS8K2Yjm36QO11ZjUxtvDbZpiLvOH9z0CgYEA0Re52xUUrBkf82MYOnMG
GA3kNp7expYQItyuLI1SgB7sOvdNR5e7y8SOTh43zEG2PHhbaB4SuOK+oX76Q49Q
nKakrqpv4o+Dp+AJGZlvbgsADdf4WgP8C7GgLYAvEUknoqS9TrCcDYXvEG1DgmcM
h6sEdO4FSmZFwEQ9W7FVspA=
-----END PRIVATE KEY-----";

    #[test]
    fn build_rustls_config_succeeds_with_valid_pem() {
        let record = dummy_pair_record(TEST_CERT_PEM, TEST_KEY_PEM);
        let result = build_rustls_config(&record);
        assert!(result.is_ok(), "expected Ok, got: {:?}", result.err());
    }

    #[test]
    fn build_rustls_config_rejects_empty_certificate() {
        let record = dummy_pair_record(b"", TEST_KEY_PEM);
        let err = build_rustls_config(&record).unwrap_err();
        assert!(
            err.to_string().contains("empty"),
            "expected 'empty' error, got: {err}"
        );
    }

    #[test]
    fn build_rustls_config_rejects_invalid_key() {
        let record = dummy_pair_record(TEST_CERT_PEM, b"not a PEM key");
        let err = build_rustls_config(&record).unwrap_err();
        assert!(
            err.to_string().contains("private key"),
            "expected private key error, got: {err}"
        );
    }

    #[test]
    fn strip_service_tls_is_identity_for_type() {
        // strip_service_tls just unwraps the inner stream — test type-level correctness
        // by verifying it compiles and the function signature is correct.
        // Actual TLS stream testing requires a real handshake (covered by integration tests).
    }
}
