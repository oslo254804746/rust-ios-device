use super::*;
use std::io;
use tokio::time::{timeout, Duration};

struct Directory(PathBuf);
impl Directory {
    fn new() -> Self {
        let directory =
            Self(std::env::temp_dir().join(format!("ios-dproxy-record-{}", uuid::Uuid::new_v4())));
        fs::create_dir(&directory.0).unwrap();
        directory
    }

    fn missing_error(&self) -> io::Error {
        File::open(self.0.join("owned-private-missing")).unwrap_err()
    }
}
impl Drop for Directory {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn malformed_plist() -> Vec<u8> {
    let payload = b"bplist00owned-private-invalid-plist";
    assert!(plist::from_bytes::<plist::Value>(payload).is_err());
    let mut bytes = (payload.len() as u32).to_be_bytes().to_vec();
    bytes.extend_from_slice(payload);
    bytes
}

fn native_then_fatal() -> Vec<u8> {
    let mut message = crate::xpc::message::encode_message(&crate::xpc::XpcMessage {
        flags: crate::xpc::message::flags::ALWAYS_SET,
        msg_id: 3,
        body: Some(crate::xpc::XpcValue::String("owned-private-value".into())),
    })
    .unwrap()
    .to_vec();
    let offset = message
        .windows(19)
        .position(|bytes| bytes == b"owned-private-value")
        .unwrap();
    message[offset] = 0xff;
    let len = message.len();
    let mut bytes = vec![
        (len >> 16) as u8,
        (len >> 8) as u8,
        len as u8,
        0,
        0,
        0,
        0,
        0,
        3,
    ];
    bytes.extend_from_slice(&message);
    let oversized = crate::xpc::h2_raw::MAX_FRAME_PAYLOAD + 1;
    bytes.extend_from_slice(&[
        (oversized >> 16) as u8,
        (oversized >> 8) as u8,
        oversized as u8,
        0,
        0,
        0,
        0,
        0,
        3,
    ]);
    bytes
}

fn event() -> ProxyEvent {
    ProxyEvent {
        timestamp_ms: 0,
        direction: "host->device".into(),
        protocol: "lockdown".into(),
        summary: "owned-private-summary".into(),
        decoded: serde_json::json!({"payload":"owned-private-data"}),
    }
}

#[test]
fn dproxy_recording_observer_owns_plist_before_persisted_event() {
    let directory = Directory::new();
    let mut recorder = ProxyRecorder::new(&directory.0, ProxyProtocol::Lockdown).unwrap();
    let mut failures = Vec::new();
    recorder
        .record_chunk_with_observer(
            Direction::HostToDevice,
            &malformed_plist(),
            &mut |failure| {
                assert!(fs::read(directory.0.join("events.ndjson"))
                    .unwrap()
                    .is_empty());
                failures.push(failure);
            },
        )
        .unwrap();
    assert_eq!(failures.len(), 1);
    assert!(failures[0].error().unwrap().is::<plist::Error>());
    assert_eq!(failures[0].event_index(), Some(0));
    assert_eq!(failures[0].error_class(), Some(crate::ErrorClass::Protocol));
    assert_eq!(
        fs::read(directory.0.join("host-to-device.bin")).unwrap(),
        malformed_plist()
    );
    let lines = fs::read_to_string(directory.0.join("events.ndjson")).unwrap();
    assert_eq!(lines.lines().count(), 1);
    let decoded: serde_json::Value = serde_json::from_str(lines.trim()).unwrap();
    assert_eq!(
        decoded["decoded"]["raw"],
        hex::encode(&malformed_plist()[4..])
    );
    assert!(failures
        .remove(0)
        .into_error()
        .unwrap()
        .is::<plist::Error>());
}

#[test]
fn dproxy_recording_observes_native_and_guard_before_fatal_return() {
    let directory = Directory::new();
    let mut recorder = ProxyRecorder::new(&directory.0, ProxyProtocol::Xpc).unwrap();
    let bytes = native_then_fatal();
    let mut failures = Vec::new();
    let error = recorder
        .record_chunk_with_observer(Direction::DeviceToHost, &bytes, &mut |failure| {
            failures.push(failure)
        })
        .unwrap_err();
    assert!(matches!(error, DproxyError::Protocol(_)));
    assert_eq!(failures.len(), 2);
    assert!(failures[0].error().unwrap().is::<crate::xpc::XpcError>());
    assert_eq!(failures[0].event_index(), Some(0));
    assert_eq!(failures[1].kind(), DecodeFailureKind::ResourceLimit);
    assert_eq!(failures[1].error_class(), None);
    assert_eq!(failures[1].event_index(), None);
    assert_eq!(
        fs::read(directory.0.join("device-to-host.bin")).unwrap(),
        bytes
    );
    assert!(fs::read(directory.0.join("events.ndjson"))
        .unwrap()
        .is_empty());
}

#[test]
fn dproxy_recording_observation_survives_actual_event_file_write_failure() {
    let directory = Directory::new();
    let mut recorder = ProxyRecorder::new(&directory.0, ProxyProtocol::Lockdown).unwrap();
    recorder.events = File::open(directory.0.join("events.ndjson")).unwrap();
    let mut failures = Vec::new();
    let error = recorder
        .record_chunk_with_observer(
            Direction::HostToDevice,
            &malformed_plist(),
            &mut |failure| failures.push(failure),
        )
        .unwrap_err();
    assert_eq!(failures.len(), 1);
    assert!(failures[0].error().unwrap().is::<plist::Error>());
    let DproxyError::Serde(error) = error else {
        panic!("expected serializer IO failure")
    };
    assert!(error.is_io());
    let original: io::Error = error.into();
    assert!(original.raw_os_error().is_some());
    assert!(fs::read(directory.0.join("events.ndjson"))
        .unwrap()
        .is_empty());
}

#[test]
fn dproxy_recording_raw_file_failure_precedes_decoder_and_callback() {
    let directory = Directory::new();
    let mut recorder = ProxyRecorder::new(&directory.0, ProxyProtocol::Lockdown).unwrap();
    recorder.host_to_device_raw = File::open(directory.0.join("host-to-device.bin")).unwrap();
    let mut calls = 0;
    let error = recorder
        .record_chunk_with_observer(Direction::HostToDevice, &malformed_plist(), &mut |_| {
            calls += 1
        })
        .unwrap_err();
    let DproxyError::Io(error) = error else {
        panic!("expected raw IO failure")
    };
    assert!(error.raw_os_error().is_some());
    assert_eq!(calls, 0);
    assert!(recorder.host_to_device_decoder.buffer.is_empty());
    assert!(fs::read(directory.0.join("events.ndjson"))
        .unwrap()
        .is_empty());
}

struct Writer {
    bytes: Vec<u8>,
    limit: usize,
    write_error: Option<io::Error>,
    flush_error: Option<io::Error>,
    flush_calls: usize,
}
impl Writer {
    fn new(limit: usize, error: Option<io::Error>) -> Self {
        Self {
            bytes: Vec::new(),
            limit,
            write_error: error,
            flush_error: None,
            flush_calls: 0,
        }
    }
}
impl Write for Writer {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        let count = bytes.len().min(self.limit.saturating_sub(self.bytes.len()));
        if count == 0 && !bytes.is_empty() {
            return Err(self.write_error.take().expect("one scripted write failure"));
        }
        self.bytes.extend_from_slice(&bytes[..count]);
        Ok(count)
    }
    fn flush(&mut self) -> io::Result<()> {
        self.flush_calls += 1;
        self.flush_error.take().map_or(Ok(()), Err)
    }
}

#[test]
fn dproxy_recording_diagnostic_error_returns_original_io_without_panic() {
    let directory = Directory::new();
    let original = directory.missing_error();
    let code = original.raw_os_error();
    let class = crate::classify_error(&original);
    let mut diagnostic = Writer::new(0, Some(original));
    let mut events = Writer::new(usize::MAX, None);
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        write_events_to(&mut events, vec![event(), event()], |event| {
            write_event_diagnostic(&mut diagnostic, event)
        })
    }));
    assert!(result.is_ok(), "diagnostic IO must be returned, not panic");
    let error = result.unwrap().unwrap_err();
    assert_eq!(crate::classify_error(&error), class);
    let DproxyError::Io(error) = error else {
        panic!("expected diagnostic IO failure")
    };
    assert_eq!(error.raw_os_error(), code);
    assert_eq!(
        events.bytes.iter().filter(|byte| **byte == b'\n').count(),
        1
    );
    assert_eq!(events.flush_calls, 0);
}

#[test]
fn dproxy_recording_event_write_newline_and_flush_keep_native_failures() {
    let directory = Directory::new();
    let json_size = serde_json::to_vec(&event()).unwrap().len();
    for limit in [0, json_size, usize::MAX] {
        let original = directory.missing_error();
        let code = original.raw_os_error();
        let mut writer = Writer::new(limit, None);
        if limit == usize::MAX {
            writer.flush_error = Some(original);
        } else {
            writer.write_error = Some(original);
        }
        let mut diagnostic_calls = 0;
        let error = write_events_to(&mut writer, vec![event()], |_| {
            diagnostic_calls += 1;
            Ok(())
        })
        .unwrap_err();
        assert_eq!(crate::classify_error(&error), crate::ErrorClass::NotFound);
        let original = match error {
            DproxyError::Io(error) => {
                assert_ne!(limit, 0);
                error
            }
            DproxyError::Serde(error) => {
                assert_eq!(limit, 0);
                error.into()
            }
            error => panic!("unexpected event write failure: {error:?}"),
        };
        assert_eq!(original.raw_os_error(), code);
        assert_eq!(diagnostic_calls, usize::from(limit == usize::MAX));
        assert_eq!(writer.flush_calls, usize::from(limit == usize::MAX));
    }
    let mut lines = Writer::new(usize::MAX, None);
    let mut diagnostics = Vec::new();
    write_events_to(&mut lines, vec![event()], |event| {
        write_event_diagnostic(&mut diagnostics, event)
    })
    .unwrap();
    assert_eq!(lines.flush_calls, 1);
    assert!(String::from_utf8(lines.bytes)
        .unwrap()
        .contains("owned-private-data"));
    let diagnostic = String::from_utf8(diagnostics).unwrap();
    assert!(diagnostic.starts_with("[lockdown] host->device summary=id:"));
    assert!(!diagnostic.contains("owned-private"));
}

#[tokio::test]
async fn dproxy_recording_proxy_delivers_both_directions_and_same_wire() {
    let directory = Directory::new();
    let recorder = ProxyRecorder::new(&directory.0, ProxyProtocol::Lockdown).unwrap();
    let (local, mut client) = tokio::io::duplex(1024);
    let (remote, mut device) = tokio::io::duplex(1024);
    let bytes = malformed_plist();
    let mut failures = Vec::new();
    let proxy = proxy_bidirectional_with_observer(local, remote, recorder, |failure| {
        failures.push(failure)
    });
    let peers = async {
        client.write_all(&bytes).await.unwrap();
        device.write_all(&bytes).await.unwrap();
        client.shutdown().await.unwrap();
        device.shutdown().await.unwrap();
        let mut from_device = Vec::new();
        let mut from_client = Vec::new();
        let (a, b) = tokio::join!(
            client.read_to_end(&mut from_device),
            device.read_to_end(&mut from_client)
        );
        a.unwrap();
        b.unwrap();
        assert_eq!(from_client, bytes);
        assert_eq!(from_device, bytes);
    };
    let (result, ()) = timeout(Duration::from_secs(1), async { tokio::join!(proxy, peers) })
        .await
        .unwrap();
    result.unwrap();
    assert_eq!(failures.len(), 2);
    assert_eq!(
        failures
            .iter()
            .filter(|failure| matches!(failure.direction(), Direction::HostToDevice))
            .count(),
        1
    );
    for failure in failures {
        assert!(failure.into_error().unwrap().is::<plist::Error>());
    }
    assert_eq!(
        fs::read(directory.0.join("host-to-device.bin")).unwrap(),
        bytes
    );
    assert_eq!(
        fs::read(directory.0.join("device-to-host.bin")).unwrap(),
        bytes
    );
    assert_eq!(
        fs::read_to_string(directory.0.join("events.ndjson"))
            .unwrap()
            .lines()
            .count(),
        2
    );
}

#[tokio::test]
async fn dproxy_recording_proxy_fatal_observations_precede_zero_forwarding() {
    let directory = Directory::new();
    let recorder = ProxyRecorder::new(&directory.0, ProxyProtocol::Xpc).unwrap();
    let (local, mut client) = tokio::io::duplex(1024);
    let (remote, mut device) = tokio::io::duplex(1024);
    let bytes = native_then_fatal();
    let mut failures = Vec::new();
    let proxy = proxy_bidirectional_with_observer(local, remote, recorder, |failure| {
        failures.push(failure)
    });
    let peers = async {
        client.write_all(&bytes).await.unwrap();
        client.shutdown().await.unwrap();
        let mut forwarded = Vec::new();
        device.read_to_end(&mut forwarded).await.unwrap();
        assert!(forwarded.is_empty());
        let mut reply = Vec::new();
        client.read_to_end(&mut reply).await.unwrap();
        assert!(reply.is_empty());
    };
    let (result, ()) = timeout(Duration::from_secs(1), async { tokio::join!(proxy, peers) })
        .await
        .unwrap();
    assert!(matches!(result, Err(DproxyError::Protocol(_))));
    assert_eq!(failures.len(), 2);
    assert!(failures[0].error().unwrap().is::<crate::xpc::XpcError>());
    assert_eq!(failures[1].phase(), DecodePhase::H2Frame);
    assert_eq!(
        fs::read(directory.0.join("host-to-device.bin")).unwrap(),
        bytes
    );
    assert!(fs::read(directory.0.join("events.ndjson"))
        .unwrap()
        .is_empty());
}

#[test]
fn dproxy_recording_broken_dtx_keeps_raw_and_one_source_only() {
    let directory = Directory::new();
    let mut recorder = ProxyRecorder::new(&directory.0, ProxyProtocol::Dtx).unwrap();
    let mut failures = Vec::new();
    let mut observer = |failure| failures.push(failure);
    recorder
        .record_chunk_with_observer(Direction::HostToDevice, &[0; 32], &mut observer)
        .unwrap();
    recorder
        .record_chunk_with_observer(Direction::HostToDevice, &[1; 8], &mut observer)
        .unwrap();
    assert_eq!(failures.len(), 1);
    assert!(matches!(
        failures[0]
            .error()
            .unwrap()
            .downcast_ref::<crate::services::dtx::DtxError>(),
        Some(crate::services::dtx::DtxError::BadMagic(0))
    ));
    assert!(recorder.host_to_device_decoder.dtx_broken);
    assert!(recorder.host_to_device_decoder.buffer.is_empty());
    assert_eq!(
        fs::read(directory.0.join("host-to-device.bin")).unwrap(),
        [vec![0; 32], vec![1; 8]].concat()
    );
}

#[test]
fn dproxy_recording_empty_input_skips_all_writes_and_callback() {
    let directory = Directory::new();
    let mut recorder = ProxyRecorder::new(&directory.0, ProxyProtocol::Lockdown).unwrap();
    recorder.events = File::open(directory.0.join("events.ndjson")).unwrap();
    recorder.host_to_device_raw = File::open(directory.0.join("host-to-device.bin")).unwrap();
    let mut calls = 0;
    recorder
        .record_chunk_with_observer(Direction::HostToDevice, &[], &mut |_| calls += 1)
        .unwrap();
    assert_eq!(calls, 0);
    assert!(recorder.host_to_device_decoder.buffer.is_empty());
}

#[cfg(target_os = "linux")]
#[test]
fn dproxy_recording_real_stderr_failure_returns_native_io() {
    const CHILD: &str = "IOS_DPROXY_NATIVE_STDERR_CHILD";
    if std::env::var_os(CHILD).is_some() {
        let directory = Directory::new();
        let mut recorder = ProxyRecorder::new(&directory.0, ProxyProtocol::Binary).unwrap();
        let error = recorder
            .record_meta_event(
                Direction::HostToDevice,
                "custom",
                "owned-private-summary",
                serde_json::Value::Null,
            )
            .unwrap_err();
        assert_eq!(crate::classify_error(&error), crate::ErrorClass::Io);
        let DproxyError::Io(error) = error else {
            panic!("expected actual stderr IO")
        };
        assert_eq!(error.raw_os_error(), Some(28)); // Linux ENOSPC from /dev/full.
        assert_eq!(
            fs::read_to_string(directory.0.join("events.ndjson"))
                .unwrap()
                .lines()
                .count(),
            1
        );
        println!("DPROXY_NATIVE_STDERR_ENOSPC_RETURNED");
        return;
    }
    // Only the owned child has redirected stderr; no device or global parent
    // descriptors are touched. --nocapture exercises actual std::io::stderr.
    let full = fs::OpenOptions::new()
        .write(true)
        .open("/dev/full")
        .unwrap();
    let output = std::process::Command::new(std::env::current_exe().unwrap())
        .arg("--exact")
        .arg("services::dproxy::recording_contract_tests::dproxy_recording_real_stderr_failure_returns_native_io")
        .arg("--nocapture")
        .env(CHILD, "1")
        .stderr(full)
        .output().unwrap();
    assert!(
        output.status.success(),
        "stderr child: {}",
        String::from_utf8_lossy(&output.stdout)
    );
    assert!(String::from_utf8(output.stdout)
        .unwrap()
        .contains("DPROXY_NATIVE_STDERR_ENOSPC_RETURNED"));
}
