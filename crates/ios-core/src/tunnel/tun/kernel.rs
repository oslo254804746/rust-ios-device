//! Kernel TUN device via tun-rs.
//!
//! Creates a TUN interface using the OS kernel networking stack.
//! Requires administrator/root privileges.

use tun_rs::{AsyncDevice, DeviceBuilder};

use crate::tunnel::tun::TunDevice;
use crate::tunnel::{TunnelError, ValidatedMtu};

pub struct KernelTunDevice {
    device: AsyncDevice,
    packet_buf: Vec<u8>,
}

impl KernelTunDevice {
    /// Create a kernel TUN device with the given IPv6 address and MTU.
    #[allow(dead_code)]
    pub async fn create(client_address: &str, mtu: u32) -> Result<Self, TunnelError> {
        let mtu = ValidatedMtu::new(mtu)?;
        Self::create_validated(client_address, mtu).await
    }

    pub(crate) async fn create_validated(
        client_address: &str,
        mtu: ValidatedMtu,
    ) -> Result<Self, TunnelError> {
        let packet_buf_len = mtu.as_usize().checked_add(64).ok_or_else(|| {
            TunnelError::Protocol(format!(
                "kernel TUN packet buffer length overflow for MTU {}",
                mtu.as_u16()
            ))
        })?;
        let device = DeviceBuilder::new()
            .ipv6(client_address, 64)
            .mtu(mtu.as_u16())
            .build_async()
            .map_err(TunnelError::Io)?;

        Ok(Self {
            device,
            packet_buf: vec![0u8; packet_buf_len],
        })
    }
}

impl TunDevice for KernelTunDevice {
    async fn read_packet(&mut self) -> Result<Vec<u8>, TunnelError> {
        let n = self.device.recv(&mut self.packet_buf).await?;
        Ok(self.packet_buf[..n].to_vec())
    }

    async fn write_packet(&mut self, packet: &[u8]) -> Result<(), TunnelError> {
        self.device.send(packet).await?;
        Ok(())
    }
}
