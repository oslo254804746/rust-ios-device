use super::*;
use crate::xpc::message::{borrowed, encode_message, flags, XpcMessage, XpcValue};
use indexmap::IndexMap;
use std::{
    pin::Pin,
    sync::{
        atomic::{AtomicUsize, Ordering},
        Mutex,
    },
    task::{Context, Poll},
};
use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};

const TOTAL: usize = 1024 * 1024;
fn dict(entries: impl IntoIterator<Item = (&'static str, XpcValue)>) -> XpcValue {
    XpcValue::Dictionary(entries.into_iter().map(|(k, v)| (k.into(), v)).collect())
}
fn message(services: Option<XpcValue>) -> XpcMessage {
    let mut body = IndexMap::from_iter([
        ("MessageType".into(), XpcValue::String("Handshake".into())),
        (
            "Properties".into(),
            dict([("UniqueDeviceID", XpcValue::String("original-uid".into()))]),
        ),
    ]);
    if let Some(services) = services {
        body.insert("Services".into(), services);
    }
    XpcMessage {
        flags: flags::ALWAYS_SET,
        msg_id: 0,
        body: Some(XpcValue::Dictionary(body)),
    }
}
fn sample() -> XpcMessage {
    message(Some(dict([
        (
            "svc.shim.remote",
            dict([
                ("Port", XpcValue::String("+42".into())),
                (
                    "Properties",
                    dict([(
                        "Features",
                        XpcValue::Array(vec![
                            XpcValue::String("".into()),
                            XpcValue::Bool(false),
                            XpcValue::String("repeat".into()),
                            XpcValue::String("repeat".into()),
                        ]),
                    )]),
                ),
            ]),
        ),
        ("zero", dict([("Port", XpcValue::Uint64(0))])),
        ("max", dict([("Port", XpcValue::Uint64(65535))])),
        ("bad", dict([("Port", XpcValue::Int64(-1))])),
    ])))
}
fn compare(message: XpcMessage) {
    let wire = encode_message(&message).unwrap();
    let bank = Arc::new(Semaphore::new(TOTAL));
    let tree = borrowed::decode(&wire[..24], &wire[24..], &bank).unwrap();
    let legacy = super::super::initial::parse_handshake(tree.as_ref());
    let mut owned = OwnedRsdHandshake::prepare(&bank).unwrap();
    match (legacy, owned.fill(tree.as_ref())) {
        (Ok(legacy), Ok(())) => {
            assert_eq!(owned.udid(), legacy.udid);
            assert_eq!(owned.len(), legacy.services.len());
            for service in owned.services() {
                let expected = &legacy.services[service.name()];
                assert_eq!(service.port(), expected.port);
                assert_eq!(
                    service.features().collect::<Vec<_>>(),
                    expected
                        .features
                        .iter()
                        .map(String::as_str)
                        .collect::<Vec<_>>()
                );
                for feature in ["", "repeat", "absent"] {
                    assert_eq!(
                        service.supports_feature(feature),
                        expected.supports_feature(feature)
                    );
                    assert_eq!(
                        service.feature_evidence(feature),
                        expected.feature_evidence(feature)
                    );
                    assert_eq!(
                        owned.feature_evidence(service.name(), feature),
                        legacy.feature_evidence(service.name(), feature)
                    );
                }
            }
        }
        (Err(legacy), Err(actual)) => assert_eq!(actual.to_string(), legacy.to_string()),
        (legacy, actual) => panic!("owned initial parser diverged: {legacy:?}; {actual:?}"),
    }
    drop(tree);
    assert_eq!(bank.available_permits(), TOTAL - owned.known_bytes());
    drop(owned);
    assert_eq!(bank.available_permits(), TOTAL);
}

#[test]
fn owned_initial_legacy_semantics_keep_empty_uid_ports_features_and_all_error_priorities() {
    compare(sample());
    for services in [
        None,
        Some(XpcValue::Null),
        Some(XpcValue::Array(vec![])),
        Some(dict([])),
    ] {
        compare(message(services));
    }
    for body in [
        None,
        Some(XpcValue::Null),
        Some(dict([])),
        Some(dict([("MessageType", XpcValue::Bool(false))])),
        Some(dict([("MessageType", XpcValue::String("other".into()))])),
        Some(dict([(
            "MessageType",
            XpcValue::String("Handshake".into()),
        )])),
    ] {
        compare(XpcMessage {
            flags: flags::ALWAYS_SET,
            msg_id: 0,
            body,
        });
    }
    let mut empty = message(None);
    let XpcValue::Dictionary(body) = empty.body.as_mut().unwrap() else {
        unreachable!()
    };
    body.insert(
        "Properties".into(),
        dict([("UniqueDeviceID", XpcValue::String(String::new()))]),
    );
    compare(empty);
}

#[test]
fn owned_initial_complete_output_admission_precedes_first_copy_and_last_byte_refusal_allocates_nothing(
) {
    let wire = encode_message(&sample()).unwrap();
    let probe = Arc::new(Semaphore::new(TOTAL));
    let tree = borrowed::decode(&wire[..24], &wire[24..], &probe).unwrap();
    let mut owned = OwnedRsdHandshake::prepare(&probe).unwrap();
    owned.fill(tree.as_ref()).unwrap();
    let fields = owned.known_bytes() - OwnedRsdHandshake::body_bytes();
    drop((tree, owned));
    assert_eq!(probe.available_permits(), TOTAL);
    for short in [true, false] {
        let bank = Arc::new(Semaphore::new(TOTAL));
        let tree = borrowed::decode(&wire[..24], &wire[24..], &bank).unwrap();
        let mut owned = OwnedRsdHandshake::prepare(&bank).unwrap();
        let remaining = fields - usize::from(short);
        let held = bank
            .clone()
            .try_acquire_many_owned((bank.available_permits() - remaining) as u32)
            .unwrap();
        let layouts = Arc::new(Mutex::new(Vec::new()));
        let seen = layouts.clone();
        let charged = bank.clone();
        let result = with_output_allocator(
            move |layout| {
                assert!(!short, "unadmitted output allocation");
                assert_eq!(charged.available_permits(), 0);
                seen.lock().unwrap().push(layout);
            },
            || owned.fill(tree.as_ref()),
        );
        if short {
            assert_eq!(
                crate::classify_error(&result.unwrap_err()),
                crate::ErrorClass::ResourceLimit
            );
            assert!(layouts.lock().unwrap().is_empty());
            assert!(owned.is_empty() && owned.udid().is_empty());
            assert_eq!(bank.available_permits(), remaining);
        } else {
            result.unwrap();
            assert_eq!(
                layouts
                    .lock()
                    .unwrap()
                    .iter()
                    .map(Layout::size)
                    .sum::<usize>(),
                fields
            );
            assert_eq!(bank.available_permits(), 0);
        }
        drop((owned, tree, held));
        assert_eq!(bank.available_permits(), TOTAL);
    }
}

#[test]
fn owned_initial_clone_iterator_and_service_export_keep_original_buffers_and_fees_after_input_drop()
{
    let wire = encode_message(&sample()).unwrap();
    let bank = Arc::new(Semaphore::new(TOTAL));
    let tree = borrowed::decode(&wire[..24], &wire[24..], &bank).unwrap();
    let mut owned = OwnedRsdHandshake::prepare(&bank).unwrap();
    owned.fill(tree.as_ref()).unwrap();
    drop(tree);
    drop(wire);
    let bytes = owned.known_bytes();
    let uid = owned.udid().as_ptr();
    assert!(owned.belongs_to(&bank));
    assert_eq!(owned.get_port("svc"), Some(42));
    assert_eq!(owned.get_port("zero"), Some(0));
    assert_eq!(owned.get_port("max"), Some(65535));
    assert_eq!(owned.get_port("bad"), None);
    assert_eq!(
        owned.feature_evidence("svc", "repeat"),
        RsdFeatureEvidence::ServiceMissing
    );
    assert_eq!(
        owned.resolved_feature_evidence("svc", "repeat"),
        RsdFeatureEvidence::Advertised
    );
    let clone = owned.clone();
    assert_eq!(clone.udid().as_ptr(), uid);
    let service = owned.get_resolved_service("svc").unwrap();
    let name = service.name().as_ptr();
    let feature = service.features().nth(1).unwrap().as_ptr();
    let mut iterator = owned.into_services();
    let exported = iterator.next().unwrap();
    assert_eq!(iterator.len(), 2);
    bank.close();
    drop((iterator, clone, exported));
    assert_eq!(bank.available_permits(), TOTAL - bytes);
    assert_eq!(service.name().as_ptr(), name);
    assert_eq!(service.features().nth(1).unwrap().as_ptr(), feature);
    assert_eq!(
        service.features().collect::<Vec<_>>(),
        ["", "repeat", "repeat"]
    );
    drop(service);
    assert_eq!(bank.available_permits(), TOTAL);
}

#[test]
fn owned_initial_parallel_last_service_exports_keep_original_directory_identity_and_refund_once() {
    let wire = encode_message(&sample()).unwrap();
    let bank = Arc::new(Semaphore::new(TOTAL));
    let tree = borrowed::decode(&wire[..24], &wire[24..], &bank).unwrap();
    let mut owned = OwnedRsdHandshake::prepare(&bank).unwrap();
    owned.fill(tree.as_ref()).unwrap();
    drop(tree);
    let service = owned.get_resolved_service("svc").unwrap();
    drop(owned);
    bank.close();
    let name = service.name().as_ptr() as usize;
    let barrier = Arc::new(std::sync::Barrier::new(8));
    std::thread::scope(|scope| {
        for _ in 0..7 {
            let kept = service.clone();
            let barrier = barrier.clone();
            scope.spawn(move || {
                assert_eq!(kept.name().as_ptr() as usize, name);
                assert_eq!(kept.port(), 42);
                barrier.wait();
                drop(kept);
            });
        }
        barrier.wait();
        drop(service);
    });
    assert_eq!(bank.available_permits(), TOTAL);
}

fn with_after_free<T>(hook: impl FnMut() + 'static, work: impl FnOnce() -> T) -> T {
    struct Reset;
    impl Drop for Reset {
        fn drop(&mut self) {
            AFTER_FREE.with(|hook| hook.borrow_mut().take());
        }
    }
    AFTER_FREE.with(|current| {
        assert!(current.borrow().is_none());
        *current.borrow_mut() = Some(Box::new(hook));
    });
    let _reset = Reset;
    work()
}

#[test]
fn owned_initial_actual_root_requires_the_exact_whole_original_grant_before_backing() {
    use crate::device::prepared_rsd::with_allocation_hook;
    let bytes = OwnedRsdHandshake::body_bytes();
    assert_eq!(bytes, Layout::new::<Allocation>().size());
    assert!(bytes > mem::size_of::<Directory>());
    for (foreign, amount) in [(true, bytes), (false, bytes - 1), (false, bytes + 1)] {
        let bank = Arc::new(Semaphore::new(2 * bytes));
        let other = Arc::new(Semaphore::new(2 * bytes));
        let grant = charge(if foreign { &other } else { &bank }, amount).unwrap();
        let error = with_allocation_hook(
            |_, _| panic!("invalid initial root grant allocated a backing"),
            || OwnedRsdHandshake::from_grant(&bank, grant).unwrap_err(),
        );
        assert_eq!(
            crate::classify_error(&error),
            crate::ErrorClass::ResourceLimit
        );
        assert_eq!(bank.available_permits(), 2 * bytes);
        assert_eq!(other.available_permits(), 2 * bytes);
    }
    let bank = Arc::new(Semaphore::new(bytes - 1));
    with_allocation_hook(
        |_, _| panic!("short initial root pool allocated a backing"),
        || assert!(OwnedRsdHandshake::prepare(&bank).is_err()),
    );
    assert_eq!(bank.available_permits(), bytes - 1);
    let bank = Arc::new(Semaphore::new(bytes));
    let seen = Arc::new(Mutex::new(Vec::new()));
    let observed = seen.clone();
    let observed_bank = bank.clone();
    let owned = with_allocation_hook(
        move |layout, pointer| {
            assert_eq!(observed_bank.available_permits(), 0);
            observed.lock().unwrap().push((layout, pointer));
        },
        || OwnedRsdHandshake::prepare(&bank).unwrap(),
    );
    assert_eq!(
        &*seen.lock().unwrap(),
        &[(Layout::new::<Allocation>(), owned.allocation_address())]
    );
    let directory = owned.data() as *const Directory as usize;
    assert!((owned.allocation_address()..owned.allocation_address() + bytes).contains(&directory));
    assert_eq!(owned.known_bytes(), bytes);
    assert!(owned.belongs_to(&bank));
    let observed_bank = bank.clone();
    with_after_free(
        move || {
            assert_eq!(
                observed_bank.available_permits(),
                0,
                "root credit preceded actual free"
            )
        },
        || drop(owned),
    );
    assert_eq!(bank.available_permits(), bytes);
}

#[test]
fn owned_initial_last_export_frees_the_original_whole_root_before_any_credit_in_a_closed_pool() {
    let wire = encode_message(&sample()).unwrap();
    let bank = Arc::new(Semaphore::new(TOTAL));
    let tree = borrowed::decode(&wire[..24], &wire[24..], &bank).unwrap();
    let mut owned = OwnedRsdHandshake::prepare(&bank).unwrap();
    let root = owned.allocation_address();
    owned.fill(tree.as_ref()).unwrap();
    drop(tree);
    drop(wire);
    let bytes = owned.known_bytes();
    let uid = owned.udid().as_ptr();
    let service = owned.get_resolved_service("svc").unwrap();
    let name = service.name().as_ptr();
    let feature = service.features().nth(1).unwrap().as_ptr();
    let kept = owned.clone();
    let mut services = owned.into_services();
    let exported = services.next().unwrap();
    assert_eq!(kept.allocation_address(), root);
    assert_eq!(service.directory.allocation_address(), root);
    assert_eq!(exported.directory.allocation_address(), root);
    let rest = bank
        .clone()
        .try_acquire_many_owned(bank.available_permits() as u32)
        .unwrap();
    bank.close();
    assert_eq!(kept.udid().as_ptr(), uid);
    drop((kept, services, exported));
    assert_eq!(bank.available_permits(), 0);
    assert_eq!(service.name().as_ptr(), name);
    assert_eq!(service.features().nth(1).unwrap().as_ptr(), feature);
    let observed_bank = bank.clone();
    let calls = Arc::new(AtomicUsize::new(0));
    let observed_calls = calls.clone();
    with_after_free(
        move || {
            observed_calls.fetch_add(1, Ordering::SeqCst);
            assert_eq!(
                observed_bank.available_permits(),
                0,
                "an original field or root fee was returned before whole-root free"
            );
        },
        || drop(service),
    );
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    assert_eq!(bank.available_permits(), bytes);
    drop(rest);
    assert_eq!(bank.available_permits(), TOTAL);
}

struct Script {
    input: Vec<u8>,
    offset: usize,
    reads: Arc<AtomicUsize>,
}
impl AsyncRead for Script {
    fn poll_read(
        mut self: Pin<&mut Self>,
        _: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<std::io::Result<()>> {
        self.reads.fetch_add(1, Ordering::SeqCst);
        let n = buf.remaining().min(self.input.len() - self.offset);
        let end = self.offset + n;
        buf.put_slice(&self.input[self.offset..end]);
        self.offset = end;
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
fn frame(ty: u8, stream: u32, payload: &[u8]) -> Vec<u8> {
    let len = payload.len() as u32;
    let mut wire = vec![(len >> 16) as u8, (len >> 8) as u8, len as u8, ty, 0];
    wire.extend_from_slice(&stream.to_be_bytes());
    wire.extend_from_slice(payload);
    wire
}
async fn framer(
    messages: Vec<XpcMessage>,
    bank: &Arc<Semaphore>,
) -> (crate::xpc::h2_raw::H2Framer<Script>, Arc<AtomicUsize>) {
    let mut input = frame(4, 0, &[]);
    for message in messages {
        input.extend_from_slice(&frame(0, 1, &encode_message(&message).unwrap()));
    }
    let reads = Arc::new(AtomicUsize::new(0));
    let framer = crate::xpc::h2_raw::H2Framer::connect_with_bank(
        Script {
            input,
            offset: 0,
            reads: reads.clone(),
        },
        Some(bank),
    )
    .await
    .unwrap();
    (framer, reads)
}

#[tokio::test]
async fn owned_initial_public_framer_api_keeps_six_message_semantic_limit_and_persistent_export() {
    let mut wrong = sample();
    let XpcValue::Dictionary(body) = wrong.body.as_mut().unwrap() else {
        unreachable!()
    };
    body.insert(
        "MessageType".into(),
        XpcValue::String("semantic-skip".into()),
    );
    for succeeds in [true, false] {
        let bank = Arc::new(Semaphore::new(TOTAL));
        let mut messages = vec![wrong.clone(); if succeeds { 5 } else { 6 }];
        messages.push(sample());
        let (mut framer, _) = framer(messages, &bank).await;
        let result = super::super::owned_handshake_on_framer(&mut framer).await;
        drop(framer);
        if succeeds {
            let owned = result.unwrap();
            assert_eq!(owned.get_port("svc"), Some(42));
            assert_eq!(bank.available_permits(), TOTAL - owned.known_bytes());
            let service = owned.get_resolved_service("svc").unwrap();
            drop(owned);
            assert_eq!(service.port(), 42);
            drop(service);
        } else {
            assert_eq!(
                result.unwrap_err().to_string(),
                XpcError::Tls("RSD: unexpected MessageType; expected Handshake".into()).to_string()
            );
        }
        assert_eq!(bank.available_permits(), TOTAL);
    }
}

#[tokio::test]
async fn owned_initial_closed_bank_refusal_is_before_another_read_and_never_creates_a_new_pool() {
    let bank = Arc::new(Semaphore::new(TOTAL));
    let (mut framer, reads) = framer(vec![sample()], &bank).await;
    let before = reads.load(Ordering::SeqCst);
    let permits = bank.available_permits();
    bank.close();
    let error = super::super::owned_handshake_on_framer(&mut framer)
        .await
        .unwrap_err();
    assert_eq!(
        crate::classify_error(&error),
        crate::ErrorClass::ResourceLimit
    );
    assert_eq!(reads.load(Ordering::SeqCst), before);
    assert_eq!(bank.available_permits(), permits);
    drop(framer);
    assert_eq!(bank.available_permits(), TOTAL);
}
