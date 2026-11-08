use anyhow::Result;

#[derive(clap::Args)]
pub struct ListCmd {}

impl ListCmd {
    pub async fn run(self, json: bool) -> Result<()> {
        let devices = ios_core::list_devices().await?;
        if devices.is_empty() {
            if json {
                crate::output::outln!("[]");
            } else {
                crate::output::outln!("No devices connected.");
            }
            return Ok(());
        }
        if json {
            let list: Vec<_> = devices
                .iter()
                .map(|d| {
                    serde_json::json!({
                        "udid": d.udid,
                        "device_id": d.device_id,
                        "connection_type": d.connection_type,
                        "product_id": d.product_id,
                    })
                })
                .collect();
            crate::output::outln!("{}", serde_json::to_string_pretty(&list)?);
        } else {
            for d in &devices {
                crate::output::outln!("{:<45} {} (id={})", d.udid, d.connection_type, d.device_id);
            }
        }
        Ok(())
    }
}
