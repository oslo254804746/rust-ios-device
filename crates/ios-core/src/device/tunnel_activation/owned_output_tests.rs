//! Public owned output transfer; scripted H2 and a channel-only native handle.
use super::*;
use crate::{OwnedConnectedDevice, OwnedRsdHandshake, XpcError, XpcMessage, XpcValue};
use std::{
    pin::Pin,
    task::{Context, Poll},
};
use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};
use tokio::sync::Semaphore;
const QUOTA: usize = 1024 * 1024;
struct Script(std::io::Cursor<Vec<u8>>);
impl AsyncRead for Script {
    fn poll_read(
        mut self: Pin<&mut Self>,
        _: &mut Context<'_>,
        buffer: &mut ReadBuf<'_>,
    ) -> Poll<std::io::Result<()>> {
        let count = std::io::Read::read(&mut self.0, buffer.initialize_unfilled())?;
        buffer.advance(count);
        Poll::Ready(Ok(()))
    }
}
impl AsyncWrite for Script {
    fn poll_write(
        self: Pin<&mut Self>,
        _: &mut Context<'_>,
        bytes: &[u8],
    ) -> Poll<std::io::Result<usize>> {
        Poll::Ready(Ok(bytes.len()))
    }
    fn poll_flush(self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        Poll::Ready(Ok(()))
    }
    fn poll_shutdown(self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        Poll::Ready(Ok(()))
    }
}
fn dict(values: impl IntoIterator<Item = (&'static str, XpcValue)>) -> XpcValue {
    XpcValue::Dictionary(values.into_iter().map(|(k, v)| (k.into(), v)).collect())
}
fn frame(kind: u8, stream: u32, payload: &[u8]) -> Vec<u8> {
    let length = payload.len();
    let mut out = vec![
        (length >> 16) as u8,
        (length >> 8) as u8,
        length as u8,
        kind,
        0,
    ];
    out.extend_from_slice(&stream.to_be_bytes());
    out.extend_from_slice(payload);
    out
}
async fn directory(bank: &Arc<Semaphore>) -> OwnedRsdHandshake {
    let message = XpcMessage {
        flags: crate::xpc_message_flags::ALWAYS_SET,
        msg_id: 0,
        body: Some(dict([
            ("MessageType", XpcValue::String("Handshake".into())),
            (
                "Properties",
                dict([(
                    "UniqueDeviceID",
                    XpcValue::String("owned-output-fixture".into()),
                )]),
            ),
            (
                "Services",
                dict([(
                    "svc.shim.remote",
                    dict([
                        ("Port", XpcValue::Uint64(12345)),
                        (
                            "Properties",
                            dict([(
                                "Features",
                                XpcValue::Array(vec![XpcValue::String("original-feature".into())]),
                            )]),
                        ),
                    ]),
                )]),
            ),
        ])),
    };
    let mut input = frame(4, 0, &[]);
    input.extend_from_slice(&frame(0, 1, &crate::encode_xpc_message(&message).unwrap()));
    let mut framer =
        crate::H2Framer::connect_with_bank(Script(std::io::Cursor::new(input)), Some(bank))
            .await
            .unwrap();
    crate::owned_handshake_on_framer(&mut framer).await.unwrap()
}
fn connection(bank: &Arc<Semaphore>) -> TunnelConnection {
    TunnelConnection::new(
        DeviceInfo {
            udid: "owned-output-fixture".into(),
            device_id: 42,
            connection_type: "USB".into(),
            product_id: 0,
        },
        None,
        LockdownTransport::Tcp { host: "::1".into() },
        "owned output fixture",
    )
    .with_initial_rsd_bank(Some(bank))
}
fn native() -> (Arc<TunnelHandle>, tokio::sync::watch::Receiver<()>) {
    let (handle, receiver) = TunnelHandle::new(
        "owned-output-fixture".into(),
        crate::TunnelInfo {
            server_address: "::1".into(),
            server_rsd_port: 58783,
            client_address: "::1".into(),
            client_mtu: 1280,
        },
        None,
    );
    (Arc::new(handle), receiver)
}
#[tokio::test]
async fn owned_initial_connection_public_getter_services_metadata_and_exports_keep_original_fee() {
    let bank = Arc::new(Semaphore::new(QUOTA));
    let directory = directory(&bank).await;
    let paid = directory.known_bytes();
    let uid = directory.udid().as_ptr();
    let (native, receiver) = native();
    let mut device = connection(&bank).into_device(
        native.clone(),
        InitialProbe::Owned(super::super::rsd_observation::RsdProbeOutcome::Completed(
            Ok(directory),
        )),
    );
    let full = device.retained_metadata_bytes().unwrap();
    let directory = device.owned_initial_rsd.take().unwrap();
    assert_eq!(full, device.retained_metadata_bytes().unwrap() + paid);
    device.owned_initial_rsd = Some(directory);
    let device = OwnedConnectedDevice::new(device);
    assert_eq!(device.info().device_id, 42);
    assert_eq!(
        device.initial_rsd_probe().state(),
        crate::InitialRsdProbeState::Available
    );
    assert!(Arc::ptr_eq(device.tunnel_handle().unwrap(), &native));
    assert_eq!(device.initial_rsd_bytes_in(&bank).unwrap(), paid);
    let other = Arc::new(Semaphore::new(QUOTA));
    assert_eq!(
        device.initial_rsd_bytes_in(&other).unwrap_err().class(),
        crate::ErrorClass::ResourceLimit
    );
    assert_eq!(other.available_permits(), QUOTA);
    let service = device.rsd().unwrap().get_resolved_service("svc").unwrap();
    let pointer = service.name().as_ptr();
    bank.close();
    let missing = tokio::time::timeout(
        std::time::Duration::from_millis(100),
        device.connect_rsd_service("missing"),
    )
    .await
    .unwrap()
    .err()
    .unwrap();
    assert!(matches!(
        missing,
        CoreError::Xpc(XpcError::ServiceNotFound(_))
    ));
    let shim = device.connect_xpc_service("svc").await.err().unwrap();
    assert_eq!(shim.class(), crate::ErrorClass::Unsupported);
    let exported = device.into_rsd().unwrap();
    assert_eq!(exported.udid().as_ptr(), uid);
    drop(exported);
    native.request_stop();
    drop((native, receiver));
    assert_eq!(service.name().as_ptr(), pointer);
    assert_eq!(service.features().collect::<Vec<_>>(), ["original-feature"]);
    assert_eq!(bank.available_permits(), QUOTA - paid);
    drop(service);
    assert_eq!(bank.available_permits(), QUOTA);
}
#[test]
fn owned_initial_failed_observation_keeps_original_io_source_and_bank_validation_before_native() {
    let bank = Arc::new(Semaphore::new(QUOTA));
    let other = Arc::new(Semaphore::new(QUOTA));
    assert!(connection(&bank).validate_initial_bank(Some(&bank)).is_ok());
    for proposed in [None, Some(&other)] {
        assert_eq!(
            connection(&bank)
                .validate_initial_bank(proposed)
                .unwrap_err()
                .class(),
            crate::ErrorClass::ResourceLimit
        );
    }
    let error = std::io::Error::new(
        std::io::ErrorKind::PermissionDenied,
        "original-owned-probe-source",
    );
    let pointer = error.get_ref().unwrap() as *const dyn std::error::Error as *const ();
    let (native, receiver) = native();
    let device = OwnedConnectedDevice::new(connection(&bank).into_device(
        native.clone(),
        InitialProbe::Owned(super::super::rsd_observation::RsdProbeOutcome::Completed(
            Err(CoreError::Io(error)),
        )),
    ));
    assert!(device.rsd().is_none());
    assert_eq!(
        device.initial_rsd_probe().state(),
        crate::InitialRsdProbeState::Failed
    );
    let CoreError::Io(error) = device.initial_rsd_probe().error().unwrap() else {
        panic!("original IO variant lost")
    };
    assert_eq!(
        error.get_ref().unwrap() as *const dyn std::error::Error as *const (),
        pointer
    );
    assert_eq!(
        device.initial_rsd_probe().error_class(),
        Some(crate::ErrorClass::PermissionDenied)
    );
    drop((device, native, receiver));
    assert_eq!(bank.available_permits(), QUOTA);
}

#[tokio::test]
async fn prepared_owned_connection_swap_prefers_current_catalogue_and_keeps_export_after_device_drop(
) {
    use super::super::existing_rsd::{RsdHandshakeMode, RsdSelectionReceipt};
    use super::super::prepared_rsd::{exact_vec, PreparedRsdCatalogue};
    let bank = Arc::new(Semaphore::new(QUOTA));
    let initial = directory(&bank).await;
    let initial_paid = initial.known_bytes();
    let (native, receiver) = native();
    let mut device = OwnedConnectedDevice::new(connection(&bank).into_device(
        native.clone(),
        InitialProbe::Owned(super::super::rsd_observation::RsdProbeOutcome::Completed(
            Ok(initial),
        )),
    ));
    let XpcValue::Dictionary(input) = dict([
        ("MessageType", XpcValue::String("Handshake".into())),
        (
            "Properties",
            dict([(
                "UniqueDeviceID",
                XpcValue::String("owned-output-fixture".into()),
            )]),
        ),
        // Removing the original service proves current refresh is preferred
        // without opening a socket or retrying initial discovery.
        ("Services", dict([])),
    ]) else {
        unreachable!()
    };
    let prepared = PreparedRsdCatalogue::decode(&input, &bank).unwrap();
    let (fee, mut modes) = exact_vec(&bank, 1).unwrap();
    modes.push(RsdHandshakeMode::Direct);
    let current = prepared
        .finish_owned(
            RsdSelectionReceipt {
                handshake_mode: RsdHandshakeMode::Direct,
                attempted_modes: modes,
            },
            fee,
        )
        .unwrap();
    let paid = current.known_bytes();
    let address = std::ptr::from_ref(&*current);
    let (other, other_receiver) = self::native();
    assert!(device
        .swap_current_rsd_catalogue(&other, Some(current.clone()))
        .is_err());
    assert!(device.current_rsd_catalogue().is_none());
    assert!(device
        .swap_current_rsd_catalogue(&native, Some(current.clone()))
        .unwrap()
        .is_none());
    assert!(device.prepared_rsd_owner().unwrap().ptr_eq(&current));
    assert_eq!(
        std::ptr::from_ref(device.current_rsd_catalogue().unwrap()),
        address
    );
    bank.close();
    let error = device.connect_rsd_service("svc").await.err().unwrap();
    assert!(matches!(
        error,
        CoreError::Xpc(XpcError::ServiceNotFound(_))
    ));
    assert_eq!(bank.available_permits(), QUOTA - initial_paid - paid);
    drop((device, native, receiver, other, other_receiver));
    assert_eq!(bank.available_permits(), QUOTA - paid);
    assert_eq!(current.udid(), "owned-output-fixture");
    drop(current);
    assert_eq!(bank.available_permits(), QUOTA);
}
