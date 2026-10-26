//! External Rust consumer of the production CLI workflow engine.
use ios_cli::{TestWorkflowArgs, TestWorkflowEngine, TestWorkflowRunOptions};
use std::time::Duration;

#[tokio::test]
async fn exported_engine_is_executable_and_validates_before_invoking_plan_resolver() {
    let workflow = TestWorkflowArgs {
        crash_output: Some(std::env::temp_dir().join(uuid::Uuid::new_v4().to_string())),
        ..Default::default()
    };
    let engine = TestWorkflowEngine::new(
        "explicit-test-device",
        &workflow,
        tokio::time::Instant::now() + Duration::from_secs(1),
        true,
        None,
    );
    let error = engine
        .run(
            TestWorkflowRunOptions {
                startup_timeout: Duration::from_secs(1),
                result_timeout: Duration::from_secs(1),
                json_output: false,
                untested_banner: false,
                untested_receipt: false,
                monitored_bundle_ids: Vec::new(),
            },
            |_| {
                Box::pin(async {
                    panic!("invalid local arguments must not discover or connect a device")
                })
            },
        )
        .await
        .unwrap_err();
    assert!(error.chain().any(|cause| cause.is::<std::io::Error>()));
}
