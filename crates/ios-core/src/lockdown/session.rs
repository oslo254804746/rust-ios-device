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
