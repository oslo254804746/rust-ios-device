use anyhow::Result;
use ios_core::instruments::{DeviceInfoClient, RunningProcess};

#[derive(clap::Args)]
pub struct MemlimitoffCmd {
    #[arg(
        value_name = "PID",
        required_unless_present = "process",
        conflicts_with = "process",
        help = "Process ID"
    )]
    pid: Option<u64>,
    #[arg(
        long,
        required_unless_present = "pid",
        conflicts_with = "pid",
        help = "Exact process name (must resolve to one running process)"
    )]
    process: Option<String>,
}

impl MemlimitoffCmd {
    pub async fn run(self, udid: Option<String>) -> Result<()> {
        let udid = udid.ok_or_else(|| anyhow::anyhow!("--udid required for memlimitoff"))?;
        let pid = match (self.pid, self.process) {
            (Some(pid), None) => {
                if pid == 0 {
                    return Err(anyhow::anyhow!("PID must be greater than zero"));
                }
                pid
            }
            (None, Some(name)) => {
                let name = name.trim();
                if name.is_empty() {
                    return Err(anyhow::anyhow!("--process must not be empty"));
                }
                let (_device, stream) = super::instruments::connect_instruments(&udid).await?;
                let mut info = DeviceInfoClient::connect(stream)
                    .await
                    .map_err(|err| anyhow::anyhow!("DeviceInfo error: {err}"))?;
                let processes = info
                    .running_processes()
                    .await
                    .map_err(|err| anyhow::anyhow!("runningProcesses error: {err}"))?;
                resolve_process_pid(&processes, name)?
            }
            _ => return Err(anyhow::anyhow!("provide exactly one of PID or --process")),
        };
        let (_device, stream) = super::instruments::connect_instruments(&udid).await?;
        let mut pc = ios_core::instruments::process_control::ProcessControl::connect(stream)
            .await
            .map_err(|err| anyhow::anyhow!("DTX error: {err}"))?;
        let disabled = pc
            .disable_memory_limit(pid)
            .await
            .map_err(|err| anyhow::anyhow!("disableMemoryLimit error: {err}"))?;
        if !disabled {
            return Err(anyhow::anyhow!(
                "device refused to disable memory limit for pid {}",
                pid
            ));
        }
        println!("Disabled memory limit for PID {pid}");
        Ok(())
    }
}

/// Resolve an exact process name without silently selecting an arbitrary
/// duplicate. `real_app_name` is accepted because iOS uses it for the app's
/// user-facing executable name on some releases.
pub(crate) fn resolve_process_pid(processes: &[RunningProcess], name: &str) -> Result<u64> {
    let mut matches = processes
        .iter()
        .filter(|process| process.name == name || process.real_app_name == name)
        .map(|process| process.pid)
        .filter(|pid| *pid != 0)
        .collect::<Vec<_>>();
    matches.sort_unstable();
    matches.dedup();
    match matches.as_slice() {
        [] => Err(anyhow::anyhow!("no running process named {name:?}")),
        [pid] => Ok(*pid),
        _ => Err(anyhow::anyhow!(
            "process name {name:?} matches multiple PIDs: {matches:?}"
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn process(pid: u64, name: &str, real_app_name: &str) -> RunningProcess {
        RunningProcess {
            pid,
            name: name.into(),
            real_app_name: real_app_name.into(),
            is_application: true,
        }
    }

    #[test]
    fn process_name_requires_exactly_one_match() {
        let processes = [process(42, "Safari", "MobileSafari")];
        assert_eq!(resolve_process_pid(&processes, "Safari").unwrap(), 42);
        assert_eq!(resolve_process_pid(&processes, "MobileSafari").unwrap(), 42);
        assert!(resolve_process_pid(&processes, "safari").is_err());
        assert!(resolve_process_pid(&[], "Safari").is_err());
    }

    #[test]
    fn duplicate_process_name_is_an_error() {
        let processes = [process(42, "worker", ""), process(43, "worker", "")];
        let error = resolve_process_pid(&processes, "worker").unwrap_err();
        assert!(error.to_string().contains("multiple PIDs"));
    }
}
