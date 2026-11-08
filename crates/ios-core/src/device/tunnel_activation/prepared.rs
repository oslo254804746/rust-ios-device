//! Private verification on the original authenticated packet link.
use super::*;
use crate::device::{activation_policy::Prepared, PreparedConnectedDevice};
use crate::tunnel::tun::userspace::UserspaceTunDevice;
use std::net::{Ipv6Addr, SocketAddrV6};

pub(in crate::device) async fn prepare_tunnel<T: TunnelTransport>(
    connection: TunnelConnection,
    stream: T,
    mode: TunMode,
    reservation: Option<super::super::TunnelReservationOwner<'_>>,
    policy: Prepared<'_>,
) -> Result<PreparedConnectedDevice, CoreError> {
    super::super::existing_rsd::Clock::at(policy.deadline).check()?;
    let original_bank = reservation.as_ref().and_then(|owner| owner.byte_bank());
    if !original_bank
        .as_ref()
        .is_some_and(|bank| Arc::ptr_eq(bank, policy.bank))
    {
        return Err(CoreError::Xpc(
            super::super::existing_rsd::ExistingRsdProbeError::metadata_limit(),
        ));
    }
    connection.validate_initial_bank(Some(policy.bank))?;
    // Keep the large protocol state machine out of every authenticated strategy
    // frame. Admission precedes its actual Box and even its future constructor.
    super::super::admitted_future::prepare(policy.bank, || {
        prepare_tunnel_inner(connection, stream, mode, reservation, policy)
    })?
    .await
}

async fn prepare_tunnel_inner<T: TunnelTransport>(
    connection: TunnelConnection,
    mut stream: T,
    mode: TunMode,
    mut reservation: Option<super::super::TunnelReservationOwner<'_>>,
    policy: Prepared<'_>,
) -> Result<PreparedConnectedDevice, CoreError> {
    let clock = super::super::existing_rsd::Clock::at(policy.deadline);
    clock
        .run(async {
            clock.check()?;
            let info = crate::tunnel::handshake::exchange_tunnel_parameters_with_timeout(
                &mut stream,
                TUNNEL_HANDSHAKE_TIMEOUT,
            )
            .await
            .map_err(CoreError::Tunnel)?;
            clock.check()?;
            let mtu = ValidatedMtu::new(info.client_mtu).map_err(CoreError::Tunnel)?;
            if (mode == TunMode::Kernel && !cfg!(feature = "tunnel-kernel"))
                || (mode == TunMode::Userspace && !cfg!(feature = "tunnel-userspace"))
            {
                return Err(CoreError::Unsupported(
                    "requested final tunnel backend is unavailable".into(),
                ));
            }
            let endpoint = SocketAddrV6::new(
                info.server_address.parse::<Ipv6Addr>().map_err(|source| {
                    CoreError::Io(std::io::Error::new(
                        std::io::ErrorKind::InvalidInput,
                        source,
                    ))
                })?,
                info.server_rsd_port,
                0,
                0,
            );
            if endpoint.port() == 0 {
                return Err(CoreError::Io(std::io::Error::new(
                    std::io::ErrorKind::InvalidInput,
                    "private RSD port is zero",
                )));
            }
            // Both original protocol selectors make at most one fresh connector
            // call per mode. This is their actual combined retained-flow capacity.
            let capacity = crate::xpc::rsd::RSD_HANDSHAKE_ATTEMPTS * 2;
            let (runtime, launch) = UserspaceTunDevice::prepare_private_validated_with_bank(
                &info.client_address,
                &info.server_address,
                mtu,
                stream,
                policy.bank.clone(),
                capacity,
                mode == TunMode::Kernel,
            )
            .await
            .map_err(CoreError::Tunnel)?;
            let handle = Arc::new(TunnelHandle::new_prepared(
                connection.info.udid.clone(),
                info,
                mode,
                runtime,
            ));
            let connector = handle
                .private_connector()
                .expect("same prepared private route");
            let mut cleanup = StopUnlessTransferred {
                handle: &handle,
                transferred: false,
            };
            if let Some(owner) = reservation.as_mut() {
                owner.attach(handle.clone())?;
            }
            clock.check()?;
            launch().map_err(CoreError::Tunnel)?;

            // Preserve the independent optional observation. Its success is never
            // substituted for the required fresh checked catalogue below.
            let initial = tokio::select! {
                biased;
                error = handle.private_interrupted() => return Err(CoreError::Tunnel(error)),
                result = super::super::rsd_connect::attempt_owned_rsd_via_private(
                    &connector, endpoint, policy.deadline, policy.bank,
                ) => result,
            };
            handle.private_work_result().map_err(CoreError::Tunnel)?;
            clock.check()?;
            let catalogue = tokio::select! {
                biased;
                error = handle.private_interrupted() => return Err(CoreError::Tunnel(error)),
                result = super::super::existing_rsd::prepare_owned_with_connector(
                    crate::xpc::rsd::default_handshake_uuid,
                    || async { connector.connect(endpoint).await.map_err(CoreError::Io) },
                    Some(endpoint), policy.deadline, policy.bank,
                ) => result?,
            };
            handle.private_work_result().map_err(CoreError::Tunnel)?;
            clock.check()?;
            if catalogue.udid() != connection.info.udid {
                return Err(CoreError::Protocol(
                    "prepared RSD device identity changed".into(),
                ));
            }
            let mut device = connection.into_device(handle.clone(), InitialProbe::Owned(initial));
            device.owned_prepared_rsd = Some(catalogue);
            if let Some(owner) = reservation.as_mut() {
                owner.complete();
            }
            cleanup.transferred = true;
            Ok(PreparedConnectedDevice::new(device, policy.deadline))
        })
        .await
}

struct StopUnlessTransferred<'a> {
    handle: &'a Arc<TunnelHandle>,
    transferred: bool,
}
impl Drop for StopUnlessTransferred<'_> {
    fn drop(&mut self) {
        if !self.transferred {
            self.handle.request_stop();
        }
    }
}

#[cfg(test)]
mod tests;
#[cfg(test)]
mod wire_tests;
