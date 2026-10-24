use super::*;

fn usb(udid: String) -> source::UsbDevice {
    source::UsbDevice {
        udid,
        device_id: 1,
        usb: true,
    }
}

fn resource_error(error: &anyhow::Error) {
    let rendered = crate::output::operation_failure_message(
        crate::output::ErrorFormat::Json,
        "tunnel-snapshot",
        error,
    )
    .unwrap();
    let body: serde_json::Value = serde_json::from_str(&rendered).unwrap();
    assert_eq!(body["error"]["class"], "resource_limit");
}

async fn reject_usb(devices: Vec<source::UsbDevice>) {
    let source = FakeSource::new();
    let state = state(source.clone(), TUNNEL_CONNECT_TIMEOUT);
    let mut control_input_0 = state.prepare_cutover::<1>().unwrap();
    let mut control_input_1 = state.prepare_cutover::<1>().unwrap();

    state
        .ensure_tunnel(A, None, Some(StartTunnelConnectionType::Usbmux))
        .await
        .unwrap();
    let publication = state.inner.tunnels.read().await[A].publication.clone();
    let ticket = state.inner.lifecycle.ticket(B).await.unwrap();
    let slot = state.reserve_failure_slot(B).await.unwrap();
    state
        .record_failed_attempt(
            B,
            AttemptIdentity {
                route: Route::Mobdev2("192.0.2.8".into()),
                native_id: None,
            },
            slot,
            &ticket,
        )
        .await;
    drop(ticket);
    let before = state.inner.failed_attempts.read().await[B].clone();
    let coordinators_before = state.inner.lifecycle.coordinator_count().await;
    *source.usb_details.lock().unwrap() = Some(devices);
    let result = state.refresh_once().await;
    let coordinators_after = state.inner.lifecycle.coordinator_count().await;
    let retained = state
        .inner
        .tunnels
        .read()
        .await
        .get(A)
        .is_some_and(|row| row.publication.ptr_eq(&publication));
    let unchanged = state
        .inner
        .failed_attempts
        .read()
        .await
        .get(B)
        .is_some_and(|row| {
            row.identity == before.identity
                && row.fail_count == before.fail_count
                && row.last_attempt == before.last_attempt
        });
    let calls = (
        source.calls.lock().unwrap().len(),
        source.paired_calls.load(Ordering::SeqCst),
    );
    let ready = state.is_ready().await;
    state.notify_shutdown(&mut control_input_0);
    drop(control_input_0);
    state.clear(&mut control_input_1).await;
    drop(control_input_1);
    state.drain_native(LOCAL_BUDGET).await.unwrap();
    resource_error(&result.unwrap_err());
    assert!(retained && unchanged && !ready);
    assert_eq!(calls, (1, 0));
    assert_eq!(coordinators_after, coordinators_before);
    assert_eq!(source.active.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn admission_usb_rows_reject_before_publication_or_backoff_eviction() {
    reject_usb(vec![usb(A.into()); 16_385]).await;
}

#[tokio::test]
async fn admission_usb_unique_reject_before_extra_candidate_ticket() {
    let mut devices = vec![usb(A.into())];
    devices.extend((0..4096).map(|index| usb(format!("metadata-{index}"))));
    reject_usb(devices).await;
}

#[tokio::test]
async fn admission_usb_field_checks_utf8_bytes_after_first_matching_row() {
    let mut long = "é".repeat(2048);
    long.push('x');
    reject_usb(vec![usb(A.into()), usb(long)]).await;
}

#[tokio::test]
async fn admission_usb_aggregate_rejects_next_byte_without_field_overflow() {
    let mut devices = vec![usb("x".repeat(4096)); 4096];
    devices.push(usb("x".into()));
    reject_usb(devices).await;
}

#[tokio::test]
async fn admission_usb_exact_boundaries_enter_paired_discovery_without_connection() {
    let cases = [
        vec![usb(A.into()); 16_384],
        vec![usb("é".repeat(2048))],
        vec![usb("x".repeat(4096)); 4096],
        (0..4096)
            .map(|index| usb(format!("accepted-{index}")))
            .collect(),
    ];
    for devices in cases {
        let source = FakeSource::new();
        *source.usb_details.lock().unwrap() = Some(devices);
        *source.paired_gate.lock().unwrap() = Some(Arc::new(Semaphore::new(0)));
        let state = state(source.clone(), TUNNEL_CONNECT_TIMEOUT);
        let mut control_input_0 = state.prepare_cutover::<1>().unwrap();
        let mut control_input_1 = state.prepare_cutover::<1>().unwrap();

        let mut scan = Box::pin(state.refresh_once());
        let reached_while_pending = tokio::time::timeout(LOCAL_BUDGET, async {
            loop {
                if futures_util::poll!(&mut scan).is_ready() {
                    return false;
                }
                if source.paired_calls.load(Ordering::SeqCst) == 1 {
                    return true;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap_or(false);
        let reached = source.paired_calls.load(Ordering::SeqCst);
        drop(scan);
        state.notify_shutdown(&mut control_input_0);
        drop(control_input_0);
        state.clear(&mut control_input_1).await;
        drop(control_input_1);
        state.drain_native(LOCAL_BUDGET).await.unwrap();
        assert!(reached_while_pending);
        assert_eq!(reached, 1);
        assert_eq!(source.paired_active.load(Ordering::SeqCst), 0);
        assert_eq!(source.paired_exited.load(Ordering::SeqCst), 1);
        assert!(source.calls.lock().unwrap().is_empty());
        assert_eq!(state.inner.lifecycle.connects.available_permits(), 4);
    }
}

#[tokio::test]
async fn admission_paired_rejection_preserves_unknown_backoff_and_publishes_usb() {
    let source = FakeSource::new();
    source.paired.lock().unwrap().push(PairedMobdev2Device {
        udid: B.into(),
        host: "192.0.2.2".into(),
    });
    source.script(B, [Behavior::Failure(std::io::ErrorKind::ConnectionReset)]);
    let state = state(source.clone(), TUNNEL_CONNECT_TIMEOUT);
    let mut control_input_0 = state.prepare_cutover::<1>().unwrap();
    let mut control_input_1 = state.prepare_cutover::<1>().unwrap();

    state.refresh_once().await.unwrap();
    let before = state.inner.failed_attempts.read().await[B].clone();
    source.usb.lock().unwrap().push(A.into());
    source.paired.lock().unwrap().push(PairedMobdev2Device {
        udid: "fresh-valid-paired-prefix".into(),
        host: "192.0.2.4".into(),
    });
    source.paired.lock().unwrap().push(PairedMobdev2Device {
        udid: "x".repeat(4097),
        host: "192.0.2.3".into(),
    });
    let result = state.refresh_once().await;
    let usb_published = state.get_record(A).await.unwrap().is_some();
    let after = state.inner.failed_attempts.read().await.get(B).cloned();
    let ready = state.is_ready().await;
    let calls = (source.count(A), source.count(B));
    let prefix_calls = source.count("fresh-valid-paired-prefix");
    let coordinators = state.inner.lifecycle.coordinator_count().await;
    state.notify_shutdown(&mut control_input_0);
    drop(control_input_0);
    state.clear(&mut control_input_1).await;
    drop(control_input_1);
    state.drain_native(LOCAL_BUDGET).await.unwrap();
    resource_error(&result.unwrap_err());
    let after = after.unwrap();
    assert_eq!(after.identity, before.identity);
    assert_eq!(after.fail_count, before.fail_count);
    assert_eq!(after.last_attempt, before.last_attempt);
    assert!(
        usb_published && ready,
        "prior readiness must remain unchanged"
    );
    assert_eq!(calls, (1, 1));
    assert_eq!(prefix_calls, 0);
    assert_eq!(coordinators, 2);
}

#[tokio::test]
async fn admission_manual_paired_fallback_validates_tail_after_matching_target() {
    let source = FakeSource::new();
    source.script(A, [Behavior::Failure(std::io::ErrorKind::ConnectionReset)]);
    *source.paired.lock().unwrap() = vec![
        PairedMobdev2Device {
            udid: A.into(),
            host: "192.0.2.2".into(),
        },
        PairedMobdev2Device {
            udid: "x".repeat(4097),
            host: "192.0.2.3".into(),
        },
    ];
    let state = state(source.clone(), TUNNEL_CONNECT_TIMEOUT);
    let mut control_input_0 = state.prepare_cutover::<1>().unwrap();
    let mut control_input_1 = state.prepare_cutover::<1>().unwrap();

    let result = state.ensure_tunnel(A, None, None).await;
    let calls = source.count(A);
    let empty = state.list_records().await.unwrap().as_slice().is_empty();
    state.notify_shutdown(&mut control_input_0);
    drop(control_input_0);
    state.clear(&mut control_input_1).await;
    drop(control_input_1);
    state.drain_native(LOCAL_BUDGET).await.unwrap();
    resource_error(&result.unwrap_err());
    assert!(empty);
    assert_eq!(calls, 1);
    assert_eq!(source.paired_calls.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn admission_paired_combined_count_and_bytes_reject_without_partial_tickets() {
    let oversized = [
        (0..4096)
            .map(|index| PairedMobdev2Device {
                udid: format!("paired-{index}"),
                host: "192.0.2.8".into(),
            })
            .collect::<Vec<_>>(),
        (0..2048)
            .map(|index| PairedMobdev2Device {
                udid: format!("{index:04}").repeat(1024),
                host: "h".repeat(4096),
            })
            .collect(),
    ];
    for targets in oversized {
        let source = FakeSource::new();
        source.paired.lock().unwrap().push(PairedMobdev2Device {
            udid: B.into(),
            host: "192.0.2.2".into(),
        });
        source.script(B, [Behavior::Failure(std::io::ErrorKind::ConnectionReset)]);
        let state = state(source.clone(), TUNNEL_CONNECT_TIMEOUT);
        let mut control_input_0 = state.prepare_cutover::<1>().unwrap();
        let mut control_input_1 = state.prepare_cutover::<1>().unwrap();

        state.refresh_once().await.unwrap();
        let before = state.inner.failed_attempts.read().await[B].clone();
        source.usb.lock().unwrap().push(A.into());
        *source.paired.lock().unwrap() = targets;
        let result = state.refresh_once().await;
        let publication = state.get_record(A).await.unwrap().is_some();
        let after = state.inner.failed_attempts.read().await.get(B).cloned();
        let coordinators = state.inner.lifecycle.coordinator_count().await;
        let calls = source.calls.lock().unwrap().len();
        state.notify_shutdown(&mut control_input_0);
        drop(control_input_0);
        state.clear(&mut control_input_1).await;
        drop(control_input_1);
        state.drain_native(LOCAL_BUDGET).await.unwrap();
        resource_error(&result.unwrap_err());
        let after = after.unwrap();
        assert_eq!(after.identity, before.identity);
        assert_eq!(after.fail_count, before.fail_count);
        assert_eq!(after.last_attempt, before.last_attempt);
        assert!(publication);
        assert_eq!(calls, 2); // Initial B failure, then USB A; no paired plan prefix.
        assert_eq!(coordinators, 2);
    }
}

#[tokio::test]
async fn admission_paired_combined_exact_count_keeps_four_future_window() {
    let source = FakeSource::new();
    source.usb.lock().unwrap().push(A.into());
    source.script(A, [Behavior::Pending]);
    let targets = (0..4095)
        .map(|index| {
            let udid = format!("paired-exact-{index}");
            source.script(&udid, [Behavior::Pending]);
            PairedMobdev2Device {
                udid,
                host: "192.0.2.8".into(),
            }
        })
        .collect();
    *source.paired.lock().unwrap() = targets;
    let state = state(source.clone(), TUNNEL_CONNECT_TIMEOUT);
    let mut control_input_0 = state.prepare_cutover::<1>().unwrap();
    let mut control_input_1 = state.prepare_cutover::<1>().unwrap();

    let mut scan = Box::pin(state.refresh_once());
    let reached = tokio::time::timeout(LOCAL_BUDGET, async {
        loop {
            if futures_util::poll!(&mut scan).is_ready() {
                return false;
            }
            if source.active.load(Ordering::SeqCst) == 4 {
                return true;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap_or(false);
    let coordinators = state.inner.lifecycle.coordinator_count().await;
    drop(scan);
    state.notify_shutdown(&mut control_input_0);
    drop(control_input_0);
    state.clear(&mut control_input_1).await;
    drop(control_input_1);
    state.drain_native(LOCAL_BUDGET).await.unwrap();
    assert!(reached);
    assert_eq!(coordinators, 4096);
    assert_eq!(source.calls.lock().unwrap().len(), 4);
    assert_eq!(source.peak.load(Ordering::SeqCst), 4);
    assert_eq!(source.active.load(Ordering::SeqCst), 0);
    assert_eq!(source.exited.load(Ordering::SeqCst), 4);
    assert_eq!(state.inner.lifecycle.connects.available_permits(), 4);
    assert_eq!(
        state.inner.failure_slots.available_permits(),
        MAX_FAILURE_RECORDS
    );
}

#[tokio::test]
async fn admission_paired_combined_exact_bytes_keeps_four_future_window() {
    let source = FakeSource::new();
    source.usb.lock().unwrap().push(A.into());
    source.script(A, [Behavior::Pending]);
    let mut targets = (0..2048)
        .map(|index| {
            let udid = format!("{index:04}").repeat(1024);
            source.script(&udid, [Behavior::Pending]);
            PairedMobdev2Device {
                udid,
                host: "h".repeat(4096),
            }
        })
        .collect::<Vec<_>>();
    targets.last_mut().unwrap().host.truncate(4096 - A.len());
    *source.paired.lock().unwrap() = targets;
    let state = state(source.clone(), TUNNEL_CONNECT_TIMEOUT);
    let mut control_input_0 = state.prepare_cutover::<1>().unwrap();
    let mut control_input_1 = state.prepare_cutover::<1>().unwrap();

    let mut scan = Box::pin(state.refresh_once());
    let reached = tokio::time::timeout(LOCAL_BUDGET, async {
        loop {
            if futures_util::poll!(&mut scan).is_ready() {
                return false;
            }
            if source.active.load(Ordering::SeqCst) == 4 {
                return true;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap_or(false);
    let coordinators = state.inner.lifecycle.coordinator_count().await;
    drop(scan);
    state.notify_shutdown(&mut control_input_0);
    drop(control_input_0);
    state.clear(&mut control_input_1).await;
    drop(control_input_1);
    state.drain_native(LOCAL_BUDGET).await.unwrap();
    assert!(reached);
    assert_eq!(coordinators, 2049);
    assert_eq!(source.calls.lock().unwrap().len(), 4);
    assert_eq!(source.active.load(Ordering::SeqCst), 0);
    assert_eq!(source.exited.load(Ordering::SeqCst), 4);
    assert_eq!(state.inner.lifecycle.connects.available_permits(), 4);
    assert_eq!(
        state.inner.failure_slots.available_permits(),
        MAX_FAILURE_RECORDS
    );
}

fn policy_devices(fixture: &crate::access::test_support::PolicyFixture, devices: &[&str]) {
    let mut policy: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&fixture.path).unwrap()).unwrap();
    policy["grants"][0]["devices"] = serde_json::json!(devices);
    ios_core::secret_file::write_secret(&fixture.path, &serde_json::to_vec(&policy).unwrap())
        .unwrap();
}

#[tokio::test]
async fn admission_usb_unique_counts_unauthorized_rows_before_gate_selection() {
    use crate::access::test_support::{grant, PolicyFixture, DEVICE, TOKEN};
    let fixture = PolicyFixture::new(vec![grant(TOKEN, vec![Permission::ManageTunnels])]);
    let source = FakeSource::new();
    let mut devices = vec![usb(DEVICE.into())];
    devices.extend((0..4096).map(|index| usb(format!("unauthorized-{index}"))));
    *source.usb_details.lock().unwrap() = Some(devices);
    let state = TunnelAgentState::with_source(
        TunMode::Userspace,
        AccessControl::from_file(fixture.path.clone())
            .await
            .unwrap(),
        Upstreams::default(),
        source.clone(),
        TUNNEL_CONNECT_TIMEOUT,
    );
    let mut prepared_outer_0 = state.prepare_cutover::<1>().unwrap();
    let mut prepared_outer_1 = state.prepare_cutover::<1>().unwrap();

    let result = state.refresh_once().await;
    let coordinators = state.inner.lifecycle.coordinator_count().await;
    state.notify_shutdown(&mut prepared_outer_0);
    drop(prepared_outer_0);
    state.clear(&mut prepared_outer_1).await;
    drop(prepared_outer_1);
    state.drain_native(LOCAL_BUDGET).await.unwrap();
    resource_error(&result.unwrap_err());
    assert_eq!(coordinators, 0);
    assert!(source.calls.lock().unwrap().is_empty());
    assert_eq!(source.paired_calls.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn admission_policy_reload_removes_planned_target_without_adding_new_one() {
    use crate::access::test_support::{grant, PolicyFixture, TOKEN};
    const C: &str = "newly-authorized-paired";
    let fixture = PolicyFixture::new(vec![grant(
        TOKEN,
        vec![Permission::ManageServer, Permission::ManageTunnels],
    )]);
    policy_devices(&fixture, &[A, B]);
    let source = FakeSource::new();
    *source.paired.lock().unwrap() = [A, B, C]
        .into_iter()
        .map(|udid| PairedMobdev2Device {
            udid: udid.into(),
            host: "192.0.2.8".into(),
        })
        .collect();
    let state = TunnelAgentState::with_source(
        TunMode::Userspace,
        AccessControl::from_file(fixture.path.clone())
            .await
            .unwrap(),
        Upstreams::default(),
        source.clone(),
        TUNNEL_CONNECT_TIMEOUT,
    );
    let mut prepared_outer_0 = state.prepare_cutover::<1>().unwrap();
    let mut prepared_outer_1 = state.prepare_cutover::<1>().unwrap();

    let paired_gate = Arc::new(Semaphore::new(0));
    *source.paired_gate.lock().unwrap() = Some(paired_gate.clone());
    let mut scan = Box::pin(state.refresh_once());
    assert!(futures_util::poll!(&mut scan).is_pending());
    assert_eq!(source.paired_active.load(Ordering::SeqCst), 1);
    // Pause the original post-discovery ticket admission, rather than the
    // newly added initial desired-state read of this same coordinator map.
    let coordinator = state.inner.lifecycle.hold_coordinator().await;
    paired_gate.add_permits(1);
    let pending = futures_util::poll!(&mut scan).is_pending();
    let paired_calls = source.paired_calls.load(Ordering::SeqCst);
    *source.paired_gate.lock().unwrap() = None;
    policy_devices(&fixture, &[A, C]);
    let (port, server) = spawn_test_router(agent_router(state.clone())).await;
    let response = client()
        .post(format!("http://127.0.0.1:{port}/auth/reload"))
        .bearer_auth(TOKEN)
        .send()
        .await
        .unwrap();
    let status = response.status();
    drop(response);
    drop(coordinator);
    let result = tokio::time::timeout(LOCAL_BUDGET, scan.as_mut()).await;
    drop(scan);
    let calls = (source.count(A), source.count(B), source.count(C));
    server.abort();
    server.await.unwrap_err();
    state.notify_shutdown(&mut prepared_outer_0);
    drop(prepared_outer_0);
    state.clear(&mut prepared_outer_1).await;
    drop(prepared_outer_1);
    state.drain_native(LOCAL_BUDGET).await.unwrap();
    assert!(pending);
    assert_eq!(paired_calls, 1);
    assert_eq!(status, StatusCode::OK);
    result.unwrap().unwrap();
    assert_eq!(calls, (1, 0, 0));
}
