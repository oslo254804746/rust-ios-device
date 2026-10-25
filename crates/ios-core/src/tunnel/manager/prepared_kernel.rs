//! Final exposure on the same private, authenticated and checked runtime.
use super::*;
use crate::tunnel::owner::phase_outcomes;
use tokio::time::Instant;

impl TunnelHandle {
    pub(super) fn prepared_kernel(&self) -> Option<&KernelRuntime> {
        match &self._runtime {
            TunnelRuntime::Prepared { kernel, .. } => kernel.get(),
            _ => None,
        }
    }

    pub(crate) async fn finish_prepared_kernel_with(
        &self,
        bank: Arc<tokio::sync::Semaphore>,
        deadline: Instant,
        prepare: impl FnOnce(
            &UserspaceTunDevice,
            Arc<tokio::sync::Semaphore>,
            &TunnelInfo,
            Instant,
        ) -> Result<KernelRuntime, TunnelError>,
    ) -> Result<(), TunnelError> {
        let TunnelRuntime::Prepared {
            runtime,
            mode: TunMode::Kernel,
            ready,
            kernel,
            activation_claimed,
        } = &self._runtime
        else {
            return Err(TunnelError::Protocol(
                "prepared runtime is not selected for Kernel".into(),
            ));
        };
        let mut stop = StopUnlessReady {
            handle: self,
            completed: false,
        };
        runtime.private_work_result()?;
        if activation_claimed
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .is_err()
        {
            return Err(TunnelError::Protocol(
                "prepared Kernel activation already claimed".into(),
            ));
        }
        if Instant::now() >= deadline {
            let elapsed = tokio::time::timeout_at(deadline, std::future::pending::<()>())
                .await
                .expect_err("expired original preparation clock");
            return runtime
                .phase_outcomes()
                .expect("original Kernel phase hub")
                .attempt_result(runtime.ledger(), None, None, Some(elapsed))
                .map_err(TunnelError::Phases);
        }
        // Admission creates no native adapter. All native/undo destinations
        // become owned by this exact handle before quiescence or NativeCreate.
        let prepared = prepare(runtime, bank, &self.info, deadline)?;
        if let Err(uninstalled) = kernel.set(prepared) {
            uninstalled.request_stop();
            return Err(TunnelError::Protocol(
                "original prepared Kernel owner already installed".into(),
            ));
        }
        let kernel = kernel.get().expect("same installed Kernel owner");
        phase_outcomes::activation_until(
            deadline,
            runtime.phase_outcomes().expect("original Kernel phase hub"),
            runtime.ledger(),
            kernel.native_ledger(),
            kernel.undo_ledger(),
            async {
                runtime.private_work_result()?;
                let lease = runtime.quiesce_and_flush().await?;
                tokio::select! {
                    biased;
                    original = runtime.transport_interrupted() => {
                        // A native failure stops the original transport too.
                        // Its already captured same-hub cause wins that local
                        // cancellation fallback in this very selected poll.
                        kernel.work_result()?;
                        return Err(original);
                    },
                    result = kernel.activate(lease) => result?,
                }
                kernel.work_result()?;
                if !runtime.transport_is_alive() || !kernel.ready() || !kernel.is_alive() {
                    return Err(std::io::Error::other(crate::OperationCancelled::new()).into());
                }
                ready.store(true, Ordering::Release);
                Ok(())
            },
        )
        .await?;
        stop.completed = true;
        Ok(())
    }

    pub(super) async fn shutdown_prepared_kernel_until(
        &self,
        runtime: &UserspaceTunDevice,
        kernel: &KernelRuntime,
        deadline: Result<Instant, TunnelError>,
    ) -> Result<(), TunnelError> {
        #[cfg(any(target_os = "linux", target_os = "macos"))]
        let cleanup_at = deadline.as_ref().ok().copied();
        phase_outcomes::join_until(
            deadline,
            runtime.phase_outcomes().expect("original Kernel phase hub"),
            runtime.ledger(),
            kernel.native_ledger(),
            kernel.undo_ledger(),
            async {
                tokio::join!(kernel.join_work(), runtime.collect_original());
                // Original and native packet/control users have really joined.
                // Undo was admitted before exposure and retains exact receipts.
                #[cfg(target_os = "windows")]
                kernel.undo().await;
                #[cfg(any(target_os = "linux", target_os = "macos"))]
                kernel
                    .undo_at(cleanup_at.expect("validated same cleanup clock"))
                    .await;
                #[cfg(target_os = "windows")]
                {
                    runtime.release_original();
                    kernel.release_flows();
                }
                #[cfg(any(target_os = "linux", target_os = "macos"))]
                if kernel.is_terminal() {
                    runtime.release_original();
                    kernel.release_flows();
                }
            },
        )
        .await
    }
}

struct StopUnlessReady<'a> {
    handle: &'a TunnelHandle,
    completed: bool,
}
impl Drop for StopUnlessReady<'_> {
    fn drop(&mut self) {
        if !self.completed {
            self.handle.request_stop();
        }
    }
}

#[cfg(all(test, target_os = "windows"))]
mod tests {
    use super::*;
    mod cancellation_tests;
    use crate::tunnel::tun::userspace::{ScriptOperation as Op, WindowsScript};
    use std::sync::atomic::AtomicUsize;
    use std::time::Duration;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::sync::Semaphore;

    const TOTAL: usize = 32 * 1024 * 1024;
    const OBSERVE: Duration = Duration::from_secs(5);

    async fn candidate(bank: &Arc<Semaphore>) -> (Arc<TunnelHandle>, tokio::io::DuplexStream) {
        let (stream, peer) = tokio::io::duplex(1280);
        let (runtime, launch) = UserspaceTunDevice::prepare_private_validated_with_bank(
            "fd00::2",
            "fd00::1",
            crate::tunnel::ValidatedMtu::new(1280).unwrap(),
            stream,
            bank.clone(),
            1,
            true,
        )
        .await
        .unwrap();
        let handle = Arc::new(TunnelHandle::new_prepared(
            "internal-scripted-native".into(),
            TunnelInfo {
                client_address: "fd00::2".into(),
                server_address: "fd00::1".into(),
                client_mtu: 1280,
                server_rsd_port: 58783,
            },
            TunMode::Kernel,
            runtime,
        ));
        launch().unwrap();
        (handle, peer)
    }

    fn udp(from: &str, to: &str) -> Vec<u8> {
        let mut packet = vec![0; 48];
        packet[0] = 0x60;
        packet[5] = 8;
        packet[6] = 17;
        packet[7] = 64;
        packet[8..24].copy_from_slice(&from.parse::<std::net::Ipv6Addr>().unwrap().octets());
        packet[24..40].copy_from_slice(&to.parse::<std::net::Ipv6Addr>().unwrap().octets());
        packet[40..].copy_from_slice(&[0x51, 0x52, 0x53, 0x54, 0, 8, 0x55, 0x56]);
        packet
    }

    async fn activate(
        handle: &TunnelHandle,
        bank: &Arc<Semaphore>,
        script: &WindowsScript,
    ) -> Result<(), TunnelError> {
        handle
            .finish_prepared_kernel_with(
                bank.clone(),
                Instant::now() + OBSERVE,
                |runtime, bank, info, deadline| {
                    runtime.prepare_kernel_scripted(bank, info, deadline, script)
                },
            )
            .await
    }

    async fn eof(peer: &mut tokio::io::DuplexStream) {
        assert!(matches!(
            tokio::time::timeout(OBSERVE, peer.read(&mut [0; 1])).await,
            Ok(Ok(0))
        ));
    }

    #[tokio::test]
    async fn same_owner_native_activation_forwards_both_directions_then_joins_before_exact_undo() {
        for derived in [false, true] {
            let bank = Arc::new(Semaphore::new(TOTAL));
            let script = WindowsScript::new().unwrap();
            script.automatic_route(derived);
            let (handle, mut peer) = candidate(&bank).await;
            assert!(!handle.ready_for_publication());
            activate(&handle, &bank, &script).await.unwrap();
            assert!(handle.ready_for_publication());
            assert!(handle.is_alive());
            assert!(handle.userspace_port.is_none());
            let inbound = udp("fd00::1", "fd00::2");
            peer.write_all(&inbound).await.unwrap();
            tokio::time::timeout(OBSERVE, script.wait_for(Op::Send))
                .await
                .unwrap();
            assert_eq!(script.take_sent().unwrap(), inbound);
            let outbound = udp("fd00::2", "fd00::1");
            script.queue_received(outbound.clone()).unwrap();
            let mut received = vec![0; outbound.len()];
            tokio::time::timeout(OBSERVE, peer.read_exact(&mut received))
                .await
                .unwrap()
                .unwrap();
            assert_eq!(received, outbound);
            let held = bank
                .clone()
                .try_acquire_many_owned(bank.available_permits() as u32)
                .unwrap();
            bank.close();
            assert_eq!(bank.available_permits(), 0);
            handle.shutdown(OBSERVE).await.unwrap();
            handle.shutdown_status().unwrap();
            assert!(!handle.ready_for_publication());
            assert!(!handle.is_alive());
            eof(&mut peer).await;
            let calls = script.calls();
            let end = calls.iter().position(|op| *op == Op::EndSession).unwrap();
            let close = calls.iter().position(|op| *op == Op::Close).unwrap();
            assert!(end < close);
            assert!(calls.iter().enumerate().all(|(index, op)| !matches!(
                op,
                Op::Read | Op::Send | Op::AllocateSend
            ) || index < end));
            assert_eq!(script.count(Op::RemoveRoute), usize::from(!derived));
            assert_eq!(script.count(Op::RemoveAddress), 1);
            assert_eq!(script.count(Op::Close), 1);
            handle.shutdown(OBSERVE).await.unwrap();
            assert_eq!(script.count(Op::Close), 1);
            drop((handle, held));
            assert_eq!(bank.available_permits(), TOTAL);
        }
    }

    #[tokio::test]
    async fn cancelled_dad_wait_keeps_actual_native_joins_and_undo_until_retry_at_closed_zero() {
        let bank = Arc::new(Semaphore::new(TOTAL));
        let script = WindowsScript::new().unwrap();
        script.tentative(true);
        let (handle, mut peer) = candidate(&bank).await;
        let mut activation = Box::pin(activate(&handle, &bank, &script));
        tokio::select! {
            biased;
            result = &mut activation => panic!("tentative native address became ready: {result:?}"),
            _ = tokio::time::timeout(OBSERVE, script.wait_for(Op::Read)) => {},
        }
        assert!(script.count(Op::Read) > 0);
        assert!(!handle.ready_for_publication());
        let held = bank
            .clone()
            .try_acquire_many_owned(bank.available_permits() as u32)
            .unwrap();
        bank.close();
        drop(activation);
        assert_eq!(script.count(Op::Close), 0);
        handle.shutdown(OBSERVE).await.unwrap();
        handle.shutdown_status().unwrap();
        eof(&mut peer).await;
        assert_eq!(script.count(Op::Create), 1);
        assert_eq!(script.count(Op::RemoveAddress), 1);
        assert_eq!(script.count(Op::EndSession), 1);
        assert_eq!(script.count(Op::Close), 1);
        drop((handle, held));
        assert_eq!(bank.available_permits(), TOTAL);
    }

    #[tokio::test]
    async fn independent_route_and_address_undo_faults_keep_original_phase_causes_after_actual_close(
    ) {
        let bank = Arc::new(Semaphore::new(TOTAL));
        let script = WindowsScript::new().unwrap();
        script.automatic_route(false);
        let (handle, mut peer) = candidate(&bank).await;
        activate(&handle, &bank, &script).await.unwrap();
        script.fail(Op::RemoveRoute, 5);
        script.fail(Op::RemoveAddress, 87);
        let held = bank
            .clone()
            .try_acquire_many_owned(bank.available_permits() as u32)
            .unwrap();
        bank.close();
        let first = handle.shutdown(OBSERVE).await.unwrap_err();
        let TunnelError::Phases(first) = first else {
            panic!("same original phase report")
        };
        assert!(first.joins_complete());
        assert!(first.has_retained_outcomes());
        assert_eq!(first.cleanup().count(), 1);
        let repeated = handle.shutdown(OBSERVE).await.unwrap_err();
        let TunnelError::Phases(repeated) = repeated else {
            panic!("same repeated phase report")
        };
        assert!(Arc::ptr_eq(
            first.primary_shared().unwrap(),
            repeated.primary_shared().unwrap()
        ));
        assert_eq!(script.count(Op::RemoveRoute), 1);
        assert_eq!(script.count(Op::RemoveAddress), 1);
        assert_eq!(script.count(Op::Close), 1);
        eof(&mut peer).await;
        drop((first, repeated, handle, held));
        assert_eq!(bank.available_permits(), TOTAL);
    }

    #[tokio::test]
    async fn expired_preparation_never_constructs_native_owner_at_closed_zero_and_joins_original() {
        const TOTAL: usize = 16 * 1024 * 1024;
        let bank = Arc::new(Semaphore::new(TOTAL));
        let (stream, mut peer) = tokio::io::duplex(1280);
        let (runtime, launch) = UserspaceTunDevice::prepare_private_validated_with_bank(
            "fd00::2",
            "fd00::1",
            crate::tunnel::ValidatedMtu::new(1280).unwrap(),
            stream,
            bank.clone(),
            1,
            true,
        )
        .await
        .unwrap();
        let handle = Arc::new(TunnelHandle::new_prepared(
            "internal-scripted-transport".into(),
            TunnelInfo {
                client_address: "fd00::2".into(),
                server_address: "fd00::1".into(),
                client_mtu: 1280,
                server_rsd_port: 58783,
            },
            TunMode::Kernel,
            runtime,
        ));
        launch().unwrap();
        let calls = AtomicUsize::new(0);
        let held = bank
            .clone()
            .try_acquire_many_owned(bank.available_permits() as u32)
            .unwrap();
        bank.close();
        assert_eq!(bank.available_permits(), 0);
        let error = handle
            .finish_prepared_kernel_with(bank.clone(), Instant::now(), |_, _, _, _| {
                calls.fetch_add(1, Ordering::SeqCst);
                panic!("expired original clock constructed native factory");
            })
            .await
            .unwrap_err();
        assert_eq!(calls.load(Ordering::SeqCst), 0);
        let TunnelError::Phases(original) = &error else {
            panic!("original actual elapsed");
        };
        assert!(original.primary().is::<tokio::time::error::Elapsed>());
        assert!(handle.prepared_kernel().is_none());
        assert!(!handle.ready_for_publication());
        handle.shutdown(Duration::from_secs(2)).await.unwrap();
        handle.shutdown_status().unwrap();
        assert!(matches!(
            tokio::time::timeout(Duration::from_secs(2), peer.read(&mut [0; 1])).await,
            Ok(Ok(0)),
        ));
        drop((error, handle, held));
        assert_eq!(bank.available_permits(), TOTAL);
    }
}
