use super::*;
use metadata::{
    MAX_APPLICATIONS, MAX_LISTING_ENTRIES, MAX_METADATA_BYTES, MAX_PAGES, MAX_PAGE_APPLICATIONS,
    MAX_SNAPSHOT_BYTES,
};
use serde_json::json;
use std::future::Future;
use tokio::io::{duplex, AsyncReadExt, AsyncWriteExt, DuplexStream};

const BOUND: Duration = Duration::from_secs(10);

async fn bounded<T>(operation: impl Future<Output = T>) -> T {
    timeout(BOUND, operation)
        .await
        .expect("owned metadata fixture stalled")
}

fn message(selector: &str, argument: plist::Dictionary) -> plist::Dictionary {
    plist::Dictionary::from_iter([
        (
            "__selector".to_owned(),
            plist::Value::String(selector.into()),
        ),
        ("__argument".to_owned(), plist::Value::Dictionary(argument)),
    ])
}

fn application(id: u64, name: &str) -> plist::Dictionary {
    plist::Dictionary::from_iter([
        (
            "WIRApplicationIdentifierKey".to_owned(),
            plist::Value::String(format!("PID:{id}")),
        ),
        (
            "WIRApplicationBundleIdentifierKey".to_owned(),
            plist::Value::String(format!("owned-bundle-{id}")),
        ),
        (
            "WIRApplicationNameKey".to_owned(),
            plist::Value::String(name.into()),
        ),
        (
            "WIRAutomationAvailabilityKey".to_owned(),
            plist::Value::String("WIRAutomationAvailabilityAvailable".into()),
        ),
        (
            "WIRIsApplicationActiveKey".to_owned(),
            plist::Value::Boolean(true),
        ),
        (
            "WIRIsApplicationProxyKey".to_owned(),
            plist::Value::Boolean(false),
        ),
        (
            "WIRIsApplicationReadyKey".to_owned(),
            plist::Value::Boolean(true),
        ),
    ])
}

fn update(id: u64, name: &str, connected: bool) -> plist::Dictionary {
    message(
        if connected {
            "_rpc_applicationConnected:"
        } else {
            "_rpc_applicationUpdated:"
        },
        application(id, name),
    )
}

fn applications(entries: impl IntoIterator<Item = (String, plist::Value)>) -> plist::Dictionary {
    message(
        "_rpc_reportConnectedApplicationList:",
        plist::Dictionary::from_iter([(
            "WIRApplicationDictionaryKey".to_owned(),
            plist::Value::Dictionary(plist::Dictionary::from_iter(entries)),
        )]),
    )
}

fn page(id: u64, title: &str) -> plist::Value {
    plist::Value::Dictionary(plist::Dictionary::from_iter([
        (
            "WIRPageIdentifierKey".to_owned(),
            plist::Value::Integer(id.into()),
        ),
        (
            "WIRTypeKey".to_owned(),
            plist::Value::String("WIRTypeWebPage".into()),
        ),
        ("WIRTitleKey".to_owned(), plist::Value::String(title.into())),
    ]))
}

fn listing(
    application_id: &str,
    entries: impl IntoIterator<Item = (String, plist::Value)>,
) -> plist::Dictionary {
    message(
        "_rpc_applicationSentListing:",
        plist::Dictionary::from_iter([
            (
                "WIRApplicationIdentifierKey".to_owned(),
                plist::Value::String(application_id.into()),
            ),
            (
                "WIRListingKey".to_owned(),
                plist::Value::Dictionary(plist::Dictionary::from_iter(entries)),
            ),
        ]),
    )
}

fn disconnect(id: &str) -> plist::Dictionary {
    message(
        "_rpc_applicationDisconnected:",
        plist::Dictionary::from_iter([(
            "WIRApplicationIdentifierKey".to_owned(),
            plist::Value::String(id.into()),
        )]),
    )
}

fn availability(value: &str) -> plist::Dictionary {
    message(
        "_rpc_reportCurrentState:",
        plist::Dictionary::from_iter([(
            "WIRAutomationAvailabilityKey".to_owned(),
            plist::Value::String(value.into()),
        )]),
    )
}

async fn send(peer: &mut DuplexStream, dictionary: plist::Dictionary) {
    let mut body = Vec::new();
    plist::to_writer_xml(&mut body, &plist::Value::Dictionary(dictionary)).unwrap();
    assert!(body.len() <= 16 * 1024 * 1024);
    bounded(peer.write_u32(body.len() as u32)).await.unwrap();
    bounded(peer.write_all(&body)).await.unwrap();
}

async fn send_binary(peer: &mut DuplexStream, dictionary: plist::Dictionary) {
    let mut body = Vec::new();
    plist::to_writer_binary(&mut body, &plist::Value::Dictionary(dictionary)).unwrap();
    assert_eq!(&body[..8], b"bplist00");
    assert!(
        body.len() < MAX_METADATA_BYTES,
        "fixture must fit the real wire ceiling"
    );
    bounded(peer.write_u32(body.len() as u32)).await.unwrap();
    bounded(peer.write_all(&body)).await.unwrap();
}

#[tokio::test]
async fn lifecycle_webinspector_cache_actual_binary_shared_strings_bound_staged_duplicate_listing()
{
    let (stream, mut peer) = duplex(1024 * 1024);
    let mut client = WebInspectorClient::with_connection_id(stream, "owned");
    let large = "a".repeat(MAX_METADATA_BYTES / 2);
    let caller = async {
        let error = client.next_event().await.unwrap_err();
        limit(&error);
        assert!(error.to_string().contains("staged listing bytes"));
    };
    let server = async {
        send_binary(
            &mut peer,
            listing(
                "orphan",
                [
                    ("first".into(), page(1, &large)),
                    ("second".into(), page(1, &large)),
                ],
            ),
        )
        .await;
    };
    bounded(async {
        tokio::join!(caller, server);
    })
    .await;
    invalid(&mut client).await;
}

#[tokio::test]
async fn lifecycle_webinspector_cache_actual_binary_shared_strings_bound_staged_application_list() {
    let (stream, mut peer) = duplex(1024 * 1024);
    let mut client = WebInspectorClient::with_connection_id(stream, "owned");
    let large = "a".repeat(MAX_METADATA_BYTES / 2);
    let caller = async {
        let error = client.next_event().await.unwrap_err();
        limit(&error);
        assert!(error.to_string().contains("staged metadata bytes"));
    };
    let server = async {
        send_binary(
            &mut peer,
            applications([
                (
                    "42".into(),
                    plist::Value::Dictionary(application(42, &large)),
                ),
                (
                    "99".into(),
                    plist::Value::Dictionary(application(99, &large)),
                ),
            ]),
        )
        .await;
    };
    bounded(async {
        tokio::join!(caller, server);
    })
    .await;
    invalid(&mut client).await;
    drop(client);
    let mut extra = Vec::new();
    bounded(peer.read_to_end(&mut extra)).await.unwrap();
    assert!(
        extra.is_empty(),
        "no automatic listing before staged admission"
    );
}

#[tokio::test]
async fn lifecycle_webinspector_cache_actual_aggregate_page_frames_reject_growth_and_drop_metadata()
{
    let (stream, mut peer) = duplex(1024 * 1024);
    let mut client = WebInspectorClient::with_connection_id(stream, "owned");
    let caller = async {
        client.next_event().await.unwrap();
        assert_eq!(client.application_pages["orphan-a"].len(), MAX_PAGES);
        let error = client.next_event().await.unwrap_err();
        limit(&error);
    };
    let server = async {
        send(
            &mut peer,
            listing(
                "orphan-a",
                (0..MAX_PAGES).map(|id| (id.to_string(), page(id as u64, "small"))),
            ),
        )
        .await;
        send(
            &mut peer,
            listing("orphan-b", [("1".into(), page(1, "small"))]),
        )
        .await;
    };
    bounded(async {
        tokio::join!(caller, server);
    })
    .await;
    invalid(&mut client).await;
}

fn limit(error: &WebInspectorError) {
    assert!(matches!(error, WebInspectorError::Io(io) if io.kind() == std::io::ErrorKind::Other));
    assert_eq!(
        crate::classify_error(error),
        crate::ErrorClass::ResourceLimit
    );
    assert!(crate::test_native::has_source::<
        super::super::WebInspectorLimit,
    >(error));
    assert_eq!(error.timeout_duration(), None);
    assert!(!format!("{error:?}").contains("owned"));
}

fn cache_json<S>(client: &WebInspectorClient<S>) -> JsonValue {
    // Independent normal JSON materialization is used only by the fixture;
    // production admits borrowed projections without allocating this object.
    let apps: serde_json::Map<String, JsonValue> = client
        .applications
        .iter()
        .map(|(key, value)| (key.clone(), serde_json::to_value(value).unwrap()))
        .collect();
    let pages: serde_json::Map<String, JsonValue> = client
        .application_pages
        .iter()
        .map(|(key, pages)| {
            let pages: serde_json::Map<String, JsonValue> = pages
                .iter()
                .map(|(key, page)| (key.to_string(), serde_json::to_value(page).unwrap()))
                .collect();
            (key.clone(), JsonValue::Object(pages))
        })
        .collect();
    json!({"automation_availability": client.automation_availability, "applications": apps, "application_pages": pages})
}

async fn invalid(client: &mut WebInspectorClient<DuplexStream>) {
    assert!(!client.is_usable());
    assert!(client.automation_availability.is_none());
    assert_eq!(client.applications.capacity(), 0);
    assert_eq!(client.application_pages.capacity(), 0);
    assert_eq!(client.pending_events.capacity(), 0);
    assert!(client.open_pages_snapshot().is_empty());
    assert!(matches!(
        client.next_event().await,
        Err(WebInspectorError::Protocol(_))
    ));
    assert!(matches!(
        client.report_identifier().await,
        Err(WebInspectorError::Protocol(_))
    ));
}

#[tokio::test]
async fn lifecycle_webinspector_cache_raw_application_and_listing_counts_reject_before_parse_or_mutation(
) {
    for apps in [false, true] {
        let mut client = WebInspectorClient::with_connection_id(tokio::io::empty(), "owned");
        client
            .handle_message(update(42, "old", false))
            .await
            .unwrap();
        let before = cache_json(&client);
        let count = if apps {
            MAX_APPLICATIONS
        } else {
            MAX_LISTING_ENTRIES
        };
        // Entries are deliberately not dictionaries. Count rejection precedes
        // model parsing and any cache mutation or outgoing listing send.
        let entries = (0..=count).map(|id| (id.to_string(), plist::Value::Boolean(false)));
        let dictionary = if apps {
            applications(entries)
        } else {
            listing("PID:42", entries)
        };
        let error = client.handle_message(dictionary).await.unwrap_err();
        limit(&error);
        assert_eq!(cache_json(&client), before);
        assert!(
            client.is_usable(),
            "private handler bypasses frame owner for mutation audit"
        );
    }
}

#[tokio::test]
async fn lifecycle_webinspector_cache_actual_repeated_application_frames_bound_growth_and_invalidate(
) {
    let (stream, mut peer) = duplex(1024 * 1024);
    let mut client = WebInspectorClient::with_connection_id(stream, "owned");
    let caller = async {
        for id in 0..MAX_APPLICATIONS {
            let event = client.next_event().await.unwrap();
            assert!(matches!(
                event,
                WebInspectorEvent::ApplicationConnected { .. }
                    | WebInspectorEvent::ApplicationUpdated { .. }
            ));
            assert_eq!(client.applications.len(), id + 1);
        }
        let error = client.next_event().await.unwrap_err();
        limit(&error);
    };
    let server = async {
        for id in 0..=MAX_APPLICATIONS {
            send(&mut peer, update(id as u64, "owned-name", id % 2 == 0)).await;
        }
    };
    bounded(async {
        tokio::join!(caller, server);
    })
    .await;
    invalid(&mut client).await;
    drop(client);
    let mut extra = Vec::new();
    bounded(peer.read_to_end(&mut extra)).await.unwrap();
    assert!(extra.is_empty());
}

#[tokio::test]
async fn lifecycle_webinspector_cache_orphan_page_application_count_includes_empty_sets_and_disconnect_release(
) {
    let mut client = WebInspectorClient::with_connection_id(tokio::io::empty(), "owned");
    for id in 0..MAX_PAGE_APPLICATIONS {
        client
            .handle_message(listing(&format!("orphan-{id}"), []))
            .await
            .unwrap();
    }
    assert_eq!(client.application_pages.len(), MAX_PAGE_APPLICATIONS);
    assert!(client.open_pages_snapshot().is_empty());
    let error = client
        .handle_message(listing("over-limit", []))
        .await
        .unwrap_err();
    limit(&error);
    assert!(!client.application_pages.contains_key("over-limit"));
    client.handle_message(disconnect("orphan-0")).await.unwrap();
    client
        .handle_message(listing("after-release", []))
        .await
        .unwrap();
    assert_eq!(client.application_pages.len(), MAX_PAGE_APPLICATIONS);
}

#[tokio::test]
async fn lifecycle_webinspector_cache_aggregate_page_limit_replacement_and_disconnect_restore_capacity(
) {
    let mut client = WebInspectorClient::with_connection_id(tokio::io::empty(), "owned");
    client
        .handle_message(listing(
            "orphan-a",
            (0..MAX_PAGES).map(|id| (id.to_string(), page(id as u64, "owned"))),
        ))
        .await
        .unwrap();
    assert_eq!(client.application_pages["orphan-a"].len(), MAX_PAGES);
    let error = client
        .handle_message(listing("orphan-b", [("1".into(), page(1, "over"))]))
        .await
        .unwrap_err();
    limit(&error);
    assert!(!client.application_pages.contains_key("orphan-b"));
    client
        .handle_message(listing("orphan-a", []))
        .await
        .unwrap();
    assert_eq!(client.application_pages["orphan-a"].capacity(), 0);
    client
        .handle_message(listing("orphan-b", [("1".into(), page(1, "after"))]))
        .await
        .unwrap();
    assert_eq!(client.application_pages["orphan-b"].len(), 1);
    client.handle_message(disconnect("orphan-a")).await.unwrap();
    client.handle_message(disconnect("orphan-b")).await.unwrap();
    assert_eq!(client.application_pages.capacity(), 0);
}

#[tokio::test]
async fn lifecycle_webinspector_cache_duplicate_page_ids_keep_original_event_order_and_last_cached_value(
) {
    let mut client = WebInspectorClient::with_connection_id(tokio::io::empty(), "owned");
    client
        .handle_message(update(42, "owned", false))
        .await
        .unwrap();
    let event = client
        .handle_message(listing(
            "PID:42",
            [
                ("first".into(), page(7, "first")),
                ("middle".into(), page(9, "middle")),
                ("last".into(), page(7, "last")),
            ],
        ))
        .await
        .unwrap();
    let WebInspectorEvent::Listing { pages, .. } = event else {
        unreachable!()
    };
    assert_eq!(
        pages
            .iter()
            .map(|page| page.listing_key.as_str())
            .collect::<Vec<_>>(),
        ["first", "middle", "last"]
    );
    assert_eq!(
        client.application_pages["PID:42"]
            .keys()
            .copied()
            .collect::<Vec<_>>(),
        [7, 9]
    );
    assert_eq!(
        client.page("PID:42", 7).unwrap().title.as_deref(),
        Some("last")
    );
    assert_eq!(
        client
            .open_pages_snapshot()
            .iter()
            .map(|value| value.page.title.as_deref().unwrap())
            .collect::<Vec<_>>(),
        ["last", "middle"]
    );
}

#[tokio::test]
async fn lifecycle_webinspector_cache_connected_snapshot_duplicate_ids_and_absent_page_capacity_release(
) {
    let (stream, mut peer) = duplex(16384);
    let mut client = WebInspectorClient::with_connection_id(stream, "owned");
    client
        .handle_message(update(42, "old", false))
        .await
        .unwrap();
    client
        .handle_message(update(99, "drop", false))
        .await
        .unwrap();
    client
        .handle_message(listing("PID:99", [("9".into(), page(9, "drop"))]))
        .await
        .unwrap();
    let event = client
        .handle_message(applications([
            (
                "first".into(),
                plist::Value::Dictionary(application(42, "first")),
            ),
            (
                "second".into(),
                plist::Value::Dictionary(application(42, "last")),
            ),
        ]))
        .await
        .unwrap();
    assert!(
        matches!(event, WebInspectorEvent::ConnectedApplications { applications } if applications.len() == 1 && applications[0].name == "last")
    );
    assert_eq!(client.applications.len(), 1);
    assert_eq!(client.applications["PID:42"].name, "last");
    assert_eq!(client.application_pages.capacity(), 0);
    let size = bounded(peer.read_u32()).await.unwrap() as usize;
    let mut body = vec![0; size];
    bounded(peer.read_exact(&mut body)).await.unwrap();
    let sent: plist::Dictionary = plist::from_bytes(&body).unwrap();
    assert_eq!(
        sent["__selector"].as_string(),
        Some("_rpc_forwardGetListing:")
    );
    assert_eq!(
        sent["__argument"].as_dictionary().unwrap()["WIRApplicationIdentifierKey"].as_string(),
        Some("PID:42")
    );
    client.handle_message(disconnect("PID:42")).await.unwrap();
    assert_eq!(client.applications.capacity(), 0);
}

#[tokio::test]
async fn lifecycle_webinspector_cache_exact_metadata_bytes_include_keys_availability_and_json_escaping(
) {
    let mut client = WebInspectorClient::with_connection_id(tokio::io::empty(), "owned");
    client
        .handle_message(availability("unknown-\"\\你好"))
        .await
        .unwrap();
    client.handle_message(update(42, "", false)).await.unwrap();
    let overhead = serde_json::to_vec(&cache_json(&client)).unwrap().len();
    client
        .handle_message(update(
            42,
            &"a".repeat(MAX_METADATA_BYTES - overhead),
            false,
        ))
        .await
        .unwrap();
    assert_eq!(
        serde_json::to_vec(&cache_json(&client)).unwrap().len(),
        MAX_METADATA_BYTES
    );
    let before = client.applications["PID:42"].name.len();
    let error = client
        .handle_message(update(42, &"a".repeat(before + 1), true))
        .await
        .unwrap_err();
    limit(&error);
    assert_eq!(client.applications["PID:42"].name.len(), before);
    // Replacement releases the previous payload in the projection; growth is
    // judged against the resulting cache, not old+new temporary copies.
    client
        .handle_message(update(42, "smaller-\"\\\n你好", true))
        .await
        .unwrap();
    client
        .handle_message(listing(
            "PID:42",
            [("key-\"\\你好".into(), page(7, "title-\"\\\n你好"))],
        ))
        .await
        .unwrap();
    assert!(serde_json::to_vec(&cache_json(&client)).unwrap().len() < 2048);
    client.handle_message(disconnect("PID:42")).await.unwrap();
    assert_eq!(client.applications.capacity(), 0);
    assert_eq!(client.application_pages.capacity(), 0);
}

#[tokio::test]
async fn lifecycle_webinspector_cache_combined_app_page_bytes_reject_without_mutating_previous_snapshot(
) {
    let mut client = WebInspectorClient::with_connection_id(tokio::io::empty(), "owned");
    let half = "a".repeat(MAX_METADATA_BYTES / 2);
    client
        .handle_message(update(42, &half, false))
        .await
        .unwrap();
    let error = client
        .handle_message(listing("orphan", [("1".into(), page(1, &half))]))
        .await
        .unwrap_err();
    limit(&error);
    assert!(!client.application_pages.contains_key("orphan"));
    assert_eq!(client.applications["PID:42"].name.len(), half.len());
    client
        .handle_message(update(42, "small", false))
        .await
        .unwrap();
    client
        .handle_message(listing("orphan", [("1".into(), page(1, &half))]))
        .await
        .unwrap();
    assert!(client.open_pages_snapshot().is_empty());
    client.handle_message(disconnect("orphan")).await.unwrap();
    client
        .handle_message(update(42, &half, false))
        .await
        .unwrap();
    assert_eq!(client.applications["PID:42"].name.len(), half.len());
}

#[tokio::test]
async fn lifecycle_webinspector_cache_actual_scalar_plus_application_bytes_share_budget_and_clear_owner(
) {
    let (stream, mut peer) = duplex(1024 * 1024);
    let mut client = WebInspectorClient::with_connection_id(stream, "owned");
    let half = "a".repeat(MAX_METADATA_BYTES / 2);
    let caller = async {
        assert!(matches!(
            client.next_event().await.unwrap(),
            WebInspectorEvent::CurrentState {
                availability: AutomationAvailability::Unknown(_)
            }
        ));
        let error = client.next_event().await.unwrap_err();
        limit(&error);
    };
    let server = async {
        send(&mut peer, availability(&half)).await;
        send(&mut peer, update(42, &half, false)).await;
    };
    bounded(async {
        tokio::join!(caller, server);
    })
    .await;
    invalid(&mut client).await;
}

#[tokio::test]
async fn lifecycle_webinspector_cache_actual_flattened_snapshot_amplification_rejects_two_small_pages(
) {
    let (stream, mut peer) = duplex(1024 * 1024);
    let mut client = WebInspectorClient::with_connection_id(stream, "owned");
    let large = "a".repeat(MAX_SNAPSHOT_BYTES / 2);
    let caller = async {
        client.next_event().await.unwrap();
        assert!(client.open_pages_snapshot().is_empty());
        let error = client.next_event().await.unwrap_err();
        limit(&error);
        assert!(error.to_string().contains("flattened snapshot bytes"));
    };
    let server = async {
        send(&mut peer, update(42, &large, false)).await;
        send(
            &mut peer,
            listing(
                "PID:42",
                [
                    ("1".into(), page(1, "small")),
                    ("2".into(), page(2, "small")),
                ],
            ),
        )
        .await;
    };
    bounded(async {
        tokio::join!(caller, server);
    })
    .await;
    invalid(&mut client).await;
}

#[tokio::test]
async fn lifecycle_webinspector_cache_exact_flattened_snapshot_budget_is_checked_before_cache_admission(
) {
    let mut client = WebInspectorClient::with_connection_id(tokio::io::empty(), "owned");
    client.handle_message(update(42, "", false)).await.unwrap();
    client
        .handle_message(listing(
            "PID:42",
            [("1".into(), page(1, "a")), ("2".into(), page(2, "b"))],
        ))
        .await
        .unwrap();
    let overhead = serde_json::to_vec(&client.open_pages_snapshot())
        .unwrap()
        .len();
    let name_len = (MAX_SNAPSHOT_BYTES - overhead) / 2;
    client
        .handle_message(update(42, &"a".repeat(name_len), false))
        .await
        .unwrap();
    let size = serde_json::to_vec(&client.open_pages_snapshot())
        .unwrap()
        .len();
    assert!(size <= MAX_SNAPSHOT_BYTES && MAX_SNAPSHOT_BYTES - size < 2);
    let error = client
        .handle_message(update(42, &"a".repeat(name_len + 1), false))
        .await
        .unwrap_err();
    limit(&error);
    assert_eq!(client.applications["PID:42"].name.len(), name_len);
}

#[tokio::test]
async fn lifecycle_webinspector_cache_staged_duplicate_listing_payload_is_bounded_even_with_one_cached_id(
) {
    let mut client = WebInspectorClient::with_connection_id(tokio::io::empty(), "owned");
    client
        .handle_message(listing("orphan", [("old".into(), page(9, "old"))]))
        .await
        .unwrap();
    let large = "a".repeat(MAX_METADATA_BYTES / 2);
    let error = client
        .handle_message(listing(
            "orphan",
            [
                ("first".into(), page(1, &large)),
                ("second".into(), page(1, &large)),
            ],
        ))
        .await
        .unwrap_err();
    limit(&error);
    assert_eq!(client.application_pages["orphan"].len(), 1);
    assert_eq!(client.application_pages["orphan"][&9].listing_key, "old");
}

#[tokio::test]
async fn lifecycle_webinspector_cache_actual_discovery_count_failure_is_not_successful_idle_snapshot(
) {
    let (stream, mut peer) = duplex(1024 * 1024);
    let mut client = WebInspectorClient::with_connection_id(stream, "owned");
    let caller = client.open_application_pages(Duration::from_secs(2));
    let server = async {
        let size = peer.read_u32().await.unwrap() as usize;
        let mut body = vec![0; size];
        peer.read_exact(&mut body).await.unwrap();
        send(
            &mut peer,
            applications(
                (0..=MAX_APPLICATIONS).map(|id| (id.to_string(), plist::Value::Boolean(false))),
            ),
        )
        .await;
    };
    let (result, ()) = bounded(async { tokio::join!(caller, server) }).await;
    limit(&result.unwrap_err());
    invalid(&mut client).await;
}
