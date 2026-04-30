use std::collections::HashMap;
#[cfg(feature = "mdns")]
use std::time::{Duration, Instant};

use crate::mux::MuxClient;
use tokio_stream::Stream;

use crate::error::CoreError;

/// Summary info about a connected iOS device.
#[derive(Debug, Clone, serde::Serialize)]
pub struct DeviceInfo {
    pub udid: String,
    pub device_id: u32,
    pub connection_type: String,
    pub product_id: u16,
}

impl DeviceInfo {
    pub(crate) fn from_mux(d: crate::mux::MuxDevice) -> Self {
        Self {
            udid: d.serial_number,
            device_id: d.device_id,
            connection_type: d.connection_type,
            product_id: d.product_id,
        }
    }
}

/// A device plug/unplug event.
#[derive(Debug, Clone)]
pub enum DeviceEvent {
    Attached(DeviceInfo),
    Detached { udid: String, device_id: u32 },
}

/// List all currently connected devices via usbmuxd.
pub async fn list_devices() -> Result<Vec<DeviceInfo>, CoreError> {
    let mut mux = MuxClient::connect().await?;
    let devices = mux.list_devices().await?;
    Ok(devices.into_iter().map(DeviceInfo::from_mux).collect())
}

/// Watch for device events using a dedicated usbmuxd Listen connection.
pub async fn watch_devices() -> Result<impl Stream<Item = Result<DeviceEvent, CoreError>>, CoreError>
{
    use tokio_stream::StreamExt;

    let events = crate::mux::listener::listen_events().await?;
    let attached_devices = list_devices().await?;

    Ok(async_stream::stream! {
        let mut mapper = DeviceEventMapper::with_attached_devices(attached_devices);
        tokio::pin!(events);

        while let Some(event) = events.next().await {
            match event {
                Ok(event) => {
                    if let Some(mapped) = mapper.map(event) {
                        yield Ok(mapped);
                    }
                }
                Err(err) => yield Err(CoreError::from(err)),
            }
        }
    })
}

/// Discover iOS 17+ devices via mDNS (for wireless / USB-Ethernet connections).
///
/// Looks for `_remoted._tcp` services, which are advertised by iOS 17+ devices
/// on their USB-Ethernet (or Wi-Fi) interfaces.
///
/// Returns a stream of `(ipv6_address, rsd_port)` pairs for discovered devices.
/// The caller should perform an RSD handshake to get the full service list.
#[cfg(feature = "mdns")]
pub async fn discover_mdns() -> Result<impl Stream<Item = MdnsDevice>, CoreError> {
    use mdns_sd::{ServiceDaemon, ServiceEvent};

    let mdns = ServiceDaemon::new().map_err(|e| CoreError::Other(format!("mDNS daemon: {e}")))?;

    let service_type = "_remoted._tcp.local.";
    let receiver = mdns
        .browse(service_type)
        .map_err(|e| CoreError::Other(format!("mDNS browse: {e}")))?;

    // Convert mdns_sd sync channel to async stream
    let stream = async_stream::stream! {
        loop {
            match receiver.recv_async().await {
                Ok(ServiceEvent::ServiceResolved(info)) => {
                    // Extract IPv6 addresses
                    for addr in info.get_addresses() {
                        if let std::net::IpAddr::V6(v6) = addr {
                            let port = info.get_port();
                            let props = info.get_properties();
                            let udid = props.get("UniqueDeviceID")
                                .map(|v| v.val_str().to_string())
                                .unwrap_or_default();
                            yield MdnsDevice {
                                ipv6:     *v6,
                                rsd_port: port,
                                udid,
                                name:     info.get_fullname().to_string(),
                            };
                        }
                    }
                }
                Ok(_) => continue,
                Err(_) => break,
            }
        }
    };

    Ok(stream)
}

#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg(feature = "mdns")]
pub struct BonjourService {
    pub instance: String,
    pub port: u16,
    pub addresses: Vec<String>,
    pub properties: HashMap<String, String>,
}

#[cfg(feature = "mdns")]
pub async fn browse_mobdev2(timeout: Duration) -> Result<Vec<BonjourService>, CoreError> {
    browse_bonjour_service("_apple-mobdev2._tcp.local.", timeout).await
}

#[cfg(feature = "mdns")]
pub async fn browse_remotepairing(timeout: Duration) -> Result<Vec<BonjourService>, CoreError> {
    browse_bonjour_service("_remotepairing._tcp.local.", timeout).await
}

pub fn mobdev2_wifi_mac(instance: &str) -> Option<&str> {
    instance.split_once('@').map(|(mac, _)| mac)
}

/// A device discovered via mDNS.
#[derive(Debug, Clone)]
#[cfg(feature = "mdns")]
pub struct MdnsDevice {
    /// Device's IPv6 address (USB-Ethernet or Wi-Fi)
    pub ipv6: std::net::Ipv6Addr,
    /// RSD port (typically 58783)
    pub rsd_port: u16,
    /// UDID from mDNS TXT record (may be empty)
    pub udid: String,
    /// mDNS service full name
    pub name: String,
}

#[cfg(feature = "mdns")]
async fn browse_bonjour_service(
    service_type: &str,
    timeout: Duration,
) -> Result<Vec<BonjourService>, CoreError> {
    use mdns_sd::{ServiceDaemon, ServiceEvent};

    let mdns = ServiceDaemon::new().map_err(|e| CoreError::Other(format!("mDNS daemon: {e}")))?;
    let receiver = mdns
        .browse(service_type)
        .map_err(|e| CoreError::Other(format!("mDNS browse: {e}")))?;

    let deadline = Instant::now() + timeout;
    let mut services = HashMap::<String, BonjourService>::new();

    loop {
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            break;
        }

        match tokio::time::timeout(remaining, receiver.recv_async()).await {
            Ok(Ok(ServiceEvent::ServiceResolved(info))) => {
                let instance = info.get_fullname().to_string();
                let entry = services
                    .entry(instance.clone())
                    .or_insert_with(|| BonjourService {
                        instance,
                        port: info.get_port(),
                        addresses: Vec::new(),
                        properties: info
                            .get_properties()
                            .iter()
                            .map(|property| {
                                (property.key().to_string(), property.val_str().to_string())
                            })
                            .collect(),
                    });

                entry.port = info.get_port();
                for address in info.get_addresses() {
                    let full = address.to_string();
                    if !entry.addresses.contains(&full) {
                        entry.addresses.push(full);
                    }
                }
            }
            Ok(Ok(_)) => {}
            Ok(Err(_)) | Err(_) => break,
        }
    }

    Ok(services.into_values().collect())
}

#[derive(Default)]
struct DeviceEventMapper {
    attached_devices: HashMap<u32, DeviceInfo>,
}

impl DeviceEventMapper {
    fn with_attached_devices(attached_devices: Vec<DeviceInfo>) -> Self {
        let attached_devices = attached_devices
            .into_iter()
            .map(|device| (device.device_id, device))
            .collect();
        Self { attached_devices }
    }

    fn map(&mut self, event: crate::mux::MuxEvent) -> Option<DeviceEvent> {
        match event {
            crate::mux::MuxEvent::Attached(device) => {
                let info = DeviceInfo::from_mux(device);
                self.attached_devices.insert(info.device_id, info.clone());
                Some(DeviceEvent::Attached(info))
            }
            crate::mux::MuxEvent::Detached { device_id } => {
                let udid = self
                    .attached_devices
                    .remove(&device_id)
                    .map(|device| device.udid)
                    .unwrap_or_default();
                Some(DeviceEvent::Detached { udid, device_id })
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mapper_preserves_attached_device_details() {
        let mut mapper = DeviceEventMapper::default();
        let event = mapper
            .map(crate::mux::MuxEvent::Attached(crate::mux::MuxDevice {
                device_id: 2,
                serial_number: "00008150-000A584C0E62401C".into(),
                connection_type: "USB".into(),
                product_id: 0,
            }))
            .expect("attached event should map");

        match event {
            DeviceEvent::Attached(device) => {
                assert_eq!(device.udid, "00008150-000A584C0E62401C");
                assert_eq!(device.device_id, 2);
                assert_eq!(device.connection_type, "USB");
                assert_eq!(device.product_id, 0);
            }
            DeviceEvent::Detached { .. } => panic!("expected attached event"),
        }
    }

    #[test]
    fn mapper_rehydrates_udid_for_detached_device() {
        let mut mapper = DeviceEventMapper::default();
        mapper.map(crate::mux::MuxEvent::Attached(crate::mux::MuxDevice {
            device_id: 7,
            serial_number: "detaching-udid".into(),
            connection_type: "USB".into(),
            product_id: 0,
        }));

        let event = mapper
            .map(crate::mux::MuxEvent::Detached { device_id: 7 })
            .expect("detached event should map");

        assert!(matches!(
            event,
            DeviceEvent::Detached {
                udid,
                device_id: 7
            } if udid == "detaching-udid"
        ));
    }

    #[test]
    fn mapper_emits_empty_udid_when_detach_arrives_without_prior_attach() {
        let mut mapper = DeviceEventMapper::default();
        let event = mapper
            .map(crate::mux::MuxEvent::Detached { device_id: 99 })
            .expect("detached event should still map");

        assert!(matches!(
            event,
            DeviceEvent::Detached {
                udid,
                device_id: 99
            } if udid.is_empty()
        ));
    }

    #[test]
    fn mapper_uses_seeded_devices_for_initial_detach_events() {
        let mut mapper = DeviceEventMapper::with_attached_devices(vec![DeviceInfo {
            udid: "seeded-udid".into(),
            device_id: 42,
            connection_type: "USB".into(),
            product_id: 0,
        }]);

        let event = mapper
            .map(crate::mux::MuxEvent::Detached { device_id: 42 })
            .expect("detached event should still map");

        assert!(matches!(
            event,
            DeviceEvent::Detached {
                udid,
                device_id: 42
            } if udid == "seeded-udid"
        ));
    }

    #[test]
    fn extracts_wifi_mac_from_mobdev2_instance() {
        let mac = mobdev2_wifi_mac(
            "34:10:be:1b:a6:4c@fe80::3610:beff:fe1b:a64c-supportsRP-24._apple-mobdev2._tcp.local.",
        )
        .expect("mobdev2 instance should contain Wi-Fi MAC");

        assert_eq!(mac, "34:10:be:1b:a6:4c");
    }

    #[test]
    fn rejects_non_mobdev2_instance_without_wifi_mac() {
        assert!(mobdev2_wifi_mac("_apple-mobdev2._tcp.local.").is_none());
    }
}
