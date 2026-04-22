use anyhow::Result;

#[derive(clap::Args)]
pub struct MemlimitoffCmd {
    #[arg(help = "Process ID")]
    pid: u64,
}

impl MemlimitoffCmd {
    pub async fn run(self, udid: Option<String>) -> Result<()> {
        let udid = udid.ok_or_else(|| anyhow::anyhow!("--udid required for memlimitoff"))?;
        let (_device, stream) = super::instruments::connect_instruments(&udid).await?;
        let mut pc = ios_services::instruments::process_control::ProcessControl::connect(stream)
            .await
            .map_err(|err| anyhow::anyhow!("DTX error: {err}"))?;
        let disabled = pc
            .disable_memory_limit(self.pid)
            .await
            .map_err(|err| anyhow::anyhow!("disableMemoryLimit error: {err}"))?;
        if !disabled {
            return Err(anyhow::anyhow!(
                "device refused to disable memory limit for pid {}",
                self.pid
            ));
        }
        println!("Disabled memory limit for PID {}", self.pid);
        Ok(())
    }
}
