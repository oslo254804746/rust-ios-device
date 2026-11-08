//! Lockdown credentials, port dialing and CoreDeviceProxy startup.

use super::activation_policy::ActivationPolicy;
#[cfg(feature = "tunnel")]
use super::tunnel_activation::TunnelConnection;
use super::{
    ConnectOptions, ConnectedDevice, InitialRsdProbe, InitialRsdProbeSkipReason, LockdownTransport,
    ServiceStream,
};
use crate::discovery::DeviceInfo;
use crate::error::CoreError;
use crate::lockdown::pair_record::PairRecord;
#[cfg(feature = "tunnel")]
use crate::lockdown::session::{
    start_lockdown_session, start_service, wrap_service_tls, CORE_DEVICE_PROXY,
};
#[cfg(feature = "tunnel")]
use crate::lockdown::LOCKDOWN_PORT;
use crate::mux::MuxClient;
use std::sync::Arc;
#[cfg(feature = "tunnel")]
use std::{
    pin::Pin,
    task::{Context, Poll},
};
#[cfg(feature = "tunnel")]
use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};
use tokio::net::TcpStream;

pub(super) fn load_pair_record(
    udid: &str,
    pair_record_path: Option<&std::path::Path>,
) -> Result<Arc<PairRecord>, CoreError> {
    Ok(Arc::new(if let Some(path) = pair_record_path {
        PairRecord::load_from_path(path, udid)?
    } else {
        PairRecord::load(udid)?
    }))
}

pub(super) fn try_load_pair_record(
    udid: &str,
    pair_record_path: Option<&std::path::Path>,
) -> Option<Arc<PairRecord>> {
    load_pair_record(udid, pair_record_path).ok()
}

pub(super) fn require_pair_record(
    pair_record: Option<Arc<PairRecord>>,
    udid: &str,
    context: &str,
) -> Result<Arc<PairRecord>, CoreError> {
    pair_record.ok_or_else(|| {
        CoreError::Unsupported(format!(
            "{context} a lockdown pair record for device {}",
            crate::redaction::diagnostic_id(udid)
        ))
    })
}

pub(super) async fn connect_lockdown_port(
    udid: &str,
    transport: &LockdownTransport,
    port: u16,
    read_pair_record: bool,
) -> Result<ServiceStream, CoreError> {
    match transport {
        LockdownTransport::Usbmux { device_id } => {
            let mut mux = MuxClient::connect().await?;
            if read_pair_record {
                mux.read_pair_record(udid).await?;
            }
            let stream = mux.connect_to_port(*device_id, port).await?;
            Ok(Box::new(stream))
        }
        LockdownTransport::Tcp { host, .. } => {
            let stream = tokio::time::timeout(
                crate::tunnel::TUNNEL_CONNECT_TIMEOUT,
                TcpStream::connect((host.as_str(), port)),
            )
            .await
            .map_err(|source| {
                crate::error::context::deadline_io(
                    "lockdown TCP dial",
                    crate::tunnel::TUNNEL_CONNECT_TIMEOUT,
                    source,
                )
            })??;
            Ok(Box::new(stream))
        }
    }
}

static TCP_HEARTBEATS: std::sync::OnceLock<
    std::sync::Mutex<std::collections::HashMap<(String, String), tokio::task::AbortHandle>>,
> = std::sync::OnceLock::new();

pub(super) async fn ensure_tcp_heartbeat(
    udid: &str,
    transport: &LockdownTransport,
    pair_record: &Arc<PairRecord>,
) -> Result<(), CoreError> {
    let LockdownTransport::Tcp { host } = transport else {
        return Ok(());
    };
    if host.is_empty() || host == "127.0.0.1" || host == "::1" {
        return Ok(());
    }
    let key = (udid.to_string(), host.clone());
    let map =
        TCP_HEARTBEATS.get_or_init(|| std::sync::Mutex::new(std::collections::HashMap::new()));
    if let Ok(guard) = map.lock() {
        if let Some(handle) = guard.get(&key) {
            if !handle.is_finished() {
                return Ok(());
            }
        }
    }

    let lockdown_stream =
        connect_lockdown_port(udid, transport, crate::lockdown::LOCKDOWN_PORT, true).await?;
    let (_session_id, mut tls_reader, mut tls_writer) =
        crate::lockdown::session::start_lockdown_session(lockdown_stream, pair_record).await?;
    let (hb_port, enable_ssl) = crate::lockdown::session::start_service(
        &mut tls_reader,
        &mut tls_writer,
        "com.apple.mobile.heartbeat",
    )
    .await?;
    let raw_hb = connect_lockdown_port(udid, transport, hb_port, false).await?;
    let mut hb_stream: ServiceStream = if enable_ssl {
        Box::new(crate::lockdown::session::wrap_service_tls(raw_hb, pair_record).await?)
    } else {
        raw_hb
    };

    #[derive(serde::Serialize)]
    #[serde(rename_all = "PascalCase")]
    struct Polo {
        command: &'static str,
    }

    let _marco: plist::Value = tokio::time::timeout(
        std::time::Duration::from_secs(5),
        crate::lockdown::protocol::recv_lockdown(&mut hb_stream),
    )
    .await
    .map_err(|source| {
        crate::error::context::deadline_io(
            "heartbeat Marco recv",
            std::time::Duration::from_secs(5),
            source,
        )
    })??;
    crate::lockdown::protocol::send_lockdown(&mut hb_stream, &Polo { command: "Polo" }).await?;

    let join = tokio::spawn(async move {
        while crate::lockdown::protocol::recv_lockdown::<_, plist::Value>(&mut hb_stream)
            .await
            .is_ok()
        {
            if crate::lockdown::protocol::send_lockdown(&mut hb_stream, &Polo { command: "Polo" })
                .await
                .is_err()
            {
                break;
            }
        }
    });
    if let Ok(mut guard) = map.lock() {
        if let Some(old) = guard.insert(key, join.abort_handle()) {
            old.abort();
        }
    }
    Ok(())
}

pub(super) async fn connect_via_lockdown_transport<P: ActivationPolicy>(
    info: DeviceInfo,
    pair_record: Arc<PairRecord>,
    lockdown_transport: LockdownTransport,
    opts: ConnectOptions,
    owner: super::tunnel_owner::InitialOwner<'_, P>,
) -> Result<P::Output, CoreError> {
    let initial_rsd_bank = owner.bank;
    if opts.skip_tunnel {
        return owner.policy.without_tunnel(ConnectedDevice {
            info,
            tunnel: None,
            rsd: None,
            #[cfg(feature = "tunnel")]
            owned_initial_rsd: None,
            #[cfg(feature = "tunnel")]
            owned_initial_bank: None,
            #[cfg(feature = "tunnel")]
            prepared_rsd: None,
            #[cfg(feature = "tunnel")]
            owned_prepared_rsd: None,
            initial_rsd_probe: InitialRsdProbe::skipped(InitialRsdProbeSkipReason::TunnelSkipped),
            pair_record: Some(pair_record),
            lockdown_transport,
        });
    }

    #[cfg(not(feature = "tunnel"))]
    {
        let _ = (
            info,
            pair_record,
            lockdown_transport,
            owner,
            initial_rsd_bank,
        );
        Err(CoreError::Unsupported(
            "CoreDevice tunnel support requires ios-core feature 'tunnel'".into(),
        ))
    }

    #[cfg(feature = "tunnel")]
    {
        let reservation = owner.reserve()?;
        let _ = ensure_tcp_heartbeat(&info.udid, &lockdown_transport, &pair_record).await;
        let lockdown_stream =
            connect_lockdown_port(&info.udid, &lockdown_transport, LOCKDOWN_PORT, true).await?;

        tracing::info!("tunnel connect: starting lockdown session");
        let (_session_id, mut tls_reader, mut tls_writer) =
            start_lockdown_session(lockdown_stream, &pair_record).await?;
        tracing::info!("tunnel connect: lockdown session established");

        tracing::info!("tunnel connect: requesting CoreDeviceProxy");
        let (service_port, enable_service_ssl) =
            start_service(&mut tls_reader, &mut tls_writer, CORE_DEVICE_PROXY).await?;
        tracing::info!(
        "tunnel connect: CoreDeviceProxy started on port {service_port} (ssl={enable_service_ssl})"
    );

        let proxy_stream_raw =
            connect_lockdown_port(&info.udid, &lockdown_transport, service_port, false).await?;

        let proxy_stream = if enable_service_ssl {
            tracing::info!("tunnel connect: wrapping CoreDeviceProxy with TLS");
            ProxyStream::Tls(Box::new(
                wrap_service_tls(proxy_stream_raw, &pair_record).await?,
            ))
        } else {
            tracing::info!("tunnel connect: CoreDeviceProxy is plaintext");
            ProxyStream::Plain(proxy_stream_raw)
        };
        tracing::info!("tunnel connect: CoreDeviceProxy stream ready");

        owner
            .policy
            .activate(
                TunnelConnection::new(
                    info,
                    Some(pair_record),
                    lockdown_transport,
                    "lockdown CoreDeviceProxy",
                )
                .with_initial_rsd_bank(initial_rsd_bank),
                proxy_stream,
                opts.tun_mode,
                reservation,
            )
            .await
    }
}

#[cfg(not(feature = "tunnel"))]
pub(super) fn tunnel_unavailable() -> CoreError {
    CoreError::Unsupported("CoreDevice tunnel support requires ios-core feature 'tunnel'".into())
}
// ── ProxyStream ───────────────────────────────────────────────────────────────

#[cfg(feature = "tunnel")]
pub(super) enum ProxyStream {
    Plain(ServiceStream),
    Tls(Box<tokio_rustls::client::TlsStream<ServiceStream>>),
}

#[cfg(feature = "tunnel")]
impl Unpin for ProxyStream {}

#[cfg(feature = "tunnel")]
impl AsyncRead for ProxyStream {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<std::io::Result<()>> {
        match &mut *self {
            ProxyStream::Plain(s) => Pin::new(s).poll_read(cx, buf),
            ProxyStream::Tls(s) => Pin::new(s).poll_read(cx, buf),
        }
    }
}

#[cfg(feature = "tunnel")]
impl AsyncWrite for ProxyStream {
    fn poll_write(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<std::io::Result<usize>> {
        match &mut *self {
            ProxyStream::Plain(s) => Pin::new(s).poll_write(cx, buf),
            ProxyStream::Tls(s) => Pin::new(s).poll_write(cx, buf),
        }
    }
    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        match &mut *self {
            ProxyStream::Plain(s) => Pin::new(s).poll_flush(cx),
            ProxyStream::Tls(s) => Pin::new(s).poll_flush(cx),
        }
    }
    fn poll_shutdown(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        match &mut *self {
            ProxyStream::Plain(s) => Pin::new(s).poll_shutdown(cx),
            ProxyStream::Tls(s) => Pin::new(s).poll_shutdown(cx),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[tokio::test]
    async fn actual_skip_tunnel_connection_reports_an_unattempted_initial_probe() {
        let credentials = Arc::new(PairRecord {
            device_certificate: vec![],
            host_certificate: vec![],
            host_private_key: zeroize::Zeroizing::new(vec![]),
            root_certificate: vec![],
            host_id: "owned-unusable-fixture".into(),
            system_buid: "owned-unusable-fixture".into(),
            wifi_mac_address: None,
        });
        let device = connect_via_lockdown_transport(
            DeviceInfo {
                udid: "synthetic-private-skipped-probe-device".into(),
                device_id: 0,
                connection_type: "Network".into(),
                product_id: 0,
            },
            credentials,
            LockdownTransport::Tcp {
                host: String::new(),
            },
            ConnectOptions {
                skip_tunnel: true,
                ..Default::default()
            },
            super::super::tunnel_owner::InitialOwner::new(None, None),
        )
        .await
        .unwrap();
        let observation = device.initial_rsd_probe();
        assert_eq!(
            observation.state(),
            super::super::InitialRsdProbeState::NotAttempted
        );
        assert_eq!(
            observation.skip_reason(),
            Some(InitialRsdProbeSkipReason::TunnelSkipped)
        );
        assert!(observation.error().is_none());
        assert!(device.rsd().is_none());
        assert!(device.tunnel_handle().is_none());
        assert!(!format!("{observation:?} {observation:#?}").contains("synthetic-private"));
        assert!(device.into_rsd().is_none());
    }

    #[tokio::test]
    async fn tcp_lockdown_dial_preserves_bytes_without_usbmux_pair_record_reads() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
            .await
            .unwrap();
        let port = listener.local_addr().unwrap().port();
        let server = async {
            for _ in 0..2 {
                let (mut stream, _) = listener.accept().await.unwrap();
                let mut request = [0; 7];
                stream.read_exact(&mut request).await.unwrap();
                assert_eq!(&request, b"request");
                stream.write_all(b"reply").await.unwrap();
            }
        };
        let client = async {
            for read_pair_record in [true, false] {
                let mut stream = connect_lockdown_port(
                    "synthetic-unpaired-tcp-device",
                    &LockdownTransport::Tcp {
                        host: "127.0.0.1".into(),
                    },
                    port,
                    read_pair_record,
                )
                .await
                .unwrap();
                stream.write_all(b"request").await.unwrap();
                let mut reply = [0; 5];
                stream.read_exact(&mut reply).await.unwrap();
                assert_eq!(&reply, b"reply");
            }
        };
        tokio::time::timeout(std::time::Duration::from_secs(2), async {
            tokio::join!(server, client);
        })
        .await
        .expect("owned TCP lockdown fixture must finish");
    }

    fn temp_test_dir(label: &str) -> PathBuf {
        let unique = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("system time should be after unix epoch")
            .as_nanos();
        std::env::temp_dir().join(format!("ios_core_device_{label}_{unique}"))
    }

    #[test]
    fn try_load_pair_record_returns_none_for_missing_pair_record() {
        let missing_dir = temp_test_dir("missing_pair_record");

        let loaded = try_load_pair_record("missing-udid", Some(&missing_dir));

        assert!(loaded.is_none());

        let _ = std::fs::remove_dir_all(missing_dir);
    }

    #[test]
    fn require_pair_record_rejects_missing_lockdown_pair_record() {
        let err = require_pair_record(None, "test-udid", "remote pairing lockdown access requires")
            .expect_err("missing pair record should fail");

        assert!(err
            .to_string()
            .contains("remote pairing lockdown access requires"));
        assert!(!err.to_string().contains("test-udid"));
        assert!(err.to_string().contains("pair record for device id:"));
        assert!(matches!(err, CoreError::Unsupported(_)));
    }
}
