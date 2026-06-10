/// Attempt RSD handshake; returns None on failure (e.g. iOS <17).
#[cfg(feature = "tunnel")]
async fn attempt_rsd(server_addr: &str, rsd_port: u16) -> Option<RsdHandshake> {
    let addr = Ipv6Addr::from_str(server_addr).ok()?;
    match rsd_handshake(addr, rsd_port).await {
        Ok(h) => {
            tracing::info!(
                "RSD: {} services discovered for {}",
                h.services.len(),
                h.udid
            );
            Some(h)
        }
        Err(e) => {
            tracing::debug!("RSD handshake failed (may be iOS <17): {e}");
            None
        }
    }
}

/// Attempt RSD via go-ios-compatible userspace proxy.
#[cfg(feature = "tunnel")]
async fn attempt_rsd_via_proxy(
    proxy_port: u16,
    server_addr: &str,
    rsd_port: u16,
) -> Option<RsdHandshake> {
    tracing::info!(
        "RSD via proxy: probing [{server_addr}]:{rsd_port} through proxy port {proxy_port}"
    );

    let mut framer = match open_rsd_proxy_framer(proxy_port, server_addr, rsd_port).await {
        Some(framer) => framer,
        None => return None,
    };

    match tokio::time::timeout(
        Duration::from_secs(3),
        crate::xpc::rsd::queue_rsd_handshake_bootstrap_on_framer(&mut framer),
    )
    .await
    {
        Ok(Ok(())) => match tokio::time::timeout(
            Duration::from_secs(4),
            crate::xpc::rsd::handshake_on_framer(&mut framer),
        )
        .await
        {
            Ok(Ok(handshake)) => {
                tracing::info!(
                    "RSD via proxy: queued bootstrap succeeded with {} services for {}",
                    handshake.services.len(),
                    handshake.udid
                );
                return Some(handshake);
            }
            Ok(Err(e)) => {
                tracing::warn!(
                    "RSD via proxy: queued bootstrap handshake failed: {e}; trying legacy bootstrap"
                );
            }
            Err(_) => {
                tracing::warn!(
                    "RSD via proxy: queued bootstrap handshake timed out; trying legacy bootstrap"
                );
            }
        },
        Ok(Err(e)) => {
            tracing::warn!("RSD via proxy: queued bootstrap failed: {e}; trying legacy bootstrap");
        }
        Err(_) => {
            tracing::warn!("RSD via proxy: queued bootstrap timed out; trying legacy bootstrap");
        }
    }

    let mut framer = match open_rsd_proxy_framer(proxy_port, server_addr, rsd_port).await {
        Some(framer) => framer,
        None => return None,
    };

    match tokio::time::timeout(
        Duration::from_secs(3),
        crate::xpc::rsd::initialize_xpc_connection_on_framer(&mut framer),
    )
    .await
    {
        Ok(Ok(())) => match tokio::time::timeout(
            Duration::from_secs(3),
            crate::xpc::rsd::handshake_on_framer(&mut framer),
        )
        .await
        {
            Ok(Ok(h)) => {
                tracing::info!(
                    "RSD via proxy: legacy bootstrap succeeded with {} services for {}",
                    h.services.len(),
                    h.udid
                );
                Some(h)
            }
            Ok(Err(e)) => {
                tracing::warn!(
                    "RSD handshake via proxy after legacy bootstrap: {e}; trying passive fallback"
                );
                match tokio::time::timeout(
                    Duration::from_secs(2),
                    crate::xpc::rsd::handshake_on_framer(&mut framer),
                )
                .await
                {
                    Ok(Ok(h)) => {
                        tracing::info!(
                            "RSD via proxy (passive fallback): {} services for {}",
                            h.services.len(),
                            h.udid
                        );
                        Some(h)
                    }
                    Ok(Err(e)) => {
                        tracing::warn!("RSD passive fallback failed: {e}");
                        None
                    }
                    Err(_) => {
                        tracing::warn!("RSD passive fallback timed out");
                        None
                    }
                }
            }
            Err(_) => {
                tracing::warn!("RSD handshake via proxy timed out after legacy bootstrap");
                None
            }
        },
        Ok(Err(e)) => {
            tracing::warn!("RSD legacy bootstrap failed: {e}; trying passive fallback");
            match tokio::time::timeout(
                Duration::from_secs(2),
                crate::xpc::rsd::handshake_on_framer(&mut framer),
            )
            .await
            {
                Ok(Ok(h)) => {
                    tracing::info!(
                        "RSD via proxy (passive fallback): {} services for {}",
                        h.services.len(),
                        h.udid
                    );
                    Some(h)
                }
                Ok(Err(e)) => {
                    tracing::warn!("RSD passive fallback failed: {e}");
                    None
                }
                Err(_) => {
                    tracing::warn!("RSD passive fallback timed out");
                    None
                }
            }
        }
        Err(_) => {
            tracing::warn!("RSD legacy bootstrap timed out; trying passive fallback");
            match tokio::time::timeout(
                Duration::from_secs(2),
                crate::xpc::rsd::handshake_on_framer(&mut framer),
            )
            .await
            {
                Ok(Ok(h)) => {
                    tracing::info!(
                        "RSD via proxy (passive fallback): {} services for {}",
                        h.services.len(),
                        h.udid
                    );
                    Some(h)
                }
                Ok(Err(e)) => {
                    tracing::warn!("RSD passive fallback failed: {e}");
                    None
                }
                Err(_) => {
                    tracing::warn!("RSD passive fallback timed out");
                    None
                }
            }
        }
    }
}

#[cfg(feature = "tunnel")]
async fn open_rsd_proxy_framer(
    proxy_port: u16,
    server_addr: &str,
    rsd_port: u16,
) -> Option<crate::xpc::h2_raw::H2Framer<tokio::net::TcpStream>> {
    use tokio::io::AsyncWriteExt;
    use tokio::net::TcpStream;

    tracing::info!("RSD via proxy: connecting to 127.0.0.1:{proxy_port}");
    let mut proxy = match TcpStream::connect(format!("127.0.0.1:{proxy_port}")).await {
        Ok(stream) => {
            tracing::info!("RSD via proxy: connected to proxy");
            stream
        }
        Err(e) => {
            tracing::warn!("RSD proxy connect failed: {e}");
            return None;
        }
    };

    let addr_bytes = match Ipv6Addr::from_str(server_addr) {
        Ok(addr) => addr.octets(),
        Err(e) => {
            tracing::warn!("RSD bad server addr '{server_addr}': {e}");
            return None;
        }
    };

    if let Err(e) = proxy.write_all(&addr_bytes).await {
        tracing::warn!("RSD write addr: {e}");
        return None;
    }
    if let Err(e) = proxy.write_all(&(rsd_port as u32).to_le_bytes()).await {
        tracing::warn!("RSD write port: {e}");
        return None;
    }
    if let Err(e) = proxy.flush().await {
        tracing::warn!("RSD flush header: {e}");
        return None;
    }

    tracing::info!(
        "RSD via proxy: connecting to [{server_addr}]:{rsd_port} through proxy port {proxy_port}"
    );
    tracing::info!("RSD via proxy: starting H2 framer connect");
    match crate::xpc::h2_raw::H2Framer::connect(proxy).await {
        Ok(framer) => {
            tracing::info!("RSD via proxy: H2 framer connected");
            Some(framer)
        }
        Err(e) => {
            tracing::warn!("RSD H2 framer: {e}");
            None
        }
    }
}

// ── ProxyStream ───────────────────────────────────────────────────────────────

#[cfg(feature = "tunnel")]
pub(crate) enum ProxyStream {
    Plain(ServiceStream),
    Tls(Box<tokio_rustls::client::TlsStream<ServiceStream>>),
}

#[cfg(feature = "tunnel")]
impl Unpin for ProxyStream {}

#[cfg(feature = "tunnel")]
impl AsyncRead for ProxyStream {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<std::io::Result<()>> {
        match &mut *self {
            ProxyStream::Plain(s) => Pin::new(s).poll_read(cx, buf),
            ProxyStream::Tls(s) => Pin::new(s).poll_read(cx, buf),
        }
    }
}

#[cfg(feature = "tunnel")]
impl AsyncWrite for ProxyStream {
    fn poll_write(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<std::io::Result<usize>> {
        match &mut *self {
            ProxyStream::Plain(s) => Pin::new(s).poll_write(cx, buf),
            ProxyStream::Tls(s) => Pin::new(s).poll_write(cx, buf),
        }
    }
    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        match &mut *self {
            ProxyStream::Plain(s) => Pin::new(s).poll_flush(cx),
            ProxyStream::Tls(s) => Pin::new(s).poll_flush(cx),
        }
    }
    fn poll_shutdown(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        match &mut *self {
            ProxyStream::Plain(s) => Pin::new(s).poll_shutdown(cx),
            ProxyStream::Tls(s) => Pin::new(s).poll_shutdown(cx),
        }
    }
}
