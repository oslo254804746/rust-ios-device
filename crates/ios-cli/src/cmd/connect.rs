use anyhow::{Context, Result};
use ios_core::{ConnectOptions, ConnectedDevice, TunMode};

/// One setup clock covers selected-device discovery, lockdown session,
/// StartService, port connection and any service TLS handshake.
pub async fn connect_lockdown_service(
    udid: &str,
    service: &str,
    budget: std::time::Duration,
) -> Result<(ConnectedDevice, ios_core::device::ServiceStream)> {
    bounded_service_setup(budget, async {
        let device = ios_core::connect(udid, userspace_options(true)).await?;
        let stream = device.connect_service(service).await?;
        Ok((device, stream))
    })
    .await
}

async fn bounded_service_setup<T>(
    budget: std::time::Duration,
    work: impl std::future::Future<Output = Result<T>>,
) -> Result<T> {
    if budget.is_zero() {
        return Err(std::io::Error::new(
            std::io::ErrorKind::TimedOut,
            "service setup budget is zero",
        )
        .into());
    }
    let deadline = tokio::time::Instant::now()
        .checked_add(budget)
        .ok_or_else(|| {
            std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "service setup deadline overflow",
            )
        })?;
    let result = tokio::time::timeout_at(deadline, Box::pin(work))
        .await
        .context("selected-device service setup deadline")?;
    match result {
        Ok(_) if tokio::time::Instant::now() >= deadline => Err(std::io::Error::new(
            std::io::ErrorKind::TimedOut,
            "late service setup completion",
        )
        .into()),
        result => result,
    }
}

pub fn userspace_options(skip_tunnel: bool) -> ConnectOptions {
    ConnectOptions {
        tun_mode: TunMode::Userspace,
        pair_record_path: None,
        skip_tunnel,
    }
}

pub async fn connect_lockdown_only(udid: &str) -> Result<ConnectedDevice> {
    ios_core::connect(udid, userspace_options(true))
        .await
        .with_context(|| format!("failed to connect {udid} through lockdown"))
}

pub async fn connect_userspace_tunnel(udid: &str) -> Result<ConnectedDevice> {
    ios_core::connect(udid, userspace_options(false))
        .await
        .with_context(|| format!("failed to establish userspace tunnel for {udid}"))
}

pub async fn probe_product_version(udid: &str) -> Result<semver::Version> {
    let device = connect_lockdown_only(udid)
        .await
        .context("failed to connect device for product version probe")?;
    device.product_version().await.map_err(Into::into)
}

pub async fn connect_by_ios_major(
    udid: &str,
    requires_tunnel: impl FnOnce(u64) -> bool,
) -> Result<(ConnectedDevice, semver::Version)> {
    let version = probe_product_version(udid).await?;
    let device = if requires_tunnel(version.major) {
        connect_userspace_tunnel(udid).await?
    } else {
        connect_lockdown_only(udid).await?
    };
    Ok((device, version))
}

#[cfg(test)]
mod setup_tests {
    use super::*;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    #[tokio::test(start_paused = true)]
    async fn setup_clock_includes_send_and_reply_and_drops_original_transport() {
        let (stream, mut peer) = tokio::io::duplex(1024);
        let started = tokio::time::Instant::now();
        let error = bounded_service_setup(std::time::Duration::from_millis(100), async move {
            let mut stream = stream;
            tokio::time::sleep(std::time::Duration::from_millis(60)).await;
            stream.write_all(b"StartService").await?;
            stream.read_u8().await?;
            Ok(())
        })
        .await
        .unwrap_err();
        assert_eq!(started.elapsed(), std::time::Duration::from_millis(100));
        assert!(error.chain().any(|e| e.is::<tokio::time::error::Elapsed>()));
        let mut sent = Vec::new();
        peer.read_to_end(&mut sent).await.unwrap();
        assert_eq!(sent, b"StartService");
    }
    #[tokio::test]
    async fn zero_setup_does_not_poll_and_caller_drop_releases_owned_transport() {
        let polled = std::sync::atomic::AtomicBool::new(false);
        assert!(bounded_service_setup(std::time::Duration::ZERO, async {
            polled.store(true, std::sync::atomic::Ordering::SeqCst);
            Ok(())
        })
        .await
        .is_err());
        assert!(!polled.load(std::sync::atomic::Ordering::SeqCst));
        let (stream, mut peer) = tokio::io::duplex(8);
        {
            let setup = bounded_service_setup(std::time::Duration::from_secs(1), async move {
                let mut stream = stream;
                stream.write_all(b"partial").await?;
                stream.read_u8().await?;
                Ok(())
            });
            tokio::pin!(setup);
            tokio::select! {result=&mut setup=>panic!("unexpected {result:?}"),read=peer.read_u8()=>{read.unwrap();}}
        }
        let mut remaining = Vec::new();
        peer.read_to_end(&mut remaining).await.unwrap();
        assert_eq!(remaining, b"artial");
    }
}
