//! Checked, fresh usbmux routing retained inside one operation lease.

use super::{DeviceMetadata, DeviceQueryLimits, Endpoint, MuxResponseError, Route};
use crate::mux::{
    connection::UsbmuxStream,
    protocol::{send_plist, ConnectRequest, ListDevicesRequest},
    MuxError,
};
use crate::session::reply_budget::{limit, OutputBudget, ReplyBudget};
use crate::session::{OperationContext, OperationIo};
use crate::CoreError;
use std::io;
use tokio::io::{AsyncRead, AsyncReadExt};

fn protocol(reason: &'static str) -> MuxError {
    MuxError::Protocol(reason.into())
}

async fn recv_reply<S: AsyncRead + Unpin>(
    stream: &mut S,
    tag: u32,
    budget: &ReplyBudget,
) -> Result<plist::Dictionary, MuxError> {
    let mut header = [0; 16];
    stream.read_exact(&mut header).await?;
    let word = |offset| u32::from_le_bytes(header[offset..offset + 4].try_into().unwrap());
    let total = usize::try_from(word(0)).map_err(|error| {
        crate::error::context::retained_io(
            io::ErrorKind::InvalidData,
            "decode SDK mux frame length",
            error,
        )
    })?;
    if total < 16 {
        return Err(protocol("SDK mux frame is shorter than its header"));
    }
    budget.mux(total)?;
    if word(4) != 1 || word(8) != 8 || word(12) != tag {
        return Err(protocol(
            "SDK mux reply header version/type/tag did not match",
        ));
    }
    let mut payload = vec![0; total - 16];
    stream.read_exact(&mut payload).await?;
    plist::from_bytes(&payload).map_err(|error| {
        MuxError::Io(crate::error::context::retained_io(
            io::ErrorKind::InvalidData,
            "decode SDK mux response plist",
            error,
        ))
    })
}

fn check_result(reply: &plist::Dictionary) -> Result<Option<u64>, MuxError> {
    let number = reply
        .get("Number")
        .map(|value| {
            value
                .as_unsigned_integer()
                .ok_or_else(|| protocol("SDK mux Result Number was not an unsigned integer"))
        })
        .transpose()?;
    if let Some(code) = number.filter(|code| *code != 0) {
        return Err(MuxError::Io(io::Error::other(MuxResponseError { code })));
    }
    Ok(number)
}

// Inspect the whole list before cloning any selected metadata. Optional
// ProductID remains optional; malformed unselected entries are not skipped.
// Match Core connect and CLI selected_devices: first case-insensitive USB,
// otherwise the last matching non-USB entry, retaining discovery order.
fn select(
    reply: &plist::Dictionary,
    udid: &str,
    limits: DeviceQueryLimits,
    output: &mut OutputBudget,
) -> Result<DeviceMetadata, CoreError> {
    check_result(reply)?;
    // Existing replies need not include MessageType. This additive checked
    // profile accepts an optional ListDevices echo, but not an unrelated reply.
    if let Some(message_type) = reply.get("MessageType") {
        if message_type.as_string() != Some("ListDevices") {
            return Err(protocol("SDK mux ListDevices received an unrelated MessageType").into());
        }
    }
    let entries = reply
        .get("DeviceList")
        .and_then(plist::Value::as_array)
        .ok_or_else(|| protocol("SDK mux DeviceList was absent or not an array"))?;
    if entries.len() > limits.metadata_entries {
        return Err(limit("device query metadata entry limit exceeded").into());
    }
    let mut utf8 = 0usize;
    let mut selected = None;
    let mut selected_usb = false;
    for entry in entries {
        let entry = entry
            .as_dictionary()
            .ok_or_else(|| protocol("SDK mux device entry was not a dictionary"))?;
        let device_id = entry
            .get("DeviceID")
            .and_then(plist::Value::as_unsigned_integer)
            .ok_or_else(|| protocol("SDK mux DeviceID was absent or invalid"))?;
        let device_id = u32::try_from(device_id).map_err(|error| {
            crate::error::context::retained_io(
                io::ErrorKind::InvalidData,
                "decode SDK mux DeviceID",
                error,
            )
        })?;
        let properties = entry
            .get("Properties")
            .and_then(plist::Value::as_dictionary)
            .ok_or_else(|| protocol("SDK mux Properties was absent or invalid"))?;
        let serial = properties
            .get("SerialNumber")
            .and_then(plist::Value::as_string)
            .ok_or_else(|| protocol("SDK mux SerialNumber was absent or invalid"))?;
        let connection = properties
            .get("ConnectionType")
            .and_then(plist::Value::as_string)
            .ok_or_else(|| protocol("SDK mux ConnectionType was absent or invalid"))?;
        utf8 = utf8
            .checked_add(serial.len())
            .and_then(|bytes| bytes.checked_add(connection.len()))
            .ok_or_else(|| limit("device query metadata UTF-8 length overflow"))?;
        if utf8 > limits.metadata_utf8 {
            return Err(limit("device query metadata UTF-8 limit exceeded").into());
        }
        if properties.contains_key("ProductID") && properties.contains_key("ProductId") {
            return Err(protocol("SDK mux ProductID was duplicated by its legacy alias").into());
        }
        let product_id = properties
            .get("ProductID")
            .or_else(|| properties.get("ProductId"))
            .map(|value| {
                let product = value
                    .as_unsigned_integer()
                    .ok_or_else(|| protocol("SDK mux ProductID was invalid"))?;
                u16::try_from(product).map_err(|error| {
                    MuxError::Io(crate::error::context::retained_io(
                        io::ErrorKind::InvalidData,
                        "decode SDK mux ProductID",
                        error,
                    ))
                })
            })
            .transpose()?;
        if serial == udid && !selected_usb {
            selected = Some((device_id, connection, product_id));
            selected_usb = connection.eq_ignore_ascii_case("USB");
        }
    }
    let (device_id, connection, product_id) =
        selected.ok_or_else(|| CoreError::DeviceNotFound(udid.into()))?;
    output.admit(connection.len())?;
    Ok(DeviceMetadata {
        device_id,
        connection_type: connection.to_owned(),
        product_id,
    })
}

/// The returned IO borrows the operation context and is never a public result.
/// The socket is new for each invocation, then upgraded in place by Connect.
pub(super) async fn connect_lockdown<'ctx>(
    context: &'ctx OperationContext,
    route: &Route,
    limits: DeviceQueryLimits,
    budget: &ReplyBudget,
    output: &mut OutputBudget,
) -> Result<(DeviceMetadata, OperationIo<'ctx, UsbmuxStream>), CoreError> {
    let raw = context
        .run(async {
            match &route.endpoint.0 {
                Endpoint::Tcp(address) => tokio::net::TcpStream::connect(*address)
                    .await
                    .map(UsbmuxStream::Tcp),
                #[cfg(unix)]
                Endpoint::Unix(path) => tokio::net::UnixStream::connect(path)
                    .await
                    .map(UsbmuxStream::Unix),
            }
        })
        .await?;
    let mut stream = context.guard(raw);
    send_plist(
        &mut stream,
        &ListDevicesRequest {
            message_type: "ListDevices",
            prog_name: "ios-rs",
            client_version_string: "ios-rs-0.1",
        },
        1,
    )
    .await?;
    let reply = recv_reply(&mut stream, 1, budget).await?;
    let metadata = select(&reply, &route.udid, limits, output)?;
    drop(reply);
    send_plist(
        &mut stream,
        &ConnectRequest {
            message_type: "Connect",
            prog_name: "ios-rs",
            client_version_string: "ios-rs-0.1",
            bundle_id: "rs.ios",
            lib_usbmux_version: 3,
            device_id: metadata.device_id,
            port_number: crate::lockdown::LOCKDOWN_PORT.swap_bytes(),
        },
        2,
    )
    .await?;
    let reply = recv_reply(&mut stream, 2, budget).await?;
    let number = check_result(&reply)?;
    if number != Some(0)
        || reply.get("MessageType").and_then(plist::Value::as_string) != Some("Result")
    {
        return Err(
            protocol("SDK mux Connect did not return an explicit successful Result").into(),
        );
    }
    Ok((metadata, stream))
}
