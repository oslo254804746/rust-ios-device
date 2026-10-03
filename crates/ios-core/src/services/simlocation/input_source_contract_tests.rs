use super::*;
use crate::{test_native::has_source, ErrorClass};

fn route(previous: &str, current: &str) -> String {
    format!("<gpx><trk><trkseg><trkpt lat=\"owned-private\" lon=\"1\"><time>{previous}</time></trkpt><trkpt lat=\"2\" lon=\"3\"><time>{current}</time></trkpt></trkseg></trk></gpx>")
}

#[test]
fn input_native_gpx_xml_keeps_deserializer_error_and_safe_debug() {
    let error =
        parse_gpx_route("<gpx><trk><trkseg><trkpt lat=\"owned-private\"/></trkseg></trk></gpx>")
            .unwrap_err();
    assert!(has_source::<quick_xml::DeError>(&error));
    assert_eq!(crate::classify_error(&error), ErrorClass::InvalidInput);
    assert!(matches!(error, SimLocationError::Io(_)));
    assert!(!format!("{error:?}").contains("owned-private"));
}

#[test]
fn input_native_gpx_both_timestamp_positions_keep_time_parser() {
    let errors = [
        ("owned-private-invalid", "2026-10-03T00:00:00Z"),
        ("2026-10-03T00:00:00Z", "owned-private-invalid"),
    ]
    .map(|(previous, current)| parse_gpx_route(&route(previous, current)).unwrap_err());
    for error in errors {
        assert!(has_source::<time::error::Parse>(&error));
        assert_eq!(crate::classify_error(&error), ErrorClass::InvalidInput);
        assert!(matches!(error, SimLocationError::Io(_)));
        assert!(!format!("{error:?}").contains("owned-private"));
    }
    let valid = parse_gpx_route(&route("2026-10-03T00:00:03Z", "2026-10-03T00:00:00Z")).unwrap();
    assert_eq!(valid[1].delay_from_previous, Duration::ZERO);
}

#[tokio::test]
async fn input_native_gpx_replay_preserves_cause_before_any_stream_write() {
    let (mut stream, mut peer) = tokio::io::duplex(64);
    let error = replay_gpx_route(
        &mut stream,
        &route("2026-10-03T00:00:00Z", "owned-private-invalid"),
    )
    .await
    .unwrap_err();
    assert!(has_source::<time::error::Parse>(&error));
    assert_eq!(crate::classify_error(&error), ErrorClass::InvalidInput);
    drop(stream);
    let mut wire = Vec::new();
    tokio::time::timeout(
        Duration::from_secs(1),
        tokio::io::AsyncReadExt::read_to_end(&mut peer, &mut wire),
    )
    .await
    .unwrap()
    .unwrap();
    assert!(wire.is_empty());
}
