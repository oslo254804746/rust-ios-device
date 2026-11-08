//! Real supplied-stream discovery ownership; no dialing/device/browser claim.
use super::*;
use std::future::Future;
use tokio::io::{AsyncReadExt, AsyncWriteExt, DuplexStream};

const BOUND: Duration = Duration::from_secs(3);
const IDLE: Duration = Duration::from_millis(20);
const BUNDLE: &str = "owned.workflow.bundle";

async fn bounded<T>(operation: impl Future<Output = T>) -> T {
    tokio::time::timeout(BOUND, operation)
        .await
        .expect("owned discovery fixture stalled")
}

fn client(stream: DuplexStream) -> WebInspectorClient<ServiceStream> {
    let stream: ServiceStream = Box::new(stream);
    let mut client = WebInspectorClient::with_connection_id(stream, "owned-private");
    client.set_operation_timeout(Some(IDLE));
    client
}

fn message(selector: &str, argument: plist::Dictionary) -> plist::Value {
    plist::Value::Dictionary(plist::Dictionary::from_iter([
        (
            "__selector".to_owned(),
            plist::Value::String(selector.into()),
        ),
        ("__argument".to_owned(), plist::Value::Dictionary(argument)),
    ]))
}

fn application() -> plist::Dictionary {
    plist::Dictionary::from_iter([
        (
            "WIRApplicationIdentifierKey".to_owned(),
            plist::Value::String("PID:42".into()),
        ),
        (
            "WIRApplicationBundleIdentifierKey".to_owned(),
            plist::Value::String(BUNDLE.into()),
        ),
        (
            "WIRApplicationNameKey".to_owned(),
            plist::Value::String("owned-name".into()),
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

fn listing() -> plist::Value {
    message(
        "_rpc_applicationSentListing:",
        plist::Dictionary::from_iter([
            (
                "WIRApplicationIdentifierKey".to_owned(),
                plist::Value::String("PID:42".into()),
            ),
            (
                "WIRListingKey".to_owned(),
                plist::Value::Dictionary(plist::Dictionary::from_iter([(
                    "selected-key".to_owned(),
                    plist::Value::Dictionary(plist::Dictionary::from_iter([
                        (
                            "WIRPageIdentifierKey".to_owned(),
                            plist::Value::Integer(7u64.into()),
                        ),
                        (
                            "WIRTypeKey".to_owned(),
                            plist::Value::String("WIRTypeWebPage".into()),
                        ),
                        (
                            "WIRTitleKey".to_owned(),
                            plist::Value::String("owned-title".into()),
                        ),
                    ])),
                )])),
            ),
        ]),
    )
}

fn encoded(value: &plist::Value) -> Vec<u8> {
    let mut body = Vec::new();
    plist::to_writer_xml(&mut body, value).unwrap();
    body
}

async fn send(peer: &mut DuplexStream, value: plist::Value) {
    let body = encoded(&value);
    bounded(peer.write_u32(body.len() as u32)).await.unwrap();
    bounded(peer.write_all(&body)).await.unwrap();
}

async fn request(peer: &mut DuplexStream) {
    let size = bounded(peer.read_u32()).await.unwrap() as usize;
    assert!(size < 4096);
    let mut body = vec![0; size];
    bounded(peer.read_exact(&mut body)).await.unwrap();
    let sent: plist::Dictionary = plist::from_bytes(&body).unwrap();
    assert_eq!(
        sent["__selector"].as_string(),
        Some("_rpc_getConnectedApplications:")
    );
}

async fn discover(client: &mut WebInspectorClient<ServiceStream>, page: bool) -> Result<()> {
    let deadline = Instant::now() + BOUND;
    if page {
        // Preserve the explicit-key precedence over an unrelated bundle filter.
        let (id, found) = wait_for_inspectable_page(
            client,
            Some("selected-key"),
            Some("other.bundle"),
            deadline,
            BOUND,
        )
        .await?;
        assert_eq!(id, "PID:42");
        assert_eq!(found.id, 7);
        assert_eq!(found.listing_key, "selected-key");
        assert_eq!(found.title.as_deref(), Some("owned-title"));
    } else {
        let found = wait_for_application(client, BUNDLE, deadline, BOUND).await?;
        assert_eq!(found.id, "PID:42");
        assert_eq!(found.name, "owned-name");
    }
    Ok(())
}

fn native_expiry(error: &anyhow::Error) {
    assert_eq!(
        ios_core::classify_error(error.as_ref()),
        ios_core::ErrorClass::Timeout
    );
    assert!(error
        .chain()
        .any(|source| source.is::<tokio::time::error::Elapsed>()));
    let issued = error.downcast_ref::<WebInspectorError>().unwrap();
    assert_eq!(issued.timeout_duration(), Some(IDLE));
    assert!(!error.to_string().contains("unusable"));
}

async fn invalid(client: &mut WebInspectorClient<ServiceStream>) {
    assert!(!client.is_usable());
    assert!(client.applications().is_empty());
    assert!(client.open_pages_snapshot().is_empty());
    assert!(matches!(
        client.next_event().await,
        Err(WebInspectorError::Protocol(_))
    ));
    assert!(matches!(
        client.request_connected_applications().await,
        Err(WebInspectorError::Protocol(_))
    ));
}

async fn partial_reply(page: bool, header: bool) {
    let (stream, mut peer) = tokio::io::duplex(4096);
    let mut client = client(stream);
    let body = encoded(&message("_rpc_applicationUpdated:", application()));
    let length = (body.len() as u32).to_be_bytes();
    let server = async {
        request(&mut peer).await;
        if header {
            peer.write_all(&length[..2]).await.unwrap();
        } else {
            peer.write_all(&length).await.unwrap();
            peer.write_all(&body[..8]).await.unwrap();
        }
    };
    let (result, ()) = bounded(async { tokio::join!(discover(&mut client, page), server) }).await;
    native_expiry(&result.unwrap_err());
    assert!(!client.is_usable());
    // A complete late reply remains queued on the actual supplied transport.
    if header {
        peer.write_all(&length[2..]).await.unwrap();
        peer.write_all(&body).await.unwrap();
    } else {
        peer.write_all(&body[8..]).await.unwrap();
    }
    invalid(&mut client).await;
    drop(client);
    let mut extra = Vec::new();
    bounded(peer.read_to_end(&mut extra)).await.unwrap();
    assert!(
        extra.is_empty(),
        "discovery must not issue a retry after partial expiry"
    );
}

#[tokio::test]
async fn lifecycle_webinspector_discovery_partial_header_keeps_primary_elapsed_for_both_workflows()
{
    for page in [false, true] {
        partial_reply(page, true).await;
    }
}

#[tokio::test]
async fn lifecycle_webinspector_discovery_partial_body_keeps_primary_elapsed_for_both_workflows() {
    for page in [false, true] {
        partial_reply(page, false).await;
    }
}

#[tokio::test]
async fn lifecycle_webinspector_discovery_automatic_listing_block_keeps_primary_timeout() {
    for page in [false, true] {
        let (stream, mut peer) = tokio::io::duplex(64);
        let mut client = client(stream);
        let server = async {
            request(&mut peer).await;
            send(
                &mut peer,
                message(
                    "_rpc_reportConnectedApplicationList:",
                    plist::Dictionary::from_iter([(
                        "WIRApplicationDictionaryKey".to_owned(),
                        plist::Value::Dictionary(plist::Dictionary::from_iter([(
                            "42".to_owned(),
                            plist::Value::Dictionary(application()),
                        )])),
                    )]),
                ),
            )
            .await;
            // No listing reads: the real automatic send fills the64-byte stream.
        };
        let (result, ()) =
            bounded(async { tokio::join!(discover(&mut client, page), server) }).await;
        native_expiry(&result.unwrap_err());
        invalid(&mut client).await;
        drop(client);
        let mut outgoing = Vec::new();
        bounded(peer.read_to_end(&mut outgoing)).await.unwrap();
        assert_eq!(outgoing.len(), 64);
        let size = u32::from_be_bytes(outgoing[..4].try_into().unwrap()) as usize;
        assert!(
            size > outgoing.len() - 4,
            "automatic listing must be incomplete"
        );
    }
}

#[tokio::test]
async fn lifecycle_webinspector_discovery_empty_usable_idle_retries_then_returns_original_selection(
) {
    for page in [false, true] {
        let (stream, mut peer) = tokio::io::duplex(65536);
        let mut client = client(stream);
        let server = async {
            request(&mut peer).await;
            // Observing the second real request proves an empty idle elapsed;
            // do not replace it with a synthetic error or scheduler sleep.
            request(&mut peer).await;
            send(
                &mut peer,
                message("_rpc_applicationUpdated:", application()),
            )
            .await;
            if page {
                send(&mut peer, listing()).await;
            }
        };
        let (result, ()) =
            bounded(async { tokio::join!(discover(&mut client, page), server) }).await;
        result.unwrap();
        assert!(client.is_usable());
        // A cached match keeps the original no-additional-request policy.
        bounded(discover(&mut client, page)).await.unwrap();
        drop(client);
        let mut extra = Vec::new();
        bounded(peer.read_to_end(&mut extra)).await.unwrap();
        assert!(extra.is_empty());
    }
}

#[tokio::test]
async fn lifecycle_webinspector_discovery_partial_eof_preserves_native_io() {
    for page in [false, true] {
        let (stream, mut peer) = tokio::io::duplex(4096);
        let mut client = client(stream);
        let server = async move {
            request(&mut peer).await;
            peer.write_all(&[0, 0]).await.unwrap();
            drop(peer);
        };
        let (result, ()) =
            bounded(async { tokio::join!(discover(&mut client, page), server) }).await;
        let error = result.unwrap_err();
        let native = error
            .chain()
            .find_map(|source| source.downcast_ref::<std::io::Error>())
            .unwrap();
        assert_eq!(native.kind(), std::io::ErrorKind::UnexpectedEof);
        assert!(!error
            .chain()
            .any(|source| source.is::<tokio::time::error::Elapsed>()));
        invalid(&mut client).await;
    }
}

#[tokio::test]
async fn lifecycle_webinspector_discovery_malformed_plist_keeps_original_source() {
    for page in [false, true] {
        let (stream, mut peer) = tokio::io::duplex(4096);
        let mut client = client(stream);
        let server = async {
            request(&mut peer).await;
            peer.write_u32(1).await.unwrap();
            peer.write_all(b"!").await.unwrap();
        };
        let (result, ()) =
            bounded(async { tokio::join!(discover(&mut client, page), server) }).await;
        let error = result.unwrap_err();
        assert!(error.chain().any(|source| source.is::<plist::Error>()));
        assert!(matches!(
            error.downcast_ref::<WebInspectorError>(),
            Some(WebInspectorError::Plist(_))
        ));
        invalid(&mut client).await;
        drop(client);
        let mut extra = Vec::new();
        bounded(peer.read_to_end(&mut extra)).await.unwrap();
        assert!(extra.is_empty());
    }
}

#[tokio::test]
async fn lifecycle_webinspector_discovery_resource_rejection_is_not_idle_or_retry() {
    for page in [false, true] {
        let (stream, mut peer) = tokio::io::duplex(65536);
        let mut client = client(stream);
        let entries = (0..1025).map(|id| (id.to_string(), plist::Value::Boolean(false)));
        let incoming = message(
            "_rpc_reportConnectedApplicationList:",
            plist::Dictionary::from_iter([(
                "WIRApplicationDictionaryKey".to_owned(),
                plist::Value::Dictionary(plist::Dictionary::from_iter(entries)),
            )]),
        );
        let server = async {
            request(&mut peer).await;
            send(&mut peer, incoming).await;
        };
        let (result, ()) =
            bounded(async { tokio::join!(discover(&mut client, page), server) }).await;
        let error = result.unwrap_err();
        assert_eq!(
            ios_core::classify_error(error.as_ref()),
            ios_core::ErrorClass::ResourceLimit
        );
        assert!(matches!(
            error.downcast_ref::<WebInspectorError>(),
            Some(WebInspectorError::Io(_))
        ));
        invalid(&mut client).await;
        drop(client);
        let mut extra = Vec::new();
        bounded(peer.read_to_end(&mut extra)).await.unwrap();
        assert!(extra.is_empty());
    }
}
