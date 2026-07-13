//! Protocol-independent CoreDevice tunnel activation.
//!
//! Each connection strategy is responsible for authenticating its control
//! channel.  Once it yields the encrypted CDTunnel byte stream, the remaining
//! lifecycle is identical: negotiate tunnel parameters, start the selected TUN
//! implementation, probe RSD, and construct a [`ConnectedDevice`].

#[cfg(feature = "tunnel")]
use std::sync::Arc;

#[cfg(feature = "tunnel")]
use tokio::io::{AsyncRead, AsyncWrite};

#[cfg(feature = "tunnel")]
use super::{
    attempt_rsd, attempt_rsd_via_proxy, ConnectedDevice, CoreError, DeviceInfo, LockdownTransport,
    PairRecord, TUNNEL_HANDSHAKE_TIMEOUT,
};

#[cfg(feature = "tunnel-kernel")]
use crate::tunnel::forward::forward_packets;
#[cfg(feature = "tunnel-kernel")]
use crate::tunnel::tun::kernel::KernelTunDevice;
#[cfg(feature = "tunnel-userspace")]
use crate::tunnel::tun::userspace::UserspaceTunDevice;
#[cfg(feature = "tunnel")]
use crate::tunnel::{TunMode, TunnelHandle};

/// An authenticated CDTunnel byte stream.
///
/// This deliberately describes the capability needed after protocol-specific
/// pairing, rather than the discovery or pairing protocol that produced it.
/// It lets lockdown, direct RSD, and remote-pairing connections share the
/// tunnel lifecycle and makes additional connection strategies testable with
/// an in-memory async stream.
#[cfg(feature = "tunnel")]
pub(super) trait TunnelTransport: AsyncRead + AsyncWrite + Unpin + Send + 'static {}

#[cfg(feature = "tunnel")]
impl<T> TunnelTransport for T where T: AsyncRead + AsyncWrite + Unpin + Send + 'static {}

/// Context that remains stable across tunnel implementations.
#[cfg(feature = "tunnel")]
pub(super) struct TunnelConnection {
    info: DeviceInfo,
    pair_record: Option<Arc<PairRecord>>,
    lockdown_transport: LockdownTransport,
    strategy: &'static str,
}

#[cfg(feature = "tunnel")]
impl TunnelConnection {
    pub(super) fn new(
        info: DeviceInfo,
        pair_record: Option<Arc<PairRecord>>,
        lockdown_transport: LockdownTransport,
        strategy: &'static str,
    ) -> Self {
        Self {
            info,
            pair_record,
            lockdown_transport,
            strategy,
        }
    }

    fn into_device(
        self,
        handle: TunnelHandle,
        rsd: Option<crate::xpc::rsd::RsdHandshake>,
    ) -> ConnectedDevice {
        ConnectedDevice {
            info: self.info,
            tunnel: Some(Arc::new(handle)),
            rsd,
            pair_record: self.pair_record,
            lockdown_transport: self.lockdown_transport,
        }
    }
}

/// Activate an authenticated tunnel stream using the requested TUN mode.
///
/// The stream may come from any connection protocol.  Keeping this generic is
/// important: the forwarding task retains the exact stream type, while callers
/// do not need to box, downcast, or duplicate the lifecycle implementation.
#[cfg(feature = "tunnel")]
pub(super) async fn activate_tunnel<T>(
    connection: TunnelConnection,
    mut stream: T,
    tun_mode: TunMode,
) -> Result<ConnectedDevice, CoreError>
where
    T: TunnelTransport,
{
    tracing::info!(
        strategy = connection.strategy,
        timeout_ms = TUNNEL_HANDSHAKE_TIMEOUT.as_millis(),
        "tunnel connect: exchanging CDTunnel parameters"
    );
    let tunnel_info = crate::tunnel::handshake::exchange_tunnel_parameters_with_timeout(
        &mut stream,
        TUNNEL_HANDSHAKE_TIMEOUT,
    )
    .await
    .map_err(CoreError::Tunnel)?;
    tracing::info!(
        strategy = connection.strategy,
        server = %tunnel_info.server_address,
        rsd_port = tunnel_info.server_rsd_port,
        client = %tunnel_info.client_address,
        mtu = tunnel_info.client_mtu,
        "tunnel connect: CDTunnel parameters received"
    );

    match tun_mode {
        TunMode::Kernel => {
            #[cfg(not(feature = "tunnel-kernel"))]
            {
                let _ = (connection, stream, tunnel_info);
                Err(CoreError::Unsupported(
                    "kernel TUN support requires ios-core feature 'tunnel-kernel'".into(),
                ))
            }
            #[cfg(feature = "tunnel-kernel")]
            {
                let (handle, cancel_rx) =
                    TunnelHandle::new(connection.info.udid.clone(), tunnel_info.clone(), None);
                let tun =
                    KernelTunDevice::create(&tunnel_info.client_address, tunnel_info.client_mtu)
                        .await
                        .map_err(CoreError::Tunnel)?;
                let mtu = tunnel_info.client_mtu;
                let strategy = connection.strategy;
                tokio::spawn(async move {
                    if let Err(err) = forward_packets(stream, tun, mtu, cancel_rx).await {
                        tracing::error!(strategy, "kernel TUN forward failed: {err}");
                    }
                });
                let rsd =
                    attempt_rsd(&tunnel_info.server_address, tunnel_info.server_rsd_port).await;
                Ok(connection.into_device(handle, rsd))
            }
        }
        TunMode::Userspace => {
            #[cfg(not(feature = "tunnel-userspace"))]
            {
                let _ = (connection, stream, tunnel_info);
                Err(CoreError::Unsupported(
                    "userspace tunnel support requires ios-core feature 'tunnel-userspace'".into(),
                ))
            }
            #[cfg(feature = "tunnel-userspace")]
            {
                let userspace = UserspaceTunDevice::start(
                    &tunnel_info.client_address,
                    &tunnel_info.server_address,
                    tunnel_info.client_mtu,
                    stream,
                )
                .await
                .map_err(CoreError::Tunnel)?;

                let proxy_port = userspace.local_port;
                let handle = TunnelHandle::new_userspace(
                    connection.info.udid.clone(),
                    tunnel_info.clone(),
                    userspace,
                );
                let rsd = attempt_rsd_via_proxy(
                    proxy_port,
                    &tunnel_info.server_address,
                    tunnel_info.server_rsd_port,
                )
                .await;
                Ok(connection.into_device(handle, rsd))
            }
        }
    }
}
