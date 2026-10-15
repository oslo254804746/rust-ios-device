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

#[cfg(feature = "tunnel-userspace")]
use super::rsd_connect::attempt_rsd_via_proxy_with_bank;
#[cfg(feature = "tunnel-kernel")]
use super::rsd_connect::attempt_rsd_with_bank;
#[cfg(feature = "tunnel")]
use super::{ConnectedDevice, LockdownTransport};
use crate::discovery::DeviceInfo;
use crate::error::CoreError;
use crate::lockdown::pair_record::PairRecord;

const TUNNEL_HANDSHAKE_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(5);

#[cfg(all(test, feature = "tunnel-userspace"))]
mod ownership_tests;

#[cfg(feature = "tunnel-kernel")]
use crate::tunnel::forward::forward_packets_inner_owned;
#[cfg(feature = "tunnel-kernel")]
use crate::tunnel::tun::kernel::KernelTunDevice;
#[cfg(feature = "tunnel-userspace")]
use crate::tunnel::tun::userspace::UserspaceTunDevice;
#[cfg(feature = "tunnel")]
use crate::tunnel::{TunMode, TunnelHandle, ValidatedMtu};

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
        handle: impl Into<Arc<TunnelHandle>>,
        probe: super::rsd_observation::RsdProbeOutcome,
    ) -> ConnectedDevice {
        let (rsd, initial_rsd_probe) = probe.into_parts();
        ConnectedDevice {
            info: self.info,
            tunnel: Some(handle.into()),
            rsd,
            #[cfg(feature = "tunnel")]
            prepared_rsd: None,
            initial_rsd_probe,
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
    reservation: Option<Box<dyn super::TunnelReservation + '_>>,
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
    let mtu = ValidatedMtu::new(tunnel_info.client_mtu).map_err(CoreError::Tunnel)?;
    tracing::info!(
        strategy = connection.strategy,
        server = %crate::redaction::diagnostic_id(&tunnel_info.server_address),
        rsd_port = tunnel_info.server_rsd_port,
        client = %crate::redaction::diagnostic_id(&tunnel_info.client_address),
        mtu = mtu.as_u16(),
        "tunnel connect: CDTunnel parameters received"
    );

    match tun_mode {
        TunMode::Kernel => {
            #[cfg(not(feature = "tunnel-kernel"))]
            {
                let _ = (connection, stream, tunnel_info, reservation);
                Err(CoreError::Unsupported(
                    "kernel TUN support requires ios-core feature 'tunnel-kernel'".into(),
                ))
            }
            #[cfg(feature = "tunnel-kernel")]
            {
                // attach may consume the owner's reservation. Capture the
                // original bank before starting or attaching native workers.
                let byte_bank = reservation.as_ref().and_then(|owner| owner.byte_bank());
                let tun = KernelTunDevice::create_validated(&tunnel_info.client_address, mtu)
                    .await
                    .map_err(CoreError::Tunnel)?;
                let strategy = connection.strategy;
                // The synchronous factory attaches the typed parent JoinHandle
                // before the following probe can suspend or be cancelled.
                let handle = TunnelHandle::new_owned_kernel(
                    connection.info.udid.clone(),
                    tunnel_info.clone(),
                    move |cancel_rx, outcomes, reader| {
                        tokio::spawn(async move {
                            let result = forward_packets_inner_owned(
                                stream, tun, mtu, cancel_rx, outcomes, reader,
                            )
                            .await;
                            if let Err(err) = &result {
                                tracing::error!(strategy, error = %crate::redaction::diagnostic_error(err), "kernel TUN forward failed");
                            }
                            result
                        })
                    },
                );
                let handle = Arc::new(handle);
                let mut reservation = reservation;
                if let Some(owner) = reservation.as_mut() {
                    if let Err(error) = owner.attach(Arc::clone(&handle)) {
                        handle.request_stop();
                        return Err(error);
                    }
                }
                let rsd = attempt_rsd_with_bank(
                    &tunnel_info.server_address,
                    tunnel_info.server_rsd_port,
                    byte_bank.as_ref(),
                )
                .await;
                if let Some(owner) = reservation.as_mut() {
                    owner.complete();
                }
                Ok(connection.into_device(handle, rsd))
            }
        }
        TunMode::Userspace => {
            #[cfg(not(feature = "tunnel-userspace"))]
            {
                let _ = (connection, stream, tunnel_info, reservation);
                Err(CoreError::Unsupported(
                    "userspace tunnel support requires ios-core feature 'tunnel-userspace'".into(),
                ))
            }
            #[cfg(feature = "tunnel-userspace")]
            {
                let byte_bank = reservation.as_ref().and_then(|owner| owner.byte_bank());
                let userspace = UserspaceTunDevice::start_validated_with_bank(
                    &tunnel_info.client_address,
                    &tunnel_info.server_address,
                    mtu,
                    stream,
                    byte_bank.clone(),
                )
                .await
                .map_err(CoreError::Tunnel)?;

                let proxy_port = userspace.local_port;
                let handle = TunnelHandle::new_userspace(
                    connection.info.udid.clone(),
                    tunnel_info.clone(),
                    userspace,
                );
                let handle = Arc::new(handle);
                let mut reservation = reservation;
                if let Some(owner) = reservation.as_mut() {
                    if let Err(error) = owner.attach(Arc::clone(&handle)) {
                        handle.request_stop();
                        return Err(error);
                    }
                }
                let rsd = attempt_rsd_via_proxy_with_bank(
                    proxy_port,
                    &tunnel_info.server_address,
                    tunnel_info.server_rsd_port,
                    byte_bank.as_ref(),
                )
                .await;
                if let Some(owner) = reservation.as_mut() {
                    owner.complete();
                }
                Ok(connection.into_device(handle, rsd))
            }
        }
    }
}

#[cfg(test)]
mod observation_tests {
    use super::*;
    use crate::{InitialRsdProbeState, RsdHandshake};

    #[test]
    fn established_device_keeps_initial_failure_and_success_with_legacy_cache() {
        let outcomes = [
            super::super::rsd_observation::RsdProbeOutcome::Completed(Err(CoreError::Io(
                std::io::Error::new(
                    std::io::ErrorKind::TimedOut,
                    "synthetic-private-probe-failure",
                ),
            ))),
            super::super::rsd_observation::RsdProbeOutcome::Completed(Ok(RsdHandshake {
                udid: "synthetic-initial-directory".into(),
                services: std::collections::HashMap::new(),
            })),
        ];
        for (probe, expected) in outcomes.into_iter().zip([
            InitialRsdProbeState::Failed,
            InitialRsdProbeState::Available,
        ]) {
            let info = DeviceInfo {
                udid: "synthetic-private-owned-tunnel".into(),
                device_id: 0,
                connection_type: "Network".into(),
                product_id: 0,
            };
            let connection = TunnelConnection::new(
                info.clone(),
                None,
                LockdownTransport::Tcp {
                    host: String::new(),
                },
                "owned-fixture",
            );
            // Only a local cancellation channel: no TUN, forwarding task or IO.
            let (handle, receiver) = TunnelHandle::new(
                info.udid,
                crate::TunnelInfo {
                    server_address: "fd00::1".into(),
                    server_rsd_port: 58783,
                    client_address: "fd00::2".into(),
                    client_mtu: 1280,
                },
                None,
            );
            let device = connection.into_device(handle, probe);
            assert_eq!(device.initial_rsd_probe().state(), expected);
            if expected == InitialRsdProbeState::Failed {
                assert!(device.rsd().is_none());
                assert_eq!(
                    device.initial_rsd_probe().error_class(),
                    Some(crate::ErrorClass::Timeout)
                );
                let CoreError::Io(error) = device.initial_rsd_probe().error().unwrap() else {
                    panic!("typed error lost")
                };
                assert_eq!(error.to_string(), "synthetic-private-probe-failure");
                assert!(!format!("{:?}", device.initial_rsd_probe()).contains("synthetic-private"));
            } else {
                assert_eq!(device.rsd().unwrap().udid, "synthetic-initial-directory");
                assert!(device.initial_rsd_probe().error().is_none());
            }
            assert_eq!(
                device.into_rsd().is_some(),
                expected == InitialRsdProbeState::Available
            );
            assert!(
                receiver.has_changed().is_err(),
                "consuming the device still drops its local handle"
            );
        }
    }
}
