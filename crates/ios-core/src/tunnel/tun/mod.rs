pub mod kernel;
pub mod userspace;

use crate::tunnel::TunnelError;

/// Abstraction over kernel TUN and userspace (smoltcp) network devices.
pub trait TunDevice: Send + 'static {
    fn read_packet(
        &mut self,
    ) -> impl std::future::Future<Output = Result<Vec<u8>, TunnelError>> + Send;
    fn write_packet(
        &mut self,
        packet: &[u8],
    ) -> impl std::future::Future<Output = Result<(), TunnelError>> + Send;
}
