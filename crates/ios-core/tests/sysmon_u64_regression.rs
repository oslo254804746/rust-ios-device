#![cfg(feature = "instruments")]

//! Offline regression coverage for the iOS 26 sysmontap payload shape.
//!
//! iOS 26 can encode a process attribute such as `threadsSystem` as the
//! unsigned value `u64::MAX`.  The complete payload must still be decoded and
//! delivered to the sysmontap parser; one large value must not discard the
//! whole process snapshot.

use bytes::Bytes;
use ios_core::dtx::{encode_dtx, read_dtx_frame};
use ios_core::instruments::{SysmontapConfig, SysmontapService};
use plist::{Dictionary, Integer, Value};
use tokio::io::{duplex, AsyncWriteExt};
use tokio::time::{timeout, Duration};

fn uid(index: u64) -> Value {
    Value::Uid(plist::Uid::new(index))
}

fn class_descriptor(name: &str, classes: &[&str]) -> Value {
    Value::Dictionary(Dictionary::from_iter([
        ("$classname".to_string(), Value::String(name.to_string())),
        (
            "$classes".to_string(),
            Value::Array(
                classes
                    .iter()
                    .map(|class| Value::String((*class).to_string()))
                    .collect(),
            ),
        ),
    ]))
}

/// Build a keyed archive containing the same relevant nesting as a
/// `DTSysmonTapMessage`: an array of result dictionaries, with
/// `Processes` mapping a PID to an ordered values array.
fn sysmontap_process_archive() -> Vec<u8> {
    let array_class = class_descriptor("NSArray", &["NSArray", "NSObject"]);
    let dictionary_class = class_descriptor("NSDictionary", &["NSDictionary", "NSObject"]);

    let objects = vec![
        // 0: NSKeyedArchiver's null sentinel.
        Value::String("$null".to_string()),
        // 1: root NSArray -> result dictionary (3).
        Value::Dictionary(Dictionary::from_iter([
            ("$class".to_string(), uid(2)),
            ("NS.objects".to_string(), Value::Array(vec![uid(3)])),
        ])),
        // 2: NSArray class descriptor.
        array_class,
        // 3: result dictionary: Processes -> process dictionary (6).
        Value::Dictionary(Dictionary::from_iter([
            ("$class".to_string(), uid(4)),
            ("NS.keys".to_string(), Value::Array(vec![uid(5)])),
            ("NS.objects".to_string(), Value::Array(vec![uid(6)])),
        ])),
        // 4: NSDictionary class descriptor.
        dictionary_class.clone(),
        // 5: result key.
        Value::String("Processes".to_string()),
        // 6: process dictionary: PID 77 -> values array (8).
        Value::Dictionary(Dictionary::from_iter([
            ("$class".to_string(), uid(4)),
            ("NS.keys".to_string(), Value::Array(vec![uid(7)])),
            ("NS.objects".to_string(), Value::Array(vec![uid(8)])),
        ])),
        // 7: PID key.
        Value::String("77".to_string()),
        // 8: ordered process attributes.
        Value::Dictionary(Dictionary::from_iter([
            ("$class".to_string(), uid(2)),
            (
                "NS.objects".to_string(),
                Value::Array(vec![uid(9), uid(10)]),
            ),
        ])),
        // 9: iOS 26's large unsigned NSNumber value.
        Value::Integer(Integer::from(u64::MAX)),
        // 10: ordinary small integer, retained for compatibility coverage.
        Value::Integer(Integer::from(7_u64)),
        // 11: the DTX tap wrapper used by real sysmontap DATA payloads.
        Value::Dictionary(Dictionary::from_iter([
            ("$class".to_string(), uid(12)),
            ("DTTapMessagePlist".to_string(), uid(1)),
        ])),
        // 12: DTTapMessage subclass descriptor.
        class_descriptor(
            "DTSysmonTapMessage",
            &["DTSysmonTapMessage", "DTTapMessage", "NSObject"],
        ),
    ];

    let document = Value::Dictionary(Dictionary::from_iter([
        (
            "$archiver".to_string(),
            Value::String("NSKeyedArchiver".to_string()),
        ),
        (
            "$version".to_string(),
            Value::Integer(Integer::from(100_000_u64)),
        ),
        (
            "$top".to_string(),
            Value::Dictionary(Dictionary::from_iter([("root".to_string(), uid(11))])),
        ),
        ("$objects".to_string(), Value::Array(objects)),
    ]));

    let mut encoded = Vec::new();
    plist::to_writer_binary(&mut encoded, &document).expect("encode keyed archive fixture");
    encoded
}

#[tokio::test]
async fn sysmontap_snapshot_keeps_u64_max_unsigned_attribute() {
    let archive = sysmontap_process_archive();
    let (client, mut server) = duplex(64 * 1024);

    let server_task = tokio::spawn(async move {
        // requestChannelWithCode:identifier:
        let request = read_dtx_frame(&mut server).await.expect("channel request");
        assert!(request.expects_reply);
        server
            .write_all(&encode_dtx(request.identifier, 1, 0, false, 3, &[], &[]))
            .await
            .expect("channel reply");

        // setConfig:
        let config = read_dtx_frame(&mut server)
            .await
            .expect("setConfig request");
        assert!(config.expects_reply);
        server
            .write_all(&encode_dtx(config.identifier, 1, 1, false, 3, &[], &[]))
            .await
            .expect("setConfig reply");

        // start (fire-and-forget), followed by one broadcast data frame.
        let start = read_dtx_frame(&mut server).await.expect("start request");
        assert!(!start.expects_reply);
        server
            .write_all(&encode_dtx(8, 0, -1, false, 1, &archive, &[]))
            .await
            .expect("sysmontap data");
    });

    let mut service = SysmontapService::start(client, &SysmontapConfig::default(), None, None)
        .await
        .expect("start offline sysmontap service");

    let attrs = ["threadsSystem".to_string(), "smallInteger".to_string()];
    let snapshot = timeout(
        Duration::from_secs(1),
        service.next_process_snapshot(&attrs),
    )
    .await
    .expect("sysmontap fixture should produce a snapshot")
    .expect("sysmontap fixture should decode")
    .expect("sysmontap fixture should contain process data");

    assert_eq!(snapshot.processes.len(), 1);
    assert_eq!(
        snapshot.processes[0]
            .get("threadsSystem")
            .and_then(serde_json::Value::as_u64),
        Some(u64::MAX)
    );
    assert_eq!(
        snapshot.processes[0]
            .get("smallInteger")
            .and_then(serde_json::Value::as_i64),
        Some(7)
    );

    server_task.await.expect("offline DTX server task");
}

#[test]
fn archive_fixture_has_binary_plist_signature() {
    let archive = sysmontap_process_archive();
    assert_eq!(&archive[..6], b"bplist");
    assert!(Bytes::from(archive).len() > 6);
}
