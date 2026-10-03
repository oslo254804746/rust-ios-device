use super::*;
use crate::ErrorClass;

fn h2(stream: u32, flags: u8, payload: &[u8]) -> Vec<u8> {
    let len = payload.len();
    let mut frame = vec![(len >> 16) as u8, (len >> 8) as u8, len as u8, 0, flags];
    frame.extend_from_slice(&stream.to_be_bytes());
    frame.extend_from_slice(payload);
    frame
}

fn oversized_h2() -> Vec<u8> {
    let len = crate::xpc::h2_raw::MAX_FRAME_PAYLOAD + 1;
    vec![
        (len >> 16) as u8,
        (len >> 8) as u8,
        len as u8,
        0,
        0,
        0,
        0,
        0,
        3,
    ]
}

fn xpc() -> Vec<u8> {
    crate::xpc::message::encode_message(&crate::xpc::XpcMessage {
        flags: crate::xpc::message::flags::ALWAYS_SET,
        msg_id: 7,
        body: Some(crate::xpc::XpcValue::String("owned-private-value".into())),
    })
    .unwrap()
    .to_vec()
}

fn bad_xpc() -> Vec<u8> {
    let mut bytes = xpc();
    let offset = bytes
        .windows(19)
        .position(|part| part == b"owned-private-value")
        .unwrap();
    bytes[offset] = 0xff;
    bytes
}

fn lockdown(payload: &[u8]) -> Vec<u8> {
    let mut bytes = (payload.len() as u32).to_be_bytes().to_vec();
    bytes.extend_from_slice(payload);
    bytes
}

fn assert_guard(failure: &DecodeFailure, phase: DecodePhase, kind: DecodeFailureKind) {
    assert_eq!(failure.phase(), phase);
    assert_eq!(failure.kind(), kind);
    assert!(failure.error().is_none());
    assert_eq!(failure.error_class(), None);
}

#[test]
fn dproxy_observations_plist_source_fallback_and_debug() {
    let payload = b"bplist00owned-private-invalid-plist";
    assert!(plist::from_bytes::<plist::Value>(payload).is_err());
    let mut bytes = lockdown(payload);
    let mut valid = Vec::new();
    plist::to_writer_binary(
        &mut valid,
        &plist::Value::String("owned-private-success".into()),
    )
    .unwrap();
    bytes.extend_from_slice(&lockdown(&valid));
    let outcome = StreamDecoder::new(ProxyProtocol::Lockdown)
        .push_preserving_errors(Direction::HostToDevice, &bytes);
    assert_eq!(outcome.events().len(), 2);
    assert_eq!(outcome.events()[0].decoded["raw"], hex::encode(payload));
    assert_eq!(outcome.failures().len(), 1);
    let failure = &outcome.failures()[0];
    assert!(failure.error().unwrap().is::<plist::Error>());
    assert_eq!(failure.error_class(), Some(ErrorClass::Protocol));
    assert_eq!(failure.phase(), DecodePhase::Lockdown);
    assert_eq!(failure.kind(), DecodeFailureKind::Error);
    assert_eq!(failure.event_index(), Some(0));
    assert_eq!(failure.stream_id(), None);
    assert!(matches!(failure.direction(), Direction::HostToDevice));
    for debug in [format!("{outcome:?}"), format!("{failure:?}")] {
        assert!(!debug.contains("owned-private"));
        assert!(!debug.contains(&hex::encode(payload)));
    }
    let (_, mut failures, fatal) = outcome.into_parts();
    assert!(fatal.is_none());
    assert!(failures
        .remove(0)
        .into_error()
        .unwrap()
        .is::<plist::Error>());
}

#[test]
fn dproxy_observations_dtx_source_and_later_ignored_input() {
    let mut decoder = StreamDecoder::new(ProxyProtocol::Dtx);
    let outcome = decoder.push_preserving_errors(Direction::DeviceToHost, &[0; 32]);
    assert_eq!(outcome.events().len(), 1);
    assert!(!outcome.input_ignored());
    assert_eq!(outcome.failures().len(), 1);
    let failure = &outcome.failures()[0];
    assert!(matches!(
        failure
            .error()
            .unwrap()
            .downcast_ref::<crate::services::dtx::DtxError>(),
        Some(crate::services::dtx::DtxError::BadMagic(0))
    ));
    assert_eq!(failure.error_class(), Some(ErrorClass::Protocol));
    assert_eq!(failure.event_index(), Some(0));
    assert_eq!(failure.phase(), DecodePhase::Dtx);
    let ignored = decoder.push_preserving_errors(Direction::DeviceToHost, &[1; 8]);
    assert!(ignored.input_ignored());
    assert!(ignored.events().is_empty() && ignored.failures().is_empty());
    assert!(ignored.fatal_error().is_none());
    assert!(decoder.buffer.is_empty() && decoder.dtx_broken);
}

#[test]
fn dproxy_observations_xpc_utf8_source_and_next_frame_recovery() {
    let mut bytes = h2(3, 0, &bad_xpc());
    bytes.extend_from_slice(&h2(3, 0, &xpc()));
    let mut decoder = StreamDecoder::new(ProxyProtocol::Xpc);
    let outcome = decoder.push_preserving_errors(Direction::DeviceToHost, &bytes);
    assert_eq!(outcome.events().len(), 2);
    assert_eq!(outcome.events()[1].decoded, "owned-private-value");
    assert_eq!(outcome.failures().len(), 1);
    let failure = &outcome.failures()[0];
    let native = failure.error().unwrap();
    assert!(native.is::<crate::xpc::XpcError>());
    assert_eq!(failure.error_class(), Some(ErrorClass::Protocol));
    let mut current: Option<&(dyn std::error::Error + 'static)> = Some(native);
    let mut found = false;
    for _ in 0..32 {
        let Some(error) = current else { break };
        found |= error.is::<std::str::Utf8Error>();
        current = error.source();
    }
    assert!(found);
    assert_eq!(failure.stream_id(), Some(3));
    assert_eq!(failure.phase(), DecodePhase::XpcMessage);
    assert_eq!(failure.event_index(), Some(0));
    assert!(decoder.xpc_streams.is_empty());
    assert_eq!(decoder.xpc_buffered_bytes, 0);
}

#[test]
fn dproxy_observations_declared_length_keeps_original_io_error() {
    let mut payload = vec![0; 24];
    payload[..4].copy_from_slice(&crate::xpc::message::WRAPPER_MAGIC.to_le_bytes());
    payload[8..16].copy_from_slice(&u64::MAX.to_le_bytes());
    let mut decoder = StreamDecoder::new(ProxyProtocol::Xpc);
    let outcome = decoder.push_preserving_errors(Direction::DeviceToHost, &h2(3, 0, &payload));
    assert_eq!(outcome.events().len(), 1);
    assert!(outcome.events()[0].summary.contains(&u64::MAX.to_string()));
    assert_eq!(outcome.failures().len(), 1);
    let native = outcome.failures()[0]
        .error()
        .unwrap()
        .downcast_ref::<std::io::Error>()
        .unwrap();
    assert_eq!(native.kind(), std::io::ErrorKind::InvalidData);
    assert_eq!(
        outcome.failures()[0].error_class(),
        Some(ErrorClass::Protocol)
    );
    assert!(decoder.xpc_streams.is_empty());
    assert_eq!(decoder.xpc_buffered_bytes, 0);
}

#[test]
fn dproxy_observations_native_failure_survives_later_fatal_guard() {
    let mut bytes = h2(3, 0, &bad_xpc());
    bytes.extend_from_slice(&oversized_h2());
    let outcome = StreamDecoder::new(ProxyProtocol::Xpc)
        .push_preserving_errors(Direction::DeviceToHost, &bytes);
    assert_eq!(outcome.events().len(), 1);
    assert_eq!(outcome.failures().len(), 2);
    assert!(outcome.failures()[0]
        .error()
        .unwrap()
        .is::<crate::xpc::XpcError>());
    assert_guard(
        &outcome.failures()[1],
        DecodePhase::H2Frame,
        DecodeFailureKind::ResourceLimit,
    );
    assert_eq!(outcome.failures()[1].event_index(), None);
    assert_eq!(outcome.fatal_error_class(), Some(ErrorClass::Protocol));
    let (events, failures, fatal) = outcome.into_parts();
    assert_eq!((events.len(), failures.len()), (1, 2));
    assert!(matches!(fatal, Some(DproxyError::Protocol(_))));
    let legacy_result = StreamDecoder::new(ProxyProtocol::Xpc)
        .push_preserving_errors(Direction::DeviceToHost, &bytes)
        .into_result();
    assert!(matches!(legacy_result, Err(DproxyError::Protocol(_))));
}

#[test]
fn dproxy_observations_success_survives_later_fatal_guard() {
    let mut bytes = h2(3, 0, &xpc());
    bytes.extend_from_slice(&oversized_h2());
    let outcome = StreamDecoder::new(ProxyProtocol::Xpc)
        .push_preserving_errors(Direction::HostToDevice, &bytes);
    assert_eq!(outcome.events().len(), 1);
    assert_eq!(outcome.events()[0].decoded, "owned-private-value");
    assert_eq!(outcome.failures().len(), 1);
    assert_guard(
        &outcome.failures()[0],
        DecodePhase::H2Frame,
        DecodeFailureKind::ResourceLimit,
    );
    assert!(outcome.fatal_error().is_some());
}

#[test]
fn dproxy_observations_lockdown_limit_has_no_fabricated_source() {
    let mut decoder = StreamDecoder::new(ProxyProtocol::Lockdown);
    let outcome = decoder.push_preserving_errors(Direction::HostToDevice, &u32::MAX.to_be_bytes());
    assert_eq!(outcome.events().len(), 1);
    assert_eq!(outcome.failures().len(), 1);
    assert_guard(
        &outcome.failures()[0],
        DecodePhase::Lockdown,
        DecodeFailureKind::ResourceLimit,
    );
    assert_eq!(outcome.failures()[0].event_index(), Some(0));
    assert!(outcome.fatal_error().is_none());
    assert!(decoder.buffer.is_empty());
}

#[test]
fn dproxy_observations_incomplete_end_stream_is_a_typed_decision() {
    let mut decoder = StreamDecoder::new(ProxyProtocol::Xpc);
    let outcome =
        decoder.push_preserving_errors(Direction::DeviceToHost, &h2(3, FLAG_END_STREAM, &[1; 12]));
    assert_eq!(outcome.events().len(), 1);
    assert_eq!(outcome.failures().len(), 1);
    assert_guard(
        &outcome.failures()[0],
        DecodePhase::XpcEndStream,
        DecodeFailureKind::IncompleteMessage,
    );
    assert_eq!(outcome.failures()[0].stream_id(), Some(3));
    assert_eq!(outcome.failures()[0].event_index(), Some(0));
    assert!(outcome.fatal_error().is_none());
    assert!(decoder.xpc_streams.is_empty());
    assert_eq!(decoder.xpc_buffered_bytes, 0);
}

#[test]
fn dproxy_observations_unknown_stream_flood_keeps_existing_bound() {
    let mut bytes = Vec::new();
    let limit = crate::xpc::h2_raw::MAX_UNKNOWN_BUFFERED_STREAMS;
    for stream in 100..100 + limit as u32 + 1 {
        bytes.extend_from_slice(&h2(stream, 0, &[1]));
    }
    let mut decoder = StreamDecoder::new(ProxyProtocol::Xpc);
    let outcome = decoder.push_preserving_errors(Direction::DeviceToHost, &bytes);
    assert_eq!(outcome.failures().len(), 1);
    assert_guard(
        &outcome.failures()[0],
        DecodePhase::XpcStream,
        DecodeFailureKind::ResourceLimit,
    );
    assert_eq!(outcome.failures()[0].stream_id(), Some(100 + limit as u32));
    assert!(matches!(
        outcome.fatal_error(),
        Some(DproxyError::Protocol(_))
    ));
    assert_eq!(
        (decoder.xpc_streams.len(), decoder.xpc_buffered_bytes),
        (limit, limit)
    );
}

#[test]
fn dproxy_observations_empty_data_keeps_existing_fatal_policy() {
    let outcome = StreamDecoder::new(ProxyProtocol::Xpc)
        .push_preserving_errors(Direction::DeviceToHost, &h2(3, 0, &[]));
    assert!(outcome.events().is_empty());
    assert_eq!(outcome.failures().len(), 1);
    assert_guard(
        &outcome.failures()[0],
        DecodePhase::XpcStream,
        DecodeFailureKind::InvalidState,
    );
    assert_eq!(outcome.failures()[0].stream_id(), Some(3));
    assert!(outcome
        .fatal_error()
        .unwrap()
        .to_string()
        .contains("buffer disappeared"));
}

#[test]
fn dproxy_observations_admission_limit_does_not_append_input() {
    let mut decoder = StreamDecoder::new(ProxyProtocol::Binary);
    let bytes = vec![0; MAX_DECODER_BUFFER + 1];
    let outcome = decoder.push_preserving_errors(Direction::HostToDevice, &bytes);
    assert_eq!(outcome.failures().len(), 1);
    assert_guard(
        &outcome.failures()[0],
        DecodePhase::Buffer,
        DecodeFailureKind::ResourceLimit,
    );
    assert!(outcome.fatal_error().is_some());
    assert!(outcome.events().is_empty() && decoder.buffer.is_empty());
}

#[test]
fn dproxy_observations_partial_and_binary_input_are_not_ignored() {
    for protocol in [
        ProxyProtocol::Lockdown,
        ProxyProtocol::Dtx,
        ProxyProtocol::Xpc,
        ProxyProtocol::Binary,
    ] {
        let mut decoder = StreamDecoder::new(protocol);
        let outcome = decoder.push_preserving_errors(Direction::HostToDevice, &[1]);
        assert!(!outcome.input_ignored());
        assert!(outcome.events().is_empty() && outcome.failures().is_empty());
        assert!(outcome.fatal_error().is_none());
        assert_eq!(decoder.buffer.len(), 1);
    }
}

#[test]
fn dproxy_observations_legacy_result_and_state_match_across_chunks() {
    let cases = [
        (
            ProxyProtocol::Lockdown,
            lockdown(b"bplist00owned-private-invalid-plist"),
        ),
        (ProxyProtocol::Lockdown, u32::MAX.to_be_bytes().to_vec()),
        (ProxyProtocol::Dtx, vec![0; 40]),
        (
            ProxyProtocol::Xpc,
            [
                crate::xpc::h2_raw::H2_PREFACE,
                &h2(3, 0, &xpc()),
                &h2(3, 0, &bad_xpc()),
                &oversized_h2(),
            ]
            .concat(),
        ),
        (ProxyProtocol::Xpc, h2(3, FLAG_END_STREAM, &[1; 12])),
        (ProxyProtocol::Binary, vec![1; 40]),
    ];
    for (protocol, bytes) in cases {
        for size in [1, 7, bytes.len()] {
            let mut legacy = StreamDecoder::new(protocol);
            let mut observed = StreamDecoder::new(protocol);
            for chunk in bytes.chunks(size) {
                let old = legacy.push(Direction::DeviceToHost, chunk);
                let new = observed
                    .push_preserving_errors(Direction::DeviceToHost, chunk)
                    .into_result();
                match (old, new) {
                    (Ok(old), Ok(new)) => {
                        let normalize = |events: Vec<ProxyEvent>| {
                            events
                                .into_iter()
                                .map(|event| {
                                    (
                                        event.direction,
                                        event.protocol,
                                        event.summary,
                                        event.decoded,
                                    )
                                })
                                .collect::<Vec<_>>()
                        };
                        assert_eq!(normalize(old), normalize(new));
                    }
                    (Err(old), Err(new)) => assert_eq!(old.to_string(), new.to_string()),
                    results => panic!("different legacy results: {results:?}"),
                }
                assert_eq!(legacy.buffer, observed.buffer);
                assert_eq!(legacy.xpc_streams, observed.xpc_streams);
                assert_eq!(legacy.xpc_buffered_bytes, observed.xpc_buffered_bytes);
                assert_eq!(legacy.xpc_preface_handled, observed.xpc_preface_handled);
                assert_eq!(legacy.dtx_broken, observed.dtx_broken);
            }
        }
    }
}
