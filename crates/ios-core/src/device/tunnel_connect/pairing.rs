//! Direct-RSD and remote-pairing authentication flows.

use std::{
    net::Ipv6Addr,
    pin::Pin,
    task::{Context, Poll},
};

use rand::RngCore;
use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};
use tokio::net::TcpStream;

use crate::error::CoreError;
use crate::lockdown::pairing::{build_verify_start_tlv, build_verify_step2_tlv};
use crate::proto::tlv::TlvBuffer;
use crate::xpc::XpcClient;

use super::credentials::load_remote_pairing_credentials;
use super::protocol::{
    build_direct_handshake_request, build_direct_pair_verify_failed_event, build_direct_pairing_event,
    build_remote_pairing_handshake_request, build_remote_pairing_pair_verify_failed_event,
    build_remote_pairing_pairing_event, create_direct_tcp_listener,
    create_remote_pairing_tcp_listener, extract_direct_pairing_tlv,
    extract_direct_remote_identifier, extract_remote_pairing_tlv,
};
use super::{DIRECT_PAIRING_TYPE_ERROR, DIRECT_PAIRING_TYPE_PUBLIC_KEY};

pub(super) struct GuardedTunnelStream<G> {
    stream: tokio_openssl::SslStream<TcpStream>,
    _guard: G,
}

impl<G> Unpin for GuardedTunnelStream<G> {}

impl<G> AsyncRead for GuardedTunnelStream<G> {
    fn poll_read(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<std::io::Result<()>> {
        Pin::new(&mut self.get_mut().stream).poll_read(cx, buf)
    }
}

impl<G> AsyncWrite for GuardedTunnelStream<G> {
    fn poll_write(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<std::io::Result<usize>> {
        Pin::new(&mut self.get_mut().stream).poll_write(cx, buf)
    }

    fn poll_flush(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        Pin::new(&mut self.get_mut().stream).poll_flush(cx)
    }

    fn poll_shutdown(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        Pin::new(&mut self.get_mut().stream).poll_shutdown(cx)
    }
}

pub(super) struct RemotePairingControlChannel {
    stream: TcpStream,
}

impl RemotePairingControlChannel {
    pub(super) async fn connect(host: &str, port: u16) -> Result<Self, CoreError> {
        let stream = tokio::time::timeout(
            crate::tunnel::TUNNEL_CONNECT_TIMEOUT,
            TcpStream::connect((host, port)),
        )
        .await
        .map_err(|_| {
            std::io::Error::new(
                std::io::ErrorKind::TimedOut,
                format!("remote pairing dial to {host}:{port} timed out"),
            )
        })??;
        Ok(Self {
            stream,
        })
    }

    pub(super) async fn send(&mut self, payload: &serde_json::Value) -> Result<(), CoreError> {
        use tokio::io::AsyncWriteExt;

        let body = serde_json::to_vec(payload)?;
        if body.len() > u16::MAX as usize {
            return Err(CoreError::Protocol(format!(
                "remote pairing payload too large: {} bytes",
                body.len()
            )));
        }

        self.stream.write_all(b"RPPairing").await?;
        self.stream
            .write_all(&(body.len() as u16).to_be_bytes())
            .await?;
        self.stream.write_all(&body).await?;
        self.stream.flush().await?;
        Ok(())
    }

    pub(super) async fn recv(&mut self) -> Result<serde_json::Value, CoreError> {
        use tokio::io::AsyncReadExt;

        let mut magic = [0u8; 9];
        self.stream.read_exact(&mut magic).await?;
        if &magic != b"RPPairing" {
            return Err(CoreError::Protocol(format!(
                "invalid RPPairing magic: {magic:?}"
            )));
        }

        let mut length = [0u8; 2];
        self.stream.read_exact(&mut length).await?;
        let body_len = u16::from_be_bytes(length) as usize;
        let mut body = vec![0u8; body_len];
        self.stream.read_exact(&mut body).await?;
        Ok(serde_json::from_slice(&body)?)
    }
}

pub(super) async fn establish_direct_tunnel_stream(
    rsd_addr: Ipv6Addr,
    service_port: u16,
) -> Result<GuardedTunnelStream<XpcClient>, CoreError> {
    let mut client = XpcClient::connect(rsd_addr, service_port).await?;
    let mut sequence_number = 0u64;

    client
        .send(build_direct_handshake_request(sequence_number))
        .await?;
    sequence_number += 1;

    let handshake = client.recv().await?;
    let remote_identifier =
        extract_direct_remote_identifier(handshake.body.as_ref().ok_or_else(|| {
            CoreError::Protocol("direct handshake response missing body".into())
        })?)?;

    let loaded = {
        let id = remote_identifier.clone();
        tokio::task::spawn_blocking(move || load_remote_pairing_credentials(&id))
            .await
            .map_err(|e| CoreError::Other(format!("spawn_blocking join error: {e}")))?
    }?;

    let mut our_secret = [0u8; 32];
    rand::thread_rng().fill_bytes(&mut our_secret);
    let static_secret = x25519_dalek::StaticSecret::from(our_secret);
    let our_public = x25519_dalek::PublicKey::from(&static_secret).to_bytes();

    client
        .send(build_direct_pairing_event(
            &build_verify_start_tlv(&our_public),
            "verifyManualPairing",
            true,
            None,
            sequence_number,
        ))
        .await?;
    sequence_number += 1;

    let verify_start = client.recv().await?;
    let verify_start_tlv =
        extract_direct_pairing_tlv(verify_start.body.as_ref().ok_or_else(|| {
            CoreError::Protocol("verifyManualPairing start missing body".into())
        })?)?;
    let verify_start_fields = TlvBuffer::decode(&verify_start_tlv);
    if let Some(error) = verify_start_fields.get(&DIRECT_PAIRING_TYPE_ERROR) {
        send_pair_verify_failed(&mut client, sequence_number).await?;
        return Err(CoreError::Protocol(format!(
            "verifyManualPairing start rejected: {error:?}"
        )));
    }

    let device_public: [u8; 32] = verify_start_fields
        .get(&DIRECT_PAIRING_TYPE_PUBLIC_KEY)
        .ok_or_else(|| {
            CoreError::Protocol("verifyManualPairing start missing device public key".into())
        })?
        .as_ref()
        .try_into()
        .map_err(|_| {
            CoreError::Protocol("verifyManualPairing device public key must be 32 bytes".into())
        })?;

    let verify_session = build_verify_step2_tlv(
        our_secret,
        &our_public,
        &device_public,
        &loaded.host_identity,
    )
    .map_err(|e| CoreError::Other(format!("verifyManualPairing finish build failed: {e}")))?;

    client
        .send(build_direct_pairing_event(
            &verify_session.tlv,
            "verifyManualPairing",
            false,
            None,
            sequence_number,
        ))
        .await?;
    sequence_number += 1;

    let verify_finish = client.recv().await?;
    let verify_finish_tlv =
        extract_direct_pairing_tlv(verify_finish.body.as_ref().ok_or_else(|| {
            CoreError::Protocol("verifyManualPairing finish missing body".into())
        })?)?;
    let verify_finish_fields = TlvBuffer::decode(&verify_finish_tlv);
    if let Some(error) = verify_finish_fields.get(&DIRECT_PAIRING_TYPE_ERROR) {
        send_pair_verify_failed(&mut client, sequence_number).await?;
        return Err(CoreError::Protocol(format!(
            "verifyManualPairing finish rejected: {error:?}"
        )));
    }

    let listener_port =
        create_direct_tcp_listener(&mut client, &verify_session, sequence_number).await?;
    let stream = crate::psk_tls::connect_psk_tls(
        &rsd_addr.to_string(),
        listener_port,
        &verify_session.encryption_key,
    )
    .await
    .map_err(|e| CoreError::Other(format!("direct TLS-PSK listener connect failed: {e}")))?;

    Ok(GuardedTunnelStream {
        stream,
        _guard: client,
    })
}

pub(super) async fn establish_remote_pairing_tunnel_stream(
    remote_identifier: &str,
    host: &str,
    port: u16,
) -> Result<GuardedTunnelStream<RemotePairingControlChannel>, CoreError> {
    let loaded = {
        let id = remote_identifier.to_owned();
        tokio::task::spawn_blocking(move || load_remote_pairing_credentials(&id))
            .await
            .map_err(|e| CoreError::Other(format!("spawn_blocking join error: {e}")))?
    }?;
    let mut control = RemotePairingControlChannel::connect(host, port).await?;
    let mut sequence_number = 0u64;

    control
        .send(&build_remote_pairing_handshake_request(sequence_number))
        .await?;
    sequence_number += 1;
    let _handshake = control.recv().await?;

    let mut our_secret = [0u8; 32];
    rand::thread_rng().fill_bytes(&mut our_secret);
    let static_secret = x25519_dalek::StaticSecret::from(our_secret);
    let our_public = x25519_dalek::PublicKey::from(&static_secret).to_bytes();

    control
        .send(&build_remote_pairing_pairing_event(
            &build_verify_start_tlv(&our_public),
            "verifyManualPairing",
            true,
            None,
            sequence_number,
        ))
        .await?;
    sequence_number += 1;

    let verify_start = control.recv().await?;
    let verify_start_tlv = extract_remote_pairing_tlv(&verify_start)?;
    let verify_start_fields = TlvBuffer::decode(&verify_start_tlv);
    if let Some(error) = verify_start_fields.get(&DIRECT_PAIRING_TYPE_ERROR) {
        control
            .send(&build_remote_pairing_pair_verify_failed_event(
                sequence_number,
            ))
            .await?;
        return Err(CoreError::Protocol(format!(
            "remote pairing verify start rejected: {error:?}"
        )));
    }

    let device_public: [u8; 32] = verify_start_fields
        .get(&DIRECT_PAIRING_TYPE_PUBLIC_KEY)
        .ok_or_else(|| {
            CoreError::Protocol("remote pairing verify start missing device public key".into())
        })?
        .as_ref()
        .try_into()
        .map_err(|_| {
            CoreError::Protocol("remote pairing device public key must be 32 bytes".into())
        })?;

    let verify_session = build_verify_step2_tlv(
        our_secret,
        &our_public,
        &device_public,
        &loaded.host_identity,
    )
    .map_err(|e| CoreError::Other(format!("remote pairing verify finish build failed: {e}")))?;

    control
        .send(&build_remote_pairing_pairing_event(
            &verify_session.tlv,
            "verifyManualPairing",
            false,
            None,
            sequence_number,
        ))
        .await?;
    sequence_number += 1;

    let verify_finish = control.recv().await?;
    let verify_finish_tlv = extract_remote_pairing_tlv(&verify_finish)?;
    let verify_finish_fields = TlvBuffer::decode(&verify_finish_tlv);
    if let Some(error) = verify_finish_fields.get(&DIRECT_PAIRING_TYPE_ERROR) {
        control
            .send(&build_remote_pairing_pair_verify_failed_event(
                sequence_number,
            ))
            .await?;
        return Err(CoreError::Protocol(format!(
            "remote pairing verify finish rejected: {error:?}"
        )));
    }

    let listener_port =
        create_remote_pairing_tcp_listener(&mut control, &verify_session, sequence_number).await?;
    let stream =
        crate::psk_tls::connect_psk_tls(host, listener_port, &verify_session.encryption_key)
            .await
            .map_err(|e| {
                CoreError::Other(format!(
                    "remote pairing TLS-PSK listener connect failed: {e}"
                ))
            })?;

    Ok(GuardedTunnelStream {
        stream,
        _guard: control,
    })
}

pub(super) async fn send_pair_verify_failed(
    client: &mut XpcClient,
    sequence_number: u64,
) -> Result<(), CoreError> {
    client
        .send(build_direct_pair_verify_failed_event(sequence_number))
        .await
        .map_err(CoreError::from)
}
