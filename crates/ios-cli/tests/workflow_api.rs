//! External Rust consumer of the production CLI workflow engine and reusable `ios-core` workflow API.
use ios_cli::{
    RunnerLaunchUnconfirmed, TestWorkflowArgs, TestWorkflowEngine, TestWorkflowRunOptions,
};
use ios_core::testmanager::results::{TestCaseStatus, TestExecutionEvent, TestRunRecorder};
use ios_core::testmanager::workflow::{
    execute_test_workflow_attempts, InstalledAppInfo, PendingRunnerLaunch, TestLaunchPlan,
};
use std::collections::HashMap;
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

#[tokio::test]
async fn core_workflow_executor_and_unconfirmed_launch_types_are_reusable_from_external_crates() {
    let pending = PendingRunnerLaunch {
        bundle_id: Some("com.example.runner".into()),
        baseline_pids: Some(vec![100]),
        launch_submitted: true,
        confirmed_pid: None,
    };
    assert!(pending.needs_reconciliation(true));
    assert!(RunnerLaunchUnconfirmed
        .to_string()
        .contains("XCTest launch was submitted"));

    let plan = TestLaunchPlan {
        runner: InstalledAppInfo {
            bundle_id: "com.example.runner".into(),
            path: "/Applications/Runner.app".into(),
            executable: "Runner".into(),
            container: None,
        },
        target: None,
        xctest_bundle_name: "RunnerTests.xctest".into(),
        is_xctest: true,
        args: Vec::new(),
        env: HashMap::new(),
        tests_to_run: Vec::new(),
        tests_to_skip: Vec::new(),
    };

    let report = execute_test_workflow_attempts(plan, 1, |attempt, attempt_plan| async move {
        let mut recorder = TestRunRecorder::default();
        recorder.apply(TestExecutionEvent::BeganPlan);
        recorder.apply(TestExecutionEvent::SuiteStarted {
            name: "Suite".into(),
            started_at: None,
        });
        if attempt == 1 {
            recorder.apply(TestExecutionEvent::CaseStarted {
                class_name: "Suite".into(),
                method_name: "testFlaky".into(),
            });
            recorder.apply(TestExecutionEvent::CaseFinished {
                class_name: "Suite".into(),
                method_name: "testFlaky".into(),
                status: TestCaseStatus::Failed,
                duration_seconds: 0.2,
            });
        } else {
            assert_eq!(attempt_plan.tests_to_run, vec!["Suite/testFlaky"]);
            recorder.apply(TestExecutionEvent::CaseStarted {
                class_name: "Suite".into(),
                method_name: "testFlaky".into(),
            });
            recorder.apply(TestExecutionEvent::CaseFinished {
                class_name: "Suite".into(),
                method_name: "testFlaky".into(),
                status: TestCaseStatus::Passed,
                duration_seconds: 0.1,
            });
        }
        recorder.apply(TestExecutionEvent::FinishedPlan);
        Ok::<_, String>(recorder.summary())
    })
    .await
    .unwrap();

    assert_eq!(report.attempts.len(), 2);
    assert_eq!(report.summary.failed_tests, 0);
}
