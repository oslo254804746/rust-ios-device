#[cfg(feature = "tunnel")]
struct GuardedTunnelStream<G> {
    stream: tokio_openssl::SslStream<TcpStream>,
    _guard: G,
}

#[cfg(feature = "tunnel")]
impl<G> Unpin for GuardedTunnelStream<G> {}

#[cfg(feature = "tunnel")]
impl<G> AsyncRead for GuardedTunnelStream<G> {
    fn poll_read(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<std::io::Result<()>> {
        Pin::new(&mut self.get_mut().stream).poll_read(cx, buf)
    }
}

#[cfg(feature = "tunnel")]
impl<G> AsyncWrite for GuardedTunnelStream<G> {
    fn poll_write(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<std::io::Result<usize>> {
        Pin::new(&mut self.get_mut().stream).poll_write(cx, buf)
    }

    fn poll_flush(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        Pin::new(&mut self.get_mut().stream).poll_flush(cx)
    }

    fn poll_shutdown(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        Pin::new(&mut self.get_mut().stream).poll_shutdown(cx)
    }
}

#[cfg(feature = "tunnel")]
struct LoadedRemotePairingCredentials {
    host_identity: HostIdentity,
}

#[cfg(feature = "tunnel")]
struct RemotePairingControlChannel {
    stream: TcpStream,
}

#[cfg(feature = "tunnel")]
impl RemotePairingControlChannel {
    async fn connect(host: &str, port: u16) -> Result<Self, CoreError> {
        Ok(Self {
            stream: TcpStream::connect((host, port)).await?,
        })
    }

    async fn send(&mut self, payload: &serde_json::Value) -> Result<(), CoreError> {
        use tokio::io::AsyncWriteExt;

        let body = serde_json::to_vec(payload)?;
        if body.len() > u16::MAX as usize {
            return Err(CoreError::Protocol(format!(
                "remote pairing payload too large: {} bytes",
                body.len()
            )));
        }

        self.stream.write_all(b"RPPairing").await?;
        self.stream
            .write_all(&(body.len() as u16).to_be_bytes())
            .await?;
        self.stream.write_all(&body).await?;
        self.stream.flush().await?;
        Ok(())
    }

    async fn recv(&mut self) -> Result<serde_json::Value, CoreError> {
        use tokio::io::AsyncReadExt;

        let mut magic = [0u8; 9];
        self.stream.read_exact(&mut magic).await?;
        if &magic != b"RPPairing" {
            return Err(CoreError::Protocol(format!(
                "invalid RPPairing magic: {magic:?}"
            )));
        }

        let mut length = [0u8; 2];
        self.stream.read_exact(&mut length).await?;
        let body_len = u16::from_be_bytes(length) as usize;
        let mut body = vec![0u8; body_len];
        self.stream.read_exact(&mut body).await?;
        Ok(serde_json::from_slice(&body)?)
    }
}

#[cfg(all(feature = "tunnel", feature = "mdns"))]
async fn discover_direct_rsd_targets(
    udid: &str,
    ip_filter: Option<&str>,
) -> Result<Vec<MdnsDevice>, CoreError> {
    let stream = crate::discovery::discover_mdns().await?;
    tokio::pin!(stream);

    let deadline = Instant::now() + DIRECT_RSD_DISCOVERY_TIMEOUT;
    let mut targets = Vec::new();
    let mut seen = std::collections::HashSet::new();

    loop {
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            break;
        }

        match tokio::time::timeout(remaining, stream.next()).await {
            Ok(Some(device)) => {
                let ip = device.ipv6.to_string();
                if ip_filter.map(|filter| filter != ip).unwrap_or(false) {
                    continue;
                }

                let key = (device.ipv6, device.rsd_port);
                if !seen.insert(key) {
                    continue;
                }

                targets.push(device);
            }
            Ok(None) | Err(_) => break,
        }
    }

    targets.sort_by_key(|device| {
        if device.udid == udid {
            0
        } else if device.udid.is_empty() {
            1
        } else {
            2
        }
    });
    Ok(targets)
}

#[cfg(all(feature = "tunnel", feature = "mdns"))]
async fn discover_remote_pairing_targets(
    udid: &str,
    host_filter: Option<&str>,
) -> Result<Vec<(String, u16)>, CoreError> {
    let services = browse_remotepairing(MOBDEV2_DISCOVERY_TIMEOUT).await?;
    let mut targets = Vec::new();
    let mut seen = std::collections::HashSet::new();

    for service in services {
        let Some(host) = preferred_lockdown_address(&service.addresses) else {
            continue;
        };
        if host_filter.map(|filter| filter != host).unwrap_or(false) {
            continue;
        }

        let key = (host.to_string(), service.port);
        if seen.insert(key.clone()) {
            targets.push(key);
        }
    }

    if targets.is_empty() {
        return Err(CoreError::Unsupported(format!(
            "no browse_remotepairing target matched udid={udid} host={host_filter:?}"
        )));
    }

    Ok(targets)
}

#[cfg(all(feature = "tunnel", feature = "mdns"))]
async fn connect_via_direct_rsd_target(
    info: DeviceInfo,
    pair_record: Option<Arc<PairRecord>>,
    lockdown_transport: LockdownTransport,
    opts: ConnectOptions,
    target: MdnsDevice,
) -> Result<ConnectedDevice, CoreError> {
    let rsd = rsd_handshake(target.ipv6, target.rsd_port).await?;
    if rsd.udid != info.udid {
        return Err(CoreError::Protocol(format!(
            "direct RSD target {} resolved to unexpected udid {}",
            target.ipv6, rsd.udid
        )));
    }

    let service_port = rsd
        .get_port(crate::pairing_transport::UNTRUSTED_SERVICE_NAME)
        .ok_or_else(|| {
            CoreError::Unsupported(format!(
                "direct RSD target {} does not expose {}",
                target.ipv6,
                crate::pairing_transport::UNTRUSTED_SERVICE_NAME
            ))
        })?;
    let mut direct_stream = establish_direct_tunnel_stream(target.ipv6, service_port).await?;

    let tunnel_info = crate::tunnel::handshake::exchange_tunnel_parameters_with_timeout(
        &mut direct_stream,
        TUNNEL_HANDSHAKE_TIMEOUT,
    )
    .await
    .map_err(CoreError::Tunnel)?;

    match opts.tun_mode {
        TunMode::Kernel => {
            #[cfg(not(feature = "tunnel-kernel"))]
            {
                return Err(CoreError::Unsupported(
                    "kernel TUN support requires ios-core feature 'tunnel-kernel'".into(),
                ));
            }
            #[cfg(feature = "tunnel-kernel")]
            {
                let (handle, cancel_rx) =
                    TunnelHandle::new(info.udid.clone(), tunnel_info.clone(), None);
                let tun =
                    KernelTunDevice::create(&tunnel_info.client_address, tunnel_info.client_mtu)
                        .await
                        .map_err(CoreError::Tunnel)?;
                let mtu = tunnel_info.client_mtu;
                tokio::spawn(async move {
                    if let Err(err) = forward_packets(direct_stream, tun, mtu, cancel_rx).await {
                        tracing::error!("direct kernel TUN forward: {err}");
                    }
                });
                let rsd =
                    attempt_rsd(&tunnel_info.server_address, tunnel_info.server_rsd_port).await;
                Ok(ConnectedDevice {
                    info,
                    tunnel: Some(Arc::new(handle)),
                    rsd,
                    pair_record,
                    lockdown_transport,
                })
            }
        }
        TunMode::Userspace => {
            #[cfg(not(feature = "tunnel-userspace"))]
            {
                return Err(CoreError::Unsupported(
                    "userspace tunnel support requires ios-core feature 'tunnel-userspace'".into(),
                ));
            }
            #[cfg(feature = "tunnel-userspace")]
            {
                let userspace = UserspaceTunDevice::start(
                    &tunnel_info.client_address,
                    &tunnel_info.server_address,
                    tunnel_info.client_mtu,
                    direct_stream,
                )
                .await
                .map_err(CoreError::Tunnel)?;

                let proxy_port = userspace.local_port;
                let handle =
                    TunnelHandle::new_userspace(info.udid.clone(), tunnel_info.clone(), userspace);
                let rsd = attempt_rsd_via_proxy(
                    proxy_port,
                    &tunnel_info.server_address,
                    tunnel_info.server_rsd_port,
                )
                .await;
                Ok(ConnectedDevice {
                    info,
                    tunnel: Some(Arc::new(handle)),
                    rsd,
                    pair_record,
                    lockdown_transport,
                })
            }
        }
    }
}

#[cfg(all(feature = "tunnel", feature = "mdns"))]
async fn connect_via_remote_pairing_target(
    info: DeviceInfo,
    pair_record: Option<Arc<PairRecord>>,
    opts: ConnectOptions,
    remote_identifier: &str,
    host: &str,
    port: u16,
) -> Result<ConnectedDevice, CoreError> {
    let mut remote_stream =
        establish_remote_pairing_tunnel_stream(remote_identifier, host, port).await?;

    let tunnel_info = crate::tunnel::handshake::exchange_tunnel_parameters_with_timeout(
        &mut remote_stream,
        TUNNEL_HANDSHAKE_TIMEOUT,
    )
    .await
    .map_err(CoreError::Tunnel)?;

    match opts.tun_mode {
        TunMode::Kernel => {
            #[cfg(not(feature = "tunnel-kernel"))]
            {
                return Err(CoreError::Unsupported(
                    "kernel TUN support requires ios-core feature 'tunnel-kernel'".into(),
                ));
            }
            #[cfg(feature = "tunnel-kernel")]
            {
                let (handle, cancel_rx) =
                    TunnelHandle::new(info.udid.clone(), tunnel_info.clone(), None);
                let tun =
                    KernelTunDevice::create(&tunnel_info.client_address, tunnel_info.client_mtu)
                        .await
                        .map_err(CoreError::Tunnel)?;
                let mtu = tunnel_info.client_mtu;
                tokio::spawn(async move {
                    if let Err(err) = forward_packets(remote_stream, tun, mtu, cancel_rx).await {
                        tracing::error!("remote pairing kernel TUN forward: {err}");
                    }
                });
                let rsd =
                    attempt_rsd(&tunnel_info.server_address, tunnel_info.server_rsd_port).await;
                Ok(ConnectedDevice {
                    info,
                    tunnel: Some(Arc::new(handle)),
                    rsd,
                    pair_record,
                    lockdown_transport: LockdownTransport::Tcp {
                        host: host.to_string(),
                    },
                })
            }
        }
        TunMode::Userspace => {
            #[cfg(not(feature = "tunnel-userspace"))]
            {
                return Err(CoreError::Unsupported(
                    "userspace tunnel support requires ios-core feature 'tunnel-userspace'".into(),
                ));
            }
            #[cfg(feature = "tunnel-userspace")]
            {
                let userspace = UserspaceTunDevice::start(
                    &tunnel_info.client_address,
                    &tunnel_info.server_address,
                    tunnel_info.client_mtu,
                    remote_stream,
                )
                .await
                .map_err(CoreError::Tunnel)?;

                let proxy_port = userspace.local_port;
                let handle =
                    TunnelHandle::new_userspace(info.udid.clone(), tunnel_info.clone(), userspace);
                let rsd = attempt_rsd_via_proxy(
                    proxy_port,
                    &tunnel_info.server_address,
                    tunnel_info.server_rsd_port,
                )
                .await;
                Ok(ConnectedDevice {
                    info,
                    tunnel: Some(Arc::new(handle)),
                    rsd,
                    pair_record,
                    lockdown_transport: LockdownTransport::Tcp {
                        host: host.to_string(),
                    },
                })
            }
        }
    }
}

#[cfg(feature = "tunnel")]
async fn establish_direct_tunnel_stream(
    rsd_addr: Ipv6Addr,
    service_port: u16,
) -> Result<GuardedTunnelStream<XpcClient>, CoreError> {
    let mut client = XpcClient::connect(rsd_addr, service_port).await?;
    let mut sequence_number = 0u64;

    client
        .send(build_direct_handshake_request(sequence_number))
        .await?;
    sequence_number += 1;

    let handshake = client.recv().await?;
    let remote_identifier =
        extract_direct_remote_identifier(handshake.body.as_ref().ok_or_else(|| {
            CoreError::Protocol("direct handshake response missing body".into())
        })?)?;

    let loaded = {
        let id = remote_identifier.clone();
        tokio::task::spawn_blocking(move || load_remote_pairing_credentials(&id))
            .await
            .map_err(|e| CoreError::Other(format!("spawn_blocking join error: {e}")))?
    }?;

    let mut our_secret = [0u8; 32];
    rand::thread_rng().fill_bytes(&mut our_secret);
    let static_secret = x25519_dalek::StaticSecret::from(our_secret);
    let our_public = x25519_dalek::PublicKey::from(&static_secret).to_bytes();

    client
        .send(build_direct_pairing_event(
            &build_verify_start_tlv(&our_public),
            "verifyManualPairing",
            true,
            None,
            sequence_number,
        ))
        .await?;
    sequence_number += 1;

    let verify_start = client.recv().await?;
    let verify_start_tlv =
        extract_direct_pairing_tlv(verify_start.body.as_ref().ok_or_else(|| {
            CoreError::Protocol("verifyManualPairing start missing body".into())
        })?)?;
    let verify_start_fields = TlvBuffer::decode(&verify_start_tlv);
    if let Some(error) = verify_start_fields.get(&DIRECT_PAIRING_TYPE_ERROR) {
        send_pair_verify_failed(&mut client, sequence_number).await?;
        return Err(CoreError::Protocol(format!(
            "verifyManualPairing start rejected: {error:?}"
        )));
    }

    let device_public: [u8; 32] = verify_start_fields
        .get(&DIRECT_PAIRING_TYPE_PUBLIC_KEY)
        .ok_or_else(|| {
            CoreError::Protocol("verifyManualPairing start missing device public key".into())
        })?
        .as_ref()
        .try_into()
        .map_err(|_| {
            CoreError::Protocol("verifyManualPairing device public key must be 32 bytes".into())
        })?;

    let verify_session = build_verify_step2_tlv(
        our_secret,
        &our_public,
        &device_public,
        &loaded.host_identity,
    )
    .map_err(|e| CoreError::Other(format!("verifyManualPairing finish build failed: {e}")))?;

    client
        .send(build_direct_pairing_event(
            &verify_session.tlv,
            "verifyManualPairing",
            false,
            None,
            sequence_number,
        ))
        .await?;
    sequence_number += 1;

    let verify_finish = client.recv().await?;
    let verify_finish_tlv =
        extract_direct_pairing_tlv(verify_finish.body.as_ref().ok_or_else(|| {
            CoreError::Protocol("verifyManualPairing finish missing body".into())
        })?)?;
    let verify_finish_fields = TlvBuffer::decode(&verify_finish_tlv);
    if let Some(error) = verify_finish_fields.get(&DIRECT_PAIRING_TYPE_ERROR) {
        send_pair_verify_failed(&mut client, sequence_number).await?;
        return Err(CoreError::Protocol(format!(
            "verifyManualPairing finish rejected: {error:?}"
        )));
    }

    let listener_port =
        create_direct_tcp_listener(&mut client, &verify_session, sequence_number).await?;
    let stream = crate::psk_tls::connect_psk_tls(
        &rsd_addr.to_string(),
        listener_port,
        &verify_session.encryption_key,
    )
    .await
    .map_err(|e| CoreError::Other(format!("direct TLS-PSK listener connect failed: {e}")))?;

    Ok(GuardedTunnelStream {
        stream,
        _guard: client,
    })
}

#[cfg(feature = "tunnel")]
async fn establish_remote_pairing_tunnel_stream(
    remote_identifier: &str,
    host: &str,
    port: u16,
) -> Result<GuardedTunnelStream<RemotePairingControlChannel>, CoreError> {
    let loaded = {
        let id = remote_identifier.to_owned();
        tokio::task::spawn_blocking(move || load_remote_pairing_credentials(&id))
            .await
            .map_err(|e| CoreError::Other(format!("spawn_blocking join error: {e}")))?
    }?;
    let mut control = RemotePairingControlChannel::connect(host, port).await?;
    let mut sequence_number = 0u64;

    control
        .send(&build_remote_pairing_handshake_request(sequence_number))
        .await?;
    sequence_number += 1;
    let _handshake = control.recv().await?;

    let mut our_secret = [0u8; 32];
    rand::thread_rng().fill_bytes(&mut our_secret);
    let static_secret = x25519_dalek::StaticSecret::from(our_secret);
    let our_public = x25519_dalek::PublicKey::from(&static_secret).to_bytes();

    control
        .send(&build_remote_pairing_pairing_event(
            &build_verify_start_tlv(&our_public),
            "verifyManualPairing",
            true,
            None,
            sequence_number,
        ))
        .await?;
    sequence_number += 1;

    let verify_start = control.recv().await?;
    let verify_start_tlv = extract_remote_pairing_tlv(&verify_start)?;
    let verify_start_fields = TlvBuffer::decode(&verify_start_tlv);
    if let Some(error) = verify_start_fields.get(&DIRECT_PAIRING_TYPE_ERROR) {
        control
            .send(&build_remote_pairing_pair_verify_failed_event(
                sequence_number,
            ))
            .await?;
        return Err(CoreError::Protocol(format!(
            "remote pairing verify start rejected: {error:?}"
        )));
    }

    let device_public: [u8; 32] = verify_start_fields
        .get(&DIRECT_PAIRING_TYPE_PUBLIC_KEY)
        .ok_or_else(|| {
            CoreError::Protocol("remote pairing verify start missing device public key".into())
        })?
        .as_ref()
        .try_into()
        .map_err(|_| {
            CoreError::Protocol("remote pairing device public key must be 32 bytes".into())
        })?;

    let verify_session = build_verify_step2_tlv(
        our_secret,
        &our_public,
        &device_public,
        &loaded.host_identity,
    )
    .map_err(|e| CoreError::Other(format!("remote pairing verify finish build failed: {e}")))?;

    control
        .send(&build_remote_pairing_pairing_event(
            &verify_session.tlv,
            "verifyManualPairing",
            false,
            None,
            sequence_number,
        ))
        .await?;
    sequence_number += 1;

    let verify_finish = control.recv().await?;
    let verify_finish_tlv = extract_remote_pairing_tlv(&verify_finish)?;
    let verify_finish_fields = TlvBuffer::decode(&verify_finish_tlv);
    if let Some(error) = verify_finish_fields.get(&DIRECT_PAIRING_TYPE_ERROR) {
        control
            .send(&build_remote_pairing_pair_verify_failed_event(
                sequence_number,
            ))
            .await?;
        return Err(CoreError::Protocol(format!(
            "remote pairing verify finish rejected: {error:?}"
        )));
    }

    let listener_port =
        create_remote_pairing_tcp_listener(&mut control, &verify_session, sequence_number).await?;
    let stream =
        crate::psk_tls::connect_psk_tls(host, listener_port, &verify_session.encryption_key)
            .await
            .map_err(|e| {
                CoreError::Other(format!(
                    "remote pairing TLS-PSK listener connect failed: {e}"
                ))
            })?;

    Ok(GuardedTunnelStream {
        stream,
        _guard: control,
    })
}

#[cfg(feature = "tunnel")]
async fn send_pair_verify_failed(
    client: &mut XpcClient,
    sequence_number: u64,
) -> Result<(), CoreError> {
    client
        .send(build_direct_pair_verify_failed_event(sequence_number))
        .await
        .map_err(CoreError::from)
}

#[cfg(feature = "tunnel")]
fn load_remote_pairing_credentials(
    remote_identifier: &str,
) -> Result<LoadedRemotePairingCredentials, CoreError> {
    load_remote_pairing_credentials_from_dirs(
        remote_identifier,
        &PersistedCredentials::default_dir(),
        &PersistedCredentials::pymobiledevice3_dir(),
        &current_hostname(),
    )
}

#[cfg(feature = "tunnel")]
fn load_remote_pairing_credentials_from_dirs(
    remote_identifier: &str,
    ios_rs_dir: &Path,
    pymobiledevice3_dir: &Path,
    hostname: &str,
) -> Result<LoadedRemotePairingCredentials, CoreError> {
    if let Some(remote_pair_record) =
        RemotePairingRecord::load_for_identifier(ios_rs_dir, remote_identifier)
    {
        if let Some(persisted) = find_persisted_host_identity(ios_rs_dir, remote_identifier) {
            return load_ios_rs_remote_pairing_credentials(
                remote_identifier,
                remote_pair_record,
                persisted,
            );
        }
    }

    if let Some(remote_pair_record) =
        RemotePairingRecord::load_for_identifier(pymobiledevice3_dir, remote_identifier)
    {
        return load_pymobiledevice3_remote_pairing_credentials(
            remote_identifier,
            hostname,
            remote_pair_record,
            pymobiledevice3_dir,
        );
    }

    if RemotePairingRecord::load_for_identifier(ios_rs_dir, remote_identifier).is_some() {
        return Err(CoreError::Unsupported(format!(
            "missing persisted host identity for remote identifier {remote_identifier}"
        )));
    }

    Err(CoreError::Unsupported(format!(
        "missing remote pairing record for {remote_identifier} in {} or {}",
        ios_rs_dir.display(),
        pymobiledevice3_dir.display()
    )))
}

#[cfg(feature = "tunnel")]
fn find_persisted_host_identity(
    creds_dir: &Path,
    remote_identifier: &str,
) -> Option<PersistedCredentials> {
    PersistedCredentials::list(creds_dir)
        .into_iter()
        .find(|creds| creds.remote_identifier.as_deref() == Some(remote_identifier))
}

#[cfg(feature = "tunnel")]
fn load_ios_rs_remote_pairing_credentials(
    remote_identifier: &str,
    remote_pair_record: RemotePairingRecord,
    persisted: PersistedCredentials,
) -> Result<LoadedRemotePairingCredentials, CoreError> {
    let host_private_key = remote_pair_record.private_key.clone();
    let host_identity =
        HostIdentity::from_private_key_bytes(persisted.host_identifier, &host_private_key)
            .map_err(|e| CoreError::Other(format!("invalid persisted host identity: {e}")))?;

    if host_identity.public_key_bytes() != remote_pair_record.public_key {
        return Err(CoreError::Protocol(format!(
            "persisted host key mismatch for remote identifier {remote_identifier}"
        )));
    }

    if let Some(host_private_key_hex) = persisted.host_private_key_hex {
        let persisted_private_key = hex::decode(host_private_key_hex)
            .map_err(|e| CoreError::Other(format!("invalid host private key hex: {e}")))?;
        if persisted_private_key != remote_pair_record.private_key {
            return Err(CoreError::Protocol(format!(
                "persisted host private key mismatch for remote identifier {remote_identifier}"
            )));
        }
    }

    Ok(LoadedRemotePairingCredentials { host_identity })
}

#[cfg(feature = "tunnel")]
fn load_pymobiledevice3_remote_pairing_credentials(
    remote_identifier: &str,
    hostname: &str,
    remote_pair_record: RemotePairingRecord,
    creds_dir: &Path,
) -> Result<LoadedRemotePairingCredentials, CoreError> {
    let host_identifier = pymobiledevice3_host_identifier(hostname);
    let host_identity =
        HostIdentity::from_private_key_bytes(host_identifier, &remote_pair_record.private_key)
            .map_err(|e| {
                CoreError::Other(format!(
                    "invalid pymobiledevice3 remote pairing identity for {remote_identifier}: {e}"
                ))
            })?;

    if host_identity.public_key_bytes() != remote_pair_record.public_key {
        return Err(CoreError::Protocol(format!(
            "pymobiledevice3 host key mismatch for remote identifier {remote_identifier} in {}",
            creds_dir.display()
        )));
    }

    Ok(LoadedRemotePairingCredentials { host_identity })
}

#[cfg(feature = "tunnel")]
fn current_hostname() -> String {
    std::env::var_os("COMPUTERNAME")
        .or_else(|| std::env::var_os("HOSTNAME"))
        .unwrap_or_default()
        .to_string_lossy()
        .into_owned()
}

#[cfg(feature = "tunnel")]
fn pymobiledevice3_host_identifier(hostname: &str) -> String {
    const NAMESPACE_DNS: [u8; 16] = [
        0x6b, 0xa7, 0xb8, 0x10, 0x9d, 0xad, 0x11, 0xd1, 0x80, 0xb4, 0x00, 0xc0, 0x4f, 0xd4, 0x30,
        0xc8,
    ];

    let mut input = Vec::with_capacity(NAMESPACE_DNS.len() + hostname.len());
    input.extend_from_slice(&NAMESPACE_DNS);
    input.extend_from_slice(hostname.as_bytes());

    let mut bytes = md5::compute(&input).0.to_vec();
    bytes[6] = (bytes[6] & 0x0f) | 0x30;
    bytes[8] = (bytes[8] & 0x3f) | 0x80;

    format!(
        "{:02x}{:02x}{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}{:02x}{:02x}{:02x}{:02x}",
        bytes[0],
        bytes[1],
        bytes[2],
        bytes[3],
        bytes[4],
        bytes[5],
        bytes[6],
        bytes[7],
        bytes[8],
        bytes[9],
        bytes[10],
        bytes[11],
        bytes[12],
        bytes[13],
        bytes[14],
        bytes[15]
    )
    .to_uppercase()
}

#[cfg(feature = "tunnel")]
fn build_direct_handshake_request(sequence_number: u64) -> XpcValue {
    build_direct_control_envelope(
        xpc_dict(&[(
            "plain",
            xpc_dict(&[(
                "_0",
                xpc_dict(&[(
                    "request",
                    xpc_dict(&[(
                        "_0",
                        xpc_dict(&[(
                            "handshake",
                            xpc_dict(&[(
                                "_0",
                                xpc_dict(&[
                                    (
                                        "hostOptions",
                                        xpc_dict(&[("attemptPairVerify", XpcValue::Bool(true))]),
                                    ),
                                    ("wireProtocolVersion", XpcValue::Int64(19)),
                                ]),
                            )]),
                        )]),
                    )]),
                )]),
            )]),
        )]),
        sequence_number,
    )
}

#[cfg(feature = "tunnel")]
fn build_direct_pairing_event(
    tlv_data: &[u8],
    kind: &str,
    start_new_session: bool,
    sending_host: Option<&str>,
    sequence_number: u64,
) -> XpcValue {
    let mut pairs = vec![
        (
            "data",
            XpcValue::Data(bytes::Bytes::copy_from_slice(tlv_data)),
        ),
        ("kind", XpcValue::String(kind.to_string())),
        ("startNewSession", XpcValue::Bool(start_new_session)),
    ];
    if let Some(host) = sending_host {
        pairs.push(("sendingHost", XpcValue::String(host.to_string())));
    }

    build_direct_control_envelope(
        xpc_dict(&[(
            "plain",
            xpc_dict(&[(
                "_0",
                xpc_dict(&[(
                    "event",
                    xpc_dict(&[(
                        "_0",
                        xpc_dict(&[("pairingData", xpc_dict(&[("_0", xpc_dict(&pairs))]))]),
                    )]),
                )]),
            )]),
        )]),
        sequence_number,
    )
}

#[cfg(feature = "tunnel")]
fn build_direct_pair_verify_failed_event(sequence_number: u64) -> XpcValue {
    build_direct_control_envelope(
        xpc_dict(&[(
            "plain",
            xpc_dict(&[(
                "_0",
                xpc_dict(&[(
                    "event",
                    xpc_dict(&[("_0", xpc_dict(&[("pairVerifyFailed", xpc_dict(&[]))]))]),
                )]),
            )]),
        )]),
        sequence_number,
    )
}

#[cfg(feature = "tunnel")]
fn build_direct_control_envelope(message: XpcValue, sequence_number: u64) -> XpcValue {
    xpc_dict(&[
        (
            "mangledTypeName",
            XpcValue::String(DIRECT_CONTROL_CHANNEL_ENVELOPE_TYPE.to_string()),
        ),
        (
            "value",
            xpc_dict(&[
                ("message", message),
                (
                    "originatedBy",
                    XpcValue::String(DIRECT_CONTROL_CHANNEL_ORIGIN.to_string()),
                ),
                ("sequenceNumber", XpcValue::Uint64(sequence_number)),
            ]),
        ),
    ])
}

#[cfg(feature = "tunnel")]
async fn create_direct_tcp_listener(
    client: &mut XpcClient,
    session: &VerifyPairSession,
    sequence_number: u64,
) -> Result<u16, CoreError> {
    let nonce = make_direct_encrypted_nonce(0);
    let request = serde_json::json!({
        "request": {
            "_0": {
                "createListener": {
                    "key": BASE64_STANDARD.encode(session.encryption_key),
                    "peerConnectionsInfo": [{
                        "owningPID": std::process::id(),
                        "owningProcessName": "CoreDeviceService",
                    }],
                    "transportProtocolType": "tcp",
                }
            }
        }
    });
    let client_cipher = chacha20poly1305::ChaCha20Poly1305::new((&session.client_key).into());
    let encrypted = client_cipher
        .encrypt((&nonce).into(), request.to_string().as_bytes())
        .map_err(|e| CoreError::Other(format!("createListener encrypt failed: {e}")))?;

    client
        .send(build_direct_control_envelope(
            xpc_dict(&[(
                "streamEncrypted",
                xpc_dict(&[("_0", XpcValue::Data(bytes::Bytes::from(encrypted)))]),
            )]),
            sequence_number,
        ))
        .await?;

    let response = client.recv().await?;
    let encrypted_response = extract_direct_stream_encrypted(
        response
            .body
            .as_ref()
            .ok_or_else(|| CoreError::Protocol("createListener response missing body".into()))?,
    )?;
    let server_cipher = chacha20poly1305::ChaCha20Poly1305::new((&session.server_key).into());
    let plaintext = server_cipher
        .decrypt((&nonce).into(), encrypted_response.as_ref())
        .map_err(|e| CoreError::Other(format!("createListener decrypt failed: {e}")))?;
    let response: serde_json::Value = serde_json::from_slice(&plaintext)?;
    let response_body = response
        .get("response")
        .and_then(|value| value.get("_1"))
        .ok_or_else(|| CoreError::Protocol("createListener response missing response._1".into()))?;

    if let Some(message) = extract_direct_error_extended_message(response_body) {
        return Err(CoreError::Protocol(format!(
            "createListener returned errorExtended: {message}"
        )));
    }

    let port = response_body
        .get("createListener")
        .and_then(|value| value.get("port"))
        .and_then(serde_json::Value::as_u64)
        .ok_or_else(|| CoreError::Protocol("createListener response missing port".into()))?;
    u16::try_from(port)
        .ok()
        .filter(|port| *port != 0)
        .ok_or_else(|| CoreError::Protocol(format!("invalid createListener port {port}")))
}

#[cfg(feature = "tunnel")]
async fn create_remote_pairing_tcp_listener(
    control: &mut RemotePairingControlChannel,
    session: &VerifyPairSession,
    sequence_number: u64,
) -> Result<u16, CoreError> {
    let nonce = make_direct_encrypted_nonce(0);
    let request = serde_json::json!({
        "request": {
            "_0": {
                "createListener": {
                    "key": BASE64_STANDARD.encode(session.encryption_key),
                    "peerConnectionsInfo": [{
                        "owningPID": std::process::id(),
                        "owningProcessName": "CoreDeviceService",
                    }],
                    "transportProtocolType": "tcp",
                }
            }
        }
    });
    let client_cipher = chacha20poly1305::ChaCha20Poly1305::new((&session.client_key).into());
    let encrypted = client_cipher
        .encrypt((&nonce).into(), request.to_string().as_bytes())
        .map_err(|e| {
            CoreError::Other(format!("remote pairing createListener encrypt failed: {e}"))
        })?;

    control
        .send(&serde_json::json!({
            "message": {
                "streamEncrypted": {
                    "_0": BASE64_STANDARD.encode(encrypted),
                }
            },
            "originatedBy": DIRECT_CONTROL_CHANNEL_ORIGIN,
            "sequenceNumber": sequence_number,
        }))
        .await?;

    let response = control.recv().await?;
    let encrypted_response = extract_remote_pairing_stream_encrypted(&response)?;
    let server_cipher = chacha20poly1305::ChaCha20Poly1305::new((&session.server_key).into());
    let plaintext = server_cipher
        .decrypt((&nonce).into(), encrypted_response.as_ref())
        .map_err(|e| {
            CoreError::Other(format!("remote pairing createListener decrypt failed: {e}"))
        })?;
    let response: serde_json::Value = serde_json::from_slice(&plaintext)?;
    let response_body = response
        .get("response")
        .and_then(|value| value.get("_1"))
        .ok_or_else(|| {
            CoreError::Protocol("remote pairing createListener response missing response._1".into())
        })?;

    if let Some(message) = extract_direct_error_extended_message(response_body) {
        return Err(CoreError::Protocol(format!(
            "remote pairing createListener returned errorExtended: {message}"
        )));
    }

    let port = response_body
        .get("createListener")
        .and_then(|value| value.get("port"))
        .and_then(serde_json::Value::as_u64)
        .ok_or_else(|| {
            CoreError::Protocol("remote pairing createListener response missing port".into())
        })?;
    u16::try_from(port)
        .ok()
        .filter(|port| *port != 0)
        .ok_or_else(|| {
            CoreError::Protocol(format!("invalid remote pairing createListener port {port}"))
        })
}

#[cfg(feature = "tunnel")]
fn xpc_dict(pairs: &[(&str, XpcValue)]) -> XpcValue {
    let mut map = IndexMap::new();
    for (key, value) in pairs {
        map.insert((*key).to_string(), value.clone());
    }
    XpcValue::Dictionary(map)
}

#[cfg(feature = "tunnel")]
fn extract_direct_remote_identifier(body: &XpcValue) -> Result<String, CoreError> {
    direct_plain_message(body)?
        .get("response")
        .and_then(XpcValue::as_dict)
        .and_then(|response| response.get("_1"))
        .and_then(XpcValue::as_dict)
        .and_then(|response| response.get("handshake"))
        .and_then(XpcValue::as_dict)
        .and_then(|handshake| handshake.get("_0"))
        .and_then(XpcValue::as_dict)
        .and_then(|handshake| handshake.get("peerDeviceInfo"))
        .and_then(XpcValue::as_dict)
        .and_then(|peer| peer.get("identifier"))
        .and_then(XpcValue::as_str)
        .map(ToOwned::to_owned)
        .ok_or_else(|| CoreError::Protocol("handshake missing peerDeviceInfo.identifier".into()))
}

#[cfg(feature = "tunnel")]
fn build_remote_pairing_handshake_request(sequence_number: u64) -> serde_json::Value {
    serde_json::json!({
        "message": {
            "plain": {
                "_0": {
                    "request": {
                        "_0": {
                            "handshake": {
                                "_0": {
                                    "hostOptions": {
                                        "attemptPairVerify": true,
                                    },
                                    "wireProtocolVersion": 19,
                                }
                            }
                        }
                    }
                }
            }
        },
        "originatedBy": DIRECT_CONTROL_CHANNEL_ORIGIN,
        "sequenceNumber": sequence_number,
    })
}

#[cfg(feature = "tunnel")]
fn build_remote_pairing_pairing_event(
    tlv_data: &[u8],
    kind: &str,
    start_new_session: bool,
    sending_host: Option<&str>,
    sequence_number: u64,
) -> serde_json::Value {
    let mut body = serde_json::Map::new();
    body.insert(
        "data".into(),
        serde_json::Value::String(BASE64_STANDARD.encode(tlv_data)),
    );
    body.insert("kind".into(), serde_json::Value::String(kind.to_string()));
    body.insert(
        "startNewSession".into(),
        serde_json::Value::Bool(start_new_session),
    );
    if let Some(host) = sending_host {
        body.insert(
            "sendingHost".into(),
            serde_json::Value::String(host.to_string()),
        );
    }

    serde_json::json!({
        "message": {
            "plain": {
                "_0": {
                    "event": {
                        "_0": {
                            "pairingData": {
                                "_0": serde_json::Value::Object(body),
                            }
                        }
                    }
                }
            }
        },
        "originatedBy": DIRECT_CONTROL_CHANNEL_ORIGIN,
        "sequenceNumber": sequence_number,
    })
}

#[cfg(feature = "tunnel")]
fn build_remote_pairing_pair_verify_failed_event(sequence_number: u64) -> serde_json::Value {
    serde_json::json!({
        "message": {
            "plain": {
                "_0": {
                    "event": {
                        "_0": {
                            "pairVerifyFailed": {}
                        }
                    }
                }
            }
        },
        "originatedBy": DIRECT_CONTROL_CHANNEL_ORIGIN,
        "sequenceNumber": sequence_number,
    })
}

#[cfg(feature = "tunnel")]
fn extract_direct_pairing_tlv(body: &XpcValue) -> Result<Vec<u8>, CoreError> {
    let event = direct_plain_message(body)?
        .get("event")
        .and_then(XpcValue::as_dict)
        .and_then(|event| event.get("_0"))
        .and_then(XpcValue::as_dict)
        .ok_or_else(|| CoreError::Protocol("pairing response missing event._0".into()))?;

    if let Some(message) = event
        .get("pairingRejectedWithError")
        .and_then(extract_direct_rejection_message)
    {
        return Err(CoreError::Protocol(format!("pairing rejected: {message}")));
    }

    event
        .get("pairingData")
        .and_then(XpcValue::as_dict)
        .and_then(|pairing| pairing.get("_0"))
        .and_then(XpcValue::as_dict)
        .and_then(|pairing| pairing.get("data"))
        .and_then(|value| match value {
            XpcValue::Data(bytes) => Some(bytes.to_vec()),
            _ => None,
        })
        .ok_or_else(|| CoreError::Protocol("pairing response missing pairingData._0.data".into()))
}

#[cfg(feature = "tunnel")]
fn extract_remote_pairing_tlv(body: &serde_json::Value) -> Result<Vec<u8>, CoreError> {
    let event = body
        .get("message")
        .and_then(|value| value.get("plain"))
        .and_then(|value| value.get("_0"))
        .and_then(|value| value.get("event"))
        .and_then(|value| value.get("_0"))
        .ok_or_else(|| {
            CoreError::Protocol("remote pairing response missing message.plain._0.event._0".into())
        })?;

    if let Some(message) = event
        .get("pairingRejectedWithError")
        .and_then(extract_remote_pairing_rejection_message)
    {
        return Err(CoreError::Protocol(format!(
            "remote pairing rejected: {message}"
        )));
    }

    let data = event
        .get("pairingData")
        .and_then(|value| value.get("_0"))
        .and_then(|value| value.get("data"))
        .and_then(serde_json::Value::as_str)
        .ok_or_else(|| {
            CoreError::Protocol("remote pairing response missing pairingData._0.data".into())
        })?;
    BASE64_STANDARD
        .decode(data)
        .map_err(|e| CoreError::Other(format!("invalid remote pairing TLV base64: {e}")))
}

#[cfg(feature = "tunnel")]
fn extract_direct_stream_encrypted(body: &XpcValue) -> Result<Vec<u8>, CoreError> {
    direct_control_value(body)?
        .get("message")
        .and_then(XpcValue::as_dict)
        .and_then(|message| message.get("streamEncrypted"))
        .and_then(XpcValue::as_dict)
        .and_then(|encrypted| encrypted.get("_0"))
        .and_then(|value| match value {
            XpcValue::Data(bytes) => Some(bytes.to_vec()),
            _ => None,
        })
        .ok_or_else(|| {
            CoreError::Protocol("encrypted response missing message.streamEncrypted._0".into())
        })
}

#[cfg(feature = "tunnel")]
fn extract_remote_pairing_stream_encrypted(body: &serde_json::Value) -> Result<Vec<u8>, CoreError> {
    let encoded = body
        .get("message")
        .and_then(|value| value.get("streamEncrypted"))
        .and_then(|value| value.get("_0"))
        .and_then(serde_json::Value::as_str)
        .ok_or_else(|| {
            CoreError::Protocol(
                "remote pairing encrypted response missing message.streamEncrypted._0".into(),
            )
        })?;
    BASE64_STANDARD.decode(encoded).map_err(|e| {
        CoreError::Other(format!(
            "invalid remote pairing encrypted payload base64: {e}"
        ))
    })
}

#[cfg(feature = "tunnel")]
fn direct_control_value(body: &XpcValue) -> Result<&IndexMap<String, XpcValue>, CoreError> {
    let envelope = body.as_dict().ok_or_else(|| {
        CoreError::Protocol("direct control message body must be a dictionary".into())
    })?;
    let mangled_type = envelope
        .get("mangledTypeName")
        .and_then(XpcValue::as_str)
        .ok_or_else(|| {
            CoreError::Protocol("direct control message missing mangledTypeName".into())
        })?;
    if mangled_type != DIRECT_CONTROL_CHANNEL_ENVELOPE_TYPE {
        return Err(CoreError::Protocol(format!(
            "unexpected direct control channel type {mangled_type}"
        )));
    }
    envelope
        .get("value")
        .and_then(XpcValue::as_dict)
        .ok_or_else(|| CoreError::Protocol("direct control message missing value".into()))
}

#[cfg(feature = "tunnel")]
fn direct_plain_message(body: &XpcValue) -> Result<&IndexMap<String, XpcValue>, CoreError> {
    direct_control_value(body)?
        .get("message")
        .and_then(XpcValue::as_dict)
        .and_then(|message| message.get("plain"))
        .and_then(XpcValue::as_dict)
        .and_then(|plain| plain.get("_0"))
        .and_then(XpcValue::as_dict)
        .ok_or_else(|| {
            CoreError::Protocol("direct control message missing message.plain._0".into())
        })
}

#[cfg(feature = "tunnel")]
fn extract_direct_rejection_message(value: &XpcValue) -> Option<String> {
    value
        .as_dict()
        .and_then(|wrapped| wrapped.get("wrappedError"))
        .and_then(XpcValue::as_dict)
        .and_then(|wrapped| wrapped.get("userInfo"))
        .and_then(XpcValue::as_dict)
        .and_then(|user_info| user_info.get("NSLocalizedDescription"))
        .and_then(XpcValue::as_str)
        .map(ToOwned::to_owned)
}

#[cfg(feature = "tunnel")]
fn extract_remote_pairing_rejection_message(value: &serde_json::Value) -> Option<String> {
    value
        .get("wrappedError")
        .and_then(|wrapped| wrapped.get("userInfo"))
        .and_then(|user_info| user_info.get("NSLocalizedDescription"))
        .and_then(serde_json::Value::as_str)
        .map(ToOwned::to_owned)
}

#[cfg(feature = "tunnel")]
fn extract_direct_error_extended_message(value: &serde_json::Value) -> Option<String> {
    value
        .get("errorExtended")
        .and_then(|value| value.get("_0"))
        .and_then(|value| value.get("userInfo"))
        .and_then(|value| value.get("NSLocalizedDescription"))
        .and_then(serde_json::Value::as_str)
        .map(ToOwned::to_owned)
}

#[cfg(feature = "tunnel")]
fn make_direct_encrypted_nonce(sequence_number: u64) -> [u8; 12] {
    let mut nonce = [0u8; 12];
    nonce[..8].copy_from_slice(&sequence_number.to_le_bytes());
    nonce
}
