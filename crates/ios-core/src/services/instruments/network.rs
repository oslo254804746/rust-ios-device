use std::net::{Ipv4Addr, Ipv6Addr};

use tokio::io::{AsyncRead, AsyncWrite};

use crate::services::dtx::codec::{DtxConnection, DtxError};
use crate::services::dtx::types::{DtxMessage, DtxPayload, NSObject};

const MESSAGE_TYPE_INTERFACE_DETECTION: u64 = 0;
const MESSAGE_TYPE_CONNECTION_DETECTION: u64 = 1;
const MESSAGE_TYPE_CONNECTION_UPDATE: u64 = 2;

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct SocketAddress {
    pub family: u8,
    pub port: u16,
    pub address: String,
    pub flow_info: Option<u32>,
    pub scope_id: Option<u32>,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct InterfaceDetectionEvent {
    pub interface_index: u64,
    pub name: String,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct ConnectionDetectionEvent {
    pub local_address: SocketAddress,
    pub remote_address: SocketAddress,
    pub interface_index: u64,
    pub pid: u64,
    pub recv_buffer_size: u64,
    pub recv_buffer_used: u64,
    pub serial_number: u64,
    pub kind: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct ConnectionUpdateEvent {
    pub rx_packets: u64,
    pub rx_bytes: u64,
    pub tx_packets: u64,
    pub tx_bytes: u64,
    pub rx_dups: u64,
    pub rx_ooo: u64,
    pub tx_retx: u64,
    pub min_rtt: u64,
    pub avg_rtt: u64,
    pub connection_serial: u64,
    pub time: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum NetworkMonitorEvent {
    InterfaceDetection(InterfaceDetectionEvent),
    ConnectionDetection(ConnectionDetectionEvent),
    ConnectionUpdate(ConnectionUpdateEvent),
}

pub struct NetworkMonitorClient<S> {
    conn: DtxConnection<S>,
    channel_code: i32,
}

impl<S: AsyncRead + AsyncWrite + Unpin + Send> NetworkMonitorClient<S> {
    pub async fn connect(stream: S) -> Result<Self, DtxError> {
        let mut conn = DtxConnection::new(stream);
        let channel_code = conn.request_channel(super::NETWORK_MONITOR_SVC).await?;
        conn.method_call_async(channel_code, "startMonitoring", &[])
            .await?;
        Ok(Self { conn, channel_code })
    }

    pub async fn next_event(&mut self) -> Result<NetworkMonitorEvent, DtxError> {
        loop {
            let msg = self.conn.recv().await?;
            if msg.expects_reply {
                self.conn.send_ack(&msg).await?;
            }
            if let Some(event) = parse_network_message(&msg)? {
                return Ok(event);
            }
        }
    }

    pub async fn stop(&mut self) -> Result<(), DtxError> {
        self.conn
            .method_call(self.channel_code, "stopMonitoring", &[])
            .await?;
        Ok(())
    }
}

fn parse_network_message(msg: &DtxMessage) -> Result<Option<NetworkMonitorEvent>, DtxError> {
    let Some((kind, args)) = extract_event_tuple(&msg.payload)? else {
        return Ok(None);
    };

    let event = match kind {
        MESSAGE_TYPE_INTERFACE_DETECTION => {
            let Some(interface_index) = args.first().and_then(as_u64) else {
                return Ok(None);
            };
            let Some(name) = args.get(1).and_then(as_string) else {
                return Ok(None);
            };
            NetworkMonitorEvent::InterfaceDetection(InterfaceDetectionEvent {
                interface_index,
                name,
            })
        }
        MESSAGE_TYPE_CONNECTION_DETECTION => {
            if args.len() < 8 {
                return Ok(None);
            }
            let Ok(local_address) = parse_socket_address(&args[0]) else {
                return Ok(None);
            };
            let Ok(remote_address) = parse_socket_address(&args[1]) else {
                return Ok(None);
            };
            let Some(interface_index) = as_u64(&args[2]) else {
                return Ok(None);
            };
            let Some(pid) = as_u64(&args[3]) else {
                return Ok(None);
            };
            let Some(recv_buffer_size) = as_u64(&args[4]) else {
                return Ok(None);
            };
            let Some(recv_buffer_used) = as_u64(&args[5]) else {
                return Ok(None);
            };
            let Some(serial_number) = as_u64(&args[6]) else {
                return Ok(None);
            };
            let Some(kind) = as_u64(&args[7]) else {
                return Ok(None);
            };
            NetworkMonitorEvent::ConnectionDetection(ConnectionDetectionEvent {
                local_address,
                remote_address,
                interface_index,
                pid,
                recv_buffer_size,
                recv_buffer_used,
                serial_number,
                kind,
            })
        }
        MESSAGE_TYPE_CONNECTION_UPDATE => {
            if args.len() < 11 {
                return Ok(None);
            }
            let Some(rx_packets) = as_u64(&args[0]) else {
                return Ok(None);
            };
            let Some(rx_bytes) = as_u64(&args[1]) else {
                return Ok(None);
            };
            let Some(tx_packets) = as_u64(&args[2]) else {
                return Ok(None);
            };
            let Some(tx_bytes) = as_u64(&args[3]) else {
                return Ok(None);
            };
            let Some(rx_dups) = as_u64(&args[4]) else {
                return Ok(None);
            };
            let Some(rx_ooo) = as_u64(&args[5]) else {
                return Ok(None);
            };
            let Some(tx_retx) = as_u64(&args[6]) else {
                return Ok(None);
            };
            let Some(min_rtt) = as_u64(&args[7]) else {
                return Ok(None);
            };
            let Some(avg_rtt) = as_u64(&args[8]) else {
                return Ok(None);
            };
            let Some(connection_serial) = as_u64(&args[9]) else {
                return Ok(None);
            };
            let Some(time) = as_u64(&args[10]) else {
                return Ok(None);
            };
            NetworkMonitorEvent::ConnectionUpdate(ConnectionUpdateEvent {
                rx_packets,
                rx_bytes,
                tx_packets,
                tx_bytes,
                rx_dups,
                rx_ooo,
                tx_retx,
                min_rtt,
                avg_rtt,
                connection_serial,
                time,
            })
        }
        _ => return Ok(None),
    };

    Ok(Some(event))
}

fn extract_event_tuple(payload: &DtxPayload) -> Result<Option<(u64, Vec<NSObject>)>, DtxError> {
    match payload {
        DtxPayload::MethodInvocation { selector, args } => {
            if let Ok(kind) = selector.parse::<u64>() {
                return Ok(Some((kind, args.clone())));
            }
            if let Some((kind, values)) = event_tuple_from_objects(args) {
                return Ok(Some((kind, values)));
            }
            Ok(None)
        }
        DtxPayload::Response(value) => Ok(event_tuple_from_value(value)),
        DtxPayload::Raw(bytes) => match super::unarchive_raw_payload(bytes) {
            Some(value) => Ok(event_tuple_from_value(&value)),
            None => Ok(None),
        },
        _ => Ok(None),
    }
}

fn event_tuple_from_value(value: &NSObject) -> Option<(u64, Vec<NSObject>)> {
    match value {
        NSObject::Array(values) => event_tuple_from_objects(values),
        _ => None,
    }
}

fn event_tuple_from_objects(values: &[NSObject]) -> Option<(u64, Vec<NSObject>)> {
    if values.len() < 2 {
        return None;
    }
    let kind = as_u64(&values[0])?;
    match &values[1] {
        NSObject::Array(items) => Some((kind, items.clone())),
        _ => Some((kind, values[1..].to_vec())),
    }
}

fn parse_socket_address(value: &NSObject) -> Result<SocketAddress, DtxError> {
    let data = match value {
        NSObject::Data(bytes) => bytes,
        other => {
            return Err(DtxError::Protocol(format!(
                "socket address expected NSData, got {other:?}"
            )))
        }
    };

    if data.len() < 4 {
        return Err(DtxError::Protocol(
            "socket address payload too short".into(),
        ));
    }

    let length = data[0] as usize;
    let family = data[1];
    let port = u16::from_be_bytes([data[2], data[3]]);

    match length {
        0x10 if data.len() >= 16 => {
            let address = Ipv4Addr::new(data[4], data[5], data[6], data[7]).to_string();
            Ok(SocketAddress {
                family,
                port,
                address,
                flow_info: None,
                scope_id: None,
            })
        }
        0x1C if data.len() >= 28 => {
            let flow_info = u32::from_le_bytes([data[4], data[5], data[6], data[7]]);
            let mut address = [0u8; 16];
            address.copy_from_slice(&data[8..24]);
            let scope_id = u32::from_le_bytes([data[24], data[25], data[26], data[27]]);
            Ok(SocketAddress {
                family,
                port,
                address: Ipv6Addr::from(address).to_string(),
                flow_info: Some(flow_info),
                scope_id: Some(scope_id),
            })
        }
        _ => Err(DtxError::Protocol(format!(
            "unsupported socket address length {length}"
        ))),
    }
}

fn as_u64(value: &NSObject) -> Option<u64> {
    match value {
        NSObject::Uint(value) => Some(*value),
        NSObject::Int(value) if *value >= 0 => Some(*value as u64),
        _ => None,
    }
}

fn as_string(value: &NSObject) -> Option<String> {
    match value {
        NSObject::String(value) => Some(value.clone()),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use bytes::Bytes;

    use super::*;

    #[test]
    fn parses_interface_detection_payload() {
        let msg = DtxMessage {
            identifier: 1,
            conversation_idx: 0,
            channel_code: 4,
            expects_reply: false,
            payload: DtxPayload::Response(NSObject::Array(vec![
                NSObject::Int(0),
                NSObject::Array(vec![NSObject::Int(3), NSObject::String("en0".into())]),
            ])),
        };

        let event = parse_network_message(&msg).unwrap().unwrap();
        assert_eq!(
            event,
            NetworkMonitorEvent::InterfaceDetection(InterfaceDetectionEvent {
                interface_index: 3,
                name: "en0".into(),
            })
        );
    }

    #[test]
    fn parses_connection_detection_payload() {
        let msg = DtxMessage {
            identifier: 1,
            conversation_idx: 0,
            channel_code: 4,
            expects_reply: false,
            payload: DtxPayload::Response(NSObject::Array(vec![
                NSObject::Int(1),
                NSObject::Array(vec![
                    NSObject::Data(Bytes::from_static(&[
                        0x10, 0x02, 0x00, 0x50, 127, 0, 0, 1, 0, 0, 0, 0, 0, 0, 0, 0,
                    ])),
                    NSObject::Data(Bytes::from_static(&[
                        0x10, 0x02, 0x01, 0xbb, 8, 8, 8, 8, 0, 0, 0, 0, 0, 0, 0, 0,
                    ])),
                    NSObject::Int(2),
                    NSObject::Int(123),
                    NSObject::Int(4096),
                    NSObject::Int(512),
                    NSObject::Int(99),
                    NSObject::Int(1),
                ]),
            ])),
        };

        let event = parse_network_message(&msg).unwrap().unwrap();
        match event {
            NetworkMonitorEvent::ConnectionDetection(event) => {
                assert_eq!(event.local_address.address, "127.0.0.1");
                assert_eq!(event.local_address.port, 80);
                assert_eq!(event.remote_address.address, "8.8.8.8");
                assert_eq!(event.remote_address.port, 443);
                assert_eq!(event.pid, 123);
                assert_eq!(event.serial_number, 99);
            }
            other => panic!("unexpected event: {other:?}"),
        }
    }

    #[test]
    fn parses_connection_update_payload() {
        let msg = DtxMessage {
            identifier: 1,
            conversation_idx: 0,
            channel_code: 4,
            expects_reply: false,
            payload: DtxPayload::Response(NSObject::Array(vec![
                NSObject::Int(2),
                NSObject::Array(vec![
                    NSObject::Int(1),
                    NSObject::Int(2),
                    NSObject::Int(3),
                    NSObject::Int(4),
                    NSObject::Int(5),
                    NSObject::Int(6),
                    NSObject::Int(7),
                    NSObject::Int(8),
                    NSObject::Int(9),
                    NSObject::Int(10),
                    NSObject::Int(11),
                ]),
            ])),
        };

        let event = parse_network_message(&msg).unwrap().unwrap();
        match event {
            NetworkMonitorEvent::ConnectionUpdate(event) => {
                assert_eq!(event.rx_packets, 1);
                assert_eq!(event.tx_bytes, 4);
                assert_eq!(event.connection_serial, 10);
                assert_eq!(event.time, 11);
            }
            other => panic!("unexpected event: {other:?}"),
        }
    }
}
