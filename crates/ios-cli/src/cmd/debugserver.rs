use anyhow::Result;
use ios_core::debugserver::{select_service_name, GdbRemoteClient};
use ios_core::lockdown::pair_record::PairRecord;
use ios_core::lockdown::session::{
    handshake_only_service_tls, start_lockdown_session, start_service,
};
use ios_core::lockdown::LOCKDOWN_PORT;
use ios_core::mux::MuxClient;
use ios_core::tunnel::TunMode;
use ios_core::{connect, ConnectOptions};
use tokio::io::{AsyncRead, AsyncWrite};

#[derive(clap::Args)]
pub struct DebugserverCmd {
    #[command(subcommand)]
    sub: DebugserverSub,
}

#[derive(clap::Subcommand)]
enum DebugserverSub {
    /// Send a raw GDB remote packet and print the reply payload
    Send {
        #[arg(help = "Raw GDB remote payload, e.g. qSupported")]
        packet: String,
    },
}

impl DebugserverCmd {
    pub async fn run(self, udid: Option<String>) -> Result<()> {
        let udid = udid.ok_or_else(|| anyhow::anyhow!("--udid required for debugserver"))?;

        match self.sub {
            DebugserverSub::Send { packet } => {
                let stream = connect_debugserver(&udid).await?;
                let mut client = GdbRemoteClient::new(stream);
                let reply = client.request(&packet).await?;
                println!("{reply}");
            }
        }

        Ok(())
    }
}

trait DebugStream: AsyncRead + AsyncWrite + Unpin + Send {}
impl<T: AsyncRead + AsyncWrite + Unpin + Send> DebugStream for T {}

async fn connect_debugserver(udid: &str) -> Result<Box<dyn DebugStream>> {
    let opts = ConnectOptions {
        tun_mode: TunMode::Userspace,
        pair_record_path: None,
        skip_tunnel: true,
    };
    let device = connect(udid, opts).await?;
    let version = device.product_version().await?;
    let service_name = select_service_name(&version);
    let pair_record = PairRecord::load(udid)?;

    let mut lockdown_mux = MuxClient::connect().await?;
    lockdown_mux.read_pair_record(udid).await?;
    let lockdown_stream = lockdown_mux
        .connect_to_port(device.info.device_id, LOCKDOWN_PORT)
        .await?;
    let (_session_id, mut tls_reader, mut tls_writer) =
        start_lockdown_session(lockdown_stream, &pair_record).await?;
    let (resolved_service_name, port, enable_ssl) =
        start_debugserver_service(&mut tls_reader, &mut tls_writer, service_name).await?;

    tracing::info!(
        "debugserver start_service service={} port={} enable_ssl={}",
        resolved_service_name,
        port,
        enable_ssl
    );

    let svc_stream = MuxClient::connect()
        .await?
        .connect_to_port(device.info.device_id, port)
        .await?;

    if enable_ssl {
        let stream = handshake_only_service_tls(svc_stream, &pair_record, "debugserver").await?;
        return Ok(Box::new(stream));
    }

    Ok(Box::new(svc_stream))
}

async fn start_debugserver_service<R, W>(
    reader: &mut R,
    writer: &mut W,
    preferred_service: &str,
) -> Result<(&'static str, u16, bool)>
where
    R: AsyncRead + Unpin,
    W: AsyncWrite + Unpin,
{
    for service_name in candidate_service_names(preferred_service) {
        match start_service(reader, writer, service_name).await {
            Ok((port, enable_ssl)) => return Ok((service_name, port, enable_ssl)),
            Err(err) if is_invalid_service(&err) => {
                tracing::debug!(
                    "debugserver service {} unavailable, trying fallback",
                    service_name
                );
            }
            Err(err) => return Err(err.into()),
        }
    }

    Err(anyhow::anyhow!(
        "debugserver is not exposed on this device via lockdown (tried {}, {})",
        ios_core::debugserver::LEGACY_SERVICE_NAME,
        ios_core::debugserver::SECURE_SERVICE_NAME
    ))
}

fn candidate_service_names(preferred_service: &str) -> [&'static str; 2] {
    if preferred_service == ios_core::debugserver::SECURE_SERVICE_NAME {
        [
            ios_core::debugserver::SECURE_SERVICE_NAME,
            ios_core::debugserver::LEGACY_SERVICE_NAME,
        ]
    } else {
        [
            ios_core::debugserver::LEGACY_SERVICE_NAME,
            ios_core::debugserver::SECURE_SERVICE_NAME,
        ]
    }
}

fn is_invalid_service(err: &ios_core::lockdown::LockdownError) -> bool {
    matches!(err, ios_core::lockdown::LockdownError::Protocol(message) if message.contains("InvalidService"))
}

#[cfg(test)]
mod tests {
    use clap::Parser;

    use super::DebugserverSub;

    #[derive(Parser)]
    struct TestCli {
        #[command(subcommand)]
        command: DebugserverSub,
    }

    #[test]
    fn parses_debugserver_send_subcommand() {
        let cmd = TestCli::parse_from(["debugserver", "send", "qSupported"]);
        match cmd.command {
            DebugserverSub::Send { packet } => assert_eq!(packet, "qSupported"),
        }
    }

    #[test]
    fn secure_preference_falls_back_to_legacy() {
        assert_eq!(
            super::candidate_service_names(ios_core::debugserver::SECURE_SERVICE_NAME),
            [
                ios_core::debugserver::SECURE_SERVICE_NAME,
                ios_core::debugserver::LEGACY_SERVICE_NAME,
            ]
        );
    }
}
