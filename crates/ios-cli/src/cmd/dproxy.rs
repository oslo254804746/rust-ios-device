use std::path::PathBuf;

use anyhow::{Context, Result};
use ios_core::lockdown::pair_record::PairRecord;
use ios_core::lockdown::protocol::{
    recv_lockdown, send_lockdown, QueryTypeRequest, QueryTypeResponse, StartServiceRequest,
    StartServiceResponse, StartSessionRequest, StartSessionResponse, LOCKDOWN_PORT,
};
use ios_core::lockdown::session::{strip_service_tls, wrap_service_tls};
use ios_core::mux::MuxClient;
use ios_core::tunnel::TunMode;
use ios_core::{connect, ConnectOptions, ServiceStream};
use tokio::io::{AsyncRead, AsyncWrite, BufReader, BufWriter};
use tokio::net::TcpListener;
use tokio_rustls::client::TlsStream;

#[derive(clap::Args)]
pub struct DproxyCmd {
    #[command(subcommand)]
    sub: DproxySub,
}

#[derive(clap::Subcommand)]
enum DproxySub {
    /// Proxy a single device service to a local TCP port while recording traffic
    Service {
        service: String,
        #[arg(long, default_value_t = 9100)]
        listen_port: u16,
        #[arg(short = 'H', long, default_value = "127.0.0.1")]
        host: String,
        #[arg(long, value_enum, default_value = "auto")]
        transport: TransportArg,
        #[arg(long, value_enum, default_value = "auto")]
        protocol: ProtocolArg,
        #[arg(long, default_value = "ios-rs-tmp/dproxy")]
        output: PathBuf,
        #[arg(long, help = "Accept a single client connection, then exit")]
        once: bool,
    },
}

#[derive(Clone, Copy, Debug, clap::ValueEnum, Eq, PartialEq)]
enum TransportArg {
    Auto,
    Lockdown,
    Rsd,
}

#[derive(Clone, Copy, Debug, clap::ValueEnum, Eq, PartialEq)]
enum ProtocolArg {
    Auto,
    Lockdown,
    Dtx,
    Xpc,
    Binary,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum TransportKind {
    Lockdown,
    Rsd,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum DproxyConnectPlan {
    LockdownOnly,
    TunnelOnly,
    Auto,
}

impl DproxyCmd {
    pub async fn run(self, udid: Option<String>) -> Result<()> {
        let udid = udid.ok_or_else(|| anyhow::anyhow!("--udid required for dproxy"))?;

        match self.sub {
            DproxySub::Service {
                service,
                listen_port,
                host,
                transport,
                protocol,
                output,
                once,
            } => {
                let (device, transport) = connect_dproxy_device(&udid, &service, transport).await?;
                let protocol = choose_protocol(&service, transport, protocol);
                let listener = TcpListener::bind(format!("{host}:{listen_port}"))
                    .await
                    .with_context(|| format!("failed to bind {host}:{listen_port}"))?;

                eprintln!(
                    "dproxy listening on {}:{} for {} via {:?} ({:?})",
                    host, listen_port, service, transport, protocol
                );

                let mut counter = 0usize;
                loop {
                    let (client, peer) = listener.accept().await?;
                    counter += 1;
                    let capture_dir = output.join(format!(
                        "{}-{}-{}",
                        sanitize_service_name(&service),
                        counter,
                        timestamp_ms()
                    ));
                    let mut recorder =
                        ios_core::services::dproxy::ProxyRecorder::new(&capture_dir, protocol)?;
                    eprintln!(
                        "accepted {} -> capture {}",
                        peer,
                        recorder.output_dir().display()
                    );

                    let remote = match transport {
                        TransportKind::Rsd => device.connect_rsd_service(&service).await?,
                        TransportKind::Lockdown => {
                            connect_lockdown_service(
                                &udid,
                                device.info.device_id,
                                &service,
                                &mut recorder,
                            )
                            .await?
                        }
                    };

                    ios_core::services::dproxy::proxy_bidirectional(client, remote, recorder)
                        .await?;

                    if once {
                        break;
                    }
                }
            }
        }

        Ok(())
    }
}

fn choose_transport(
    rsd: Option<&ios_core::xpc::RsdHandshake>,
    service: &str,
    arg: TransportArg,
) -> TransportKind {
    match arg {
        TransportArg::Lockdown => TransportKind::Lockdown,
        TransportArg::Rsd => TransportKind::Rsd,
        TransportArg::Auto => {
            if rsd.and_then(|value| value.get_port(service)).is_some() {
                TransportKind::Rsd
            } else {
                TransportKind::Lockdown
            }
        }
    }
}

fn choose_protocol(
    service: &str,
    transport: TransportKind,
    arg: ProtocolArg,
) -> ios_core::services::dproxy::ProxyProtocol {
    match arg {
        ProtocolArg::Lockdown => ios_core::services::dproxy::ProxyProtocol::Lockdown,
        ProtocolArg::Dtx => ios_core::services::dproxy::ProxyProtocol::Dtx,
        ProtocolArg::Xpc => ios_core::services::dproxy::ProxyProtocol::Xpc,
        ProtocolArg::Binary => ios_core::services::dproxy::ProxyProtocol::Binary,
        ProtocolArg::Auto => {
            if let Some(protocol) = protocol_override(service, transport) {
                return protocol;
            }

            if is_dtx_service(service) {
                ios_core::services::dproxy::ProxyProtocol::Dtx
            } else if transport == TransportKind::Rsd {
                ios_core::services::dproxy::ProxyProtocol::Xpc
            } else {
                ios_core::services::dproxy::ProxyProtocol::Binary
            }
        }
    }
}

async fn connect_dproxy_device(
    udid: &str,
    service: &str,
    transport: TransportArg,
) -> Result<(ios_core::device::ConnectedDevice, TransportKind)> {
    let product_major = probe_product_major(udid).await?;
    match dproxy_connect_plan(transport, product_major)? {
        DproxyConnectPlan::LockdownOnly => {
            let device = connect_lockdown_device(udid).await?;
            Ok((device, TransportKind::Lockdown))
        }
        DproxyConnectPlan::TunnelOnly => {
            let device = connect_tunnel_device(udid).await?;
            Ok((device, TransportKind::Rsd))
        }
        DproxyConnectPlan::Auto => match connect_tunnel_device(udid).await {
            Ok(device) => {
                let transport = choose_transport(device.rsd.as_ref(), service, TransportArg::Auto);
                if transport == TransportKind::Rsd {
                    Ok((device, transport))
                } else {
                    drop(device);
                    let device = connect_lockdown_device(udid).await?;
                    Ok((device, TransportKind::Lockdown))
                }
            }
            Err(err) => {
                tracing::info!(
                    "dproxy: tunnel unavailable for {udid}: {err}; falling back to lockdown"
                );
                let device = connect_lockdown_device(udid).await?;
                Ok((device, TransportKind::Lockdown))
            }
        },
    }
}

fn dproxy_connect_plan(transport: TransportArg, product_major: u64) -> Result<DproxyConnectPlan> {
    if product_major < 17 {
        return match transport {
            TransportArg::Rsd => Err(anyhow::anyhow!(
                "RSD transport requires iOS 17+; use --transport lockdown on older devices"
            )),
            TransportArg::Auto | TransportArg::Lockdown => Ok(DproxyConnectPlan::LockdownOnly),
        };
    }

    Ok(match transport {
        TransportArg::Lockdown => DproxyConnectPlan::LockdownOnly,
        TransportArg::Rsd => DproxyConnectPlan::TunnelOnly,
        TransportArg::Auto => DproxyConnectPlan::Auto,
    })
}

async fn probe_product_major(udid: &str) -> Result<u64> {
    let probe = connect(
        udid,
        ConnectOptions {
            tun_mode: TunMode::Userspace,
            pair_record_path: None,
            skip_tunnel: true,
        },
    )
    .await?;
    let version = probe.product_version().await?;
    Ok(version.major)
}

async fn connect_lockdown_device(udid: &str) -> Result<ios_core::device::ConnectedDevice> {
    connect(
        udid,
        ConnectOptions {
            tun_mode: TunMode::Userspace,
            pair_record_path: None,
            skip_tunnel: true,
        },
    )
    .await
    .map_err(Into::into)
}

async fn connect_tunnel_device(udid: &str) -> Result<ios_core::device::ConnectedDevice> {
    connect(
        udid,
        ConnectOptions {
            tun_mode: TunMode::Userspace,
            pair_record_path: None,
            skip_tunnel: false,
        },
    )
    .await
    .map_err(Into::into)
}

fn protocol_override(
    service: &str,
    transport: TransportKind,
) -> Option<ios_core::services::dproxy::ProxyProtocol> {
    match service {
        "com.apple.webinspector" => Some(ios_core::services::dproxy::ProxyProtocol::Lockdown),
        "com.apple.webinspector.shim.remote" if transport == TransportKind::Rsd => {
            Some(ios_core::services::dproxy::ProxyProtocol::Lockdown)
        }
        "com.apple.accessibility.axAuditDaemon.remoteserver.shim.remote"
        | "com.apple.dt.testmanagerd.remote"
        | "com.apple.dt.testmanagerd.remote.automation" => {
            Some(ios_core::services::dproxy::ProxyProtocol::Dtx)
        }
        _ => None,
    }
}

fn is_dtx_service(service: &str) -> bool {
    matches!(
        service,
        "com.apple.instruments.dtservicehub"
            | "com.apple.instruments.remoteserver"
            | "com.apple.instruments.remoteserver.DVTSecureSocketProxy"
            | "com.apple.testmanagerd.lockdown"
            | "com.apple.testmanagerd.lockdown.secure"
            | "com.apple.accessibility.axAuditDaemon.remoteserver"
    )
}

fn should_strip_service_ssl(service: &str) -> bool {
    matches!(
        service,
        "com.apple.instruments.remoteserver"
            | "com.apple.accessibility.axAuditDaemon.remoteserver"
            | "com.apple.testmanagerd.lockdown"
    )
}

fn sanitize_service_name(service: &str) -> String {
    service
        .chars()
        .map(|value| match value {
            'a'..='z' | 'A'..='Z' | '0'..='9' => value,
            _ => '-',
        })
        .collect()
}

fn timestamp_ms() -> u128 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|value| value.as_millis())
        .unwrap_or(0)
}

async fn connect_lockdown_service(
    udid: &str,
    device_id: u32,
    service_name: &str,
    recorder: &mut ios_core::services::dproxy::ProxyRecorder,
) -> Result<ServiceStream> {
    let pair_record = PairRecord::load(udid)?;

    let mut mux = MuxClient::connect().await?;
    mux.read_pair_record(udid).await?;
    let stream = mux.connect_to_port(device_id, LOCKDOWN_PORT).await?;

    let (_session_id, mut reader, mut writer) =
        start_lockdown_session_with_recording(stream, &pair_record, recorder).await?;
    let (port, enable_ssl) =
        start_service_with_recording(&mut reader, &mut writer, service_name, recorder).await?;

    let raw_service = MuxClient::connect()
        .await?
        .connect_to_port(device_id, port)
        .await?;

    if enable_ssl {
        let tls = wrap_service_tls(raw_service, &pair_record).await?;
        if should_strip_service_ssl(service_name) {
            let stream = strip_service_tls(tls)?;
            Ok(Box::new(stream))
        } else {
            Ok(Box::new(tls))
        }
    } else {
        Ok(Box::new(raw_service))
    }
}

async fn start_lockdown_session_with_recording<S>(
    stream: S,
    pair_record: &PairRecord,
    recorder: &mut ios_core::services::dproxy::ProxyRecorder,
) -> Result<(
    String,
    BufReader<tokio::io::ReadHalf<TlsStream<S>>>,
    BufWriter<tokio::io::WriteHalf<TlsStream<S>>>,
)>
where
    S: AsyncRead + AsyncWrite + Unpin + Send + 'static,
{
    let (reader_raw, writer_raw) = tokio::io::split(stream);
    let mut reader = BufReader::new(reader_raw);
    let mut writer = BufWriter::new(writer_raw);

    send_lockdown(
        &mut writer,
        &QueryTypeRequest {
            label: "ios-rs",
            request: "QueryType",
        },
    )
    .await?;
    recorder.record_meta_event(
        ios_core::services::dproxy::Direction::HostToDevice,
        "lockdown",
        "QueryType",
        serde_json::json!({
            "Label": "ios-rs",
            "Request": "QueryType",
        }),
    )?;
    let query_response: QueryTypeResponse = recv_lockdown(&mut reader).await?;
    recorder.record_meta_event(
        ios_core::services::dproxy::Direction::DeviceToHost,
        "lockdown",
        "QueryType",
        serde_json::json!({
            "Type": query_response.type_,
        }),
    )?;

    send_lockdown(
        &mut writer,
        &StartSessionRequest {
            label: "ios-rs",
            protocol_version: "2",
            request: "StartSession",
            host_id: pair_record.host_id.clone(),
            system_buid: pair_record.system_buid.clone(),
        },
    )
    .await?;
    recorder.record_meta_event(
        ios_core::services::dproxy::Direction::HostToDevice,
        "lockdown",
        "StartSession",
        serde_json::json!({
            "Label": "ios-rs",
            "ProtocolVersion": "2",
            "Request": "StartSession",
            "HostID": pair_record.host_id.clone(),
            "SystemBUID": pair_record.system_buid.clone(),
        }),
    )?;

    let session_response: StartSessionResponse = recv_lockdown(&mut reader).await?;
    recorder.record_meta_event(
        ios_core::services::dproxy::Direction::DeviceToHost,
        "lockdown",
        "StartSession",
        serde_json::json!({
            "SessionID": session_response.session_id,
            "EnableSessionSSL": session_response.enable_session_ssl,
        }),
    )?;

    let stream = reader.into_inner().unsplit(writer.into_inner());
    let tls_stream = wrap_service_tls(stream, pair_record).await?;
    let (tls_reader, tls_writer) = tokio::io::split(tls_stream);
    Ok((
        session_response.session_id,
        BufReader::new(tls_reader),
        BufWriter::new(tls_writer),
    ))
}

async fn start_service_with_recording<R, W>(
    reader: &mut R,
    writer: &mut W,
    service_name: &str,
    recorder: &mut ios_core::services::dproxy::ProxyRecorder,
) -> Result<(u16, bool)>
where
    R: AsyncRead + Unpin,
    W: AsyncWrite + Unpin,
{
    send_lockdown(
        writer,
        &StartServiceRequest {
            label: "ios-rs",
            request: "StartService",
            service: service_name.to_string(),
        },
    )
    .await?;
    recorder.record_meta_event(
        ios_core::services::dproxy::Direction::HostToDevice,
        "lockdown",
        format!("StartService {service_name}"),
        serde_json::json!({
            "Label": "ios-rs",
            "Request": "StartService",
            "Service": service_name,
        }),
    )?;

    let response: StartServiceResponse = recv_lockdown(reader).await?;
    recorder.record_meta_event(
        ios_core::services::dproxy::Direction::DeviceToHost,
        "lockdown",
        format!("StartService {service_name}"),
        serde_json::json!({
            "Port": response.port,
            "EnableServiceSSL": response.enable_service_ssl,
            "Error": response.error,
        }),
    )?;

    if let Some(error) = response.error {
        return Err(anyhow::anyhow!(
            "StartService '{service_name}' failed: {error}"
        ));
    }

    let port = response
        .port
        .ok_or_else(|| anyhow::anyhow!("StartService '{service_name}' missing port"))?;
    Ok((port, response.enable_service_ssl.unwrap_or(false)))
}

#[cfg(test)]
mod tests {
    use clap::Parser;

    use super::*;

    #[derive(Parser)]
    struct TestCli {
        #[command(flatten)]
        command: DproxyCmd,
    }

    #[test]
    fn parses_dproxy_service_subcommand() {
        let parsed = TestCli::try_parse_from([
            "dproxy",
            "service",
            "com.apple.instruments.dtservicehub",
            "--listen-port",
            "9100",
        ]);
        assert!(parsed.is_ok(), "dproxy service should parse");
    }

    #[test]
    fn auto_protocol_prefers_dtx_for_instruments() {
        let protocol = choose_protocol(
            "com.apple.instruments.dtservicehub",
            TransportKind::Rsd,
            ProtocolArg::Auto,
        );
        assert_eq!(protocol, ios_core::services::dproxy::ProxyProtocol::Dtx);
    }

    #[test]
    fn auto_protocol_overrides_known_non_xpc_rsd_services() {
        assert_eq!(
            choose_protocol(
                "com.apple.webinspector",
                TransportKind::Lockdown,
                ProtocolArg::Auto,
            ),
            ios_core::services::dproxy::ProxyProtocol::Lockdown
        );
        assert_eq!(
            choose_protocol(
                "com.apple.webinspector.shim.remote",
                TransportKind::Rsd,
                ProtocolArg::Auto,
            ),
            ios_core::services::dproxy::ProxyProtocol::Lockdown
        );
        assert_eq!(
            choose_protocol(
                "com.apple.accessibility.axAuditDaemon.remoteserver.shim.remote",
                TransportKind::Rsd,
                ProtocolArg::Auto,
            ),
            ios_core::services::dproxy::ProxyProtocol::Dtx
        );
        assert_eq!(
            choose_protocol(
                "com.apple.dt.testmanagerd.remote",
                TransportKind::Rsd,
                ProtocolArg::Auto,
            ),
            ios_core::services::dproxy::ProxyProtocol::Dtx
        );
        assert_eq!(
            choose_protocol(
                "com.apple.dt.testmanagerd.remote.automation",
                TransportKind::Rsd,
                ProtocolArg::Auto,
            ),
            ios_core::services::dproxy::ProxyProtocol::Dtx
        );
    }

    #[test]
    fn strip_ssl_selection_matches_legacy_lockdown_dtx_services() {
        assert!(should_strip_service_ssl(
            "com.apple.instruments.remoteserver"
        ));
        assert!(should_strip_service_ssl(
            "com.apple.accessibility.axAuditDaemon.remoteserver"
        ));
        assert!(should_strip_service_ssl("com.apple.testmanagerd.lockdown"));
        assert!(!should_strip_service_ssl(
            "com.apple.instruments.remoteserver.DVTSecureSocketProxy"
        ));
        assert!(!should_strip_service_ssl(
            "com.apple.testmanagerd.lockdown.secure"
        ));
    }

    #[test]
    fn sanitize_service_name_replaces_separators() {
        assert_eq!(
            sanitize_service_name("com.apple.instruments.dtservicehub"),
            "com-apple-instruments-dtservicehub"
        );
    }

    #[test]
    fn connect_plan_uses_lockdown_for_pre_ios17_devices() {
        assert_eq!(
            dproxy_connect_plan(TransportArg::Auto, 15).unwrap(),
            DproxyConnectPlan::LockdownOnly
        );
        assert_eq!(
            dproxy_connect_plan(TransportArg::Lockdown, 15).unwrap(),
            DproxyConnectPlan::LockdownOnly
        );
    }

    #[test]
    fn connect_plan_rejects_rsd_on_pre_ios17_devices() {
        let err = dproxy_connect_plan(TransportArg::Rsd, 15).unwrap_err();
        assert_eq!(
            err.to_string(),
            "RSD transport requires iOS 17+; use --transport lockdown on older devices"
        );
    }

    #[test]
    fn connect_plan_distinguishes_ios17_plus_transports() {
        assert_eq!(
            dproxy_connect_plan(TransportArg::Lockdown, 17).unwrap(),
            DproxyConnectPlan::LockdownOnly
        );
        assert_eq!(
            dproxy_connect_plan(TransportArg::Rsd, 17).unwrap(),
            DproxyConnectPlan::TunnelOnly
        );
        assert_eq!(
            dproxy_connect_plan(TransportArg::Auto, 17).unwrap(),
            DproxyConnectPlan::Auto
        );
    }
}
