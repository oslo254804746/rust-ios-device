use anyhow::Result;
use ios_core::debugserver::{select_service_name, GdbRemoteClient};
use ios_core::MuxClient;
use ios_core::TunMode;
use ios_core::{
    connect, handshake_only_service_tls, start_lockdown_session, start_service_preserving_errors,
    ConnectOptions, LockdownError, PairRecord, LOCKDOWN_PORT,
};
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
                crate::output::outln!("{reply}");
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
    let mut last_missing = None;
    for service_name in candidate_service_names(preferred_service) {
        match start_service_preserving_errors(reader, writer, service_name).await {
            Ok((port, enable_ssl)) => return Ok((service_name, port, enable_ssl)),
            Err(err) if is_invalid_service(&err) => {
                tracing::debug!(
                    "debugserver service {} unavailable, trying fallback",
                    service_name
                );
                last_missing = Some(err);
            }
            Err(err) => return Err(err.into()),
        }
    }

    let context = format!(
        "debugserver is not exposed on this device via lockdown (tried {}, {})",
        ios_core::debugserver::LEGACY_SERVICE_NAME,
        ios_core::debugserver::SECURE_SERVICE_NAME
    );
    Err(match last_missing {
        Some(error) => anyhow::Error::new(error).context(context),
        None => anyhow::anyhow!(context),
    })
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

fn is_invalid_service(err: &LockdownError) -> bool {
    ios_core::LockdownResponseError::in_error_chain(err).is_some_and(|response| {
        response.operation() == "StartService" && response.code() == "InvalidService"
    })
}

#[cfg(test)]
mod tests {
    use clap::Parser;

    use super::DebugserverSub;

    #[tokio::test]
    async fn response_codes_debugserver_falls_back_only_for_exact_owned_code() {
        use ios_core::{recv_lockdown, send_lockdown, ErrorClass, LockdownResponseError};
        use tokio::io::AsyncReadExt;
        for preferred in [
            ios_core::debugserver::SECURE_SERVICE_NAME,
            ios_core::debugserver::LEGACY_SERVICE_NAME,
        ] {
            for codes in [
                vec![Some("InvalidService"), None],
                vec![Some("InvalidService"), Some("InvalidService")],
                vec![Some("owned-private-InvalidService")],
                vec![Some("MissingValue")],
            ] {
                let (host, mut peer) = tokio::io::duplex(4096);
                let (mut reader, mut writer) = tokio::io::split(host);
                let names = super::candidate_service_names(preferred);
                let caller = async {
                    let result =
                        super::start_debugserver_service(&mut reader, &mut writer, preferred).await;
                    drop(reader);
                    drop(writer);
                    result
                };
                let server = async {
                    for (index, code) in codes.iter().enumerate() {
                        let request: plist::Value = recv_lockdown(&mut peer).await.unwrap();
                        assert_eq!(
                            request.as_dictionary().unwrap()["Service"].as_string(),
                            Some(names[index])
                        );
                        let reply = if let Some(code) = code {
                            plist::Dictionary::from_iter([(
                                "Error".to_owned(),
                                plist::Value::String((*code).into()),
                            )])
                        } else {
                            plist::Dictionary::from_iter([
                                ("Port".to_owned(), plist::Value::Integer(1234.into())),
                                ("EnableServiceSSL".to_owned(), plist::Value::Boolean(true)),
                            ])
                        };
                        send_lockdown(&mut peer, &reply).await.unwrap();
                    }
                    let mut extra = Vec::new();
                    peer.read_to_end(&mut extra).await.unwrap();
                    assert!(extra.is_empty(), "unexpected fallback request");
                };
                let (result, ()) = tokio::time::timeout(std::time::Duration::from_secs(1), async {
                    tokio::join!(caller, server)
                })
                .await
                .unwrap();
                if codes.last().unwrap().is_none() {
                    assert_eq!(result.unwrap(), (names[1], 1234, true));
                } else {
                    let error = result.unwrap_err();
                    let observed = LockdownResponseError::in_error_chain(error.as_ref()).unwrap();
                    assert_eq!(observed.code(), codes.last().unwrap().unwrap());
                    assert_eq!(
                        ios_core::classify_error(error.as_ref()),
                        if codes.len() == 2 {
                            ErrorClass::ServiceMissing
                        } else {
                            ErrorClass::ServiceFailure
                        }
                    );
                    let envelope = crate::output::operation_failure_message(
                        crate::output::ErrorFormat::Json,
                        "debugserver",
                        &error,
                    )
                    .unwrap();
                    assert!(!envelope.contains("owned-private"));
                }
            }
        }
        assert!(!super::is_invalid_service(
            &ios_core::LockdownError::Protocol("InvalidService".into())
        ));
    }

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
