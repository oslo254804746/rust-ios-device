use crate::connection::{usbmuxd_socket_address, UsbmuxStream};
use crate::protocol::*;
use crate::{MuxDevice, MuxError};

/// usbmuxd client for device discovery and connection.
pub struct MuxClient {
    stream: UsbmuxStream,
    tag: u32,
}

impl MuxClient {
    /// Connect to the local usbmuxd daemon.
    pub async fn connect() -> Result<Self, MuxError> {
        let addr = usbmuxd_socket_address();
        let stream = UsbmuxStream::connect(&addr).await?;
        Ok(Self { stream, tag: 1 })
    }

    /// List all currently connected devices.
    pub async fn list_devices(&mut self) -> Result<Vec<MuxDevice>, MuxError> {
        let req = ListDevicesRequest {
            message_type: "ListDevices",
            prog_name: "ios-rs",
            client_version_string: "ios-rs-0.1",
        };
        let tag = self.next_tag();
        send_plist(&mut self.stream, &req, tag).await?;
        let resp: DeviceList = recv_plist(&mut self.stream).await?;
        Ok(resp
            .device_list
            .into_iter()
            .map(MuxDevice::from_raw)
            .collect())
    }

    /// Read the system BUID from usbmuxd.
    pub async fn read_buid(&mut self) -> Result<String, MuxError> {
        let req = ReadBuidRequest {
            message_type: "ReadBUID",
            prog_name: "ios-rs",
            client_version_string: "ios-rs-0.1",
            bundle_id: "rs.ios",
            lib_usbmux_version: 3,
        };
        let tag = self.next_tag();
        send_plist(&mut self.stream, &req, tag).await?;
        let resp: ReadBuidResponse = recv_plist(&mut self.stream).await?;
        Ok(resp.buid)
    }

    /// Send ReadPairRecord to usbmuxd (acknowledgment only; pair record is loaded from filesystem).
    pub async fn read_pair_record(&mut self, udid: &str) -> Result<(), MuxError> {
        let req = ReadPairRecordRequest {
            message_type: "ReadPairRecord",
            prog_name: "ios-rs",
            client_version_string: "ios-rs-0.1",
            bundle_id: "rs.ios",
            lib_usbmux_version: 3,
            pair_record_id: udid.to_string(),
        };
        let tag = self.next_tag();
        send_plist(&mut self.stream, &req, tag).await?;
        let _: plist::Value = recv_plist(&mut self.stream).await?;
        Ok(())
    }

    /// Connect the stream to a device port. Consumes self; returns the raw stream.
    pub async fn connect_to_port(
        mut self,
        device_id: u32,
        port: u16,
    ) -> Result<UsbmuxStream, MuxError> {
        let be_port = port.to_be();
        let req = ConnectRequest {
            message_type: "Connect",
            prog_name: "ios-rs",
            client_version_string: "ios-rs-0.1",
            bundle_id: "rs.ios",
            lib_usbmux_version: 3,
            device_id,
            port_number: be_port,
        };
        let tag = self.next_tag();
        send_plist(&mut self.stream, &req, tag).await?;
        let resp: ConnectResponse = recv_plist(&mut self.stream).await?;
        if resp.number != 0 {
            return Err(MuxError::Protocol(format!(
                "usbmuxd connect failed: code {}",
                resp.number
            )));
        }
        Ok(self.stream)
    }

    fn next_tag(&mut self) -> u32 {
        let t = self.tag;
        self.tag += 1;
        t
    }
}
