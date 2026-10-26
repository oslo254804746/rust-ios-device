//! XCTest result event parsing and run summary accumulation.
//!
//! Testmanager reports progress as DTX method invocations using private XCTest
//! selectors. This module translates the selectors into stable Rust events and
//! accumulates those events into a serializable summary for CLI and binding users.

use std::io::{self, Write};
use std::path::Path;

use crate::services::dtx::{DtxMessage, DtxPayload, NSObject};
use serde::Serialize;

const MAX_CAPTURED_LOG_MESSAGES: usize = 16_384;
const MAX_CAPTURED_LOG_BYTES: usize = 8 * 1024 * 1024;
const MAX_JUNIT_DIAGNOSTIC_BYTES: usize = 64 * 1024;
const MAX_SUITES: usize = 1024;
const MAX_CASES_PER_SUITE: usize = 4096;
const MAX_TOTAL_CASES: usize = 16_384;
const MAX_NAME_LEN: usize = 512;
const MAX_FAILURE_MESSAGE_BYTES: usize = 64 * 1024;
const MAX_FAILURE_FILE_BYTES: usize = 1024;

#[cfg(test)]
#[test]
fn review_regression_finish_only_cases_cannot_bypass_limits() {
    let mut recorder = TestRunRecorder::default();
    recorder.apply(TestExecutionEvent::BeganPlan);
    recorder.apply(TestExecutionEvent::SuiteStarted {
        name: "suite".into(),
        started_at: None,
    });
    for index in 0..=MAX_CASES_PER_SUITE {
        recorder.apply(TestExecutionEvent::CaseFinished {
            class_name: "Class".into(),
            method_name: format!("case{index}"),
            status: TestCaseStatus::Passed,
            duration_seconds: 0.0,
        });
    }
    recorder.apply(TestExecutionEvent::FinishedPlan);
    let summary = recorder.summary();
    assert!(summary.suites[0].cases.len() <= MAX_CASES_PER_SUITE);
    assert!(
        summary.terminal_outcome().is_err(),
        "discarded results cannot pass CI"
    );
}

#[cfg(test)]
#[test]
fn review_regression_failure_only_cases_and_finish_only_suites_are_bounded() {
    let mut recorder = TestRunRecorder::default();
    for index in 0..=MAX_CASES_PER_SUITE {
        recorder.apply(TestExecutionEvent::CaseFailed {
            class_name: "Class".into(),
            method_name: format!("case{index}"),
            message: "failure".into(),
            file: None,
            line: None,
        });
    }
    assert_eq!(recorder.total_cases(), MAX_CASES_PER_SUITE);
    assert!(recorder.summary().results_truncated);
    for index in 0..=MAX_SUITES {
        recorder.apply(TestExecutionEvent::SuiteFinished {
            name: format!("suite{index}"),
            finished_at: None,
            test_count: 0,
            skipped: 0,
            failures: 0,
            expected_failures: 0,
            unexpected_failures: 0,
            uncaught_exceptions: 0,
            test_duration_seconds: 0.0,
            total_duration_seconds: 0.0,
        });
    }
    assert_eq!(recorder.summary().suites.len(), MAX_SUITES);
}

#[cfg(test)]
#[test]
fn review_regression_long_case_identifiers_never_alias_and_truncation_reaches_junit() {
    let mut recorder = TestRunRecorder::default();
    recorder.apply(TestExecutionEvent::BeganPlan);
    for suffix in ["a", "b"] {
        recorder.apply(TestExecutionEvent::CaseFinished {
            class_name: "Class".into(),
            method_name: format!("{}{suffix}", "m".repeat(MAX_NAME_LEN)),
            status: TestCaseStatus::Passed,
            duration_seconds: 0.0,
        });
    }
    recorder.apply(TestExecutionEvent::FinishedPlan);
    let summary = recorder.summary();
    assert_eq!(summary.total_tests, 2);
    assert_ne!(
        summary.suites[0].cases[0].method_name,
        summary.suites[0].cases[1].method_name
    );
    assert!(summary.results_truncated);
    assert_eq!(
        summary.terminal_outcome(),
        Err(TestRunIncomplete::ResultsTruncated)
    );
    assert!(summary
        .to_junit_xml()
        .contains("XCTest result evidence exceeded ingestion limits"));
}

#[cfg(test)]
#[test]
fn review_regression_log_truncation_stays_within_byte_budget() {
    let mut recorder = TestRunRecorder::default();
    recorder.apply(TestExecutionEvent::Log {
        message: "界".repeat(MAX_CAPTURED_LOG_BYTES / 3 + 1),
        debug: false,
    });
    let summary = recorder.summary();
    assert!(summary.logs_truncated);
    assert!(summary.logs.iter().map(String::len).sum::<usize>() <= MAX_CAPTURED_LOG_BYTES);
}

fn truncate_name(mut s: String, max: usize) -> String {
    if s.len() > max {
        let mut end = max;
        while end > 0 && !s.is_char_boundary(end) {
            end -= 1;
        }
        s.truncate(end);
    }
    s
}

/// XCTest selector emitted when a test plan begins.
pub const DID_BEGIN_EXECUTING_TEST_PLAN_SELECTOR: &str = "_XCT_didBeginExecutingTestPlan";
/// XCTest selector emitted when a test plan finishes.
pub const DID_FINISH_EXECUTING_TEST_PLAN_SELECTOR: &str = "_XCT_didFinishExecutingTestPlan";
/// XCTest selector for normal log messages.
pub const LOG_MESSAGE_SELECTOR: &str = "_XCT_logMessage:";
/// XCTest selector for debug log messages.
pub const LOG_DEBUG_MESSAGE_SELECTOR: &str = "_XCT_logDebugMessage:";
/// XCTest selector emitted when a suite starts.
pub const TEST_SUITE_STARTED_SELECTOR: &str = "_XCT_testSuite:didStartAt:";
/// XCTest selector emitted when older XCTest runtimes finish a suite.
pub const TEST_SUITE_FINISHED_SELECTOR: &str =
    "_XCT_testSuite:didFinishAt:runCount:withFailures:unexpected:testDuration:totalDuration:";
/// XCTest selector emitted when newer XCTest runtimes finish a suite with skip counts.
pub const TEST_SUITE_FINISHED_WITH_SKIP_SELECTOR: &str =
    "_XCT_testSuiteWithIdentifier:didFinishAt:runCount:skipCount:failureCount:expectedFailureCount:uncaughtExceptionCount:testDuration:totalDuration:";
/// XCTest selector emitted when a test case starts.
pub const TEST_CASE_STARTED_SELECTOR: &str = "_XCT_testCaseDidStartForTestClass:method:";
/// XCTest selector emitted when a test case finishes.
pub const TEST_CASE_FINISHED_SELECTOR: &str =
    "_XCT_testCaseDidFinishForTestClass:method:withStatus:duration:";
/// XCTest selector emitted when a test case records a failure.
pub const TEST_CASE_FAILED_SELECTOR: &str =
    "_XCT_testCaseDidFailForTestClass:method:withMessage:file:line:";

/// A normalized XCTest execution event decoded from a DTX method invocation.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum TestExecutionEvent {
    /// The test plan started.
    BeganPlan,
    /// The test plan finished.
    FinishedPlan,
    /// XCTest emitted a log message.
    Log {
        /// Log message text.
        message: String,
        /// Whether the message came from the debug log selector.
        debug: bool,
    },
    /// A test suite started.
    SuiteStarted {
        /// Suite name.
        name: String,
        /// Start timestamp string as reported by XCTest.
        started_at: Option<String>,
    },
    /// A test suite finished and reported aggregate counts.
    SuiteFinished {
        /// Suite name.
        name: String,
        /// Finish timestamp string as reported by XCTest.
        finished_at: Option<String>,
        /// Number of tests reported by the suite.
        test_count: u64,
        /// Number of skipped tests.
        skipped: u64,
        /// Number of failures.
        failures: u64,
        /// Number of expected failures.
        expected_failures: u64,
        /// Number of unexpected failures.
        unexpected_failures: u64,
        /// Number of uncaught exceptions.
        uncaught_exceptions: u64,
        /// XCTest execution duration in seconds.
        test_duration_seconds: f64,
        /// Total suite duration in seconds.
        total_duration_seconds: f64,
    },
    /// A test case started.
    CaseStarted {
        /// XCTest class name.
        class_name: String,
        /// XCTest method name.
        method_name: String,
    },
    /// A test case reported a failure.
    CaseFailed {
        /// XCTest class name.
        class_name: String,
        /// XCTest method name.
        method_name: String,
        /// Failure message.
        message: String,
        /// Source file when XCTest reports one.
        file: Option<String>,
        /// Source line when XCTest reports one.
        line: Option<u64>,
    },
    /// A test case finished with a final status.
    CaseFinished {
        /// XCTest class name.
        class_name: String,
        /// XCTest method name.
        method_name: String,
        /// Final test status.
        status: TestCaseStatus,
        /// Test case duration in seconds.
        duration_seconds: f64,
    },
}

impl TestExecutionEvent {
    /// Decode a supported XCTest DTX method invocation.
    pub fn from_dtx_message(message: &DtxMessage) -> Option<Self> {
        let DtxPayload::MethodInvocation { selector, args } = &message.payload else {
            return None;
        };
        match selector.as_str() {
            DID_BEGIN_EXECUTING_TEST_PLAN_SELECTOR => Some(Self::BeganPlan),
            DID_FINISH_EXECUTING_TEST_PLAN_SELECTOR => Some(Self::FinishedPlan),
            LOG_MESSAGE_SELECTOR => Some(Self::Log {
                message: string_arg(args, 0)?,
                debug: false,
            }),
            LOG_DEBUG_MESSAGE_SELECTOR => Some(Self::Log {
                message: string_arg(args, 0)?,
                debug: true,
            }),
            TEST_SUITE_STARTED_SELECTOR => Some(Self::SuiteStarted {
                name: string_arg(args, 0)?,
                started_at: optional_string_arg(args, 1),
            }),
            TEST_SUITE_FINISHED_SELECTOR => Some(Self::SuiteFinished {
                name: string_arg(args, 0)?,
                finished_at: optional_string_arg(args, 1),
                test_count: uint_arg(args, 2).unwrap_or(0),
                skipped: 0,
                failures: uint_arg(args, 3).unwrap_or(0),
                expected_failures: 0,
                unexpected_failures: uint_arg(args, 4).unwrap_or(0),
                uncaught_exceptions: 0,
                test_duration_seconds: double_arg(args, 5).unwrap_or(0.0),
                total_duration_seconds: double_arg(args, 6).unwrap_or(0.0),
            }),
            TEST_SUITE_FINISHED_WITH_SKIP_SELECTOR => {
                let name = identifier_suite_name(args.first())?;
                Some(Self::SuiteFinished {
                    name,
                    finished_at: optional_string_arg(args, 1),
                    test_count: uint_arg(args, 2).unwrap_or(0),
                    skipped: uint_arg(args, 3).unwrap_or(0),
                    failures: uint_arg(args, 4).unwrap_or(0),
                    expected_failures: uint_arg(args, 5).unwrap_or(0),
                    unexpected_failures: 0,
                    uncaught_exceptions: uint_arg(args, 6).unwrap_or(0),
                    test_duration_seconds: double_arg(args, 7).unwrap_or(0.0),
                    total_duration_seconds: double_arg(args, 8).unwrap_or(0.0),
                })
            }
            TEST_CASE_STARTED_SELECTOR => Some(Self::CaseStarted {
                class_name: string_arg(args, 0)?,
                method_name: string_arg(args, 1)?,
            }),
            TEST_CASE_FAILED_SELECTOR => Some(Self::CaseFailed {
                class_name: string_arg(args, 0)?,
                method_name: string_arg(args, 1)?,
                message: string_arg(args, 2).unwrap_or_default(),
                file: optional_string_arg(args, 3),
                line: uint_arg(args, 4),
            }),
            TEST_CASE_FINISHED_SELECTOR => Some(Self::CaseFinished {
                class_name: string_arg(args, 0)?,
                method_name: string_arg(args, 1)?,
                status: TestCaseStatus::from_wda_status(&string_arg(args, 2)?),
                duration_seconds: double_arg(args, 3).unwrap_or(0.0),
            }),
            _ => None,
        }
    }

    /// Return true when this event marks the end of the plan.
    pub fn is_finished_plan(&self) -> bool {
        matches!(self, Self::FinishedPlan)
    }
}

/// Normalized XCTest case status.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum TestCaseStatus {
    /// Test passed.
    Passed,
    /// Test failed.
    Failed,
    /// XCTest reported an expected failure.
    ExpectedFailure,
    /// XCTest reported a stalled case.
    Stalled,
    /// Test was skipped.
    Skipped,
    /// Status string not modeled by ios-core yet.
    Other(String),
}

impl TestCaseStatus {
    fn from_wda_status(status: &str) -> Self {
        match status {
            "passed" => Self::Passed,
            "failed" => Self::Failed,
            "expected failure" => Self::ExpectedFailure,
            "stalled" => Self::Stalled,
            "skipped" => Self::Skipped,
            other => Self::Other(other.to_string()),
        }
    }
}

/// Failure details for a single test case.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct TestFailure {
    /// Failure message emitted by XCTest.
    pub message: String,
    /// Source file path when available.
    pub file: Option<String>,
    /// Source line when available.
    pub line: Option<u64>,
}

/// Summary for one XCTest case.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct TestCaseSummary {
    /// XCTest class name.
    pub class_name: String,
    /// XCTest method name.
    pub method_name: String,
    /// Final case status, if observed.
    pub status: Option<TestCaseStatus>,
    /// Case duration in seconds, if reported.
    pub duration_seconds: Option<f64>,
    /// First failure associated with the case, if any.
    pub failure: Option<TestFailure>,
}

/// Summary for one XCTest suite.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct TestSuiteSummary {
    /// Suite name.
    pub name: String,
    /// Start timestamp string as reported by XCTest.
    pub started_at: Option<String>,
    /// Finish timestamp string as reported by XCTest.
    pub finished_at: Option<String>,
    /// Total tests reported by the suite.
    pub test_count: Option<u64>,
    /// Skipped test count.
    pub skipped: Option<u64>,
    /// Failure count.
    pub failures: Option<u64>,
    /// Expected failure count.
    pub expected_failures: Option<u64>,
    /// Unexpected failure count.
    pub unexpected_failures: Option<u64>,
    /// Uncaught exception count.
    pub uncaught_exceptions: Option<u64>,
    /// XCTest execution duration in seconds.
    pub test_duration_seconds: Option<f64>,
    /// Total suite duration in seconds.
    pub total_duration_seconds: Option<f64>,
    /// Case summaries accumulated for this suite.
    pub cases: Vec<TestCaseSummary>,
}

/// Summary for an XCTest run.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct TestRunSummary {
    /// Whether a plan-start event was observed.
    pub began: bool,
    /// Whether a plan-finish event was observed.
    pub finished: bool,
    /// Total test count across suites.
    pub total_tests: u64,
    /// Total failed test count across suites.
    pub failed_tests: u64,
    /// Total skipped test count across suites.
    pub skipped_tests: u64,
    /// Non-debug log messages.
    pub logs: Vec<String>,
    /// Debug log messages.
    pub debug_logs: Vec<String>,
    /// Suite summaries.
    pub suites: Vec<TestSuiteSummary>,
    /// Whether log output exceeded bounds and was truncated.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub logs_truncated: bool,
    /// Result events or fields were rejected/truncated at ingestion; cannot pass CI.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub results_truncated: bool,
}

/// Supported terminal test outcomes, independent of workflow cleanup or reporting.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum TestRunOutcome {
    /// Every observed case has a supported terminal status and no failure was reported.
    Passed,
    /// XCTest reported failed cases or a nonzero suite failure counter.
    FailedTests,
}

/// Observed result evidence that cannot establish a supported terminal outcome.
#[derive(Clone, PartialEq, Eq)]
pub enum TestRunIncomplete {
    /// Result evidence was discarded by an ingestion limit.
    ResultsTruncated,
    /// No plan-start event was observed.
    PlanNotBegun,
    /// No plan-finish event was observed.
    PlanNotFinished,
    /// A case has no final status, is stalled, or has an unsupported status.
    CaseStatus {
        /// Class name reported by XCTest.
        class_name: String,
        /// Method name reported by XCTest.
        method_name: String,
        /// Actual status, if one was reported.
        status: Option<TestCaseStatus>,
    },
    /// A suite reported uncaught exceptions.
    UncaughtExceptions {
        /// Suite name reported by XCTest.
        suite_name: String,
        /// Number of reported exceptions.
        count: u64,
    },
}

impl std::fmt::Debug for TestRunIncomplete {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::ResultsTruncated => formatter.write_str("TestRunIncomplete::ResultsTruncated"),
            Self::PlanNotBegun => formatter.write_str("TestRunIncomplete::PlanNotBegun"),
            Self::PlanNotFinished => formatter.write_str("TestRunIncomplete::PlanNotFinished"),
            Self::CaseStatus { status, .. } => formatter
                .debug_struct("TestRunIncomplete::CaseStatus")
                .field("class_name_retained", &true)
                .field("method_name_retained", &true)
                .field("status", &terminal_status_label(status.as_ref()))
                .field("status_retained", &status.is_some())
                .finish(),
            Self::UncaughtExceptions { count, .. } => formatter
                .debug_struct("TestRunIncomplete::UncaughtExceptions")
                .field("suite_name_retained", &true)
                .field("count", count)
                .finish(),
        }
    }
}

fn terminal_status_label(status: Option<&TestCaseStatus>) -> &'static str {
    match status {
        None => "unfinished",
        Some(TestCaseStatus::Stalled) => "stalled",
        Some(TestCaseStatus::Other(_)) => "unsupported",
        Some(TestCaseStatus::Passed) => "passed",
        Some(TestCaseStatus::Failed) => "failed",
        Some(TestCaseStatus::Skipped) => "skipped",
        Some(TestCaseStatus::ExpectedFailure) => "expected_failure",
    }
}

impl std::fmt::Display for TestRunIncomplete {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::ResultsTruncated => {
                formatter.write_str("XCTest result evidence exceeded ingestion limits")
            }
            Self::PlanNotBegun => formatter.write_str("XCTest plan did not report a start"),
            Self::PlanNotFinished => formatter.write_str("XCTest plan did not report completion"),
            Self::CaseStatus { status, .. } => write!(
                formatter,
                "XCTest case has no supported terminal status ({})",
                terminal_status_label(status.as_ref())
            ),
            Self::UncaughtExceptions { count, .. } => write!(
                formatter,
                "XCTest suite reported {count} uncaught exception(s)"
            ),
        }
    }
}

impl std::error::Error for TestRunIncomplete {}

impl TestRunSummary {
    /// Count failure evidence without allowing a suite counter to hide concrete failed cases.
    pub fn failure_count(&self) -> u64 {
        self.failed_tests
            .max(self.suites.iter().fold(0u64, |total, suite| {
                let cases = suite
                    .cases
                    .iter()
                    .filter(|case| case.status == Some(TestCaseStatus::Failed))
                    .count() as u64;
                total.saturating_add(
                    suite
                        .failures
                        .unwrap_or(0)
                        .max(suite.unexpected_failures.unwrap_or(0))
                        .max(cases),
                )
            }))
    }

    /// Validate actual observed terminal evidence. An empty began-and-finished plan is valid;
    /// this does not infer selection coverage or compare nested suite counters.
    pub fn terminal_outcome(&self) -> Result<TestRunOutcome, TestRunIncomplete> {
        if self.results_truncated {
            return Err(TestRunIncomplete::ResultsTruncated);
        }
        if !self.began {
            return Err(TestRunIncomplete::PlanNotBegun);
        }
        if !self.finished {
            return Err(TestRunIncomplete::PlanNotFinished);
        }
        for suite in &self.suites {
            if let Some(count) = suite.uncaught_exceptions.filter(|count| *count > 0) {
                return Err(TestRunIncomplete::UncaughtExceptions {
                    suite_name: suite.name.clone(),
                    count,
                });
            }
            for case in &suite.cases {
                if matches!(
                    case.status,
                    None | Some(TestCaseStatus::Stalled) | Some(TestCaseStatus::Other(_))
                ) {
                    return Err(TestRunIncomplete::CaseStatus {
                        class_name: case.class_name.clone(),
                        method_name: case.method_name.clone(),
                        status: case.status.clone(),
                    });
                }
            }
        }
        Ok(if self.failure_count() > 0 {
            TestRunOutcome::FailedTests
        } else {
            TestRunOutcome::Passed
        })
    }
}

/// Stateful accumulator for XCTest events.
#[derive(Debug, Default, Clone)]
pub struct TestRunRecorder {
    began: bool,
    finished: bool,
    logs: Vec<String>,
    debug_logs: Vec<String>,
    log_bytes: usize,
    debug_log_bytes: usize,
    suites: Vec<TestSuiteSummary>,
    logs_truncated: bool,
    results_truncated: bool,
}

impl TestRunRecorder {
    fn total_cases(&self) -> usize {
        self.suites.iter().map(|s| s.cases.len()).sum()
    }
    /// Apply one event to the current run summary.
    pub fn apply(&mut self, event: TestExecutionEvent) {
        match event {
            TestExecutionEvent::BeganPlan => self.began = true,
            TestExecutionEvent::FinishedPlan => self.finished = true,
            TestExecutionEvent::Log { message, debug } => {
                self.push_log(message, debug);
            }
            TestExecutionEvent::SuiteStarted { name, started_at } => {
                if self.suites.len() >= MAX_SUITES {
                    self.results_truncated = true;
                    return;
                }
                let name = self.bounded_identifier(name);
                let started_at = self.bounded_field(started_at, MAX_NAME_LEN);
                self.suites.push(TestSuiteSummary {
                    name,
                    started_at,
                    finished_at: None,
                    test_count: None,
                    skipped: None,
                    failures: None,
                    expected_failures: None,
                    unexpected_failures: None,
                    uncaught_exceptions: None,
                    test_duration_seconds: None,
                    total_duration_seconds: None,
                    cases: Vec::new(),
                });
            }
            TestExecutionEvent::SuiteFinished {
                name,
                finished_at,
                test_count,
                skipped,
                failures,
                expected_failures,
                unexpected_failures,
                uncaught_exceptions,
                test_duration_seconds,
                total_duration_seconds,
            } => {
                let finished_at = self.bounded_field(finished_at, MAX_NAME_LEN);
                let Some(suite) = self.find_or_create_suite(&name) else {
                    return;
                };
                suite.finished_at = finished_at;
                suite.test_count = Some(test_count);
                suite.skipped = Some(skipped);
                suite.failures = Some(failures);
                suite.expected_failures = Some(expected_failures);
                suite.unexpected_failures = Some(unexpected_failures);
                suite.uncaught_exceptions = Some(uncaught_exceptions);
                suite.test_duration_seconds = Some(test_duration_seconds);
                suite.total_duration_seconds = Some(total_duration_seconds);
            }
            TestExecutionEvent::CaseStarted {
                class_name,
                method_name,
            } => {
                let class_name = self.bounded_identifier(class_name);
                let method_name = self.bounded_identifier(method_name);
                let total_cases = self.total_cases();
                let Some(index) = self.active_suite_index(&class_name) else {
                    return;
                };
                if total_cases >= MAX_TOTAL_CASES
                    || self.suites[index].cases.len() >= MAX_CASES_PER_SUITE
                {
                    self.results_truncated = true;
                    return;
                }
                self.suites[index].cases.push(TestCaseSummary {
                    class_name,
                    method_name,
                    status: None,
                    duration_seconds: None,
                    failure: None,
                });
            }
            TestExecutionEvent::CaseFailed {
                class_name,
                method_name,
                message,
                file,
                line,
            } => {
                let message = if message.len() > MAX_FAILURE_MESSAGE_BYTES {
                    self.results_truncated = true;
                    let mut message =
                        truncate_name(message, MAX_FAILURE_MESSAGE_BYTES - "... [truncated]".len());
                    message.push_str("... [truncated]");
                    message
                } else {
                    message
                };
                let file = self.bounded_field(file, MAX_FAILURE_FILE_BYTES);
                let Some(case) = self.find_or_create_case(&class_name, &method_name) else {
                    return;
                };
                case.status = Some(TestCaseStatus::Failed);
                case.failure = Some(TestFailure {
                    message,
                    file,
                    line,
                });
            }
            TestExecutionEvent::CaseFinished {
                class_name,
                method_name,
                status,
                duration_seconds,
            } => {
                if matches!(&status, TestCaseStatus::Other(value) if value.len() > MAX_NAME_LEN) {
                    self.results_truncated = true;
                    return;
                }
                let Some(case) = self.find_or_create_case(&class_name, &method_name) else {
                    return;
                };
                if case.status != Some(TestCaseStatus::Stalled) {
                    case.status = Some(status);
                }
                case.duration_seconds = Some(duration_seconds);
            }
        }
    }

    /// Build a serializable summary from the events applied so far.
    pub fn summary(&self) -> TestRunSummary {
        let total_tests = self
            .suites
            .iter()
            .map(|suite| suite.test_count.unwrap_or(suite.cases.len() as u64))
            .sum();
        let failed_tests = self
            .suites
            .iter()
            .map(|suite| {
                suite
                    .failures
                    .unwrap_or(0)
                    .max(suite.unexpected_failures.unwrap_or(0))
                    .max(
                        suite
                            .cases
                            .iter()
                            .filter(|case| case.status == Some(TestCaseStatus::Failed))
                            .count() as u64,
                    )
            })
            .sum();
        let skipped_tests = self
            .suites
            .iter()
            .map(|suite| {
                suite.skipped.unwrap_or_else(|| {
                    suite
                        .cases
                        .iter()
                        .filter(|case| {
                            matches!(
                                case.status,
                                Some(TestCaseStatus::Skipped)
                                    | Some(TestCaseStatus::ExpectedFailure)
                            )
                        })
                        .count() as u64
                })
            })
            .sum();

        TestRunSummary {
            began: self.began,
            finished: self.finished,
            total_tests,
            failed_tests,
            skipped_tests,
            logs: self.logs.clone(),
            debug_logs: self.debug_logs.clone(),
            suites: self.suites.clone(),
            logs_truncated: self.logs_truncated,
            results_truncated: self.results_truncated,
        }
    }

    fn push_log(&mut self, mut message: String, debug: bool) {
        let (logs, used) = if debug {
            (&mut self.debug_logs, &mut self.debug_log_bytes)
        } else {
            (&mut self.logs, &mut self.log_bytes)
        };
        if logs.len() >= MAX_CAPTURED_LOG_MESSAGES || *used >= MAX_CAPTURED_LOG_BYTES {
            if !self.logs_truncated {
                self.logs_truncated = true;
                if logs.len() < MAX_CAPTURED_LOG_MESSAGES && *used + 64 <= MAX_CAPTURED_LOG_BYTES {
                    logs.push("[log output truncated: message or byte limit reached]".into());
                }
            }
            return;
        }
        let remaining = MAX_CAPTURED_LOG_BYTES - *used;
        if message.len() > remaining {
            let marker = "... [truncated]";
            let mut end = remaining.saturating_sub(marker.len());
            while end > 0 && !message.is_char_boundary(end) {
                end -= 1;
            }
            message.truncate(end);
            message.push_str(&marker[..marker.len().min(remaining)]);
            self.logs_truncated = true;
        }
        if message.is_empty() {
            return;
        }
        *used = (*used).saturating_add(message.len());
        logs.push(message);
    }

    fn bounded_field(&mut self, value: Option<String>, max: usize) -> Option<String> {
        value.map(|value| {
            if value.len() > max {
                self.results_truncated = true;
            }
            truncate_name(value, max)
        })
    }

    fn bounded_identifier(&mut self, value: String) -> String {
        if value.len() <= MAX_NAME_LEN {
            return value;
        }
        use sha2::Digest;
        self.results_truncated = true;
        // Retain readable bounded labels without conflating common-prefix identities.
        let digest = hex::encode(sha2::Sha256::digest(value.as_bytes()));
        let mut label = truncate_name(value, MAX_NAME_LEN - digest.len() - 1);
        label.push('#');
        label.push_str(&digest);
        label
    }

    fn find_or_create_case(
        &mut self,
        class_name: &str,
        method_name: &str,
    ) -> Option<&mut TestCaseSummary> {
        let class_name = self.bounded_identifier(class_name.to_owned());
        let method_name = self.bounded_identifier(method_name.to_owned());
        let total_cases = self.total_cases();
        let suite_index = self.active_suite_index(&class_name)?;
        let existing = self.suites[suite_index]
            .cases
            .iter()
            .rposition(|case| case.class_name == class_name && case.method_name == method_name);
        let index = if let Some(index) = existing {
            index
        } else {
            if total_cases >= MAX_TOTAL_CASES
                || self.suites[suite_index].cases.len() >= MAX_CASES_PER_SUITE
            {
                self.results_truncated = true;
                return None;
            }
            let suite = &mut self.suites[suite_index];
            let index = suite.cases.len();
            suite.cases.push(TestCaseSummary {
                class_name: class_name.to_string(),
                method_name: method_name.to_string(),
                status: None,
                duration_seconds: None,
                failure: None,
            });
            index
        };
        Some(&mut self.suites[suite_index].cases[index])
    }

    fn suite_index(&mut self, name: &str) -> Option<usize> {
        let name = self.bounded_identifier(name.to_owned());
        if let Some(index) = self.suites.iter().rposition(|suite| suite.name == name) {
            return Some(index);
        }
        if self.suites.len() >= MAX_SUITES {
            self.results_truncated = true;
            return None;
        }
        let index = self.suites.len();
        self.suites.push(TestSuiteSummary {
            name: name.to_string(),
            started_at: None,
            finished_at: None,
            test_count: None,
            skipped: None,
            failures: None,
            expected_failures: None,
            unexpected_failures: None,
            uncaught_exceptions: None,
            test_duration_seconds: None,
            total_duration_seconds: None,
            cases: Vec::new(),
        });
        Some(index)
    }

    fn find_or_create_suite(&mut self, name: &str) -> Option<&mut TestSuiteSummary> {
        let index = self.suite_index(name)?;
        Some(&mut self.suites[index])
    }

    fn active_suite_index(&mut self, class_name: &str) -> Option<usize> {
        if let Some(index) = self
            .suites
            .iter()
            .rposition(|suite| suite.test_count.is_none())
        {
            return Some(index);
        }
        self.suite_index(class_name)
    }
}

impl TestRunSummary {
    /// Serialize this result summary as JUnit XML.
    ///
    /// Expected failures retain an explicit status property and use JUnit's standard `<skipped>`
    /// element, matching go-ios. Stalled/unknown cases use `<error>` so consumers do not mistake
    /// an incomplete result for a passing test.
    pub fn to_junit_xml(&self) -> String {
        self.to_junit_xml_with_diagnostic(None)
    }

    /// Serialize this result summary as JUnit XML and include a diagnostic
    /// error when startup/result collection was incomplete.
    pub fn to_junit_xml_with_diagnostic(&self, diagnostic: Option<&str>) -> String {
        let mut suites = String::new();
        let mut total_tests = 0u64;
        let mut total_failures = 0u64;
        let mut total_errors = 0u64;
        let mut total_skipped = 0u64;
        let mut total_time = 0.0f64;

        for suite in &self.suites {
            let report = JUnitSuiteReport::from_suite(suite);
            total_tests = total_tests.saturating_add(report.tests);
            total_failures = total_failures.saturating_add(report.failures);
            total_errors = total_errors.saturating_add(report.errors);
            total_skipped = total_skipped.saturating_add(report.skipped);
            total_time += report.time;
            render_junit_suite(&mut suites, suite, &report);
        }

        let diagnostic = diagnostic
            .map(|value| truncate_utf8(value, MAX_JUNIT_DIAGNOSTIC_BYTES))
            .or_else(|| {
                self.results_truncated
                    .then(|| TestRunIncomplete::ResultsTruncated.to_string())
            })
            .or_else(|| (!self.finished).then(|| "XCTest run did not finish".to_string()));
        if let Some(diagnostic) = diagnostic.as_deref() {
            total_errors = total_errors.saturating_add(1);
            suites.push_str("<testsuite");
            xml_attr(&mut suites, "name", "xctest-diagnostic");
            xml_attr(&mut suites, "tests", "0");
            xml_attr(&mut suites, "failures", "0");
            xml_attr(&mut suites, "errors", "1");
            xml_attr(&mut suites, "skipped", "0");
            xml_attr(&mut suites, "time", "0.000");
            suites.push('>');
            suites.push_str("<error");
            xml_attr(&mut suites, "type", "xctest_diagnostic");
            xml_attr(&mut suites, "message", diagnostic);
            suites.push('>');
            xml_text(&mut suites, diagnostic);
            suites.push_str("</error></testsuite>");
        }

        if !self.logs.is_empty() || !self.debug_logs.is_empty() {
            suites.push_str("<testsuite");
            xml_attr(&mut suites, "name", "xctest-output");
            xml_attr(&mut suites, "tests", "0");
            xml_attr(&mut suites, "failures", "0");
            xml_attr(&mut suites, "errors", "0");
            xml_attr(&mut suites, "skipped", "0");
            xml_attr(&mut suites, "time", "0.000");
            suites.push('>');
            if !self.logs.is_empty() {
                suites.push_str("<system-out>");
                xml_text(&mut suites, &self.logs.join("\n"));
                suites.push_str("</system-out>");
            }
            if !self.debug_logs.is_empty() {
                suites.push_str("<system-err>");
                xml_text(&mut suites, &self.debug_logs.join("\n"));
                suites.push_str("</system-err>");
            }
            suites.push_str("</testsuite>");
        }

        format!(
            "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<testsuites name=\"XCTest\" tests=\"{total_tests}\" failures=\"{total_failures}\" errors=\"{total_errors}\" skipped=\"{total_skipped}\" time=\"{:.3}\">{suites}</testsuites>\n",
            finite_duration(total_time)
        )
    }
}

/// Atomically write a JUnit report next to the requested destination.
///
/// The temporary file is created in the destination directory, flushed, and
/// renamed into place. On Unix, rename atomically replaces an existing file; Windows uses
/// MoveFileEx(REPLACE_EXISTING) so overwriting an existing report has the same contract.
pub fn write_junit_xml_atomic(summary: &TestRunSummary, path: &Path) -> io::Result<()> {
    write_junit_xml_text_atomic(&summary.to_junit_xml(), path)
}

/// Atomically write a JUnit report with a startup/result-collection diagnostic.
pub fn write_junit_xml_atomic_with_diagnostic(
    summary: &TestRunSummary,
    path: &Path,
    diagnostic: &str,
) -> io::Result<()> {
    write_junit_xml_text_atomic(
        &summary.to_junit_xml_with_diagnostic(Some(diagnostic)),
        path,
    )
}

fn write_junit_xml_text_atomic(xml: &str, path: &Path) -> io::Result<()> {
    let (mut file, pending) = crate::staged_file::create(path)?;
    let result = file.write_all(xml.as_bytes()).and_then(|_| file.sync_all());
    drop(file);
    result?;
    pending.commit()
}

#[derive(Debug, Clone, Copy)]
struct JUnitSuiteReport {
    tests: u64,
    failures: u64,
    errors: u64,
    skipped: u64,
    time: f64,
}

impl JUnitSuiteReport {
    fn from_suite(suite: &TestSuiteSummary) -> Self {
        let case_failures = suite
            .cases
            .iter()
            .filter(|case| case.status == Some(TestCaseStatus::Failed))
            .count() as u64;
        let case_errors = suite
            .cases
            .iter()
            .filter(|case| {
                matches!(
                    case.status,
                    None | Some(TestCaseStatus::Stalled) | Some(TestCaseStatus::Other(_))
                )
            })
            .count() as u64;
        let case_skipped = suite
            .cases
            .iter()
            .filter(|case| {
                matches!(
                    case.status,
                    Some(TestCaseStatus::Skipped) | Some(TestCaseStatus::ExpectedFailure)
                )
            })
            .count() as u64;
        // JUnit counts concrete testcase children. XCTest's suite counter may include cases for
        // which no event was delivered, but emitting a larger count would violate the common
        // `tests == number of testcase elements` contract used by go-ios consumers.
        let tests = suite.cases.len() as u64;
        // JUnit's suite time is execution time. XCTest's totalDuration also includes setup and
        // teardown, so match go-ios and use testDuration, falling back to case durations when
        // XCTest reported zero for an incomplete suite.
        let test_duration = suite.test_duration_seconds.unwrap_or(0.0);
        let time = if test_duration.is_finite() && test_duration > 0.0 {
            test_duration
        } else {
            suite
                .cases
                .iter()
                .filter_map(|case| case.duration_seconds)
                .sum()
        };
        Self {
            tests,
            failures: case_failures,
            errors: suite
                .uncaught_exceptions
                .unwrap_or(0)
                .saturating_add(case_errors),
            skipped: case_skipped,
            time: finite_duration(time),
        }
    }
}

fn render_junit_suite(output: &mut String, suite: &TestSuiteSummary, report: &JUnitSuiteReport) {
    output.push_str("<testsuite");
    xml_attr(output, "name", &suite.name);
    xml_attr(output, "tests", &report.tests.to_string());
    xml_attr(output, "failures", &report.failures.to_string());
    xml_attr(output, "errors", &report.errors.to_string());
    xml_attr(output, "skipped", &report.skipped.to_string());
    xml_attr(output, "time", &format!("{:.3}", report.time));
    output.push('>');
    if let Some(expected_failures) = suite.expected_failures {
        output.push_str("<properties><property");
        xml_attr(output, "name", "expected_failures");
        xml_attr(output, "value", &expected_failures.to_string());
        output.push_str("/></properties>");
    }

    for case in &suite.cases {
        let duration = finite_duration(case.duration_seconds.unwrap_or(0.0));
        output.push_str("<testcase");
        xml_attr(output, "classname", &case.class_name);
        xml_attr(output, "name", &case.method_name);
        xml_attr(output, "time", &format!("{duration:.3}"));
        output.push('>');
        match case.status.as_ref() {
            Some(TestCaseStatus::Failed) => {
                if let Some(failure) = &case.failure {
                    render_failure(output, failure);
                } else {
                    output.push_str("<failure type=\"XCTestFailure\" message=\"failed\"/>");
                }
            }
            Some(TestCaseStatus::Skipped) => output.push_str("<skipped/>"),
            Some(TestCaseStatus::ExpectedFailure) => {
                output.push_str("<skipped");
                xml_attr(output, "message", "expected failure");
                output.push_str("/>");
                render_status_property(output, "expected_failure");
            }
            Some(TestCaseStatus::Stalled) => {
                render_error(output, "stalled", "XCTest case stalled");
            }
            Some(TestCaseStatus::Other(status)) => {
                render_status_property(output, status);
                render_error(output, "unknown_status", status);
            }
            None => {
                render_status_property(output, "unknown");
                render_error(
                    output,
                    "incomplete",
                    "XCTest case did not report a final status",
                );
            }
            Some(TestCaseStatus::Passed) => {}
        }
        output.push_str("</testcase>");
    }
    output.push_str("</testsuite>");
}

fn render_failure(output: &mut String, failure: &TestFailure) {
    output.push_str("<failure");
    xml_attr(output, "type", "XCTestFailure");
    xml_attr(output, "message", &failure.message);
    output.push('>');
    if let Some(file) = &failure.file {
        xml_text(output, file);
        if let Some(line) = failure.line {
            output.push(':');
            output.push_str(&line.to_string());
        }
        output.push_str(": ");
    }
    xml_text(output, &failure.message);
    output.push_str("</failure>");
}

fn render_error(output: &mut String, error_type: &str, message: &str) {
    output.push_str("<error");
    xml_attr(output, "type", error_type);
    xml_attr(output, "message", message);
    output.push_str("/>");
}

fn render_status_property(output: &mut String, status: &str) {
    output.push_str("<properties><property");
    xml_attr(output, "name", "status");
    xml_attr(output, "value", status);
    output.push_str("/></properties>");
}

fn xml_attr(output: &mut String, name: &str, value: &str) {
    output.push(' ');
    output.push_str(name);
    output.push_str("=\"");
    xml_text(output, value);
    output.push('"');
}

fn xml_text(output: &mut String, value: &str) {
    for character in value.chars() {
        if !is_xml_10_character(character) {
            output.push('\u{FFFD}');
            continue;
        }
        match character {
            '&' => output.push_str("&amp;"),
            '<' => output.push_str("&lt;"),
            '>' => output.push_str("&gt;"),
            '"' => output.push_str("&quot;"),
            '\'' => output.push_str("&apos;"),
            character => output.push(character),
        }
    }
}

fn is_xml_10_character(character: char) -> bool {
    matches!(
        character,
        '\u{9}' | '\u{A}' | '\u{D}' | '\u{20}'..='\u{D7FF}' | '\u{E000}'..='\u{FFFD}' | '\u{10000}'..='\u{10FFFF}'
    )
}

fn finite_duration(value: f64) -> f64 {
    if value.is_finite() && value >= 0.0 {
        value
    } else {
        0.0
    }
}

fn truncate_utf8(value: &str, limit: usize) -> String {
    if value.len() <= limit {
        return value.to_string();
    }
    let mut end = limit;
    while end > 0 && !value.is_char_boundary(end) {
        end -= 1;
    }
    value[..end].to_string()
}

fn string_arg(args: &[NSObject], index: usize) -> Option<String> {
    args.get(index)
        .and_then(NSObject::as_str)
        .map(ToString::to_string)
}

fn optional_string_arg(args: &[NSObject], index: usize) -> Option<String> {
    string_arg(args, index).filter(|value| !value.is_empty())
}

fn uint_arg(args: &[NSObject], index: usize) -> Option<u64> {
    match args.get(index)? {
        NSObject::Uint(value) => Some(*value),
        NSObject::Int(value) if *value >= 0 => Some(*value as u64),
        _ => None,
    }
}

fn double_arg(args: &[NSObject], index: usize) -> Option<f64> {
    match args.get(index)? {
        NSObject::Double(value) => Some(*value),
        NSObject::Int(value) => Some(*value as f64),
        NSObject::Uint(value) => Some(*value as f64),
        _ => None,
    }
}

fn identifier_suite_name(value: Option<&NSObject>) -> Option<String> {
    match value? {
        NSObject::String(value) => Some(value.clone()),
        NSObject::Array(values) => values.first().and_then(|value| match value {
            NSObject::String(name) => Some(name.clone()),
            _ => None,
        }),
        NSObject::Dict(dict) => dict
            .get("container")
            .or_else(|| dict.get("suite"))
            .or_else(|| dict.get("testClass"))
            .and_then(NSObject::as_str)
            .map(ToString::to_string),
        _ => None,
    }
}

#[cfg(test)]
mod terminal_outcome_tests {
    use super::*;

    fn recorded_case(status: Option<TestCaseStatus>) -> TestRunRecorder {
        let mut recorder = TestRunRecorder::default();
        recorder.apply(TestExecutionEvent::BeganPlan);
        recorder.apply(TestExecutionEvent::CaseStarted {
            class_name: "Cases".into(),
            method_name: "example".into(),
        });
        if let Some(status) = status {
            recorder.apply(TestExecutionEvent::CaseFinished {
                class_name: "Cases".into(),
                method_name: "example".into(),
                status,
                duration_seconds: 0.1,
            });
        }
        recorder.apply(TestExecutionEvent::FinishedPlan);
        recorder
    }

    fn finish_suite(recorder: &mut TestRunRecorder, failures: u64, uncaught_exceptions: u64) {
        recorder.apply(TestExecutionEvent::SuiteFinished {
            name: "Cases".into(),
            finished_at: None,
            test_count: 1,
            skipped: 0,
            failures,
            expected_failures: 0,
            unexpected_failures: 0,
            uncaught_exceptions,
            test_duration_seconds: 0.1,
            total_duration_seconds: 0.1,
        });
    }

    #[test]
    fn terminal_outcome_requires_observed_start_and_finish_but_allows_empty_plan() {
        let mut recorder = TestRunRecorder::default();
        recorder.apply(TestExecutionEvent::FinishedPlan);
        assert_eq!(
            recorder.summary().terminal_outcome(),
            Err(TestRunIncomplete::PlanNotBegun)
        );
        let mut recorder = TestRunRecorder::default();
        recorder.apply(TestExecutionEvent::BeganPlan);
        assert_eq!(
            recorder.summary().terminal_outcome(),
            Err(TestRunIncomplete::PlanNotFinished)
        );
        recorder.apply(TestExecutionEvent::FinishedPlan);
        assert_eq!(
            recorder.summary().terminal_outcome(),
            Ok(TestRunOutcome::Passed)
        );
        assert_eq!(recorder.summary().total_tests, 0);
    }

    #[test]
    fn terminal_outcome_rejects_unfinished_stalled_unknown_and_uncaught_evidence() {
        for status in [
            None,
            Some(TestCaseStatus::Stalled),
            Some(TestCaseStatus::Other("future".into())),
        ] {
            let summary = recorded_case(status.clone()).summary();
            assert_eq!(
                summary.terminal_outcome(),
                Err(TestRunIncomplete::CaseStatus {
                    class_name: "Cases".into(),
                    method_name: "example".into(),
                    status,
                })
            );
            assert!(summary.finished);
        }
        let mut recorder = recorded_case(Some(TestCaseStatus::Passed));
        finish_suite(&mut recorder, 0, 2);
        assert_eq!(
            recorder.summary().terminal_outcome(),
            Err(TestRunIncomplete::UncaughtExceptions {
                suite_name: "Cases".into(),
                count: 2,
            })
        );
    }

    #[test]
    fn terminal_failure_evidence_survives_zero_suite_counter_and_zero_summary_counter() {
        let mut recorder = recorded_case(Some(TestCaseStatus::Failed));
        finish_suite(&mut recorder, 0, 0);
        let mut summary = recorder.summary();
        assert_eq!(summary.suites[0].failures, Some(0));
        assert_eq!(summary.failed_tests, 1);
        summary.failed_tests = 0;
        assert_eq!(summary.failure_count(), 1);
        assert_eq!(summary.terminal_outcome(), Ok(TestRunOutcome::FailedTests));
        let mut recorder = recorded_case(Some(TestCaseStatus::Passed));
        finish_suite(&mut recorder, 3, 0);
        assert_eq!(recorder.summary().failure_count(), 3);
        assert_eq!(
            recorder.summary().terminal_outcome(),
            Ok(TestRunOutcome::FailedTests)
        );
        let mut recorder = recorded_case(Some(TestCaseStatus::Passed));
        recorder.apply(TestExecutionEvent::SuiteFinished {
            name: "Cases".into(),
            finished_at: None,
            test_count: 1,
            skipped: 0,
            failures: 0,
            expected_failures: 0,
            unexpected_failures: 2,
            uncaught_exceptions: 0,
            test_duration_seconds: 0.1,
            total_duration_seconds: 0.1,
        });
        let summary = recorder.summary();
        assert_eq!(summary.suites[0].failures, Some(0));
        assert_eq!(summary.suites[0].unexpected_failures, Some(2));
        assert_eq!(summary.failed_tests, 2);
        assert_eq!(summary.failure_count(), 2);
        assert_eq!(summary.terminal_outcome(), Ok(TestRunOutcome::FailedTests));
    }

    #[test]
    fn expected_failure_details_and_skipped_cases_are_supported_terminal_outcomes() {
        for status in [
            TestCaseStatus::Passed,
            TestCaseStatus::Skipped,
            TestCaseStatus::ExpectedFailure,
        ] {
            let mut recorder = TestRunRecorder::default();
            recorder.apply(TestExecutionEvent::BeganPlan);
            recorder.apply(TestExecutionEvent::CaseFailed {
                class_name: "Cases".into(),
                method_name: "example".into(),
                message: "retained description".into(),
                file: None,
                line: None,
            });
            recorder.apply(TestExecutionEvent::CaseFinished {
                class_name: "Cases".into(),
                method_name: "example".into(),
                status,
                duration_seconds: 0.1,
            });
            recorder.apply(TestExecutionEvent::FinishedPlan);
            let summary = recorder.summary();
            assert!(summary.suites[0].cases[0].failure.is_some());
            assert_eq!(summary.failed_tests, 0);
            assert_eq!(summary.terminal_outcome(), Ok(TestRunOutcome::Passed));
        }
    }

    #[test]
    fn incomplete_error_keeps_names_and_status_while_debug_and_display_are_safe() {
        let error = TestRunIncomplete::CaseStatus {
            class_name: "private-class-marker".into(),
            method_name: "private-method-marker".into(),
            status: Some(TestCaseStatus::Other("private-status-marker".into())),
        };
        let debug = format!("{error:?}");
        let display = error.to_string();
        for marker in [
            "private-class-marker",
            "private-method-marker",
            "private-status-marker",
        ] {
            assert!(!debug.contains(marker));
            assert!(!display.contains(marker));
        }
        let TestRunIncomplete::CaseStatus {
            class_name,
            method_name,
            status,
        } = error
        else {
            panic!("original terminal evidence lost");
        };
        assert_eq!(class_name, "private-class-marker");
        assert_eq!(method_name, "private-method-marker");
        assert_eq!(
            status,
            Some(TestCaseStatus::Other("private-status-marker".into()))
        );
        let error = TestRunIncomplete::UncaughtExceptions {
            suite_name: "private-suite-marker".into(),
            count: 2,
        };
        assert!(!format!("{error:?}").contains("private-suite-marker"));
        assert!(!error.to_string().contains("private-suite-marker"));
    }

    #[test]
    fn test_recorder_bounds_and_truncation_markers() {
        let mut recorder = TestRunRecorder::default();
        recorder.apply(TestExecutionEvent::BeganPlan);

        // Long suite name gets truncated
        let long_name = "s".repeat(1000);
        recorder.apply(TestExecutionEvent::SuiteStarted {
            name: long_name.clone(),
            started_at: None,
        });
        assert_eq!(recorder.suites[0].name.len(), MAX_NAME_LEN);

        // Long case class and method get truncated
        let long_case = "c".repeat(1000);
        recorder.apply(TestExecutionEvent::CaseStarted {
            class_name: long_case.clone(),
            method_name: long_case.clone(),
        });
        assert_eq!(recorder.suites[0].cases[0].class_name.len(), MAX_NAME_LEN);
        assert_eq!(recorder.suites[0].cases[0].method_name.len(), MAX_NAME_LEN);

        // Long failure message gets truncated with marker
        let long_message = "m".repeat(100_000);
        recorder.apply(TestExecutionEvent::CaseFailed {
            class_name: long_case.clone(),
            method_name: long_case.clone(),
            message: long_message,
            file: Some("f".repeat(2000)),
            line: Some(42),
        });
        let case = &recorder.suites[0].cases[0];
        let failure = case.failure.as_ref().unwrap();
        assert!(failure.message.ends_with("... [truncated]"));
        assert!(failure.message.len() <= MAX_FAILURE_MESSAGE_BYTES + 32);
        assert_eq!(failure.file.as_ref().unwrap().len(), MAX_FAILURE_FILE_BYTES);

        // Huge log message gets truncated with marker
        let huge_log = "L".repeat(9 * 1024 * 1024);
        recorder.apply(TestExecutionEvent::Log {
            message: huge_log,
            debug: false,
        });
        assert!(recorder.logs_truncated);
        assert!(recorder.logs.iter().any(|l| l.contains("truncated")));

        recorder.apply(TestExecutionEvent::FinishedPlan);
        let summary = recorder.summary();
        assert!(summary.logs_truncated);
    }
}
