//! Explicit synthetic contact/media lifecycle with original foreground ownership.
use std::time::Duration;

use anyhow::{Context, Result};
use ios_core::device::ResolvedServiceMetadata;
use ios_core::display::{DisplayServiceClient, MediaStreamOptions, MediaStreamSession};
use ios_core::hid::{
    TouchCoordinate, TouchPhase, UniversalHidServiceClient, DEFAULT_TOUCHSCREEN_SERVICE_ID,
};
use ios_core::{TunMode, XpcValue};
use sha2::{Digest, Sha256};
use tokio::time::Instant;

use crate::cmd::tunnel::{NativeConnection, NativeScope};

#[derive(Default)]
struct Owners {
    device: Option<NativeConnection>,
    hid: Option<UniversalHidServiceClient>,
    media: Option<MediaStreamSession>,
    contact_pending: bool,
    media_may_have_started: bool,
    media_stopped: bool,
    reports_flushed: usize,
    local_contact_closed: bool,
}

async fn display(device: &NativeConnection) -> Result<DisplayServiceClient> {
    let (client, metadata) =
        Box::pin(device.connect_xpc_service_with_metadata(ios_core::display::SERVICE_NAME)).await?;
    Ok(DisplayServiceClient::from_resolved_metadata(
        client, &metadata,
    )?)
}

fn status_digest(status: &XpcValue) -> Result<String> {
    let encoded = ios_core::encode_xpc_message(&ios_core::XpcMessage {
        flags: ios_core::xpc_message_flags::ALWAYS_SET | ios_core::xpc_message_flags::DATA,
        msg_id: 0,
        body: Some(status.clone()),
    })?;
    anyhow::ensure!(
        encoded.len() <= 1024 * 1024,
        crate::output::resource_decision("media status observation exceeds one MiB")
    );
    Ok(hex::encode(Sha256::digest(&encoded)))
}

fn universal(
    client: ios_core::XpcClient,
    metadata: &ResolvedServiceMetadata,
) -> Result<UniversalHidServiceClient> {
    Ok(UniversalHidServiceClient::from_resolved_metadata(
        client, metadata,
    )?)
}

pub(super) async fn run(
    udid: &str,
    expected_status_sha256: &str,
    confirm_stop_all: bool,
    timeout: Duration,
    json: bool,
) -> Result<()> {
    anyhow::ensure!(
        confirm_stop_all,
        crate::output::input_decision(
            "media observer requires --confirm-stop-all for the protocol's stopAll operation"
        )
    );
    anyhow::ensure!(
        expected_status_sha256.len() == 64
            && expected_status_sha256
                .bytes()
                .all(|byte| byte.is_ascii_hexdigit()),
        crate::output::input_decision(
            "expected media status SHA-256 must contain 64 hexadecimal characters"
        )
    );
    let coordinate = TouchCoordinate::new(0.5, 0.5)?;
    let deadline = Instant::now()
        .checked_add(timeout)
        .context("HID observer deadline overflow")?;
    let scope = NativeScope::new()?;
    let mut owners = Owners::default();
    let work = Box::pin(async {
        owners.device = Some(Box::pin(scope.connect(udid, TunMode::Kernel)).await?);
        let device = owners
            .device
            .as_ref()
            .expect("stored original native connection");
        anyhow::ensure!(
            device.info.udid == udid,
            crate::output::input_decision(
                "HID observer device identity differs from explicit selection"
            )
        );
        let before = Box::pin(display(device))
            .await?
            .get_media_stream_server_status()
            .await?;
        let before_digest = status_digest(&before)?;
        anyhow::ensure!(
            before_digest.eq_ignore_ascii_case(expected_status_sha256),
            crate::output::input_decision(
                "current media status differs from approved synthetic baseline"
            )
        );
        let sender_ip = device
            .server_address()
            .context("native tunnel has no device media address")?
            .to_owned();
        let receiver_ip = device
            .client_address()
            .context("native tunnel has no host media address")?
            .to_owned();
        owners.media_may_have_started = true;
        owners.media = Some(
            Box::pin(MediaStreamSession::start_video(
                display(device).await?,
                MediaStreamOptions {
                    sender_ip,
                    receiver_ip,
                    timeout: deadline.saturating_duration_since(Instant::now()),
                    max_access_unit_bytes: 1024 * 1024,
                    max_access_unit_packets: 256,
                    ..MediaStreamOptions::default()
                },
            ))
            .await?,
        );
        owners
            .media
            .as_mut()
            .expect("stored original media session")
            .spawn_drain();
        tokio::time::sleep(Duration::from_millis(300)).await;
        let (client, metadata) = Box::pin(
            device.connect_xpc_service_with_metadata(ios_core::hid::UNIVERSAL_SERVICE_NAME),
        )
        .await?;
        owners.hid = Some(universal(client, &metadata)?);
        // Retain potential contact ownership before the first possible write.
        // Cancellation may interrupt a partial one-way report transmission.
        owners.contact_pending = true;
        {
            let mut touch = owners
                .hid
                .as_mut()
                .expect("stored HID service")
                .touch_session(DEFAULT_TOUCHSCREEN_SERVICE_ID);
            touch.touch(0, TouchPhase::Down, coordinate).await?;
            owners.reports_flushed += 1;
            touch
                .close(
                    deadline
                        .saturating_duration_since(Instant::now())
                        .min(Duration::from_secs(5)),
                )
                .await?;
            owners.reports_flushed += 1;
            owners.local_contact_closed = touch.is_closed() && !touch.has_active_contact();
            owners.contact_pending = false;
        }
        owners
            .media
            .as_mut()
            .expect("stored original media session")
            .stop_strict(
                deadline
                    .saturating_duration_since(Instant::now())
                    .min(Duration::from_secs(5)),
                true,
                &[],
            )
            .await?;
        owners.media_stopped = true;
        let after = Box::pin(display(device))
            .await?
            .get_media_stream_server_status()
            .await?;
        let after_digest = status_digest(&after)?;
        anyhow::ensure!(
            before == after,
            ios_core::CoreError::Protocol(
                "media status did not return to the approved baseline".into()
            )
        );
        Ok::<_, anyhow::Error>((before_digest, after_digest))
    });
    let primary = tokio::select! {
        result = tokio::time::timeout_at(deadline, work) => result.context("HID observer total deadline expired").and_then(|result| result),
        signal = tokio::signal::ctrl_c() => {
            match signal {
                Ok(()) => Err(anyhow::Error::new(ios_core::OperationCancelled::new())),
                Err(error) => Err(anyhow::Error::new(error).context("waiting for HID observer cancellation")),
            }
        }
    };
    let cleanup_deadline = Instant::now() + Duration::from_secs(10);
    let mut cleanup = Ok(());
    if owners.contact_pending {
        if let Some(client) = &mut owners.hid {
            let released = tokio::time::timeout_at(
                cleanup_deadline,
                client.send_touchscreen(
                    TouchPhase::Cancel,
                    coordinate,
                    DEFAULT_TOUCHSCREEN_SERVICE_ID,
                    None,
                ),
            )
            .await
            .context("HID observer release deadline expired")
            .and_then(|result| result.map_err(anyhow::Error::new));
            if released.is_ok() {
                owners.contact_pending = false;
            }
            cleanup = crate::cmd::test_workflow::combine_results(
                cleanup,
                released,
                "HID observer release failed",
            );
        }
    }
    if !owners.media_stopped {
        if let Some(media) = &mut owners.media {
            let stopped = tokio::time::timeout_at(
                cleanup_deadline,
                media.stop_strict(
                    cleanup_deadline.saturating_duration_since(Instant::now()),
                    true,
                    &[],
                ),
            )
            .await
            .context("HID observer media cleanup deadline expired")
            .and_then(|result| result.map(|_| ()).map_err(anyhow::Error::new));
            cleanup = crate::cmd::test_workflow::combine_results(
                cleanup,
                stopped,
                "HID observer media stop failed",
            );
        } else if owners.media_may_have_started {
            if let Some(device) = &owners.device {
                // Start cancellation can occur after the daemon accepted the
                // request but before MediaStreamSession was returned. The
                // original device owner remains available for a fresh stop.
                let stopped = tokio::time::timeout_at(cleanup_deadline, async {
                    display(device)
                        .await?
                        .stop_media_streams_strict(true, &[])
                        .await?;
                    Ok::<(), anyhow::Error>(())
                })
                .await
                .context("HID observer partial-start cleanup deadline expired")
                .and_then(|result| result);
                cleanup = crate::cmd::test_workflow::combine_results(
                    cleanup,
                    stopped,
                    "HID observer partial-start stop failed",
                );
            }
        }
    }
    owners.hid = None;
    owners.media = None;
    owners.device = None;
    let joined = scope
        .close(cleanup_deadline.saturating_duration_since(Instant::now()))
        .await;
    cleanup = crate::cmd::test_workflow::combine_results(
        cleanup,
        joined,
        "HID observer original native joins failed",
    );
    let (before, after) = match (primary, cleanup) {
        (Ok(value), Ok(())) => value,
        (Err(primary), cleanup) => {
            return crate::cmd::test_workflow::combine_results(
                Err(primary),
                cleanup,
                "HID observer cleanup failed",
            )
        }
        (Ok(_), Err(cleanup)) => return Err(cleanup),
    };
    let value = serde_json::json!({
        "schema_version": 1, "operation": "display_hid_stop_observation", "selected_udid_matched": true,
        "hid_reports_flushed": owners.reports_flushed, "local_contact_closed": owners.local_contact_closed,
        "hid_device_ack": "unavailable-in-protocol", "media_stop_reply_received": true,
        "original_media_tasks_joined": true, "status_before_sha256": before, "status_after_sha256": after,
        "media_status_restored": true, "stop_scope": "explicit-stop-all", "original_native_joins_complete": true,
        "cleanup_complete": true
    });
    if json {
        crate::output::outln!("{}", serde_json::to_string_pretty(&value)?);
    } else {
        crate::output::outln!("Synthetic HID release was written; strict media reply and baseline restoration observed; original native owners joined.");
    }
    Ok(())
}
