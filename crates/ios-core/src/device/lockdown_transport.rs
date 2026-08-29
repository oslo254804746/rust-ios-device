pub async fn connect(udid: &str, opts: ConnectOptions) -> Result<ConnectedDevice, CoreError> {
    let mut mux = MuxClient::connect().await?;
    let devices = mux.list_devices().await?;
    let dev = select_mux_device(devices, udid)
        .ok_or_else(|| CoreError::DeviceNotFound(udid.to_string()))?;

    let info = DeviceInfo {
        udid: dev.serial_number.clone(),
        device_id: dev.device_id,
        connection_type: dev.connection_type.clone(),
        product_id: dev.product_id,
    };

    let pair_record = load_pair_record(udid, opts.pair_record_path.as_deref())?;
    connect_via_lockdown_transport(
        info,
        pair_record,
        LockdownTransport::Usbmux {
            device_id: dev.device_id,
        },
        opts,
    )
    .await
}

pub async fn connect_direct_usb_tunnel(
    udid: &str,
    rsd_ip: Option<&str>,
    opts: ConnectOptions,
) -> Result<ConnectedDevice, CoreError> {
    let mut mux = MuxClient::connect().await?;
    let devices = mux.list_devices().await?;
    let dev = select_mux_device(devices, udid)
        .ok_or_else(|| CoreError::DeviceNotFound(udid.to_string()))?;
    let pair_record = try_load_pair_record(udid, opts.pair_record_path.as_deref());
    let info = DeviceInfo {
        udid: dev.serial_number.clone(),
        device_id: dev.device_id,
        connection_type: dev.connection_type.clone(),
        product_id: dev.product_id,
    };
    let lockdown_transport = LockdownTransport::Usbmux {
        device_id: dev.device_id,
    };

    if opts.skip_tunnel {
        let pair_record =
            require_pair_record(pair_record, udid, "direct USB lockdown access requires")?;
        return Ok(ConnectedDevice {
            info,
            tunnel: None,
            rsd: None,
            pair_record: Some(pair_record),
            lockdown_transport,
        });
    }

    #[cfg(not(all(feature = "tunnel", feature = "mdns")))]
    {
        let _ = rsd_ip;
        Err(CoreError::Unsupported(
            "direct USB tunnel support requires ios-core features 'tunnel' and 'mdns'".into(),
        ))
    }

    #[cfg(all(feature = "tunnel", feature = "mdns"))]
    {
        let targets = discover_direct_rsd_targets(udid, rsd_ip).await?;
        if targets.is_empty() {
            return Err(CoreError::Unsupported(format!(
                "no _remoted target matched udid={udid} ip={rsd_ip:?}"
            )));
        }

        let mut last_error = None;
        for target in targets {
            match connect_via_direct_rsd_target(
                info.clone(),
                pair_record.clone(),
                lockdown_transport.clone(),
                opts.clone(),
                target,
            )
            .await
            {
                Ok(device) => return Ok(device),
                Err(err) => last_error = Some(err),
            }
        }

        Err(last_error.unwrap_or_else(|| {
            CoreError::Unsupported(format!(
                "no direct RSD target produced a tunnel for udid={udid}"
            ))
        }))
    }
}

pub async fn connect_remote_pairing_tunnel(
    udid: &str,
    host: Option<&str>,
    opts: ConnectOptions,
) -> Result<ConnectedDevice, CoreError> {
    let pair_record = try_load_pair_record(udid, opts.pair_record_path.as_deref());
    let info = DeviceInfo {
        udid: udid.to_string(),
        device_id: 0,
        connection_type: "Network".into(),
        product_id: 0,
    };

    if opts.skip_tunnel {
        let pair_record =
            require_pair_record(pair_record, udid, "remote pairing lockdown access requires")?;
        return Ok(ConnectedDevice {
            info,
            tunnel: None,
            rsd: None,
            pair_record: Some(pair_record),
            lockdown_transport: LockdownTransport::Tcp {
                host: host.unwrap_or_default().to_string(),
            },
        });
    }

    #[cfg(not(all(feature = "tunnel", feature = "mdns")))]
    {
        let _ = host;
        Err(CoreError::Unsupported(
            "remote pairing tunnel support requires ios-core features 'tunnel' and 'mdns'".into(),
        ))
    }

    #[cfg(all(feature = "tunnel", feature = "mdns"))]
    {
        let targets = discover_remote_pairing_targets(udid, host).await?;
        if targets.is_empty() {
            return Err(CoreError::Unsupported(format!(
                "no _remotepairing target matched udid={udid} host={host:?}"
            )));
        }

        let mut last_error = None;
        for (remote_host, port) in targets {
            match connect_via_remote_pairing_target(
                info.clone(),
                pair_record.clone(),
                opts.clone(),
                udid,
                &remote_host,
                port,
            )
            .await
            {
                Ok(device) => return Ok(device),
                Err(err) => last_error = Some(err),
            }
        }

        Err(last_error.unwrap_or_else(|| {
            CoreError::Unsupported(format!(
                "no remote pairing target produced a tunnel for udid={udid}"
            ))
        }))
    }
}

pub async fn connect_tcp_lockdown_tunnel(
    udid: &str,
    host: &str,
    opts: ConnectOptions,
) -> Result<ConnectedDevice, CoreError> {
    let pair_record = load_pair_record(udid, opts.pair_record_path.as_deref())?;
    let info = DeviceInfo {
        udid: udid.to_string(),
        device_id: 0,
        connection_type: "Network".into(),
        product_id: 0,
    };
    connect_via_lockdown_transport(
        info,
        pair_record,
        LockdownTransport::Tcp {
            host: host.to_string(),
        },
        opts,
    )
    .await
}

#[cfg(feature = "mdns")]
pub async fn discover_paired_mobdev2_devices() -> Result<Vec<PairedMobdev2Device>, CoreError> {
    let wifi_mac_to_udid = tokio::task::spawn_blocking(load_wifi_mac_pairings)
        .await
        .map_err(|e| CoreError::Other(format!("join error: {e}")))??;
    let services = browse_mobdev2(MOBDEV2_DISCOVERY_TIMEOUT).await?;
    Ok(match_paired_mobdev2_targets(&services, &wifi_mac_to_udid))
}

fn select_mux_device(
    devices: Vec<crate::mux::MuxDevice>,
    udid: &str,
) -> Option<crate::mux::MuxDevice> {
    let mut fallback = None;

    for device in devices {
        if device.serial_number != udid {
            continue;
        }

        let is_usb = device.connection_type.eq_ignore_ascii_case("USB");
        fallback = Some(device);

        if is_usb {
            return fallback;
        }
    }

    fallback
}

fn load_pair_record(
    udid: &str,
    pair_record_path: Option<&std::path::Path>,
) -> Result<Arc<PairRecord>, CoreError> {
    Ok(Arc::new(if let Some(path) = pair_record_path {
        PairRecord::load_from_path(path, udid)?
    } else {
        PairRecord::load(udid)?
    }))
}

fn try_load_pair_record(
    udid: &str,
    pair_record_path: Option<&std::path::Path>,
) -> Option<Arc<PairRecord>> {
    load_pair_record(udid, pair_record_path).ok()
}

fn require_pair_record(
    pair_record: Option<Arc<PairRecord>>,
    udid: &str,
    context: &str,
) -> Result<Arc<PairRecord>, CoreError> {
    pair_record.ok_or_else(|| {
        CoreError::Unsupported(format!("{context} a lockdown pair record for {udid}"))
    })
}

async fn connect_lockdown_port(
    udid: &str,
    transport: &LockdownTransport,
    port: u16,
    read_pair_record: bool,
) -> Result<ServiceStream, CoreError> {
    match transport {
        LockdownTransport::Usbmux { device_id } => {
            let mut mux = MuxClient::connect().await?;
            if read_pair_record {
                mux.read_pair_record(udid).await?;
            }
            let stream = mux.connect_to_port(*device_id, port).await?;
            Ok(Box::new(stream))
        }
        LockdownTransport::Tcp { host, .. } => {
            let stream = tokio::time::timeout(
                crate::tunnel::TUNNEL_CONNECT_TIMEOUT,
                TcpStream::connect((host.as_str(), port)),
            )
            .await
            .map_err(|_| {
                std::io::Error::new(
                    std::io::ErrorKind::TimedOut,
                    format!("lockdown dial to {host}:{port} timed out"),
                )
            })??;
            Ok(Box::new(stream))
        }
    }
}

async fn connect_via_lockdown_transport(
    info: DeviceInfo,
    pair_record: Arc<PairRecord>,
    lockdown_transport: LockdownTransport,
    opts: ConnectOptions,
) -> Result<ConnectedDevice, CoreError> {
    if opts.skip_tunnel {
        return Ok(ConnectedDevice {
            info,
            tunnel: None,
            rsd: None,
            pair_record: Some(pair_record),
            lockdown_transport,
        });
    }

    #[cfg(not(feature = "tunnel"))]
    {
        let _ = (info, pair_record, lockdown_transport);
        Err(CoreError::Unsupported(
            "CoreDevice tunnel support requires ios-core feature 'tunnel'".into(),
        ))
    }

    #[cfg(feature = "tunnel")]
    {
        let lockdown_stream =
            connect_lockdown_port(&info.udid, &lockdown_transport, LOCKDOWN_PORT, true).await?;

        tracing::info!("tunnel connect: starting lockdown session");
        let (_session_id, mut tls_reader, mut tls_writer) =
            start_lockdown_session(lockdown_stream, &pair_record).await?;
        tracing::info!("tunnel connect: lockdown session established");

        tracing::info!("tunnel connect: requesting CoreDeviceProxy");
        let (service_port, enable_service_ssl) =
            start_service(&mut tls_reader, &mut tls_writer, CORE_DEVICE_PROXY).await?;
        tracing::info!(
        "tunnel connect: CoreDeviceProxy started on port {service_port} (ssl={enable_service_ssl})"
    );

        let proxy_stream_raw =
            connect_lockdown_port(&info.udid, &lockdown_transport, service_port, false).await?;

        let proxy_stream = if enable_service_ssl {
            tracing::info!("tunnel connect: wrapping CoreDeviceProxy with TLS");
            ProxyStream::Tls(Box::new(
                wrap_service_tls(proxy_stream_raw, &pair_record).await?,
            ))
        } else {
            tracing::info!("tunnel connect: CoreDeviceProxy is plaintext");
            ProxyStream::Plain(proxy_stream_raw)
        };
        tracing::info!("tunnel connect: CoreDeviceProxy stream ready");

        activate_tunnel(
            TunnelConnection::new(
                info,
                Some(pair_record),
                lockdown_transport,
                "lockdown CoreDeviceProxy",
            ),
            proxy_stream,
            opts.tun_mode,
        )
        .await
    }
}

#[cfg(not(feature = "tunnel"))]
fn tunnel_unavailable() -> CoreError {
    CoreError::Unsupported("CoreDevice tunnel support requires ios-core feature 'tunnel'".into())
}
