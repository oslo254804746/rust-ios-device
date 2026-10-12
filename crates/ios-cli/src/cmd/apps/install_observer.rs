//! Opt-in read-only wire evidence through a caller-selected existing RSD route.
use std::{io, net::SocketAddrV6, time::Duration};

use anyhow::Result;
use ios_core::device::{ExistingRsdServiceKind, ExistingRsdSession};
use ios_core::installcoordination::{InstallCoordinationError, InstallCoordinationProxyClient};
use ios_core::{CoreError, ExistingRsdRoute, XpcClient};
use sha2::{Digest, Sha256};

fn service_error(error: InstallCoordinationError) -> CoreError {
    CoreError::Io(io::Error::other(error))
}

pub(super) async fn run(
    udid: &str,
    bundle_id: &str,
    address: SocketAddrV6,
    proxy_port: Option<u16>,
    timeout_secs: u64,
    json: bool,
) -> Result<()> {
    anyhow::ensure!(
        (1..=60).contains(&timeout_secs),
        crate::output::input_decision("install observation timeout must be in 1..=60 seconds")
    );
    let budget = Duration::from_secs(timeout_secs);
    let route = match proxy_port {
        Some(port) => ExistingRsdRoute::via_userspace_proxy(address, port)?,
        None => ExistingRsdRoute::direct(address)?,
    };
    let mut cancellation = Box::pin(tokio::signal::ctrl_c());
    let session = tokio::select! {
        result = ExistingRsdSession::observe(route, budget) => result?,
        signal = &mut cancellation => {
            return match signal {
                Ok(()) => Err(anyhow::Error::new(ios_core::OperationCancelled::new())),
                Err(error) => Err(anyhow::Error::new(error).context("waiting for install observer cancellation")),
            };
        }
    };
    let work = Box::pin(session.run(async {
        let connection = session.open_service(
            udid,
            ios_core::installcoordination::SERVICE_NAME,
            ExistingRsdServiceKind::RemoteXpc,
            1024 * 1024,
        ).await?;
        let client = XpcClient::connect_stream(connection.stream).await?;
        let mut proxy = InstallCoordinationProxyClient::with_timeout(client, budget);
        let zero_started = std::time::Instant::now();
        match proxy.query_with_budget(bundle_id, Duration::ZERO).await {
            Err(InstallCoordinationError::Timeout(duration)) if duration.is_zero() => {}
            Err(error) => return Err(service_error(error)),
            Ok(_) => return Err(CoreError::Protocol("zero-budget query unexpectedly succeeded".into())),
        }
        let zero_elapsed_us = zero_started.elapsed().as_micros();
        if !proxy.is_usable() || proxy.timeout() != budget {
            return Err(CoreError::Protocol("zero-budget query changed original client usability or default budget".into()));
        }
        let finite_started = std::time::Instant::now();
        let observed = proxy.query_observed_with_budget(bundle_id, session.remaining_budget())
            .await.map_err(service_error)?;
        let finite_elapsed_us = finite_started.elapsed().as_micros();
        let default_budget_unchanged = proxy.timeout() == budget;
        let usable_after_finite_query = proxy.is_usable();
        if !default_budget_unchanged || !usable_after_finite_query {
            return Err(CoreError::Protocol("completed observed query did not preserve original client state".into()));
        }
        drop(proxy); // This observer owns its service socket, not the external tunnel.
        let record = observed.record;
        Ok(serde_json::json!({
            "schema_version": 1,
            "scope": "existing-rsd-sockets",
            "selected_udid_matched": true,
            "canonical_service_observed": true,
            "zero_budget": {"error_class": "timeout", "no_business_io": true, "client_usable": true,
                            "elapsed_us": u64::try_from(zero_elapsed_us).unwrap_or(u64::MAX)},
            "same_service_connection": true,
            "default_budget_unchanged": default_budget_unchanged,
            "client_usable_after_finite_query": usable_after_finite_query,
            "finite_query_elapsed_us": u64::try_from(finite_elapsed_us).unwrap_or(u64::MAX),
            "db_sequence_wire_type": observed.db_sequence_wire_type,
            "record": {"db_sequence": record.db_sequence,
                       "db_uuid_sha256": hex::encode(Sha256::digest(record.db_uuid.as_bytes())),
                       "persistent_identifier_size": record.persistent_identifier.len(),
                       "persistent_identifier_sha256": hex::encode(Sha256::digest(&record.persistent_identifier)),
                       "install_path_present": record.install_path.is_some(),
                       "extra_fields": record.extra.len()},
            "cleanup_complete": true,
            "cleanup_scope": "original-sockets-shutdown-and-peer-eof"
        }))
    }));
    let primary = tokio::select! {
        result = work => result.map_err(anyhow::Error::new),
        signal = &mut cancellation => match signal {
            Ok(()) => Err(anyhow::Error::new(ios_core::OperationCancelled::new())),
            Err(error) => Err(anyhow::Error::new(error).context("waiting for install observer cancellation")),
        }
    };
    let cleanup = session
        .close(Duration::from_secs(5))
        .await
        .map_err(anyhow::Error::new);
    let (mut value, receipt) = match (primary, cleanup) {
        (Ok(value), Ok(receipt)) => (value, receipt),
        (Err(primary), cleanup) => {
            return crate::cmd::test_workflow::combine_results(
                Err(primary),
                cleanup.map(|_| ()),
                "install observer original socket cleanup failed",
            )
        }
        (Ok(_), Err(cleanup)) => return Err(cleanup),
    };
    anyhow::ensure!(
        receipt.opened_sockets == 1 && receipt.closed_sockets == 1 && receipt.strict_eof,
        CoreError::Protocol(
            "install observer original service socket did not supply a strict cleanup receipt"
                .into()
        )
    );
    value["cleanup_receipt"] = serde_json::to_value(receipt)?;
    if json {
        crate::output::outln!("{}", serde_json::to_string_pretty(&value)?);
    } else {
        crate::output::outln!("InstallCoordination observed wire type: {}; same-client zero-budget recovery completed; owned service socket released.", value["db_sequence_wire_type"]);
    }
    Ok(())
}
