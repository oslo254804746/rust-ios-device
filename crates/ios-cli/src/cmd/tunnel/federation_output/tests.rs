use super::super::{record_owner, response_json};
use super::*;
use futures_util::StreamExt;
use std::sync::Arc;
use tokio::sync::Semaphore;

fn raw(uid: &str, userspace: bool) -> TunnelRecord {
    TunnelRecord {
        udid: uid.into(),
        interface: format!("usbmux-{uid}"),
        connection_type: "USB".into(),
        mode: if userspace { "userspace" } else { "kernel" }.into(),
        address: "fd00::1".into(),
        rsd_port: 12345,
        tunnel_address: "fd00::1".into(),
        tunnel_port: 12345,
        userspace_tun: userspace,
        userspace_tun_host: userspace.then(|| "127.0.0.1".into()),
        userspace_tun_port: userspace.then_some(23456),
        userspace_port: userspace.then_some(23456),
    }
}

fn admitted(bank: &Arc<Semaphore>, raw: &[TunnelRecord]) -> KnownVec<PaidRecord> {
    let mut local = KnownVec::new(bank.clone(), raw.len());
    local.reserve(raw.len()).unwrap();
    for record in raw {
        local.push_reserved(PaidRecord::from_fixture(record, bank).unwrap());
    }
    local
}

#[test]
fn federation_output_moves_original_paid_fields_and_serializes_at_zero_remaining_quota() {
    let bank = Arc::new(Semaphore::new(16384));
    let raw = [raw("雪🦀-b", true), raw("雪🦀-a", false)];
    let local = admitted(&bank, &raw);
    let pointers = local
        .as_slice()
        .iter()
        .map(|record| (record.udid.clone(), record.udid.as_ptr()))
        .collect::<Vec<_>>();
    let before = bank.available_permits();
    let held = bank.clone().try_acquire_many_owned(before as u32).unwrap();
    let hook = record_owner::on_text_allocated(|_, _| panic!("listing cloned local text"));
    let listing = Listing::new(local);
    for (uid, pointer) in pointers {
        let actual = listing
            .local
            .as_slice()
            .iter()
            .find(|record| record.udid == uid)
            .unwrap();
        assert_eq!(actual.udid.as_ptr(), pointer);
    }
    let value = serde_json::to_value(&listing).unwrap();
    for record in &raw {
        assert_eq!(
            value[&record.udid][0],
            serde_json::to_value(record).unwrap()
        );
    }
    assert_eq!(bank.available_permits(), 0);
    drop(hook);
    drop(listing);
    assert_eq!(bank.available_permits(), 16384 - before);
    drop(held);
    assert_eq!(bank.available_permits(), 16384);
}

#[test]
fn federation_output_same_uid_keeps_local_first_remote_order_unknown_fields_and_one_key() {
    let bank = Arc::new(Semaphore::new(16384));
    let raw = [raw("same-雪", true), raw("only-local", false)];
    let mut listing = Listing::new(admitted(&bank, &raw));
    let remote = serde_json::json!({"same-雪":[{"extension":{"snow":"雪","binary":[0,255]},"upstream":"first"},{"upstream":"second"}],"only-remote":[{"future-field":null,"upstream":"third"}]});
    listing.remote = serde_json::from_value(remote.clone()).unwrap();
    let output = serde_json::to_value(&listing).unwrap();
    assert_eq!(output.as_object().unwrap().len(), 3);
    assert_eq!(output["same-雪"][0], serde_json::to_value(&raw[0]).unwrap());
    assert_eq!(output["same-雪"][1], remote["same-雪"][0]);
    assert_eq!(output["same-雪"][2], remote["same-雪"][1]);
    assert_eq!(output["only-remote"], remote["only-remote"]);
    assert_eq!(
        output["only-local"][0],
        serde_json::to_value(&raw[1]).unwrap()
    );
    let wire = serde_json::to_string(&listing).unwrap();
    assert_eq!(wire.matches("\"same-雪\":").count(), 1);
}

#[test]
fn federation_output_scope_removes_both_local_and_remote_without_reintroducing_hidden_keys() {
    let bank = Arc::new(Semaphore::new(16384));
    let raw = [raw("hidden", false), raw("allowed", true)];
    let mut listing = Listing::new(admitted(&bank, &raw));
    listing.remote = serde_json::from_value(serde_json::json!({"hidden":[{"secret":true}],"allowed":[{"upstream":"route"}],"remote-hidden":[{}]})).unwrap();
    let before = bank.available_permits();
    listing.retain(|uid| uid == "allowed");
    assert!(
        bank.available_permits() > before,
        "filtered paid text must be freed"
    );
    let value = serde_json::to_value(&listing).unwrap();
    assert_eq!(value.as_object().unwrap().len(), 1);
    assert_eq!(value["allowed"].as_array().unwrap().len(), 2);
    assert!(!serde_json::to_string(&listing).unwrap().contains("hidden"));
}

#[test]
fn federation_output_root_local_object_preserves_legacy_sorted_bytes_and_option_omissions() {
    for userspace in [false, true] {
        let record = raw("quote\"\n雪🦀", userspace);
        let expected = serde_json::to_vec(&serde_json::to_value(&record).unwrap()).unwrap();
        assert_eq!(serde_json::to_vec(&LocalObject(&record)).unwrap(), expected);
    }
}

#[tokio::test]
async fn federation_output_over_one_mib_keeps_record_and_wire_fees_separate_until_last_slice() {
    let bank = Arc::new(Semaphore::new(16 * 1024 * 1024));
    let mut record = raw("large-local", true);
    record.interface = "🦀".repeat(300000);
    let listing = Listing::new(admitted(&bank, &[record]));
    let paid = 16 * 1024 * 1024 - bank.available_permits();
    let replies = response_json::Replies::new(&bank).unwrap();
    let response = replies.json(&listing).unwrap();
    let mut body = response.into_body().into_data_stream();
    let frame = body.next().await.unwrap().unwrap();
    assert!(frame.len() > super::super::MAX_LIST_BYTES);
    let wire = frame.len();
    let last = frame.slice(1..2);
    assert_eq!(
        bank.available_permits(),
        16 * 1024 * 1024 - paid - 2048 - wire
    );
    drop(listing);
    drop(replies);
    drop(frame);
    drop(body);
    assert_eq!(bank.available_permits(), 16 * 1024 * 1024 - wire);
    drop(last);
    assert_eq!(bank.available_permits(), 16 * 1024 * 1024);
}
