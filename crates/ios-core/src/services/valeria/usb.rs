use std::io::{Read, Write};
use std::thread;
use std::time::{Duration, Instant};

use nusb::descriptors::{InterfaceDescriptor, TransferType};
use nusb::io::{EndpointRead, EndpointWrite};
use nusb::transfer::{Bulk, ControlOut, ControlType, Direction, In, Out, Recipient};
use nusb::{DeviceInfo, Interface, MaybeFuture};

use super::protocol::{self, Packet};
use super::session::ValeriaTransport;
use super::{CaptureOptions, ValeriaError};

const APPLE_VENDOR_ID: u16 = 0x05ac;
const QUICKTIME_INTERFACE_CLASS: u8 = 0xff;
const QUICKTIME_INTERFACE_SUBCLASS: u8 = 0x2a;
const USBMUX_INTERFACE_SUBCLASS: u8 = 0xfe;
const USBMUX_INTERFACE_PROTOCOL: u8 = 0x02;
const QUICKTIME_CONFIGURATION_REQUEST: u8 = 0x52;
const QUICKTIME_ENABLE_INDEX: u16 = 0x0002;
const QUICKTIME_DISABLE_INDEX: u16 = 0x0000;
const USB_TIMEOUT: Duration = Duration::from_secs(5);
const REENUMERATE_TIMEOUT: Duration = Duration::from_secs(8);
const REENUMERATE_POLL: Duration = Duration::from_millis(250);

pub struct UsbValeriaCapture {
    _interface: Interface,
    reader: EndpointRead<Bulk>,
    writer: EndpointWrite<Bulk>,
}

struct QuicktimeInterface {
    interface_number: u8,
    endpoints: AvEndpoints,
}

impl UsbValeriaCapture {
    pub fn open(options: CaptureOptions) -> Result<Self, ValeriaError> {
        let info = select_device_info(&options)?;
        if let Some(capture) = open_quicktime_capture(&info)? {
            return Ok(capture);
        }

        activate_quicktime_configuration(&info)?;
        wait_for_quicktime_capture(&options)
    }

    pub fn record_annex_b(
        options: CaptureOptions,
        output: &std::path::Path,
        duration_secs: u64,
    ) -> Result<super::CaptureSummary, ValeriaError> {
        let mut transport = Self::open(options.clone())?;
        let mut session = super::ValeriaSession::new(options);
        let result = session.record_annex_b(&mut transport, output, duration_secs);
        let _ = session.close(&mut transport);
        result
    }

    fn disable_quicktime_configuration(&mut self) -> Result<(), ValeriaError> {
        send_quicktime_configuration_via_interface(&self._interface, false)
    }
}

impl Drop for UsbValeriaCapture {
    fn drop(&mut self) {
        let _ = self.disable_quicktime_configuration();
    }
}

impl ValeriaTransport for UsbValeriaCapture {
    fn read_packet(&mut self) -> Result<Packet, ValeriaError> {
        let mut len_bytes = [0u8; 4];
        self.reader.read_exact(&mut len_bytes)?;
        let total_len = u32::from_le_bytes(len_bytes) as usize;
        if total_len < 8 {
            return Err(ValeriaError::Protocol(format!(
                "USB packet length {total_len} is shorter than Valeria header"
            )));
        }

        let mut bytes = vec![0u8; total_len];
        bytes[..4].copy_from_slice(&len_bytes);
        self.reader.read_exact(&mut bytes[4..])?;
        protocol::decode_packet(&bytes)
    }

    fn write_bytes(&mut self, bytes: &[u8]) -> Result<(), ValeriaError> {
        self.writer.write_all(bytes)?;
        self.writer.flush()?;
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct EndpointInfo {
    pub(crate) address: u8,
    pub(crate) direction_in: bool,
    pub(crate) transfer_bulk: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct AvEndpoints {
    pub(crate) read_endpoint: u8,
    pub(crate) write_endpoint: u8,
}

pub(crate) fn select_av_endpoints(endpoints: &[EndpointInfo]) -> Result<AvEndpoints, ValeriaError> {
    let bulk_in: Vec<u8> = endpoints
        .iter()
        .filter(|endpoint| endpoint.transfer_bulk && endpoint.direction_in)
        .map(|endpoint| endpoint.address)
        .collect();
    let bulk_out: Vec<u8> = endpoints
        .iter()
        .filter(|endpoint| endpoint.transfer_bulk && !endpoint.direction_in)
        .map(|endpoint| endpoint.address)
        .collect();

    if bulk_in.len() < 2 || bulk_out.len() < 2 {
        return Err(ValeriaError::Protocol(
            "QuickTime config did not expose USBMUX and AV bulk endpoints".into(),
        ));
    }

    Ok(AvEndpoints {
        read_endpoint: bulk_in[1],
        write_endpoint: bulk_out[1],
    })
}

fn select_device_info(options: &CaptureOptions) -> Result<DeviceInfo, ValeriaError> {
    let devices = nusb::list_devices()
        .wait()
        .map_err(|err| ValeriaError::Usb(format!("failed to enumerate USB devices: {err}")))?;
    let candidates = devices
        .filter(|device| device.vendor_id() == APPLE_VENDOR_ID)
        .filter(is_apple_mobile_device)
        .filter(|device| matches_requested_udid(device, options.udid.as_deref()))
        .collect::<Vec<_>>();

    match candidates.len() {
        0 => Err(ValeriaError::DeviceNotFound(
            options
                .udid
                .clone()
                .unwrap_or_else(|| "Apple USB device".to_string()),
        )),
        1 => Ok(candidates.into_iter().next().expect("length checked")),
        _ => Err(ValeriaError::MultipleDevices),
    }
}

fn matches_requested_udid(device: &DeviceInfo, udid: Option<&str>) -> bool {
    serial_matches_requested_udid(device.serial_number(), udid)
}

fn is_apple_mobile_device(device: &DeviceInfo) -> bool {
    device.interfaces().any(|interface| {
        is_apple_mobile_interface(
            interface.class(),
            interface.subclass(),
            interface.protocol(),
        )
    })
}

fn is_apple_mobile_interface(class: u8, subclass: u8, protocol: u8) -> bool {
    activation_interface_rank(class, subclass, protocol).is_some()
}

fn activation_interface_rank(class: u8, subclass: u8, protocol: u8) -> Option<u8> {
    if class != QUICKTIME_INTERFACE_CLASS {
        return None;
    }

    if subclass == USBMUX_INTERFACE_SUBCLASS && protocol == USBMUX_INTERFACE_PROTOCOL {
        Some(0)
    } else if subclass == QUICKTIME_INTERFACE_SUBCLASS {
        Some(1)
    } else {
        None
    }
}

fn is_quicktime_streaming_interface(class: u8, subclass: u8) -> bool {
    class == QUICKTIME_INTERFACE_CLASS && subclass == QUICKTIME_INTERFACE_SUBCLASS
}

fn serial_matches_requested_udid(serial: Option<&str>, udid: Option<&str>) -> bool {
    let Some(udid) = udid else {
        return true;
    };
    let Some(serial) = serial else {
        return false;
    };

    serial.eq_ignore_ascii_case(udid)
        || normalize_udid(serial).eq_ignore_ascii_case(&normalize_udid(udid))
}

fn normalize_udid(value: &str) -> String {
    value
        .chars()
        .filter(|ch| *ch != '-' && *ch != '\0' && !ch.is_ascii_whitespace())
        .collect()
}

fn open_quicktime_capture(info: &DeviceInfo) -> Result<Option<UsbValeriaCapture>, ValeriaError> {
    let device = info
        .open()
        .wait()
        .map_err(|err| ValeriaError::Usb(format!("failed to open USB device: {err}")))?;
    let Some(quicktime) = find_quicktime_interface(&device)? else {
        return Ok(None);
    };

    let interface = device
        .detach_and_claim_interface(quicktime.interface_number)
        .wait()
        .map_err(|err| {
            ValeriaError::Usb(format!(
                "failed to claim QuickTime interface {}: {err}",
                quicktime.interface_number
            ))
        })?;
    let reader = interface
        .endpoint::<Bulk, In>(quicktime.endpoints.read_endpoint)
        .map_err(|err| {
            ValeriaError::Usb(format!(
                "failed to open AV IN endpoint 0x{:02x}: {err}",
                quicktime.endpoints.read_endpoint
            ))
        })?
        .reader(64 * 1024)
        .with_num_transfers(8);
    let writer = interface
        .endpoint::<Bulk, Out>(quicktime.endpoints.write_endpoint)
        .map_err(|err| {
            ValeriaError::Usb(format!(
                "failed to open AV OUT endpoint 0x{:02x}: {err}",
                quicktime.endpoints.write_endpoint
            ))
        })?
        .writer(16 * 1024)
        .with_num_transfers(2);

    Ok(Some(UsbValeriaCapture {
        _interface: interface,
        reader,
        writer,
    }))
}

fn find_quicktime_interface(
    device: &nusb::Device,
) -> Result<Option<QuicktimeInterface>, ValeriaError> {
    let active = device.active_configuration().map_err(|err| {
        ValeriaError::Usb(format!("failed to read active USB configuration: {err}"))
    })?;

    for interface in active.interface_alt_settings() {
        if !is_quicktime_streaming_interface(interface.class(), interface.subclass()) {
            continue;
        }

        let endpoint_infos = endpoint_infos(&interface);
        if let Ok(endpoints) = select_av_endpoints(&endpoint_infos) {
            return Ok(Some(QuicktimeInterface {
                interface_number: interface.interface_number(),
                endpoints,
            }));
        }
    }

    Ok(None)
}

fn endpoint_infos(interface: &InterfaceDescriptor<'_>) -> Vec<EndpointInfo> {
    interface
        .endpoints()
        .map(|endpoint| EndpointInfo {
            address: endpoint.address(),
            direction_in: endpoint.direction() == Direction::In,
            transfer_bulk: endpoint.transfer_type() == TransferType::Bulk,
        })
        .collect()
}

fn activate_quicktime_configuration(info: &DeviceInfo) -> Result<(), ValeriaError> {
    let device = info.open().wait().map_err(|err| {
        ValeriaError::Usb(format!(
            "failed to open USB device for QuickTime activation: {err}"
        ))
    })?;
    send_quicktime_configuration(&device, true)
}

fn wait_for_quicktime_capture(options: &CaptureOptions) -> Result<UsbValeriaCapture, ValeriaError> {
    let deadline = Instant::now() + REENUMERATE_TIMEOUT;
    while Instant::now() < deadline {
        thread::sleep(REENUMERATE_POLL);
        let info = match select_device_info(options) {
            Ok(info) => info,
            Err(ValeriaError::DeviceNotFound(_)) => continue,
            Err(err) => return Err(err),
        };
        if let Some(capture) = open_quicktime_capture(&info)? {
            return Ok(capture);
        }
    }

    Err(ValeriaError::Usb(
        "timed out waiting for QuickTime USB configuration to appear".into(),
    ))
}

fn quicktime_control_request(enable: bool) -> ControlOut<'static> {
    ControlOut {
        control_type: ControlType::Vendor,
        recipient: Recipient::Device,
        request: QUICKTIME_CONFIGURATION_REQUEST,
        value: 0,
        index: if enable {
            QUICKTIME_ENABLE_INDEX
        } else {
            QUICKTIME_DISABLE_INDEX
        },
        data: &[],
    }
}

#[cfg(any(target_os = "linux", target_os = "macos", target_os = "android"))]
fn send_quicktime_configuration(device: &nusb::Device, enable: bool) -> Result<(), ValeriaError> {
    device
        .control_out(quicktime_control_request(enable), USB_TIMEOUT)
        .wait()
        .map_err(|err| ValeriaError::Usb(format!("QuickTime USB control request failed: {err}")))
}

#[cfg(target_os = "windows")]
fn send_quicktime_configuration(device: &nusb::Device, enable: bool) -> Result<(), ValeriaError> {
    let interface_number = device
        .active_configuration()
        .map_err(|err| {
            ValeriaError::Usb(format!("failed to read active USB configuration: {err}"))
        })?
        .interface_alt_settings()
        .filter_map(|interface| {
            activation_interface_rank(
                interface.class(),
                interface.subclass(),
                interface.protocol(),
            )
            .map(|rank| (rank, interface.interface_number()))
        })
        .min()
        .map(|(_, interface_number)| interface_number)
        .ok_or_else(|| {
            ValeriaError::Usb(
                "USB device exposes no Apple USBMUX or QuickTime activation interface".into(),
            )
        })?;
    let interface = device
        .claim_interface(interface_number)
        .wait()
        .map_err(|err| {
            ValeriaError::Usb(format_windows_claim_interface_error(
                interface_number,
                &err.to_string(),
            ))
        })?;
    send_quicktime_configuration_via_interface(&interface, enable)
}

#[cfg(any(target_os = "windows", test))]
fn format_windows_claim_interface_error(interface_number: u8, err: &str) -> String {
    if err.contains("error 50") {
        format!(
            "failed to claim Apple USB interface {interface_number} for QuickTime activation: \
             {err}. Windows error 50 indicates this interface cannot be opened through \
             nusb/WinUSB with the current Apple driver stack; the current Windows raw USB backend \
             is not supported on this host. Use a Linux/libusb-compatible host or bind the device \
             to a driver stack that permits raw USB access."
        )
    } else {
        format!("failed to claim interface {interface_number} for QuickTime activation: {err}")
    }
}

fn send_quicktime_configuration_via_interface(
    interface: &Interface,
    enable: bool,
) -> Result<(), ValeriaError> {
    interface
        .control_out(quicktime_control_request(enable), USB_TIMEOUT)
        .wait()
        .map_err(|err| ValeriaError::Usb(format!("QuickTime USB control request failed: {err}")))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identifies_quicktime_streaming_interface() {
        let endpoints = vec![
            EndpointInfo {
                address: 0x81,
                direction_in: true,
                transfer_bulk: true,
            },
            EndpointInfo {
                address: 0x02,
                direction_in: false,
                transfer_bulk: true,
            },
            EndpointInfo {
                address: 0x83,
                direction_in: true,
                transfer_bulk: true,
            },
            EndpointInfo {
                address: 0x04,
                direction_in: false,
                transfer_bulk: true,
            },
        ];

        let selected = select_av_endpoints(&endpoints).unwrap();
        assert_eq!(selected.read_endpoint, 0x83);
        assert_eq!(selected.write_endpoint, 0x04);
    }

    #[test]
    fn rejects_interfaces_without_two_bulk_pairs() {
        let endpoints = vec![
            EndpointInfo {
                address: 0x81,
                direction_in: true,
                transfer_bulk: true,
            },
            EndpointInfo {
                address: 0x02,
                direction_in: false,
                transfer_bulk: true,
            },
        ];

        let err = select_av_endpoints(&endpoints).expect_err("missing AV pair");
        assert!(err.to_string().contains("AV bulk endpoints"));
    }

    #[test]
    fn matches_usb_serial_when_requested_udid_contains_dash() {
        assert!(serial_matches_requested_udid(
            Some("0000813000065DD90E40001C"),
            Some("00008130-00065DD90E40001C")
        ));
    }

    #[test]
    fn matches_usb_serial_ignoring_case_and_dash() {
        assert!(serial_matches_requested_udid(
            Some("0000813000065dd90e40001c"),
            Some("00008130-00065DD90E40001C")
        ));
    }

    #[test]
    fn matches_usb_serial_with_trailing_nul_padding() {
        assert!(serial_matches_requested_udid(
            Some("0000813000065DD90E40001C\0\0\0\0"),
            Some("00008130-00065DD90E40001C")
        ));
    }

    #[test]
    fn identifies_apple_mobile_candidate_interfaces() {
        assert!(is_apple_mobile_interface(0xff, 0xfe, 0x02));
        assert!(is_apple_mobile_interface(0xff, 0x2a, 0x00));
        assert!(!is_apple_mobile_interface(0x03, 0x01, 0x01));
    }

    #[test]
    fn ranks_usbmux_interface_first_for_quicktime_activation() {
        assert_eq!(activation_interface_rank(0xff, 0xfe, 0x02), Some(0));
        assert_eq!(activation_interface_rank(0xff, 0x2a, 0x00), Some(1));
        assert_eq!(activation_interface_rank(0x06, 0x01, 0x01), None);
    }

    #[test]
    fn windows_error_50_claim_message_explains_backend_limit() {
        let message = format_windows_claim_interface_error(1, "failed to open device (error 50)");

        assert!(message.contains("Apple USB interface 1"));
        assert!(message.contains("Windows error 50"));
        assert!(message.contains("current Windows raw USB backend is not supported"));
    }
}
