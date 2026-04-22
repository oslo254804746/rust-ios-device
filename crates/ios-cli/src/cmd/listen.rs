use anyhow::Result;
use tokio_stream::StreamExt;

#[derive(clap::Args)]
pub struct ListenCmd {}

impl ListenCmd {
    pub async fn run(self, json: bool) -> Result<()> {
        let events = ios_core::watch_devices().await?;
        tokio::pin!(events);

        while let Some(event) = events.next().await {
            let event = event?;
            if json {
                println!("{}", serde_json::to_string(&event_to_json(&event))?);
            } else {
                print_event(&event);
            }
        }

        Ok(())
    }
}

fn event_to_json(event: &ios_core::DeviceEvent) -> serde_json::Value {
    match event {
        ios_core::DeviceEvent::Attached(device) => serde_json::json!({
            "event": "attached",
            "udid": device.udid,
            "device_id": device.device_id,
            "connection_type": device.connection_type,
            "product_id": device.product_id,
        }),
        ios_core::DeviceEvent::Detached { udid, device_id } => serde_json::json!({
            "event": "detached",
            "udid": udid,
            "device_id": device_id,
        }),
    }
}

fn print_event(event: &ios_core::DeviceEvent) {
    match event {
        ios_core::DeviceEvent::Attached(device) => {
            println!(
                "attached udid={} connection_type={} device_id={} product_id={}",
                device.udid, device.connection_type, device.device_id, device.product_id
            );
        }
        ios_core::DeviceEvent::Detached { udid, device_id } => {
            if udid.is_empty() {
                println!("detached device_id={device_id}");
            } else {
                println!("detached udid={udid} device_id={device_id}");
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn attached_event_renders_as_json() {
        let event = ios_core::DeviceEvent::Attached(ios_core::DeviceInfo {
            udid: "00008150-000A584C0E62401C".into(),
            device_id: 2,
            connection_type: "USB".into(),
            product_id: 0,
        });

        assert_eq!(
            event_to_json(&event),
            serde_json::json!({
                "event": "attached",
                "udid": "00008150-000A584C0E62401C",
                "device_id": 2,
                "connection_type": "USB",
                "product_id": 0,
            })
        );
    }

    #[test]
    fn detached_event_renders_as_json() {
        let event = ios_core::DeviceEvent::Detached {
            udid: "00008150-000A584C0E62401C".into(),
            device_id: 7,
        };
        assert_eq!(
            event_to_json(&event),
            serde_json::json!({
                "event": "detached",
                "udid": "00008150-000A584C0E62401C",
                "device_id": 7,
            })
        );
    }
}
