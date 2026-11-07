//! Sealed activation selection shared by the original authenticated strategies.
use super::ConnectedDevice;
use crate::CoreError;

pub(super) trait ActivationPolicy: Copy + Send + Sync {
    type Output;

    fn without_tunnel(self, device: ConnectedDevice) -> Result<Self::Output, CoreError>;

    #[cfg(feature = "tunnel")]
    fn activate<T: super::tunnel_activation::TunnelTransport>(
        self,
        connection: super::tunnel_activation::TunnelConnection,
        stream: T,
        mode: crate::TunMode,
        reservation: Option<super::TunnelReservationOwner<'_>>,
    ) -> impl std::future::Future<Output = Result<Self::Output, CoreError>> + Send;
}

#[derive(Clone, Copy)]
pub(super) struct Immediate;

impl ActivationPolicy for Immediate {
    type Output = ConnectedDevice;

    fn without_tunnel(self, device: ConnectedDevice) -> Result<Self::Output, CoreError> {
        Ok(device)
    }

    #[cfg(feature = "tunnel")]
    async fn activate<T: super::tunnel_activation::TunnelTransport>(
        self,
        connection: super::tunnel_activation::TunnelConnection,
        stream: T,
        mode: crate::TunMode,
        reservation: Option<super::TunnelReservationOwner<'_>>,
    ) -> Result<Self::Output, CoreError> {
        Box::pin(super::tunnel_activation::activate_tunnel(
            connection,
            stream,
            mode,
            reservation,
        ))
        .await
    }
}

#[cfg(feature = "tunnel-private-ip")]
#[derive(Clone, Copy)]
pub(super) struct Prepared<'a> {
    pub(super) deadline: tokio::time::Instant,
    pub(super) bank: &'a std::sync::Arc<tokio::sync::Semaphore>,
}

#[cfg(feature = "tunnel-private-ip")]
impl ActivationPolicy for Prepared<'_> {
    type Output = super::PreparedConnectedDevice;
    fn without_tunnel(self, _: ConnectedDevice) -> Result<Self::Output, CoreError> {
        Err(CoreError::Unsupported(
            "prepared connection requires a tunnel".into(),
        ))
    }
    async fn activate<T: super::tunnel_activation::TunnelTransport>(
        self,
        connection: super::tunnel_activation::TunnelConnection,
        stream: T,
        mode: crate::TunMode,
        reservation: Option<super::TunnelReservationOwner<'_>>,
    ) -> Result<Self::Output, CoreError> {
        super::tunnel_activation::prepared::prepare_tunnel(
            connection,
            stream,
            mode,
            reservation,
            self,
        )
        .await
    }
}
