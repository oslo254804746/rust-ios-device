//! XCTest result event parsing and run summary accumulation.
//!
//! Testmanager reports progress as DTX method invocations using private XCTest
//! selectors. This module translates the selectors into stable Rust events and
//! accumulates those events into a serializable summary for CLI and binding users.

use crate::services::dtx::{DtxMessage, DtxPayload, NSObject};
use serde::Serialize;

pub const DID_BEGIN_EXECUTING_TEST_PLAN_SELECTOR: &str = "_XCT_didBeginExecutingTestPlan";
pub const DID_FINISH_EXECUTING_TEST_PLAN_SELECTOR: &str = "_XCT_didFinishExecutingTestPlan";
pub const LOG_MESSAGE_SELECTOR: &str = "_XCT_logMessage:";
pub const LOG_DEBUG_MESSAGE_SELECTOR: &str = "_XCT_logDebugMessage:";
pub const TEST_SUITE_STARTED_SELECTOR: &str = "_XCT_testSuite:didStartAt:";
pub const TEST_SUITE_FINISHED_SELECTOR: &str =
    "_XCT_testSuite:didFinishAt:runCount:withFailures:unexpected:testDuration:totalDuration:";
pub const TEST_SUITE_FINISHED_WITH_SKIP_SELECTOR: &str =
    "_XCT_testSuiteWithIdentifier:didFinishAt:runCount:skipCount:failureCount:expectedFailureCount:uncaughtExceptionCount:testDuration:totalDuration:";
pub const TEST_CASE_STARTED_SELECTOR: &str = "_XCT_testCaseDidStartForTestClass:method:";
pub const TEST_CASE_FINISHED_SELECTOR: &str =
    "_XCT_testCaseDidFinishForTestClass:method:withStatus:duration:";
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
    Log { message: String, debug: bool },
    /// A test suite started.
    SuiteStarted {
        name: String,
        started_at: Option<String>,
    },
    /// A test suite finished and reported aggregate counts.
    SuiteFinished {
        name: String,
        finished_at: Option<String>,
        test_count: u64,
        skipped: u64,
        failures: u64,
        expected_failures: u64,
        unexpected_failures: u64,
        uncaught_exceptions: u64,
        test_duration_seconds: f64,
        total_duration_seconds: f64,
    },
    /// A test case started.
    CaseStarted {
        class_name: String,
        method_name: String,
    },
    /// A test case reported a failure.
    CaseFailed {
        class_name: String,
        method_name: String,
        message: String,
        file: Option<String>,
        line: Option<u64>,
    },
    /// A test case finished with a final status.
    CaseFinished {
        class_name: String,
        method_name: String,
        status: TestCaseStatus,
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
}

/// Stateful accumulator for XCTest events.
#[derive(Debug, Default, Clone)]
pub struct TestRunRecorder {
    began: bool,
    finished: bool,
    logs: Vec<String>,
    debug_logs: Vec<String>,
    suites: Vec<TestSuiteSummary>,
}

impl TestRunRecorder {
    /// Apply one event to the current run summary.
    pub fn apply(&mut self, event: TestExecutionEvent) {
        match event {
            TestExecutionEvent::BeganPlan => self.began = true,
            TestExecutionEvent::FinishedPlan => self.finished = true,
            TestExecutionEvent::Log { message, debug } => {
                if debug {
                    self.debug_logs.push(message);
                } else {
                    self.logs.push(message);
                }
            }
            TestExecutionEvent::SuiteStarted { name, started_at } => {
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
                let suite = self.find_or_create_suite(&name);
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
                let suite = self.find_or_create_suite(&class_name);
                suite.cases.push(TestCaseSummary {
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
                let case = self.find_or_create_case(&class_name, &method_name);
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
                let case = self.find_or_create_case(&class_name, &method_name);
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
                suite.failures.unwrap_or_else(|| {
                    suite
                        .cases
                        .iter()
                        .filter(|case| case.status == Some(TestCaseStatus::Failed))
                        .count() as u64
                })
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
                        .filter(|case| case.status == Some(TestCaseStatus::Skipped))
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
        }
    }

    fn find_or_create_case(&mut self, class_name: &str, method_name: &str) -> &mut TestCaseSummary {
        let suite = self.find_or_create_suite(class_name);
        if let Some(index) = suite
            .cases
            .iter()
            .rposition(|case| case.class_name == class_name && case.method_name == method_name)
        {
            return &mut suite.cases[index];
        }
        suite.cases.push(TestCaseSummary {
            class_name: class_name.to_string(),
            method_name: method_name.to_string(),
            status: None,
            duration_seconds: None,
            failure: None,
        });
        suite.cases.last_mut().expect("just pushed test case")
    }

    fn find_or_create_suite(&mut self, name: &str) -> &mut TestSuiteSummary {
        if let Some(index) = self.suites.iter().rposition(|suite| suite.name == name) {
            return &mut self.suites[index];
        }
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
        self.suites.last_mut().expect("just pushed suite")
    }
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
