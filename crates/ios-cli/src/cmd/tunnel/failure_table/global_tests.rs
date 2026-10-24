use super::super::{birth_bank::BirthBank, lifecycle::Lifecycle, marker_identity};
use super::*;

#[tokio::test]
async fn failure_global_root_same_prepared_value_commits_at_closed_zero_and_outlives_original_node()
{
    let total = 65536;
    let origin = BirthBank::new(total);
    let lifecycle = Lifecycle::with_metadata(origin.clone()).unwrap();
    let ticket = lifecycle.ticket("global-node-root").await.unwrap();
    let bank = Arc::new(Semaphore::new(total));
    let mut node = Node::owned(
        "global-node-root".into(),
        AttemptIdentity {
            route: Route::Usbmux,
            native_id: Some(3),
        },
        &bank,
    )
    .unwrap();
    node.prepare_global(&ticket, &bank).unwrap();
    let pointer = &**node.prepared_global.as_ref().unwrap() as *const super::super::ControlIdentity;
    let held_b = bank
        .clone()
        .try_acquire_many_owned(bank.available_permits() as u32)
        .unwrap();
    let held_a = origin
        .raw()
        .clone()
        .try_acquire_many_owned(origin.available_permits() as u32)
        .unwrap();
    bank.close();
    origin.close();
    let hook =
        super::super::metadata_vec::on_allocation(|_, _| panic!("late global commit allocated"));
    node.commit_global();
    drop(hook);
    assert_eq!(
        &**node.row.global.as_ref().unwrap() as *const super::super::ControlIdentity,
        pointer
    );
    let root = node.row.global.as_ref().unwrap().clone();
    assert!(node.row.matches_global(&ticket.global_epoch()));
    drop((node, ticket, lifecycle, held_a, held_b));
    assert_eq!(
        origin.available_permits(),
        total - marker_identity::Owner::body_bytes() - super::super::ControlIdentity::body_bytes()
    );
    assert_eq!(
        bank.available_permits(),
        total - marker_identity::Owner::body_bytes()
    );
    drop(root);
    assert_eq!(
        (origin.available_permits(), bank.available_permits()),
        (total, total)
    );
}
