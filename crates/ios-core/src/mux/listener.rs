use tokio_stream::Stream;

use crate::mux::connection::{usbmuxd_socket_address, UsbmuxStream};
use crate::mux::protocol::*;
use crate::mux::{MuxDevice, MuxError};

/// A device attach/detach event from usbmuxd.
#[derive(Debug, Clone)]
pub enum MuxEvent {
    Attached(MuxDevice),
    Detached { device_id: u32 },
}

/// Listen for device events on a dedicated usbmuxd connection.
///
/// usbmuxd requires a separate connection for the Listen command.
pub async fn listen_events() -> Result<impl Stream<Item = Result<MuxEvent, MuxError>>, MuxError> {
    let addr = usbmuxd_socket_address();
    let mut stream = UsbmuxStream::connect(&addr).await?;

    let req = ListenRequest {
        message_type: "Listen",
        prog_name: "ios-rs",
        client_version_string: "ios-rs-0.1",
    };
    send_plist(&mut stream, &req, 1).await?;
    // Read the ACK response
    let _: plist::Value = recv_plist(&mut stream).await?;

    Ok(async_stream::try_stream! {
        loop {
            let evt: DeviceEvent = recv_plist(&mut stream).await?;
            match evt.message_type.as_str() {
                "Attached" => {
                    if let Some(props) = evt.properties {
                        yield MuxEvent::Attached(MuxDevice {
                            device_id: evt.device_id,
                            serial_number: props.serial_number,
                            connection_type: props.connection_type,
                            product_id: props.product_id.unwrap_or(0),
                        });
                    }
                }
                "Detached" => {
                    yield MuxEvent::Detached { device_id: evt.device_id };
                }
                _ => {}
            }
        }
    })
}
