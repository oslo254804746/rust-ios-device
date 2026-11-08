use super::*;
use crate::services::restore::test_assets::{http, os_failure};
use crate::{test_native::has_source, ErrorClass};
use std::{collections::VecDeque, error::Error, future::ready, io};

fn rider(updater: &str, ticket_key: &str) -> TssRider {
    TssRider {
        updater: updater.into(),
        ticket_key: ticket_key.into(),
        nonce: vec![17, 18],
        request: plist::Dictionary::from_iter([
            ("ApECID".into(), plist::Value::Integer(42.into())),
            (format!("@{ticket_key}"), plist::Value::Boolean(true)),
        ]),
    }
}

fn tickets(keys: &[&str]) -> plist::Dictionary {
    keys.iter()
        .map(|key| ((*key).to_string(), plist::Value::Data(vec![201, 202])))
        .collect()
}

#[tokio::test]
async fn batch_observations_retain_combined_and_individual_native_errors() {
    let elapsed = tokio::time::timeout(Duration::ZERO, std::future::pending::<()>())
        .await
        .unwrap_err();
    let original_io = os_failure();
    let os_code = original_io.raw_os_error();
    let mut replies = VecDeque::from([
        Err(native_error(
            io::ErrorKind::TimedOut,
            "owned timer",
            elapsed,
        )),
        Err(RestoreError::Io(original_io)),
        parse_tss_response(b"STATUS=0&\xff"),
    ]);
    let outcome = prefetch_tss_batch_preserving_errors(
        &[rider("owned-private-A", "A,Ticket"), rider("B", "B,Ticket")],
        |_| ready(replies.pop_front().unwrap()),
    )
    .await
    .unwrap();
    assert!(replies.is_empty());
    assert_eq!(outcome.result().failed_updaters, ["owned-private-A", "B"]);
    assert!(outcome.result().tickets.is_empty());
    assert_eq!(outcome.failures().len(), 3);
    let failures = outcome.failures();
    assert_eq!(failures[0].phase(), TssBatchPhase::SignCombinedRequest);
    assert_eq!(failures[0].rider_index(), None);
    assert_eq!(failures[0].error_class(), Some(ErrorClass::Timeout));
    assert!(has_source::<tokio::time::error::Elapsed>(
        failures[0].error().unwrap()
    ));
    assert_eq!(failures[1].phase(), TssBatchPhase::SignIndividualRequest);
    assert_eq!(failures[1].rider_index(), Some(0));
    assert_eq!(failures[1].error_class(), Some(ErrorClass::NotFound));
    let RestoreError::Io(original) = failures[1].error().unwrap() else {
        panic!("original IO variant lost");
    };
    assert_eq!(original.raw_os_error(), os_code);
    assert_eq!(failures[2].rider_index(), Some(1));
    assert_eq!(failures[2].error_class(), Some(ErrorClass::Protocol));
    assert!(has_source::<std::str::Utf8Error>(
        failures[2].error().unwrap()
    ));
    for failure in failures {
        assert_eq!(failure.kind(), TssBatchFailureKind::Error);
        assert!(!format!("{failure:?}").contains("owned-private"));
    }
    assert!(!format!("{outcome:?}").contains("owned-private"));
    let (result, mut failures) = outcome.into_parts();
    assert_eq!(result.failed_updaters.len(), 2);
    let owned = failures.remove(0).into_error().unwrap();
    assert!(has_source::<tokio::time::error::Elapsed>(&owned));
}

#[tokio::test]
async fn batch_observations_keep_recovered_combined_error() {
    let mut replies = VecDeque::from([
        parse_tss_response(b"STATUS=0&\xff"),
        Ok(tickets(&["A,Ticket"])),
        Ok(tickets(&["B,Ticket"])),
    ]);
    let outcome = prefetch_tss_batch_preserving_errors(
        &[rider("A", "A,Ticket"), rider("B", "B,Ticket")],
        |_| ready(replies.pop_front().unwrap()),
    )
    .await
    .unwrap();
    assert!(replies.is_empty());
    assert!(outcome.result().failed_updaters.is_empty());
    assert_eq!(outcome.result().tickets.len(), 2);
    assert_eq!(outcome.failures().len(), 1);
    assert!(has_source::<std::str::Utf8Error>(
        outcome.failures()[0].error().unwrap()
    ));
    assert_eq!(
        outcome.result().tickets["A"].for_nonce(&[17, 18]),
        Some(&[201, 202][..])
    );
    assert_eq!(outcome.result().tickets["A"].for_nonce(&[17, 19]), None);
}

#[tokio::test]
async fn batch_observations_distinguish_build_conflict_without_combined_signing() {
    let first = rider("A", "A,Ticket");
    let mut second = rider("B", "B,Ticket");
    second
        .request
        .insert("ApECID".into(), plist::Value::Integer(43.into()));
    let riders = [first, second];
    let mut requests = Vec::new();
    let outcome = prefetch_tss_batch_preserving_errors(&riders, |request| {
        requests.push(request);
        ready(Ok(tickets(&["A,Ticket", "B,Ticket"])))
    })
    .await
    .unwrap();
    assert_eq!(
        requests,
        riders.iter().map(|r| r.request.clone()).collect::<Vec<_>>()
    );
    assert_eq!(outcome.result().tickets.len(), 2);
    assert_eq!(outcome.failures().len(), 1);
    let failure = &outcome.failures()[0];
    assert_eq!(failure.phase(), TssBatchPhase::BuildCombinedRequest);
    assert_eq!(failure.rider_index(), None);
    assert_eq!(failure.kind(), TssBatchFailureKind::Error);
    assert_eq!(failure.error_class(), Some(ErrorClass::Protocol));
    let error = failure.error().unwrap();
    assert!(matches!(error, RestoreError::Protocol(_)));
    assert!(error.source().is_none());
}

#[tokio::test]
async fn batch_observations_distinguish_missing_empty_and_wrong_type_tickets() {
    for value in [
        None,
        Some(plist::Value::Data(vec![])),
        Some(plist::Value::String("owned-private".into())),
    ] {
        let mut individual = plist::Dictionary::new();
        if let Some(value) = value {
            individual.insert("B,Ticket".into(), value);
        }
        let mut replies = VecDeque::from([Ok(tickets(&["A,Ticket"])), Ok(individual)]);
        let outcome = prefetch_tss_batch_preserving_errors(
            &[rider("A", "A,Ticket"), rider("B", "B,Ticket")],
            |_| ready(replies.pop_front().unwrap()),
        )
        .await
        .unwrap();
        assert!(replies.is_empty());
        assert_eq!(outcome.result().tickets.len(), 1);
        assert_eq!(outcome.result().failed_updaters, ["B"]);
        assert_eq!(outcome.failures().len(), 2);
        for (failure, phase) in outcome.failures().iter().zip([
            TssBatchPhase::SignCombinedRequest,
            TssBatchPhase::SignIndividualRequest,
        ]) {
            assert_eq!(failure.phase(), phase);
            assert_eq!(failure.rider_index(), Some(1));
            assert_eq!(failure.kind(), TssBatchFailureKind::MissingTicket);
            assert!(failure.error().is_none());
            assert!(failure.error_class().is_none());
        }
    }
}

#[tokio::test]
async fn batch_observations_keep_recovered_missing_ticket() {
    let mut replies = VecDeque::from([Ok(tickets(&["A,Ticket"])), Ok(tickets(&["B,Ticket"]))]);
    let outcome = prefetch_tss_batch_preserving_errors(
        &[rider("A", "A,Ticket"), rider("B", "B,Ticket")],
        |_| ready(replies.pop_front().unwrap()),
    )
    .await
    .unwrap();
    assert_eq!(outcome.result().tickets.len(), 2);
    assert!(outcome.result().failed_updaters.is_empty());
    assert_eq!(outcome.failures().len(), 1);
    assert_eq!(
        outcome.failures()[0].kind(),
        TssBatchFailureKind::MissingTicket
    );
    assert_eq!(outcome.failures()[0].rider_index(), Some(1));
}

#[tokio::test]
async fn batch_observations_keep_native_http_timeout_after_successful_fallback() {
    let (outcome, wire) = http(None, true, |client, url| async move {
        let mut calls = 0;
        let outcome = prefetch_tss_batch_preserving_errors(&[rider("A", "A,Ticket")], |request| {
            calls += 1;
            let is_combined = calls == 1;
            let client = client.clone();
            let url = url.clone();
            async move {
                if is_combined {
                    sign_tss_request_with_client(&client, &url, &request).await
                } else {
                    Ok(tickets(&["A,Ticket"]))
                }
            }
        })
        .await
        .unwrap();
        assert_eq!(calls, 2);
        outcome
    })
    .await;
    assert!(wire.starts_with(b"POST /owned-private HTTP/1.1\r\n"));
    assert_eq!(outcome.result().tickets.len(), 1);
    assert_eq!(outcome.failures().len(), 1);
    let failure = &outcome.failures()[0];
    assert_eq!(failure.error_class(), Some(ErrorClass::Timeout));
    assert!(has_source::<reqwest::Error>(failure.error().unwrap()));
    assert!(!format!("{failure:?} {outcome:?}").contains("owned-private"));
}

#[tokio::test]
async fn batch_observations_empty_and_complete_input_have_no_failures() {
    let outcome = prefetch_tss_batch_preserving_errors(
        &[],
        |_| -> std::future::Ready<Result<plist::Dictionary, RestoreError>> {
            panic!("empty input must not invoke signer")
        },
    )
    .await
    .unwrap();
    assert!(outcome.result().tickets.is_empty());
    assert!(outcome.result().failed_updaters.is_empty());
    assert!(outcome.failures().is_empty());
    let mut calls = 0;
    let outcome = prefetch_tss_batch_preserving_errors(&[rider("A", "A,Ticket")], |_| {
        calls += 1;
        ready(Ok(tickets(&["A,Ticket"])))
    })
    .await
    .unwrap();
    assert_eq!(calls, 1);
    assert!(outcome.failures().is_empty());
    assert_eq!(outcome.into_result().tickets.len(), 1);
}

#[tokio::test]
async fn batch_observations_validate_all_riders_before_any_signing() {
    let valid = rider("A", "A,Ticket");
    for field in 0..6 {
        let mut invalid = rider("B", "B,Ticket");
        match field {
            0 => invalid.updater.clear(),
            1 => invalid.ticket_key.clear(),
            2 => invalid.nonce.clear(),
            3 => invalid.nonce = vec![1; 65],
            4 => {
                invalid.request.remove("@B,Ticket");
            }
            _ => {
                invalid
                    .request
                    .insert("@ApImg4Ticket".into(), plist::Value::Boolean(true));
            }
        }
        let mut calls = 0;
        let error = prefetch_tss_batch_preserving_errors(&[valid.clone(), invalid], |_| {
            calls += 1;
            ready(Ok(plist::Dictionary::new()))
        })
        .await
        .unwrap_err();
        assert_eq!(calls, 0);
        assert!(matches!(error, RestoreError::Protocol(_)));
    }
}

#[tokio::test]
async fn batch_observations_duplicate_updaters_retain_input_indices_and_legacy_result() {
    let mut replies = VecDeque::from([
        Ok(plist::Dictionary::new()),
        Err(RestoreError::Io(os_failure())),
        Ok(tickets(&["B,Ticket"])),
    ]);
    let outcome = prefetch_tss_batch_preserving_errors(
        &[rider("same", "A,Ticket"), rider("same", "B,Ticket")],
        |_| ready(replies.pop_front().unwrap()),
    )
    .await
    .unwrap();
    assert_eq!(outcome.result().failed_updaters, ["same"]);
    assert_eq!(outcome.result().tickets["same"].ticket_key, "B,Ticket");
    assert_eq!(outcome.failures().len(), 3);
    assert_eq!(
        outcome
            .failures()
            .iter()
            .map(TssBatchFailure::rider_index)
            .collect::<Vec<_>>(),
        [Some(0), Some(0), Some(1)]
    );
}

#[tokio::test]
async fn batch_observations_legacy_and_retaining_paths_have_identical_signer_policy() {
    for scenario in 0..5 {
        let mut riders = [rider("A", "A,Ticket"), rider("B", "B,Ticket")];
        if scenario == 4 {
            riders[1]
                .request
                .insert("ApECID".into(), plist::Value::Integer(43.into()));
        }
        let response = |call| match (scenario, call) {
            (0, 0) => Ok(tickets(&["A,Ticket", "B,Ticket"])),
            (1, 0) => Ok(tickets(&["A,Ticket"])),
            (2 | 3, 0) => Err(RestoreError::Protocol("owned-private timeout".into())),
            (3, 2) => Ok(plist::Dictionary::new()),
            _ => Ok(tickets(&["A,Ticket", "B,Ticket"])),
        };
        let mut legacy_requests = Vec::new();
        let legacy = prefetch_tss_batch(&riders, |request| {
            let reply = response(legacy_requests.len());
            legacy_requests.push(request);
            ready(reply)
        })
        .await
        .unwrap();
        let mut observed_requests = Vec::new();
        let observed = prefetch_tss_batch_preserving_errors(&riders, |request| {
            let reply = response(observed_requests.len());
            observed_requests.push(request);
            ready(reply)
        })
        .await
        .unwrap();
        let expected_calls = [1, 2, 3, 3, 2][scenario];
        assert_eq!(legacy_requests.len(), expected_calls);
        assert_eq!(observed_requests.len(), expected_calls);
        for requests in [&mut legacy_requests, &mut observed_requests] {
            if scenario != 4 {
                let uuid = requests[0].remove("@UUID").unwrap();
                assert!(uuid
                    .as_string()
                    .is_some_and(|value| uuid::Uuid::parse_str(value).is_ok()));
            }
        }
        assert_eq!(legacy_requests, observed_requests);
        assert_eq!(legacy.failed_updaters, observed.result().failed_updaters);
        assert_eq!(legacy.tickets.len(), observed.result().tickets.len());
        for (updater, ticket) in legacy.tickets {
            let actual = &observed.result().tickets[&updater];
            assert_eq!(actual.ticket_key, ticket.ticket_key);
            assert_eq!(actual.nonce, ticket.nonce);
            assert_eq!(actual.ticket, ticket.ticket);
        }
    }
}

#[tokio::test]
async fn batch_observations_debug_keeps_callback_payload_private() {
    let outcome = prefetch_tss_batch_preserving_errors(
        &[rider("owned-private", "owned-private,Ticket")],
        |_| {
            ready(Err(RestoreError::Protocol(
                "owned-private timeout permission unsupported".into(),
            )))
        },
    )
    .await
    .unwrap();
    assert_eq!(outcome.failures().len(), 2);
    for failure in outcome.failures() {
        assert_eq!(failure.error_class(), Some(ErrorClass::Protocol));
        assert_eq!(
            failure.error().unwrap().to_string(),
            "protocol error: owned-private timeout permission unsupported"
        );
        assert!(!format!("{failure:?}").contains("owned-private"));
    }
    assert!(!format!("{outcome:?}").contains("owned-private"));
}

#[tokio::test]
async fn batch_observations_legacy_drops_each_error_before_next_signer_call() {
    use std::sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    };
    #[derive(Debug)]
    struct Counted(Arc<AtomicUsize>);
    impl std::fmt::Display for Counted {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            f.write_str("owned-private counted error")
        }
    }
    impl Error for Counted {}
    impl Drop for Counted {
        fn drop(&mut self) {
            self.0.fetch_add(1, Ordering::SeqCst);
        }
    }
    for preserve in [false, true] {
        let drops = Arc::new(AtomicUsize::new(0));
        let mut calls = 0;
        let signer = |_| {
            assert_eq!(
                drops.load(Ordering::SeqCst),
                if preserve { 0 } else { calls }
            );
            calls += 1;
            ready(Err(RestoreError::Io(io::Error::other(Counted(
                drops.clone(),
            )))))
        };
        let riders = [rider("A", "A,Ticket"), rider("B", "B,Ticket")];
        let outcome = if preserve {
            Some(
                prefetch_tss_batch_preserving_errors(&riders, signer)
                    .await
                    .unwrap(),
            )
        } else {
            let result = prefetch_tss_batch(&riders, signer).await.unwrap();
            assert_eq!(result.failed_updaters, ["A", "B"]);
            None
        };
        assert_eq!(calls, 3);
        assert_eq!(drops.load(Ordering::SeqCst), if preserve { 0 } else { 3 });
        drop(outcome);
        assert_eq!(drops.load(Ordering::SeqCst), 3);
    }
}
